// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The pack-independent subcommands, for a binary to flatten into its own command
//! line (`#[command(flatten)] Core(CoreCmd)`) next to its pack's commands, and the runner.
//!
//! Owner: latlas-cli.
//! Invariants: `serve` writes nothing to stdout but the MCP protocol; every other command prints
//! one JSON document (or the bundle JSONL for `derive`).

use crate::app::App;
use crate::gates;
use crate::host::PackHost;
use crate::mcp::Server;
use anyhow::Result;
use clap::Subcommand;
use latlas_core::Pack;
use std::path::{Path, PathBuf};

/// 2026-10-23: The core subcommands.
#[derive(Subcommand)]
pub enum CoreCmd {
    /// Print the bundle JSONL (or write it to --out).
    Derive {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Derive twice, compare digests, list fatal findings, measure every scope's default status;
    /// --store-roundtrip also rebuilds two fresh stores and compares their exports.
    Check {
        #[arg(long)]
        store_roundtrip: bool,
    },
    /// la_status sections as JSON.
    Status {
        #[arg(long)]
        scope: String,
        #[arg(long = "section")]
        sections: Vec<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    /// la_impact as JSON.
    Impact {
        id: String,
        #[arg(long)]
        direction: String,
        #[arg(long = "relation")]
        relations: Vec<String>,
        #[arg(long)]
        max_depth: Option<u32>,
    },
    /// Read-only Cypher.
    Query {
        cypher: String,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// la_search as JSON; --graph-only declares the embedder off for this run.
    Search {
        text: String,
        #[arg(long = "kind")]
        kinds: Vec<String>,
        #[arg(long)]
        k: Option<u32>,
        #[arg(long)]
        hops: Option<u32>,
        #[arg(long)]
        graph_only: bool,
    },
    /// The recall gate: each `probe<TAB>id` line must hit the id in the top 3.
    Probes {
        file: PathBuf,
        #[arg(long)]
        graph_only: bool,
        /// The least hits out of the probe count for the gate to pass.
        #[arg(long)]
        need: u32,
    },
    /// The tools/list body the server sends (for the token gate).
    ToolsList,
    /// Serve the 5 tools over MCP stdio.
    Serve,
}

/// 2026-10-23: What the binary names itself at `initialize`.
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    pub name: &'static str,
    pub version: &'static str,
}

/// 2026-10-23: Run one core subcommand for pack `P` over host `H`.
pub async fn run_core<P, H>(
    cfg: &Path,
    data: Option<&Path>,
    cmd: CoreCmd,
    id: Identity,
) -> Result<()>
where
    P: Pack + Send + Sync + 'static,
    H: PackHost<P> + Send + Sync + 'static,
    App<P, H>: Send,
{
    match cmd {
        CoreCmd::Derive { out } => gates::derive_cmd::<P, H>(cfg, out.as_deref()),
        CoreCmd::Check { store_roundtrip } => gates::check_cmd::<P, H>(cfg, store_roundtrip).await,
        CoreCmd::ToolsList => {
            println!("{}", latlas_core::tools::tools_list_json::<P>());
            Ok(())
        }
        CoreCmd::Serve => serve::<P, H>(cfg, data, id).await,
        CoreCmd::Search {
            text,
            kinds,
            k,
            hops,
            graph_only,
        } => {
            let args = serde_json::json!({"text": text, "kinds": kinds, "k": k, "hops": hops});
            let mut app = App::<P, H>::open(cfg, data).await?;
            if graph_only {
                app.declare_graph_only("--graph-only");
            }
            let v = crate::handlers::call(&mut app, "la_search", &args).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
        CoreCmd::Probes {
            file,
            graph_only,
            need,
        } => gates::probes_cmd::<P, H>(cfg, data, &file, graph_only, need).await,
        CoreCmd::Status {
            scope,
            sections,
            instance,
        } => {
            let args =
                serde_json::json!({"scope": scope, "sections": sections, "instance": instance});
            tool::<P, H>(cfg, data, "la_status", args).await
        }
        CoreCmd::Impact {
            id,
            direction,
            relations,
            max_depth,
        } => {
            let args = serde_json::json!({"id": id, "direction": direction, "relations": relations, "max_depth": max_depth});
            tool::<P, H>(cfg, data, "la_impact", args).await
        }
        CoreCmd::Query { cypher, limit } => {
            let args = serde_json::json!({"cypher": cypher, "limit": limit});
            tool::<P, H>(cfg, data, "la_query", args).await
        }
    }
}

async fn tool<P: Pack, H: PackHost<P>>(
    cfg: &Path,
    data: Option<&Path>,
    name: &str,
    args: serde_json::Value,
) -> Result<()> {
    let mut app = App::<P, H>::open(cfg, data).await?;
    let v = crate::handlers::call(&mut app, name, &args).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

async fn serve<P, H>(cfg: &Path, data: Option<&Path>, id: Identity) -> Result<()>
where
    P: Pack + Send + Sync + 'static,
    H: PackHost<P> + Send + Sync + 'static,
    App<P, H>: Send,
{
    use rmcp::ServiceExt;
    let app = App::<P, H>::open(cfg, data).await?;
    crate::search::start_warming(&app).await?;
    let (pause, host) = (app.pause.clone(), app.embed.host().cloned());
    let track = app
        .track
        .as_ref()
        .map(|t| (t.git.clone(), t.tracker.cfg.clone()));
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    tokio::spawn(crate::pause::watch(app.clone(), pause, host));
    if let Some((git, cfg)) = track {
        tokio::spawn(crate::track::watch(app.clone(), git, cfg));
    }
    let server = Server {
        app,
        name: id.name,
        version: id.version,
    };
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
