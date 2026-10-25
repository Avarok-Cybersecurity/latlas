// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: A minimal stored-codec `.kbp` for the host tests: the 88-byte header, the raw
//! body as its body section, a placeholder signature. Its BlueprintId is
//! `sha256("kbp-body\0" ‖ raw)`. The real-file cross-check is `tests/blueprint.rs`.

use latlas_core::hash::sha256;
use latlas_core::kbp::BlueprintId;

/// 2026-10-24: The file bytes of a stored-codec blueprint whose raw body is `raw`.
pub fn stored(raw: &[u8]) -> Vec<u8> {
    let sig = b"sig";
    let n = |l: usize| u64::try_from(l).unwrap().to_le_bytes();
    let mut b = b"MKBP\x01\x00\x00\x00".to_vec();
    b.extend_from_slice(&n(raw.len()));
    b.extend_from_slice(&n(raw.len()));
    b.extend_from_slice(&sha256(raw).0);
    b.extend_from_slice(&[0u8; 16]);
    b.extend_from_slice(&n(sig.len()));
    b.extend_from_slice(&[0u8; 8]);
    b.extend_from_slice(raw);
    b.extend_from_slice(sig);
    b
}

/// 2026-10-24: Write `stored(raw)` to `path` and return its BlueprintId.
pub fn write(path: &std::path::Path, raw: &[u8]) -> BlueprintId {
    std::fs::write(path, stored(raw)).unwrap();
    let mut m = b"kbp-body\0".to_vec();
    m.extend_from_slice(raw);
    BlueprintId(sha256(&m))
}
