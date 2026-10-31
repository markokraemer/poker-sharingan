//! Matches: a planned series of hands at one table, optionally duplicate (TABLE_API.md section 7).
//!
//! Duplicate schedule. Deal `i` has the deck `deal_deck(seed, i)` and its button at physical seat
//! `i mod 6`. In rotation `r` participant `p` sits in seat `(p + r) mod 6`, so every participant
//! plays every seat with the same cards. To keep a bot from recognising a repeated deal, deals
//! are played in blocks of `block_deals` (default 12): rotation 0 of every deal in the block,
//! then rotation 1 of the block, and so on. `hand_start` carries `deal_no` and `rotation`.

use crate::model::*;
use crate::state::AppState;
use crate::stats::Builder;
use crate::store::Store;
use crate::util::*;
use bench::aivat::{EquityEstimator, Mode, Query, Scratch, Tracker, ValueEstimator};
use bench::mix;
use bench::policy::{Event, Player, View};
use nlhe::{Action, State, Step};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct HandPlan {
    pub hand_no: u32,
    pub deal_no: Option<u32>,
    pub rotation: Option<u32>,
    /// `None` = choose the next button (free tables).
    pub button: usize,
    /// Player id per physical seat (matches reseat every hand).
    pub assignment: Option<Vec<String>>,
    /// Physical seats dealt in.
    pub dealt: Vec<usize>,
    pub deck_index: u64,
    pub crn_key: u64,
    pub seed: String,
    pub match_id: Option<String>,
    pub hide_cards: bool,
}

pub enum MatchPoll {
    Play(HandPlan),
    Wait,
    /// This lane has nothing left to play (`"finished"` or `"aborted"`).
    Finished(&'static str),
}

pub struct HandRec {
    pub hand_no: u32,
    pub nets: [i64; 6],
}

pub struct MatchState {
    pub rec: MatchRecord,
    pub recs: Vec<HandRec>,
    pub aivat: Option<HashMap<u32, [f64; 6]>>,
    pub abort: bool,
    /// Hands played per lane.
    pub cursors: Vec<u32>,
}

/// Link from a match to the benchmark run it belongs to.
#[derive(Clone, Debug)]
pub struct BenchLink {
    pub run_id: String,
    pub track: String,
    pub player_id: String,
}

pub struct MatchRuntime {
    pub id: String,
    pub table_id: String,
    pub table_ids: Vec<String>,
    pub participants: Vec<String>,
    pub duplicate: bool,
    pub seed: String,
    pub planned: u32,
    pub block_deals: u32,
    pub hide_cards: bool,
    pub bb: i64,
    pub names: HashMap<String, String>,
    pub lanes: usize,
    pub bench: Option<BenchLink>,
    pub update_every: u32,
    pub lane_done: AtomicU32,
    pub state: Mutex<MatchState>,
}

impl MatchRuntime {
    pub fn new(rec: MatchRecord, names: HashMap<String, String>, block_deals: u32, hide_cards: bool, bench: Option<BenchLink>) -> Self {
        let lanes = rec.table_ids.len().max(1);
        let planned = rec.hands_planned;
        MatchRuntime {
            id: rec.match_id.clone(),
            table_id: rec.table_id.clone(),
            table_ids: rec.table_ids.clone(),
            participants: rec.participants.clone(),
            duplicate: rec.duplicate.is_some(),
            seed: rec.seed.clone(),
            planned,
            block_deals: block_deals.max(1),
            hide_cards,
            bb: rec.table_config.big_blind,
            names,
            lanes,
            update_every: if bench.is_some() { (planned / 100).max(50) } else { 50 },
            bench,
            lane_done: AtomicU32::new(0),
            state: Mutex::new(MatchState { rec, recs: Vec::new(), aivat: None, abort: false, cursors: vec![0; lanes] }),
        }
    }

    pub fn record(&self) -> MatchRecord {
        self.state.lock().unwrap().rec.clone()
    }

    /// Deal number and rotation of the `hand_no`-th hand (1-based). See [`locate`].
    fn locate(&self, hand_no: u32) -> (Option<u32>, u32) {
        locate(self.duplicate, self.planned, self.block_deals, hand_no)
    }

