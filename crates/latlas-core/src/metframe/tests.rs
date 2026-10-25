// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The met-local codec. Frames are built by hand here (not with the encoder) so a
//! symmetric bug in encoder and decoder cannot pass.

use super::*;
use crate::hash::sha256;

fn sha() -> BlueprintId {
    BlueprintId(sha256(b"blueprint"))
}

fn expect(s: &BlueprintId, count: usize, dim: u32) -> Expect<'_> {
    Expect {
        model: "m",
        blueprint: s,
        input_type: "passage",
        count,
        dim,
    }
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = u32::try_from(payload.len()).unwrap().to_le_bytes().to_vec();
    v.extend_from_slice(payload);
    v
}

fn header(s: &BlueprintId, model: &str, prefix: &str, count: usize) -> Vec<u8> {
    let data: Vec<Value> = (0..count)
        .map(|i| serde_json::json!({"index": i, "object": "embedding"}))
        .collect();
    serde_json::json!({
        "object": "list", "data": data, "model": model, "usage": {"prompt_tokens": 3},
        "prefix": prefix, "blueprint_sha256": s.hex(),
    })
    .to_string()
    .into_bytes()
}

fn le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn the_request_is_two_length_prefixed_frames_with_the_exact_body() {
    let s = sha();
    let inputs = vec!["a".to_string(), "b\u{e9}".to_string()];
    let bytes = encode_request("m", &s, "passage", &inputs).unwrap();
    let n1 = usize::try_from(u32::from_le_bytes(bytes[..4].try_into().unwrap())).unwrap();
    let h: Value = serde_json::from_slice(&bytes[4..4 + n1]).unwrap();
    assert_eq!(
        h,
        serde_json::json!({"model": "m", "blueprint_sha256": s.hex(), "input_type": "passage",
            "count": 2, "encoding": "f32le"})
    );
    let rest = &bytes[4 + n1..];
    let n2 = usize::try_from(u32::from_le_bytes(rest[..4].try_into().unwrap())).unwrap();
    assert_eq!(rest.len(), 4 + n2, "exactly two frames");
    assert_eq!(
        core::str::from_utf8(&rest[4..]).unwrap(),
        crate::embed::request_body("m", "passage", &inputs)
    );
}

#[test]
fn the_reader_reassembles_frames_split_anywhere() {
    let mut stream = frame(b"hello");
    stream.extend(frame(b""));
    stream.extend(frame(b"xyz"));
    let mut r = FrameReader::default();
    let mut got = Vec::new();
    for b in &stream {
        r.push(core::slice::from_ref(b));
        while let Some(f) = r.take("f", 16).unwrap() {
            got.push(f);
        }
    }
    assert_eq!(got, vec![b"hello".to_vec(), Vec::new(), b"xyz".to_vec()]);
    assert!(r.is_empty());
}

#[test]
fn an_oversized_frame_is_refused_from_its_prefix_alone() {
    let mut r = FrameReader::default();
    r.push(&u32::MAX.to_le_bytes());
    let e = r.take("response vectors", 1024).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedFrame);
    assert!(e.to_string().contains("4294967295"), "{e}");
    let mut r = FrameReader::default();
    r.push(&frame(&[0u8; 17]));
    assert!(r.take("f", 16).is_err(), "one past the limit");
    let mut r = FrameReader::default();
    r.push(&frame(&[0u8; 16]));
    assert_eq!(r.take("f", 16).unwrap().unwrap().len(), 16, "at the limit");
}

#[test]
fn a_truncated_stream_names_how_far_it_got() {
    let mut r = FrameReader::default();
    r.push(&[5, 0]);
    assert_eq!(r.take("h", 64).unwrap(), None);
    let e = r.truncated("h");
    assert_eq!(e.code(), ErrorCode::LatlasEmbedFrame);
    assert!(e.to_string().contains("2 of 4 length bytes"), "{e}");
    let mut r = FrameReader::default();
    r.push(&frame(b"hello")[..7]);
    assert_eq!(r.take("h", 64).unwrap(), None);
    assert!(r.truncated("h").to_string().contains("3 of 5 bytes"));
}

#[test]
fn the_header_checks_its_echoes_with_the_space_code() {
    let s = sha();
    check_header(&header(&s, "m", "passage: ", 2), &expect(&s, 2, 4)).unwrap();
    let other = BlueprintId(sha256(b"other blueprint"));
    for (bytes, what) in [
        (header(&other, "m", "passage: ", 2), "blueprint_sha256"),
        (header(&s, "m2", "passage: ", 2), "model"),
        (header(&s, "m", "query: ", 2), "prefix"),
    ] {
        let e = check_header(&bytes, &expect(&s, 2, 4)).unwrap_err();
        assert_eq!(e.code(), ErrorCode::LatlasEmbedSpace, "{what}: {e}");
        assert!(e.to_string().contains(what), "{e}");
    }
}

