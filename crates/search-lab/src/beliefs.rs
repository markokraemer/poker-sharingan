//! Beliefs from an outside observer's view (supp.: "probability distribution over pairs of
//! private cards ... updated using Bayes' rule based on strategy profile sigma").
//!
//! The joint reach `pi^sigma(h) = chance * prod_i sigma_i` of every tree node is computed
//! in one forward pass; normalising it over the nodes of a public state is exactly Bayes'
//! rule with card removal (impossible deals simply have no node).

use crate::view::{View, NONE};
use cfr_lab::tree::{Kind, Tree};

/// Joint reach probability of every node under `profile` (chance included).
pub fn reach(tree: &Tree, profile: &[f64]) -> Vec<f64> {
    let mut r = vec![0.0; tree.nodes.len()];
    r[0] = 1.0;
    for (id, n) in tree.nodes.iter().enumerate() {
        if n.kind == Kind::Terminal || r[id] == 0.0 {
            continue;
        }
        let probs = tree.kid_probs(n);
        let off = (n.kind == Kind::Player).then(|| tree.info[n.infoset as usize].offset);
        for (a, &c) in tree.kids(n).iter().enumerate() {
            let p = match off {
                Some(o) => profile[o + a],
                None => probs[a],
            };
            r[c as usize] = r[id] * p;
        }
    }
    r
}

/// Decision nodes of public state `pkey`.
pub fn state_nodes(tree: &Tree, view: &View, pkey: u32) -> Vec<u32> {
    (0..tree.nodes.len() as u32)
        .filter(|&i| tree.nodes[i as usize].kind == Kind::Player && view.pkey[i as usize] == pkey)
        .collect()
}

/// P(player `p` holds card c | public state), for the given decision nodes.
pub fn marginal(view: &View, reach: &[f64], nodes: &[u32], p: usize, deck: usize) -> Vec<f64> {
    let mut m = vec![0.0; deck];
    for &h in nodes {
        let c = view.cards[h as usize][p];
        debug_assert!(c != NONE);
        m[c as usize] += reach[h as usize];
    }
    let tot: f64 = m.iter().sum();
    if tot > 0.0 {
        m.iter_mut().for_each(|x| *x /= tot);
    }
    m
}
