// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: Latlas's pure core (DESIGN-latlas.md §2.2): an already-read source set and a
//! config in, a deterministic `latlas-bundle/v1` out through one domain [`pack::Pack`], and the
//! closures, ranking and status sections computed as pure functions over that bundle. The graph
//! never holds a fact that is not in a file.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants:
//! - Pure (SBIO): no I/O, no environment, no clock. Bytes arrive as a [`sources::SourceSet`];
//!   hosts live in `latlas-host-native`, the binary chooses them.
//! - `no_std` + `alloc`, `forbid(unsafe_code)`; builds for wasm32 (NSCC).
//! - No trait objects (NDIC): the pack is a type parameter.
//! - Every fallible function returns the registry error (CERF, `latlas-error`).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![deny(
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

extern crate alloc;

pub mod prelude;

pub mod bundle;
pub mod config;
pub mod derive;
pub mod embed;
pub mod hash;
pub mod id;
pub mod jsonl;
pub mod kbp;
pub mod metframe;
pub mod metlocal;
pub mod pack;
pub mod queries;
pub mod record;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod sources;
pub mod status;
pub mod text;
pub mod tools;

pub use bundle::{Builder, Bundle, Edge, Node, Text};
pub use config::Config;
pub use derive::derive;
pub use id::{Label, NodeId, Rel, Vocab};
pub use pack::Pack;
pub use sources::SourceSet;
