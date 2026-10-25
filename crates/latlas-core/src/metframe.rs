// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The met-local wire codec (DESIGN-latlas-local-embedder.md §4), pure: the alias's
//! u32-LE length-prefixed frames (two per direction) and the resident's base64 JSON body.
//!
//! Request: frame 1 is the JSON header `{model, blueprint_sha256, input_type, count, encoding}`,
//! frame 2 the exact `/v1/embeddings` body ([`crate::embed::request_body`]). Response: frame 1 is
//! the OpenAI response without the `embedding` arrays (`object`, `data[{index, object}]`, `model`,
//! `usage`, the `prefix` met applied and the `blueprint_sha256`), frame 2 the raw little-endian
//! f32 `[count][dim]` in request order.
//!
//! Owner: latlas-core.
//! Invariants:
//! - A frame longer than its declared [`Limits`] is refused from its length prefix alone, before
//!   its bytes are buffered.
//! - Vectors are bit-exact: `len == count * dim * 4`, else `LatlasEmbedDim` when the length is a
//!   whole number of vectors of another width, else `LatlasEmbedFrame`.
//! - An echo of another blueprint, model or prefix than the request's is `LatlasEmbedSpace`. The
//!   wire field is met's `blueprint_sha256`; its value is the BlueprintId ([`crate::kbp`]).

use crate::embed::{canon_body, request_body, request_object};
use crate::kbp::BlueprintId;
use crate::prelude::*;
use base64::Engine as _;
use latlas_error::{Error, ErrorCode, Result, error};
use serde_json::Value;

/// 2026-10-24: The vector encoding the alias request names (`frame 2` of the response).
pub const ENCODING: &str = "f32le";

/// 2026-10-24: The response header's size bound: a fixed part (object, model, usage, prefix,
/// sha: a few hundred bytes in practice, bounded at 64 KiB) plus one `data` item per input (an
/// `{"index":N,"object":"embedding"}` is under 48 bytes, bounded at 256).
pub const HEADER_FIXED_BYTES: u64 = 64 * 1024;
pub const HEADER_PER_ITEM_BYTES: u64 = 256;

/// 2026-10-24: The longest response frames accepted, derived from the config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub header_max: u64,
    pub vectors_max: u64,
}

impl Limits {
    /// 2026-10-24: `header = fixed + per_item * max_batch`, `vectors = max_batch * dim * 4`.
    pub fn of(max_batch: u32, dim: u32) -> Self {
        let b = u64::from(max_batch);
        Self {
            header_max: HEADER_FIXED_BYTES + HEADER_PER_ITEM_BYTES * b,
            vectors_max: b * u64::from(dim) * 4,
        }
    }
}

/// 2026-10-24: What one request expects its answer to echo.
#[derive(Clone, Copy, Debug)]
pub struct Expect<'a> {
    pub model: &'a str,
    pub blueprint: &'a BlueprintId,
    pub input_type: &'a str,
    pub count: usize,
    pub dim: u32,
}

/// 2026-10-24: The prefix met applies for `input_type` (the `"<input_type>: "` convention of the
/// asymmetric models the `input_type` names, e.g. `passage: `), which the answer must echo.
pub fn expected_prefix(input_type: &str) -> String {
    format!("{input_type}: ")
}

fn frame_err(which: &str, why: impl Into<String>) -> Error {
    error!(ErrorCode::LatlasEmbedFrame, which.to_string(), why.into())
}

fn push_frame(out: &mut Vec<u8>, which: &str, payload: &[u8]) -> Result<()> {
    let n = u32::try_from(payload.len())
        .map_err(|_| frame_err(which, format!("{} bytes exceed u32", payload.len())))?;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

/// 2026-10-24: The two request frames for `inputs`, all of one `input_type`.
pub fn encode_request(
    model: &str,
    blueprint: &BlueprintId,
    input_type: &str,
    inputs: &[String],
) -> Result<Vec<u8>> {
    let mut h = serde_json::Map::new();
    h.insert("model".into(), Value::String(model.to_string()));
    h.insert("blueprint_sha256".into(), Value::String(blueprint.hex()));
    h.insert("input_type".into(), Value::String(input_type.to_string()));
    h.insert("count".into(), Value::from(inputs.len()));
    h.insert("encoding".into(), Value::String(ENCODING.into()));
    let body = request_body(model, input_type, inputs);
    let mut out = Vec::new();
    push_frame(&mut out, "request header", canon_body(h).as_bytes())?;
    push_frame(&mut out, "request body", body.as_bytes())?;
    Ok(out)
}

/// 2026-10-24: Reassembles length-prefixed frames from a byte stream read in arbitrary pieces.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    /// 2026-10-24: Append bytes read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    fn declared(&self) -> Option<u64> {
        let p: [u8; 4] = self.buf.get(..4)?.try_into().ok()?;
        Some(u64::from(u32::from_le_bytes(p)))
    }

    /// 2026-10-24: The next whole frame, `None` while it is incomplete; a declared length over
    /// `max` is refused as soon as the prefix is read.
    pub fn take(&mut self, which: &str, max: u64) -> Result<Option<Vec<u8>>> {
        let Some(n) = self.declared() else {
            return Ok(None);
        };
        if n > max {
            return Err(frame_err(
                which,
                format!("declares {n} bytes; the limit is {max}"),
            ));
        }
        let end = usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_add(4))
            .ok_or_else(|| frame_err(which, format!("{n} bytes do not fit in memory")))?;
        if self.buf.len() < end {
            return Ok(None);
        }
        let rest = self.buf.split_off(end);
        let mut frame = core::mem::replace(&mut self.buf, rest);
        frame.drain(..4);
        Ok(Some(frame))
    }

    /// 2026-10-24: Whether no byte is buffered.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 2026-10-24: The error for a stream that ended inside (or before) frame `which`.
    pub fn truncated(&self, which: &str) -> Error {
        let why = match self.declared() {
            None => format!("truncated stream: {} of 4 length bytes", self.buf.len()),
            Some(n) => format!(
                "truncated stream: {} of {n} bytes",
                self.buf.len().saturating_sub(4)
            ),
        };
        frame_err(which, why)
    }
}

