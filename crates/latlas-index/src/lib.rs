// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The derived LatticeDB index of a bundle (DESIGN-latlas.md §2.2): the graph
//! collection ([`store`]), the content-hash embedding cache ([`textstore`]) and the read-only
//! Cypher gate ([`cypher`]). Rebuildable from the bundle: delete the directory, load the same
//! bundle, and the export is byte-identical.
//!
//! Owner: latlas-index (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants: every fallible function returns the registry error (CERF/CLER, `latlas-error`);
//! the vocabulary is a type parameter fixed at open (NDIC, ROTI).
#![forbid(unsafe_code)]

pub mod cypher;
pub mod store;
pub mod textstore;
