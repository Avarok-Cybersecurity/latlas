// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: Latlas's command line and MCP server as a library: a binary picks a pack and its
//! [`host::PackHost`] once (NDIC) and runs [`cli::run_core`] next to its own commands. The five
//! tools (`la_query`, `la_search`, `la_impact`, `la_record`, `la_status`) are [`handlers::call`],
//! shared by the CLI and the server so both run the same code.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants: the only Latlas crate with `anyhow`; the server writes nothing to stdout but MCP.
#![forbid(unsafe_code)]

pub mod app;
pub mod cli;
pub mod embedhost;
pub mod gates;
pub mod handlers;
pub mod host;
pub mod mcp;
pub mod pause;
pub mod score;
pub mod search;
pub mod track;

pub use app::App;
pub use cli::{CoreCmd, Identity, run_core};
pub use host::PackHost;
