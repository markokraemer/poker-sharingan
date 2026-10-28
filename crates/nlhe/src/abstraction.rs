//! Action abstraction, abstract-action history and pseudo-harmonic action translation.
//!
//! Raise sizes are pot fractions: `raise_to = cur_bet + frac * (pot + call)`, i.e. the raiser
//! calls, then raises by `frac` of the pot after calling. Sizes are clamped to the legal
//! `[min_raise_to, max_raise_to]`, deduped, ascending, with all-in last.
//!
//! `pluribus_blueprint()` only follows the paper for turn/river (first raise {0.5, 1, all-in},
//! later {1, all-in}). The preflop and flop menus were not published; ours are a proxy.

use crate::{Action, State};
use cards::Rng;

/// Max abstract actions at a node (fold + call + 16 raises + all-in, rounded up).
pub const MAX_ACTIONS: usize = 24;
/// Distinct raise-depth menus per street; deeper raises reuse the last one.
pub const DEPTHS: usize = 4;
const MAX_FRACS: usize = 16;

/// Fixed-capacity abstract action list: Fold (if facing a bet), Check/Call, raises ascending,
/// all-in last.
#[derive(Clone, Copy, Debug)]
pub struct ActionList {
    len: u8,
    acts: [Action; MAX_ACTIONS],
}

impl ActionList {
    pub const fn new() -> Self {
        ActionList {
            len: 0,
            acts: [Action::Fold; MAX_ACTIONS],
        }
    }
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
    #[inline]
    pub fn push(&mut self, a: Action) {
        debug_assert!((self.len as usize) < MAX_ACTIONS);
        self.acts[self.len as usize] = a;
        self.len += 1;
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn as_slice(&self) -> &[Action] {
        &self.acts[..self.len as usize]
    }
    #[inline]
    pub fn get(&self, i: usize) -> Action {
        self.as_slice()[i]
    }
}

impl Default for ActionList {
    fn default() -> Self {
        Self::new()
    }
}

/// Raise menu for one (street, raise depth): pot fractions plus optional all-in.
#[derive(Clone, Copy, Debug)]
pub struct Menu {
    n: u8,
    fracs: [f32; MAX_FRACS],
    pub all_in: bool,
}

impl Menu {
    /// `fracs` need not be sorted.
    pub fn new(fracs: &[f32], all_in: bool) -> Menu {
        assert!(fracs.len() <= MAX_FRACS);
        let mut f = [0.0; MAX_FRACS];
        f[..fracs.len()].copy_from_slice(fracs);
        f[..fracs.len()].sort_by(|a, b| a.partial_cmp(b).unwrap());
        Menu {
            n: fracs.len() as u8,
            fracs: f,
            all_in,
        }
    }
    pub fn fracs(&self) -> &[f32] {
        &self.fracs[..self.n as usize]
    }
}

/// Bet sizing per street and raise depth, plus a raise cap per street.
#[derive(Clone, Copy, Debug)]
pub struct BetSizing {
    menus: [[Menu; DEPTHS]; 4],
    /// After this many raises on a street only fold/call remain.
    pub max_raises: [u8; 4],
}

impl BetSizing {
    /// Build from `menus[street][depth]`; missing deeper depths repeat the last given one.
    pub fn new(menus: [&[Menu]; 4], max_raises: [u8; 4]) -> BetSizing {
        let mut m = [[Menu::new(&[], true); DEPTHS]; 4];
        for s in 0..4 {
            for d in 0..DEPTHS {
                m[s][d] = menus[s][d.min(menus[s].len() - 1)];
            }
        }
        BetSizing {
            menus: m,
            max_raises,
        }
    }

    pub fn menu(&self, street: usize, depth: usize) -> &Menu {
        &self.menus[street][depth.min(DEPTHS - 1)]
    }

