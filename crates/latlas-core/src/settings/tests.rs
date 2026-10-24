// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The settings tests (moved whole out of `settings.rs` to keep it under 500 lines).

use super::*;

const GLOBAL: &str = r#"schema = "latlas-config/v1"
[embedder]
mode = "http"
endpoint = "https://openrouter.ai/api/v1/embeddings"
model = "global-model"
dimensions = 2048
headers = { "X-Title" = "latlas" }
key_file = "/k/openrouter.embedder.txt"
key_prefix = "sk-or-"
timeout_ms = 70000
batch = 64
max_attempts = 5
backoff_ms = 2000
min_interval_ms = 0
max_input_chars = 8000
input_type = { query = "query", document = "passage" }
"#;

fn layer(t: &str) -> EmbedLayer {
    toml::from_str(t).unwrap()
}

#[test]
fn the_project_layer_wins_per_key_and_provenance_says_so() {
    let g = parse_global(GLOBAL).unwrap();
    let p = layer("model = \"project-model\"\nbatch = 8\n");
    let (m, prov) = merge(&g, &p);
    let Embedder::Available(s) = resolve(&m).unwrap() else {
        panic!("must be available")
    };
    assert_eq!(
        (s.model.as_str(), s.batch, s.dimensions),
        ("project-model", 8, 2048)
    );
    assert_eq!(prov["model"], Layer::Project);
    assert_eq!(prov["endpoint"], Layer::Global);
    assert_eq!(
        s.key,
        KeyRef::File {
            path: "/k/openrouter.embedder.txt".into(),
            prefix: "sk-or-".into()
        }
    );
    let (m, prov) = merge(&g, &layer("key_env = \"OPENROUTER_KEY\"\n"));
    let Embedder::Available(s) = resolve(&m).unwrap() else {
        panic!("must be available")
    };
    assert_eq!(
        s.key,
        KeyRef::Env {
            name: "OPENROUTER_KEY".into()
        }
    );
    assert!(!prov.contains_key("key_file"));
}

#[test]
fn no_embedder_anywhere_is_the_declared_unavailable_mode() {
    let (m, _) = merge(&EmbedLayer::default(), &EmbedLayer::default());
    assert!(matches!(resolve(&m).unwrap(), Embedder::Unavailable { .. }));
    let p = layer("mode = \"unavailable\"\nreason = \"P0\"\n");
    let (m, _) = merge(&parse_global(GLOBAL).unwrap(), &p);
    assert_eq!(
        resolve(&m).unwrap(),
        Embedder::Unavailable {
            reason: "P0".into()
        }
    );
}

#[test]
fn a_named_embedder_missing_a_key_fails_with_that_key() {
    let without: String = GLOBAL
        .lines()
        .filter(|l| !l.starts_with("min_interval_ms"))
        .collect::<Vec<_>>()
        .join("\n");
    let (m, _) = merge(&parse_global(&without).unwrap(), &EmbedLayer::default());
    let e = resolve(&m).unwrap_err().to_string();
    assert!(e.contains("embedder.min_interval_ms"), "{e}");
    let (m, _) = merge(&EmbedLayer::default(), &layer("model = \"x\"\n"));
    assert!(
        resolve(&m)
            .unwrap_err()
            .to_string()
            .contains("embedder.mode")
    );
}

#[test]
fn an_inline_key_or_two_key_references_are_refused() {
    let (m, _) = merge(
        &parse_global(GLOBAL).unwrap(),
        &layer("headers = { Authorization = \"Bearer sk-or-v1-abc\" }\n"),
    );
    assert!(resolve(&m).unwrap_err().to_string().contains("credential"));
    let both =
        layer("key_env = \"OPENROUTER_KEY\"\nkey_file = \"/k/f\"\nkey_prefix = \"sk-or-\"\n");
    let (m, _) = merge(&parse_global(GLOBAL).unwrap(), &both);
    assert!(resolve(&m).unwrap_err().to_string().contains("both set"));
    assert!(parse_global("schema = \"other\"\n").is_err());
    let (m, _) = merge(
        &parse_global(GLOBAL).unwrap(),
        &layer("key_file = \"/k/f\"\nkey_prefix = \"sk-or-v1-0123456789abcdef\"\n"),
    );
    assert!(resolve(&m).unwrap_err().to_string().contains("key_prefix"));
}
