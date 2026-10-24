// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The request handler: `{"op": "derive"|"status"|"search"|"score", "config": <toml>,
//! "files": {path: text}, ...}` in, a JSON value out (`{"error": code, "message"}` on failure).
//!
//! Owner: latlas-web.
//! Invariants: every op derives from the request alone; `derived_at_ms` is the caller's
//! (`now_ms`), never read from a clock here.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_check::report::{Sides, json};
use latlas_check::{CheckConfig, CheckFacts, compare, score};
use latlas_core::config::{Config, resolve_repo_paths};
use latlas_core::jsonl::{digest, render};
use latlas_core::search::{lexical, named, rank};
use latlas_core::status::{Freshness, status};
use latlas_core::{Bundle, SourceSet, derive};
use latlas_decl::{Decl, DeclSources};
use latlas_error::{ErrorCode, Result, error};
use serde_json::{Map, Value, json};

/// 2026-10-24: The virtual repository root.
pub const REPO: &str = "/repo";
/// 2026-10-24: Where the page puts the config among the files.
pub const CONFIG_PATH: &str = ".latlas/config.toml";

fn arg<'a>(r: &'a Value, k: &str) -> Result<&'a str> {
    r.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| error!(ErrorCode::LatlasToolArgument, k.to_string(), "required string"))
}

fn open(r: &Value) -> Result<(Config<DeclSources>, Bundle)> {
    let raw = arg(r, "config")?;
    let text = resolve_repo_paths(raw, REPO)?.unwrap_or_else(|| raw.to_string());
    let mut cfg = Config::parse::<Decl>(&text)?;
    cfg.origin = CONFIG_PATH.to_string();
    let mut src = SourceSet::default();
    src.add(CONFIG_PATH, raw.to_string());
    if let Some(files) = r.get("files").and_then(Value::as_object) {
        for (p, t) in files {
            if let Some(t) = t.as_str() {
                src.add(p, t.to_string());
            }
        }
    }
    let b = derive::<Decl>(&cfg, &src, None)?;
    Ok((cfg, b))
}

fn op(r: &Value) -> Result<Value> {
    let (cfg, b) = open(r)?;
    let d = digest(&b).hex();
    Ok(match arg(r, "op")? {
        "derive" => json!({"digest": d, "nodes": b.nodes.len(), "edges": b.edges.len(),
            "texts": b.texts.len(), "fatal": b.fatal().len(), "jsonl": render(&b)}),
        "status" => {
            let fresh = Freshness {
                stale_reason: None,
                derived_at_ms: r.get("now_ms").and_then(Value::as_i64).ok_or_else(|| {
                    error!(ErrorCode::LatlasToolArgument, "now_ms", "required integer (the page's clock)")
                })?,
                source_commit: None,
                newest_change: None,
            };
            let mut v = status::<Decl>(&b, &cfg, "repo", &[], &fresh, None)?;
            if let Value::Object(o) = &mut v {
                o.insert("bundle_digest".into(), Value::String(d));
            }
            v
        }
        "search" => {
            let text = arg(r, "text")?;
            let k = r
                .get("k")
                .and_then(Value::as_u64)
                .map_or(latlas_core::tools::DEFAULT_SEARCH_K, |k| u32::try_from(k.min(50)).unwrap_or(50));
            let k = usize::try_from(k).unwrap_or(50);
            let sims = lexical(&b, text);
            let names = named::<Decl>(&b, text);
            let hits: Vec<Value> = rank::<Decl>(&b, &sims, &names, &[], k, 1, &cfg.search)
                .into_iter()
                .map(|h| json!({"id": h.id.to_string(), "score": h.score, "snippet": h.snippet, "src": h.src}))
                .collect();
            json!({"mode": "graph_only", "mode_reason": "the browser module embeds nothing", "hits": hits, "bundle_digest": d})
        }
        "score" => {
            let check = CheckConfig::parse(arg(r, "check")?)?;
            let facts = CheckFacts::default();
            let s = score::<Decl>(&b, None, &check, &facts)?;
            let cmp = compare(&check, None, &s, &b);
            let sides = Sides { base_digest: None, head_digest: &d, mode: "graph_only", packs: "decl 0.1.0" };
            json(&sides, &check, None, &s, &cmp)
        }
        other => return Err(error!(ErrorCode::LatlasToolArgument, "op", alloc::format!("unknown op {other}"))),
    })
}

/// 2026-10-24: Handle one JSON request; never panics on bad input.
pub fn handle(request: &str) -> String {
    let out = match serde_json::from_str::<Value>(request) {
        Ok(r) => op(&r),
        Err(e) => Err(error!(ErrorCode::LatlasToolArgument, "request", e.to_string())),
    };
    match out {
        Ok(v) => v.to_string(),
        Err(e) => {
            let mut m = Map::new();
            m.insert("error".into(), Value::String(e.code().name().into()));
            m.insert("message".into(), Value::String(e.to_string()));
            Value::Object(m).to_string()
        }
    }
}
