//! Validation: parser roundtrips, replay legality, plausible scores, solver vs analytic answers.

use bench::bp::train_toy;
use bench::builtin::builtin;
use bench::policy::PolicyFactory;
use referee::pokerbench::*;
use referee::river::{self, RangeMode, RiverSpot};
use referee::solver::{showdown_brute, Ctx, Kind, Strategy, N};
use referee::toy::toy_game;

fn data_present() -> bool {
    default_dir().join(file_name(true, true)).exists()
}

#[test]
fn parser_roundtrips() {
    if !data_present() {
        eprintln!("PokerBench data missing; skipping");
        return;
    }
    let dir = default_dir();
    let pre = std::fs::read_to_string(dir.join(file_name(true, true))).unwrap();
    let rows = parse_csv(&pre);
    let ci = rows[0].iter().position(|x| x == "prev_line").unwrap();
    for r in &rows[1..] {
        let l = parse_pre_line(&r[ci]).unwrap();
        assert_eq!(fmt_pre_line(&l), r[ci]);
    }
    let post = std::fs::read_to_string(dir.join(file_name(false, true))).unwrap();
    let rows = parse_csv(&post);
    let h = &rows[0];
    let col = |n: &str| h.iter().position(|x| x == n).unwrap();
    let (cp, cl, ct, cr, cf) = (
        col("postflop_action"),
        col("preflop_action"),
        col("board_turn"),
        col("board_river"),
        col("board_flop"),
    );
    for r in &rows[1..] {
        assert_eq!(fmt_pre_line(&parse_pre_line(&r[cl]).unwrap()), r[cl]);
        let l = parse_post_line(&r[cp]).unwrap();
        let dealt = cards::parse_cards(&format!("{}{}", r[ct], r[cr])).unwrap();
        assert_eq!(fmt_post_line(&l, &dealt), r[cp]);
        assert!(cards::parse_cards(&r[cf]).unwrap().len() == 3);
    }
}

#[test]
fn replayed_states_are_legal_and_covered() {
    if !data_present() {
        return;
    }
    let (spots, bad) = load(&default_dir(), Which::All, true).unwrap();
    assert_eq!(bad, 0);
    assert_eq!(spots.len(), 11_000);
    let mut ok = 0;
    let mut pot_ok = 0;
    for s in &spots {
        if let Ok(r) = replay_spot(s) {
            ok += 1;
            let st = &r.replay.state;
            assert_eq!(st.to_act(), Some(r.hero_seat));
            assert_eq!(st.street(), s.street);
            assert!(!st.is_terminal());
            pot_ok += (r.pot_diff_bb.abs() <= 1.0 + 0.03 * s.pot_bb) as usize;
        }
    }
    assert!(
        ok as f64 / spots.len() as f64 > 0.99,
        "coverage {ok}/{}",
        spots.len()
    );
    assert!(pot_ok as f64 / ok as f64 > 0.9);
}

fn acc(name: &str, limit: usize) -> f64 {
    let f = builtin(name).unwrap();
    let (spots, bad) = load(&default_dir(), Which::All, true).unwrap();
    let spots = stride(&spots, limit);
    run(&spots, bad, &*f, 7).overall.exact
}

#[test]
fn builtin_accuracy_is_plausible() {
    if !data_present() {
        return;
    }
    let (tag, station, random) = (acc("tag", 3000), acc("station", 3000), acc("random", 3000));
    eprintln!("tag {tag:.3} station {station:.3} random {random:.3}");
    // Labels are balanced over fold/check/call/raise, so chance for a 4-way guess is ~25-30%.
    assert!(tag > 0.40 && tag < 0.80);
    assert!(tag > station && tag > random);
}

#[test]
fn showdown_matches_brute_force() {
    let board: [u8; 5] = cards::parse_cards("Ks7h2dJc7c")
        .unwrap()
        .try_into()
        .unwrap();
    let ctx = Ctx::new(board);
    let mut rng = cards::Rng::new(5);
    let mut r = vec![0.0; N];
    for x in r.iter_mut() {
        *x = rng.f64();
    }
    ctx.mask(&mut r);
    let mut fast = vec![0.0; N];
    ctx.showdown(&r, 3.0, &mut fast);
    let slow = showdown_brute(&ctx, &r, 3.0);
    for h in 0..N {
        assert!(
            (fast[h] - slow[h]).abs() < 1e-9,
            "hand {h}: {} vs {}",
            fast[h],
            slow[h]
        );
    }
}

