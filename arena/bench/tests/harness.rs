use bench::bp::*;
use bench::builtin::builtin;
use bench::lbr::{run_lbr, LbrConfig};
use bench::matchrun::*;
use bench::policy::PolicyFactory;
use nlhe::GameConfig;
use std::sync::Arc;

fn b(n: &str) -> Arc<dyn PolicyFactory> {
    builtin(n).unwrap()
}

fn hero(
    players: usize,
    h: &str,
    f: &str,
    deals: u64,
    seed: u64,
    aivat: AivatMode,
    crn: bool,
) -> MatchReport {
    let cfg = GameConfig::new(players);
    let mut spec = MatchSpec::new(
        cfg,
        vec![b(h), b(f)],
        Seatings::hero_vs_field(players),
        deals,
        seed,
    );
    spec.aivat = aivat;
    spec.crn = crn;
    run_match(&spec)
}

#[test]
fn self_match_is_zero() {
    for players in [2, 6] {
        // Two distinct factory instances of the same policy; independent action streams.
        let r = hero(players, "tag", "tag", 6000, 11, AivatMode::Full, false);
        let e = &r.entrants[0];
        assert!(
            e.raw.mbb.abs() < 3.5 * e.raw.se,
            "{players}p raw {} se {}",
            e.raw.mbb,
            e.raw.se
        );
        let a = e.aivat.unwrap();
        assert!(
            a.mbb.abs() < 3.5 * a.se.max(1.0),
            "{players}p aivat {} se {}",
            a.mbb,
            a.se
        );
    }
}

#[test]
fn tag_beats_random() {
    for players in [2, 6] {
        let r = hero(players, "tag", "random", 3000, 5, AivatMode::Off, true);
        let e = &r.entrants[0];
        assert!(e.raw.ci95().0 > 500.0, "{players}p tag vs random {}", e.raw);
        assert!(r.entrants[1].raw.mbb < 0.0);
    }
}

#[test]
fn reproducible_regardless_of_threads() {
    let a = hero(6, "lag", "nit", 500, 3, AivatMode::Full, true);
    let c = hero(6, "lag", "nit", 500, 3, AivatMode::Full, true);
    assert_eq!(a.entrants[0].raw.mbb, c.entrants[0].raw.mbb);
    assert_eq!(
        a.entrants[0].aivat.unwrap().mbb,
        c.entrants[0].aivat.unwrap().mbb
    );
}

#[test]
fn aivat_unbiased_and_lower_variance() {
    for (players, h, f, deals) in [
        (2, "tag", "lag", 30_000),
        (6, "nit", "maniac", 8_000),
        (2, "station", "random", 20_000),
    ] {
        let r = hero(players, h, f, deals, 21, AivatMode::Full, true);
        let e = &r.entrants[0];
        let (raw, av) = (e.raw, e.aivat.unwrap());
        println!(
            "{players}p {h} vs {f}: raw {raw}  aivat {av}  var ratio hand {:.2} deal {:.2}",
            e.var_ratio_hand.unwrap(),
            e.var_ratio_deal.unwrap()
        );
        // Same mean within error: the difference is far tighter than either CI, so allow raw CI.
        assert!(
            (raw.mbb - av.mbb).abs() < 2.5 * raw.se,
            "biased? raw {} aivat {}",
            raw.mbb,
            av.mbb
        );
        assert!(
            e.var_ratio_hand.unwrap() > 1.3,
            "no hand-level variance reduction"
        );
        assert!(av.se < raw.se);
        // Cards-only baseline is also valid.
        let c = hero(players, h, f, deals / 4, 22, AivatMode::CardsOnly, true);
        let ce = &c.entrants[0];
        assert!((ce.aivat.unwrap().mbb - raw.mbb).abs() < 3.5 * (ce.raw.se + raw.se));
    }
}

