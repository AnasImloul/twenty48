//! 2048 in the terminal. Run with `cargo run --release --example play`.
//!
//! Reads a line at a time rather than raw keys, so it needs no terminal
//! handling: type `w`, `a`, `s` or `d` and press enter. Several moves on one
//! line work too, which makes `wasd` a convenient opening.
//!
//! Pass a seed to replay a game: `cargo run --example play -- 12345`.

use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use twenty48::{Direction, Game, MoveError, Status};

fn main() {
    let seed = match std::env::args().nth(1) {
        Some(arg) => arg.parse().expect("the seed must be a u64"),
        // The engine is deterministic on purpose, so the clock is the frontend's
        // problem rather than the library's.
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_nanos() as u64,
    };

    let mut game = Game::new(seed);
    println!("seed {seed}, wasd to move, q to quit\n");
    draw(&game, None);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin closed mid-line");

        for key in line.chars() {
            let dir = match key {
                'w' => Direction::Up,
                'a' => Direction::Left,
                's' => Direction::Down,
                'd' => Direction::Right,
                'q' => return,
                _ => continue,
            };

            match game.play(dir) {
                Ok(_) => draw(&game, None),
                Err(err) => draw(&game, Some(&err)),
            }

            if game.status() == Status::Over {
                println!("game over: {} in {} moves", game.score(), game.moves());
                return;
            }
        }
    }
}

fn draw(game: &Game, note: Option<&MoveError>) {
    println!("{}", game.board());
    print!("score {}  moves {}", game.score(), game.moves());
    match note {
        Some(err) => println!("  ({err})"),
        None => println!(),
    }
    println!();
    io::stdout().flush().expect("stdout closed");
}
