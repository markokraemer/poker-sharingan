//! `openpoker` CLI.
//!
//! ```text
//! openpoker live    --policy builtin:tag|compact:PATH|search:PATH [--sizing S] [--buckets DIR] [--hands N]
//!                   [--search-threads 16] [--search-budget-ms 5000] [--search-buckets DIR] [--set k=v,...]
//!                   [--buy-in 2000] [--timeout-sec S] [--trace FILE] [--seed N] [--hu fallback|policy]
//!                   [--env FILE] [--no-raw] [--no-auto-rebuy] [--max-stack-bb 150]
//! openpoker offline --policy ... [--hands N] [--seed S] [--trace FILE]   (mock table, no network)
//! openpoker analyze --trace FILE | --history FILE [--json] [--mc N]
//! openpoker export  --out FILE [--limit N] [--offset N] [--env FILE]
//! openpoker season  [--env FILE]
//! ```

use bench::bp::{bucketer_by_name, sizing_by_name, BlueprintPolicy};
use bench::builtin::builtin;
use bench::compact::CompactPolicy;
use bench::policy::PolicyFactory;
use openpoker::analyze::{analyze, load_history, load_trace};
use openpoker::live::{self, LiveOpts};
use openpoker::searchpol::{self, SearchOpts};
use openpoker::session::{HuMode, PolicySpec, Session, SessionConfig};
use openpoker::trace::FileSink;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct Args(HashMap<String, String>);
impl Args {
    fn get(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| d.to_string())
    }
    fn opt(&self, k: &str) -> Option<String> {
        self.0.get(k).cloned().filter(|v| !v.is_empty())
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.0.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn flag(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let cmd = argv.get(1).cloned().unwrap_or_default();
    let mut m = HashMap::new();
    let mut i = 2;
    while i < argv.len() {
        let k = argv[i].trim_start_matches("--").to_string();
        if i + 1 < argv.len() && !argv[i + 1].starts_with("--") {
            m.insert(k, argv[i + 1].clone());
            i += 2;
        } else {
            m.insert(k, String::new());
            i += 1;
        }
    }
    if let Err(e) = run(&cmd, &Args(m)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn parse_policy(spec: &str, a: &Args) -> Result<(Arc<dyn PolicyFactory>, bool), String> {
    let buckets = a.get("buckets", "coarse");
    let bucketer_name = if buckets == "coarse" || buckets.starts_with("tables:") { buckets } else { format!("tables:{buckets}") };
    let sizing = a.get("sizing", "pluribus");
    if let Some(name) = spec.strip_prefix("builtin:") {
        return Ok((builtin(name).ok_or(format!("unknown builtin {name}"))?, false));
    }
    if let Some(path) = spec.strip_prefix("compact:") {
        let b = bucketer_by_name(&bucketer_name).ok_or(format!("cannot load buckets {bucketer_name}"))?;
        let s = sizing_by_name(&sizing).ok_or(format!("unknown sizing {sizing}"))?;
        return Ok((Arc::new(CompactPolicy::open(Path::new(path), b, s)?), true));
    }
    if let Some(path) = spec.strip_prefix("search:") {
        let b = bucketer_by_name(&bucketer_name).ok_or(format!("cannot load buckets {bucketer_name}"))?;
        let s = sizing_by_name(&sizing).ok_or(format!("unknown sizing {sizing}"))?;
        let o = SearchOpts {
            threads: a.num("search-threads", 16),
            budget_ms: a.num("search-budget-ms", 5000),
            sets: a.get("set", ""),
            search_buckets: a.opt("search-buckets"),
        };
        return Ok((Arc::new(searchpol::build(Path::new(path), b, s, &o)?), true));
    }
    if let Some(path) = spec.strip_prefix("bp:") {
        let p = BlueprintPolicy::from_file(Path::new(path), &bucketer_name, &sizing, a.num("capacity", 1usize << 26))?;
        return Ok((Arc::new(p), true));
    }
    Err(format!("policy must be builtin:NAME | compact:PATH | bp:PATH, got {spec}"))
}

fn run(cmd: &str, a: &Args) -> Result<(), String> {
    match cmd {
        "live" => cmd_live(a),
        "offline" => cmd_offline(a),
        "analyze" => cmd_analyze(a),
        "export" => {
            let key = live::load_api_key(a.opt("env").as_deref().map(Path::new))?;
            let v = live::fetch_history(&key, live::API_URL, a.num("limit", 100), a.num("offset", 0))?;
            let out = a.opt("out").ok_or("--out FILE required")?;
            std::fs::write(&out, serde_json::to_string(&v).unwrap()).map_err(|e| e.to_string())?;
            println!("wrote {out}");
            Ok(())
        }
        "season" => {
            let key = live::load_api_key(a.opt("env").as_deref().map(Path::new))?;
            println!("{}", serde_json::to_string_pretty(&live::fetch_season_me(&key, live::API_URL)?).unwrap());
            Ok(())
        }
        _ => {
            println!("usage: openpoker <live|analyze|export|season> ... (see crates/openpoker/README.md)");
            Ok(())
        }
    }
}

fn cmd_live(a: &Args) -> Result<(), String> {
    let policy = a.get("policy", "builtin:tag");
    let (main_f, embed6) = parse_policy(&policy, a)?;
    let fallback = builtin(&a.get("fallback", "tag")).ok_or("unknown --fallback builtin")?;
    let hu = match a.get("hu", "fallback").as_str() {
        "policy" => HuMode::Policy,
        "fallback" => HuMode::Fallback,
        o => return Err(format!("--hu must be fallback|policy, got {o}")),
    };
    let key = live::load_api_key(a.opt("env").as_deref().map(Path::new))?;
    let ts = openpoker::trace::now_ms();
    let trace_path: PathBuf = a.opt("trace").map(PathBuf::from).unwrap_or_else(|| format!("logs/openpoker-{ts}.jsonl").into());
    let sink = FileSink::create(&trace_path).map_err(|e| format!("trace {}: {e}", trace_path.display()))?;
    let cfg = SessionConfig {
        buy_in: a.num("buy-in", 2000),
        target_hands: a.num("hands", 10),
        seed: a.num("seed", 1),
        auto_rebuy: !a.flag("no-auto-rebuy"),
        raw: !a.flag("no-raw"),
        sizing: a.get("sizing", "pluribus"),
        buckets: a.get("buckets", "coarse"),
        max_stack_bb: a.num("max-stack-bb", 150.0),
    };
    let spec_main = main_f.clone();
    let spec = PolicySpec { label: policy.clone(), main: main_f, embed6, fallback, hu };
    let mut session = Session::new(cfg, spec, Box::new(sink));
    let opts = LiveOpts {
        ws_url: a.get("ws-url", live::WS_URL),
        api_url: a.get("api-url", live::API_URL),
        api_key: key,
        timeout: Duration::from_secs(a.num("timeout-sec", 3600)),
        max_reconnects: a.num("max-reconnects", 8),
    };
    eprintln!("openpoker live: policy={policy} hands={} buy_in={} trace={}", session.cfg.target_hands, session.cfg.buy_in, trace_path.display());
    let reason = live::run(&opts, &mut session)?;
    println!("ended: {reason}");
    if let Some(r) = spec_main.report() {
        println!("policy report: {r}");
    }
    println!("{}", serde_json::to_string_pretty(&session.counters).unwrap());
    if let Ok(text) = std::fs::read_to_string(&trace_path) {
        let (hands, sum) = load_trace(&text);
        println!("trace: {} lines, {} rejects, {} anomalies, {} hand records", sum.lines, sum.rejects, sum.anomalies, hands.len());
        println!("{}", analyze(&hands, 20_000).render());
    }
    Ok(())
}

fn cmd_analyze(a: &Args) -> Result<(), String> {
    let mc: usize = a.num("mc", 20_000);
    let (hands, note) = if let Some(p) = a.opt("trace") {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
        let (h, s) = load_trace(&text);
        (h, format!("trace {p}: {} lines ({} bad), {} decisions, {} acks, {} rejects, {} anomalies", s.lines, s.bad_lines, s.decisions, s.acks, s.rejects, s.anomalies))
    } else if let Some(p) = a.opt("history") {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{p}: {e}"))?;
        (load_history(&v), format!("history {p}"))
    } else {
        return Err("analyze needs --trace FILE or --history FILE".into());
    };
    let r = analyze(&hands, mc);
    if a.flag("json") {
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
    } else {
        println!("{note}\n{}", r.render());
    }
    Ok(())
}

/// Play a mock table (5 simple bots) through the real `Session` message path; prints counters and
/// the per-decision latency distribution. No network, no API key.
fn cmd_offline(a: &Args) -> Result<(), String> {
    let policy = a.get("policy", "builtin:tag");
    let (main_f, embed6) = parse_policy(&policy, a)?;
    let fallback = builtin(&a.get("fallback", "tag")).ok_or("unknown --fallback builtin")?;
    let sink = openpoker::trace::MemSink::default();
    let cfg = SessionConfig {
        buy_in: 2000,
        target_hands: a.num("hands", 100),
        seed: a.num("seed", 1),
        auto_rebuy: true,
        raw: false,
        sizing: a.get("sizing", "pluribus"),
        buckets: a.get("buckets", "coarse"),
        max_stack_bb: a.num("max-stack-bb", 0.0),
    };
    let spec_main = main_f.clone();
    let spec = PolicySpec { label: policy.clone(), main: main_f, embed6, fallback, hu: HuMode::Fallback };
    let mut session = Session::new(cfg, spec, Box::new(sink.clone()));
    let hands: usize = a.num("hands", 100);
    let t = std::time::Instant::now();
    let res = openpoker::offline::run(&mut session, hands, a.num("seed", 1));
    let el = t.elapsed().as_secs_f64();
    if let Some(p) = a.opt("trace") {
        let mut f = FileSink::create(Path::new(&p)).map_err(|e| e.to_string())?;
        for r in sink.records() {
            openpoker::trace::TraceSink::emit(&mut f, r);
        }
    }
    let dec = sink.of_kind("decision");
    let mut lat: Vec<u64> = dec.iter().filter_map(|d| d["latency_us"].as_u64()).collect();
    lat.sort_unstable();
    let fb = dec.iter().filter(|d| !d["fallback_reason"].is_null()).count();
    println!("offline: {} hands in {el:.1}s, {} decisions, policy fallbacks {fb}", res.hands, dec.len());
    if !lat.is_empty() {
        let q = |p: f64| lat[((lat.len() - 1) as f64 * p).round() as usize] as f64 / 1e3;
        println!(
            "decision latency ms (all decisions): mean {:.1} p50 {:.1} p95 {:.1} max {:.1}",
            lat.iter().sum::<u64>() as f64 / lat.len() as f64 / 1e3,
            q(0.5),
            q(0.95),
            *lat.last().unwrap() as f64 / 1e3
        );
    }
    if let Some(r) = spec_main.report() {
        println!("policy report (search decisions only): {r}");
    }
    println!("hero net {:+} op-chips over {} hands; session counters:", res.hero_net, res.hands);
    println!("{}", serde_json::to_string(&session.counters).unwrap());
    res.error.map_or(Ok(()), Err)
}
