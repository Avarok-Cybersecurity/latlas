// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: A core-vocabulary test pack with declared check rules, and an in-memory ADR
//! fixture built through the production `Builder` (no files, no mocks).

#![allow(dead_code)]

use latlas_check::{CheckConfig, Checks, Obligation};
use latlas_core::bundle::{Builder, Bundle, SourceDigest};
use latlas_core::hash::sha256;
use latlas_core::id::{Label, NodeId, Rel, Vocab, core_labels, core_rels};
use latlas_core::pack::{Pack, SectionCtx};
use latlas_core::{Config, Node, SourceSet};
use latlas_error::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct Adr;

impl Vocab for Adr {
    const LABELS: &'static [Label] = &[];
    const RELS: &'static [Rel] = &[];
}

impl Pack for Adr {
    const ID: &'static str = "adr";
    const VERSION: &'static str = "0.0.0";
    const CONFIG_SCHEMA: &'static str = "adr/v1";
    const NAMED_LABELS: &'static [Label] = &[];
    const SEARCH_EXPAND: &'static [Rel] = &[];
    const IMPACT_FOLLOW_OUT: &'static [Rel] = &[];
    const SECTIONS: &'static [&'static str] = &[];
    const DEFAULT_SECTIONS: &'static [&'static str] = &[];
    const RECORD_KINDS: &'static [&'static str] = &[];
    const INSTRUCTIONS: &'static str = "";
    const STATUS_DESCRIPTION: &'static str = "";
    const IMPACT_DESCRIPTION: &'static str = "";
    type Sources = BTreeMap<String, String>;
    type Facts = ();
    fn root(_: &Self::Sources) -> &str {
        "/"
    }
    fn validate(_: &Self::Sources) -> Result<()> {
        Ok(())
    }
    fn source_paths(_: &Self::Sources) -> Vec<String> {
        Vec::new()
    }
    fn source_dirs(_: &Self::Sources) -> Vec<String> {
        Vec::new()
    }
    fn scopes(_: &Self::Sources) -> Vec<String> {
        Vec::new()
    }
    fn default_instance(_: &Self::Sources, _: &str) -> Option<String> {
        None
    }
    fn derive(
        _: &Config<Self::Sources>,
        _: &SourceSet,
        _: Option<&()>,
        _: &mut Builder,
    ) -> Result<()> {
        Ok(())
    }
    fn sealed(_: &mut Bundle, _: Option<&()>) {}
    fn section(_: &SectionCtx<'_, Self>, _: &str) -> Result<Value> {
        Ok(Value::Null)
    }
    fn impact_rels(_: Label, _: bool) -> Option<Vec<Rel>> {
        None
    }
    fn impact_extra(
        _: &Bundle,
        _: &Config<Self::Sources>,
        _: &NodeId,
        _: &mut Map<String, Value>,
    ) -> Result<()> {
        Ok(())
    }
}

/// 2026-10-23: The test pack's rules: every Decision is an outcome; a `manual` claim earns 0.5;
/// a retraction needs a `why`; every Decision must be IMPLEMENTED_BY something.
impl Checks for Adr {
    fn is_outcome(n: &Node) -> bool {
        n.id.label() == core_labels::Decision
    }
    fn claim_credit(b: &Bundle, c: &NodeId) -> f64 {
        match b.field(c, "kind") {
            Some("manual") => 0.5,
            Some("none") => 0.0,
            _ => 1.0,
        }
    }
    fn retraction_has_reason(b: &Bundle, r: &NodeId) -> bool {
        b.field(r, "why").is_some_and(|w| !w.is_empty())
    }
    fn obligations(b: &Bundle) -> Vec<Obligation> {
        b.of(core_labels::Decision)
            .map(|d| Obligation {
                rule: "implemented".into(),
                subject: d.id.clone(),
                missing: if b.out(&d.id, core_rels::ImplementedBy).next().is_some() {
                    Vec::new()
                } else {
                    vec!["IMPLEMENTED_BY".into()]
                },
            })
            .collect()
    }
}

pub fn id(s: &str) -> NodeId {
    NodeId::parse::<Adr>(s).unwrap()
}

/// 2026-10-23: The ADR fixture, edited by `f` before sealing.
pub struct Fx {
    pub b: Builder,
    line: u32,
}

impl Fx {
    pub fn new() -> Fx {
        Fx {
            b: Builder::default(),
            line: 0,
        }
    }
    pub fn decision(&mut self, key: &str, fields: &[(&str, &str)]) -> NodeId {
        self.line += 1;
        let n = id(&format!("decision:{key}"));
        let p: BTreeMap<String, Value> = fields
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect();
        self.b
            .define(n.clone(), format!("adr.md:{}", self.line), key, p);
        n
    }
    pub fn claim(&mut self, of: &NodeId, kind: &str, gate: &str) {
        let c = id(&format!("claim:{}/1", of.key()));
        let mut p = BTreeMap::new();
        p.insert("kind".to_string(), Value::String(kind.into()));
        self.b
            .define(c.clone(), "adr.md:90".into(), &format!("{of}{kind}"), p);
        self.b.edge(of, core_rels::Claims, &c, 1.0, "adr.md:90");
        let g = id(&format!("gate:{gate}"));
        self.b.mention(&g, "adr.md:90");
        self.b.edge(&c, core_rels::GatedBy, &g, 1.0, "adr.md:90");
    }
    pub fn edge(&mut self, a: &NodeId, rel: Rel, b: &NodeId) {
        self.b.edge(a, rel, b, 1.0, "adr.md:99");
    }
    pub fn seal(self) -> Bundle {
        self.b.seal(vec![SourceDigest {
            path: "adr.md".into(),
            sha: sha256(b"adr"),
            git: None,
        }])
    }
}

/// 2026-10-23: The baseline fixture: d1 gated by a test, d2 by a manual review, d3 unclaimed;
/// all implemented except d3.
pub fn baseline() -> Fx {
    let mut f = Fx::new();
    let d1 = f.decision("d1", &[]);
    let d2 = f.decision("d2", &[]);
    let d3 = f.decision("d3", &[]);
    let code = f.decision("code", &[]);
    f.claim(&d1, "test", "test/a");
    f.claim(&d2, "manual", "manual/review");
    for d in [&d1, &d2, &code] {
        f.edge(d, core_rels::ImplementedBy, &code);
    }
    let _ = d3;
    f
}

pub const CONFIG: &str = r#"
schema = "latlas-check/v1"
[weights]
churn = "off"
importance = [{ prefix = "", value = 0.5 }, { prefix = "adr.md", value = 1.0 }]
pagerank = { damping = 0.85, iterations = 30 }
[ratchet.epsilon]
F = 0.005
R = 0.0
C = 0.005
G = 0.005
P = 0.005
S = 0.005
H = 0.0
T = 0.005
X = 0.005
O = 0.005
"#;

pub fn config() -> CheckConfig {
    CheckConfig::parse(CONFIG).unwrap()
}
