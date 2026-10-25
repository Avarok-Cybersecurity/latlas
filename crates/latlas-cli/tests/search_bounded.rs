// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `la_search` answers in bounded time whatever the embedder does (the 2026-10-23
//! defect: a cold cache made the first search embed 1311 texts inline, ~2.5 min under the app
//! lock). The embedder is the production OpenRouter host pointed at a loopback HTTP endpoint
//! whose replies the test scripts (the only fake: the remote provider itself); the pack is a
//! one-label test pack, so the App, the store, the warmer and the search are all production code.

mod support;

use latlas_cli::app::App;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use support::{Reply, Tiny, TinyHost, project, provider};

async fn open(cfg: &Path, data: &Path) -> App<Tiny, TinyHost> {
    App::<Tiny, TinyHost>::open(cfg, Some(data)).await.unwrap()
}

async fn search(app: &mut App<Tiny, TinyHost>) -> (Value, Duration) {
    let t0 = Instant::now();
    let v = latlas_cli::handlers::call(app, "la_search", &json!({"text": "alpha kernel"}))
        .await
        .unwrap();
    (v, t0.elapsed())
}

/// 2026-10-23: Poll searches until `done(v)` (bounded), asserting each one answers within
/// `bound`; returns the last answer.
async fn until(
    app: &mut App<Tiny, TinyHost>,
    bound: Duration,
    done: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..100 {
        let (v, took) = search(app).await;
        assert!(took < bound, "a search took {took:?}: {v}");
        if done(&v) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never reached the expected state");
}

fn reason(v: &Value) -> &str {
    v["mode_reason"].as_str().unwrap_or("")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cold_cache_search_answers_graph_only_at_once_and_the_warmed_cache_persists() {
    let (port, _) = provider(|_, _| Reply::After(300)).await;
    let (cfg, data) = project("cold", port, 3000, 2000);
    let mut app = open(&cfg, &data).await;
    let (v, took) = search(&mut app).await;
    assert!(
        took < Duration::from_millis(250),
        "the cold search waited {took:?}"
    );
    assert_eq!(v["mode"], "graph_only", "{v}");
    assert!(reason(&v).starts_with("embedding index warming: 0/"), "{v}");
    assert!(
        !v["hits"].as_array().unwrap().is_empty(),
        "graph_only still answers: {v}"
    );
    let v = until(&mut app, Duration::from_millis(1200), |v| {
        v["mode"] == "hybrid"
    })
    .await;
    assert_eq!(v["cache"]["pending"], 0, "{v}");
    drop(app);
    let mut app = open(&cfg, &data).await;
    let (v, _) = search(&mut app).await;
    assert_eq!(v["mode"], "hybrid", "the warmed cache did not persist: {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_warmer_keeps_every_batch_it_finished_and_says_why() {
    let (port, seen) = provider(|kind, nth| match (kind, nth) {
        ("passage", 0) => Reply::After(10),
        _ => Reply::Hang,
    })
    .await;
    let (cfg, data) = project("stopped", port, 600, 2000);
    let mut app = open(&cfg, &data).await;
    let v = until(&mut app, Duration::from_millis(250), |v| {
        reason(v).contains("the last run stopped")
    })
    .await;
    assert_eq!(v["mode"], "graph_only", "{v}");
    assert_eq!(v["cache"]["cached"], 1, "the finished batch was lost: {v}");
    assert!(reason(&v).starts_with("embedding index warming: 1/"), "{v}");
    assert!(seen.load(Ordering::SeqCst) >= 2, "the warmer did not retry");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_query_embedding_that_hangs_answers_graph_only_at_the_deadline() {
    let (port, _) = provider(|kind, _| match kind {
        "query" => Reply::Hang,
        _ => Reply::After(10),
    })
    .await;
    let (cfg, data) = project("deadline", port, 3000, 300);
    let mut app = open(&cfg, &data).await;
    latlas_cli::search::warm_blocking(&app).await.unwrap();
    let (v, took) = search(&mut app).await;
    assert!(
        took >= Duration::from_millis(300) && took < Duration::from_millis(1500),
        "the search took {took:?}"
    );
    assert_eq!(v["mode"], "graph_only", "{v}");
    assert!(reason(&v).contains("limits.search_deadline_ms 300"), "{v}");
}
