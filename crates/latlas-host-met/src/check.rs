// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The preflight that decides the declared mode once: `met_path` is an executable
//! file (alias), the blueprint hashes to the configured sha256, and (resident) the endpoint
//! answers `GET .../models` with 200. Blocking; [`crate::MetLocal::open`] runs it off the executor.
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - Every refusal is a reason string for [`latlas_core::embed::Mode::Unavailable`]; the file's
//!   contents are never in it, only its path and hashes.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use latlas_core::hash::{Sha256, sha256};
use latlas_core::metlocal::{Invocation, MetSettings, models_url};

/// 2026-10-24: `path` is a regular file with an execute bit.
pub fn executable(path: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => Ok(()),
        Ok(_) => Err(format!("met_path {path} is not an executable file")),
        Err(e) => Err(format!("met_path {path} is unusable ({:?})", e.kind())),
    }
}

/// 2026-10-24: The blueprint at `path` hashes to `want`.
pub fn blueprint(path: &str, want: &Sha256) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("blueprint {path} is unreadable ({:?})", e.kind()))?;
    let got = sha256(&bytes);
    if got != *want {
        return Err(format!(
            "blueprint {path} hashes to {}; the config declares {}",
            got.hex(),
            want.hex()
        ));
    }
    Ok(())
}

/// 2026-10-24: The resident endpoint's `GET .../models` answers 200 within `deadline_ms`.
pub fn probe(endpoint: &str, deadline_ms: u64) -> Result<(), String> {
    let url = models_url(endpoint).ok_or_else(|| format!("{endpoint} does not end /embeddings"))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(deadline_ms)))
        .http_status_as_error(false)
        .build()
        .into();
    match agent.get(&url).call() {
        Ok(r) if r.status().as_u16() == 200 => Ok(()),
        Ok(r) => Err(format!("GET {url} answered HTTP {}", r.status().as_u16())),
        Err(e) => Err(format!("GET {url} refused: {e}")),
    }
}

/// 2026-10-24: Every check the configured invocation needs.
pub fn preflight(s: &MetSettings) -> Result<(), String> {
    if let Invocation::Alias { met_path, .. } = &s.invocation {
        executable(met_path)?;
    }
    blueprint(&s.blueprint, &s.blueprint_sha256)?;
    if let Invocation::Resident { endpoint } = &s.invocation {
        probe(endpoint, s.request_deadline_ms)?;
    }
    Ok(())
}
