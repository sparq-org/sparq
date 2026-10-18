//! sq-vkz7 — differential guard for the ONE-PASS COMPRESSED external build.
//!
//! `build_external` writes RAW `[u32;3]` perms; producing the 2.5–2.75×-smaller
//! `SPQCPRM1` block-compressed perms used to require a SEPARATE `open` + `decode_all` +
//! re-encode `recompress` pass over the whole index. sq-vkz7 folds the FoR+varint block
//! encoder into the sort/merge tail so the build emits compressed perms in ONE pass when
//! `SPARQ_BUILD_COMPRESSED` is on (raw stays the default per
//! `research/compressed-perms-verdict.md`).
//!
//! Gate, exactly as the bead specifies:
//!   * BYTE-IDENTITY — every `perm{i}.bin` from the one-pass compressed build must equal
//!     the file a raw build followed by `recompress` (`save_compressed`) would write, and
//!     the non-perm files (dict / numerics / temporals / predstats) must be unchanged.
//!   * OPEN/QUERY PARITY — the opened compressed dir answers identically to a plain
//!     in-memory load and to the raw build.
//!
//! The compressed build is forced via `sparq_core::with_build_compressed` (a per-thread
//! override) so the test never mutates the process-global environment — the `set_var`
//! data-race hazard the dict-spill `from_lookup` note documents.

use sparq_core::Graph;
use sparq_engine::query;

/// A small dataset with enough distinct subjects/predicates/objects to span multiple
/// 128-row blocks in every permutation order (so block boundaries are exercised), plus
/// the term-kind variety the dict/numeric/temporal caches store.
fn synthetic_nt(n: u32) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("<http://ex/s{i}> <http://ex/name> \"value {i}\" .\n"));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", i % 90));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/when> \"2026-01-{:02}T0{}:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n", 1 + i % 28, i % 10));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/follows> <http://ex/s{}> .\n", (i * 7 + 3) % n.max(1)));
    }
    s
}

/// The data files of the store at `dir`: every file but its marker (`sparq-store`), which
/// records the store's own random identity and so differs between any two stores.
fn dir_files(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "sparq-store")
        .collect();
    v.sort();
    v
}

fn dump(g: &Graph) -> Vec<String> {
    let scan = g.store().scan(&[None, None, None]);
    let mut v: Vec<String> = scan
        .rows
        .iter()
        .map(|r| {
            let spo = scan.to_spo(r);
            format!("{} {} {}", g.dict().term(spo[0]), g.dict().term(spo[1]), g.dict().term(spo[2]))
        })
        .collect();
    v.sort();
    v
}

const QUERIES: &[&str] = &[
    "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }",
    "SELECT ?s WHERE { ?s <http://ex/age> ?a . FILTER(?a > 40) }",
    "SELECT ?s ?o WHERE { ?s <http://ex/follows> ?o }",
    "SELECT ?s WHERE { ?s <http://ex/when> ?t . FILTER(?t > \"2026-01-15T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>) }",
    "SELECT ?o WHERE { <http://ex/s3> ?p ?o }",
];

