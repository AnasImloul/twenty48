//! Statistics over many random playouts.
//!
//! Run with `cargo run --release --example random_agent`, optionally with a
//! game count: `-- 100000`.
//!
//! Doubles as a smoke test. Random play is a known quantity: it finishes on a
//! 64 or a 128 most of the time, reaches 256 a few percent of the time, 512
//! rarely and 1024 never, so a distribution far from that means the engine is
//! wrong rather than lucky. Both the tiles and the moves are seeded from the
//! game index, so a run reproduces exactly.

use std::collections::BTreeMap;
use std::time::Instant;

use twenty48::{Game, Rng, Status, Xoshiro256pp};

fn main() {
    let games: u64 = match std::env::args().nth(1) {
        Some(arg) => arg.parse().expect("the game count must be a number"),
        None => 10_000,
    };

    let mut best = BTreeMap::<u16, u64>::new();
    let mut scores = Vec::with_capacity(games as usize);
    let mut rounds = 0u64;

    let start = Instant::now();
    for seed in 0..games {
        let mut game = Game::new(seed);
        // A separate stream, so the policy's choices cannot perturb the tiles.
        let mut policy = Xoshiro256pp::seed_from_u64(!seed);

        while game.status() == Status::Playing {
            let legal = game.legal_moves();
            let dir = legal
                .into_iter()
                .nth(policy.next_u64() as usize % legal.len())
                .expect("a playable game has a legal move");
            game.play(dir).expect("the move was reported legal");
        }

        *best.entry(game.board().max_tile()).or_default() += 1;
        scores.push(game.score());
        rounds += u64::from(game.moves());
    }
    let elapsed = start.elapsed();

    scores.sort_unstable();
    let total: u64 = scores.iter().map(|&score| u64::from(score)).sum();

    println!("{games} games, {rounds} rounds in {elapsed:.2?}");
    println!(
        "{:.1} M rounds/s, {:.0} rounds per game",
        rounds as f64 / elapsed.as_secs_f64() / 1e6,
        rounds as f64 / games as f64
    );
    println!(
        "\nscore: mean {:.0}, median {}, best {}",
        total as f64 / games as f64,
        scores[scores.len() / 2],
        scores[scores.len() - 1]
    );

    println!("\nlargest tile reached:");
    for (tile, count) in best.iter().rev() {
        println!("  {tile:>5}  {count:>7}  {:>3}%", count * 100 / games);
    }
}
