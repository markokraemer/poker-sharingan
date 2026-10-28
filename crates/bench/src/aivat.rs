//! AIVAT (Burch et al. 2018) in the multi-player form of the Pluribus supplement.
//!
//! For one hand, with realised payoffs `u` (per seat), the estimate for every seat is
//!
//! ```text
//!   u  +  sum over chance nodes   ( E_c[v(h.c)] - v(h.c*) )
//!      +  sum over known-policy decisions  ( sum_a sigma(a) v(h.a) - v(h.a*) )
//! ```
//!
//! where `v` is any [`ValueEstimator`] that only looks at what has been dealt/played so far
//! (never at undealt board cards). Each bracketed term has conditional mean zero (the sampled
//! card/action is drawn from exactly the distribution inside the sum), so the estimate is
//! unbiased for any `v`; a good `v` merely lowers the variance. Chance nodes are the hole-card
//! deal and each board reveal (including all-in run-outs); decision nodes are those of players
//! that expose their probabilities. Decisions of non-exposing players get no correction.
//!
//! [`EquityEstimator`] is the default baseline: every live player calls the outstanding bet and
//! the hand is checked down; the value is the exact expected (fractional-chop) payoff given
//! all hole cards, enumerating the remaining board postflop and sampling it preflop. Its
//! chance-node mean is exact by the tower property (E_c[v(h.c)] = v(h)). Fractional chop means
//! AIVAT's target differs from integer-chip payoffs only by odd-chip rounding (< 0.5 chip on
//! a tied odd pot, i.e. << 0.1 mbb).
//!
//! `Mode::CardsOnly` is the paper's simpler baseline: only the deal is corrected.

use crate::policy::Dist;
use cards::{bit, evaluate, mask_of, Card, Rng, FULL_DECK};
use nlhe::{Deal, State, MAX_PLAYERS};
use std::collections::HashMap;

pub struct Query<'a> {
    pub state: &'a State,
    pub holes: &'a [[Card; 2]; MAX_PLAYERS],
    /// Board cards revealed at this node (never pass undealt cards).
    pub board: &'a [Card],
}

/// Per-deal scratch: independent RNG for sampling inside estimators and a result cache.
pub struct Scratch {
    pub rng: Rng,
    cache: HashMap<(u8, u8), [f64; MAX_PLAYERS]>,
}

impl Scratch {
    pub fn new(seed: u64) -> Self {
        Scratch {
            rng: Rng::new(seed),
            cache: HashMap::new(),
        }
    }
}

pub trait ValueEstimator: Send + Sync {
    fn value(&self, q: &Query, sc: &mut Scratch) -> [f64; MAX_PLAYERS];

    /// E over the next board cards (up to `next_len` visible) of `value`. Default: Monte Carlo
    /// with fresh independent samples (unbiased).
    fn chance_mean(&self, q: &Query, next_len: usize, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        const M: usize = 32;
        let n = q.state.num_players();
        let dead = q.holes[..n]
            .iter()
            .fold(mask_of(q.board), |m, h| m | bit(h[0]) | bit(h[1]));
        let mut acc = [0.0; MAX_PLAYERS];
        for _ in 0..M {
            let mut b = q.board.to_vec();
            let mut d = dead;
            while b.len() < next_len {
                let c = sc.rng.draw(d);
                d |= bit(c);
                b.push(c);
            }
            let v = self.value(
                &Query {
                    state: q.state,
                    holes: q.holes,
                    board: &b,
                },
                sc,
            );
            for i in 0..MAX_PLAYERS {
                acc[i] += v[i] / M as f64;
            }
        }
        acc
    }

    /// E over all hole-card deals of `value(root, cards, [])`. Default: Monte Carlo.
    fn deal_mean(&self, root: &State, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        const M: usize = 128;
        let mut acc = [0.0; MAX_PLAYERS];
        for _ in 0..M {
            let d = Deal::random(&mut sc.rng);
            let v = self.value(
                &Query {
                    state: root,
                    holes: &d.holes,
                    board: &[],
                },
                sc,
            );
            for i in 0..MAX_PLAYERS {
                acc[i] += v[i] / M as f64;
            }
        }
        acc
    }
}

