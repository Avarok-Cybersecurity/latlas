// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `score` (check part §3, K2): derive the head bundle (and the base, from a second
//! config naming the base's sources) with this binary in this run, gather the host's facts (git
//! history, the commit window, CI gate results), score both with `latlas-check`, print the report JSON and optionally write the PR comment. No stored baseline.
//!
//! Owner: latlas-cli.
//! Invariants: stdout is the report JSON; the exit code is the ratchet's (0, 3 regression,
//! 4 fatal), carried as [`CheckExit`].

use crate::app::{derive_once, open_hosts};
use crate::host::PackHost;
use anyhow::Result;
use latlas_check::report::{Sides, json, markdown};
use latlas_check::{CheckConfig, CheckFacts, Checks, WindowCommit, compare, score};
use latlas_host_native::git;
use std::collections::BTreeMap;
use std::path::PathBuf;
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

fn bundle<P: Checks, H: PackHost<P>>(cfg: &Path) -> Result<(Bundle, PathBuf)> {
    let (cfg, fs, host) = open_hosts::<P, H>(cfg)?;
    Ok((derive_once::<P, H>(&cfg, &fs, &host)?.1, fs.root))
}

/// 2026-10-24: The host's facts for the head: git history of every source and declared derived
/// path, the window's commits, and the CI gate results file (`{"<gate key>": "pass"|"fail"}`).
fn facts(head: &Bundle, root: &Path, check: &CheckConfig, gates: Option<&Path>) -> Result<CheckFacts> {
    let mut paths: Vec<String> = head.sources.iter().map(|s| s.path.clone()).collect();
    for d in &check.derived {
        paths.push(d.output.clone());
        paths.extend(d.inputs.iter().filter(|i| !i.ends_with('/')).cloned());
    }
    paths.sort();
    paths.dedup();
    let window = git::window(root, &paths, check.window.days)?
        .into_iter()
        .map(|(sha, paths)| WindowCommit { sha, paths })
        .collect();
    let gate_results = match gates {
        None => None,
        Some(p) => {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p)?)?;
            let obj = v
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("{}: not a JSON object", p.display()))?;
            let mut m = BTreeMap::new();
            for (k, r) in obj {
                let pass = match r.as_str() {
                    Some("pass") => true,
                    Some("fail") => false,
                    _ => anyhow::bail!("{}: `{k}` is neither \"pass\" nor \"fail\"", p.display()),
                };
                m.insert(k.trim_start_matches("gate:").to_string(), pass);
            }
            Some(m)
        }
    };
    Ok(CheckFacts {
        last_commit: Some(git::last_commits(root, &paths)?),
        window: Some(window),
        gate_results,
    })
}

/// 2026-10-23: The command.
pub fn score_cmd<P: Checks, H: PackHost<P>>(
    head_cfg: &Path,
    base_cfg: Option<&Path>,
    check_cfg: &Path,
    comment: Option<&Path>,
    gates: Option<&Path>,
) -> Result<()> {
    let check = CheckConfig::parse(&std::fs::read_to_string(check_cfg)?)?;
    let (head, root) = bundle::<P, H>(head_cfg)?;
    let base = base_cfg.map(bundle::<P, H>).transpose()?.map(|b| b.0);
    let f = facts(&head, &root, &check, gates)?;
    let hs = score::<P>(&head, base.as_ref(), &check, &f)?;
    let bs = base
        .as_ref()
        .map(|b| score::<P>(b, None, &check, &f))
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
