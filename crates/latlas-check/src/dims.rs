// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The ten dimensions (check part §1): the types, the shared ratio and the `score`
//! entry point. R, C, H and O come from the bundle ([`crate::dim_graph`]); F, G and T from host
//! facts ([`crate::dim_host`]); X from pack keys ([`crate::dim_x`]); P and S are `absent` with the
//! input each still lacks.
//!
//! Owner: latlas-check.
//! Invariants:
//! - Every ratio is weighted by [`crate::weights`] and counts live nodes only (not retracted, not
//!   superseded), so retracting a bad record is honest and deleting it is not (H `vanished`).
//! - An empty denominator is `absent`, never 0 or 1.

use crate::config::CheckConfig;
use crate::dim_graph::{coverage, hygiene, owed, referential, vanished};
use crate::dim_host::{freshness, gate_rate, traceability};
use crate::dim_x::contradictions;
use crate::facts::CheckFacts;
use crate::pack::Checks;
use crate::weights::weights;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use latlas_core::Bundle;
use latlas_core::id::{NodeId, core_rels};
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
    /// 2026-10-24: A node id, or the path / commit a host-input dimension names.
    pub id: String,
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

pub(crate) fn absent(dim: Dim, reason: &str) -> DimResult {
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
pub(crate) fn ratio(
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

pub(crate) struct Ctx<'a> {
    pub(crate) b: &'a Bundle,
    pub(crate) w: BTreeMap<NodeId, f64>,
}

impl Ctx<'_> {
    pub(crate) fn w(&self, id: &NodeId) -> f64 {
        self.w.get(id).copied().unwrap_or(0.0)
    }
    pub(crate) fn src(&self, id: &NodeId) -> String {
        self.b.node(id).map(|n| n.src.clone()).unwrap_or_default()
    }
    pub(crate) fn live(&self, id: &NodeId) -> bool {
        self.b.into(id, core_rels::Retracts).next().is_none()
            && self.b.into(id, core_rels::Supersedes).next().is_none()
    }
    pub(crate) fn contributor(&self, id: &NodeId, w: f64, reason: String) -> Contributor {
        Contributor {
            id: id.to_string(),
            src: self.src(id),
            w,
            reason,
        }
    }
}

/// 2026-10-23: Score `head` (and its deletions against `base`, when given) with the host's facts.
pub fn score<P: Checks>(
    head: &Bundle,
    base: Option<&Bundle>,
    cfg: &CheckConfig,
    facts: &CheckFacts,
) -> Result<Score> {
    let c = Ctx {
        b: head,
        w: weights(head, cfg, facts)?,
    };
    let gone = match base {
        Some(b) => Some(vanished(&c, b, &weights(b, cfg, facts)?)),
        None => None,
    };
    let not_yet = |d: Dim, why: &str| absent(d, &format!("not computed yet: {why}"));
    let dims = vec![
        freshness(cfg, facts),
        referential::<P>(&c, cfg.prose.weight),
        coverage::<P>(&c),
        gate_rate::<P>(&c, facts),
        not_yet(Dim::P, "run fingerprints are not scored"),
        not_yet(Dim::S, "no syntax anchors are recorded"),
        hygiene::<P>(&c, gone.as_deref()),
        traceability(&c, cfg, facts),
        contradictions::<P>(&c),
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
    o.insert("id".into(), Value::String(c.id.clone()));
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
