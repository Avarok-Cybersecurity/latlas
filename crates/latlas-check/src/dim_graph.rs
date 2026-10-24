// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The dimensions computed from the bundle alone: R (structural references), C, H and
//! O (check part §1).
//!
//! Owner: latlas-check.
//! Invariants: ratios count live nodes only and are weighted by [`crate::weights`].

use crate::dims::{Contributor, Ctx, Dim, DimResult, Mode, absent, ratio};
use crate::pack::Checks;
use crate::prose::prose_refs;
use crate::weights::src_path;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::Bundle;
use latlas_core::id::{NodeId, Vocab, core_labels, core_rels};
use serde_json::Value;

/// 2026-10-23: R: references whose target is undefined, each weighted by its referrer
/// (`GATED_BY` ×2), plus prose references at `prose_w` of their referrer
/// ([`crate::prose`]). Memory forward links are not integrity errors.
pub(crate) fn referential<V: Vocab>(c: &Ctx<'_>, prose_w: f64) -> DimResult {
    let (mut all, mut bad, mut out) = (0.0, 0.0, Vec::new());
    for e in &c.b.edges {
        let rw = c.w(&e.from)
            * if e.rel == core_rels::GatedBy {
                2.0
            } else {
                1.0
            };
        all += rw;
        if e.to.label() == core_labels::Unresolved {
            let forward =
                c.b.node(&e.to).and_then(|n| n.p.get("forward")) == Some(&Value::Bool(true));
            if forward {
                all -= rw;
                continue;
            }
            bad += rw;
            out.push(c.contributor(
                &e.from,
                rw,
                format!("{} -> {} is undefined", e.rel.name(), e.to),
            ));
        }
    }
    for r in prose_refs::<V>(c.b) {
        let rw = prose_w * c.w(&r.referrer);
        all += rw;
        if !r.resolves {
            bad += rw;
            out.push(c.contributor(
                &r.referrer,
                rw,
                format!("prose reference `{}` does not resolve", r.text),
            ));
        }
    }
    ratio(
        Dim::R,
        all - bad,
        all,
        Mode::Partial,
        Some("backticked symbols are not resolved against a symbol index yet".into()),
        out,
    )
}

/// 2026-10-23: C: live outcome-bearing nodes backed by a claim with a gate, at the pack's credit.
pub(crate) fn coverage<P: Checks>(c: &Ctx<'_>) -> DimResult {
    let (mut all, mut good, mut out) = (0.0, 0.0, Vec::new());
    for n in
        c.b.nodes
            .iter()
            .filter(|n| P::is_outcome(n) && c.live(&n.id))
    {
        let w = c.w(&n.id);
        all += w;
        let credit =
            c.b.out(&n.id, core_rels::Claims)
                .filter(|e| c.b.out(&e.to, core_rels::GatedBy).next().is_some())
                .map(|e| P::claim_credit(c.b, &e.to))
                .fold(0.0_f64, f64::max);
        good += w * credit;
        if credit < 1.0 {
            out.push(c.contributor(&n.id, w * (1.0 - credit), format!("claim credit {credit}")));
        }
    }
    ratio(
        Dim::C,
        good,
        all,
        Mode::Partial,
        Some("a gate counts when named; it is not checked against the test list yet".into()),
        out,
    )
}

/// 2026-10-23: The DEPENDS_ON closure downstream of `roots` (the nodes that rest on them).
fn downstream(b: &Bundle, roots: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    let mut seen: BTreeSet<NodeId> = BTreeSet::new();
    let mut work: Vec<NodeId> = roots.iter().cloned().collect();
    while let Some(t) = work.pop() {
        for e in b.into(&t, core_rels::DependsOn) {
            if !roots.contains(&e.from) && seen.insert(e.from.clone()) {
                work.push(e.from.clone());
            }
        }
    }
    seen
}

