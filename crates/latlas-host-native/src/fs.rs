// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The configured sources on disk: [`FsSources::read`] fills a `SourceSet`,
//! [`FsSources::stamp`] is the per-call freshness check (path, size, mtime) without reading any
//! file. The pack's single files and the `.md` files of its source directories are found here;
//! files only a pack's host can list (a checkout's manifests) are passed in as `extra`.
//!
//! Owner: latlas-host-native (cut from metrale-io-host-kg, 2026-10-23 K0).
//! Invariants:
//! - Credential-named files (`secrets::name_is_secret`) are stamped and listed, never read.
//! - A configured file that is missing is `LatlasSource`, never skipped.
//! - (2026-10-25) With a tracked snapshot ([`Tracked`]), every path under its repository is
//!   listed, read and stamped from the snapshot (the commit's tree and blobs), never from disk.

use latlas_core::secrets::name_is_secret;
use latlas_core::{Config, Pack, SourceSet};
use latlas_error::{ErrorCode, Result, error};
use latlas_core::track::Snapshot;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// 2026-10-23: One file's stat: root-relative path, size, mtime (ns since the epoch).
pub type Stamp = Vec<(String, u64, u128)>;

/// 2026-10-23: The file-system host over the pack's root.
#[derive(Clone, Debug)]
pub struct FsSources {
    pub root: PathBuf,
    /// 2026-10-25: The tracked repository's snapshot, when `[repo_track]` has landed one.
    pub track: Option<Tracked>,
}

/// 2026-10-25: A root-relative repository directory served from a snapshot.
#[derive(Clone, Debug)]
pub struct Tracked {
    pub repo: String,
    pub snap: Arc<Snapshot>,
}

fn not_tracked(rel: &str) -> latlas_error::Error {
    error!(
        ErrorCode::LatlasSource,
        rel.to_string(),
        "not in the tracked commit"
    )
}

/// 2026-10-23: A source the host could not read or stat.
pub fn source_err(path: &str, e: &std::io::Error) -> latlas_error::Error {
    error!(ErrorCode::LatlasSource, path.to_string(), e.to_string())
}

impl FsSources {
    /// 2026-10-23: The host for `cfg`.
    pub fn new<P: Pack>(cfg: &Config<P::Sources>) -> Self {
        FsSources {
            root: PathBuf::from(P::root(&cfg.sources)),
            track: None,
        }
    }

