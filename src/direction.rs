use core::fmt;

/// One of the four slide directions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Direction {
    /// Slide toward column 0.
    Left = 0,
    /// Slide toward column 3.
    Right = 1,
    /// Slide toward row 0.
    Up = 2,
    /// Slide toward row 3.
    Down = 3,
}

impl Direction {
    /// Every direction, in the order used by [`Board::shift_all`].
    ///
    /// [`Board::shift_all`]: crate::Board::shift_all
    pub const ALL: [Direction; 4] = [Self::Left, Self::Right, Self::Up, Self::Down];

    /// Position of this direction in [`Direction::ALL`].
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        })
    }
}

/// A set of directions, packed into four bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DirSet(u8);

impl DirSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);
    /// All four directions.
    pub const ALL: Self = Self(0b1111);

    #[inline(always)]
    pub(crate) const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether `dir` is in the set.
    #[inline(always)]
    pub const fn contains(self, dir: Direction) -> bool {
        self.0 & (1 << dir as u8) != 0
    }

    /// Adds `dir` to the set.
    #[inline(always)]
    pub const fn insert(&mut self, dir: Direction) {
        self.0 |= 1 << dir as u8;
    }

    /// Whether the set is empty. For a board, this means the game is over.
    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of directions in the set.
    #[inline(always)]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// The first direction in the set, in [`Direction::ALL`] order.
    #[inline(always)]
    pub fn first(self) -> Option<Direction> {
        self.into_iter().next()
    }
}

impl IntoIterator for DirSet {
    type Item = Direction;
    type IntoIter = DirSetIter;

    fn into_iter(self) -> DirSetIter {
        DirSetIter(self.0)
    }
}

impl FromIterator<Direction> for DirSet {
    fn from_iter<I: IntoIterator<Item = Direction>>(iter: I) -> Self {
        let mut set = Self::EMPTY;
        for dir in iter {
            set.insert(dir);
        }
        set
    }
}

impl fmt::Debug for DirSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(*self).finish()
    }
}

/// Iterator over the directions in a [`DirSet`].
#[derive(Clone, Debug)]
pub struct DirSetIter(u8);

impl Iterator for DirSetIter {
    type Item = Direction;

    #[inline]
    fn next(&mut self) -> Option<Direction> {
        // An exhausted set has `trailing_zeros() == 8`, which `get` turns into
        // the `None` that ends the iteration, so there is no separate branch
        // for it and no index that could be out of range.
        let i = self.0.trailing_zeros() as usize;
        self.0 &= self.0.wrapping_sub(1);
        Direction::ALL.get(i).copied()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.0.count_ones() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for DirSetIter {}
impl core::iter::FusedIterator for DirSetIter {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_round_trips_through_iteration() {
        for bits in 0u8..16 {
            let set = DirSet::from_bits(bits);
            let collected: DirSet = set.into_iter().collect();
            assert_eq!(set, collected);
            assert_eq!(set.len(), bits.count_ones() as usize);
        }
    }

    #[test]
    fn iteration_order_matches_all() {
        let set: DirSet = Direction::ALL.into_iter().collect();
        let seen: Vec<_> = set.into_iter().collect();
        assert_eq!(seen, Direction::ALL);
        assert_eq!(set, DirSet::ALL);
    }

    #[test]
    fn membership_agrees_with_insertion() {
        for dir in Direction::ALL {
            let mut set = DirSet::EMPTY;
            assert!(!set.contains(dir));
            set.insert(dir);
            assert!(set.contains(dir));
            assert_eq!(set.first(), Some(dir));
        }
    }
}
