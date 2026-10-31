//! Headless match worker: `sharingan-arena run --bot "<command>" --track ladder-v1 ...`.
//!
//! No HTTP server, no database. The worker plays the duplicate benchmark of a track (same frozen
//! panel, seed, deal schedule, house-bot randomness and AIVAT as the server's benchmark runs) against
//! a bot that runs as a SUBPROCESS and talks newline-delimited JSON over its stdin/stdout (the "pipe
//! protocol", TABLE_API.md section 14). One bot process per lane; lanes are OS threads.
//!
//! Failure policy (documented in TABLE_API.md 14.6):
//! * a decision that does not arrive in `decision_timeout_ms` is played as check-if-free-else-fold
//!   (`timeout:true`), the process group is killed and a fresh process takes over (up to `max_restarts`
//!   per lane, then the lane is dead);
//! * a crash (exit, EOF, protocol garbage, `bye`) kills the lane: the bot folds (or checks when free)
//!   every remaining decision, i.e. the remaining hands of that lane are FORFEITS. They are still
//!   dealt, played and AIVAT-scored (so `hands` == `hands_planned`), and counted in `forfeit_hands`.
//!
//! The result is signed with HMAC-SHA256 over its canonical JSON (`--sign-key-file`), see
//! `sign_result` / `verify_result`; `POST /v1/results` (results.rs) accepts it as verified.

use crate::benchmarks::{self, Track};
use crate::config::ServerConfig;
use crate::deck;
use crate::hand::{ActReq, HandRunner, HandSetup, SeatInfo};
use crate::house::{house_id, HouseRegistry};
use crate::matches::{self, HandRec};
use crate::model::*;
use crate::util::*;
use bench::aivat::{EquityEstimator, Mode};
use bench::mix;
use bench::policy::Player;
use nlhe::Action;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const PROTOCOL: u32 = 1;
pub const RESULT_SCHEMA: &str = "sharingan-worker-result/1";
pub const BOT_ID: &str = "bot";
const BLOCK_DEALS: u32 = 12;
const MAX_LINE: usize = 4 << 20;

#[derive(Clone, Debug)]
pub struct WorkerOpts {
    pub bot_cmd: String,
    pub track: String,
    pub hands: u32,
    pub lanes: usize,
    /// `None` = the track's frozen seed (comparable with server runs). A secret per-run seed (`run:<hex>`, from `--seed-file`)
    /// gives fresh deals; it never reaches the bot (see `hand_prefix`).
    pub seed: Option<String>,
    pub decision_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub max_restarts: u32,
    pub player_id: Option<String>,
    /// "full" (default, as the server), "cards" or "off".
    pub aivat: String,
    pub quiet: bool,
    /// Write one JSON line per hand (hand_no, deal_no, rotation, deck digest, nets) here.
    pub hands_out: Option<String>,
    /// House-bot configuration (blueprint, buckets, sizing); `None` = from the environment (`ARENA_BLUEPRINT`, ...).
    pub house_cfg: Option<ServerConfig>,
    /// Isolation for duplicate deals (`--rotation-uids`, the hosted runner always sets it): rotation r's bot processes run
    /// as uid `rotation_uids[r]` (the worker must be root), and `scrub_cmd UID` ends everything that uid left behind
    /// (processes, files, IPC) before the next rotation starts. Each deal appears once per rotation, so with this no bot
    /// process can remember a deal it saw from another seat.
    pub rotation_uids: Option<[u32; 6]>,
    pub scrub_cmd: String,
}

impl Default for WorkerOpts {
    fn default() -> Self {
        WorkerOpts {
            bot_cmd: String::new(),
            track: "ladder-v1".into(),
            hands: benchmarks::DEFAULT_HANDS,
            lanes: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8),
            seed: None,
            decision_timeout_ms: 5000,
            startup_timeout_ms: 30_000,
            max_restarts: 3,
            player_id: None,
            aivat: "full".into(),
            rotation_uids: None,
            scrub_cmd: "sharingan-scrub".into(),
            quiet: false,
            hands_out: None,
            house_cfg: None,
        }
    }
}

// ------------------------------------------------------------------------------------ pipe IO

#[derive(Debug)]
pub enum IoErr {
    Timeout,
    Eof,
    Broken(String),
}

fn remaining_ms(deadline: Instant) -> Option<i32> {
    let d = deadline.checked_duration_since(Instant::now())?;
    if d.is_zero() {
        return None;
    }
    Some(((d.as_micros() + 999) / 1000).min(i32::MAX as u128) as i32)
}

fn set_nonblocking(fd: RawFd) {
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
    }
}

fn poll_fd(fd: RawFd, events: i16, deadline: Instant) -> Result<(), IoErr> {
    let Some(ms) = remaining_ms(deadline) else { return Err(IoErr::Timeout) };
    let mut p = libc::pollfd { fd, events, revents: 0 };
    let r = unsafe { libc::poll(&mut p, 1, ms) };
    if r == 0 {
        return Err(IoErr::Timeout);
    }
    Ok(())
}

/// Copy a bot's stderr into the worker's, line by line, up to 1 MiB per process (the rest is drained and dropped), so
/// the bot's own prints still reach the job log. The thread ends when the last holder of the pipe exits.
fn forward_stderr(err: std::process::ChildStderr) {
    std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        let (mut left, mut cut) = (1usize << 20, false);
        for line in io::BufReader::new(err).split(b'\n') {
            let Ok(line) = line else { break };
            if line.len() < left {
                left -= line.len() + 1;
                let mut e = io::stderr().lock();
                let _ = e.write_all(&line);
                let _ = e.write_all(b"\n");
            } else if !cut {
                cut = true;
                let _ = writeln!(io::stderr(), "[bot stderr cut at 1 MiB]");
            }
        }
    });
}

/// A bot subprocess. Both pipes are non-blocking and driven with `poll` against a deadline, so a bot
/// that stops reading or never answers can never hang the lane thread.
pub struct BotProc {
    child: Child,
    _stdin: ChildStdin,
    _stdout: ChildStdout,
    wfd: RawFd,
    rfd: RawFd,
    buf: Vec<u8>,
    start: usize,
}

