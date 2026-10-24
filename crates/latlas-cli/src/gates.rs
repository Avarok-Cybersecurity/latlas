// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The pack-independent gates as commands: `derive` (the bundle JSONL), `check` (derive
//! twice ⇒ one digest; fatal findings; two fresh stores ⇒ one export; reopen ⇒ the same export;
//! cold start; the default status of every scope within its token budget) and `probes` (the
//! recall gate). Each prints one JSON verdict and fails the process on any mismatch.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp `gates.rs`, 2026-10-23 K0).
//! Invariants: the gates never write a source file; stores are created under `store.scratch_dir`.

use crate::app::{App, derive_once, open_hosts};
use crate::host::PackHost;
use anyhow::Result;
use latlas_core::Pack;
use latlas_core::jsonl::{digest, render};
use latlas_host_native::clock::SystemClock;
use latlas_index::store::Store;
use latlas_ports::Clock;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 2026-10-23: A gate whose report is already on stdout failed; the binary exits 1 without
/// printing a second JSON document.
#[derive(Debug)]
pub struct GateFailed(pub &'static str);

impl std::fmt::Display for GateFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed (report on stdout)", self.0)
    }
}

impl std::error::Error for GateFailed {}

/// 2026-10-23: `derive`: the bundle JSONL to stdout or a file.
pub fn derive_cmd<P: Pack, H: PackHost<P>>(cfg_path: &Path, out: Option<&Path>) -> Result<()> {
    let (cfg, fs, host) = open_hosts::<P, H>(cfg_path)?;
    let (_, b, _) = derive_once::<P, H>(&cfg, &fs, &host)?;
    let text = render(&b);
    match out {
        Some(p) => std::fs::write(p, text)?,
        None => print!("{text}"),
    }
    Ok(())
}

fn fresh_dir(base: &str, name: &str) -> Result<PathBuf> {
    let d = PathBuf::from(base).join(name);
    if d.exists() {
        std::fs::remove_dir_all(&d)?;
    }
    Ok(d)
}

/// 2026-10-23: `check`.
pub async fn check_cmd<P: Pack, H: PackHost<P>>(cfg_path: &Path, roundtrip: bool) -> Result<()> {
    let clock = SystemClock;
    let (cfg, fs, host) = open_hosts::<P, H>(cfg_path)?;
    let t0 = clock.monotonic_ns().0;
    let (_, a, source_commit) = derive_once::<P, H>(&cfg, &fs, &host)?;
    let t1 = clock.monotonic_ns().0;
    let (_, b, _) = derive_once::<P, H>(&cfg, &fs, &host)?;
    let (da, db) = (digest(&a).hex(), digest(&b).hex());
    let mut by_code: BTreeMap<&str, u32> = BTreeMap::new();
    for f in &a.errors {
        *by_code.entry(f.code).or_default() += 1;
    }
    let fatal: Vec<Value> = a
        .fatal()
        .iter()
        .map(|f| json!([f.code, f.at, f.msg]))
        .collect();
    let mut v = json!({
        "digest": da, "digest_again": db, "fixpoint": da == db,
        "counts": {"nodes": a.nodes.len(), "edges": a.edges.len(), "texts": a.texts.len(), "sources": a.sources.len()},
        "findings_by_code": by_code, "fatal": fatal, "derive_ms": (t1 - t0) / 1_000_000,
        "source_commit": source_commit, "pack": format!("{} {}", P::ID, P::VERSION),
    });
    let mut ok = da == db && fatal.is_empty();
    if roundtrip {
        let s0 = clock.monotonic_ns().0;
        let dir_a = fresh_dir(&cfg.store.scratch_dir, "kg-check-a")?;
        let mut sa = Store::open::<P>(&dir_a).await?;
        sa.load(&a).await?;
        let cold_ms = (clock.monotonic_ns().0 - s0) / 1_000_000 + (t1 - t0) / 1_000_000;
        let ea = sa.export()?;
        drop(sa);
        let mut sb = Store::open::<P>(&fresh_dir(&cfg.store.scratch_dir, "kg-check-b")?).await?;
        sb.load(&b).await?;
        let eb = sb.export()?;
        let mut sa2 = Store::open::<P>(&dir_a).await?;
        let rewrote = sa2.load(&a).await?;
        let ea2 = sa2.export()?;
        let same = ea == eb && ea == ea2 && !rewrote;
        ok &= same && cold_ms < 2000;
        v["roundtrip"] = json!({
            "fresh_stores_identical": ea == eb, "reopen_identical": ea == ea2, "reopen_rewrote": rewrote,
            "export_lines": ea.lines().count(), "export_sha256": latlas_core::hash::sha256(ea.as_bytes()).hex(),
            "cold_start_ms": cold_ms, "vector_size": 1,
        });
    }
    let budget = status_budget::<P, H>(cfg_path).await?;
    ok &= budget.iter().all(|b| b["within"] == true);
    v["status_budget"] = Value::from(budget);
    println!("{}", serde_json::to_string_pretty(&v)?);
    if !ok {
        return Err(GateFailed("check").into());
    }
    Ok(())
}

