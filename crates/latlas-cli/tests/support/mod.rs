// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The one-label test pack, its host, the scripted loopback embeddings endpoint and
//! the project on disk that the App tests share (moved from search_bounded.rs 2026-10-25).
#![allow(dead_code)]

use latlas_cli::host::PackHost;
use latlas_core::bundle::{Builder, Bundle};
use latlas_core::derive::need;
use latlas_core::id::{Label, NodeId, Rel, Vocab, core_labels};
use latlas_core::pack::{Pack, SectionCtx};
use latlas_core::{Config, SourceSet};
use latlas_error::{ErrorCode, Result, error};
use latlas_host_native::fs::{FsSources, Stamp};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub struct Tiny;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TinySources {
    root: String,
    files: Vec<String>,
}

impl Vocab for Tiny {
    const LABELS: &'static [Label] = &[];
    const RELS: &'static [Rel] = &[];
}

impl Pack for Tiny {
    const ID: &'static str = "tiny";
    const VERSION: &'static str = "0.0.0";
    const CONFIG_SCHEMA: &'static str = "tiny/v1";
    const NAMED_LABELS: &'static [Label] = &[];
    const SEARCH_EXPAND: &'static [Rel] = &[];
    const IMPACT_FOLLOW_OUT: &'static [Rel] = &[];
    const SECTIONS: &'static [&'static str] = &[];
    const DEFAULT_SECTIONS: &'static [&'static str] = &[];
    const RECORD_SCHEMAS: &'static [latlas_core::record_schema::KindSchema] = &[];
    const INSTRUCTIONS: &'static str = "test pack";
    const STATUS_DESCRIPTION: &'static str = "status";
    const IMPACT_DESCRIPTION: &'static str = "impact";
    type Sources = TinySources;
    type Facts = ();

    fn root(s: &TinySources) -> &str {
        &s.root
    }
    fn validate(_: &TinySources) -> Result<()> {
        Ok(())
    }
    fn source_paths(s: &TinySources) -> Vec<String> {
        s.files.clone()
    }
    fn source_dirs(_: &TinySources) -> Vec<String> {
        Vec::new()
    }
    fn scopes(_: &TinySources) -> Vec<String> {
        vec!["t".into()]
    }
    fn default_instance(_: &TinySources, _: &str) -> Option<String> {
        None
    }
    fn derive(
        cfg: &Config<TinySources>,
        src: &SourceSet,
        _: Option<&()>,
        b: &mut Builder,
    ) -> Result<()> {
        for f in &cfg.sources.files {
            let text = need(src, f)?;
            let id = NodeId::new(core_labels::Design, f.clone())?;
            b.define(id.clone(), f.clone(), text, Default::default());
            b.text(&id, "body", text);
        }
        Ok(())
    }
    fn sealed(_: &mut Bundle, _: Option<&()>) {}
    fn section(_: &SectionCtx<'_, Self>, name: &str) -> Result<Value> {
        Err(error!(
            ErrorCode::LatlasToolArgument,
            "section",
            name.to_string()
        ))
    }
    fn impact_rels(_: Label, _: bool) -> Option<Vec<Rel>> {
        None
    }
    fn impact_extra(
        _: &Bundle,
        _: &Config<TinySources>,
        _: &NodeId,
        _: &mut Map<String, Value>,
    ) -> Result<()> {
        Ok(())
    }
}

pub struct TinyHost;

impl PackHost<Tiny> for TinyHost {
    fn open(_: &Config<TinySources>, _: &FsSources) -> anyhow::Result<Self> {
        Ok(TinyHost)
    }
    fn extra_files(&self, _: &Config<TinySources>, _: &FsSources) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn extra_stamp(&self, _: &Config<TinySources>, _: &FsSources) -> anyhow::Result<Stamp> {
        Ok(Vec::new())
    }
    fn facts(
        &self,
        _: &Config<TinySources>,
        _: &FsSources,
    ) -> anyhow::Result<(Option<()>, Option<String>)> {
        Ok((None, None))
    }
    async fn record(
        &self,
        _: &Config<TinySources>,
        _: &FsSources,
        _: &Bundle,
        _: &Value,
        _: latlas_ports::UnixMs,
    ) -> Result<(Value, bool)> {
        Err(error!(ErrorCode::LatlasToolArgument, "kind", "none"))
    }
}

