//! A deliberately naive 2048, written to be read rather than run.
//!
//! Tiles live in a `[[u8; 4]; 4]` of exponents and a slide is a filter into a
//! `Vec` followed by a pairwise walk, which shares no structure with the
//! engine's packed rows and table lookups. That independence is the point: the
//! two implementations are wrong in the same way only by coincidence.

use twenty48::{Direction, SIZE};

/// A board as a grid of tile exponents, row-major. `0` is an empty cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grid(pub [[u8; SIZE]; SIZE]);

impl Grid {
    pub fn from_raw(bits: u64) -> Grid {
        let mut cells = [[0u8; SIZE]; SIZE];
        for (r, row) in cells.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = ((bits >> shift_of(r, c)) & 0xF) as u8;
            }
        }
        Grid(cells)
    }

    pub fn to_raw(self) -> u64 {
        let mut bits = 0u64;
        for (r, row) in self.0.iter().enumerate() {
            for (c, &cell) in row.iter().enumerate() {
                bits |= u64::from(cell) << shift_of(r, c);
            }
        }
        bits
    }

    pub fn shift(self, dir: Direction) -> (Grid, u32) {
        let mut out = [[0u8; SIZE]; SIZE];
        let mut score = 0;

        for i in 0..SIZE {
            let mut line = [0u8; SIZE];
            for (j, cell) in line.iter_mut().enumerate() {
                let (r, c) = cell_of(dir, i, j);
                *cell = self.0[r][c];
            }

            let (slid, gained) = slide(line);
            score += gained;

            for (j, &value) in slid.iter().enumerate() {
                let (r, c) = cell_of(dir, i, j);
                out[r][c] = value;
            }
        }

        (Grid(out), score)
    }

    pub fn legal_moves(self) -> Vec<Direction> {
        Direction::ALL
            .into_iter()
            .filter(|&dir| self.shift(dir).0 != self)
            .collect()
    }

    pub fn is_game_over(self) -> bool {
        self.legal_moves().is_empty()
    }

    /// Sum of the tile values on the board.
    pub fn total(self) -> u32 {
        self.0
            .iter()
            .flatten()
            .map(|&e| if e == 0 { 0 } else { 1u32 << e })
            .sum()
    }
}

/// Bit position of cell `(row, col)` in the packed board.
fn shift_of(row: usize, col: usize) -> u32 {
    (60 - 16 * row - 4 * col) as u32
}

/// Cell `j` of line `i`, counted from the edge the tiles move toward.
fn cell_of(dir: Direction, i: usize, j: usize) -> (usize, usize) {
    match dir {
        Direction::Left => (i, j),
        Direction::Right => (i, SIZE - 1 - j),
        Direction::Up => (j, i),
        Direction::Down => (SIZE - 1 - j, i),
    }
}

/// Slides one line toward index 0.
fn slide(line: [u8; SIZE]) -> ([u8; SIZE], u32) {
    let tiles: Vec<u8> = line.into_iter().filter(|&e| e != 0).collect();

    let mut merged = Vec::with_capacity(SIZE);
    let mut score = 0;
    let mut i = 0;
    while i < tiles.len() {
        // 32768 is the largest representable tile, so a pair of them stays put.
        if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] && tiles[i] < 15 {
            merged.push(tiles[i] + 1);
            score += 1u32 << (tiles[i] + 1);
            i += 2;
        } else {
            merged.push(tiles[i]);
            i += 1;
        }
    }

    let mut out = [0u8; SIZE];
    out[..merged.len()].copy_from_slice(&merged);
    (out, score)
}
