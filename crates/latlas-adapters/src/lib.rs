// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The built-in adapters (DESIGN-latlas-ontology.md §4.2): pure functions from a
//! source text to records in a [`latlas_core::Builder`]. Today the `claude-code` pack: Claude
//! Code memory files ([`memory`]) and CLAUDE.md principle tables ([`principles`]).
//!
//! Owner: latlas-adapters.
//! Invariants: pure (SBIO), `no_std` + `alloc`, `forbid(unsafe_code)`; adapters return findings
//! through the builder and never read config fields themselves.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod memory;
pub mod principles;
