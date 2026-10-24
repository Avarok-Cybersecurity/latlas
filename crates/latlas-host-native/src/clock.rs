// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The system wall clock and the process monotonic clock ([`Clock`]), and the tokio
//! wait on it ([`Timer`]).
//!
//! Owner: latlas-host-native (cut from metrale-io-host-std, 2026-10-23 K0).
//! Invariants:
//! - `monotonic_ns` counts from one process origin (the first read by any `SystemClock` or
//!   `TokioTimer`), so every clock in the process agrees and none decreases.
//! - A wall clock before the Unix epoch is a host fault and reads as 0, reported nowhere else.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use latlas_ports::{Clock, MonoNs, Timer, UnixMs};

/// 2026-10-23: The instant `MonoNs(0)` names, fixed once per process (written once, at the first
/// read; ROTI).
fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// 2026-10-23: The host's clocks; every instance reads the same process origin.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> UnixMs {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        UnixMs(i64::try_from(ms).unwrap_or(i64::MAX))
    }

    fn monotonic_ns(&self) -> MonoNs {
        MonoNs(u64::try_from(origin().elapsed().as_nanos()).unwrap_or(u64::MAX))
    }
}

/// 2026-10-23: `sleep_until` on tokio's timer, against the same origin as [`SystemClock`].
#[derive(Clone, Copy, Debug)]
pub struct TokioTimer;

impl Timer for TokioTimer {
    async fn sleep_until(&self, at: MonoNs) {
        let deadline = origin() + Duration::from_nanos(at.0);
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
    }
}
