//! Subprocess bridge for third-party bots (typically Python): JSON lines over stdin/stdout.
//!
//! One request line per decision, one response line back. The protocol is *stateless*
//! (every request carries the full public hand history), so a pool of long-lived worker
//! processes can be shared by all rayon threads and all seats.
//!
//! Request (one JSON object per line):
//! ```json
//! {"id":7,"seat":2,"players":6,"hole":["As","Kd"],"board":["2c","7h","9d"],"street":1,
//!  "pot":450,"sb":50,"bb":100,"stacks":[..remaining..],"street_bets":[..],"contrib":[..],
//!  "folded":[..bool..],"allin":[..bool..],"cur_bet":150,"call_amount":150,
//!  "legal":{"fold":true,"check":false,"raise":true,"min_raise_to":300,"max_raise_to":9850},
//!  "history":[{"seat":2,"street":0,"kind":"raise","to":250}, ...]}
//! ```
//! Response: `{"id":7,"action":"fold|check|call|raise|allin","to":N}` (`to` = total committed on
//! the street, only for raise). The bridge makes the answer legal: raise sizes are clamped to
//! the legal range, a raise that is impossible becomes a call/check, a fold of a free check
//! becomes a check, and a malformed answer, a timeout or a dead process becomes the fallback
//! (check if free, else fold) and is counted in [`PolicyFactory::report`].
//!
//! Stateful bots (bot name ending in `+obs`): every finished hand is also appended, once per hand, to a
//! shared log as an `observe` line (`{"observe":1,"players":6,"sb":..,"bb":..,"button":5,"holes":[..],
//! "board":[..],"showdown":bool,"history":[..],"who":[entrant per seat]}`, no reply expected). Each
//! worker replays the whole log, in order, before answering its next request, so every worker
//! process holds the same opponent model and the run behaves like one persistent bot. Requests
//! also carry `"who"` (entrant index per seat) so a bot can tell identical opponents apart from itself.

