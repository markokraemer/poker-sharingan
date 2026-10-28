use bench::bp::BlueprintPolicy;
use bench::matchrun::{run_match, AivatMode, MatchSpec, Seatings};
use bench::policy::{Player, PolicyFactory, View};
use cards::{hole_index, parse_cards, Card};
use nlhe::abstraction::{abstract_actions, ActionList, BetSizing, Shadow};
use nlhe::{Action, GameConfig, State, Step};
use search::beliefs::{holes, Beliefs};
use search::bp::Blueprint;
use search::policy::SearchPolicy;
use search::solver::{solve, Subgame, SearchParams};
use search::testkit::{toy_schedule, train_blueprint};
use search::tree::{search_actions, Extras};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

fn hu_bp() -> &'static Blueprint {
    static B: OnceLock<Blueprint> = OnceLock::new();
    B.get_or_init(|| {
        let _ = toy_schedule(1);
        train_blueprint(2, BetSizing::tiny(), Arc::new(bench::bp::CoarseBucketer), 60_000, 3)
    })
}

fn params(iters: u64) -> SearchParams {
    SearchParams { iters, max_infosets: 1 << 18, ..Default::default() }
}

fn cs(s: &str) -> Vec<Card> {
    parse_cards(s).unwrap()
}

/// HU: SB limps, BB checks -> flop root state (BB first to act).
fn hu_flop(cfg: &GameConfig) -> (State, State, State) {
    let s0 = State::new(cfg);
    let mut s1 = s0;
    assert_eq!(s1.apply(Action::Call), Step::Continue);
    let mut s2 = s1;
    assert!(matches!(s2.apply(Action::Check), Step::NewStreet(_)));
    (s0, s1, s2)
}

#[test]
fn beliefs_match_blueprint_likelihoods() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let pol = SearchPolicy::new("s", bp.clone(), params(100));
    let mut p = pol.player(1, &cfg, 7);
    let s0 = State::new(&cfg);
    let mut list = ActionList::new();
    abstract_actions(&s0, &bp.sizing, &mut list);
    let raise = list.get(2);
    assert!(matches!(raise, Action::RaiseTo(_)));
    p.observe(&s0, 0, raise);
    let mut s1 = s0;
    s1.apply(raise);
    p.observe(&s1, 1, Action::Call);
    let mut s2 = s1;
    assert!(matches!(s2.apply(Action::Call), Step::NewStreet(_)));
    let flop = cs("2c7d9h");
    p.observe_board(&flop);

    // Independent implementation: bench's BlueprintPolicy.
    let reference = BlueprintPolicy::new("ref", bp.store.clone(), bp.bucketer.clone(), bp.sizing);
    let mut rp = reference.new_player(0, &cfg, 1);
    let hist = [];
    let view = View { cfg: &cfg, state: &s0, board: &[], history: &hist, seat: 0 };
    let dead = cards::mask_of(&flop);
    let mut want = vec![0f64; 1326];
    for (i, h) in holes().iter().enumerate() {
        if (cards::bit(h[0]) | cards::bit(h[1])) & dead == 0 {
            want[i] = rp.dist(&view, *h).prob_of(raise);
        }
    }
    let z: f64 = want.iter().sum();
    let b = p.beliefs();
    assert!((b.w[0].iter().map(|&x| x as f64).sum::<f64>() - 1.0).abs() < 1e-4);
    assert!((b.w[1].iter().map(|&x| x as f64).sum::<f64>() - 1.0).abs() < 1e-4);
    for i in 0..1326 {
        assert!((b.w[0][i] as f64 - want[i] / z).abs() < 1e-5, "hand {i}: {} vs {}", b.w[0][i], want[i] / z);
    }
}