#[test]
fn toy_river_matches_analytic_equilibrium() {
    let g = toy_game();
    let eq = g.solve(10_000);
    let v = g.value(&eq);
    assert!(
        (v[0] - 50.0).abs() < 0.5 && (v[1] + 50.0).abs() < 0.5,
        "values {v:?}"
    );
    assert!(
        g.exploitability(&eq) * 10.0 < 5.0,
        "expl mbb {}",
        g.exploitability(&eq) * 10.0
    );
    // Root: OOP acts; actions = [check, bet pot].
    let Kind::Action {
        acts, kids, player, ..
    } = &g.tree.nodes[0].kind
    else {
        panic!()
    };
    let player = &(*player);
    assert_eq!((*player as usize, acts.len()), (0, 2));
    let avg = |node: usize, a: usize, pl: usize| {
        let h: Vec<usize> = (0..N).filter(|&h| g.range[pl][h] > 0.0).collect();
        (
            h.iter().map(|&i| eq.0[node][a * N + i]).sum::<f64>() / h.len() as f64,
            h,
        )
    };
    // Value hands (AA) bet always, air (TT-as-3s: pairs of threes are our air) mix 1/2.
    let holes = cards::all_holes();
    let bet_freq = |rank: u8| {
        let hs: Vec<usize> = (0..N)
            .filter(|&h| {
                g.range[0][h] > 0.0 && holes[h][0] >> 2 == rank && holes[h][1] >> 2 == rank
            })
            .collect();
        hs.iter().map(|&i| eq.0[0][N + i]).sum::<f64>() / hs.len() as f64
    };
    assert!(bet_freq(12) > 0.99, "value bet freq {}", bet_freq(12));
    assert!(
        (bet_freq(1) - 0.5).abs() < 0.03,
        "bluff freq {}",
        bet_freq(1)
    );
    // IP facing the bet (kids[1]): actions [fold, call]; calls half.
    let (call, _) = avg(kids[1], 1, 1);
    assert!((call - 0.5).abs() < 0.03, "call freq {call}");
}

#[test]
fn equilibrium_loses_nothing_and_station_does() {
    let g = toy_game();
    let reference = g.solve(10_000);
    let mid = g.solve(300);
    let l = g.ev_loss(&mid, &reference);
    assert!(
        l[0].abs() < 100.0 && l[1].abs() < 100.0,
        "converged-strategy loss {l:?}"
    );
    let l_ref = g.ev_loss(&reference, &reference);
    assert!(
        l_ref[0].abs() < 10.0 && l_ref[1].abs() < 10.0,
        "self loss {l_ref:?}"
    );
    let station = river::station_strategy(&g);
    let ls = g.ev_loss(&station, &reference);
    // OOP station never bluffs/value-bets: loses a lot; IP station is exploited by value bets.
    assert!(ls[0] > 100.0 && ls[1] > 100.0, "station loss {ls:?}");
    let _: &Strategy = &station;
}

#[test]
fn river_spots_end_to_end() {
    if !data_present() {
        return;
    }
    let (spots, _) = load(&default_dir(), Which::Postflop, true).unwrap();
    let mut done = 0;
    let tag = builtin("tag").unwrap();
    let toy = train_toy(6, 2000, 1);
    for s in spots
        .iter()
        .filter(|s| s.street == nlhe::Street::River)
        .step_by(997)
    {
        let Ok(rs) = RiverSpot::from_spot(s) else {
            continue;
        };
        for f in [&*tag as &dyn PolicyFactory, &toy] {
            let r = river::evaluate(&rs, f, RangeMode::Policy, 400, 3).unwrap();
            assert!(r.ev_loss_mbb.iter().all(|x| x.is_finite()));
            let tol = 0.05 * r.pot_bb * 1000.0;
            assert!(r.ref_expl_mbb < tol, "solver not converged: {r:?}");
            // Losses are non-negative up to solver error.
            assert!(r.ev_loss_mbb.iter().all(|&x| x > -tol), "{r:?}");
            assert!(r.station_loss_mbb.iter().all(|&x| x > 0.0), "{r:?}");
            assert!(
                (r.eq_value_mbb[0] + r.eq_value_mbb[1]).abs() < 1.0,
                "zero-sum: {r:?}"
            );
        }
        done += 1;
        if done == 3 {
            break;
        }
    }
    assert!(done >= 1);
}
