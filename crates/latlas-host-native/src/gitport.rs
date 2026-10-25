// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The native [`GitPort`]: the `git` binary over one repository's object store (as
//! [`crate::git`] already reads it). A fetch stores the tracked ref under the private namespace
//! `refs/latlas/track/<remote>/<ref>` (it keeps the objects from gc and names the commit without
//! racing another fetch's `FETCH_HEAD` or remote-tracking ref); trees and blobs are read with
//! `ls-tree` and `cat-file --batch`. No worktree, index or HEAD is touched.
//!
//! Owner: latlas-host-native.
//! Invariants:
//! - `remote` and `ref` arrive validated (`RepoTrack::validate`: plain names, no leading `-`).
//! - Every git run is bounded by `timeout` and killed on expiry; git never prompts.
//! - Every failure is `LatlasRepoFetch` or `LatlasRepoRead` with what git said.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use latlas_core::track::{BlobId, CommitId, GitPort, Tree};
use latlas_error::{ErrorCode, Result, error};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// 2026-10-25: The repository at `dir` (any checkout of it, or its git directory).
#[derive(Clone, Debug)]
pub struct GitCli {
    dir: PathBuf,
    timeout: Duration,
}

impl GitCli {
    /// 2026-10-25: `git -C dir`, each run bounded by `timeout`.
    #[must_use]
    pub fn new(dir: PathBuf, timeout: Duration) -> Self {
        Self { dir, timeout }
    }

    async fn run(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
    ) -> core::result::Result<Vec<u8>, String> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("git: {:?}", e.kind()))?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            tokio::spawn(async move {
                // 2026-10-25: A write failure shows as git's own error or a short read below.
                let _ = stdin.write_all(&bytes).await;
            });
        }
        let out = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("exceeded fetch_timeout_s {}", self.timeout.as_secs()))?
            .map_err(|e| format!("git: {:?}", e.kind()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "{}: {}",
                out.status,
                err.trim().lines().last().unwrap_or("")
            ));
        }
        Ok(out.stdout)
    }
}

/// 2026-10-25: `ls-tree -r -z` output: the regular files (mode 100644 or 100755).
fn parse_tree(out: &[u8]) -> core::result::Result<Tree, String> {
    let mut tree = Tree::new();
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = std::str::from_utf8(rec).map_err(|_| "a path is not UTF-8".to_string())?;
        let (meta, path) = rec
            .split_once('\t')
            .ok_or_else(|| format!("bad ls-tree record {rec:?}"))?;
        let mut f = meta.split(' ');
        let (Some(mode), Some(kind), Some(oid)) = (f.next(), f.next(), f.next()) else {
            return Err(format!("bad ls-tree record {rec:?}"));
        };
        if kind == "blob" && matches!(mode, "100644" | "100755") {
            tree.insert(path.to_string(), BlobId(oid.to_string()));
        }
    }
    Ok(tree)
}

/// 2026-10-25: `cat-file --batch` output for `ids`, in order.
fn parse_batch(mut out: &[u8], ids: &[BlobId]) -> core::result::Result<Vec<Vec<u8>>, String> {
    let mut blobs = Vec::with_capacity(ids.len());
    for id in ids {
        let nl = out
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("a truncated batch header")?;
        let head = std::str::from_utf8(&out[..nl]).map_err(|_| "a batch header is not UTF-8")?;
        let mut f = head.split(' ');
        let (Some(oid), Some(kind), Some(size)) = (f.next(), f.next(), f.next()) else {
            return Err(format!("blob {}: {head}", id.0));
        };
        if oid != id.0 || kind != "blob" {
            return Err(format!("blob {}: {head}", id.0));
        }
        let n: usize = size
            .parse()
            .map_err(|_| format!("blob {}: bad size {size}", id.0))?;
        let body = out.get(nl + 1..nl + 1 + n).ok_or("a truncated blob")?;
        blobs.push(body.to_vec());
        out = out.get(nl + 2 + n..).unwrap_or(&[]);
    }
    Ok(blobs)
}

impl GitPort for GitCli {
    async fn fetch(&self, remote: &str, git_ref: &str) -> Result<CommitId> {
        let what = format!("{remote} {git_ref}");
        let fail = |why: String| error!(ErrorCode::LatlasRepoFetch, what.clone(), why);
        let local = format!("refs/latlas/track/{remote}/{git_ref}");
        let spec = format!("+{git_ref}:{local}");
        self.run(
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                remote,
                &spec,
            ],
            None,
        )
        .await
        .map_err(fail)?;
        let rev = format!("{local}^{{commit}}");
        let out = self
            .run(
                &["rev-parse", "--verify", "--quiet", "--end-of-options", &rev],
                None,
            )
            .await
            .map_err(fail)?;
        Ok(CommitId(String::from_utf8_lossy(&out).trim().to_string()))
    }

    async fn tree(&self, commit: &CommitId) -> Result<Tree> {
        let fail = |why: String| error!(ErrorCode::LatlasRepoRead, commit.0.clone(), why);
        let out = self
            .run(
                &[
                    "ls-tree",
                    "-r",
                    "-z",
                    "--full-tree",
                    "--end-of-options",
                    &commit.0,
                ],
                None,
            )
            .await
            .map_err(fail)?;
        parse_tree(&out).map_err(fail)
    }

    async fn blobs(&self, ids: &[BlobId]) -> Result<Vec<Vec<u8>>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let what = format!("{} blobs", ids.len());
        let fail = |why: String| error!(ErrorCode::LatlasRepoRead, what.clone(), why);
        let input: Vec<u8> = ids
            .iter()
            .flat_map(|b| format!("{}\n", b.0).into_bytes())
            .collect();
        let out = self
            .run(&["cat-file", "--batch"], Some(input))
            .await
            .map_err(fail)?;
        parse_batch(&out, ids).map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_batch, parse_tree};
    use latlas_core::track::BlobId;

    #[test]
    fn ls_tree_keeps_regular_files_and_cat_file_batches_split_by_size() {
        let out = b"100644 blob aa\ta.md\x00100755 blob bb\tbin/x\x00120000 blob cc\tlink\x00160000 commit dd\tsub\x00";
        let t = parse_tree(out).unwrap();
        assert_eq!(
            t.keys().collect::<Vec<_>>(),
            ["a.md", "bin/x"],
            "no symlink, no submodule"
        );
        let ids = [BlobId("aa".into()), BlobId("bb".into())];
        let batch = b"aa blob 3\na\nb\nbb blob 0\n\n";
        assert_eq!(
            parse_batch(batch, &ids).unwrap(),
            [b"a\nb".to_vec(), Vec::new()]
        );
        assert!(parse_batch(b"aa missing\n", &ids).is_err());
    }
}
