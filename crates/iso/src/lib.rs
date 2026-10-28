//! Poker hand isomorphism indexing (Kevin Waugh, "Fast and optimal hand isomorphism
//! algorithms", 2013; port of github.com/kdub0/hand-isomorphism `hand_index.c`).
//! Original work Copyright (c) 2013 Kevin Waugh (waugh@cs.cmu.edu); redistributed
//! under its original notice, reproduced in full in the repository's NOTICE file.
//!
//! A hand is a sequence of rounds of cards (e.g. 2 hole cards, then 3 flop cards).
//! Cards within a round form a set; hands that differ only by a suit permutation map
//! to the same dense index in `0..size(round)`.
//!
//! Terminology follows the reference: a *configuration* is, per suit, how many cards of
//! each round that suit holds (4 bits per round, sorted descending across suits); a
//! *permutation* is the same count vector before sorting, which selects the suit
//! permutation `pi` and the configuration.

use cards::Card;
use std::sync::OnceLock;

const SUITS: usize = 4;
const RANKS: usize = 13;
const MAX_ROUNDS: usize = 8;
const ROUND_SHIFT: u32 = 4;
const ROUND_MASK: u32 = 0xf;
const NUM_PERMS: usize = 24;

/// Global lookup tables shared by every indexer.
struct Tables {
    /// `nth_unset[set][n]`: n-th rank not in `set` (0xff if none).
    nth_unset: Vec<[u8; RANKS]>,
    /// `equal[mask][j]`: suit j is "equal" to suit j-1 (mask bit j-1).
    equal: [[bool; SUITS]; 1 << (SUITS - 1)],
    ncr_ranks: [[u32; RANKS + 1]; RANKS + 1],
    rank_set_to_index: Vec<u32>,
    /// `index_to_rank_set[popcount][index]`.
    index_to_rank_set: Vec<Vec<u32>>,
    suit_permutations: [[u8; SUITS]; NUM_PERMS],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build_tables)
}

fn build_tables() -> Tables {
    let mut equal = [[false; SUITS]; 1 << (SUITS - 1)];
    for (i, row) in equal.iter_mut().enumerate() {
        for j in 1..SUITS {
            row[j] = i & (1 << (j - 1)) != 0;
        }
    }

    let mut nth_unset = vec![[0xffu8; RANKS]; 1 << RANKS];
    for (i, row) in nth_unset.iter_mut().enumerate() {
        let mut set = !i & ((1 << RANKS) - 1);
        for slot in row.iter_mut() {
            *slot = if set != 0 {
                set.trailing_zeros() as u8
            } else {
                0xff
            };
            set &= set.wrapping_sub(1);
        }
    }

    let mut ncr_ranks = [[0u32; RANKS + 1]; RANKS + 1];
    ncr_ranks[0][0] = 1;
    for i in 1..=RANKS {
        ncr_ranks[i][0] = 1;
        ncr_ranks[i][i] = 1;
        for j in 1..i {
            ncr_ranks[i][j] = ncr_ranks[i - 1][j - 1] + ncr_ranks[i - 1][j];
        }
    }

    let mut rank_set_to_index = vec![0u32; 1 << RANKS];
    let mut index_to_rank_set = vec![vec![0u32; 1 << RANKS]; RANKS + 1];
    for i in 0..1u32 << RANKS {
        let mut set = i;
        let mut j = 1;
        while set != 0 {
            rank_set_to_index[i as usize] += ncr_ranks[set.trailing_zeros() as usize][j];
            set &= set - 1;
            j += 1;
        }
        index_to_rank_set[i.count_ones() as usize][rank_set_to_index[i as usize] as usize] = i;
    }

    let mut suit_permutations = [[0u8; SUITS]; NUM_PERMS];
    for (i, perm) in suit_permutations.iter_mut().enumerate() {
        let (mut index, mut used) = (i, 0usize);
        for (j, slot) in perm.iter_mut().enumerate() {
            let suit = index % (SUITS - j);
            index /= SUITS - j;
            let shifted = nth_unset[used][suit] as usize;
            *slot = shifted as u8;
            used |= 1 << shifted;
        }
    }

    Tables {
        nth_unset,
        equal,
        ncr_ranks,
        rank_set_to_index,
        index_to_rank_set,
        suit_permutations,
    }
}

/// `C(n, k)` for `k <= SUITS`, `n < len`; sized per indexer to its largest per-suit index space
/// (the reference allocates a fixed 0x100000-row table instead).
struct Groups(Vec<u64>);