/// 2026-10-23: The nodes of `base` the head lost without a retraction, from sources the head
/// still reads.
pub(crate) fn vanished(c: &Ctx<'_>, base: &Bundle, base_w: &BTreeMap<NodeId, f64>) -> Vec<Contributor> {
    let head_paths: BTreeSet<&str> = c.b.sources.iter().map(|s| s.path.as_str()).collect();
    // A retraction of a deleted record points at its `Unresolved` stand-in, keyed by the id.
    let retracted: BTreeSet<String> = c
        .b
        .edges
        .iter()
        .filter(|e| e.rel == core_rels::Retracts)
        .map(|e| {
            if e.to.label() == core_labels::Unresolved {
                e.to.key().to_string()
            } else {
                e.to.to_string()
            }
        })
        .collect();
    base.nodes
        .iter()
        .filter(|n| n.id.label() != core_labels::Unresolved && !n.id.label().implicit())
        .filter(|n| c.b.node(&n.id).is_none() && !retracted.contains(&n.id.to_string()))
        .filter(|n| head_paths.contains(src_path(&n.src)))
        .map(|n| Contributor {
            id: n.id.to_string(),
            src: n.src.clone(),
            w: base_w.get(&n.id).copied().unwrap_or(0.0),
            reason: "deleted without a retraction".into(),
        })
        .collect()
}

/// 2026-10-23: H: tainted nodes still live, deletions, and retractions without a reason.
pub(crate) fn hygiene<P: Checks>(c: &Ctx<'_>, gone: Option<&[Contributor]>) -> DimResult {
    let retractions: Vec<(&NodeId, &NodeId)> =
        c.b.edges
            .iter()
            .filter(|e| e.rel == core_rels::Retracts)
            .map(|e| (&e.from, &e.to))
            .collect();
    let targets: BTreeSet<NodeId> = retractions.iter().map(|(_, t)| (*t).clone()).collect();
    let tainted = downstream(c.b, &targets);
    let mut out = Vec::new();
    let (mut bad, mut all) = (0.0, 0.0);
    for t in &tainted {
        let w = c.w(t);
        all += w;
        let own = c.b.out(t, core_rels::Retracts).next().is_some()
            || c.b.out(t, core_rels::Supersedes).next().is_some();
        if c.live(t) && !own {
            bad += w;
            out.push(c.contributor(t, w, "rests on a retracted record and is still live".into()));
        }
    }
    let mut reasonless = BTreeSet::new();
    for (r, _) in &retractions {
        if !reasonless.contains(*r) && !P::retraction_has_reason(c.b, r) {
            reasonless.insert((*r).clone());
        }
    }
    let rs: BTreeSet<&NodeId> = retractions.iter().map(|(r, _)| *r).collect();
    for r in rs {
        let w = c.w(r);
        all += w;
        if reasonless.contains(r) {
            bad += w;
            out.push(c.contributor(r, w, "retracts without a reason".into()));
        }
    }
    let (mode, reason) = match gone {
        Some(v) => {
            for g in v {
                all += g.w;
                bad += g.w;
                out.push(g.clone());
            }
            (Mode::Full, None)
        }
        None => (
            Mode::Partial,
            Some("no base bundle: deletions (vanished) are not counted".into()),
        ),
    };
    ratio(Dim::H, all - bad, all, mode, reason, out)
}

/// 2026-10-23: O: subjects of the pack's obligation rules that lack a requirement.
pub(crate) fn owed<P: Checks>(c: &Ctx<'_>) -> DimResult {
    let (mut all, mut bad, mut out) = (0.0, 0.0, Vec::new());
    for o in P::obligations(c.b) {
        let w = c.w(&o.subject);
        all += w;
        if !o.missing.is_empty() {
            bad += w;
            out.push(c.contributor(
                &o.subject,
                w,
                format!("{}: lacks {}", o.rule, o.missing.join(", ")),
            ));
        }
    }
    if all <= 0.0 {
        return absent(Dim::O, "no obligation rule matched a subject");
    }
    ratio(Dim::O, all - bad, all, Mode::Full, None, out)
}

