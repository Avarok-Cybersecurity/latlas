// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The `met-local` embedder settings (DESIGN-latlas-local-embedder.md §4): `met`
//! serves the embedding model, either spawned per request as the `met embed` alias or as a
//! running `met serve`. Pure: the host (latlas-host-met) checks the files and the endpoint.
//!
//! Owner: latlas-core.
//! Invariants:
//! - Every key the mode uses is required; nothing is defaulted (PCND). A key of the other
//!   invocation, or of mode `http`, is refused when present: no config is silently ignored.
//! - The settings are resolved once into [`MetSettings`] (ROTI); the blueprint's sha256 is part of
//!   [`MetSettings::space`], so a changed blueprint invalidates the vector cache.

use crate::hash::Sha256;
use crate::prelude::*;
use crate::settings::{EmbedLayer, InputTypes, need_for, positive};
use latlas_error::{ErrorCode, Result, error};
use serde_json::Value;

/// 2026-10-24: The keys only `met-local` uses (mode `http` refuses them).
pub const MET_ONLY: &[&str] = &[
    "invocation",
    "met_path",
    "blueprint",
    "blueprint_sha256",
    "max_batch",
    "boot_deadline_ms",
    "request_deadline_ms",
    "idle_exit_ms",
];

/// 2026-10-24: The keys only mode `http` uses (and `reason`, which only `unavailable` uses);
/// `met-local` refuses them.
pub const NOT_MET: &[&str] = &[
    "reason",
    "headers",
    "key_file",
    "key_prefix",
    "key_env",
    "timeout_ms",
    "batch",
    "max_attempts",
    "backoff_ms",
    "min_interval_ms",
];

/// 2026-10-24: The keys only `invocation = "alias"` uses.
pub const ALIAS_ONLY: &[&str] = &["met_path", "boot_deadline_ms", "idle_exit_ms"];

/// 2026-10-24: The keys only `invocation = "resident"` uses.
pub const RESIDENT_ONLY: &[&str] = &["endpoint"];

/// 2026-10-24: The attempts per batch the pipeline makes against met-local: the host never
/// answers `Reply::Retryable` (a local process has no rate limit), so a failure is final.
pub const MET_ATTEMPTS: u32 = 1;

/// 2026-10-24: How Latlas reaches `met`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invocation {
    /// 2026-10-24: Spawn `met_path embed` per request (or keep it `idle_exit_ms` for the next).
    Alias {
        met_path: String,
        /// 2026-10-24: Spawn to the first response frame header.
        boot_deadline_ms: u64,
        /// 2026-10-24: 0: the child exits after its one request; else it is kept for further
        /// requests and exits by itself after this long without one.
        idle_exit_ms: u64,
    },
    /// 2026-10-24: POST a running `met serve` at `endpoint` (`.../v1/embeddings`, loopback).
    Resident { endpoint: String },
}

/// 2026-10-24: The resolved, immutable met-local configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetSettings {
    pub invocation: Invocation,
    pub blueprint: String,
    pub blueprint_sha256: Sha256,
    pub model: String,
    pub dimensions: u32,
    pub input_type: InputTypes,
    pub max_batch: u32,
    pub max_input_chars: u32,
    pub request_deadline_ms: u64,
}

impl MetSettings {
    /// 2026-10-24: The vector space: model, dimension, document `input_type` and the blueprint's
    /// sha256 (a different blueprint can mean different numerics).
    pub fn space(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.model,
            self.dimensions,
            self.input_type.document,
            self.blueprint_sha256.hex()
        )
    }
}

/// 2026-10-24: Refuse every key of `keys` the merged layer sets, naming `why` (the mode or
/// invocation that does not use it).
pub fn refuse_keys(m: &EmbedLayer, keys: &[&str], why: &str) -> Result<()> {
    let set = serde_json::to_value(m).unwrap_or(Value::Null);
    for k in keys {
        if set.get(*k).is_some_and(|v| !v.is_null()) {
            return Err(error!(
                ErrorCode::LatlasConfig,
                format!("embedder.{k}"),
                format!("is not used by {why}; remove it")
            ));
        }
    }
    Ok(())
}