use crate::policy::{Dist, Event, Player, PolicyFactory, View};
use cards::{card_str, Card};
use nlhe::{Action, GameConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use nlhe::Deal;
use std::time::Duration;

pub fn event_json(e: &Event) -> Value {
    let (kind, to) = match e.action {
        Action::Fold => ("fold", 0),
        Action::Check => ("check", 0),
        Action::Call => ("call", 0),
        Action::RaiseTo(t) => ("raise", t),
    };
    json!({"seat": e.seat, "street": e.street.idx(), "kind": kind, "to": to})
}

pub fn request_json(id: u64, v: &View, hole: [Card; 2]) -> Value {
    let st = v.state;
    let n = st.num_players();
    let l = st.legal();
    json!({
        "id": id,
        "seat": v.seat,
        "players": n,
        "hole": [card_str(hole[0]), card_str(hole[1])],
        "board": v.board.iter().map(|&c| card_str(c)).collect::<Vec<_>>(),
        "street": st.street().idx(),
        "pot": st.pot(),
        "sb": v.cfg.small_blind,
        "bb": v.cfg.big_blind,
        "stacks": (0..n).map(|s| st.stack(s)).collect::<Vec<_>>(),
        "street_bets": (0..n).map(|s| st.street_bet(s)).collect::<Vec<_>>(),
        "contrib": (0..n).map(|s| st.contributed(s)).collect::<Vec<_>>(),
        "folded": (0..n).map(|s| st.is_folded(s)).collect::<Vec<_>>(),
        "allin": (0..n).map(|s| st.is_all_in(s)).collect::<Vec<_>>(),
        "cur_bet": st.cur_bet(),
        "call_amount": l.call_amount,
        "legal": {"fold": l.can_fold, "check": l.can_check, "raise": l.can_raise,
                  "min_raise_to": l.min_raise_to, "max_raise_to": l.max_raise_to},
        "history": v.history.iter().map(event_json).collect::<Vec<_>>(),
    })
}

fn fallback(v: &View) -> Action {
    if v.state.legal().can_check {
        Action::Check
    } else {
        Action::Fold
    }
}

/// Parse and legalize a response; `None` if malformed (caller falls back).
pub fn parse_response(line: &str, id: u64, v: &View) -> Option<Action> {
    let r: Value = serde_json::from_str(line).ok()?;
    if r.get("id")?.as_u64()? != id {
        return None;
    }
    let l = v.state.legal();
    let raise = |to: i64| {
        if l.can_raise {
            Action::RaiseTo((to.clamp(l.min_raise_to as i64, l.max_raise_to as i64)) as i32)
        } else if l.can_check {
            Action::Check
        } else {
            Action::Call
        }
    };
    Some(match r.get("action")?.as_str()? {
        "fold" => {
            if l.can_check {
                Action::Check
            } else {
                Action::Fold
            }
        }
        "check" | "call" => {
            if l.can_check {
                Action::Check
            } else {
                Action::Call
            }
        }
        "raise" | "bet" => raise(r.get("to").and_then(|x| x.as_f64()).unwrap_or(0.0).round() as i64),
        "allin" => raise(i64::MAX / 4),
        _ => return None,
    })
}

/// Extra time allowed for a worker's first answer (interpreter + model load on a busy box).
const DEFAULT_STARTUP_GRACE: Duration = Duration::from_secs(180);

struct Worker {
    /// Next entry of the shared observation log this worker has not been sent yet.
    cursor: usize,
    /// False until the first answer: the first request gets `STARTUP_GRACE` extra (model load).
    warm: bool,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl Worker {
    fn spawn(cmd: &[String]) -> std::io::Result<Worker> {
        let mut child = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Worker { cursor: 0, warm: false, child, stdin, rx })
    }
    fn ask(&mut self, pre: &[Arc<str>], req: &Value, id: u64, timeout: Duration, grace: Duration) -> Option<String> {
        for l in pre {
            writeln!(self.stdin, "{l}").ok()?;
        }
        self.cursor += pre.len();
        writeln!(self.stdin, "{req}").ok()?;
        self.stdin.flush().ok()?;
        let timeout = if self.warm { timeout } else { timeout + grace };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(l) => {
                    // Skip stale answers to earlier (timed-out) ids.
                    if serde_json::from_str::<Value>(&l).ok().and_then(|v| v.get("id")?.as_u64()) == Some(id) {
                        self.warm = true;
                        return Some(l);
                    }
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct ExtBot {
    name: String,
    cmd: Vec<String>,
    timeout: Duration,
    grace: Duration,
    pool: Vec<Mutex<Option<Worker>>>,
    next: AtomicU64,
    ids: AtomicU64,
    requests: AtomicU64,
    fallbacks: AtomicU64,
    restarts: AtomicU64,
    /// Feed finished hands to the workers (bot name ends in `+obs`).
    observe: bool,
    log: Mutex<Vec<Arc<str>>>,
}

impl ExtBot {
    /// `cmd` = program + args, e.g. `["python3","-I","bot.py","model.pt"]`.
    pub fn new(name: &str, cmd: Vec<String>, workers: usize, timeout: Duration) -> Arc<ExtBot> {
        assert!(!cmd.is_empty() && workers > 0);
        Arc::new(ExtBot {
            name: name.to_string(),
            cmd,
            timeout,
            grace: DEFAULT_STARTUP_GRACE,
            pool: (0..workers).map(|_| Mutex::new(None)).collect(),
            next: AtomicU64::new(0),
            ids: AtomicU64::new(1),
            requests: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            observe: name.ends_with("+obs"),
            log: Mutex::new(Vec::new()),
        })
    }

    /// Extra time for a worker's first answer (default 180 s: interpreter + model load).
    pub fn with_grace(self: Arc<Self>, grace: Duration) -> Arc<Self> {
        let mut me = Arc::try_unwrap(self).ok().expect("unshared");
        me.grace = grace;
        Arc::new(me)
    }

    /// As a bench `PolicyFactory`.
    pub fn factory(self: Arc<Self>) -> Arc<dyn PolicyFactory> {
        Arc::new(self)
    }

    fn decide(&self, v: &View, hole: [Card; 2], who: &[usize]) -> Action {
        let id = self.ids.fetch_add(1, Relaxed);
        self.requests.fetch_add(1, Relaxed);
        let mut req = request_json(id, v, hole);
        if !who.is_empty() {
            req["who"] = json!(who);
        }
        let n = self.pool.len();
        let start = self.next.fetch_add(1, Relaxed) as usize % n;
        let mut guard = (0..n)
            .find_map(|k| self.pool[(start + k) % n].try_lock().ok())
            .unwrap_or_else(|| self.pool[start].lock().unwrap());
        if guard.is_none() {
            *guard = Worker::spawn(&self.cmd).ok();
        }
        let pre: Vec<Arc<str>> = match (self.observe, guard.as_ref()) {
            (true, Some(w)) => {
                let l = self.log.lock().unwrap();
                l[w.cursor.min(l.len())..].to_vec()
            }
            _ => Vec::new(),
        };
        let answer = guard.as_mut().and_then(|w| w.ask(&pre, &req, id, self.timeout, self.grace));
        match answer.as_deref().and_then(|l| parse_response(l, id, v)) {
            Some(a) => a,
            None => {
                self.fallbacks.fetch_add(1, Relaxed);
                // No answer at all (hung/dead worker): drop it (kills it); the next call respawns.
                // A well-formed id'd but invalid answer (e.g. an error object) keeps the worker.
                if answer.is_none() && guard.take().is_some() {
                    self.restarts.fetch_add(1, Relaxed);
                }
                fallback(v)
            }
        }
    }
}

impl PolicyFactory for Arc<ExtBot> {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn new_player(&self, seat: usize, cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(ExtPlayer { bot: self.clone(), seat, who: Vec::new(), blinds: (cfg.small_blind, cfg.big_blind), stacks: cfg.stacks.unwrap_or([cfg.stack; nlhe::MAX_PLAYERS])[..cfg.players].to_vec() })
    }
    fn new_player_ctx(&self, seat: usize, cfg: &GameConfig, _seed: u64, who: &[usize]) -> Box<dyn Player> {
        Box::new(ExtPlayer { bot: self.clone(), seat, who: who.to_vec(), blinds: (cfg.small_blind, cfg.big_blind), stacks: cfg.stacks.unwrap_or([cfg.stack; nlhe::MAX_PLAYERS])[..cfg.players].to_vec() })
    }
    fn report(&self) -> Option<String> {
        Some(
            json!({"bot": self.name, "requests": self.requests.load(Relaxed),
                   "fallbacks": self.fallbacks.load(Relaxed), "restarts": self.restarts.load(Relaxed)})
            .to_string(),
        )
    }
}

struct ExtPlayer {
    bot: Arc<ExtBot>,
    seat: usize,
    who: Vec<usize>,
    blinds: (i32, i32),
    stacks: Vec<i32>,
}

impl Player for ExtPlayer {
    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        Dist::pure(self.bot.decide(v, hole, &self.who))
    }
    fn exposes_probs(&self) -> bool {
        false
    }
    fn act(&mut self, v: &View, hole: [Card; 2], _u: f64) -> Action {
        self.bot.decide(v, hole, &self.who)
    }
    fn finish(&mut self, deal: &Deal, hist: &[Event], end: &nlhe::State) {
        if !self.bot.observe || self.who.is_empty() {
            return;
        }
        // One observation per hand: only the lowest seat held by this bot's entrant reports it.
        let mine = self.who[self.seat];
        if (0..self.seat).any(|s| self.who[s] == mine) {
            return;
        }
        let n = self.who.len();
        let showdown = end.showdown();
        let blen = if showdown { 5 } else { hist.last().map_or(0, |e| e.street.board_len()) };
        let msg = json!({
            "observe": 1, "players": n, "stacks": self.stacks, "sb": self.blinds.0, "bb": self.blinds.1, "button": n - 1,
            "holes": (0..n).map(|s| [card_str(deal.holes[s][0]), card_str(deal.holes[s][1])]).collect::<Vec<_>>(),
            "board": deal.board[..blen].iter().map(|&c| card_str(c)).collect::<Vec<_>>(),
            "showdown": showdown,
            "history": hist.iter().map(event_json).collect::<Vec<_>>(),
            "who": self.who,
        });
        self.bot.log.lock().unwrap().push(Arc::from(msg.to_string()));
    }
}

/// Resolve an external/rule-bot spec (`chart6`, `chart6-loose`, `chart6-tight`, `rlcard:<ckpt>`,
/// `ext:<name>=<json argv>`) to a factory; `None` if the spec is not one of these. `py`/`dir` locate the
/// python interpreter and the `arena/adapters` directory for `rlcard:`.
pub fn parse_external(base: &str, py: &str, dir: &str, workers: usize, timeout: Duration) -> Option<Result<Arc<dyn PolicyFactory>, String>> {
    if let Some(c) = crate::chart::Chart6::by_name(base) {
        return Some(Ok(Arc::new(c)));
    }
    if let Some(model) = base.strip_prefix("rlcard:") {
        let cmd = vec![py.to_string(), "-I".into(), format!("{dir}/rlcard_bot_server.py"), model.to_string()];
        return Some(Ok(ExtBot::new("rlcard", cmd, workers, timeout).factory()));
    }
    if let Some(json) = base.strip_prefix("ext:") {
        return Some((|| {
            let (name, arr) = json.split_once('=').ok_or("ext:<name>=<json argv>")?;
            let cmd: Vec<String> = serde_json::from_str(arr).map_err(|e| format!("bad ext argv: {e}"))?;
            Ok(ExtBot::new(name, cmd, workers, timeout).factory())
        })());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchrun::play_hand;
    use cards::Rng;
    use nlhe::{Deal, State};

    fn view_fixture<'a>(cfg: &'a GameConfig, st: &'a State, hist: &'a [Event]) -> View<'a> {
        View { cfg, state: st, board: &[], history: hist, seat: st.to_act().unwrap() }
    }

    #[test]
    fn request_has_full_public_state() {
        let cfg = GameConfig::new(6);
        let st = State::new(&cfg);
        let v = view_fixture(&cfg, &st, &[]);
        let r = request_json(3, &v, [cards::parse_card("As").unwrap(), cards::parse_card("Kd").unwrap()]);
        assert_eq!(r["seat"], 2);
        assert_eq!(r["hole"], json!(["As", "Kd"]));
        assert_eq!(r["pot"], 150);
        assert_eq!(r["call_amount"], 100);
        assert_eq!(r["legal"]["fold"], true);
        assert_eq!(r["stacks"].as_array().unwrap().len(), 6);
        assert_eq!(r["contrib"][1], 100);
    }

    #[test]
    fn responses_are_legalized() {
        let cfg = GameConfig::new(6);
        let st = State::new(&cfg);
        let v = view_fixture(&cfg, &st, &[]);
        let l = st.legal();
        // raise below min -> min; above max -> max
        assert_eq!(parse_response(r#"{"id":1,"action":"raise","to":5}"#, 1, &v), Some(Action::RaiseTo(l.min_raise_to)));
        assert_eq!(parse_response(r#"{"id":1,"action":"raise","to":99999999}"#, 1, &v), Some(Action::RaiseTo(l.max_raise_to)));
        assert_eq!(parse_response(r#"{"id":1,"action":"allin"}"#, 1, &v), Some(Action::RaiseTo(l.max_raise_to)));
        assert_eq!(parse_response(r#"{"id":1,"action":"check"}"#, 1, &v), Some(Action::Call)); // cannot check facing the BB
        assert_eq!(parse_response(r#"{"id":2,"action":"call"}"#, 1, &v), None); // wrong id
        assert_eq!(parse_response("garbage", 1, &v), None);
        assert_eq!(parse_response(r#"{"id":1,"action":"dance"}"#, 1, &v), None);
        // free check: fold becomes check
        let mut st2 = st;
        for a in [Action::Call, Action::Call, Action::Call, Action::Call, Action::Call] {
            st2.apply(a);
        }
        let v2 = view_fixture(&cfg, &st2, &[]);
        assert!(st2.legal().can_check);
        assert_eq!(parse_response(r#"{"id":1,"action":"fold"}"#, 1, &v2), Some(Action::Check));
    }

    const PY_ECHO: &str = "import sys,json\nfor l in sys.stdin:\n r=json.loads(l)\n print(json.dumps({'id':r['id'],'action':'raise','to':r['cur_bet']*3}),flush=True)\n";

    fn py(code: &str) -> Vec<String> {
        vec!["python3".into(), "-I".into(), "-c".into(), code.into()]
    }

    #[test]
    fn subprocess_roundtrip_plays_full_hands() {
        let bot = ExtBot::new("pyraiser", py(PY_ECHO), 2, Duration::from_secs(5)).with_grace(Duration::ZERO);
        let cfg = GameConfig::new(6);
        let mut rng = Rng::new(1);
        for k in 0..10 {
            let deal = Deal::random(&mut rng);
            let mut ps: Vec<_> = (0..6).map(|s| bot.new_player(s, &cfg, k)).collect();
            let o = play_hand(&cfg, &deal, &mut ps, &mut Rng::new(k), None);
            assert_eq!(o.payoffs.iter().sum::<i32>(), 0);
        }
        let rep: Value = serde_json::from_str(&bot.report().unwrap()).unwrap();
        assert!(rep["requests"].as_u64().unwrap() > 10);
        assert_eq!(rep["fallbacks"], 0);
    }

    #[test]
    fn timeout_and_crash_fall_back_safely() {
        // Never answers: every decision times out -> fallback (fold/check); hands still finish.
        let hung = ExtBot::new("hung", py("import sys,time\nfor l in sys.stdin:\n time.sleep(60)\n"), 1, Duration::from_millis(150)).with_grace(Duration::ZERO);
        // Crashes on first request.
        let crash = ExtBot::new("crash", py("import sys\nsys.stdin.readline()\nsys.exit(3)\n"), 1, Duration::from_millis(500));
        let cfg = GameConfig::new(6);
        for bot in [hung, crash] {
            let deal = Deal::random(&mut Rng::new(9));
            let mut ps: Vec<_> = (0..6).map(|s| bot.new_player(s, &cfg, 0)).collect();
            let o = play_hand(&cfg, &deal, &mut ps, &mut Rng::new(0), None);
            assert_eq!(o.payoffs.iter().sum::<i32>(), 0);
            let rep: Value = serde_json::from_str(&bot.report().unwrap()).unwrap();
            assert!(rep["fallbacks"].as_u64().unwrap() >= 1, "{rep}");
        }
    }
    #[test]
    fn parse_external_specs() {
        let t = Duration::from_secs(1);
        assert!(parse_external("chart6-tight", "python3", "x", 1, t).unwrap().is_ok());
        assert!(parse_external("tag", "python3", "x", 1, t).is_none());
        assert!(parse_external("ext:a", "python3", "x", 1, t).unwrap().is_err());
        assert!(parse_external("ext:a=[\"python3\"]", "python3", "x", 1, t).unwrap().is_ok());
    }
}
