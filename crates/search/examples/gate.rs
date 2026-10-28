//! Correctness gates for the search solvers, measured with exact best response.
//!
//! cargo run --release -p search --example gate -- --street river|turn|flop --spots 6 \
//!     --iters-mc 5000,100000 --iters-vec 50,200,1000 --ranges bp|uniform [--no-mc] [--ref]
//!
//! Exploitability is exact (all 1326 hands, every board runout enumerated) on the SAME tree the
//! solvers use (`search::vector::exploitability`; for rivers it is cross-checked against the
//! referee's own best response). Reports % of the root pot, mean over spots.
use blueprint::store::Store;
use nlhe::abstraction::BetSizing;
use nlhe::{GameConfig, Street};
use search::beliefs::{holes, Beliefs};
use search::bp::Blueprint;
use search::solver::{solve, SearchParams, Solution, Subgame};
use search::testkit::{best_bucketer, sample_spot, Spot};
use search::tree::step_path;
use search::vector::{exploitability, solve_vec, Bucketing, Query, Strat, VecGame, VecParams};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn args() -> HashMap<String, String> {
    let mut kv = HashMap::new();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < a.len() {
        let k = a[i].trim_start_matches("--").to_string();
        if i + 1 < a.len() && !a[i + 1].starts_with("--") {
            kv.insert(k, a[i + 1].clone());
            i += 2;
        } else {
            kv.insert(k, "true".into());
            i += 1;
        }
    }
    kv
}

/// MC solution as a full strategy: root round by exact hole, later rounds by blueprint bucket.
struct McStrat<'a> {
    sol: &'a Solution,
    bp: &'a Blueprint,
    fin: bool,
    root: Street,
}

impl Strat for McStrat<'_> {
    fn probs(&self, q: &Query, n: usize, out: &mut [f64]) {
        let m = q.hands.len();
        let hl = holes();
        let mut buf = [0f32; 24];
        for h in 0..m {
            let hi = q.hands.idx[h] as usize;
            let ok = if q.street == self.root {
                self.sol.probs(q.akey, hi, n, self.fin, &mut buf)
            } else {
                let b = self.bp.bucket(q.street, hl[hi], q.board);
                self.sol.probs_later(q.akey, q.street, b, n, self.fin, &mut buf)
            };
            for a in 0..n {
                out[a * m + h] = if ok { buf[a] as f64 } else { 1.0 / n as f64 };
            }
        }
    }
}

/// The blueprint itself as a strategy on the search tree (search actions translated to its
/// abstraction), exact hole pairs mapped to its buckets.
struct BpStrat<'a> {
    bp: &'a Blueprint,
    root: nlhe::State,
    bph: nlhe::abstraction::Shadow,
    prm: &'a SearchParams,
}

impl Strat for BpStrat<'_> {
    fn probs(&self, q: &Query, n: usize, out: &mut [f64]) {
        let mut st = self.root;
        let mut bph = self.bph;
        for &a in q.path {
            self.bp.observe(&mut bph, &st, a, false);
            st.apply(a);
        }
        let mut list = nlhe::abstraction::ActionList::new();
        let sz = if q.st.street() == self.root.street() { &self.prm.sizing } else { &self.prm.later_sizing };
        search::tree::search_actions(q.st, sz, &[], &mut list);
        assert_eq!(list.len(), n);
        let m = q.hands.len();
        let hl = holes();
        for h in 0..m {
            let bucket = self.bp.bucket(q.street, hl[q.hands.idx[h] as usize], q.board);
            let pr = self.bp.prior(&bph, q.st, &list, bucket);
            for a in 0..n {
                out[a * m + h] = pr.map_or(1.0 / n as f64, |p| p[a] as f64);
            }
        }
    }
}

