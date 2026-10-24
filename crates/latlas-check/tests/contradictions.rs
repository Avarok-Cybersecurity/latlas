// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: X is keyed: one key, two values, both live, nothing between them ⇒ a contradiction;
//! a supersession, a retraction or a settling Decision clears it; equal values never contradict.

mod common;

use common::{Adr, Fx, config, id};
use latlas_check::dims::{Dim, Mode};
use latlas_check::{CheckFacts, score};
use latlas_core::id::core_rels;
use serde_json::Value;

fn pair(b_value: &str) -> Fx {
    let mut f = Fx::new();
    f.decision("ef-a", &[("subject", "hnsw"), ("slot", "ef"), ("value", "64")]);
    f.decision("ef-b", &[("subject", "hnsw"), ("slot", "ef"), ("value", b_value)]);
    f.decision("loose", &[]);
    f
}

fn x(f: Fx) -> latlas_check::DimResult {
    score::<Adr>(&f.seal(), None, &config(), &CheckFacts::default())
        .unwrap()
        .get(Dim::X)
        .unwrap()
        .clone()
}

#[test]
fn two_live_values_for_one_key_contradict_and_unkeyed_records_are_not_scored() {
    let r = x(pair("128"));
    assert_eq!(r.mode, Mode::Full);
    assert_eq!(r.value, Some(0.0), "{r:?}");
    let ids: Vec<&str> = r.contributors.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"decision:ef-a") && ids.contains(&"decision:ef-b"), "{ids:?}");
    assert!(r.reason.unwrap().contains("1 unkeyed"));
    assert_eq!(x(pair("64")).value, Some(1.0), "equal values agree");
}

#[test]
fn a_supersession_or_a_settlement_clears_the_contradiction() {
    let mut f = pair("128");
    f.edge(&id("decision:ef-b"), core_rels::Supersedes, &id("decision:ef-a"));
    let r = x(f);
    assert_eq!(r.value, Some(1.0), "the superseded one is not live: {r:?}");

    let mut f = pair("128");
    let s = id("decision:settle");
    let mut p = std::collections::BTreeMap::new();
    p.insert(
        "settles".to_string(),
        Value::from(vec!["decision:ef-a", "decision:ef-b"]),
    );
    f.b.define(s, "adr.md:50".into(), "settle", p);
    assert_eq!(x(f).value, Some(1.0), "a settling decision clears it");
}

#[test]
fn no_keyed_record_is_absent_not_one() {
    let mut f = Fx::new();
    f.decision("loose", &[]);
    let r = x(f);
    assert_eq!(r.mode, Mode::Absent);
    assert!(r.value.is_none());
}
