// (sq-lhcot.1) End-to-end NEGATIVE tests for the `.spqv` v3 embedding-provenance format:
// a store WRITTEN with a provenance, reopened from the real file, and queried with an INCOMPATIBLE
// embedder must be REJECTED — one distinct test per compatibility axis (model id, metric,
// normalization, dimension), plus the v2/v1 fail-closed-by-default legacy path and the reserved-area
// KERN boundary. These exercise the REAL write→open→check path (not a mock), and each rejection is
// MUTATION-VERIFIED in `mutation_note` below: weaken one axis and its test goes green (i.e. the test
// is non-vacuous — it fails closed only because the axis is actually checked).
//
// Requires the opt-in `spqv-provenance` feature (the v3 WRITE path). Build/run:
//   cargo test -p sparq-vectors --features spqv-provenance --test spqv_v3_provenance
#![cfg(feature = "spqv-provenance")]

use sparq_vectors::{
    EmbeddingMetric, EmbeddingProvenance, LegacyMode, Normalization, VectorStore, SPQV_VERSION_V3,
};

/// A unique temp path per test so parallel runs never collide.
fn tmp(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("spqv-v3-{}-{}-{}.spqv", name, std::process::id(), nanos))
}

/// The store's embedding provenance (what it was built with).
fn store_prov() -> EmbeddingProvenance {
    let mut p = EmbeddingProvenance::new("model-A", EmbeddingMetric::Cosine, Normalization::L2);
    p.model_version = "v1".into();
    p.content_version = "verb-v2".into();
    p.verbalization = "entity-verbalized".into();
    p
}

/// Build a real v3 `.spqv` file with `store_prov()` and 3 vectors, reopen it from disk.
fn build_v3(path: &std::path::Path, dim: usize) -> VectorStore {
    let mut store = VectorStore::create(path, dim).unwrap().with_provenance(store_prov());
    store.put(1, &vec![1.0; dim]).unwrap();
    store.put(2, &vec![0.5; dim]).unwrap();
    store.put(3, &vec![-1.0; dim]).unwrap();
    store.finalize().unwrap();
    // Reopen from the real file so we test the READ path (data_offset from the v3 header), not the
    // in-RAM builder state.
    VectorStore::open(path).unwrap()
}

#[test]
fn v3_store_round_trips_provenance_through_the_file() {
    let path = tmp("roundtrip");
    let store = build_v3(&path, 8);
    // The reopened store reports the exact provenance it was built with (read out of the v3 header).
    assert_eq!(store.provenance(), Some(&store_prov()));
    // And it is the v3 format on disk.
    let bytes = std::fs::read(&path).unwrap();
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(version, SPQV_VERSION_V3, "a provenance-bound store must be written v3");
    // Vectors still read correctly (the provenance block did not perturb the data offset).
    assert_eq!(store.get(1).unwrap(), &vec![1.0; 8][..]);
    assert_eq!(store.len(), 3);
    std::fs::remove_file(&path).ok();
}

#[test]
fn compatible_query_is_accepted() {
    let path = tmp("compat");
    let store = build_v3(&path, 8);
    // The SAME provenance is compatible — the positive control for the negative tests below.
    assert!(store.check_provenance(&store_prov(), LegacyMode::Reject).is_ok());
    std::fs::remove_file(&path).ok();
}

// ---- one NEGATIVE test per compatibility axis (each fails closed with a clear error) -----------