impl BotProc {
    pub fn spawn(cmd: &str, lane: usize, lanes: usize) -> io::Result<BotProc> {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        // stderr: a pipe of its own, copied into ours (`forward_stderr`). Never our stderr itself: a bot that shares the
        // worker's log file could read or measure what an earlier rotation wrote there.
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // the bot must not inherit the operator's secrets (or a seed) from the worker's environment
        for k in ["ARENA_ADMIN_TOKEN", "ARENA_RUNNER_TOKEN", "ARENA_WORKER_KEYS", "ARENA_DATABASE_URL", "ARENA_SEED", "SHARINGAN_SEED"] {
            c.env_remove(k);
        }
        c.env("SHARINGAN_PIPE", "1").env("SHARINGAN_LANE", lane.to_string()).env("SHARINGAN_LANES", lanes.to_string());
        c.process_group(0);
        let mut child = c.spawn()?;
        if let Some(err) = child.stderr.take() {
            forward_stderr(err);
        }
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (wfd, rfd) = (stdin.as_raw_fd(), stdout.as_raw_fd());
        set_nonblocking(wfd);
        set_nonblocking(rfd);
        Ok(BotProc { child, _stdin: stdin, _stdout: stdout, wfd, rfd, buf: Vec::with_capacity(1 << 16), start: 0 })
    }

    pub fn send(&mut self, line: &str, deadline: Instant) -> Result<(), IoErr> {
        let mut data = Vec::with_capacity(line.len() + 1);
        data.extend_from_slice(line.as_bytes());
        data.push(b'\n');
        let mut off = 0;
        while off < data.len() {
            let n = unsafe { libc::write(self.wfd, data[off..].as_ptr() as *const libc::c_void, data.len() - off) };
            if n >= 0 {
                off += n as usize;
                continue;
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => poll_fd(self.wfd, libc::POLLOUT, deadline)?,
                io::ErrorKind::Interrupted => {}
                io::ErrorKind::BrokenPipe => return Err(IoErr::Eof),
                _ => return Err(IoErr::Broken(e.to_string())),
            }
        }
        Ok(())
    }

    pub fn recv(&mut self, deadline: Instant) -> Result<String, IoErr> {
        let mut scan = self.start;
        loop {
            if let Some(p) = self.buf[scan..].iter().position(|&b| b == b'\n') {
                let end = scan + p;
                let line = String::from_utf8_lossy(&self.buf[self.start..end]).into_owned();
                self.start = end + 1;
                if self.start == self.buf.len() {
                    self.buf.clear();
                    self.start = 0;
                }
                return Ok(line);
            }
            scan = self.buf.len();
            // A bot that keeps the pipe busy (a flood of noise, a never-ending line) never makes `read` block, so
            // the poll below would never see the deadline: check it on every pass.
            if Instant::now() >= deadline {
                return Err(IoErr::Timeout);
            }
            if self.buf.len() - self.start > MAX_LINE {
                return Err(IoErr::Broken("line longer than 4 MiB".into()));
            }
            if self.start > (1 << 20) {
                self.buf.drain(..self.start);
                scan -= self.start;
                self.start = 0;
            }
            let mut tmp = [0u8; 65536];
            let n = unsafe { libc::read(self.rfd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
            if n > 0 {
                self.buf.extend_from_slice(&tmp[..n as usize]);
                continue;
            }
            if n == 0 {
                return Err(IoErr::Eof);
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => poll_fd(self.rfd, libc::POLLIN, deadline)?,
                io::ErrorKind::Interrupted => {}
                _ => return Err(IoErr::Broken(e.to_string())),
            }
        }
    }

    /// SIGKILL the whole process group (the `sh -c` wrapper and whatever it started) and reap.
    pub fn kill(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        // Never block on a process that does not die (a different uid we may not signal, an uninterruptible
        // state): close our end of its stdin so it sees EOF, wait a bounded time, then leave it to init.
        let t0 = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) if t0.elapsed() > Duration::from_secs(3) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        self.close_stdin();
    }

    /// Replace our write end of the bot's stdin by /dev/null so the bot reads EOF.
    fn close_stdin(&mut self) {
        unsafe {
            let nul = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
            if nul >= 0 {
                libc::dup2(nul, self.wfd);
                libc::close(nul);
            }
        }
    }

    /// Polite end: `bye`, close stdin, wait a little, then kill.
    pub fn finish(&mut self, bye: &str) {
        let _ = self.send(bye, Instant::now() + Duration::from_millis(200));
        let t0 = Instant::now();
        self.close_stdin(); // the bot reads EOF
        while t0.elapsed() < Duration::from_millis(1500) {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.kill();
    }
}

impl Drop for BotProc {
    fn drop(&mut self) {
        self.kill();
    }
}

// ------------------------------------------------------------------------------------ canonical JSON + signature

/// Canonical JSON: UTF-8, object keys sorted (recursively), no whitespace, integers as integers, floats with
/// exactly 6 decimals (`{:.6}`). This is what is signed.
pub fn canonical_json(v: &Value) -> String {
    let mut s = String::new();
    canon(v, &mut s);
    s
}

fn canon(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                canon(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon(x, out);
            }
            out.push(']');
        }
        // Floats are written with exactly 6 decimals: a 1-ulp difference between two JSON libraries' parsers
        // (serde_json's default float parsing is not exactly round-trip) must not change the signed text.
        Value::Number(n) if n.is_f64() => out.push_str(&format!("{:.6}", n.as_f64().unwrap())),
        other => out.push_str(&other.to_string()),
    }
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// `id=secret` or just `secret` (id `default`).
pub fn parse_key_spec(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.split_once('=') {
        Some((id, k)) if !id.is_empty() && !k.is_empty() && !id.contains(char::is_whitespace) => Some((id.trim().into(), k.trim().into())),
        _ => Some(("default".into(), s.into())),
    }
}

/// `ARENA_WORKER_KEYS`: comma separated `id=secret` entries.
pub fn parse_keys(s: &str) -> Vec<(String, String)> {
    s.split(',').filter_map(parse_key_spec).collect()
}

fn signing_input(result: &Value) -> String {
    let mut v = result.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("signature");
    }
    canonical_json(&v)
}