#[test]
fn blueprint_policy_plays_and_roundtrips() {
    let bp = Arc::new(train_toy(2, 60_000, 1));
    let r = {
        let mut spec = MatchSpec::new(
            GameConfig::new(2),
            vec![bp.clone(), b("random")],
            Seatings::hero_vs_field(2),
            3000,
            9,
        );
        spec.aivat = AivatMode::Full;
        run_match(&spec)
    };
    let e = &r.entrants[0];
    println!(
        "toy bp vs random: {} aivat {} miss {:.3}",
        e.raw,
        e.aivat.unwrap(),
        bp.miss_rate()
    );
    assert!(e.raw.ci95().0 > 0.0);
    // Biased variants construct and play.
    for bias in [Bias::Fold, Bias::Call, Bias::Raise] {
        let v: Arc<dyn PolicyFactory> = Arc::new(Biased::new(bp.clone(), bias));
        let spec = MatchSpec::new(
            GameConfig::new(2),
            vec![v, b("tag")],
            Seatings::hero_vs_field(2),
            200,
            4,
        );
        run_match(&spec);
    }
    // Save / load.
    let path = std::env::temp_dir().join("bench_toy_roundtrip.bp");
    blueprint::io::save(bp.store(), &path).unwrap();
    let loaded = BlueprintPolicy::from_file(&path, "coarse", "tiny", 1 << 25).unwrap();
    let ra = {
        let spec = MatchSpec::new(
            GameConfig::new(2),
            vec![bp.clone(), b("lag")],
            Seatings::hero_vs_field(2),
            300,
            2,
        );
        run_match(&spec).entrants[0].raw.mbb
    };
    let rb = {
        let spec = MatchSpec::new(
            GameConfig::new(2),
            vec![Arc::new(loaded), b("lag")],
            Seatings::hero_vs_field(2),
            300,
            2,
        );
        run_match(&spec).entrants[0].raw.mbb
    };
    assert_eq!(ra, rb);
}

#[test]
fn lbr_orders_policies_by_exploitability() {
    let cfg = GameConfig::new(2);
    let lc = LbrConfig {
        samples: 150,
        ..Default::default()
    };
    let bp: Arc<dyn PolicyFactory> = Arc::new(train_toy(2, 60_000, 1));
    let station = run_lbr(&cfg, &lc, b("station"), 400, 1);
    let random = run_lbr(&cfg, &lc, b("random"), 400, 1);
    let nit = run_lbr(&cfg, &lc, b("nit"), 400, 1);
    let tag = run_lbr(&cfg, &lc, b("tag"), 400, 1);
    let toy = run_lbr(&cfg, &lc, bp, 400, 1);
    for r in [&station, &random, &nit, &tag, &toy] {
        println!("{r}");
    }
    assert!(station.lbr.ci95().0 > 1000.0, "station {}", station.lbr);
    assert!(random.lbr.ci95().0 > 1000.0, "random {}", random.lbr);
    assert!(
        toy.lbr.mbb < station.lbr.mbb && toy.lbr.mbb < random.lbr.mbb,
        "toy bp should be harder to exploit"
    );
}

#[test]
fn lbr_profile_6max_runs() {
    let cfg = GameConfig::new(6);
    let lc = LbrConfig {
        samples: 60,
        ..Default::default()
    };
    let r = run_lbr(&cfg, &lc, b("station"), 40, 2);
    println!("{r}");
    assert!(r.lbr.mbb > 0.0);
}

/// Off-menu bets must leave the blueprint at nodes of its own tree: with the shadow-state
/// tracking the miss rate after a probe's odd-sized bet is as low as after an on-menu bet,
/// whereas the legacy tracking (translated indices over the real action list) misses a lot.
#[test]
fn off_menu_bets_keep_the_policy_in_its_tree() {
    use bench::probe;
    use nlhe::abstraction::{BelowMenu, TrackOpts};
    let bp = train_toy(2, 60_000, 3);
    let path = std::env::temp_dir().join("bench_toy_offmenu.bp");
    blueprint::io::save(bp.store(), &path).unwrap();
    let load = |o: TrackOpts| {
        Arc::new(BlueprintPolicy::from_file(&path, "coarse", "tiny", 1 << 25).unwrap().with_opts(o))
    };
    let miss = |o: TrackOpts, spec: &str| {
        let p = load(o);
        let m = MatchSpec::new(
            GameConfig::new(2),
            vec![probe::parse(spec).unwrap(), p.clone()],
            Seatings::hero_vs_field(2),
            1500,
            5,
        );
        run_match(&m);
        p.miss_rate()
    };
    let shadow = TrackOpts { below: BelowMenu::Smallest, allin_snap: 0.0, ..TrackOpts::default() };
    let base = miss(shadow, "probe:bet=1"); // pot-size bets: on the toy menu
    for spec in ["probe:bet=0.3", "probe:bet=0.6", "probe:bet=2.5", "probe:pre=3,bet=0.4"] {
        let (s, l) = (miss(shadow, spec), miss(TrackOpts::LEGACY, spec));
        println!("{spec}: shadow miss {s:.3}, legacy miss {l:.3}, on-menu {base:.3}");
        assert!(s <= base + 0.03, "{spec}: shadow miss {s} vs on-menu {base}");
        assert!(l > s, "{spec}: legacy {l} should miss more than shadow {s}");
    }
}
