//! The public surface, used the way a frontend would use it.
//!
//! The differential suite covers whether the rules are right. These cover
//! whether the crate is usable from outside: what is reachable, what round
//! trips, and what a `Game` guarantees to a caller across a whole session.

use twenty48::{Board, DirSet, Direction, Game, MoveError, Rng, SIZE, Status, Xoshiro256pp};

#[test]
fn tiles_round_trip_through_from_tiles() {
    let grid = [
        [2, 4, 8, 16],
        [32, 64, 128, 256],
        [512, 1024, 2048, 4096],
        [8192, 16384, 32768, 0],
    ];
    let board = Board::from_tiles(grid).expect("every entry is a legal tile");
    assert_eq!(board.tiles(), grid);
    assert_eq!(board.max_tile(), 32768);
    assert_eq!(board.empty_count(), 1);
    assert_eq!(board.get(3, 3), 0);
    assert_eq!(board.get(0, 1), 4);
}

#[test]
fn from_tiles_rejects_values_that_are_not_tiles() {
    for bad in [1, 3, 6, 100, 65535] {
        let mut grid = [[0u16; SIZE]; SIZE];
        grid[1][2] = bad;
        assert!(Board::from_tiles(grid).is_none(), "accepted {bad}");
    }
}

#[test]
fn the_raw_layout_is_stable() {
    // Documented in the crate docs and relied on by hand-written test vectors.
    let board = Board::from_raw(0x0000_0000_0011_2300);
    assert_eq!(board.get(2, 2), 2);
    assert_eq!(board.get(2, 3), 2);
    assert_eq!(board.get(3, 0), 4);
    assert_eq!(Board::from_tiles(board.tiles()).unwrap(), board);
    assert_eq!(Board::from_raw(board.raw()), board);
}

#[test]
fn empty_cells_enumerates_offsets_that_spawn_at_accepts() {
    let board = Board::from_tiles([[2, 0, 4, 0], [0, 8, 0, 0], [0; 4], [16, 0, 0, 32]]).unwrap();
    let offsets: Vec<u32> = board.empty_cells().collect();

    assert_eq!(offsets.len() as u32, board.empty_count());
    for offset in offsets {
        let (row, col) = Board::coords(offset);
        assert_eq!(board.get(row, col), 0);
        assert_eq!(board.spawn_at(offset, 1).get(row, col), 2);
        assert_eq!(board.spawn_at(offset, 2).get(row, col), 4);
    }
}

#[test]
fn an_illegal_shift_returns_the_same_board() {
    // A full top row cannot go left, right or up; only down is legal.
    let board = Board::from_tiles([[2, 4, 8, 16], [0; 4], [0; 4], [0; 4]]).unwrap();

    for dir in [Direction::Left, Direction::Right, Direction::Up] {
        assert_eq!(board.shift(dir), board, "{dir} moved something");
        assert_eq!(board.shift_scored(dir), (board, 0));
        assert!(!board.can_move(dir));
        assert!(!board.legal_moves().contains(dir));
    }

    assert!(board.can_move(Direction::Down));
    assert_eq!(board.legal_moves().len(), 1);
    assert_eq!(board.legal_moves().first(), Some(Direction::Down));
}

#[test]
fn dir_set_behaves_like_a_set() {
    assert!(DirSet::EMPTY.is_empty());
    assert_eq!(DirSet::EMPTY.len(), 0);
    assert_eq!(DirSet::EMPTY.first(), None);
    assert_eq!(DirSet::ALL.len(), 4);
    assert_eq!(
        DirSet::ALL.into_iter().collect::<Vec<_>>(),
        Direction::ALL.to_vec()
    );

    let set: DirSet = [Direction::Down, Direction::Left, Direction::Down]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
    assert!(set.contains(Direction::Down) && set.contains(Direction::Left));
    assert!(!set.contains(Direction::Up));
    // Iteration is in `Direction::ALL` order, not insertion order.
    assert_eq!(
        set.into_iter().collect::<Vec<_>>(),
        vec![Direction::Left, Direction::Down]
    );
}

