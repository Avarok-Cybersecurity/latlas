// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The config: the pack's `[sources]` table plus the core tables (embedder, limits,
//! store, search, validator). Every field is required and unknown fields are refused (PCND): a
//! missing field fails at start with `LatlasConfig` and the field's name. The source list is
//! explicit; nothing is discovered except the files of declared source directories.
//!
//! Owner: latlas-core (cut from metrale-kg-derive, 2026-10-23 K0).
//! Invariants:
//! - Source paths are relative to the pack's root; none names a credential file
//!   ([`crate::secrets::name_is_secret`]).
//! - The embedder is either `http` with every field, or the declared `unavailable` mode with its
//!   reason. There is no third state.

use crate::pack::Pack;
use crate::prelude::*;
use crate::secrets::name_is_secret;
use crate::settings::EmbedLayer;
use latlas_error::{ErrorCode, Result, error};
use serde::Deserialize;

/// 2026-10-23: The whole config.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, bound = "S: serde::de::DeserializeOwned")]
pub struct Config<S> {
    pub schema: String,
    /// 2026-10-23: The pack's `[sources]` table ([`Pack::Sources`]).
    pub sources: S,
    /// 2026-10-23: The project layer of the embedder settings ([`crate::settings`]); it
    /// supersedes the global layer per key. An absent table sets no key (layering, not a default).
    #[serde(default)]
    pub embedder: EmbedLayer,
    pub limits: Limits,
    pub store: Store,
    pub search: Search,
    pub validator: Validator,
    /// 2026-10-25: `[repo_track]`: follow a remote ref from the git object store
    /// ([`crate::track`]). Absent means tracking is off (declared and reported by `la_status`).
    #[serde(default)]
    pub repo_track: Option<crate::track::RepoTrack>,
    /// 2026-10-23: The root-relative path this config was read from, set by the host that read
    /// it (never by the file), so the config's own node has a `src`.
    #[serde(skip)]
    pub origin: String,
}

/// 2026-10-23: How a file's own validator runs (the interpreter is named, never looked up).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Validator {
    /// 2026-10-23: The absolute path of the Python interpreter `ledger.py` runs under.
    pub python: String,
}

/// 2026-10-23: Tool result limits.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_rows: u32,
    pub cypher_timeout_ms: u64,
    pub token_budget: u32,
    /// 2026-10-23: The token budget of a default `kg_status` (no sections named); `check` holds
    /// the live answer to it.
    pub status_tokens: u32,
    /// 2026-10-23: The longest a search waits for its query's embedding before it answers
    /// `graph_only` with the reason (a search never waits for the cache warmer at all).
    pub search_deadline_ms: u64,
}

/// 2026-10-23: Where the derived database lives (rebuildable, not committed).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub data_dir: String,
    /// 2026-10-23: An absolute directory for private copies (validator runs, atomic writes).
    pub scratch_dir: String,
}

/// 2026-10-23: The hybrid ranking weights (ontology §5), declared and printed in results.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub w_cosine: f64,
    pub w_hops: f64,
    pub w_kind: f64,
}

impl<S> Config<S> {
    /// 2026-10-23: Parse and validate a config text for pack `P`.
    pub fn parse<P: Pack<Sources = S>>(text: &str) -> Result<Config<S>>
    where
        S: serde::de::DeserializeOwned,
    {
        let cfg: Config<S> = toml::from_str(text)
            .map_err(|e| error!(ErrorCode::LatlasConfig, "(file)", e.to_string()))?;
        cfg.validate::<P>()?;
        Ok(cfg)
    }

    fn validate<P: Pack<Sources = S>>(&self) -> Result<()> {
        if self.schema != P::CONFIG_SCHEMA {
            return Err(error!(
                ErrorCode::LatlasConfig,
                "schema",
                format!("must be {}", P::CONFIG_SCHEMA)
            ));
        }
        if !P::root(&self.sources).starts_with('/') {
            return Err(error!(
                ErrorCode::LatlasConfig,
                "sources.root", "must be absolute"
            ));
        }
        let dirs = P::source_dirs(&self.sources);
        for p in self.source_paths::<P>().into_iter().chain(dirs) {
            if p.starts_with('/') || p.split('/').any(|c| c == "..") {
                return Err(error!(
                    ErrorCode::LatlasConfig,
                    p, "must be root-relative, without .."
                ));
            }
            if name_is_secret(&p) {
                return Err(error!(
                    ErrorCode::LatlasConfig,
                    p, "names a credential file"
                ));
            }
        }
        P::validate(&self.sources)?;
        if let Some(t) = &self.repo_track {
            t.validate()?;
        }
        if self.limits.search_deadline_ms == 0 {
            return Err(error!(
                ErrorCode::LatlasConfig,
                "limits.search_deadline_ms", "must be positive"
            ));
        }
        let w = self.search;
        if !(w.w_cosine >= 0.0 && w.w_hops >= 0.0 && w.w_kind >= 0.0) {
            return Err(error!(
                ErrorCode::LatlasConfig,
                "search", "weights must be >= 0"
            ));
        }
        Ok(())
    }

    /// 2026-10-23: Every single-file source path (root-relative), sorted, with this config's own.
    pub fn source_paths<P: Pack<Sources = S>>(&self) -> Vec<String> {
        let mut out = P::source_paths(&self.sources);
        if !self.origin.is_empty() {
            out.push(self.origin.clone());
        }
        out.sort();
        out.dedup();
        out
    }

    /// 2026-10-23: Refuse a scope the pack does not declare, naming the known ones.
    pub fn scope<P: Pack<Sources = S>>(&self, id: &str) -> Result<()> {
        let known = P::scopes(&self.sources);
        if known.iter().any(|s| s == id) {
            return Ok(());
        }
        Err(error!(
            ErrorCode::LatlasToolArgument,
            "scope",
            format!("no scope `{id}`; known: {}", known.join(", "))
        ))
    }
}

/// 2026-10-24: Resolve a repository config's relative paths against `repo` (the directory holding
/// `.latlas/`; native: its absolute path, browser: the virtual root): `sources.root = "."` becomes
/// `repo`, and a relative `store.data_dir` / `store.scratch_dir` is joined to it. A config whose
/// root is not `"."` is returned unchanged (the core refuses any other relative root). Pure, so
/// every host resolves the same way (HPAR).
pub fn resolve_repo_paths(text: &str, repo: &str) -> Result<Option<String>> {
    let mut t: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| error!(ErrorCode::LatlasConfig, "(file)", e.to_string()))?;
    let dot = t
        .get("sources")
        .and_then(|s| s.get("root"))
        .and_then(toml::Value::as_str)
        == Some(".");
    if !dot {
        return Ok(None);
    }
    let join = |rel: &str| format!("{}/{}", repo.trim_end_matches('/'), rel.trim_start_matches("./"));
    if let Some(toml::Value::Table(s)) = t.get_mut("sources") {
        s.insert("root".into(), toml::Value::String(repo.to_string()));
    }
    if let Some(toml::Value::Table(s)) = t.get_mut("store") {
        for k in ["data_dir", "scratch_dir"] {
            if let Some(toml::Value::String(v)) = s.get_mut(k)
                && !v.starts_with('/')
            {
                *v = join(v);
            }
        }
    }
    toml::to_string(&t)
        .map(Some)
        .map_err(|e| error!(ErrorCode::LatlasConfig, "(file)", e.to_string()))
}