/// Add `signature: {alg, key_id, value}` (HMAC-SHA256 over the canonical result without `signature`).
pub fn sign_result(result: &mut Value, key_id: &str, secret: &str) {
    let mac = hex::encode(hmac_sha256(secret.as_bytes(), signing_input(result).as_bytes()));
    result["signature"] = json!({"alg": "HMAC-SHA256", "key_id": key_id, "value": mac});
}

/// Check the signature of `result` against the configured keys. Returns the matching key id.
pub fn verify_result(result: &Value, keys: &[(String, String)]) -> Result<String, &'static str> {
    let sig = result.get("signature").and_then(|s| s.as_object()).ok_or("unsigned result")?;
    if sig.get("alg").and_then(|a| a.as_str()).map(|a| !a.eq_ignore_ascii_case("hmac-sha256")).unwrap_or(false) {
        return Err("unsupported signature algorithm");
    }
    let value = sig.get("value").and_then(|v| v.as_str()).ok_or("signature has no value")?;
    if keys.is_empty() {
        return Err("no worker keys configured (ARENA_WORKER_KEYS)");
    }
    let input = signing_input(result);
    let want_id = sig.get("key_id").and_then(|v| v.as_str());
    for (id, secret) in keys {
        if want_id.map(|w| w != id).unwrap_or(false) {
            continue;
        }
        let mac = hex::encode(hmac_sha256(secret.as_bytes(), input.as_bytes()));
        if ct_eq(&mac, &value.to_ascii_lowercase()) {
            return Ok(id.clone());
        }
    }
    Err("bad signature")
}

// ------------------------------------------------------------------------------------ the match

#[derive(Default, Clone, Debug)]
struct LaneStats {
    lane: usize,
    hands: u32,
    decisions: u64,
    timeouts: u32,
    restarts: u32,
    illegal_actions: u32,
    forfeit_hands: u32,
    noise_lines: u64,
    anomalies: u32,
    crashed: Option<String>,
    wall_ms: u64,
    bot_wait_ms: f64,
    aivat_ms: f64,
}

struct LaneOut {
    recs: Vec<HandRec>,
    aivat: HashMap<u32, [f64; 6]>,
    log: Vec<(u32, Option<u32>, u32, u64, [i64; 6])>,
    stats: LaneStats,
}

struct Cx<'a> {
    opts: &'a WorkerOpts,
    house: &'a HouseRegistry,
    est: &'a EquityEstimator,
    mode: Option<Mode>,
    participants: Vec<String>,
    cfg: TableConfig,
    seed: String,
    seed64: u64,
    /// Prefix of every `hand_id` the bot sees. The track seed itself for the frozen deals (public anyway);
    /// for any other seed a one-way hash of it, so a secret per-run seed never reaches the bot.
    hand_prefix: String,
    planned: u32,
    lanes: usize,
    done: AtomicU32,
    /// The deals come from a secret per-run seed (not the public track seed).
    seed_secret: bool,
}

enum Decision {
    Act(Action, u64),
    Timeout(u64),
}

struct Lane<'a> {
    cx: &'a Cx<'a>,
    lane: usize,
    table_id: String,
    bot: Option<BotProc>,
    /// The bot command of the rotation being played (wrapped for its uid when isolated).
    bot_cmd: String,
    table_seq: u64,
    st: LaneStats,
}

fn envelope(kind: &str, table_id: &str, hand_id: Option<&str>, seq: Option<u32>, table_seq: u64, data: &str) -> String {
    let mut s = String::with_capacity(data.len() + 160);
    s.push_str("{\"v\":1,\"type\":\"");
    s.push_str(kind);
    s.push_str("\",\"table_id\":\"");
    s.push_str(table_id);
    s.push('"');
    if let Some(h) = hand_id {
        s.push_str(",\"hand_id\":\"");
        s.push_str(h);
        s.push('"');
    }
    if let Some(q) = seq {
        s.push_str(",\"seq\":");
        s.push_str(&q.to_string());
    }
    s.push_str(",\"table_seq\":");
    s.push_str(&table_seq.to_string());
    s.push_str(",\"t_ms\":");
    s.push_str(&now_ms().to_string());
    s.push_str(",\"data\":");
    s.push_str(data);
    s.push('}');
    s
}

impl<'a> Lane<'a> {
    fn new(cx: &'a Cx<'a>, lane: usize) -> Self {
        Lane { cx, lane, table_id: format!("lane-{lane}"), bot: None, bot_cmd: String::new(), table_seq: 0, st: LaneStats { lane, ..Default::default() } }
    }

    fn alive(&self) -> bool {
        self.st.crashed.is_none()
    }

    fn die(&mut self, why: String) {
        if self.st.crashed.is_none() {
            self.st.crashed = Some(why);
        }
        if let Some(mut b) = self.bot.take() {
            b.kill();
        }
    }

