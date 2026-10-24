// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The `alloc` names a `std` crate's prelude would give, for the `no_std` build.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants: only re-exports of `alloc` items.

pub use alloc::collections::{BTreeMap, BTreeSet};
pub use alloc::format;
pub use alloc::string::{String, ToString};
pub use alloc::vec;
pub use alloc::vec::Vec;
