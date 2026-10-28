//! Postflop strategy snapshots and blueprint export.
//!
//! Paper: "For situations after the first betting round, a snapshot of the current strategy
//! was taken every 200 minutes after the initial 800 minutes of training. The blueprint
//! strategy after the first betting round was constructed by averaging together these
//! snapshots", kept on disk to halve memory. The first betting round uses the average φ.
//!
//! Instead of one file per snapshot (≈ one byte per regret slot each — too much disk at
//! paper scale) we fold every snapshot into one running-sum file as it is taken:
//! `PLSA1\0\0\0`, u64 snapshot count k, k × u64 `used_j` (arena size when snapshot j was
//! taken), u64 n, then n × u16 sums of `round(255 · p)`. Slot `i` was covered by the
//! snapshots with `used_j > i` (the arena is append-only, so offsets are stable). Updates
//! stream in chunks so no extra RAM proportional to the store is needed.
//!
//! `export_in_place` then overwrites the trained store's regrets with the final action
//! probabilities × `EXPORT_SCALE` (φ preflop, snapshot average postflop), so regret matching
//! (`Store::strategy`) reproduces the blueprint and every consumer reads it unchanged.

use crate::store::{Slot, Store};
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;

const MAGIC: &[u8; 8] = b"PLSA1\0\0\0";
pub const EXPORT_SCALE: f32 = 1_000_000.0;
/// Slots per pass; each pass is one parallel scan of the whole index.
const CHUNK: u64 = 1 << 30;
const SLACK: u64 = 64;

struct Header {
    used: Vec<u64>,
    n: u64,
    data_start: u64,
}

fn read_header(f: &File) -> io::Result<Header> {
    let mut r = f;
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    if &b != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a snapshot-sum file"));
    }
    let mut u = || -> io::Result<u64> {
        r.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    };
    let k = u()?;
    let used = (0..k).map(|_| u()).collect::<io::Result<Vec<_>>>()?;
    let n = u()?;
    Ok(Header { data_start: 8 * (3 + k), used, n })
}

/// Sums for slots `[lo, hi)` (zeros past the end of the file's coverage).
fn read_sums(f: &File, h: &Header, lo: u64, hi: u64) -> io::Result<Vec<u16>> {
    let mut out = vec![0u16; (hi - lo) as usize];
    let have = h.n.saturating_sub(lo).min(hi - lo) as usize;
    if have > 0 {
        let mut bytes = vec![0u8; have * 2];
        f.read_exact_at(&mut bytes, h.data_start + lo * 2)?;
        for (o, c) in out.iter_mut().zip(bytes.chunks_exact(2)) {
            *o = u16::from_le_bytes([c[0], c[1]]);
        }
    }
    Ok(out)
}

/// Raw pointer wrapper so disjoint per-infoset writes can happen from the parallel scan.
#[derive(Clone, Copy)]
struct Out(*mut u16);
unsafe impl Send for Out {}
unsafe impl Sync for Out {}

/// Fold the store's current postflop strategy into the running-sum file at `path`.
pub fn accumulate(store: &Store, path: &Path) -> io::Result<()> {
    let used = store.used_slots();
    let old = if path.exists() { Some(File::open(path)?) } else { None };
    let hdr = match &old {
        Some(f) => read_header(f)?,
        None => Header { used: vec![], n: 0, data_start: 0 },
    };
    let tmp = path.with_extension("tmp");
    let mut w = BufWriter::with_capacity(1 << 24, File::create(&tmp)?);
    w.write_all(MAGIC)?;
    w.write_all(&(hdr.used.len() as u64 + 1).to_le_bytes())?;
    for u in hdr.used.iter().chain(std::iter::once(&used)) {
        w.write_all(&u.to_le_bytes())?;
    }
    w.write_all(&used.to_le_bytes())?;
    let mut lo = 0u64;
    while lo < used {
        let hi = (lo + CHUNK).min(used);
        let mut sums = match &old {
            Some(f) => read_sums(f, &hdr, lo, hi)?,
            None => vec![0u16; (hi - lo) as usize],
        };
        let out = Out(sums.as_mut_ptr());
        store.par_scan(0, store.table_len(), |_, s: Slot| {
            if s.with_avg || s.offset >= hi || s.offset + s.n_actions as u64 <= lo {
                return;
            }
            let out = out;
            let n = s.n_actions as usize;
            let mut p = [0f32; 32];
            store.strategy(s, &mut p[..n]);
            for (a, &pa) in p.iter().enumerate().take(n) {
                let i = s.offset + a as u64;
                if i >= lo && i < hi {
                    // SAFETY: every slot belongs to exactly one infoset → disjoint writes.
                    unsafe {
                        let cell = out.0.add((i - lo) as usize);
                        *cell = cell.read().saturating_add((pa * 255.0).round() as u16);
                    }
                }
            }
        });
        let bytes: Vec<u8> = sums.iter().flat_map(|s| s.to_le_bytes()).collect();
        w.write_all(&bytes)?;
        lo = hi;
    }
    w.flush()?;
    drop(w);
    std::fs::rename(tmp, path)
}

