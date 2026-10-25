// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: What a pack needs from the process beyond the generic hosts: files only it can list,
//! stamps that are not files (a checkout's commit), facts only a subprocess or git can compute, and
//! its record kinds' validators. Implemented by the binary that picks the pack (NDIC).
//!
//! Owner: latlas-cli.
//! Invariants: every method is called with the config resolved once at open (ROTI); none prints.

use latlas_core::{Bundle, Config, Pack};
use latlas_host_native::fs::{FsSources, Stamp};
use latlas_ports::UnixMs;
use serde_json::Value;

/// 2026-10-23: The process side of a pack.
pub trait PackHost<P: Pack>: Sized {
    /// Build once from the config (validators, scratch paths).
    fn open(cfg: &Config<P::Sources>, fs: &FsSources) -> anyhow::Result<Self>;
    /// Root-relative files only this host can list; read and stamped with the pack's own.
    fn extra_files(&self, cfg: &Config<P::Sources>, fs: &FsSources) -> anyhow::Result<Vec<String>>;
    /// Stamp entries that are not files (`<repo>@git`): part of the freshness check, never the
    /// newest-change report.
    fn extra_stamp(&self, cfg: &Config<P::Sources>, fs: &FsSources) -> anyhow::Result<Stamp>;
    /// The facts the derive needs and the source commit every answer names.
    fn facts(
        &self,
        cfg: &Config<P::Sources>,
        fs: &FsSources,
    ) -> anyhow::Result<(Option<P::Facts>, Option<String>)>;
    /// `la_record`: write one record through its validator (fails closed). Returns the answer
    /// and whether a file was written (then the app re-derives). `now` is the Clock port's
    /// reading, the only time a declared fill rule (`Fill::NowUtcMinute`) may use.
    /// The future is `Send`: the MCP server answers on a multi-task runtime.
    fn record(
        &self,
        cfg: &Config<P::Sources>,
        fs: &FsSources,
        bundle: &Bundle,
        args: &Value,
        now: UnixMs,
    ) -> impl core::future::Future<Output = latlas_error::Result<(Value, bool)>> + Send;
}
