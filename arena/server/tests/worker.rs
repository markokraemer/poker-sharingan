//! Match worker (`sharingan-arena run`, pipe protocol) and the control-plane intake `POST /v1/results`.
//!
//! The worker plays the real ladder-v1 panel, which contains SHARINGAN-1, so these tests need a tiny
//! blueprint (`SHARINGAN_TEST_BLUEPRINT`, default `~/dev-serve/tinybp/tiny.plcp`, sizing `tiny`) and skip
//! without one, like tests/conformance.rs. Bots are the binary's own `pipe-house` subcommand plus tiny
//! shell/python scripts for the failure modes.

use arena_server::config::ServerConfig;
use arena_server::worker::{run_worker, sign_result, verify_result, WorkerOpts, RESULT_SCHEMA};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const BIN: &str = env!("CARGO_BIN_EXE_sharingan-arena");

fn blueprint() -> Option<PathBuf> {
    let p = std::env::var("SHARINGAN_TEST_BLUEPRINT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("dev-serve/tinybp/tiny.plcp"));
    p.exists().then_some(p)
}

fn house_cfg(bp: PathBuf) -> ServerConfig {
    ServerConfig { blueprint: Some(bp), sizing: "tiny".into(), ..Default::default() }
}

fn opts(bp: &PathBuf, cmd: &str, hands: u32, lanes: usize) -> WorkerOpts {
    WorkerOpts { bot_cmd: cmd.into(), hands, lanes, quiet: true, house_cfg: Some(house_cfg(bp.clone())), ..Default::default() }
}

fn station() -> String {
    format!("{BIN} pipe-house calling_station")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("shw-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const HELLO: &str = r#"{"v":1,"type":"hello","data":{"protocol":1,"client":"test"}}"#;

// ---------------------------------------------------------------------------------------------- protocol

#[test]
fn protocol_conformance_hello_welcome_your_turn_hand_end_bye() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let d = tmp("proto");
    let (inlog, outlog) = (d.join("in.log"), d.join("out.log"));
    // tee records what the worker sends (stdin of the bot) and what the bot answers (its stdout)
    let cmd = format!("tee {} | {} | tee {}", inlog.display(), station(), outlog.display());
    let r = run_worker(&opts(&bp, &cmd, 24, 1)).unwrap();
    assert_eq!(r["hands"], 24);
    assert_eq!(r["forfeit_hands"], 0);
    let ins: Vec<Value> = std::fs::read_to_string(&inlog).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let outs: Vec<Value> = std::fs::read_to_string(&outlog).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    // handshake: bot speaks first (hello), worker answers welcome
    assert_eq!(outs[0]["type"], "hello");
    assert_eq!(outs[0]["data"]["protocol"], 1);
    assert_eq!(ins[0]["type"], "welcome");
    assert_eq!(ins[0]["v"], 1);
    assert_eq!(ins[0]["data"]["protocol"], 1);
    assert_eq!(ins[0]["data"]["mode"], "benchmark");
    assert_eq!(ins[0]["data"]["tables"], json!(["lane-0"]));
    // benchmark mode: only your_turn + hand_end between welcome and bye
    let kinds: std::collections::BTreeSet<&str> = ins.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["bye", "hand_end", "welcome", "your_turn"].into_iter().collect());
    assert_eq!(ins.last().unwrap()["type"], "bye");
    // table_seq: gap-free, +1 per message delivered
    for (i, m) in ins.iter().enumerate() {
        assert_eq!(m["table_seq"], (i + 1) as u64, "table_seq at message {i}");
    }
    // your_turn carries the section 5.3 payload
    let turns: Vec<&Value> = ins.iter().filter(|m| m["type"] == "your_turn").collect();
    assert!(turns.len() >= 24);
    for t in &turns {
        assert_eq!(t["table_id"], "lane-0");
        assert!(t["hand_id"].is_string() && t["seq"].is_u64() && t["t_ms"].is_u64());
        let d = &t["data"];
        for k in ["hand_id", "seq", "decision_id", "seat", "street", "hole_cards", "board", "history", "players", "button_seat", "small_blind", "big_blind", "pot", "pots", "legal", "deadline_ms", "time_bank_ms"] {
            assert!(!d[k].is_null(), "your_turn.data.{k} missing");
        }
        assert_eq!(d["hole_cards"].as_array().unwrap().len(), 2);
        assert_eq!(d["players"].as_array().unwrap().len(), 6);
        assert!(d["history"].as_array().unwrap().len() >= 2); // at least the blinds
    }
    // every decision is answered by exactly one act echoing the decision_id
    let acts: Vec<&Value> = outs.iter().filter(|m| m["type"] == "act").collect();
    assert_eq!(acts.len(), turns.len());
    for (t, a) in turns.iter().zip(&acts) {
        assert_eq!(a["data"]["decision_id"], t["data"]["decision_id"]);
        assert_eq!(a["data"]["hand_id"], t["data"]["hand_id"]);
    }
    // hand_end: results sum to zero; other players' cards are redacted unless shown (reveal match_end)
    let ends: Vec<&Value> = ins.iter().filter(|m| m["type"] == "hand_end").collect();
    assert_eq!(ends.len(), 24);
    for e in &ends {
        let d = &e["data"];
        let net: i64 = d["results"].as_array().unwrap().iter().map(|r| r["net"].as_i64().unwrap()).sum();
        assert_eq!(net, 0);
        assert!(d["deck_reveal"].is_null());
        for seat in d["hole_cards"].as_object().unwrap().keys() {
            assert!(d["showdown"].as_array().unwrap().iter().any(|s| s["seat"].to_string() == *seat && s["shown"] == true));
        }
    }
}

#[test]
fn noise_on_stdout_is_ignored_and_illegal_actions_are_retried_with_the_same_decision() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        return eprintln!("skipped: no python3");
    }
    let py = r#"