/// 2026-10-23: A token estimate that errs high for JSON of ids and short strings (~3 bytes a
/// token; English prose runs nearer 4).
pub fn tokens_estimate(bytes: usize) -> usize {
    bytes.div_ceil(3)
}

/// 2026-10-23: The default `la_status` of every scope, exactly as the tool answers it (envelope
/// included, compact JSON as MCP sends it), against `limits.status_tokens`.
async fn status_budget<P: Pack, H: PackHost<P>>(cfg_path: &Path) -> Result<Vec<Value>> {
    let (cfg, _, _) = open_hosts::<P, H>(cfg_path)?;
    let dir = fresh_dir(&cfg.store.scratch_dir, "kg-check-status")?;
    let mut app = App::<P, H>::open(cfg_path, Some(&dir)).await?;
    let mut out = Vec::new();
    for scope in P::scopes(&cfg.sources) {
        let r = crate::handlers::call(&mut app, "la_status", &json!({ "scope": scope })).await?;
        let bytes = serde_json::to_string(&r)?.len();
        let tokens = tokens_estimate(bytes);
        out.push(json!({"scope": scope, "bytes": bytes, "tokens_est": tokens,
            "budget": cfg.limits.status_tokens, "within": tokens <= cfg.limits.status_tokens as usize}));
    }
    Ok(out)
}

/// 2026-10-23: The recall gate: every non-comment `probe<TAB>id` line of `file` is an
/// `la_search`; a hit is the id in the top 3. Prints each probe's rank and mode, then the count;
/// fails under `need` hits or when any result's mode is not the one asked for.
pub async fn probes_cmd<P: Pack, H: PackHost<P>>(
    cfg_path: &Path,
    data: Option<&Path>,
    file: &Path,
    graph_only: bool,
    need: u32,
) -> Result<()> {
    let text = std::fs::read_to_string(file)?;
    let mut app = App::<P, H>::open(cfg_path, data).await?;
    if graph_only {
        app.declare_graph_only("--graph-only (the recall gate's graph_only leg)");
    }
    let want_mode = if graph_only { "graph_only" } else { "hybrid" };
    if !graph_only {
        crate::search::warm_blocking(&app).await?;
    }
    let (mut hits, mut total, mut modes_ok) = (0u32, 0u32, true);
    let mut rows = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((probe, id)) = line.split_once('\t') else {
            anyhow::bail!("probe line without a tab: {line}");
        };
        total += 1;
        let v =
            crate::handlers::call(&mut app, "la_search", &json!({"text": probe, "k": 10})).await?;
        let ids: Vec<&str> = v["hits"]
            .as_array()
            .map(|a| a.iter().filter_map(|h| h["id"].as_str()).collect())
            .unwrap_or_default();
        let rank = ids.iter().position(|x| *x == id).map(|r| r + 1);
        let hit = rank.is_some_and(|r| r <= 3);
        hits += u32::from(hit);
        let mode = v["mode"].as_str().unwrap_or("");
        modes_ok &= mode == want_mode;
        rows.push(
            json!({"probe": probe, "want": id, "rank": rank, "hit": hit, "mode": mode,
            "mode_reason": v.get("mode_reason"), "top3": ids.iter().take(3).collect::<Vec<_>>()}),
        );
    }
    let ok = hits >= need && modes_ok;
    let v = json!({"mode": want_mode, "hits": hits, "probes": total, "need": need,
        "modes_ok": modes_ok, "pass": ok, "rows": rows});
    println!("{}", serde_json::to_string_pretty(&v)?);
    if !ok {
        return Err(GateFailed("recall probes").into());
    }
    Ok(())
}
