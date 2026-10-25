// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The tracker over a fake git port and a manual clock: a new commit re-reads only the
//! changed blobs, the same commit is no work, a failure keeps the last good snapshot and records
//! the coded error, and a config without the section is off.

use super::{BlobId, CommitId, GitPort, RepoTrack, Synced, Tracker, status_off};
use crate::config::Config;
use latlas_error::{ErrorCode, Result, error};
use latlas_ports::{Clock, MonoNs, UnixMs};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

fn block<F: Future>(f: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    match pin!(f).as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("the fake port never waits"),
    }
}

/// 2026-10-25: Commits as `path -> content`; a blob id is its content's name.
#[derive(Default)]
struct FakeGit {
    commits: RefCell<BTreeMap<String, BTreeMap<String, String>>>,
    trees: Cell<u32>,
    read: RefCell<Vec<String>>,
}

impl FakeGit {
    fn commit(&self, id: &str, files: &[(&str, &str)]) {
        let m = files
            .iter()
            .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
            .collect();
        self.commits.borrow_mut().insert(id.into(), m);
    }
}

impl GitPort for FakeGit {
    async fn fetch(&self, _: &str, _: &str) -> Result<CommitId> {
        unreachable!("the test passes the fetch result")
    }
    async fn tree(&self, c: &CommitId) -> Result<BTreeMap<String, BlobId>> {
        self.trees.set(self.trees.get() + 1);
        let commits = self.commits.borrow();
        let files = commits
            .get(&c.0)
            .ok_or_else(|| error!(ErrorCode::LatlasRepoRead, c.0.clone(), "no such commit"))?;
        Ok(files
            .iter()
            .map(|(p, t)| (p.clone(), BlobId(format!("b-{t}"))))
            .collect())
    }
    async fn blobs(&self, ids: &[BlobId]) -> Result<Vec<Vec<u8>>> {
        self.read
            .borrow_mut()
            .extend(ids.iter().map(|b| b.0.clone()));
        Ok(ids
            .iter()
            .map(|b| b.0.trim_start_matches("b-").as_bytes().to_vec())
            .collect())
    }
}

struct Manual(Cell<i64>);

impl Clock for Manual {
    fn now_unix_ms(&self) -> UnixMs {
        UnixMs(self.0.get())
    }
    fn monotonic_ns(&self) -> MonoNs {
        MonoNs(0)
    }
}

fn cfg() -> RepoTrack {
    RepoTrack {
        repo: "src".into(),
        remote: "origin".into(),
        git_ref: "main".into(),
        interval_s: 300,
        fetch_timeout_s: 60,
    }
}

/// 2026-10-25: The sources read `a.md` and `b.md` (not `big.bin`).
fn wanted(_: &super::Snapshot) -> Result<BTreeSet<String>> {
    Ok(["a.md", "b.md"].map(String::from).into())
}

fn text<'a>(t: &'a Tracker, path: &str) -> &'a str {
    let s = t.snapshot().unwrap();
    &s.texts[&s.tree[path]]
}

#[test]
fn a_new_commit_reads_only_the_changed_wanted_blobs_and_the_same_commit_is_no_work() {
    let (git, clock, mut t) = (
        FakeGit::default(),
        Manual(Cell::new(1_000)),
        Tracker::new(cfg()),
    );
    git.commit(
        "c1",
        &[("a.md", "alpha"), ("b.md", "beta"), ("big.bin", "huge")],
    );
    git.commit(
        "c2",
        &[
            ("a.md", "alpha"),
            ("b.md", "beta two"),
            ("big.bin", "huger"),
        ],
    );
    let r = block(t.advance(&git, &clock, Ok(CommitId("c1".into())), wanted)).unwrap();
    assert_eq!(r, Synced::New { read: 2 });
    assert_eq!(
        *git.read.borrow(),
        ["b-alpha", "b-beta"],
        "only wanted blobs"
    );
    assert_eq!(t.status()["last_sync_ms"], 1_000);
    clock.0.set(2_000);
    let r = block(t.advance(&git, &clock, Ok(CommitId("c2".into())), wanted)).unwrap();
    assert_eq!(r, Synced::New { read: 1 }, "a.md's blob is unchanged");
    assert_eq!(git.read.borrow().last().unwrap(), "b-beta two");
    assert_eq!(text(&t, "b.md"), "beta two");
    assert_eq!(text(&t, "a.md"), "alpha");
    assert_eq!(t.status()["commit"], "c2");
    let trees = git.trees.get();
    clock.0.set(3_000);
    let r = block(t.advance(&git, &clock, Ok(CommitId("c2".into())), |_| {
        unreachable!("no listing for the same commit")
    }));
    assert_eq!(r.unwrap(), Synced::Same);
    assert_eq!(git.trees.get(), trees, "the same commit lists no tree");
    assert_eq!(git.read.borrow().len(), 3, "and reads no blob");
    assert_eq!(
        t.status()["last_sync_ms"],
        3_000,
        "a no-op tick is still a successful sync"
    );
}

