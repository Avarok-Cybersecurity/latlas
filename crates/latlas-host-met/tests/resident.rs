// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The resident invocation end to end: the production `MetLocal` host against a
//! scripted loopback HTTP server standing in for `met serve` (the only fake: met has no
//! `/v1/embeddings` yet). The server answers in reversed `data` order, so placement by `index`
//! is exercised, with base64 little-endian f32 vectors.

#[path = "support/kbp.rs"]
mod kbp;

use base64::Engine as _;
use latlas_core::embed::{Embedder, Mode, Reply, Role};
use latlas_core::kbp::BlueprintId;
use latlas_core::metlocal::{Invocation, MetSettings};
use latlas_core::settings::InputTypes;
use latlas_error::ErrorCode;
use latlas_host_met::MetLocal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const DIM: u32 = 3;

#[derive(Clone, Copy, PartialEq)]
enum Script {
    Good,
    WrongDim,
    Slow,
    NoModels,
}

fn vector(text: &str) -> Vec<f32> {
    let n = u32::try_from(text.len()).unwrap();
    vec![-0.0, f32::from_bits(n), f32::from_bits(0x3f80_0000 | n)]
}

async fn reply(s: &mut TcpStream, status: &str, body: &str) {
    let r = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(r.as_bytes()).await;
}

async fn handle(mut s: TcpStream, script: Script, sha: BlueprintId) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head_end, len) = loop {
        let n = s.read(&mut chunk).await.unwrap();
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map_or(0, |v| v.trim().parse::<usize>().unwrap());
            break (i + 4, len);
        }
    };
    while buf.len() < head_end + len {
        let n = s.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
    }
    let line = String::from_utf8_lossy(&buf[..head_end]).to_string();
    if line.starts_with("GET /v1/models ") {
        if script == Script::NoModels {
            return reply(&mut s, "404 Not Found", "{}").await;
        }
        return reply(&mut s, "200 OK", r#"{"object":"list","data":[]}"#).await;
    }
    assert!(line.starts_with("POST /v1/embeddings "), "{line}");
    let req: Value = serde_json::from_slice(&buf[head_end..]).unwrap();
    assert_eq!(req["encoding_format"], "base64");
    let input_type = req["input_type"].as_str().unwrap().to_string();
    if script == Script::Slow {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let inputs: Vec<String> = serde_json::from_value(req["input"].clone()).unwrap();
    let data: Vec<Value> = inputs
        .iter()
        .enumerate()
        .rev()
        .map(|(i, t)| {
            let mut v = vector(t);
            if script == Script::WrongDim {
                v.push(1.0);
            }
            let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            json!({"index": i, "object": "embedding",
                "embedding": base64::engine::general_purpose::STANDARD.encode(bytes)})
        })
        .collect();
    let body = json!({"object": "list", "data": data, "model": req["model"], "usage": {},
        "prefix": format!("{input_type}: "), "blueprint_sha256": sha.hex(),
        "blueprint_trust": "dev-anchor"});
    reply(&mut s, "200 OK", &body.to_string()).await;
}

async fn serve(script: Script, sha: BlueprintId) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = l.accept().await {
            tokio::spawn(handle(s, script, sha));
        }
    });
    port
}

fn blueprint(name: &str) -> (String, BlueprintId) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("resident-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("embed.kbp");
    let id = kbp::write(&p, format!("resident {name}\n").as_bytes());
    (p.display().to_string(), id)
}

fn settings(name: &str, port: u16, request_ms: u64) -> MetSettings {
    let (blueprint, blueprint_id) = blueprint(name);
    MetSettings {
        invocation: Invocation::Resident {
            endpoint: format!("http://127.0.0.1:{port}/v1/embeddings"),
        },
        blueprint,
        blueprint_id,
        model: "m".into(),
        dimensions: DIM,
        input_type: InputTypes {
            query: "query".into(),
            document: "passage".into(),
        },
        max_batch: 8,
        max_input_chars: 100,
        request_deadline_ms: request_ms,
    }
}

async fn host(name: &str, script: Script, request_ms: u64) -> MetLocal {
    let sha = blueprint(name).1;
    let port = serve(script, sha).await;
    MetLocal::open(
        &settings(name, port, request_ms),
        PathBuf::from("/dev/null"),
    )
    .await
}

fn texts() -> Vec<String> {
    vec!["a".into(), "bbbb".into(), "cc".into()]
}

#[tokio::test(flavor = "multi_thread")]
async fn resident_happy_path_vectors_are_bit_exact_and_placed_by_index() {
    let h = host("happy", Script::Good, 5_000).await;
    assert!(
        matches!(h.mode(), Mode::Available { dim: DIM, .. }),
        "{:?}",
        h.mode()
    );
    let Reply::Vectors(v) = h.embed(Role::Document, &texts()).await.unwrap() else {
        panic!("vectors")
    };
    let got: Vec<Vec<u32>> = v
        .iter()
        .map(|r| r.iter().map(|x| x.to_bits()).collect())
        .collect();
    let want: Vec<Vec<u32>> = texts()
        .iter()
        .map(|t| vector(t).iter().map(|x| x.to_bits()).collect())
        .collect();
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resident_dimension_mismatch_is_the_dim_error() {
    let h = host("wrongdim", Script::WrongDim, 5_000).await;
    let e = h.embed(Role::Query, &texts()).await.unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedDim, "{e}");
    assert_eq!(
        e.to_string(),
        "embedder returned dimension 4; the config declares 3"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_resident_is_the_request_deadline() {
    let h = host("slow", Script::Slow, 300).await;
    let t0 = std::time::Instant::now();
    let e = h.embed(Role::Query, &texts()).await.unwrap_err();
    assert_eq!(e.to_string(), "embedder request deadline 300 ms passed");
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_models_probe_is_the_declared_unavailable_mode() {
    let h = host("nomodels", Script::NoModels, 5_000).await;
    let Mode::Unavailable { reason } = h.mode() else {
        panic!("must be unavailable")
    };
    assert!(reason.contains("/v1/models answered HTTP 404"), "{reason}");
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let h = MetLocal::open(&settings("closed", port, 2_000), PathBuf::from("/dev/null")).await;
    let Mode::Unavailable { reason } = h.mode() else {
        panic!("a closed port must be unavailable")
    };
    assert!(reason.contains("/v1/models refused"), "{reason}");
}
