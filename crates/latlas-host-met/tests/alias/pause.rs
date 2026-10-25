// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: `MetLocal::suspend`/`resume` (the pause): a suspend stops every kept child and
//! cuts off the exchange in flight within the grace, a request while suspended spawns nothing and
//! answers `LatlasPaused`, and a resume spawns again.

use super::{Rig, block, expect_err, gone, texts};
use latlas_core::embed::{Embedder, Role};
use latlas_error::ErrorCode;
use std::time::{Duration, Instant};

pub fn a_suspend_stops_the_kept_child_and_spawns_nothing_until_resume() {
    let rig = Rig::new("pause-idle", "good");
    block(async {
        let h = rig.host(10_000, 10_000, 60_000).await;
        h.embed(Role::Document, &texts(2)).await.unwrap();
        let kept = rig.pids();
        assert_eq!(kept.len(), 1);
        assert!(!gone(kept[0]), "kept for idle_exit_ms");
        let t0 = Instant::now();
        h.suspend().await;
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        assert!(gone(kept[0]), "the kept child outlived the suspend");
        for role in [Role::Document, Role::Query] {
            let e = expect_err(h.embed(role, &texts(1)).await);
            assert_eq!(e.code(), ErrorCode::LatlasPaused, "{e}");
        }
        assert_eq!(rig.pids(), kept, "nothing was spawned while suspended");
        h.resume();
        h.embed(Role::Document, &texts(1)).await.unwrap();
        assert_eq!(rig.pids().len(), 2, "a resume spawns again");
    });
}

pub fn a_suspend_cuts_off_the_exchange_in_flight() {
    let rig = Rig::new("pause-flight", "sleep");
    block(async {
        let h = rig.host(60_000, 60_000, 60_000).await;
        let h2 = h.clone();
        let flight = tokio::spawn(async move { h2.embed(Role::Document, &texts(1)).await });
        while rig.pids().len() < 2 {
            tokio::task::yield_now().await;
        }
        let t0 = Instant::now();
        h.suspend().await;
        let e = expect_err(flight.await.unwrap());
        assert_eq!(e.code(), ErrorCode::LatlasPaused, "{e}");
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    });
    let pids = rig.pids();
    assert_eq!(pids.len(), 2, "the fake and its grandchild");
    for p in pids {
        assert!(gone(p), "pid {p} outlived the suspend");
    }
}
