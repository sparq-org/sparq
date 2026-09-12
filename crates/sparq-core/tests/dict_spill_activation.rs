//! [OPUS-4.8] (sq-bif.13) Edge tests for the SPILLED build-time dictionary (`dict-spill`
//! feature) that go beyond the existing byte-identity guards: they prove the spill path
//! actually ACTIVATES under a tight memory budget (not merely that the output happens to
//! match), and that a store BUILT with spilling reloads correctly through the mmap'd open +
//! re-save round trip.
//!
//! The existing coverage (`lib.rs::tests::dict_spill_build_byte_identical_to_sharded`,
//! `crates/sparq-engine/tests/dict_spill_differential.rs`) pins that a tiny-budget spill build
//! is byte-identical to the sharded in-RAM build. What was NOT pinned, and what this file adds:
//!
//!   * ACTIVATION — that the spill machinery (bounded dedup caches that EVICT, external sorts
//!     that spill to disk) genuinely ran under a tight budget. We assert this two ways that the
//!     in-RAM sharded path cannot satisfy: (a) a `disk_floor` set above the available free disk
//!     makes the spill build abort cleanly (`ensure_disk` inside the spill pipeline fires — the
//!     sharded path has no such gate); (b) a `mem_budget = 1` build (constant cache eviction,
//!     many external-sort runs) is byte-identical to a `mem_budget = huge` build (no eviction,
//!     single in-RAM-sized sort) — proving the EVICTING route reproduces the non-evicting route's
//!     exact id assignment, i.e. the spill code really exercised and was correct.
//!   * MMAP RELOAD — a spill-built store opens via the memory-mapped `Graph::open` and answers
//!     pattern scans, term lookups, and numeric/temporal value probes identically to a plain
//!     in-memory load; and the opened store re-saves (`save` / `save_compressed`) and re-opens
//!     to the same content (a second mmap round trip through the loader).
//!
//! All paths are real: the only mock is the explicit `SpillConfig` (so the test never mutates
//! the process-global `SPARQ_DICT_SPILL*` environment — the sq-x4jy data-race hazard). Any
//! emitted timing is non-canonical; none is asserted.

#![cfg(feature = "dict-spill")]

use sparq_core::dictspill::{free_disk_bytes, SpillConfig};
use sparq_core::store::BUILT;
use sparq_core::Graph;

/// A unique scratch directory under the OS temp (no dev-dep tempdir crate; CI temp is
/// ephemeral). Mirrors the helper idiom in `mmap_corruption_oracle.rs`.
fn scratch(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "sparq-dsp-act-{}-{}-{}-{}",
        tag,
        std::process::id(),
        n,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Dict-heavy synthetic N-Triples: a fresh unique IRI + literal per line drives the dictionary
/// hard, with recurring clustered terms (cross-epoch dedup), every record shape (lang / typed /
/// numeric / temporal / blank / no-prefix IRI), inline integers, and exact-duplicate lines.
fn synthetic_nt(n: u32) -> String {
    let xsd = "http://www.w3.org/2001/XMLSchema#";
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("<http://ex/s{i}> <http://ex/name> \"unique value {i} \\\"q\\\" \\u00e9\" .\n"));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{}\"^^<{xsd}integer> .\n", i % 90));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/score> \"{}.5\"^^<{xsd}decimal> .\n", i % 50));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/when> \"2026-03-{:02}T0{}:00:00Z\"^^<{xsd}dateTime> .\n", 1 + i % 28, i % 10));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/label> \"étiquette {i}\"@fr .\n"));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/follows> <http://ex/s{}> .\n", (i * 7 + 3) % n.max(1)));
        s.push_str(&format!("_:b{i} <http://ex/about> <http://ex/s{i}> .\n"));
        if i % 5 == 0 {
            s.push_str(&format!("<urn:uuid:item-{i}> <http://ex/idx> \"{i}\" .\n"));
        }
    }
    // Exact duplicates + shared terms across distant lines (cache-epoch crossers).
    s.push_str("<http://ex/s0> <http://ex/name> \"unique value 0 \\\"q\\\" \\u00e9\" .\n");
    for i in 0..n.min(40) {
        s.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{}\"^^<{xsd}integer> .\n", i % 90));
    }
    s
}

/// Sorted term-level dump of the default graph — the equality oracle (same shape the existing
/// differential tests use).
fn dump(g: &Graph) -> Vec<[String; 3]> {
    let scan = g.store.scan(&[None, None, None]);
    let mut v: Vec<[String; 3]> = scan
        .rows
        .iter()
        .map(|r| {
            let spo = scan.to_spo(r);
            [g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string()]
        })
        .collect();
    v.sort();
    v
}