import sys, json
print("this is not json, a stray print()", flush=True)
print(json.dumps({"v":1,"type":"hello","data":{"protocol":1}}), flush=True)
tried = set()
errors = 0
for line in sys.stdin:
    m = json.loads(line)
    t = m["type"]
    if t == "error":
        errors += 1
    elif t == "your_turn":
        d = m["data"]
        did = d["decision_id"]
        if did not in tried:
            tried.add(did)
            print(json.dumps({"v":1,"type":"act","data":{"hand_id":d["hand_id"],"decision_id":did,"type":"raise","to":1}}), flush=True)
            continue
        l = d["legal"]
        a = "check" if l["check"] else "call"
        print(json.dumps({"v":1,"type":"act","data":{"hand_id":d["hand_id"],"decision_id":did,"type":a}}), flush=True)
    elif t == "bye":
        break
"#;
    let d = tmp("illegal");
    let script = d.join("bot.py");
    std::fs::write(&script, py).unwrap();
    let r = run_worker(&opts(&bp, &format!("python3 {}", script.display()), 24, 1)).unwrap();
    let lane = &r["per_lane"][0];
    assert_eq!(r["hands"], 24);
    assert_eq!(r["forfeit_hands"], 0, "{r}");
    assert_eq!(lane["timeouts"], 0, "{r}");
    assert_eq!(lane["noise_lines"], 1);
    assert_eq!(lane["illegal_actions"], lane["decisions"], "every decision was first answered illegally once");
    // same score as the plain calling station: illegal answers change nothing
    let base = run_worker(&opts(&bp, &station(), 24, 1)).unwrap();
    assert_eq!(r["mbb_per_game"], base["mbb_per_game"]);
}

// ---------------------------------------------------------------------------------------------- failures

#[test]
fn hung_bot_is_killed_folded_restarted_then_forfeits() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    // says hello, then never answers anything
    let cmd = format!("echo '{HELLO}'; exec sleep 600");
    let mut o = opts(&bp, &cmd, 24, 1);
    o.decision_timeout_ms = 150;
    o.max_restarts = 1;
    let t0 = std::time::Instant::now();
    let r = run_worker(&o).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(20), "hang must not stall the worker");
    let lane = &r["per_lane"][0];
    assert_eq!(r["hands"], 24, "all planned hands are still played and scored");
    assert_eq!(lane["restarts"], 1);
    assert_eq!(lane["timeouts"], 2); // first hang, then the restarted process hangs too
    assert!(lane["crashed"].as_str().unwrap().contains("restarts"), "{lane}");
    assert_eq!(r["crashed_lanes"], 1);
    let ff = r["forfeit_hands"].as_u64().unwrap();
    assert!(ff >= 22 && ff <= 24, "forfeit_hands {ff}");
    assert!(r["mbb_per_game"].as_f64().unwrap().is_finite());
}

