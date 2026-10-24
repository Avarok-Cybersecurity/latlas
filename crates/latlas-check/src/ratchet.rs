// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The ratchet (check part §3): base and head scored by the same binary and config in
//! the same run (no stored baseline), compared per dimension, plus the fatal findings that fail
//! whatever the deltas.
//!
//! Owner: latlas-check.
//! Invariants:
//! - A dimension regresses only when both sides are comparable: `head < base − ε` (or below its
//!   declared floor).
//! - A head that lost a capability the base had (`partial` vs `full`) is excluded, never failed:
//!   an outage is not a regression of the repository.
//! - Exit codes: 0 ok, 3 regression, 4 fatal (fatal wins).

use crate::config::CheckConfig;
use crate::dims::{Contributor, Dim, DimResult, Mode, Score};
use crate::weights::src_path;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::Bundle;

/// 2026-10-23: One dimension's verdict.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ok,
    Regressed { by: f64, epsilon: f64 },
    BelowFloor { floor: f64 },
    Excluded(String),
    NotComparable(String),
}

impl Status {
    pub fn name(&self) -> String {
        match self {
            Status::Ok => "ok".into(),
            Status::Regressed { epsilon, .. } => format!("REGRESSED (ε {epsilon})"),
            Status::BelowFloor { floor } => format!("BELOW FLOOR ({floor})"),
            Status::Excluded(r) => format!("excluded: {r}"),
            Status::NotComparable(r) => format!("not compared: {r}"),
        }
    }
    pub fn fails(&self) -> bool {
        matches!(self, Status::Regressed { .. } | Status::BelowFloor { .. })
    }
}

/// 2026-10-23: A fatal finding.
#[derive(Clone, Debug, PartialEq)]
pub struct Fatal {
    pub code: String,
    pub at: String,
    pub msg: String,
}

/// 2026-10-23: The comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub rows: Vec<(Dim, Option<f64>, Option<f64>, Status)>,
    pub fatal: Vec<Fatal>,
    pub exit: u8,
}

fn verdict(cfg: &CheckConfig, d: Dim, base: Option<&DimResult>, head: &DimResult) -> Status {
    let floor = cfg.floor(d);
    let Some(h) = head.value else {
        return Status::NotComparable(format!("head {}", head.mode.name()));
    };
    if let Some(f) = floor
        && h < f
    {
        return Status::BelowFloor { floor: f };
    }
    let Some(base) = base else {
        return Status::NotComparable("no base".into());
    };
    let Some(b) = base.value else {
        return Status::NotComparable(format!("base {}", base.mode.name()));
    };
    if head.mode == Mode::Partial && base.mode == Mode::Full {
        return Status::Excluded(format!(
            "capability unavailable ({})",
            head.reason.clone().unwrap_or_default()
        ));
    }
    let epsilon = cfg.epsilon(d);
    if h < b - epsilon {
        Status::Regressed { by: b - h, epsilon }
    } else {
        Status::Ok
    }
}

/// 2026-10-23: The fatal findings of `head`: the bundle's own fatal codes and deletions from a
/// source of importance 1.
pub fn fatal(head: &Bundle, vanished: Option<&[Contributor]>, cfg: &CheckConfig) -> Vec<Fatal> {
    let mut out: Vec<Fatal> = head
        .fatal()
        .into_iter()
        .map(|f| Fatal {
            code: f.code.to_string(),
            at: f.at.clone(),
            msg: f.msg.clone(),
        })
        .collect();
    for v in vanished.into_iter().flatten() {
        if cfg.importance(src_path(&v.src)) == Some(1.0) {
            out.push(Fatal {
                code: "vanished".into(),
                at: v.src.clone(),
                msg: format!(
                    "{} was deleted without a retraction from an importance-1.0 source",
                    v.id
                ),
            });
        }
    }
    out
}

/// 2026-10-23: Compare `head` with `base` (`None`: score only, nothing can regress).
pub fn compare(
    cfg: &CheckConfig,
    base: Option<&Score>,
    head: &Score,
    head_bundle: &Bundle,
) -> Comparison {
    let rows: Vec<_> = Dim::ALL
        .iter()
        .filter_map(|d| {
            let h = head.get(*d)?;
            let b = base.and_then(|s| s.get(*d));
            Some((*d, b.and_then(|x| x.value), h.value, verdict(cfg, *d, b, h)))
        })
        .collect();
    let fatal = fatal(head_bundle, head.vanished.as_deref(), cfg);
    let exit = if !fatal.is_empty() {
        4
    } else if rows.iter().any(|r| r.3.fails()) {
        3
    } else {
        0
    };
    Comparison { rows, fatal, exit }
}