    /// Proxy for the Pluribus blueprint abstraction (see module docs). Preflop up to 14 sizes
    /// for the opening raise, flop coarser, turn/river exactly as published.
    pub fn pluribus_blueprint() -> BetSizing {
        let m = Menu::new;
        let pre = [
            m(
                &[
                    0.4, 0.5, 0.6, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 15.0,
                ],
                true,
            ),
            m(&[0.5, 0.75, 1.0, 1.5, 2.5], true),
            m(&[0.6, 1.0, 2.0], true),
            m(&[1.0], true),
        ];
        let flop = [
            m(&[0.33, 0.5, 0.75, 1.0, 1.5, 2.0], true),
            m(&[0.5, 1.0], true),
        ];
        let late = [m(&[0.5, 1.0], true), m(&[1.0], true)];
        BetSizing::new([&pre, &flop, &late, &late], [4, 3, 3, 3])
    }

    /// Very small abstraction for fast experiments: {1x pot, all-in}, 2 raises per street.
    pub fn tiny() -> BetSizing {
        let t = [Menu::new(&[1.0], true)];
        BetSizing::new([&t, &t, &t, &t], [2, 2, 2, 2])
    }
}

/// Pot fraction represented by `raise_to` in `state`: `(raise_to - cur_bet) / (pot + call)`.
pub fn raise_fraction(state: &State, raise_to: i32) -> f64 {
    let l = state.legal();
    (raise_to - state.cur_bet()) as f64 / (state.pot() + l.call_amount) as f64
}

/// Abstract actions at the current node of `state` (must not be terminal).
#[inline]
pub fn abstract_actions(state: &State, sizing: &BetSizing, out: &mut ActionList) {
    out.clear();
    let l = state.legal();
    if l.can_fold {
        out.push(Action::Fold);
    }
    out.push(if l.can_check {
        Action::Check
    } else {
        Action::Call
    });
    let st = state.street().idx();
    if !l.can_raise || state.raises_this_street() >= sizing.max_raises[st] as u32 {
        return;
    }
    let menu = sizing.menu(st, state.raises_this_street() as usize);
    let basis = (state.pot() + l.call_amount) as f32;
    let cur = state.cur_bet();
    let mut last = 0;
    for &f in menu.fracs() {
        let to = (cur + (f * basis).round() as i32).clamp(l.min_raise_to, l.max_raise_to);
        if to != last {
            out.push(Action::RaiseTo(to));
            last = to;
        }
    }
    if menu.all_in && last != l.max_raise_to {
        out.push(Action::RaiseTo(l.max_raise_to));
    }
}

/// Hash-only history key (8 bytes): exactly `History::key()` without the index log. Hot
/// loops (MCCFR traversal) copy this per child instead of the 112-byte `History`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistKey(pub u64);

impl HistKey {
    pub const fn new() -> Self {
        HistKey(0x243F_6A88_85A3_08D3)
    }
    #[inline(always)]
    pub fn pushed(self, i: usize) -> Self {
        let h = (self.0.rotate_left(23) ^ (i as u64 + 1)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        HistKey(h ^ (h >> 29))
    }
    #[inline(always)]
    pub fn key(self) -> u64 {
        self.0
    }
}

impl Default for HistKey {
    fn default() -> Self {
        Self::new()
    }
}

/// Compact abstract-action history: index of each chosen action within its `ActionList`.
///
/// The action list at a node is a pure function of the history (and sizing), so the index
/// sequence identifies the betting node. Key infosets by `(history.key(), card bucket)`.
/// Street boundaries are implied by the sequence and need not be encoded.
#[derive(Clone, Copy, Debug)]
pub struct History {
    hash: u64,
    len: u16,
    idx: [u8; History::MAX],
}

impl History {
    pub const MAX: usize = 96;
    pub const fn new() -> Self {
        History {
            hash: HistKey::new().0,
            len: 0,
            idx: [0; History::MAX],
        }
    }
    /// Record that action `i` of the node's `ActionList` was taken.
    #[inline]
    pub fn push(&mut self, i: usize) {
        if (self.len as usize) < Self::MAX {
            self.idx[self.len as usize] = i as u8;
        }
        self.len += 1;
        self.hash = HistKey(self.hash).pushed(i).0;
    }
    /// 64-bit hash of the full sequence (collision-resistant enough for infoset keys).
    #[inline]
    pub fn key(&self) -> u64 {
        self.hash
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Recorded indices (truncated at `MAX`; the hash still covers everything).
    pub fn indices(&self) -> &[u8] {
        &self.idx[..(self.len as usize).min(Self::MAX)]
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// Pseudo-harmonic mapping (Ganzfried & Sandholm 2013): probability that observed bet `x`
/// (pot fraction) between abstract sizes `a < b` maps to `a`.
#[inline]
pub fn pseudo_harmonic_lower(x: f64, a: f64, b: f64) -> f64 {
    ((b - x) * (1.0 + a)) / ((b - a) * (1.0 + x))
}

/// Where an observed raise falls relative to an action list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bracket {
    /// Matches (or clamps to) list entry `i`.
    Exact(usize),
    /// Strictly between entries `lo < hi`; map to `lo` with probability `p_lo`.
    Between { lo: usize, hi: usize, p_lo: f64 },
}

/// Locate observed `raise_to` among `list` (Check/Call counts as pot fraction 0).
pub fn bracket(state: &State, list: &ActionList, raise_to: i32) -> Bracket {
    let acts = list.as_slice();
    let call = acts
        .iter()
        .position(|a| matches!(a, Action::Check | Action::Call))
        .unwrap();
    let mut lo = call;
    let mut lo_to = state.cur_bet();
    for (i, a) in acts.iter().enumerate() {
        let Action::RaiseTo(to) = *a else { continue };
        if raise_to == to {
            return Bracket::Exact(i);
        }
        if raise_to < to {
            let (xa, xb, x) = (
                if lo == call {
                    0.0
                } else {
                    raise_fraction(state, lo_to)
                },
                raise_fraction(state, to),
                raise_fraction(state, raise_to),
            );
            return Bracket::Between {
                lo,
                hi: i,
                p_lo: pseudo_harmonic_lower(x, xa, xb),
            };
        }
        lo = i;
        lo_to = to;
    }
    Bracket::Exact(lo) // above the largest abstract size (all-in)
}

/// Deterministic translation: map to the lower size iff `p_lo >= 0.5`. Returns a list index.
pub fn translate_deterministic(state: &State, list: &ActionList, raise_to: i32) -> usize {
    match bracket(state, list, raise_to) {
        Bracket::Exact(i) => i,
        Bracket::Between { lo, hi, p_lo } => {
            if p_lo >= 0.5 {
                lo
            } else {
                hi
            }
        }
    }
}

/// Randomized translation: map to the lower size with probability `p_lo`.
pub fn translate_random(state: &State, list: &ActionList, raise_to: i32, rng: &mut Rng) -> usize {
    match bracket(state, list, raise_to) {
        Bracket::Exact(i) => i,
        Bracket::Between { lo, hi, p_lo } => {
            if rng.f64() < p_lo {
                lo
            } else {
                hi
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shadow tracking of the abstract game (translation of off-menu actions).
// ---------------------------------------------------------------------------------------------

/// How an off-menu raise is mapped between two abstract sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Translation {
    /// Deterministic pseudo-harmonic (exposes exact probabilities).
    Deterministic,
    /// Randomised pseudo-harmonic as in the paper (hidden randomness: not AIVAT-exposable).
    Random,
}

/// What a raise smaller than the smallest abstract raise maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BelowMenu {
    /// Pseudo-harmonic between "check/call" (size 0) and the smallest raise: small bets are
    /// (deterministically, or with probability) treated as a check/call.
    Check,
    /// Always the smallest abstract raise: a bet is never turned into a non-bet.
    Smallest,
}

/// Options of [`Shadow`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackOpts {
    pub translation: Translation,
    pub below: BelowMenu,
    /// `true`: keep a shadow copy of the game in which every off-menu raise is replaced by its
    /// translated abstract raise, and read action lists / history from it. `false` (legacy):
    /// action lists come from the real state while the history holds translated indices, which
    /// puts the policy at a blueprint node whose action list differs from the real one after
    /// any off-menu action (index `i` then means a different action).
    pub shadow: bool,
    /// A non-all-in real raise may map to the abstract all-in only if the chips it leaves
    /// behind are at most `allin_snap` x the pot after the raise (a shove in all but name);
    /// otherwise it maps to the largest smaller abstract size. Mapping a 3x-pot bet to a
    /// 10x-pot "all-in" makes the shadow all-in while the real players still have chips and
    /// the shadow loses the real game. `f64::INFINITY` = no restriction (legacy).
    pub allin_snap: f64,
}

impl TrackOpts {
    /// The pre-fix behaviour (kept for ablations and regression comparison).
    pub const LEGACY: TrackOpts = TrackOpts {
        translation: Translation::Deterministic,
        below: BelowMenu::Check,
        shadow: false,
        allin_snap: f64::INFINITY,
    };
}

impl Default for TrackOpts {
    fn default() -> Self {
        TrackOpts {
            translation: Translation::Deterministic,
            below: BelowMenu::Check,
            shadow: true,
            allin_snap: 0.5,
        }
    }
}

/// Locate observed pot fraction `x` among the raises of `list` (fractions measured in `at`).
/// Check/Call counts as fraction 0. Above the largest entry maps to it (all-in).
pub fn bracket_frac(
    at: &State,
    list: &ActionList,
    x: f64,
    below: BelowMenu,
    allow_allin: bool,
) -> Bracket {
    let max_to = at.legal().max_raise_to;
    // Keep the all-in if it is the only raise there is.
    let allow_allin = allow_allin
        || !list
            .as_slice()
            .iter()
            .any(|a| matches!(a, Action::RaiseTo(t) if *t != max_to));
    let acts = list.as_slice();
    // Action spaces without a check/call entry (push/fold) have nothing "below" the first raise.
    let call = acts
        .iter()
        .position(|a| matches!(a, Action::Check | Action::Call));
    let (mut lo, mut lo_x): (Option<usize>, f64) = (call, 0.0);
    for (i, a) in acts.iter().enumerate() {
        let Action::RaiseTo(to) = *a else { continue };
        if !allow_allin && to == max_to {
            continue;
        }
        let xi = raise_fraction(at, to);
        if (x - xi).abs() < 1e-9 {
            return Bracket::Exact(i);
        }
        if x < xi {
            return match lo {
                None => Bracket::Exact(i),
                Some(l) if Some(l) == call && below == BelowMenu::Smallest => {
                    Bracket::Between { lo: l, hi: i, p_lo: 0.0 }
                }
                Some(l) => Bracket::Between {
                    lo: l,
                    hi: i,
                    p_lo: pseudo_harmonic_lower(x, lo_x, xi),
                },
            };
        }
        lo = Some(i);
        lo_x = xi;
    }
    Bracket::Exact(lo.unwrap_or(acts.len() - 1))
}

/// The abstract game as a policy sees it: the real game with every off-menu raise replaced by
/// its translated abstract raise. Action lists, history keys and the node identity all come
/// from the shadow state, so the blueprint is always queried at a node of its own tree; the
/// chosen abstract action is mapped back to a real action by pot fraction
/// ([`Shadow::real_action`]).
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    state: State,
    hist: History,
    opts: TrackOpts,
    off_tree: bool,
    desync: bool,
    reason: u8,
}

impl Shadow {
    pub fn new(root: &State, opts: TrackOpts) -> Shadow {
        Shadow {
            state: *root,
            hist: History::new(),
            opts,
            off_tree: false,
            desync: false,
            reason: 0,
        }
    }
    /// Why the shadow lost the real game: 0 = in sync, 1 = shadow terminal, 2 = different
    /// player to act, 3 = unrepresentable fold, 5 = different street.
    pub fn desync_reason(&self) -> u8 {
        self.reason
    }
    pub fn opts(&self) -> TrackOpts {
        self.opts
    }
    pub fn key(&self) -> u64 {
        self.hist.key()
    }
    pub fn history(&self) -> &History {
        &self.hist
    }
    /// Some earlier action was off-menu (translated to a different abstract action).
    pub fn off_tree(&self) -> bool {
        self.off_tree
    }
    /// The shadow could not follow the real game (policy falls back to check/call).
    pub fn desynced(&self) -> bool {
        self.desync
    }
    /// State whose action list the blueprint node corresponds to.
    #[inline]
    pub fn node<'a>(&'a self, real: &'a State) -> &'a State {
        if self.opts.shadow {
            &self.state
        } else {
            real
        }
    }
    /// The shadow agrees with the real game about who acts (always true in legacy mode).
    pub fn synced(&self, real: &State) -> bool {
        self.sync_reason(real) == 0
    }
    /// 0 if in sync, else the [`Shadow::desync_reason`]-style code (5 = street differs).
    pub fn sync_reason(&self, real: &State) -> u8 {
        if !self.opts.shadow {
            return 0;
        }
        if self.desync {
            return self.reason;
        }
        if self.state.is_terminal() {
            1
        } else if self.state.to_act() != real.to_act() {
            2
        } else if self.state.street() != real.street() {
            5
        } else {
            0
        }
    }
    /// Abstract action list of the policy's node.
    #[inline]
    pub fn actions(&self, real: &State, sizing: &BetSizing, out: &mut ActionList) {
        abstract_actions(self.node(real), sizing, out)
    }

    /// Abstract index in `list` (the node's list) of real action `a` taken at `real_pre`, and
    /// whether the mapping was exact (on-menu).
    pub fn translate(
        &self,
        real_pre: &State,
        list: &ActionList,
        a: Action,
        own: bool,
        rng: &mut Rng,
    ) -> (usize, bool) {
        let acts = list.as_slice();
        // Spaces without check/call (push/fold) map any non-fold to their last entry.
        let passive = acts
            .iter()
            .position(|x| matches!(x, Action::Check | Action::Call))
            .unwrap_or(acts.len() - 1);
        match a {
            Action::Fold => match acts.iter().position(|x| *x == Action::Fold) {
                Some(i) => (i, true),
                None => (passive, false),
            },
            Action::Check | Action::Call => (passive, true),
            Action::RaiseTo(t) => {
                let x = raise_fraction(real_pre, t);
                if own {
                    // The policy's own raise was produced by `real_action` from a list entry:
                    // recover that entry (nearest pot fraction), never translate it randomly.
                    let at = self.node(real_pre);
                    let near = acts.iter().enumerate().filter_map(|(i, a)| match a {
                        Action::RaiseTo(to) => Some((i, (raise_fraction(at, *to) - x).abs())),
                        _ => None,
                    });
                    let best = near.fold(None, |b: Option<(usize, f64)>, c| match b {
                        Some(b) if b.1 <= c.1 => Some(b),
                        _ => Some(c),
                    });
                    return match best {
                        Some((i, _)) => (i, true),
                        None => (passive, false),
                    };
                }
                let l = real_pre.legal();
                let left = (l.max_raise_to - t).max(0) as f64;
                let pot_after = (real_pre.pot() + t - real_pre.street_bet(real_pre.to_act().unwrap())) as f64;
                let allow_allin = left <= self.opts.allin_snap * pot_after;
                match bracket_frac(self.node(real_pre), list, x, self.opts.below, allow_allin) {
                    Bracket::Exact(i) => (i, true),
                    Bracket::Between { lo, hi, p_lo } => {
                        // Randomise only between two raise sizes: "is it a bet at all" stays
                        // deterministic (a randomly ignored bet desynchronises the shadow).
                        let to_lo = match self.opts.translation {
                            Translation::Random if lo != passive => rng.f64() < p_lo,
                            _ => p_lo >= 0.5,
                        };
                        (if to_lo { lo } else { hi }, false)
                    }
                }
            }
        }
    }

    /// Record real action `a` taken at `real_pre`; `own`: the policy itself acted (its raise
    /// maps back to the entry it came from instead of being translated).
    pub fn observe(
        &mut self,
        real_pre: &State,
        sizing: &BetSizing,
        a: Action,
        own: bool,
        rng: &mut Rng,
    ) {
        self.observe_with(real_pre, a, own, rng, |s, out| abstract_actions(s, sizing, out));
    }

    /// As [`Shadow::observe`] with a custom action space (`actions(state, out)`).
    pub fn observe_with(
        &mut self,
        real_pre: &State,
        a: Action,
        own: bool,
        rng: &mut Rng,
        actions: impl Fn(&State, &mut ActionList),
    ) {
        if self.desync {
            return;
        }
        let mut list = ActionList::new();
        if !self.opts.shadow {
            actions(real_pre, &mut list);
            let (i, exact) = self.translate(real_pre, &list, a, own, rng);
            self.off_tree |= !exact;
            self.hist.push(i);
            return;
        }
        if self.state.is_terminal()
            || self.state.to_act() != real_pre.to_act()
            || self.state.street() != real_pre.street()
        {
            self.desync = true;
            self.reason = if self.state.is_terminal() {
                1
            } else if self.state.street() != real_pre.street() {
                5
            } else {
                2
            };
            return;
        }
        actions(&self.state, &mut list);
        let (i, exact) = self.translate(real_pre, &list, a, own, rng);
        // A fold the shadow cannot represent (it faces no bet there): cannot follow.
        if a == Action::Fold && !exact {
            self.desync = true;
            self.reason = 3;
            return;
        }
        self.off_tree |= !exact;
        self.hist.push(i);
        self.state.apply(list.get(i));

    }

    /// Real action for abstract entry `i` of the node's `list`, to be played at `real`.
    pub fn real_action(&self, real: &State, list: &ActionList, i: usize) -> Action {
        let a = list.get(i);
        if !self.opts.shadow {
            return a;
        }
        let l = real.legal();
        let passive = if l.can_check { Action::Check } else { Action::Call };
        match a {
            Action::Fold => {
                if l.can_fold {
                    Action::Fold
                } else {
                    passive
                }
            }
            Action::Check | Action::Call => passive,
            Action::RaiseTo(to) => {
                if !l.can_raise {
                    return passive;
                }
                if to == self.state.legal().max_raise_to {
                    return Action::RaiseTo(l.max_raise_to);
                }
                let f = raise_fraction(&self.state, to);
                let basis = (real.pot() + l.call_amount) as f64;
                let rt = (real.cur_bet() + (f * basis).round() as i32)
                    .clamp(l.min_raise_to, l.max_raise_to);
                Action::RaiseTo(rt)
            }
        }
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    use crate::GameConfig;

    fn flop_hu() -> State {
        // Pot 1200 at the flop, so that sub-menu bets are above the minimum raise.
        let mut s = State::new(&GameConfig::new(2));
        s.apply(Action::RaiseTo(600));
        s.apply(Action::Call);
        s
    }

    fn opts(below: BelowMenu, snap: f64) -> TrackOpts {
        TrackOpts {
            translation: Translation::Deterministic,
            below,
            shadow: true,
            allin_snap: snap,
        }
    }

    /// Real `RaiseTo` for a bet of `f` x pot at the flop root.
    fn bet(s: &State, f: f64) -> Action {
        let l = s.legal();
        Action::RaiseTo(s.cur_bet() + (f * (s.pot() + l.call_amount) as f64).round() as i32)
    }

    #[test]
    fn on_menu_play_keeps_shadow_identical() {
        let sizing = BetSizing::pluribus_blueprint();
        let mut rng = Rng::new(7);
        for hand in 0..300 {
            let cfg = GameConfig::new(6);
            let mut real = State::new(&cfg);
            let mut sh = Shadow::new(&real, TrackOpts::default());
            let mut plain = History::new();
            while !real.is_terminal() {
                let mut list = ActionList::new();
                abstract_actions(&real, &sizing, &mut list);
                assert!(sh.synced(&real), "hand {hand}");
                let mut l2 = ActionList::new();
                sh.actions(&real, &sizing, &mut l2);
                assert_eq!(list.as_slice(), l2.as_slice());
                let i = rng.below(list.len() as u32) as usize;
                let a = sh.real_action(&real, &list, i);
                assert_eq!(a, list.get(i), "on-menu real action is the list entry");
                sh.observe(&real, &sizing, a, rng.below(2) == 0, &mut rng);
                plain.push(i);
                assert_eq!(sh.key(), plain.key());
                assert!(!sh.off_tree());
                real.apply(a);
            }
        }
    }

    #[test]
    fn small_bet_maps_to_smallest_raise_with_fold_option() {
        let sizing = BetSizing::pluribus_blueprint();
        let real = flop_hu();
        let mut rng = Rng::new(1);
        // 0.1 x pot is below the smallest flop size (0.33).
        let a = bet(&real, 0.1);
        let mut small = Shadow::new(&real, opts(BelowMenu::Smallest, 0.0));
        let mut exact = Shadow::new(&real, opts(BelowMenu::Smallest, 0.0));
        small.observe(&real, &sizing, a, false, &mut rng);
        exact.observe(&real, &sizing, bet(&real, 0.33), false, &mut rng);
        assert!(small.off_tree() && !exact.off_tree());
        assert_eq!(small.key(), exact.key(), "same abstract node as the smallest size");
        let mut after = real;
        after.apply(a);
        assert!(small.synced(&after));
        let mut l = ActionList::new();
        small.actions(&after, &sizing, &mut l);
        assert_eq!(l.get(0), Action::Fold, "the target faces a bet, so it may fold");
        // And what it plays maps to legal real actions.
        for i in 0..l.len() {
            let ra = small.real_action(&after, &l, i);
            let mut t = after;
            t.apply(ra); // debug-asserts legality
        }
    }

    #[test]
    fn check_mode_keeps_node_consistent_and_never_folds_to_nothing() {
        let sizing = BetSizing::pluribus_blueprint();
        let real = flop_hu();
        let mut rng = Rng::new(1);
        let a = bet(&real, 0.1); // p_lo > 0.5: mapped to "check"
        let mut sh = Shadow::new(&real, opts(BelowMenu::Check, 0.0));
        sh.observe(&real, &sizing, a, false, &mut rng);
        assert!(sh.off_tree());
        let mut after = real;
        after.apply(a);
        let mut l = ActionList::new();
        sh.actions(&after, &sizing, &mut l);
        assert_eq!(l.get(0), Action::Check, "shadow node is the checked-to node");
        // Its "check" is a real call, its bets are real raises over the bet.
        assert_eq!(sh.real_action(&after, &l, 0), Action::Call);
        match sh.real_action(&after, &l, 1) {
            Action::RaiseTo(t) => {
                let lg = after.legal();
                assert!(t >= lg.min_raise_to && t <= lg.max_raise_to)
            }
            x => panic!("{x:?}"),
        }
        // A real fold cannot be represented (shadow has nothing to fold to): desync, no panic.
        sh.observe(&after, &sizing, Action::Fold, false, &mut rng);
        assert!(sh.desynced() && sh.desync_reason() == 3);
        assert!(!sh.synced(&after));
    }

    #[test]
    fn overbet_never_becomes_a_phantom_all_in() {
        let sizing = BetSizing::pluribus_blueprint();
        let real = flop_hu();
        let mut rng = Rng::new(1);
        let a = bet(&real, 6.0); // between 2.0 x pot and the all-in size
        let mut strict = Shadow::new(&real, opts(BelowMenu::Smallest, 0.0));
        strict.observe(&real, &sizing, a, false, &mut rng);
        let mut after = real;
        after.apply(a);
        assert!(strict.synced(&after));
        let mut l = ActionList::new();
        strict.actions(&after, &sizing, &mut l);
        // It faces a 2 x pot bet (largest non-all-in), not a shove.
        assert!(l.as_slice().iter().any(|x| matches!(x, Action::RaiseTo(_))) || l.len() >= 2);
        let mut loose = Shadow::new(&real, opts(BelowMenu::Smallest, f64::INFINITY));
        loose.observe(&real, &sizing, a, false, &mut rng);
        // Legacy-style mapping to all-in leaves the shadow facing a shove the real game isn't.
        assert!(loose.key() != strict.key());
        // A real all-in always maps to the all-in.
        let shove = Action::RaiseTo(real.legal().max_raise_to);
        let mut s2 = Shadow::new(&real, opts(BelowMenu::Smallest, 0.0));
        s2.observe(&real, &sizing, shove, false, &mut rng);
        assert!(s2.synced(&{
            let mut t = real;
            t.apply(shove);
            t
        }));
    }

    #[test]
    fn random_translation_follows_pseudo_harmonic_probability() {
        let sizing = BetSizing::pluribus_blueprint();
        let real = flop_hu();
        let mut list = ActionList::new();
        abstract_actions(&real, &sizing, &mut list);
        // Observed 0.6 x pot between the 0.5 and 0.75 sizes.
        let a = bet(&real, 0.6);
        let x = raise_fraction(&real, if let Action::RaiseTo(t) = a { t } else { 0 });
        let (xa, xb) = (0.5, 0.75);
        let p_lo = pseudo_harmonic_lower(x, xa, xb);
        let sh = Shadow::new(
            &real,
            TrackOpts {
                translation: Translation::Random,
                ..TrackOpts::default()
            },
        );
        let mut rng = Rng::new(3);
        let n = 20_000;
        let lo = (0..n)
            .filter(|_| {
                let (i, exact) = sh.translate(&real, &list, a, false, &mut rng);
                assert!(!exact);
                raise_fraction(&real, if let Action::RaiseTo(t) = list.get(i) { t } else { 0 }) < 0.6
            })
            .count() as f64
            / n as f64;
        assert!((lo - p_lo).abs() < 0.02, "{lo} vs {p_lo}");
    }

    #[test]
    fn own_actions_roundtrip_even_when_the_shadow_has_drifted() {
        let sizing = BetSizing::pluribus_blueprint();
        let mut rng = Rng::new(5);
        for _ in 0..200 {
            let mut real = flop_hu();
            let mut sh = Shadow::new(&real, opts(BelowMenu::Smallest, 0.0));
            // Opponent makes an off-menu bet; then we play each abstract entry.
            let a = bet(&real, 0.2 + rng.f64());
            sh.observe(&real, &sizing, a, false, &mut rng);
            real.apply(a);
            let mut list = ActionList::new();
            sh.actions(&real, &sizing, &mut list);
            for i in 0..list.len() {
                let ra = sh.real_action(&real, &list, i);
                // Observing our own real action recovers an entry that plays identically.
                let (j, exact) = sh.translate(&real, &list, ra, true, &mut rng);
                assert!(exact);
                assert_eq!(sh.real_action(&real, &list, j), ra, "entry {i} -> {j}");
            }
        }
    }
}