#[test]
fn a_failed_fetch_or_read_keeps_the_last_good_commit_and_says_why_until_a_tick_succeeds() {
    let (git, clock, mut t) = (
        FakeGit::default(),
        Manual(Cell::new(1_000)),
        Tracker::new(cfg()),
    );
    git.commit("c1", &[("a.md", "alpha"), ("b.md", "beta")]);
    block(t.advance(&git, &clock, Ok(CommitId("c1".into())), wanted)).unwrap();
    clock.0.set(5_000);
    let e = error!(
        ErrorCode::LatlasRepoFetch,
        "origin main", "could not resolve host"
    );
    let r = block(t.advance(&git, &clock, Err(e), wanted));
    assert_eq!(r.unwrap_err().code(), ErrorCode::LatlasRepoFetch);
    assert_eq!(
        t.commit(),
        Some(&CommitId("c1".into())),
        "the last good commit is kept"
    );
    assert!(
        t.error().unwrap().starts_with("LatlasRepoFetch: "),
        "{:?}",
        t.error()
    );
    assert_eq!(t.status()["last_sync_ms"], 1_000, "a failure is not a sync");
    let r = block(t.advance(&git, &clock, Ok(CommitId("gone".into())), wanted));
    assert_eq!(r.unwrap_err().code(), ErrorCode::LatlasRepoRead);
    assert_eq!(text(&t, "a.md"), "alpha");
    let r = block(t.advance(&git, &clock, Ok(CommitId("c1".into())), wanted));
    assert_eq!(r.unwrap(), Synced::Same);
    assert_eq!(t.error(), None, "a successful tick clears the error");
}

#[test]
fn a_config_without_the_section_is_off_and_a_bad_section_is_refused() {
    let base = "schema = \"s\"\n[sources]\n[limits]\nmax_rows = 1\ncypher_timeout_ms = 1\ntoken_budget = 1\nstatus_tokens = 1\nsearch_deadline_ms = 1\n[store]\ndata_dir = \"d\"\nscratch_dir = \"s\"\n[search]\nw_cosine = 0.0\nw_hops = 0.0\nw_kind = 0.0\n[validator]\npython = \"p\"\n";
    let parse = |t: &str| toml::from_str::<Config<toml::Table>>(t);
    assert_eq!(parse(base).unwrap().repo_track, None, "no section: off");
    let on = format!(
        "{base}[repo_track]\nrepo = \"src\"\nremote = \"origin\"\nref = \"main\"\ninterval_s = 300\nfetch_timeout_s = 60\n"
    );
    assert_eq!(parse(&on).unwrap().repo_track, Some(cfg()));
    assert!(
        parse(&on.replace("interval_s = 300\n", "")).is_err(),
        "every field is required"
    );
    assert_eq!(status_off()["tracking"], false);
    assert!(
        status_off()["reason"]
            .as_str()
            .unwrap()
            .contains("no [repo_track]")
    );
    for (field, bad) in [
        ("ref", "-upload-pack=x"),
        ("ref", "a..b"),
        ("remote", ""),
        ("repo", "/abs"),
    ] {
        let mut c = cfg();
        match field {
            "ref" => c.git_ref = bad.into(),
            "remote" => c.remote = bad.into(),
            _ => c.repo = bad.into(),
        }
        let e = c.validate().unwrap_err();
        assert_eq!(e.code(), ErrorCode::LatlasConfig, "{field}={bad}");
    }
    assert!(cfg().validate().is_ok());
}
