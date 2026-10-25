// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The BlueprintId of a met `.kbp` blueprint, derived exactly as met derives it
//! (metrale-blueprint `frame::header`, `codec`, `transport`, `ids::BlueprintId::of_body`), so the
//! met-local host can pin a blueprint by the id `met blueprint verify` prints and `met embed` /
//! `met serve` echo as `blueprint_sha256`.
//!
//! ```text
//! [0..4]   magic "MKBP"   [4..6] KBP_FORMAT u16
//! [6]      flags u8 (bit0 full, bit1 solid payload, bit2 sealed)   [7] body codec id u8
//! [8..16]  body_len (stored)        [16..24] body_raw_len      [24..56] body_sha256 (raw)
//! [56..64] locator_len    [64..72] payload_len    [72..80] sig_len    [80..88] sealed_len
//! then body, locator, payload, sealed, sig
//! ```
//!
//! BlueprintId = `sha256("kbp-body\0" ‖ raw body)`, the raw body being the body section after
//! decompression. A lite and a full file of one build share it (only the payload, the flags and
//! the codec differ, none of which it covers); their file sha256 differ, so a file hash is never
//! the pin.
//!
//! Owner: latlas-core. Reimplemented rather than depending on metrale-blueprint, whose crate
//! pulls in the circuit, settings and seal crates (DESIGN-latlas-local-embedder.md §4).
//! Invariants:
//! - Little-endian; every length is checked against met's cap before anything is sliced,
//!   allocated or decoded, and the sections must tile the file exactly.
//! - Unknown format, flag bits or codec are refused, as is a sealed flag without a sealed
//!   section (or the reverse).
//! - Decompression is bounded by the declared raw length (itself capped) and, for kzx, by the
//!   caller's [`DecodeMemory`]; output of another length is refused.
//! - The raw body must hash to the header's `body_sha256`, else no id is returned.
//! - Every refusal is `LatlasBlueprint`; no file content is in it.

use crate::hash::{Sha256, sha256, sha256_cat};
use crate::prelude::*;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use core::ops::Range;
use latlas_error::{Error, ErrorCode, Result, error};

/// 2026-10-24: The file magic.
pub const MAGIC: &[u8; 4] = b"MKBP";
/// 2026-10-24: The framing version met writes and reads.
pub const KBP_FORMAT: u16 = 1;
/// 2026-10-24: The header length.
pub const HEADER_LEN: usize = 88;
const HEADER_LEN_U64: u64 = HEADER_LEN as u64;
/// 2026-10-24: Every flag bit met defines (full, solid payload, sealed).
pub const KNOWN_FLAGS: u8 = 1 | 2 | 4;
/// 2026-10-24: The sealed flag.
pub const FLAG_SEALED: u8 = 4;
/// 2026-10-24: met's section caps.
pub const MAX_BODY: u64 = 4 << 20;
pub const MAX_BODY_RAW: u64 = 64 << 20;
pub const MAX_LOCATOR: u64 = 16 << 20;
pub const MAX_PAYLOAD: u64 = 1 << 30;
pub const MAX_SIG: u64 = 256 << 10;
pub const MAX_SEALED: u64 = 1 << 20;
/// 2026-10-24: The `kbp1.` text transport's prefix; met reads a file that starts with it as
/// base64 text (standard padded alphabet, surrounding whitespace ignored).
pub const TEXT_PREFIX: &[u8] = b"kbp1.";
/// 2026-10-24: The domain tag of the BlueprintId.
pub const BODY_DOMAIN: &[u8] = b"kbp-body\0";

/// 2026-10-24: The longest binary file met's text transport decodes to.
pub const MAX_TEXT_FILE: u64 = HEADER_LEN_U64 + MAX_BODY + MAX_LOCATOR + MAX_PAYLOAD + MAX_SIG;

/// 2026-10-24: The longest `kbp1.` text met accepts (its base64 length check), plus room for
/// surrounding whitespace a host may read before trimming.
pub const MAX_TEXT_LEN: u64 = MAX_TEXT_FILE / 3 * 4 + 4096;

