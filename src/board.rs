//! The bitboard: sixteen tile exponents packed into a `u64`.
//!
//! Cell `(row, col)` occupies bits `60 - 16*row - 4*col ..= 63 - 16*row - 4*col`,
//! so a board written as a hex literal reads in the same order as the grid:
//! `0x0000_0000_0011_2300` has two 2s on row 2 and a 4 on row 3. A nibble of
//! `n` means the tile `1 << n`, and `0` means empty.

use core::fmt;

use crate::direction::{DirSet, Direction};
use crate::rng::Rng;
use crate::tables::TABLES;

/// Side length of the board.
pub const SIZE: usize = 4;

const LOW_BIT_PER_NIBBLE: u64 = 0x1111_1111_1111_1111;

/// Nibbles whose right-hand neighbour is in the same row.
const HORIZONTAL: u64 = 0x0FFF_0FFF_0FFF_0FFF;
/// Nibbles whose downward neighbour is on the board.
const VERTICAL: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Probability of spawning a 4 rather than a 2, as a fraction of `2^32`.
const FOUR_THRESHOLD: u32 = 429_496_730;

/// A 2048 position.
///
/// The whole board is one `u64`, so `Board` is `Copy` and a search can keep
/// snapshots on the stack instead of maintaining an undo log. Nothing here
/// touches randomness or scoring history; that is [`Game`]'s job.
///
/// [`Game`]: crate::Game
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Board(u64);

impl Board {
    /// A board with no tiles.
    pub const EMPTY: Board = Board(0);

    /// Wraps a packed board. Nibbles are tile exponents; see the module docs.
    #[inline(always)]
    pub const fn from_raw(bits: u64) -> Board {
        Board(bits)
    }

    /// The packed representation.
    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Builds a board from tile values.
    ///
    /// Returns `None` unless every entry is 0 or a power of two in `2..=32768`.
    pub fn from_tiles(tiles: [[u16; SIZE]; SIZE]) -> Option<Board> {
        let mut raw = 0u64;
        for (r, row) in tiles.iter().enumerate() {
            for (c, &value) in row.iter().enumerate() {
                let exponent = match value {
                    0 => 0,
                    v if v >= 2 && v.is_power_of_two() => u64::from(v.trailing_zeros()),
                    _ => return None,
                };
                raw |= exponent << offset(r, c);
            }
        }
        Some(Board(raw))
    }

