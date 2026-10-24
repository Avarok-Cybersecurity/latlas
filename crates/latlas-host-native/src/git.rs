// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: Git facts for the score (check part §1 F, T and churn): each path's last commit,
//! and the commits of a window with the paths they touched. Read through the `git` binary, per
//! repository: a source tree may span several checkouts (the metrale pack's does), so each file's
//! repository is its own `rev-parse --show-toplevel`.
//!
//! Owner: latlas-host-native.
//! Invariants: paths in and out are relative to `root`; a file outside any repository has no last
//! commit (absent, never guessed); git failing to run is an error.

use latlas_error::{ErrorCode, Result, error};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| error!(ErrorCode::LatlasSource, "git", e.to_string()))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

fn toplevel(dir: &Path, cache: &mut BTreeMap<PathBuf, Option<PathBuf>>) -> Result<Option<PathBuf>> {
    if let Some(t) = cache.get(dir) {
        return Ok(t.clone());
    }
    let t = git(dir, &["rev-parse", "--show-toplevel"])?.map(|s| PathBuf::from(s.trim()));
    cache.insert(dir.to_path_buf(), t.clone());
    Ok(t)
}

/// 2026-10-24: path → (sha, commit time) of the last commit that touched it.
pub fn last_commits(root: &Path, paths: &[String]) -> Result<BTreeMap<String, (String, i64)>> {
    let mut out = BTreeMap::new();
    for p in paths {
        let abs = root.join(p);
        let Some(dir) = abs.parent().filter(|d| d.is_dir()) else {
            continue;
        };
        let Some(name) = abs.file_name() else { continue };
        let Some(s) = git(dir, &["log", "-1", "--format=%H %ct", "--", &name.to_string_lossy()])? else {
            continue;
        };
        if let Some((sha, t)) = s.trim().split_once(' ')
            && let Ok(t) = t.parse::<i64>()
        {
            out.insert(p.clone(), (sha.to_string(), t));
        }
    }
    Ok(out)
}

/// 2026-10-24: The commits of the last `days` days in every repository holding one of `paths`,
/// newest first per repository, each with the touched paths that lie under `root`.
pub fn window(root: &Path, paths: &[String], days: u32) -> Result<Vec<(String, Vec<String>)>> {
    let mut cache = BTreeMap::new();
    let mut tops = BTreeSet::new();
    for p in paths {
        if let Some(dir) = root.join(p).parent().filter(|d| d.is_dir())
            && let Some(t) = toplevel(dir, &mut cache)?
        {
            tops.insert(t);
        }
    }
    let root = std::fs::canonicalize(root).map_err(|e| error!(ErrorCode::LatlasSource, root.display().to_string(), e.to_string()))?;
    let since = format!("--since={days}.days.ago");
    let mut out = Vec::new();
    for top in tops {
        let Some(log) = git(&top, &["log", &since, "--format=%x01%H", "--name-only"])? else {
            continue;
        };
        for block in log.split('\u{1}').filter(|b| !b.trim().is_empty()) {
            let mut lines = block.lines();
            let Some(sha) = lines.next().map(str::trim) else { continue };
            let touched: Vec<String> = lines
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| top.join(l.trim()).strip_prefix(&root).ok().map(|r| r.to_string_lossy().into_owned()))
                .collect();
            out.push((sha.to_string(), touched));
        }
    }
    Ok(out)
}