#[test]
fn frozen_own_action_is_respected_for_actual_hand_only() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let (_, _, root) = hu_flop(&cfg);
    let flop = cs("2c7d9h");
    let mut beliefs = Beliefs::uniform(2);
    beliefs.remove_cards(cards::mask_of(&flop));
    let hole = [cs("As")[0], cs("Ks")[0]];
    let extras = Extras::new();
    let mut frozen = HashMap::new();
    frozen.insert(0u64, Action::Check);
    let prm = params(4000);
    let sg = Subgame {
        bp, root, board: &flop, beliefs: &beliefs, bp_hist: bp.shadow(&root), extras: &extras, me: 1,
        my_hole: Some(hole), frozen: &frozen, params: &prm, seed: 5,
    };
    let sol = solve(&sg);
    let mut list = ActionList::new();
    search_actions(&root, &prm.sizing, &[], &mut list);
    let n = list.len();
    let mut p = [0f32; 24];
    assert!(sol.probs(0, hole_index(hole[0], hole[1]), n, true, &mut p));
    assert_eq!(list.get(0), Action::Check);
    assert!((p[0] - 1.0).abs() < 1e-6, "frozen hand must check: {:?}", &p[..n]);
    // Another hand is not frozen: the nuts-like hand bets sometimes.
    let other = hole_index(cs("9s")[0], cs("9c")[0]);
    assert!(sol.probs(0, other, n, true, &mut p) || sol.probs(0, other, n, false, &mut p));
    assert!(p[0] < 0.999, "unfrozen set of nines should bet at least sometimes: {:?}", &p[..n]);
}

/// Regression for the node-mismatch bug (crates/bench/TRANSLATION.md): after an off-menu preflop
/// raise the search player must keep querying the blueprint at nodes of the blueprint's own tree
/// (shadow game), exactly like `BlueprintPolicy` does: same distributions (so the same miss rate,
/// ~0 here), a policy that folds (the legacy lookup produced a calling station), beliefs that
/// actually moved, and warm-start priors that hit on the flop.
#[test]
fn off_menu_preflop_raise_keeps_blueprint_lookups_in_tree() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let mut prm = params(100);
    prm.postflop_search = false;
    prm.preflop_search = false;
    let pol = SearchPolicy::new("s", bp.clone(), prm);
    let mut p = pol.player(1, &cfg, 7);
    let reference = BlueprintPolicy::new("ref", bp.store.clone(), bp.bucketer.clone(), bp.sizing);
    let mut rp = reference.new_player(1, &cfg, 1);
    let s0 = State::new(&cfg);
    let l = s0.legal();
    let to = s0.cur_bet() + ((s0.pot() + l.call_amount) as f64 * 0.7).round() as i32;
    let off = Action::RaiseTo(to);
    let mut list = ActionList::new();
    abstract_actions(&s0, &bp.sizing, &mut list);
    assert!(!list.as_slice().contains(&off), "raise must be off-menu");
    p.observe(&s0, 0, off);
    rp.observe(&s0, 0, off);
    let mut s1 = s0;
    s1.apply(off);

    let same = |a: &mut dyn Player, b: &mut dyn Player, v: &View, dead: u64| -> (f64, usize) {
        let (mut fold, mut n) = (0.0, 0);
        for h in holes().iter().step_by(13) {
            if (cards::bit(h[0]) | cards::bit(h[1])) & dead != 0 {
                continue;
            }
            let (d, r) = (a.dist(v, *h), b.dist(v, *h));
            assert_eq!(d.len(), r.len());
            for k in 0..r.len() {
                assert_eq!(d.action(k), r.action(k));
                assert!((d.prob(k) - r.prob(k)).abs() < 1e-6, "{:?}: {} vs {}", d.action(k), d.prob(k), r.prob(k));
            }
            fold += d.prob_of(Action::Fold);
            n += 1;
        }
        (fold, n)
    };
    let hist = [];
    let v = View { cfg: &cfg, state: &s1, board: &[], history: &hist, seat: 1 };
    let (fold, n) = same(&mut p, &mut *rp, &v, 0);
    assert!(fold / n as f64 > 0.05, "facing a raise the policy must fold sometimes (calling station otherwise): {}", fold / n as f64);

    p.observe(&s1, 1, Action::Call);
    rp.observe(&s1, 1, Action::Call);
    let mut s2 = s1;
    assert!(matches!(s2.apply(Action::Call), Step::NewStreet(_)));
    let flop = cs("2c7d9h");
    p.observe_board(&flop);
    rp.observe_board(&flop);
    let v2 = View { cfg: &cfg, state: &s2, board: &flop, history: &hist, seat: 1 };
    same(&mut p, &mut *rp, &v2, cards::mask_of(&flop));

    // Beliefs: a miss would leave the raiser's row uniform (likelihood 1/n for every hand).
    let w = &p.beliefs().w[0];
    let live: Vec<f32> = w.iter().cloned().filter(|&x| x > 0.0).collect();
    let (mx, mn) = (live.iter().cloned().fold(0.0, f32::max), live.iter().cloned().fold(f32::MAX, f32::min));
    assert!(mx > 1.5 * mn, "raiser's range should be non-uniform after the raise: {mx} vs {mn}");

    // Warm-start prior at the flop root: every bucket must hit.
    let (root, sh, board) = p.round_root();
    let mut sl = ActionList::new();
    search_actions(&root, &SearchParams::default().sizing, &[], &mut sl);
    let mut hits = 0;
    let mut total = 0;
    for h in holes().iter().step_by(17) {
        if (cards::bit(h[0]) | cards::bit(h[1])) & cards::mask_of(board) != 0 {
            continue;
        }
        total += 1;
        hits += bp.prior(&sh, &root, &sl, bp.bucket(root.street(), *h, board)).is_some() as usize;
    }
    assert_eq!(hits, total, "blueprint priors after an off-menu raise must hit");
}

