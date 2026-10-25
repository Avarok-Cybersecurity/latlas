// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The alias child's environment: cleared, then only `CHILD_ENV`. Split from
//! `alias.rs` (the 500-line cap); it runs as that binary's `host_env` case.

use super::{Rig, block, texts};
use latlas_core::embed::{Embedder, Role};

/// 2026-10-25: Each spawn's environment variable names, as the fake recorded them.
fn envs(rig: &Rig) -> Vec<Vec<String>> {
    std::fs::read_to_string(format!("{}.env", rig.blueprint))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.split('\t')
                .filter(|k| !k.is_empty())
                .map(String::from)
                .collect()
        })
        .collect()
}

/// 2026-10-25: Set in the `env-probe` process's own environment by
/// [`the_host_env_never_reaches_met`]: the token name the MCP host really carries, and one
/// no allowlist could plausibly name.
const SENTINELS: [&str; 2] = [
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "LATLAS_TEST_PARENT_ONLY_SENTINEL",
];

/// 2026-10-25: The alias child gets a cleared environment plus `CHILD_ENV`, never the host's:
/// this binary re-runs itself as `env-probe` with the sentinels set (setting them here would need
/// `unsafe` `set_var`), and that process spawns the fake through the production host.
pub fn the_host_env_never_reaches_met() {
    let mut probe = std::process::Command::new(std::env::current_exe().unwrap());
    probe.arg("env-probe");
    for k in SENTINELS {
        probe.env(k, "parent-only-value");
    }
    let out = probe.output().unwrap();
    assert!(
        out.status.success(),
        "env-probe failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 2026-10-25: The `env-probe` process: one embed through the production host, then the
/// assertions on what the fake saw.
pub fn env_probe() {
    for k in SENTINELS {
        assert!(
            std::env::var_os(k).is_some(),
            "the probe's parent did not set {k}"
        );
    }
    let rig = Rig::new("env", "good");
    block(async {
        let h = rig.host(10_000, 10_000, 0).await;
        h.embed(Role::Document, &texts(1)).await.unwrap();
    });
    let envs = envs(&rig);
    assert_eq!(envs.len(), 1, "one spawn");
    let names = &envs[0];
    for k in SENTINELS {
        assert!(!names.iter().any(|n| n == k), "{k} reached met: {names:?}");
    }
    assert!(
        names.iter().any(|n| n == "PATH"),
        "PATH is passed: {names:?}"
    );
    for n in names {
        assert!(
            latlas_host_met::child::CHILD_ENV.contains(&n.as_str()),
            "{n} is outside CHILD_ENV"
        );
    }
}
