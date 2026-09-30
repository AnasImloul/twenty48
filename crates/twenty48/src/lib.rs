//! A 2048 engine.
//!
//! The board is sixteen 4-bit tile exponents packed into one `u64`, so
//! [`Board`] is `Copy`, lives in a register and never allocates. A row is
//! therefore a `u16`, of which there are only 65536, so sliding one is a table
//! lookup rather than a loop: a full move costs four loads and a few shifts.
//!
//! The engine is split in two. [`Board`] is pure — it applies moves and knows
//! nothing about randomness or score history, which is what a search agent
//! wants. [`Game`] wraps it with a running score and a seeded tile source,
//! which is what a CLI or GUI wants.
//!
//! # Playing a game
//!
//! ```
//! use twenty48::{Direction, Game, MoveError};
//!
//! let mut game = Game::new(0xC0FFEE);
//! match game.play(Direction::Left) {
//!     Ok(round) => println!("scored {}, tile at {:?}", round.gained, round.spawn),
//!     Err(MoveError::Illegal(dir)) => println!("nothing moves {dir}"),
//!     Err(MoveError::GameOver) => println!("final score {}", game.score()),
//! }
//! ```
//!
//! # Searching
//!
//! The deterministic slide and the random spawn are separate operations, so a
//! search can enumerate every tile the game might place instead of sampling
//! one:
//!
//! ```
//! use twenty48::{Board, Direction};
//!
//! let board = Board::from_tiles([
//!     [2, 2, 0, 0],
//!     [0, 0, 0, 0],
//!     [0, 0, 0, 0],
//!     [0, 0, 0, 0],
//! ])
//! .unwrap();
//!
//! let (next, gained) = board.shift_scored(Direction::Left);
//! assert_eq!(gained, 4);
//! assert_eq!(next.get(0, 0), 4);
//!
//! let mut children = 0;
//! for offset in next.empty_cells() {
//!     for exponent in [1, 2] {
//!         let _child = next.spawn_at(offset, exponent);
//!         children += 1;
//!     }
//! }
//! assert_eq!(children, 2 * next.empty_count());
//! ```
//!
//! Use [`Board::shift_all`] rather than four calls to [`Board::shift`] when
//! expanding a node; it shares the transpose between the vertical moves.
//!
//! # Board layout
//!
//! Cell `(row, col)` sits at bits `60 - 16*row - 4*col`, most significant
//! nibble first, so a hex literal reads in the same order as the grid:
//!
//! ```
//! use twenty48::Board;
//!
//! let board = Board::from_raw(0x0000_0000_0011_2300);
//! assert_eq!(board.get(2, 2), 2);
//! assert_eq!(board.get(3, 1), 8);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::inline_always,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

mod board;
mod direction;
mod error;
mod game;
mod rng;
mod tables;

pub use board::{Board, EmptyCells, SIZE};
pub use direction::{DirSet, DirSetIter, Direction};
pub use error::MoveError;
pub use game::{Game, Round, Spawn, Status};
pub use rng::{Rng, Xoshiro256pp};