    /// 2026-10-25: The snapshot and the repo-relative path, when `rel` lies in the tracked repo.
    fn tracked<'a>(&'a self, rel: &'a str) -> Option<(&'a Snapshot, &'a str)> {
        let t = self.track.as_ref()?;
        let inner = rel.strip_prefix(t.repo.as_str())?;
        let inner = if inner.is_empty() { inner } else { inner.strip_prefix('/')? };
        Some((&t.snap, inner))
    }

    /// 2026-10-25: The tracked commit, when a snapshot is served.
    pub fn tracked_commit(&self) -> Option<&str> {
        let t = self.track.as_ref()?;
        t.snap.commit.as_ref().map(|c| c.0.as_str())
    }

    /// 2026-10-23: The absolute path of a root-relative one.
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// 2026-10-23: The files of `dir` (root-relative) whose names end in `ext`, sorted.
    pub fn files_in(&self, dir: &str, ext: &str) -> Result<Vec<String>> {
        if let Some((snap, inner)) = self.tracked(dir.trim_end_matches('/')) {
            let pre = if inner.is_empty() { String::new() } else { format!("{inner}/") };
            let base = dir.trim_end_matches('/');
            return Ok(snap
                .tree
                .keys()
                .filter_map(|p| p.strip_prefix(&pre))
                .filter(|n| !n.contains('/') && n.ends_with(ext))
                .map(|n| format!("{base}/{n}"))
                .collect());
        }
        let rd = std::fs::read_dir(self.abs(dir)).map_err(|e| source_err(dir, &e))?;
        let mut out = Vec::new();
        for entry in rd {
            let entry = entry.map_err(|e| source_err(dir, &e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(ext) && entry.path().is_file() {
                out.push(format!("{}/{name}", dir.trim_end_matches('/')));
            }
        }
        out.sort();
        Ok(out)
    }

    fn all_paths<P: Pack>(
        &self,
        cfg: &Config<P::Sources>,
        extra: &[String],
    ) -> Result<Vec<String>> {
        let mut v = cfg.source_paths::<P>();
        for d in P::source_dirs(&cfg.sources) {
            v.extend(self.files_in(&d, ".md")?);
        }
        v.extend(extra.iter().cloned());
        Ok(v)
    }

    /// 2026-10-25: The repo-relative paths under the tracked repository that a read of `cfg` and
    /// `extra` would open (what a new snapshot must hold texts for).
    pub fn tracked_paths<P: Pack>(
        &self,
        cfg: &Config<P::Sources>,
        extra: &[String],
    ) -> Result<std::collections::BTreeSet<String>> {
        Ok(self
            .all_paths::<P>(cfg, extra)?
            .iter()
            .filter(|rel| !name_is_secret(rel))
            .filter_map(|rel| self.tracked(rel).map(|(_, inner)| inner.to_string()))
            .collect())
    }

    /// 2026-10-23: Read every configured source and `extra` (credential-named files are withheld
    /// unread).
    pub fn read<P: Pack>(&self, cfg: &Config<P::Sources>, extra: &[String]) -> Result<SourceSet> {
        let mut set = SourceSet::default();
        for rel in self.all_paths::<P>(cfg, extra)? {
            if name_is_secret(&rel) {
                set.withheld.insert(rel);
                continue;
            }
            let text = match self.tracked(&rel) {
                Some((snap, inner)) => snap
                    .tree
                    .get(inner)
                    .and_then(|id| snap.texts.get(id))
                    .cloned()
                    .ok_or_else(|| not_tracked(&rel))?,
                None => std::fs::read_to_string(self.abs(&rel)).map_err(|e| source_err(&rel, &e))?,
            };
            set.add(&rel, text);
        }
        Ok(set)
    }

    /// 2026-10-23: The freshness stamp of every source and `extra` (no file is opened).
    pub fn stamp<P: Pack>(&self, cfg: &Config<P::Sources>, extra: &[String]) -> Result<Stamp> {
        let mut out = Vec::new();
        for rel in self.all_paths::<P>(cfg, extra)? {
            if let Some((snap, inner)) = self.tracked(&rel) {
                let id = snap.tree.get(inner).ok_or_else(|| not_tracked(&rel))?;
                let h = latlas_core::hash::sha256(id.0.as_bytes());
                let mut b = [0u8; 16];
                b.copy_from_slice(&h.0[..16]);
                out.push((rel, 0, u128::from_le_bytes(b)));
                continue;
            }
            let md = std::fs::metadata(self.abs(&rel)).map_err(|e| source_err(&rel, &e))?;
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            out.push((rel, md.len(), mtime));
        }
        Ok(out)
    }
}

/// 2026-10-23: Read the config file at `path` and parse it for pack `P` (a relative path is
/// resolved, so `--config config.toml` and the absolute path name the same project layer).
pub fn read_config<P: Pack>(path: &Path) -> Result<Config<P::Sources>> {
    let shown = path.display().to_string();
    let path = std::fs::canonicalize(path).map_err(|e| source_err(&shown, &e))?;
    let text = std::fs::read_to_string(&path).map_err(|e| source_err(&shown, &e))?;
    let text = resolve_dot_root(&text, &path, &shown)?;
    let mut cfg = Config::parse::<P>(&text)?;
    let root_s = P::root(&cfg.sources).to_string();
    let root = std::fs::canonicalize(&root_s).map_err(|e| source_err(&root_s, &e))?;
    let rel = path.strip_prefix(&root).map_err(|_| {
        error!(
            ErrorCode::LatlasConfig,
            shown.clone(),
            "must lie under sources.root (it ingests as a node)"
        )
    })?;
    cfg.origin = rel.to_string_lossy().into_owned();
    Ok(cfg)
}

/// 2026-10-24: `sources.root = "."` declares "the repository whose `.latlas/` holds this file", so
/// a committed config needs no absolute path; resolved once here through the core's
/// [`latlas_core::config::resolve_repo_paths`] (with relative store paths).
fn resolve_dot_root(text: &str, path: &Path, shown: &str) -> Result<String> {
    let dir = path.parent().filter(|d| d.file_name().is_some_and(|n| n == ".latlas"));
    match dir.and_then(Path::parent) {
        Some(repo) => Ok(latlas_core::config::resolve_repo_paths(text, &repo.to_string_lossy())?
            .unwrap_or_else(|| text.to_string())),
        None => match latlas_core::config::resolve_repo_paths(text, "/")? {
            None => Ok(text.to_string()),
            Some(_) => Err(error!(
                ErrorCode::LatlasConfig,
                shown.to_string(),
                "sources.root = \".\" is only valid for a config inside a `.latlas/` directory"
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_dot_root;
    use std::path::Path;

    #[test]
    fn a_dot_root_is_the_repository_holding_dot_latlas_and_nothing_else() {
        let t = "schema = \"s\"\n[sources]\nroot = \".\"\n";
        let out = resolve_dot_root(t, Path::new("/r/repo/.latlas/config.toml"), "c").unwrap();
        assert!(out.contains("root = \"/r/repo\""), "{out}");
        assert!(resolve_dot_root(t, Path::new("/r/repo/config.toml"), "c").is_err());
        let store = "[sources]\nroot = \".\"\n[store]\ndata_dir = \".latlas/data\"\nscratch_dir = \"/abs/s\"\n";
        let out = resolve_dot_root(store, Path::new("/r/repo/.latlas/config.toml"), "c").unwrap();
        assert!(out.contains("data_dir = \"/r/repo/.latlas/data\""), "{out}");
        assert!(out.contains("scratch_dir = \"/abs/s\""), "{out}");
        let abs = "[sources]\nroot = \"/abs\"\n";
        assert_eq!(resolve_dot_root(abs, Path::new("/x/.latlas/c.toml"), "c").unwrap(), abs);
    }
}
