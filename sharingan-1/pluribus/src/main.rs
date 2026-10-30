//! `pluribus` CLI.
//!
//! train: pluribus train --out data/run-local --iters 2000000000 [--sizing local] [--players 6] [--stack 10000]
//!        [--buckets data/buckets-200] [--mem-gb 24] [--checkpoint-min 30] [--resume] [--seed 1]
//!
//! evaluation pipeline (see EVAL_PIPELINE.md):
//!   pluribus compact --run DIR --out FILE [--ckpt F] [--sums F] [--min-visits N] [--threads 8] [--group-gslots G]
//!   pluribus h2h --a FILE --sizing-a S --b FILE --sizing-b S [--players 6] [--deals N] [--threads N]
//!   pluribus exploit-compact --target FILE --sizing S --iters N [--mem-gb N] [--threads N] ...

use abstraction::tables::BucketTables;
use blueprint::{io, snapshot, trainer::Trainer};
use nlhe::GameConfig;
use pluribus::{scaled_schedule, sizing, TableBucketer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

fn args() -> (String, HashMap<String, String>) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut kv = HashMap::new();
    let rest: Vec<String> = it.collect();
    let mut i = 0;
    while i < rest.len() {
        let k = rest[i].trim_start_matches("--").to_string();
        if i + 1 < rest.len() && !rest[i + 1].starts_with("--") {
            kv.insert(k, rest[i + 1].clone());
            i += 2;
        } else {
            kv.insert(k, "true".into());
            i += 1;
        }
    }
    (cmd, kv)
}

fn rss_gb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    out.ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<f64>().ok()).map_or(0.0, |kb| kb / 1e6)
}