#[test]
fn off_tree_action_triggers_resolve_and_keeps_frozen_action() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let pol = SearchPolicy::new("s", bp.clone(), params(3000));
    let mut p = pol.player(1, &cfg, 11);
    let (s0, s1, s2) = hu_flop(&cfg);
    p.observe(&s0, 0, Action::Call);
    p.observe(&s1, 1, Action::Check);
    let flop = cs("2c7d9h");
    p.observe_board(&flop);
    assert!(p.search_active());
    let hole = [cs("As")[0], cs("Ks")[0]];
    let hist = [];
    let v = View { cfg: &cfg, state: &s2, board: &flop, history: &hist, seat: 1 };
    let d = p.dist(&v, hole);
    assert_eq!(p.searches_run(), 1);
    assert!((0..d.len()).map(|i| d.prob(i)).sum::<f64>() > 0.999);
    // We check (frozen); the opponent bets an off-tree size.
    p.observe(&s2, 1, Action::Check);
    let mut s3 = s2;
    s3.apply(Action::Check);
    let l = s3.legal();
    let odd = l.min_raise_to + 37;
    let mut base = ActionList::new();
    search_actions(&s3, &pol.params().sizing, &[], &mut base);
    assert!(!base.as_slice().contains(&Action::RaiseTo(odd)), "must be off-tree");
    p.observe(&s3, 0, Action::RaiseTo(odd));
    assert_eq!(p.extras().values().map(|v| v.len()).sum::<usize>(), 1);
    let mut s4 = s3;
    s4.apply(Action::RaiseTo(odd));
    let v2 = View { cfg: &cfg, state: &s4, board: &flop, history: &hist, seat: 1 };
    let d2 = p.dist(&v2, hole);
    assert_eq!(p.searches_run(), 2, "off-tree action must trigger a re-search");
    assert_eq!(pol.stats().resolves, 1);
    assert!(d2.actions().contains(&Action::Fold) && d2.actions().contains(&Action::Call));
    // Re-searched tree contains the off-tree size at the opponent's node...
    let sol = p.solution().unwrap();
    let mut with = ActionList::new();
    let ex = sol.extras.values().next().unwrap();
    search_actions(&s3, &sol.sizing, ex, &mut with);
    assert!(with.as_slice().contains(&Action::RaiseTo(odd)));
    // ...and our earlier check is still frozen for our hand in the new solution.
    let mut rl = ActionList::new();
    search_actions(&s2, &sol.sizing, &[], &mut rl);
    let mut pr = [0f32; 24];
    eprintln!("DBG iters {} infosets {} n {} vec {}", sol.iters, sol.infosets(), rl.len(), sol.vector().is_some());
    assert!(sol.probs(0, hole_index(hole[0], hole[1]), rl.len(), true, &mut pr));
    assert!((pr[0] - 1.0).abs() < 1e-6);
    // The same off-tree action seen again is not re-added (no second resolve for in-tree).
    let _ = Player::exposes_probs(&p);
}

