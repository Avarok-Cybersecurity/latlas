// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The `Decl` pack: its `[sources]` table (`latlas-project/v1`), its one extra label
//! (`Crate`), the derive over the two adapters, and its score rules: table rows declared
//! `outcome = true` are outcomes (C), a retraction states why in `why` (H), and every published
//! crate owes at least one test (O).
//!
//! Owner: latlas-decl.
//! Invariants: every key of the table is required except `rust` (absent: the repository has no
//! Rust workspace to read); unknown keys are refused.

use crate::{rust, table};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use latlas_check::{Checks, Key, Obligation};
use latlas_core::bundle::{Builder, Bundle, Node};
use latlas_core::config::Config;
use latlas_core::derive::need;
use latlas_core::id::{Label, NodeId, Rel, Vocab, label_by_name};
use latlas_core::pack::{Pack, SectionCtx};
use latlas_core::sources::SourceSet;
use latlas_error::{ErrorCode, Result, error};
use serde::Deserialize;
use serde_json::{Map, Value};

/// 2026-10-24: The pack's labels (the core's plus `Crate`).
pub mod labels {
    pub use latlas_core::id::core_labels::*;
    latlas_core::labels! {
        pub Crate = "crate";
    }
}

/// 2026-10-24: The declarative pack.
pub struct Decl;

/// 2026-10-24: `[sources]`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeclSources {
    /// 2026-10-24: Absolute, or `"."` for the repository holding `.latlas/` (resolved by the host).
    pub root: String,
    pub tables: Vec<TableSource>,
    pub rust: Option<RustSource>,
}

/// 2026-10-24: One markdown table.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TableSource {
    pub id: String,
    pub path: String,
    /// 2026-10-24: The heading text the table sits under (without the `#`s).
    pub heading: String,
    /// 2026-10-24: A label name (`Evidence`, `Decision`, ...).
    pub label: String,
    /// 2026-10-24: The column whose cell is the row's key.
    pub key: String,
    /// 2026-10-24: Columns copied into the node's payload (each must exist).
    pub fields: Vec<String>,
    pub outcome: bool,
    /// 2026-10-24: Columns that key a row for contradictions (X; only Claim/Decision rows are
    /// keyed). Absent: the table's rows are unkeyed.
    pub keyed: Option<Keyed>,
}

/// 2026-10-24: The columns of a row's contradiction key.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Keyed {
    pub subject: String,
    pub slot: String,
    pub value: String,
}

/// 2026-10-24: `[sources.rust]`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RustSource {
    /// 2026-10-24: The workspace's root manifest (`Cargo.toml`).
    pub manifest: String,
}

impl Vocab for Decl {
    const LABELS: &'static [Label] = &[labels::Crate];
    const RELS: &'static [Rel] = &[];
}

fn config_err(key: String, why: String) -> latlas_error::Error {
    error!(ErrorCode::LatlasConfig, key, why)
}

fn derive_table(t: &TableSource, src: &SourceSet, b: &mut Builder) -> Result<()> {
    let text = need(src, &t.path)?;
    let label = label_by_name::<Decl>(&t.label)
        .ok_or_else(|| config_err(format!("tables.{}.label", t.id), format!("no label {}", t.label)))?;
    let tab = table::under_heading(text, &t.heading).ok_or_else(|| {
        error!(
            ErrorCode::LatlasSource,
            t.path.clone(),
            format!("no table under the heading `{}`", t.heading)
        )
    })?;
    let col = |name: &str| {
        tab.header.iter().position(|h| h == name).ok_or_else(|| {
            config_err(
                format!("tables.{}", t.id),
                format!("no column `{name}` in {} (columns: {})", t.path, tab.header.join(", ")),
            )
        })
    };
    let key = col(&t.key)?;
    let keyed = match &t.keyed {
        Some(k) => Some((col(&k.subject)?, col(&k.slot)?, col(&k.value)?)),
        None => None,
    };
    let fields: Vec<(String, usize)> = t
        .fields
        .iter()
        .map(|f| col(f).map(|i| (f.clone(), i)))
        .collect::<Result<_>>()?;
    for (line, cells) in &tab.rows {
        let at = format!("{}:{line}", t.path);
        let Some(k) = cells.get(key).filter(|k| !k.is_empty()) else {
            continue;
        };
        let id = match NodeId::new(label, k.clone()) {
            Ok(id) => id,
            Err(e) => {
                b.error(&e, &at);
                continue;
            }
        };
        let mut p = BTreeMap::new();
        let mut body = Vec::new();
        for (f, i) in &fields {
            let v = cells.get(*i).cloned().unwrap_or_default();
            body.push(format!("{f}: {v}"));
            p.insert(f.clone(), Value::String(v));
        }
        if let Some((s, l, v)) = keyed {
            let cell = |i: usize| Value::String(cells.get(i).cloned().unwrap_or_default());
            p.insert("x_subject".to_string(), cell(s));
            p.insert("x_slot".to_string(), cell(l));
            p.insert("x_value".to_string(), cell(v));
        }
        p.insert("table".to_string(), Value::String(t.id.clone()));
        p.insert("outcome".to_string(), Value::Bool(t.outcome));
        let canonical = cells.join("|");
        b.define(id.clone(), at, &canonical, p);
        b.text(&id, "row", &body.join("; "));
    }
    Ok(())
}

