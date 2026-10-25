// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: Repo tracking as the process runs it: the tracker (`latlas_core::track`) over the
//! native git host, its snapshot installed into the file host, and [`watch`], the server's loop
//! that fetches every `interval_s`.
//!
//! Owner: latlas-cli.
//! Invariants:
//! - The fetch runs without the app lock; listing, reading the changed blobs and the re-derive
//!   run under it, as every refresh does.
//! - A failed tick changes nothing served: the error is the tracker's, reported as the
//!   freshness `stale_reason` until a tick succeeds.
//! - Tracking does not pause: while paused the new texts wait in the warmer's to-do set.

use crate::app::App;
use crate::host::PackHost;
use latlas_core::Pack;
use latlas_core::track::{CommitId, GitPort as _, RepoTrack, Snapshot, Synced, Tracker};
use latlas_error::{ErrorCode, Result, error};
use latlas_host_native::clock::{SystemClock, TokioTimer};
use latlas_host_native::fs::Tracked;
use latlas_host_native::gitport::GitCli;
use latlas_ports::{Clock as _, DurNs, Timer as _};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// 2026-10-25: The tracker and the git host it reads through.
pub struct TrackCtl {
    pub tracker: Tracker,
    pub git: GitCli,
}

impl TrackCtl {
    /// 2026-10-25: Tracking `cfg` in the repository at `dir` (nothing fetched yet).
    #[must_use]
    pub fn new(cfg: RepoTrack, dir: std::path::PathBuf) -> Self {
        let git = GitCli::new(dir, Duration::from_secs(cfg.fetch_timeout_s));
        Self {
            tracker: Tracker::new(cfg),
            git,
        }
    }
}

/// 2026-10-25: Take one fetch result: on a new commit install its snapshot, re-derive and start
/// the warmer for the new texts (it waits while paused). Returns what the tick did; a failure is
/// also kept by the tracker for the freshness report.
pub async fn sync<P: Pack, H: PackHost<P>>(
    app: &mut App<P, H>,
    fetched: Result<CommitId>,
) -> Result<Synced> {
    let App {
        track,
        cfg,
        fs,
        host,
        ..
    } = &mut *app;
    let Some(ctl) = track.as_mut() else {
        return Err(error!(ErrorCode::LatlasConfig, "repo_track", "absent"));
    };
    let repo = ctl.tracker.cfg.repo.clone();
    let wanted = |listing: &Snapshot| -> Result<BTreeSet<String>> {
        let mut view = fs.clone();
        view.track = Some(Tracked {
            repo: repo.clone(),
            snap: Arc::new(listing.clone()),
        });
        let extra = host.extra_files(cfg, &view).map_err(|e| {
            let at = listing
                .commit
                .as_ref()
                .map_or_else(String::new, |c| c.0.clone());
            error!(ErrorCode::LatlasRepoRead, at, format!("{e:#}"))
        })?;
        view.tracked_paths::<P>(cfg, &extra)
    };
    let synced = ctl
        .tracker
        .advance(&ctl.git, &SystemClock, fetched, wanted)
        .await?;
    if let (Synced::New { .. }, Some(snap)) = (synced, ctl.tracker.snapshot()) {
        fs.track = Some(Tracked {
            repo,
            snap: snap.clone(),
        });
        app.refresh().await;
        crate::search::resume_warming(app).await;
    }
    Ok(synced)
}

/// 2026-10-25: The server's tracking loop; runs for the life of the process.
pub async fn watch<P: Pack, H: PackHost<P>>(
    app: Arc<Mutex<App<P, H>>>,
    git: GitCli,
    cfg: RepoTrack,
) {
    let every = DurNs::from_ms(cfg.interval_s.saturating_mul(1_000));
    loop {
        let next = SystemClock.monotonic_ns().plus(every);
        let fetched = git.fetch(&cfg.remote, &cfg.git_ref).await;
        // 2026-10-25: The outcome, failure included, is the tracker's state (`la_status`).
        let _ = sync(&mut *app.lock().await, fetched).await;
        TokioTimer.sleep_until(next).await;
    }
}
