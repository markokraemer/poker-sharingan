//! `CompactPolicy`: a [`PolicyFactory`] over a `blueprint::compact::Compact` file (mmap).
//!
//! It *is* a [`BlueprintPolicy`] (same abstract-history tracking, pseudo-harmonic translation
//! and check/call fallback for unknown infosets) whose lookups go to the compact file instead
//! of a `Store`. Probabilities are the file's u8 quantisation (error < 1/255 per action).

use crate::bp::{BlueprintPolicy, Source};
use crate::policy::{Player, PolicyFactory};
use blueprint::compact::Compact;
use blueprint::trainer::Bucketer;
use nlhe::abstraction::BetSizing;
use nlhe::GameConfig;
use std::path::Path;
use std::sync::Arc;

impl Source for Compact {
    #[inline]
    fn probs(&self, hist: u64, bucket: u32, out: &mut [f32]) -> bool {
        Compact::probs(self, hist, bucket, out)
    }
}

pub struct CompactPolicy {
    inner: BlueprintPolicy,
    compact: Arc<Compact>,
}

impl CompactPolicy {
    pub fn new(name: &str, compact: Arc<Compact>, bucketer: Arc<dyn Bucketer + Send>, sizing: BetSizing) -> Self {
        CompactPolicy { inner: BlueprintPolicy::from_source(name, compact.clone(), bucketer, sizing), compact }
    }

    pub fn open(path: &Path, bucketer: Arc<dyn Bucketer + Send>, sizing: BetSizing) -> Result<Self, String> {
        let c = Compact::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::new(&format!("compact:{}", path.display()), Arc::new(c), bucketer, sizing))
    }

    pub fn with_opts(self, o: nlhe::abstraction::TrackOpts) -> Self {
        let CompactPolicy { inner, compact } = self;
        CompactPolicy { inner: inner.with_opts(o), compact }
    }
    pub fn with_translation(self, t: nlhe::abstraction::Translation) -> Self {
        let CompactPolicy { inner, compact } = self;
        CompactPolicy { inner: inner.with_translation(t), compact }
    }
    pub fn with_below(self, b: nlhe::abstraction::BelowMenu) -> Self {
        let CompactPolicy { inner, compact } = self;
        CompactPolicy { inner: inner.with_below(b), compact }
    }
    pub fn inner(&self) -> &BlueprintPolicy {
        &self.inner
    }

    pub fn compact(&self) -> &Arc<Compact> {
        &self.compact
    }

    /// Fraction of decisions whose infoset was missing from the file (fallback: check/call).
    pub fn miss_rate(&self) -> f64 {
        self.inner.miss_rate()
    }
}

impl PolicyFactory for CompactPolicy {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn report(&self) -> Option<String> {
        self.inner.report()
    }
    fn new_player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> Box<dyn Player> {
        self.inner.new_player(seat, cfg, seed)
    }
}
