//! Prints the RESULTS.md tables. `cargo run --release -p search-lab --example report [2|3]`.
use cfr_lab::eval;
use cfr_lab::leduc::Leduc;
use cfr_lab::Tree;
use search_lab::blueprint::*;
use search_lab::composite::*;
use search_lab::subgame::{Depth, Leaf};

fn variants(iters: u64) -> Vec<(String, SearchCfg)> {
    let base = SearchCfg::pluribus(iters);
    let mut v = vec![];
    let mut add = |n: &str, c: SearchCfg| v.push((n.to_string(), c));
    let r1 = SearchCfg {
        round0: false,
        ..base
    };
    add("R2 only (blueprint R1), final", r1);
    add(
        "R2 only, average",
        SearchCfg {
            play: Play::Average,
            ..r1
        },
    );
    let r0 = SearchCfg {
        round1: false,
        ..base
    };
    for (n, l) in [
        ("cont4", Leaf::Cont4),
        ("blueprint-leaf", Leaf::Blueprint),
        ("Modicum", Leaf::Modicum),
    ] {
        add(
            &format!("R1 only {n} depth-limited, final"),
            SearchCfg { leaf: l, ..r0 },
        );
        add(
            &format!("R1 only {n} depth-limited, average"),
            SearchCfg {
                leaf: l,
                play: Play::Average,
                ..r0
            },
        );
    }
    add(
        "R1 only full-depth (re-solve whole game), average",
        SearchCfg {
            depth: Depth::Game,
            play: Play::Average,
            ..r0
        },
    );
    add("R1+R2 cont4 depth-limited, final", base);
    add(
        "R1+R2 cont4 depth-limited, average",
        SearchCfg {
            play: Play::Average,
            ..base
        },
    );
    add(
        "R1+R2 blueprint-leaf, final",
        SearchCfg {
            leaf: Leaf::Blueprint,
            ..base
        },
    );
    add(
        "R1+R2 blueprint-leaf, average",
        SearchCfg {
            leaf: Leaf::Blueprint,
            play: Play::Average,
            ..base
        },
    );
    add(
        "R1+R2 Modicum, final",
        SearchCfg {
            leaf: Leaf::Modicum,
            ..base
        },
    );
    add(
        "R1+R2 Modicum, average",
        SearchCfg {
            leaf: Leaf::Modicum,
            play: Play::Average,
            ..base
        },
    );
    add(
        "R1+R2 full-depth, final",
        SearchCfg {
            depth: Depth::Game,
            ..base
        },
    );
    add(
        "R1+R2 full-depth, average",
        SearchCfg {
            depth: Depth::Game,
            play: Play::Average,
            ..base
        },
    );
    v
}

fn main() {
    let players: usize = std::env::args().nth(1).map_or(2, |s| s.parse().unwrap());
    let iters: u64 = std::env::args()
        .nth(2)
        .map_or(if players == 2 { 2000 } else { 200 }, |s| {
            s.parse().unwrap()
        });
    let game = Leduc::new(players, 3);
    let tree = Tree::build(&game);
    let score = |p: &[f64]| {
        if players == 2 {
            eval::exploitability(&tree, p)
        } else {
            eval::nash_conv(&tree, p)
        }
    };
    let (mc_it, ab_it) = if players == 2 {
        (3000, 2000)
    } else {
        (20000, 300)
    };
    let eq = equilibrium_blueprint(&tree, if players == 2 { 5000 } else { 100 });
    let bps: Vec<(String, Vec<f64>)> = vec![
        (
            format!("MCCFR-P {mc_it} it"),
            mccfr_blueprint(&tree, mc_it, 7),
        ),
        (
            "card-abstract CFR+".into(),
            abstract_blueprint(&game, &tree, ab_it),
        ),
        ("equilibrium CFR+".into(), eq.clone()),
        (
            "round-1 flawed (MCCFR-P 300 it), round-2 equilibrium".into(),
            splice(&tree, &mccfr_blueprint(&tree, 300, 3), &eq, 1),
        ),
    ];
    println!(
        "metric: {} (milli-chips), search iters {iters}\n",
        if players == 2 {
            "exploitability"
        } else {
            "NashConv"
        }
    );
    for (name, bp) in &bps {
        let s = Searcher::new(&tree, &game, bp);
        println!("### blueprint: {name}: {:.1}", score(bp) * 1000.0);
        for (vn, cfg) in variants(iters) {
            let t = std::time::Instant::now();
            let c = s.compose(&cfg);
            let h2h = if players == 2 {
                format!(
                    "  h2h vs bp {:+.1}",
                    head_to_head(&tree, &c.played, bp) * 1000.0
                )
            } else {
                String::new()
            };
            println!(
                "| {vn} | {:.1} |{h2h}  ({:.1}s)",
                score(&c.played) * 1000.0,
                t.elapsed().as_secs_f64()
            );
        }
    }
}
