//! Compact final-policy file (`PLCP1`): the blueprint's *final* action probabilities, one u8
//! distribution per infoset, built by streaming a PLBP4 checkpoint (+ optional PLSA1 snapshot
//! sums) without ever loading the store into RAM.
//!
//! # Final-policy semantics (identical to `snapshot::export_in_place` + `Store::strategy`)
//! * first-round infosets (`with_avg`): normalised φ counters; if they are all zero, the
//!   current regret-matching strategy;
//! * other infosets covered by a snapshot sum (some snapshot taken after the infoset was
//!   allocated and the sums are not all zero): normalised snapshot sums;
//! * otherwise: the current regret-matching strategy (uniform if no positive regret).
//!
//! # File layout (little endian, every section starts on a 4096-byte boundary)
//! ```text
//! header (4096 B) magic "PLCP1\0\0\0", n, dir_bits D, blob_len, section offsets, source stats
//! dir   (2^D + 1) × u64   dir[b] = index of the first key whose top D bits are >= b
//! keys  n × u64           infoset_key(history, bucket), strictly ascending
//! na    n × u8            number of actions of entry i
//! base  ceil(n/64) × u64  blob offset of entry 64·j
//! blob  Σ na bytes        per entry: na probabilities as u8 (value/255), summing to 255
//! ```
//! Cost: 8 (key) + 1 (na) + 1/8 (base) + ~0.3..1 (dir, ≈ 32 entries per bucket) ≈ 9.5 bytes
//! per infoset + 1 byte per action. Lookup = directory slot → binary search of ~32 keys →
//! sum of ≤ 63 `na` bytes (one cache line) → probabilities.
//!
//! # Quantisation
//! u8 with largest-remainder rounding so each distribution sums to exactly 255: every
//! probability is within 1/255 (0.39 %) of the exact one (typically ≤ 0.2 %), an action never
//! has a non-zero probability turned into more than +0.39 %. u16 would double the blob
//! (≈ +2.5 B/infoset, +25 %) to cut an error that is far below the sampling noise of any
//! match we can run (mbb effect of a 0.2 % mis-weighted action is ≪ 1 mbb), so u8.
//!
//! # Streaming build (memory-bounded, sequential IO)
//! The checkpoint is memory-mapped and never loaded. Records (key, arena offset, n, φ-flag)
//! are in hash-table order, i.e. *key* order, but each record's data lives at a random place
//! of the arena (allocation order), so a key-ordered pass would do one random page read per
//! infoset (tens of TB of IO at paper scale). Instead:
//! 1. **pass 1, arena order**: the arena (+ snapshot sums) is split into groups of
//!    `group_slots`; for each group the group's byte range is prefetched sequentially into the
//!    page cache and *all* records are scanned (sequential), processing the ones whose offset
//!    falls in the group; each result `[key][n][n prob bytes]` is appended to one of `2^pb`
//!    temporary key-partition files (top `pb` bits of the key; ~0.5 GB each);
//! 2. **pass 2, key order**: each partition (in parallel) is read, sorted by key and written
//!    at its exact final position of every section (positions follow from the per-partition
//!    counts), including its slice of the directory and base array.
//! Peak RSS ≈ `threads x ~2.5 GB` in pass 2, a few hundred MB in pass 1; the page-cache working
//! set of a pass is `group_slots x 6 B`. Temp disk ≈ output size. The input order is irrelevant
//! (no assumption on how the checkpoint writer ordered its records).
//!
use crate::store::infoset_key;
use rayon::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::time::Instant;

const MAGIC: &[u8; 8] = b"PLCP1\0\0\0";
const PAGE: u64 = 4096;
const CK_MAGIC: &[u8; 8] = b"PLBP4\0\0\0";
const CK_REC: usize = 18;
const CK_HDR: usize = 24;
const SUM_MAGIC: &[u8; 8] = b"PLSA1\0\0\0";
/// Largest action count handled (nlhe::abstraction::MAX_ACTIONS is 24).
const MAX_N: usize = 64;

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

// ---------------------------------------------------------------------------------------
// mmap (std only; no extra crates)
// ---------------------------------------------------------------------------------------

