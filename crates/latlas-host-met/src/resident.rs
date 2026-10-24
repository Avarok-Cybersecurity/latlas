// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The resident invocation: POST the standard OpenAI embeddings JSON to a running
//! `met serve` on loopback, with `encoding_format: "base64"` (bit-exact little-endian f32), one
//! blocking `ureq` call per batch on tokio's blocking pool.
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - The whole call is bounded by `request_deadline_ms`; passing it is `LatlasEmbedderDeadline`.
//! - The body is read up to the size the config allows (`Limits` in base64), never unbounded.
//! - The answer's blueprint, model and prefix echoes are checked as the alias header's are.

use std::sync::Arc;
use std::time::Duration;

use latlas_core::embed::Role;
use latlas_core::metframe::{Expect, Limits, parse_resident, resident_body};
use latlas_core::metlocal::MetSettings;
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-24: The resident host.
pub struct Resident {
    s: MetSettings,
    endpoint: String,
    agent: ureq::Agent,
}

impl Resident {
    /// 2026-10-24: The host for checked settings (`MetLocal::open` probed the endpoint).
    pub fn new(s: MetSettings, endpoint: String) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(s.request_deadline_ms)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { s, endpoint, agent }
    }

    /// 2026-10-24: The settings it runs with.
    pub fn settings(&self) -> &MetSettings {
        &self.s
    }

    /// 2026-10-24: Embed `inputs` (one request, all of `role`).
    pub async fn embed(self: Arc<Self>, role: Role, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let inputs = inputs.to_vec();
        tokio::task::spawn_blocking(move || self.post(role, &inputs))
            .await
            .map_err(|_| {
                error!(
                    ErrorCode::LatlasEmbedderUnavailable,
                    "the embed task was cancelled"
                )
            })?
    }

    fn post(&self, role: Role, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let input_type = self.s.input_type.of(role);
        let body = resident_body(&self.s.model, input_type, inputs);
        let sent = self
            .agent
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .send(body.as_str());
        let deadline = || {
            error!(
                ErrorCode::LatlasEmbedderDeadline,
                "request", self.s.request_deadline_ms
            )
        };
        let mut resp = match sent {
            Ok(r) => r,
            Err(ureq::Error::Timeout(_)) => return Err(deadline()),
            Err(e) => {
                return Err(error!(
                    ErrorCode::LatlasEmbedderUnavailable,
                    format!("POST {}: {e}", self.endpoint)
                ));
            }
        };
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(error!(
                ErrorCode::LatlasEmbedProtocol,
                format!("HTTP {status}")
            ));
        }
        let l = Limits::of(self.s.max_batch, self.s.dimensions);
        let cap = l.header_max + l.vectors_max.div_ceil(3) * 4;
        let text = resp
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_string()
            .map_err(|e| match e {
                ureq::Error::Timeout(_) => deadline(),
                e => error!(ErrorCode::LatlasEmbedFrame, "resident body", e.to_string()),
            })?;
        let expect = Expect {
            model: &self.s.model,
            blueprint_sha256: &self.s.blueprint_sha256,
            input_type,
            count: inputs.len(),
            dim: self.s.dimensions,
        };
        parse_resident(&text, &expect)
    }
}
