// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The running graph: config, the chosen hosts, the current bundle and the store, with
//! the per-call freshness check: stat every source; on any change re-read, re-derive and reload; if
//! that fails, keep the last bundle and say `stale` with the reason.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants:
//! - Every tool result carries `bundle_digest`, `derived_at_ms`, `stale` and `mode`.
//! - The store only ever holds the current bundle (`Store::load` replaces).

use crate::embedhost::EmbedHost;
use crate::host::PackHost;
use crate::pause::PauseCtl;
use crate::track::TrackCtl;
use crate::search::Embed;
use anyhow::Result;
use latlas_core::embed::{Embedder as _, Mode, Plan};
use latlas_core::jsonl::digest;
use latlas_core::metlocal::MET_ATTEMPTS;
use latlas_core::settings::Embedder;
use latlas_core::status::Freshness;
use latlas_core::{Bundle, Config, Pack, SourceSet, derive};
use latlas_host_met::MetLocal;
use latlas_host_native::clock::SystemClock;
use latlas_host_native::fs::{FsSources, Stamp, read_config};
use latlas_host_native::openrouter::OpenRouter;
use latlas_host_native::pause::LockFile;
use latlas_host_native::settings::{Resolved, resolve_settings};
use latlas_index::store::{Store, data_dir};
use latlas_index::textstore::TextStore;
use latlas_ports::Clock;
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 2026-10-23: The application state behind the CLI and the MCP server.
pub struct App<P: Pack, H: PackHost<P>> {
    pub cfg: Config<P::Sources>,
    pub fs: FsSources,
    pub host: H,
    pub bundle: Bundle,
    pub digest: String,
    /// 2026-10-23: The file stamps (what `newest_change` reports) and the host's other stamps.
    pub stamp: (Stamp, Stamp),
    pub fresh: Freshness,
    pub store: Arc<Mutex<Store>>,
    /// 2026-10-23: The embedder settings, resolved once at open (ROTI).
    pub settings: Resolved,
    /// 2026-10-23: The embedder host the settings chose, with its cache (or the declared
    /// unavailable mode and why). The mode every result reports comes from here.
    pub embed: Embed,
    /// 2026-10-25: The pause gate over its signal, shared with the server's poll loop.
    pub pause: Arc<PauseCtl>,
    /// 2026-10-25: `[repo_track]`'s tracker (`None`: tracking is off).
    pub track: Option<TrackCtl>,
    clock: SystemClock,
}

/// 2026-10-23: The most recently modified source file in a stamp and its mtime (unix ms).
fn newest(stamp: &Stamp) -> Option<(String, i64)> {
    stamp.iter().max_by_key(|(_, _, ns)| *ns).map(|(p, _, ns)| {
        (
            p.clone(),
            i64::try_from(*ns / 1_000_000).unwrap_or(i64::MAX),
        )
    })
}

/// 2026-10-23: The embedder host for the resolved settings: off with the reason when the settings
/// declare it unavailable or the host cannot run (no key; 2026-10-24: or met-local's preflight
/// refused), else the host and its cache under `dir`.
async fn open_embed(e: &Embedder, dir: &Path) -> Result<Embed> {
    let (host, space, dim, plan) = match e {
        Embedder::Unavailable { reason } => return Ok(Embed::Off(reason.clone())),
        Embedder::Available(s) => (
            EmbedHost::Http(OpenRouter::new(s, dir.join("embed-calls.jsonl"))),
            s.space(),
            s.dimensions,
            Plan {
                batch: usize::try_from(s.batch)?,
                max_attempts: s.max_attempts,
                backoff_ms: s.backoff_ms,
                max_input_chars: usize::try_from(s.max_input_chars)?,
            },
        ),
        Embedder::MetLocal(s) => (
            EmbedHost::MetLocal(MetLocal::open(s, dir.join("embed-met.stderr.log")).await),
            s.space(),
            s.dimensions,
            Plan {
                batch: usize::try_from(s.max_batch)?,
                max_attempts: MET_ATTEMPTS,
                // 2026-10-24: Never waited on: met-local never answers `Reply::Retryable`.
                backoff_ms: 0,
                max_input_chars: usize::try_from(s.max_input_chars)?,
            },
        ),
    };
    if let Mode::Unavailable { reason } = host.mode() {
        return Ok(Embed::Off(reason));
    }
    let cache = TextStore::open(dir, &space, dim).await?;
    Ok(Embed::On {
        host,
        cache: Arc::new(Mutex::new(cache)),
        plan,
        warm: Default::default(),
    })
}