/// Check-down equity baseline (see module docs).
pub struct EquityEstimator {
    /// Preflop board samples (postflop is exact enumeration).
    pub preflop_samples: usize,
}

impl Default for EquityEstimator {
    fn default() -> Self {
        EquityEstimator {
            preflop_samples: 256,
        }
    }
}

/// Call-up totals: everyone live and not all-in matches the outstanding bet.
fn callup_totals(s: &State) -> [i32; MAX_PLAYERS] {
    let mut t = [0; MAX_PLAYERS];
    for (i, ti) in t.iter_mut().enumerate().take(s.num_players()) {
        *ti = s.contributed(i);
        if !s.is_folded(i) && !s.is_all_in(i) {
            *ti += (s.cur_bet() - s.street_bet(i)).clamp(0, s.stack(i));
        }
    }
    t
}

impl EquityEstimator {
    /// Expected fractional win share of each member of `elig` (bitmask) at showdown.
    fn shares(&self, q: &Query, elig: u8, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        let len = q.board.len();
        if let Some(v) = sc.cache.get(&(elig, len as u8)) {
            return *v;
        }
        let n = q.state.num_players();
        let members: Vec<usize> = (0..n).filter(|&i| elig >> i & 1 != 0).collect();
        let bm = mask_of(q.board);
        let hm: Vec<u64> = (0..n)
            .map(|i| bit(q.holes[i][0]) | bit(q.holes[i][1]))
            .collect();
        let dead = hm.iter().fold(bm, |m, h| m | h);
        let mut out = [0.0; MAX_PLAYERS];
        let mut count = 0.0;
        let score = |run: u64, out: &mut [f64; MAX_PLAYERS]| {
            let mut best = 0;
            let mut w = [0usize; MAX_PLAYERS];
            let mut nw = 0;
            for &i in &members {
                let r = evaluate(hm[i] | bm | run);
                if r > best {
                    best = r;
                    nw = 0;
                }
                if r == best {
                    w[nw] = i;
                    nw += 1;
                }
            }
            for &i in &w[..nw] {
                out[i] += 1.0 / nw as f64;
            }
        };
        let live: Vec<Card> = cards::cards_in(FULL_DECK & !dead).collect();
        match 5 - len {
            0 => {
                score(0, &mut out);
                count = 1.0;
            }
            1 => {
                for &c in &live {
                    score(bit(c), &mut out);
                    count += 1.0;
                }
            }
            2 => {
                for (a, &c1) in live.iter().enumerate() {
                    for &c2 in &live[a + 1..] {
                        score(bit(c1) | bit(c2), &mut out);
                        count += 1.0;
                    }
                }
            }
            _ => {
                for _ in 0..self.preflop_samples {
                    let mut run = 0;
                    for _ in 0..5 - len {
                        run |= bit(sc.rng.draw(dead | run));
                    }
                    score(run, &mut out);
                    count += 1.0;
                }
            }
        }
        out.iter_mut().for_each(|x| *x /= count);
        sc.cache.insert((elig, len as u8), out);
        out
    }
}

impl ValueEstimator for EquityEstimator {
    fn value(&self, q: &Query, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        let s = q.state;
        let n = s.num_players();
        let mut out = [0.0; MAX_PLAYERS];
        if s.live_count() == 1 {
            for (i, o) in out.iter_mut().enumerate().take(n) {
                *o = -(s.contributed(i) as f64);
                if !s.is_folded(i) {
                    *o += s.pot() as f64;
                }
            }
            return out;
        }
        let t = callup_totals(s);
        let live = (0..n)
            .filter(|&i| !s.is_folded(i))
            .fold(0u8, |m, i| m | 1 << i);
        let mut levels: Vec<i32> = (0..n)
            .filter(|&i| live >> i & 1 != 0)
            .map(|i| t[i])
            .collect();
        levels.sort_unstable();
        levels.dedup();
        let mut prev = 0;
        for &lv in &levels {
            if lv <= prev {
                continue;
            }
            let pot: i32 = (0..n).map(|i| t[i].min(lv) - t[i].min(prev)).sum();
            let elig = (0..n)
                .filter(|&i| live >> i & 1 != 0 && t[i] >= lv)
                .fold(0u8, |m, i| m | 1 << i);
            if elig.count_ones() == 1 {
                out[elig.trailing_zeros() as usize] += pot as f64;
            } else {
                let sh = self.shares(q, elig, sc);
                for i in 0..n {
                    out[i] += pot as f64 * sh[i];
                }
            }
            prev = lv;
        }
        for i in 0..n {
            out[i] -= t[i] as f64;
        }
        out
    }

