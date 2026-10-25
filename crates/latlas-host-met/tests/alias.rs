// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The alias invocation end to end: the production `MetLocal` host spawns a fake
//! `met` (this same binary, `harness = false`, dispatching on `argv[1] == "embed"`; the only fake
//! is the `met` process itself, which does not exist yet) and every frame, deadline, kill and
//! check runs through production code. `cargo test --test alias -- <substring>` runs a subset.

#[path = "alias/fake.rs"]
mod fake;
#[path = "support/kbp.rs"]
mod kbp;

use latlas_core::embed::{Embedder, Mode, Reply, Role};
use latlas_core::hash::sha256;
use latlas_core::kbp::BlueprintId;
use latlas_core::metlocal::{Invocation, MetSettings};
use latlas_core::settings::InputTypes;
use latlas_error::ErrorCode;
use latlas_host_met::MetLocal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DIM: u32 = 5;

struct Rig {
    dir: PathBuf,
    blueprint: String,
    blueprint_id: BlueprintId,
}

impl Rig {
    fn new(name: &str, mode: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("alias-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("embed.kbp");
        let blueprint_id = kbp::write(&path, format!("\nmode={mode}\ndim={DIM}\n").as_bytes());
        Self {
            dir,
            blueprint: path.display().to_string(),
            blueprint_id,
        }
    }

    fn settings(&self, met_path: &str, boot_ms: u64, request_ms: u64, idle_ms: u64) -> MetSettings {
        MetSettings {
            invocation: Invocation::Alias {
                met_path: met_path.into(),
                boot_deadline_ms: boot_ms,
                idle_exit_ms: idle_ms,
            },
            blueprint: self.blueprint.clone(),
            blueprint_id: self.blueprint_id,
            model: "nvidia/llama-nemotron-embed-vl-1b-v2".into(),
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

    async fn host(&self, boot_ms: u64, request_ms: u64, idle_ms: u64) -> MetLocal {
        let exe = std::env::current_exe().unwrap().display().to_string();
        let s = self.settings(&exe, boot_ms, request_ms, idle_ms);
        let h = MetLocal::open(&s, self.dir.join("met.stderr.log")).await;
        assert!(
            matches!(h.mode(), Mode::Available { dim: DIM, .. }),
            "{:?}",
            h.mode()
        );
        h
    }

    /// 2026-10-24: A `met_path` that runs the fake with SIGINT ignored (inherited across exec,
    /// so the fake and its grandchild survive SIGINT and need the SIGKILL escalation).
    fn stubborn_met(&self) -> String {
        let p = self.dir.join("stubborn-met");
        let exe = std::env::current_exe().unwrap();
        std::fs::write(
            &p,
            format!("#!/bin/sh\ntrap '' INT\nexec '{}' \"$@\"\n", exe.display()),
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&p)
            .status()
            .unwrap();
        p.display().to_string()
    }

    fn pids(&self) -> Vec<u32> {
        std::fs::read_to_string(format!("{}.pids", self.blueprint))
            .unwrap_or_default()
            .lines()
            .map(|l| l.parse().unwrap())
            .collect()
    }
}

fn texts(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("text number {i} \u{e9}")).collect()
}

/// 2026-10-24: Whether `pid` is gone (no `/proc` entry, or a zombie awaiting its reaper).
fn gone(pid: u32) -> bool {
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

fn block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

fn bits(v: &[Vec<f32>]) -> Vec<Vec<u32>> {
    v.iter()
        .map(|r| r.iter().map(|x| x.to_bits()).collect())
        .collect()
}

fn expect_err(r: latlas_error::Result<Reply>) -> latlas_error::Error {
    match r {
        Ok(r) => panic!("expected an error, got {r:?}"),
        Err(e) => e,
    }
}

fn alias_happy_path_vectors_are_bit_exact_in_request_order() {
    let rig = Rig::new("happy", "good");
    block(async {
        let h = rig.host(10_000, 10_000, 0).await;
        let inputs = texts(7);
        let Reply::Vectors(v) = h.embed(Role::Document, &inputs).await.unwrap() else {
            panic!("vectors")
        };
        let want: Vec<Vec<f32>> = inputs.iter().map(|t| fake::vector(t, 5)).collect();
        assert_eq!(bits(&v), bits(&want));
        let Reply::Vectors(q) = h.embed(Role::Query, &inputs[..1]).await.unwrap() else {
            panic!("vectors")
        };
        assert_eq!(bits(&q), bits(&want[..1]));
    });
    let pids = rig.pids();
    assert_eq!(pids.len(), 2, "idle_exit_ms 0: one child per request");
    assert!(
        pids.iter().all(|p| gone(*p)),
        "each child exited after its request"
    );
}

fn an_idle_child_is_kept_per_role_and_dies_with_the_host() {
    let rig = Rig::new("idle", "good");
    block(async {
        let h = rig.host(10_000, 10_000, 60_000).await;
        for _ in 0..3 {
            h.embed(Role::Document, &texts(2)).await.unwrap();
        }
        assert_eq!(rig.pids().len(), 1, "three document requests, one child");
        h.embed(Role::Query, &texts(1)).await.unwrap();
        assert_eq!(
            rig.pids().len(),
            2,
            "a query child beside the document child"
        );
        assert!(
            rig.pids().iter().all(|p| !gone(*p)),
            "kept alive while the host lives"
        );
        drop(h);
    });
    assert!(
        rig.pids().iter().all(|p| gone(*p)),
        "the host's drop kills its children"
    );
}

fn a_boot_deadline_is_reported_and_the_whole_group_is_killed() {
    let rig = Rig::new("boot", "sleep");
    block(async {
        let h = rig.host(400, 30_000, 0).await;
        let t0 = Instant::now();
        let e = expect_err(h.embed(Role::Document, &texts(1)).await);
        assert_eq!(e.code(), ErrorCode::LatlasEmbedderDeadline, "{e}");
        assert_eq!(e.to_string(), "embedder boot deadline 400 ms passed");
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "SIGINT sufficed: {:?}",
            t0.elapsed()
        );
    });
    let pids = rig.pids();
    assert_eq!(pids.len(), 2, "the fake and its grandchild");
    for p in pids {
        assert!(gone(p), "pid {p} outlived the deadline");
    }
}

fn a_child_ignoring_sigint_is_sigkilled_after_the_grace() {
    let rig = Rig::new("stubborn", "sleep");
    let met = rig.stubborn_met();
    block(async {
        let s = rig.settings(&met, 30_000, 300, 0);
        let h = MetLocal::open(&s, rig.dir.join("met.stderr.log")).await;
        let t0 = Instant::now();
        let e = expect_err(h.embed(Role::Document, &texts(1)).await);
        assert_eq!(e.to_string(), "embedder request deadline 300 ms passed");
        let took = t0.elapsed();
        assert!(
            took >= Duration::from_millis(2_300),
            "escalated before the grace: {took:?}"
        );
        assert!(took < Duration::from_secs(5), "{took:?}");
    });
    for p in rig.pids() {
        assert!(gone(p), "pid {p} survived SIGKILL");
    }
}

fn malformed_frames_are_frame_errors() {
    for (mode, says) in [
        ("truncated", "truncated stream: 10 of 100 bytes"),
        ("badjson", "embedder frame response header"),
        ("oversize", "declares 4294967295 bytes"),
        ("badlen", "are not 3 x 5 x 4 = 60"),
    ] {
        let rig = Rig::new(mode, mode);
        block(async {
            let h = rig.host(10_000, 10_000, 0).await;
            let e = expect_err(h.embed(Role::Document, &texts(3)).await);
            assert_eq!(e.code(), ErrorCode::LatlasEmbedFrame, "{mode}: {e}");
            assert!(e.to_string().contains(says), "{mode}: {e}");
        });
        assert!(rig.pids().iter().all(|p| gone(*p)), "{mode}: child killed");
    }
}

fn a_dimension_mismatch_is_the_dim_error() {
    let rig = Rig::new("wrongdim", "wrongdim");
    block(async {
        let h = rig.host(10_000, 10_000, 0).await;
        let e = expect_err(h.embed(Role::Document, &texts(3)).await);
        assert_eq!(e.code(), ErrorCode::LatlasEmbedDim, "{e}");
        assert_eq!(
            e.to_string(),
            "embedder returned dimension 4; the config declares 5"
        );
    });
}

fn another_blueprint_or_prefix_echo_is_the_space_error() {
    for (mode, says) in [("wrongsha", "blueprint_sha256"), ("wrongprefix", "prefix")] {
        let rig = Rig::new(mode, mode);
        block(async {
            let h = rig.host(10_000, 10_000, 0).await;
            let e = expect_err(h.embed(Role::Document, &texts(2)).await);
            assert_eq!(e.code(), ErrorCode::LatlasEmbedSpace, "{mode}: {e}");
            assert!(e.to_string().contains(says), "{mode}: {e}");
        });
    }
}

fn a_child_that_dies_reports_its_status_and_last_stderr_line() {
    let rig = Rig::new("crash", "crash");
    block(async {
        let h = rig.host(10_000, 10_000, 0).await;
        let e = expect_err(h.embed(Role::Document, &texts(1)).await);
        assert_eq!(e.code(), ErrorCode::LatlasEmbedderExit, "{e}");
        assert!(e.to_string().contains("exit status: 3"), "{e}");
        assert!(
            e.to_string()
                .ends_with("MetModelLoad 0x0901: fake weights are absent"),
            "{e}"
        );
    });
    let log = std::fs::read_to_string(rig.dir.join("met.stderr.log")).unwrap();
    assert!(
        log.starts_with("loading weights\n"),
        "stderr goes to the host log: {log:?}"
    );
}

fn a_bad_met_path_or_blueprint_is_the_declared_unavailable_mode() {
    let rig = Rig::new("unavailable", "good");
    block(async {
        let exe = std::env::current_exe().unwrap().display().to_string();
        let log = rig.dir.join("log");
        let not_exec = rig.blueprint.clone();
        for (path, says) in [
            ("/nonexistent/met".to_string(), "unusable (NotFound)"),
            (not_exec, "is not an executable file"),
        ] {
            let h = MetLocal::open(&rig.settings(&path, 1, 1, 0), log.clone()).await;
            let Mode::Unavailable { reason } = h.mode() else {
                panic!("{path} must be unavailable")
            };
            assert!(reason.contains(says), "{reason}");
            let e = expect_err(h.embed(Role::Query, &texts(1)).await);
            assert_eq!(e.code(), ErrorCode::LatlasEmbedderUnavailable);
        }
        let mut s = rig.settings(&exe, 1, 1, 0);
        s.blueprint_id = BlueprintId(sha256(b"another blueprint"));
        let Mode::Unavailable { reason } = MetLocal::open(&s, log).await.mode() else {
            panic!("an id mismatch must be unavailable")
        };
        assert!(
            reason.contains(&format!("has BlueprintId {}", rig.blueprint_id.hex()))
                && reason.contains(&s.blueprint_id.hex()),
            "{reason}"
        );
    });
    assert!(rig.pids().is_empty(), "nothing was spawned");
}

fn more_inputs_than_max_batch_are_refused_unsent() {
    let rig = Rig::new("maxbatch", "good");
    block(async {
        let h = rig.host(10_000, 10_000, 0).await;
        let e = expect_err(h.embed(Role::Document, &texts(9)).await);
        assert_eq!(e.code(), ErrorCode::InvalidArgument, "{e}");
    });
    assert!(rig.pids().is_empty());
}

const CASES: &[(&str, fn())] = &[
    (
        "alias_happy_path",
        alias_happy_path_vectors_are_bit_exact_in_request_order,
    ),
    (
        "idle_child_kept",
        an_idle_child_is_kept_per_role_and_dies_with_the_host,
    ),
    (
        "boot_deadline",
        a_boot_deadline_is_reported_and_the_whole_group_is_killed,
    ),
    (
        "sigkill_after_grace",
        a_child_ignoring_sigint_is_sigkilled_after_the_grace,
    ),
    ("malformed_frames", malformed_frames_are_frame_errors),
    ("dimension_mismatch", a_dimension_mismatch_is_the_dim_error),
    (
        "space_mismatch",
        another_blueprint_or_prefix_echo_is_the_space_error,
    ),
    (
        "child_dies",
        a_child_that_dies_reports_its_status_and_last_stderr_line,
    ),
    (
        "unavailable_mode",
        a_bad_met_path_or_blueprint_is_the_declared_unavailable_mode,
    ),
    ("max_batch", more_inputs_than_max_batch_are_refused_unsent),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("embed") {
        fake::run(&args);
    }
    let filter: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let mut failed = Vec::new();
    for (name, case) in CASES {
        if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        let ok = std::panic::catch_unwind(case).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            failed.push(*name);
        }
    }
    println!("alias: {} failed {failed:?}", failed.len());
    if !failed.is_empty() {
        std::process::exit(1);
    }
}
