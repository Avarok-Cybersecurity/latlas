// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: `latlas`: the core commands (derive, check, status, impact, query, search, probes,
//! tools-list, serve) and `score` over the declarative pack, for any repository with a
//! `.latlas/config.toml`.
//!
//! Owner: latlas (the binary).
//! Invariants: stdout is JSON on every path; the exit code of `score` is the ratchet's.

mod host;

use anyhow::Result;
use clap::{Parser, Subcommand};
use host::DeclHost;
use latlas_cli::{CoreCmd, Identity, run_core};
use latlas_decl::Decl;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "latlas", about = "Latlas over a repository's .latlas/config.toml")]
struct Cli {
    /// The latlas-project/v1 file (no default).
    #[arg(long)]
    config: PathBuf,
    /// Use this store directory instead of the config's `store.data_dir`.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(flatten)]
    Core(CoreCmd),
    /// The integrity score vector and ratchet: the head is --config, the base (when given) a
    /// second config; exits 3 on a regression, 4 on a fatal finding.
    Score {
        #[arg(long)]
        check: PathBuf,
        #[arg(long)]
        base_config: Option<PathBuf>,
        #[arg(long)]
        comment: Option<PathBuf>,
        /// CI gate results, `{"<gate key>": "pass"|"fail"}` (G is `absent` without them).
        #[arg(long)]
        gate_results: Option<PathBuf>,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            if let Some(x) = e.downcast_ref::<latlas_cli::score::CheckExit>() {
                eprintln!("latlas: {e}");
                return std::process::ExitCode::from(x.0);
            }
            let code = e
                .downcast_ref::<latlas_error::Error>()
                .map_or("Failed", |k| k.code().name());
            println!("{}", serde_json::json!({"error": code, "message": format!("{e:#}")}));
            eprintln!("latlas: {code}: {e:#}");
            std::process::ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let data = cli.data_dir.as_deref();
    match cli.cmd {
        Cmd::Core(c) => {
            let id = Identity {
                name: "latlas",
                version: env!("CARGO_PKG_VERSION"),
            };
            run_core::<Decl, DeclHost>(&cli.config, data, c, id).await
        }
        Cmd::Score {
            check,
            base_config,
            comment,
            gate_results,
        } => latlas_cli::score::score_cmd::<Decl, DeclHost>(
            &cli.config,
            base_config.as_deref(),
            &check,
            comment.as_deref(),
            gate_results.as_deref(),
        ),
    }
}