    /// Start the bot process and do the hello/welcome handshake.
    fn start_bot(&mut self) -> Result<(), String> {
        let o = self.cx.opts;
        let mut b = BotProc::spawn(&self.bot_cmd, self.lane, self.cx.lanes).map_err(|e| format!("spawn failed: {e}"))?;
        let deadline = Instant::now() + Duration::from_millis(o.startup_timeout_ms);
        loop {
            if Instant::now() >= deadline {
                return Err(format!("no hello within {} ms", o.startup_timeout_ms));
            }
            let line = match b.recv(deadline) {
                Ok(l) => l,
                Err(IoErr::Timeout) => return Err(format!("no hello within {} ms", o.startup_timeout_ms)),
                Err(IoErr::Eof) => return Err("bot exited before hello".into()),
                Err(IoErr::Broken(e)) => return Err(e),
            };
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                self.st.noise_lines += 1;
                continue;
            };
            if v["type"] != "hello" {
                continue;
            }
            if let Some(p) = v["data"]["protocol"].as_u64() {
                if p > PROTOCOL as u64 {
                    return Err(format!("bot speaks pipe protocol {p}, this worker {PROTOCOL}"));
                }
            }
            break;
        }
        let data = json!({
            "player_id": BOT_ID, "name": BOT_ID, "protocol": PROTOCOL, "mode": "benchmark",
            "tables": [self.table_id], "lane": self.lane, "lanes": self.cx.lanes, "track": o.track,
            "decision_time_ms": o.decision_timeout_ms, "server": format!("sharingan-arena-worker/{}", env!("CARGO_PKG_VERSION")),
        });
        self.table_seq += 1;
        let env = envelope("welcome", &self.table_id, None, None, self.table_seq, &data.to_string());
        b.send(&env, Instant::now() + Duration::from_millis(o.startup_timeout_ms)).map_err(|e| format!("welcome not delivered: {e:?}"))?;
        self.bot = Some(b);
        Ok(())
    }

    /// After a hang: kill and replace the process (state of the old one is lost), or give up.
    fn restart_after_timeout(&mut self) {
        if let Some(mut b) = self.bot.take() {
            b.kill();
        }
        if self.st.restarts >= self.cx.opts.max_restarts {
            self.die(format!("decision timeouts: gave up after {} restarts", self.st.restarts));
            return;
        }
        self.st.restarts += 1;
        if let Err(e) = self.start_bot() {
            self.die(format!("restart failed: {e}"));
        }
    }

    /// Ask the bot for the pending decision. `turn` is the `your_turn` envelope line.
    fn ask(&mut self, runner: &HandRunner, decision_id: &str, turn: &str) -> Decision {
        let t0 = Instant::now();
        let deadline = t0 + Duration::from_millis(self.cx.opts.decision_timeout_ms);
        let think = |t0: Instant| t0.elapsed().as_millis() as u64;
        let Some(bot) = self.bot.as_mut() else { return Decision::Timeout(0) };
        if let Err(e) = bot.send(turn, deadline) {
            return match e {
                IoErr::Timeout => self.timed_out(t0),
                IoErr::Eof => {
                    self.die("bot exited mid-game (closed its stdin)".into());
                    Decision::Timeout(think(t0))
                }
                IoErr::Broken(m) => {
                    self.die(m);
                    Decision::Timeout(think(t0))
                }
            };
        }
        loop {
            if Instant::now() >= deadline {
                return self.timed_out(t0);
            }
            let bot = self.bot.as_mut().unwrap();
            let line = match bot.recv(deadline) {
                Ok(l) => l,
                Err(IoErr::Timeout) => return self.timed_out(t0),
                Err(IoErr::Eof) => {
                    let code = bot.child.try_wait().ok().flatten().map(|s| s.to_string()).unwrap_or_default();
                    self.die(format!("bot exited mid-game {code}").trim().to_string());
                    return Decision::Timeout(think(t0));
                }
                Err(IoErr::Broken(m)) => {
                    self.die(m);
                    return Decision::Timeout(think(t0));
                }
            };
            if !line.starts_with('{') {
                self.st.noise_lines += 1;
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                self.st.noise_lines += 1;
                continue;
            };
            match v["type"].as_str() {
                Some("act") => {
                    let d = &v["data"];
                    if d["decision_id"].as_str() != Some(decision_id) {
                        continue; // stale answer to an earlier decision
                    }
                    let req = ActReq { kind: d["type"].as_str().unwrap_or("").to_string(), to: d["to"].as_i64() };
                    match runner.validate(&req) {
                        Ok(a) => {
                            self.st.bot_wait_ms += t0.elapsed().as_secs_f64() * 1000.0;
                            return Decision::Act(a, think(t0));
                        }
                        Err(e) => {
                            self.st.illegal_actions += 1;
                            let data = json!({"code": e.code, "message": e.message, "details": e.details, "decision_id": decision_id});
                            self.table_seq += 1;
                            let env = envelope("error", &self.table_id, None, None, self.table_seq, &data.to_string());
                            let bot = self.bot.as_mut().unwrap();
                            if bot.send(&env, deadline).is_err() || bot.send(turn, deadline).is_err() {
                                return self.timed_out(t0);
                            }
                        }
                    }
                }
                Some("bye") => {
                    self.die("bot said bye".into());
                    return Decision::Timeout(think(t0));
                }
                _ => {} // hello / ping / unknown: ignored
            }
        }
    }

    fn timed_out(&mut self, t0: Instant) -> Decision {
        self.st.timeouts += 1;
        self.st.bot_wait_ms += t0.elapsed().as_secs_f64() * 1000.0;
        self.restart_after_timeout();
        Decision::Timeout(t0.elapsed().as_millis() as u64)
    }

    fn play_hand(&mut self, hand_no: u32) -> Option<(HandRec, Option<[f64; 6]>, (u32, Option<u32>, u32, u64, [i64; 6]))> {
        let cx = self.cx;
        let (deal_no, rotation) = matches::locate(true, cx.planned, BLOCK_DEALS, hand_no);
        let deal = deal_no.unwrap();
        let button = (deal % 6) as usize;
        let mut seats = Vec::with_capacity(6);
        for s in 0..6usize {
            let pid = &cx.participants[(s + 6 - rotation as usize) % 6];
            seats.push(SeatInfo { seat: s, player_id: pid.clone(), name: pid.clone(), kind: "bot".into(), stack: cx.cfg.starting_stack });
        }
        let dk = deck::deal_deck(&cx.seed, deal as u64);
        // With a secret seed the id is opaque (keyed by the seed): hand ids would otherwise number the deals, and a bot that
        // kept notes could match a deal across rotations by its number. Frozen (public) deals keep readable ids.
        let hand_id = if cx.seed_secret {
            format!("{}-{}", cx.hand_prefix, &sha256_hex(format!("hand:{}:{hand_no}", cx.seed).as_bytes())[..12])
        } else {
            format!("{}-h{}", cx.hand_prefix, hand_no)
        };
        let deck_digest = seed64(&cards::cards_str(&dk));
        let setup = HandSetup {
            table_id: self.table_id.clone(),
            hand_id: hand_id.clone(),
            match_id: None,
            hand_no,
            deal_no,
            rotation: Some(rotation),
            cfg: cx.cfg.clone(),
            button_seat: button,
            seats,
            sitting_out: vec![],
            deck: dk,
            salt: format!("{:016x}", seed64(&hand_id)),
            started_ms: now_ms(),
            lean: true,
        };
        let (mut runner, _opening) = HandRunner::new(setup).ok()?;
        let mut houses: Vec<Option<Box<dyn Player>>> = Vec::new();
        for idx in 0..runner.n() {
            let pid = &runner.info[idx].player_id;
            let p = (pid != BOT_ID)
                .then(|| cx.house.key_of(pid))
                .flatten()
                .and_then(|k| cx.house.get(&k))
                .map(|f| f.new_player(idx, &runner.ecfg, mix(mix(cx.seed64, hand_no as u64), idx as u64)));
            houses.push(p);
        }
        let mut crn = cards::Rng::new(mix(cx.seed64, deal as u64));
        let mut forfeited = false;
        let mut restarted = false;
        while !runner.is_over() {
            let idx = runner.to_act_idx().unwrap();
            let applied = if houses[idx].is_some() {
                let u = crn.f64();
                let hole = runner.deal.holes[idx];
                let action = {
                    let view = bench::policy::View { cfg: &runner.ecfg, state: &runner.st, board: runner.board(), history: &runner.ehist, seat: idx };
                    houses[idx].as_mut().unwrap().act(&view, hole, u)
                };
                let (action, timeout) = if crate::table::engine_legal(&runner.st, action) {
                    (action, false)
                } else {
                    runner.anomalies.push(format!("house bot at seat {} chose an illegal action {action:?}", runner.seat_of(idx)));
                    (runner.timeout_action(), true)
                };
                runner.apply(action, timeout, 0)
            } else if self.bot.is_some() && self.alive() {
                self.st.decisions += 1;
                let deadline_ms = now_ms() + cx.opts.decision_timeout_ms;
                let (decision_id, seq, data) = {
                    let t = runner.begin_turn(deadline_ms, 0).unwrap();
                    (t.decision_id.clone(), t.event.seq.unwrap_or(0), t.event.data.to_string())
                };
                self.table_seq += 1;
                let turn = envelope("your_turn", &self.table_id, Some(&hand_id), Some(seq), self.table_seq, &data);
                let before = self.st.restarts;
                let d = self.ask(&runner, &decision_id, &turn);
                restarted |= self.st.restarts != before;
                match d {
                    Decision::Act(a, think) => runner.apply(a, false, think),
                    Decision::Timeout(think) => {
                        if !self.alive() {
                            forfeited = true;
                        }
                        let a = runner.timeout_action();
                        runner.apply(a, true, think)
                    }
                }
            } else {
                forfeited = true;
                let a = runner.timeout_action();
                runner.apply(a, true, 0)
            };
            for p in houses.iter_mut().flatten() {
                p.observe(&applied.pre, applied.idx, applied.action);
                for &l in &applied.reveals {
                    p.observe_board(&runner.deal.board[..l]);
                }
            }
        }
        for p in houses.iter_mut().flatten() {
            p.finish(&runner.deal, &runner.ehist, &runner.st);
        }
        let (end_events, hist) = runner.finish(true);
        if forfeited {
            self.st.forfeit_hands += 1;
        }
        self.st.anomalies += hist.anomalies.len() as u32;
        if self.alive() && !restarted {
            if let Some(mut e) = end_events.into_iter().find(|e| e.kind == "hand_end") {
                crate::table::redact_hand_end(&mut e.data);
                self.table_seq += 1;
                let line = envelope("hand_end", &self.table_id, Some(&hand_id), e.seq, self.table_seq, &e.data.to_string());
                let deadline = Instant::now() + Duration::from_millis(cx.opts.decision_timeout_ms);
                if let Some(b) = self.bot.as_mut() {
                    match b.send(&line, deadline) {
                        Ok(()) => {}
                        Err(IoErr::Timeout) => {
                            self.st.timeouts += 1;
                            self.restart_after_timeout();
                        }
                        Err(_) => self.die("bot exited mid-game (closed its stdin)".into()),
                    }
                }
            }
        }
        let mut nets = [0i64; 6];
        for r in &hist.results {
            nets[r.seat as usize] = r.net;
        }
        let av = cx.mode.and_then(|m| {
            let t = Instant::now();
            let v = matches::aivat_hand(cx.house, &hist, cx.est, m);
            self.st.aivat_ms += t.elapsed().as_secs_f64() * 1000.0;
            v
        });
        self.st.hands += 1;
        cx.done.fetch_add(1, Ordering::Relaxed);
        Some((HandRec { hand_no, nets }, av, (hand_no, deal_no, rotation, deck_digest, nets)))
    }

    /// Play this lane's share of rotation `rot` with a fresh bot process (`bot_cmd`), then end it. A lane that gave up on
    /// its bot (too many restarts) stays given up: its remaining hands are forfeited.
    fn play_rotation(&mut self, rot: u32, bot_cmd: String, out: &mut LaneOut) {
        let t0 = Instant::now();
        self.bot_cmd = bot_cmd;
        self.table_seq = 0; // a new process is a new session
        if self.alive() {
            if let Err(e) = self.start_bot() {
                self.die(format!("start failed: {e}"));
            }
        }
        // this lane's deals of the rotation, in an order keyed by the seed (secret for hosted runs): the order of play says
        // nothing about which deal is which
        let mut hands = Vec::new();
        while let Some(h) = matches::lane_rotation_hand_no(self.cx.planned, BLOCK_DEALS, self.cx.lanes, self.lane, rot, hands.len() as u32) {
            hands.push(h);
        }
        let mut rng = cards::Rng::new(mix(mix(self.cx.seed64 ^ 0x5eed_04de_u64, rot as u64), self.lane as u64));
        for i in (1..hands.len()).rev() {
            hands.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
        }
        for h in hands {
            match self.play_hand(h) {
                Some((rec, av, log)) => {
                    if let Some(a) = av {
                        out.aivat.insert(h, a);
                    }
                    out.recs.push(rec);
                    out.log.push(log);
                }
                None => {
                    self.st.anomalies += 1;
                }
            }
        }
        if let Some(mut b) = self.bot.take() {
            let bye = envelope("bye", &self.table_id, None, None, self.table_seq + 1, "{}");
            b.finish(&bye);
        }
        self.st.wall_ms += t0.elapsed().as_millis() as u64;
    }
}

