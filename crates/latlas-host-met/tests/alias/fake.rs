// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The fake `met embed`: this test binary run as `<exe> embed --blueprint <b>
//! --input-type <t> --idle-exit-ms <n> --trust-mode <m> [--trust-anchor <a>]`. It speaks the frames by hand (not through
//! latlas-core's codec, so a codec bug cannot hide behind a matching fake). Its behaviour is the
//! blueprint file's `mode=` line, so the production argv is unchanged; every spawn appends its
//! pid (and a grandchild's, for the deadline modes) to `<blueprint>.pids` and (2026-10-25) its
//! argv, tab-joined, to `<blueprint>.argv`. Like met, it refuses to boot without a well-formed
//! `--trust-mode`.

use serde_json::{Value, json};
use std::io::{Read, Write};

/// 2026-10-24: The vector the fake answers for `text` (bit patterns that a decimal round trip
/// or a reordering would change: a sign-flipped zero, a subnormal, values keyed by the text).
pub fn vector(text: &str, dim: usize) -> Vec<f32> {
    let h = text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    (0..dim)
        .map(|j| match j {
            0 => -0.0,
            1 => f32::from_bits(h & 0x007f_ffff),
            _ => f32::from_bits(
                (h.rotate_left(u32::try_from(j).unwrap()) & 0xbf7f_ffff) | 0x3000_0000,
            ),
        })
        .collect()
}

fn read_frame(r: &mut impl Read) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    let mut b = vec![0u8; usize::try_from(u32::from_le_bytes(len)).unwrap()];
    r.read_exact(&mut b).ok()?;
    Some(b)
}

fn frame(w: &mut impl Write, payload: &[u8]) {
    w.write_all(&u32::try_from(payload.len()).unwrap().to_le_bytes())
        .unwrap();
    w.write_all(payload).unwrap();
}

fn record_pid(blueprint: &str, pid: u32) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{blueprint}.pids"))
        .unwrap();
    writeln!(f, "{pid}").unwrap();
}

/// 2026-10-24: Run as the fake; never returns.
pub fn run(args: &[String]) -> ! {
    let refuse = || -> ! {
        eprintln!("fake-met: argv {args:?}");
        std::process::exit(64);
    };
    let [_, embed, b, blueprint, t, input_type, i, _idle, trust @ ..] = args else {
        refuse()
    };
    if embed != "embed" || b != "--blueprint" || t != "--input-type" || i != "--idle-exit-ms" {
        refuse()
    }
    let trusted = match trust {
        [m, p] => m == "--trust-mode" && p == "production",
        [m, d, a, path] => {
            m == "--trust-mode" && d == "dev-anchor" && a == "--trust-anchor" && !path.is_empty()
        }
        _ => false,
    };
    if !trusted {
        refuse()
    }
    let mut argv = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{blueprint}.argv"))
        .unwrap();
    writeln!(argv, "{}", args[1..].join("\t")).unwrap();
    record_pid(blueprint, std::process::id());
    let conf = String::from_utf8_lossy(&std::fs::read(blueprint).unwrap()).into_owned();
    let get = |k: &str| {
        conf.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("blueprint lacks {k}="))
            .to_string()
    };
    let mode = get("mode");
    let dim: usize = get("dim").parse().unwrap();
    if mode == "sleep" {
        let mut g = std::process::Command::new("sleep")
            .arg("600")
            .spawn()
            .unwrap();
        record_pid(blueprint, g.id());
        let _ = g.wait();
        std::process::exit(0);
    }
    if mode == "crash" {
        eprintln!("loading weights");
        eprintln!("MetModelLoad 0x0901: fake weights are absent");
        std::process::exit(3);
    }
    let mut stdin = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    while let Some(h) = read_frame(&mut stdin) {
        let h: Value = serde_json::from_slice(&h).unwrap();
        let body: Value = serde_json::from_slice(&read_frame(&mut stdin).unwrap()).unwrap();
        assert_eq!(h["input_type"], input_type.as_str());
        assert_eq!(h["encoding"], "f32le");
        let inputs: Vec<String> = serde_json::from_value(body["input"].clone()).unwrap();
        assert_eq!(h["count"], inputs.len());
        let sha = match mode.as_str() {
            "wrongsha" => "0".repeat(64),
            _ => h["blueprint_sha256"].as_str().unwrap().to_string(),
        };
        let prefix = match mode.as_str() {
            "wrongprefix" => "query: ".to_string(),
            _ => format!("{input_type}: "),
        };
        let data: Vec<Value> = (0..inputs.len())
            .map(|i| json!({"index": i, "object": "embedding"}))
            .collect();
        let header = json!({"object": "list", "data": data, "model": body["model"],
            "usage": {"prompt_tokens": 1, "total_tokens": 1}, "prefix": prefix,
            "blueprint_sha256": sha});
        match mode.as_str() {
            "badjson" => frame(&mut out, b"{\"object\": \"list\", "),
            "oversize" => out.write_all(&u32::MAX.to_le_bytes()).unwrap(),
            "truncated" => {
                out.write_all(&100u32.to_le_bytes()).unwrap();
                out.write_all(&[b'{'; 10]).unwrap();
                out.flush().unwrap();
                std::process::exit(0);
            }
            _ => frame(&mut out, header.to_string().as_bytes()),
        }
        let width = if mode == "wrongdim" { dim - 1 } else { dim };
        let mut v: Vec<u8> = inputs
            .iter()
            .flat_map(|t| vector(t, width))
            .flat_map(f32::to_le_bytes)
            .collect();
        if mode == "badlen" {
            v.truncate(v.len() - 2);
        }
        if !matches!(mode.as_str(), "badjson" | "oversize") {
            frame(&mut out, &v);
        }
        out.flush().unwrap();
    }
    std::process::exit(0);
}
