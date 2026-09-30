//! An expectimax agent, run over many games at once.
//!
//! Run with `cargo run --release --example expectimax`. Flags:
//! `--games N`, `--depth N`, `--floor F`, or a bare number for the game count.
//!
//! The search is the ordinary one for this game: maximise over the four moves,
//! average over the tile the game then places, weighted 9:1 between a 2 and a
//! 4. What makes it play well rather than merely legally is the position
//! evaluation.
//!
//! Two knobs set the time per move, and they bind at opposite ends of the
//! game. `--depth` bounds the late game, where the board is crowded and a
//! chance node has only a few children to branch over. `--floor` bounds the
//! early game, where thirty children per chance node means depth alone is no
//! bound at all; a branch whose probability of being reached falls below it is
//! evaluated instead of expanded.
//!
//! Games are independent, so the parallelism is one game per core rather than
//! anything inside the search. A split search would have to share a
//! transposition table across threads and would cap out at the four root
//! moves; sharding games keeps every core busy on work that never
//! synchronises, and the thing being measured is a score distribution over
//! games anyway.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use twenty48::{Board, Direction, Game, Status};

struct Config {
    games: usize,
    depth: u32,
    floor: f32,
}

fn parse_args() -> Config {
    // Depth 5 at this floor is about 2.4ms per move and reaches 32768; depth 4
    // is under 1ms but scores roughly half as much, and depth 5 at 1e-4 costs
    // 11ms for little more. See the table in the README.
    let mut config = Config {
        games: 12,
        depth: 5,
        floor: 1e-3,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if !flag.starts_with("--") {
            config.games = flag.parse().expect("the game count must be a number");
            i += 1;
            continue;
        }
        let value = args
            .get(i + 1)
            .unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag {
            "--games" => config.games = value.parse().expect("--games must be a number"),
            "--depth" => config.depth = value.parse().expect("--depth must be a number"),
            "--floor" => config.floor = value.parse().expect("--floor must be a number"),
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }

    assert!(config.depth >= 1, "--depth must be at least 1");
    config
}

fn main() {
    let config = parse_args();
    let games = config.games;
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(games);

    let heuristic = Heuristic::new();
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(games));

    println!(
        "expectimax over {games} games on {threads} threads, \
         depth {} floor {}\n",
        config.depth, config.floor
    );
    let start = Instant::now();

    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut search = Search::new(&heuristic, &config);
                loop {
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= games {
                        break;
                    }
                    let outcome = search.play(seed as u64);
                    let mut done = done.lock().expect("a worker panicked");
                    println!(
                        "  game {:>3}  score {:>7}  tile {:>5}  {:>5} moves  \
                         {:>5.1} M nodes/s  {:.0?}",
                        seed,
                        outcome.score,
                        outcome.max_tile,
                        outcome.moves,
                        outcome.nodes as f64 / outcome.elapsed.as_secs_f64() / 1e6,
                        outcome.elapsed
                    );
                    done.push(outcome);
                }
            });
        }
    });

    let elapsed = start.elapsed();
    report(done.into_inner().expect("a worker panicked"), elapsed);
}

struct Outcome {
    score: u32,
    max_tile: u16,
    moves: u32,
    nodes: u64,
    elapsed: Duration,
}

fn report(mut outcomes: Vec<Outcome>, elapsed: Duration) {
    outcomes.sort_unstable_by_key(|outcome| outcome.score);

    let games = outcomes.len();
    let total: u64 = outcomes.iter().map(|o| u64::from(o.score)).sum();
    let nodes: u64 = outcomes.iter().map(|o| o.nodes).sum();
    let moves: u64 = outcomes.iter().map(|o| u64::from(o.moves)).sum();
    let cpu: Duration = outcomes.iter().map(|o| o.elapsed).sum();

    println!("\n{games} games in {elapsed:.0?}, {cpu:.0?} of CPU time");
    println!(
        "{:.1} M nodes/s per core, {:.1} M across the machine",
        nodes as f64 / cpu.as_secs_f64() / 1e6,
        nodes as f64 / elapsed.as_secs_f64() / 1e6
    );
    println!(
        "{:.0} nodes and {:.2} ms per move, {moves} moves over all games",
        nodes as f64 / moves as f64,
        cpu.as_secs_f64() * 1e3 / moves as f64
    );
    println!(
        "\nscore: mean {:.0}, median {}, best {}",
        total as f64 / games as f64,
        outcomes[games / 2].score,
        outcomes[games - 1].score
    );

    println!("\nlargest tile reached:");
    let mut tiles: Vec<u16> = outcomes.iter().map(|o| o.max_tile).collect();
    tiles.sort_unstable();
    tiles.dedup();
    for tile in tiles.into_iter().rev() {
        let count = outcomes.iter().filter(|o| o.max_tile == tile).count();
        println!("  {tile:>5}  {count:>4}  {:>3}%", count * 100 / games);
    }
}