    fn chance_mean(&self, q: &Query, _next_len: usize, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        self.value(q, sc) // tower property
    }

    fn deal_mean(&self, root: &State, sc: &mut Scratch) -> [f64; MAX_PLAYERS] {
        let t = callup_totals(root);
        let n = root.num_players();
        let live: Vec<i32> = (0..n)
            .filter(|&i| !root.is_folded(i))
            .map(|i| t[i])
            .collect();
        if live.len() == n && live.iter().all(|&x| x == live[0]) {
            return [0.0; MAX_PLAYERS]; // exchangeable seats: share = pot/n = own total
        }
        let mut acc = [0.0; MAX_PLAYERS];
        for _ in 0..128 {
            let d = Deal::random(&mut sc.rng);
            let v = self.value(
                &Query {
                    state: root,
                    holes: &d.holes,
                    board: &[],
                },
                &mut Scratch::new(sc.rng.next_u64()),
            );
            for i in 0..MAX_PLAYERS {
                acc[i] += v[i] / 128.0;
            }
        }
        acc
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Chance nodes and every known-policy decision.
    Full,
    /// Only the hole-card deal (the paper's per-human baseline).
    CardsOnly,
}

/// Accumulates the AIVAT correction over one hand.
pub struct Tracker<'a> {
    pub est: &'a dyn ValueEstimator,
    pub sc: &'a mut Scratch,
    pub mode: Mode,
    pub deal: Deal,
    pub corr: [f64; MAX_PLAYERS],
}

impl<'a> Tracker<'a> {
    pub fn new(est: &'a dyn ValueEstimator, sc: &'a mut Scratch, mode: Mode, deal: Deal) -> Self {
        Tracker {
            est,
            sc,
            mode,
            deal,
            corr: [0.0; MAX_PLAYERS],
        }
    }
    fn val(&mut self, st: &State, len: usize) -> [f64; MAX_PLAYERS] {
        let (est, deal) = (self.est, self.deal);
        est.value(
            &Query {
                state: st,
                holes: &deal.holes,
                board: &deal.board[..len],
            },
            self.sc,
        )
    }
    pub fn start(&mut self, root: &State) {
        let m = self.est.deal_mean(root, self.sc);
        let v = self.val(root, 0);
        for i in 0..MAX_PLAYERS {
            self.corr[i] += m[i] - v[i];
        }
    }
    /// A known-policy player at `st` drew entry `chosen` of `dist`.
    pub fn decision(&mut self, st: &State, len: usize, dist: &Dist, chosen: usize) {
        if self.mode != Mode::Full {
            return;
        }
        let mut sampled = [0.0; MAX_PLAYERS];
        let mut mean = [0.0; MAX_PLAYERS];
        for a in 0..dist.len() {
            let mut s2 = *st;
            s2.apply(dist.action(a));
            let v = self.val(&s2, len);
            let p = dist.prob(a);
            for i in 0..MAX_PLAYERS {
                mean[i] += p * v[i];
                if a == chosen {
                    sampled[i] = v[i];
                }
            }
        }
        for i in 0..MAX_PLAYERS {
            self.corr[i] += mean[i] - sampled[i];
        }
    }
    /// Board reveal from `from` to `to` visible cards at (post-action) state `st`.
    pub fn chance(&mut self, st: &State, from: usize, to: usize) {
        if self.mode != Mode::Full {
            return;
        }
        let deal = self.deal;
        let q = Query {
            state: st,
            holes: &deal.holes,
            board: &deal.board[..from],
        };
        let m = self.est.chance_mean(&q, to, self.sc);
        let v = self.val(st, to);
        for i in 0..MAX_PLAYERS {
            self.corr[i] += m[i] - v[i];
        }
    }
}