#[test]
fn crash_at_start_forfeits_every_hand_without_restart() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let cmd = format!("echo '{HELLO}'; read x; exit 3");
    let r = run_worker(&opts(&bp, &cmd, 144, 2)).unwrap(); // 24 deals = 2 blocks = 2 lanes
    assert_eq!(r["hands"], 144);
    assert_eq!(r["forfeit_hands"], 144);
    assert_eq!(r["crashed_lanes"], 2);
    assert_eq!(r["timeouts"], 0);
    for l in r["per_lane"].as_array().unwrap() {
        assert_eq!(l["restarts"], 0);
        assert!(l["crashed"].as_str().unwrap().contains("exited"), "{l}");
    }
    // a bot that cannot even start
    let r = run_worker(&opts(&bp, "exit 1", 12, 1)).unwrap();
    assert_eq!(r["forfeit_hands"], 12);
    assert!(r["per_lane"][0]["crashed"].as_str().unwrap().contains("start failed"));
}

#[test]
fn crash_midway_forfeits_only_the_rest_of_that_lane() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    // the bot dies after 41 lines (hello + 40 acts, `sed -u 41q` forwards unbuffered)
    let cmd = format!("{} | sed -u 41q", station());
    let r = run_worker(&opts(&bp, &cmd, 120, 1)).unwrap();
    assert_eq!(r["hands"], 120);
    let ff = r["forfeit_hands"].as_u64().unwrap();
    assert!(ff > 40 && ff < 120, "forfeit_hands {ff}");
    assert_eq!(r["crashed_lanes"], 1);
    // lane-scoped: with 2 lanes (24 deals = 2 blocks of 12) one crash leaves the other lane intact
    let cmd = format!("{} | sed -u 41q", station());
    let r = run_worker(&opts(&bp, &cmd, 144, 2)).unwrap();
    assert_eq!(r["crashed_lanes"], 2); // each lane has its own process that dies the same way
    assert!(r["forfeit_hands"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------------------------------------- determinism

#[test]
fn same_seed_same_deals_and_results_whatever_the_lane_count() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let d = tmp("det");
    let (f1, f2) = (d.join("h1.jsonl"), d.join("h4.jsonl"));
    let mut a = opts(&bp, &station(), 360, 1);
    a.hands_out = Some(f1.display().to_string());
    let mut b = opts(&bp, &station(), 360, 4);
    b.hands_out = Some(f2.display().to_string());
    let (ra, rb) = (run_worker(&a).unwrap(), run_worker(&b).unwrap());
    assert_eq!(ra["deals_digest"], rb["deals_digest"]);
    assert_eq!(ra["mbb_per_game"], rb["mbb_per_game"]);
    assert_eq!(ra["ci95_mbb"], rb["ci95_mbb"]);
    assert_eq!(std::fs::read_to_string(&f1).unwrap(), std::fs::read_to_string(&f2).unwrap(), "per-hand nets and decks identical");
    assert_eq!(ra["seed"], "bench:ladder-v1:v1");
    // another seed deals other cards
    let mut c = opts(&bp, &station(), 360, 4);
    c.seed = Some("another-seed".into());
    let rc = run_worker(&c).unwrap();
    assert_ne!(ra["deals_digest"], rc["deals_digest"]);
    // each deal is played in all 6 rotations: the same deck 6 times
    let lines: Vec<Value> = std::fs::read_to_string(&f1).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let mut by_deal: std::collections::HashMap<u64, Vec<&Value>> = Default::default();
    for l in &lines {
        by_deal.entry(l["deal_no"].as_u64().unwrap()).or_default().push(l);
    }
    assert_eq!(by_deal.len(), 60);
    for (_, v) in by_deal {
        assert_eq!(v.len(), 6);
        assert!(v.iter().all(|x| x["deck_digest"] == v[0]["deck_digest"]));
        let mut rots: Vec<u64> = v.iter().map(|x| x["rotation"].as_u64().unwrap()).collect();
        rots.sort();
        assert_eq!(rots, vec![0, 1, 2, 3, 4, 5]);
    }
}

// ---------------------------------------------------------------------------------------------- CLI + signing

#[test]
fn cli_run_writes_a_signed_result_json() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let d = tmp("cli");
    let (key, out) = (d.join("key"), d.join("result.json"));
    std::fs::write(&key, "w1=cli-secret\n").unwrap();
    let st = std::process::Command::new(BIN)
        .args(["run", "--bot", &station(), "--track", "sharingan-1", "--hands", "120", "--lanes", "2", "--quiet", "--player-id", "p_abc"])
        .arg("--out").arg(&out).arg("--sign-key-file").arg(&key)
        .env("ARENA_BLUEPRINT", bp.display().to_string())
        .env("ARENA_SIZING", "tiny")
        .status()
        .unwrap();
    assert!(st.success());
    let r: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(r["schema"], RESULT_SCHEMA);
    assert_eq!(r["track"], "sharingan-1");
    assert_eq!(r["panel_version"], "1");
    assert_eq!(r["hands"], 120);
    assert_eq!(r["lanes"], 2);
    assert_eq!(r["player_id"], "p_abc");
    assert_eq!(r["seed"], "bench:sharingan-1:v1");
    assert_eq!(r["panel"], json!(vec!["house:sharingan-1"; 5]));
    assert_eq!(r["bot"]["command_sha256"].as_str().unwrap().len(), 64);
    assert!(r["mbb_per_game"].is_f64() && r["ci95_mbb"].is_f64() && r["method"] == "aivat");
    assert_eq!(r["per_lane"].as_array().unwrap().len(), 2);
    assert!(r["timings"]["hands_per_sec"].as_f64().unwrap() > 0.0);
    assert!(r["blueprint_fingerprint"].is_string());
    assert_eq!(r["signature"]["alg"], "HMAC-SHA256");
    let keys = vec![("w1".to_string(), "cli-secret".to_string())];
    assert_eq!(verify_result(&r, &keys), Ok("w1".to_string()));
    let mut forged = r.clone();
    forged["mbb_per_game"] = json!(99999.0);
    assert!(verify_result(&forged, &keys).is_err());
    assert!(verify_result(&r, &[("w1".to_string(), "other".to_string())]).is_err());
    // unsigned when no key is given
    let out2 = d.join("r2.json");
    assert!(std::process::Command::new(BIN)
        .args(["run", "--bot", &station(), "--hands", "12", "--quiet"])
        .arg("--out").arg(&out2)
        .env("ARENA_BLUEPRINT", bp.display().to_string())
        .env("ARENA_SIZING", "tiny")
        .status().unwrap().success());
    let r2: Value = serde_json::from_str(&std::fs::read_to_string(&out2).unwrap()).unwrap();
    assert!(r2.get("signature").is_none());
}

