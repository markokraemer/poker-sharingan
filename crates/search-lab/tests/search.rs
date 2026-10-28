//! Key claims, fixed seeds. Numbers in comments are the values measured when written
//! (milli-chips); see RESULTS.md.

use cfr_lab::eval;
use cfr_lab::leduc::Leduc;
use cfr_lab::{Game, Profile, Tree, Turn};
use search_lab::beliefs;
use search_lab::blueprint::*;
use search_lab::composite::*;
use search_lab::subgame::{self, Conts, Ctx, Depth, Leaf};
use search_lab::view::View;
use std::collections::HashMap;

const ITERS: u64 = 2000;

struct Setup {
    game: Leduc,
    tree: Tree,
}

fn setup(players: usize) -> Setup {
    let game = Leduc::new(players, 3);
    let tree = Tree::build(&game);
    Setup { game, tree }
}

fn expl(s: &Setup, p: &[f64]) -> f64 {
    eval::exploitability(&s.tree, p) * 1000.0
}

fn run(s: &Setup, bp: &[f64], cfg: SearchCfg) -> Profile {
    Searcher::new(&s.tree, &s.game, bp).compose(&cfg).played
}

fn cfg(leaf: Leaf, depth: Depth, play: Play, r0: bool, r1: bool) -> SearchCfg {
    SearchCfg {
        leaf,
        depth,
        iters: ITERS,
        play,
        belief: Play::Average,
        round0: r0,
        round1: r1,
    }
}

/// Independent Bayes: replay the Game from the root over every deal, multiplying the
/// profile's probability of the observed action path; collect P(card of p | public state).
fn brute(
    g: &Leduc,
    tree: &Tree,
    prof: &[f64],
    path: &[u8],
    board: Option<u8>,
    p: usize,
) -> Vec<f64> {
    let idx: HashMap<(usize, String), usize> = tree
        .info
        .iter()
        .map(|i| ((i.player, i.key.clone()), i.offset))
        .collect();
    let mut m = vec![0.0; 6];
    fn cards(dbg: &str) -> Vec<usize> {
        let a = dbg.find("cards: [").unwrap() + 8;
        let b = a + dbg[a..].find(']').unwrap();
        dbg[a..b]
            .split(',')
            .map(|x| x.trim().parse().unwrap())
            .collect()
    }
    #[allow(clippy::too_many_arguments)]
    fn rec(
        g: &Leduc,
        idx: &HashMap<(usize, String), usize>,
        prof: &[f64],
        s: &<Leduc as Game>::State,
        path: &[u8],
        board: Option<u8>,
        p: usize,
        w: f64,
        m: &mut [f64],
    ) {
        match g.turn(s) {
            Turn::Chance => {
                let pr = g.chance_probs(s);
                for a in 0..g.num_outcomes(s) {
                    rec(g, idx, prof, &g.child(s, a), path, board, p, w * pr[a], m);
                }
            }
            Turn::Player(q) => {
                let key = g.info_key(s);
                if path.is_empty() {
                    let f: Vec<&str> = key.splitn(4, '|').collect();
                    let b_ok = match board {
                        None => f[2] == "-",
                        Some(b) => f[2] == b.to_string(),
                    };
                    if b_ok {
                        m[cards(&format!("{s:?}"))[p]] += w;
                    }
                    return;
                }
                let off = idx[&(q, key)];
                let a = path[0] as usize;
                rec(
                    g,
                    idx,
                    prof,
                    &g.child(s, a),
                    &path[1..],
                    board,
                    p,
                    w * prof[off + a],
                    m,
                );
            }
            Turn::Terminal => {}
        }
    }
    rec(g, &idx, prof, &g.root(), path, board, p, 1.0, &mut m);
    let tot: f64 = m.iter().sum();
    m.iter_mut().for_each(|x| *x /= tot);
    m
}