fn echo<'v>(v: &'v Value, which: &str, key: &str) -> Result<&'v str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| frame_err(which, format!("no `{key}` string")))
}

fn space_err(what: &str, got: &str, want: &str) -> Error {
    error!(
        ErrorCode::LatlasEmbedSpace,
        format!("{what} {got:?}"),
        format!("{what} {want:?}")
    )
}

/// 2026-10-24: The echoed BlueprintId (`blueprint_sha256`), model and prefix against the request's.
fn check_echo(v: &Value, which: &str, e: &Expect<'_>) -> Result<()> {
    let sha = echo(v, which, "blueprint_sha256")?;
    let want = e.blueprint.hex();
    if sha != want {
        return Err(space_err("blueprint_sha256", sha, &want));
    }
    let model = echo(v, which, "model")?;
    if model != e.model {
        return Err(space_err("model", model, e.model));
    }
    let prefix = echo(v, which, "prefix")?;
    let want = expected_prefix(e.input_type);
    if prefix != want {
        return Err(space_err("prefix", prefix, &want));
    }
    Ok(())
}

fn data<'v>(v: &'v Value, which: &str, count: usize) -> Result<&'v Vec<Value>> {
    let d = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| frame_err(which, "no `data` array"))?;
    if d.len() != count {
        return Err(frame_err(
            which,
            format!("{} data items for {count} inputs", d.len()),
        ));
    }
    Ok(d)
}

fn index_of(item: &Value) -> Option<usize> {
    item.get("index")
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
}

/// 2026-10-24: Check response frame 1 (the header): JSON, the echoes, and `data[i].index == i`
/// (the vectors frame is in request order).
pub fn check_header(bytes: &[u8], e: &Expect<'_>) -> Result<()> {
    const W: &str = "response header";
    let v: Value = serde_json::from_slice(bytes).map_err(|x| frame_err(W, x.to_string()))?;
    check_echo(&v, W, e)?;
    for (i, item) in data(&v, W, e.count)?.iter().enumerate() {
        if index_of(item) != Some(i) {
            return Err(frame_err(W, format!("data[{i}] is not index {i}")));
        }
    }
    Ok(())
}

/// 2026-10-24: `count` vectors of `dim` little-endian f32 from `bytes`, bit-exact.
pub fn decode_vectors(bytes: &[u8], which: &str, count: usize, dim: u32) -> Result<Vec<Vec<f32>>> {
    let width = usize::try_from(dim).map_err(|_| frame_err(which, "dim does not fit usize"))?;
    let want = count
        .checked_mul(width)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| frame_err(which, "count x dim overflows"))?;
    if bytes.len() != want {
        let per = count.saturating_mul(4);
        if count > 0 && bytes.len().is_multiple_of(per) {
            return Err(error!(ErrorCode::LatlasEmbedDim, bytes.len() / per, dim));
        }
        return Err(frame_err(
            which,
            format!("{} bytes are not {count} x {dim} x 4 = {want}", bytes.len()),
        ));
    }
    if want == 0 {
        return Ok(vec![Vec::new(); count]);
    }
    Ok(bytes
        .chunks_exact(width * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        })
        .collect())
}

/// 2026-10-24: The resident request body: [`request_body`]'s fields plus
/// `encoding_format: "base64"` (the bit-exact option).
pub fn resident_body(model: &str, input_type: &str, inputs: &[String]) -> String {
    let mut o = request_object(model, input_type, inputs);
    o.insert("encoding_format".into(), Value::String("base64".into()));
    canon_body(o)
}

/// 2026-10-24: Parse a resident response: the echoes as the alias header, then each
/// `data[].embedding` (base64 of little-endian f32) placed by its `index`.
pub fn parse_resident(body: &str, e: &Expect<'_>) -> Result<Vec<Vec<f32>>> {
    const W: &str = "resident body";
    let v: Value = serde_json::from_str(body).map_err(|x| frame_err(W, x.to_string()))?;
    check_echo(&v, W, e)?;
    let mut out: Vec<Option<Vec<f32>>> = vec![None; e.count];
    for item in data(&v, W, e.count)? {
        let i = index_of(item)
            .filter(|i| *i < e.count)
            .ok_or_else(|| frame_err(W, "an item has no index in range"))?;
        let b64 = echo(item, W, "embedding")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|x| frame_err(W, format!("embedding {i}: {x}")))?;
        let mut one = decode_vectors(&bytes, W, 1, e.dim)?;
        if out[i].replace(one.remove(0)).is_some() {
            return Err(frame_err(W, format!("index {i} answered twice")));
        }
    }
    Ok(out.into_iter().flatten().collect())
}

#[cfg(test)]
#[path = "metframe/tests.rs"]
mod tests;
