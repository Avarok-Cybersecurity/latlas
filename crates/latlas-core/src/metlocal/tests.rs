// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: met-local config resolution: every key required, the other invocation's keys and
//! mode `http`'s keys refused, paths absolute, the sha well-formed, the space keyed by the sha.

use super::*;
use crate::settings::{Embedder, merge, parse_global, resolve as resolve_all};

const SHA: &str = "2192a98e00000000000000000000000000000000000000000000000000000001";

const ALIAS: &str = r#"mode = "met-local"
invocation = "alias"
met_path = "/opt/met/met"
blueprint = "/opt/met/embed.blueprint"
blueprint_id = "2192a98e00000000000000000000000000000000000000000000000000000001"
model = "nvidia/llama-nemotron-embed-vl-1b-v2"
dimensions = 2048
input_type = { query = "query", document = "passage" }
max_batch = 256
max_input_chars = 8000
boot_deadline_ms = 30000
request_deadline_ms = 60000
idle_exit_ms = 0
trust_mode = "dev-anchor"
trust_anchor = "/etc/latlas/kbp-root-dev.anchor"
"#;

const ANCHOR: &str = "trust_anchor = \"/etc/latlas/kbp-root-dev.anchor\"";

fn layer(t: &str) -> EmbedLayer {
    toml::from_str(t).unwrap()
}

fn resident() -> String {
    ALIAS
        .replace("invocation = \"alias\"", "invocation = \"resident\"")
        .lines()
        .filter(|l| !ALIAS_ONLY.iter().any(|k| l.starts_with(k)))
        .chain(["endpoint = \"http://127.0.0.1:18471/v1/embeddings\""])
        .collect::<Vec<_>>()
        .join("\n")
}

fn err(t: &str) -> String {
    let (m, _) = merge(&EmbedLayer::default(), &layer(t));
    resolve_all(&m).unwrap_err().to_string()
}