    /// The `c`-th (0-based) hand of a lane. See [`lane_hand_no`].
    fn lane_hand_no(&self, lane: usize, c: u32) -> Option<u32> {
        lane_hand_no(self.duplicate, self.planned, self.block_deals, self.lanes, lane, c)
    }

    /// Player at each physical seat for `hand_no` (participant `p` sits in seat `(p + r) mod 6`).
    fn seat_players(&self, rotation: u32) -> impl Iterator<Item = &String> + '_ {
        (0..6usize).map(move |s| &self.participants[(s + 6 - rotation as usize) % 6])
    }

    pub fn plan_for(&self, hand_no: u32) -> HandPlan {
        let (deal_no, r) = self.locate(hand_no);
        let k = hand_no - 1;
        let (button, deck_index, crn) = match deal_no {
            Some(d) => ((d % 6) as usize, d as u64, d as u64),
            None => ((k % 6) as usize, k as u64, hand_no as u64),
        };
        HandPlan {
            hand_no,
            deal_no,
            rotation: deal_no.map(|_| r),
            button,
            assignment: Some(self.seat_players(r).cloned().collect()),
            dealt: (0..6).collect(),
            deck_index,
            crn_key: crn,
            seed: self.seed.clone(),
            match_id: Some(self.id.clone()),
            hide_cards: self.hide_cards,
        }
    }

    pub fn poll(&self, lane: usize) -> MatchPoll {
        let s = self.state.lock().unwrap();
        if s.abort {
            return MatchPoll::Finished("aborted");
        }
        let Some(h) = self.lane_hand_no(lane, s.cursors[lane]) else { return MatchPoll::Finished("finished") };
        if s.rec.status == "running" {
            drop(s);
            MatchPoll::Play(self.plan_for(h))
        } else {
            MatchPoll::Wait
        }
    }

    /// Account a finished hand. Returns a `match_update` payload (and, for benchmark runs, a
    /// progress payload) every `update_every` hands.
    pub fn record_hand(&self, lane: usize, h: &HandHistory, store: &dyn Store) -> Option<(Value, Option<Value>)> {
        let mut nets = [0i64; 6];
        for r in &h.results {
            nets[r.seat as usize] = r.net;
        }
        let mut s = self.state.lock().unwrap();
        s.recs.push(HandRec { hand_no: h.hand_no, nets });
        s.cursors[lane] += 1;
        s.rec.hands_played += 1;
        let hp = s.rec.hands_played;
        if hp % 100 == 0 {
            let _ = store.put_match(&s.rec);
        }
        if hp % self.update_every == 0 {
            let standings = self.build(&s).0;
            let progress = self.bench.as_ref().map(|b| self.progress_value(b, &s, &standings));
            return Some((json!({"match_id": self.id, "hands_played": hp, "standings": standings}), progress));
        }
        None
    }

    fn progress_value(&self, b: &BenchLink, s: &MatchState, standings: &[Standing]) -> Value {
        let me = standings.iter().find(|x| x.player_id == b.player_id);
        json!({
            "run_id": b.run_id, "track": b.track, "match_id": self.id, "status": s.rec.status,
            "hands_done": s.rec.hands_played, "hands_total": self.planned,
            "mbb_per_game": me.map(|m| m.mbb_per_game), "ci95_mbb": me.map(|m| m.ci95_mbb),
            "method": me.map(|m| m.method.clone()), "deals_done": me.map(|m| m.deals),
        })
    }

    /// Live progress of a benchmark run.
    pub fn progress(&self) -> Option<Value> {
        let b = self.bench.as_ref()?;
        let s = self.state.lock().unwrap();
        let st = self.build(&s).0;
        Some(self.progress_value(b, &s, &st))
    }

    fn slot_keys(&self) -> Vec<String> {
        slot_keys_of(&self.participants)
    }

    fn build(&self, s: &MatchState) -> (Vec<Standing>, Vec<(String, StoredResult)>) {
        build_standings(&self.slot_keys(), &self.participants, &self.names, self.bb, self.duplicate, self.planned, self.block_deals, &s.recs, s.aivat.as_ref())
    }

    pub fn standings(&self) -> Vec<Standing> {
        let s = self.state.lock().unwrap();
        self.build(&s).0
    }

