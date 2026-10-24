// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `score` (check part §3, K2): derive the head bundle (and the base, from a second
//! config naming the base's sources) with this binary in this run, score both with
//! `latlas-check`, print the report JSON and optionally write the PR comment. No stored baseline.
//!
//! Owner: latlas-cli.
//! Invariants: stdout is the report JSON; the exit code is the ratchet's (0, 3 regression,
//! 4 fatal), carried as [`CheckExit`].

use crate::app::{derive_once, open_hosts};
use crate::host::PackHost;
use anyhow::Result;
use latlas_check::report::{Sides, json, markdown};
use latlas_check::{CheckConfig, Checks, compare, score};
use latlas_core::Bundle;
use latlas_core::jsonl::digest;
use std::path::Path;

/// 2026-10-23: The ratchet's non-zero exit code (3 regression, 4 fatal).
#[derive(Debug)]
pub struct CheckExit(pub u8);

impl std::fmt::Display for CheckExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            3 => write!(f, "latlas-check: a dimension regressed"),
            4 => write!(f, "latlas-check: fatal findings"),
            n => write!(f, "latlas-check: exit {n}"),
        }
    }
}

impl std::error::Error for CheckExit {}

fn bundle<P: Checks, H: PackHost<P>>(cfg: &Path) -> Result<Bundle> {
    let (cfg, fs, host) = open_hosts::<P, H>(cfg)?;
    Ok(derive_once::<P, H>(&cfg, &fs, &host)?.1)
}

/// 2026-10-23: The command.
pub fn score_cmd<P: Checks, H: PackHost<P>>(
    head_cfg: &Path,
    base_cfg: Option<&Path>,
    check_cfg: &Path,
    comment: Option<&Path>,
) -> Result<()> {
    let check = CheckConfig::parse(&std::fs::read_to_string(check_cfg)?)?;
    let head = bundle::<P, H>(head_cfg)?;
    let base = base_cfg.map(bundle::<P, H>).transpose()?;
    let hs = score::<P>(&head, base.as_ref(), &check)?;
    let bs = base
        .as_ref()
        .map(|b| score::<P>(b, None, &check))
        .transpose()?;
    let cmp = compare(&check, bs.as_ref(), &hs, &head);
    let (hd, bd) = (digest(&head).hex(), base.as_ref().map(|b| digest(b).hex()));
    let packs = format!("{} {}", P::ID, P::VERSION);
    let sides = Sides {
        base_digest: bd.as_deref(),
        head_digest: &hd,
        mode: "graph_only",
        packs: &packs,
    };
    let report = json(&sides, &check, bs.as_ref(), &hs, &cmp);
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(p) = comment {
        std::fs::write(p, markdown(&report, P::ID))?;
    }
    if cmp.exit != 0 {
        return Err(CheckExit(cmp.exit).into());
    }
    Ok(())
}
