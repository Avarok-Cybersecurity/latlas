// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: X, contradictions — keyed, never embedded (check part §1; arXiv 2606.26511 /
//! 2606.06240: embedding-based contradiction detection scores AUROC 0.59). Every live Claim and
//! Decision the pack can key is grouped by `(subject, slot, scope)`; two in a group with different
//! values contradict unless a Decision's `settles` field names both (the settlement record is part
//! of the model); a superseded or retracted record is not live and contradicts nothing.
//!
//! Owner: latlas-check.
//! Invariants: `X = 1 − w(nodes in an unsettled contradiction) / w(keyed live records)`; unkeyed
//! records are counted in the reason, not scored; no semantic candidate ever enters X.

use crate::dims::{Ctx, Dim, DimResult, Mode, absent, ratio};
use crate::pack::{Checks, Key};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::id::{NodeId, core_labels};
use serde_json::Value;

/// 2026-10-24: The pairs a Decision settles (its `settles` field: an array of ids).
fn settled(c: &Ctx<'_>) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for d in c.b.of(core_labels::Decision).filter(|d| c.live(&d.id)) {
        let ids: Vec<&str> = d
            .p
            .get("settles")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for x in &ids {
            for y in &ids {
                if x < y {
                    out.insert((String::from(*x), String::from(*y)));
                }
            }
        }
    }
    out
}

/// 2026-10-24: X.
pub(crate) fn contradictions<P: Checks>(c: &Ctx<'_>) -> DimResult {
    let mut groups: BTreeMap<(String, String, String), Vec<(NodeId, String)>> = BTreeMap::new();
    let mut unkeyed = 0usize;
    for n in c.b.nodes.iter().filter(|n| {
        matches!(n.id.label(), l if l == core_labels::Claim || l == core_labels::Decision) && c.live(&n.id)
    }) {
        match P::key(c.b, n) {
            Some(Key { subject, slot, scope, value }) => {
                groups.entry((subject, slot, scope)).or_default().push((n.id.clone(), value));
            }
            None => unkeyed += 1,
        }
    }
    let keyed: usize = groups.values().map(Vec::len).sum();
    if keyed == 0 {
        return absent(Dim::X, &format!("no keyed records ({unkeyed} unkeyed)"));
    }
    let settles = settled(c);
    let mut bad: BTreeMap<NodeId, String> = BTreeMap::new();
    for ((subject, slot, scope), members) in &groups {
        for (i, (a, va)) in members.iter().enumerate() {
            for (b, vb) in &members[i + 1..] {
                let (x, y) = if a.to_string() < b.to_string() { (a, b) } else { (b, a) };
                if va == vb || settles.contains(&(x.to_string(), y.to_string())) {
                    continue;
                }
                let why = format!("{subject}.{slot}@{scope}: `{va}` vs `{vb}` ({a} / {b})");
                bad.entry(a.clone()).or_insert_with(|| why.clone());
                bad.entry(b.clone()).or_insert(why);
            }
        }
    }
    let all: f64 = groups.values().flatten().map(|(id, _)| c.w(id)).sum();
    let gone: f64 = bad.keys().map(|id| c.w(id)).sum();
    let out = bad.iter().map(|(id, why)| c.contributor(id, c.w(id), why.clone())).collect();
    let reason = (unkeyed > 0).then(|| format!("{unkeyed} unkeyed records are not scored"));
    ratio(Dim::X, all - gone, all, Mode::Full, reason, out)
}
