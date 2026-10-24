// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The ten dimensions (check part §1). This slice (K2.1) computes R, C, H and O from
//! the bundle; F, G, P, S, T and X need host inputs not supplied yet (derived-artifact digests, CI
//! results, run fingerprints, syntax anchors, commit windows, keyed records) and are `absent`
//! with that reason.
//!
//! Owner: latlas-check.
//! Invariants:
//! - Every ratio is weighted by [`crate::weights`] and counts live nodes only (not retracted, not
//!   superseded), so retracting a bad record is honest and deleting it is not (H `vanished`).
//! - An empty denominator is `absent`, never 0 or 1.

use crate::config::CheckConfig;
use crate::pack::Checks;
use crate::weights::{src_path, weights};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use latlas_core::Bundle;
use latlas_core::id::{NodeId, core_labels, core_rels};
use latlas_error::Result;
use serde_json::{Map, Value};

/// 2026-10-23: A dimension of the score vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dim {
    F,
    R,
    C,
    G,
    P,
    S,
    H,
    T,
    X,
    O,
}

impl Dim {
    /// 2026-10-23: Report order.
    pub const ALL: [Dim; 10] = [
        Dim::F,
        Dim::R,
        Dim::C,
        Dim::G,
        Dim::P,
        Dim::S,
        Dim::H,
        Dim::T,
        Dim::X,
        Dim::O,
    ];

    pub fn letter(self) -> &'static str {
        match self {
            Dim::F => "F",
            Dim::R => "R",
            Dim::C => "C",
            Dim::G => "G",
            Dim::P => "P",
            Dim::S => "S",
            Dim::H => "H",
            Dim::T => "T",
            Dim::X => "X",
            Dim::O => "O",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dim::F => "freshness",
            Dim::R => "referential",
            Dim::C => "claim coverage",
            Dim::G => "gate pass rate",
            Dim::P => "provenance",
            Dim::S => "staleness",
            Dim::H => "retraction hygiene",
            Dim::T => "traceability",
            Dim::X => "contradictions",
            Dim::O => "owed work",
        }
    }
}

/// 2026-10-23: How completely a dimension was computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Full,
    Partial,
    Absent,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Partial => "partial",
            Mode::Absent => "absent",
        }
    }
}

/// 2026-10-23: One contributor to a dimension's shortfall.
#[derive(Clone, Debug, PartialEq)]
pub struct Contributor {
    pub id: NodeId,
    pub src: String,
    pub w: f64,
    pub reason: String,
}

/// 2026-10-23: One dimension's value.
#[derive(Clone, Debug, PartialEq)]
pub struct DimResult {
    pub dim: Dim,
    pub value: Option<f64>,
    pub mode: Mode,
    pub reason: Option<String>,
    pub numerator_w: f64,
    pub denominator_w: f64,
    /// 2026-10-23: The heaviest ten, heaviest first.
    pub contributors: Vec<Contributor>,
}

/// 2026-10-23: A bundle's score.
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub dims: Vec<DimResult>,
    /// 2026-10-23: Nodes in the base, absent in the head, unretracted, from a source the head still
    /// reads (`None`: no base given).
    pub vanished: Option<Vec<Contributor>>,
}

impl Score {
    pub fn get(&self, d: Dim) -> Option<&DimResult> {
        self.dims.iter().find(|r| r.dim == d)
    }
}

const TOP: usize = 10;

fn absent(dim: Dim, reason: &str) -> DimResult {
    DimResult {
        dim,
        value: None,
        mode: Mode::Absent,
        reason: Some(reason.to_string()),
        numerator_w: 0.0,
        denominator_w: 0.0,
        contributors: Vec::new(),
    }
}

/// 2026-10-23: `1 − bad/all` (or `good/all`), absent on an empty denominator.
fn ratio(
    dim: Dim,
    good: f64,
    all: f64,
    mode: Mode,
    reason: Option<String>,
    mut contributors: Vec<Contributor>,
) -> DimResult {
    contributors.sort_by(|a, b| b.w.total_cmp(&a.w).then_with(|| a.id.cmp(&b.id)));
    contributors.truncate(TOP);
    if all <= 0.0 {
        return DimResult {
            contributors,
            ..absent(dim, "empty denominator")
        };
    }
    DimResult {
        dim,
        value: Some(good / all),
        mode,
        reason,
        numerator_w: good,
        denominator_w: all,
        contributors,
    }
}

struct Ctx<'a> {
    b: &'a Bundle,
    w: BTreeMap<NodeId, f64>,
}

