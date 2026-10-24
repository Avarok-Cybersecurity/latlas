// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The derive: a [`Config`] and an already-read [`SourceSet`] in, a sealed [`Bundle`]
//! out. Pure (SBIO): the same inputs give the same bundle digest, whatever order the host read the
//! files in.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants:
//! - A configured source that is absent from the set is an error (`LatlasSource`), never skipped.
//! - The pack's adapters run first; the project config's own node is the core's.

use crate::bundle::Builder;
use crate::bundle::Bundle;
use crate::config::Config;
use crate::pack::Pack;
use crate::prelude::*;
use crate::sources::SourceSet;
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-23: The text of a configured source, or `LatlasSource` when the host did not read it.
pub fn need<'a>(src: &'a SourceSet, path: &str) -> Result<&'a str> {
    src.get(path).ok_or_else(|| {
        error!(
            ErrorCode::LatlasSource,
            path.to_string(),
            "configured but not read"
        )
    })
}

/// 2026-10-23: Derive the bundle. `facts` carries what only the host can compute (the pack says
/// when it is required).
pub fn derive<P: Pack>(
    cfg: &Config<P::Sources>,
    src: &SourceSet,
    facts: Option<&P::Facts>,
) -> Result<Bundle> {
    let mut b = Builder::default();
    P::derive(cfg, src, facts, &mut b)?;
    if !cfg.origin.is_empty() {
        crate::settings::project_node(&mut b, &cfg.origin, need(src, &cfg.origin)?, &cfg.embedder);
    }
    let mut bundle = b.seal(src.digests());
    P::sealed(&mut bundle, facts);
    Ok(bundle)
}
