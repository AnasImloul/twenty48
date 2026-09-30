use std::sync::atomic::{AtomicU64, Ordering};

/// Direct-mapped transposition table, shared by every thread of a search.
///
/// A `HashMap` spends more time hashing and probing than the eight lookups an
/// evaluation costs, which is the wrong trade for a cache that is allowed to
/// miss. One multiply picks a slot, a mismatched key is simply a miss, and
/// there is no bookkeeping to clear between moves.
///
/// Entries are scoped to one move. A stored value depends on the probability
/// at which its node happened to be expanded, because a node reached on an
/// unlikely path has its children cut off by the floor, so the value is only
/// meaningful inside the search that produced it. Carrying entries across
/// moves costs about a third fewer nodes per move and is tempting for that
/// reason, but it hands later searches values computed under a floor they did
/// not use. The stamp scopes entries to a move without walking several MiB
/// between them.
///
/// The table is not only a cache. A stored value satisfies any request for
/// the same board at its depth *or shallower*, so a hit often returns a
/// better answer than the caller would have computed. Splitting the search
/// across threads without sharing this measures three times the nodes and
/// plays five times worse, which is why the slots are atomic rather than the
/// table being cloned per thread.
///
/// Sharing is lockless. A writer stores the payload and, beside it, the key
/// mixed into that payload; a reader that recovers the key it asked for knows
/// it read a matched pair. Two writers racing on one slot leave a mismatch,
/// which reads as a miss, so a race costs a recomputation and never a wrong
/// answer. The alternative, a lock per slot, would put a read-modify-write on
/// the hottest path in the program to save work that is cheap to redo.
pub struct Table {
    slots: Box<[Slot]>,
    /// Bumped once per move, and folded into the checksum rather than stored
    /// beside it, so an entry left over from an earlier move fails the same
    /// comparison a wrong key does.
    stamp: AtomicU64,
}

#[derive(Default)]
struct Slot {
    check: AtomicU64,
    data: AtomicU64,
}

impl Table {
    /// 256K slots is 4 MiB, enough to hold a single-threaded search and small
    /// enough that the twelve of them in a batch of games stay out of each
    /// other's way. A split search puts several threads through one table, so
    /// it gets a bigger one.
    const SLOTS: usize = 1 << 18;

    pub fn new(threads: usize) -> Table {
        let slots = Table::SLOTS * threads.next_power_of_two();
        Table {
            slots: (0..slots).map(|_| Slot::default()).collect(),
            // Slots start zeroed, which reads back as key zero, so the first
            // move must not use the salt that would make that a hit.
            stamp: AtomicU64::new(1),
        }
    }

    pub fn clear(&self) {
        self.stamp.fetch_add(1, Ordering::Relaxed);
    }

    /// What the current move's keys are mixed with. An odd multiplier, so
    /// distinct moves never share a salt.
    fn salt(&self) -> u64 {
        self.stamp
            .load(Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    fn slot(&self, key: u64) -> &Slot {
        let mut x = key;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 29;
        &self.slots[(x as usize) & (self.slots.len() - 1)]
    }

    pub fn get(&self, key: u64, depth: u32) -> Option<f32> {
        let slot = self.slot(key);
        let data = slot.data.load(Ordering::Relaxed);
        let check = slot.check.load(Ordering::Relaxed);

        let usable = check ^ data ^ self.salt() == key && data as u32 >= depth;
        usable.then(|| f32::from_bits((data >> 32) as u32))
    }

    pub fn insert(&self, key: u64, depth: u32, value: f32) {
        let slot = self.slot(key);
        let salt = self.salt();

        // Prefer the deeper result when two boards collide, since it cost more
        // to produce and is the better answer if it is ever asked for again.
        let old = slot.data.load(Ordering::Relaxed);
        if slot.check.load(Ordering::Relaxed) ^ old ^ salt == key && depth < old as u32 {
            return;
        }

        let data = (u64::from(value.to_bits()) << 32) | u64::from(depth);
        slot.check.store(key ^ data ^ salt, Ordering::Relaxed);
        slot.data.store(data, Ordering::Relaxed);
    }
}
