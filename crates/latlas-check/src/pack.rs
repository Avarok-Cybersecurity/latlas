// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: What a pack declares for the score (check part §1): which nodes bear outcomes
//! (C), how much a claim's gate is worth (C), whether a retraction states its reason (H), and its
//! obligation rules (O). A native pack implements this; a declarative pack's rules compile to the
//! same answers.
//!
//! Owner: latlas-check.
//! Invariants: every hook is pure over the bundle.

use alloc::string::String;
use alloc::vec::Vec;
use latlas_core::id::NodeId;
use latlas_core::{Bundle, Node, Pack};

/// 2026-10-23: One obligation subject: the rule, the subject, and what it lacks (empty: met).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub rule: String,
    pub subject: NodeId,
    pub missing: Vec<String>,
}

/// 2026-10-24: A record's contradiction key (check part §1 X): what it is about (`subject`), which
/// property (`slot`), where it holds (`scope`), and what it says (`value`). Two live records with
/// one key and different values contradict, unless one supersedes or retracts the other or a
/// Decision settles both.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub subject: String,
    pub slot: String,
    pub scope: String,
    pub value: String,
}

/// 2026-10-23: The pack's side of the score.
pub trait Checks: Pack {
    /// Whether `n` bears an outcome that should be backed by a gated claim.
    fn is_outcome(n: &Node) -> bool;
    /// What claim `c` contributes to its outcome's coverage: 1 for an executable gate, the
    /// declared fraction for a review-only (`manual`) one, 0 for none.
    fn claim_credit(b: &Bundle, c: &NodeId) -> f64;
    /// Whether the retracting record `r` states why.
    fn retraction_has_reason(b: &Bundle, r: &NodeId) -> bool;
    /// Every subject the pack's obligation rules match, met or not.
    fn obligations(b: &Bundle) -> Vec<Obligation>;
    /// The contradiction key of a Claim or Decision `n`, or `None` when the pack cannot key it
    /// (counted and listed as `unkeyed`, never scored; never an embedding).
    fn key(b: &Bundle, n: &Node) -> Option<Key>;
}
