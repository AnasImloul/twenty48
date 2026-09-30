//! A fixed set of threads that chew through a job list.
//!
//! The threads outlive a move because a move does not last long enough to pay
//! for them: at the default depth a search is a few milliseconds, and spawning
//! ten threads costs a visible fraction of that. So they park on a condition
//! variable between moves instead.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::Config;
use crate::heuristic::Heuristic;
use crate::table::Table;
use crate::worker::{Job, Worker};

struct Batch {
    /// Bumped once per job list. A thread compares it against the last batch
    /// it served, so a spurious wake-up, or one it has already answered,
    /// costs nothing.
    generation: u64,
    jobs: Arc<Vec<Job>>,
    config: Config,
    shutdown: bool,
}

struct Shared {
    batch: Mutex<Batch>,
    wake: Condvar,
    /// The next job nobody has claimed. Jobs differ by orders of magnitude in
    /// cost, so they are handed out one at a time rather than split up front.
    next: AtomicUsize,
}

/// Threads, and the handshake that feeds them.
pub struct Pool {
    shared: Arc<Shared>,
    done: Receiver<u64>,
    threads: Vec<JoinHandle<()>>,
}

impl Pool {
    pub fn new(heuristic: &Arc<Heuristic>, table: &Arc<Table>, threads: usize) -> Pool {
        let shared = Arc::new(Shared {
            batch: Mutex::new(Batch {
                generation: 0,
                jobs: Arc::new(Vec::new()),
                config: Config::default(),
                shutdown: false,
            }),
            wake: Condvar::new(),
            next: AtomicUsize::new(0),
        });

        let (sender, done) = channel();
        let threads = (0..threads)
            .map(|n| {
                let shared = Arc::clone(&shared);
                let done = sender.clone();
                let mut worker = Worker::new(Arc::clone(heuristic), Arc::clone(table));
                std::thread::Builder::new()
                    .name(format!("expectimax-{n}"))
                    .spawn(move || serve(&shared, &done, &mut worker))
                    .expect("a search thread could not start")
            })
            .collect();

        Pool {
            shared,
            done,
            threads,
        }
    }

    /// Runs every job, and returns how many nodes the pool visited doing it.
    ///
    /// The calling thread works too, rather than blocking on threads it could
    /// be helping.
    pub fn run(&self, jobs: &Arc<Vec<Job>>, config: Config, mine: &mut Worker) -> u64 {
        {
            let mut batch = self.shared.batch.lock().expect("a search thread panicked");
            batch.generation += 1;
            batch.jobs = Arc::clone(jobs);
            batch.config = config;
            self.shared.next.store(0, Ordering::Relaxed);
        }
        self.shared.wake.notify_all();

        consume(&self.shared, jobs, config, mine);

        // One message per thread, so the count is exact and the batch is
        // provably finished before the caller reads any job back.
        (0..self.threads.len())
            .map(|_| self.done.recv().expect("a search thread panicked"))
            .sum()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        if let Ok(mut batch) = self.shared.batch.lock() {
            batch.shutdown = true;
        }
        self.shared.wake.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn serve(shared: &Shared, done: &Sender<u64>, worker: &mut Worker) {
    let mut served = 0;
    loop {
        let (jobs, config) = {
            let mut batch = shared.batch.lock().expect("a search thread panicked");
            while batch.generation == served && !batch.shutdown {
                batch = shared.wake.wait(batch).expect("a search thread panicked");
            }
            if batch.shutdown {
                return;
            }
            served = batch.generation;
            (Arc::clone(&batch.jobs), batch.config)
        };

        let nodes = consume(shared, &jobs, config, worker);
        if done.send(nodes).is_err() {
            return;
        }
    }
}

fn consume(shared: &Shared, jobs: &[Job], config: Config, worker: &mut Worker) -> u64 {
    let before = worker.nodes();
    while let Some(job) = jobs.get(shared.next.fetch_add(1, Ordering::Relaxed)) {
        worker.run(job, config);
    }
    worker.nodes() - before
}
