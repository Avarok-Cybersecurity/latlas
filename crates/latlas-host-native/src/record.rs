// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The native host of `la_record`'s file port: [`FsRecord`] reads, and replaces
//! atomically through [`crate::write::replace`].
//!
//! Owner: latlas-host-native (cut from metrale-io-host-kg, 2026-10-23 K0).
//! Invariants:
//! - Paths are root-relative; one that is absolute or climbs out of the root is refused.

use std::path::{Path, PathBuf};

use latlas_core::record::RecordFs;
use latlas_error::{ErrorCode, Result, error};

/// 2026-10-23: The file port over `root`.
pub struct FsRecord {
    pub root: PathBuf,
}

impl FsRecord {
    fn abs(&self, rel: &str) -> Result<PathBuf> {
        let p = Path::new(rel);
        if p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(error!(
                ErrorCode::LatlasRecordWrite,
                rel.to_string(),
                "must be root-relative without .."
            ));
        }
        Ok(self.root.join(p))
    }
}

impl RecordFs for FsRecord {
    async fn read(&self, path: &str) -> Result<Option<String>> {
        match std::fs::read_to_string(self.abs(path)?) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error!(
                ErrorCode::LatlasSource,
                path.to_string(),
                e.to_string()
            )),
        }
    }

    async fn replace(&self, path: &str, expected: &str, new: &str) -> Result<()> {
        crate::write::replace(&self.abs(path)?, expected, new)
    }
}
