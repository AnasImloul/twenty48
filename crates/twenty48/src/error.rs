use core::fmt;

use crate::direction::Direction;

/// Why a call to [`Game::play`] did nothing.
///
/// [`Game::play`]: crate::Game::play
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MoveError {
    /// The move would not change the board. The game is still playable.
    Illegal(Direction),
    /// No move is legal; the game has ended.
    GameOver,
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveError::Illegal(dir) => write!(f, "no tile can move {dir}"),
            MoveError::GameOver => f.write_str("the game is over"),
        }
    }
}

impl core::error::Error for MoveError {}
