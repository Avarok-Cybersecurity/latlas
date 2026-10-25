// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-25: Both durable stores survive a reopen after writing more than one WAL page.
//!
//! The defect (latticedb-storage 0.3.4 `DiskStorage`): every page lived at `page_id * 4096` in one
//! `pages.dat`, while a WAL page grows to 256 KiB and a single batch can exceed even that (64
//! 2048-dimension vectors are ~530 KB). Each rotation wrote the new page over the tail of the one
//! before, the WAL replay skipped the clobbered entries, and the embedding cache reopened without
//! its space meta point, so it emptied itself on every open.
//!
//! The batches below mirror the live shapes: a 2048-dimension space, 64 vectors a put; a graph
//! bundle large enough that one load spans pages.

use latlas_core::hash::{Sha256, sha256};
use latlas_core::id::{CoreOnly, NodeId, core_labels};
use latlas_core::jsonl::digest;
use latlas_core::{Bundle, Node};
use latlas_error::ErrorCode;
use latlas_index::store::{STORE_FORMAT, Store};
use latlas_index::textstore::TextStore;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn fresh(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("latlas-reopen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const DIM: u32 = 2048;
const BATCH: usize = 64;

fn batch(round: u32) -> Vec<(Sha256, Vec<f32>)> {
    (0..BATCH)
        .map(|i| {
            let tag = format!("{round}/{i}");
            let mut v = vec![0.0f32; DIM as usize];
            v[i] = 1.0;
            v[DIM as usize - 1] = f32::from(u16::try_from(round).unwrap());
            (sha256(tag.as_bytes()), v)
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn the_embedding_cache_keeps_its_vectors_and_space_across_a_reopen() {
    let dir = fresh("text");
    let rounds = 4u32;
    let mut want = BTreeMap::new();
    {
        let mut s = TextStore::open(&dir, "space-a", DIM).await.unwrap();
        for r in 0..rounds {
            let b = batch(r);
            want.extend(b.iter().cloned());
            assert_eq!(s.put(b).await.unwrap(), BATCH);
        }
        assert_eq!(s.vectors().unwrap(), want);
    }
    let s = TextStore::open(&dir, "space-a", DIM).await.unwrap();
    let got = s.vectors().unwrap();
    assert_eq!(
        got.len(),
        want.len(),
        "the reopened cache holds {} of {} vectors (an emptied cache means the space meta was lost)",
        got.len(),
        want.len()
    );
    assert_eq!(got, want, "a reopened vector differs from the one written");
    drop(s);

    // The space meta is what survived: another space still empties the cache.
    let other = TextStore::open(&dir, "space-b", DIM).await.unwrap();
    assert!(other.cached().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

fn bundle(tag: &str, n: usize) -> Bundle {
    let nodes = (0..n)
        .map(|i| {
            let id = NodeId::new(core_labels::Decision, format!("{tag}-{i:05}")).unwrap();
            let mut p = BTreeMap::new();
            p.insert(
                "body".to_string(),
                Value::String(format!("{tag} {i} ").repeat(64)),
            );
            Node {
                id,
                src: format!("d.md:{i}"),
                sha: sha256(format!("{tag}{i}").as_bytes()),
                p,
            }
        })
        .collect::<Vec<_>>();
    let mut b = Bundle {
        nodes,
        edges: Vec::new(),
        texts: Vec::new(),
        sources: Vec::new(),
        errors: Vec::new(),
    };
    b.nodes.sort_by(|x, y| x.id.cmp(&y.id));
    b
}

/// 2026-10-25: Found by a sweep of (A, B, C) sizes with a reopen after every load: a store that
/// has once held a bundle past one WAL page (400 nodes) loses a later small load on reopen, so
/// the store reopens holding an earlier load's digest and the next open reloads the bundle. A load of 400 then 50 then 50
/// nodes is the smallest such case of the sweep.
#[tokio::test(flavor = "current_thread")]
async fn the_graph_store_reopens_with_the_bundle_it_was_last_loaded_with() {
    let dir = fresh("graph");
    let seq = [bundle("a", 400), bundle("b", 50), bundle("c", 50)];
    for (k, b) in seq.iter().enumerate() {
        let want = {
            let fresh_dir = fresh(&format!("graph-fresh-{k}"));
            let mut s = Store::open::<CoreOnly>(&fresh_dir).await.unwrap();
            assert!(s.load(b).await.unwrap());
            let e = s.export().unwrap();
            drop(s);
            let _ = std::fs::remove_dir_all(&fresh_dir);
            e
        };
        {
            let mut s = Store::open::<CoreOnly>(&dir).await.unwrap();
            assert!(
                s.load(b).await.unwrap(),
                "load {k} found its bundle already stored"
            );
        }
        let mut s = Store::open::<CoreOnly>(&dir).await.unwrap();
        assert_eq!(
            s.stored_digest(),
            Some(format!("{STORE_FORMAT}/{}", digest(b).hex())),
            "after load {k} the reopened store's digest meta is not the last load's"
        );
        let export = s.export().unwrap();
        assert!(
            !s.load(b).await.unwrap(),
            "after load {k} the reopened store reloaded"
        );
        assert_eq!(
            export, want,
            "after load {k} the reopened store differs from a fresh load"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 2026-10-25: A store written by latticedb-storage 0.3.4 (`pages.dat`, pages that may overlap)
/// is refused with its own code, for both stores, and nothing is read from it.
#[tokio::test(flavor = "current_thread")]
async fn a_store_in_the_overlapping_page_layout_is_refused_with_a_rebuild_code() {
    for part in ["kg", "kg_text"] {
        let dir = fresh(&format!("legacy-{part}"));
        let wal = dir.join(part).join("wal");
        std::fs::create_dir_all(&wal).unwrap();
        std::fs::write(wal.join("pages.dat"), vec![0u8; 8192]).unwrap();
        let e = match part {
            "kg" => Store::open::<CoreOnly>(&dir).await.err(),
            _ => TextStore::open(&dir, "space-a", DIM).await.err(),
        }
        .unwrap_or_else(|| panic!("{part}: a pages.dat store opened"));
        assert_eq!(e.code(), ErrorCode::LatlasStoreFormat, "{part}: {e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
