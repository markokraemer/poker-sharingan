//! Full-traversal CFR family with **alternating updates** (players take turns being the
//! updating player within an iteration; the opponents play their current strategy).
//!
//! * `Vanilla`  - Zinkevich et al. 2007: regrets and average accumulate with weight 1.
//! * `Plus`     - CFR+ (Tammelin 2014): regrets clamped to >= 0 after every update, average
//!                weighted by iteration `t`.
//! * `Linear`   - LCFR (Brown & Sandholm 2019, AAAI "Solving Imperfect-Information Games
//!                via Discounted Regret Minimization"): iteration `t` weighted by `t` for
//!                both regrets and the average strategy (equivalent to discounting old
//!                iterations by `t/(t+1)`; this is the schedule Pluribus approximates).
//! * `Dcfr`     - DCFR(alpha, beta, gamma), same paper; (1.5, 0, 2) is the recommended
//!                setting. After each iteration positive regrets are multiplied by
//!                `t^a/(t^a+1)`, negative by `t^b/(t^b+1)`, average by `(t/(t+1))^g`.
//!
//! Regret increments are collected in a buffer during a traversal and applied afterwards,
//! so every visit to an infoset within one traversal sees the same strategy.

use crate::tree::{Kind, Profile, Tree};
use crate::{Solver, MAX_ACTIONS};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Variant {
    Vanilla,
    Plus,
    Linear,
    Dcfr { alpha: f64, beta: f64, gamma: f64 },
}

impl Variant {
    pub const DCFR: Variant = Variant::Dcfr {
        alpha: 1.5,
        beta: 0.0,
        gamma: 2.0,
    };
}

pub struct Cfr<'a> {
    tree: &'a Tree,
    pub variant: Variant,
    regret: Vec<f64>,
    delta: Vec<f64>,
    sum: Vec<f64>,
    t: u64,
}

pub(crate) fn regret_match(r: &[f64], out: &mut [f64]) {
    let tot: f64 = r.iter().map(|x| x.max(0.0)).sum();
    for (o, &x) in out.iter_mut().zip(r) {
        *o = if tot > 0.0 {
            x.max(0.0) / tot
        } else {
            1.0 / r.len() as f64
        };
    }
}

impl<'a> Cfr<'a> {
    pub fn new(tree: &'a Tree, variant: Variant) -> Self {
        let n = tree.total_actions;
        Cfr {
            tree,
            variant,
            regret: vec![0.0; n],
            delta: vec![0.0; n],
            sum: vec![0.0; n],
            t: 0,
        }
    }

    /// Returns player `p`'s expected utility below `id`. `opp` = reach of everyone but `p`
    /// (chance included), `own` = `p`'s own reach.
    fn walk(&mut self, id: u32, p: usize, opp: f64, own: f64, ws: f64) -> f64 {
        let tree = self.tree;
        let n = &tree.nodes[id as usize];
        match n.kind {
            Kind::Terminal => tree.utility(n, p),
            Kind::Chance => {
                let mut v = 0.0;
                for (&c, &pr) in tree.kids(n).iter().zip(tree.kid_probs(n)) {
                    v += pr * self.walk(c, p, opp * pr, own, ws);
                }
                v
            }
            Kind::Player => {
                if opp == 0.0 && own == 0.0 {
                    return 0.0;
                }
                let info = &tree.info[n.infoset as usize];
                let (off, na) = (info.offset, info.n_actions);
                let mut sigma = [0.0; MAX_ACTIONS];
                regret_match(&self.regret[off..off + na], &mut sigma[..na]);
                let mine = info.player == p;
                let mut vals = [0.0; MAX_ACTIONS];
                let mut v = 0.0;
                for (a, &c) in tree.kids(n).iter().enumerate() {
                    vals[a] = if mine {
                        self.walk(c, p, opp, own * sigma[a], ws)
                    } else {
                        self.walk(c, p, opp * sigma[a], own, ws)
                    };
                    v += sigma[a] * vals[a];
                }
                if mine {
                    for a in 0..na {
                        self.delta[off + a] += opp * (vals[a] - v);
                        self.sum[off + a] += ws * own * sigma[a];
                    }
                }
                v
            }
        }
    }
}

impl Solver for Cfr<'_> {
    fn iterate(&mut self) {
        self.t += 1;
        let t = self.t as f64;
        let (wr, ws) = match self.variant {
            Variant::Linear => (t, t),
            Variant::Plus => (1.0, t),
            _ => (1.0, 1.0),
        };
        for p in 0..self.tree.n_players {
            self.walk(Tree::ROOT, p, 1.0, 1.0, ws);
            let plus = self.variant == Variant::Plus;
            for (r, d) in self.regret.iter_mut().zip(self.delta.iter_mut()) {
                *r += wr * *d;
                if plus {
                    *r = r.max(0.0);
                }
                *d = 0.0;
            }
        }
        if let Variant::Dcfr { alpha, beta, gamma } = self.variant {
            let (pa, pb) = (t.powf(alpha), t.powf(beta));
            let (dp, dn) = (pa / (pa + 1.0), pb / (pb + 1.0));
            for r in &mut self.regret {
                *r *= if *r > 0.0 { dp } else { dn };
            }
            let ds = (t / (t + 1.0)).powf(gamma);
            self.sum.iter_mut().for_each(|s| *s *= ds);
        }
    }
    fn iterations(&self) -> u64 {
        self.t
    }
    fn average(&self) -> Profile {
        self.tree.normalise(&self.sum)
    }
    fn current(&self) -> Profile {
        let mut out = vec![0.0; self.tree.total_actions];
        for i in &self.tree.info {
            let r = i.offset..i.offset + i.n_actions;
            regret_match(&self.regret[r.clone()], &mut out[r]);
        }
        out
    }
}