#[test]
fn beliefs_match_brute_force_bayes() {
    let s = setup(2);
    let bp = mccfr_blueprint(&s.tree, 3000, 7);
    let view = View::build(&s.tree, &s.game);
    let reach = beliefs::reach(&s.tree, &bp);
    let mut checked = 0;
    for (pk, (path, board)) in view.publics.iter().enumerate() {
        let nodes = beliefs::state_nodes(&s.tree, &view, pk as u32);
        if nodes.is_empty() || path.len() > 4 {
            continue;
        }
        for p in 0..2 {
            let m = beliefs::marginal(&view, &reach, &nodes, p, 6);
            let tot: f64 = m.iter().sum();
            if tot == 0.0 {
                continue; // unreachable under the profile
            }
            assert!((tot - 1.0).abs() < 1e-12);
            let b = (*board != 255).then_some(*board);
            let bf = brute(&s.game, &s.tree, &bp, path, b, p);
            for c in 0..6 {
                assert!((m[c] - bf[c]).abs() < 1e-12, "{path:?} {board} p{p} c{c}");
            }
            checked += 1;
        }
    }
    assert!(checked > 40, "{checked}");
    // Card removal: at the very root every card is uniform; with a board, the board card
    // has zero probability of being held and the same-rank twin is less likely than 1/5.
    let root = beliefs::state_nodes(&s.tree, &view, view.lookup(&[], None).unwrap());
    let m = beliefs::marginal(&view, &reach, &root, 0, 6);
    assert!(m.iter().all(|x| (x - 1.0 / 6.0).abs() < 1e-12));
    let (pk, board) = view
        .publics
        .iter()
        .enumerate()
        .find(|(_, (p, b))| p.len() == 2 && *b != 255)
        .map(|(i, (_, b))| (i as u32, *b))
        .unwrap();
    let nodes = beliefs::state_nodes(&s.tree, &view, pk);
    let m = beliefs::marginal(&view, &reach, &nodes, 0, 6);
    assert_eq!(m[board as usize], 0.0);
}

/// Subgame plumbing: with blueprint leaf values and the blueprint as the strategy, the
/// subgame's value equals the real game's value (checks root weights, infoset sharing and
/// exact leaf evaluation), for round-end and full-depth subgames.
#[test]
fn subgame_values_match_full_game() {
    let s = setup(2);
    let bp = mccfr_blueprint(&s.tree, 1000, 5);
    let view = View::build(&s.tree, &s.game);
    let kinds = subgame::action_kinds(&s.tree, &s.game);
    let conts = Conts::new(&s.tree, &kinds, &bp);
    let cx = Ctx {
        tree: &s.tree,
        view: &view,
        conts: &conts,
    };
    let reach = beliefs::reach(&s.tree, &bp);
    let full = eval::values(&s.tree, &bp);
    let (_, nodes) = view.round_roots(&s.tree, 0).remove(0);
    for depth in [Depth::RoundEnd, Depth::Game] {
        let roots = subgame::root_weights(&reach, &nodes);
        let sub = subgame::build(&cx, &roots, 0, depth, Leaf::Blueprint, None);
        assert_eq!(depth == Depth::RoundEnd, sub.leaves > 0);
        let mut prof = vec![0.0; sub.tree.total_actions];
        for (i, &f) in sub.full.iter().enumerate() {
            let (a, b) = (&sub.tree.info[i], &s.tree.info[f as usize]);
            prof[a.offset..a.offset + a.n_actions]
                .copy_from_slice(&bp[b.offset..b.offset + b.n_actions]);
        }
        let v = eval::values(&sub.tree, &prof);
        assert!((v[0] - full[0]).abs() < 1e-9, "{depth:?} {v:?} {full:?}");
    }
    // Continuations are valid distributions.
    for c in &conts.profs {
        for i in &s.tree.info {
            let t: f64 = c[i.offset..i.offset + i.n_actions].iter().sum();
            assert!((t - 1.0).abs() < 1e-9);
        }
    }
}

/// Exact-equilibrium blueprint: searching does not blow it up. Solving the whole game at
/// the root is ~equilibrium; the full nested pipeline (unsafe round-2 re-solves) stays far
/// below uniform play (2374). Measured: eq 0.0, R1 full-depth avg 3.3, R2 only avg 51.8,
/// R1+R2 full-depth avg 221.
#[test]
fn equilibrium_blueprint_stays_near_equilibrium() {
    let s = setup(2);
    let bp = equilibrium_blueprint(&s.tree, 5000);
    assert!(expl(&s, &bp) < 0.1);
    let g = |r0, r1, play| {
        expl(
            &s,
            &run(&s, &bp, cfg(Leaf::Cont4, Depth::Game, play, r0, r1)),
        )
    };
    assert!(g(true, false, Play::Average) < 10.0);
    assert!(g(false, true, Play::Average) < 150.0);
    assert!(g(true, true, Play::Average) < 300.0);
}

