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
    /// The split is the root chance layer rather than the four root moves:
    /// every (move, spawned tile) pair below the root is an independent
    /// subtree, so a mid-game position with six empty cells gives around
    /// fifty pieces of work, which is enough to keep a desktop machine busy.
    /// Splitting on moves alone would give four, one of which is usually much
    /// the largest.
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
        let mut jobs = Vec::new();

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
                    Job::new(
                        dir.index(),
                        next.spawn_at(offset, exponent),
                        depth - 1,
                        probability,
                        weight,
                    )
                };
                jobs.push(at(1, two, 0.9));
                jobs.push(at(2, four, 0.1));
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

        // Summed in job order and divided per direction at the end, which is
        // the order a single thread walking the tree would have used.
        let mut totals = [0.0f32; 4];
        for job in jobs.iter() {
            totals[job.dir] += job.weight * job.value();
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
