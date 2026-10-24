// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The score and the ratchet on a fixture with a known vector (check part §8): each
//! test edits one thing and asserts the dimension it must move, by how much, and that the others
//! do not move.

mod common;

use common::{Adr, baseline, config, id};
use latlas_check::dims::{Dim, Mode};
use latlas_check::weights::{pagerank, weights};
use latlas_check::{CheckConfig, CheckFacts, compare, score};
use latlas_core::id::core_rels;

fn value(s: &latlas_check::Score, d: Dim) -> f64 {
    s.get(d).unwrap().value.unwrap()
}

#[test]
fn claim_coverage_is_the_weighted_credit_of_live_outcomes() {
    let b = baseline().seal();
    let cfg = config();
    let w = weights(&b, &cfg, &CheckFacts::default()).unwrap();
    let s = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let wd = |k: &str| w[&id(&format!("decision:{k}"))];
    let want = (wd("d1") + 0.5 * wd("d2")) / (wd("d1") + wd("d2") + wd("d3") + wd("code"));
    let c = s.get(Dim::C).unwrap();
    assert!((c.value.unwrap() - want).abs() < 1e-12, "{c:?} want {want}");
    let ids: Vec<_> = c.contributors.iter().map(|x| x.id.clone()).collect();
    for k in ["decision:d2", "decision:d3", "decision:code"] {
        assert!(ids.contains(&id(k).to_string()), "{k} is short of full credit: {ids:?}");
    }
    assert!(!ids.contains(&id("decision:d1").to_string()));
}

#[test]
fn removing_a_gate_lowers_claim_coverage_and_nothing_counts_a_bare_claim() {
    let base = baseline().seal();
    let mut f = baseline();
    let d4 = f.decision("d4", &[]);
    let c = id("claim:d4/1");
    f.b.define(c.clone(), "adr.md:91".into(), "c4", Default::default());
    f.edge(&d4, core_rels::Claims, &c);
    let head = f.seal();
    let cfg = config();
    let (sb, sh) = (
        score::<Adr>(&base, None, &cfg, &CheckFacts::default()).unwrap(),
        score::<Adr>(&head, None, &cfg, &CheckFacts::default()).unwrap(),
    );
    assert!(
        value(&sh, Dim::C) < value(&sb, Dim::C),
        "an ungated claim counted"
    );
    assert!(
        sh.get(Dim::C)
            .unwrap()
            .contributors
            .iter()
            .any(|x| x.id == d4.to_string())
    );
}

#[test]
fn a_reasoned_retraction_keeps_hygiene_and_a_silent_deletion_lowers_it() {
    let cfg = config();
    let base = baseline().seal();
    let mut retracted = baseline();
    let r = retracted.decision("r1", &[("why", "the A/B was noise")]);
    retracted.edge(&r, core_rels::Retracts, &id("decision:d3"));
    let retracted = retracted.seal();
    let s = score::<Adr>(&retracted, Some(&base), &cfg, &CheckFacts::default()).unwrap();
    assert_eq!(value(&s, Dim::H), 1.0, "{:?}", s.get(Dim::H));
    assert!(s.vanished.as_ref().unwrap().is_empty());
    let unretracted = score::<Adr>(&base, None, &cfg, &CheckFacts::default()).unwrap();
    assert!(
        value(&s, Dim::C) > value(&unretracted, Dim::C),
        "a retracted outcome still counts in C"
    );

    let mut both = common::Fx::new();
    let d1 = both.decision("d1", &[]);
    let r = both.decision("r1", &[("why", "wrong premise")]);
    both.edge(&r, core_rels::Retracts, &id("decision:d3"));
    both.claim(&d1, "test", "test/a");
    let both = both.seal();
    let s = score::<Adr>(&both, Some(&base), &cfg, &CheckFacts::default()).unwrap();
    let gone: Vec<_> = s.vanished.as_ref().unwrap().iter().map(|c| c.id.clone()).collect();
    assert!(!gone.contains(&id("decision:d3").to_string()), "a retracted, deleted record vanished: {gone:?}");
    assert!(gone.contains(&id("decision:d2").to_string()), "{gone:?}");

    let mut deleted = common::Fx::new();
    let d1 = deleted.decision("d1", &[]);
    let d2 = deleted.decision("d2", &[]);
    let code = deleted.decision("code", &[]);
    deleted.claim(&d1, "test", "test/a");
    deleted.claim(&d2, "manual", "manual/review");
    for d in [&d1, &d2, &code] {
        deleted.edge(d, core_rels::ImplementedBy, &code);
    }
    let deleted = deleted.seal();
    let s = score::<Adr>(&deleted, Some(&base), &cfg, &CheckFacts::default()).unwrap();
    let gone = s.vanished.as_ref().unwrap();
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert_eq!(gone[0].id, id("decision:d3").to_string());
    assert!(value(&s, Dim::H) < 1.0);
    let cmp = compare(
        &cfg,
        Some(&score::<Adr>(&base, None, &cfg, &CheckFacts::default()).unwrap()),
        &s,
        &deleted,
    );
    assert_eq!(
        cmp.exit, 4,
        "a deletion from an importance-1.0 source is fatal: {cmp:?}"
    );
    assert!(cmp.fatal.iter().any(|f| f.code == "vanished"));
}

