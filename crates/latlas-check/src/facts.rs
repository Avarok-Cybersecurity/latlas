// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: What only the host can supply to the score: the last commit of each path, the
//! commits of the window, and the CI gate results. Each is `None` when the host did not supply it,
//! and the dimensions that need it say so (never a guessed value).
//!
//! Owner: latlas-check.
//! Invariants: paths are root-relative, as in the bundle's sources.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// 2026-10-24: One commit of the window and the root-relative paths it touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowCommit {
    pub sha: String,
    pub paths: Vec<String>,
}

/// 2026-10-24: The host's facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckFacts {
    /// 2026-10-24: path → (last commit sha, its commit time in unix seconds).
    pub last_commit: Option<BTreeMap<String, (String, i64)>>,
    /// 2026-10-24: The commits of the last `[window] days`, newest first.
    pub window: Option<Vec<WindowCommit>>,
    /// 2026-10-24: gate key (`test/…`, without the `gate:` prefix) → passed.
    pub gate_results: Option<BTreeMap<String, bool>>,
}

impl CheckFacts {
    /// 2026-10-24: How many window commits touched `path`.
    pub fn touches(&self, path: &str) -> Option<usize> {
        let w = self.window.as_ref()?;
        Some(w.iter().filter(|c| c.paths.iter().any(|p| p == path)).count())
    }
}
