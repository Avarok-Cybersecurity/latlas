// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The check config (`.latlas/check.toml`, check part §2–§3): the weights and the
//! ratchet's tolerances. Every dimension's epsilon is listed explicitly and every source path
//! gets an importance from an explicit prefix table (PCND: nothing defaulted).
//!
//! Owner: latlas-check.
//! Invariants: unknown keys are refused; epsilons are finite and >= 0; importances in [0.1, 1].

use crate::dims::Dim;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_error::{ErrorCode, Result, error};
use serde::Deserialize;

/// 2026-10-23: The schema id a check config carries.
pub const SCHEMA: &str = "latlas-check/v1";

/// 2026-10-23: The whole check config.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    pub schema: String,
    pub weights: Weights,
    pub ratchet: Ratchet,
    pub window: Window,
    pub prose: Prose,
    /// 2026-10-24: The derived artifacts F is computed over; an absent list declares none (F is
    /// then `absent`, check part §1).
    #[serde(default)]
    pub derived: Vec<Derived>,
}

/// 2026-10-24: The commit window churn and traceability count over.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub days: u32,
}

/// 2026-10-24: Prose references (R): their weight relative to a structural edge.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Prose {
    pub weight: f64,
}

/// 2026-10-24: One derived artifact (F): fresh when its last commit is no older than every
/// input's. An input ending in `/` is every tracked path under it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Derived {
    pub output: String,
    pub inputs: Vec<String>,
}

/// 2026-10-23: `w(n) = imp(n) · churn(n) · pr(n)` (check part §2).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Weights {
    /// 2026-10-23: Source-path prefixes and their importance; the longest matching prefix wins,
    /// and a source no prefix matches is a config error (declare `""` to cover everything).
    pub importance: Vec<Importance>,
    /// 2026-10-24: `off`, or `commits`: `1 + ln(1 + commits touching the node's source in the
    /// window)` (needs the host's commit window).
    pub churn: String,
    pub pagerank: PageRank,
}

/// 2026-10-23: One importance prefix.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Importance {
    pub prefix: String,
    pub value: f64,
}

/// 2026-10-23: The PageRank factor (damping, iterations).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageRank {
    pub damping: f64,
    pub iterations: u32,
}

/// 2026-10-23: The ratchet (check part §3).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ratchet {
    /// 2026-10-23: Every dimension's tolerance, by letter (`R = 0.0`), all ten listed.
    pub epsilon: BTreeMap<String, f64>,
    /// 2026-10-23: Absolute minimums by letter; an absent table is "floor: none", reported.
    #[serde(default)]
    pub floor: Option<BTreeMap<String, f64>>,
}

fn bad(key: String, why: &str) -> latlas_error::Error {
    error!(ErrorCode::LatlasConfig, key, why.to_string())
}

impl CheckConfig {
    /// 2026-10-23: Parse and validate.
    pub fn parse(text: &str) -> Result<CheckConfig> {
        let c: CheckConfig = toml::from_str(text)
            .map_err(|e| error!(ErrorCode::LatlasConfig, "(check config)", e.to_string()))?;
        if c.schema != SCHEMA {
            return Err(bad("schema".into(), "must be latlas-check/v1"));
        }
        if !matches!(c.weights.churn.as_str(), "off" | "commits") {
            return Err(bad("weights.churn".into(), "must be `off` or `commits`"));
        }
        if c.window.days == 0 {
            return Err(bad("window.days".into(), "must be >= 1"));
        }
        if !(0.0..=1.0).contains(&c.prose.weight) {
            return Err(bad("prose.weight".into(), "must lie in [0, 1]"));
        }
        for d in &c.derived {
            if d.inputs.is_empty() {
                return Err(bad(format!("derived `{}`", d.output), "lists no input"));
            }
        }
        let pr = c.weights.pagerank;
        if !(pr.damping > 0.0 && pr.damping < 1.0) || pr.iterations == 0 {
            return Err(bad(
                "weights.pagerank".into(),
                "damping in (0, 1) and iterations >= 1",
            ));
        }
        if c.weights.importance.is_empty() {
            return Err(bad("weights.importance".into(), "lists no prefix"));
        }
        for i in &c.weights.importance {
            if !(0.1..=1.0).contains(&i.value) {
                return Err(bad(
                    format!("weights.importance `{}`", i.prefix),
                    "must lie in [0.1, 1]",
                ));
            }
        }
        for d in Dim::ALL {
            let k = d.letter();
            match c.ratchet.epsilon.get(k) {
                Some(e) if e.is_finite() && *e >= 0.0 => {}
                Some(_) => return Err(bad(format!("ratchet.epsilon.{k}"), "must be >= 0")),
                None => return Err(bad(format!("ratchet.epsilon.{k}"), "is required")),
            }
        }
        let known: Vec<&str> = Dim::ALL.iter().map(|d| d.letter()).collect();
        let tables = [Some(&c.ratchet.epsilon), c.ratchet.floor.as_ref()];
        for t in tables.into_iter().flatten() {
            if let Some(k) = t.keys().find(|k| !known.contains(&k.as_str())) {
                return Err(bad(format!("ratchet.{k}"), "is not a dimension letter"));
            }
        }
        Ok(c)
    }

    /// 2026-10-23: The importance of a source path (the longest matching prefix).
    pub fn importance(&self, path: &str) -> Option<f64> {
        self.weights
            .importance
            .iter()
            .filter(|i| path.starts_with(i.prefix.as_str()))
            .max_by_key(|i| i.prefix.len())
            .map(|i| i.value)
    }

    /// 2026-10-23: Dimension `d`'s tolerance (validated present at parse).
    pub fn epsilon(&self, d: Dim) -> f64 {
        self.ratchet.epsilon.get(d.letter()).copied().unwrap_or(0.0)
    }

    /// 2026-10-23: Dimension `d`'s floor, if declared.
    pub fn floor(&self, d: Dim) -> Option<f64> {
        self.ratchet.floor.as_ref()?.get(d.letter()).copied()
    }
}
