//! Standings and leaderboard statistics.
//!
//! Unit of independence: the deal. In a duplicate match one deal is six hands (every rotation);
//! a player's sample for a deal is the mean payoff over its seats in those six hands (AIVAT
//! corrected when available). Non-duplicate matches use the hand itself as the sample. Scores
//! are mbb/hand with a 95% interval from the sample standard error.

use crate::model::*;
use bench::stats::Acc;
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
struct PlayerAcc {
    hands: u64,
    net: i64,
    per_hand: Acc,
    /// sample key -> (sum raw, sum av, count, av missing)
    samples: BTreeMap<u32, (f64, f64, u32, bool)>,
}

pub struct Builder {
    accs: BTreeMap<String, PlayerAcc>,
    key_hands: HashMap<u32, u32>,
    bb: f64,
    duplicate: bool,
    /// hands per sample key for it to count as complete (6 duplicate, 1 otherwise)
    need: u32,
}

impl Builder {
    pub fn new(bb: i64, duplicate: bool) -> Self {
        Builder { accs: BTreeMap::new(), key_hands: HashMap::new(), bb: bb as f64, duplicate, need: if duplicate { 6 } else { 1 } }
    }
    /// Count a hand toward its sample key; call once per hand before `add`.
    pub fn add_hand(&mut self, key: u32) {
        *self.key_hands.entry(key).or_default() += 1;
    }
    pub fn add(&mut self, pid: &str, key: u32, net: i64, av: Option<f64>) {
        let a = self.accs.entry(pid.to_string()).or_default();
        a.hands += 1;
        a.net += net;
        a.per_hand.add(net as f64);
        let e = a.samples.entry(key).or_insert((0.0, 0.0, 0, false));
        e.0 += net as f64;
        match av {
            Some(v) => e.1 += v,
            None => e.3 = true,
        }
        e.2 += 1;
    }

    pub fn finish(self, names: &HashMap<String, String>) -> (Vec<Standing>, Vec<(String, StoredResult)>) {
        let bb = self.bb;
        let mut st = Vec::new();
        let mut rows = Vec::new();
        for (pid, a) in &self.accs {
            let n = a.hands as f64;
            let mean = a.per_hand.mean();
            let se = a.per_hand.se();
            let (mut raw, mut av) = (Acc::default(), Acc::default());
            let mut av_ok = true;
            for (k, (r, v, c, miss)) in &a.samples {
                if self.key_hands.get(k).copied().unwrap_or(0) < self.need {
                    continue;
                }
                raw.add(r / *c as f64);
                av.add(v / *c as f64);
                av_ok &= !miss;
            }
            let deals = raw.n;
            let has_av = av_ok && av.n > 0;
            let (method, sc_mean, sc_se) = if has_av {
                ("aivat", av.mean(), av.se())
            } else if self.duplicate && deals > 0 {
                ("duplicate", raw.mean(), raw.se())
            } else {
                ("raw", mean, se)
            };
            let mbb = |x: f64| x / bb * 1000.0;
            st.push(Standing {
                player_id: pid.clone(),
                slot: 0,
                name: names.get(pid).cloned().unwrap_or_default(),
                hands: a.hands,
                net_chips: a.net,
                bb_per_100: mean / bb * 100.0,
                mbb_per_hand: mbb(mean),
                ci95_bb_per_100: 1.96 * se / bb * 100.0,
                duplicate_bb_per_100: (self.duplicate && deals > 0).then(|| raw.mean() / bb * 100.0),
                duplicate_mbb_per_hand: (self.duplicate && deals > 0).then(|| mbb(raw.mean())),
                duplicate_ci95_mbb_per_hand: (self.duplicate && deals > 0).then(|| mbb(1.96 * raw.se())),
                aivat_mbb_per_hand: has_av.then(|| mbb(av.mean())),
                aivat_ci95_mbb_per_hand: has_av.then(|| mbb(1.96 * av.se())),
                method: method.into(),
                score_mbb_per_hand: mbb(sc_mean),
                score_ci95_mbb_per_hand: mbb(1.96 * sc_se),
                mbb_per_game: mbb(sc_mean),
                ci95_mbb: mbb(1.96 * sc_se),
                deals,
            });
            let _ = n;
            let samples = if method == "raw" { a.hands } else { deals };
            rows.push((
                pid.clone(),
                StoredResult {
                    match_id: String::new(),
                    player_id: pid.clone(),
                    hands: a.hands,
                    deals: samples,
                    net_chips: a.net,
                    score_mbb: mbb(sc_mean),
                    se_mbb: mbb(sc_se),
                    method: method.into(),
                },
            ));
        }
        (st, rows)
    }
}