/// The bot command for rotation `rot`: as that rotation's own uid when isolated.
fn rotation_cmd(opts: &WorkerOpts, rot: u32) -> String {
    match &opts.rotation_uids {
        Some(u) => format!("exec setpriv --reuid={0} --regid={0} --clear-groups {1}", u[rot as usize], opts.bot_cmd),
        None => opts.bot_cmd.clone(),
    }
}

/// Take the sandbox's loopback interface down (the worker is root with CAP_NET_ADMIN in the sandbox's own network
/// namespace). Bots talk to the worker over pipes and need no network; a TCP connection a bot makes to itself would
/// leave TIME_WAIT state (ports it chose) that outlives its processes and that the next rotation could read back.
fn loopback_down() -> Result<(), String> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(format!("isolation: socket: {}", io::Error::last_os_error()));
        }
        let mut ifr: libc::ifreq = std::mem::zeroed();
        for (i, b) in b"lo".iter().enumerate() {
            ifr.ifr_name[i] = *b as libc::c_char;
        }
        let r = if libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr) < 0 {
            Err(io::Error::last_os_error())
        } else {
            ifr.ifr_ifru.ifru_flags &= !(libc::IFF_UP as libc::c_short);
            if libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &ifr) < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
        };
        libc::close(fd);
        r.map_err(|e| format!("isolation: cannot take the loopback interface down ({e}); the sandbox needs CAP_NET_ADMIN"))
    }
}

