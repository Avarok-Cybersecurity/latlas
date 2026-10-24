// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: Typed ids (TIDE) over an open vocabulary: `<label-prefix>:<key>` strings stop at
//! [`NodeId::parse`]; inside, every node is a [`NodeId`] holding its [`Label`] and key. Labels and
//! relations are registered at build time: the core's ([`CORE_LABELS`], [`CORE_RELS`]) followed
//! by one pack's ([`Vocab`]); the position in that order is the LatticeDB label bit / relation id,
//! so each list is append-only (ontology part §1, NDIC: the pack is a type parameter).
//!
//! Owner: latlas-core (cut from metrale-kg-derive `id.rs`, 2026-10-23 K0).
//! Invariants:
//! - A key is non-empty and holds no newline or tab.
//! - `Ord` on [`NodeId`] is the order of its displayed string (prefixes are lowercase letters and
//!   `:` sorts below them, so (prefix, key) order and string order agree).
//! - A pack may not reuse a core label name, prefix or relation name (`vocab_is_valid`).

use crate::prelude::*;
use core::cmp::Ordering;
use core::fmt;
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-23: A node label's definition.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelDef {
    /// The label's name as Cypher and the bundle spell it.
    pub name: &'static str,
    /// The id prefix (`ledger` in `ledger:l2-prefetch-6`).
    pub prefix: &'static str,
    /// Nodes exist because something mentions them (a commit sha, a gate name), not because a
    /// source defines them, so an undefined mention is not dangling.
    pub implicit: bool,
    /// An undefined target is a forward reference (a memory worth writing later), reported apart
    /// from integrity errors.
    pub forward_link: bool,
}

/// 2026-10-23: A node label (a registered [`LabelDef`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label(pub &'static LabelDef);

/// 2026-10-23: A relation's definition.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelDef {
    /// The relation's name as Cypher and the bundle spell it.
    pub name: &'static str,
}

/// 2026-10-23: An edge relation (a registered [`RelDef`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rel(pub &'static RelDef);

impl Label {
    /// 2026-10-23: A label definition as a const (`Label::def("Run", "run", true, false)`).
    pub const fn name(self) -> &'static str {
        self.0.name
    }

    /// 2026-10-23: The id prefix.
    pub const fn prefix(self) -> &'static str {
        self.0.prefix
    }

    /// 2026-10-23: See [`LabelDef::implicit`].
    pub const fn implicit(self) -> bool {
        self.0.implicit
    }

    /// 2026-10-23: See [`LabelDef::forward_link`].
    pub const fn forward_link(self) -> bool {
        self.0.forward_link
    }
}

impl Rel {
    /// 2026-10-23: The relation's name.
    pub const fn name(self) -> &'static str {
        self.0.name
    }
}

