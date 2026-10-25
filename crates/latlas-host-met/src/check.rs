// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The preflight that decides the declared mode once: `met_path` is an executable
//! file (alias), the blueprint's BlueprintId is the configured `blueprint_id`, and (resident) the
//! endpoint answers `GET .../models` with 200. Blocking; [`crate::MetLocal::open`] runs it off the
//! executor.
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - The id is derived as met derives it ([`latlas_core::kbp`]): from the decoded body section,
//!   never the file's bytes, so a lite and a full file of one build both pass.
//! - Only the header and the body section of a binary file are read (a full file's payload can
//!   be a GiB); a `kbp1.` text file is read whole, after its length is checked.
//! - Every refusal is a reason string for [`latlas_core::embed::Mode::Unavailable`]; the file's
//!   contents are never in it, only its path, the refusal and the ids.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use latlas_core::kbp::{self, BlueprintId, DecodeMemory, HEADER_LEN, Header, MAX_TEXT_LEN};
use latlas_core::metlocal::{Invocation, MetSettings, models_url};

/// 2026-10-24: The most model memory one kzx block may use while the id is derived: 1 GiB, the
/// budget met's own server passes (an embed blueprint's lite body needs far less).
pub const DECODE_MEMORY: DecodeMemory = DecodeMemory(1 << 30);

/// 2026-10-24: `path` is a regular file with an execute bit.
pub fn executable(path: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => Ok(()),
        Ok(_) => Err(format!("met_path {path} is not an executable file")),
        Err(e) => Err(format!("met_path {path} is unusable ({:?})", e.kind())),
    }
}

fn io(path: &str, e: &std::io::Error) -> String {
    format!("blueprint {path} is unreadable ({:?})", e.kind())
}

/// 2026-10-24: The BlueprintId of the `.kbp` (binary or `kbp1.` text) at `path`.
pub fn blueprint_id(path: &str) -> Result<BlueprintId, String> {
    let mut f = File::open(path).map_err(|e| io(path, &e))?;
    let len = f.metadata().map_err(|e| io(path, &e))?.len();
    let mut head = Vec::with_capacity(HEADER_LEN);
    (&mut f)
        .take(u64::try_from(HEADER_LEN).unwrap_or(u64::MAX))
        .read_to_end(&mut head)
        .map_err(|e| io(path, &e))?;
    let refused = |e: latlas_error::Error| format!("blueprint {path} is refused: {e}");
    if kbp::is_text(&head) {
        if len > MAX_TEXT_LEN {
            return Err(format!(
                "blueprint {path} is {len} bytes of kbp1. text, over the {MAX_TEXT_LEN}-byte cap"
            ));
        }
        f.read_to_end(&mut head).map_err(|e| io(path, &e))?;
        return kbp::file_id(&head, DECODE_MEMORY).map_err(refused);
    }
    let h = Header::parse(&head, len).map_err(refused)?;
    let body_len = usize::try_from(h.body.end - h.body.start)
        .map_err(|_| format!("blueprint {path}: the body section does not fit in memory"))?;
    let mut body = vec![0u8; body_len];
    f.seek(SeekFrom::Start(h.body.start))
        .and_then(|_| f.read_exact(&mut body))
        .map_err(|e| io(path, &e))?;
    kbp::body_id(&h, &body, DECODE_MEMORY).map_err(refused)
}

/// 2026-10-24: The blueprint at `path` has BlueprintId `want`.
pub fn blueprint(path: &str, want: &BlueprintId) -> Result<(), String> {
    let got = blueprint_id(path)?;
    if got != *want {
        return Err(format!(
            "blueprint {path} has BlueprintId {}; the config declares blueprint_id {}",
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
    blueprint(&s.blueprint, &s.blueprint_id)?;
    if let Invocation::Resident { endpoint } = &s.invocation {
        probe(endpoint, s.request_deadline_ms)?;
    }
    Ok(())
}