// ---------------------------------------------------------------------------------------------- POST /v1/results

struct Srv {
    base: String,
    ws: String,
    http: reqwest::Client,
    _rs: arena_server::RunningServer,
}

async fn server(bp: Option<PathBuf>, keys: &str) -> Srv {
    let cfg = ServerConfig {
        bind: "127.0.0.1:0".into(),
        db_path: ":memory:".into(),
        admin_token: Some("admin".into()),
        runner_token: Some("runner".into()),
        worker_keys: arena_server::worker::parse_keys(keys),
        blueprint: bp,
        sizing: "tiny".into(),
        rate_limit_rps: 0.0,
        hand_delay_ms: 0,
        min_leaderboard_hands: 6,
        ..Default::default()
    };
    let rs = arena_server::start(cfg).await.unwrap();
    Srv { base: format!("http://{}", rs.addr), ws: format!("ws://{}", rs.addr), http: reqwest::Client::new(), _rs: rs }
}

impl Srv {
    async fn call(&self, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let mut r = self.http.request(method.parse().unwrap(), format!("{}{}", self.base, path));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        let txt = resp.text().await.unwrap();
        (st, serde_json::from_str(&txt).unwrap_or(Value::String(txt)))
    }
}

fn err_code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_results_accepts_signed_rejects_forged_and_the_leaderboard_picks_it_up() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let s = server(Some(bp.clone()), "w1=top-secret,w2=second").await;
    let (st, reg) = s.call("POST", "/v1/bots", None, Some(json!({"name": "evalbot", "kind": "bot"}))).await;
    assert_eq!(st, 201, "{reg}");
    let (pid, ptoken) = (reg["player_id"].as_str().unwrap().to_string(), reg["token"].as_str().unwrap().to_string());

    let o = opts(&bp, &station(), 120, 2);
    let mut result = tokio::task::spawn_blocking(move || run_worker(&o).unwrap()).await.unwrap();
    result["player_id"] = json!(pid);
    let mut signed = result.clone();
    sign_result(&mut signed, "w1", "top-secret");

    // 401: no token / bad token; 403: a player token (even with a valid signature)
    assert_eq!(s.call("POST", "/v1/results", None, Some(signed.clone())).await.0, 401);
    assert_eq!(s.call("POST", "/v1/results", Some("nope"), Some(signed.clone())).await.0, 401);
    assert_eq!(s.call("POST", "/v1/results", Some(&ptoken), Some(signed.clone())).await.0, 403);
    // 401: unsigned, forged number, wrong key, unknown key id
    let (st, e) = s.call("POST", "/v1/results", Some("runner"), Some(result.clone())).await;
    assert_eq!((st, err_code(&e)), (401, "invalid_signature"), "{e}");
    let mut forged = signed.clone();
    forged["mbb_per_game"] = json!(5000.0);
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(forged)).await.0, 401);
    let mut badkey = result.clone();
    sign_result(&mut badkey, "w1", "guess");
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(badkey)).await.0, 401);
    let mut unknown = result.clone();
    sign_result(&mut unknown, "w9", "top-secret");
    assert_eq!(s.call("POST", "/v1/results", Some("admin"), Some(unknown)).await.0, 401);
    // nothing was stored by any of that
    let (_, lb) = s.call("GET", "/v1/leaderboard?track=ladder-v1", None, None).await;
    assert_eq!(lb["rows"].as_array().unwrap().len(), 0);

    // valid, from the runner token
    let (st, run) = s.call("POST", "/v1/results", Some("runner"), Some(signed.clone())).await;
    assert_eq!(st, 201, "{run}");
    assert_eq!(run["verified"], true);
    assert_eq!(run["hosted"], true);
    assert_eq!(run["status"], "finished");
    assert_eq!(run["track"], "ladder-v1");
    assert_eq!(run["player_id"], pid);
    assert_eq!(run["mbb_per_game"], signed["mbb_per_game"]);
    assert_eq!(run["key_id"], "w1");
    let rid = run["run_id"].as_str().unwrap();
    // replay is refused
    let (st, e) = s.call("POST", "/v1/results", Some("runner"), Some(signed.clone())).await;
    assert_eq!((st, err_code(&e)), (409, "duplicate_result"), "{e}");
    // readable like any run
    let (st, got) = s.call("GET", &format!("/v1/benchmarks/ladder-v1/runs/{rid}"), None, None).await;
    assert_eq!(st, 200);
    assert_eq!(got["verified"], true);
    assert_eq!(got["hosted"], true);
    // leaderboard
    let (_, lb) = s.call("GET", "/v1/leaderboard?track=ladder-v1", None, None).await;
    let row = &lb["rows"][0];
    assert_eq!(row["player_id"], pid);
    assert_eq!(row["verified"], true);
    assert_eq!(row["tracks"]["ladder-v1"]["verified"], true);
    assert_eq!(row["vs_ladder_v1"]["mbb"], signed["mbb_per_game"]);

    // wrapper form with the player id outside the signed body; admin token; second key
    let mut r2 = run_worker_clone(&bp).await;
    sign_result(&mut r2, "w2", "second");
    let (st, run2) = s.call("POST", "/v1/results", Some("admin"), Some(json!({"result": r2, "player_id": pid}))).await;
    assert_eq!(st, 201, "{run2}");
    assert_eq!(run2["key_id"], "w2");

    // signed but not the frozen deals / panel / blueprint: refused with 400/409 even though the signature is valid
    let resign = |mut v: Value| {
        sign_result(&mut v, "w1", "top-secret");
        v
    };
    let fresh = || async { run_worker_clone(&bp).await };
    let mut a = fresh().await;
    a["player_id"] = json!(pid);
    a["seed"] = json!("my-own-seed");
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(resign(a))).await.0, 400);
    let mut a = fresh().await;
    a["player_id"] = json!(pid);
    a["panel"] = json!(vec!["house:random"; 5]);
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(resign(a))).await.0, 400);
    let mut a = fresh().await;
    a["player_id"] = json!(pid);
    a["blueprint_fingerprint"] = json!("deadbeefdeadbeef");
    let (st, e) = s.call("POST", "/v1/results", Some("runner"), Some(resign(a))).await;
    assert_eq!((st, err_code(&e)), (409, "blueprint_mismatch"), "{e}");
    let mut a = fresh().await;
    a["player_id"] = json!("p_does_not_exist");
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(resign(a))).await.0, 404);
    let a = fresh().await; // no player_id anywhere
    assert_eq!(s.call("POST", "/v1/results", Some("runner"), Some(resign(a))).await.0, 400);
}

