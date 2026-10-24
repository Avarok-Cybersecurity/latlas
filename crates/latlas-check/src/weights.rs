// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: Node weights `w(n) = imp(n) · churn(n) · pr(n)` (check part §2). `pr` is PageRank
//! over the bundle's edges, normalised to mean 1, so a node many records rest on carries much
//! weight and an orphan little; `churn` (when `commits`) is `1 + ln(1 + commits touching the
//! node's source in the window)`, so the code being worked on counts most.
//!
//! Owner: latlas-check.
//! Invariants: deterministic (nodes in bundle order, fixed iteration count); every node of the
//! bundle has a weight; a source path no importance prefix matches is an error.

use crate::config::CheckConfig;
use crate::facts::CheckFacts;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use latlas_core::Bundle;
use latlas_core::id::NodeId;
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-23: The source path of a `path:line` src.
pub fn src_path(src: &str) -> &str {
    match src.rsplit_once(':') {
        Some((p, l)) if !l.is_empty() && l.bytes().all(|c| c.is_ascii_digit()) => p,
        _ => src,
    }
}

/// 2026-10-23: PageRank over `b` (dangling mass spread evenly), normalised to mean 1.
#[allow(clippy::cast_precision_loss)]
pub fn pagerank(b: &Bundle, damping: f64, iterations: u32) -> Vec<f64> {
    let n = b.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index = |id: &NodeId| b.nodes.binary_search_by(|x| x.id.cmp(id)).ok();
    let mut out_deg = vec![0usize; n];
    let mut links = Vec::with_capacity(b.edges.len());
    for e in &b.edges {
        if let (Some(f), Some(t)) = (index(&e.from), index(&e.to)) {
            out_deg[f] += 1;
            links.push((f, t));
        }
    }
    let nf = n as f64;
    let mut r = vec![1.0 / nf; n];
    for _ in 0..iterations {
        let dangling: f64 = (0..n).filter(|&i| out_deg[i] == 0).map(|i| r[i]).sum();
        let base = (1.0 - damping) / nf + damping * dangling / nf;
        let mut next = vec![base; n];
        for &(f, t) in &links {
            next[t] += damping * r[f] / out_deg[f] as f64;
        }
        r = next;
    }
    let mean: f64 = r.iter().sum::<f64>() / nf;
    r.iter().map(|x| x / mean).collect()
}

/// 2026-10-23: Every node's weight.
pub fn weights(b: &Bundle, cfg: &CheckConfig, facts: &CheckFacts) -> Result<BTreeMap<NodeId, f64>> {
    let pr = pagerank(
        b,
        cfg.weights.pagerank.damping,
        cfg.weights.pagerank.iterations,
    );
    let churn = cfg.weights.churn == "commits";
    if churn && facts.window.is_none() {
        return Err(error!(
            ErrorCode::LatlasConfig,
            "weights.churn",
            "is `commits` but the host supplied no commit window"
        ));
    }
    let mut out = BTreeMap::new();
    for (n, p) in b.nodes.iter().zip(pr) {
        let path = src_path(&n.src);
        let imp = cfg.importance(path).ok_or_else(|| {
            error!(
                ErrorCode::LatlasConfig,
                "weights.importance",
                alloc::format!("no prefix matches the source `{path}`")
            )
        })?;
        let c = if churn {
            #[allow(clippy::cast_precision_loss)]
            let k = facts.touches(path).unwrap_or(0) as f64;
            1.0 + libm::log1p(k)
        } else {
            1.0
        };
        out.insert(n.id.clone(), imp * c * p);
    }
    Ok(out)
}
