// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The dimensions that need host facts (F, G, T), prose references in R, and churn
//! weights, each moved by exactly the fact it reads.

mod common;

use common::{Adr, baseline, config, id};
use latlas_check::dims::{Dim, Mode};
use latlas_check::weights::weights;
use latlas_check::{CheckConfig, CheckFacts, WindowCommit, score};
use latlas_core::id::core_rels;
use std::collections::BTreeMap;

fn cfg_with(extra: &str) -> CheckConfig {
    CheckConfig::parse(&format!("{}\n{extra}", common::CONFIG)).unwrap()
}

fn commits(m: &[(&str, &str, i64)]) -> CheckFacts {
    CheckFacts {
        last_commit: Some(m.iter().map(|(p, s, t)| (p.to_string(), (s.to_string(), *t))).collect()),
        ..CheckFacts::default()
    }
}

#[test]
fn a_derived_output_older_than_an_input_is_stale_and_a_dir_input_counts_every_path() {
    let cfg = cfg_with("[[derived]]\noutput = \"docs/ref.md\"\ninputs = [\"adr.md\", \"src/\"]\n");
    let b = baseline().seal();
    let fresh = commits(&[("docs/ref.md", "c3", 300), ("adr.md", "c1", 100), ("src/a.rs", "c2", 200)]);
    let s = score::<Adr>(&b, None, &cfg, &fresh).unwrap();
    assert_eq!(s.get(Dim::F).unwrap().value, Some(1.0));
    let stale = commits(&[("docs/ref.md", "c3", 300), ("adr.md", "c1", 100), ("src/a.rs", "c4", 400)]);
    let f = score::<Adr>(&b, None, &cfg, &stale).unwrap();
    let f = f.get(Dim::F).unwrap();
    assert_eq!(f.value, Some(0.0));
    assert_eq!(f.contributors[0].id, "docs/ref.md");
    let none = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    assert_eq!(none.get(Dim::F).unwrap().mode, Mode::Absent, "no git history: F is absent, not 1");
}

#[test]
fn gate_pass_rate_counts_results_and_lists_unevaluated_claims_unscored() {
    let b = baseline().seal();
    let cfg = config();
    let w = weights(&b, &cfg, &CheckFacts::default()).unwrap();
    let mut res = BTreeMap::new();
    res.insert("test/a".to_string(), true);
    res.insert("manual/review".to_string(), false);
    let facts = CheckFacts {
        gate_results: Some(res),
        ..CheckFacts::default()
    };
    let g = score::<Adr>(&b, None, &cfg, &facts).unwrap();
    let g = g.get(Dim::G).unwrap();
    let (c1, c2) = (w[&id("claim:d1/1")], w[&id("claim:d2/1")]);
    assert!((g.value.unwrap() - c1 / (c1 + c2)).abs() < 1e-12, "{g:?}");
    let mut only = BTreeMap::new();
    only.insert("test/a".to_string(), true);
    let facts = CheckFacts {
        gate_results: Some(only),
        ..CheckFacts::default()
    };
    let s = score::<Adr>(&b, None, &cfg, &facts).unwrap();
    let g = s.get(Dim::G).unwrap();
    assert_eq!(g.value, Some(1.0), "an unevaluated claim is not a failure");
    assert!(g.contributors.iter().any(|c| c.reason.starts_with("unevaluated")));
}

#[test]
fn traceability_is_the_geometric_mean_and_an_unnamed_commit_lowers_it() {
    let cfg = config();
    let mut f = baseline();
    let d1 = id("decision:d1");
    let ev = f.decision("ev", &[]);
    f.edge(&ev, core_rels::EvidenceFor, &d1);
    let commit = id("commit:abc1234");
    f.b.mention(&commit, "adr.md:5");
    f.edge(&d1, core_rels::ImplementedBy, &commit);
    let b = f.seal();
    let win = |shas: &[&str]| CheckFacts {
        window: Some(
            shas.iter()
                .map(|s| WindowCommit { sha: s.to_string(), paths: vec!["adr.md".into()] })
                .collect(),
        ),
        ..CheckFacts::default()
    };
    let traced = score::<Adr>(&b, None, &cfg, &win(&["abc1234ffff"])).unwrap();
    let both = score::<Adr>(&b, None, &cfg, &win(&["abc1234ffff", "deadbeef00"])).unwrap();
    let (t1, t2) = (traced.get(Dim::T).unwrap(), both.get(Dim::T).unwrap());
    assert_eq!(t1.mode, Mode::Full);
    let td = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let td = td.get(Dim::T).unwrap();
    assert_eq!(td.mode, Mode::Partial, "no window: T_d only");
    assert!((t1.value.unwrap() - td.value.unwrap().sqrt()).abs() < 1e-12, "T_c = 1 ⇒ T = sqrt(T_d)");
    assert!((t2.value.unwrap() - (td.value.unwrap() * 0.5).sqrt()).abs() < 1e-12, "{t2:?}");
    assert!(t2.contributors.iter().any(|c| c.id == "commit:deadbeef00"));
}

#[test]
fn a_dangling_prose_reference_lowers_r_at_the_prose_weight() {
    let cfg = config();
    let mut f = baseline();
    let d1 = id("decision:d1");
    f.b.text(&d1, "body", "rests on `decision:d2` and `decision:nowhere`, arXiv 2113.00001 and 2109.06049");
    let b = f.seal();
    let s = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let r = s.get(Dim::R).unwrap();
    let bad: Vec<&str> = r.contributors.iter().map(|c| c.reason.as_str()).collect();
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad.iter().any(|x| x.contains("decision:nowhere")) && bad.iter().any(|x| x.contains("2113.00001")));
    let w = weights(&b, &cfg, &CheckFacts::default()).unwrap()[&d1];
    assert!((r.contributors[0].w - 0.5 * w).abs() < 1e-12, "prose weight 0.5 of the referrer");
}

#[test]
fn churn_weights_the_worked_on_source_up_and_needs_a_window() {
    let cfg = CheckConfig::parse(&common::CONFIG.replace("churn = \"off\"", "churn = \"commits\"")).unwrap();
    let b = baseline().seal();
    assert!(weights(&b, &cfg, &CheckFacts::default()).is_err(), "churn without a window fails fast");
    let three = CheckFacts {
        window: Some((0..3).map(|i| WindowCommit { sha: format!("{i}"), paths: vec!["adr.md".into()] }).collect()),
        ..CheckFacts::default()
    };
    let off = weights(&b, &config(), &CheckFacts::default()).unwrap();
    let on = weights(&b, &cfg, &three).unwrap();
    let d = id("decision:d1");
    assert!((on[&d] / off[&d] - (1.0 + 3f64.ln_1p())).abs() < 1e-12);
}
