// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The pause end to end through the production App, warmer and search, over the test
//! pack and a scripted loopback provider (support/): a lock file present at open answers every
//! search `graph_only` with the pause's reason and sends nothing; a change recorded while paused
//! is embedded once the lock is gone; a dangling symlink does not pause. The poll is driven by
//! hand on a manual clock (no 5 s waits).

mod support;

use latlas_cli::app::App;
use latlas_cli::handlers::call;
use latlas_core::pause::{Edge, PAUSE_FILE};
use latlas_error::ErrorCode;
use latlas_host_native::pause::LockFile;
use latlas_ports::{Clock, MonoNs, UnixMs};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::{Reply, Tiny, TinyHost, project, provider};

struct Manual;

impl Clock for Manual {
    fn now_unix_ms(&self) -> UnixMs {
        UnixMs(42)
    }
    fn monotonic_ns(&self) -> MonoNs {
        MonoNs(0)
    }
}

const PAUSED: &str = "paused: latlas.lock present";

async fn rig(name: &str) -> (PathBuf, PathBuf, PathBuf, Arc<AtomicUsize>) {
    let (port, seen) = provider(|_, _| Reply::After(5)).await;
    let (cfg, data) = project(name, port, 3000, 2000);
    let lock = cfg.parent().unwrap().join(PAUSE_FILE);
    (cfg, data, lock, seen)
}

async fn open(cfg: &Path, data: &Path, lock: &Path) -> App<Tiny, TinyHost> {
    App::<Tiny, TinyHost>::open_with(cfg, Some(data), LockFile::at(lock.to_path_buf()))
        .await
        .unwrap()
}

async fn search(app: &mut App<Tiny, TinyHost>) -> Value {
    call(app, "la_search", &json!({"text": "alpha kernel"}))
        .await
        .unwrap()
}

async fn poll(app: &App<Tiny, TinyHost>) -> Option<Edge> {
    app.pause.poll(&Manual, app.embed.host()).await.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_at_open_answers_graph_only_with_the_reason_and_embeds_nothing() {
    let (cfg, data, lock, seen) = rig("paused").await;
    std::fs::write(&lock, b"").unwrap();
    let mut app = open(&cfg, &data, &lock).await;
    let v = search(&mut app).await;
    assert_eq!(v["mode"], "graph_only", "{v}");
    assert_eq!(v["mode_reason"], PAUSED, "{v}");
    assert_eq!(
        v["cache"]["warming"], false,
        "the warmer started while paused: {v}"
    );
    assert!(!v["hits"].as_array().unwrap().is_empty(), "{v}");
    let e = latlas_cli::search::warm_blocking(&app).await.unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasPaused, "{e}");
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "a batch was sent while paused"
    );
    let st = call(
        &mut app,
        "la_status",
        &json!({"scope": "t", "sections": ["freshness"]}),
    )
    .await
    .unwrap();
    assert_eq!(st["pause"]["paused"], true, "{st}");
    assert_eq!(st["pause"]["reason"], PAUSED, "{st}");
    assert_eq!(st["pause"]["signal"], lock.display().to_string(), "{st}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_recorded_while_paused_is_embedded_once_the_lock_is_gone() {
    let (cfg, data, lock, seen) = rig("resume").await;
    std::fs::write(&lock, b"").unwrap();
    let mut app = open(&cfg, &data, &lock).await;
    std::fs::write(cfg.parent().unwrap().join("a.md"), "alpha kernel, revised").unwrap();
    assert_eq!(search(&mut app).await["mode_reason"], PAUSED);
    assert_eq!(poll(&app).await, None, "still present");
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(poll(&app).await, Some(Edge::Resumed));
    assert_eq!(latlas_cli::search::warm_blocking(&app).await.unwrap(), 3);
    assert_eq!(
        seen.load(Ordering::SeqCst),
        3,
        "batch 1: three texts, one each"
    );
    let v = search(&mut app).await;
    assert_eq!(v["mode"], "hybrid", "{v}");
    assert_eq!(v["cache"]["pending"], 0, "{v}");
    std::fs::write(&lock, b"").unwrap();
    assert_eq!(poll(&app).await, Some(Edge::Paused));
    assert_eq!(
        search(&mut app).await["mode_reason"],
        PAUSED,
        "a pause after open"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dangling_symlink_is_no_pause() {
    let (cfg, data, lock, _) = rig("dangling").await;
    std::os::unix::fs::symlink(cfg.parent().unwrap().join("held"), &lock).unwrap();
    let mut app = open(&cfg, &data, &lock).await;
    let v = search(&mut app).await;
    assert!(
        v["mode_reason"]
            .as_str()
            .unwrap()
            .starts_with("embedding index warming"),
        "{v}"
    );
    assert_eq!(v["cache"]["warming"], true, "{v}");
}
