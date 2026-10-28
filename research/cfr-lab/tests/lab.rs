use cfr_lab::cfr::{Cfr, Variant};
use cfr_lab::eval::{best_response, exploitability, nash_conv, values};
use cfr_lab::kuhn::Kuhn;
use cfr_lab::leduc::Leduc;
use cfr_lab::mccfr::{Mccfr, MccfrConfig};
use cfr_lab::{Solver, Tree};

fn run<S: Solver>(s: &mut S, n: u64) {
    for _ in 0..n {
        s.iterate();
    }
}

fn kuhn2() -> Tree {
    Tree::build(&Kuhn::new(2))
}
fn leduc() -> Tree {
    Tree::build(&Leduc::standard())
}

#[test]
fn infoset_counts() {
    let k2 = kuhn2();
    assert_eq!(k2.info.len(), 12);
    assert_eq!(leduc().info.len(), 936);
    let k3 = Tree::build(&Kuhn::new(3));
    println!("Kuhn 3p infosets: {}", k3.info.len());
    // zero-sum sanity on every game
    for t in [&k2, &k3, &leduc(), &Tree::build(&Leduc::new(3, 3))] {
        let v = values(t, &t.uniform_profile());
        assert!(v.iter().sum::<f64>().abs() < 1e-9, "{v:?}");
    }
}

#[test]
fn uniform_baselines() {
    for (name, t) in [("kuhn2", kuhn2()), ("leduc2", leduc())] {
        let u = t.uniform_profile();
        println!(
            "{name} uniform: exploitability {:.4} ({:.1} milli), values {:?}",
            exploitability(&t, &u),
            exploitability(&t, &u) * 1e3,
            values(&t, &u)
        );
        assert!(exploitability(&t, &u) > 0.1);
    }
}

/// Independent check of best_response: brute-force all pure strategies of P1 in Kuhn.
#[test]
fn best_response_matches_brute_force() {
    let t = kuhn2();
    let mut prof = t.uniform_profile();
    // an arbitrary non-uniform P2 strategy
    for (k, i) in t.info.iter().enumerate().filter(|(_, i)| i.player == 1) {
        let w = 0.2 + 0.13 * (k % 5) as f64;
        prof[i.offset] = w;
        prof[i.offset + 1] = 1.0 - w;
    }
    let mine: Vec<usize> = (0..t.info.len())
        .filter(|&k| t.info[k].player == 0)
        .collect();
    let mut best = f64::MIN;
    for mask in 0..(1u32 << mine.len()) {
        let mut q = prof.clone();
        for (b, &k) in mine.iter().enumerate() {
            let i = &t.info[k];
            let a = (mask >> b & 1) as usize;
            q[i.offset..i.offset + 2].fill(0.0);
            q[i.offset + a] = 1.0;
        }
        best = best.max(values(&t, &q)[0]);
    }
    assert!((best - best_response(&t, &prof, 0)).abs() < 1e-12);
}

#[test]
fn kuhn2_cfr_family_converges() {
    let t = kuhn2();
    for (name, v, iters) in [
        ("cfr", Variant::Vanilla, 20_000),
        ("cfr+", Variant::Plus, 5_000),
        ("lcfr", Variant::Linear, 5_000),
        ("dcfr", Variant::DCFR, 5_000),
    ] {
        let mut s = Cfr::new(&t, v);
        run(&mut s, iters);
        let avg = s.average();
        let e = exploitability(&t, &avg);
        let v1 = values(&t, &avg)[0];
        println!("kuhn2 {name} {iters}: expl {e:.2e} v1 {v1:.5}");
        assert!(e < 1e-3, "{name}: {e}");
        assert!((v1 + 1.0 / 18.0).abs() < 1e-3, "{name}: {v1}");
    }
}

#[test]
fn kuhn2_mccfr_converges() {
    let t = kuhn2();
    let n = 200_000;
    let mut s = Mccfr::new(&t, MccfrConfig::small_game(n, 3));
    run(&mut s, n);
    let avg = s.average();
    let (e, v1) = (exploitability(&t, &avg), values(&t, &avg)[0]);
    println!(
        "kuhn2 mccfr-p {n}: expl {e:.2e} v1 {v1:.5} (pruned {})",
        s.pruned
    );
    assert!(e < 5e-3, "{e}");
    assert!((v1 + 1.0 / 18.0).abs() < 5e-3, "{v1}");
}

#[test]
fn leduc_cfr_family_converges() {
    let t = leduc();
    // (name, variant, iterations, threshold): thresholds are ~2-4x the measured values.
    for (name, v, iters, thr) in [
        ("cfr", Variant::Vanilla, 2_000, 0.012),
        ("cfr+", Variant::Plus, 1_000, 1e-3),
        ("lcfr", Variant::Linear, 2_000, 6e-3),
        ("dcfr", Variant::DCFR, 1_000, 1e-3),
    ] {
        let mut s = Cfr::new(&t, v);
        run(&mut s, iters);
        let e = exploitability(&t, &s.average());
        println!("leduc {name} {iters}: expl {e:.2e}");
        assert!(e < thr, "{name}: {e}");
    }
}

#[test]
fn leduc_mccfr_p_decreases_and_converges() {
    let t = leduc();
    let n = 1_000_000;
    let mut s = Mccfr::new(&t, MccfrConfig::small_game(n, 11));
    let mut curve = vec![];
    for k in 1..=n {
        s.iterate();
        if matches!(k, 1_000 | 10_000 | 100_000 | 1_000_000) {
            let e = exploitability(&t, &s.average());
            println!("leduc mccfr-p {k}: expl {e:.4} (pruned {})", s.pruned);
            curve.push(e);
        }
    }
    assert!(s.pruned > 0, "pruning never triggered");
    assert!(curve.windows(2).all(|w| w[1] < w[0]), "{curve:?}");
    assert!(curve[3] < 0.08, "{curve:?}");
    // The current iterate is also a valid profile, just much noisier.
    println!(
        "leduc mccfr-p current-iterate expl {:.4}",
        exploitability(&t, &s.current())
    );
}

#[test]
fn kuhn3_nash_conv_decreases() {
    let t = Tree::build(&Kuhn::new(3));
    let start = nash_conv(&t, &t.uniform_profile());
    let mut s = Cfr::new(&t, Variant::Plus);
    run(&mut s, 3_000);
    let end = nash_conv(&t, &s.average());
    println!("kuhn3 NashConv: uniform {start:.4} -> cfr+ {end:.5}");
    assert!(end < start / 20.0);
    let n = 300_000;
    let mut m = Mccfr::new(&t, MccfrConfig::small_game(n, 5));
    run(&mut m, n);
    let em = nash_conv(&t, &m.average());
    println!("kuhn3 NashConv: mccfr-p {em:.5}");
    assert!(em < start / 10.0);
}

#[test]
fn leduc3_nash_conv_decreases() {
    let t = Tree::build(&Leduc::new(3, 3));
    println!("leduc3: {} nodes, {} infosets", t.nodes.len(), t.info.len());
    let start = nash_conv(&t, &t.uniform_profile());
    let mut s = Cfr::new(&t, Variant::Plus);
    run(&mut s, 100);
    let end = nash_conv(&t, &s.average());
    println!("leduc3 NashConv: uniform {start:.4} -> cfr+(100) {end:.4}");
    assert!(end < start / 2.0);
}