impl Groups {
    fn new(len: usize) -> Self {
        let w = SUITS + 1;
        let mut v = vec![0u64; len * w];
        v[0] = 1;
        for i in 1..len {
            v[i * w] = 1;
            if i < w {
                v[i * w + i] = 1;
            }
            for j in 1..i.min(w) {
                v[i * w + j] = v[(i - 1) * w + j - 1] + v[(i - 1) * w + j];
            }
        }
        Groups(v)
    }
}

/// Per-round lookup data.
#[derive(Default)]
struct Round {
    /// Configurations, sorted ascending (lexicographic over suits).
    configuration: Vec<[u32; SUITS]>,
    suit_size: Vec<[u32; SUITS]>,
    offset: Vec<u64>,
    equal: Vec<u8>,
    perm_to_config: Vec<u32>,
    perm_to_pi: Vec<u8>,
    size: u64,
}

/// Indexer for a fixed sequence of rounds (cards dealt per round).
pub struct HandIndexer {
    t: &'static Tables,
    groups: Groups,
    cards_per_round: Vec<u8>,
    round_start: Vec<u8>,
    rounds: Vec<Round>,
}

/// Walks all suit-count configurations (`equal_mode`: only canonical, descending-for-equal
/// suits) or all raw per-suit count permutations, calling `observe(round, counts)`.
struct Enumerator<'a> {
    cpr: &'a [u8],
    equal_mode: bool,
    used: [u32; SUITS],
    cfg: [u32; SUITS],
}

impl Enumerator<'_> {
    fn rec(
        &mut self,
        round: usize,
        remaining: u32,
        suit: usize,
        equal: u32,
        observe: &mut dyn FnMut(usize, &[u32; SUITS]),
    ) {
        let rounds = self.cpr.len();
        if suit == SUITS {
            observe(round, &self.cfg);
            if round + 1 < rounds {
                self.rec(round + 1, self.cpr[round + 1] as u32, 0, equal, observe);
            }
            return;
        }
        let min = if suit == SUITS - 1 { remaining } else { 0 };
        let mut max = (RANKS as u32 - self.used[suit]).min(remaining);
        let shift = ROUND_SHIFT * (rounds - round - 1) as u32;
        let mut previous = RANKS as u32 + 1;
        let was_equal = self.equal_mode && equal & (1 << suit) != 0;
        if was_equal {
            previous = (self.cfg[suit - 1] >> shift) & ROUND_MASK;
            max = max.min(previous);
        }
        let (old_cfg, old_used) = (self.cfg[suit], self.used[suit]);
        for i in min..=max {
            let new_equal = (equal & !(1 << suit)) | ((was_equal && i == previous) as u32) << suit;
            self.used[suit] = old_used + i;
            self.cfg[suit] = old_cfg | i << shift;
            self.rec(round, remaining - i, suit + 1, new_equal, observe);
            self.cfg[suit] = old_cfg;
            self.used[suit] = old_used;
        }
    }

    fn run(cpr: &[u8], equal_mode: bool, observe: &mut dyn FnMut(usize, &[u32; SUITS])) {
        let mut e = Enumerator {
            cpr,
            equal_mode,
            used: [0; SUITS],
            cfg: [0; SUITS],
        };
        e.rec(0, cpr[0] as u32, 0, (1 << SUITS) - 2, observe);
    }
}

/// Index of a raw suit-count vector in the permutation table of `round`.
fn perm_slot(cpr: &[u8], round: usize, count: &[u32; SUITS]) -> usize {
    let rounds = cpr.len();
    let (mut idx, mut mult) = (0usize, 1usize);
    for i in 0..=round {
        let mut remaining = cpr[i] as usize;
        for c in &count[..SUITS - 1] {
            let size = ((c >> ((rounds - i - 1) as u32 * ROUND_SHIFT)) & ROUND_MASK) as usize;
            idx += mult * size;
            mult *= remaining + 1;
            remaining -= size;
        }
    }
    idx
}

