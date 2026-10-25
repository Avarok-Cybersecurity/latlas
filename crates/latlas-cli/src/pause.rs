// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The pause as the process runs it: the gate (`latlas_core::pause`) over the native
//! lock file, the embedder host told on each edge, and [`watch`], the server's poll loop.
//!
//! Owner: latlas-cli.
//! Invariants:
//! - The gate is read by searches and the warmer without the app lock and without waiting.
//! - [`watch`] reads the signal every `PAUSE_POLL_MS` on the Timer port; on a resume it starts
//!   the warmer for whatever the cache lacks (texts recorded while paused).

use crate::app::App;
use crate::embedhost::EmbedHost;
use crate::host::PackHost;
use latlas_core::Pack;
use latlas_core::pause::{Edge, PauseGate};
use latlas_host_native::clock::{SystemClock, TokioTimer};
use latlas_host_native::pause::LockFile;
use latlas_ports::{Clock, MonoNs, Timer as _};
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

/// 2026-10-25: The pause gate and the signal it reads.
pub struct PauseCtl {
    gate: StdMutex<PauseGate>,
    signal: LockFile,
}

impl PauseCtl {
    /// 2026-10-25: Not paused until the first [`poll`](Self::poll).
    #[must_use]
    pub fn new(signal: LockFile) -> Self {
        Self {
            gate: StdMutex::new(PauseGate::default()),
            signal,
        }
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, PauseGate> {
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 2026-10-25: Whether the process is paused.
    pub fn paused(&self) -> bool {
        self.gate().paused()
    }

    /// 2026-10-25: The declared reason a search answers `graph_only` with, while paused.
    pub fn reason(&self) -> Option<String> {
        self.gate().reason()
    }

    /// 2026-10-25: The `la_status` view.
    pub fn status(&self) -> Value {
        self.gate()
            .status(&self.signal.path().display().to_string())
    }

    /// 2026-10-25: Read the signal once and tell `host` on an edge (a pause waits until its
    /// processes are stopped); the edge and when the next read is due.
    pub async fn poll<C: Clock>(
        &self,
        clock: &C,
        host: Option<&EmbedHost>,
    ) -> (Option<Edge>, MonoNs) {
        let (edge, next) = self.gate().step(&self.signal, clock);
        match (edge, host) {
            (Some(Edge::Paused), Some(h)) => h.suspend().await,
            (Some(Edge::Resumed), Some(h)) => h.resume(),
            _ => {}
        }
        (edge, next)
    }
}

/// 2026-10-25: The server's poll loop; runs for the life of the process.
pub async fn watch<P: Pack, H: PackHost<P>>(
    app: Arc<Mutex<App<P, H>>>,
    ctl: Arc<PauseCtl>,
    host: Option<EmbedHost>,
) {
    loop {
        let (edge, next) = ctl.poll(&SystemClock, host.as_ref()).await;
        if edge == Some(Edge::Resumed) {
            crate::search::resume_warming(&*app.lock().await).await;
        }
        TokioTimer.sleep_until(next).await;
    }
}
