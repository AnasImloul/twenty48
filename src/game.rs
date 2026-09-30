//! A playable game: a board, a score and a tile source.

use crate::board::Board;
use crate::direction::{DirSet, Direction};
use crate::error::MoveError;
use crate::rng::{Rng, Xoshiro256pp};

/// Whether a game can still be played.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    /// At least one move is legal.
    Playing,
    /// No move is legal.
    Over,
}

/// The tile the engine added after a move.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Spawn {
    /// Row it landed in.
    pub row: usize,
    /// Column it landed in.
    pub col: usize,
    /// Its value, either 2 or 4.
    pub value: u16,
}

/// What one move did.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Round {
    /// Points the merges scored.
    pub gained: u32,
    /// The tile that appeared afterwards.
    pub spawn: Spawn,
    /// Whether the game is still playable.
    pub status: Status,
}

/// A 2048 game in progress.
///
/// Wraps a [`Board`] with the things a player-facing frontend needs: a running
/// score, a move counter and a seeded tile source. A search agent should drive
/// [`Board`] directly instead, so that it can enumerate spawns rather than
/// sample them.
#[derive(Clone, Debug)]
pub struct Game<R: Rng = Xoshiro256pp> {
    board: Board,
    score: u32,
    moves: u32,
    status: Status,
    rng: R,
}

impl Game<Xoshiro256pp> {
    /// Starts a game from a seed.
    ///
    /// The seed fixes the entire sequence of spawned tiles, so replaying the
    /// same moves against the same seed reproduces the game exactly.
    pub fn new(seed: u64) -> Self {
        Self::with_rng(Xoshiro256pp::seed_from_u64(seed))
    }
}

impl<R: Rng> Game<R> {
    /// Starts a game from a tile source.
    pub fn with_rng(mut rng: R) -> Self {
        let board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        Self {
            board,
            score: 0,
            moves: 0,
            status: Status::Playing,
            rng,
        }
    }

    /// Resumes from an existing position.
    ///
    /// Useful for loading a saved game or setting up a puzzle. The status is
    /// derived from `board`, and the move counter restarts at zero.
    pub fn resume(board: Board, score: u32, rng: R) -> Self {
        Self {
            board,
            score,
            moves: 0,
            status: if board.is_game_over() {
                Status::Over
            } else {
                Status::Playing
            },
            rng,
        }
    }

    /// The current position.
    #[inline]
    pub fn board(&self) -> Board {
        self.board
    }

    /// Points scored so far.
    #[inline]
    pub fn score(&self) -> u32 {
        self.score
    }

    /// Number of moves played.
    #[inline]
    pub fn moves(&self) -> u32 {
        self.moves
    }

    /// Whether the game can still be played.
    #[inline]
    pub fn status(&self) -> Status {
        self.status
    }

    /// The moves that would change the board.
    #[inline]
    pub fn legal_moves(&self) -> DirSet {
        self.board.legal_moves()
    }