impl HandIndexer {
    /// Builds an indexer for rounds of `cards_per_round` cards (e.g. `&[2, 3, 1, 1]`).
    pub fn new(cards_per_round: &[u8]) -> Self {
        let n = cards_per_round.len();
        assert!(n >= 1 && n <= MAX_ROUNDS, "1..=8 rounds supported");
        assert!(cards_per_round.iter().map(|&c| c as usize).sum::<usize>() <= 52);
        assert!(
            cards_per_round.iter().all(|&c| c >= 1),
            "every round needs a card"
        );
        let t = tables();
        let cpr = cards_per_round;
        // Largest per-suit index space: every card in one suit.
        let mut max_suit = 1usize;
        let mut left = RANKS;
        for &c in cpr {
            max_suit *= t.ncr_ranks[left][c as usize] as usize;
            left -= c as usize;
        }
        let groups = Groups::new(max_suit + SUITS);
        let g = |n: u64, k: usize| groups.0[n as usize * (SUITS + 1) + k];

        let mut round_start = vec![0u8; n];
        for i in 1..n {
            round_start[i] = round_start[i - 1] + cpr[i - 1];
        }

        // Pass 1: count configurations per round.
        let mut counts = vec![0usize; n];
        Enumerator::run(cpr, true, &mut |r, _| counts[r] += 1);

        let mut rounds: Vec<Round> = counts
            .iter()
            .map(|&c| Round {
                configuration: vec![[0; SUITS]; c],
                suit_size: vec![[0; SUITS]; c],
                offset: vec![0; c],
                equal: vec![0; c],
                ..Default::default()
            })
            .collect();

        // Pass 2: insert configurations in sorted order, computing sizes/offsets.
        let mut filled = vec![0usize; n];
        Enumerator::run(cpr, true, &mut |round, configuration| {
            let rd = &mut rounds[round];
            let mut id = filled[round];
            filled[round] += 1;
            while id > 0 {
                if configuration >= &rd.configuration[id - 1] {
                    break;
                }
                rd.configuration[id] = rd.configuration[id - 1];
                rd.suit_size[id] = rd.suit_size[id - 1];
                rd.offset[id] = rd.offset[id - 1];
                rd.equal[id] = rd.equal[id - 1];
                id -= 1;
            }
            rd.configuration[id] = *configuration;
            let mut offset = 1u64;
            let mut equal = 0u32;
            let mut i = 0;
            while i < SUITS {
                let mut size = 1u64;
                let mut remaining = RANKS;
                for j in 0..=round {
                    let ranks = ((configuration[i] >> (ROUND_SHIFT * (n - j - 1) as u32))
                        & ROUND_MASK) as usize;
                    size *= t.ncr_ranks[remaining][ranks] as u64;
                    remaining -= ranks;
                }
                let mut j = i + 1;
                while j < SUITS && configuration[j] == configuration[i] {
                    j += 1;
                }
                for k in i..j {
                    rd.suit_size[id][k] = size as u32;
                }
                offset *= g(size + (j - i) as u64 - 1, j - i);
                for k in i + 1..j {
                    equal |= 1 << k;
                }
                i = j;
            }
            rd.offset[id] = offset;
            rd.equal[id] = (equal >> 1) as u8;
        });

        for rd in rounds.iter_mut() {
            let mut accum = 0u64;
            for o in rd.offset.iter_mut() {
                let next = accum + *o;
                *o = accum;
                accum = next;
            }
            rd.size = accum;
        }

        // Pass 3: permutation tables.
        let mut perms = vec![0usize; n];
        Enumerator::run(cpr, false, &mut |r, count| {
            perms[r] = perms[r].max(perm_slot(cpr, r, count) + 1);
        });
        for (rd, &p) in rounds.iter_mut().zip(&perms) {
            rd.perm_to_config = vec![0; p];
            rd.perm_to_pi = vec![0; p];
        }
        Enumerator::run(cpr, false, &mut |round, count| {
            let idx = perm_slot(cpr, round, count);
            let rd = &mut rounds[round];
            // Stable insertion sort of suits by count, descending.
            let mut pi = [0usize, 1, 2, 3];
            for i in 1..SUITS {
                let pi_i = pi[i];
                let mut j = i;
                while j > 0 && count[pi_i] > count[pi[j - 1]] {
                    pi[j] = pi[j - 1];
                    j -= 1;
                }
                pi[j] = pi_i;
            }
            // Lehmer-style code of pi, matching `suit_permutations`.
            let (mut pi_idx, mut pi_mult, mut pi_used) = (0usize, 1usize, 0u32);
            for (i, &p) in pi.iter().enumerate() {
                let bit = 1u32 << p;
                let smaller = ((bit - 1) & pi_used).count_ones() as usize;
                pi_idx += (p - smaller) * pi_mult;
                pi_mult *= SUITS - i;
                pi_used |= bit;
            }
            rd.perm_to_pi[idx] = pi_idx as u8;

            let key = [count[pi[0]], count[pi[1]], count[pi[2]], count[pi[3]]];
            let (mut low, mut high) = (0usize, rd.configuration.len());
            while low < high {
                let mid = (low + high) / 2;
                match rd.configuration[mid].cmp(&key) {
                    std::cmp::Ordering::Greater => high = mid,
                    std::cmp::Ordering::Equal => {
                        low = mid;
                        break;
                    }
                    std::cmp::Ordering::Less => low = mid + 1,
                }
            }
            rd.perm_to_config[idx] = low as u32;
        });

        HandIndexer {
            t,
            groups,
            cards_per_round: cpr.to_vec(),
            round_start,
            rounds,
        }
    }

