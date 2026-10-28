//! Information abstraction for Pluribus.
//!
//! Paper: preflop lossless (169 classes); flop/turn/river lossy with 200 buckets each,
//! k-means on domain features (Johanson et al. 2013) for the blueprint; real-time search uses
//! 500 buckets per later round. Our features:
//! - flop, turn: distribution-aware histograms of final river equity, clustered with EMD
//!   (the paper's search abstraction is potential-aware EMD; that is a planned upgrade),
//! - river: OCHS — equity against 8 opponent preflop-hand clusters, L2 k-means.

pub mod equity;
pub mod kmeans;
pub mod search_buckets;
pub mod tables;
