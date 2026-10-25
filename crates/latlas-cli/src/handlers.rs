// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The five tools (design §3), JSON in, JSON out, shared by the MCP server and the CLI
//! so both run the same code. Argument defaults are the declared ones of `latlas_core::tools`.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants:
//! - Every call runs the freshness check first and returns the envelope (`App::envelope`).
//! - An argument outside its schema is `KgToolArgument`, never coerced.

use crate::app::App;
use crate::host::PackHost;
use latlas_core::Pack;
use latlas_core::id::{Rel, rel_by_name, rels};
use latlas_core::queries::{self, impact};
use latlas_core::status::status;
use latlas_core::tools::{DEFAULT_MAX_DEPTH, DEFAULT_QUERY_LIMIT, tools};
use latlas_error::{ErrorCode, Result, error};
use latlas_index::cypher;
use serde_json::{Map, Value};
use std::time::Duration;

fn arg_str<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a.get(k).and_then(Value::as_str).ok_or_else(|| {
        error!(
            ErrorCode::LatlasToolArgument,
            k.to_string(),
            "required string"
        )
    })
}

fn arg_u32(a: &Value, k: &str, default: u32, max: u32) -> Result<u32> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| {
                error!(
                    ErrorCode::LatlasToolArgument,
                    k.to_string(),
                    format!("an integer in 1..={max}")
                )
            }),
    }
}

fn arg_strs(a: &Value, k: &str) -> Result<Vec<String>> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str().map(str::to_string).ok_or_else(|| {
                    error!(ErrorCode::LatlasToolArgument, k.to_string(), "strings only")
                })
            })
            .collect(),
        Some(_) => Err(error!(
            ErrorCode::LatlasToolArgument,
            k.to_string(),
            "must be an array"
        )),
    }
}

/// 2026-10-23: Dispatch one tool call.
pub async fn call<P: Pack, H: PackHost<P>>(
    app: &mut App<P, H>,
    tool: &str,
    args: &Value,
) -> Result<Value> {
    app.refresh().await;
    let body = match tool {
        "la_query" => la_query(app, args).await?,
        "la_impact" => la_impact(app, args)?,
        "la_status" => {
            let scope = arg_str(args, "scope")?;
            let sections = arg_strs(args, "sections")?;
            let instance = args.get("instance").and_then(Value::as_str);
            let mut v = status::<P>(
                &app.bundle,
                &app.cfg,
                scope,
                &sections,
                &app.fresh,
                instance,
            )?;
            if sections.is_empty() || sections.iter().any(|s| s == "freshness") {
                let prov: Map<String, Value> = app
                    .settings
                    .provenance
                    .iter()
                    .map(|(k, l)| ((*k).to_string(), Value::String(l.name().into())))
                    .collect();
                let mut st = Map::new();
                st.insert(
                    "global_layer".into(),
                    Value::String(app.settings.global.describe()),
                );
                st.insert(
                    "project_layer".into(),
                    Value::String(app.cfg.origin.clone()),
                );
                st.insert("provenance".into(), Value::Object(prov));
                if let Value::Object(o) = &mut v {
                    o.insert("settings".into(), Value::Object(st));
                    o.insert("pause".into(), app.pause.status());
                }
            }
            v
        }
        "la_search" => crate::search::la_search(app, args).await?,
        "la_record" => {
            let now = app.now();
            let (v, written) = app
                .host
                .record(&app.cfg, &app.fs, &app.bundle, args, now)
                .await?;
            if written {
                app.refresh().await;
            }
            v
        }
        other => return Err(no_tool::<P>(other)),
    };
    Ok(app.envelope(body))
}

/// 2026-10-24: The refusal of an unknown tool name: it lists the served tools, and a pre-rename
/// `kg_*` name says which `la_*` tool replaced it.
pub fn no_tool<P: Pack>(name: &str) -> latlas_error::Error {
    let served: Vec<String> = tools::<P>()
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    no_tool_in(name, &served)
}

fn no_tool_in(name: &str, served: &[String]) -> latlas_error::Error {
    let renamed = name
        .strip_prefix("kg_")
        .map(|rest| format!("la_{rest}"))
        .filter(|n| served.contains(n))
        .map_or(String::new(), |n| {
            format!("; the kg_* tools were renamed la_* on 2026-10-23: use {n}")
        });
    error!(
        ErrorCode::LatlasToolArgument,
        "name",
        format!("no tool {name}; tools: {}{renamed}", served.join(", "))
    )
}

