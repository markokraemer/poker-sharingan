//! Exploitability-vs-iterations CSV for every solver on Leduc 2p.
//! `cargo run --release -p cfr-lab --example curves [max_iters]`

use cfr_lab::cfr::{Cfr, Variant};
use cfr_lab::eval::exploitability;
use cfr_lab::leduc::Leduc;
use cfr_lab::mccfr::{Mccfr, MccfrConfig};
use cfr_lab::{Solver, Tree};
use std::time::Instant;

fn run(name: &str, s: &mut dyn Solver, tree: &Tree, checkpoints: &[u64]) {
    let t0 = Instant::now();
    let mut done = 0;
    for &c in checkpoints {
        while done < c {
            s.iterate();
            done += 1;
        }
        let (a, c2) = (
            exploitability(tree, &s.average()),
            exploitability(tree, &s.current()),
        );
        println!(
            "{name},{c},{a:.6},{:.3},{c2:.6},{:.3},{:.2}",
            a * 1000.0,
            c2 * 1000.0,
            t0.elapsed().as_secs_f64()
        );
    }
}

fn main() {
    let max: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let tree = Tree::build(&Leduc::standard());
    eprintln!(
        "Leduc 2p: {} nodes, {} infosets",
        tree.nodes.len(),
        tree.info.len()
    );
    let mut cps = vec![];
    let mut c = 1;
    while c < max {
        cps.push(c);
        cps.push(c * 2);
        cps.push(c * 5);
        c *= 10;
    }
    cps.retain(|&c| c <= max);
    cps.sort();
    cps.dedup();
    println!("solver,iters,expl_avg,expl_avg_milli,expl_cur,expl_cur_milli,secs");
    for (name, v) in [
        ("cfr", Variant::Vanilla),
        ("cfr+", Variant::Plus),
        ("lcfr", Variant::Linear),
        ("dcfr", Variant::DCFR),
    ] {
        run(name, &mut Cfr::new(&tree, v), &tree, &cps);
    }
    let big = max * 100;
    let mut cps2 = vec![];
    let mut c = 100;
    while c <= big {
        cps2.push(c);
        cps2.push(c * 3);
        c *= 10;
    }
    cps2.retain(|&c| c <= big);
    run(
        "mccfr-p",
        &mut Mccfr::new(&tree, MccfrConfig::small_game(big, 1)),
        &tree,
        &cps2,
    );
}
