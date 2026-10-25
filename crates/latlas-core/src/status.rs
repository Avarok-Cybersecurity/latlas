// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: `la_status` sections as JSON rows (design §3 tool 5), computed from the bundle by
//! [`crate::queries`]. Tables, not prose.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants: an unknown section name is refused (`LatlasToolArgument`), never ignored.

use crate::bundle::Bundle;
use crate::config::Config;
use crate::id::{NodeId, labels, rels};
use crate::pack::{Pack, SectionCtx};
use crate::prelude::*;
use crate::queries;
use crate::record_schema;
use crate::tools::LIST_CAP;
use latlas_error::{ErrorCode, Result, error};
use serde_json::{Map, Value};

/// 2026-10-23: What the binary knows about freshness that the pure bundle cannot (stat results).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Freshness {
    /// 2026-10-23: Set when the last re-derive failed (then the bundle is the previous one).
    pub stale_reason: Option<String>,
    /// 2026-10-23: When the bundle was derived (unix ms, from the Clock port).
    pub derived_at_ms: i64,
    /// 2026-10-23: The LKB checkout's commit the bundle was derived from (`None`: no LKB source).
    pub source_commit: Option<String>,
    /// 2026-10-23: The most recently modified source file and when (unix ms), from the host's
    /// stat results.
    pub newest_change: Option<(String, i64)>,
}

/// 2026-10-23: Every source file outside the pack's source directories, as `[path, sha prefix]`.
fn files<P: Pack>(b: &Bundle, cfg: &Config<P::Sources>) -> Vec<Value> {
    let dirs: Vec<String> = P::source_dirs(&cfg.sources)
        .into_iter()
        .map(|d| format!("{d}/"))
        .collect();
    b.sources
        .iter()
        .filter(|s| !dirs.iter().any(|d| s.path.starts_with(d.as_str())))
        .map(|s| {
            Value::from(vec![
                Value::String(s.path.clone()),
                Value::String(s.sha.hex()[..12].to_string()),
            ])
        })
        .collect()
}

