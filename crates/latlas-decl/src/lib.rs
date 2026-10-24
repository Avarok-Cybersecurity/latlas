// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The declarative pack (DESIGN-latlas.md §6 K3): a repository describes its records
//! in `.latlas/config.toml` and needs no pack code. Two adapters:
//! - `[[sources.tables]]`: one node per row of the markdown table under a named heading (the key
//!   column, the copied fields, whether rows bear outcomes);
//! - `[sources.rust]`: one `Crate` node per workspace member and one `Gate` per test function,
//!   with the obligation that every published crate has at least one test.
//!
//! Owner: latlas-decl.
//! Invariants:
//! - Pure (SBIO), `no_std` + `alloc`: the host lists and reads the crate files (`extra_files`).
//! - Every table is found or the derive reports `LatlasSource` (a renamed heading is never
//!   silently an empty table).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod pack;
pub mod rust;
pub mod table;

pub use pack::{Decl, DeclSources, RustSource, TableSource};