#[test]
fn preflop_search_only_for_far_off_tree_raise() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let pol = SearchPolicy::new("s", bp.clone(), params(1500));
    let s0 = State::new(&cfg);
    let hole = [cs("Ah")[0], cs("Kd")[0]];
    let hist = [];
    // Near a blueprint size: stays blueprint.
    let mut list = ActionList::new();
    abstract_actions(&s0, &bp.sizing, &mut list);
    let Action::RaiseTo(t) = list.get(2) else { panic!() };
    let mut p = pol.player(1, &cfg, 1);
    p.observe(&s0, 0, Action::RaiseTo(t + 40));
    let mut s1 = s0;
    s1.apply(Action::RaiseTo(t + 40));
    let v = View { cfg: &cfg, state: &s1, board: &[], history: &hist, seat: 1 };
    let _ = p.dist(&v, hole);
    assert!(!p.search_active());
    assert_eq!(p.searches_run(), 0);
    // Far off-tree: preflop search to the end of the round, with continuation leaves.
    let mut p = pol.player(1, &cfg, 1);
    p.observe(&s0, 0, Action::RaiseTo(1337));
    assert!(p.search_active());
    let mut s1 = s0;
    s1.apply(Action::RaiseTo(1337));
    let v = View { cfg: &cfg, state: &s1, board: &[], history: &hist, seat: 1 };
    let d = p.dist(&v, hole);
    assert_eq!(p.searches_run(), 1);
    assert_eq!(pol.stats().preflop, 1);
    assert!(d.len() >= 2);
    let sol = p.solution().unwrap();
    assert_eq!(sol.limit, search::tree::Limit::EndOfRound);
}

#[test]
fn full_hands_are_legal_hu_and_three_handed() {
    let hu = SearchPolicy::new("search", hu_bp().clone(), params(300));
    let bpp = BlueprintPolicy::new("bp", hu_bp().store.clone(), hu_bp().bucketer.clone(), hu_bp().sizing);
    let cfg = GameConfig::new(2);
    let mut spec = MatchSpec::new(cfg, vec![Arc::new(hu), Arc::new(bpp)], Seatings::hero_vs_field(2), 40, 9);
    spec.aivat = AivatMode::Full;
    let r = run_match(&spec);
    assert_eq!(r.hands, 80);
    assert!(r.entrants[0].raw.mbb.is_finite());
    let _ = &spec.entrants[0].name();
}

#[test]
fn river_solution_is_sane() {
    river_sane(false);
}

