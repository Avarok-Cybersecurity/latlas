// SPDX-License-Identifier: MIT OR Apache-2.0
// 2026-10-24: The HPAR gate for latlas-web (DESIGN-latlas.md §6 K4): derive one repository through
// the WebAssembly module (this script plays the page: it lists and reads the files the native
// DeclHost would) and through the native `latlas` binary, and require the same bundle digest.
// Usage: node hpar.mjs <latlas_web.wasm> <latlas binary> <repo dir>
import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const [wasmPath, latlas, repo] = process.argv.slice(2);
const cfgText = readFileSync(join(repo, ".latlas/config.toml"), "utf8");

// The page's file listing: the declared table paths and, for [sources.rust], every member's
// Cargo.toml and .rs files under src/ and tests/ (the same rule as the native DeclHost).
const files = {};
const add = (rel) => { files[rel] = readFileSync(join(repo, rel), "utf8"); };
for (const m of cfgText.matchAll(/^path\s*=\s*"([^"]+)"/gm)) add(m[1]);
const manifest = cfgText.match(/manifest\s*=\s*"([^"]+)"/);
const rsUnder = (rel) => {
  if (!existsSync(join(repo, rel)) || !statSync(join(repo, rel)).isDirectory()) return;
  for (const n of readdirSync(join(repo, rel)).sort()) {
    const r = `${rel}/${n}`;
    if (statSync(join(repo, r)).isDirectory()) rsUnder(r);
    else if (n.endsWith(".rs")) add(r);
  }
};
if (manifest) {
  add(manifest[1]);
  const ws = readFileSync(join(repo, manifest[1]), "utf8");
  const block = ws.match(/members\s*=\s*\[([^\]]*)\]/s);
  const members = block ? [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]) : [];
  for (const m of members) {
    const dirs = m.endsWith("/*")
      ? readdirSync(join(repo, m.slice(0, -2))).sort().map((d) => `${m.slice(0, -2)}/${d}`)
          .filter((d) => existsSync(join(repo, d, "Cargo.toml")))
      : [m];
    for (const d of dirs) { add(`${d}/Cargo.toml`); rsUnder(`${d}/src`); rsUnder(`${d}/tests`); }
  }
}

const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const { memory, la_alloc, la_call, la_free } = instance.exports;
const call = (req) => {
  const bytes = new TextEncoder().encode(JSON.stringify(req));
  const p = la_alloc(bytes.length);
  new Uint8Array(memory.buffer, p, bytes.length).set(bytes);
  const r = la_call(p, bytes.length);
  const op = Number(r >> 32n), on = Number(r & 0xffffffffn);
  const out = new TextDecoder().decode(new Uint8Array(memory.buffer, op, on));
  la_free(p, bytes.length);
  la_free(op, on);
  return JSON.parse(out);
};

const web = call({ op: "derive", config: cfgText, files });
if (web.error) { console.error("web:", web); process.exit(2); }
const native = JSON.parse(execFileSync(latlas, ["--config", join(repo, ".latlas/config.toml"), "check"], { encoding: "utf8" }));
const status = call({ op: "status", config: cfgText, files, now_ms: 0 });
const search = call({ op: "search", config: cfgText, files, text: "SmallVec neighbors", k: 3 });
const ok = web.digest === native.digest;
console.log(JSON.stringify({ web: web.digest, native: native.digest, identical: ok,
  nodes: web.nodes, files: Object.keys(files).length,
  status_ok: !status.error, search_top: search.hits?.[0]?.id ?? search }, null, 1));
process.exit(ok ? 0 : 1);
