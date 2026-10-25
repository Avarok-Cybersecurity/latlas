// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The port traits Latlas's pure code depends on: [`Clock`], [`Timer`], (2026-10-25)
//! [`PauseSignal`] and the shared time values. Hosts implement them (`latlas-host-native`); a binary is the only place a host
//! is chosen.
//!
//! Owner: latlas-ports.
//! Invariants:
//! - No I/O; `no_std`.
//! - A read that answers at once is a plain `fn`; a wait is an `async fn` with no `Send` bound
//!   (ACNB).
#![no_std]
#![forbid(unsafe_code)]
#![expect(
    async_fn_in_trait,
    reason = "2026-10-23: single-threaded callers; no Send bound by design"
)]

mod clock;
mod pause;
mod time;
mod timer;

pub use clock::Clock;
pub use pause::PauseSignal;
pub use time::{DurNs, MonoNs, UnixMs};
pub use timer::Timer;