    pub fn set_status(&self, status: &str) {
        self.state.lock().unwrap().rec.status = status.to_string();
    }
}

/// Deal number and rotation of the `hand_no`-th hand (1-based). Duplicate hands are numbered
/// block-major: within a block of `block_deals` deals, rotation 0 of every deal, then rotation 1, ...
/// Shared by the table server and the headless match worker (`worker.rs`).
pub fn locate(duplicate: bool, planned: u32, block_deals: u32, hand_no: u32) -> (Option<u32>, u32) {
    if !duplicate {
        return (None, 0);
    }
    let k = hand_no - 1;
    let deals = planned / 6;
    let b = block_deals;
    let blk = k / (6 * b);
    let within = k % (6 * b);
    let blk_deals = b.min(deals - blk * b);
    (Some(blk * b + within % blk_deals), within / blk_deals)
}

/// The `c`-th (0-based) hand of a lane. Lane `L` of `N` plays blocks `L, L+N, L+2N, ...`.
pub fn lane_hand_no(duplicate: bool, planned: u32, block_deals: u32, lanes: usize, lane: usize, c: u32) -> Option<u32> {
    if !duplicate {
        return (c < planned).then_some(c + 1);
    }
    let deals = planned / 6;
    let nblocks = (deals + block_deals - 1) / block_deals;
    let (mut rem, mut b) = (c, lane as u32);
    while b < nblocks {
        let sz = 6 * block_deals.min(deals - b * block_deals);
        if rem < sz {
            return Some(b * 6 * block_deals + rem + 1);
        }
        rem -= sz;
        b += lanes as u32;
    }
    None
}

/// The `c`-th (0-based) hand of rotation `rot` for a lane (the headless worker plays rotation-major): lane `L` of `N`
/// plays rotation `rot` of the deals in blocks `L, L+N, ...`. Over one rotation every deal appears exactly once.
pub fn lane_rotation_hand_no(planned: u32, block_deals: u32, lanes: usize, lane: usize, rot: u32, c: u32) -> Option<u32> {
    let deals = planned / 6;
    let nblocks = (deals + block_deals - 1) / block_deals;
    let (mut rem, mut b) = (c, lane as u32);
    while b < nblocks {
        let sz = block_deals.min(deals - b * block_deals);
        if rem < sz {
            return Some(b * 6 * block_deals + rot * sz + rem + 1);
        }
        rem -= sz;
        b += lanes as u32;
    }
    None
}

/// Standings from per-hand nets (and AIVAT-corrected payoffs when available). `keys` are the stats
/// keys per participant slot (see `MatchRuntime::slot_keys`).
#[allow(clippy::too_many_arguments)]
pub fn build_standings(
    keys: &[String],
    participants: &[String],
    names: &HashMap<String, String>,
    bb: i64,
    duplicate: bool,
    planned: u32,
    block_deals: u32,
    recs: &[HandRec],
    aivat: Option<&HashMap<u32, [f64; 6]>>,
) -> (Vec<Standing>, Vec<(String, StoredResult)>) {
    let mut b = Builder::new(bb, duplicate);
    for rec in recs {
        let (deal, r) = locate(duplicate, planned, block_deals, rec.hand_no);
        let key = deal.unwrap_or(rec.hand_no);
        b.add_hand(key);
        let av = aivat.and_then(|m| m.get(&rec.hand_no));
        for seat in 0..6usize {
            let slot = (seat + 6 - r as usize) % 6;
            b.add(&keys[slot], key, rec.nets[seat], av.map(|a| a[seat]));
        }
    }
    let names: HashMap<String, String> =
        keys.iter().enumerate().map(|(i, k)| (k.clone(), names.get(&participants[i]).cloned().unwrap_or_default())).collect();
    let (mut st, rows) = b.finish(&names);
    for x in st.iter_mut() {
        x.slot = keys.iter().position(|k| *k == x.player_id).unwrap_or(0) as u8;
        if let Some((p, _)) = x.player_id.split_once('#') {
            x.player_id = p.to_string();
        }
    }
    // stored results only for the first occurrence of each player id
    let rows = rows.into_iter().filter(|(k, _)| !k.contains('#')).collect();
    (st, rows)
}

