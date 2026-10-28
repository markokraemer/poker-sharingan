use cards::Rng;
use nlhe::abstraction::*;
use nlhe::tree::*;
use nlhe::*;
use std::time::{Duration, Instant};

fn check_list(s: &State, sz: &BetSizing, list: &ActionList) {
    let l = s.legal();
    let a = list.as_slice();
    let mut i = 0;
    if l.can_fold {
        assert_eq!(a[0], Action::Fold);
        i = 1;
    }
    assert_eq!(
        a[i],
        if l.can_check {
            Action::Check
        } else {
            Action::Call
        }
    );
    let raises: Vec<i32> = a[i + 1..]
        .iter()
        .map(|x| match x {
            Action::RaiseTo(t) => *t,
            _ => panic!("non-raise after call"),
        })
        .collect();
    for w in raises.windows(2) {
        assert!(w[0] < w[1], "ascending and deduped");
    }
    for &t in &raises {
        assert!(l.can_raise && t >= l.min_raise_to && t <= l.max_raise_to);
    }
    let menu = sz.menu(s.street().idx(), s.raises_this_street() as usize);
    if let Some(&last) = raises.last() {
        if menu.all_in {
            assert_eq!(last, l.max_raise_to, "all-in last");
        }
    }
    if s.raises_this_street() >= sz.max_raises[s.street().idx()] as u32 {
        assert!(raises.is_empty());
    }
}

fn walk(sz: &BetSizing, hands: u64, seed: u64) {
    let mut rng = Rng::new(seed);
    let mut list = ActionList::new();
    for _ in 0..hands {
        let n = 2 + rng.below(5) as usize;
        let mut stacks = [0i32; 6];
        for st in stacks.iter_mut().take(n) {
            *st = 200 + rng.below(20_000) as i32;
        }
        let mut s = State::new(&GameConfig::new(n).with_stacks(&stacks));
        let mut h = History::new();
        while !s.is_terminal() {
            abstract_actions(&s, sz, &mut list);
            check_list(&s, sz, &list);
            let i = rng.below(list.len() as u32) as usize;
            h.push(i);
            s.apply(list.get(i));
        }
        assert!(h.len() > 0);
    }
}

#[test]
fn abstract_actions_are_legal() {
    walk(&BetSizing::pluribus_blueprint(), 100_000, 1);
    walk(&BetSizing::tiny(), 100_000, 2);
}

#[test]
fn opening_menus() {
    let s = State::new(&GameConfig::new(6));
    let mut l = ActionList::new();
    abstract_actions(&s, &BetSizing::pluribus_blueprint(), &mut l);
    // fold, call, 13 fractions + all-in = 16
    println!("preflop open menu: {:?}", l.as_slice());
    assert_eq!(l.len(), 16);
    assert_eq!(l.get(0), Action::Fold);
    assert_eq!(l.get(1), Action::Call);
    assert_eq!(l.get(15), Action::RaiseTo(10_000));
    // 1x pot: pot 150 + call 100 = 250 -> 100 + 250
    abstract_actions(&s, &BetSizing::tiny(), &mut l);
    assert_eq!(
        l.as_slice(),
        &[
            Action::Fold,
            Action::Call,
            Action::RaiseTo(350),
            Action::RaiseTo(10_000)
        ]
    );
}

#[test]
fn clamps_and_dedupes_on_short_stacks() {
    // effective stack barely above a min-raise: all sizes collapse to the all-in
    let s = State::new(&GameConfig::new(2).with_stacks(&[210, 10_000]));
    let mut l = ActionList::new();
    abstract_actions(&s, &BetSizing::pluribus_blueprint(), &mut l);
    assert_eq!(
        l.as_slice(),
        &[
            Action::Fold,
            Action::Call,
            Action::RaiseTo(200),
            Action::RaiseTo(210)
        ]
    );
}

#[test]
fn history_hash_distinguishes() {
    let mut a = History::new();
    let mut b = History::new();
    for i in [1, 2, 3] {
        a.push(i);
    }
    for i in [1, 3, 2] {
        b.push(i);
    }
    assert_ne!(a.key(), b.key());
    let mut c = a;
    c.push(0);
    assert_ne!(c.key(), a.key());
    assert_eq!(a.indices(), &[1, 2, 3]);
}

#[test]
fn translation() {
    // x == A -> 1, x == B -> 0, monotone decreasing in x
    assert!((pseudo_harmonic_lower(0.5, 0.5, 1.0) - 1.0).abs() < 1e-12);
    assert!(pseudo_harmonic_lower(1.0, 0.5, 1.0).abs() < 1e-12);
    // paper's example: A=0.5, B=1, x=0.75 -> 0.4286 (~ 3/7)
    assert!((pseudo_harmonic_lower(0.75, 0.5, 1.0) - 3.0 / 7.0).abs() < 1e-12);

    let s = State::new(&GameConfig::new(6));
    let mut l = ActionList::new();
    abstract_actions(&s, &BetSizing::tiny(), &mut l); // fold, call, 350, 10000
    assert_eq!(translate_deterministic(&s, &l, 350), 2);
    assert_eq!(translate_deterministic(&s, &l, 200), 2); // small raise is nearer 1x than call
    assert_eq!(translate_deterministic(&s, &l, 20_000), 3);
    assert_eq!(translate_deterministic(&s, &l, 9_000), 3);
    let mut rng = Rng::new(7);
    let (mut lo, trials) = (0, 20_000);
    // x between 1x pot and all-in: lower-size frequency matches p_lo
    let to = 1500;
    let Bracket::Between { p_lo, .. } = bracket(&s, &l, to) else {
        panic!()
    };
    for _ in 0..trials {
        if translate_random(&s, &l, to, &mut rng) == 2 {
            lo += 1;
        }
    }
    assert!((lo as f64 / trials as f64 - p_lo).abs() < 0.02);
}

#[test]
fn tree_sizes() {
    let paper = 664_845_654u64;
    for (name, sz) in [
        ("tiny", BetSizing::tiny()),
        ("pluribus_blueprint", BetSizing::pluribus_blueprint()),
    ] {
        for n in [2usize, 6] {
            let cfg = GameConfig::new(n);
            let t = Instant::now();
            let pre = count_tree(
                &cfg,
                &sz,
                Street::Preflop,
                Some(Instant::now() + Duration::from_secs(30)),
            );
            println!(
                "tree[{name}, {n}p] preflop: {} decision nodes, {} leaves, complete={} ({:.1?})",
                pre.decisions,
                pre.leaves,
                pre.complete,
                t.elapsed()
            );
            let t = Instant::now();
            let full = count_tree(
                &cfg,
                &sz,
                Street::River,
                Some(Instant::now() + Duration::from_secs(90)),
            );
            if full.complete {
                println!(
                    "tree[{name}, {n}p] FULL exact: {} decision nodes, {} leaves ({:.1?}); paper/ours = {:.2}",
                    full.decisions,
                    full.leaves,
                    t.elapsed(),
                    paper as f64 / full.decisions as f64
                );
            } else {
                let mut rng = Rng::new(99);
                let est = estimate_tree(&cfg, &sz, Street::River, 2_000_000, &mut rng);
                println!(
                    "tree[{name}, {n}p] FULL exact timed out after {} nodes; Knuth estimate {:.3e} decision nodes (paper {:.3e})",
                    full.decisions, est, paper as f64
                );
            }
        }
    }
}