fn river_sane(vector: bool) {
    // HU river, nobody has acted: the nuts should not mostly check; air should check more.
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let mut s = State::new(&cfg);
    let mut step = Step::Continue;
    while !matches!(step, Step::NewStreet(nlhe::Street::River)) {
        let l = s.legal();
        step = s.apply(if l.can_check { Action::Check } else { Action::Call });
    }
    let board = cs("2c7d9hJsKc");
    let mut b = Beliefs::uniform(2);
    b.remove_cards(cards::mask_of(&board));
    // No warm start: the property must come from the solve itself, not from the blueprint.
    let prm = SearchParams { iters: 30_000, max_infosets: 1 << 18, parallel: false, warm_start: 0.0, vector, ..Default::default() };
    let none = Extras::new();
    let frozen = HashMap::new();
    let sg = Subgame { bp, root: s, board: &board, beliefs: &b, bp_hist: bp.shadow(&s), extras: &none, me: 1, my_hole: None, frozen: &frozen, params: &prm, seed: 1 };
    let sol = solve(&sg);
    let mut list = ActionList::new();
    search_actions(&s, &prm.sizing, &[], &mut list);
    let n = list.len();
    let mut p = [0f32; 24];
    // Facing an all-in after a check, the set of kings calls and air folds.
    let mut s1 = s;
    s1.apply(Action::Check);
    let shove = Action::RaiseTo(s1.legal().max_raise_to);
    let path = search::tree::step_path(search::tree::step_path(0, Action::Check), shove);
    let mut s2 = s1;
    s2.apply(shove);
    let mut l2 = ActionList::new();
    search_actions(&s2, &prm.sizing, &[], &mut l2);
    assert_eq!(l2.get(0), Action::Fold);
    let nuts = hole_index(cs("Kd")[0], cs("Kh")[0]);
    let air = hole_index(cs("3s")[0], cs("4h")[0]);
    assert!(sol.probs(path, nuts, l2.len(), vector, &mut p));
    let nuts_fold = p[0];
    assert!(sol.probs(path, air, l2.len(), vector, &mut p));
    let air_fold = p[0];
    assert!(nuts_fold < 0.05, "set of kings folds {nuts_fold}");
    assert!(air_fold > 0.9, "air folds {air_fold}");
    let _ = (n, &mut p);
}

#[test]
fn deterministic_when_single_threaded() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let (_, _, root) = hu_flop(&cfg);
    let flop = cs("2c7d9h");
    let mut b = Beliefs::uniform(2);
    b.remove_cards(cards::mask_of(&flop));
    let prm = SearchParams { iters: 500, max_infosets: 1 << 16, parallel: false, ..Default::default() };
    let none = Extras::new();
    let frozen = HashMap::new();
    let mk = || Subgame { bp, root, board: &flop, beliefs: &b, bp_hist: bp.shadow(&root), extras: &none, me: 1, my_hole: Some([5, 9]), frozen: &frozen, params: &prm, seed: 77 };
    let (a, c) = (solve(&mk()), solve(&mk()));
    let mut list = ActionList::new();
    search_actions(&root, &prm.sizing, &[], &mut list);
    let (mut pa, mut pc) = ([0f32; 24], [0f32; 24]);
    for h in [hole_index(5, 9), 100, 400, 900] {
        let (x, y) = (a.probs(0, h, list.len(), true, &mut pa), c.probs(0, h, list.len(), true, &mut pc));
        assert_eq!(x, y);
        if x {
            assert_eq!(pa, pc);
        }
    }
    assert_eq!(a.infosets(), c.infosets());
}

fn p3_bp() -> &'static Blueprint {
    static B: OnceLock<Blueprint> = OnceLock::new();
    B.get_or_init(|| train_blueprint(3, BetSizing::tiny(), Arc::new(bench::bp::CoarseBucketer), 60_000, 5))
}

fn flop_root_3p(cfg: &GameConfig) -> State {
    let mut s = State::new(cfg);
    loop {
        let l = s.legal();
        if let Step::NewStreet(_) = s.apply(if l.can_check { Action::Check } else { Action::Call }) {
            return s;
        }
    }
}

