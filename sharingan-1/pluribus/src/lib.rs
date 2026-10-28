//! Glue between the crates: abstraction presets and the table-backed bucketer.

pub mod h2h;
pub mod live_eval;

use abstraction::tables::BucketTables;
use blueprint::trainer::{Bucketer, Schedule};
use cards::Card;
use nlhe::abstraction::BetSizing;
use nlhe::Street;

pub struct TableBucketer(pub BucketTables);

impl Bucketer for TableBucketer {
    #[inline]
    fn bucket(&self, _street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        self.0.bucket(hole, board)
    }
    fn n_buckets(&self, street: Street) -> u32 {
        match street {
            Street::Preflop => 169,
            s => self.0.buckets[s.idx() - 1] as u32,
        }
    }
}

/// Bet-size presets (single source of truth: `bench::bp::sizing_by_name`). `paper` is our
/// proxy of the published abstraction (~0.5 TB); `medium` and `local` keep the paper's
/// turn/river menus exactly and trim preflop/flop; `tiny` is for smoke tests.
pub fn sizing(name: &str) -> BetSizing {
    bench::bp::sizing_by_name(name).unwrap_or_else(|| panic!("unknown sizing {name} (paper|medium|local|tiny)"))
}

/// The paper's schedule (8 days = 11,520 minutes) rescaled to a run of `total` iterations:
/// discount every 10/11520 of the run until 400/11520, prune after 200/11520, accumulate the
/// preflop average after 800/11520.
pub fn scaled_schedule(total: u64) -> Schedule {
    let per_min = (total / 11_520).max(1);
    Schedule::paper(per_min)
}