    /// Tile values, row by row. Empty cells read as 0.
    pub fn tiles(self) -> [[u16; SIZE]; SIZE] {
        let mut out = [[0u16; SIZE]; SIZE];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = self.get(r, c);
            }
        }
        out
    }

    /// The tile value at `(row, col)`, or 0 if the cell is empty.
    ///
    /// This is the value, not the packed exponent: a nibble of 11 reads 2048.
    ///
    /// # Panics
    ///
    /// If `row` or `col` is 4 or more.
    #[inline]
    pub fn get(self, row: usize, col: usize) -> u16 {
        assert!(
            row < SIZE && col < SIZE,
            "cell ({row}, {col}) is off the board"
        );
        match (self.0 >> offset(row, col)) & 0xF {
            0 => 0,
            e => 1u16 << e,
        }
    }

    /// The largest tile on the board, or 0 if it is empty.
    pub fn max_tile(self) -> u16 {
        let mut best = 0;
        let mut bits = self.0;
        while bits != 0 {
            best = best.max((bits & 0xF) as u32);
            bits >>= 4;
        }
        if best == 0 { 0 } else { 1u16 << best }
    }

    /// Number of empty cells.
    #[inline(always)]
    pub const fn empty_count(self) -> u32 {
        empty_mask(self.0).count_ones()
    }

    /// Bit offsets of the empty cells, for use with [`Board::spawn_at`].
    ///
    /// Use [`Board::coords`] to turn an offset into a `(row, col)` pair.
    #[inline(always)]
    pub const fn empty_cells(self) -> EmptyCells {
        EmptyCells(empty_mask(self.0))
    }

    /// Grid coordinates of a bit offset produced by [`Board::empty_cells`].
    #[inline(always)]
    pub const fn coords(offset: u32) -> (usize, usize) {
        let cell = 15 - (offset as usize) / 4;
        (cell / SIZE, cell % SIZE)
    }

    /// Applies a move. Returns the board unchanged if the move is illegal.
    #[inline(always)]
    #[must_use]
    pub fn shift(self, dir: Direction) -> Board {
        Board(match dir {
            Direction::Left => slide_left(self.0),
            Direction::Right => slide_right(self.0),
            Direction::Up => transpose(slide_left(transpose(self.0))),
            Direction::Down => transpose(slide_right(transpose(self.0))),
        })
    }

    /// Applies a move and reports the points its merges scored.
    ///
    /// An illegal move returns `(self, 0)`.
    #[inline]
    pub fn shift_scored(self, dir: Direction) -> (Board, u32) {
        match dir {
            Direction::Left => (Board(slide_left(self.0)), score_left(self.0)),
            Direction::Right => (Board(slide_right(self.0)), score_right(self.0)),
            Direction::Up => {
                let t = transpose(self.0);
                (Board(transpose(slide_left(t))), score_left(t))
            }
            Direction::Down => {
                let t = transpose(self.0);
                (Board(transpose(slide_right(t))), score_right(t))
            }
        }
    }

    /// Applies all four moves, in [`Direction::ALL`] order.
    ///
    /// Cheaper than four calls to [`Board::shift`] because the transpose is
    /// shared and the sixteen table loads have no dependencies between them.
    /// This is what a search should call at each node.
    #[inline]
    pub fn shift_all(self) -> [Board; 4] {
        let t = transpose(self.0);
        [
            Board(slide_left(self.0)),
            Board(slide_right(self.0)),
            Board(transpose(slide_left(t))),
            Board(transpose(slide_right(t))),
        ]
    }

    /// Reflects the board about its main diagonal, so cell `(r, c)` ends up at
    /// `(c, r)`.
    ///
    /// Two delta swaps and no memory traffic. This is its own inverse. It is
    /// public because an evaluation function that scores a line at a time
    /// needs the columns as rows, which is otherwise the one thing a search
    /// has to reimplement.
    ///
    /// ```
    /// # use twenty48::Board;
    /// let board = Board::from_tiles([
    ///     [2, 4, 0, 0],
    ///     [0, 0, 0, 0],
    ///     [0, 0, 0, 0],
    ///     [0, 0, 0, 0],
    /// ]).unwrap();
    ///
    /// assert_eq!(board.transpose().get(1, 0), 4);
    /// assert_eq!(board.transpose().transpose(), board);
    /// ```
    #[inline(always)]
    #[must_use]
    pub const fn transpose(self) -> Board {
        Board(transpose(self.0))
    }

    /// Whether `dir` changes the board.
    #[inline(always)]
    pub fn can_move(self, dir: Direction) -> bool {
        self.shift(dir) != self
    }

    /// The set of moves that change the board.
    #[inline]
    pub fn legal_moves(self) -> DirSet {
        let next = self.shift_all();
        let bits = u8::from(next[0] != self)
            | u8::from(next[1] != self) << 1
            | u8::from(next[2] != self) << 2
            | u8::from(next[3] != self) << 3;
        DirSet::from_bits(bits)
    }

    /// Whether no move is legal.
    ///
    /// Always agrees with `self.legal_moves().is_empty()`. A full board is not
    /// necessarily over: it is over only when no two adjacent tiles can merge.
    /// A board with no tiles at all has nothing to slide, so it counts as
    /// over rather than leaving [`Game`] in a state that rejects every move.
    ///
    /// [`Game`]: crate::Game
    #[inline]
    pub fn is_game_over(self) -> bool {
        if self.0 == 0 {
            return true;
        }
        if empty_mask(self.0) != 0 {
            return false;
        }
        mergeable(self.0, self.0 >> 4, HORIZONTAL) == 0
            && mergeable(self.0, self.0 >> 16, VERTICAL) == 0
    }

    /// Places a tile of value `1 << exponent` at a bit `offset`.
    ///
    /// The cell must be empty and `exponent` must be in `1..=15`; both are
    /// checked only with `debug_assert!`, so that the chance nodes of a search
    /// compile to a single `or`.
    #[inline(always)]
    #[must_use]
    pub const fn spawn_at(self, offset: u32, exponent: u8) -> Board {
        debug_assert!(
            offset < 64 && offset.is_multiple_of(4),
            "offset is not a cell"
        );
        debug_assert!(empty_mask(self.0) & (1 << offset) != 0, "cell is occupied");
        debug_assert!(exponent >= 1 && exponent <= 15, "exponent out of range");
        Board(self.0 | ((exponent as u64) << offset))
    }

    /// Picks where the next tile goes and what it is, without placing it.
    ///
    /// Returns a bit offset and an exponent (1 for a 2, 2 for a 4), or `None`
    /// if the board is full. Consumes exactly one value from `rng`.
    #[inline]
    pub fn draw_spawn<R: Rng>(self, rng: &mut R) -> Option<(u32, u8)> {
        let mut mask = empty_mask(self.0);
        if mask == 0 {
            return None;
        }
        let count = mask.count_ones();
        let draw = rng.next_u64();

        // Lemire's bounded draw: uniform over `0..count` without a division.
        let mut skip = ((draw as u32 as u64) * u64::from(count)) >> 32;
        while skip > 0 {
            mask &= mask - 1;
            skip -= 1;
        }

        let exponent = if ((draw >> 32) as u32) < FOUR_THRESHOLD {
            2
        } else {
            1
        };
        Some((mask.trailing_zeros(), exponent))
    }

    /// Places a random tile. Returns the board unchanged if it is full.
    #[inline]
    #[must_use]
    pub fn spawn<R: Rng>(self, rng: &mut R) -> Board {
        match self.draw_spawn(rng) {
            Some((offset, exponent)) => self.spawn_at(offset, exponent),
            None => self,
        }
    }
}

