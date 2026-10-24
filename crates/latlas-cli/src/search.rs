// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `la_search` (design §3 tool 2, P2): a background warmer brings the embedding cache
//! up to date with the bundle's texts (only new shas are sent, each batch persisted as it lands);
//! a search embeds the query and ranks with `latlas_core::search`. While the cache is incomplete,
//! or on any embedder failure, the search answers `graph_only` with the reason, for this call
//! only; the declared mode is unchanged.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants:
//! - Every result names its mode and, for `graph_only`, the reason.
//! - The query text is sent to the embedder only in `hybrid` mode and is never logged or cached.
//! - A search never waits for the warmer (it answers `embedding index warming: n/m`), and the
//!   query's embedding is cut off at `limits.search_deadline_ms` (2026-10-23: a cold cache made
//!   the first search embed 1311 texts inline, minutes under the app lock).

use crate::app::App;
use crate::host::PackHost;
use latlas_core::Pack;
use latlas_core::embed::{Embedder, Mode, Plan, Role, cosine, embed_all, missing};
use latlas_core::hash::Sha256;
use latlas_core::id::{NodeId, label_by_name};
use latlas_core::search::{lexical, named, rank};
use latlas_core::tools::{DEFAULT_SEARCH_HOPS, DEFAULT_SEARCH_K};
use latlas_error::{ErrorCode, Result, error};
use latlas_host_native::clock::{SystemClock, TokioTimer};
use latlas_host_native::openrouter::OpenRouter;
use latlas_index::textstore::TextStore;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

/// 2026-10-23: The embedder as the binary chose it.
pub enum Embed {
    Off(String),
    On {
        host: OpenRouter,
        /// 2026-10-23: Shared with the background warmer, which persists each batch as it lands.
        cache: Arc<Mutex<TextStore>>,
        plan: Plan,
        warm: Arc<StdMutex<Warm>>,
    },
}

/// 2026-10-23: The background warmer's state.
#[derive(Debug, Default)]
pub struct Warm {
    /// 2026-10-23: A warmer task is in flight (at most one per process).
    pub running: bool,
    /// 2026-10-23: Texts the provider refused one by one, with why; not re-sent by this process
    /// (a restart retries them).
    pub refused: BTreeMap<Sha256, String>,
    /// 2026-10-23: Why the last warmer run stopped before the cache was complete.
    pub stopped: Option<String>,
}

fn warm_of(w: &StdMutex<Warm>) -> std::sync::MutexGuard<'_, Warm> {
    w.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn store_err(e: impl core::fmt::Display) -> latlas_error::Error {
    error!(ErrorCode::LatlasStore, "kg_text", e.to_string())
}

/// 2026-10-23: The answer while the cache lacks texts: `None` once every text is cached or
/// refused, else the declared reason a search answers `graph_only` with.
pub fn warming_reason(
    cached: usize,
    total: usize,
    todo: usize,
    stopped: Option<&str>,
) -> Option<String> {
    if todo == 0 {
        return None;
    }
    let mut r = format!("embedding index warming: {cached}/{total} texts cached");
    if let Some(s) = stopped {
        r.push_str(&format!(" (the last run stopped: {s}; retrying)"));
    }
    Some(r)
}

/// 2026-10-23: Embed `todo` one batch at a time, persisting each batch before the next is sent,
/// so a stopped or killed run keeps its progress. Never holds the app.
async fn warm_run(
    host: OpenRouter,
    cache: Arc<Mutex<TextStore>>,
    plan: Plan,
    warm: Arc<StdMutex<Warm>>,
    todo: Vec<(Sha256, String)>,
) -> Result<usize> {
    let mut n = 0;
    let mut res = Ok(());
    for chunk in todo.chunks(plan.batch.max(1)) {
        let got = embed_all(
            &host,
            &SystemClock,
            &TokioTimer,
            &plan,
            Role::Document,
            chunk,
        )
        .await;
        warm_of(&warm).refused.extend(got.refused);
        n += got.done.len();
        if let Err(e) = cache.lock().await.put(got.done).await.map_err(store_err) {
            res = Err(e);
            break;
        }
        if let Some(e) = got.stopped {
            res = Err(e);
            break;
        }
    }
    let mut w = warm_of(&warm);
    w.running = false;
    w.stopped = res
        .as_ref()
        .err()
        .map(|e| format!("{}: {e}", e.code().name()));
    res.map(|()| n)
}

