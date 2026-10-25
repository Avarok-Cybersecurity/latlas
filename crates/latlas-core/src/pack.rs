// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The pack contract (DESIGN-latlas-ontology.md §4.1): a domain's vocabulary, its
//! `[sources]` table, its adapters and its status sections, chosen once in the binary as a type
//! parameter (NDIC). The core derives, stores, ranks and serves; the pack says what the files
//! mean.
//!
//! Owner: latlas-core.
//! Invariants:
//! - A pack never redefines a core label or relation ([`crate::id::vocab_is_valid`]).
//! - Every hook is pure (SBIO): host-only knowledge arrives as [`Pack::Facts`].

use crate::bundle::{Builder, Bundle};
use crate::config::Config;
use crate::id::{Label, NodeId, Rel, Vocab};
use crate::prelude::*;
use crate::record_schema::KindSchema;
use crate::sources::SourceSet;
use crate::status::Freshness;
use latlas_error::Result;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// 2026-10-23: What a status section is computed from.
pub struct SectionCtx<'a, P: Pack> {
    pub bundle: &'a Bundle,
    pub cfg: &'a Config<P::Sources>,
    pub scope: &'a str,
    pub fresh: &'a Freshness,
    /// The instance a section about one deployment reports (`None`: the call named none and the
    /// scope declares none).
    pub instance: Option<&'a str>,
}

/// 2026-10-23: A domain pack.
pub trait Pack: Vocab + Sized {
    /// The pack id (`metrale`).
    const ID: &'static str;
    /// Semver; bumped with any label or relation change.
    const VERSION: &'static str;
    /// The config schema id the pack's config file carries.
    const CONFIG_SCHEMA: &'static str;
    /// Labels whose bare key a search text may name (`l2-prefetch-6` for `ledger:l2-prefetch-6`).
    const NAMED_LABELS: &'static [Label];
    /// Relations a search hit expands over to its neighbours.
    const SEARCH_EXPAND: &'static [Rel];
    /// Out-edges a downstream impact also follows from each reached node (a row's pinned profile,
    /// its runs, the decisions that cite it).
    const IMPACT_FOLLOW_OUT: &'static [Rel];
    /// The status sections the pack answers, in schema order.
    const SECTIONS: &'static [&'static str];
    /// The pack sections of a default status (none named).
    const DEFAULT_SECTIONS: &'static [&'static str];
    /// The record kinds `la_record` accepts, each with its columns, fill rules and grammar
    /// (served as the `record_schema` status section).
    const RECORD_SCHEMAS: &'static [KindSchema];
    /// The server instructions sent at `initialize`.
    const INSTRUCTIONS: &'static str;
    /// One-line description of the `la_status` tool.
    const STATUS_DESCRIPTION: &'static str;
    /// One-line description of the `la_impact` tool.
    const IMPACT_DESCRIPTION: &'static str;

    /// The pack's `[sources]` table.
    type Sources: DeserializeOwned + Clone + core::fmt::Debug + PartialEq + Send + Sync;
    /// Facts only a host can compute (subprocess output, git state), handed to [`Pack::derive`].
    type Facts;

    /// The absolute directory every source path is relative to.
    fn root(s: &Self::Sources) -> &str;
    /// Validate the table beyond its types (duplicate ids, unsafe paths are the core's).
    fn validate(s: &Self::Sources) -> Result<()>;
    /// Every single-file source path (root-relative).
    fn source_paths(s: &Self::Sources) -> Vec<String>;
    /// Every source directory whose files are all read (root-relative).
    fn source_dirs(s: &Self::Sources) -> Vec<String>;
    /// The scope ids tools name (`glm/megakernel`).
    fn scopes(s: &Self::Sources) -> Vec<String>;
    /// The instance a scope reports when the call names none (declared, never guessed).
    fn default_instance(s: &Self::Sources, scope: &str) -> Option<String>;
    /// Run the pack's adapters over the read sources.
    fn derive(
        cfg: &Config<Self::Sources>,
        src: &SourceSet,
        facts: Option<&Self::Facts>,
        b: &mut Builder,
    ) -> Result<()>;
    /// Annotate the sealed bundle with host facts (stale measurements); no new records.
    fn sealed(bundle: &mut Bundle, facts: Option<&Self::Facts>);
    /// One of [`Pack::SECTIONS`].
    fn section(ctx: &SectionCtx<'_, Self>, name: &str) -> Result<Value>;
    /// The relations `la_impact` follows from a root of `label` when the call names none
    /// (`None`: every relation).
    fn impact_rels(label: Label, downstream: bool) -> Option<Vec<Rel>>;
    /// What `la_impact` adds for `root` beyond the closure (a ledger row's `ledger.py` views).
    fn impact_extra(
        b: &Bundle,
        cfg: &Config<Self::Sources>,
        root: &NodeId,
        out: &mut Map<String, Value>,
    ) -> Result<()>;
}