/// Bit offset of cell `(row, col)`.
#[inline(always)]
const fn offset(row: usize, col: usize) -> u32 {
    (60 - 16 * row - 4 * col) as u32
}

/// Low bit of every empty nibble.
///
/// `count_ones` is the number of empty cells and `trailing_zeros` is directly
/// a bit offset, so neither needs a multiply.
#[inline(always)]
pub(crate) const fn empty_mask(bits: u64) -> u64 {
    let folded = bits | (bits >> 2);
    let folded = folded | (folded >> 1);
    !folded & LOW_BIT_PER_NIBBLE
}

/// Reflects the board about its main diagonal, so that a column move can be
/// done with the row tables.
///
/// Two delta swaps: within each 2x2 block (3 nibbles apart), then between the
/// off-diagonal blocks (6 nibbles apart).
#[inline(always)]
pub(crate) const fn transpose(bits: u64) -> u64 {
    let a = (bits & 0xF0F0_0F0F_F0F0_0F0F)
        | ((bits & 0x0000_F0F0_0000_F0F0) << 12)
        | ((bits & 0x0F0F_0000_0F0F_0000) >> 12);
    (a & 0xFF00_FF00_00FF_00FF)
        | ((a & 0x00FF_00FF_0000_0000) >> 24)
        | ((a & 0x0000_0000_FF00_FF00) << 24)
}

/// Low bit set for each in-`mask` nibble of `a` that can merge with the
/// corresponding nibble of `b`.
///
/// Callers pass `b` pre-shifted so that nibble `k` of `b` is nibble `k`'s
/// neighbour, and a `mask` that drops the pairs which would wrap across an
/// edge. Two 32768s do not merge, because 65536 has no exponent left.
#[inline(always)]
const fn mergeable(a: u64, b: u64, mask: u64) -> u64 {
    let equal = empty_mask((a ^ b) | !mask);
    equal & !saturated_mask(a & b)
}

/// Low bit of every nibble equal to 15.
#[inline(always)]
const fn saturated_mask(bits: u64) -> u64 {
    let folded = bits & (bits >> 2);
    let folded = folded & (folded >> 1);
    folded & LOW_BIT_PER_NIBBLE
}

// Indices are written `as u16 as usize` rather than with an explicit mask so
// that LLVM sees the range as a type fact and folds the bounds check away. CI
// greps the release assembly for `panic_bounds_check` to keep it that way.

