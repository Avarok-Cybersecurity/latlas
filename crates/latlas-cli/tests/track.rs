// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: `[repo_track]` end to end through the production App, file host and git host,
//! over real repositories in a temp dir (a bare "remote" and the configured checkout): sources are
//! served from the fetched commit, never from the checkout (whose uncommitted edit is neither read
//! nor touched); a new commit re-derives, the same commit is no work, a failed fetch keeps the last
//! good commit as `stale_reason`; tracking continues while paused; no section means off.

mod support;

use latlas_cli::app::App;
use latlas_cli::handlers::call;
use latlas_cli::track::sync;
use latlas_core::pause::PAUSE_FILE;
use latlas_core::track::{GitPort as _, Synced};
use latlas_error::ErrorCode;
use latlas_host_native::pause::LockFile;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::{Reply, Tiny, TinyHost, project, provider};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 2026-10-25: Commit `files` in the `pusher` clone and push it; returns the commit.
fn push(pusher: &Path, files: &[(&str, &str)]) -> String {
    for (f, t) in files {
        std::fs::write(pusher.join(f), t).unwrap();
    }
    git(pusher, &["add", "-A"]);
    git(pusher, &["commit", "-qm", "c"]);
    git(pusher, &["push", "-q", "origin", "HEAD:main"]);
    git(pusher, &["rev-parse", "HEAD"])
}

struct Rig {
    dir: PathBuf,
    cfg: PathBuf,
    data: PathBuf,
    pusher: PathBuf,
}

async fn rig(name: &str, track: bool) -> Rig {
    let (port, _) = provider(|_, _| Reply::After(5)).await;
    let (cfg, data) = project(&format!("track-{name}"), port, 3000, 2000);
    let dir = cfg.parent().unwrap().to_path_buf();
    let remote = dir.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    git(&dir, &["clone", "-q", remote.to_str().unwrap(), "pusher"]);
    let pusher = dir.join("pusher");
    push(
        &pusher,
        &[
            ("a.md", "alpha kernel"),
            ("b.md", "beta lever"),
            ("c.md", "gamma ledger"),
        ],
    );
    git(&dir, &["clone", "-q", remote.to_str().unwrap(), "src"]);
    let mut text = std::fs::read_to_string(&cfg).unwrap().replace(
        r#"files = ["a.md", "b.md", "c.md"]"#,
        r#"files = ["src/a.md", "src/b.md", "src/c.md"]"#,
    );
    if track {
        text.push_str("[repo_track]\nrepo = \"src\"\nremote = \"origin\"\nref = \"main\"\ninterval_s = 300\nfetch_timeout_s = 30\n");
    }
    std::fs::write(&cfg, text).unwrap();
    Rig {
        dir,
        cfg,
        data,
        pusher,
    }
}

async fn open(r: &Rig) -> App<Tiny, TinyHost> {
    let lock = LockFile::at(r.dir.join(PAUSE_FILE));
    App::<Tiny, TinyHost>::open_with(&r.cfg, Some(&r.data), lock)
        .await
        .unwrap()
}

async fn tick(app: &mut App<Tiny, TinyHost>) -> latlas_error::Result<Synced> {
    let ctl = app.track.as_ref().unwrap();
    let fetched = ctl.git.fetch("origin", "main").await;
    sync(app, fetched).await
}

fn body(app: &App<Tiny, TinyHost>, path: &str) -> String {
    app.bundle
        .texts
        .iter()
        .find(|t| t.node.to_string().ends_with(path))
        .and_then(|t| t.text.split_once("body: ").map(|(_, b)| b.to_string()))
        .unwrap_or_default()
}

