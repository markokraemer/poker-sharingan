//! `bench` CLI: gauntlet / match / matrix / lbr / slumbot / mkbp.
//!
//! Policy specs: `random|station|nit|tag|lag|maniac`, `compact:<file.plcp>` (PLCP1, same
//! `--bucketer`/`--sizing`), `bp:<checkpoint>` (uses
//! `--bucketer`/`--sizing`, default coarse/tiny), `chart6`, `rlcard:<ckpt>`,
//! `ext:<name>=<json argv>`, with optional `+fold5|+call5|+raise5`.

use bench::bp::*;
use bench::builtin::{all_builtins, builtin};
use bench::lbr::{run_lbr, LbrConfig};
use bench::matchrun::*;
use bench::policy::PolicyFactory;
use bench::slumbot;
use nlhe::GameConfig;
use std::collections::HashMap;
use std::sync::Arc;

struct Args(HashMap<String, String>);
impl Args {
    fn get(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or(d.to_string())
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.0.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn flag(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }
}

fn parse_spec(spec: &str, a: &Args) -> Result<Arc<dyn PolicyFactory>, String> {
    let mut parts = spec.split('+');
    let base = parts.next().unwrap();
    let mut p: Arc<dyn PolicyFactory> = if base.starts_with("probe:") {
        bench::probe::parse(base)?
    } else if let Some(path) = base.strip_prefix("bp:") {
        Arc::new(
            BlueprintPolicy::from_file(
                std::path::Path::new(path),
                &a.get("bucketer", "coarse"),
                &a.get("sizing", "tiny"),
                a.num("capacity", 1usize << 26),
            )?
            .with_opts(track_opts(a)?),
        )
    } else if let Some(path) = base.strip_prefix("compact:") {
        let b = bucketer_by_name(&a.get("bucketer", "coarse")).ok_or("unknown bucketer")?;
        let sz = sizing_by_name(&a.get("sizing", "tiny")).ok_or("unknown sizing")?;
        Arc::new(bench::compact::CompactPolicy::open(std::path::Path::new(path), b, sz)?.with_opts(track_opts(a)?))
    } else if let Some(c) = bench::chart::Chart6::by_name(base) {
        Arc::new(c)
    } else if let Some(model) = base.strip_prefix("rlcard:") {
        // RLCard DQN via the Python bridge. --py / --ext-dir override the dev-box defaults.
        let home = std::env::var("HOME").unwrap_or_default();
        let py = a.get("py", &format!("{home}/extbots/rlcard-env/venv/bin/python"));
        let dir = a.get("ext-dir", "scripts/extbots");
        let cmd = vec![py, "-I".into(), format!("{dir}/rlcard_bot_server.py"), model.to_string()];
        bench::ext::ExtBot::new("rlcard", cmd, a.num("ext-workers", 8usize), std::time::Duration::from_millis(a.num("ext-timeout-ms", 5000u64))).factory()
    } else if let Some(json) = base.strip_prefix("ext:") {
        // ext:<name>=<JSON array of argv>, e.g. ext:mybot=["python3","-I","bot.py"]
        let (name, arr) = json.split_once('=').ok_or("ext:<name>=<json argv>")?;
        let cmd: Vec<String> = serde_json::from_str(arr).map_err(|e| format!("bad ext argv: {e}"))?;
        bench::ext::ExtBot::new(name, cmd, a.num("ext-workers", 8usize), std::time::Duration::from_millis(a.num("ext-timeout-ms", 5000u64))).factory()
    } else {
        builtin(base).ok_or(format!("unknown policy {base}"))?
    };
    for suf in parts {
        let bias = match suf {
            "fold5" => Bias::Fold,
            "call5" => Bias::Call,
            "raise5" => Bias::Raise,
            _ => return Err(format!("unknown suffix {suf}")),
        };
        p = Arc::new(Biased::new(p, bias));
    }
    Ok(p)
}

/// `--translation det|random`, `--below check|smallest`, `--legacy` (pre-fix: no shadow state).
fn track_opts(a: &Args) -> Result<nlhe::abstraction::TrackOpts, String> {
    use nlhe::abstraction::{BelowMenu, TrackOpts, Translation};
    if a.flag("legacy") {
        return Ok(TrackOpts::LEGACY);
    }
    let mut o = TrackOpts::default();
    match a.get("translation", "det").as_str() {
        "det" => o.translation = Translation::Deterministic,
        "random" => o.translation = Translation::Random,
        x => return Err(format!("unknown --translation {x}")),
    }
    match a.get("below", "default").as_str() {
        "default" => {}
        "check" => o.below = BelowMenu::Check,
        "smallest" => o.below = BelowMenu::Smallest,
        x => return Err(format!("unknown --below {x}")),
    }
    if let Some(v) = a.0.get("allin-snap") {
        o.allin_snap = v.parse().map_err(|_| "bad --allin-snap")?;
    }
    Ok(o)
}

fn aivat_mode(a: &Args) -> AivatMode {
    match a.get("aivat", "full").as_str() {
        "off" => AivatMode::Off,
        "cards" => AivatMode::CardsOnly,
        _ => AivatMode::Full,
    }
}

fn hero_match(
    cfg: GameConfig,
    hero: Arc<dyn PolicyFactory>,
    field: Arc<dyn PolicyFactory>,
    a: &Args,
) -> MatchReport {
    let mut spec = MatchSpec::new(
        cfg,
        vec![hero, field],
        Seatings::hero_vs_field(cfg.players),
        a.num("deals", 2000),
        a.num("seed", 1),
    );
    spec.aivat = aivat_mode(a);
    spec.crn = !a.flag("no-crn");
    let r = run_match(&spec);
    for p in spec.entrants.iter() {
        if let Some(rep) = p.report() {
            eprintln!("report {}: {rep}", p.name());
        }
    }
    r
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
    let a = Args(m);
    if let Err(e) = run(&cmd, &a) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cmd: &str, a: &Args) -> Result<(), String> {
    match cmd {
        "gauntlet" => {
            let hero = parse_spec(&a.get("hero", "tag"), a)?;
            for players in [6usize, 2] {
                println!(
                    "== {} vs each built-in, {players}-max (hero rotates through every seat) ==",
                    hero.name()
                );
                for f in all_builtins() {
                    let r = hero_match(GameConfig::new(players), hero.clone(), f.clone(), a);
                    let e = &r.entrants[0];
                    print!("  vs {:<8} raw {}", f.name(), e.raw);
                    if let Some(av) = &e.aivat {
                        print!("  aivat {av}  var-ratio {:.1}x", e.var_ratio_hand.unwrap());
                    }
                    println!();
                }
            }
        }
        "match" => {
            let (x, y) = (
                parse_spec(&a.get("a", "tag"), a)?,
                parse_spec(&a.get("b", "random"), a)?,
            );
            let players: usize = a.num("players", 2);
            hero_match(GameConfig::new(players), x, y, a).print();
        }
        "matrix" => {
            let bs = all_builtins();
            for players in [6usize, 2] {
                println!("\n### {players}-max: row hero vs {} copies of column (mbb/game, AIVAT, 95% CI)\n", players - 1);
                print!("| hero \\ field |");
                bs.iter().for_each(|b| print!(" {} |", b.name()));
                println!("\n|---|{}", "---|".repeat(bs.len()));
                let mut ratios = Vec::new();
                for h in &bs {
                    print!("| {} |", h.name());
                    for f in &bs {
                        if h.name() == f.name() {
                            print!(" - |");
                            continue;
                        }
                        let r = hero_match(GameConfig::new(players), h.clone(), f.clone(), a);
                        let e = &r.entrants[0];
                        let est = e.aivat.unwrap_or(e.raw);
                        print!(" {} |", est);
                        ratios.push(e.var_ratio_hand.unwrap_or(1.0));
                    }
                    println!();
                }
                let mean = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
                println!(
                    "\nmean per-hand variance ratio raw/AIVAT: {mean:.1}x (min {:.1}, max {:.1})",
                    ratios.iter().cloned().fold(f64::MAX, f64::min),
                    ratios.iter().cloned().fold(0.0, f64::max)
                );
            }
        }
        "table" => {
            // Mixed table: --hero SPEC --field "specA,specB,..." (up to 5 distinct opponents,
            // seated cyclically: entrant 0 = hero, then the field in order; every rotation is
            // played on every deal). With a single field spec it is hero vs 5 copies.
            let hero = parse_spec(&a.get("hero", "tag"), a)?;
            let specs: Vec<String> = a.get("field", "tag").split(',').map(str::to_string).collect();
            let players: usize = a.num("players", 6);
            let mut entrants = vec![hero];
            for sp in &specs {
                entrants.push(parse_spec(sp, a)?);
            }
            let base: Vec<usize> = if specs.len() == 1 {
                std::iter::once(0).chain(std::iter::repeat(1).take(players - 1)).collect()
            } else {
                assert_eq!(specs.len(), players - 1, "--field needs {} specs or 1", players - 1);
                (0..players).collect()
            };
            let cfg = GameConfig::new(players);
            let mut spec = MatchSpec::new(cfg, entrants, Seatings::rotations(&base), a.num("deals", 2000), a.num("seed", 1));
            spec.aivat = aivat_mode(a);
            spec.crn = !a.flag("no-crn");
            let r = run_match(&spec);
            for p in spec.entrants.iter() {
                if let Some(rep) = p.report() {
                    eprintln!("report {}: {rep}", p.name());
                }
            }
            r.print();
        }
        "lbr" => {
            let p = parse_spec(&a.get("policy", "tag"), a)?;
            let cfg = GameConfig::new(a.num("players", 2));
            let lc = LbrConfig {
                samples: a.num("samples", 300),
                ..Default::default()
            };
            println!(
                "{}",
                run_lbr(&cfg, &lc, p, a.num("deals", 500), a.num("seed", 1))
            );
        }
        "slumbot" => {
            let p = parse_spec(&a.get("policy", "tag"), a)?;
            let client = slumbot::SlumbotClient::new(a.get("host", "https://slumbot.com"));
            let token = a.0.get("token").cloned().filter(|t| !t.is_empty());
            let opts = slumbot::LiveOpts { pace_ms: a.num("pace-ms", 300), log: a.0.get("log").map(std::path::PathBuf::from) };
            let r = slumbot::play_live(&client, &*p, a.num("hands", 100), token, a.num("seed", 1), &opts)?;
            println!(
                "{} hands: {} chips, raw {} mbb/hand ({:.1} bb/100), luck-adjusted {} mbb/hand ({:.1} bb/100), token {:?}",
                r.hands,
                r.total_chips,
                r.mbb,
                r.mbb.mbb / 10.0,
                r.mbb_adj,
                r.mbb_adj.mbb / 10.0,
                r.token
            );
        }
        "mkbp" => {
            let players = a.num("players", 2);
            let bp = train_toy(players, a.num("iters", 60_000), a.num("seed", 1));
            let out = a.get("out", "toy.bp");
            blueprint::io::save(bp.store(), std::path::Path::new(&out))
                .map_err(|e| e.to_string())?;
            println!(
                "saved {} infosets to {out} (bucketer coarse, sizing tiny)",
                bp.store().sequences()
            );
        }
        _ => {
            println!("usage: bench <gauntlet|match|matrix|table|lbr|slumbot|mkbp> [--hero tag] [--a X --b Y] [--players N] [--deals N] [--seed S] [--aivat full|cards|off] [--bucketer coarse] [--sizing tiny]");
        }
    }
    Ok(())
}
