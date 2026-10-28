//! Lazily allocated, lock-free infoset storage for multi-threaded MCCFR.
//!
//! Mirrors Pluribus' memory scheme: regrets are 4-byte integers floored at `REGRET_FLOOR` and
//! allocated only when first encountered. Allocation is per infoset (betting history × card
//! bucket): in six-player poker each history is reached with only a few of its buckets, so this
//! is ~10× leaner than allocating every bucket of a history at once (measured).
//!
//! The index is a fixed-capacity open-addressing hash table of (key, packed slot) atomics —
//! 16 bytes per entry, inserts via CAS, no locks. Threads update regrets "hogwild" with relaxed
//! atomics, as in the paper's 64-core run.

use rayon::prelude::*;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering::*};

pub const REGRET_FLOOR: i32 = -310_000_000;

/// One infoset: `n_actions` regrets (+ `n_actions` φ counters when `with_avg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub offset: u64,
    pub n_actions: u8,
    pub with_avg: bool,
}

impl Slot {
    #[inline(always)]
    fn pack(self) -> u64 {
        // offset: 46 bits, n_actions: 8 bits, with_avg: 1 bit, bit 63 = "ready" marker.
        (1 << 63) | ((self.with_avg as u64) << 62) | ((self.n_actions as u64) << 46) | self.offset
    }
    #[inline(always)]
    fn unpack(v: u64) -> Slot {
        Slot { offset: v & ((1 << 46) - 1), n_actions: ((v >> 46) & 0xFF) as u8, with_avg: (v >> 62) & 1 == 1 }
    }
    #[inline(always)]
    pub fn len(&self) -> u64 {
        self.n_actions as u64 * if self.with_avg { 2 } else { 1 }
    }
}

/// Mix (history key, bucket) into a non-zero table key.
#[inline(always)]
pub fn infoset_key(history: u64, bucket: u32) -> u64 {
    let mut z = history ^ (bucket as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

fn zeroed<T>(n: usize) -> Vec<T> {
    // Zeroed allocation: pages are committed only when first touched.
    let bytes = n * std::mem::size_of::<T>();
    let mut v = std::mem::ManuallyDrop::new(vec![0u8; bytes]);
    debug_assert_eq!(v.as_ptr() as usize % std::mem::align_of::<T>(), 0);
    advise_huge_pages(v.as_mut_ptr(), bytes);
    // SAFETY: only used for atomic integer types, for which all-zero bytes are valid.
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut T, n, v.capacity() / std::mem::size_of::<T>()) }
}

