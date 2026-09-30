//! Precomputed results of sliding a single row.
//!
//! A row is four 4-bit exponents, so there are only 65536 of them and every
//! slide can be looked up. The tables are built by const evaluation, which
//! keeps their base address a link-time constant: no lazy-init branch and no
//! atomic load on the hot path.

/// Slide results for one direction, indexed by the packed row.
pub(crate) struct RowTable {
    /// Row after sliding and merging.
    pub result: [u16; 65536],
    /// Points the merges scored.
    ///
    /// `u32` rather than `u16` because `[16384, 16384, 16384, 16384]` scores
    /// 65536. This table is only read by [`Board::shift_scored`], so the extra
    /// 128 KiB never competes for cache with the move itself.
    ///
    /// [`Board::shift_scored`]: crate::Board::shift_scored
    pub score: [u32; 65536],
}

#[repr(align(64))]
pub(crate) struct Tables {
    pub left: RowTable,
    pub right: RowTable,
}

pub(crate) static TABLES: Tables = build();

/// The rules of 2048, for one row, sliding toward index 0.
///
/// Tiles compact toward the moving edge and equal neighbours merge, but each
/// tile takes part in at most one merge per move: `[2,2,2,2]` gives `[4,4]`,
/// not `[8]`, and `[4,4,4]` gives `[8,4]`, not `[4,8]`.
const fn slide_left(line: [u8; 4]) -> ([u8; 4], u32) {
    let mut out = [0u8; 4];
    let mut score = 0u32;
    let mut read = 0usize;
    let mut write = 0usize;

    while read < 4 {
        if line[read] == 0 {
            read += 1;
            continue;
        }
        let v = line[read];

        let mut peek = read + 1;
        while peek < 4 && line[peek] == 0 {
            peek += 1;
        }

        if peek < 4 && line[peek] == v && v < 15 {
            out[write] = v + 1;
            score += 1u32 << (v + 1);
            read = peek + 1; // both tiles consumed: neither can merge again
        } else {
            out[write] = v;
            read += 1;
        }
        write += 1;
    }

    (out, score)
}

/// Reverses the four nibbles of a row.
#[inline(always)]
pub(crate) const fn reverse_row(row: u16) -> u16 {
    let x = row.swap_bytes();
    ((x & 0x0F0F) << 4) | ((x >> 4) & 0x0F0F)
}

const fn unpack(row: u16) -> [u8; 4] {
    [
        (row >> 12) as u8 & 0xF,
        (row >> 8) as u8 & 0xF,
        (row >> 4) as u8 & 0xF,
        row as u8 & 0xF,
    ]
}

const fn pack(line: [u8; 4]) -> u16 {
    ((line[0] as u16) << 12) | ((line[1] as u16) << 8) | ((line[2] as u16) << 4) | line[3] as u16
}

/// Builds both direction tables in a single pass.
///
/// Right is the mirror of left, so each iteration fills `left[row]` and
/// `right[reverse_row(row)]`. `reverse_row` is a bijection, so every right
/// entry is written exactly once.
// `large_stack_arrays` does not apply: this runs at compile time and its result
// is a `static`, so the arrays below never exist on a runtime stack.
#[allow(clippy::large_stack_arrays)]
const fn build() -> Tables {
    let mut left = RowTable {
        result: [0; 65536],
        score: [0; 65536],
    };
    let mut right = RowTable {
        result: [0; 65536],
        score: [0; 65536],
    };

    let mut i = 0usize;
    while i < 65536 {
        let row = i as u16;
        let (line, score) = slide_left(unpack(row));
        let packed = pack(line);

        left.result[i] = packed;
        left.score[i] = score;

        let mirrored = reverse_row(row) as usize;
        right.result[mirrored] = reverse_row(packed);
        right.score[mirrored] = score;

        i += 1;
    }

    Tables { left, right }
}

