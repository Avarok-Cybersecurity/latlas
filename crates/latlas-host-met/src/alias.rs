// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The alias invocation: one `met embed` child per request (or kept `idle_exit_ms`
//! for the next request of the same role), two frames each way (`latlas_core::metframe`).
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - Deadlines: the response header within `boot_deadline_ms` of the spawn (a fresh child) and
//!   the whole exchange within `request_deadline_ms` of the request; whichever passes first is
//!   reported, after the child's group is terminated (SIGINT, SIGKILL after 2 s).
//! - Any failure terminates the child: its stream position is unknown, so it is never reused.
//! - A child is kept per role, because `--input-type` is fixed for its life.

use std::path::PathBuf;
use std::time::Duration;

use latlas_core::embed::Role;
use latlas_core::metframe::{
    Expect, FrameReader, Limits, check_header, decode_vectors, encode_request,
};
use latlas_core::metlocal::MetSettings;
use latlas_error::{Error, ErrorCode, Result, error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::{Instant, timeout_at};

use crate::child::MetChild;

/// 2026-10-24: The alias host.
pub struct Alias {
    s: MetSettings,
    met_path: String,
    boot_deadline_ms: u64,
    idle_exit_ms: u64,
    log: PathBuf,
    /// 2026-10-24: The kept child per role (`[query, document]`), when `idle_exit_ms > 0`.
    kept: [Mutex<Option<MetChild>>; 2],
}

/// 2026-10-24: Why an exchange stopped, before the child is dealt with.
enum Stop {
    Deadline(&'static str, u64),
    /// 2026-10-24: Stdout ended or stdin broke inside frame `which`.
    Ended(&'static str, FrameReader),
    Err(Error),
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Self::Err(e)
    }
}

fn slot(role: Role) -> usize {
    match role {
        Role::Query => 0,
        Role::Document => 1,
    }
}

impl Alias {
    /// 2026-10-24: The host for checked settings (`MetLocal::open` ran the preflight).
    pub fn new(
        s: MetSettings,
        met_path: String,
        boot_deadline_ms: u64,
        idle_exit_ms: u64,
        log: PathBuf,
    ) -> Self {
        Self {
            s,
            met_path,
            boot_deadline_ms,
            idle_exit_ms,
            log,
            kept: [Mutex::new(None), Mutex::new(None)],
        }
    }

    /// 2026-10-24: The settings it runs with.
    pub fn settings(&self) -> &MetSettings {
        &self.s
    }

    /// 2026-10-24: Embed `inputs` (one request, all of `role`).
    pub async fn embed(&self, role: Role, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let input_type = self.s.input_type.of(role);
        let request = encode_request(&self.s.model, &self.s.blueprint_sha256, input_type, inputs)?;
        let mut kept = self.kept[slot(role)].lock().await;
        let reuse = kept.take().and_then(|mut c| c.alive().then_some(c));
        let mut child = match reuse {
            Some(c) => c,
            None => MetChild::spawn(
                &self.met_path,
                &self.s.blueprint,
                input_type,
                self.idle_exit_ms,
                &self.log,
            )?,
        };
        let expect = Expect {
            model: &self.s.model,
            blueprint_sha256: &self.s.blueprint_sha256,
            input_type,
            count: inputs.len(),
            dim: self.s.dimensions,
        };
        match self.exchange(&mut child, &request, &expect).await {
            Ok(v) => {
                child.fresh = false;
                if self.idle_exit_ms > 0 {
                    *kept = Some(child);
                } else {
                    child.finish().await;
                }
                Ok(v)
            }
            Err(stop) => {
                let e = match stop {
                    Stop::Deadline(which, ms) => {
                        error!(ErrorCode::LatlasEmbedderDeadline, which, ms)
                    }
                    Stop::Ended(which, reader) => match child.exit_error().await {
                        Some(e) => e,
                        None => reader.truncated(which),
                    },
                    Stop::Err(e) => e,
                };
                child.terminate().await;
                Err(e)
            }
        }
    }

    /// 2026-10-24: Write the request, read the two response frames, under the deadlines.
    async fn exchange(
        &self,
        child: &mut MetChild,
        request: &[u8],
        e: &Expect<'_>,
    ) -> core::result::Result<Vec<Vec<f32>>, Stop> {
        let start = Instant::now();
        let req_ms = self.s.request_deadline_ms;
        let req_end = start + Duration::from_millis(req_ms);
        let boot_end = child.spawned + Duration::from_millis(self.boot_deadline_ms);
        let (head_end, head_stop) = if child.fresh && boot_end < req_end {
            (boot_end, Stop::Deadline("boot", self.boot_deadline_ms))
        } else {
            (req_end, Stop::Deadline("request", req_ms))
        };
        let limits = Limits::of(self.s.max_batch, self.s.dimensions);
        let mut reader = FrameReader::default();
        let head = async {
            let Some(stdin) = child.stdin.as_mut() else {
                return Err(Stop::Ended("request", FrameReader::default()));
            };
            if stdin.write_all(request).await.is_err() || stdin.flush().await.is_err() {
                return Err(Stop::Ended("request", FrameReader::default()));
            }
            read_frame(child, &mut reader, "response header", limits.header_max).await
        };
        let header = match timeout_at(head_end, head).await {
            Ok(r) => r?,
            Err(_) => return Err(head_stop),
        };
        check_header(&header, e)?;
        let body = read_frame(child, &mut reader, "response vectors", limits.vectors_max);
        let vectors = match timeout_at(req_end, body).await {
            Ok(r) => r?,
            Err(_) => return Err(Stop::Deadline("request", req_ms)),
        };
        if !reader.is_empty() {
            return Err(Stop::Err(error!(
                ErrorCode::LatlasEmbedFrame,
                "response", "bytes follow the vectors frame"
            )));
        }
        Ok(decode_vectors(
            &vectors,
            "response vectors",
            e.count,
            e.dim,
        )?)
    }
}

/// 2026-10-24: Read stdout until `reader` holds frame `which` (at most `max` bytes).
async fn read_frame(
    child: &mut MetChild,
    reader: &mut FrameReader,
    which: &'static str,
    max: u64,
) -> core::result::Result<Vec<u8>, Stop> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if let Some(f) = reader.take(which, max)? {
            return Ok(f);
        }
        match child.stdout.read(&mut buf).await {
            Ok(0) | Err(_) => return Err(Stop::Ended(which, core::mem::take(reader))),
            Ok(n) => reader.push(&buf[..n]),
        }
    }
}