#[test]
fn a_board_prints_as_a_grid() {
    let board =
        Board::from_tiles([[2, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 2048]]).unwrap();
    assert_eq!(
        board.to_string(),
        concat!(
            "    2     .     .     .\n",
            "    .     .     .     .\n",
            "    .     .     .     .\n",
            "    .     .     .  2048",
        )
    );
    assert_eq!(format!("{:?}", Board::EMPTY), "Board(0x0000000000000000)");
}

#[test]
fn a_game_plays_to_completion() {
    let mut game = Game::new(7);
    let mut expected_moves = 0;

    while game.status() == Status::Playing {
        let dir = game
            .legal_moves()
            .first()
            .expect("playing means a legal move exists");
        let round = game.play(dir).expect("the move is legal");
        expected_moves += 1;

        assert_eq!(game.moves(), expected_moves);
        assert_eq!(round.status, game.status());
        assert_eq!(
            game.board().get(round.spawn.row, round.spawn.col),
            round.spawn.value
        );
        assert!(round.spawn.value == 2 || round.spawn.value == 4);
    }

    assert!(game.board().is_game_over());
    assert!(game.legal_moves().is_empty());
    assert!(game.score() > 0);
    for dir in Direction::ALL {
        assert_eq!(game.play(dir), Err(MoveError::GameOver));
    }
}

#[test]
fn replaying_a_seed_reproduces_the_game() {
    let transcript = |seed: u64| {
        let mut game = Game::new(seed);
        let mut log = Vec::new();
        while let Some(dir) = game.legal_moves().first() {
            log.push((game.play(dir).unwrap(), game.board(), game.score()));
        }
        log
    };

    for seed in [0, 1, 0x00C0_FFEE, u64::MAX] {
        assert_eq!(transcript(seed), transcript(seed), "seed {seed} drifted");
    }
    assert_ne!(transcript(1), transcript(2));
}

#[test]
fn a_custom_rng_drives_the_game() {
    /// Always returns zero, which puts every spawn in the lowest empty cell
    /// and, being under the 4-threshold, makes it a 4.
    struct Zeros;

    impl Rng for Zeros {
        fn next_u64(&mut self) -> u64 {
            0
        }
    }

    let game = Game::with_rng(Zeros);
    assert_eq!(game.board().get(3, 3), 4);
    assert_eq!(game.board().get(3, 2), 4);
    assert_eq!(game.board().empty_count(), 14);
}

#[test]
fn resume_derives_the_status_from_the_board() {
    let stuck =
        Board::from_tiles([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]]).unwrap();
    let rng = Xoshiro256pp::seed_from_u64(1);

    let over = Game::resume(stuck, 500, rng.clone());
    assert_eq!(over.status(), Status::Over);
    assert_eq!(over.score(), 500);
    assert_eq!(over.moves(), 0);

    let playing = Game::resume(Board::from_raw(0x0000_0000_0000_0011), 0, rng);
    assert_eq!(playing.status(), Status::Playing);
}

#[test]
fn move_errors_describe_themselves() {
    assert_eq!(
        MoveError::Illegal(Direction::Up).to_string(),
        "no tile can move up"
    );
    assert_eq!(MoveError::GameOver.to_string(), "the game is over");

    let err: &dyn std::error::Error = &MoveError::GameOver;
    assert!(err.source().is_none());
}

/// The workflow the crate docs promise a search agent: expand a node into its
/// four moves, then enumerate the tiles the engine could place.
#[test]
fn a_search_can_enumerate_chance_nodes() {
    let board = Board::from_tiles([[2, 2, 0, 0], [0; 4], [0; 4], [0; 4]]).unwrap();
    let mut leaves = 0;

    for (dir, next) in Direction::ALL.into_iter().zip(board.shift_all()) {
        assert_eq!(next, board.shift(dir));
        if next == board {
            continue;
        }
        for offset in next.empty_cells() {
            for exponent in [1, 2] {
                let child = next.spawn_at(offset, exponent);
                assert_eq!(child.empty_count(), next.empty_count() - 1);
                leaves += 1;
            }
        }
    }

    assert!(leaves > 0);
}