#[inline(always)]
fn slide_left(bits: u64) -> u64 {
    let t = &TABLES.left.result;
    ((t[(bits >> 48) as u16 as usize] as u64) << 48)
        | ((t[(bits >> 32) as u16 as usize] as u64) << 32)
        | ((t[(bits >> 16) as u16 as usize] as u64) << 16)
        | (t[bits as u16 as usize] as u64)
}

#[inline(always)]
fn slide_right(bits: u64) -> u64 {
    let t = &TABLES.right.result;
    ((t[(bits >> 48) as u16 as usize] as u64) << 48)
        | ((t[(bits >> 32) as u16 as usize] as u64) << 32)
        | ((t[(bits >> 16) as u16 as usize] as u64) << 16)
        | (t[bits as u16 as usize] as u64)
}

#[inline(always)]
fn score_left(bits: u64) -> u32 {
    let t = &TABLES.left.score;
    t[(bits >> 48) as u16 as usize]
        + t[(bits >> 32) as u16 as usize]
        + t[(bits >> 16) as u16 as usize]
        + t[bits as u16 as usize]
}

#[inline(always)]
fn score_right(bits: u64) -> u32 {
    let t = &TABLES.right.score;
    t[(bits >> 48) as u16 as usize]
        + t[(bits >> 32) as u16 as usize]
        + t[(bits >> 16) as u16 as usize]
        + t[bits as u16 as usize]
}

/// Iterator over the empty cells of a board, as bit offsets.
#[derive(Clone, Debug)]
pub struct EmptyCells(u64);

