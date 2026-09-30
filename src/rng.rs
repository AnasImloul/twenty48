//! A small seedable PRNG.
//!
//! The engine needs a stream of `u64`s and nothing else, so it carries its own
//! generator rather than depending on `rand`. That keeps the value stream
//! frozen across versions, which is what makes a seeded playout comparison
//! reproducible months later, and lets [`Rng::next_u64`] inline into the
//! spawn site without link-time optimisation.

/// A source of random `u64`s.
///
/// Implement this to drive the engine from your own generator:
///
/// ```
/// # struct Wyrand(u64);
/// # impl Wyrand { fn sample(&mut self) -> u64 { 0 } }
/// impl twenty48::Rng for Wyrand {
///     fn next_u64(&mut self) -> u64 {
///         self.sample()
///     }
/// }
/// ```
pub trait Rng {
    /// Returns the next value in the stream.
    fn next_u64(&mut self) -> u64;
}

impl<R: Rng + ?Sized> Rng for &mut R {
    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        (**self).next_u64()
    }
}

/// xoshiro256++, seeded through `SplitMix64`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Xoshiro256pp {
    state: [u64; 4],
}

impl Xoshiro256pp {
    /// Builds a generator from a seed.
    ///
    /// Any seed works, including zero: `SplitMix64` expands it into the four
    /// words of state, which both avoids the forbidden all-zero state and
    /// decorrelates adjacent seeds.
    pub const fn seed_from_u64(seed: u64) -> Self {
        let (seed, a) = splitmix64(seed);
        let (seed, b) = splitmix64(seed);
        let (seed, c) = splitmix64(seed);
        let (_, d) = splitmix64(seed);
        Self {
            state: [a, b, c, d],
        }
    }
}

impl Rng for Xoshiro256pp {
    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);

        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);

        result
    }
}

/// One `SplitMix64` step: returns the advanced state and the output it produces.
const fn splitmix64(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (state, z ^ (z >> 31))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        let mut a = Xoshiro256pp::seed_from_u64(12345);
        let mut b = Xoshiro256pp::seed_from_u64(12345);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_eq!(a, b);
    }

    #[test]
    fn a_zero_seed_does_not_produce_a_zero_state() {
        let mut rng = Xoshiro256pp::seed_from_u64(0);
        assert_ne!(rng.state, [0; 4]);
        assert!((0..100).any(|_| rng.next_u64() != 0));
    }

    #[test]
    fn adjacent_seeds_diverge_immediately() {
        let mut a = Xoshiro256pp::seed_from_u64(0);
        let mut b = Xoshiro256pp::seed_from_u64(1);
        let first: Vec<_> = (0..8).map(|_| a.next_u64()).collect();
        let second: Vec<_> = (0..8).map(|_| b.next_u64()).collect();
        assert!(first.iter().zip(&second).all(|(x, y)| x != y));
    }

    /// Not a randomness test, just a check that the low bits are not stuck.
    #[test]
    fn bits_are_roughly_balanced() {
        let mut rng = Xoshiro256pp::seed_from_u64(0xDECAF);
        let mut ones = 0u32;
        let draws = 20_000;
        for _ in 0..draws {
            ones += rng.next_u64().count_ones();
        }
        let ratio = f64::from(ones) / f64::from(draws * 64);
        assert!((ratio - 0.5).abs() < 0.01, "set-bit ratio was {ratio}");
    }
}