/// 2026-10-23: What the warmer still has to send, and the counts a result reports.
async fn todo_of<P: Pack, H: PackHost<P>>(
    app: &App<P, H>,
) -> Result<(Vec<(Sha256, String)>, usize, usize)> {
    let Embed::On { cache, warm, .. } = &app.embed else {
        return Ok((Vec::new(), 0, 0));
    };
    let shas: BTreeSet<Sha256> = app.bundle.texts.iter().map(|t| t.sha).collect();
    let mut skip = cache.lock().await.cached().map_err(store_err)?;
    let cached = skip.intersection(&shas).count();
    skip.extend(warm_of(warm).refused.keys().copied());
    let todo = missing(
        app.bundle.texts.iter().map(|t| (t.sha, t.text.as_str())),
        &skip,
    );
    Ok((todo, cached, shas.len()))
}

/// 2026-10-23: Start the background warmer when the cache lacks texts and none is running; never
/// waits for it. Returns the reason a search answers `graph_only` until the cache is complete.
pub async fn start_warming<P: Pack, H: PackHost<P>>(app: &App<P, H>) -> Result<Option<String>> {
    let (todo, cached, total) = todo_of(app).await?;
    let Embed::On {
        host,
        cache,
        plan,
        warm,
    } = &app.embed
    else {
        return Ok(None);
    };
    let mut w = warm_of(warm);
    let reason = warming_reason(cached, total, todo.len(), w.stopped.as_deref());
    if reason.is_some() && !w.running {
        w.running = true;
        tokio::spawn(warm_run(
            host.clone(),
            cache.clone(),
            *plan,
            warm.clone(),
            todo,
        ));
    }
    Ok(reason)
}

/// 2026-10-23: Bring the cache up to date and wait for it (the recall gate, which must measure
/// `hybrid`); the MCP tools never call this. Returns how many texts were embedded.
pub async fn warm_blocking<P: Pack, H: PackHost<P>>(app: &App<P, H>) -> Result<usize> {
    let (todo, _, _) = todo_of(app).await?;
    let Embed::On {
        host,
        cache,
        plan,
        warm,
    } = &app.embed
    else {
        return Ok(0);
    };
    warm_of(warm).running = true;
    warm_run(host.clone(), cache.clone(), *plan, warm.clone(), todo).await
}

async fn query_vector(host: &OpenRouter, text: &str, plan: &Plan) -> Result<Vec<f32>> {
    let todo = [(latlas_core::hash::record_sha(text), text.to_string())];
    let mut got = embed_all(host, &SystemClock, &TokioTimer, plan, Role::Query, &todo).await;
    match (got.done.pop(), got.stopped, got.refused.pop()) {
        (_, Some(e), _) => Err(e),
        (Some((_, v)), None, _) => Ok(v),
        (None, None, Some((_, why))) => Err(error!(
            ErrorCode::LatlasEmbedProtocol,
            format!("the provider refused the query: {why}")
        )),
        (None, None, None) => Err(error!(
            ErrorCode::LatlasEmbedProtocol,
            "the query got no vector"
        )),
    }
}

/// 2026-10-23: Per-node cosine similarity of `q` to the node's cached texts.
fn cosines<P: Pack, H: PackHost<P>>(
    app: &App<P, H>,
    vecs: &BTreeMap<Sha256, Vec<f32>>,
    q: &[f32],
) -> BTreeMap<NodeId, f64> {
    let mut out: BTreeMap<NodeId, f64> = BTreeMap::new();
    for t in &app.bundle.texts {
        let Some(v) = vecs.get(&t.sha) else { continue };
        let s = cosine(q, v);
        let e = out.entry(t.node.clone()).or_insert(f64::MIN);
        if s > *e {
            *e = s;
        }
    }
    out
}

fn arg_range(a: &Value, k: &str, default: u32, lo: u32, hi: u32) -> Result<u32> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (lo..=hi).contains(n))
            .ok_or_else(|| {
                error!(
                    ErrorCode::LatlasToolArgument,
                    k.to_string(),
                    format!("an integer in {lo}..={hi}")
                )
            }),
    }
}