/// 2026-10-23: Declare label consts: `labels! { pub Run = ("run"), implicit; ... }`.
#[macro_export]
macro_rules! labels {
    ($( $(#[$m:meta])* $vis:vis $label:ident = $prefix:literal $(, $flag:ident)*; )*) => {
        $(
            $(#[$m])*
            #[allow(non_upper_case_globals)]
            $vis const $label: $crate::id::Label = $crate::id::Label(&$crate::id::LabelDef {
                name: stringify!($label),
                prefix: $prefix,
                implicit: $crate::__flag!(implicit $(, $flag)*),
                forward_link: $crate::__flag!(forward_link $(, $flag)*),
            });
        )*
    };
}

/// 2026-10-23: Whether `$want` is among the flags (for [`labels!`]).
#[doc(hidden)]
#[macro_export]
macro_rules! __flag {
    (implicit) => { false };
    (forward_link) => { false };
    (implicit, implicit $(, $rest:ident)*) => { true };
    (forward_link, forward_link $(, $rest:ident)*) => { true };
    ($want:ident, $other:ident $(, $rest:ident)*) => { $crate::__flag!($want $(, $rest)*) };
}

/// 2026-10-23: Declare relation consts: `rels! { pub DependsOn = "DEPENDS_ON"; ... }`.
#[macro_export]
macro_rules! rels {
    ($( $vis:vis $rel:ident = $name:literal; )*) => {
        $(
            #[allow(non_upper_case_globals)]
            $vis const $rel: $crate::id::Rel = $crate::id::Rel(&$crate::id::RelDef { name: $name });
        )*
    };
}

/// 2026-10-23: The core labels (ontology part §2, the subset cut from metrale-kg).
pub mod core_labels {
    labels! {
        pub Evidence = "ev";
        pub Run = "run";
        pub Claim = "claim";
        pub Gate = "gate", implicit;
        pub Decision = "decision";
        pub Principle = "principle";
        /// 2026-10-23: A feedback memory (a behavioural rule).
        pub Rule = "feedback", forward_link;
        pub Memory = "memory", forward_link;
        pub Tool = "tool", implicit;
        pub Friction = "friction";
        pub Idea = "idea";
        pub Paper = "paper", implicit;
        pub Design = "design", implicit;
        pub Commit = "commit", implicit;
        /// 2026-10-23: A project config file (its root-relative path).
        pub Config = "config";
        /// 2026-10-23: A reference no source defines; key `<label>:<key>` of what was referenced.
        pub Unresolved = "unresolved";
    }
}

/// 2026-10-23: The core relations.
pub mod core_rels {
    rels! {
        pub DependsOn = "DEPENDS_ON";
        pub Retracts = "RETRACTS";
        pub Claims = "CLAIMS";
        pub GatedBy = "GATED_BY";
        pub ImplementedBy = "IMPLEMENTED_BY";
        pub MeasuredIn = "MEASURED_IN";
        pub Cites = "CITES";
        pub Measures = "MEASURES";
        pub EvidenceFor = "EVIDENCE_FOR";
        pub DecidedBy = "DECIDED_BY";
        pub Supersedes = "SUPERSEDES";
        pub Touches = "TOUCHES";
        pub Applies = "APPLIES";
        pub Violates = "VIOLATES";
        pub Proposes = "PROPOSES";
        pub ResolvedBy = "RESOLVED_BY";
        pub Recurs = "RECURS";
        pub Links = "LINKS";
        pub LandsAs = "LANDS_AS";
        pub RestsOn = "RESTS_ON";
        pub StatedIn = "STATED_IN";
    }
}

/// 2026-10-23: The core labels in bit order (append-only).
pub const CORE_LABELS: &[Label] = {
    use core_labels::*;
    &[
        Evidence, Run, Claim, Gate, Decision, Principle, Rule, Memory, Tool, Friction, Idea, Paper,
        Design, Commit, Config, Unresolved,
    ]
};

/// 2026-10-23: The core relations in id order (append-only).
pub const CORE_RELS: &[Rel] = {
    use core_rels::*;
    &[
        DependsOn,
        Retracts,
        Claims,
        GatedBy,
        ImplementedBy,
        MeasuredIn,
        Cites,
        Measures,
        EvidenceFor,
        DecidedBy,
        Supersedes,
        Touches,
        Applies,
        Violates,
        Proposes,
        ResolvedBy,
        Recurs,
        Links,
        LandsAs,
        RestsOn,
        StatedIn,
    ]
};

/// 2026-10-23: One pack's vocabulary, appended to the core's. Chosen once, in the binary.
pub trait Vocab {
    /// The pack's labels, in bit order after [`CORE_LABELS`] (append-only).
    const LABELS: &'static [Label];
    /// The pack's relations, in id order after [`CORE_RELS`] (append-only).
    const RELS: &'static [Rel];
}

/// 2026-10-23: The core alone (no pack), for tools and tests that need no pack vocabulary.
pub struct CoreOnly;

impl Vocab for CoreOnly {
    const LABELS: &'static [Label] = &[];
    const RELS: &'static [Rel] = &[];
}

/// 2026-10-23: Every label of `V`, in bit order.
pub fn labels<V: Vocab>() -> impl Iterator<Item = Label> {
    CORE_LABELS.iter().chain(V::LABELS).copied()
}

/// 2026-10-23: Every relation of `V`, in id order.
pub fn rels<V: Vocab>() -> impl Iterator<Item = Rel> {
    CORE_RELS.iter().chain(V::RELS).copied()
}

/// 2026-10-23: The label of `V` whose id prefix is `p`.
pub fn label_by_prefix<V: Vocab>(p: &str) -> Option<Label> {
    labels::<V>().find(|l| l.prefix() == p)
}

/// 2026-10-23: The label of `V` named `n`.
pub fn label_by_name<V: Vocab>(n: &str) -> Option<Label> {
    labels::<V>().find(|l| l.name() == n)
}

/// 2026-10-23: The relation of `V` named `n`.
pub fn rel_by_name<V: Vocab>(n: &str) -> Option<Rel> {
    rels::<V>().find(|r| r.name() == n)
}

/// 2026-10-23: Every id prefix of `V`, comma-separated (for refusals that must not dead-end).
pub fn prefixes<V: Vocab>() -> String {
    let all: Vec<&str> = labels::<V>().map(Label::prefix).collect();
    all.join(", ")
}

/// 2026-10-23: Whether `V`'s names and prefixes are unique across core and pack (a pack may not
/// redefine a core label or relation).
pub fn vocab_is_valid<V: Vocab>() -> bool {
    let n: BTreeSet<&str> = labels::<V>().map(Label::name).collect();
    let p: BTreeSet<&str> = labels::<V>().map(Label::prefix).collect();
    let r: BTreeSet<&str> = rels::<V>().map(Rel::name).collect();
    let nl = labels::<V>().count();
    n.len() == nl && p.len() == nl && r.len() == rels::<V>().count()
}

/// 2026-10-23: The most labels one LatticeDB collection holds (a `u64` label bitmap).
pub const LABEL_LIMIT: usize = 64;

/// 2026-10-23: A node id: its label and the key after the prefix.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    label: Label,
    key: String,
}

impl NodeId {
    /// 2026-10-23: The id of `label` with `key`; refuses an empty key, a tab or a newline.
    pub fn new(label: Label, key: impl Into<String>) -> Result<NodeId> {
        let key = key.into();
        if key.is_empty() || key.contains(['\n', '\t', '\r']) {
            return Err(error!(
                ErrorCode::LatlasBadId,
                format!("{}:{key}", label.prefix()),
                "the key is empty or holds a tab or newline"
            ));
        }
        Ok(NodeId { label, key })
    }

    /// 2026-10-23: Parse `<prefix>:<key>` against `V`'s labels.
    pub fn parse<V: Vocab>(s: &str) -> Result<NodeId> {
        let Some((p, k)) = s.split_once(':') else {
            return Err(error!(
                ErrorCode::LatlasBadId,
                s.to_string(),
                format!("no `<prefix>:` part; prefixes: {}", prefixes::<V>())
            ));
        };
        let Some(label) = label_by_prefix::<V>(p) else {
            return Err(error!(
                ErrorCode::LatlasBadId,
                s.to_string(),
                format!("unknown label prefix `{p}`; prefixes: {}", prefixes::<V>())
            ));
        };
        NodeId::new(label, k)
    }

    /// 2026-10-23: The `Unresolved` node standing for an undefined `target`.
    pub fn unresolved(target: &NodeId) -> NodeId {
        NodeId {
            label: core_labels::Unresolved,
            key: target.to_string(),
        }
    }

    /// 2026-10-23: The node's label.
    pub fn label(&self) -> Label {
        self.label
    }

    /// 2026-10-23: The key after the prefix.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.label.prefix(), self.key)
    }
}

impl Ord for NodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.label.prefix(), self.key()).cmp(&(other.label.prefix(), other.key()))
    }
}