/// Ask Linux for transparent huge pages on big tables: with hundreds of GB of randomly
/// accessed memory, 4 KB pages make nearly every lookup a TLB miss.
#[cfg(target_os = "linux")]
fn advise_huge_pages(ptr: *mut u8, len: usize) {
    extern "C" {
        fn madvise(addr: *mut std::ffi::c_void, len: usize, advice: i32) -> i32;
    }
    const MADV_HUGEPAGE: i32 = 14;
    const HUGE: usize = 2 << 20;
    if len < 4 * HUGE {
        return;
    }
    let start = (ptr as usize + HUGE - 1) & !(HUGE - 1);
    let end = (ptr as usize + len) & !(HUGE - 1);
    if end > start {
        // SAFETY: the range lies inside our own live allocation.
        unsafe { madvise(start as *mut std::ffi::c_void, end - start, MADV_HUGEPAGE) };
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_huge_pages(_: *mut u8, _: usize) {}

pub struct Store {
    keys: Vec<AtomicU64>,
    vals: Vec<AtomicU64>,
    count: AtomicU64,
    full: std::sync::atomic::AtomicBool,
    arena: Vec<AtomicI32>,
    next: AtomicU64,
}

impl Store {
    /// `capacity` = i32 regret slots to reserve; the index gets room for `capacity / 2`
    /// infosets.
    pub fn new(capacity: usize) -> Self {
        Store::with_index(capacity, (capacity / 2).max(16))
    }

    /// Index sized for `max_infosets` at ≤ 70% load (any size — no power-of-two rounding,
    /// which would waste up to half the RAM on a large box).
    pub fn with_index(capacity: usize, max_infosets: usize) -> Self {
        let table = (max_infosets * 10 / 7).max(16);
        Store { keys: zeroed(table), vals: zeroed(table), count: AtomicU64::new(0), full: Default::default(), arena: zeroed(capacity), next: AtomicU64::new(0) }
    }

    /// Home bucket of `key` (Lemire's multiply-shift range reduction).
    #[inline(always)]
    fn home(&self, key: u64) -> usize {
        ((key as u128 * self.keys.len() as u128) >> 64) as usize
    }

    #[inline(always)]
    fn probe_next(&self, i: usize) -> usize {
        if i + 1 == self.keys.len() {
            0
        } else {
            i + 1
        }
    }

    #[inline]
    fn find(&self, key: u64) -> Option<Slot> {
        let mut i = self.home(key);
        loop {
            let k = self.keys[i].load(Acquire);
            if k == key {
                return Some(self.wait_ready(i));
            }
            if k == 0 {
                return None;
            }
            i = self.probe_next(i);
        }
    }

    #[inline]
    fn wait_ready(&self, i: usize) -> Slot {
        loop {
            let v = self.vals[i].load(Acquire);
            if v != 0 {
                return Slot::unpack(v);
            }
            std::hint::spin_loop();
        }
    }

    /// The infoset (`history`, `bucket`), if allocated.
    #[inline]
    pub fn lookup(&self, history: u64, bucket: u32) -> Option<Slot> {
        self.find(infoset_key(history, bucket))
    }

    /// Find or allocate infoset (`history`, `bucket`); panics if the store is full.
    /// `_n_buckets` is accepted for API stability (allocation is per infoset).
    #[inline]
    pub fn infoset(&self, history: u64, bucket: u32, n_actions: usize, n_buckets: usize, with_avg: bool) -> Slot {
        self.try_infoset(history, bucket, n_actions, n_buckets, with_avg).expect("infoset store full; raise --mem-gb")
    }

    /// Like `infoset`, but returns `None` instead of allocating once the index is 80% full
    /// or the regret arena is exhausted (training then treats the infoset as unvisited).
    #[inline]
    pub fn try_infoset(&self, history: u64, bucket: u32, n_actions: usize, _n_buckets: usize, with_avg: bool) -> Option<Slot> {
        let key = infoset_key(history, bucket);
        let mut i = self.home(key);
        loop {
            let k = self.keys[i].load(Acquire);
            if k == key {
                let s = self.wait_ready(i);
                debug_assert_eq!(s.n_actions as usize, n_actions, "infoset key collision or action-count mismatch");
                return Some(s);
            }
            if k == 0 {
                let len = n_actions as u64 * if with_avg { 2 } else { 1 };
                if self.count.load(Relaxed) as usize >= self.keys.len() * 17 / 20 || self.next.load(Relaxed) + len > self.arena.len() as u64 {
                    self.full.store(true, Relaxed);
                    return None;
                }
                match self.keys[i].compare_exchange(0, key, AcqRel, Acquire) {
                    Ok(_) => {
                        self.count.fetch_add(1, Relaxed);
                        let offset = self.next.fetch_add(len, Relaxed);
                        assert!(offset + len <= self.arena.len() as u64, "regret arena overrun");
                        let s = Slot { offset, n_actions: n_actions as u8, with_avg };
                        self.vals[i].store(s.pack(), Release);
                        return Some(s);
                    }
                    Err(other) if other == key => return Some(self.wait_ready(i)),
                    Err(_) => {}
                }
            }
            i = self.probe_next(i);
        }
    }

    /// True once an allocation has been refused because the store is full.
    pub fn is_full(&self) -> bool {
        self.full.load(Relaxed)
    }

    #[inline(always)]
    pub fn regrets(&self, s: Slot) -> &[AtomicI32] {
        &self.arena[s.offset as usize..s.offset as usize + s.n_actions as usize]
    }

    #[inline(always)]
    pub fn avg(&self, s: Slot) -> &[AtomicI32] {
        debug_assert!(s.with_avg);
        let o = s.offset as usize + s.n_actions as usize;
        &self.arena[o..o + s.n_actions as usize]
    }

    /// Regret matching on the current regrets (Algorithm 1, CALCULATE-STRATEGY).
    #[inline]
    pub fn strategy(&self, s: Slot, out: &mut [f32]) {
        let mut sum = 0f32;
        for (o, x) in out.iter_mut().zip(self.regrets(s)) {
            let v = x.load(Relaxed).max(0) as f32;
            *o = v;
            sum += v;
        }
        if sum > 0.0 {
            out.iter_mut().for_each(|o| *o /= sum);
        } else {
            out.iter_mut().for_each(|o| *o = 1.0 / s.n_actions as f32);
        }
    }

    /// Normalised average strategy φ (first betting round); falls back to the current
    /// strategy when no counts were accumulated.
    pub fn average(&self, s: Slot, out: &mut [f32]) {
        if s.with_avg {
            let total: i64 = self.avg(s).iter().map(|x| x.load(Relaxed) as i64).sum();
            if total > 0 {
                for (o, x) in out.iter_mut().zip(self.avg(s)) {
                    *o = x.load(Relaxed) as f32 / total as f32;
                }
                return;
            }
        }
        self.strategy(s, out)
    }

    /// Add a (rounded) regret delta, saturating at the floor and at i32::MAX.
    #[inline(always)]
    pub fn add_regret(&self, s: Slot, a: usize, delta: f32) {
        let cell = &self.regrets(s)[a];
        let next = (cell.load(Relaxed) as i64 + delta.round() as i64).clamp(REGRET_FLOOR as i64, i32::MAX as i64);
        cell.store(next as i32, Relaxed);
    }

    #[inline(always)]
    pub fn bump_avg(&self, s: Slot, a: usize) {
        let cell = &self.avg(s)[a];
        if cell.load(Relaxed) < i32::MAX {
            cell.fetch_add(1, Relaxed);
        }
    }

    /// Linear-CFR discount: multiply every regret and average counter by `d` (Pluribus applies
    /// this every 10 min for the first 400 min with d = (t/10)/(t/10+1)).
    pub fn discount(&self, d: f64) {
        let used = self.next.load(Relaxed) as usize;
        self.arena[..used].par_iter().for_each(|x| x.store((x.load(Relaxed) as f64 * d) as i32, Relaxed));
    }

    pub fn infosets(&self) -> usize {
        self.count.load(Relaxed) as usize
    }

    /// Alias of `infosets` (allocation is per infoset).
    pub fn sequences(&self) -> usize {
        self.infosets()
    }

    pub fn used_slots(&self) -> u64 {
        self.next.load(Relaxed)
    }

    pub fn capacity(&self) -> usize {
        self.arena.len()
    }

    pub fn index_capacity(&self) -> usize {
        self.keys.len()
    }

    /// Bytes of address space actually in use (index + used regret slots).
    pub fn bytes_used(&self) -> u64 {
        self.keys.len() as u64 * 16 + self.used_slots() * 4
    }

    /// Index table length (occupied + empty entries).
    pub fn table_len(&self) -> usize {
        self.keys.len()
    }

    /// Parallel scan of index entries `[a, b)`: calls `f(key, slot)` for each occupied one.
    pub fn par_scan(&self, a: usize, b: usize, f: impl Fn(u64, Slot) + Sync + Send) {
        (a..b.min(self.keys.len())).into_par_iter().with_min_len(1 << 14).for_each(|i| {
            let k = self.keys[i].load(Acquire);
            if k != 0 {
                f(k, self.wait_ready(i));
            }
        });
    }

    /// The occupied entry at index-table position `i`, if any.
    #[inline]
    pub fn entry_at(&self, i: usize) -> Option<(u64, Slot)> {
        let k = self.keys[i].load(Acquire);
        (k != 0).then(|| (k, self.wait_ready(i)))
    }

    /// Insert a known (key, slot) without allocating (used when loading checkpoints).
    pub fn insert_raw(&self, key: u64, s: Slot) {
        let mut i = self.home(key);
        loop {
            match self.keys[i].compare_exchange(0, key, AcqRel, Acquire) {
                Ok(_) => {
                    self.vals[i].store(s.pack(), Release);
                    self.count.fetch_add(1, Relaxed);
                    return;
                }
                Err(k) if k == key => return,
                Err(_) => i = self.probe_next(i),
            }
        }
    }

    /// Set the allocation cursor and slot values `[0, values.len())` (checkpoint loading).
    pub fn set_arena_prefix(&self, start: usize, values: &[i32]) {
        self.arena[start..start + values.len()].par_iter().zip(values).for_each(|(a, &v)| a.store(v, Relaxed));
        self.next.fetch_max((start + values.len()) as u64, Relaxed);
    }

    /// Every allocated infoset (table key, slot), in arena order.
    pub fn entries(&self) -> Vec<(u64, Slot)> {
        let mut v: Vec<(u64, Slot)> = (0..self.keys.len())
            .into_par_iter()
            .filter_map(|i| {
                let k = self.keys[i].load(Acquire);
                (k != 0).then(|| (k, self.wait_ready(i)))
            })
            .collect();
        v.par_sort_unstable_by_key(|e| e.1.offset);
        v
    }

    pub fn raw(&self, offset: u64, len: usize) -> &[AtomicI32] {
        &self.arena[offset as usize..offset as usize + len]
    }

    /// Rebuild a store from serialized parts (see `io`). `entries` hold table keys.
    pub fn from_parts(capacity: usize, max_infosets: usize, entries: &[(u64, Slot)], values: &[i32]) -> Self {
        let st = Store::with_index(capacity.max(values.len()), max_infosets.max(entries.len() * 5 / 4));
        st.arena[..values.len()].par_iter().zip(values).for_each(|(a, &v)| a.store(v, Relaxed));
        st.next.store(values.len() as u64, Relaxed);
        for &(key, s) in entries {
            let mut i = st.home(key);
            while st.keys[i].load(Relaxed) != 0 {
                i = st.probe_next(i);
            }
            st.keys[i].store(key, Relaxed);
            st.vals[i].store(s.pack(), Relaxed);
        }
        st.count.store(entries.len() as u64, Relaxed);
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_regret_matching() {
        let st = Store::new(10_000);
        let s = st.infoset(42, 3, 3, 10, true);
        assert_eq!(st.infoset(42, 3, 3, 10, true), s);
        assert_eq!(st.used_slots(), 6);
        assert_eq!(st.infoset(42, 4, 3, 10, true).offset, 6);
        let mut p = [0f32; 3];
        st.strategy(s, &mut p);
        assert_eq!(p, [1.0 / 3.0; 3]);
        st.add_regret(s, 0, 30.0);
        st.add_regret(s, 1, 10.0);
        st.add_regret(s, 2, -5.0);
        st.strategy(s, &mut p);
        assert_eq!(p, [0.75, 0.25, 0.0]);
        st.add_regret(s, 2, -1e12);
        assert_eq!(st.regrets(s)[2].load(Relaxed), REGRET_FLOOR);
        st.bump_avg(s, 1);
        assert_eq!(st.avg(s)[1].load(Relaxed), 1);
        st.average(s, &mut p);
        assert_eq!(p, [0.0, 1.0, 0.0]);
        st.discount(0.5);
        assert_eq!(st.regrets(s)[0].load(Relaxed), 15);
        assert!(st.lookup(42, 3).is_some() && st.lookup(43, 0).is_none());
        assert_eq!(Slot::unpack(s.pack()), s);
    }

    #[test]
    fn concurrent_inserts_are_unique() {
        let st = Store::new(1 << 20);
        (0..64u64).into_par_iter().for_each(|t| {
            for k in 0..2000u64 {
                st.infoset(k, (t % 4) as u32, 2, 4, false);
            }
        });
        assert_eq!(st.infosets(), 8000);
        assert_eq!(st.used_slots(), 16000);
        let mut offs: Vec<u64> = st.entries().iter().map(|e| e.1.offset).collect();
        offs.dedup();
        assert_eq!(offs.len(), 8000);
    }
}