    /// Number of rounds.
    pub fn rounds(&self) -> usize {
        self.cards_per_round.len()
    }

    /// Number of isomorphism classes of the hand through `round` (0-based).
    pub fn size(&self, round: usize) -> u64 {
        self.rounds[round].size
    }

    /// Index of the full sequence (all rounds, in round order).
    #[inline]
    pub fn index_last(&self, cards: &[Card]) -> u64 {
        let mut out = [0u64; MAX_ROUNDS];
        self.index_all(cards, &mut out[..self.rounds()]);
        out[self.rounds() - 1]
    }

    /// Index of every round prefix; `out.len()` must be at least `rounds()`.
    #[inline]
    pub fn index_all(&self, cards: &[Card], out: &mut [u64]) {
        let mut st = State::new();
        let mut j = 0;
        for r in 0..self.rounds() {
            let n = self.cards_per_round[r] as usize;
            out[r] = self.next_round(r, &cards[j..j + n], &mut st);
            j += n;
        }
    }

    #[inline(always)]
    fn next_round(&self, round: usize, cards: &[Card], st: &mut State) -> u64 {
        let t = self.t;
        let g = |n: u64, k: usize| self.groups.0[n as usize * (SUITS + 1) + k];
        let mut ranks = [0u32; SUITS];
        let mut shifted = [0u32; SUITS];
        for &c in cards {
            let (rank, suit) = ((c >> 2) as u32, (c & 3) as usize);
            let bit = 1u32 << rank;
            debug_assert!(
                ranks[suit] & bit == 0 && st.used[suit] & bit == 0,
                "duplicate card"
            );
            ranks[suit] |= bit;
            shifted[suit] |= bit >> ((bit - 1) & st.used[suit]).count_ones();
        }
        for i in 0..SUITS {
            let used_size = st.used[i].count_ones() as usize;
            let this_size = ranks[i].count_ones() as usize;
            st.suit_index[i] += st.suit_mult[i] * t.rank_set_to_index[shifted[i] as usize] as u64;
            st.suit_mult[i] *= t.ncr_ranks[RANKS - used_size][this_size] as u64;
            st.used[i] |= ranks[i];
        }
        let mut remaining = cards.len();
        for &r in &ranks[..SUITS - 1] {
            let sz = r.count_ones() as usize;
            st.perm_index += st.perm_mult * sz;
            st.perm_mult *= remaining + 1;
            remaining -= sz;
        }

        let rd = &self.rounds[round];
        let cfg = rd.perm_to_config[st.perm_index] as usize;
        let pi = &t.suit_permutations[rd.perm_to_pi[st.perm_index] as usize];
        let eq = &t.equal[rd.equal[cfg] as usize];
        let mut index = rd.offset[cfg];
        let mut si = [0u64; SUITS];
        let mut sm = [0u64; SUITS];
        for i in 0..SUITS {
            si[i] = st.suit_index[pi[i] as usize];
            sm[i] = st.suit_mult[pi[i] as usize];
        }

        macro_rules! swap {
            ($u:expr, $v:expr) => {
                if si[$u] > si[$v] {
                    si.swap($u, $v);
                }
            };
        }
        let mut multiplier = 1u64;
        let mut i = 0;
        while i < SUITS {
            let (part, size);
            if i + 1 < SUITS && eq[i + 1] {
                if i + 2 < SUITS && eq[i + 2] {
                    if i + 3 < SUITS && eq[i + 3] {
                        swap!(i, i + 1);
                        swap!(i + 2, i + 3);
                        swap!(i, i + 2);
                        swap!(i + 1, i + 3);
                        swap!(i + 1, i + 2);
                        part =
                            si[i] + g(si[i + 1] + 1, 2) + g(si[i + 2] + 2, 3) + g(si[i + 3] + 3, 4);
                        size = g(sm[i] + 3, 4);
                        i += 4;
                    } else {
                        swap!(i, i + 1);
                        swap!(i, i + 2);
                        swap!(i + 1, i + 2);
                        part = si[i] + g(si[i + 1] + 1, 2) + g(si[i + 2] + 2, 3);
                        size = g(sm[i] + 2, 3);
                        i += 3;
                    }
                } else {
                    swap!(i, i + 1);
                    part = si[i] + g(si[i + 1] + 1, 2);
                    size = g(sm[i] + 1, 2);
                    i += 2;
                }
            } else {
                part = si[i];
                size = sm[i];
                i += 1;
            }
            index += multiplier * part;
            multiplier *= size;
        }
        index
    }

