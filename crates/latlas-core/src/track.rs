// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: Repo tracking (`[repo_track]`): follow `remote ref` of one source repository from
//! its git object store. Each tick fetches the ref through the [`GitPort`]; a new commit's tree is
//! listed, and only the blobs the sources read and the previous snapshot lacks are read. The
//! result is a [`Snapshot`] a host serves those paths from instead of the checkout on disk, so no
//! worktree is moved and there is nothing to clobber.
//!
//! Owner: latlas-core.
//! Invariants:
//! - Pure (SBIO): git is reached only through [`GitPort`] (async; the host's executor drives it,
//!   ACNB) and time only through the Clock port.
//! - A failed tick keeps the last good snapshot and records the coded error; the next tick
//!   retries. A tick never panics.
//! - The same commit is no work: no tree listing, no blob read.

use crate::prelude::*;
use alloc::sync::Arc;
use latlas_error::{Error, ErrorCode, Result, error};
use latlas_ports::{Clock, UnixMs};
use serde::Deserialize;
use serde_json::{Map, Value};

/// 2026-10-25: A commit object id (hex).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(pub String);

/// 2026-10-25: A blob object id (hex).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId(pub String);

/// 2026-10-25: A commit's regular files: repo-relative path to blob id.
pub type Tree = BTreeMap<String, BlobId>;

/// 2026-10-25: `[repo_track]`. Every field is required.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepoTrack {
    /// 2026-10-25: The root-relative directory of the repository (any checkout of it).
    pub repo: String,
    pub remote: String,
    /// 2026-10-25: The remote ref to follow (a branch or tag name); never guessed.
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub interval_s: u64,
    /// 2026-10-25: The longest one fetch may take before the tick fails.
    pub fetch_timeout_s: u64,
}

/// 2026-10-25: A remote or ref name git cannot read as an option or a revision expression.
fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

impl RepoTrack {
    /// 2026-10-25: Refuse a field git could misread or a zero interval.
    pub fn validate(&self) -> Result<()> {
        let bad = |f: &str, why: &str| {
            Err(error!(
                ErrorCode::LatlasConfig,
                f.to_string(),
                why.to_string()
            ))
        };
        if self.repo.is_empty()
            || self.repo.starts_with('/')
            || self.repo.split('/').any(|c| c == "..")
        {
            return bad("repo_track.repo", "must be root-relative, without ..");
        }
        if !plain_name(&self.remote) {
            return bad(
                "repo_track.remote",
                "must be a plain name ([A-Za-z0-9._/-], no leading -, no ..)",
            );
        }
        if !plain_name(&self.git_ref) {
            return bad(
                "repo_track.ref",
                "must be a plain name ([A-Za-z0-9._/-], no leading -, no ..)",
            );
        }
        if self.interval_s == 0 || self.fetch_timeout_s == 0 {
            return bad(
                "repo_track",
                "interval_s and fetch_timeout_s must be positive",
            );
        }
        Ok(())
    }

    /// 2026-10-25: `remote/ref`, as reports name it.
    #[must_use]
    pub fn tracked(&self) -> String {
        format!("{}/{}", self.remote, self.git_ref)
    }
}

/// 2026-10-25: The git object store of the tracked repository; hosts implement it.
#[expect(
    async_fn_in_trait,
    reason = "2026-10-25: as latlas-ports: the host's executor drives the future"
)]
pub trait GitPort {
    /// 2026-10-25: Fetch `remote ref` and return the commit it names now.
    async fn fetch(&self, remote: &str, git_ref: &str) -> Result<CommitId>;
    /// 2026-10-25: The regular files of `commit`.
    async fn tree(&self, commit: &CommitId) -> Result<Tree>;
    /// 2026-10-25: The bytes of `ids`, in order.
    async fn blobs(&self, ids: &[BlobId]) -> Result<Vec<Vec<u8>>>;
}

/// 2026-10-25: One tracked commit: its tree and the texts of the blobs the sources read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub commit: Option<CommitId>,
    pub tree: Tree,
    pub texts: BTreeMap<BlobId, String>,
}

impl Snapshot {
    /// 2026-10-25: A tree with no texts read yet (what a host lists the wanted paths from).
    #[must_use]
    pub fn listing(commit: CommitId, tree: Tree) -> Self {
        Self {
            commit: Some(commit),
            tree,
            texts: BTreeMap::new(),
        }
    }
}

/// 2026-10-25: What a successful tick did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Synced {
    /// 2026-10-25: The ref still names the snapshot's commit.
    Same,
    /// 2026-10-25: A new snapshot; `read` blobs were read (the rest were the previous ones).
    New { read: usize },
}

/// 2026-10-25: The tracking state: the last good snapshot, when a tick last succeeded, and why the
/// last one failed.
#[derive(Clone, Debug)]
pub struct Tracker {
    pub cfg: RepoTrack,
    snap: Option<Arc<Snapshot>>,
    synced_at: Option<UnixMs>,
    error: Option<String>,
}

