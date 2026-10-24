// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `latlas-check` (DESIGN-latlas-check.md, K2): the integrity score of a bundle as a
//! vector `[F, R, C, G, P, S, H, T, X, O]` (never a weighted sum), each dimension with a mode and
//! its worst contributors, and the ratchet that compares a head bundle with its base dimension by
//! dimension. A pure function of the bundles and the declared check config; the CLI and the
//! Action are hosts of it and print the same JSON.
//!
//! Owner: latlas-check.
//! Invariants:
//! - Pure (SBIO), `no_std` + `alloc`, `forbid(unsafe_code)`.
//! - An undefined dimension (empty denominator, or a part not computed yet) is `absent` with the
//!   reason, never 0.0 or 1.0 (PCND).
//! - Every weight and tolerance comes from the check config; none is defaulted.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod config;
pub mod dim_graph;
pub mod dim_host;
pub mod dim_x;
pub mod dims;
pub mod facts;
pub mod prose;
pub mod pack;
pub mod ratchet;
pub mod report;
pub mod weights;

pub use config::CheckConfig;
pub use dims::{Dim, DimResult, Mode, Score, score};
pub use facts::{CheckFacts, WindowCommit};
pub use pack::{Checks, Key, Obligation};
pub use ratchet::{Comparison, Status, compare};
