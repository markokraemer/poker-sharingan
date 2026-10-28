//! Lock-free, fixed-capacity infoset table for the search solver.
//!
//! Same scheme as `blueprint::store` (open addressing, CAS key insert, hogwild relaxed
//! atomics) but with `f32` regrets so that Linear-CFR iteration weights `t` can multiply the
//! updates directly, plus an average-strategy accumulator for every infoset (search needs the
//! *average* strategy for belief updates, supplementary "Structure of imperfect-information
//! subgames"). Allocation is lazy; when the table or arena is full `get_or_insert` returns
//! `None` and the solver treats that infoset as uniform and unlearned.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    off: usize,
    pub n: usize,
}

fn zeroed_u64(n: usize) -> Vec<AtomicU64> {
    let mut v = std::mem::ManuallyDrop::new(vec![0u64; n]);
    // SAFETY: AtomicU64 has the same size and alignment as u64; all-zero is a valid value.
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU64, v.len(), v.capacity()) }
}

fn zeroed_u32(n: usize) -> Vec<AtomicU32> {
    let mut v = std::mem::ManuallyDrop::new(vec![0u32; n]);
    // SAFETY: as above for AtomicU32.
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU32, v.len(), v.capacity()) }
}

pub struct Table {
    keys: Vec<AtomicU64>,
    vals: Vec<AtomicU64>,
    arena: Vec<AtomicU32>,
    next: AtomicUsize,
    count: AtomicUsize,
    cap_infosets: usize,
}

const READY: u64 = 1 << 62;

#[inline(always)]
fn ld(x: &AtomicU32) -> f32 {
    f32::from_bits(x.load(Relaxed))
}
#[inline(always)]
fn st(x: &AtomicU32, v: f32) {
    x.store(v.to_bits(), Relaxed)
}

impl Table {
    /// Room for `max_infosets` infosets (index at <= 70% load) and `arena_slots` f32 cells
    /// (two per action per infoset). Memory is zero-allocated and committed on first touch.
    pub fn new(max_infosets: usize, arena_slots: usize) -> Table {
        let len = (max_infosets * 10 / 7).max(16);
        Table {
            keys: zeroed_u64(len),
            vals: zeroed_u64(len),
            arena: zeroed_u32(arena_slots),
            next: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            cap_infosets: max_infosets,
        }
    }

    #[inline(always)]
    fn home(&self, key: u64) -> usize {
        ((key as u128 * self.keys.len() as u128) >> 64) as usize
    }
    #[inline(always)]
    fn nxt(&self, i: usize) -> usize {
        if i + 1 == self.keys.len() {
            0
        } else {
            i + 1
        }
    }
    #[inline]
    fn ready(&self, i: usize) -> Slot {
        loop {
            let v = self.vals[i].load(Acquire);
            if v != 0 {
                return Slot {
                    off: ((v & (READY - 1)) >> 8) as usize,
                    n: (v & 0xFF) as usize,
                };
            }
            std::hint::spin_loop();
        }
    }

