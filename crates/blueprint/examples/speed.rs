//! Throughput probe: 6-max MCCFR-P iterations/sec with a cheap bucketer.
use blueprint::trainer::*;
use cards::Card;
use nlhe::abstraction::BetSizing;
use nlhe::*;
use std::sync::atomic::Ordering::Relaxed;
struct Coarse;
impl Bucketer for Coarse {
    fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        if street == Street::Preflop { return cards::preflop_class(hole[0], hole[1]) as u32; }
        let m = cards::mask_of(board) | cards::bit(hole[0]) | cards::bit(hole[1]);
        cards::category(cards::evaluate(m)) as u32
    }
    fn n_buckets(&self, street: Street) -> u32 { if street == Street::Preflop { 169 } else { 9 } }
}
fn main() {
    let sizing = match std::env::args().nth(1).as_deref() { Some("paper") => BetSizing::pluribus_blueprint(), _ => BetSizing::tiny() };
    let iters: u64 = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(200_000);
    let tr = Trainer::new(GameConfig::new(6), sizing, Schedule::paper(1_000_000), &Coarse, 1 << 26, 7);
    tr.run(0, iters, |tr, t, secs| {
        if t % 40960 < 4096 {
            eprintln!("t={t} {:.0} it/s nodes/s {:.2}M sequences {} slots {:.1}M",
                t as f64 / secs, tr.nodes.load(Relaxed) as f64 / secs / 1e6, tr.store.sequences(), tr.store.used_slots() as f64 / 1e6);
        }
    });
}
