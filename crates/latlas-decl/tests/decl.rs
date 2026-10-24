// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The declarative pack on an in-memory repository: tables under a heading become
//! nodes (a renamed heading fails, never an empty table), test functions become gates, and a
//! crate without a test owes one.

use latlas_check::dims::Dim;
use latlas_check::{CheckConfig, CheckFacts, score};
use latlas_core::id::NodeId;
use latlas_core::{Config, SourceSet, derive};
use latlas_decl::Decl;
use latlas_decl::rust::tests_in;
use latlas_decl::table::under_heading;

const LOG: &str = "# Log\n\n## Tracking Table\n\n| # | Optimization | Status |\n|---|---|---|\n| 2 | **Slice** entry | Current |\n| 1 | `SmallVec` | - |\n\n## Next\n| x | y |\n";

const CFG: &str = r##"
schema = "latlas-project/v1"
[sources]
root = "/repo"
rust = { manifest = "Cargo.toml" }
[[sources.tables]]
id = "opt"
path = "LOG.md"
heading = "Tracking Table"
label = "Evidence"
key = "#"
fields = ["Optimization", "Status"]
outcome = true
[embedder]
mode = "unavailable"
reason = "test"
[limits]
max_rows = 10
cypher_timeout_ms = 10
token_budget = 10
status_tokens = 10
search_deadline_ms = 10
[store]
data_dir = "/tmp/x"
scratch_dir = "/tmp/y"
[search]
w_cosine = 0.6
w_hops = 0.25
w_kind = 0.15
[validator]
python = "/usr/bin/python3"
"##;

const CHECK: &str = "schema = \"latlas-check/v1\"\n[weights]\nchurn = \"off\"\nimportance = [{ prefix = \"\", value = 1.0 }]\npagerank = { damping = 0.85, iterations = 30 }\n[window]\ndays = 30\n[prose]\nweight = 0.5\n[ratchet.epsilon]\nF = 0.0\nR = 0.0\nC = 0.0\nG = 0.0\nP = 0.0\nS = 0.0\nH = 0.0\nT = 0.0\nX = 0.0\nO = 0.0\n";

fn repo(log: &str) -> SourceSet {
    let mut s = SourceSet::default();
    s.add("LOG.md", log.into());
    s.add("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n".into());
    s.add("crates/a/Cargo.toml", "[package]\nname = \"a\"\n".into());
    s.add("crates/a/src/lib.rs", "#[cfg(test)]\nmod t {\n    #[test]\n    fn adds() {}\n}\n".into());
    s.add("crates/b/Cargo.toml", "[package]\nname = \"b\"\n".into());
    s.add("crates/b/src/lib.rs", "pub fn f() {}\n".into());
    s.add("crates/c/Cargo.toml", "[package]\nname = \"c\"\npublish = false\n".into());
    s
}

fn id(s: &str) -> NodeId {
    NodeId::parse::<Decl>(s).unwrap()
}

#[test]
fn the_table_under_the_heading_is_read_and_cells_are_cleaned() {
    let t = under_heading(LOG, "Tracking Table").unwrap();
    assert_eq!(t.header, ["#", "Optimization", "Status"]);
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[0], (7, vec!["2".into(), "Slice entry".into(), "Current".into()]));
    assert!(under_heading(LOG, "Missing").is_none());
    assert!(under_heading("## H\ntext\n## Next\n| a |\n", "H").is_none(), "a table under the next heading is not this one");
}

#[test]
fn tests_are_found_by_attribute_and_name() {
    let t = tests_in("#[test]\nfn a() {}\n#[tokio::test(flavor = \"multi_thread\")]\nasync fn b() {}\nfn not_a_test() {}\n");
    assert_eq!(t, vec![(2, "a".to_string()), (4, "b".to_string())]);
}

#[test]
fn rows_and_crates_derive_and_an_untested_published_crate_owes_a_test() {
    let cfg = Config::parse::<Decl>(CFG).unwrap();
    let b = derive::<Decl>(&cfg, &repo(LOG), None).unwrap();
    let row = b.node(&id("ev:2")).unwrap();
    assert_eq!(row.p["Optimization"], "Slice entry");
    assert_eq!(row.src, "LOG.md:7");
    assert!(b.out(&id("crate:a"), latlas_core::id::core_rels::GatedBy).any(|e| e.to == id("gate:test/a::adds")));
    let s = score::<Decl>(&b, None, &CheckConfig::parse(CHECK).unwrap(), &CheckFacts::default()).unwrap();
    let o = s.get(Dim::O).unwrap();
    let owing: Vec<_> = o.contributors.iter().map(|c| c.id.clone()).collect();
    assert_eq!(owing, vec![id("crate:b").to_string()], "c is unpublished, a is tested");
    assert_eq!(s.get(Dim::C).unwrap().value, Some(0.0), "rows without claims cover nothing");
}

#[test]
fn a_renamed_heading_fails_the_derive() {
    let cfg = Config::parse::<Decl>(CFG).unwrap();
    let e = derive::<Decl>(&cfg, &repo(&LOG.replace("Tracking Table", "Tracking")), None).unwrap_err();
    assert!(e.to_string().contains("no table under the heading"), "{e}");
}
