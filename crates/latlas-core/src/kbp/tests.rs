// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The BlueprintId over synthetic files of every codec, the lite/full invariance it
//! exists for, and each refusal (format, flags, codec, caps, tiling, sealed, raw hash, length,
//! kzx memory). The real-file cross-check against `met blueprint verify` is in
//! latlas-host-met's `tests/blueprint.rs`.

use super::*;

const MEM: DecodeMemory = DecodeMemory(1 << 30);

fn raw() -> Vec<u8> {
    (0..20_000u32)
        .flat_map(|i| format!("kernel {} ptx .reg .b32 %r{};\n", i % 97, i % 13).into_bytes())
        .collect()
}

struct File {
    flags: u8,
    codec: u8,
    stored: Vec<u8>,
    raw_len: u64,
    raw_sha: Sha256,
    payload: Vec<u8>,
    sealed: Vec<u8>,
    sig: Vec<u8>,
}

impl File {
    fn new(codec: u8, raw: &[u8]) -> Self {
        let stored = match codec {
            0 => raw.to_vec(),
            1 => miniz_oxide::deflate::compress_to_vec(raw, 9),
            _ => kzx::compress(raw),
        };
        Self {
            flags: 0,
            codec,
            stored,
            raw_len: u64::try_from(raw.len()).unwrap(),
            raw_sha: sha256(raw),
            payload: Vec::new(),
            sealed: Vec::new(),
            sig: b"signature".to_vec(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let n = |v: &Vec<u8>| u64::try_from(v.len()).unwrap().to_le_bytes();
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&KBP_FORMAT.to_le_bytes());
        b.extend_from_slice(&[self.flags, self.codec]);
        b.extend_from_slice(&n(&self.stored));
        b.extend_from_slice(&self.raw_len.to_le_bytes());
        b.extend_from_slice(&self.raw_sha.0);
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&n(&self.payload));
        b.extend_from_slice(&n(&self.sig));
        b.extend_from_slice(&n(&self.sealed));
        assert_eq!(b.len(), HEADER_LEN);
        for s in [&self.stored, &self.payload, &self.sealed, &self.sig] {
            b.extend_from_slice(s);
        }
        b
    }
}

fn expected(raw: &[u8]) -> String {
    let mut m = b"kbp-body\0".to_vec();
    m.extend_from_slice(raw);
    sha256(&m).hex()
}

fn refusal(file: &[u8]) -> String {
    let e = file_id(file, MEM).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasBlueprint, "{e}");
    e.to_string()
}

#[test]
fn every_codec_gives_the_domain_tagged_hash_of_the_raw_body() {
    let r = raw();
    for codec in [0u8, 1, 2] {
        let f = File::new(codec, &r);
        let id = file_id(&f.bytes(), MEM).unwrap();
        assert_eq!(id.hex(), expected(&r), "codec {codec}");
        assert_ne!(id.0, sha256(&f.bytes()), "never the file hash");
        assert_ne!(id.0, f.raw_sha, "never the bare raw-body hash");
    }
}

#[test]
fn lite_and_full_of_one_body_share_the_id_and_differ_in_file_hash() {
    let r = raw();
    let lite = File::new(2, &r);
    let mut full = File::new(2, &r);
    full.flags = 1 | 2;
    full.payload = vec![7u8; 4096];
    let (l, f) = (lite.bytes(), full.bytes());
    assert_ne!(sha256(&l), sha256(&f));
    assert_eq!(file_id(&l, MEM).unwrap(), file_id(&f, MEM).unwrap());
}

#[test]
fn the_text_transport_gives_the_binary_files_id() {
    let f = File::new(2, &raw()).bytes();
    let mut text = b"kbp1.".to_vec();
    text.extend_from_slice(STANDARD.encode(&f).as_bytes());
    text.push(b'\n');
    assert_eq!(file_id(&text, MEM).unwrap(), file_id(&f, MEM).unwrap());
    assert!(refusal(b"kbp1.@@@@").contains("base64"));
    assert!(
        refusal(b" kbp1.AAAA").contains("truncated header"),
        "met reads it as binary"
    );
}

#[test]
fn a_raw_body_that_misses_the_header_hash_is_refused() {
    let mut f = File::new(2, &raw());
    f.raw_sha.0[0] ^= 1;
    assert!(refusal(&f.bytes()).contains("body_sha256"));
    let mut f = File::new(0, &raw());
    f.stored[10] ^= 1;
    assert!(refusal(&f.bytes()).contains("body_sha256"));
}

#[test]
fn header_fields_outside_the_format_are_refused() {
    let good = File::new(1, &raw()).bytes();
    let at = |i: usize, v: u8| {
        let mut b = good.clone();
        b[i] = v;
        refusal(&b)
    };
    assert!(at(0, b'X').contains("bad magic"));
    assert!(at(4, 2).contains("KBP_FORMAT 2"));
    assert!(at(6, 8).contains("unknown flag bits"));
    assert!(at(7, 3).contains("codec id 3"));
    assert!(at(6, FLAG_SEALED).contains("sealed flag"));
    assert!(refusal(&good[..HEADER_LEN - 1]).contains("truncated header"));
    assert!(refusal(&good[..good.len() - 1]).contains("truncated"));
    let mut long = good.clone();
    long.push(0);
    assert!(refusal(&long).contains("trailing bytes"));
}

#[test]
fn caps_are_refused_from_the_header_before_any_decoding() {
    let mut f = File::new(2, &raw());
    f.raw_len = MAX_BODY_RAW + 1;
    assert!(refusal(&f.bytes()).contains("raw body is"));
    let mut b = File::new(2, &raw()).bytes();
    b[8..16].copy_from_slice(&(MAX_BODY + 1).to_le_bytes());
    assert!(refusal(&b).contains("body is"));
    b = File::new(2, &raw()).bytes();
    b[80..88].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(refusal(&b).contains("sealed section is"));
}

#[test]
fn a_declared_raw_length_the_stream_does_not_yield_is_refused() {
    for codec in [0u8, 1, 2] {
        let mut f = File::new(codec, &raw());
        f.raw_len -= 1;
        let e = refusal(&f.bytes());
        assert!(
            e.contains("length") || e.contains("kzx refused"),
            "{codec}: {e}"
        );
    }
}

#[test]
fn a_kzx_block_over_the_memory_budget_is_refused() {
    let f = File::new(2, &raw()).bytes();
    let e = file_id(&f, DecodeMemory(1)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasBlueprint);
    assert!(e.to_string().contains("MemoryCapExceeded"), "{e}");
}

#[test]
fn body_id_refuses_a_stored_slice_of_another_length() {
    let b = File::new(2, &raw()).bytes();
    let h = Header::parse(&b, u64::try_from(b.len()).unwrap()).unwrap();
    let body = &b[HEADER_LEN..usize::try_from(h.body.end).unwrap()];
    assert!(body_id(&h, body, MEM).is_ok());
    let e = body_id(&h, &body[1..], MEM).unwrap_err();
    assert!(e.to_string().contains("stored bytes"), "{e}");
}
