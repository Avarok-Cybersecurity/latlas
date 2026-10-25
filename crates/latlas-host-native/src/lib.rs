// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: Latlas's native process hosts: [`fs::FsSources`] reads and stamps the configured
//! sources, [`record::FsRecord`] is the record file port, [`openrouter::OpenRouter`] the HTTP
//! embedder, [`settings`] reads the global layer `~/.latlas/config.toml`, (2026-10-25)
//! [`pause::LockFile`] is the pause signal. The binary is the only
//! place these are chosen.
//!
//! Owner: latlas-host-native.
//! Invariants:
//! - A credential-named file is never opened: it is listed by name only; a key file is read only
//!   through its reference, and the key never reaches argv, logs or results.
//! - Every failure is a registry error (`LatlasSource`, `LatlasRecordWrite`, ...).
#![forbid(unsafe_code)]

pub mod clock;
pub mod fs;
pub mod git;
pub mod openrouter;
pub mod pause;
pub mod record;
pub mod settings;
pub mod write;