    /// Slides the tiles, then adds a new one.
    ///
    /// A rejected move leaves the game completely untouched: the board, the
    /// score, the move counter and the tile source are all unchanged, so a
    /// frontend can offer the move again without perturbing the sequence.
    ///
    /// # Errors
    ///
    /// [`MoveError::Illegal`] if `dir` does not change the board, and
    /// [`MoveError::GameOver`] if no direction would.
    pub fn play(&mut self, dir: Direction) -> Result<Round, MoveError> {
        if self.status == Status::Over {
            return Err(MoveError::GameOver);
        }

        let (slid, gained) = self.board.shift_scored(dir);
        if slid == self.board {
            return Err(MoveError::Illegal(dir));
        }

        // A legal move always frees a cell, either by merging or by vacating
        // the one a tile slid out of, so `draw_spawn` cannot come back empty.
        let (offset, exponent) = slid
            .draw_spawn(&mut self.rng)
            .expect("a legal move leaves an empty cell");
        let (row, col) = Board::coords(offset);

        self.board = slid.spawn_at(offset, exponent);
        self.score += gained;
        self.moves += 1;
        self.status = if self.board.is_game_over() {
            Status::Over
        } else {
            Status::Playing
        };

        Ok(Round {
            gained,
            spawn: Spawn {
                row,
                col,
                value: 1 << exponent,
            },
            status: self.status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_game_has_two_tiles() {
        for seed in 0..64 {
            let game = Game::new(seed);
            assert_eq!(game.board().empty_count(), 14);
            assert_eq!(game.score(), 0);
            assert_eq!(game.moves(), 0);
            assert_eq!(game.status(), Status::Playing);
        }
    }

    #[test]
    fn the_same_seed_replays_identically() {
        let script = [
            Direction::Left,
            Direction::Down,
            Direction::Right,
            Direction::Up,
        ];
        let replay = |seed: u64| {
            let mut game = Game::new(seed);
            let mut log = Vec::new();
            for dir in script.into_iter().cycle().take(400) {
                match game.play(dir) {
                    Ok(round) => log.push((game.board(), game.score(), round)),
                    Err(MoveError::GameOver) => break,
                    Err(MoveError::Illegal(_)) => {}
                }
            }
            log
        };
        for seed in [0, 1, 0x00C0_FFEE, u64::MAX] {
            assert_eq!(replay(seed), replay(seed));
        }
        assert_ne!(replay(1), replay(2));
    }

    #[test]
    fn an_illegal_move_changes_nothing() {
        let mut game = Game::resume(
            Board::from_tiles([[2, 4, 8, 16], [0; 4], [0; 4], [0; 4]]).unwrap(),
            100,
            Xoshiro256pp::seed_from_u64(9),
        );
        let before = game.clone();

        assert_eq!(
            game.play(Direction::Left),
            Err(MoveError::Illegal(Direction::Left))
        );
        assert_eq!(
            game.play(Direction::Up),
            Err(MoveError::Illegal(Direction::Up))
        );

        assert_eq!(game.board(), before.board());
        assert_eq!(game.score(), before.score());
        assert_eq!(game.moves(), before.moves());
        // The tile source must not have advanced either.
        assert_eq!(game.rng, before.rng);
    }

    #[test]
    fn score_accumulates_the_reported_gains() {
        let mut game = Game::new(0xBEEF);
        let mut total = 0;
        for dir in Direction::ALL.into_iter().cycle().take(2000) {
            match game.play(dir) {
                Ok(round) => {
                    total += round.gained;
                    assert_eq!(game.score(), total);
                }
                Err(MoveError::GameOver) => break,
                Err(MoveError::Illegal(_)) => {}
            }
        }
        assert!(total > 0);
    }

    #[test]
    fn the_reported_spawn_is_where_the_tile_appeared() {
        let mut game = Game::new(4);
        let mut previous = game.board();
        for dir in Direction::ALL.into_iter().cycle().take(2000) {
            if let Ok(round) = game.play(dir) {
                let board = game.board();
                assert_eq!(
                    board.get(round.spawn.row, round.spawn.col),
                    round.spawn.value
                );
                assert!(round.spawn.value == 2 || round.spawn.value == 4);
                assert!(previous.empty_count() > board.empty_count() || round.gained > 0);
                previous = board;
            }
        }
    }

    #[test]
    fn a_finished_game_rejects_every_move() {
        let stuck =
            Board::from_tiles([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]]).unwrap();
        let mut game = Game::resume(stuck, 0, Xoshiro256pp::seed_from_u64(0));
        assert_eq!(game.status(), Status::Over);
        for dir in Direction::ALL {
            assert_eq!(game.play(dir), Err(MoveError::GameOver));
        }
    }

    #[test]
    fn games_end_only_when_no_move_is_legal() {
        for seed in 0..32 {
            let mut game = Game::new(seed);
            let mut guard = 0;
            loop {
                let legal = game.legal_moves();
                if legal.is_empty() {
                    break;
                }
                let dir = legal.first().unwrap();
                game.play(dir).unwrap();
                guard += 1;
                assert!(guard < 100_000, "playout did not terminate");
            }
            assert_eq!(game.status(), Status::Over);
            assert!(game.board().is_game_over());
            assert_eq!(game.board().empty_count(), 0);
        }
    }
}