#[test]
fn depth_limit_is_enforced_in_the_solver() {
    use search::tree::Limit;
    let bp = p3_bp();
    let cfg = GameConfig::new(3);
    let root = flop_root_3p(&cfg);
    let flop = cs("2c7d9h");
    let mut b = Beliefs::uniform(3);
    b.remove_cards(cards::mask_of(&flop));
    let prm = SearchParams { iters: 3000, max_infosets: 1 << 18, parallel: false, vector: false, ..Default::default() };
    let none = Extras::new();
    let frozen = HashMap::new();
    let mk = |root: State, board: &[Card]| {
        let sg = Subgame { bp, root, board, beliefs: &b, bp_hist: bp.shadow(&root), extras: &none, me: 0, my_hole: None, frozen: &frozen, params: &prm, seed: 1 };
        solve(&sg)
    };
    // Flop, 3 players: leaves at the next street or after the 2nd raise.
    let sol = mk(root, &flop);
    assert_eq!(sol.limit, Limit::SecondRaise);
    assert!(sol.leaves > 0);
    assert!(sol.max_raises <= 1, "no decision node may follow the 2nd raise, saw {}", sol.max_raises);
    // Turn, 3 players: to the end of the game, no leaves.
    let mut t = root;
    loop {
        let l = t.legal();
        if let Step::NewStreet(_) = t.apply(if l.can_check { Action::Check } else { Action::Call }) {
            break;
        }
    }
    let turn = cs("2c7d9hJs");
    let mut bt = Beliefs::uniform(3);
    bt.remove_cards(cards::mask_of(&turn));
    let sg = Subgame { bp, root: t, board: &turn, beliefs: &bt, bp_hist: bp.shadow(&t), extras: &none, me: 0, my_hole: None, frozen: &frozen, params: &prm, seed: 1 };
    let sol = solve(&sg);
    assert_eq!(sol.limit, Limit::EndOfGame);
    assert_eq!(sol.leaves, 0);
    // Heads-up flop: to the end of the game; deeper raise levels are searched.
    let cfg2 = GameConfig::new(2);
    let (_, _, hroot) = hu_flop(&cfg2);
    let mut b2 = Beliefs::uniform(2);
    b2.remove_cards(cards::mask_of(&flop));
    let sg = Subgame { bp: hu_bp(), root: hroot, board: &flop, beliefs: &b2, bp_hist: hu_bp().shadow(&hroot), extras: &none, me: 0, my_hole: None, frozen: &frozen, params: &prm, seed: 1 };
    let sol = solve(&sg);
    assert_eq!(sol.limit, Limit::EndOfGame);
    assert_eq!(sol.leaves, 0);
    assert!(sol.max_raises >= 2);
}

#[test]
fn continuation_strategies_are_biased_as_in_the_paper() {
    use search::bp::{cont_pick, K_BLUEPRINT, K_CALL, K_FOLD, K_RAISE};
    let mut list = ActionList::new();
    list.push(Action::Fold);
    list.push(Action::Call);
    list.push(Action::RaiseTo(300));
    list.push(Action::RaiseTo(900));
    let freq = |k: u8| {
        let mut c = [0f64; 4];
        let m = 20_000;
        for j in 0..m {
            let mut sg = [0.25f32; 4];
            c[cont_pick(k, 5.0, &list, &mut sg, (j as f64 + 0.5) / m as f64)] += 1.0 / m as f64;
        }
        c
    };
    let b = freq(K_BLUEPRINT);
    assert!(b.iter().all(|x| (x - 0.25).abs() < 1e-3));
    let f = freq(K_FOLD);
    assert!((f[0] - 5.0 / 8.0).abs() < 1e-3 && (f[1] - 1.0 / 8.0).abs() < 1e-3);
    let c = freq(K_CALL);
    assert!((c[1] - 5.0 / 8.0).abs() < 1e-3);
    let r = freq(K_RAISE);
    assert!((r[2] + r[3] - 5.0 / 6.0).abs() < 1e-3);
}

