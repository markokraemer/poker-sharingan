//! Checkpoint format for a `Store`: little-endian, streamed in chunks so saving/loading a
//! several-hundred-GB store needs no extra RAM proportional to it.
//!
//! `PLBP4\0\0\0` magic, u64 infoset count, u64 slot count, then infoset records
//! (table key u64, offset u64, n_actions u8, with_avg u8) in index order, then every slot
//! as i32.

use crate::store::{Slot, Store};
use rayon::prelude::*;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;

const MAGIC: &[u8; 8] = b"PLBP4\0\0\0";
const REC: usize = 18;
const SCAN: usize = 1 << 26;
const SLOTS: usize = 1 << 27;

pub fn save(store: &Store, path: &Path) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut w = BufWriter::with_capacity(1 << 24, File::create(&tmp)?);
    let used = store.used_slots();
    w.write_all(MAGIC)?;
    w.write_all(&(store.infosets() as u64).to_le_bytes())?;
    w.write_all(&used.to_le_bytes())?;
    // Each parallel task scans a contiguous index range into its own buffer (no shared
    // lock: one mutex-guarded Vec for all threads ran at ~54 MB/s on 128 threads).
    let mut written = 0u64;
    let mut a = 0;
    while a < store.table_len() {
        let b = (a + SCAN).min(store.table_len());
        let parts: Vec<Vec<u8>> = (a..b)
            .into_par_iter()
            .step_by(1 << 16)
            .map(|lo| {
                let mut buf = Vec::new();
                for i in lo..(lo + (1 << 16)).min(b) {
                    if let Some((k, s)) = store.entry_at(i) {
                        buf.extend_from_slice(&k.to_le_bytes());
                        buf.extend_from_slice(&s.offset.to_le_bytes());
                        buf.push(s.n_actions);
                        buf.push(s.with_avg as u8);
                    }
                }
                buf
            })
            .collect();
        for p in parts {
            written += (p.len() / REC) as u64;
            w.write_all(&p)?;
        }
        a = b;
    }
    assert_eq!(written, store.infosets() as u64, "store changed during save");
    let mut lo = 0usize;
    while lo < used as usize {
        let hi = (lo + SLOTS).min(used as usize);
        let bytes: Vec<u8> = store.raw(lo as u64, hi - lo).par_iter().flat_map_iter(|x| x.load(Relaxed).to_le_bytes()).collect();
        w.write_all(&bytes)?;
        lo = hi;
    }
    w.flush()?;
    drop(w);
    std::fs::rename(tmp, path)
}

/// Load a checkpoint into a store with room for `capacity` slots and `max_infosets` index
/// entries (each grown to at least the saved size).
pub fn load(path: &Path, capacity: usize, max_infosets: usize) -> io::Result<Store> {
    let mut r = BufReader::with_capacity(1 << 24, File::open(path)?);
    let mut b8 = [0u8; 8];
    r.read_exact(&mut b8)?;
    if &b8 != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a PLBP4 blueprint checkpoint"));
    }
    let mut u64r = |r: &mut BufReader<File>| -> io::Result<u64> {
        r.read_exact(&mut b8)?;
        Ok(u64::from_le_bytes(b8))
    };
    let n = u64r(&mut r)? as usize;
    let used = u64r(&mut r)? as usize;
    // The index table is `max_infosets × 10/7` entries and training stops allocating at 85% load,
    // so a full checkpoint can hold up to ~1.21 × max_infosets infosets. Only grow the table if the
    // checkpoint would load above 80% (n × 7/8); the old `n × 5/4` sized a 16B-infoset paper
    // checkpoint at 457 GB of index (vs 320 GB in training) and the resume was OOM-killed.
    let st = Store::with_index(capacity.max(used), max_infosets.max(n * 7 / 8 + 1));
    let mut left = n;
    let mut buf = vec![0u8; SCAN * REC];
    while left > 0 {
        let k = left.min(SCAN);
        r.read_exact(&mut buf[..k * REC])?;
        buf[..k * REC].par_chunks_exact(REC).for_each(|c| {
            let key = u64::from_le_bytes(c[..8].try_into().unwrap());
            let offset = u64::from_le_bytes(c[8..16].try_into().unwrap());
            st.insert_raw(key, Slot { offset, n_actions: c[16], with_avg: c[17] != 0 });
        });
        left -= k;
    }
    let mut lo = 0usize;
    let mut bytes = vec![0u8; SLOTS * 4];
    while lo < used {
        let hi = (lo + SLOTS).min(used);
        r.read_exact(&mut bytes[..(hi - lo) * 4])?;
        let vals: Vec<i32> = bytes[..(hi - lo) * 4].par_chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
        st.set_arena_prefix(lo, &vals);
        lo = hi;
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let st = Store::new(1000);
        let a = st.infoset(1, 2, 3, 4, true);
        let b = st.infoset(5, 0, 2, 1, false);
        st.add_regret(a, 1, 77.0);
        st.add_regret(b, 0, -5.0);
        st.bump_avg(a, 2);
        let p = std::env::temp_dir().join("plbp_roundtrip.bin");
        save(&st, &p).unwrap();
        let ld = load(&p, 2000, 100).unwrap();
        let a2 = ld.lookup(1, 2).unwrap();
        assert_eq!(ld.regrets(a2)[1].load(Relaxed), 77);
        assert_eq!(ld.avg(a2)[2].load(Relaxed), 1);
        assert_eq!(ld.regrets(ld.lookup(5, 0).unwrap())[0].load(Relaxed), -5);
        assert_eq!(ld.used_slots(), st.used_slots());
        assert_eq!(ld.infosets(), 2);
        let c = ld.infoset(9, 0, 2, 1, false);
        assert_eq!(c.offset, st.used_slots());
    }
}