#[test]
fn a_malformed_header_is_a_frame_error() {
    let s = sha();
    let good = String::from_utf8(header(&s, "m", "passage: ", 2)).unwrap();
    let swapped = good.replace("\"index\":0", "\"index\":9");
    for bad in [
        b"{not json".to_vec(),
        header(&s, "m", "passage: ", 1),
        swapped.into_bytes(),
        br#"{"model":"m","prefix":"passage: ","data":[]}"#.to_vec(),
    ] {
        let e = check_header(&bad, &expect(&s, 2, 4)).unwrap_err();
        assert_eq!(e.code(), ErrorCode::LatlasEmbedFrame, "{e}");
    }
}

#[test]
fn vectors_are_bit_exact_in_order_and_their_width_is_checked() {
    let odd = [
        -0.0f32,
        f32::from_bits(1),
        f32::MAX,
        1.5,
        f32::MIN_POSITIVE,
        -7.25,
    ];
    let v = decode_vectors(&le(&odd), "v", 2, 3).unwrap();
    let bits: Vec<Vec<u32>> = v
        .iter()
        .map(|r| r.iter().map(|x| x.to_bits()).collect())
        .collect();
    let want: Vec<Vec<u32>> = odd
        .chunks(3)
        .map(|r| r.iter().map(|x| x.to_bits()).collect())
        .collect();
    assert_eq!(bits, want);
    let e = decode_vectors(&le(&odd), "v", 2, 4).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedDim, "6 floats = 2 x 3");
    assert!(e.to_string().contains("dimension 3"), "{e}");
    let e = decode_vectors(&le(&odd)[..23], "v", 2, 3).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedFrame, "{e}");
}

#[test]
fn limits_derive_from_max_batch_and_dim() {
    let l = Limits::of(256, 2048);
    assert_eq!(l.vectors_max, 256 * 2048 * 4);
    assert_eq!(
        l.header_max,
        HEADER_FIXED_BYTES + 256 * HEADER_PER_ITEM_BYTES
    );
}

fn resident_json(s: &BlueprintId, items: &[(usize, Vec<f32>)], prefix: &str) -> String {
    use base64::Engine as _;
    let data: Vec<Value> = items
        .iter()
        .map(|(i, v)| {
            serde_json::json!({"index": i, "object": "embedding",
                "embedding": base64::engine::general_purpose::STANDARD.encode(le(v))})
        })
        .collect();
    serde_json::json!({"object": "list", "data": data, "model": "m", "usage": {},
        "prefix": prefix, "blueprint_sha256": s.hex()})
    .to_string()
}

#[test]
fn the_resident_body_asks_for_base64_and_its_answer_is_placed_by_index() {
    let b: Value = serde_json::from_str(&resident_body("m", "query", &["q".into()])).unwrap();
    assert_eq!(b["encoding_format"], "base64");
    assert_eq!(b["input_type"], "query");
    let s = sha();
    let body = resident_json(
        &s,
        &[(1, vec![2.0, -0.0]), (0, vec![f32::from_bits(1), 1.0])],
        "passage: ",
    );
    let v = parse_resident(&body, &expect(&s, 2, 2)).unwrap();
    assert_eq!(v[0][0].to_bits(), 1);
    assert_eq!(v[1][1].to_bits(), (-0.0f32).to_bits());
    let e = parse_resident(&body, &expect(&s, 2, 3)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedDim, "{e}");
    assert!(e.to_string().contains("dimension 2"), "{e}");
    let wide = resident_json(&s, &[(0, vec![1.0; 3]), (1, vec![1.0; 3])], "passage: ");
    let e = parse_resident(&wide, &expect(&s, 2, 2)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::LatlasEmbedDim, "{e}");
    let dup = resident_json(&s, &[(0, vec![1.0; 2]), (0, vec![1.0; 2])], "passage: ");
    assert_eq!(
        parse_resident(&dup, &expect(&s, 2, 2)).unwrap_err().code(),
        ErrorCode::LatlasEmbedFrame
    );
    let q = resident_json(&s, &[(0, vec![1.0; 2])], "query: ");
    assert_eq!(
        parse_resident(&q, &expect(&s, 1, 2)).unwrap_err().code(),
        ErrorCode::LatlasEmbedSpace
    );
    let bad = body.replace("\"embedding\":\"", "\"embedding\":\"!");
    assert_eq!(
        parse_resident(&bad, &expect(&s, 2, 2)).unwrap_err().code(),
        ErrorCode::LatlasEmbedFrame
    );
}