    /// Writes the canonical representative of `index` at `round` into `out`
    /// (cards of rounds `0..=round`, round order). Returns false if out of range.
    pub fn unindex(&self, round: usize, index: u64, out: &mut [Card]) -> bool {
        if round >= self.rounds() || index >= self.rounds[round].size {
            return false;
        }
        let t = self.t;
        let g = |n: u64, k: usize| self.groups.0[n as usize * (SUITS + 1) + k];
        let n_rounds = self.rounds();
        let rd = &self.rounds[round];
        let mut index = index;

        // Last configuration whose offset <= index.
        let cfg_idx = rd.offset.partition_point(|&o| o <= index) - 1;
        index -= rd.offset[cfg_idx];
        let cfg = &rd.configuration[cfg_idx];

        let mut suit_index = [0u64; SUITS];
        let mut i = 0;
        while i < SUITS {
            let mut j = i + 1;
            while j < SUITS && cfg[j] == cfg[i] {
                j += 1;
            }
            let suit_size = rd.suit_size[cfg_idx][i] as u64;
            let group_size = g(suit_size + (j - i) as u64 - 1, j - i);
            let mut group_index = index % group_size;
            index /= group_size;
            while i < j - 1 {
                // Largest m with C(m + k - 1, k) <= group_index, k = suits left in group
                // (reference brackets this analytically; a full binary search is
                // equivalent and unindex is not hot).
                let k = j - i;
                let (mut low, mut high) = (0u64, suit_size);
                let mut best = 0;
                while low < high {
                    let mid = (low + high) / 2;
                    if g(mid + k as u64 - 1, k) <= group_index {
                        best = mid;
                        low = mid + 1;
                    } else {
                        high = mid;
                    }
                }
                suit_index[i] = best;
                group_index -= g(best + k as u64 - 1, k);
                i += 1;
            }
            suit_index[i] = group_index;
            i += 1;
        }

        let mut location = [0u8; MAX_ROUNDS];
        location[..n_rounds].copy_from_slice(&self.round_start);
        for s in 0..SUITS {
            let (mut used, mut m) = (0usize, 0usize);
            for j in 0..n_rounds {
                let n =
                    ((cfg[s] >> (ROUND_SHIFT * (n_rounds - j - 1) as u32)) & ROUND_MASK) as usize;
                let round_size = t.ncr_ranks[RANKS - m][n] as u64;
                m += n;
                let round_idx = (suit_index[s] % round_size) as usize;
                suit_index[s] /= round_size;
                let mut shifted = t.index_to_rank_set[n][round_idx];
                let mut rank_set = 0usize;
                for _ in 0..n {
                    let low = shifted & shifted.wrapping_neg();
                    shifted ^= low;
                    let rank = t.nth_unset[used][low.trailing_zeros() as usize] as usize;
                    rank_set |= 1 << rank;
                    out[location[j] as usize] = (rank * 4 + s) as Card;
                    location[j] += 1;
                }
                used |= rank_set;
            }
        }
        true
    }
}

struct State {
    suit_index: [u64; SUITS],
    suit_mult: [u64; SUITS],
    used: [u32; SUITS],
    perm_index: usize,
    perm_mult: usize,
}

impl State {
    #[inline(always)]
    fn new() -> Self {
        State {
            suit_index: [0; SUITS],
            suit_mult: [1; SUITS],
            used: [0; SUITS],
            perm_index: 0,
            perm_mult: 1,
        }
    }
}
