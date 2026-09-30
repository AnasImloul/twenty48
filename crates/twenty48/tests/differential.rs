//! The packed engine against the naive one in [`reference`].
//!
//! Row slides are checked exhaustively; whole boards are checked over seeded
//! playouts, because the reachable positions are a vanishing fraction of the
//! 2^64 a `Board` can hold and uniform-random bits would miss the ones that
//! matter (full boards, saturated pairs, near-game-over).

mod reference;

use reference::Grid;
use twenty48::{Board, Direction, Rng, Xoshiro256pp};

/// Boards taken from real games, sampled after every move.
fn corpus() -> Vec<Board> {
    let mut rng = Xoshiro256pp::seed_from_u64(0x5EED);
    let mut boards = vec![Board::EMPTY];

    for _ in 0..64 {
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        loop {
            boards.push(board);
            let legal = board.legal_moves();
            let Some(dir) = legal
                .into_iter()
                .nth(rng.next_u64() as usize % legal.len().max(1))
            else {
                break;
            };
            board = board.shift(dir).spawn(&mut rng);
        }
    }

    boards
}

/// The four corpus-driven tests below are only as good as the corpus, so this
/// pins down that it reaches the positions where the bit tricks are delicate.
#[test]
fn the_corpus_reaches_full_and_finished_boards() {
    let boards = corpus();
    assert!(
        boards.len() > 5_000,
        "corpus is only {} boards",
        boards.len()
    );
    assert!(boards.iter().any(|b| b.empty_count() == 0));
    assert!(boards.iter().any(|b| b.is_game_over()));
    assert!(boards.iter().any(|b| b.max_tile() >= 256));
}

#[test]
fn horizontal_slides_match_the_oracle_on_every_row() {
    for row in 0..=u16::MAX {
        let board = Board::from_raw(u64::from(row) << 48);
        let grid = Grid::from_raw(board.raw());

        for dir in [Direction::Left, Direction::Right] {
            let (got, gained) = board.shift_scored(dir);
            let (want, want_score) = grid.shift(dir);
            assert_eq!(got.raw(), want.to_raw(), "{dir} on row {row:#06x}");
            assert_eq!(gained, want_score, "{dir} score on row {row:#06x}");
        }
    }
}

#[test]
fn vertical_slides_match_the_oracle_on_every_column() {
    for row in 0..=u16::MAX {
        // Spread the four nibbles down column 0, so the transpose is exercised.
        let mut raw = 0u64;
        for j in 0..4 {
            let nibble = u64::from(row >> (12 - 4 * j)) & 0xF;
            raw |= nibble << (60 - 16 * j);
        }

        let board = Board::from_raw(raw);
        let grid = Grid::from_raw(raw);

        for dir in [Direction::Up, Direction::Down] {
            let (got, gained) = board.shift_scored(dir);
            let (want, want_score) = grid.shift(dir);
            assert_eq!(got.raw(), want.to_raw(), "{dir} on column {row:#06x}");
            assert_eq!(gained, want_score, "{dir} score on column {row:#06x}");
        }
    }
}

#[test]
fn every_shift_of_every_reachable_board_matches_the_oracle() {
    for board in corpus() {
        let grid = Grid::from_raw(board.raw());
        let all = board.shift_all();

        for dir in Direction::ALL {
            let (want, want_score) = grid.shift(dir);
            let (got, gained) = board.shift_scored(dir);
            assert_eq!(got.raw(), want.to_raw(), "{dir} on {board:?}");
            assert_eq!(gained, want_score, "{dir} score on {board:?}");
            assert_eq!(all[dir.index()], got, "shift_all disagrees on {dir}");
        }
    }
}

#[test]
fn playouts_agree_move_by_move() {
    for seed in 0..16 {
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
        let mut grid = Grid::from_raw(board.raw());
        let mut score = 0u32;
        let mut oracle_score = 0u32;

        loop {
            assert_eq!(board.raw(), grid.to_raw());
            assert_eq!(score, oracle_score);
            assert_eq!(
                board.legal_moves().into_iter().collect::<Vec<_>>(),
                grid.legal_moves(),
                "legal moves disagree on {board:?}"
            );
            assert_eq!(
                board.is_game_over(),
                grid.is_game_over(),
                "game over disagrees on {board:?}"
            );

            let legal = grid.legal_moves();
            if legal.is_empty() {
                break;
            }
            let dir = legal[rng.next_u64() as usize % legal.len()];

            let (slid, gained) = board.shift_scored(dir);
            let (slid_grid, oracle_gained) = grid.shift(dir);
            score += gained;
            oracle_score += oracle_gained;

            // One draw feeds both, so the two games stay in lockstep.
            let (offset, exponent) = slid
                .draw_spawn(&mut rng)
                .expect("a legal move frees a cell");
            let (r, c) = Board::coords(offset);

            board = slid.spawn_at(offset, exponent);
            grid = slid_grid;
            assert_eq!(grid.0[r][c], 0, "spawn landed on an occupied cell");
            grid.0[r][c] = exponent;
        }

        assert!(score > 0, "seed {seed} scored nothing");
    }
}

#[test]
fn shifts_conserve_the_total_tile_value() {
    for board in corpus() {
        let before = Grid::from_raw(board.raw()).total();
        for dir in Direction::ALL {
            let after = Grid::from_raw(board.shift(dir).raw()).total();
            assert_eq!(before, after, "{dir} on {board:?}");
        }
    }
}

/// A move's score is the sum of the tiles it created, which is a different
/// statement from "the engine and the oracle agree" and fails differently.
#[test]
fn a_move_scores_the_value_of_the_tiles_it_creates() {
    let histogram = |board: Board| {
        let mut counts = [0i32; 16];
        for &e in Grid::from_raw(board.raw()).0.iter().flatten() {
            counts[e as usize] += 1;
        }
        counts
    };

    for board in corpus() {
        let before = histogram(board);
        for dir in Direction::ALL {
            let (next, gained) = board.shift_scored(dir);
            let after = histogram(next);

            // A merge at exponent k consumes two k and yields one k+1, so the
            // merge counts solve upward from `merges[0] == 0`.
            let mut expected = 0u32;
            let mut carried = 0i32;
            for k in 1..15 {
                let delta = before[k] + carried - after[k];
                assert!(delta >= 0 && delta % 2 == 0, "{dir} on {board:?} at {k}");
                let merges = delta / 2;
                expected += u32::try_from(merges).unwrap() * (1u32 << (k + 1));
                carried = merges;
            }
            assert_eq!(expected, gained, "{dir} on {board:?}");
        }
    }
}

#[test]
fn game_over_agrees_with_the_oracle() {
    for board in corpus() {
        assert_eq!(
            board.is_game_over(),
            Grid::from_raw(board.raw()).is_game_over(),
            "{board:?}"
        );
    }
}