extern "C" {
    fn mmap(addr: *mut std::ffi::c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut std::ffi::c_void;
    fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
    fn madvise(addr: *mut std::ffi::c_void, len: usize, advice: i32) -> i32;
}

#[derive(Clone, Copy, Debug)]
pub enum Advice {
    Random = 1,
    Sequential = 2,
    WillNeed = 3,
}

/// Read-only private mapping of a whole file.
pub struct Mmap {
    ptr: *const u8,
    len: usize,
}
// SAFETY: the mapping is read-only and never mutated through this type.
unsafe impl Send for Mmap {}
unsafe impl Sync for Mmap {}

impl Mmap {
    pub fn open(path: &Path) -> io::Result<Mmap> {
        let f = File::open(path)?;
        let len = f.metadata()?.len() as usize;
        if len == 0 {
            return Ok(Mmap { ptr: std::ptr::NonNull::<u64>::dangling().as_ptr() as *const u8, len: 0 });
        }
        // PROT_READ = 1, MAP_PRIVATE = 2 on Linux and macOS.
        // SAFETY: valid fd, length from metadata; failure is checked.
        let p = unsafe { mmap(std::ptr::null_mut(), len, 1, 2, f.as_raw_fd(), 0) };
        if p as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Mmap { ptr: p as *const u8, len })
    }
    #[inline(always)]
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: ptr/len describe a live mapping (or an empty dangling slice).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn advise(&self, a: Advice) {
        if self.len > 0 {
            // SAFETY: advisory call on our own mapping.
            unsafe { madvise(self.ptr as *mut std::ffi::c_void, self.len, a as i32) };
        }
    }
    /// Advise only `[off, off+len)` (page-rounded inwards).
    pub fn advise_range(&self, off: usize, len: usize, a: Advice) {
        let start = (off + 4095) & !4095;
        let end = (off + len).min(self.len) & !4095;
        if end > start {
            // SAFETY: range inside our mapping.
            unsafe { madvise((self.ptr as usize + start) as *mut std::ffi::c_void, end - start, a as i32) };
        }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: unmapping exactly what we mapped.
            unsafe { munmap(self.ptr as *mut std::ffi::c_void, self.len) };
        }
    }
}

// ---------------------------------------------------------------------------------------
// Quantisation
// ---------------------------------------------------------------------------------------

/// Quantise a distribution to u8 values summing to exactly 255 (largest remainder).
pub fn quantize(p: &[f32], out: &mut [u8]) {
    let n = p.len();
    debug_assert!(n >= 1 && n <= MAX_N && out.len() == n);
    let sum: f32 = p.iter().map(|x| x.max(0.0)).sum();
    if !(sum > 0.0) || !sum.is_finite() {
        let (q, r) = (255 / n, 255 % n);
        for (i, o) in out.iter_mut().enumerate() {
            *o = (q + usize::from(i < r)) as u8;
        }
        return;
    }
    let mut rem = [0f32; MAX_N];
    let mut tot = 0i32;
    for a in 0..n {
        let x = p[a].max(0.0) / sum * 255.0;
        let f = x.floor().min(255.0);
        out[a] = f as u8;
        rem[a] = x - f;
        tot += f as i32;
    }
    let mut deficit = 255 - tot;
    while deficit > 0 {
        let mut best = 0;
        for a in 1..n {
            if rem[a] > rem[best] {
                best = a;
            }
        }
        out[best] += 1;
        rem[best] = -1.0;
        deficit -= 1;
    }
}

// ---------------------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------------------

/// Parsed header of a PLCP1 file.
#[derive(Clone, Debug)]
pub struct Info {
    pub n: u64,
    pub dir_bits: u32,
    pub blob_len: u64,
    pub src_infosets: u64,
    pub src_slots: u64,
    pub used_snapshots: bool,
    pub min_visits: u64,
    pub dropped: u64,
    pub file_len: u64,
}

/// Memory-mapped compact policy. Lookups are `&self` and lock-free.
pub struct Compact {
    mm: Mmap,
    info: Info,
    off: [usize; 5], // dir, keys, na, base, blob
}

impl Compact {
    pub fn open(path: &Path) -> io::Result<Compact> {
        assert!(cfg!(target_endian = "little"), "PLCP1 is little-endian only");
        let mm = Mmap::open(path)?;
        let b = mm.bytes();
        if b.len() < PAGE as usize || &b[..8] != MAGIC {
            return Err(bad("not a PLCP1 compact policy"));
        }
        let u = |i: usize| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
        let info = Info {
            n: u(1),
            dir_bits: u(2) as u32,
            blob_len: u(3),
            src_infosets: u(9),
            src_slots: u(10),
            used_snapshots: u(11) & 1 == 1,
            min_visits: u(12),
            dropped: u(13),
            file_len: u(14),
        };
        let off = [u(4) as usize, u(5) as usize, u(6) as usize, u(7) as usize, u(8) as usize];
        if info.file_len as usize != b.len() {
            return Err(bad(format!("file length {} != header {} (truncated copy?)", b.len(), info.file_len)));
        }
        let n = info.n as usize;
        let ok = info.dir_bits >= 1
            && info.dir_bits < 40
            && off[0] + 8 * ((1usize << info.dir_bits) + 1) <= off[1]
            && off[1] + 8 * n <= off[2]
            && off[2] + n <= off[3]
            && off[3] + 8 * n.div_ceil(64) <= off[4]
            && off[4] + info.blob_len as usize <= b.len();
        if !ok {
            return Err(bad("inconsistent PLCP1 header"));
        }
        Ok(Compact { mm, info, off })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }
    pub fn len(&self) -> usize {
        self.info.n as usize
    }
    pub fn is_empty(&self) -> bool {
        self.info.n == 0
    }
    pub fn file_bytes(&self) -> u64 {
        self.info.file_len
    }
    /// Ask the OS to read the whole file ahead (use before a long match run when RAM allows).
    pub fn warm(&self) {
        self.mm.advise(Advice::WillNeed);
    }
    /// Declare random access (disables read-around; good when the file exceeds RAM).
    pub fn random_access(&self) {
        self.mm.advise(Advice::Random);
    }