/// 2026-10-23: What the scripted provider does with one request.
#[derive(Clone, Copy)]
pub enum Reply {
    After(u64),
    Hang,
}

/// 2026-10-23: A loopback embeddings endpoint: `script(input_type, nth request of that type)`
/// picks each reply; vectors are 4-dimensional and distinct per input.
pub async fn provider(script: fn(&str, usize) -> Reply) -> (u16, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let passages = Arc::new(AtomicUsize::new(0));
    let queries = Arc::new(AtomicUsize::new(0));
    let seen = passages.clone();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            let (passages, queries) = (passages.clone(), queries.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let body = loop {
                    let mut chunk = [0u8; 4096];
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let Some(h) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let len: usize = text[..h]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= h + 4 + len {
                        break text[h + 4..h + 4 + len].to_string();
                    }
                };
                let req: Value = serde_json::from_str(&body).unwrap();
                let kind = req["input_type"].as_str().unwrap().to_string();
                let counter = if kind == "query" { &queries } else { &passages };
                let nth = counter.fetch_add(1, Ordering::SeqCst);
                match script(&kind, nth) {
                    Reply::Hang => {
                        tokio::time::sleep(Duration::from_secs(3600)).await;
                    }
                    Reply::After(ms) => {
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        let data: Vec<Value> = req["input"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .enumerate()
                            .map(|(i, t)| {
                                let x = t.as_str().unwrap().len() as f64;
                                json!({"index": i, "embedding": [1.0, x, 0.5, 0.25]})
                            })
                            .collect();
                        let out = json!({"data": data}).to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                            out.len()
                        );
                        let _ = s.write_all(resp.as_bytes()).await;
                    }
                }
            });
        }
    });
    (port, seen)
}

/// 2026-10-23: A project of three one-line files with the embedder at `port`. `timeout_ms` is
/// what a search that waited on the provider would cost; `deadline_ms` is the search bound.
pub fn project(name: &str, port: u16, timeout_ms: u64, deadline_ms: u64) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("latlas-search-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, t) in [
        ("a.md", "alpha kernel"),
        ("b.md", "beta lever"),
        ("c.md", "gamma ledger"),
    ] {
        std::fs::write(dir.join(f), t).unwrap();
    }
    std::fs::write(dir.join("key.txt"), "tk-fixturekey\n").unwrap();
    let d = dir.display();
    let cfg = format!(
        r#"schema = "tiny/v1"
[sources]
root = "{d}"
files = ["a.md", "b.md", "c.md"]
[embedder]
mode = "http"
endpoint = "http://127.0.0.1:{port}/v1/embeddings"
model = "fake"
dimensions = 4
key_file = "{d}/key.txt"
key_prefix = "tk-"
timeout_ms = {timeout_ms}
batch = 1
max_attempts = 1
backoff_ms = 1
min_interval_ms = 1
max_input_chars = 1000
input_type = {{ query = "query", document = "passage" }}
[limits]
max_rows = 100
cypher_timeout_ms = 1000
token_budget = 1200
status_tokens = 1500
search_deadline_ms = {deadline_ms}
[store]
data_dir = "{d}/data"
scratch_dir = "{d}/scratch"
[search]
w_cosine = 0.6
w_hops = 0.25
w_kind = 0.15
[validator]
python = "/usr/bin/python3"
"#
    );
    let path = dir.join("latlas.toml");
    std::fs::write(&path, cfg).unwrap();
    (path, dir.join("data"))
}
