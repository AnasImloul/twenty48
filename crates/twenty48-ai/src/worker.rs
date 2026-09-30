//! The recursion, and the unit of work a thread claims.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use twenty48::Board;

use crate::Config;
use crate::heuristic::Heuristic;
use crate::table::Table;

/// One subtree two plies below the root: a tile the game could place in reply
/// to a root move, and one of the answers to it.
///
/// The obvious unit of work is a ply shallower, one job per tile the game
/// could place, but that is too coarse to fill a desktop machine. The largest
/// of those jobs measured around 12% of a move's total work, more than an
/// even share of twelve cores, so eleven threads finished and waited on it and
/// no scheduling of that list could have done better. Splitting the reply as
/// well triples the count and cuts the largest job to roughly a third.
pub struct Job {
    /// Which chance-layer entry this is one answer to.
    pub group: usize,
    board: Board,
    depth: u32,
    probability: f32,
    value: AtomicU32,
}

impl Job {
    pub fn new(group: usize, board: Board, depth: u32, probability: f32) -> Job {
        Job {
            group,
            board,
            depth,
            probability,
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
        let value = self.chance(job.board, job.depth, job.probability);
        job.value.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Value of `board` averaged over the tile about to appear on it.
    ///
    /// Inlined on purpose. Folded into [`Worker::best`] the two halves of the
    /// recursion are one function, and left to itself the compiler stops
    /// folding them as soon as [`Worker::run`] gives this a second caller,
    /// which costs a real call at every level of the tree and measured a fifth
    /// of the node rate.
    #[inline(always)]
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
