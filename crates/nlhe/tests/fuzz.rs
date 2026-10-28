use cards::Rng;
use nlhe::*;

fn random_action(s: &State, rng: &mut Rng) -> Action {
    let l = s.legal();
    let r = rng.below(100);
    if l.can_raise && r < 40 {
        let (lo, hi) = (l.min_raise_to, l.max_raise_to);
        return Action::RaiseTo(match rng.below(4) {
            0 => lo,
            1 => hi,
            2 => lo + rng.below((hi - lo + 1) as u32) as i32,
            _ => (lo + (s.pot() / 2)).clamp(lo, hi),
        });
    }
    if l.can_fold && r < 55 {
        return Action::Fold;
    }
    if l.can_check {
        Action::Check
    } else {
        Action::Call
    }
}

fn run(hands: u64, seed: u64) {
    let mut rng = Rng::new(seed);
    let (mut showdowns, mut folds, mut allin_runs) = (0u64, 0u64, 0u64);
    for _ in 0..hands {
        let n = 2 + rng.below(5) as usize;
        let mut stacks = [0i32; 6];
        for st in stacks.iter_mut().take(n) {
            *st = match rng.below(3) {
                0 => 10_000,
                1 => 1 + rng.below(3_000) as i32,
                _ => 100 + rng.below(30_000) as i32,
            };
        }
        let cfg = GameConfig::new(n).with_stacks(&stacks);
        let mut s = State::new(&cfg);
        let mut street = Street::Preflop;
        let mut acted_after_fold = [false; 6];
        let mut steps = 0;
        let mut step = if s.is_terminal() {
            Step::Terminal
        } else {
            Step::Continue
        };
        while !s.is_terminal() {
            steps += 1;
            assert!(steps < 500, "no termination");
            let p = s.to_act().unwrap();
            assert!(!s.is_folded(p) && !s.is_all_in(p), "dead player acts");
            assert!(!acted_after_fold[p]);
            let before: i32 = (0..n).map(|i| s.stack(i) + s.contributed(i)).sum();
            let a = random_action(&s, &mut rng);
            if a == Action::Fold {
                acted_after_fold[p] = true;
            }
            step = s.apply(a);
            let after: i32 = (0..n).map(|i| s.stack(i) + s.contributed(i)).sum();
            assert_eq!(before, after, "chip conservation");
            assert_eq!(s.pot(), (0..n).map(|i| s.contributed(i)).sum::<i32>());
            for i in 0..n {
                assert!(s.stack(i) >= 0);
                assert_eq!(s.is_all_in(i), s.stack(i) == 0);
            }
            match step {
                Step::NewStreet(ns) => {
                    assert_eq!(ns.idx(), street.idx() + 1, "street sequencing");
                    assert_eq!(s.street(), ns);
                    assert!(s.to_act().is_some());
                    assert!(s.active_count() >= 2);
                    street = ns;
                }
                Step::Continue => assert_eq!(s.street(), street),
                Step::Terminal => assert!(s.is_terminal() && s.to_act().is_none()),
            }
        }
        assert_eq!(step, Step::Terminal);
        if s.showdown() {
            showdowns += 1;
            assert_eq!(s.street(), Street::River);
            if s.active_count() < 2 {
                allin_runs += 1;
            }
        } else {
            folds += 1;
        }
        let deal = Deal::random(&mut rng);
        let pay = s.payoffs(&deal);
        assert_eq!(pay.iter().sum::<i32>(), 0, "zero sum");
        for i in 0..6 {
            if i >= n {
                assert_eq!(pay[i], 0);
                continue;
            }
            assert!(pay[i] >= -s.contributed(i), "lost more than committed");
            assert!(s.contributed(i) <= stacks[i]);
            if s.is_folded(i) {
                assert_eq!(pay[i], -s.contributed(i));
            } else {
                let cap: i32 = (0..n).map(|j| s.contributed(j).min(s.contributed(i))).sum();
                assert!(pay[i] + s.contributed(i) <= cap, "won more than eligible");
            }
        }
    }
    println!(
        "fuzz: {hands} hands, {showdowns} showdowns ({allin_runs} run-outs), {folds} fold-outs"
    );
}

#[test]
fn fuzz_one_million_hands() {
    let hands = if cfg!(debug_assertions) {
        200_000
    } else {
        1_000_000
    };
    run(hands, 0xC0FFEE);
}