impl Iterator for EmptyCells {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        if self.0 == 0 {
            return None;
        }
        let offset = self.0.trailing_zeros();
        self.0 &= self.0 - 1;
        Some(offset)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.0.count_ones() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for EmptyCells {}
impl core::iter::FusedIterator for EmptyCells {}

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in 0..SIZE {
            if row > 0 {
                writeln!(f)?;
            }
            for col in 0..SIZE {
                if col > 0 {
                    f.write_str(" ")?;
                }
                match self.get(row, col) {
                    0 => f.write_str("    .")?,
                    v => write!(f, "{v:>5}")?,
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Board({:#018x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Xoshiro256pp;

    /// Reverses all sixteen nibbles, reflecting the board about both axes.
    ///
    /// The engine does not need this: it carries a table for each direction.
    /// It is the cheaper one-table alternative, kept here to cross-check the
    /// right-hand table.
    const fn rotate_half(bits: u64) -> u64 {
        let x = bits.swap_bytes();
        ((x & 0x0F0F_0F0F_0F0F_0F0F) << 4) | ((x >> 4) & 0x0F0F_0F0F_0F0F_0F0F)
    }

    /// Boards drawn from actual play, which have a very different nibble
    /// distribution from uniform random `u64`s.
    fn played_boards(count: usize) -> Vec<Board> {
        let mut rng = Xoshiro256pp::seed_from_u64(0x5EED);
        let mut boards = Vec::with_capacity(count);
        while boards.len() < count {
            let mut board = Board::EMPTY.spawn(&mut rng).spawn(&mut rng);
            loop {
                boards.push(board);
                if boards.len() == count {
                    break;
                }
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

    fn corpus() -> Vec<Board> {
        let mut rng = Xoshiro256pp::seed_from_u64(1);
        let mut boards = played_boards(4096);
        // Adversarial extras: uniform noise reaches states play never does.
        boards.extend((0..4096).map(|_| Board::from_raw(rng.next_u64())));
        boards.push(Board::EMPTY);
        boards
    }

    #[test]
    fn transpose_is_an_involution() {
        for board in corpus() {
            assert_eq!(transpose(transpose(board.raw())), board.raw(), "{board:?}");
        }
    }

    #[test]
    fn transpose_swaps_rows_and_columns() {
        for board in corpus() {
            let t = Board::from_raw(transpose(board.raw()));
            for row in 0..SIZE {
                for col in 0..SIZE {
                    assert_eq!(t.get(row, col), board.get(col, row), "{board:?}");
                }
            }
        }
    }

    #[test]
    fn rotate_half_is_an_involution() {
        for board in corpus() {
            assert_eq!(rotate_half(rotate_half(board.raw())), board.raw());
        }
    }

    #[test]
    fn rotate_half_reflects_both_axes() {
        for board in corpus() {
            let r = Board::from_raw(rotate_half(board.raw()));
            for row in 0..SIZE {
                for col in 0..SIZE {
                    assert_eq!(r.get(row, col), board.get(3 - row, 3 - col), "{board:?}");
                }
            }
        }
    }

    /// Right can be built either from its own table or from the left table
    /// sandwiched between half-turns. The two must agree.
    #[test]
    fn right_table_agrees_with_the_rotate_sandwich() {
        for board in corpus() {
            let via_table = slide_right(board.raw());
            let via_rotate = rotate_half(slide_left(rotate_half(board.raw())));
            assert_eq!(via_table, via_rotate, "{board:?}");
        }
    }

    #[test]
    fn empty_mask_marks_exactly_the_empty_cells() {
        for board in corpus() {
            let offsets: Vec<_> = board.empty_cells().collect();
            assert_eq!(offsets.len() as u32, board.empty_count());
            for offset in offsets {
                let (row, col) = Board::coords(offset);
                assert_eq!(board.get(row, col), 0, "{board:?} at {offset}");
            }
            let by_cell = board.tiles().iter().flatten().filter(|&&v| v == 0).count();
            assert_eq!(by_cell as u32, board.empty_count(), "{board:?}");
        }
    }

    #[test]
    fn coords_inverts_the_cell_offset() {
        for row in 0..SIZE {
            for col in 0..SIZE {
                assert_eq!(Board::coords(offset(row, col)), (row, col));
            }
        }
    }

    #[test]
    fn hex_literals_read_like_the_grid() {
        let board = Board::from_raw(0x0000_0000_0011_2300);
        assert_eq!(board.get(2, 2), 2);
        assert_eq!(board.get(2, 3), 2);
        assert_eq!(board.get(3, 0), 4);
        assert_eq!(board.get(3, 1), 8);
        assert_eq!(board.get(0, 0), 0);
        // Row 2 merges to a single 4; row 3 is already packed left.
        assert_eq!(board.shift(Direction::Left).raw(), 0x0000_0000_2000_2300);
        assert_eq!(board.shift(Direction::Right).raw(), 0x0000_0000_0002_0023);
    }

    #[test]
    fn shift_all_matches_shift() {
        for board in corpus() {
            let all = board.shift_all();
            for dir in Direction::ALL {
                assert_eq!(all[dir.index()], board.shift(dir), "{board:?} {dir}");
            }
        }
    }

    #[test]
    fn game_over_agrees_with_the_legal_move_set() {
        for board in corpus() {
            assert_eq!(
                board.is_game_over(),
                board.legal_moves().is_empty(),
                "{board:?}\n{board}"
            );
        }
    }

    #[test]
    fn a_full_board_with_a_mergeable_pair_is_not_over() {
        let board =
            Board::from_tiles([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 4]]).unwrap();
        assert_eq!(board.empty_count(), 0);
        assert!(!board.is_game_over());
        assert!(!board.legal_moves().is_empty());

        let stuck =
            Board::from_tiles([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]]).unwrap();
        assert_eq!(stuck.empty_count(), 0);
        assert!(stuck.is_game_over());
        assert!(stuck.legal_moves().is_empty());
    }

    /// 65536 has no exponent left, so a pair of 32768s is not a legal move and
    /// must not read as one. Unreachable in play, but `from_raw` is public.
    #[test]
    fn adjacent_maximum_tiles_do_not_merge() {
        let mut tiles = [[0u16; SIZE]; SIZE];
        for (r, row) in tiles.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = if (r + c) % 2 == 0 { 2 } else { 4 };
            }
        }
        tiles[0][0] = 32768;
        tiles[0][1] = 32768;
        let board = Board::from_tiles(tiles).unwrap();

        assert_eq!(board.empty_count(), 0);
        assert!(board.legal_moves().is_empty());
        assert!(board.is_game_over());
    }

    #[test]
    fn from_tiles_rejects_non_powers_of_two() {
        assert!(Board::from_tiles([[3, 0, 0, 0], [0; 4], [0; 4], [0; 4]]).is_none());
        assert!(Board::from_tiles([[1, 0, 0, 0], [0; 4], [0; 4], [0; 4]]).is_none());
        assert!(Board::from_tiles([[32768, 0, 0, 0], [0; 4], [0; 4], [0; 4]]).is_some());
    }

    #[test]
    fn tiles_round_trips() {
        for board in played_boards(512) {
            assert_eq!(Board::from_tiles(board.tiles()), Some(board));
        }
    }

    #[test]
    fn max_tile_tracks_the_largest_exponent() {
        assert_eq!(Board::EMPTY.max_tile(), 0);
        assert_eq!(Board::from_raw(0xF000_0000_0000_0001).max_tile(), 32768);
        for board in played_boards(512) {
            let want = board.tiles().iter().flatten().copied().max().unwrap();
            assert_eq!(board.max_tile(), want, "{board:?}");
        }
    }

    #[test]
    fn spawns_land_only_on_empty_cells() {
        let mut rng = Xoshiro256pp::seed_from_u64(7);
        for board in played_boards(2048) {
            let Some((offset, exponent)) = board.draw_spawn(&mut rng) else {
                assert_eq!(board.empty_count(), 0);
                continue;
            };
            let (row, col) = Board::coords(offset);
            assert_eq!(board.get(row, col), 0, "{board:?}");
            assert!(exponent == 1 || exponent == 2);
            let after = board.spawn_at(offset, exponent);
            assert_eq!(after.empty_count(), board.empty_count() - 1);
            assert_eq!(after.get(row, col), 1 << exponent);
        }
    }

    #[test]
    fn four_spawns_about_one_time_in_ten() {
        let mut rng = Xoshiro256pp::seed_from_u64(0xABCD);
        let mut fours = 0;
        let trials = 200_000;
        for _ in 0..trials {
            let (_, exponent) = Board::EMPTY.draw_spawn(&mut rng).unwrap();
            fours += u32::from(exponent == 2);
        }
        let ratio = f64::from(fours) / f64::from(trials);
        assert!(
            (ratio - 0.1).abs() < 0.005,
            "spawned a 4 {ratio} of the time"
        );
    }

    #[test]
    fn spawn_positions_are_uniform() {
        let mut rng = Xoshiro256pp::seed_from_u64(0x1234);
        let mut counts = [0u32; 16];
        let trials = 320_000;
        for _ in 0..trials {
            let (offset, _) = Board::EMPTY.draw_spawn(&mut rng).unwrap();
            counts[(offset / 4) as usize] += 1;
        }
        let expected = trials as f64 / 16.0;
        for (cell, &count) in counts.iter().enumerate() {
            let error = (f64::from(count) - expected).abs() / expected;
            assert!(
                error < 0.05,
                "cell {cell} got {count}, expected ~{expected}"
            );
        }
    }

    /// A full top row with nothing below it can only go down: it is already
    /// flush left, flush right and flush up.
    #[test]
    fn an_illegal_shift_leaves_the_board_alone() {
        let board =
            Board::from_tiles([[2, 4, 8, 16], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]).unwrap();
        for dir in [Direction::Left, Direction::Right, Direction::Up] {
            assert!(!board.can_move(dir), "{dir} should not be legal");
            assert_eq!(board.shift(dir), board);
            assert_eq!(board.shift_scored(dir), (board, 0));
        }
        assert!(board.can_move(Direction::Down));
        assert_eq!(board.legal_moves().len(), 1);
    }

    /// An empty board has nothing to slide, and the two ways of asking must
    /// still agree.
    #[test]
    fn an_empty_board_is_over() {
        assert!(Board::EMPTY.legal_moves().is_empty());
        assert!(Board::EMPTY.is_game_over());
    }

    #[test]
    fn display_lines_up_with_the_grid() {
        let board = Board::from_tiles([
            [2, 0, 0, 0],
            [0, 1024, 0, 0],
            [0, 0, 0, 0],
            [0, 0, 0, 32768],
        ])
        .unwrap();
        let rendered = board.to_string();
        let lines: Vec<_> = rendered.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "    2     .     .     .");
        assert_eq!(lines[1], "    .  1024     .     .");
        assert_eq!(lines[3], "    .     .     . 32768");
    }
}