/// 2026-10-24: A blueprint's identity: lowercase hex of `sha256("kbp-body\0" ‖ raw body)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlueprintId(pub Sha256);

impl BlueprintId {
    /// 2026-10-24: The id of a raw (decompressed) body.
    pub fn of_body(raw: &[u8]) -> Self {
        Self(sha256_cat(&[BODY_DOMAIN, raw]))
    }

    /// 2026-10-24: The 64-digit lowercase hex form (what met prints and echoes).
    pub fn hex(&self) -> String {
        self.0.hex()
    }
}

/// 2026-10-24: The most model memory one kzx block may use while it decodes; chosen by the host
/// (PCND: no default). Stored and deflate bodies need none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeMemory(pub u64);

/// 2026-10-24: How the body section is stored (met's codec ids, append-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Stored,
    Deflate,
    Kzx,
}

impl Codec {
    /// 2026-10-24: The codec of a wire id.
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            0 => Ok(Self::Stored),
            1 => Ok(Self::Deflate),
            2 => Ok(Self::Kzx),
            _ => Err(refuse("header", format!("unknown body codec id {id}"))),
        }
    }
}

fn refuse(what: &'static str, why: impl Into<String>) -> Error {
    error!(ErrorCode::LatlasBlueprint, what, why.into())
}

/// 2026-10-24: What the BlueprintId needs from the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub codec: Codec,
    pub body_raw_len: u64,
    pub body_sha256: Sha256,
    /// 2026-10-24: The body section's byte range in the file.
    pub body: Range<u64>,
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn cap(section: &str, len: u64, max: u64) -> Result<()> {
    if len > max {
        return Err(refuse(
            "header",
            format!("{section} is {len} bytes, over the {max}-byte cap"),
        ));
    }
    Ok(())
}

impl Header {
    /// 2026-10-24: Parse the header at the start of `head` (the file's first bytes, at least
    /// [`HEADER_LEN`]) of a file `file_len` bytes long; the sections must tile the file.
    pub fn parse(head: &[u8], file_len: u64) -> Result<Self> {
        if head.len() < HEADER_LEN {
            return Err(refuse("header", "truncated header"));
        }
        if &head[0..4] != MAGIC {
            return Err(refuse("header", "bad magic"));
        }
        let format = u16::from_le_bytes([head[4], head[5]]);
        if format != KBP_FORMAT {
            return Err(refuse(
                "header",
                format!("KBP_FORMAT {format}; this host reads {KBP_FORMAT}"),
            ));
        }
        let flags = head[6];
        if flags & !KNOWN_FLAGS != 0 {
            return Err(refuse("header", "unknown flag bits"));
        }
        let codec = Codec::from_id(head[7])?;
        let lens = [
            ("body", u64_at(head, 8), MAX_BODY),
            ("locator", u64_at(head, 56), MAX_LOCATOR),
            ("payload", u64_at(head, 64), MAX_PAYLOAD),
            ("sealed section", u64_at(head, 80), MAX_SEALED),
            ("signature", u64_at(head, 72), MAX_SIG),
        ];
        let body_raw_len = u64_at(head, 16);
        cap("raw body", body_raw_len, MAX_BODY_RAW)?;
        for (section, len, max) in lens {
            cap(section, len, max)?;
        }
        if (flags & FLAG_SEALED != 0) != (lens[3].1 != 0) {
            return Err(refuse(
                "header",
                "the sealed flag disagrees with the sealed section",
            ));
        }
        let total = lens
            .iter()
            .try_fold(HEADER_LEN_U64, |a, s| a.checked_add(s.1))
            .ok_or_else(|| refuse("header", "section lengths overflow"))?;
        if total != file_len {
            return Err(refuse(
                "header",
                if total > file_len {
                    "truncated: the sections extend past the end"
                } else {
                    "trailing bytes after the sections"
                },
            ));
        }
        let mut body_sha256 = [0u8; 32];
        body_sha256.copy_from_slice(&head[24..56]);
        Ok(Self {
            codec,
            body_raw_len,
            body_sha256: Sha256(body_sha256),
            body: HEADER_LEN_U64..HEADER_LEN_U64 + lens[0].1,
        })
    }
}