/// 2026-10-23: The tool.
pub async fn la_search<P: Pack, H: PackHost<P>>(
    app: &mut App<P, H>,
    args: &Value,
) -> Result<Value> {
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| error!(ErrorCode::LatlasToolArgument, "text", "required string"))?
        .to_string();
    let mut kinds = Vec::new();
    for k in args
        .get("kinds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = k.as_str().unwrap_or("");
        kinds.push(label_by_name::<P>(name).ok_or_else(|| {
            error!(
                ErrorCode::LatlasToolArgument,
                "kinds",
                format!("no label {name}")
            )
        })?);
    }
    let k = arg_range(args, "k", DEFAULT_SEARCH_K, 1, 50)?;
    let hops = arg_range(args, "hops", DEFAULT_SEARCH_HOPS, 0, 2)?;
    let mut sims = lexical(&app.bundle, &text);
    let (mode, reason) = match &app.embed {
        Embed::Off(r) => ("graph_only", Some(r.clone())),
        Embed::On { host, .. } => match host.mode() {
            Mode::Unavailable { reason } => ("graph_only", Some(reason)),
            Mode::Available { .. } => match hybrid(app, &text).await {
                Ok(cos) => {
                    for (id, c) in cos {
                        let e = sims.entry(id).or_insert(0.0);
                        if c > *e {
                            *e = c;
                        }
                    }
                    ("hybrid", None)
                }
                Err(Degraded::Warming(r)) => ("graph_only", Some(r)),
                Err(Degraded::Failed(e)) => {
                    ("graph_only", Some(format!("{}: {e}", e.code().name())))
                }
            },
        },
    };
    let names = named::<P>(&app.bundle, &text);
    let hits = rank::<P>(
        &app.bundle,
        &sims,
        &names,
        &kinds,
        usize::try_from(k).unwrap_or(10),
        hops,
        &app.cfg.search,
    );
    let hits: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            let mut o = Map::new();
            o.insert("id".into(), Value::String(h.id.to_string()));
            o.insert(
                "kind".into(),
                Value::String(h.id.label().name().to_string()),
            );
            o.insert(
                "score".into(),
                Value::from((h.score * 1000.0).round() / 1000.0),
            );
            o.insert("snippet".into(), Value::String(h.snippet));
            o.insert("src".into(), Value::String(h.src));
            o.insert("via".into(), Value::from(h.via));
            Value::Object(o)
        })
        .collect();
    let mut o = Map::new();
    o.insert("mode".into(), Value::String(mode.to_string()));
    if let Some(r) = reason {
        o.insert("mode_reason".into(), Value::String(r));
    }
    o.insert("hits".into(), Value::from(hits));
    if let Embed::On { warm, .. } = &app.embed {
        let (todo, cached, total) = todo_of(app).await?;
        let w = warm_of(warm);
        let mut c = Map::new();
        c.insert("texts".into(), Value::from(total));
        c.insert("cached".into(), Value::from(cached));
        c.insert("refused".into(), Value::from(w.refused.len()));
        c.insert("pending".into(), Value::from(todo.len()));
        c.insert("warming".into(), Value::Bool(w.running));
        if let Some((_, why)) = w.refused.iter().next() {
            c.insert("first_refusal".into(), Value::String(why.clone()));
        }
        o.insert("cache".into(), Value::Object(c));
    }
    Ok(Value::Object(o))
}

/// 2026-10-23: Why a search did not rank by cosine.
enum Degraded {
    /// The cache is incomplete; the warmer runs in the background.
    Warming(String),
    Failed(latlas_error::Error),
}

/// 2026-10-23: Cosine similarities, or why not: never waits for the warmer, and the query's own
/// embedding is cut off at `limits.search_deadline_ms`.
async fn hybrid<P: Pack, H: PackHost<P>>(
    app: &App<P, H>,
    text: &str,
) -> core::result::Result<BTreeMap<NodeId, f64>, Degraded> {
    if let Some(r) = start_warming(app).await.map_err(Degraded::Failed)? {
        return Err(Degraded::Warming(r));
    }
    let Embed::On {
        host, cache, plan, ..
    } = &app.embed
    else {
        return Err(Degraded::Failed(error!(
            ErrorCode::LatlasEmbedderUnavailable,
            "no embedder"
        )));
    };
    let ms = app.cfg.limits.search_deadline_ms;
    let q = match tokio::time::timeout(
        std::time::Duration::from_millis(ms),
        query_vector(host, text, plan),
    )
    .await
    {
        Ok(r) => r.map_err(Degraded::Failed)?,
        Err(_) => {
            return Err(Degraded::Failed(error!(
                ErrorCode::LatlasEmbedderUnavailable,
                format!("the query embedding exceeded limits.search_deadline_ms {ms}")
            )));
        }
    };
    let vecs = cache
        .lock()
        .await
        .vectors()
        .map_err(|e| Degraded::Failed(store_err(e)))?;
    Ok(cosines(app, &vecs, &q))
}
