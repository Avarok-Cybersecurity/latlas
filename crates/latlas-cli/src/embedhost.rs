// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The embedder hosts the binary can choose, as one enum of concrete hosts (NDIC: no
//! trait object). `app::open_embed` picks the variant from the resolved `[embedder]` mode.
//!
//! Owner: latlas-cli.
//! Invariants: each variant delegates the port unchanged; there is no fallback between them.

use latlas_core::embed::{Embedder, Mode, Reply, Role};
use latlas_error::Result;
use latlas_host_met::MetLocal;
use latlas_host_native::openrouter::OpenRouter;

/// 2026-10-24: The chosen embedder host. A clone shares the host's state (both are `Arc`s).
#[derive(Clone)]
pub enum EmbedHost {
    /// 2026-10-24: `mode = "http"`.
    Http(OpenRouter),
    /// 2026-10-24: `mode = "met-local"`.
    MetLocal(MetLocal),
}

impl Embedder for EmbedHost {
    fn mode(&self) -> Mode {
        match self {
            Self::Http(h) => h.mode(),
            Self::MetLocal(h) => h.mode(),
        }
    }

    async fn embed(&self, role: Role, inputs: &[String]) -> Result<Reply> {
        match self {
            Self::Http(h) => h.embed(role, inputs).await,
            Self::MetLocal(h) => h.embed(role, inputs).await,
        }
    }
}

impl EmbedHost {
    /// 2026-10-25: Hold no embedder process (`MetLocal::suspend`); the HTTP host holds none.
    pub async fn suspend(&self) {
        if let Self::MetLocal(h) = self {
            h.suspend().await;
        }
    }

    /// 2026-10-25: Undo [`suspend`](Self::suspend).
    pub fn resume(&self) {
        if let Self::MetLocal(h) = self {
            h.resume();
        }
    }
}