/// Stats key per participant slot: the player id, with `#slot` on repeats (a house bot may fill
/// several seats), so every slot is scored separately and the first occurrence keeps the plain id.
pub fn slot_keys_of(participants: &[String]) -> Vec<String> {
    let mut seen: Vec<&String> = Vec::new();
    participants
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if seen.contains(&p) {
                format!("{p}#{i}")
            } else {
                seen.push(p);
                p.clone()
            }
        })
        .collect()
}

pub struct NewMatch {
    pub owner: String,
    pub cfg: TableConfig,
    pub participants: Vec<String>,
    pub hands: u32,
    pub duplicate: Option<DuplicateConfig>,
    pub seed: String,
    pub aivat: String,
    pub block_deals: u32,
    pub hide_cards: bool,
    pub lanes: usize,
    pub bench: Option<BenchLink>,
}

/// Register a match and spawn one table task per lane (lanes only apply to duplicate matches).
pub fn create(app: &Arc<AppState>, n: NewMatch) -> crate::error::Res<Arc<MatchRuntime>> {
    let mut names = HashMap::new();
    for p in &n.participants {
        let rec = app.store.get_player(p)?.ok_or_else(|| crate::error::ApiError::not_found(&format!("participant {p}")))?;
        names.insert(rec.player_id.clone(), rec.name.clone());
    }
    let block = n.block_deals.max(1);
    let lanes = if n.duplicate.is_some() {
        let blocks = ((n.hands / 6) + block - 1) / block;
        n.lanes.clamp(1, 32).min(blocks.max(1) as usize)
    } else {
        1
    };
    let table_ids: Vec<String> = (0..lanes).map(|_| new_id("t")).collect();
    let mut cfg = n.cfg.clone();
    cfg.seed = None;
    let rec = MatchRecord {
        match_id: new_id("m"),
        table_id: table_ids[0].clone(),
        participants: n.participants.clone(),
        hands_planned: n.hands,
        duplicate: n.duplicate.clone(),
        seed: n.seed.clone(),
        status: "scheduled".into(),
        hands_played: 0,
        created_at: now_iso(),
        started_at: None,
        finished_at: None,
        table_config: cfg.clone(),
        aivat: n.aivat.clone(),
        table_ids: table_ids.clone(),
        lanes: lanes as u32,
        run_id: n.bench.as_ref().map(|b| b.run_id.clone()),
    };
    app.store.put_match(&rec)?;
    let id = rec.match_id.clone();
    let rt = Arc::new(MatchRuntime::new(rec, names, block, n.hide_cards, n.bench));
    app.matches.write().unwrap().insert(id, rt.clone());
    for (lane, tid) in table_ids.into_iter().enumerate() {
        crate::table::spawn_table(app, tid, n.owner.clone(), cfg.clone(), Some(rt.clone()), lane);
    }
    Ok(rt)
}

/// Start (or resume) a scheduled/paused match and wake its lane tables.
pub fn start(app: &AppState, rt: &MatchRuntime) -> crate::error::Res<()> {
    {
        let mut s = rt.state.lock().unwrap();
        match s.rec.status.as_str() {
            "scheduled" => {
                s.rec.status = "running".into();
                s.rec.started_at = Some(now_iso());
            }
            "paused" => s.rec.status = "running".into(),
            "running" => {}
            _ => return Err(crate::error::ApiError::conflict("match_not_running", format!("match is {}", s.rec.status))),
        }
        app.store.put_match(&s.rec)?;
    }
    wake_all(app, rt);
    Ok(())
}

pub fn wake_all(app: &AppState, rt: &MatchRuntime) {
    let tables = app.tables.read().unwrap();
    for id in &rt.table_ids {
        if let Some(t) = tables.get(id) {
            t.wake.notify_one();
        }
    }
}

/// A lane has run out of hands; the last lane to finish finalizes the match.
pub async fn lane_finished(app: Arc<AppState>, rt: Arc<MatchRuntime>, status: &'static str) {
    if rt.lane_done.fetch_add(1, Ordering::SeqCst) + 1 == rt.lanes as u32 {
        finalize(app, rt, status).await;
    }
}

