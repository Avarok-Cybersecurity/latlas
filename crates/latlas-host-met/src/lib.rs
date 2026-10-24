// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The `met-local` embedder host (DESIGN-latlas-local-embedder.md §4): `met` serves
//! the embedding model, either spawned per request as the `met embed` alias ([`alias`], over the
//! frames of `latlas_core::metframe`) or as a running `met serve` ([`resident`]). The only crate
//! that runs `met` or talks to its HTTP.
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - The mode is decided once, at [`MetLocal::open`]: [`Mode::Unavailable`] with the reason when
//!   `met_path` is not an executable file, the blueprint is missing or hashes otherwise, or the
//!   resident endpoint refuses `GET /v1/models`. There is no fallback to another embedder.
//! - The host never answers [`Reply::Retryable`]: a local failure is final for the batch.
//! - A spawned `met` never outlives the host: its process group is killed on every exit path.
#![forbid(unsafe_code)]

pub mod alias;
pub mod check;
pub mod child;
pub mod resident;

use std::path::PathBuf;
use std::sync::Arc;

use latlas_core::embed::{Embedder, Mode, Reply, Role};
use latlas_core::metlocal::{Invocation, MetSettings};
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-24: The met-local host as configured, or its declared unavailable mode. A clone shares
/// the host (`Arc`), so a kept alias child is shared too.
#[derive(Clone)]
pub enum MetLocal {
    Alias(Arc<alias::Alias>),
    Resident(Arc<resident::Resident>),
    Unavailable(String),
}

impl MetLocal {
    /// 2026-10-24: Check the files (alias) or probe the endpoint (resident) once, off the
    /// executor; `log` receives the alias child's stderr.
    pub async fn open(s: &MetSettings, log: PathBuf) -> Self {
        let s2 = s.clone();
        let checked = tokio::task::spawn_blocking(move || check::preflight(&s2)).await;
        match checked {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Self::Unavailable(reason),
            Err(_) => return Self::Unavailable("the preflight task was cancelled".into()),
        }
        match &s.invocation {
            Invocation::Alias {
                met_path,
                boot_deadline_ms,
                idle_exit_ms,
            } => Self::Alias(Arc::new(alias::Alias::new(
                s.clone(),
                met_path.clone(),
                *boot_deadline_ms,
                *idle_exit_ms,
                log,
            ))),
            Invocation::Resident { endpoint } => Self::Resident(Arc::new(resident::Resident::new(
                s.clone(),
                endpoint.clone(),
            ))),
        }
    }

    fn settings(&self) -> Option<&MetSettings> {
        match self {
            Self::Alias(a) => Some(a.settings()),
            Self::Resident(r) => Some(r.settings()),
            Self::Unavailable(_) => None,
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::Unavailable(r) => r.clone(),
            Self::Alias(_) | Self::Resident(_) => String::new(),
        }
    }
}

impl Embedder for MetLocal {
    fn mode(&self) -> Mode {
        match self.settings() {
            Some(s) => Mode::Available {
                model: s.model.clone(),
                dim: s.dimensions,
            },
            None => Mode::Unavailable {
                reason: self.reason(),
            },
        }
    }

    async fn embed(&self, role: Role, inputs: &[String]) -> Result<Reply> {
        if let Some(s) = self.settings() {
            let max = usize::try_from(s.max_batch).unwrap_or(usize::MAX);
            if inputs.len() > max {
                return Err(error!(
                    ErrorCode::InvalidArgument,
                    "inputs",
                    format!("{} exceed max_batch {max}", inputs.len())
                ));
            }
        }
        match self {
            Self::Alias(a) => a.embed(role, inputs).await.map(Reply::Vectors),
            Self::Resident(r) => Arc::clone(r).embed(role, inputs).await.map(Reply::Vectors),
            Self::Unavailable(_) => {
                Err(error!(ErrorCode::LatlasEmbedderUnavailable, self.reason()))
            }
        }
    }
}