fn main() {
    let kv = args();
    let get = |k: &str, d: &str| kv.get(k).cloned().unwrap_or(d.into());
    let street = match get("street", "river").as_str() {
        "river" => Street::River,
        "turn" => Street::Turn,
        "flop" => Street::Flop,
        o => panic!("street {o}"),
    };
    let nspots: usize = get("spots", "4").parse().unwrap();
    let seed: u64 = get("seed", "1").parse().unwrap();
    let list = |k: &str, d: &str| -> Vec<u64> { get(k, d).split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect() };
    let it_mc = list("iters-mc", "5000,100000");
    let it_vec = list("iters-vec", "50,200,1000");
    let cfg = GameConfig::new(2);
    let (bucketer, _) = best_bucketer();
    let bsz = if get("sizing", "tiny") == "local" { pluribus::sizing("local") } else { BetSizing::tiny() };
    let p = PathBuf::from(get("bp-cache", "data/search-bp/hu_tiny_3m.bin"));
    let store: Arc<Store> = Arc::new(blueprint::io::load(&p, 1 << 27, 1 << 24).expect("load"));
    let bp = Blueprint::new(store, bucketer, bsz);
    let uniform = get("ranges", "bp") == "uniform";
    let do_mc = !kv.contains_key("no-mc");

    let mut agg: HashMap<String, (f64, f64, usize)> = HashMap::new();
    let mut k = 0u64;
    let mut done = 0;
    while done < nspots {
        k += 1;
        let sseed = bench::mix(seed, k);
        let Some(sp) = sample_spot(&bp, &cfg, street, sseed) else { continue };
        done += 1;
        let Spot { root, bph, board, mut beliefs } = sp;
        if uniform {
            beliefs = Beliefs::uniform(2);
            beliefs.remove_cards(cards::mask_of(&board));
        }
        let pot = root.pot() as f64;
        let prm0 = SearchParams::default();
        let ranges = [1usize, 0].map(|s| beliefs.w[s].iter().map(|&x| x as f64).collect::<Vec<_>>());
        let none = Default::default();
        let frozen = HashMap::new();
        // player 0 = seat 1 (first to act postflop heads-up)
        let g = VecGame {
            root,
            board: &board,
            seats: [1, 0],
            ranges: ranges.clone(),
            sizing: &prm0.sizing,
            later: &prm0.later_sizing,
            extras: &none,
            me: None,
            my_hole: None,
            frozen: &frozen,
            pooled: None,
        };
        let supp = |s: usize| beliefs.w[s].iter().filter(|&&x| x > 0.0).count();
        println!("== spot {done} (seed {sseed}) board {} pot {pot} supp {}/{}", cards::cards_str(&board), supp(1), supp(0));
        let mut rec = |name: &str, it: u64, expl: f64, ms: f64| {
            println!("  {name:<20} iters {it:>8}  expl {:8.3}% pot  {ms:8.0} ms", 100.0 * expl / pot);
            let e = agg.entry(format!("{name} {it:>8}")).or_insert((0.0, 0.0, 0));
            e.0 += 100.0 * expl / pot;
            e.1 += ms;
            e.2 += 1;
        };
        if street == Street::River && kv.contains_key("ref") {
            let b5: [cards::Card; 5] = board.clone().try_into().unwrap();
            let rg = referee::solver::Game::new(root, [1, 0], b5, ranges.clone(), &prm0.sizing, cfg.big_blind).expect("game");
            for &it in &[50u64, 200, 1000] {
                let t = Instant::now();
                let s = rg.solve(it as usize);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                rec("ref-vector-cfr+", it, rg.exploitability(&s), ms);
            }
        }
        {
            let st = BpStrat { bp: &bp, root, bph, prm: &prm0 };
            let (e, _) = exploitability(&g, &st);
            rec("blueprint", 0, e, 0.0);
        }
        for &(name, plus, pooled) in &[("vec-lcfr", false, false), ("vec-cfr+", true, false), ("vecP-cfr+", true, true)] {
            if (plus && !pooled && !kv.contains_key("plus")) || (pooled && !kv.contains_key("pooled")) {
                continue;
            }
            let pb: Option<&dyn Bucketing> = if pooled { Some(&bp) } else { None };
            let gv = VecGame {
                root,
                board: &board,
                seats: [1, 0],
                ranges: ranges.clone(),
                sizing: &prm0.sizing,
                later: &prm0.later_sizing,
                extras: &none,
                me: None,
                my_hole: None,
                frozen: &frozen,
                pooled: pb,
            };
            for &it in &it_vec {
                let vp = VecParams { iters: it, plus, seed: 3, keep_all: true, ..Default::default() };
                let t = Instant::now();
                let sol = solve_vec(&gv, &vp);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let (ea, _) = exploitability(&g, &sol.strategy(true, pb));
                rec(&format!("{name}-avg"), it, ea, ms);
                let (ef, _) = exploitability(&g, &sol.strategy(false, pb));
                rec(&format!("{name}-final"), it, ef, ms);
            }
        }
        if do_mc {
            let variants: [(&str, f32, f32, bool); 3] = [("mc-default-final", 8.0, 0.95, true), ("mc-default-avg", 8.0, 0.95, false), ("mc-clean-avg", 0.0, 0.0, false)];
            for (name, warm, prune, fin) in variants {
                for &it in &it_mc {
                    let prm = SearchParams { iters: it, warm_start: warm, prune_prob: prune, max_infosets: 1 << 20, seed: 3, vector: false, ..Default::default() };
                    let sg = Subgame { bp: &bp, root, board: &board, beliefs: &beliefs, bp_hist: bph, extras: &none, me: 0, my_hole: None, frozen: &frozen, params: &prm, seed: 3 };
                    let t = Instant::now();
                    let sol = solve(&sg);
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    let st = McStrat { sol: &sol, bp: &bp, fin, root: street };
                    let (e, _) = exploitability(&g, &st);
                    rec(name, it, e, ms);
                    if street == Street::River && kv.contains_key("ref") {
                        // cross-check of our exact BR against the referee's on the same strategy
                        let b5: [cards::Card; 5] = board.clone().try_into().unwrap();
                        let rg = referee::solver::Game::new(root, [1, 0], b5, ranges.clone(), &prm0.sizing, cfg.big_blind).unwrap();
                        let paths: Vec<u64> = rg.tree.nodes.iter().map(|nd| nd.path.iter().fold(0u64, |p, &a| step_path(p, a))).collect();
                        let mut cache: (usize, usize, [f32; 24]) = (usize::MAX, usize::MAX, [0.0; 24]);
                        let s = referee::solver::Strategy::from_fn(&rg.tree, |id, a, n, h| {
                            if cache.0 != id || cache.1 != h {
                                if !sol.probs(paths[id], h, n, fin, &mut cache.2) {
                                    cache.2[..n].iter_mut().for_each(|x| *x = 1.0 / n as f32);
                                }
                                cache.0 = id;
                                cache.1 = h;
                            }
                            cache.2[a] as f64
                        });
                        let er = rg.exploitability(&s);
                        println!("     cross-check: mine {e:.4} referee {er:.4} chips");
                    }
                }
            }
        }
    }
    println!("\n== MEAN over {done} spots, {street:?} (expl % of pot) ==");
    let mut keys: Vec<_> = agg.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let (e, ms, n) = agg[&k];
        println!("  {k:<30} {:8.3}%   {:8.0} ms", e / n as f64, ms / n as f64);
    }
}