/// 2026-10-23: The default answer's form: every list longer than [`LIST_CAP`] keeps its first
/// items, and its total goes beside it (`<key>_total`; a section that is a list becomes
/// `{total, first}`), so nothing is dropped without saying how much.
pub fn compact(v: Value) -> Value {
    match v {
        Value::Array(a) => {
            let total = a.len();
            let first: Vec<Value> = a.into_iter().take(LIST_CAP).map(compact).collect();
            let mut o = Map::new();
            o.insert("total".into(), Value::from(total));
            o.insert("first".into(), Value::from(first));
            Value::Object(o)
        }
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, v) in o {
                match v {
                    Value::Array(a) if a.len() > LIST_CAP => {
                        out.insert(format!("{k}_total"), Value::from(a.len()));
                        let first: Vec<Value> =
                            a.into_iter().take(LIST_CAP).map(compact_item).collect();
                        out.insert(k, Value::from(first));
                    }
                    Value::Array(a) => {
                        out.insert(
                            k,
                            Value::from(a.into_iter().map(compact_item).collect::<Vec<_>>()),
                        );
                    }
                    other => {
                        out.insert(k, compact_item(other));
                    }
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// 2026-10-23: [`compact`] below the top level: an inner list keeps the same cap.
fn compact_item(v: Value) -> Value {
    match v {
        Value::Object(_) => compact(v),
        Value::Array(a) if a.len() > LIST_CAP => {
            let mut keep: Vec<Value> = a.into_iter().take(LIST_CAP).map(compact_item).collect();
            keep.push(Value::String("…".into()));
            Value::from(keep)
        }
        Value::Array(a) => Value::from(a.into_iter().map(compact_item).collect::<Vec<_>>()),
        other => other,
    }
}

/// 2026-10-23: Ids as a JSON list of strings.
pub fn ids(v: &[crate::id::NodeId]) -> Value {
    Value::from(v.iter().map(ToString::to_string).collect::<Vec<_>>())
}

/// 2026-10-23: The sections the core answers for every pack, in schema order (2026-10-24:
/// `record_schema`, what `la_record` accepts).
pub const CORE_SECTIONS: &[&str] = &[
    "dangling",
    "freshness",
    "schema",
    "files",
    record_schema::SECTION,
];

/// 2026-10-23: The core sections of a default status.
pub const CORE_DEFAULT_SECTIONS: &[&str] = &["dangling", "freshness"];

/// 2026-10-23: The Cypher features LatticeDB's executor lacks (design §1.1), for `schema`.
pub const CYPHER_UNSUPPORTED: &[&str] = &[
    "OPTIONAL MATCH",
    "MERGE",
    "UNWIND",
    "SET",
    "shortestPath",
    "function calls (count, collect, labels, length, exists)",
];

/// 2026-10-23: One section's JSON.
pub fn section<P: Pack>(ctx: &SectionCtx<'_, P>, name: &str) -> Result<Value> {
    let (b, cfg, fresh) = (ctx.bundle, ctx.cfg, ctx.fresh);
    if P::SECTIONS.contains(&name) {
        return P::section(ctx, name);
    }
    Ok(match name {
        "dangling" => {
            let rows = |v: Vec<(NodeId, Vec<NodeId>)>| -> Vec<Value> {
                let mut rows: Vec<Value> = v
                    .into_iter()
                    .map(|(u, by)| {
                        let mut o = Map::new();
                        let kind = u.key().split_once(':').map_or("", |(k, _)| k).to_string();
                        o.insert("id".into(), Value::String(u.to_string()));
                        o.insert("kind".into(), Value::String(kind));
                        o.insert("referenced_by".into(), ids(&by));
                        Value::Object(o)
                    })
                    .collect();
                rows.truncate(500);
                rows
            };
            let refs = rows(queries::dangling(b));
            let fwd = rows(queries::forward_links(b));
            let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
            for r in &refs {
                *by_kind
                    .entry(r["kind"].as_str().unwrap_or("").to_string())
                    .or_insert(0) += 1;
            }
            let mut o = Map::new();
            o.insert("count".into(), Value::from(refs.len()));
            o.insert(
                "by_kind".into(),
                Value::Object(
                    by_kind
                        .into_iter()
                        .map(|(k, n)| (k, Value::from(n)))
                        .collect(),
                ),
            );
            o.insert("refs".into(), Value::from(refs));
            let uncommitted: Vec<Value> = b
                .nodes
                .iter()
                .filter(|n| n.p.get("dangling_commit") == Some(&Value::Bool(true)))
                .map(|n| Value::String(n.id.to_string()))
                .collect();
            o.insert(
                "no_commit_in_source_count".into(),
                Value::from(uncommitted.len()),
            );
            o.insert("no_commit_in_source".into(), Value::from(uncommitted));
            o.insert("forward_links_count".into(), Value::from(fwd.len()));
            o.insert("forward_links".into(), Value::from(fwd));
            Value::Object(o)
        }

        "freshness" => {
            let mut o = Map::new();
            o.insert("derived_at_ms".into(), Value::from(fresh.derived_at_ms));
            o.insert(
                "source_commit".into(),
                fresh
                    .source_commit
                    .clone()
                    .map_or(Value::Null, Value::String),
            );
            o.insert(
                "stale_reason".into(),
                fresh
                    .stale_reason
                    .clone()
                    .map_or(Value::Null, Value::String),
            );
            o.insert(
                "digest".into(),
                Value::String(crate::jsonl::digest(b).hex()),
            );
            o.insert("sources".into(), Value::from(b.sources.len()));
            o.insert("files".into(), Value::from(files::<P>(b, cfg).len()));
            o.insert(
                "newest_change".into(),
                fresh.newest_change.as_ref().map_or(Value::Null, |(p, ms)| {
                    let mut n = Map::new();
                    n.insert("path".into(), Value::String(p.clone()));
                    n.insert("ts_ms".into(), Value::from(*ms));
                    Value::Object(n)
                }),
            );
            Value::Object(o)
        }
        "files" => {
            let mut o = Map::new();
            o.insert(
                "source_dirs".into(),
                Value::from(P::source_dirs(&cfg.sources)),
            );
            o.insert("files".into(), Value::from(files::<P>(b, cfg)));
            Value::Object(o)
        }
        record_schema::SECTION => record_schema::section(P::RECORD_SCHEMAS, ctx.scope),
        "schema" => {
            let mut o = Map::new();
            let labels: Map<String, Value> = labels::<P>()
                .map(|l| (l.name().to_string(), Value::from(b.of(l).count())))
                .collect();
            let rels: Map<String, Value> = rels::<P>()
                .map(|r| {
                    (
                        r.name().to_string(),
                        Value::from(b.edges.iter().filter(|e| e.rel == r).count()),
                    )
                })
                .collect();
            o.insert("labels".into(), Value::Object(labels));
            o.insert("relations".into(), Value::Object(rels));
            o.insert(
                "cypher_unsupported".into(),
                Value::from(CYPHER_UNSUPPORTED.to_vec()),
            );
            o.insert(
                "node_property".into(),
                Value::String("n.id is the typed id; payload fields as in the bundle".into()),
            );
            Value::Object(o)
        }
        other => {
            return Err(error!(
                ErrorCode::LatlasToolArgument,
                "sections",
                format!("unknown section {other}")
            ));
        }
    })
}

/// 2026-10-23: A section that cannot be computed, as a declared mode (PCND): the others are
/// still returned.
fn unavailable(e: &latlas_error::Error) -> Value {
    let mut o = Map::new();
    o.insert("unavailable".into(), Value::Bool(true));
    o.insert("code".into(), Value::String(e.code().name().to_string()));
    o.insert("reason".into(), Value::String(e.to_string()));
    Value::Object(o)
}

/// 2026-10-23: Every section name: the pack's, then the core's.
pub fn section_names<P: Pack>() -> Vec<&'static str> {
    P::SECTIONS.iter().chain(CORE_SECTIONS).copied().collect()
}

/// 2026-10-23: The sections of a default status: the pack's, then the core's.
pub fn default_section_names<P: Pack>() -> Vec<&'static str> {
    P::DEFAULT_SECTIONS
        .iter()
        .chain(CORE_DEFAULT_SECTIONS)
        .copied()
        .collect()
}

/// 2026-10-23: The requested sections (the default set when `names` is empty), keyed by name. A
/// section that cannot be computed is returned as `{unavailable, code, reason}`, never failing
/// the rest; an unknown scope or section name is refused.
pub fn status<P: Pack>(
    b: &Bundle,
    cfg: &Config<P::Sources>,
    scope: &str,
    names: &[String],
    fresh: &Freshness,
    instance: Option<&str>,
) -> Result<Value> {
    cfg.scope::<P>(scope)?;
    let short = names.is_empty();
    let all: Vec<String> = if short {
        default_section_names::<P>()
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    } else {
        names.to_vec()
    };
    let known = section_names::<P>();
    if let Some(bad) = all.iter().find(|n| !known.contains(&n.as_str())) {
        return Err(error!(
            ErrorCode::LatlasToolArgument,
            "sections",
            format!("no section `{bad}`; sections: {}", known.join(", "))
        ));
    }
    let default_instance = P::default_instance(&cfg.sources, scope);
    let ctx = SectionCtx::<P> {
        bundle: b,
        cfg,
        scope,
        fresh,
        instance: instance.or(default_instance.as_deref()),
    };
    let mut o = Map::new();
    for n in &all {
        let v = section(&ctx, n).unwrap_or_else(|e| unavailable(&e));
        o.insert(n.clone(), if short { compact(v) } else { v });
    }
    Ok(Value::Object(o))
}