/// Direct-mapped transposition table.
///
/// A `HashMap` spends more time hashing and probing than the eight lookups an
/// evaluation costs, which is the wrong trade for a cache that is allowed to
/// miss. One multiply picks a slot, a mismatched key is simply a miss, and
/// there is no bookkeeping to clear between moves.
///
/// Entries are scoped to one move. A stored value depends on the probability
/// at which its node happened to be expanded, because a node reached on an
/// unlikely path has its children cut off by the floor, so the value is only
/// meaningful inside the search that produced it. Carrying entries across
/// moves costs about a third fewer nodes per move and is tempting for that
/// reason, but it hands later searches values computed under a floor they did
/// not use. The `stamp` scopes entries to a move without walking several MiB
/// between them.
struct Table {
    slots: Box<[Entry]>,
    stamp: u32,
}

#[derive(Clone, Copy, Default)]
struct Entry {
    key: u64,
    value: f32,
    depth: u32,
    stamp: u32,
}

impl Table {
    /// 256K slots is 6 MiB per thread, enough to hold a whole search and small
    /// enough that twelve of them stay out of each other's way.
    const SLOTS: usize = 1 << 18;

    fn new() -> Table {
        Table {
            slots: vec![Entry::default(); Self::SLOTS].into_boxed_slice(),
            // Entries start at zero, so the first move must not be stamp zero.
            stamp: 1,
        }
    }

    fn clear(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.stamp = 1;
        }
    }

    fn slot(key: u64) -> usize {
        let mut x = key;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 29;
        (x as usize) & (Table::SLOTS - 1)
    }

    fn get(&self, key: u64, depth: u32) -> Option<f32> {
        let entry = &self.slots[Self::slot(key)];
        let usable = entry.stamp == self.stamp && entry.key == key && entry.depth >= depth;
        usable.then_some(entry.value)
    }

    fn insert(&mut self, key: u64, depth: u32, value: f32) {
        let stamp = self.stamp;
        let slot = &mut self.slots[Self::slot(key)];
        // Prefer the deeper result when two boards collide, since it cost more
        // to produce and is the better answer if it is ever asked for again.
        if slot.stamp != stamp || slot.key != key || depth >= slot.depth {
            *slot = Entry {
                key,
                value,
                depth,
                stamp,
            };
        }
    }
}

struct Search<'a> {
    heuristic: &'a Heuristic,
    config: &'a Config,
    table: Table,
    nodes: u64,
}

