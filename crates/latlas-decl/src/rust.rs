// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The `rust` adapter: workspace members (the host lists `<member>/Cargo.toml` and the
//! `.rs` files under `src/` and `tests/`) become `Crate` nodes, and every test function a `Gate`
//! (`test/<crate>::<fn>`) the crate is `GATED_BY`.
//!
//! Owner: latlas-decl.
//! Invariants: a test is a `#[test]`, `#[tokio::test…]` or `#[wasm_bindgen_test]` attribute
//! followed, within four lines, by `fn <name>`; a file belongs to the member with the longest
//! directory prefix.

use crate::pack::labels;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::bundle::Builder;
use latlas_core::id::{NodeId, core_labels, core_rels};
use latlas_core::sources::SourceSet;
use latlas_error::{ErrorCode, error};
use serde_json::Value;

/// 2026-10-24: The `[workspace] members` of a root manifest (globs are the host's to expand).
pub fn members(manifest: &str) -> Vec<String> {
    let Ok(t) = manifest.parse::<toml::Table>() else {
        return Vec::new();
    };
    t.get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn is_test_attr(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with("#[test]") || t.starts_with("#[tokio::test") || t.starts_with("#[wasm_bindgen_test")
}

fn fn_name(l: &str) -> Option<String> {
    let t = l.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    let t = t.strip_prefix("async ").unwrap_or(t);
    let rest = t.strip_prefix("fn ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// 2026-10-24: `(line, fn name)` of every test function in `text`.
pub fn tests_in(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if !is_test_attr(l) {
            continue;
        }
        if let Some((j, n)) = lines
            .iter()
            .enumerate()
            .skip(i + 1)
            .take(4)
            .find_map(|(j, l)| fn_name(l).map(|n| (j, n)))
        {
            out.push((j + 1, n));
        }
    }
    out
}

/// 2026-10-24: Add the crates and their test gates.
pub fn derive(src: &SourceSet, root_manifest: &str, b: &mut Builder) {
    let mut crates: BTreeMap<String, NodeId> = BTreeMap::new();
    for (path, text) in &src.files {
        let Some(dir) = path.strip_suffix("/Cargo.toml") else {
            continue;
        };
        if path == root_manifest {
            continue;
        }
        let at = format!("{path}:1");
        let pkg = text.parse::<toml::Table>().ok();
        let pkg = pkg.as_ref().and_then(|t| t.get("package"));
        let Some(name) = pkg.and_then(|p| p.get("name")).and_then(toml::Value::as_str) else {
            b.error(
                &error!(ErrorCode::LatlasBadRow, at.clone(), "a member manifest without [package] name"),
                &at,
            );
            continue;
        };
        let publish = pkg.and_then(|p| p.get("publish")).and_then(toml::Value::as_bool) != Some(false);
        let Ok(id) = NodeId::new(labels::Crate, name) else {
            continue;
        };
        let mut p = BTreeMap::new();
        p.insert("dir".to_string(), Value::String(dir.to_string()));
        p.insert("publish".to_string(), Value::Bool(publish));
        b.define(id.clone(), at, name, p);
        crates.insert(format!("{dir}/"), id);
    }
    for (path, text) in &src.files {
        if !path.ends_with(".rs") {
            continue;
        }
        let Some((_, krate)) = crates
            .iter()
            .filter(|(d, _)| path.starts_with(d.as_str()))
            .max_by_key(|(d, _)| d.len())
        else {
            continue;
        };
        for (line, name) in tests_in(text) {
            let at = format!("{path}:{line}");
            let Ok(g) = NodeId::new(core_labels::Gate, format!("test/{}::{name}", krate.key())) else {
                continue;
            };
            b.mention(&g, &at);
            b.edge(krate, core_rels::GatedBy, &g, 1.0, &at);
        }
    }
}
