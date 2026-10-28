//! Poker-specific facts the search needs that cfr-lab's `Game` trait does not expose.
//! Convention (satisfied by Leduc): the first `players()` chance nodes deal private cards
//! in seat order, later chance nodes deal public cards; outcome `k` of a deal is the
//! k-th still-undealt card in ascending order.

use cfr_lab::leduc::Leduc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActKind {
    Fold,
    Call,
    Raise,
}

pub trait Poker {
    fn players(&self) -> usize;
    fn deck(&self) -> usize;
    /// Kinds of the actions at an infoset, in action order (needed to bias continuations).
    fn action_kinds(&self, key: &str, n_actions: usize) -> Vec<ActKind>;
}

/// Raises per round in cfr-lab's Leduc (private const there).
const LEDUC_MAX_RAISES: usize = 2;

impl Poker for Leduc {
    fn players(&self) -> usize {
        self.players
    }
    fn deck(&self) -> usize {
        self.ranks * 2
    }
    /// Leduc legal actions are `[Fold if facing a bet] Call [Raise if raises < 2]`; the
    /// round-local raise count comes from the history in the infoset key.
    fn action_kinds(&self, key: &str, n: usize) -> Vec<ActKind> {
        let hist = key.rsplit('|').next().unwrap();
        let local = hist.rsplit('/').next().unwrap();
        let can_raise = local.bytes().filter(|&b| b == b'r').count() < LEDUC_MAX_RAISES;
        let fold = n - 1 - can_raise as usize;
        let mut v = vec![];
        if fold == 1 {
            v.push(ActKind::Fold);
        }
        v.push(ActKind::Call);
        if can_raise {
            v.push(ActKind::Raise);
        }
        assert_eq!(v.len(), n, "action kinds for {key}");
        v
    }
}