/// 2026-10-23: Read and derive once (no store); also the source commit the host reports.
pub fn derive_once<P: Pack, H: PackHost<P>>(
    cfg: &Config<P::Sources>,
    fs: &FsSources,
    host: &H,
) -> Result<(SourceSet, Bundle, Option<String>)> {
    let extra = host.extra_files(cfg, fs)?;
    let src = fs.read::<P>(cfg, &extra)?;
    let (facts, commit) = host.facts(cfg, fs)?;
    let b = derive::<P>(cfg, &src, facts.as_ref())?;
    Ok((src, b, commit))
}

/// 2026-10-23: The config, the file host and the pack host, as every command opens them.
pub fn open_hosts<P: Pack, H: PackHost<P>>(
    cfg_path: &Path,
) -> Result<(Config<P::Sources>, FsSources, H)> {
    let cfg = read_config::<P>(cfg_path)?;
    let fs = FsSources::new::<P>(&cfg);
    let host = H::open(&cfg, &fs)?;
    Ok((cfg, fs, host))
}

impl<P: Pack, H: PackHost<P>> App<P, H> {
    /// 2026-10-24: The Clock port's wall time (what `la_record`'s time fills read).
    pub fn now(&self) -> latlas_ports::UnixMs {
        self.clock.now_unix_ms()
    }

    fn stamps(cfg: &Config<P::Sources>, fs: &FsSources, host: &H) -> Result<(Stamp, Stamp)> {
        let extra = host.extra_files(cfg, fs)?;
        Ok((fs.stamp::<P>(cfg, &extra)?, host.extra_stamp(cfg, fs)?))
    }

    /// 2026-10-23: Load the config, derive, open the store at `data_dir` (or the config's) and load;
    /// (2026-10-25) the pause signal is `latlas.lock` beside the running executable.
    pub async fn open(cfg_path: &Path, data_override: Option<&Path>) -> Result<Self> {
        Self::open_with(cfg_path, data_override, LockFile::beside_current_exe()?).await
    }

    /// 2026-10-25: [`open`](Self::open) with the pause signal `lock`, read once before it returns.
    pub async fn open_with(
        cfg_path: &Path,
        data_override: Option<&Path>,
        lock: LockFile,
    ) -> Result<Self> {
        let (cfg, fs, host) = open_hosts::<P, H>(cfg_path)?;
        let settings = resolve_settings(&cfg.embedder)?;
        let stamp = Self::stamps(&cfg, &fs, &host)?;
        let (_, bundle, source_commit) = derive_once::<P, H>(&cfg, &fs, &host)?;
        let dir = match data_override {
            Some(d) => d.to_path_buf(),
            None => data_dir(&cfg.store.data_dir)?,
        };
        let mut store = Store::open::<P>(&dir).await?;
        store.load(&bundle).await?;
        let embed = open_embed(&settings.embedder, &dir).await?;
        let clock = SystemClock;
        let pause = Arc::new(PauseCtl::new(lock));
        pause.poll(&clock, embed.host()).await;
        let track = cfg
            .repo_track
            .clone()
            .map(|t| TrackCtl::new(t.clone(), fs.abs(&t.repo)));
        let fresh = Freshness {
            stale_reason: None,
            derived_at_ms: clock.now_unix_ms().0,
            source_commit,
            newest_change: newest(&stamp.0),
        };
        Ok(App {
            digest: digest(&bundle).hex(),
            cfg,
            fs,
            host,
            bundle,
            stamp,
            fresh,
            store: Arc::new(Mutex::new(store)),
            settings,
            embed,
            pause,
            track,
            clock,
        })
    }

