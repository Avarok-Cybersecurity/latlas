// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The file port `la_record` writes through (DESIGN-latlas.md §4): a pack builds the
//! new file text and runs its validators; the host reads and replaces. The core never touches a
//! file.
//!
//! Owner: latlas-core (cut from metrale-kg-derive `record_flow`, 2026-10-23 K0).
//! Invariants: `replace` is atomic and refuses when the file no longer holds `expected`.

use alloc::string::String;
use latlas_error::Result;

/// 2026-10-23: The file port (a host implements it).
#[expect(
    async_fn_in_trait,
    reason = "2026-10-23: as latlas-ports: the host's executor drives the future (ACNB)"
)]
pub trait RecordFs {
    /// 2026-10-23: The file's text, `None` when it does not exist.
    async fn read(&self, path: &str) -> Result<Option<String>>;
    /// 2026-10-23: Replace the file atomically if it still holds `expected`, else refuse.
    async fn replace(&self, path: &str, expected: &str, new: &str) -> Result<()>;
}