fn decompress(h: &Header, stored: &[u8], memory: DecodeMemory) -> Result<Vec<u8>> {
    let limit = usize::try_from(h.body_raw_len)
        .map_err(|_| refuse("body", "the raw length does not fit in memory"))?;
    let raw = match h.codec {
        Codec::Stored => stored.to_vec(),
        Codec::Deflate => miniz_oxide::inflate::decompress_to_vec_with_limit(stored, limit)
            .map_err(|_| {
                refuse(
                    "body",
                    "deflate stream is corrupt or longer than its declared length",
                )
            })?,
        Codec::Kzx => {
            let limits = kzx::Limits {
                max_output: h.body_raw_len,
                max_memory: memory.0,
            };
            kzx::decompress(stored, &limits)
                .map_err(|e| refuse("body", format!("kzx refused it ({e:?})")))?
        }
    };
    if raw.len() != limit {
        return Err(refuse(
            "body",
            "the decoded length differs from the declared raw length",
        ));
    }
    Ok(raw)
}

/// 2026-10-24: The BlueprintId from a parsed header and its stored body section: decode it
/// (bounded), check the raw body against the header's `body_sha256`, then hash it with the
/// domain tag.
pub fn body_id(h: &Header, stored: &[u8], memory: DecodeMemory) -> Result<BlueprintId> {
    let want = h.body.end - h.body.start;
    if u64::try_from(stored.len()).ok() != Some(want) {
        return Err(refuse(
            "body",
            format!("{} stored bytes; the header declares {want}", stored.len()),
        ));
    }
    let raw = decompress(h, stored, memory)?;
    if sha256(&raw) != h.body_sha256 {
        return Err(refuse(
            "body",
            "the raw body does not hash to the header's body_sha256",
        ));
    }
    Ok(BlueprintId::of_body(&raw))
}

/// 2026-10-24: Whether a file that starts with `prefix` is the `kbp1.` text form (met's own
/// test: the raw bytes start with the prefix, no whitespace skipped).
pub fn is_text(prefix: &[u8]) -> bool {
    prefix.starts_with(TEXT_PREFIX)
}

/// 2026-10-24: The binary file of a `kbp1.` text (surrounding whitespace ignored).
pub fn from_text(text: &[u8]) -> Result<Vec<u8>> {
    let t = text.trim_ascii();
    let b64 = t
        .strip_prefix(TEXT_PREFIX)
        .ok_or_else(|| refuse("transport text", "does not start with kbp1."))?;
    let len = u64::try_from(b64.len()).unwrap_or(u64::MAX);
    if len / 4 * 3 > MAX_TEXT_FILE {
        return Err(refuse(
            "transport text",
            format!("{len} bytes, over the {}-byte cap", MAX_TEXT_FILE / 3 * 4),
        ));
    }
    STANDARD
        .decode(b64)
        .map_err(|_| refuse("transport text", "is not standard padded base64"))
}

/// 2026-10-24: The BlueprintId of a whole file held in memory, binary or `kbp1.` text.
pub fn file_id(file: &[u8], memory: DecodeMemory) -> Result<BlueprintId> {
    if is_text(file) {
        return binary_id(&from_text(file)?, memory);
    }
    binary_id(file, memory)
}

fn binary_id(file: &[u8], memory: DecodeMemory) -> Result<BlueprintId> {
    let len = u64::try_from(file.len()).map_err(|_| refuse("header", "file length"))?;
    let h = Header::parse(file, len)?;
    let body = usize::try_from(h.body.start)
        .ok()
        .zip(usize::try_from(h.body.end).ok());
    let (start, end) = body.ok_or_else(|| refuse("body", "section offset"))?;
    body_id(&h, &file[start..end], memory)
}

#[cfg(test)]
#[path = "kbp/tests.rs"]
mod tests;
