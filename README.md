# Latlas

Latlas builds a graph-plus-vector knowledge base from the files in a git repository. Its
decisions, claims, gates, evidence, retractions and rules become typed nodes and edges. The index
is *derived*: it is rebuilt from the repository and is never a second source of truth. The storage
engine is [LatticeDB](https://crates.io/crates/latticedb-core).

Latlas does three things:

- **Derive** a deterministic bundle (nodes, edges, texts and a digest) from a repository's
  sources.
- **Serve** it to agents over MCP with five tools: `la_status`, `la_search`, `la_query`
  (read-only Cypher), `la_impact` and `la_record`.
- **Score** it with `latlas-check`, an integrity vector (freshness, referential integrity,
  coverage, gate rate, hygiene, traceability, keyed contradictions, owed work). A per-dimension
  ratchet can gate CI.

## Status

This is early software (0.1). The APIs and the config schemas (`latlas-project/v1`,
`latlas-check/v1`) may still change.

## Crates

| Crate | Role |
|---|---|
| `latlas-core` | `no_std` model: ids, packs, bundle, derive, search fusion, config |
| `latlas-error` | Coded error registry (`#[repr(u16)]`, append-only `codes.lock`) |
| `latlas-ports` | `no_std` port traits hosts implement (clock, timer) |
| `latlas-adapters` | Built-in adapters (the `claude-code` pack: memory files, CLAUDE.md principle tables) |
| `latlas-decl` | Declarative pack: Markdown tables and a Rust workspace, with no pack code |
| `latlas-check` | Integrity score vector and ratchet |
| `latlas-index` | LatticeDB-backed graph and vector index, read-only Cypher |
| `latlas-host-native` | Native host: filesystem, git facts, OpenRouter-compatible embedder |
| `latlas-host-met` | met-local embedder host: spawns the `met embed` alias or POSTs a resident `met serve` |
| `latlas-cli` | CLI and MCP server library |
| `latlas` | The `latlas` binary |
| `latlas-web` | `wasm32-unknown-unknown` build with a JSON-in/JSON-out C ABI |

## Quick start

Put a project config in `.latlas/config.toml` in the repository:

```toml
schema = "latlas-project/v1"

[sources]
root = "."                         # the repository holding .latlas/
rust = { manifest = "Cargo.toml" } # one Crate node per crate, one Gate per test

[[sources.tables]]                 # a Markdown table becomes typed nodes
id = "decisions"
path = "DECISIONS.md"
heading = "Decisions"
label = "Decision"
key = "#"
fields = ["Decision", "Status"]
outcome = false                    # true: each row is an outcome scored by coverage

[embedder]
mode = "unavailable"
reason = "graph-only for now"

[store]
data_dir = ".latlas/data"          # relative paths resolve against the repository
scratch_dir = ".latlas/scratch"

[limits]
max_rows = 200
cypher_timeout_ms = 2000
token_budget = 1200
status_tokens = 1500
search_deadline_ms = 20000

[search]                           # hybrid ranking weights
w_cosine = 0.6
w_hops = 0.25
w_kind = 0.15

[validator]
python = "/usr/bin/python3"
```

Every setting is explicit: Latlas has no implicit defaults, and a missing required key is an
error. Run `latlas --help` for the full command list.

```sh
cargo build --release -p latlas
latlas --config .latlas/config.toml check    # derive twice, compare digests, list fatal findings
latlas --config .latlas/config.toml status
latlas --config .latlas/config.toml score --check .latlas/check.toml
latlas --config .latlas/config.toml serve    # MCP over stdio
```

## Secrets

Embedder keys are given only as references: a `key_env` variable name or a `key_file` path,
plus the key's expected prefix. A secret written inline in a config is refused. Latlas reads
the key when it uses it and never logs it.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
