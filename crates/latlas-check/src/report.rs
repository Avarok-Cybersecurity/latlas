// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The report (check part §5): one JSON value for every host, and the PR comment
//! rendered from it (≤ 60 lines). Built from the bundles and the config only, so no secret can
//! reach it.
//!
//! Owner: latlas-check.
//! Invariants: the `vanished` list is printed even when empty; both digests are named.

use crate::config::CheckConfig;
use crate::dims::{Score, contributor_json, dim_json, round};
use crate::ratchet::Comparison;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde_json::{Map, Value, json};

/// 2026-10-23: The digests and labels a report names.
pub struct Sides<'a> {
    pub base_digest: Option<&'a str>,
    pub head_digest: &'a str,
    pub mode: &'a str,
    pub packs: &'a str,
}

/// 2026-10-23: The report as JSON.
pub fn json(
    sides: &Sides<'_>,
    cfg: &CheckConfig,
    base: Option<&Score>,
    head: &Score,
    cmp: &Comparison,
) -> Value {
    let rows: Vec<Value> = cmp
        .rows
        .iter()
        .map(|(d, b, h, s)| {
            json!({"dim": d.letter(), "name": d.name(),
                "base": b.map_or(Value::Null, round), "head": h.map_or(Value::Null, round),
                "delta": match (b, h) { (Some(b), Some(h)) => round(h - b), _ => Value::Null },
                "status": s.name()})
        })
        .collect();
    let mut o = Map::new();
    o.insert(
        "schema".into(),
        Value::String("latlas-check-report/v1".into()),
    );
    o.insert("mode".into(), Value::String(sides.mode.into()));
    o.insert("packs".into(), Value::String(sides.packs.into()));
    o.insert(
        "bundle_digest_base".into(),
        sides
            .base_digest
            .map_or(Value::Null, |d| Value::String(d.into())),
    );
    o.insert(
        "bundle_digest_head".into(),
        Value::String(sides.head_digest.into()),
    );
    o.insert("rows".into(), Value::from(rows));
    o.insert(
        "head".into(),
        Value::from(head.dims.iter().map(dim_json).collect::<Vec<_>>()),
    );
    if let Some(b) = base {
        o.insert(
            "base".into(),
            Value::from(b.dims.iter().map(dim_json).collect::<Vec<_>>()),
        );
    }
    o.insert(
        "vanished".into(),
        head.vanished.as_ref().map_or(Value::Null, |v| {
            Value::from(v.iter().map(contributor_json).collect::<Vec<_>>())
        }),
    );
    o.insert(
        "fatal".into(),
        Value::from(
            cmp.fatal
                .iter()
                .map(|f| json!({"code": f.code, "at": f.at, "msg": f.msg}))
                .collect::<Vec<_>>(),
        ),
    );
    o.insert(
        "weights".into(),
        json!({"churn": cfg.weights.churn, "pagerank": {"damping": cfg.weights.pagerank.damping,
            "iterations": cfg.weights.pagerank.iterations},
            "importance": cfg.weights.importance.iter().map(|i| json!({"prefix": i.prefix, "value": i.value})).collect::<Vec<_>>()}),
    );
    o.insert(
        "floor".into(),
        cfg.ratchet
            .floor
            .as_ref()
            .map_or(Value::String("none".into()), |f| json!(f)),
    );
    o.insert("exit".into(), Value::from(cmp.exit));
    Value::Object(o)
}

fn num(v: &Value) -> String {
    v.as_f64().map_or("—".to_string(), |x| format!("{x:.3}"))
}

/// 2026-10-23: The PR comment (markdown, ≤ 60 lines) from the JSON report.
pub fn markdown(report: &Value, marker: &str) -> String {
    let mut l: Vec<String> = Vec::new();
    l.push(format!("<!-- latlas-check:{marker} -->"));
    let short = |k: &str| {
        report[k]
            .as_str()
            .map_or("—".to_string(), |d| d.chars().take(7).collect())
    };
    l.push(format!(
        "## latlas-check  base {} → head {}   mode: {}   packs: {}",
        short("bundle_digest_base"),
        short("bundle_digest_head"),
        report["mode"].as_str().unwrap_or(""),
        report["packs"].as_str().unwrap_or("")
    ));
    l.push("| dim | base | head | Δ | status |".into());
    l.push("|---|---|---|---|---|".into());
    for r in report["rows"].as_array().into_iter().flatten() {
        l.push(format!(
            "| {} {} | {} | {} | {} | {} |",
            r["dim"].as_str().unwrap_or(""),
            r["name"].as_str().unwrap_or(""),
            num(&r["base"]),
            num(&r["head"]),
            num(&r["delta"]),
            r["status"].as_str().unwrap_or("")
        ));
    }
    for d in report["head"].as_array().into_iter().flatten() {
        let c = d["contributors"].as_array().cloned().unwrap_or_default();
        if c.is_empty() || l.len() > 48 {
            continue;
        }
        l.push(format!(
            "**{}** top contributors (head):",
            d["dim"].as_str().unwrap_or("")
        ));
        for x in c.iter().take(3) {
            l.push(format!(
                "- `{}` · {} · {} · w {}",
                x["id"].as_str().unwrap_or(""),
                x["reason"].as_str().unwrap_or(""),
                x["src"].as_str().unwrap_or(""),
                num(&x["w"])
            ));
        }
    }
    let fatal = report["fatal"].as_array().map_or(0, Vec::len);
    let vanished = match report["vanished"].as_array() {
        Some(v) => format!("{}", v.len()),
        None => "not counted (no base)".into(),
    };
    l.push(format!(
        "Fatal: {}.   Vanished: {vanished}.   Exit {}.",
        if fatal == 0 {
            "none".to_string()
        } else {
            fatal.to_string()
        },
        report["exit"]
    ));
    l.truncate(60);
    l.join("\n") + "\n"
}