impl PartialOrd for NodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_their_wire_form() {
        for s in ["run:r1", "memory:x @ y", "unresolved:ledger:nope"] {
            assert_eq!(NodeId::parse::<CoreOnly>(s).unwrap().to_string(), s);
        }
        assert_eq!(
            NodeId::parse::<CoreOnly>("feedback:x").unwrap().label(),
            core_labels::Rule
        );
    }

    #[test]
    fn malformed_ids_are_refused_with_the_bad_id_code() {
        for s in ["nolabel", "bogus:x", "run:", "run:a\tb"] {
            assert_eq!(
                NodeId::parse::<CoreOnly>(s).unwrap_err().code(),
                ErrorCode::LatlasBadId,
                "{s}"
            );
        }
    }

    #[test]
    fn id_order_is_string_order() {
        let mut ids: Vec<NodeId> = ["memory:a", "run:z", "idea:b", "run:a"]
            .iter()
            .map(|s| NodeId::parse::<CoreOnly>(s).unwrap())
            .collect();
        ids.sort();
        let got: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let mut want = got.clone();
        want.sort();
        assert_eq!(got, want);
    }

    struct Clash;
    impl Vocab for Clash {
        const LABELS: &'static [Label] = &[core_labels::Run];
        const RELS: &'static [Rel] = &[];
    }

    #[test]
    fn the_core_fits_one_collection_and_a_pack_cannot_reuse_a_core_name() {
        assert!(CORE_LABELS.len() <= LABEL_LIMIT);
        assert!(vocab_is_valid::<CoreOnly>());
        assert!(!vocab_is_valid::<Clash>());
        assert!(core_labels::Gate.implicit() && !core_labels::Gate.forward_link());
        assert!(core_labels::Memory.forward_link() && !core_labels::Run.implicit());
    }
}
