//! Live learning curve: periodically play the *current* blueprint (read concurrently from the
//! trainer's store) against the gauntlet in a background thread with its own small thread
//! pool, scored by duplicate deals + AIVAT. One JSON line per (checkpoint, opponent) goes to
//! `eval.jsonl`, so every hill-climbing change is judged on the same numbers.

use bench::bp::{Bias, Biased, BlueprintPolicy, TableBucketer};
use bench::builtin::builtin;
use bench::matchrun::{run_match, AivatMode, MatchSpec, Seatings};
use bench::policy::PolicyFactory;
use blueprint::store::Store;
use nlhe::abstraction::BetSizing;
use nlhe::GameConfig;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

pub struct LiveEval {
    pub store: Arc<Store>,
    pub bucketer: Arc<TableBucketer>,
    pub sizing: BetSizing,
    pub game: GameConfig,
    pub out: PathBuf,
    pub deals: u64,
    pub threads: usize,
}

impl LiveEval {
    /// Run one evaluation round for iteration `t` (blocking; call from a background thread).
    pub fn run(&self, t: u64) {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(self.threads).build().unwrap();
        let hero = BlueprintPolicy::new("blueprint", self.store.clone(), self.bucketer.clone(), self.sizing.clone());
        let hero: Arc<dyn PolicyFactory> = Arc::new(hero);
        let mut field: Vec<Arc<dyn PolicyFactory>> = ["tag", "lag", "nit", "station", "maniac"].iter().map(|n| builtin(n).unwrap()).collect();
        for b in [Bias::Fold, Bias::Call, Bias::Raise] {
            field.push(Arc::new(Biased::new(hero.clone(), b)));
        }
        for opp in field {
            let mut spec = MatchSpec::new(self.game, vec![hero.clone(), opp.clone()], Seatings::hero_vs_field(self.game.players), self.deals, 0xE7A1 ^ t);
            spec.aivat = AivatMode::Full;
            let rep = pool.install(|| run_match(&spec));
            let h = &rep.entrants[0];
            let est = h.aivat.as_ref().unwrap_or(&h.raw);
            let (lo, hi) = est.ci95();
            let line = serde_json::json!({
                "t": t, "opponent": opp.name(), "hands": rep.hands,
                "mbb": est.mbb, "ci_lo": lo, "ci_hi": hi, "raw_mbb": h.raw.mbb,
                "var_ratio": h.var_ratio_deal,
                "unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            });
            eprintln!("  eval it {t} vs {}: {:+.0} mbb/game [{:+.0}, {:+.0}]", opp.name(), est.mbb, lo, hi);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.out) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}