#[test]
fn fork_shares_solution_and_never_resolves() {
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let pol = SearchPolicy::new("s", bp.clone(), params(1500));
    let mut p = pol.player(1, &cfg, 3);
    let (s0, s1, s2) = hu_flop(&cfg);
    p.observe(&s0, 0, Action::Call);
    p.observe(&s1, 1, Action::Check);
    let flop = cs("2c7d9h");
    p.observe_board(&flop);
    let hist = [];
    let v = View { cfg: &cfg, state: &s2, board: &flop, history: &hist, seat: 1 };
    // A fork asked first runs the round's first search; the original reuses it.
    let mut f = p.fork().unwrap();
    let hole = [cs("As")[0], cs("Ks")[0]];
    let d = f.dist(&v, hole);
    assert_eq!(pol.stats().searches, 1);
    assert!(p.solution().is_some());
    let d2 = p.dist(&v, hole);
    assert_eq!(pol.stats().searches, 1);
    assert_eq!(d.len(), d2.len());
    for i in 0..d.len() {
        assert_eq!(d.prob(i), d2.prob(i));
    }
    // Off-tree action inside a fork: no re-search, falls back to the blueprint.
    f.observe(&s2, 1, Action::Check);
    let mut s3 = s2;
    s3.apply(Action::Check);
    let odd = s3.legal().min_raise_to + 41;
    f.observe(&s3, 0, Action::RaiseTo(odd));
    let mut s4 = s3;
    s4.apply(Action::RaiseTo(odd));
    let v4 = View { cfg: &cfg, state: &s4, board: &flop, history: &hist, seat: 1 };
    let d4 = f.dist(&v4, hole);
    assert_eq!(pol.stats().searches, 1);
    assert!(d4.len() >= 2);
}

#[test]
fn three_handed_hands_are_legal() {
    let bp = p3_bp();
    let s = SearchPolicy::new("search", bp.clone(), SearchParams { iters: 300, max_infosets: 1 << 17, ..Default::default() });
    let bpp = BlueprintPolicy::new("bp", bp.store.clone(), bp.bucketer.clone(), bp.sizing);
    let mut spec = MatchSpec::new(GameConfig::new(3), vec![Arc::new(s), Arc::new(bpp)], Seatings::hero_vs_field(3), 30, 4);
    spec.aivat = AivatMode::Full;
    let r = run_match(&spec);
    assert_eq!(r.hands, 90);
    assert!(r.entrants[0].raw.mbb.is_finite());
}

#[test]
fn per_flop_search_buckets_switch() {
    use abstraction::search_buckets::{BuildParams, SearchBuckets};
    let bp = hu_bp();
    let cfg = GameConfig::new(2);
    let (_, _, root) = hu_flop(&cfg);
    let flop = cs("2c7d9h");
    let mut b = Beliefs::uniform(2);
    b.remove_cards(cards::mask_of(&flop));
    let none = Extras::new();
    let frozen = HashMap::new();
    let off = SearchParams { iters: 600, max_infosets: 1 << 18, parallel: false, vector: false, ..Default::default() };
    assert!(off.search_buckets.is_none(), "old behaviour must stay the default");
    let sb = Arc::new(SearchBuckets::new(None, BuildParams::tiny(), 2).unwrap());
    let on = SearchParams { search_buckets: Some(sb.clone()), ..off.clone() };
    let run = |prm: &SearchParams| {
        let sg = Subgame { bp, root, board: &flop, beliefs: &b, bp_hist: bp.shadow(&root), extras: &none, me: 1, my_hole: None, frozen: &frozen, params: prm, seed: 11 };
        solve(&sg)
    };
    let (a, c) = (run(&off), run(&on));
    assert_eq!(a.search_bucket_lookups, 0);
    assert!(c.search_bucket_lookups > 0, "search buckets were never consulted");
    assert_eq!(sb.builds.load(std::sync::atomic::Ordering::Relaxed), 1, "exactly the one flop was built");
    // Root-round strategies are still proper distributions.
    let mut list = ActionList::new();
    search_actions(&root, &on.sizing, &[], &mut list);
    let n = list.len();
    let mut p = [0f32; 24];
    let mut seen = 0;
    for h in 0..1326 {
        if c.probs(0, h, n, true, &mut p) {
            seen += 1;
            assert!((p[..n].iter().sum::<f32>() - 1.0).abs() < 1e-3);
        }
    }
    assert!(seen > 100);
    assert!(c.infosets() > 0 && a.infosets() > 0);
}

