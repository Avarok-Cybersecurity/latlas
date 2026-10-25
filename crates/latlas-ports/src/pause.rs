// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The pause port: whether the host's pause signal is present right now.
//!
//! Owner: latlas-ports.
//! Invariants: a read answers at once (a plain `fn`, no wait); polling it on a schedule is the
//! host's job (ACNB), and what the signal is (a file, a flag) is the host's choice.

/// 2026-10-25: A host-supplied signal asking the process to hold no embedder while it is present.
pub trait PauseSignal {
    /// 2026-10-25: Whether the signal is present now.
    fn present(&self) -> bool;
}