fn train(kv: &HashMap<String, String>) {
    let get = |k: &str, d: &str| kv.get(k).cloned().unwrap_or(d.to_string());
    let out = PathBuf::from(get("out", "data/run-local"));
    let iters: u64 = get("iters", "2000000000").parse().unwrap();
    let players: usize = get("players", "6").parse().unwrap();
    let mem_gb: f64 = get("mem-gb", "24").parse().unwrap();
    let ckpt_min: f64 = get("checkpoint-min", "30").parse().unwrap();
    let seed: u64 = get("seed", "1").parse().unwrap();
    let sizing_name = get("sizing", "local");
    // Starting stack in chips (blinds stay 50/100); default 10,000 = 100 bb. HU vs Slumbot: 20000.
    let stack: i32 = get("stack", "10000").parse().unwrap();
    let mut game = GameConfig::new(players);
    game.stack = stack;
    std::fs::create_dir_all(&out).unwrap();

    eprintln!("loading bucket tables");
    let bdir = get("buckets", "data/buckets-200");
    let tables = TableBucketer(BucketTables::load(Path::new(&bdir)).unwrap_or_else(|e| {
        eprintln!("cannot load bucket tables from {bdir}: {e}\nbuild them (~30 s on 128 cores): cargo run --release -p abstraction --bin build_buckets -- {bdir} 200 200 200 200000\nor download buckets-200.tar with the weights (MODEL_CARD.md)");
        std::process::exit(2);
    }));
    // Budget per infoset (measured on the paper run): ≈2.6 regret slots (10.4 B) and one
    // 16 B index entry used up to 85% load (18.8 B) → index table gets 64% of memory, the
    // regret arena 36%. (`with_index` sizes the table at max_infosets × 10/7.)
    let max_infosets = (mem_gb * 1e9 * 0.64 / 16.0 * 0.7) as usize;
    let capacity = (mem_gb * 1e9 * 0.36 / 4.0) as usize;
    let sched = scaled_schedule(iters);
    let state_path = out.join("progress.txt");
    let ckpt = out.join("blueprint.bin");
    let mut start_it = 0u64;
    let mut tr = Trainer::new(game, sizing(&sizing_name), sched.clone(), &tables, 0, seed);
    if kv.contains_key("resume") && ckpt.exists() {
        tr.store = std::sync::Arc::new(io::load(&ckpt, capacity, max_infosets).expect("load checkpoint"));
        start_it = std::fs::read_to_string(&state_path).unwrap().trim().parse().unwrap();
        eprintln!("resumed at iteration {start_it} ({} infosets)", tr.store.infosets());
        // Snapshots folded after this checkpoint belong to discarded training: cap the sums at
        // the checkpoint's arena (slots past it get other offsets now).
        let sums = out.join("snapshots.sum");
        if sums.exists() {
            let (k, capped) = snapshot::trim(&sums, tr.store.used_slots()).expect("trim snapshot sums");
            eprintln!("snapshot sums: {k} snapshots, {capped} capped to the checkpoint's {} slots", tr.store.used_slots());
        }
    } else {
        tr.store = std::sync::Arc::new(blueprint::store::Store::with_index(capacity, max_infosets));
    }
    std::fs::write(out.join("config.txt"), format!("players {players}\nstack {stack}\nsizing {sizing_name}\niters {iters}\nseed {seed}\nschedule {sched:?}\n")).unwrap();
    eprintln!("schedule {sched:?}");

    // Live learning curve in the background (shares the store read-only).
    let eval_min: f64 = get("eval-min", "60").parse().unwrap();
    let eval = std::sync::Arc::new(pluribus::live_eval::LiveEval {
        store: tr.store.clone(),
        bucketer: std::sync::Arc::new(bench::bp::TableBucketer(BucketTables::load(Path::new(&get("buckets", "data/buckets-200"))).expect("bucket tables"))),
        sizing: sizing(&sizing_name),
        game,
        out: out.join("eval.jsonl"),
        deals: get("eval-deals", "2000").parse().unwrap(),
        threads: get("eval-threads", "8").parse().unwrap(),
    });
    let eval_busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut last_eval = Instant::now() - std::time::Duration::from_secs_f64(eval_min * 60.0 * 0.9);

    let t0 = Instant::now();
    let mut last_log = Instant::now();
    let mut last_ckpt = Instant::now();
    let base_nodes = tr.nodes.load(Relaxed);
    tr.run(start_it, iters, |tr, t, secs| {
        if last_log.elapsed().as_secs_f64() > 30.0 || t == iters {
            last_log = Instant::now();
            let done = t - start_it;
            eprintln!(
                "[{:>7.1} min] it {t} ({:.1}%) {:.0} it/s {:.1}M nodes/s | infosets {} slots {:.2}G ({:.0}% cap) rss {:.1} GB",
                (t0.elapsed().as_secs_f64()) / 60.0,
                100.0 * t as f64 / iters as f64,
                done as f64 / secs,
                (tr.nodes.load(Relaxed) - base_nodes) as f64 / secs / 1e6,
                tr.store.infosets(),
                tr.store.used_slots() as f64 / 1e9,
                100.0 * (tr.store.used_slots() as f64 / tr.store.capacity() as f64).max(tr.store.infosets() as f64 / (tr.store.index_capacity() as f64 * 0.85)),
                rss_gb()
            );
            if tr.store.is_full() {
                eprintln!("  WARNING: store full — new infosets are no longer allocated");
            }
            if tr.store.collisions() > 0 {
                eprintln!("  key collisions refused: {}", tr.store.collisions());
            }
        }
        if eval_min > 0.0 && last_eval.elapsed().as_secs_f64() > eval_min * 60.0 && !eval_busy.swap(true, Relaxed) {
            last_eval = Instant::now();
            let (ev, busy) = (eval.clone(), eval_busy.clone());
            std::thread::spawn(move || {
                ev.run(t);
                busy.store(false, Relaxed);
            });
        }
        // Postflop snapshots every 200 paper-minutes after minute 800 (scaled to the run),
        // folded into one running-sum file.
        let snap_every = 200 * (iters / 11_520).max(1);
        // Each fold is committed together with a checkpoint, so a crash can never leave folds
        // of training that a resume would redo.
        let mut fold = None;
        if t >= sched.avg_after && t / snap_every != (t.saturating_sub(4096)) / snap_every {
            let c = Instant::now();
            let next = out.join("snapshots.sum.next");
            snapshot::accumulate_into(&tr.store, &out.join("snapshots.sum"), &next).expect("snapshot");
            eprintln!("  snapshot folded at it {t} ({:.0}s)", c.elapsed().as_secs_f64());
            fold = Some(next);
        }
        if fold.is_some() || last_ckpt.elapsed().as_secs_f64() > ckpt_min * 60.0 || t == iters {
            last_ckpt = Instant::now();
            let c = Instant::now();
            io::save(&tr.store, &ckpt).expect("checkpoint");
            if let Some(next) = fold {
                std::fs::rename(next, out.join("snapshots.sum")).expect("commit snapshot");
            }
            std::fs::write(&state_path, t.to_string()).unwrap();
            eprintln!("  checkpoint at it {t} ({:.0}s)", c.elapsed().as_secs_f64());
        }
    });
}

