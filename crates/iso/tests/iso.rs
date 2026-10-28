use cards::Card;
use iso::HandIndexer;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Random deal of `n` distinct cards.
    fn deal(&mut self, n: usize) -> Vec<Card> {
        let mut d: Vec<Card> = (0..52).collect();
        for i in 0..n {
            let j = i + (self.next() % (52 - i as u64)) as usize;
            d.swap(i, j);
        }
        d[..n].to_vec()
    }
}

fn perms() -> Vec<[u8; 4]> {
    let mut out = vec![];
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let m = 1 << a | 1 << b | 1 << c | 1 << d;
                    if m == 15 {
                        out.push([a, b, c, d]);
                    }
                }
            }
        }
    }
    out
}

/// Brute-force canonical form: min over 24 suit permutations of the per-round sorted cards.
fn canon(cpr: &[u8], cards: &[Card], perms: &[[u8; 4]]) -> Vec<Card> {
    perms
        .iter()
        .map(|p| {
            let mut out = Vec::new();
            let mut j = 0;
            for &n in cpr {
                let mut r: Vec<Card> = cards[j..j + n as usize]
                    .iter()
                    .map(|&c| (c & !3) | p[(c & 3) as usize])
                    .collect();
                r.sort();
                out.extend(r);
                j += n as usize;
            }
            out
        })
        .min()
        .unwrap()
}

#[test]
fn sizes() {
    let cases: &[(&[u8], u64)] = &[
        (&[2], 169),
        (&[2, 3], 1_286_792),
        (&[2, 4], 13_960_050),
        (&[2, 5], 123_156_254),
        (&[2, 3, 1], 55_190_538),
        (&[2, 3, 1, 1], 2_428_287_420),
    ];
    for (cpr, want) in cases {
        let ix = HandIndexer::new(cpr);
        assert_eq!(ix.size(cpr.len() - 1), *want, "{cpr:?}");
        assert_eq!(ix.rounds(), cpr.len());
    }
}

#[test]
fn exhaustive_preflop() {
    let ix = HandIndexer::new(&[2]);
    let ps = perms();
    let mut seen = vec![false; 169];
    let mut by_canon = std::collections::HashMap::new();
    for a in 0..52u8 {
        for b in a + 1..52 {
            let i = ix.index_last(&[a, b]);
            assert_eq!(i, ix.index_last(&[b, a]));
            assert!(i < 169);
            seen[i as usize] = true;
            let c = canon(&[2], &[a, b], &ps);
            assert_eq!(*by_canon.entry(c).or_insert(i), i);
            let mut out = [0u8; 2];
            assert!(ix.unindex(0, i, &mut out));
            assert_eq!(ix.index_last(&out), i);
        }
    }
    assert!(seen.iter().all(|&s| s));
    assert_eq!(by_canon.len(), 169);
}

#[test]
fn exhaustive_flop() {
    let ix = HandIndexer::new(&[2, 3]);
    let size = ix.size(1);
    let mut seen = vec![false; size as usize];
    let mut count = 0u64;
    for a in 0..52u8 {
        for b in a + 1..52 {
            for x in 0..52u8 {
                if x == a || x == b {
                    continue;
                }
                for y in x + 1..52 {
                    if y == a || y == b {
                        continue;
                    }
                    for z in y + 1..52 {
                        if z == a || z == b {
                            continue;
                        }
                        let i = ix.index_last(&[a, b, x, y, z]);
                        assert!(i < size);
                        seen[i as usize] = true;
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 1326 * 19600);
    assert!(seen.iter().all(|&s| s), "not all indices covered");
    let mut out = [0u8; 5];
    for i in (0..size).step_by(7) {
        assert!(ix.unindex(1, i, &mut out));
        assert_eq!(ix.index_last(&out), i);
    }
}

/// Same index iff suit-permutation equivalent, checked on random deals and on
/// random suit-permuted/reordered copies.
fn check_iso(cpr: &[u8], n: usize) {
    let ix = HandIndexer::new(cpr);
    let ps = perms();
    let total: usize = cpr.iter().map(|&c| c as usize).sum();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut map: std::collections::HashMap<u64, Vec<Card>> = Default::default();
    let mut rev: std::collections::HashMap<Vec<Card>, u64> = Default::default();
    for _ in 0..n {
        let d = rng.deal(total);
        let i = ix.index_last(&d);
        let c = canon(cpr, &d, &ps);
        assert_eq!(
            map.entry(i).or_insert_with(|| c.clone()),
            &c,
            "collision {cpr:?}"
        );
        assert_eq!(*rev.entry(c).or_insert(i), i, "split class {cpr:?}");
        // permuted + shuffled-within-round copy must agree
        let p = ps[(rng.next() % 24) as usize];
        let mut e = d.clone();
        let mut j = 0;
        for &k in cpr {
            let s = &mut e[j..j + k as usize];
            for c in s.iter_mut() {
                *c = (*c & !3) | p[(*c & 3) as usize];
            }
            s.reverse();
            j += k as usize;
        }
        assert_eq!(ix.index_last(&e), i);
    }
}

#[test]
fn isomorphism_random() {
    check_iso(&[2, 3], 300_000);
    check_iso(&[2, 5], 300_000);
    check_iso(&[2, 3, 1, 1], 300_000);
}

fn roundtrip(cpr: &[u8], n: usize) {
    let ix = HandIndexer::new(cpr);
    let last = cpr.len() - 1;
    let subs: Vec<HandIndexer> = (0..cpr.len())
        .map(|r| HandIndexer::new(&cpr[..=r]))
        .collect();
    let total: usize = cpr.iter().map(|&c| c as usize).sum();
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let mut out = vec![0u8; total];
    let mut all = [0u64; 8];
    for _ in 0..n {
        let d = rng.deal(total);
        let i = ix.index_last(&d);
        assert!(i < ix.size(last));
        ix.index_all(&d, &mut all);
        assert_eq!(all[last], i);
        assert!(ix.unindex(last, i, &mut out));
        assert_eq!(ix.index_last(&out), i, "{d:?} -> {out:?}");
        // prefix rounds
        let r = (rng.next() % cpr.len() as u64) as usize;
        let m: usize = cpr[..=r].iter().map(|&c| c as usize).sum();
        assert!(ix.unindex(r, all[r], &mut out[..m]));
        let mut all2 = [0u64; 8];
        let mut full = out[..m].to_vec();
        full.resize(total, 0);
        // index only the prefix rounds
        subs[r].index_all(&full[..m], &mut all2);
        assert_eq!(all2[r], all[r]);
    }
    assert!(!ix.unindex(last, ix.size(last), &mut out));
}

#[test]
fn roundtrip_2_4() {
    roundtrip(&[2, 4], 1_000_000);
}
#[test]
fn roundtrip_2_5() {
    roundtrip(&[2, 5], 1_000_000);
}
#[test]
fn roundtrip_2_3_1_1() {
    roundtrip(&[2, 3, 1, 1], 1_000_000);
}