/// End of a match: AIVAT pass, standings, stored results, final status.
pub async fn finalize(app: Arc<AppState>, rt: Arc<MatchRuntime>, status: &'static str) {
    let t0 = std::time::Instant::now();
    app.writer.flush().await;
    let (mode, played) = {
        let s = rt.state.lock().unwrap();
        (s.rec.aivat.clone(), s.rec.hands_played)
    };
    if played > 0 && mode != "off" {
        let (a2, r2, m2) = (app.clone(), rt.clone(), mode.clone());
        match tokio::task::spawn_blocking(move || compute_aivat(&a2, &r2, &m2)).await {
            Ok(Ok(m)) => rt.state.lock().unwrap().aivat = Some(m),
            Ok(Err(e)) => tracing::error!("match {}: aivat failed: {e}", rt.id),
            Err(e) => tracing::error!("match {}: aivat join: {e}", rt.id),
        }
    }
    let (standings, rows, rec) = {
        let mut s = rt.state.lock().unwrap();
        let (st, rows) = rt.build(&s);
        s.rec.status = status.to_string();
        s.rec.finished_at = Some(now_iso());
        (st, rows, s.rec.clone())
    };
    tracing::info!("match {} finalized ({status}, {played} hands) in {:?}", rt.id, t0.elapsed());
    let rows: Vec<StoredResult> = rows
        .into_iter()
        .map(|(_, mut r)| {
            r.match_id = rt.id.clone();
            r
        })
        .collect();
    if let Err(e) = app.store.put_results(&rt.id, &rows).and_then(|_| app.store.put_match(&rec)) {
        tracing::error!("match {}: store results: {e}", rt.id);
    }
    if let Some(b) = &rt.bench {
        match app.store.get_run(&b.run_id) {
            Ok(Some(mut run)) => {
                let me = standings.iter().find(|x| x.player_id == b.player_id);
                run.status = status.to_string();
                run.hands_done = played;
                run.finished_at = rec.finished_at.clone();
                run.mbb_per_game = me.map(|m| m.mbb_per_game);
                run.ci95_mbb = me.map(|m| m.ci95_mbb);
                run.method = me.map(|m| m.method.clone());
                if let Err(e) = app.store.put_run(&run) {
                    tracing::error!("run {}: {e}", b.run_id);
                }
                let prog = json!({"run_id": b.run_id, "track": b.track, "match_id": rt.id, "status": status,
                    "hands_done": played, "hands_total": rt.planned, "mbb_per_game": run.mbb_per_game,
                    "ci95_mbb": run.ci95_mbb, "method": run.method, "verified": run.verified});
                let env = json!({"v": 1, "type": "benchmark_progress", "t_ms": now_ms(), "data": prog});
                app.hub.send(&b.player_id, &env.to_string());
            }
            Ok(None) => {}
            Err(e) => tracing::error!("run {}: {e}", b.run_id),
        }
    }
    let table = app.tables.read().unwrap().get(&rt.table_id).cloned();
    if let Some(t) = table {
        t.emit(&app.hub, None, "match_update", None, None, json!({"match_id": rt.id, "status": status, "hands_played": played, "standings": standings}));
    }
}

fn compute_aivat(app: &AppState, rt: &MatchRuntime, mode: &str) -> Result<HashMap<u32, [f64; 6]>, String> {
    let mode = if mode == "cards" { Mode::CardsOnly } else { Mode::Full };
    let est = EquityEstimator::default();
    let mut out = HashMap::new();
    let mut after = 0u32;
    loop {
        let page = app.store.list_hands(&rt.id, after, 500).map_err(|e| e.to_string())?;
        if page.is_empty() {
            break;
        }
        after = page.last().unwrap().hand_no;
        let res: Vec<(u32, Option<[f64; 6]>)> =
            page.par_iter().map(|h| (h.hand_no, aivat_hand(&app.house, h, &est, mode))).collect();
        for (n, v) in res {
            if let Some(v) = v {
                out.insert(n, v);
            }
        }
    }
    Ok(out)
}