/// 2026-10-24: A 64-digit lowercase hex sha256.
pub fn parse_sha256(hex: &str) -> Option<Sha256> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let nib = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        out[i] = (nib(pair[0])? << 4) | nib(pair[1])?;
    }
    Some(Sha256(out))
}

/// 2026-10-24: Whether `url` is `http://` or `https://` to a loopback host (`127.0.0.1`,
/// `localhost` or `[::1]`, then a port, a path or the end), with a path ending `/embeddings`.
pub fn loopback_embeddings(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let host_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..host_end];
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if !h.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && rest[host_end..].ends_with("/embeddings")
}

/// 2026-10-24: The `GET .../models` URL beside a `.../embeddings` endpoint (the resident probe).
pub fn models_url(endpoint: &str) -> Option<String> {
    endpoint
        .strip_suffix("/embeddings")
        .map(|base| format!("{base}/models"))
}

fn absolute(v: &str, key: &'static str) -> Result<()> {
    if v.starts_with('/') {
        return Ok(());
    }
    Err(error!(
        ErrorCode::LatlasConfig,
        format!("embedder.{key}"),
        "must be an absolute path"
    ))
}

/// 2026-10-24: Resolve a merged layer whose `mode` is `met-local`.
pub fn resolve(m: &EmbedLayer) -> Result<MetSettings> {
    const MODE: &str = "mode met-local";
    let need = |v, k| need_for(v, k, MODE);
    refuse_keys(m, NOT_MET, MODE)?;
    let invocation = match need(&m.invocation, "invocation")?.as_str() {
        "alias" => {
            refuse_keys(m, RESIDENT_ONLY, "invocation alias")?;
            let met_path = need(&m.met_path, "met_path")?;
            absolute(&met_path, "met_path")?;
            let boot_deadline_ms = need_for(&m.boot_deadline_ms, "boot_deadline_ms", MODE)?;
            positive(boot_deadline_ms, "boot_deadline_ms")?;
            Invocation::Alias {
                met_path,
                boot_deadline_ms,
                idle_exit_ms: need_for(&m.idle_exit_ms, "idle_exit_ms", MODE)?,
            }
        }
        "resident" => {
            refuse_keys(m, ALIAS_ONLY, "invocation resident")?;
            let endpoint = need(&m.endpoint, "endpoint")?;
            if !loopback_embeddings(&endpoint) {
                return Err(error!(
                    ErrorCode::LatlasConfig,
                    "embedder.endpoint", "must be a loopback http(s) URL ending /embeddings"
                ));
            }
            Invocation::Resident { endpoint }
        }
        other => {
            return Err(error!(
                ErrorCode::LatlasConfig,
                "embedder.invocation",
                format!("`{other}` is not alias or resident")
            ));
        }
    };
    let blueprint = need(&m.blueprint, "blueprint")?;
    absolute(&blueprint, "blueprint")?;
    let sha = need(&m.blueprint_sha256, "blueprint_sha256")?;
    let blueprint_sha256 = parse_sha256(&sha).ok_or_else(|| {
        error!(
            ErrorCode::LatlasConfig,
            "embedder.blueprint_sha256", "must be 64 lowercase hex digits"
        )
    })?;
    let s = MetSettings {
        invocation,
        blueprint,
        blueprint_sha256,
        model: need(&m.model, "model")?,
        dimensions: need_for(&m.dimensions, "dimensions", MODE)?,
        input_type: need_for(&m.input_type, "input_type", MODE)?,
        max_batch: need_for(&m.max_batch, "max_batch", MODE)?,
        max_input_chars: need_for(&m.max_input_chars, "max_input_chars", MODE)?,
        request_deadline_ms: need_for(&m.request_deadline_ms, "request_deadline_ms", MODE)?,
    };
    if s.input_type.query.is_empty() || s.input_type.document.is_empty() {
        return Err(error!(
            ErrorCode::LatlasConfig,
            "embedder.input_type", "names a non-empty query and document"
        ));
    }
    positive(u64::from(s.dimensions), "dimensions")?;
    positive(u64::from(s.max_batch), "max_batch")?;
    positive(u64::from(s.max_input_chars), "max_input_chars")?;
    positive(s.request_deadline_ms, "request_deadline_ms")?;
    Ok(s)
}

#[cfg(test)]
#[path = "metlocal/tests.rs"]
mod tests;
