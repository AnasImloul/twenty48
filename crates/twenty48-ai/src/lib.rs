//! An expectimax agent for [`twenty48`].
//!
//! The search is the ordinary one for this game: maximise over the four moves,
//! average over the tile the game then places, weighted 9:1 between a 2 and a
//! 4. What makes it play well rather than merely legally is the position
//! evaluation, which is a table over the 65536 possible lines.
//!
//! ```
//! use twenty48::{Game, Status};
//! use twenty48_ai::{Config, Search};
//!
//! let mut search = Search::new(Config { depth: 2, ..Config::default() });
//! let mut game = Game::new(0xC0FFEE);
//!
//! while game.status() == Status::Playing && game.moves() < 20 {
//!     let dir = search.best_move(game.board()).expect("a playable game moves");
//!     game.play(dir).expect("the search only returns legal moves");
//! }
//! ```
//!
//! A [`Search`] is a worker, not a value: it carries a transposition table and
//! the evaluation table, several MiB in total, so build one per thread and
//! reuse it rather than making one per move. [`Search::with_threads`] instead
//! spreads a single move over several threads, which is what a UI playing one
//! game wants.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod heuristic;
mod pool;
mod table;
mod worker;

use std::sync::Arc;

use twenty48::{Board, Direction};

use heuristic::Heuristic;
use pool::Pool;
use table::Table;
use worker::{Job, Worker};

/// What bounds the search.
///
/// The two fields bind at opposite ends of the game, which is why both exist.
/// `depth` bounds the late game, where the board is crowded and a chance node
/// has only a few children to branch over. `floor` bounds the early game,
/// where thirty children per chance node means depth alone is no bound at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Moves to look ahead. Each level multiplies the work by roughly the
    /// number of empty cells, so this is the expensive knob.
    pub depth: u32,
    /// Probability below which a branch is evaluated instead of expanded.
    pub floor: f32,
}

impl Default for Config {
    /// Depth 5 at a floor of `1e-3`, which is about 3ms per move and reaches
    /// 32768.
    ///
    /// Depth 4 is under 1ms but scores roughly half as much, and dropping the
    /// floor to `1e-4` costs four times as long per move for no measurable
    /// gain. See the table in the README.
    fn default() -> Config {
        Config {
            depth: 5,
            floor: 1e-3,
        }
    }
}

/// An expectimax searcher, with its tables.
pub struct Search {
    config: Config,
    table: Arc<Table>,
    worker: Worker,
    pool: Option<Pool>,
    delegated: u64,
}

impl Search {
    /// Builds a single-threaded searcher. This allocates and fills several
    /// MiB of tables, so it is not something to do per move.
    #[must_use]
    pub fn new(config: Config) -> Search {
        Search::with_threads(config, 1)
    }

    /// Builds a searcher that spreads each move over `threads` threads.
    ///
    /// The split is two plies down rather than on the four root moves, so a
    /// piece of work is a root move, a tile the game could place in reply, and
    /// one answer to that tile. A mid-game position gives a hundred or so of
    /// them. Splitting on the root moves alone would give four, one of which
    /// is usually much the largest; stopping a ply short of this still left
    /// the largest piece bigger than a twelfth of the move on a twelve-core
    /// machine, which held utilisation to around 60%.
    ///
    /// The threads share one transposition table, so they help each other
    /// rather than each rediscovering the same positions. The cost of that is
    /// reproducibility: which thread reaches a position first decides what
    /// later lookups of it return, so a split search can pick a different
    /// move from run to run. A seeded game only replays exactly at
    /// `threads == 1`, which is what [`Search::new`] gives and what the
    /// evaluation runs use.
    #[must_use]
    pub fn with_threads(config: Config, threads: usize) -> Search {
        let heuristic = Arc::new(Heuristic::new());
        let table = Arc::new(Table::new(threads));
        let pool = (threads > 1).then(|| Pool::new(&heuristic, &table, threads - 1));

        Search {
            config,
            worker: Worker::new(heuristic, Arc::clone(&table)),
            table,
            pool,
            delegated: 0,
        }
    }