/// Replay one stored hand through the bench AIVAT tracker. House bots that expose their action
/// probabilities get decision-node corrections; everyone gets chance-node corrections.
/// Returns the corrected payoff per physical seat.
pub fn aivat_hand(house: &crate::house::HouseRegistry, h: &HandHistory, est: &dyn ValueEstimator, mode: Mode) -> Option<[f64; 6]> {
    let (ecfg, deal, order, actions) = crate::hand::replay_parts(h).ok()?;
    let n = order.len();
    let seed = seed64(&h.hand_id);
    let mut players: Vec<Option<Box<dyn Player>>> = (0..n)
        .map(|i| {
            let pid = &h.players.iter().find(|p| p.seat as usize == order[i])?.player_id;
            let key = house.key_of(pid)?;
            let p = house.get(&key)?.new_player(i, &ecfg, mix(seed, i as u64));
            p.exposes_probs().then_some(p)
        })
        .collect();
    let mut sc = Scratch::new(mix(seed, 2));
    let mut tr = Tracker::new(est, &mut sc, mode, deal);
    let mut st = State::new(&ecfg);
    tr.start(&st);
    let mut len = 0usize;
    let mut hist: Vec<Event> = Vec::new();
    for a in actions {
        if st.is_terminal() {
            return None;
        }
        let seat = st.to_act()?;
        if let Some(p) = players[seat].as_mut() {
            let view = View { cfg: &ecfg, state: &st, board: &deal.board[..len], history: &hist, seat };
            let d = p.dist(&view, deal.holes[seat]);
            if let Some(i) = d.actions().iter().position(|x| *x == a) {
                tr.decision(&st, len, &d, i);
            }
        }
        let pre = st;
        let street = st.street();
        let step = st.apply(a);
        hist.push(Event { seat: seat as u8, street, action: a });
        for p in players.iter_mut().flatten() {
            p.observe(&pre, seat, a);
        }
        let reveal = |to: usize, st: &State, len: &mut usize, tr: &mut Tracker, players: &mut Vec<Option<Box<dyn Player>>>| {
            tr.chance(st, *len, to);
            *len = to;
            for p in players.iter_mut().flatten() {
                p.observe_board(&deal.board[..to]);
            }
        };
        match step {
            Step::Continue => {}
            Step::NewStreet(s) => reveal(s.board_len(), &st, &mut len, &mut tr, &mut players),
            Step::Terminal => {
                if st.showdown() {
                    while len < 5 {
                        let to = if len == 0 { 3 } else { len + 1 };
                        reveal(to, &st, &mut len, &mut tr, &mut players);
                    }
                }
            }
        }
    }
    if !st.is_terminal() {
        return None;
    }
    let payoffs = st.payoffs(&deal);
    let q = Query { state: &st, holes: &tr.deal.holes, board: &tr.deal.board[..len.max(if st.showdown() { 5 } else { 0 })] };
    let v = tr.est.value(&q, tr.sc);
    let mut out = [0.0f64; 6];
    for i in 0..n {
        let u = if st.showdown() || st.live_count() == 1 { v[i] } else { payoffs[i] as f64 };
        out[order[i]] = u + tr.corr[i];
    }
    let _ = Action::Fold;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_major_lanes_cover_every_hand_once_and_never_repeat_a_deal_within_a_rotation() {
        for (planned, lanes) in [(20_004u32, 4usize), (20_004, 8), (600, 3), (144, 2), (6, 1), (78, 5)] {
            let mut seen = vec![false; planned as usize + 1];
            for rot in 0..6u32 {
                let mut deals_this_rot = std::collections::HashSet::new();
                for lane in 0..lanes {
                    let mut c = 0;
                    while let Some(h) = lane_rotation_hand_no(planned, 12, lanes, lane, rot, c) {
                        c += 1;
                        assert!(!seen[h as usize], "hand {h} played twice");
                        seen[h as usize] = true;
                        let (deal, r) = locate(true, planned, 12, h);
                        assert_eq!(r, rot, "hand {h} belongs to rotation {r}, not {rot}");
                        assert!(deals_this_rot.insert(deal.unwrap()), "deal {deal:?} twice in rotation {rot}");
                    }
                }
            }
            assert!(seen[1..].iter().all(|&x| x), "{planned}/{lanes}: every hand is played");
        }
    }
}