/// export: pluribus export --run data/run-x [--out data/run-x/final.bin]
/// Final blueprint = φ preflop + snapshot average postflop, stored as regrets = probs.
fn export(kv: &HashMap<String, String>) {
    let run = PathBuf::from(kv.get("run").expect("--run DIR"));
    let out = kv.get("out").map(PathBuf::from).unwrap_or(run.join("final.bin"));
    let store = io::load(&run.join("blueprint.bin"), 0, 0).expect("load checkpoint");
    let sums = run.join("snapshots.sum");
    eprintln!("exporting {} infosets (snapshot sums present: {})", store.infosets(), sums.exists());
    snapshot::export_in_place(&store, Some(&sums)).expect("export");
    io::save(&store, &out).expect("save");
    eprintln!("wrote {}", out.display());
}

/// compact: pluribus compact --run DIR --out FILE [--ckpt F] [--sums F] [--min-visits N]
///          [--threads 8] [--group-gslots 5] [--part-mb 512] [--tmp DIR] [--chunk RECORDS]
/// Streams a PLBP4 checkpoint (+ snapshots.sum) into the compact final-policy file; never
/// loads the store (RSS ~ a few GB; the OS page cache serves the mmapped checkpoint).
fn compact(kv: &HashMap<String, String>) {
    let run = kv.get("run").map(PathBuf::from);
    let ckpt = kv.get("ckpt").map(PathBuf::from).or_else(|| run.as_ref().map(|r| r.join("blueprint.bin"))).expect("--run DIR or --ckpt FILE");
    let sums = kv.get("sums").map(PathBuf::from).or_else(|| run.as_ref().map(|r| r.join("snapshots.sum"))).filter(|p| p.exists());
    let out = PathBuf::from(kv.get("out").expect("--out FILE"));
    let d = blueprint::compact::CompactOptions::default();
    let num = |k: &str, dflt: usize| kv.get(k).map_or(dflt, |v| v.parse().expect(k));
    let fl = |k: &str, dflt: f64| kv.get(k).map_or(dflt, |v| v.parse::<f64>().expect(k));
    let opt = blueprint::compact::CompactOptions {
        threads: num("threads", d.threads),
        min_visits: num("min-visits", 0) as u64,
        chunk: num("chunk", d.chunk),
        group_slots: (fl("group-gslots", d.group_slots as f64 / 1e9) * 1e9) as u64,
        part_bytes: (fl("part-mb", d.part_bytes as f64 / 1e6) * 1e6) as u64,
        tmp_dir: kv.get("tmp").map(PathBuf::from),
    };
    eprintln!("compact {} (+ {}) -> {} with {} threads", ckpt.display(), sums.as_ref().map_or("no snapshot sums".into(), |s| s.display().to_string()), out.display(), opt.threads);
    let st = blueprint::compact::compact_checkpoint(&ckpt, sums.as_deref(), &out, &opt, &|m| eprintln!("{m}")).expect("compact");
    println!(
        "{}",
        serde_json::json!({
            "out": out, "infosets_in": st.infosets_in, "infosets_out": st.infosets_out, "dropped": st.dropped, "slots_in": st.slots_in,
            "snapshot_covered": st.snapshot_covered, "phi_infosets": st.phi_infosets, "bytes_out": st.bytes_out,
            "bytes_per_infoset": st.bytes_out as f64 / st.infosets_out.max(1) as f64, "secs": st.secs,
        })
    );
}

