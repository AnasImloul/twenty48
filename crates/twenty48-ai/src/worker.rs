//! The recursion, and the unit of work a thread claims.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use twenty48::Board;

use crate::Config;
use crate::heuristic::Heuristic;
use crate::table::Table;

/// One subtree hanging off the root chance layer: the position after a root
/// move and one of the tiles the game could place in reply.
pub struct Job {
    /// Which root move this contributes to, as a [`twenty48::Direction`]
    /// index.
    pub dir: usize,
    board: Board,
    depth: u32,
    probability: f32,
    /// 0.9 for a spawned 2, 0.1 for a 4. Kept apart from `probability`
    /// instead of folded into it because the reduction has to add
    /// `weight * value` the way the serial search would, and
    /// `Σ 0.9·x / n` is not `Σ (0.9/n)·x` in `f32`.
    pub weight: f32,
    value: AtomicU32,
}

impl Job {
    pub fn new(dir: usize, board: Board, depth: u32, probability: f32, weight: f32) -> Job {
        Job {
            dir,
            board,
            depth,
            probability,
            weight,
            value: AtomicU32::new(0),
        }
    }

    /// What the job came to. Only meaningful once the batch it belongs to has
    /// been waited on.
    pub fn value(&self) -> f32 {
        f32::from_bits(self.value.load(Ordering::Relaxed))
    }
}

/// One thread's share of a search: a node count, and a handle on the tables
/// every thread reads.
pub struct Worker {
    heuristic: Arc<Heuristic>,
    table: Arc<Table>,
    config: Config,
    nodes: u64,
}

impl Worker {
    pub fn new(heuristic: Arc<Heuristic>, table: Arc<Table>) -> Worker {
        Worker {
            heuristic,
            table,
            config: Config::default(),
            nodes: 0,
        }
    }

    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Evaluates `job` and stores the answer in it.
    pub fn run(&mut self, job: &Job, config: Config) {
        self.config = config;
        let value = self.best(job.board, job.depth, job.probability);
        job.value.store(value.to_bits(), Ordering::Relaxed);
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
