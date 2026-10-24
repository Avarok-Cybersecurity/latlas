// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: Latlas in the browser (DESIGN-latlas.md §6 K4): the declarative pack's derive,
//! status, graph-only search and score over files the page supplies (OPFS, a picked directory or a
//! fetched tarball), as pure functions of JSON requests. The page owns storage and persistence
//! (`navigator.storage.persist()`); this module owns no I/O and no clock (SBIO).
//!
//! Owner: latlas-web.
//! Invariants:
//! - Built for `wasm32-unknown-unknown` with `std` (a host crate; the core it calls is `no_std`).
//! - HPAR: the same request gives the same bundle digest here and in the native `latlas` binary
//!   (gate/hpar.mjs checks it against `latlas derive`).
//! - The repository root is the virtual `/repo`; `root = "."` resolves to it through the core's
//!   `resolve_repo_paths`, the same function the native host calls.

extern crate alloc;

mod abi;
pub mod api;

pub use api::handle;