fn without(t: &str, key: &str) -> String {
    t.lines()
        .filter(|l| !l.starts_with(&format!("{key} ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_alias_and_a_resident_config_resolve_with_the_sha_in_the_space() {
    let Embedder::MetLocal(s) = resolve_all(&layer(ALIAS)).unwrap() else {
        panic!("must be met-local")
    };
    assert_eq!(
        s.invocation,
        Invocation::Alias {
            met_path: "/opt/met/met".into(),
            boot_deadline_ms: 30000,
            idle_exit_ms: 0,
            trust: MetTrust::DevAnchor {
                anchor: "/etc/latlas/kbp-root-dev.anchor".into()
            }
        }
    );
    assert_eq!(s.blueprint_id.hex(), SHA);
    assert_eq!(
        s.space(),
        format!("nvidia/llama-nemotron-embed-vl-1b-v2/2048/passage/{SHA}")
    );
    let Embedder::MetLocal(r) = resolve_all(&layer(&resident())).unwrap() else {
        panic!("must be met-local")
    };
    assert_eq!(
        r.invocation,
        Invocation::Resident {
            endpoint: "http://127.0.0.1:18471/v1/embeddings".into()
        }
    );
}

#[test]
fn every_key_of_the_mode_is_required() {
    let alias_keys = [
        "invocation",
        "met_path",
        "blueprint",
        "blueprint_id",
        "model",
        "dimensions",
        "input_type",
        "max_batch",
        "max_input_chars",
        "boot_deadline_ms",
        "request_deadline_ms",
        "idle_exit_ms",
        "trust_mode",
    ];
    for k in alias_keys {
        let e = err(&without(ALIAS, k));
        assert!(e.contains(&format!("embedder.{k}")), "{k}: {e}");
        assert!(e.contains("required"), "{k}: {e}");
    }
    let e = err(&without(&resident(), "endpoint"));
    assert!(
        e.contains("embedder.endpoint") && e.contains("required"),
        "{e}"
    );
}

#[test]
fn a_key_of_the_other_invocation_or_of_http_is_refused() {
    let e = err(&format!(
        "{ALIAS}endpoint = \"http://127.0.0.1:1/v1/embeddings\"\n"
    ));
    assert!(
        e.contains("embedder.endpoint") && e.contains("invocation alias"),
        "{e}"
    );
    for k in ALIAS_ONLY {
        let line = if matches!(*k, "met_path" | "trust_mode" | "trust_anchor") {
            format!("{k} = \"/opt/met/met\"")
        } else {
            format!("{k} = 1")
        };
        let e = err(&format!("{}\n{line}\n", resident()));
        assert!(
            e.contains(&format!("embedder.{k}")) && e.contains("invocation resident"),
            "{k}: {e}"
        );
    }
    let e = err(&format!("{ALIAS}batch = 64\n"));
    assert!(
        e.contains("embedder.batch") && e.contains("met-local"),
        "{e}"
    );
    let e = err("mode = \"http\"\nmax_batch = 3\n");
    assert!(
        e.contains("embedder.max_batch") && e.contains("mode http"),
        "{e}"
    );
}

#[test]
fn relative_paths_a_bad_sha_and_a_remote_endpoint_are_refused() {
    let e = err(&ALIAS.replace("\"/opt/met/met\"", "\"met\""));
    assert!(
        e.contains("embedder.met_path") && e.contains("absolute"),
        "{e}"
    );
    let e = err(&ALIAS.replace("\"/opt/met/embed.blueprint\"", "\"embed.blueprint\""));
    assert!(
        e.contains("embedder.blueprint") && e.contains("absolute"),
        "{e}"
    );
    let e = err(&ALIAS.replace(SHA, &SHA.to_uppercase()));
    assert!(e.contains("embedder.blueprint_id"), "{e}");
    let e = err(&ALIAS.replace(SHA, &SHA[..63]));
    assert!(e.contains("embedder.blueprint_id"), "{e}");
    for bad in [
        "http://10.0.0.1:18471/v1/embeddings",
        "http://127.0.0.1.evil.com/v1/embeddings",
        "ftp://127.0.0.1/v1/embeddings",
        "http://127.0.0.1:18471/v1/chat/completions",
    ] {
        let e = err(&resident().replace("http://127.0.0.1:18471/v1/embeddings", bad));
        assert!(e.contains("embedder.endpoint"), "{bad}: {e}");
    }
    for good in [
        "https://localhost/v1/embeddings",
        "http://[::1]:9/v1/embeddings",
    ] {
        assert!(loopback_embeddings(good), "{good}");
    }
    let e = err(&ALIAS.replace("invocation = \"alias\"", "invocation = \"remote\""));
    assert!(e.contains("embedder.invocation"), "{e}");
    let e = err(&ALIAS.replace("max_batch = 256", "max_batch = 0"));
    assert!(
        e.contains("embedder.max_batch") && e.contains("positive"),
        "{e}"
    );
}

#[test]
fn a_project_mode_unlike_the_global_one_drops_the_global_layer_whole() {
    let global = parse_global(
        r#"schema = "latlas-config/v1"
[embedder]
mode = "http"
endpoint = "https://openrouter.ai/api/v1/embeddings"
key_env = "OPENROUTER_KEY"
timeout_ms = 70000
"#,
    )
    .unwrap();
    let (m, prov) = merge(&global, &layer(ALIAS));
    assert!(matches!(resolve_all(&m).unwrap(), Embedder::MetLocal(_)));
    assert!(!prov.contains_key("timeout_ms"));
    let (m, prov) = merge(&global, &layer("model = \"x\"\n"));
    assert_eq!(m.timeout_ms, Some(70000), "same mode still merges per key");
    assert_eq!(prov["timeout_ms"], crate::settings::Layer::Global);
}

#[test]
fn models_url_sits_beside_the_embeddings_endpoint() {
    assert_eq!(
        models_url("http://127.0.0.1:1/v1/embeddings").as_deref(),
        Some("http://127.0.0.1:1/v1/models")
    );
    assert_eq!(models_url("http://127.0.0.1:1/v1/other"), None);
}

fn alias_trust(t: &str) -> MetTrust {
    let Embedder::MetLocal(s) = resolve_all(&layer(t)).unwrap() else {
        panic!("must be met-local")
    };
    let Invocation::Alias { trust, .. } = s.invocation else {
        panic!("must be alias")
    };
    trust
}

fn production() -> String {
    without(ALIAS, "trust_anchor").replace("\"dev-anchor\"", "\"production\"")
}

#[test]
fn trust_mode_production_resolves_and_refuses_a_trust_anchor() {
    assert_eq!(alias_trust(&production()), MetTrust::Production);
    let e = err(&format!("{}\n{ANCHOR}\n", production()));
    assert!(
        e.contains("embedder.trust_anchor") && e.contains("trust_mode production"),
        "{e}"
    );
}

#[test]
fn trust_mode_dev_anchor_needs_an_absolute_trust_anchor() {
    assert_eq!(
        alias_trust(ALIAS),
        MetTrust::DevAnchor {
            anchor: "/etc/latlas/kbp-root-dev.anchor".into()
        }
    );
    let e = err(&without(ALIAS, "trust_anchor"));
    assert!(
        e.contains("embedder.trust_anchor")
            && e.contains("required")
            && e.contains("trust_mode dev-anchor"),
        "{e}"
    );
    let e = err(&ALIAS.replace(
        "\"/etc/latlas/kbp-root-dev.anchor\"",
        "\"kbp-root-dev.anchor\"",
    ));
    assert!(
        e.contains("embedder.trust_anchor") && e.contains("absolute"),
        "{e}"
    );
}

#[test]
fn an_unknown_or_missing_trust_mode_is_refused() {
    for bad in ["dev", "none", "", "Production"] {
        let e = err(&ALIAS.replace("\"dev-anchor\"", &format!("\"{bad}\"")));
        assert!(
            e.contains("embedder.trust_mode") && e.contains("is not production or dev-anchor"),
            "{bad}: {e}"
        );
    }
    let e = err(&without(ALIAS, "trust_mode"));
    assert!(
        e.contains("embedder.trust_mode") && e.contains("required"),
        "{e}"
    );
}

#[test]
fn a_resident_config_refuses_the_trust_keys() {
    let base = without(&without(&resident(), "trust_mode"), "trust_anchor");
    for line in ["trust_mode = \"production\"", ANCHOR] {
        let e = err(&format!("{base}\n{line}\n"));
        let key = line.split(' ').next().unwrap();
        assert!(
            e.contains(&format!("embedder.{key}")) && e.contains("invocation resident"),
            "{line}: {e}"
        );
    }
}
