// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The gate over a fake signal and a manual clock: one read per step, one edge per
//! change, the declared reason, and the next read one poll interval on.

use super::{Edge, PAUSE_POLL_MS, PauseGate};
use latlas_ports::{Clock, MonoNs, PauseSignal, UnixMs};
use std::cell::Cell;

#[derive(Default)]
struct Fake {
    present: Cell<bool>,
    reads: Cell<u32>,
}

impl PauseSignal for Fake {
    fn present(&self) -> bool {
        self.reads.set(self.reads.get() + 1);
        self.present.get()
    }
}

struct Manual {
    mono: Cell<u64>,
    wall: Cell<i64>,
}

impl Clock for Manual {
    fn now_unix_ms(&self) -> UnixMs {
        UnixMs(self.wall.get())
    }
    fn monotonic_ns(&self) -> MonoNs {
        MonoNs(self.mono.get())
    }
}

fn clock() -> Manual {
    Manual {
        mono: Cell::new(7),
        wall: Cell::new(1_000),
    }
}

#[test]
fn an_absent_signal_never_pauses_and_the_next_read_is_one_poll_on() {
    let (s, c, mut g) = (Fake::default(), clock(), PauseGate::default());
    assert_eq!(
        g.step(&s, &c),
        (None, MonoNs(7 + PAUSE_POLL_MS * 1_000_000))
    );
    assert!(!g.paused());
    assert_eq!(g.reason(), None);
    assert_eq!(s.reads.get(), 1, "one read per step");
}

#[test]
fn a_present_signal_pauses_once_with_the_declared_reason_and_its_removal_resumes() {
    let (s, c, mut g) = (Fake::default(), clock(), PauseGate::default());
    s.present.set(true);
    assert_eq!(g.step(&s, &c).0, Some(Edge::Paused));
    assert_eq!(g.reason().as_deref(), Some("paused: latlas.lock present"));
    c.wall.set(9_000);
    assert_eq!(g.step(&s, &c).0, None, "still present: no second edge");
    let st = g.status("/x/latlas.lock");
    assert_eq!(st["paused"], true);
    assert_eq!(st["paused_since_ms"], 1_000, "the time the pause began");
    assert_eq!(st["signal"], "/x/latlas.lock");
    assert_eq!(st["poll_ms"], PAUSE_POLL_MS);
    s.present.set(false);
    assert_eq!(g.step(&s, &c).0, Some(Edge::Resumed));
    assert!(!g.paused());
    assert_eq!(g.status("s")["paused_since_ms"], serde_json::Value::Null);
    assert_eq!(g.step(&s, &c).0, None);
    assert_eq!(s.reads.get(), 4);
}