    #[inline(always)]
    fn dir(&self) -> &[u64] {
        let n = (1usize << self.info.dir_bits) + 1;
        // SAFETY: bounds validated in `open`; sections are 4096-aligned inside a page-aligned map.
        unsafe { std::slice::from_raw_parts(self.mm.bytes().as_ptr().add(self.off[0]) as *const u64, n) }
    }
    #[inline(always)]
    fn keys(&self) -> &[u64] {
        // SAFETY: as above.
        unsafe { std::slice::from_raw_parts(self.mm.bytes().as_ptr().add(self.off[1]) as *const u64, self.len()) }
    }
    #[inline(always)]
    fn na(&self) -> &[u8] {
        &self.mm.bytes()[self.off[2]..self.off[2] + self.len()]
    }
    #[inline(always)]
    fn base(&self) -> &[u64] {
        // SAFETY: as above.
        unsafe { std::slice::from_raw_parts(self.mm.bytes().as_ptr().add(self.off[3]) as *const u64, self.len().div_ceil(64)) }
    }
    #[inline(always)]
    fn blob(&self) -> &[u8] {
        &self.mm.bytes()[self.off[4]..self.off[4] + self.info.blob_len as usize]
    }

    /// Index of `key` (an `infoset_key`), if present.
    #[inline]
    pub fn find(&self, key: u64) -> Option<usize> {
        let b = (key >> (64 - self.info.dir_bits)) as usize;
        let dir = self.dir();
        let (lo, hi) = (dir[b] as usize, dir[b + 1] as usize);
        self.keys()[lo..hi].binary_search(&key).ok().map(|i| lo + i)
    }

    pub fn key_at(&self, i: usize) -> u64 {
        self.keys()[i]
    }

    /// Quantised distribution (sums to 255) of entry `i`.
    #[inline]
    pub fn entry(&self, i: usize) -> &[u8] {
        let na = self.na();
        let blk = i / 64;
        let s: u64 = na[blk * 64..i].iter().map(|&x| x as u64).sum();
        let st = (self.base()[blk] + s) as usize;
        &self.blob()[st..st + na[i] as usize]
    }

    /// Quantised distribution of the infoset (`history`, `bucket`).
    #[inline]
    pub fn lookup(&self, history: u64, bucket: u32) -> Option<&[u8]> {
        self.find(infoset_key(history, bucket)).map(|i| self.entry(i))
    }