impl Ctx<'_> {
    fn w(&self, id: &NodeId) -> f64 {
        self.w.get(id).copied().unwrap_or(0.0)
    }
    fn src(&self, id: &NodeId) -> String {
        self.b.node(id).map(|n| n.src.clone()).unwrap_or_default()
    }
    fn live(&self, id: &NodeId) -> bool {
        self.b.into(id, core_rels::Retracts).next().is_none()
            && self.b.into(id, core_rels::Supersedes).next().is_none()
    }
    fn contributor(&self, id: &NodeId, w: f64, reason: String) -> Contributor {
        Contributor {
            id: id.clone(),
            src: self.src(id),
            w,
            reason,
        }
    }
}

/// 2026-10-23: R: references whose target is undefined, each weighted by its referrer
/// (`GATED_BY` ×2). Memory forward links are not integrity errors.
fn referential(c: &Ctx<'_>) -> DimResult {
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
    ratio(
        Dim::R,
        all - bad,
        all,
        Mode::Partial,
        Some("structural references only; prose references are not linted yet".into()),
        out,
    )
}

/// 2026-10-23: C: live outcome-bearing nodes backed by a claim with a gate, at the pack's credit.
fn coverage<P: Checks>(c: &Ctx<'_>) -> DimResult {
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
fn vanished(c: &Ctx<'_>, base: &Bundle, base_w: &BTreeMap<NodeId, f64>) -> Vec<Contributor> {
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
            id: n.id.clone(),
            src: n.src.clone(),
            w: base_w.get(&n.id).copied().unwrap_or(0.0),
            reason: "deleted without a retraction".into(),
        })
        .collect()
}

/// 2026-10-23: H: tainted nodes still live, deletions, and retractions without a reason.
fn hygiene<P: Checks>(c: &Ctx<'_>, gone: Option<&[Contributor]>) -> DimResult {
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
fn owed<P: Checks>(c: &Ctx<'_>) -> DimResult {
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

/// 2026-10-23: Score `head` (and its deletions against `base`, when given).
pub fn score<P: Checks>(head: &Bundle, base: Option<&Bundle>, cfg: &CheckConfig) -> Result<Score> {
    let c = Ctx {
        b: head,
        w: weights(head, cfg)?,
    };
    let gone = match base {
        Some(b) => Some(vanished(&c, b, &weights(b, cfg)?)),
        None => None,
    };
    let not_yet = |d: Dim, why: &str| absent(d, &format!("not computed yet: {why}"));
    let dims = vec![
        not_yet(Dim::F, "no [[derived]] declarations are read"),
        referential(&c),
        coverage::<P>(&c),
        not_yet(Dim::G, "no CI gate results are supplied"),
        not_yet(Dim::P, "run fingerprints are not scored"),
        not_yet(Dim::S, "no syntax anchors are recorded"),
        hygiene::<P>(&c, gone.as_deref()),
        not_yet(Dim::T, "no commit window is supplied"),
        not_yet(Dim::X, "records are not keyed"),
        owed::<P>(&c),
    ];
    Ok(Score {
        dims,
        vanished: gone,
    })
}

/// 2026-10-23: One dimension as JSON.
pub fn dim_json(r: &DimResult) -> Value {
    let mut o = Map::new();
    o.insert("dim".into(), Value::String(r.dim.letter().into()));
    o.insert("name".into(), Value::String(r.dim.name().into()));
    o.insert("value".into(), r.value.map_or(Value::Null, round));
    o.insert("mode".into(), Value::String(r.mode.name().into()));
    if let Some(x) = &r.reason {
        o.insert("reason".into(), Value::String(x.clone()));
    }
    o.insert("numerator_w".into(), round(r.numerator_w));
    o.insert("denominator_w".into(), round(r.denominator_w));
    o.insert(
        "contributors".into(),
        Value::from(
            r.contributors
                .iter()
                .map(contributor_json)
                .collect::<Vec<_>>(),
        ),
    );
    Value::Object(o)
}

/// 2026-10-23: One contributor as JSON.
pub fn contributor_json(c: &Contributor) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), Value::String(c.id.to_string()));
    o.insert("src".into(), Value::String(c.src.clone()));
    o.insert("w".into(), round(c.w));
    o.insert("reason".into(), Value::String(c.reason.clone()));
    Value::Object(o)
}

/// 2026-10-23: Six decimals, so reports are stable across platforms' last-bit float noise.
pub fn round(x: f64) -> Value {
    let s = format!("{x:.6}");
    s.parse::<f64>().map_or(Value::Null, Value::from)
}