async fn run_worker_clone(bp: &PathBuf) -> Value {
    let o = opts(bp, &station(), 120, 2);
    tokio::task::spawn_blocking(move || run_worker(&o).unwrap()).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_results_without_configured_keys_refuses_everything() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let s = server(Some(bp.clone()), "").await;
    let mut r = run_worker_clone(&bp).await;
    r["player_id"] = json!("house:random");
    sign_result(&mut r, "w1", "anything");
    let (st, e) = s.call("POST", "/v1/results", Some("admin"), Some(r)).await;
    assert_eq!((st, err_code(&e)), (401, "invalid_signature"), "{e}");
}

// ---------------------------------------------------------------------------------------------- comparability with the server

/// A remote WebSocket bot that checks/calls, as `house:calling_station` does.
async fn ws_station(url: String) {
    let (ws, _) = connect_async(url).await.unwrap();
    let (mut w, mut r) = ws.split();
    w.send(Message::Text(json!({"v": 1, "type": "hello", "t_ms": 0, "data": {"client": "test"}}).to_string().into())).await.unwrap();
    while let Some(Ok(Message::Text(t))) = r.next().await {
        let m: Value = serde_json::from_str(t.as_str()).unwrap();
        if m["type"] == "your_turn" {
            let d = &m["data"];
            let kind = if d["legal"]["check"] == true { "check" } else { "call" };
            let act = json!({"v": 1, "type": "act", "table_id": m["table_id"], "t_ms": 0,
                "data": {"hand_id": d["hand_id"], "decision_id": d["decision_id"], "type": kind}});
            if w.send(Message::Text(act.to_string().into())).await.is_err() {
                break;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_results_are_comparable_with_server_runs_on_the_same_frozen_deals() {
    let Some(bp) = blueprint() else { return eprintln!("skipped: no test blueprint") };
    let s = server(Some(bp.clone()), "w1=k").await;
    let (_, reg) = s.call("POST", "/v1/bots", None, Some(json!({"name": "ws-station", "kind": "bot"}))).await;
    let (pid, tok) = (reg["player_id"].as_str().unwrap().to_string(), reg["token"].as_str().unwrap().to_string());
    tokio::spawn(ws_station(format!("{}/v1/play?token={tok}", s.ws)));
    for _ in 0..100 {
        if s._rs.app.hub.connected(&pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (st, run) = s.call("POST", "/v1/benchmarks/ladder-v1/runs", Some(&tok), Some(json!({"hands": 360, "lanes": 2}))).await;
    assert_eq!(st, 201, "{run}");
    let (rid, mid) = (run["run_id"].as_str().unwrap().to_string(), run["match_id"].as_str().unwrap().to_string());
    let mut done = Value::Null;
    for _ in 0..600 {
        let (_, r) = s.call("GET", &format!("/v1/benchmarks/ladder-v1/runs/{rid}"), None, None).await;
        if r["status"] == "finished" {
            done = r;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(done["status"], "finished", "server run did not finish");
    let (_, m) = s.call("GET", &format!("/v1/matches/{mid}"), None, None).await;
    let me = m["standings"].as_array().unwrap().iter().find(|x| x["player_id"] == json!(pid)).unwrap().clone();
    let w = run_worker_clone_hands(&bp, 360).await;
    // identical deals, identical panel randomness, identical bot => identical duplicate score and chips
    assert_eq!(w["net_chips"], me["net_chips"], "worker {w} vs server {me}");
    let (a, b) = (w["duplicate_mbb_per_hand"].as_f64().unwrap(), me["duplicate_mbb_per_hand"].as_f64().unwrap());
    assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    // AIVAT estimates agree up to the Monte-Carlo noise of the preflop equity sampler (hand ids differ)
    let (x, y) = (w["mbb_per_game"].as_f64().unwrap(), done["mbb_per_game"].as_f64().unwrap());
    assert!((x - y).abs() < 0.25 * w["ci95_mbb"].as_f64().unwrap().max(50.0), "aivat {x} vs {y}");
}

async fn run_worker_clone_hands(bp: &PathBuf, hands: u32) -> Value {
    let o = opts(bp, &station(), hands, 2);
    tokio::task::spawn_blocking(move || run_worker(&o).unwrap()).await.unwrap()
}