#[test]
fn a_reasonless_retraction_and_a_live_dependant_lower_hygiene() {
    let cfg = config();
    let mut f = baseline();
    let r = f.decision("r1", &[]);
    f.edge(&r, core_rels::Retracts, &id("decision:d1"));
    let dep = f.decision("dep", &[]);
    f.edge(&dep, core_rels::DependsOn, &id("decision:d1"));
    let b = f.seal();
    let s = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let h = s.get(Dim::H).unwrap();
    assert_eq!(h.mode, Mode::Partial, "no base: vanished is not counted");
    assert_eq!(h.value, Some(0.0), "{h:?}");
    let ids: Vec<_> = h.contributors.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&dep.to_string()) && ids.contains(&r.to_string()), "{ids:?}");
    assert!(
        s.get(Dim::C)
            .unwrap()
            .contributors
            .iter()
            .all(|c| c.id != id("decision:d1").to_string()),
        "a retracted outcome is not counted in C"
    );
}

#[test]
fn a_dangling_reference_lowers_referential_integrity_by_its_referrer() {
    let cfg = config();
    let base = baseline().seal();
    let mut f = baseline();
    let d1 = id("decision:d1");
    f.edge(&d1, core_rels::Cites, &id("decision:nowhere"));
    let head = f.seal();
    let sh = score::<Adr>(&head, None, &cfg, &CheckFacts::default()).unwrap();
    let r = sh.get(Dim::R).unwrap();
    assert!(r.value.unwrap() < 1.0);
    assert_eq!(r.contributors[0].id, d1.to_string());
    let sb = score::<Adr>(&base, None, &cfg, &CheckFacts::default()).unwrap();
    assert_eq!(value(&sb, Dim::R), 1.0);
    let cmp = compare(&cfg, Some(&sb), &sh, &head);
    assert_eq!(cmp.exit, 3, "{cmp:?}");
    let failed: Vec<_> = cmp
        .rows
        .iter()
        .filter(|r| r.3.fails())
        .map(|r| r.0)
        .collect();
    assert_eq!(failed, vec![Dim::R], "only R regressed");
}

#[test]
fn an_unchanged_graph_never_regresses_and_owed_work_is_scored() {
    let cfg = config();
    let b = baseline().seal();
    let s = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let cmp = compare(
        &cfg,
        Some(&s),
        &score::<Adr>(&b, Some(&b), &cfg, &CheckFacts::default()).unwrap(),
        &b,
    );
    assert_eq!(cmp.exit, 0, "{cmp:?}");
    let o = s.get(Dim::O).unwrap();
    assert!(
        o.value.unwrap() < 1.0 && o.contributors[0].id == id("decision:d3").to_string(),
        "{o:?}"
    );
    for d in [Dim::F, Dim::G, Dim::P, Dim::S, Dim::X] {
        assert_eq!(s.get(d).unwrap().mode, Mode::Absent, "{d:?}");
        assert!(s.get(d).unwrap().value.is_none());
    }
}

#[test]
fn a_floor_fails_even_without_a_base() {
    let cfg =
        CheckConfig::parse(&format!("{}\n[ratchet.floor]\nO = 0.99\n", common::CONFIG)).unwrap();
    let b = baseline().seal();
    let s = score::<Adr>(&b, None, &cfg, &CheckFacts::default()).unwrap();
    let cmp = compare(&cfg, None, &s, &b);
    assert_eq!(cmp.exit, 3, "{cmp:?}");
}

#[test]
fn the_config_refuses_a_missing_epsilon_an_unknown_letter_and_an_uncovered_source() {
    let missing = common::CONFIG.replace("O = 0.005\n", "");
    assert!(
        CheckConfig::parse(&missing)
            .unwrap_err()
            .to_string()
            .contains("epsilon.O")
    );
    let unknown = format!("{}Z = 0.1\n", common::CONFIG);
    assert!(CheckConfig::parse(&unknown).is_err());
    let uncovered = common::CONFIG.replace("{ prefix = \"\", value = 0.5 }, ", "");
    let cfg =
        CheckConfig::parse(&uncovered.replace("prefix = \"adr.md\"", "prefix = \"x/\"")).unwrap();
    assert!(score::<Adr>(&baseline().seal(), None, &cfg, &CheckFacts::default()).is_err());
}

#[test]
fn pagerank_is_normalised_to_mean_one_and_favours_the_referenced() {
    let b = baseline().seal();
    let pr = pagerank(&b, 0.85, 30);
    let mean: f64 = pr.iter().sum::<f64>() / pr.len() as f64;
    assert!((mean - 1.0).abs() < 1e-9);
    let i = |s: &str| b.nodes.iter().position(|n| n.id == id(s)).unwrap();
    assert!(pr[i("decision:code")] > pr[i("decision:d3")]);
}

#[test]
fn a_head_that_lost_a_capability_is_excluded_not_failed() {
    let cfg = config();
    let mut f = baseline();
    let r = f.decision("r1", &[("why", "superseded by d1")]);
    f.edge(&r, core_rels::Retracts, &id("decision:d3"));
    let hb = f.seal();
    let base = score::<Adr>(&hb, Some(&hb), &cfg, &CheckFacts::default()).unwrap();
    assert_eq!(base.get(Dim::H).unwrap().mode, Mode::Full);
    let head = score::<Adr>(&hb, None, &cfg, &CheckFacts::default()).unwrap();
    assert_eq!(head.get(Dim::H).unwrap().mode, Mode::Partial);
    let cmp = compare(&cfg, Some(&base), &head, &hb);
    let h = cmp.rows.iter().find(|r| r.0 == Dim::H).unwrap();
    assert!(h.3.name().starts_with("excluded"), "{h:?}");
    assert!(!h.3.fails());
}