/// Overwrite regrets with final blueprint probabilities × `EXPORT_SCALE`: φ for first-round
/// infosets, the snapshot average postflop (current strategy where no snapshot covers it).
pub fn export_in_place(store: &Store, sums_path: Option<&Path>) -> io::Result<()> {
    let used = store.used_slots();
    let f = match sums_path {
        Some(p) if p.exists() => Some(File::open(p)?),
        _ => None,
    };
    let hdr = match &f {
        Some(f) => read_header(f)?,
        None => Header { used: vec![], n: 0, data_start: 0 },
    };
    let mut lo = 0u64;
    while lo < used {
        let hi = (lo + CHUNK).min(used);
        // Infosets are owned by the chunk holding their first slot; SLACK covers their tails.
        let sums = match &f {
            Some(f) => read_sums(f, &hdr, lo, hi + SLACK)?,
            None => vec![0u16; (hi + SLACK - lo) as usize],
        };
        store.par_scan(0, store.table_len(), |_, s: Slot| {
            if s.offset < lo || s.offset >= hi {
                return;
            }
            let n = s.n_actions as usize;
            let base = (s.offset - lo) as usize;
            let mut p = [0f32; 32];
            let covered = !s.with_avg && hdr.used.iter().any(|&u| u > s.offset) && (0..n).any(|a| sums[base + a] > 0);
            if covered {
                let tot: f32 = (0..n).map(|a| sums[base + a] as f32).sum();
                for a in 0..n {
                    p[a] = sums[base + a] as f32 / tot;
                }
            } else {
                store.average(s, &mut p[..n]);
            }
            let regrets = store.regrets(s);
            for a in 0..n {
                regrets[a].store((p[a] * EXPORT_SCALE).round() as i32, Relaxed);
            }
            if s.with_avg {
                store.avg(s).iter().for_each(|x| x.store(0, Relaxed));
            }
        });
        lo = hi;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_average_postflop_and_phi_preflop() {
        let st = Store::new(1000);
        let pre = st.infoset(1, 0, 2, 1, true);
        let post = st.infoset(2, 0, 2, 1, false);
        st.add_regret(pre, 1, 100.0);
        for _ in 0..3 {
            st.bump_avg(pre, 0);
        }
        st.bump_avg(pre, 1);
        let path = std::env::temp_dir().join("plsa_test.bin");
        let _ = std::fs::remove_file(&path);
        // Snapshot 1: current = [1, 0]; snapshot 2: current = [0, 1] → average [0.5, 0.5].
        st.add_regret(post, 0, 10.0);
        accumulate(&st, &path).unwrap();
        st.add_regret(post, 0, -20.0);
        st.add_regret(post, 1, 10.0);
        // A new infoset appears between snapshots: only covered by the second.
        let late = st.infoset(3, 0, 2, 1, false);
        st.add_regret(late, 1, 5.0);
        accumulate(&st, &path).unwrap();
        st.add_regret(late, 0, 1000.0); // current strategy changes after the last snapshot
        export_in_place(&st, Some(&path)).unwrap();
        let mut p = [0f32; 2];
        st.strategy(post, &mut p);
        assert!((p[0] - 0.5).abs() < 0.01 && (p[1] - 0.5).abs() < 0.01, "{p:?}");
        st.strategy(pre, &mut p);
        assert!((p[0] - 0.75).abs() < 1e-4, "{p:?}");
        st.strategy(late, &mut p);
        assert!(p[1] > 0.99, "{p:?}");
    }
}
