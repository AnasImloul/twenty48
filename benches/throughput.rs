//! Throughput measurements. Run with `cargo bench`.
//!
//! Three rates, reported separately because they differ by an order of
//! magnitude and "rounds per second" is ambiguous between them:
//!
//! - **shifts/s** — one slide, the raw table-and-bitop rate.
//! - **rounds/s** — slide, legality, spawn (including the draw) and game over.
//! - **nodes/s**  — positions visited by an expectimax, what a search consumes.
//!
//! Every board fed to a bench comes from a real game. Uniform-random `u64`s
//! spread their row indices over all 65536 table entries, a distribution that
//! never occurs in play, and would make the tables look several times more
//! cache-hostile than they are. The second bench measures exactly that gap.

use std::hint::black_box;
use std::time::{Duration, Instant};

use twenty48::{Board, Direction, Rng, Xoshiro256pp};

fn main() {
    let played = corpus(512);
    let random = random_boards(played.len());

    println!("twenty48 throughput\n");

    report(
        "shift, played boards",
        "shifts",
        measure(|passes| shifts(&played, passes)),
    );
    report(
        "shift, random boards",
        "shifts",
        measure(|passes| shifts(&random, passes)),
    );
    report(
        "spawn, draw and place",
        "spawns",
        measure(|passes| spawns(&played, passes)),
    );
    report(
        "round, random policy",
        "rounds",
        measure(rounds_random_policy),
    );
    report(
        "round, down-first policy",
        "rounds",
        measure(rounds_fixed_policy),
    );
    report(
        "node, expectimax depth 3",
        "nodes",
        measure(|roots| expectimax(&played, roots)),
    );

    footprint();
}

/// Median rate over seven runs, each sized to take at least 50ms.
///
/// `body` is called with a pass count and returns the operations it performed,
/// so a bench whose size is not known in advance (a playout runs until the
/// game ends) still reports an exact denominator.
fn measure(mut body: impl FnMut(usize) -> u64) -> f64 {
    const MIN_RUN: Duration = Duration::from_millis(50);
    const REPS: usize = 7;

    let mut passes = 1;
    let mut rates = Vec::with_capacity(REPS);

    loop {
        let start = Instant::now();
        let ops = body(passes);
        let elapsed = start.elapsed();

        if elapsed < MIN_RUN {
            passes *= 2;
            continue;
        }

        rates.push(ops as f64 / elapsed.as_secs_f64());
        while rates.len() < REPS {
            let start = Instant::now();
            let ops = body(passes);
            rates.push(ops as f64 / start.elapsed().as_secs_f64());
        }

        rates.sort_by(f64::total_cmp);
        return rates[REPS / 2];
    }
}

fn report(name: &str, unit: &str, per_second: f64) {
    let (scaled, prefix) = if per_second >= 1e9 {
        (per_second / 1e9, 'G')
    } else {
        (per_second / 1e6, 'M')
    };
    println!("  {name:<26} {scaled:>7.1} {prefix} {unit}/s");
}

/// Boards from real games, stratified by how full they are.
///
/// Early, middle and late boards have quite different row distributions, and a
/// bench that saw only one of them would be measuring one phase of one game.
/// They are interleaved rather than concatenated so the loop does not run
/// through three long stretches of near-identical positions.
fn corpus(per_phase: usize) -> Vec<Board> {
    let mut rng = Xoshiro256pp::seed_from_u64(0xB0A2D);
    let mut phases: [Vec<Board>; 3] = Default::default();

    while phases.iter().any(|phase| phase.len() < per_phase) {
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        loop {
            let phase = match board.empty_count() {
                10.. => 0,
                4..10 => 1,
                _ => 2,
            };
            if phases[phase].len() < per_phase {
                phases[phase].push(board);
            }

            let Some(dir) = pick(board, &mut rng) else {
                break;
            };
            board = board.shift(dir).spawn(&mut rng);
        }
    }

    let mut out = Vec::with_capacity(3 * per_phase);
    for i in 0..per_phase {
        out.extend(phases.iter().map(|phase| phase[i]));
    }
    out
}

/// Boards built straight from the generator, for the comparison bench only.
fn random_boards(count: usize) -> Vec<Board> {
    let mut rng = Xoshiro256pp::seed_from_u64(0xDECAF);
    (0..count)
        .map(|_| Board::from_raw(rng.next_u64()))
        .collect()
}

fn pick<R: Rng>(board: Board, rng: &mut R) -> Option<Direction> {
    let legal = board.legal_moves();
    let n = legal.len();
    if n == 0 {
        return None;
    }
    legal.into_iter().nth(rng.next_u64() as usize % n)
}

fn shifts(boards: &[Board], passes: usize) -> u64 {
    let mut acc = 0u64;
    for _ in 0..passes {
        for &board in boards {
            // The board has to leave the array opaquely, or the whole chain
            // folds into a constant at compile time.
            let board = black_box(board);
            for next in board.shift_all() {
                acc ^= next.raw();
            }
        }
    }
    black_box(acc);
    (passes * boards.len() * 4) as u64
}

fn spawns(boards: &[Board], passes: usize) -> u64 {
    let mut rng = Xoshiro256pp::seed_from_u64(1);
    let mut acc = 0u64;
    let mut count = 0u64;

    for _ in 0..passes {
        for &board in boards {
            let board = black_box(board);
            if let Some((offset, exponent)) = board.draw_spawn(&mut rng) {
                acc ^= board.spawn_at(offset, exponent).raw();
                count += 1;
            }
        }
    }

    black_box(acc);
    count
}

