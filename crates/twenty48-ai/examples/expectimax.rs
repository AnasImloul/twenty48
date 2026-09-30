//! The expectimax agent, run over many games at once.
//!
//! Run with `cargo run --release --example expectimax`. Flags:
//! `--games N`, `--depth N`, `--floor F`, `--threads N`, or a bare number for
//! the game count.
//!
//! By default the parallelism is one game per core, because games are
//! independent and the thing being measured is a score distribution over
//! games. `--threads N` instead runs one game at a time with the search split
//! N ways, which is how a UI playing a single game uses the machine. The two
//! must agree game for game: that is the test that the split search did not
//! change the agent, only how fast it answers.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use twenty48::{Game, Status};
use twenty48_ai::{Config, Search};

fn main() {
    let (games, config, split) = parse_args();
    // Splitting the search and sharding games compete for the same cores, so
    // asking for one turns the other off.
    let threads = if split > 1 {
        1
    } else {
        thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(games)
    };

    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(games));

    println!(
        "expectimax over {games} games on {threads} threads, \
         {split} per search, depth {} floor {}\n",
        config.depth, config.floor
    );
    let start = Instant::now();

    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut search = Search::with_threads(config, split);
                loop {
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= games {
                        break;
                    }
                    let outcome = play(&mut search, seed as u64);
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

fn parse_args() -> (usize, Config, usize) {
    let mut games = 12;
    let mut split = 1;
    let mut config = Config::default();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if !flag.starts_with("--") {
            games = flag.parse().expect("the game count must be a number");
            i += 1;
            continue;
        }
        let value = args
            .get(i + 1)
            .unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag {
            "--games" => games = value.parse().expect("--games must be a number"),
            "--depth" => config.depth = value.parse().expect("--depth must be a number"),
            "--floor" => config.floor = value.parse().expect("--floor must be a number"),
            "--threads" => split = value.parse().expect("--threads must be a number"),
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }

    assert!(config.depth >= 1, "--depth must be at least 1");
    assert!(split >= 1, "--threads must be at least 1");
    (games, config, split)
}

struct Outcome {
    score: u32,
    max_tile: u16,
    moves: u32,
    nodes: u64,
    elapsed: Duration,
}

fn play(search: &mut Search, seed: u64) -> Outcome {
    let mut game = Game::new(seed);
    let before = search.nodes();
    let start = Instant::now();

    while game.status() == Status::Playing {
        let dir = search
            .best_move(game.board())
            .expect("a playable game moves");
        game.play(dir).expect("the search only returns legal moves");
    }

    Outcome {
        score: game.score(),
        max_tile: game.board().max_tile(),
        moves: game.moves(),
        nodes: search.nodes() - before,
        elapsed: start.elapsed(),
    }
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
