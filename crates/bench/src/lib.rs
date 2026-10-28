//! Evaluation harness for the Pluribus reproduction.
//!
//! - [`policy`]: `Policy`/`Player` abstraction (per-hand stateful players built by a factory).
//! - [`builtin`]: the gauntlet opponents (Random, Station, Nit, TAG, LAG, Maniac).
//! - [`chart`]: `chart6`, a deterministic 6-max chart + pot-odds rule bot.
//! - [`ext`]: JSON-lines subprocess bridge for third-party (Python) bots.
//! - [`bp`]: `BlueprintPolicy` (store + bucketer + sizing, with action translation) and the
//!   biased continuation-strategy wrapper (fold x5 / call x5 / raise x5).
//! - [`matchrun`]: seeded, rayon-parallel duplicate matches with mbb/game statistics.
//! - [`aivat`]: AIVAT variance reduction with pluggable [`aivat::ValueEstimator`].
//! - [`lbr`]: Local Best Response (heads-up bound, 6-max profile variant).
//! - [`slumbot`]: heads-up Slumbot client/parser on the nlhe engine.
//!
//! Seeding rule: every random stream is seeded per item (deal index, hand, seat) via
//! [`mix`], never per rayon thread, so results are reproducible regardless of scheduling.

pub mod aivat;
pub mod bp;
pub mod builtin;
pub mod chart;
pub mod compact;
pub mod ext;
pub mod lbr;
pub mod matchrun;
pub mod policy;
pub mod probe;
pub mod slumbot;
pub mod stats;

/// Deterministic 64-bit mixer for deriving per-item seeds.
pub fn mix(a: u64, b: u64) -> u64 {
    let mut z = a ^ b
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
