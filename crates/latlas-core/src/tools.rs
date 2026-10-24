// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The MCP tool surface (design §3): exactly five tools, their input schemas as
//! shipped, and the server instructions. This is the one definition `tools/list` serves and the P0
//! token gate measures.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants:
//! - Five tools; every description is at most 140 characters (a unit test holds both).
//! - Declared defaults are written in the schema (PCND); the tool layer applies exactly these.

use crate::pack::Pack;
use crate::prelude::*;
use crate::status::section_names;
use serde_json::Value;

/// 2026-10-23: The most items of any list in a default `kg_status`.
pub const LIST_CAP: usize = 3;

/// 2026-10-23: Declared defaults of the tool arguments (the schema says the same).
pub const DEFAULT_QUERY_LIMIT: u32 = 200;
/// 2026-10-23: Declared default `k` of `kg_search`.
pub const DEFAULT_SEARCH_K: u32 = 10;
/// 2026-10-23: Declared default `hops` of `kg_search`.
pub const DEFAULT_SEARCH_HOPS: u32 = 1;
/// 2026-10-23: Declared default `max_depth` of `kg_impact`.
pub const DEFAULT_MAX_DEPTH: u32 = 100;

fn tool(name: &str, description: &str, schema: &str) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("name".into(), Value::String(name.into()));
    o.insert("description".into(), Value::String(description.into()));
    let s: Value = serde_json::from_str(schema).unwrap_or(Value::Null);
    o.insert("inputSchema".into(), s);
    Value::Object(o)
}

/// 2026-10-23: The `tools/list` array.
pub fn tools<P: Pack>() -> Vec<Value> {
    let sections: Vec<String> = section_names::<P>()
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect();
    let kinds: Vec<String> = P::RECORD_KINDS.iter().map(|s| format!("\"{s}\"")).collect();
    vec![
        tool(
            "la_query",
            "Read-only Cypher (labels, relations: la_status schema).",
            r#"{"type":"object","required":["cypher"],"properties":{"cypher":{"type":"string"},"params":{"type":"object"},"limit":{"type":"integer"}}}"#,
        ),
        tool(
            "la_search",
            "Have we tried this? Vector+graph recall; reports mode hybrid|graph_only.",
            r#"{"type":"object","required":["text"],"properties":{"text":{"type":"string"},"kinds":{"type":"array","items":{"type":"string"}},"k":{"type":"integer"},"hops":{"type":"integer"}}}"#,
        ),
        tool(
            "la_impact",
            P::IMPACT_DESCRIPTION,
            r#"{"type":"object","required":["id","direction"],"properties":{"id":{"type":"string"},"direction":{"enum":["downstream","upstream"]},"relations":{"type":"array","items":{"type":"string"}},"max_depth":{"type":"integer"}}}"#,
        ),
        tool(
            "la_record",
            "Append a record to its SSOT file through its validator; fails closed. dry_run previews would_taint.",
            &format!(
                r#"{{"type":"object","required":["kind","scope","fields"],"properties":{{"kind":{{"enum":[{}]}},"scope":{{"type":"string"}},"fields":{{"type":"object"}},"dry_run":{{"type":"boolean"}},"acknowledge_taint":{{"type":"integer"}}}}}}"#,
                kinds.join(",")
            ),
        ),
        tool(
            "la_status",
            P::STATUS_DESCRIPTION,
            &format!(
                r#"{{"type":"object","required":["scope"],"properties":{{"scope":{{"type":"string"}},"sections":{{"type":"array","items":{{"enum":[{}]}}}},"instance":{{"type":"string"}}}}}}"#,
                sections.join(",")
            ),
        ),
    ]
}

/// 2026-10-23: The `tools/list` result body, canonical JSON, as the token gate counts it.
pub fn tools_list_json<P: Pack>() -> String {
    let mut o = serde_json::Map::new();
    o.insert("tools".into(), Value::Array(tools::<P>()));
    let mut s = String::new();
    crate::jsonl::canon(&Value::Object(o), &mut s);
    s
}