/// Combine per-match results into one score per player: inverse-variance weighted mean.
/// Rows without a usable standard error (fewer than two samples) carry no weight.
/// Benchmark-track cells are the latest finished run per (player, track); `track` filters and
/// sorts by that column.
pub fn leaderboard(
    results: &[StoredResult],
    players: &HashMap<String, PlayerRecord>,
    runs: &[RunRecord],
    min_hands: u64,
    include_house: bool,
    track: Option<&str>,
) -> Vec<LeaderboardRow> {
    struct A {
        w: f64,
        wm: f64,
        hands: u64,
        matches: u64,
        methods: Vec<String>,
    }
    let mut by: BTreeMap<&str, A> = BTreeMap::new();
    for r in results {
        let a = by.entry(&r.player_id).or_insert(A { w: 0.0, wm: 0.0, hands: 0, matches: 0, methods: vec![] });
        a.hands += r.hands;
        a.matches += 1;
        if !a.methods.contains(&r.method) {
            a.methods.push(r.method.clone());
        }
        if r.se_mbb > 0.0 && r.se_mbb.is_finite() {
            let w = 1.0 / (r.se_mbb * r.se_mbb);
            a.w += w;
            a.wm += w * r.score_mbb;
        }
    }
    // latest finished run per (player, track)
    let mut cells: HashMap<&str, BTreeMap<String, (String, TrackCell)>> = HashMap::new();
    for r in runs.iter().filter(|r| r.status == "finished" || (r.status == "aborted" && r.mbb_per_game.is_some())) {
        let (Some(m), Some(c)) = (r.mbb_per_game, r.ci95_mbb) else { continue };
        let fin = r.finished_at.clone().unwrap_or_default();
        let e = cells.entry(&r.player_id).or_default();
        if e.get(&r.track).map(|(f, _)| *f < fin).unwrap_or(true) {
            e.insert(
                r.track.clone(),
                (fin, TrackCell { run_id: r.run_id.clone(), mbb_per_game: m, ci95_mbb: c, hands: r.hands_done, verified: r.verified, method: r.method.clone().unwrap_or_default() }),
            );
        }
    }
    let mut ids: Vec<&str> = by.keys().copied().collect();
    for k in cells.keys() {
        if !ids.contains(k) {
            ids.push(k);
        }
    }
    let mut rows: Vec<LeaderboardRow> = ids
        .into_iter()
        .filter_map(|pid| {
            let p = players.get(pid)?;
            if p.house && !include_house {
                return None;
            }
            let tracks: BTreeMap<String, TrackCell> = cells.get(pid).map(|m| m.iter().map(|(k, v)| (k.clone(), v.1.clone())).collect()).unwrap_or_default();
            if let Some(t) = track {
                if !tracks.contains_key(t) {
                    return None;
                }
            }
            let a = by.get(pid);
            let general = a.filter(|a| a.hands >= min_hands && a.w > 0.0);
            if general.is_none() && tracks.is_empty() {
                return None;
            }
            let score = general.map(|a| a.wm / a.w);
            Some(LeaderboardRow {
                rank: 0,
                player_id: pid.to_string(),
                name: p.name.clone(),
                house: p.house,
                matches: a.map(|a| a.matches).unwrap_or(0),
                // own benchmark hands when there are runs (match results also count a house bot's seats in other bots' panels)
                hands: if tracks.is_empty() { a.map(|a| a.hands).unwrap_or(0) } else { tracks.values().map(|c| c.hands as u64).sum::<u64>() },
                score_mbb_per_hand: score,
                ci95_mbb_per_hand: general.map(|a| 1.96 / a.w.sqrt()),
                score_bb_per_100: score.map(|s| s / 10.0),
                methods: a.map(|a| a.methods.clone()).unwrap_or_default(),
                vs_ladder_v1: tracks.get("ladder-v1").map(|c| VsCell { mbb: c.mbb_per_game, ci95_mbb: c.ci95_mbb, hands: c.hands }),
                vs_sharingan_1: tracks.get("sharingan-1").map(|c| VsCell { mbb: c.mbb_per_game, ci95_mbb: c.ci95_mbb, hands: c.hands }),
                verified: !tracks.is_empty() && tracks.values().all(|c| c.verified),
                tracks,
            })
        })
        .collect();
    let key = |r: &LeaderboardRow| -> f64 {
        match track {
            Some(t) => r.tracks[t].mbb_per_game,
            None => r.score_mbb_per_hand.unwrap_or(f64::NEG_INFINITY),
        }
    };
    rows.sort_by(|a, b| key(b).partial_cmp(&key(a)).unwrap_or(std::cmp::Ordering::Equal));
    for (i, r) in rows.iter_mut().enumerate() {
        r.rank = i + 1;
    }
    rows
}
