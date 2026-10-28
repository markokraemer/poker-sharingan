//! The `Policy` abstraction.
//!
//! A [`PolicyFactory`] creates one [`Player`] per seat per hand. The harness feeds the player
//! every action (`observe`) and every board reveal (`observe_board`), then asks for a
//! distribution over concrete legal actions with [`Player::dist`], given a public [`View`]
//! and the player's private cards.
//!
//! Contract for players that expose probabilities (`exposes_probs() == true`, needed for
//! AIVAT and as LBR targets): `dist` is a pure function of (public history observed so far,
//! hole cards) and may be queried for *any* hole cards, not just the real ones (this is the
//! "range strategy"). Players that sample hidden randomness must return `false`.
//! `fork` clones a player mid-hand (needed by LBR for hypothetical responses).

use cards::Card;
use nlhe::abstraction::MAX_ACTIONS;
use nlhe::{Action, GameConfig, State, Street};

/// One public action in real chips (`RaiseTo` = total committed on that street).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub seat: u8,
    pub street: Street,
    pub action: Action,
}

/// Public game information at a decision.
pub struct View<'a> {
    pub cfg: &'a GameConfig,
    pub state: &'a State,
    /// Board cards visible now (0, 3, 4 or 5).
    pub board: &'a [Card],
    pub history: &'a [Event],
    /// Acting seat.
    pub seat: usize,
}

/// Probability distribution over concrete legal actions.
#[derive(Clone, Copy, Debug)]
pub struct Dist {
    n: usize,
    acts: [Action; MAX_ACTIONS],
    p: [f32; MAX_ACTIONS],
}

impl Default for Dist {
    fn default() -> Self {
        Self::new()
    }
}

impl Dist {
    pub fn new() -> Dist {
        Dist {
            n: 0,
            acts: [Action::Fold; MAX_ACTIONS],
            p: [0.0; MAX_ACTIONS],
        }
    }
    pub fn pure(a: Action) -> Dist {
        let mut d = Dist::new();
        d.push(a, 1.0);
        d
    }
    /// Merges probability into an existing identical action.
    pub fn push(&mut self, a: Action, p: f32) {
        if let Some(i) = (0..self.n).find(|&i| self.acts[i] == a) {
            self.p[i] += p;
        } else {
            assert!(self.n < MAX_ACTIONS);
            self.acts[self.n] = a;
            self.p[self.n] = p;
            self.n += 1;
        }
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    pub fn action(&self, i: usize) -> Action {
        self.acts[i]
    }
    pub fn actions(&self) -> &[Action] {
        &self.acts[..self.n]
    }
    fn total(&self) -> f64 {
        self.p[..self.n].iter().map(|&x| x as f64).sum()
    }
    /// Normalised probability of entry `i` (the exact law used by `sample`).
    pub fn prob(&self, i: usize) -> f64 {
        let t = self.total();
        if t <= 0.0 {
            1.0 / self.n as f64
        } else {
            self.p[i] as f64 / t
        }
    }
    /// Probability of a specific action (0 if absent).
    pub fn prob_of(&self, a: Action) -> f64 {
        (0..self.n)
            .find(|&i| self.acts[i] == a)
            .map_or(0.0, |i| self.prob(i))
    }
    /// Map a uniform `u` in [0,1) to an entry index.
    pub fn sample(&self, u: f64) -> usize {
        let mut x = u * 1.0;
        for i in 0..self.n {
            let p = self.prob(i);
            if x < p {
                return i;
            }
            x -= p;
        }
        self.n - 1
    }
}

pub trait Player: Send {
    fn observe(&mut self, _pre: &State, _seat: usize, _a: Action) {}
    fn observe_board(&mut self, _board: &[Card]) {}
    fn dist(&mut self, view: &View, hole: [Card; 2]) -> Dist;
    /// Choose an action given a uniform draw `u`. Override only for non-exposing players.
    fn act(&mut self, view: &View, hole: [Card; 2], u: f64) -> Action {
        let d = self.dist(view, hole);
        d.action(d.sample(u))
    }
    fn exposes_probs(&self) -> bool {
        true
    }
    fn fork(&self) -> Option<Box<dyn Player>> {
        None
    }
    /// Diagnostic about the last `dist`/`act` (e.g. "timeout: blueprint fallback"); read-once.
    fn take_note(&mut self) -> Option<String> {
        None
    }
    /// Called once per player when the hand is over (full deal, public history, terminal state).
    /// Stateful external bots use it to learn about opponents.
    fn finish(&mut self, _deal: &nlhe::Deal, _hist: &[Event], _end: &State) {}
}

pub trait PolicyFactory: Send + Sync {
    fn name(&self) -> String;
    /// `seed` is unique per (hand, seat); use it for any hidden randomness.
    fn new_player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> Box<dyn Player>;
    /// As `new_player`, also told who sits where (`who[s]` = entrant index at seat `s`).
    fn new_player_ctx(&self, seat: usize, cfg: &GameConfig, seed: u64, _who: &[usize]) -> Box<dyn Player> {
        self.new_player(seat, cfg, seed)
    }
    /// Optional end-of-run diagnostics (one line of JSON) printed by the `bench` CLI.
    fn report(&self) -> Option<String> {
        None
    }
}