impl<'a> Search<'a> {
    fn new(heuristic: &'a Heuristic, config: &'a Config) -> Search<'a> {
        Search {
            heuristic,
            config,
            table: Table::new(),
            nodes: 0,
        }
    }

    fn play(&mut self, seed: u64) -> Outcome {
        let mut game = Game::new(seed);
        let before = self.nodes;
        let start = Instant::now();

        while game.status() == Status::Playing {
            let dir = self.best_move(game.board()).expect("a playable game moves");
            game.play(dir).expect("the search only returns legal moves");
        }

        Outcome {
            score: game.score(),
            max_tile: game.board().max_tile(),
            moves: game.moves(),
            nodes: self.nodes - before,
            elapsed: start.elapsed(),
        }
    }

    fn best_move(&mut self, board: Board) -> Option<Direction> {
        let depth = self.config.depth;
        self.table.clear();

        let mut best = None;
        let mut best_value = f32::NEG_INFINITY;
        let moves = board.shift_all();

        for dir in Direction::ALL {
            let next = moves[dir.index()];
            if next == board {
                continue;
            }
            let value = self.chance(next, depth, 1.0);
            if value > best_value {
                best_value = value;
                best = Some(dir);
            }
        }

        best
    }

    /// Value of `board` averaged over the tile about to appear on it.
    fn chance(&mut self, board: Board, depth: u32, probability: f32) -> f32 {
        if depth == 0 || probability < self.config.floor {
            return self.heuristic.eval(board);
        }
        if let Some(cached) = self.table.get(board.raw(), depth) {
            return cached;
        }

        let empty = board.empty_count();
        let (two, four) = (
            probability * 0.9 / empty as f32,
            probability * 0.1 / empty as f32,
        );

        let mut total = 0.0;
        for offset in board.empty_cells() {
            total += 0.9 * self.best(board.spawn_at(offset, 1), depth - 1, two);
            total += 0.1 * self.best(board.spawn_at(offset, 2), depth - 1, four);
        }
        let value = total / empty as f32;

        self.table.insert(board.raw(), depth, value);
        value
    }

    /// Value of the best move available from `board`.
    ///
    /// A position with no legal move returns zero, which is far below any
    /// evaluation the heuristic produces, so losing is avoided without a
    /// special case anywhere in the search.
    fn best(&mut self, board: Board, depth: u32, probability: f32) -> f32 {
        self.nodes += 1;

        let mut best = 0.0f32;
        for next in board.shift_all() {
            if next == board {
                continue;
            }
            best = best.max(self.chance(next, depth, probability));
        }
        best
    }
}

/// Position evaluation, as a table over the 65536 possible rows.
///
/// Every term is a property of a single line, so a board is scored by eight
/// lookups: its four rows and, after a transpose, its four columns. The
/// weights are the ones from nneonneo's 2048-ai, which are well tuned and not
/// worth rediscovering by hand.
struct Heuristic {
    line: Vec<f32>,
}

impl Heuristic {
    fn new() -> Heuristic {
        let mut line = vec![0.0f32; 65536];
        for (index, value) in line.iter_mut().enumerate() {
            let ranks = [
                (index >> 12) as u32 & 0xF,
                (index >> 8) as u32 & 0xF,
                (index >> 4) as u32 & 0xF,
                index as u32 & 0xF,
            ];
            *value = score_line(ranks);
        }
        Heuristic { line }
    }

    fn eval(&self, board: Board) -> f32 {
        let rows = board.raw();
        let cols = board.transpose().raw();
        let mut total = 0.0;
        for i in 0..4 {
            let shift = 48 - 16 * i;
            total += self.line[(rows >> shift) as u16 as usize];
            total += self.line[(cols >> shift) as u16 as usize];
        }
        total
    }
}

fn score_line(ranks: [u32; 4]) -> f32 {
    /// Paid by a line in every position, so a lost game (which scores zero)
    /// sits far below any reachable evaluation.
    const LOST: f32 = 200_000.0;
    const EMPTY: f32 = 270.0;
    const MERGES: f32 = 700.0;
    const MONOTONICITY: f32 = 47.0;
    const MONOTONICITY_POWER: f32 = 4.0;
    const SUM: f32 = 11.0;
    const SUM_POWER: f32 = 3.5;

    let mut sum = 0.0;
    let mut empty = 0u8;
    let mut merges = 0u8;

    let mut previous = 0;
    let mut run = 0u8;
    for rank in ranks {
        sum += (rank as f32).powf(SUM_POWER);
        if rank == 0 {
            empty += 1;
        } else {
            if previous == rank {
                run += 1;
            } else if run > 0 {
                merges += 1 + run;
                run = 0;
            }
            previous = rank;
        }
    }
    if run > 0 {
        merges += 1 + run;
    }

    // Charge the line for however far it is from sorted, in whichever
    // direction is closer, so a descending line is as welcome as an ascending
    // one and only the zigzags are penalised.
    let (mut left, mut right) = (0.0, 0.0);
    for pair in ranks.windows(2) {
        let (a, b) = (
            (pair[0] as f32).powf(MONOTONICITY_POWER),
            (pair[1] as f32).powf(MONOTONICITY_POWER),
        );
        if pair[0] > pair[1] {
            left += a - b;
        } else {
            right += b - a;
        }
    }

    LOST + EMPTY * f32::from(empty) + MERGES * f32::from(merges)
        - MONOTONICITY * left.min(right)
        - SUM * sum
}