fn rounds_random_policy(games: usize) -> u64 {
    let mut rng = Xoshiro256pp::seed_from_u64(0xA11CE);
    let mut acc = 0u64;
    let mut rounds = 0u64;

    for _ in 0..games {
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        while !board.is_game_over() {
            let dir = pick(board, &mut rng).expect("not over means a move is legal");
            let (slid, gained) = board.shift_scored(dir);
            let (offset, exponent) = slid
                .draw_spawn(&mut rng)
                .expect("a legal move frees a cell");
            board = slid.spawn_at(offset, exponent);
            acc ^= u64::from(gained);
            rounds += 1;
        }
    }

    black_box(acc);
    rounds
}

/// The policy agents fall back on, and a much denser board than random play
/// reaches, so the row distribution differs from the bench above.
fn rounds_fixed_policy(games: usize) -> u64 {
    const ORDER: [Direction; 4] = [
        Direction::Down,
        Direction::Left,
        Direction::Right,
        Direction::Up,
    ];

    let mut rng = Xoshiro256pp::seed_from_u64(0xB0B);
    let mut acc = 0u64;
    let mut rounds = 0u64;

    for _ in 0..games {
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        'game: loop {
            for dir in ORDER {
                let (slid, gained) = board.shift_scored(dir);
                if slid == board {
                    continue;
                }
                let (offset, exponent) = slid
                    .draw_spawn(&mut rng)
                    .expect("a legal move frees a cell");
                board = slid.spawn_at(offset, exponent);
                acc ^= u64::from(gained);
                rounds += 1;
                continue 'game;
            }
            break;
        }
    }

    black_box(acc);
    rounds
}

/// Depth-limited expectimax, counting chance nodes.
///
/// The only bench that exercises the whole search path at once: `shift_all` at
/// max nodes, `empty_cells` and `spawn_at` at chance nodes, and `Board` being
/// `Copy` standing in for an undo log. The position evaluation is deliberately
/// one instruction, because what is being measured is the engine underneath a
/// search and not the quality of the search.
struct Expectimax {
    nodes: u64,
}

impl Expectimax {
    /// Value of the best move available from `board`.
    fn best(&mut self, board: Board, plies: u32) -> f64 {
        let mut best = 0.0f64;
        for next in board.shift_all() {
            if next == board {
                continue;
            }
            best = best.max(self.expected(next, plies));
        }
        best
    }

    /// Value of `board` averaged over the tile about to appear on it.
    fn expected(&mut self, board: Board, plies: u32) -> f64 {
        self.nodes += 1;
        let empty = board.empty_count();
        if plies == 0 || empty == 0 {
            return f64::from(empty);
        }

        let mut total = 0.0;
        for offset in board.empty_cells() {
            total += 0.9 * self.best(board.spawn_at(offset, 1), plies - 1);
            total += 0.1 * self.best(board.spawn_at(offset, 2), plies - 1);
        }
        total / f64::from(empty)
    }
}

fn expectimax(boards: &[Board], roots: usize) -> u64 {
    const PLIES: u32 = 3;

    let mut search = Expectimax { nodes: 0 };
    let mut acc = 0.0;

    for i in 0..roots {
        let board = black_box(boards[i % boards.len()]);
        acc += search.best(board, PLIES);
    }

    black_box(acc);
    search.nodes
}

/// How much of the row tables play actually touches.
///
/// Each table has 65536 entries, which is the usual reason to be suspicious of
/// this design. But reachable rows are dominated by small exponents and
/// mostly-monotone runs, so the resident set is a few KiB and the table size
/// is a non-issue. This counts it rather than asserting it.
fn footprint() {
    /// `u16` entries per 64-byte cache line.
    const LINE: usize = 32;
    const LINES: usize = 65536 / LINE;

    let mut rng = Xoshiro256pp::seed_from_u64(0xF007);
    let mut seen = vec![0u64; 65536 / 64];
    let mut lookups = 0u64;

    for _ in 0..256 {
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        loop {
            let raw = board.raw();
            for i in 0..4 {
                for index in [row_index(raw, i), col_index(raw, i)] {
                    let index = index as usize;
                    seen[index / 64] |= 1 << (index % 64);
                    lookups += 1;
                }
            }

            let Some(dir) = pick(board, &mut rng) else {
                break;
            };
            board = board.shift(dir).spawn(&mut rng);
        }
    }

    let rows: u32 = seen.iter().map(|word| word.count_ones()).sum();
    // A bitset word covers 64 entries, which is two cache lines of the table.
    let lines: usize = seen
        .iter()
        .map(|&word| usize::from(word as u32 != 0) + usize::from(word >> 32 != 0))
        .sum();

    println!(
        "\n  hot footprint, over {:.1} M lookups:",
        lookups as f64 / 1e6
    );
    println!("    {rows} of 65536 rows reached");
    println!(
        "    spanning {lines} of {LINES} cache lines, {} KiB of each table",
        lines * 64 / 1024
    );
}

fn row_index(raw: u64, row: usize) -> u16 {
    (raw >> (48 - 16 * row)) as u16
}

fn col_index(raw: u64, col: usize) -> u16 {
    let mut index = 0u16;
    for row in 0..4 {
        let nibble = (raw >> (60 - 16 * row - 4 * col)) & 0xF;
        index |= (nibble as u16) << (12 - 4 * row);
    }
    index
}
