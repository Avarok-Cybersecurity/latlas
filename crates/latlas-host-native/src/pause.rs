// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: The native [`PauseSignal`]: a file named `latlas_core::pause::PAUSE_FILE` in the
//! directory of the running executable. A symlink counts by its target, so a dangling one is
//! absent.
//!
//! Owner: latlas-host-native.
//! Invariants:
//! - The path is resolved once, at [`LockFile::beside_current_exe`] (ROTI).
//! - Absent means `NotFound` or `NotADirectory`; any other stat failure reads as present (fail
//!   closed: an unknown state holds no embedder).

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use latlas_core::pause::PAUSE_FILE;
use latlas_error::{ErrorCode, Result, error};
use latlas_ports::PauseSignal;

/// 2026-10-25: The pause file a host polls.
#[derive(Clone, Debug)]
pub struct LockFile {
    path: PathBuf,
}

impl LockFile {
    /// 2026-10-25: [`PAUSE_FILE`] beside `std::env::current_exe()`.
    pub fn beside_current_exe() -> Result<Self> {
        let exe = std::env::current_exe()
            .map_err(|e| error!(ErrorCode::Io, format!("current_exe ({:?})", e.kind())))?;
        let dir = exe
            .parent()
            .ok_or_else(|| error!(ErrorCode::Io, format!("{} has no directory", exe.display())))?;
        Ok(Self::at(dir.join(PAUSE_FILE)))
    }

    /// 2026-10-25: The pause file at `path`.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// 2026-10-25: Where it looks.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl PauseSignal for LockFile {
    fn present(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(_) => true,
            Err(e) => !matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LockFile;
    use latlas_ports::PauseSignal;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_file_or_a_live_symlink_is_present_and_a_dangling_symlink_or_nothing_is_absent() {
        let dir = std::env::temp_dir().join(format!("latlas-pause-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lock = LockFile::at(dir.join("latlas.lock"));
        assert!(!lock.present(), "nothing there");
        std::fs::write(lock.path(), b"").unwrap();
        assert!(lock.present(), "a plain file");
        std::fs::remove_file(lock.path()).unwrap();
        let target = dir.join("held");
        symlink(&target, lock.path()).unwrap();
        assert!(!lock.present(), "a dangling symlink is absent");
        std::fs::write(&target, b"").unwrap();
        assert!(lock.present(), "a symlink to a file that exists");
        std::fs::remove_file(&target).unwrap();
        assert!(!lock.present(), "dangling again once the target goes");
        assert!(
            !LockFile::at(dir.join("held/latlas.lock")).present(),
            "a non-directory parent"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
