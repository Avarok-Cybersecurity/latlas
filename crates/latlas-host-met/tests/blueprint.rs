// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The preflight's BlueprintId through production `check::blueprint`: a real met-built
//! `.kbp` against the id `met blueprint verify` printed for it (the cross-check that the
//! reimplemented derivation is met's), lite/full and binary/text invariance, and the refusals:
//! a file sha256 as the pin, a truncated full file, a tampered body.

#[path = "support/kbp.rs"]
mod kbp;

use base64::Engine as _;
use latlas_core::hash::sha256;
use latlas_core::kbp::BlueprintId;
use latlas_core::metlocal::parse_sha256;
use latlas_host_met::check::{blueprint, blueprint_id};
use std::path::{Path, PathBuf};

/// 2026-10-24: A met-built, dev-signed lite blueprint (public kernels and metadata only) and the
/// id `met blueprint verify <it> --trust-anchor <root>` printed for it (`blueprint sha256:<id>`).
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/llama-nemotron-embed-vl-1b-v2-bf16.kbp"
);
const FIXTURE_ID: &str = "d3b5294803ebaae694da784a891d610dc8c2fa7fead040b533c024fd3bb53087";

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("blueprint-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn a_met_built_blueprint_has_the_id_met_blueprint_verify_prints() {
    let want = BlueprintId(parse_sha256(FIXTURE_ID).unwrap());
    assert_eq!(blueprint_id(FIXTURE).unwrap(), want);
    blueprint(FIXTURE, &want).unwrap();
    let file_sha = BlueprintId(sha256(&std::fs::read(FIXTURE).unwrap()));
    let e = blueprint(FIXTURE, &file_sha).unwrap_err();
    assert!(
        e.contains(&format!("has BlueprintId {FIXTURE_ID}")) && e.contains(&file_sha.hex()),
        "a file sha256sum is not the pin: {e}"
    );
}

#[test]
fn the_kbp1_text_form_of_the_fixture_has_its_id() {
    let d = dir("text");
    let t = d.join("fixture.kbp.txt");
    let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(FIXTURE).unwrap());
    std::fs::write(&t, format!("kbp1.{b64}\n")).unwrap();
    assert_eq!(blueprint_id(&path(&t)).unwrap().hex(), FIXTURE_ID);
}

#[test]
fn a_full_file_reads_only_its_body_and_shares_the_lite_id() {
    let d = dir("full");
    let raw = b"\nbody of one build\n";
    let lite = d.join("lite.kbp");
    let id = kbp::write(&lite, raw);
    let mut full = kbp::stored(raw);
    full[6] = 1;
    let payload = vec![0xa5u8; 1 << 16];
    full[64..72].copy_from_slice(&u64::try_from(payload.len()).unwrap().to_le_bytes());
    let sig_at = full.len() - 3;
    full.splice(sig_at..sig_at, payload);
    let fp = d.join("full.kbp");
    std::fs::write(&fp, &full).unwrap();
    assert_ne!(sha256(&full), sha256(&std::fs::read(&lite).unwrap()));
    assert_eq!(blueprint_id(&path(&fp)).unwrap(), id);
    assert_eq!(blueprint_id(&path(&lite)).unwrap(), id);
    std::fs::write(&fp, &full[..full.len() - 1]).unwrap();
    let e = blueprint_id(&path(&fp)).unwrap_err();
    assert!(e.contains("is refused") && e.contains("truncated"), "{e}");
}

#[test]
fn a_tampered_body_or_a_missing_file_is_refused() {
    let d = dir("tamper");
    let p = d.join("t.kbp");
    let mut b = kbp::stored(b"\nmode=good\n");
    b[90] ^= 1;
    std::fs::write(&p, &b).unwrap();
    let e = blueprint_id(&path(&p)).unwrap_err();
    assert!(e.contains("is refused") && e.contains("body_sha256"), "{e}");
    let e = blueprint_id(&path(&d.join("absent.kbp"))).unwrap_err();
    assert!(e.contains("unreadable (NotFound)"), "{e}");
}