    /// Mix a path hash and a card tag into a non-zero key.
    #[inline(always)]
    pub fn key(path: u64, tag: u64) -> u64 {
        let mut z = path ^ tag.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) | 1
    }

    pub fn find(&self, key: u64) -> Option<Slot> {
        let mut i = self.home(key);
        loop {
            let k = self.keys[i].load(Acquire);
            if k == key {
                return Some(self.ready(i));
            }
            if k == 0 {
                return None;
            }
            i = self.nxt(i);
        }
    }

    pub fn get_or_insert(&self, key: u64, n: usize) -> Option<Slot> {
        self.get_or_insert_with(key, n, |_| {})
    }

    /// Like `get_or_insert`; a newly created infoset is initialised by `init(regrets)` *before*
    /// it becomes visible to other threads (warm start).
    pub fn get_or_insert_with(&self, key: u64, n: usize, init: impl FnOnce(&mut [f32]) -> ()) -> Option<Slot> {
        let mut init = Some(init);
        let mut i = self.home(key);
        loop {
            let k = self.keys[i].load(Acquire);
            if k == key {
                let s = self.ready(i);
                return (s.n == n).then_some(s);
            }
            if k == 0 {
                if self.count.load(Relaxed) >= self.cap_infosets {
                    return None;
                }
                let off = self.next.fetch_add(2 * n, Relaxed);
                if off + 2 * n > self.arena.len() {
                    return None;
                }
                match self.keys[i].compare_exchange(0, key, AcqRel, Acquire) {
                    Ok(_) => {
                        self.count.fetch_add(1, Relaxed);
                        let slot = Slot { off, n };
                        let mut r = [0f32; 32];
                        (init.take().unwrap())(&mut r[..n]);
                        for (c, &x) in self.reg(slot).iter().zip(&r[..n]) {
                            st(c, x);
                        }
                        self.vals[i].store(READY | ((off as u64) << 8) | n as u64, Release);
                        return Some(slot);
                    }
                    Err(o) if o == key => {
                        let s = self.ready(i);
                        return (s.n == n).then_some(s);
                    }
                    Err(_) => {}
                }
            }
            i = self.nxt(i);
        }
    }

    #[inline(always)]
    fn reg(&self, s: Slot) -> &[AtomicU32] {
        &self.arena[s.off..s.off + s.n]
    }
    #[inline(always)]
    fn avg(&self, s: Slot) -> &[AtomicU32] {
        &self.arena[s.off + s.n..s.off + 2 * s.n]
    }

    /// Regret matching on the current regrets (the "final iterate" strategy).
    #[inline]
    pub fn strategy(&self, s: Slot, out: &mut [f32]) {
        let mut sum = 0f32;
        for (o, x) in out.iter_mut().zip(self.reg(s)) {
            let v = ld(x).max(0.0);
            *o = v;
            sum += v;
        }
        if sum > 0.0 {
            out.iter_mut().for_each(|o| *o /= sum);
        } else {
            out.iter_mut().for_each(|o| *o = 1.0 / s.n as f32);
        }
    }

    /// Normalised weighted average strategy; `false` (and `out` untouched) if nothing was
    /// accumulated.
    pub fn average(&self, s: Slot, out: &mut [f32]) -> bool {
        let tot: f32 = self.avg(s).iter().map(ld).sum();
        if tot > 0.0 {
            for (o, x) in out.iter_mut().zip(self.avg(s)) {
                *o = ld(x) / tot;
            }
            true
        } else {
            false
        }
    }

    /// Pin the infoset to a pure strategy (frozen actions): regret 1 on `a`, 0 elsewhere.
    pub fn set_pure(&self, s: Slot, a: usize) {
        for (i, c) in self.reg(s).iter().enumerate() {
            st(c, (i == a) as u32 as f32);
        }
    }

    #[inline(always)]
    pub fn add_regret(&self, s: Slot, a: usize, d: f32) {
        let c = &self.reg(s)[a];
        st(c, ld(c) + d);
    }
    #[inline(always)]
    pub fn add_avg(&self, s: Slot, a: usize, w: f32) {
        let c = &self.avg(s)[a];
        st(c, ld(c) + w);
    }

    #[inline(always)]
    pub fn regret(&self, s: Slot, a: usize) -> f32 {
        ld(&self.reg(s)[a])
    }

    pub fn infosets(&self) -> usize {
        self.count.load(Relaxed)
    }
    pub fn arena_used(&self) -> usize {
        self.next.load(Relaxed).min(self.arena.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_find_update() {
        let t = Table::new(100, 1000);
        let k = Table::key(5, 7);
        let s = t.get_or_insert(k, 3).unwrap();
        assert_eq!(t.get_or_insert(k, 3), Some(s));
        assert_eq!(t.find(k), Some(s));
        assert!(t.find(Table::key(5, 8)).is_none());
        let mut p = [0f32; 3];
        t.strategy(s, &mut p);
        assert_eq!(p, [1.0 / 3.0; 3]);
        t.add_regret(s, 0, 3.0);
        t.add_regret(s, 1, 1.0);
        t.add_regret(s, 2, -4.0);
        t.strategy(s, &mut p);
        assert_eq!(p, [0.75, 0.25, 0.0]);
        assert!(!t.average(s, &mut p));
        t.add_avg(s, 2, 2.0);
        assert!(t.average(s, &mut p));
        assert_eq!(p, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn full_table_returns_none() {
        let t = Table::new(4, 1000);
        let mut ok = 0;
        for i in 0..20 {
            ok += t.get_or_insert(Table::key(i, 0), 2).is_some() as usize;
        }
        assert_eq!(ok, 4);
    }
}
