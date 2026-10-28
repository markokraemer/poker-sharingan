use cards::Rng;
use nlhe::abstraction::*;
use nlhe::*;
use std::hint::black_box;
use std::time::Instant;

#[test]
fn bench_apply_and_abstract_actions() {
    let sz = BetSizing::pluribus_blueprint();
    let mut rng = Rng::new(5);
    // Collect a pool of in-hand states with their chosen abstract actions.
    let mut pool: Vec<(State, Action)> = Vec::new();
    let mut list = ActionList::new();
    while pool.len() < 100_000 {
        let n = 2 + rng.below(5) as usize;
        let mut s = State::new(&GameConfig::new(n));
        while !s.is_terminal() {
            abstract_actions(&s, &sz, &mut list);
            let a = list.get(rng.below(list.len() as u32) as usize);
            pool.push((s, a));
            s.apply(a);
        }
    }
    let reps = 50;
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..reps {
        for (s, _) in &pool {
            abstract_actions(black_box(s), &sz, &mut list);
            acc += list.len();
        }
    }
    let ns_aa = t.elapsed().as_nanos() as f64 / (reps * pool.len()) as f64;
    let t = Instant::now();
    for _ in 0..reps {
        for (s, a) in &pool {
            let mut c = *s;
            black_box(c.apply(black_box(*a)));
            acc += c.pot() as usize;
        }
    }
    let ns_ap = t.elapsed().as_nanos() as f64 / (reps * pool.len()) as f64;
    let t = Instant::now();
    for _ in 0..1_000_000 {
        acc += State::new(&GameConfig::new(black_box(6))).pot() as usize;
    }
    let ns_new = t.elapsed().as_nanos() as f64 / 1e6;
    println!(
        "bench: apply {ns_ap:.1} ns (copy+apply), abstract_actions {ns_aa:.1} ns, State::new {ns_new:.1} ns, size_of State = {} B, ActionList = {} B (acc {acc})",
        std::mem::size_of::<State>(),
        std::mem::size_of::<ActionList>()
    );
}
