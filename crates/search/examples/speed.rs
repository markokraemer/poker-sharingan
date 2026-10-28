//! Search speed probe: iterations/s and strategy summary for flop / turn / river roots.
//! cargo run --release -p search --example speed -- BP_CACHE PLAYERS ITERS [threads:0|1]
use search::beliefs::Beliefs;
use search::bp::Blueprint;
use search::solver::{solve, SearchParams, Subgame};
use search::testkit::best_bucketer;
use nlhe::abstraction::BetSizing;
use nlhe::{Action, GameConfig, State, Step, Street};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let players: usize = a[2].parse().unwrap();
    let iters: u64 = a[3].parse().unwrap();
    let par = a.get(4).map_or(true, |s| s != "1");
    let (bk, _) = best_bucketer();
    let store = Arc::new(blueprint::io::load(std::path::Path::new(&a[1]), 1 << 27, 1 << 24).unwrap());
    let bp = Blueprint::new(store, bk, BetSizing::tiny());
    let cfg = GameConfig::new(players);
    for street in [Street::Flop, Street::Turn, Street::River] {
        let mut s = State::new(&cfg);
        let mut guard = 0;
        // limp/check around to the street
        while s.street() < street {
            let l = s.legal();
            guard += 1;
            let act = if l.can_check { Action::Check } else { Action::Call };
            if let Step::Terminal = s.apply(act) {
                break;
            }
            assert!(guard < 100);
        }
        let board = cards::parse_cards(&"2c7d9hJsKc"[..2 * street.board_len()]).unwrap();
        let mut b = Beliefs::uniform(players);
        b.remove_cards(cards::mask_of(&board));
        let prm = SearchParams { iters, parallel: par, ..Default::default() };
        let none = HashMap::new();
        let fr = HashMap::new();
        let sg = Subgame { bp: &bp, root: s, board: &board, beliefs: &b, bp_hist: bp.shadow(&s), extras: &none, me: 0, my_hole: Some([0, 5]), frozen: &fr, params: &prm, seed: 3 };
        let t = Instant::now();
        let sol = solve(&sg);
        let el = t.elapsed().as_secs_f64();
        println!(
            "{:?} {}p limit {:?}: {} it in {:.2}s = {:.0} it/s, {:.0} nodes/it, {} infosets",
            street, players, sol.limit, sol.iters, el, sol.iters as f64 / el, sol.nodes as f64 / sol.iters as f64, sol.infosets()
        );
    }
}
