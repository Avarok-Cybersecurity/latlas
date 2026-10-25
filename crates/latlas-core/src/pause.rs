// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The pause: while the host's [`PauseSignal`] is present the process holds no
//! embedder (the warmer sends nothing, a running embedder process is stopped, none is spawned) and
//! a search answers `graph_only` with [`PauseGate::reason`]. Records are stored as always; their
//! texts wait in the cache's to-do set until the signal is gone.
//!
//! Owner: latlas-core.
//! Invariants:
//! - Pure (SBIO): the signal is read through the port and time through the Clock port; the host
//!   waits for [`PauseGate::step`]'s next-due instant (ACNB).
//! - The signal is read once per step; an edge is reported once, when the state changes.
//! - The signal name and the poll interval are the declared constants below (PCND).

use crate::prelude::*;
use latlas_ports::{Clock, DurNs, MonoNs, PauseSignal, UnixMs};
use serde_json::{Map, Value};

/// 2026-10-25: The file a native host looks for beside its executable (a dangling symlink of
/// this name is absent).
pub const PAUSE_FILE: &str = "latlas.lock";

/// 2026-10-25: The interval between two reads of the signal.
pub const PAUSE_POLL_MS: u64 = 5_000;

/// 2026-10-25: A change of the pause state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Paused,
    Resumed,
}

/// 2026-10-25: The pause state: `Some(wall time it began)` while paused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PauseGate {
    since: Option<UnixMs>,
}

impl PauseGate {
    /// 2026-10-25: Read the signal once; the edge it caused, and when the next read is due.
    pub fn step<S: PauseSignal, C: Clock>(
        &mut self,
        signal: &S,
        clock: &C,
    ) -> (Option<Edge>, MonoNs) {
        let present = signal.present();
        let edge = match (self.since.is_some(), present) {
            (false, true) => {
                self.since = Some(clock.now_unix_ms());
                Some(Edge::Paused)
            }
            (true, false) => {
                self.since = None;
                Some(Edge::Resumed)
            }
            _ => None,
        };
        let next = clock.monotonic_ns().plus(DurNs::from_ms(PAUSE_POLL_MS));
        (edge, next)
    }

    /// 2026-10-25: Whether the process is paused.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.since.is_some()
    }

    /// 2026-10-25: The declared reason a search answers `graph_only` with while paused.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        self.since.map(|_| format!("paused: {PAUSE_FILE} present"))
    }

    /// 2026-10-25: The `la_status` view; `signal` is where the host looks (e.g. the file path).
    #[must_use]
    pub fn status(&self, signal: &str) -> Value {
        let mut o = Map::new();
        o.insert("paused".into(), Value::Bool(self.paused()));
        o.insert("signal".into(), Value::String(signal.to_string()));
        o.insert("poll_ms".into(), Value::from(PAUSE_POLL_MS));
        o.insert(
            "paused_since_ms".into(),
            self.since.map_or(Value::Null, |t| Value::from(t.0)),
        );
        o.insert(
            "reason".into(),
            self.reason().map_or(Value::Null, Value::String),
        );
        Value::Object(o)
    }
}

#[cfg(test)]
#[path = "pause_tests.rs"]
mod tests;