/// Under-trained MCCFR-P blueprint (3000 it). Measured: bp 714; R1+R2 cont4 depth-limited
/// avg 488 / final 506; R2-only avg 202; head-to-head vs blueprint +291 / +304 milli.
#[test]
fn search_improves_mccfr_blueprint() {
    let s = setup(2);
    let bp = mccfr_blueprint(&s.tree, 3000, 7);
    let base = expl(&s, &bp);
    assert!((base - 714.3).abs() < 1.0, "{base}");
    for (play, bound) in [(Play::Average, 0.75), (Play::Final, 0.8)] {
        let c = run(&s, &bp, cfg(Leaf::Cont4, Depth::RoundEnd, play, true, true));
        let e = expl(&s, &c);
        assert!(e < bound * base, "{play:?} {e} vs {base}");
        let h = head_to_head(&s.tree, &c, &bp) * 1000.0;
        assert!(h > 100.0, "{play:?} h2h {h}");
    }
    let r2 = run(
        &s,
        &bp,
        cfg(Leaf::Cont4, Depth::RoundEnd, Play::Average, false, true),
    );
    assert!(expl(&s, &r2) < 0.4 * base);
    assert!(head_to_head(&s.tree, &r2, &bp) > 0.1);
}

/// Card-abstracted blueprint. Measured: bp 638.5; R2-only cont4 avg 374.7, h2h +80.
#[test]
fn search_repairs_card_abstraction() {
    let s = setup(2);
    let bp = abstract_blueprint(&s.game, &s.tree, 2000);
    let base = expl(&s, &bp);
    assert!(base > 300.0, "abstraction should hurt: {base}");
    let r2 = run(
        &s,
        &bp,
        cfg(Leaf::Cont4, Depth::RoundEnd, Play::Average, false, true),
    );
    assert!(expl(&s, &r2) < 0.7 * base, "{} vs {base}", expl(&s, &r2));
    assert!(head_to_head(&s.tree, &r2, &bp) > 0.03);
}

/// Depth-limited search with a flawed round-1 blueprint and sound later rounds: this is
/// where leaf values matter. Measured: bp 1114; round-1-only search with cont4 avg 31 /
/// final 67; blueprint-leaf 585 / 760; Modicum 1 / 123.
#[test]
fn depth_limited_cont4_repairs_early_round_flaw() {
    let s = setup(2);
    let eq = equilibrium_blueprint(&s.tree, 5000);
    let bp = splice(&s.tree, &mccfr_blueprint(&s.tree, 300, 3), &eq, 1);
    let base = expl(&s, &bp);
    let go = |leaf, play| {
        expl(
            &s,
            &run(&s, &bp, cfg(leaf, Depth::RoundEnd, play, true, false)),
        )
    };
    let c4 = go(Leaf::Cont4, Play::Average);
    let bl = go(Leaf::Blueprint, Play::Average);
    let md = go(Leaf::Modicum, Play::Average);
    eprintln!("splice: bp {base:.1} cont4 {c4:.1} bp-leaf {bl:.1} modicum {md:.1}");
    assert!(c4 < 0.1 * base);
    assert!(c4 < bl, "cont4 should beat blueprint-only leaf values");
    assert!(go(Leaf::Cont4, Play::Final) < 0.2 * base);
}

/// Report-only ablation on the MCCFR blueprint.
#[test]
fn ablation_report() {
    let s = setup(2);
    let bp = mccfr_blueprint(&s.tree, 3000, 7);
    for leaf in [Leaf::Cont4, Leaf::Blueprint, Leaf::Modicum] {
        let e = expl(
            &s,
            &run(
                &s,
                &bp,
                cfg(leaf, Depth::RoundEnd, Play::Average, true, true),
            ),
        );
        eprintln!("MCCFR bp, R1+R2 {leaf:?} avg: {e:.1}");
    }
}

/// 3-player NashConv. Measured: bp 2754; R2-only avg 728; R1+R2 cont4 avg 1884.
#[test]
fn three_player_nashconv_drops() {
    let s = setup(3);
    let bp = mccfr_blueprint(&s.tree, 20000, 7);
    let base = eval::nash_conv(&s.tree, &bp) * 1000.0;
    let nc = |c: SearchCfg| {
        let c = SearchCfg { iters: 200, ..c };
        eval::nash_conv(&s.tree, &run(&s, &bp, c)) * 1000.0
    };
    let r2 = nc(cfg(
        Leaf::Cont4,
        Depth::RoundEnd,
        Play::Average,
        false,
        true,
    ));
    let both = nc(cfg(Leaf::Cont4, Depth::RoundEnd, Play::Average, true, true));
    eprintln!("3p NashConv bp {base:.0} R2-only {r2:.0} R1+R2 {both:.0}");
    assert!(r2 < 0.5 * base, "{r2} vs {base}");
    assert!(both < 0.85 * base, "{both} vs {base}");
}
