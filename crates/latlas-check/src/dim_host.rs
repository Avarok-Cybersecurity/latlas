// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The dimensions that need host facts ([`crate::facts`]): F (derived artifacts no
//! older than their inputs), G (gate pass rate from CI results) and T (traceability: decisions
//! with evidence, window commits a record names). A missing fact makes its dimension `absent`
//! with the reason (check part §1).
//!
//! Owner: latlas-check.
//! Invariants: F weights an output by its source importance; T_c weights a commit by the heaviest
//! importance among the tracked paths it touched; T is the geometric mean of T_d and T_c.

use crate::config::CheckConfig;
use crate::dims::{Contributor, Ctx, Dim, DimResult, Mode, absent, ratio};
use crate::facts::CheckFacts;
use crate::pack::Checks;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::id::{core_labels, core_rels};

fn host(id: &str, src: &str, w: f64, reason: String) -> Contributor {
    Contributor {
        id: id.to_string(),
        src: src.to_string(),
        w,
        reason,
    }
}

/// 2026-10-24: F: derived outputs whose last commit is no older than every input's.
pub(crate) fn freshness(cfg: &CheckConfig, facts: &CheckFacts) -> DimResult {
    if cfg.derived.is_empty() {
        return absent(Dim::F, "no [[derived]] declarations");
    }
    let Some(lc) = &facts.last_commit else {
        return absent(Dim::F, "the host supplied no git history");
    };
    let time = |p: &str| -> Option<i64> {
        if p.ends_with('/') {
            lc.iter().filter(|(k, _)| k.starts_with(p)).map(|(_, v)| v.1).max()
        } else {
            lc.get(p).map(|v| v.1)
        }
    };
    let (mut all, mut good, mut out) = (0.0, 0.0, Vec::new());
    for d in &cfg.derived {
        let w = cfg.importance(&d.output).unwrap_or(0.0);
        all += w;
        let newest_in = d.inputs.iter().filter_map(|i| time(i)).max();
        match (time(&d.output), newest_in) {
            (Some(o), Some(i)) if o >= i => good += w,
            (Some(_), Some(_)) => out.push(host(&d.output, &d.output, w, "older than an input".into())),
            (None, _) => out.push(host(&d.output, &d.output, w, "the output has no commit".into())),
            (Some(_), None) => out.push(host(&d.output, &d.output, w, "no input has a commit".into())),
        }
    }
    ratio(Dim::F, good, all, Mode::Full, None, out)
}

/// 2026-10-24: G: of the gated claims of live outcomes with a CI result, the weight that passed;
/// claims without a result are listed as `unevaluated`, not scored.
pub(crate) fn gate_rate<P: Checks>(c: &Ctx<'_>, facts: &CheckFacts) -> DimResult {
    let Some(res) = &facts.gate_results else {
        return absent(Dim::G, "the host supplied no CI gate results");
    };
    let (mut all, mut good, mut out) = (0.0, 0.0, Vec::new());
    for n in c.b.nodes.iter().filter(|n| P::is_outcome(n) && c.live(&n.id)) {
        for cl in c.b.out(&n.id, core_rels::Claims) {
            let w = c.w(&cl.to);
            for g in c.b.out(&cl.to, core_rels::GatedBy) {
                match res.get(g.to.key()) {
                    Some(true) => {
                        all += w;
                        good += w;
                    }
                    Some(false) => {
                        all += w;
                        out.push(c.contributor(&cl.to, w, format!("{} failed", g.to)));
                    }
                    None => out.push(c.contributor(&cl.to, 0.0, format!("unevaluated: {}", g.to))),
                }
            }
        }
    }
    ratio(Dim::G, good, all, Mode::Full, None, out)
}

/// 2026-10-24: T: `sqrt(T_d · T_c)`; T_d over live decisions with evidence (an incoming
/// `EVIDENCE_FOR`, a `CITES` of a defined node, or a gated claim), T_c over the window's commits
/// that touched a tracked path, traced when a `Commit` node's key prefixes the sha.
pub(crate) fn traceability(c: &Ctx<'_>, cfg: &CheckConfig, facts: &CheckFacts) -> DimResult {
    let (mut dall, mut dgood, mut out) = (0.0, 0.0, Vec::new());
    for d in c.b.of(core_labels::Decision).filter(|d| c.live(&d.id)) {
        let w = c.w(&d.id);
        dall += w;
        let evidenced = c.b.into(&d.id, core_rels::EvidenceFor).next().is_some()
            || c.b.out(&d.id, core_rels::Cites).any(|e| e.to.label() != core_labels::Unresolved)
            || c.b
                .out(&d.id, core_rels::Claims)
                .any(|e| c.b.out(&e.to, core_rels::GatedBy).next().is_some());
        if evidenced {
            dgood += w;
        } else {
            out.push(c.contributor(&d.id, w, "a decision with no evidence".into()));
        }
    }
    let tc = facts.window.as_ref().map(|win| {
        let tracked: BTreeSet<&str> = c.b.sources.iter().map(|s| s.path.as_str()).collect();
        let commits: Vec<&str> = c
            .b
            .of(core_labels::Commit)
            .map(|n| n.id.key())
            .filter(|k| k.len() >= 7)
            .collect();
        let (mut all, mut good) = (0.0, 0.0);
        for wc in win {
            let touched: Vec<&String> = wc.paths.iter().filter(|p| tracked.contains(p.as_str())).collect();
            let Some(w) = touched
                .iter()
                .filter_map(|p| cfg.importance(p))
                .reduce(f64::max)
            else {
                continue;
            };
            all += w;
            if commits.iter().any(|k| wc.sha.starts_with(k)) {
                good += w;
            } else {
                let short: String = wc.sha.chars().take(10).collect();
                out.push(host(&format!("commit:{short}"), touched[0], w, "no record names this commit".into()));
            }
        }
        (good, all)
    });
    let td = (dall > 0.0).then_some(dgood / dall);
    let tc_v = tc.and_then(|(g, a)| (a > 0.0).then_some(g / a));
    let (value, mode, reason) = match (td, tc_v) {
        (Some(d), Some(cm)) => (libm::sqrt(d * cm), Mode::Full, None),
        (Some(d), None) => (d, Mode::Partial, Some("T_d only: no window commits touched a tracked path, or no window was supplied".to_string())),
        (None, Some(cm)) => (cm, Mode::Partial, Some("T_c only: no live decisions".to_string())),
        (None, None) => return absent(Dim::T, "no live decisions and no window commits"),
    };
    let mut r = ratio(Dim::T, value, 1.0, mode, reason, out);
    r.numerator_w = dgood + tc.map_or(0.0, |x| x.0);
    r.denominator_w = dall + tc.map_or(0.0, |x| x.1);
    r
}