    /// The bounds currently in force.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// Changes the bounds. Cheap, and safe to do between any two moves: no
    /// state outlives a single [`Search::best_move`] call.
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Positions visited since this searcher was built.
    #[must_use]
    pub fn nodes(&self) -> u64 {
        self.worker.nodes() + self.delegated
    }

    /// The move the search likes best, or `None` if the position is over.
    pub fn best_move(&mut self, board: Board) -> Option<Direction> {
        let depth = self.config.depth.max(1);
        let moves = board.shift_all();
        self.table.clear();

        // A legal move always vacates a cell, either by merging or by moving
        // a tile out of one, so a zero here means the move was illegal.
        let mut empties = [0u32; 4];
        let mut chances = Vec::new();

        for dir in Direction::ALL {
            let next = moves[dir.index()];
            if next == board {
                continue;
            }
            let empty = next.empty_count();
            empties[dir.index()] = empty;

            let (two, four) = (0.9 / empty as f32, 0.1 / empty as f32);
            for offset in next.empty_cells() {
                let at = |exponent, probability, weight| {
                    let board = next.spawn_at(offset, exponent);
                    Chance::new(dir.index(), board, probability, weight)
                };
                chances.push(at(1, two, 0.9));
                chances.push(at(2, four, 0.1));
            }
        }

        // Built in the order a single thread would have walked them, which is
        // the order the serial path below runs them in. Reordering the list to
        // put the large jobs first balances the batch better but costs more
        // than it gains: neighbouring jobs share most of their subtree, so
        // visiting them apart empties the transposition table between them and
        // measured around a fifth off the node rate.
        let mut jobs = Vec::with_capacity(chances.len() * 3);
        for (group, chance) in chances.iter().enumerate() {
            for next in chance.board.shift_all() {
                if next == chance.board {
                    continue;
                }
                jobs.push(Job::new(group, next, depth - 1, chance.probability));
            }
        }

        let jobs = Arc::new(jobs);
        match &self.pool {
            Some(pool) => self.delegated += pool.run(&jobs, self.config, &mut self.worker),
            None => {
                for job in jobs.iter() {
                    self.worker.run(job, self.config);
                }
            }
        }
        // The chance layer is a level of the tree that no job walked, so it
        // goes on the node count here instead.
        self.delegated += chances.len() as u64;

        // Two passes, because the two reductions are not alike. A maximum is
        // exact in whatever order it is taken, so the jobs fold in the order
        // they were dispatched. A sum is not, so the weighted total is
        // accumulated over `chances`, which is the order a single thread
        // walking the tree would have used.
        for job in jobs.iter() {
            let chance = &mut chances[job.group];
            chance.value = chance.value.max(job.value());
        }

        let mut totals = [0.0f32; 4];
        for chance in &chances {
            totals[chance.dir] += chance.weight * chance.value;
        }

        let mut best = None;
        let mut best_value = f32::NEG_INFINITY;
        for dir in Direction::ALL {
            let empty = empties[dir.index()];
            if empty == 0 {
                continue;
            }
            let value = totals[dir.index()] / empty as f32;
            if value > best_value {
                best_value = value;
                best = Some(dir);
            }
        }

        best
    }
}

/// One tile the game could place in reply to a root move, and the value of
/// the best answer to it.
///
/// A position with no legal answer keeps its zero, which is far below any
/// evaluation the heuristic produces, so losing is avoided without a special
/// case anywhere in the search.
struct Chance {
    /// Which root move this contributes to, as a [`Direction`] index.
    dir: usize,
    board: Board,
    probability: f32,
    /// 0.9 for a spawned 2, 0.1 for a 4. Kept apart from `probability`
    /// instead of folded into it because the reduction has to add
    /// `weight * value` the way the serial search would, and
    /// `Σ 0.9·x / n` is not `Σ (0.9/n)·x` in `f32`.
    weight: f32,
    value: f32,
}

impl Chance {
    fn new(dir: usize, board: Board, probability: f32, weight: f32) -> Chance {
        Chance {
            dir,
            board,
            probability,
            weight,
            value: 0.0,
        }
    }
}