    /// Fill `out` with the infoset's probabilities; false if absent or the action count differs.
    #[inline]
    pub fn probs(&self, history: u64, bucket: u32, out: &mut [f32]) -> bool {
        match self.lookup(history, bucket) {
            Some(q) if q.len() == out.len() => {
                for (o, &v) in out.iter_mut().zip(q) {
                    *o = v as f32 * (1.0 / 255.0);
                }
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Streaming builder
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CompactOptions {
    pub threads: usize,
    /// Drop infosets with fewer visits. First-round infosets: total φ count; others: dropped
    /// when no regret is positive and no snapshot covers them (never-trained, uniform).
    /// 0 keeps everything (exact export semantics).
    pub min_visits: u64,
    /// Checkpoint records handled per rayon task.
    pub chunk: usize,
    /// Arena slots per pass over the records. A pass keeps its arena range (4 B/slot) and
    /// snapshot-sum range (2 B/slot) hot in the page cache, so the working set is
    /// `group_slots x 6 B` (default 5 G slots = 30 GB). Fewer, bigger passes read the record
    /// file fewer times; smaller passes need less cache.
    pub group_slots: u64,
    /// Target size of one key partition in the temporary files (bounds pass-2 memory).
    pub part_bytes: u64,
    /// Directory for the temporary partition files (default: `OUT.parts` next to the output).
    pub tmp_dir: Option<std::path::PathBuf>,
}

impl Default for CompactOptions {
    fn default() -> Self {
        CompactOptions { threads: 8, min_visits: 0, chunk: 1 << 20, group_slots: 5_000_000_000, part_bytes: 512 << 20, tmp_dir: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompactStats {
    pub infosets_in: u64,
    pub infosets_out: u64,
    pub dropped: u64,
    pub slots_in: u64,
    pub snapshot_covered: u64,
    pub phi_infosets: u64,
    pub bytes_out: u64,
    pub secs: f64,
}

#[derive(Clone, Copy)]
struct Rec {
    key: u64,
    off: u64,
    na: u8,
    avg: bool,
}

struct SumsView {
    mm: Mmap,
    data_start: usize,
    n: u64,
    used_max: u64,
}

impl SumsView {
    fn open(path: &Path) -> io::Result<SumsView> {
        let mm = Mmap::open(path)?;
        let b = mm.bytes();
        if b.len() < 16 || &b[..8] != SUM_MAGIC {
            return Err(bad("not a PLSA1 snapshot-sum file"));
        }
        let rd = |o: usize| -> io::Result<u64> {
            b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| bad("truncated PLSA1 header"))
        };
        let k = rd(8)? as usize;
        let mut used_max = 0;
        for j in 0..k {
            used_max = used_max.max(rd(16 + 8 * j)?);
        }
        let n = rd(16 + 8 * k)?;
        let data_start = 8 * (3 + k);
        if (data_start as u64 + 2 * n) > b.len() as u64 {
            return Err(bad("truncated PLSA1 data"));
        }
        Ok(SumsView { mm, data_start, n, used_max })
    }
    #[inline(always)]
    fn sum(&self, slot: u64) -> u16 {
        if slot >= self.n {
            return 0;
        }
        let o = self.data_start + 2 * slot as usize;
        let b = self.mm.bytes();
        u16::from_le_bytes([b[o], b[o + 1]])
    }
}

struct Source {
    ck: Mmap,
    arena0: usize,
    sums: Option<SumsView>,
}

impl Source {
    #[inline(always)]
    fn slot(&self, i: u64) -> i32 {
        let p = self.arena0 + 4 * i as usize;
        let b = self.ck.bytes();
        i32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
    }

    /// Final probabilities of `r` (export_in_place semantics). Returns (covered_by_snapshots).
    fn final_probs(&self, r: &Rec, p: &mut [f32; MAX_N]) -> bool {
        let n = r.na as usize;
        if !r.avg {
            if let Some(s) = &self.sums {
                if s.used_max > r.off && (0..n).any(|a| s.sum(r.off + a as u64) > 0) {
                    let tot: f32 = (0..n).map(|a| s.sum(r.off + a as u64) as f32).sum();
                    for a in 0..n {
                        p[a] = s.sum(r.off + a as u64) as f32 / tot;
                    }
                    return true;
                }
            }
        }
        if r.avg {
            let total: i64 = (0..n).map(|a| self.slot(r.off + n as u64 + a as u64) as i64).sum();
            if total > 0 {
                for a in 0..n {
                    p[a] = self.slot(r.off + n as u64 + a as u64) as f32 / total as f32;
                }
                return false;
            }
        }
        // Store::strategy: regret matching on the current regrets.
        let mut sum = 0f32;
        for a in 0..n {
            let v = self.slot(r.off + a as u64).max(0) as f32;
            p[a] = v;
            sum += v;
        }
        if sum > 0.0 {
            for a in 0..n {
                p[a] /= sum;
            }
        } else {
            for a in 0..n {
                p[a] = 1.0 / n as f32;
            }
        }
        false
    }

    /// Whether the infoset passes the `min_visits` filter.
    fn keep(&self, r: &Rec, min_visits: u64) -> bool {
        if min_visits == 0 {
            return true;
        }
        let n = r.na as usize;
        if r.avg {
            let total: i64 = (0..n).map(|a| self.slot(r.off + n as u64 + a as u64) as i64).sum();
            return total.max(0) as u64 >= min_visits;
        }
        if let Some(s) = &self.sums {
            if s.used_max > r.off && (0..n).any(|a| s.sum(r.off + a as u64) > 0) {
                return true;
            }
        }
        (0..n).any(|a| self.slot(r.off + a as u64) > 0)
    }
}

fn pad(x: u64) -> u64 {
    x.div_ceil(PAGE) * PAGE
}

/// Touch every page of `[off, off+len)` of a mapping with `threads` strips so the kernel reads
/// it sequentially into the page cache (instead of one random 4 KB fault per lookup).
fn prefetch(mm: &Mmap, off: usize, len: usize) {
    if len == 0 {
        return;
    }
    let b = mm.bytes();
    let end = (off + len).min(b.len());
    const STRIP: usize = 8 << 20;
    let strips = (end - off).div_ceil(STRIP);
    let sink: u64 = (0..strips)
        .into_par_iter()
        .map(|s| {
            let (lo, hi) = (off + s * STRIP, (off + (s + 1) * STRIP).min(end));
            let mut acc = 0u64;
            let mut i = lo;
            while i < hi {
                acc += b[i] as u64;
                i += 4096;
            }
            acc
        })
        .sum();
    std::hint::black_box(sink);
}

struct PartFiles {
    files: Vec<std::sync::Mutex<File>>,
    paths: Vec<std::path::PathBuf>,
}

/// Convert `ckpt` (PLBP4) [+ `sums` (PLSA1)] into the compact file `out`.
pub fn compact_checkpoint(ckpt: &Path, sums: Option<&Path>, out: &Path, opt: &CompactOptions, log: &(dyn Fn(&str) + Sync)) -> io::Result<CompactStats> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(opt.threads.max(1)).build().map_err(|e| bad(e.to_string()))?;
    pool.install(|| compact_inner(ckpt, sums, out, opt, log))
}

fn compact_inner(ckpt: &Path, sums: Option<&Path>, out: &Path, opt: &CompactOptions, log: &(dyn Fn(&str) + Sync)) -> io::Result<CompactStats> {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::sync::Mutex;
    assert!(cfg!(target_endian = "little"), "PLCP1 is little-endian only");
    let t0 = Instant::now();
    let ck = Mmap::open(ckpt)?;
    let b = ck.bytes();
    if b.len() < CK_HDR || &b[..8] != CK_MAGIC {
        return Err(bad("not a PLBP4 blueprint checkpoint"));
    }
    let n_in = u64::from_le_bytes(b[8..16].try_into().unwrap());
    let used = u64::from_le_bytes(b[16..24].try_into().unwrap());
    let arena0 = CK_HDR + CK_REC * n_in as usize;
    if b.len() as u64 != arena0 as u64 + 4 * used {
        return Err(bad(format!("checkpoint length {} != expected {}", b.len(), arena0 as u64 + 4 * used)));
    }
    ck.advise_range(arena0, 4 * used as usize, Advice::Random);
    ck.advise_range(CK_HDR, CK_REC * n_in as usize, Advice::Sequential);
    let sums_view = match sums {
        Some(p) if p.exists() => {
            let s = SumsView::open(p)?;
            s.mm.advise(Advice::Random);
            Some(s)
        }
        _ => None,
    };
    let used_snapshots = sums_view.is_some();
    let src = Source { ck, arena0, sums: sums_view };
    let recs = &src.ck.bytes()[CK_HDR..arena0];

    // ---- pass 1: arena-ordered passes, results scattered into key partitions ----------------
    // Temp partition p holds, for every kept infoset whose key's top `pb` bits equal p, the
    // entries [key u64][na u8][na prob bytes] in arbitrary order.
    let est_bytes = n_in * 9 + used;
    let pb = (est_bytes / opt.part_bytes.max(1)).max(1).next_power_of_two().trailing_zeros().min(9);
    let np = 1usize << pb;
    let part_of = |key: u64| -> usize {
        if pb == 0 {
            0
        } else {
            (key >> (64 - pb)) as usize
        }
    };
    let tmp_dir = opt.tmp_dir.clone().unwrap_or_else(|| out.with_extension("parts"));
    std::fs::create_dir_all(&tmp_dir)?;
    let parts = PartFiles {
        paths: (0..np).map(|p| tmp_dir.join(format!("part-{p:04}.bin"))).collect(),
        files: (0..np).map(|p| File::create(tmp_dir.join(format!("part-{p:04}.bin"))).map(Mutex::new)).collect::<io::Result<_>>()?,
    };
    let counts: Vec<AtomicU64> = (0..np).map(|_| AtomicU64::new(0)).collect();
    let (dropped, covered, phi, bad_na) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let group = opt.group_slots.max(1);
    let n_groups = used.div_ceil(group).max(1);
    let chunk = opt.chunk.max(1);
    let n_chunks = (n_in as usize).div_ceil(chunk);
    log(&format!(
        "compact: {n_in} infosets, {:.2}G slots, snapshots: {used_snapshots}; {n_groups} arena group(s) x {np} key partitions in {}",
        used as f64 / 1e9,
        tmp_dir.display()
    ));
    let mut lo = 0u64;
    let mut gi = 0;
    while lo < used || (used == 0 && gi == 0) {
        let hi = (lo + group).min(used);
        gi += 1;
        let tg = Instant::now();
        prefetch(&src.ck, arena0 + 4 * lo as usize, 4 * (hi - lo) as usize);
        if let Some(s) = &src.sums {
            if lo < s.n {
                prefetch(&s.mm, s.data_start + 2 * lo as usize, 2 * (hi.min(s.n) - lo) as usize);
            }
        }
        let tp = tg.elapsed().as_secs_f64();
        (0..n_chunks).into_par_iter().try_for_each(|c| -> io::Result<()> {
            let mut local: Vec<Vec<u8>> = vec![Vec::new(); np];
            let (mut cov, mut ph, mut dr) = (0u64, 0u64, 0u64);
            for i in c * chunk..((c + 1) * chunk).min(n_in as usize) {
                let rc = &recs[i * CK_REC..(i + 1) * CK_REC];
                let off = u64::from_le_bytes(rc[8..16].try_into().unwrap());
                if off < lo || off >= hi {
                    continue;
                }
                let r = Rec { key: u64::from_le_bytes(rc[..8].try_into().unwrap()), off, na: rc[16], avg: rc[17] != 0 };
                if r.na == 0 || r.na as usize > MAX_N {
                    bad_na.store(r.na as u64 + 1, Relaxed);
                    continue;
                }
                if !src.keep(&r, opt.min_visits) {
                    dr += 1;
                    continue;
                }
                let mut p = [0f32; MAX_N];
                cov += u64::from(src.final_probs(&r, &mut p));
                ph += u64::from(r.avg);
                let n = r.na as usize;
                let mut q = [0u8; MAX_N];
                quantize(&p[..n], &mut q[..n]);
                let buf = &mut local[part_of(r.key)];
                buf.extend_from_slice(&r.key.to_le_bytes());
                buf.push(r.na);
                buf.extend_from_slice(&q[..n]);
            }
            for (p, buf) in local.iter().enumerate() {
                if !buf.is_empty() {
                    let recs_in_buf = {
                        let (mut k, mut n) = (0usize, 0u64);
                        while k < buf.len() {
                            k += 9 + buf[k + 8] as usize;
                            n += 1;
                        }
                        n
                    };
                    counts[p].fetch_add(recs_in_buf, Relaxed);
                    parts.files[p].lock().unwrap().write_all(buf)?;
                }
            }
            covered.fetch_add(cov, Relaxed);
            phi.fetch_add(ph, Relaxed);
            dropped.fetch_add(dr, Relaxed);
            Ok(())
        })?;
        if bad_na.load(Relaxed) != 0 {
            return Err(bad(format!("infoset with {} actions (max {MAX_N})", bad_na.load(Relaxed) - 1)));
        }
        log(&format!("  pass 1, arena group {gi}/{n_groups}: slots [{:.2}G, {:.2}G) prefetch {tp:.0}s, total {:.0}s ({:.0}s elapsed)", lo as f64 / 1e9, hi as f64 / 1e9, tg.elapsed().as_secs_f64(), t0.elapsed().as_secs_f64()));
        lo = hi;
        if used == 0 {
            break;
        }
    }
    for f in &parts.files {
        f.lock().unwrap().flush()?;
    }
    drop(src);

    // ---- layout (exact sizes are known now) ---------------------------------------------------
    let counts: Vec<u64> = counts.iter().map(|c| c.load(Relaxed)).collect();
    let n_out: u64 = counts.iter().sum();
    let sizes: Vec<u64> = parts.paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len())).collect::<io::Result<_>>()?;
    let blob_sizes: Vec<u64> = sizes.iter().zip(&counts).map(|(s, c)| s - 9 * c).collect();
    let blob_len: u64 = blob_sizes.iter().sum();
    let dir_bits = ((n_out.max(1) / 32).max(1).next_power_of_two().trailing_zeros()).clamp(8, 40).max(pb);
    let dir_len = (1u64 << dir_bits) + 1;
    let off_dir = PAGE;
    let off_keys = pad(off_dir + 8 * dir_len);
    let off_na = pad(off_keys + 8 * n_out);
    let off_base = pad(off_na + n_out);
    let off_blob = pad(off_base + 8 * n_out.div_ceil(64));
    let file_len = off_blob + blob_len;
    let tmp = out.with_extension("tmp");
    let of = OpenOptions::new().create(true).write(true).truncate(true).open(&tmp)?;
    of.set_len(file_len)?;
    let start_idx: Vec<u64> = counts.iter().scan(0u64, |a, c| { let s = *a; *a += c; Some(s) }).collect();
    let start_blob: Vec<u64> = blob_sizes.iter().scan(0u64, |a, c| { let s = *a; *a += c; Some(s) }).collect();

    // ---- pass 2: sort each partition by key and write it at its final position ---------------
    let done = AtomicU64::new(0);
    (0..np).into_par_iter().try_for_each(|p| -> io::Result<()> {
        let data = std::fs::read(&parts.paths[p])?;
        let mut idx: Vec<(u64, u64)> = Vec::with_capacity(counts[p] as usize);
        let mut k = 0usize;
        while k < data.len() {
            idx.push((u64::from_le_bytes(data[k..k + 8].try_into().unwrap()), k as u64));
            k += 9 + data[k + 8] as usize;
        }
        if idx.len() as u64 != counts[p] {
            return Err(bad("partition file corrupt"));
        }
        idx.sort_unstable();
        if idx.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(bad("duplicate key in checkpoint"));
        }
        let (mut keys, mut nas) = (Vec::with_capacity(8 * idx.len()), Vec::with_capacity(idx.len()));
        let mut blob = Vec::with_capacity(blob_sizes[p] as usize);
        let mut base: Vec<u8> = Vec::new();
        let mut dir: Vec<u8> = Vec::new();
        let (g0, mut blob_pos) = (start_idx[p], start_blob[p]);
        // dir[b] for the buckets that belong to this partition.
        let bshift = 64 - dir_bits;
        let per = 1u64 << (dir_bits - pb);
        let (b_lo, b_hi) = ((p as u64) * per, (p as u64 + 1) * per);
        let mut next_b = b_lo;
        for (j, &(key, pos)) in idx.iter().enumerate() {
            let g = g0 + j as u64;
            let bucket = key >> bshift;
            while next_b <= bucket {
                dir.extend_from_slice(&g.to_le_bytes());
                next_b += 1;
            }
            let na = data[pos as usize + 8] as usize;
            keys.extend_from_slice(&key.to_le_bytes());
            nas.push(na as u8);
            if g % 64 == 0 {
                base.extend_from_slice(&blob_pos.to_le_bytes());
            }
            blob.extend_from_slice(&data[pos as usize + 9..pos as usize + 9 + na]);
            blob_pos += na as u64;
        }
        while next_b < b_hi {
            dir.extend_from_slice(&(g0 + idx.len() as u64).to_le_bytes());
            next_b += 1;
        }
        of.write_all_at(&dir, off_dir + 8 * b_lo)?;
        of.write_all_at(&keys, off_keys + 8 * g0)?;
        of.write_all_at(&nas, off_na + g0)?;
        // First multiple of 64 that is >= g0 determines where this partition's base entries go.
        of.write_all_at(&base, off_base + 8 * g0.div_ceil(64))?;
        of.write_all_at(&blob, off_blob + start_blob[p])?;
        drop(data);
        let _ = std::fs::remove_file(&parts.paths[p]);
        let d = done.fetch_add(1, Relaxed) + 1;
        if d % (np as u64 / 8).max(1) == 0 {
            log(&format!("  pass 2 (key order): {d}/{np} partitions written ({:.0}s elapsed)", t0.elapsed().as_secs_f64()));
        }
        Ok(())
    })?;
    of.write_all_at(&n_out.to_le_bytes(), off_dir + 8 * (dir_len - 1))?;

    // ---- header ---------------------------------------------------------------------------------
    let mut h = vec![0u8; PAGE as usize];
    h[..8].copy_from_slice(MAGIC);
    let dropped = dropped.load(Relaxed);
    let words: [u64; 14] = [n_out, dir_bits as u64, blob_len, off_dir, off_keys, off_na, off_base, off_blob, n_in, used, u64::from(used_snapshots), opt.min_visits, dropped, file_len];
    for (i, w) in words.iter().enumerate() {
        h[8 * (i + 1)..8 * (i + 2)].copy_from_slice(&w.to_le_bytes());
    }
    of.write_all_at(&h, 0)?;
    of.sync_all()?;
    drop(of);
    std::fs::rename(&tmp, out)?;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let st = CompactStats {
        infosets_in: n_in,
        infosets_out: n_out,
        dropped,
        slots_in: used,
        snapshot_covered: covered.load(Relaxed),
        phi_infosets: phi.load(Relaxed),
        bytes_out: file_len,
        secs: t0.elapsed().as_secs_f64(),
    };
    log(&format!("compact: wrote {} ({} infosets, {:.2} GB, {:.2} B/infoset, {:.0}s)", out.display(), n_out, file_len as f64 / 1e9, file_len as f64 / n_out.max(1) as f64, st.secs));
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::{io as ckio, snapshot};
    use std::sync::atomic::Ordering::Relaxed;

    fn sm(x: &mut u64) -> u64 {
        *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("plcp_{}_{}", std::process::id(), name))
    }

    #[test]
    fn quantize_sums_to_255_and_is_close() {
        let mut s = 7u64;
        for _ in 0..2000 {
            let n = 1 + (sm(&mut s) % 24) as usize;
            let mut p: Vec<f32> = (0..n).map(|_| if sm(&mut s) % 3 == 0 { 0.0 } else { (sm(&mut s) % 1000) as f32 }).collect();
            let t: f32 = p.iter().sum();
            if t == 0.0 {
                p[0] = 1.0;
            }
            let t: f32 = p.iter().sum();
            p.iter_mut().for_each(|x| *x /= t);
            let mut q = vec![0u8; n];
            quantize(&p, &mut q);
            assert_eq!(q.iter().map(|&x| x as u32).sum::<u32>(), 255);
            for a in 0..n {
                assert!((q[a] as f32 / 255.0 - p[a]).abs() <= 1.0 / 255.0 + 1e-6, "{p:?} {q:?}");
                if p[a] == 0.0 {
                    assert_eq!(q[a], 0);
                }
            }
        }
        let mut q = [0u8; 3];
        quantize(&[0.0, 0.0, 0.0], &mut q);
        assert_eq!(q.iter().map(|&x| x as u32).sum::<u32>(), 255);
    }

    /// Random store with φ (first-round) and plain infosets, snapshots folded twice, a late
    /// infoset, all-negative / all-zero edge cases.
    fn toy_store(n: usize, seed: u64) -> Store {
        let st = Store::new(n * 40);
        let mut s = seed;
        let mut fill = |st: &Store, count: usize, base: u64| {
            for i in 0..count {
                let h = sm(&mut s);
                let b = (sm(&mut s) % 200) as u32;
                let na = 2 + (sm(&mut s) % 8) as usize;
                let avg = sm(&mut s) % 10 == 0;
                let slot = st.infoset(h, b, na, 200, avg);
                let kind = (base + i as u64) % 7;
                for a in 0..na {
                    let v = match kind {
                        0 => 0.0,                                             // untouched -> uniform
                        1 => -((sm(&mut s) % 1000) as f32),                   // all negative -> uniform
                        _ => (sm(&mut s) % 2000) as f32 - 500.0,
                    };
                    st.add_regret(slot, a, v);
                    if avg && kind != 3 {
                        for _ in 0..(sm(&mut s) % 4) {
                            st.bump_avg(slot, a);
                        }
                    }
                }
            }
        };
        fill(&st, n / 2, 0);
        st
    }

    #[test]
    fn compact_matches_export_semantics() {
        let n = 30_000usize;
        let st = toy_store(n, 99);
        let sums = tmp("sums");
        let _ = std::fs::remove_file(&sums);
        snapshot::accumulate(&st, &sums).unwrap();
        // Perturb the regrets, add late infosets, second snapshot.
        let mut s = 5u64;
        for (_, slot) in st.entries() {
            let a = (sm(&mut s) % slot.n_actions as u64) as usize;
            st.add_regret(slot, a, (sm(&mut s) % 3000) as f32 - 1000.0);
        }
        for i in 0..2000u64 {
            let slot = st.infoset(sm(&mut s), (i % 50) as u32, 3, 50, false);
            st.add_regret(slot, (i % 3) as usize, 100.0);
        }
        snapshot::accumulate(&st, &sums).unwrap();
        for (_, slot) in st.entries().into_iter().take(500) {
            st.add_regret(slot, 0, 777.0);
        }
        let ck_random = tmp("ck.bin");
        ckio::save(&st, &ck_random).unwrap();
        // A copy of the same checkpoint: exercises reading a file that is not the writer's output
        // path (hard link semantics are irrelevant to the mmap).
        let ck = tmp("ck_copy.bin");
        std::fs::copy(&ck_random, &ck).unwrap();
        let out = tmp("out.plcp");

        for (ckpt, chunk, group, part, threads) in [(&ck_random, 1000usize, 20_000u64, 30_000u64, 4usize), (&ck, 777, 1u64 << 40, 1 << 40, 3), (&ck, 1 << 14, 100_000, 8_000, 2), (&ck_random, 1 << 20, 3_000_000_000, 512 << 20, 8)] {
            let opt = CompactOptions { threads, min_visits: 0, chunk, group_slots: group, part_bytes: part, tmp_dir: None };
            let stats = compact_checkpoint(ckpt, Some(&sums), &out, &opt, &|_| {}).unwrap();
            assert_eq!(stats.infosets_out as usize, st.infosets());
            assert!(stats.snapshot_covered > 0 && stats.phi_infosets > 0);

            // Reference: the loaded checkpoint exported in place.
            let reference = ckio::load(ckpt, 0, 0).unwrap();
            snapshot::export_in_place(&reference, Some(&sums)).unwrap();
            let c = Compact::open(&out).unwrap();
            assert_eq!(c.len(), st.infosets());
            let mut worst = 0f32;
            for (key, slot) in reference.entries() {
                let i = c.find(key).expect("infoset present");
                let q = c.entry(i);
                assert_eq!(q.len(), slot.n_actions as usize);
                assert_eq!(q.iter().map(|&x| x as u32).sum::<u32>(), 255);
                let mut p = vec![0f32; q.len()];
                reference.strategy(slot, &mut p);
                for a in 0..q.len() {
                    worst = worst.max((q[a] as f32 / 255.0 - p[a]).abs());
                }
            }
            assert!(worst <= 1.0 / 255.0 + 1e-4, "worst quantisation error {worst}");
            // Sorted, unique, absent keys miss.
            for i in 1..c.len() {
                assert!(c.key_at(i - 1) < c.key_at(i));
            }
            assert!(c.find(0x1234_5678_9ABC_DEF1).is_none());
            let mut buf = [0f32; 4];
            let k = reference.entries().into_iter().find(|e| e.1.n_actions == 4).unwrap();
            let _ = Compact::open(&out).unwrap().find(k.0).unwrap();
            assert!(!c.probs(0xDEAD, 3, &mut buf));
            eprintln!("worst err {worst:.5}, {:.2} B/infoset", stats.bytes_out as f64 / stats.infosets_out as f64);
            drop(reference);
        }

        // No snapshots: φ preflop + current strategy elsewhere, and the min_visits filter.
        let opt = CompactOptions { threads: 2, min_visits: 0, chunk: 5000, group_slots: 40_000, part_bytes: 50_000, tmp_dir: None };
        compact_checkpoint(&ck, None, &out, &opt, &|_| {}).unwrap();
        let c = Compact::open(&out).unwrap();
        let reference = ckio::load(&ck, 0, 0).unwrap();
        for (key, slot) in reference.entries() {
            let mut p = vec![0f32; slot.n_actions as usize];
            reference.average(slot, &mut p);
            let q = c.entry(c.find(key).unwrap());
            for a in 0..p.len() {
                assert!((q[a] as f32 / 255.0 - p[a]).abs() <= 1.0 / 255.0 + 1e-4);
            }
        }
        let opt = CompactOptions { threads: 2, min_visits: 1, chunk: 5000, group_slots: 40_000, part_bytes: 50_000, tmp_dir: None };
        let stats = compact_checkpoint(&ck, None, &out, &opt, &|_| {}).unwrap();
        assert!(stats.dropped > 0 && stats.infosets_out + stats.dropped == stats.infosets_in);
        let c = Compact::open(&out).unwrap();
        assert_eq!(c.len() as u64, stats.infosets_out);
        for (key, slot) in reference.entries() {
            let trained = if slot.with_avg { reference.avg(slot).iter().map(|x| x.load(Relaxed) as i64).sum::<i64>() > 0 } else { reference.regrets(slot).iter().any(|x| x.load(Relaxed) > 0) };
            assert_eq!(c.find(key).is_some(), trained, "min_visits filter");
        }
        for p in [&ck, &ck_random, &sums, &out] {
            let _ = std::fs::remove_file(p);
        }
    }
}