async fn freshness(app: &mut App<Tiny, TinyHost>) -> Value {
    let v = call(
        app,
        "la_status",
        &json!({"scope": "t", "sections": ["freshness"]}),
    )
    .await
    .unwrap();
    v["freshness"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fetched_commit_is_served_incrementally_and_the_checkout_is_never_read_or_touched() {
    let r = rig("serve", true).await;
    let src = r.dir.join("src");
    std::fs::write(src.join("a.md"), "LOCAL EDIT").unwrap();
    let head = git(&src, &["rev-parse", "HEAD"]);
    let mut app = open(&r).await;
    assert_eq!(
        body(&app, "src/a.md"),
        "LOCAL EDIT",
        "before the first tick the checkout is served"
    );
    assert_eq!(tick(&mut app).await.unwrap(), Synced::New { read: 3 });
    assert_eq!(
        body(&app, "src/a.md"),
        "alpha kernel",
        "the commit, not the uncommitted edit"
    );
    let digest = app.digest.clone();
    assert_eq!(
        tick(&mut app).await.unwrap(),
        Synced::Same,
        "the same commit is no work"
    );
    assert_eq!(app.digest, digest);
    let c2 = push(&r.pusher, &[("b.md", "beta lever, revised")]);
    assert_eq!(
        tick(&mut app).await.unwrap(),
        Synced::New { read: 1 },
        "only b.md's blob changed"
    );
    assert_eq!(body(&app, "src/b.md"), "beta lever, revised");
    let f = freshness(&mut app).await;
    assert_eq!(f["source_commit"], c2.as_str(), "{f}");
    assert_eq!(f["stale_reason"], Value::Null, "{f}");
    assert_eq!(f["repo_track"]["commit"], c2.as_str(), "{f}");
    assert!(f["repo_track"]["last_sync_ms"].as_i64().unwrap() > 0, "{f}");
    assert_eq!(
        std::fs::read_to_string(src.join("a.md")).unwrap(),
        "LOCAL EDIT",
        "the checkout is untouched"
    );
    assert_eq!(
        git(&src, &["rev-parse", "HEAD"]),
        head,
        "its HEAD did not move"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_fetch_is_the_stale_reason_and_the_last_good_commit_is_kept() {
    let r = rig("fail", true).await;
    let mut app = open(&r).await;
    tick(&mut app).await.unwrap();
    let good = freshness(&mut app).await["source_commit"].clone();
    let failed = app
        .track
        .as_ref()
        .unwrap()
        .git
        .fetch("nowhere", "main")
        .await;
    let e = sync(&mut app, failed).await.unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasRepoFetch, "{e}");
    let f = freshness(&mut app).await;
    assert!(
        f["stale_reason"]
            .as_str()
            .unwrap()
            .starts_with("LatlasRepoFetch: "),
        "{f}"
    );
    assert_eq!(
        f["source_commit"], good,
        "the last good commit is still served: {f}"
    );
    assert_eq!(body(&app, "src/a.md"), "alpha kernel");
    assert_eq!(tick(&mut app).await.unwrap(), Synced::Same);
    assert_eq!(
        freshness(&mut app).await["stale_reason"],
        Value::Null,
        "a good tick clears it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tracking_continues_while_paused_and_no_section_is_off() {
    let r = rig("paused", true).await;
    std::fs::write(r.dir.join(PAUSE_FILE), b"").unwrap();
    let mut app = open(&r).await;
    assert!(matches!(tick(&mut app).await.unwrap(), Synced::New { .. }));
    assert_eq!(
        body(&app, "src/c.md"),
        "gamma ledger",
        "derived while paused"
    );
    let v = call(&mut app, "la_search", &json!({"text": "gamma"}))
        .await
        .unwrap();
    assert_eq!(v["mode_reason"], "paused: latlas.lock present", "{v}");
    assert_eq!(
        v["cache"]["warming"], false,
        "no embedding while paused: {v}"
    );
    let off = rig("off", false).await;
    let mut app = open(&off).await;
    let f = freshness(&mut app).await;
    assert_eq!(f["repo_track"]["tracking"], false, "{f}");
    assert!(app.track.is_none());
}
