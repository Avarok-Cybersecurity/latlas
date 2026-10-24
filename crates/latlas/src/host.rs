// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The declarative pack's process host: it lists the Rust workspace's member
//! manifests and `.rs` files (the pure adapter cannot list a directory) and records nothing (the
//! declarative pack declares no record kinds).
//!
//! Owner: latlas (the binary).
//! Invariants: a member glob is literal or ends in `/*`; a missing member directory is an error.

use latlas_core::{Bundle, Config};
use latlas_decl::{Decl, DeclSources};
use latlas_error::{ErrorCode, error};
use latlas_host_native::fs::{FsSources, Stamp, source_err};
use serde_json::Value;
use std::path::Path;

/// 2026-10-24: The host.
pub struct DeclHost;

fn rs_under(fs: &FsSources, dir: &str, out: &mut Vec<String>) -> anyhow::Result<()> {
    let abs = fs.abs(dir);
    if !abs.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&abs)
        .map_err(|e| source_err(dir, &e))?
        .collect::<Result<_, _>>()
        .map_err(|e| source_err(dir, &e))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = format!("{dir}/{name}");
        if e.path().is_dir() {
            rs_under(fs, &rel, out)?;
        } else if name.ends_with(".rs") {
            out.push(rel);
        }
    }
    Ok(())
}

fn expand(fs: &FsSources, member: &str) -> anyhow::Result<Vec<String>> {
    let Some(parent) = member.strip_suffix("/*") else {
        return Ok(vec![member.to_string()]);
    };
    let mut out: Vec<String> = std::fs::read_dir(fs.abs(parent))
        .map_err(|e| source_err(parent, &e))?
        .filter_map(Result::ok)
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| format!("{parent}/{}", e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    Ok(out)
}

impl latlas_cli::host::PackHost<Decl> for DeclHost {
    fn open(_: &Config<DeclSources>, _: &FsSources) -> anyhow::Result<Self> {
        Ok(DeclHost)
    }

    fn extra_files(&self, cfg: &Config<DeclSources>, fs: &FsSources) -> anyhow::Result<Vec<String>> {
        let Some(r) = &cfg.sources.rust else {
            return Ok(Vec::new());
        };
        let text = std::fs::read_to_string(fs.abs(&r.manifest)).map_err(|e| source_err(&r.manifest, &e))?;
        let mut out = Vec::new();
        for m in latlas_decl::rust::members(&text) {
            for dir in expand(fs, &m)? {
                if !Path::new(&fs.abs(&dir)).join("Cargo.toml").is_file() {
                    return Err(error!(ErrorCode::LatlasSource, dir, "a workspace member without Cargo.toml").into());
                }
                out.push(format!("{dir}/Cargo.toml"));
                rs_under(fs, &format!("{dir}/src"), &mut out)?;
                rs_under(fs, &format!("{dir}/tests"), &mut out)?;
            }
        }
        Ok(out)
    }

    fn extra_stamp(&self, _: &Config<DeclSources>, _: &FsSources) -> anyhow::Result<Stamp> {
        Ok(Vec::new())
    }

    fn facts(&self, _: &Config<DeclSources>, _: &FsSources) -> anyhow::Result<(Option<()>, Option<String>)> {
        Ok((None, None))
    }

    async fn record(
        &self,
        _: &Config<DeclSources>,
        _: &FsSources,
        _: &Bundle,
        _: &Value,
    ) -> latlas_error::Result<(Value, bool)> {
        Err(error!(ErrorCode::LatlasToolArgument, "kind", "the declarative pack declares no record kinds"))
    }
}
