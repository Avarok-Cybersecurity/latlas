// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: Process checks the alias cases share (moved from alias.rs, unchanged).

use std::time::{Duration, Instant};

/// 2026-10-24: Whether `pid` is gone (no `/proc` entry, or a zombie awaiting its reaper).
pub fn gone(pid: u32) -> bool {
    let t0 = Instant::now();
    loop {
        let dead = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => true,
            Ok(s) => s
                .rsplit(')')
                .next()
                .is_some_and(|r| r.trim_start().starts_with('Z')),
        };
        if dead || t0.elapsed() > Duration::from_secs(3) {
            return dead;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
