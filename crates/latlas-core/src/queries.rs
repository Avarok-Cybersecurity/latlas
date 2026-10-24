// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The closures and counts the graph answers (queries part §1) as pure functions over
//! a [`Bundle`]: LatticeDB's executor has no aggregation, `OPTIONAL MATCH` or `NOT (pattern)`, so
//! these run here and are golden-tested without the DB.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants: no closure names a pack label; a pack's own queries live in the pack.

use crate::bundle::Bundle;
use crate::id::{NodeId, Rel, Vocab, core_labels, prefixes};
use crate::pack::Pack;
use crate::prelude::*;
use latlas_error::{ErrorCode, Result, error};
use serde_json::Value;

/// 2026-10-23: One row of a closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    pub id: NodeId,
    pub why: String,
    pub depth: u32,
}

/// 2026-10-23: `la_impact`: what `root` rests on (`upstream`) or what rests on it
/// (`downstream`), breadth-first to `max_depth`, following `rels` (out-edges upstream, in-edges
/// downstream, plus a downstream node's out-edges of [`Pack::IMPACT_FOLLOW_OUT`]). Returns the
/// closure and whether depth cut it.
pub fn impact<P: Pack>(
    b: &Bundle,
    root: &NodeId,
    downstream: bool,
    rels: &[Rel],
    max_depth: u32,
) -> Result<(Vec<Reach>, bool)> {
    if b.node(root).is_none() {
        return Err(error!(ErrorCode::LatlasUnknownId, root.to_string()));
    }
    let mut seen: BTreeSet<NodeId> = BTreeSet::new();
    seen.insert(root.clone());
    let mut out = Vec::new();
    let mut frontier = vec![root.clone()];
    let mut truncated = false;
    for depth in 1..=max_depth.saturating_add(1) {
        let mut next = Vec::new();
        for v in &frontier {
            let mut step: Vec<(NodeId, String)> = Vec::new();
            for e in &b.edges {
                if !rels.contains(&e.rel) {
                    continue;
                }
                if !downstream && e.from == *v {
                    step.push((e.to.clone(), format!("{} {}", e.rel.name(), e.to)));
                } else if downstream && e.to == *v {
                    step.push((e.from.clone(), format!("{} {v}", e.rel.name())));
                }
            }
            if downstream {
                for e in b.edges.iter().filter(|e| e.from == *v) {
                    if P::IMPACT_FOLLOW_OUT.contains(&e.rel) && rels.contains(&e.rel) {
                        step.push((e.to.clone(), format!("{} by {v}", e.rel.name())));
                    }
                }
            }
            for (to, why) in step {
                if seen.contains(&to) {
                    continue;
                }
                if depth > max_depth {
                    truncated = true;
                    continue;
                }
                seen.insert(to.clone());
                out.push(Reach {
                    id: to.clone(),
                    why,
                    depth,
                });
                next.push(to);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out.sort_by(|a, b| (a.depth, &a.id).cmp(&(b.depth, &b.id)));
    Ok((out, truncated))
}

/// 2026-10-23: `Unresolved` nodes with what referenced each.
pub fn dangling(b: &Bundle) -> Vec<(NodeId, Vec<NodeId>)> {
    unresolved(b, false)
}

/// 2026-10-23: Memory forward links (targets not written yet), with who links to them.
pub fn forward_links(b: &Bundle) -> Vec<(NodeId, Vec<NodeId>)> {
    unresolved(b, true)
}

fn unresolved(b: &Bundle, forward: bool) -> Vec<(NodeId, Vec<NodeId>)> {
    b.of(core_labels::Unresolved)
        .filter(|u| (u.p.get("forward") == Some(&Value::Bool(true))) == forward)
        .map(|u| {
            let mut by: Vec<NodeId> = b
                .edges
                .iter()
                .filter(|e| e.to == u.id)
                .map(|e| e.from.clone())
                .collect();
            by.dedup();
            (u.id.clone(), by)
        })
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 2026-10-23: Ids the caller probably meant by `raw` (a bare key, a wrong prefix or a typo):
/// every node whose key is `raw`'s key exactly, else up to three nodes within edit distance 3 of
/// it, nearest first. Empty when nothing is close.
pub fn suggest(b: &Bundle, raw: &str) -> Vec<NodeId> {
    let key = raw.split_once(':').map_or(raw, |(_, k)| k);
    let exact: Vec<NodeId> = b
        .nodes
        .iter()
        .filter(|n| n.id.key() == key)
        .map(|n| n.id.clone())
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let mut near: Vec<(usize, &NodeId)> = b
        .nodes
        .iter()
        .filter(|n| n.id.label() != core_labels::Unresolved)
        .map(|n| (edit_distance(key, n.id.key()), &n.id))
        .filter(|(d, _)| *d <= 3)
        .collect();
    near.sort();
    near.into_iter().take(3).map(|(_, id)| id.clone()).collect()
}

/// 2026-10-23: Parse `raw` and require the node, or refuse naming the closest ids and the
/// prefixes, so a first call never dead-ends.
pub fn resolve_id<V: Vocab>(b: &Bundle, raw: &str) -> Result<NodeId> {
    let hint = |raw: &str| {
        let s = suggest(b, raw);
        if s.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = s.iter().map(ToString::to_string).collect();
            format!(" (did you mean {}?)", names.join(" or "))
        }
    };
    match NodeId::parse::<V>(raw) {
        Ok(id) if b.node(&id).is_some() => Ok(id),
        Ok(id) => Err(error!(
            ErrorCode::LatlasUnknownId,
            format!("{id}{}", hint(raw))
        )),
        Err(_) => Err(error!(
            ErrorCode::LatlasBadId,
            raw.to_string(),
            format!(
                "{}{}; prefixes: {}",
                if raw.contains(':') {
                    "unknown label prefix"
                } else {
                    "no `<prefix>:` part"
                },
                hint(raw),
                prefixes::<V>()
            )
        )),
    }
}