fn record(slot: &mut Option<String>, e: &Error) {
    *slot = Some(format!("{}: {e}", e.code().name()));
}

impl Tracker {
    /// 2026-10-25: Nothing tracked yet (the host serves the checkout on disk until a tick lands).
    #[must_use]
    pub fn new(cfg: RepoTrack) -> Self {
        Self {
            cfg,
            snap: None,
            synced_at: None,
            error: None,
        }
    }

    /// 2026-10-25: The last good snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Option<&Arc<Snapshot>> {
        self.snap.as_ref()
    }

    /// 2026-10-25: Why the last tick failed, while it is the latest outcome.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 2026-10-25: The commit the ref named when it was last fetched (`None` before the first).
    #[must_use]
    pub fn commit(&self) -> Option<&CommitId> {
        self.snap.as_ref().and_then(|s| s.commit.as_ref())
    }

    /// 2026-10-25: Take a fetch result: the same commit is no work; a new one lists its tree, asks
    /// `wanted` which repo-relative paths the sources read, and reads only the blobs the previous
    /// snapshot lacks. On any failure the previous snapshot stays and the error is recorded.
    pub async fn advance<G: GitPort, C: Clock, W>(
        &mut self,
        git: &G,
        clock: &C,
        fetched: Result<CommitId>,
        wanted: W,
    ) -> Result<Synced>
    where
        W: FnOnce(&Snapshot) -> Result<BTreeSet<String>>,
    {
        let out = self.step(git, fetched, wanted).await;
        match &out {
            Ok(_) => {
                self.error = None;
                self.synced_at = Some(clock.now_unix_ms());
            }
            Err(e) => record(&mut self.error, e),
        }
        out
    }

    async fn step<G: GitPort, W>(
        &mut self,
        git: &G,
        fetched: Result<CommitId>,
        wanted: W,
    ) -> Result<Synced>
    where
        W: FnOnce(&Snapshot) -> Result<BTreeSet<String>>,
    {
        let commit = fetched?;
        if self.commit() == Some(&commit) {
            return Ok(Synced::Same);
        }
        let tree = git.tree(&commit).await?;
        let listing = Snapshot::listing(commit.clone(), tree);
        let paths = wanted(&listing)?;
        let empty = BTreeMap::new();
        let old = self.snap.as_ref().map_or(&empty, |s| &s.texts);
        let mut texts = BTreeMap::new();
        let mut need: Vec<BlobId> = Vec::new();
        for p in &paths {
            let Some(id) = listing.tree.get(p) else {
                continue;
            };
            match old.get(id) {
                Some(t) => {
                    texts.insert(id.clone(), t.clone());
                }
                None if !need.contains(id) => need.push(id.clone()),
                None => {}
            }
        }
        let bytes = git.blobs(&need).await?;
        if bytes.len() != need.len() {
            return Err(error!(
                ErrorCode::LatlasRepoRead,
                commit.0.clone(),
                "the blob count differs from the request"
            ));
        }
        for (id, b) in need.iter().zip(bytes) {
            let t = String::from_utf8(b).map_err(|_| {
                error!(
                    ErrorCode::LatlasRepoRead,
                    commit.0.clone(),
                    format!("blob {} is not UTF-8", id.0)
                )
            })?;
            texts.insert(id.clone(), t);
        }
        let read = need.len();
        self.snap = Some(Arc::new(Snapshot { texts, ..listing }));
        Ok(Synced::New { read })
    }

    /// 2026-10-25: Record a failure outside [`advance`](Self::advance) (a timed-out fetch).
    pub fn fail(&mut self, e: &Error) {
        record(&mut self.error, e);
    }

    /// 2026-10-25: The `la_status` view.
    #[must_use]
    pub fn status(&self) -> Value {
        let mut o = Map::new();
        o.insert("tracking".into(), Value::Bool(true));
        o.insert("repo".into(), Value::String(self.cfg.repo.clone()));
        o.insert("ref".into(), Value::String(self.cfg.tracked()));
        o.insert("interval_s".into(), Value::from(self.cfg.interval_s));
        o.insert(
            "commit".into(),
            self.commit()
                .map_or(Value::Null, |c| Value::String(c.0.clone())),
        );
        o.insert(
            "last_sync_ms".into(),
            self.synced_at.map_or(Value::Null, |t| Value::from(t.0)),
        );
        o.insert(
            "error".into(),
            self.error.clone().map_or(Value::Null, Value::String),
        );
        Value::Object(o)
    }
}

/// 2026-10-25: The `la_status` view without a `[repo_track]` section.
#[must_use]
pub fn status_off() -> Value {
    let mut o = Map::new();
    o.insert("tracking".into(), Value::Bool(false));
    o.insert(
        "reason".into(),
        Value::String("off: the config has no [repo_track] section".into()),
    );
    Value::Object(o)
}

#[cfg(test)]
#[path = "track_tests.rs"]
mod tests;