impl Pack for Decl {
    const ID: &'static str = "decl";
    const VERSION: &'static str = "0.1.0";
    const CONFIG_SCHEMA: &'static str = "latlas-project/v1";
    const NAMED_LABELS: &'static [Label] = &[labels::Crate];
    const SEARCH_EXPAND: &'static [Rel] = &[];
    const IMPACT_FOLLOW_OUT: &'static [Rel] = &[];
    const SECTIONS: &'static [&'static str] = &[];
    const DEFAULT_SECTIONS: &'static [&'static str] = &[];
    const RECORD_SCHEMAS: &'static [latlas_core::record_schema::KindSchema] = &[];
    const INSTRUCTIONS: &'static str = "Derived index of this repository's declared records (tables, crates, tests); files stay the source of truth. Start with la_status.";
    const STATUS_DESCRIPTION: &'static str = "Freshness, dangling references and the schema of the derived index.";
    const IMPACT_DESCRIPTION: &'static str = "What rests on a record (downstream) or what it rests on (upstream).";
    type Sources = DeclSources;
    type Facts = ();

    fn root(s: &DeclSources) -> &str {
        &s.root
    }

    fn validate(s: &DeclSources) -> Result<()> {
        let mut seen = alloc::collections::BTreeSet::new();
        for t in &s.tables {
            if !seen.insert(t.id.as_str()) {
                return Err(config_err(format!("tables.{}", t.id), "duplicate table id".into()));
            }
        }
        Ok(())
    }

    fn source_paths(s: &DeclSources) -> Vec<String> {
        let mut v: Vec<String> = s.tables.iter().map(|t| t.path.clone()).collect();
        v.extend(s.rust.iter().map(|r| r.manifest.clone()));
        v
    }

    fn source_dirs(_: &DeclSources) -> Vec<String> {
        Vec::new()
    }

    fn scopes(_: &DeclSources) -> Vec<String> {
        vec!["repo".to_string()]
    }

    fn default_instance(_: &DeclSources, _: &str) -> Option<String> {
        None
    }

    fn derive(cfg: &Config<DeclSources>, src: &SourceSet, _: Option<&()>, b: &mut Builder) -> Result<()> {
        for t in &cfg.sources.tables {
            derive_table(t, src, b)?;
        }
        if let Some(r) = &cfg.sources.rust {
            need(src, &r.manifest)?;
            rust::derive(src, &r.manifest, b);
        }
        Ok(())
    }

    fn sealed(_: &mut Bundle, _: Option<&()>) {}

    fn section(_: &SectionCtx<'_, Self>, name: &str) -> Result<Value> {
        Err(error!(ErrorCode::LatlasToolArgument, "sections", format!("no pack section {name}")))
    }

    fn impact_rels(_: Label, _: bool) -> Option<Vec<Rel>> {
        None
    }

    fn impact_extra(_: &Bundle, _: &Config<DeclSources>, _: &NodeId, _: &mut Map<String, Value>) -> Result<()> {
        Ok(())
    }
}

/// 2026-10-24: What a review-only claim is worth (check part §1 C).
pub const MANUAL_CREDIT: f64 = 0.5;

impl Checks for Decl {
    fn is_outcome(n: &Node) -> bool {
        n.p.get("outcome") == Some(&Value::Bool(true))
    }

    fn claim_credit(b: &Bundle, c: &NodeId) -> f64 {
        match b.field(c, "kind") {
            Some("manual") => MANUAL_CREDIT,
            Some("none") => 0.0,
            _ => 1.0,
        }
    }

    fn retraction_has_reason(b: &Bundle, r: &NodeId) -> bool {
        b.field(r, "why").is_some_and(|w| !w.trim().is_empty())
    }

    fn obligations(b: &Bundle) -> Vec<Obligation> {
        b.of(labels::Crate)
            .filter(|c| c.p.get("publish") == Some(&Value::Bool(true)))
            .map(|c| Obligation {
                rule: "rust:tested".to_string(),
                subject: c.id.clone(),
                missing: if b.out(&c.id, latlas_core::id::core_rels::GatedBy).next().is_some() {
                    Vec::new()
                } else {
                    vec!["a test".to_string()]
                },
            })
            .collect()
    }

    fn key(_: &Bundle, n: &Node) -> Option<Key> {
        let f = |k: &str| n.p.get(k).and_then(Value::as_str).map(ToString::to_string);
        Some(Key {
            subject: f("x_subject")?,
            slot: f("x_slot")?,
            scope: f("table")?,
            value: f("x_value")?,
        })
    }
}