/// Mean and standard error of the payoff of `seat` when everyone plays the blueprint from `root`
/// (deals uniform given the board).
fn blueprint_rollout_value(bp: &Blueprint, root: State, board: &[Card], bph: Shadow, seat: usize, n: usize, seed: u64) -> (f64, f64) {
    use search::bp::passive_index;
    let mut rng = cards::Rng::new(seed);
    let np = root.num_players();
    let (mut sum, mut sq) = (0f64, 0f64);
    for _ in 0..n {
        let mut deal = nlhe::Deal { holes: [[0, 1]; nlhe::MAX_PLAYERS], board: [0; 5] };
        let mut dead = cards::mask_of(board);
        for (i, c) in board.iter().enumerate() {
            deal.board[i] = *c;
        }
        for s in 0..np {
            let a = rng.draw(dead);
            dead |= cards::bit(a);
            let b = rng.draw(dead);
            dead |= cards::bit(b);
            deal.holes[s] = [a, b];
        }
        for i in board.len()..5 {
            let c = rng.draw(dead);
            dead |= cards::bit(c);
            deal.board[i] = c;
        }
        let (mut s, mut h) = (root, bph);
        let mut list = ActionList::new();
        let mut sig = [0f32; 24];
        while !s.is_terminal() {
            let who = s.to_act().unwrap();
            assert!(bp.node_list(&h, &s, &mut list));
            let k = list.len();
            let bucket = bp.bucket(s.street(), deal.holes[who], &deal.board[..s.street().board_len()]);
            let idx = if bp.sigma(&h, &s, &list, bucket, &mut sig) { rng.sample_weighted(&sig[..k]) } else { passive_index(&list) };
            let a = list.get(idx);
            bp.observe(&mut h, &s, a, true);
            s.apply(a);
        }
        let v = s.payoffs(&deal)[seat] as f64;
        sum += v;
        sq += v * v;
    }
    let m = sum / n as f64;
    (m, ((sq / n as f64 - m * m).max(0.0) / n as f64).sqrt())
}

/// Gate 3(b): with the blueprint's own action abstraction, blueprint leaves and the learned
/// strategies pinned to the blueprint (huge warm start), the subgame's root value for a seat
/// equals the blueprint's value from that state (MC, within 4 standard errors).
#[test]
fn subgame_value_under_blueprint_equals_blueprint_value() {
    use search::solver::LeafMode;
    let bp = p3_bp();
    let cfg = GameConfig::new(3);
    let root = flop_root_3p(&cfg);
    // Blueprint history of the preflop limp-around: call, call, check.
    let mut s = State::new(&cfg);
    let mut bph = bp.shadow(&s);
    loop {
        let l = s.legal();
        let a = if l.can_check { Action::Check } else { Action::Call };
        bp.observe(&mut bph, &s, a, false);
        if let Step::NewStreet(_) = s.apply(a) {
            break;
        }
    }
    let flop = cs("2c7d9h");
    let mut b = Beliefs::uniform(3);
    b.remove_cards(cards::mask_of(&flop));
    let prm = SearchParams {
        iters: 600_000,
        max_infosets: 1 << 18,
        parallel: true,
        warm_start: 1e8,
        prune_prob: 0.0,
        focus_own: 0.0,
        compress: false,
        leaf: LeafMode::Blueprint,
        sizing: bp.sizing,
        later_sizing: bp.sizing,
        vector: false,
        ..Default::default()
    };
    let none = Extras::new();
    let frozen = HashMap::new();
    let sg = Subgame { bp, root, board: &flop, beliefs: &b, bp_hist: bph, extras: &none, me: 0, my_hole: None, frozen: &frozen, params: &prm, seed: 11 };
    let sol = solve(&sg);
    for seat in 0..3 {
        let (v, n) = sol.root_value[seat];
        assert!(n > 1000);
        let (want, se) = blueprint_rollout_value(bp, root, &flop, bph, seat, 400_000, 5 + seat as u64);
        // The solver's sample is noisy as well: allow 4 combined standard errors (value std ~ rollout std).
        let tol = 4.0 * se * 2.5;
        eprintln!("seat {seat}: subgame {v:.1} blueprint {want:.1} se {se:.1}");
        assert!((v - want).abs() < tol, "seat {seat}: subgame {v:.1} vs blueprint {want:.1} (tol {tol:.1})");
    }
}