/// End everything `uid` left behind (see `WorkerOpts::rotation_uids`). A failure fails the run: a run that can't
/// guarantee the isolation must not be signed.
fn scrub(cmd: &str, uid: u32) -> Result<(), String> {
    let st = Command::new("sh").arg("-c").arg(format!("{cmd} {uid}")).stdin(Stdio::null()).status()
        .map_err(|e| format!("isolation: cannot run {cmd}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("isolation: `{cmd} {uid}` failed ({st})")) }
}

// ------------------------------------------------------------------------------------ run + result

/// What the bot sees of the seed inside `hand_id`s: the seed for the (public) track seed, a hash prefix otherwise.
fn hand_prefix(seed: &str, track_seed: &str) -> String {
    if seed == track_seed {
        seed.to_string()
    } else {
        format!("run-{}", &sha256_hex(format!("hand-prefix:{seed}").as_bytes())[..16])
    }
}

/// Read a seed from a file (`--seed-file`): trimmed, must be non-empty. The hosted runner writes a root-only file
/// the bot's user cannot open; the worker reads it once and never passes it on to the bot process.
pub fn read_seed_file(path: &str) -> Result<String, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("cannot read seed file {path}: {e}"))?;
    let seed = raw.trim();
    if seed.is_empty() {
        return Err(format!("seed file {path} is empty"));
    }
    Ok(seed.to_string())
}

pub fn build_house(track: &Track, cfg: Option<&ServerConfig>) -> Result<HouseRegistry, String> {
    let cfg = cfg.cloned().unwrap_or_else(ServerConfig::from_env);
    let house = HouseRegistry::build(&cfg)?;
    for k in track.panel {
        if house.key_of(k).is_none() {
            return Err(format!(
                "track {} needs house bot {k}: set ARENA_BLUEPRINT (and ARENA_BUCKETS / ARENA_SIZING) to the blueprint of the server's track",
                track.id
            ));
        }
    }
    Ok(house)
}