#[test]
fn one_pass_compressed_build_is_byte_identical_to_build_then_recompress() {
    let nt = synthetic_nt(400); // > 128 rows in every order ⇒ multi-block perms
    let base = std::env::temp_dir().join(format!("sparq_vkz7_diff_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let raw_dir = base.join("raw");
    let recompressed_dir = base.join("recompressed");
    let compressed_dir = base.join("compressed");

    // (1) The default RAW external build.
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &raw_dir, 256, true).unwrap();

    // (2) The TWO-PASS reference: open the raw build and `recompress` it (save_compressed).
    Graph::open(&raw_dir).unwrap().save_compressed(&recompressed_dir).unwrap();

    // (3) The ONE-PASS compressed build (forced per-thread, no env mutation).
    sparq_core::with_build_compressed(true, || {
        Graph::build_external_opts(nt.as_bytes(), "ntriples", &compressed_dir, 256, true).unwrap();
    });

    // The compressed perms must be byte-identical to the recompressed perms; the non-perm
    // files (dict/numerics/temporals/predstats) must match the raw build (the recompress
    // path rewrites them unchanged, so all three dirs agree).
    let files = dir_files(&compressed_dir);
    assert_eq!(files, dir_files(&recompressed_dir), "compressed-build file set differs from recompress");
    let mut saw_compressed_perm = false;
    for f in &files {
        let one_pass = std::fs::read(compressed_dir.join(f)).unwrap();
        let recompressed = std::fs::read(recompressed_dir.join(f)).unwrap();
        assert!(
            one_pass == recompressed,
            "{f}: one-pass compressed build differs from build-then-recompress ({} vs {} bytes)",
            one_pass.len(),
            recompressed.len()
        );
        if f.starts_with("perm") && one_pass.starts_with(b"SPQCPRM1") {
            saw_compressed_perm = true;
        }
        // The compressed perms must NOT be byte-equal to the raw perms (sanity: the build
        // actually compressed, it didn't silently fall back to raw).
        if f.starts_with("perm") && !one_pass.is_empty() {
            let raw = std::fs::read(raw_dir.join(f)).unwrap();
            assert!(one_pass != raw || raw.is_empty(), "{f}: compressed build produced a RAW perm");
            assert!(one_pass.starts_with(b"SPQCPRM1"), "{f}: built perm is not SPQCPRM1");
        }
    }
    assert!(saw_compressed_perm, "no SPQCPRM1 perm was produced — the gate did not engage");

    // OPEN/QUERY PARITY: in-memory load, raw build, and one-pass compressed build all agree.
    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let raw = Graph::open(&raw_dir).unwrap();
    let comp = Graph::open(&compressed_dir).unwrap();
    assert_eq!(mem.len(), comp.len(), "triple count differs");
    assert_eq!(dump(&mem), dump(&comp), "term-level content differs (in-memory vs compressed)");
    assert_eq!(dump(&raw), dump(&comp), "term-level content differs (raw vs compressed)");

    for q in QUERIES {
        let want = query(&mem, q).unwrap().rows.len();
        let got_raw = query(&raw, q).unwrap().rows.len();
        let got_comp = query(&comp, q).unwrap().rows.len();
        assert_eq!(want, got_raw, "raw query result differs for {q}");
        assert_eq!(want, got_comp, "compressed query result differs for {q}");
    }

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn one_pass_compressed_spill_build_matches_recompress() {
    use sparq_core::dictspill::SpillConfig;
    let nt = synthetic_nt(300);
    let base = std::env::temp_dir().join(format!("sparq_vkz7_spill_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let raw_dir = base.join("raw");
    let recompressed_dir = base.join("recompressed");
    let compressed_dir = base.join("compressed");
    // Tiny budget ⇒ many spill runs through the dict-spill external build's merge tail.
    let cfg = SpillConfig { mem_budget: 1, disk_floor: 0 };

    Graph::build_external_spill(nt.as_bytes(), "ntriples", &raw_dir, 128, &cfg).unwrap();
    Graph::open(&raw_dir).unwrap().save_compressed(&recompressed_dir).unwrap();
    sparq_core::with_build_compressed(true, || {
        Graph::build_external_spill(nt.as_bytes(), "ntriples", &compressed_dir, 128, &cfg).unwrap();
    });

    for f in dir_files(&compressed_dir) {
        let a = std::fs::read(compressed_dir.join(&f)).unwrap();
        let b = std::fs::read(recompressed_dir.join(&f)).unwrap();
        assert!(a == b, "{f}: one-pass spill compressed build differs from recompress");
    }

    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let comp = Graph::open(&compressed_dir).unwrap();
    assert_eq!(dump(&mem), dump(&comp), "spill: term-level content differs");
    for q in QUERIES {
        assert_eq!(query(&mem, q).unwrap().rows.len(), query(&comp, q).unwrap().rows.len(), "spill query differs for {q}");
    }
    std::fs::remove_dir_all(&base).ok();
}

// ===== sq-7d3dj.32.2.7 — SPQCPRM2 format migration cross-version differentials =====

/// sq-7d3dj.32.2.7 — the V2 emit config gate at the STORE level: `save_compressed`
/// under `with_emit_format(V2)` must write `SPQCPRM2` perms that AUTO-DETECT on `open` and answer
/// every query identically to the V1 save and the in-memory load. This is the migration's
/// end-to-end soundness: a V2-saved store is query-equivalent to a V1-saved store.
#[cfg(feature = "spqcprm2")]
#[test]
fn v2_saved_store_is_query_equivalent_to_v1() {
    use sparq_core::compress::{with_emit_format, EmitFormat};
    let nt = synthetic_nt(400); // > 128 rows per order ⇒ multi-block perms with reset_d1 rows
    let base = std::env::temp_dir().join(format!("sparq_spqcprm2_v2eq_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let v1_dir = base.join("v1");
    let v2_dir = base.join("v2");

    let g = Graph::load_str(&nt, "ntriples").unwrap();
    g.save_compressed(&v1_dir).unwrap(); // default emit ⇒ SPQCPRM1
    with_emit_format(EmitFormat::V2, || g.save_compressed(&v2_dir).unwrap());

    // At least one V2 perm must carry the SPQCPRM2 magic (else the gate did not engage).
    let saw_v2 = (0..6).any(|i| {
        std::fs::read(v2_dir.join(format!("perm{i}.bin")))
            .map(|b| b.starts_with(b"SPQCPRM2"))
            .unwrap_or(false)
    });
    assert!(saw_v2, "V2 save produced no SPQCPRM2 perm — the emit gate did not engage");
    // And the V1 dir must NOT carry any V2 magic (default stays SPQCPRM1).
    for i in 0..6 {
        if let Ok(b) = std::fs::read(v1_dir.join(format!("perm{i}.bin"))) {
            assert!(!b.starts_with(b"SPQCPRM2"), "default save wrote a SPQCPRM2 perm{i} — V2 leaked into the default");
        }
    }

    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let v1 = Graph::open(&v1_dir).unwrap();
    let v2 = Graph::open(&v2_dir).unwrap();
    assert_eq!(v2.len(), mem.len(), "V2 triple count differs");
    assert_eq!(dump(&v2), dump(&mem), "V2 term-level content differs from in-memory");
    assert_eq!(dump(&v2), dump(&v1), "V2 term-level content differs from V1");
    for q in QUERIES {
        let want = query(&mem, q).unwrap().rows.len();
        assert_eq!(query(&v1, q).unwrap().rows.len(), want, "V1 query differs for {q}");
        assert_eq!(query(&v2, q).unwrap().rows.len(), want, "V2 query differs for {q}");
    }
    std::fs::remove_dir_all(&base).ok();
}

/// sq-7d3dj.32.2.7 — V1→V2→V1 and V2→V1→V2 cross-version round-trips at the STORE
/// level: opening a store in one format and RE-SAVING it in the other must preserve the exact
/// logical content through every hop. This is the format-conversion soundness a real migration
/// tool relies on (a V1 store can be rewritten V2 and back with no drift).
#[cfg(feature = "spqcprm2")]
#[test]
fn cross_version_resave_round_trips_losslessly() {
    use sparq_core::compress::{with_emit_format, EmitFormat};
    let nt = synthetic_nt(350);
    let base = std::env::temp_dir().join(format!("sparq_spqcprm2_xver_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let d1 = base.join("v1");
    let d2 = base.join("v2");
    let d3 = base.join("v1-again");
    let d4 = base.join("v2-again");

    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let reference = dump(&mem);

    // V1 → open → V2.
    mem.save_compressed(&d1).unwrap();
    let g_v1 = Graph::open(&d1).unwrap();
    assert_eq!(dump(&g_v1), reference, "V1 save/open lost content");
    with_emit_format(EmitFormat::V2, || g_v1.save_compressed(&d2).unwrap());
    let g_v2 = Graph::open(&d2).unwrap();
    assert_eq!(dump(&g_v2), reference, "V1→V2 resave/open lost content");

    // V2 → open → V1 (back to the shipped format).
    g_v2.save_compressed(&d3).unwrap(); // default emit ⇒ V1
    let g_v1b = Graph::open(&d3).unwrap();
    assert_eq!(dump(&g_v1b), reference, "V2→V1 resave/open lost content");
    for i in 0..6 {
        if let Ok(b) = std::fs::read(d3.join(format!("perm{i}.bin"))) {
            assert!(!b.starts_with(b"SPQCPRM2"), "V2→V1 resave wrote a SPQCPRM2 perm{i}");
        }
    }

    // V1 → open → V2 once more (V2→V1→V2 leg): the full cycle is stable.
    with_emit_format(EmitFormat::V2, || g_v1b.save_compressed(&d4).unwrap());
    let g_v2b = Graph::open(&d4).unwrap();
    assert_eq!(dump(&g_v2b), reference, "V1→V2 (second cycle) lost content");
    // Query parity across the whole cycle.
    for q in QUERIES {
        let want = query(&mem, q).unwrap().rows.len();
        assert_eq!(query(&g_v2b, q).unwrap().rows.len(), want, "cross-version cycle query differs for {q}");
    }
    std::fs::remove_dir_all(&base).ok();
}

/// The override is per-thread and OFF by default: a build run WITHOUT `with_build_compressed`
/// (and without the env var) must still write RAW perms, so the default behaviour is intact.
#[test]
fn default_build_stays_raw() {
    let nt = synthetic_nt(200);
    let dir = std::env::temp_dir().join(format!("sparq_vkz7_default_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &dir, 256, true).unwrap();
    // perm0 (SPO) is non-empty here and must be RAW (no SPQCPRM1 magic).
    let spo = std::fs::read(dir.join("perm0.bin")).unwrap();
    assert!(!spo.is_empty());
    assert!(!spo.starts_with(b"SPQCPRM1"), "default build must write RAW perms");
    std::fs::remove_dir_all(&dir).ok();
}