    /// 2026-10-23: The per-call freshness check; returns whether the bundle changed.
    pub async fn refresh(&mut self) -> bool {
        let stamp = match Self::stamps(&self.cfg, &self.fs, &self.host) {
            Ok(s) => s,
            Err(e) => {
                self.fresh.stale_reason = Some(format!("{e:#}"));
                return false;
            }
        };
        if stamp == self.stamp && self.fresh.stale_reason.is_none() {
            return false;
        }
        match derive_once::<P, H>(&self.cfg, &self.fs, &self.host) {
            Ok((_, b, source_commit)) => {
                let changed = digest(&b).hex() != self.digest;
                if let Err(e) = self.store.lock().await.load(&b).await {
                    self.fresh.stale_reason = Some(format!("LatlasStore: {e}"));
                    return false;
                }
                self.digest = digest(&b).hex();
                self.bundle = b;
                self.fresh = Freshness {
                    stale_reason: None,
                    derived_at_ms: self.clock.now_unix_ms().0,
                    source_commit,
                    newest_change: newest(&stamp.0),
                };
                self.stamp = stamp;
                changed
            }
            Err(e) => {
                self.fresh.stale_reason = Some(format!("LatlasStaleBundle: {e:#}"));
                false
            }
        }
    }

    /// 2026-10-23: Declare the embedder off for this process (`--graph-only`): the same declared
    /// mode as an unavailable embedder, with `why` as the reason.
    pub fn declare_graph_only(&mut self, why: &str) {
        self.embed = Embed::Off(why.to_string());
    }

    /// 2026-10-23: The embedder's mode as the host reports it, as every result reports it.
    pub fn mode(&self) -> (String, Option<String>) {
        match &self.embed {
            Embed::Off(reason) => ("graph_only".into(), Some(reason.clone())),
            Embed::On { .. } => ("hybrid".into(), None),
        }
    }

    /// 2026-10-25: The freshness every result reports: a failed tracking tick is the stale
    /// reason (the last good commit is still served) and the tracked commit is the source commit.
    pub fn freshness(&self) -> Freshness {
        let mut f = self.fresh.clone();
        if let Some(t) = &self.track {
            if f.stale_reason.is_none() {
                f.stale_reason = t.tracker.error().map(str::to_string);
            }
            if let Some(c) = t.tracker.commit() {
                f.source_commit = Some(c.0.clone());
            }
        }
        f
    }

    /// 2026-10-25: `[repo_track]`'s `la_status` view (or that it is off).
    pub fn track_status(&self) -> Value {
        self.track
            .as_ref()
            .map_or_else(latlas_core::track::status_off, |t| t.tracker.status())
    }

    /// 2026-10-23: Wrap a tool result in the freshness envelope.
    pub fn envelope(&self, body: Value) -> Value {
        let mut o = match body {
            Value::Object(o) => o,
            other => {
                let mut m = Map::new();
                m.insert("result".into(), other);
                m
            }
        };
        let fresh = self.freshness();
        o.insert("bundle_digest".into(), Value::String(self.digest.clone()));
        o.insert("derived_at_ms".into(), Value::from(fresh.derived_at_ms));
        o.insert("stale".into(), Value::Bool(fresh.stale_reason.is_some()));
        if let Some(c) = &fresh.source_commit {
            o.insert("source_commit".into(), Value::String(c.clone()));
        }
        if let Some(r) = &fresh.stale_reason {
            o.insert("stale_reason".into(), Value::String(r.clone()));
        }
        if !o.contains_key("mode") {
            let (mode, reason) = self.mode();
            o.insert("mode".into(), Value::String(mode));
            if let Some(r) = reason {
                o.insert("mode_reason".into(), Value::String(r));
            }
        }
        Value::Object(o)
    }
}