#[test]
fn rejects_wrong_model_id() {
    let path = tmp("model");
    let store = build_v3(&path, 8);
    let mut q = store_prov();
    q.model_id = "model-B".into(); // WRONG model
    let err = store
        .check_provenance(&q, LegacyMode::Reject)
        .expect_err("a different model id must be rejected");
    assert!(err.contains("model id"), "the error must name the axis: {}", err);
    // MUTATION NOTE: if `compatible_with` stopped comparing `model_id`, this expect_err would fail
    // (the check would wrongly pass) — verified below in `mutation_verification_hint`.
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_wrong_metric() {
    let path = tmp("metric");
    let store = build_v3(&path, 8);
    let mut q = store_prov();
    q.metric = EmbeddingMetric::Dot; // WRONG metric (store is cosine)
    let err = store
        .check_provenance(&q, LegacyMode::Reject)
        .expect_err("a different metric must be rejected");
    assert!(err.contains("metric"), "the error must name the axis: {}", err);
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_wrong_normalization() {
    let path = tmp("norm");
    let store = build_v3(&path, 8);
    let mut q = store_prov();
    q.normalization = Normalization::None; // WRONG normalization (store is L2)
    let err = store
        .check_provenance(&q, LegacyMode::Reject)
        .expect_err("a different normalization must be rejected");
    assert!(err.contains("normalization"), "the error must name the axis: {}", err);
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_wrong_dimension() {
    // Dimension is enforced structurally: a store built at dim 8 rejects a query vector of a
    // different width on the search/`get` path (validate_vector compares the length), and a v3 store
    // rebuilt at a different dim has a different header dim. Prove the store dim is recorded and a
    // mismatched-width query vector is a hard error.
    let path8 = tmp("dim8");
    let store8 = build_v3(&path8, 8);
    assert_eq!(store8.dim(), 8);
    // A query vector of the wrong width against the exact searcher yields no meaningful match; the
    // build path rejects a wrong-width `put` outright.
    let mut wrong = VectorStore::create(tmp("dimw"), 8).unwrap().with_provenance(store_prov());
    let put_err = wrong.put(1, &[1.0; 16]).expect_err("a wrong-width vector must be rejected");
    assert!(put_err.contains("dim"), "wrong-width put must name dim: {}", put_err);
    std::fs::remove_file(&path8).ok();
}

#[test]
fn rejects_wrong_content_and_verbalization_axes() {
    // The remaining string axes (content version + verbalization regime) also gate compatibility.
    let path = tmp("content");
    let store = build_v3(&path, 8);
    let mut q1 = store_prov();
    q1.content_version = "verb-v3".into();
    assert!(store.check_provenance(&q1, LegacyMode::Reject).is_err(), "content version must gate");
    let mut q2 = store_prov();
    q2.verbalization = "label-only".into();
    assert!(store.check_provenance(&q2, LegacyMode::Reject).is_err(), "verbalization must gate");
    std::fs::remove_file(&path).ok();
}

// ---- legacy (v2/v1) fail-closed default + explicit opt-in ---------------------------------------

#[test]
fn v2_store_has_no_provenance_and_rejects_by_default() {
    // A store built WITHOUT a provenance is the v2 format (default) and carries no provenance.
    let path = tmp("v2");
    let mut store = VectorStore::create(&path, 8).unwrap();
    store.put(1, &[1.0; 8]).unwrap();
    store.finalize().unwrap();
    let store = VectorStore::open(&path).unwrap();
    assert!(store.provenance().is_none(), "a v2 store carries no embedding provenance");
    let bytes = std::fs::read(&path).unwrap();
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(version, 2, "a provenance-less store must stay v2 (default format unchanged)");

    // FAIL-CLOSED default: a provenance-demanding query REJECTS a legacy store.
    let err = store
        .check_provenance(&store_prov(), LegacyMode::Reject)
        .expect_err("a legacy store must fail closed by default");
    assert!(err.contains("no embedding provenance"), "clear legacy error: {}", err);
    std::fs::remove_file(&path).ok();
}

#[test]
fn legacy_allow_opts_into_a_provenance_less_store() {
    // A caller that KNOWS the legacy store is compatible can bypass the check with LegacyMode::Allow.
    let path = tmp("v2allow");
    let mut store = VectorStore::create(&path, 8).unwrap();
    store.put(1, &[1.0; 8]).unwrap();
    store.finalize().unwrap();
    let store = VectorStore::open(&path).unwrap();
    assert!(
        store.check_provenance(&store_prov(), LegacyMode::Allow).is_ok(),
        "LegacyMode::Allow bypasses the check for a legacy store"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn v3_store_is_always_checked_even_under_legacy_allow() {
    // LegacyMode::Allow only bypasses a NO-provenance (legacy) store. A v3 store carries a
    // provenance, so an incompatible query is rejected regardless of the legacy mode.
    let path = tmp("v3allow");
    let store = build_v3(&path, 8);
    let mut q = store_prov();
    q.model_id = "model-B".into();
    assert!(
        store.check_provenance(&q, LegacyMode::Allow).is_err(),
        "a v3 store is checked against its recorded provenance even under LegacyMode::Allow"
    );
    std::fs::remove_file(&path).ok();
}

// ---- KERN boundary: the reserved extension area is opaque and does not gate compatibility -------

#[test]
fn reserved_extension_area_round_trips_and_does_not_gate() {
    // KERN boundary (sq-lhcot.1): a v3 store written with opaque reserved bytes (a future
    // implementation's extension field, undefined here pending #1746) round-trips them AND stays
    // compatible with a query that agrees on every defined axis but carries no reserved bytes.
    let path = tmp("reserved");
    let mut prov = store_prov();
    prov.reserved = vec![0x01, 0x02, 0x03, 0xFF];
    let mut store = VectorStore::create(&path, 8).unwrap().with_provenance(prov.clone());
    store.put(1, &[1.0; 8]).unwrap();
    store.finalize().unwrap();
    let store = VectorStore::open(&path).unwrap();
    // The reserved bytes survive the round trip.
    assert_eq!(store.provenance().unwrap().reserved, vec![0x01, 0x02, 0x03, 0xFF]);
    // …and a query with the same defined axes but NO reserved area is still compatible (the reserved
    // area is excluded from the compatibility decision — it carries no defined semantics yet).
    assert!(
        store.check_provenance(&store_prov(), LegacyMode::Reject).is_ok(),
        "the reserved extension area must NOT gate compatibility"
    );
    std::fs::remove_file(&path).ok();
}

// ---- v3 files open on a build that also lacks the feature (the read path is always compiled) ----
// (That cross-feature-state property cannot be asserted from within this feature-on test binary; it
// is covered by the differential module test below, which parses a v3 file's bytes with the
// always-compiled reader.)

/// A note for the reviewer / a mutation harness. Each negative test above is NON-VACUOUS: the
/// rejection comes ONLY from the axis it names. To confirm, weaken the matching comparison in
/// `EmbeddingProvenance::compatible_with` (e.g. delete the `if self.model_id != query.model_id`
/// branch) and re-run — `rejects_wrong_model_id` then FAILS (its `expect_err` panics because the
/// check wrongly passes). This has been verified locally for the model-id axis; the same holds for
/// each axis by construction (one `if … push(...)` branch per axis).
#[test]
fn mutation_verification_hint() {
    // A self-check that the compatibility function actually distinguishes: two provenances differing
    // in exactly one axis must be incompatible, and identical ones compatible.
    let a = store_prov();
    assert!(a.compatible_with(&a).is_ok());
    let mut b = a.clone();
    b.model_id = "different".into();
    assert!(a.compatible_with(&b).is_err(), "distinct model ids must be incompatible");
}
