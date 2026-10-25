// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: One `met embed` child: spawned in its own process group with stdin and stdout
//! piped (the frames) and stderr copied line by line to the host's log, its last line kept for
//! the exit error (met prints its CERF code there).
//!
//! Owner: latlas-host-met.
//! Invariants:
//! - The child never outlives the host: dropping a [`MetChild`] SIGKILLs its process group
//!   (and tokio's `kill_on_drop` kills the leader), on every path including a panic.
//! - [`MetChild::terminate`] is SIGINT to the group, then SIGKILL after [`GRACE`], then reap.

use std::io::Write as _;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use latlas_core::metlocal::MetTrust;
use latlas_error::{Error, ErrorCode, error};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// 2026-10-24: SIGINT to SIGKILL (the design's 2 s), and the wait for a child to exit by itself
/// once its stdin is closed.
pub const GRACE: Duration = Duration::from_secs(2);

/// 2026-10-24: A running `met embed`.
pub struct MetChild {
    child: Child,
    pgid: Option<Pid>,
    pub stdin: Option<ChildStdin>,
    pub stdout: ChildStdout,
    last_line: Arc<Mutex<String>>,
    pump: Option<JoinHandle<()>>,
    /// 2026-10-24: When it was spawned (the boot deadline runs from here).
    pub spawned: Instant,
    /// 2026-10-24: No response has been read from it yet (the boot deadline applies).
    pub fresh: bool,
}

fn signal_group(pgid: Option<Pid>, sig: Signal) {
    if let Some(p) = pgid {
        // 2026-10-24: ESRCH (the group is already gone) is the expected failure; there is no
        // other recovery for a failed kill than the leader's own kill_on_drop.
        let _ = kill_process_group(p, sig);
    }
}

impl MetChild {
    /// 2026-10-24: Spawn `met_path embed --blueprint <b> --input-type <t> --idle-exit-ms <n>`,
    /// then (2026-10-25) `--trust-mode production` or `--trust-mode dev-anchor --trust-anchor <a>`.
    pub fn spawn(
        met_path: &str,
        blueprint: &str,
        input_type: &str,
        idle_exit_ms: u64,
        trust: &MetTrust,
        log: &Path,
    ) -> Result<Self, Error> {
        let spawn_err =
            |why: String| error!(ErrorCode::LatlasEmbedderSpawn, met_path.to_string(), why);
        let mut sink = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map_err(|e| spawn_err(format!("log {} ({:?})", log.display(), e.kind())))?;
        let mut cmd = Command::new(met_path);
        cmd.arg("embed")
            .arg("--blueprint")
            .arg(blueprint)
            .arg("--input-type")
            .arg(input_type)
            .arg("--idle-exit-ms")
            .arg(idle_exit_ms.to_string())
            .arg("--trust-mode");
        match trust {
            MetTrust::Production => cmd.arg("production"),
            MetTrust::DevAnchor { anchor } => {
                cmd.arg("dev-anchor").arg("--trust-anchor").arg(anchor)
            }
        };
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| spawn_err(format!("{:?}", e.kind())))?;
        let pgid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw);
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            signal_group(pgid, Signal::KILL);
            return Err(spawn_err("a pipe was not opened".into()));
        };
        let last_line = Arc::new(Mutex::new(String::new()));
        let last = Arc::clone(&last_line);
        let pump = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = writeln!(sink, "{l}");
                if !l.trim().is_empty() {
                    *last
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = l;
                }
            }
        });
        Ok(Self {
            child,
            pgid,
            stdin: Some(stdin),
            stdout,
            last_line,
            pump: Some(pump),
            spawned: Instant::now(),
            fresh: true,
        })
    }

    /// 2026-10-24: Whether it is still running.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// 2026-10-24: SIGINT the group, SIGKILL it after [`GRACE`], reap the leader.
    pub async fn terminate(&mut self) {
        signal_group(self.pgid, Signal::INT);
        if tokio::time::timeout(GRACE, self.child.wait())
            .await
            .is_err()
        {
            signal_group(self.pgid, Signal::KILL);
            let _ = self.child.wait().await;
        }
        signal_group(self.pgid, Signal::KILL);
    }

    /// 2026-10-24: Close stdin (met exits after its request), wait [`GRACE`] for the exit, then
    /// [`terminate`](Self::terminate) whatever is left.
    pub async fn finish(&mut self) {
        self.stdin = None;
        if tokio::time::timeout(GRACE, self.child.wait())
            .await
            .is_err()
        {
            self.terminate().await;
        }
        signal_group(self.pgid, Signal::KILL);
    }

    /// 2026-10-24: The exit status within `budget`, if the child exited.
    async fn exited(&mut self, budget: Duration) -> Option<ExitStatus> {
        tokio::time::timeout(budget, self.child.wait())
            .await
            .ok()
            .and_then(Result::ok)
    }

    /// 2026-10-24: After its stdout ended: `LatlasEmbedderExit` with the status and the last
    /// stderr line when it exited unsuccessfully within [`GRACE`], else `None` (the caller
    /// reports the truncated frame).
    pub async fn exit_error(&mut self) -> Option<Error> {
        let status = self.exited(GRACE).await?;
        if status.success() {
            return None;
        }
        if let Some(p) = self.pump.take() {
            let _ = tokio::time::timeout(GRACE, p).await;
        }
        let last = self
            .last_line
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Some(error!(
            ErrorCode::LatlasEmbedderExit,
            status.to_string(),
            last
        ))
    }
}

impl Drop for MetChild {
    fn drop(&mut self) {
        signal_group(self.pgid, Signal::KILL);
    }
}
