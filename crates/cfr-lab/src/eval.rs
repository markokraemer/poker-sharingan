//! Exact evaluation: profile values, best response, exploitability / NashConv.
//!
//! Best response for player `p` against a fixed profile is computed by tree walks that
//! carry the opponents' (and chance's) reach probability. Each walk accumulates, per
//! `p`-infoset and action, the counterfactual value `sum_h pi_{-p}(h) u_p(h.a, BR)` under
//! the best-response choices of the previous pass; passes repeat until the choices are
//! stable (perfect recall => at most `depth` passes), so the result is exact.
//! 2-player zero-sum: exploitability = (BR_1 + BR_2) / 2 in game units (chips per hand);
//! it equals NashConv / 2 because v_1 + v_2 = 0. n-player: NashConv = sum_i (BR_i - v_i).

use crate::tree::{Kind, Tree};

/// Expected utility of every player when all follow `profile`.
pub fn values(tree: &Tree, profile: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; tree.n_players];
    walk_values(tree, Tree::ROOT, 1.0, profile, &mut out);
    out
}

fn walk_values(t: &Tree, id: u32, reach: f64, prof: &[f64], out: &mut [f64]) {
    if reach == 0.0 {
        return;
    }
    let n = &t.nodes[id as usize];
    match n.kind {
        Kind::Terminal => {
            for (p, o) in out.iter_mut().enumerate() {
                *o += reach * t.utility(n, p);
            }
        }
        Kind::Chance => {
            for (&c, &pr) in t.kids(n).iter().zip(t.kid_probs(n)) {
                walk_values(t, c, reach * pr, prof, out);
            }
        }
        Kind::Player => {
            let i = &t.info[n.infoset as usize];
            for (a, &c) in t.kids(n).iter().enumerate() {
                walk_values(t, c, reach * prof[i.offset + a], prof, out);
            }
        }
    }
}

/// Value of player `p`'s best response against `profile` (others fixed).
pub fn best_response(tree: &Tree, profile: &[f64], p: usize) -> f64 {
    let mut choice = vec![0u32; tree.info.len()];
    let mut cf = vec![0.0; tree.total_actions];
    let mut v = 0.0;
    for _ in 0..64 {
        cf.fill(0.0);
        v = br_walk(tree, Tree::ROOT, p, 1.0, profile, &choice, &mut cf);
        let mut changed = false;
        for (k, i) in tree.info.iter().enumerate().filter(|(_, i)| i.player == p) {
            let s = &cf[i.offset..i.offset + i.n_actions];
            let mut best = 0;
            for a in 1..s.len() {
                if s[a] > s[best] + 1e-12 {
                    best = a;
                }
            }
            if choice[k] != best as u32 {
                choice[k] = best as u32;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    v
}

fn br_walk(
    t: &Tree,
    id: u32,
    p: usize,
    reach: f64,
    prof: &[f64],
    choice: &[u32],
    cf: &mut [f64],
) -> f64 {
    if reach == 0.0 {
        return 0.0;
    }
    let n = &t.nodes[id as usize];
    match n.kind {
        Kind::Terminal => reach * t.utility(n, p),
        Kind::Chance => t
            .kids(n)
            .iter()
            .zip(t.kid_probs(n))
            .map(|(&c, &pr)| br_walk(t, c, p, reach * pr, prof, choice, cf))
            .sum(),
        Kind::Player => {
            let i = &t.info[n.infoset as usize];
            if i.player == p {
                let mut chosen = 0.0;
                for (a, &c) in t.kids(n).iter().enumerate() {
                    let v = br_walk(t, c, p, reach, prof, choice, cf);
                    cf[i.offset + a] += v;
                    if a as u32 == choice[n.infoset as usize] {
                        chosen = v;
                    }
                }
                chosen
            } else {
                t.kids(n)
                    .iter()
                    .enumerate()
                    .map(|(a, &c)| br_walk(t, c, p, reach * prof[i.offset + a], prof, choice, cf))
                    .sum()
            }
        }
    }
}

/// `sum_i (BR_i - v_i)`; for 2-player zero-sum games this equals `2 * exploitability`.
pub fn nash_conv(tree: &Tree, profile: &[f64]) -> f64 {
    let v = values(tree, profile);
    (0..tree.n_players)
        .map(|p| best_response(tree, profile, p) - v[p])
        .sum()
}

/// 2-player zero-sum exploitability `(BR_1 + BR_2) / 2` in game units.
pub fn exploitability(tree: &Tree, profile: &[f64]) -> f64 {
    assert_eq!(tree.n_players, 2);
    nash_conv(tree, profile) / 2.0
}

pub fn uniform_exploitability(tree: &Tree) -> f64 {
    nash_conv(tree, &tree.uniform_profile()) / tree.n_players.min(2) as f64
}