// Tripwires against a layout flip: nibble 3 of the index is the leftmost cell.
const _: () = assert!(TABLES.left.result[0x1100] == 0x2000);
const _: () = assert!(TABLES.left.score[0x1100] == 4);
const _: () = assert!(TABLES.right.result[0x1100] == 0x0002);
const _: () = assert!(TABLES.left.result[0x1111] == 0x2200);
const _: () = assert!(TABLES.left.score[0x1111] == 8);

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases that pin down merge-once and compact-first, as exponents.
    const GOLDEN: &[([u8; 4], [u8; 4], u32)] = &[
        ([0, 0, 0, 0], [0, 0, 0, 0], 0),
        ([1, 1, 1, 1], [2, 2, 0, 0], 8),
        ([2, 1, 1, 0], [2, 2, 0, 0], 4),
        ([1, 1, 2, 0], [2, 2, 0, 0], 4),
        ([2, 2, 2, 0], [3, 2, 0, 0], 8),
        ([1, 0, 1, 2], [2, 2, 0, 0], 4),
        ([1, 1, 1, 0], [2, 1, 0, 0], 4),
        ([0, 0, 0, 1], [1, 0, 0, 0], 0),
        ([1, 2, 3, 4], [1, 2, 3, 4], 0),
        ([15, 15, 0, 0], [15, 15, 0, 0], 0),
    ];

    #[test]
    fn golden_rows() {
        for &(input, want, want_score) in GOLDEN {
            let (got, got_score) = slide_left(input);
            assert_eq!(got, want, "slide_left({input:?})");
            assert_eq!(got_score, want_score, "score of slide_left({input:?})");
        }
    }

    #[test]
    fn tables_match_the_slide_function() {
        for i in 0..=u16::MAX {
            let (line, score) = slide_left(unpack(i));
            assert_eq!(TABLES.left.result[i as usize], pack(line), "row {i:#06x}");
            assert_eq!(TABLES.left.score[i as usize], score, "row {i:#06x}");
        }
    }

    /// Cross-validates the two-table layout against the cheaper one-table
    /// alternative (`reverse -> slide left -> reverse`).
    #[test]
    fn right_is_the_mirror_of_left() {
        for i in 0..=u16::MAX {
            let mirrored = reverse_row(i) as usize;
            assert_eq!(
                TABLES.right.result[i as usize],
                reverse_row(TABLES.left.result[mirrored]),
                "row {i:#06x}"
            );
            assert_eq!(
                TABLES.right.score[i as usize], TABLES.left.score[mirrored],
                "row {i:#06x}"
            );
        }
    }

    #[test]
    fn reverse_row_is_an_involution() {
        for i in 0..=u16::MAX {
            assert_eq!(reverse_row(reverse_row(i)), i);
        }
        assert_eq!(reverse_row(0x1234), 0x4321);
    }

    #[test]
    fn slide_conserves_total_tile_value() {
        let sum = |line: [u8; 4]| -> u32 {
            line.iter()
                .map(|&e| if e == 0 { 0 } else { 1u32 << e })
                .sum()
        };
        for i in 0..=u16::MAX {
            let before = unpack(i);
            let (after, _) = slide_left(before);
            assert_eq!(sum(before), sum(after), "row {i:#06x}");
        }
    }

    /// Recovers the merge count from the exponent histograms and checks the
    /// score against it, rather than trusting `slide_left`'s own accounting.
    ///
    /// A merge turns two tiles of exponent `k` into one of `k + 1`, so
    /// `after[k] == before[k] - 2*merges[k] + merges[k-1]`, which solves
    /// upward from `merges[0] == 0`.
    #[test]
    fn score_equals_the_value_of_created_tiles() {
        let histogram = |line: [u8; 4]| {
            let mut h = [0i32; 17];
            for &e in &line {
                h[e as usize] += 1;
            }
            h
        };

        for i in 0..=u16::MAX {
            let before = unpack(i);
            let (after, score) = slide_left(before);
            let (b, a) = (histogram(before), histogram(after));

            let mut expected = 0u32;
            let mut carried = 0i32;
            for k in 1..16 {
                let delta = b[k] + carried - a[k];
                assert!(delta >= 0 && delta % 2 == 0, "row {i:#06x} at exponent {k}");
                let merges = delta / 2;
                expected += u32::try_from(merges).unwrap() * (1u32 << (k + 1));
                carried = merges;
            }
            assert_eq!(expected, score, "row {i:#06x}");
        }
    }

    /// Guards the `u32` score column: four 16384s merge for 65536, one past
    /// what a `u16` could hold.
    #[test]
    fn the_largest_score_needs_more_than_sixteen_bits() {
        let worst = (0..=u16::MAX)
            .map(|i| slide_left(unpack(i)).1)
            .max()
            .unwrap();
        assert_eq!(worst, 65536);
        assert_eq!(TABLES.left.score[0xEEEE], 65536);
    }
}