fn load_bucketer(spec: &str) -> std::sync::Arc<dyn blueprint::trainer::Bucketer + Send> {
    let name = if spec == "coarse" { "coarse".to_string() } else { format!("tables:{spec}") };
    bench::bp::bucketer_by_name(&name).unwrap_or_else(|| panic!("cannot load buckets {spec}"))
}

/// h2h: pluribus h2h --a FILE --sizing-a S --b FILE --sizing-b S [--players 6] [--deals 2000]
///      [--threads 8] [--seed 1] [--buckets DIR|coarse] [--buckets-a D] [--buckets-b D] [--warm] [--out FILE]
fn h2h(kv: &HashMap<String, String>) {
    let get = |k: &str, d: &str| kv.get(k).cloned().unwrap_or(d.to_string());
    let buckets = get("buckets", "data/buckets-200");
    let (ba, bb) = (get("buckets-a", &buckets), get("buckets-b", &buckets));
    let bucket_a = load_bucketer(&ba);
    let bucket_b = if ba == bb { bucket_a.clone() } else { load_bucketer(&bb) };
    let open = |k: &str, sz: &str, b: std::sync::Arc<dyn blueprint::trainer::Bucketer + Send>| {
        let path = PathBuf::from(kv.get(k).unwrap_or_else(|| panic!("--{k} FILE")));
        let p = bench::compact::CompactPolicy::open(&path, b, sizing(&get(sz, "local"))).expect("open compact policy");
        if kv.contains_key("warm") {
            p.compact().warm();
        } else {
            p.compact().random_access();
        }
        eprintln!("{k}: {} infosets, {:.2} GB", p.compact().len(), p.compact().file_bytes() as f64 / 1e9);
        std::sync::Arc::new(p) as std::sync::Arc<dyn bench::policy::PolicyFactory>
    };
    let a = open("a", "sizing-a", bucket_a);
    let b = open("b", "sizing-b", bucket_b);
    let cfg = pluribus::h2h::H2hConfig { players: get("players", "6").parse().unwrap(), deals: get("deals", "2000").parse().unwrap(), seed: get("seed", "1").parse().unwrap(), threads: get("threads", "8").parse().unwrap() };
    let t = Instant::now();
    let mut rep = pluribus::h2h::h2h(a.clone(), b.clone(), &cfg);
    rep["secs"] = serde_json::json!(t.elapsed().as_secs_f64());
    pluribus::h2h::print_summary(&rep);
    println!("{}", serde_json::to_string_pretty(&rep).unwrap());
    if let Some(p) = kv.get("out") {
        std::fs::write(p, serde_json::to_string_pretty(&rep).unwrap()).unwrap();
    }
}

fn main() {
    let (cmd, kv) = args();
    match cmd.as_str() {
        "train" => train(&kv),
        "export" => export(&kv),
        "compact" => compact(&kv),
        "h2h" => h2h(&kv),
        "exploit-compact" => exploit::cli::run(&kv, true),
        _ => eprintln!("usage: pluribus train|export|compact|h2h|exploit-compact ...  (train: --out DIR --iters N [--sizing local|paper|tiny] [--mem-gb 24] [--resume])"),
    }
}