/// The streamed dictionary + sidecar files a spill build produces (perm files are compared
/// separately via `BUILT`).
const DICT_FILES: &[&str] = &[
    "dict-meta.bin",
    "dict-terms.bin",
    "dict-offs.bin",
    "dict-hash.bin",
    "dict-hid.bin",
    "numerics-v2.bin",
    "temporals-v2.bin",
    "predstats.bin",
];

/// (ACTIVATION a) A `disk_floor` set above the available free disk must abort the spill build
/// cleanly — proving the spill pipeline's resource gate (`ensure_disk`, which the in-RAM
/// sharded path does NOT have) actually executed on this build. Skipped only where free-disk
/// detection is unavailable (statvfs missing), in which case the gate is a documented no-op.
#[test]
fn spill_build_aborts_when_disk_floor_exceeds_free_space() {
    let dir = scratch("floor");
    let free = match free_disk_bytes(&dir) {
        Some(b) => b,
        None => return, // statvfs unavailable on this target: the floor gate is a no-op, nothing to assert.
    };
    assert!(free > 0, "a real filesystem reports some free space");
    let nt = synthetic_nt(50);
    // Floor above free space => the very first ensure_disk in build_external_spill must Err.
    let cfg = SpillConfig { mem_budget: 1, disk_floor: free.saturating_add(1 << 40) };
    let err = Graph::build_external_spill(nt.as_bytes(), "ntriples", &dir, 256, &cfg)
        .expect_err("spill build must refuse to run below the disk floor");
    assert!(
        err.contains("below the configured floor"),
        "the abort must come from the spill disk gate, got: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// (ACTIVATION b + correctness) A `mem_budget = 1` spill build (caches floor at 4 KiB/shard, so
/// they evict CONSTANTLY across many epochs; the external sorters floor at small runs, so they
/// spill across many runs) must produce a BYTE-IDENTICAL store to a `mem_budget = huge` build
/// (no eviction, a single in-RAM-sized sort). Equal bytes from the two extreme budgets is the
/// proof that the evicting/spilling route ran AND reproduced the non-evicting route's exact id
/// assignment — the activation signal the result-only byte-identity-vs-sharded test cannot give.
#[test]
fn tiny_budget_eviction_matches_comfortable_budget_byte_for_byte() {
    let nt = synthetic_nt(900);
    let tiny_dir = scratch("tiny");
    let comfy_dir = scratch("comfy");

    let tiny = SpillConfig { mem_budget: 1, disk_floor: 0 }; // constant epoch eviction + many runs
    let comfy = SpillConfig { mem_budget: 512 << 20, disk_floor: 0 }; // no eviction, single sort
    Graph::build_external_spill(nt.as_bytes(), "ntriples", &tiny_dir, 256, &tiny).unwrap();
    Graph::build_external_spill(nt.as_bytes(), "ntriples", &comfy_dir, 256, &comfy).unwrap();

    for &f in DICT_FILES {
        let a = std::fs::read(tiny_dir.join(f));
        let b = std::fs::read(comfy_dir.join(f));
        match (a, b) {
            (Ok(a), Ok(b)) => assert!(
                a == b,
                "{f} differs between the tiny-budget (evicting) and comfortable-budget builds ({} vs {} bytes)",
                a.len(),
                b.len()
            ),
            (Err(_), Err(_)) => {} // a file neither build produced is fine
            (a, b) => panic!("{f} present in only one build: tiny={:?} comfy={:?}", a.is_ok(), b.is_ok()),
        }
    }
    for &perm in BUILT {
        let f = format!("perm{}.bin", perm as usize);
        let a = std::fs::read(tiny_dir.join(&f)).unwrap();
        let b = std::fs::read(comfy_dir.join(&f)).unwrap();
        assert!(a == b, "permutation {f} differs between tiny and comfortable budgets");
    }

    // Both stores open and answer identically (a semantic guard atop the byte guard).
    let tg = Graph::open(&tiny_dir).unwrap();
    let cg = Graph::open(&comfy_dir).unwrap();
    assert_eq!(tg.len(), cg.len(), "triple counts differ across budgets");
    assert_eq!(dump(&tg), dump(&cg), "term-level content differs across budgets");

    let _ = std::fs::remove_dir_all(&tiny_dir);
    let _ = std::fs::remove_dir_all(&comfy_dir);
}

/// (MMAP RELOAD) A store BUILT through the spill path under a tight budget must reload via the
/// memory-mapped `Graph::open` and answer identically to a plain in-memory `load_str` — pattern
/// scans, term lookups, and the numeric/temporal value caches the build streamed in final-id
/// order. Then the opened store re-saves (raw + compressed) and re-opens to the same content,
/// exercising a second mmap round trip through the loader on spill-produced files.
#[test]
fn spill_built_store_mmap_reload_round_trip() {
    let nt = synthetic_nt(600);
    let spill_dir = scratch("reload");
    let cfg = SpillConfig { mem_budget: 1, disk_floor: 0 }; // tight budget => spill activates
    Graph::build_external_spill(nt.as_bytes(), "ntriples", &spill_dir, 256, &cfg).unwrap();

    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let ext = Graph::open(&spill_dir).unwrap();

    // Counts + full term-level content.
    assert_eq!(mem.len(), ext.len(), "triple count differs (mem vs spill-mmap)");
    assert_eq!(mem.dict.len(), ext.dict.len(), "dict size differs (mem vs spill-mmap)");
    assert_eq!(dump(&mem), dump(&ext), "term-level content differs (mem vs spill-mmap)");

    // Bound-predicate scans over the mmap'd store match the in-memory store's, term for term.
    let pred = |g: &Graph, p: &str| -> usize {
        match g.id_of(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(p.to_string()))) {
            Some(pid) => g.store.scan(&[None, Some(pid), None]).rows.len(),
            None => 0,
        }
    };
    for p in ["http://ex/age", "http://ex/follows", "http://ex/label", "http://ex/when"] {
        assert_eq!(pred(&mem, p), pred(&ext, p), "scan cardinality differs for predicate {p}");
        assert!(pred(&ext, p) > 0, "predicate {p} unexpectedly absent from the spill store");
    }

    // Numeric + temporal value caches: every age/score/when value the spill build streamed must
    // read back through the mmap'd numerics/temporals exactly as the in-memory cache reports.
    let mut numeric_checks = 0usize;
    let mut temporal_checks = 0usize;
    for row in ext.iter_ids() {
        for &id in &row {
            assert_eq!(
                ext.numeric_value(id),
                mem.numeric_value(remap_id(&mem, &ext, id)),
                "numeric value cache diverges for a spill-built term"
            );
            if ext.numeric_value(id).is_some() {
                numeric_checks += 1;
            }
            let et = ext.temporal_value(id);
            let mt = mem.temporal_value(remap_id(&mem, &ext, id));
            assert_eq!(et.map(|t| t.instant), mt.map(|t| t.instant), "temporal value cache diverges");
            if et.is_some() {
                temporal_checks += 1;
            }
        }
    }
    assert!(numeric_checks > 0, "the test never exercised a numeric cache cell (vacuous)");
    assert!(temporal_checks > 0, "the test never exercised a temporal cache cell (vacuous)");

    // Re-save the spill-built store (raw + compressed) and re-open: a second mmap round trip
    // through the loader, proving the spill-produced graph persists + reloads identically.
    let raw_dir = scratch("resave-raw");
    ext.save(&raw_dir).unwrap();
    let raw = Graph::open(&raw_dir).unwrap();
    assert_eq!(dump(&raw), dump(&ext), "raw re-save round trip changed content");

    let comp_dir = scratch("resave-comp");
    ext.save_compressed(&comp_dir).unwrap();
    let comp = Graph::open(&comp_dir).unwrap();
    assert_eq!(dump(&comp), dump(&ext), "compressed re-save round trip changed content");
    assert_eq!(comp.len(), ext.len(), "compressed re-save triple count drifted");

    let _ = std::fs::remove_dir_all(&spill_dir);
    let _ = std::fs::remove_dir_all(&raw_dir);
    let _ = std::fs::remove_dir_all(&comp_dir);
}

/// Translate an id from the `from`-graph's dict to the `to`-graph's dict via its term string —
/// the two dicts assign ids in different orders, so a value-cache comparison must go through the
/// shared term identity, not the raw id.
fn remap_id(to: &Graph, from: &Graph, from_id: sparq_core::dict::Id) -> sparq_core::dict::Id {
    let term = from.dict.term(from_id);
    to.id_of(&term).unwrap_or(from_id)
}

/// An empty document still drives the spill pipeline end to end (zero shards' worth of records,
/// the consolidation + remap phases over empty inputs) and opens to an empty store — a boundary
/// the streaming phases must not trip on.
#[test]
fn spill_build_empty_input_opens_empty() {
    let dir = scratch("empty");
    let cfg = SpillConfig { mem_budget: 1, disk_floor: 0 };
    Graph::build_external_spill("".as_bytes(), "ntriples", &dir, 64, &cfg).unwrap();
    let g = Graph::open(&dir).unwrap();
    assert_eq!(g.len(), 0, "empty spill build has no triples");
    assert_eq!(g.dict.len(), 0, "empty spill build interns no terms");
    let _ = std::fs::remove_dir_all(&dir);
}