async fn la_query<P: Pack, H: PackHost<P>>(app: &mut App<P, H>, args: &Value) -> Result<Value> {
    let text = arg_str(args, "cypher")?.to_string();
    cypher::read_only(&text)?;
    let cap = app.cfg.limits.max_rows.min(1000);
    let limit = arg_u32(args, "limit", DEFAULT_QUERY_LIMIT.min(cap), cap)?;
    let params = cypher::params(args.get("params"))?;
    let store = app.store.clone();
    let ms = app.cfg.limits.cypher_timeout_ms;
    let job = tokio::task::spawn_blocking(move || store.blocking_lock().query(&text, params));
    let (cols, mut rows) = match tokio::time::timeout(Duration::from_millis(ms), job).await {
        Err(_) => {
            return Err(error!(
                ErrorCode::LatlasCypherRejected,
                format!("exceeded cypher_timeout_ms {ms}")
            ));
        }
        Ok(Err(e)) => return Err(error!(ErrorCode::LatlasStore, "query", e.to_string())),
        Ok(Ok(Err(e))) => return Err(error!(ErrorCode::LatlasCypherRejected, e.to_string())),
        Ok(Ok(Ok(r))) => r,
    };
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    let mut o = Map::new();
    o.insert("columns".into(), Value::from(cols));
    o.insert("rows".into(), Value::from(rows));
    o.insert("truncated".into(), Value::Bool(truncated));
    Ok(Value::Object(o))
}

fn la_impact<P: Pack, H: PackHost<P>>(app: &App<P, H>, args: &Value) -> Result<Value> {
    let root = queries::resolve_id::<P>(&app.bundle, arg_str(args, "id")?)?;
    let downstream = match arg_str(args, "direction")? {
        "downstream" => true,
        "upstream" => false,
        other => {
            return Err(error!(
                ErrorCode::LatlasToolArgument,
                "direction",
                format!("{other} is not downstream|upstream")
            ));
        }
    };
    let named = arg_strs(args, "relations")?;
    let rels: Vec<Rel> = if named.is_empty() {
        P::impact_rels(root.label(), downstream).unwrap_or_else(|| rels::<P>().collect())
    } else {
        named
            .iter()
            .map(|n| {
                rel_by_name::<P>(n).ok_or_else(|| {
                    error!(
                        ErrorCode::LatlasToolArgument,
                        "relations",
                        format!("no relation {n}")
                    )
                })
            })
            .collect::<Result<_>>()?
    };
    let depth = arg_u32(args, "max_depth", DEFAULT_MAX_DEPTH, 100)?;
    let (reach, cut) = impact::<P>(&app.bundle, &root, downstream, &rels, depth)?;
    let b = &app.bundle;
    let closure: Vec<Value> = reach
        .iter()
        .map(|r| {
            let mut o = Map::new();
            o.insert("id".into(), Value::String(r.id.to_string()));
            o.insert("why".into(), Value::String(r.why.clone()));
            o.insert("depth".into(), Value::from(r.depth));
            if let Some(v) = b.field(&r.id, "verdict_kind") {
                o.insert("verdict".into(), Value::String(v.to_string()));
            }
            Value::Object(o)
        })
        .collect();
    let mut o = Map::new();
    o.insert("root".into(), Value::String(root.to_string()));
    o.insert("closure".into(), Value::from(closure));
    o.insert("paths_truncated".into(), Value::Bool(cut));
    P::impact_extra(b, &app.cfg, &root, &mut o)?;
    Ok(Value::Object(o))
}

#[cfg(test)]
mod tests {
    use super::no_tool_in;

    #[test]
    fn an_unknown_tool_lists_the_tools_and_maps_a_pre_rename_name() {
        let served: Vec<String> = ["la_search", "la_record"].map(String::from).to_vec();
        let old = no_tool_in("kg_record", &served).to_string();
        assert!(
            old.contains("no tool kg_record; tools: la_search, la_record"),
            "{old}"
        );
        assert!(
            old.contains("renamed la_* on 2026-10-23: use la_record"),
            "{old}"
        );
        let other = no_tool_in("kg_bogus", &served).to_string();
        assert!(other.contains("tools: la_search, la_record"), "{other}");
        assert!(!other.contains("renamed"), "{other}");
    }
}