/// Play the benchmark and return the unsigned result JSON.
pub fn run_worker(opts: &WorkerOpts) -> Result<Value, String> {
    let track = benchmarks::track(&opts.track).map_err(|e| e.message)?;
    if opts.bot_cmd.trim().is_empty() {
        return Err("--bot is required".into());
    }
    let house = build_house(track, opts.house_cfg.as_ref())?;
    let hands = ((opts.hands.clamp(6, 1_000_000)) + 5) / 6 * 6;
    let blocks = ((hands / 6) + BLOCK_DEALS - 1) / BLOCK_DEALS;
    let lanes = opts.lanes.clamp(1, 32).min(blocks.max(1) as usize);
    let track_seed = benchmarks::seed_of(track);
    let seed = opts.seed.clone().unwrap_or_else(|| track_seed.clone());
    let mut participants = vec![BOT_ID.to_string()];
    participants.extend(track.panel.iter().map(|k| house_id(&house.key_of(k).unwrap())));
    let mode = match opts.aivat.as_str() {
        "off" => None,
        "cards" => Some(Mode::CardsOnly),
        _ => Some(Mode::Full),
    };
    let cfg = TableConfig { mode: Some("benchmark".into()), decision_time_ms: opts.decision_timeout_ms, min_players_to_deal: 6, ..Default::default() };
    cfg.validate().map_err(|e| e.message)?;
    let est = EquityEstimator::default();
    let cx = Cx { opts, house: &house, est: &est, mode, participants: participants.clone(), cfg: cfg.clone(), seed64: seed64(&seed), hand_prefix: hand_prefix(&seed, &track_seed), seed: seed.clone(), planned: hands, lanes, done: AtomicU32::new(0), seed_secret: seed != track_seed };
    let started_at = now_iso();
    let t0 = Instant::now();
    let finished = AtomicBool::new(false);
    let outs: Vec<LaneOut> = std::thread::scope(|sc| -> Result<Vec<LaneOut>, String> {
        if !opts.quiet {
            let (cx, fin) = (&cx, &finished);
            sc.spawn(move || {
                let mut last = 0;
                while !fin.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                    let d = cx.done.load(Ordering::Relaxed);
                    if d * 10 / cx.planned.max(1) > last * 10 / cx.planned.max(1) {
                        last = d;
                        eprintln!("  {} {}/{} hands ({:.0} hands/s)", cx.opts.track, d, cx.planned, d as f64 / t0.elapsed().as_secs_f64());
                    }
                }
            });
        }
        // Rotation-major: every lane plays rotation 0 of its deals, then (fresh processes) rotation 1, ... Within one rotation
        // a deal appears once, so a bot process never meets a deal it already saw from another seat; with
        // `rotation_uids` nothing it wrote down survives into the next rotation either.
        if opts.rotation_uids.is_some() {
            if let Err(e) = loopback_down() {
                finished.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
        let mut ls: Vec<Lane> = (0..lanes).map(|l| Lane::new(&cx, l)).collect();
        let mut outs: Vec<LaneOut> = (0..lanes).map(|_| LaneOut { recs: vec![], aivat: HashMap::new(), log: vec![], stats: LaneStats::default() }).collect();
        for rot in 0..6u32 {
            let cmd = rotation_cmd(opts, rot);
            let hs: Vec<_> = ls.drain(..).zip(outs.drain(..)).map(|(mut lane, mut out)| {
                let cmd = cmd.clone();
                sc.spawn(move || {
                    lane.play_rotation(rot, cmd, &mut out);
                    (lane, out)
                })
            }).collect();
            for h in hs {
                let (lane, out) = h.join().expect("lane thread panicked");
                ls.push(lane);
                outs.push(out);
            }
            if let Some(uids) = &opts.rotation_uids {
                if let Err(e) = scrub(&opts.scrub_cmd, uids[rot as usize]) {
                    finished.store(true, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
        for (lane, out) in ls.iter_mut().zip(outs.iter_mut()) {
            out.stats = std::mem::take(&mut lane.st);
        }
        finished.store(true, Ordering::Relaxed);
        Ok(outs)
    })?;
    let wall = t0.elapsed();
    let mut recs: Vec<HandRec> = Vec::new();
    let mut aivat: HashMap<u32, [f64; 6]> = HashMap::new();
    let mut log = Vec::new();
    let mut lane_stats = Vec::new();
    for o in outs {
        recs.extend(o.recs);
        aivat.extend(o.aivat);
        log.extend(o.log);
        lane_stats.push(o.stats);
    }
    recs.sort_by_key(|r| r.hand_no);
    log.sort_by_key(|l| l.0);
    let keys = matches::slot_keys_of(&participants);
    let names: HashMap<String, String> = participants.iter().map(|p| (p.clone(), p.clone())).collect();
    let (standings, _rows) = matches::build_standings(&keys, &participants, &names, cfg.big_blind, true, hands, BLOCK_DEALS, &recs, mode.map(|_| &aivat));
    let me = standings.iter().find(|s| s.player_id == BOT_ID).ok_or("no standing for the bot (no hands played?)")?;
    if let Some(path) = &opts.hands_out {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?);
        for (h, d, r, dg, nets) in &log {
            let _ = writeln!(f, "{}", json!({"hand_no": h, "deal_no": d, "rotation": r, "deck_digest": format!("{dg:016x}"), "nets": nets}));
        }
    }
    let mut dg = Vec::new();
    for (h, _, _, d, _) in &log {
        dg.extend_from_slice(&h.to_le_bytes());
        dg.extend_from_slice(&d.to_le_bytes());
    }
    let sum = |f: &dyn Fn(&LaneStats) -> f64| lane_stats.iter().map(f).sum::<f64>();
    let hands_done = recs.len() as u32;
    let finished_at = now_iso();
    Ok(json!({
        "schema": RESULT_SCHEMA,
        "run_id": new_id("w"),
        "track": track.id,
        "panel_version": track.version,
        "panel": track.panel.iter().map(|k| house_id(k)).collect::<Vec<_>>(),
        "seed": seed,
        "player_id": opts.player_id,
        "hands": hands_done,
        "hands_planned": hands,
        "deals": me.deals,
        "lanes": lanes,
        "mbb_per_game": r6(me.mbb_per_game),
        "ci95_mbb": r6(me.ci95_mbb),
        "method": me.method,
        "bb_per_100": r6(me.bb_per_100),
        "net_chips": me.net_chips,
        "duplicate_mbb_per_hand": me.duplicate_mbb_per_hand.map(r6),
        "forfeit_hands": sum(&|s| s.forfeit_hands as f64) as u64,
        "crashed_lanes": lane_stats.iter().filter(|s| s.crashed.is_some()).count(),
        "timeouts": sum(&|s| s.timeouts as f64) as u64,
        "blueprint_fingerprint": house.blueprint_fp,
        "deals_digest": sha256_hex(&dg),
        "bot": {"command_sha256": sha256_hex(opts.bot_cmd.as_bytes())},
        "worker": {"name": "sharingan-arena", "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL, "aivat": opts.aivat,
                   "decision_timeout_ms": opts.decision_timeout_ms, "max_restarts": opts.max_restarts,
                   "isolated_rotations": opts.rotation_uids.is_some()},
        "per_lane": lane_stats.iter().map(|s| json!({
            "lane": s.lane, "hands": s.hands, "decisions": s.decisions, "timeouts": s.timeouts, "restarts": s.restarts,
            "illegal_actions": s.illegal_actions, "forfeit_hands": s.forfeit_hands, "noise_lines": s.noise_lines,
            "anomalies": s.anomalies, "crashed": s.crashed, "wall_ms": s.wall_ms,
            "bot_wait_ms": s.bot_wait_ms.round(), "aivat_ms": s.aivat_ms.round(),
        })).collect::<Vec<_>>(),
        "timings": {
            "wall_ms": wall.as_millis() as u64,
            "hands_per_sec": (hands_done as f64 / wall.as_secs_f64() * 10.0).round() / 10.0,
            "bot_wait_ms_total": sum(&|s| s.bot_wait_ms).round(),
            "aivat_ms_total": sum(&|s| s.aivat_ms).round(),
        },
        "started_at": started_at,
        "finished_at": finished_at,
    }))
}

// ------------------------------------------------------------------------------------ CLI

pub const RUN_USAGE: &str = "usage: sharingan-arena run --bot \"<command>\" [--track ladder-v1|sharingan-1] [--hands N] [--lanes L] [--seed S | --seed-file F]
         [--out result.json] [--sign-key-file F] [--player-id ID] [--decision-timeout-ms 5000] [--startup-timeout-ms 30000]
         [--max-restarts 3] [--aivat full|cards|off] [--hands-out FILE] [--quiet]
         [--rotation-uids U0,U1,U2,U3,U4,U5 [--scrub-cmd sharingan-scrub]]   (as root: each rotation's bot as its own uid, wiped after)
  env: ARENA_BLUEPRINT / ARENA_BUCKETS / ARENA_SIZING select the sharingan-1 house bot (as for the server)";

/// `sharingan-arena run ...`; returns the process exit code.
pub fn cli_run(args: Vec<String>) -> i32 {
    let mut o = WorkerOpts::default();
    let (mut out, mut key_file, mut seed_file) = (None::<String>, None::<String>, None::<String>);
    let mut it = args.into_iter();
    let bad = |m: &str| -> i32 {
        eprintln!("sharingan-arena run: {m}\n{RUN_USAGE}");
        2
    };
    while let Some(a) = it.next() {
        let mut val = || it.next();
        macro_rules! v {
            () => {
                match val() {
                    Some(v) => v,
                    None => return bad(&format!("{a} needs a value")),
                }
            };
        }
        macro_rules! n {
            ($t:ty) => {
                match v!().parse::<$t>() {
                    Ok(x) => x,
                    Err(_) => return bad(&format!("{a}: bad number")),
                }
            };
        }
        match a.as_str() {
            "--bot" => o.bot_cmd = v!(),
            "--track" => o.track = v!(),
            "--hands" => o.hands = n!(u32),
            "--lanes" => o.lanes = n!(usize),
            "--seed" => o.seed = Some(v!()),
            "--seed-file" => seed_file = Some(v!()),
            "--out" => out = Some(v!()),
            "--sign-key-file" => key_file = Some(v!()),
            "--player-id" => o.player_id = Some(v!()),
            "--decision-timeout-ms" => o.decision_timeout_ms = n!(u64),
            "--startup-timeout-ms" => o.startup_timeout_ms = n!(u64),
            "--max-restarts" => o.max_restarts = n!(u32),
            "--aivat" => o.aivat = v!(),
            "--hands-out" => o.hands_out = Some(v!()),
            "--rotation-uids" => {
                let u: Vec<u32> = v!().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                let mut d = u.clone();
                d.sort_unstable();
                d.dedup();
                if u.len() != 6 || d.len() != 6 || u.iter().any(|&x| x < 1000) {
                    return bad("--rotation-uids needs 6 distinct uids >= 1000, comma separated");
                }
                o.rotation_uids = Some([u[0], u[1], u[2], u[3], u[4], u[5]]);
            }
            "--scrub-cmd" => o.scrub_cmd = v!(),
            "--quiet" => o.quiet = true,
            "-h" | "--help" => {
                println!("{RUN_USAGE}");
                return 0;
            }
            _ => return bad(&format!("unknown argument {a}")),
        }
    }
    if o.bot_cmd.is_empty() {
        return bad("--bot is required");
    }
    if !matches!(o.aivat.as_str(), "full" | "cards" | "off") {
        return bad("--aivat must be full, cards or off");
    }
    if let Some(p) = &seed_file {
        // takes precedence over --seed
        match read_seed_file(p) {
            Ok(sd) => o.seed = Some(sd),
            Err(e) => {
                eprintln!("sharingan-arena run: {e}");
                return 2;
            }
        }
    }
    let key = match &key_file {
        Some(p) => match std::fs::read_to_string(p).ok().and_then(|s| parse_key_spec(&s)) {
            Some(k) => Some(k),
            None => {
                eprintln!("sharingan-arena run: cannot read a key from {p}");
                return 2;
            }
        },
        None => None,
    };
    let mut result = match run_worker(&o) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sharingan-arena run: {e}");
            return 1;
        }
    };
    if let Some((id, secret)) = &key {
        sign_result(&mut result, id, secret);
    }
    let text = serde_json::to_string_pretty(&result).unwrap();
    match &out {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &text) {
                eprintln!("sharingan-arena run: {p}: {e}");
                return 1;
            }
        }
        None => println!("{text}"),
    }
    let r = &result;
    eprintln!(
        "{}: {:+.1} ± {:.1} mbb/game ({}) over {} hands on {}; {:.0} hands/s{}{}",
        r["track"].as_str().unwrap_or(""),
        r["mbb_per_game"].as_f64().unwrap_or(0.0),
        r["ci95_mbb"].as_f64().unwrap_or(0.0),
        r["method"].as_str().unwrap_or(""),
        r["hands"],
        r["track"].as_str().unwrap_or(""),
        r["timings"]["hands_per_sec"].as_f64().unwrap_or(0.0),
        if r["forfeit_hands"].as_u64().unwrap_or(0) > 0 { format!("; {} FORFEIT hands", r["forfeit_hands"]) } else { String::new() },
        if key.is_some() { "; signed" } else { "" },
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_keys_and_is_compact() {
        let v = json!({"b": [1, {"z": 1.5, "a": null}], "a": "x\"y", "c": 10});
        assert_eq!(canonical_json(&v), r#"{"a":"x\"y","b":[1,{"a":null,"z":1.500000}],"c":10}"#);
        // key order of the input does not matter
        let w: Value = serde_json::from_str(r#"{"c":10,"a":"x\"y","b":[1,{"a":null,"z":1.5}]}"#).unwrap();
        assert_eq!(canonical_json(&v), canonical_json(&w));
    }

    #[test]
    fn sign_verify_and_forgery() {
        let keys = parse_keys("w1=secret-one, w2=secret-two");
        assert_eq!(keys.len(), 2);
        let mut r = json!({"schema": RESULT_SCHEMA, "mbb_per_game": 12.5, "ci95_mbb": 3.0, "hands": 600});
        assert_eq!(verify_result(&r, &keys), Err("unsigned result"));
        sign_result(&mut r, "w2", "secret-two");
        assert_eq!(verify_result(&r, &keys), Ok("w2".to_string()));
        // survives a serialize/parse round trip (what the control plane receives)
        let back: Value = serde_json::from_str(&serde_json::to_string_pretty(&r).unwrap()).unwrap();
        assert_eq!(verify_result(&back, &keys), Ok("w2".to_string()));
        // any change to the signed content breaks it
        let mut forged = r.clone();
        forged["mbb_per_game"] = json!(1200.5);
        assert_eq!(verify_result(&forged, &keys), Err("bad signature"));
        let mut extra = r.clone();
        extra["verified"] = json!(true);
        assert_eq!(verify_result(&extra, &keys), Err("bad signature"));
        // wrong key / claimed key id mismatch / no keys
        let mut other = r.clone();
        sign_result(&mut other, "w2", "not-the-secret");
        assert_eq!(verify_result(&other, &keys), Err("bad signature"));
        let mut wrong_id = r.clone();
        wrong_id["signature"]["key_id"] = json!("w1");
        assert_eq!(verify_result(&wrong_id, &keys), Err("bad signature"));
        assert!(verify_result(&r, &[]).is_err());
        assert_eq!(parse_key_spec("abc").unwrap(), ("default".to_string(), "abc".to_string()));
    }
}
