//! (sq-7e50) Persisted on-disk delta-sidecar (`.spqd`) tests for `VectorStore`:
//! result-equivalence of `open_with_delta(base + persisted-delta)` against the in-RAM (base + delta)
//! view AND against `compact()`; the crash / partial-write guard (a truncated sidecar is detected
//! and rejected, never UB); and the base-fingerprint-mismatch rejection (the sq-32i5 generation tie
//! enforced through the persisted header).
//!
//! Gated on the opt-in `delta` feature — with it off the file compiles to an empty test crate (the
//! save/open APIs do not exist), mirroring `tests/delta.rs`. Style matches the crate's other test
//! modules (the workspace's intentionally-deferred reformat keeps a compact hand-written layout).
#![cfg(feature = "delta")]

use sparq_core::Graph;
use sparq_vectors::{nearest_exact, Fingerprint, VectorStore, SPQD_MAGIC, SPQD_VERSION};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sparq_dpersist_{tag}_{}_{n}.spqv", std::process::id()))
}

fn graph(ttl: &str) -> Graph {
    Graph::load_str(ttl, "turtle").expect("load test turtle")
}

fn id_of(g: &Graph, s: &str) -> u32 {
    use oxrdf::{NamedNode, Term};
    g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap())).unwrap()
}

const A: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:alice ex:knows ex:bob .
    ex:bob ex:knows ex:carol .
    ex:carol ex:knows ex:dave .
"#;
// B has a DIFFERENT dictionary AND a different triple count — a different graph generation.
const B: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:eve ex:likes ex:frank .
"#;

/// Builds a finalized 4-d store over graph A with a few base vectors.
fn build_finalized(g: &Graph, path: &Path) -> VectorStore {
    let mut s = VectorStore::create(path, 4).unwrap().with_fingerprint(g);
    s.put(id_of(g, "http://example.org/alice"), &[1.0, 0.0, 0.0, 0.0]).unwrap();
    s.put(id_of(g, "http://example.org/bob"), &[0.9, 0.1, 0.0, 0.0]).unwrap();
    s.put(id_of(g, "http://example.org/carol"), &[0.0, 0.0, 0.0, 1.0]).unwrap();
    s.finalize().unwrap();
    s
}

/// The full effective view of a store as a sorted `(id, vector)` vec — the comparison key for
/// result-equivalence across the in-RAM, persisted, and compacted forms.
fn effective(store: &VectorStore) -> Vec<(u32, Vec<f32>)> {
    let mut v: Vec<(u32, Vec<f32>)> = store.iter().map(|(id, vec)| (id, vec.to_vec())).collect();
    v.sort_by_key(|&(id, _)| id);
    v
}

/// Asserts an open result is an `Err` and returns its message. (`VectorStore` is not `Debug`, so
/// `Result::expect_err` does not apply directly — `match` on the result instead.)
fn err_of(r: Result<VectorStore, String>, ctx: &str) -> String {
    match r {
        Ok(_) => panic!("expected an error but the open succeeded: {ctx}"),
        Err(e) => e,
    }
}

// ============================ gate: result-equivalence (the REQUIRED test) ============================

#[test]
fn open_with_persisted_delta_equals_in_ram_view_and_compact() {
    let g = graph(A);
    let base_path = tmp("eq-base");
    let mut store = build_finalized(&g, &base_path);

    let alice = id_of(&g, "http://example.org/alice");
    let bob = id_of(&g, "http://example.org/bob");
    let carol = id_of(&g, "http://example.org/carol");
    let dave = id_of(&g, "http://example.org/dave");

    // A mix of mutations against the finalized base: add a new id, remove a base id, update a base id.
    store.add(dave, &[0.2, 0.8, 0.0, 0.0]).unwrap();
    assert!(store.remove(bob));
    store.update(carol, &[0.0, 0.0, 1.0, 0.0]).unwrap();
    assert!(store.has_delta());

    // (1) The IN-RAM (base + delta) view — the live handle, before persisting.
    let in_ram = effective(&store);
    // Sanity: bob gone, dave present, carol updated, alice base.
    assert_eq!(store.get(bob), None);
    assert_eq!(store.get(dave), Some(&[0.2f32, 0.8, 0.0, 0.0][..]));
    assert_eq!(store.get(carol), Some(&[0.0f32, 0.0, 1.0, 0.0][..]));
    assert_eq!(store.get(alice), Some(&[1.0f32, 0.0, 0.0, 0.0][..]));

    // Persist the delta to its sibling .spqd (crash-durable tmp + fsync + rename).
    let delta_path = store.save_delta().unwrap();
    assert_eq!(delta_path, VectorStore::sibling_delta_path(&base_path));
    assert_eq!(delta_path.extension().unwrap(), "spqd");
    assert!(VectorStore::has_persisted_delta(&base_path));

    // The persisted header must be a well-formed .spqd.
    let raw = std::fs::read(&delta_path).unwrap();
    assert_eq!(&raw[0..4], &SPQD_MAGIC);
    assert_eq!(u32::from_le_bytes(raw[4..8].try_into().unwrap()), SPQD_VERSION);

    // DROP the live handle — the in-RAM delta is gone; only the .spqv base + .spqd sidecar remain.
    drop(store);

    // (2) Reopen base + persisted delta from disk (simulating a process restart).
    let reopened = VectorStore::open_with_delta(&base_path).unwrap();
    assert!(reopened.has_delta(), "the persisted delta was replayed onto the reopened base");
    let persisted = effective(&reopened);

    // (3) compact() over the reopened (base + persisted delta) → a fresh from-scratch-equivalent base.
    let compact_path = tmp("eq-compact");
    let compacted = reopened.compact(&compact_path, &g).unwrap();
    assert!(!compacted.has_delta(), "the compacted base carries no delta");
    let compacted_view = effective(&compacted);

    // THE EQUIVALENCE: open(base + persisted-delta) == in-RAM(base + delta) == compact().
    assert_eq!(persisted, in_ram, "persisted view must equal the in-RAM view");
    assert_eq!(compacted_view, in_ram, "compacted view must equal the in-RAM view");

    // len and every get agree across all three; the tombstone really took.
    assert_eq!(reopened.len(), in_ram.len());
    assert_eq!(compacted.len(), in_ram.len());
    assert_eq!(reopened.get(bob), None);
    assert_eq!(compacted.get(bob), None);

    // A search agrees byte-for-byte between the persisted-reopen and the compacted base.
    let q = [0.1f32, 0.9, 0.0, 0.0];
    assert_eq!(nearest_exact(&reopened, &q, 3), nearest_exact(&compacted, &q, 3));

    std::fs::remove_file(&base_path).ok();
    std::fs::remove_file(&delta_path).ok();
    std::fs::remove_file(&compact_path).ok();
}

#[test]
fn open_with_delta_with_no_sidecar_is_the_bare_base() {
    // No .spqd on disk → open_with_delta is exactly open(): the bare base, no error, no delta.
    let g = graph(A);
    let base_path = tmp("nosidecar");
    let store = build_finalized(&g, &base_path);
    let base_view = effective(&store);
    drop(store);

    assert!(!VectorStore::has_persisted_delta(&base_path));
    let reopened = VectorStore::open_with_delta(&base_path).unwrap();
    assert!(!reopened.has_delta(), "no sidecar ⇒ no delta");
    assert_eq!(effective(&reopened), base_view);
    std::fs::remove_file(&base_path).ok();
}

#[test]
fn empty_delta_round_trips_and_still_records_the_generation() {
    // A store with NO mutations persists an empty (header-only) sidecar that still carries the base
    // generation — replaying it is a no-op, and a stale base is still caught (see the mismatch test).
    let g = graph(A);
    let base_path = tmp("empty");
    let store = build_finalized(&g, &base_path);
    let base_view = effective(&store);
    let delta_path = store.save_delta().unwrap();
    drop(store);

    // The sidecar exists and is header-only (no appends, no tombstones).
    let raw = std::fs::read(&delta_path).unwrap();
    assert_eq!(u64::from_le_bytes(raw[12..20].try_into().unwrap()), 0, "no appends");
    assert_eq!(u64::from_le_bytes(raw[20..28].try_into().unwrap()), 0, "no tombstones");

    let reopened = VectorStore::open_with_delta(&base_path).unwrap();
    assert_eq!(effective(&reopened), base_view, "an empty delta replays to the bare base view");
    std::fs::remove_file(&base_path).ok();
    std::fs::remove_file(&delta_path).ok();
}

// ============================ gate: crash / partial-write detection ============================

#[test]
fn a_truncated_sidecar_is_rejected_not_ub() {
    let g = graph(A);
    let base_path = tmp("trunc-base");
    let mut store = build_finalized(&g, &base_path);
    let dave = id_of(&g, "http://example.org/dave");
    store.add(dave, &[0.3, 0.7, 0.0, 0.0]).unwrap();
    let delta_path = store.save_delta().unwrap();
    drop(store);

    let full = std::fs::read(&delta_path).unwrap();
    assert!(full.len() > 56, "the sidecar has a body to truncate");

    // (a) Truncate mid-body (a torn write of the append section): the header still claims an append
    // whose bytes are now missing → the exact-length check rejects it (no out-of-bounds read).
    std::fs::write(&delta_path, &full[..full.len() - 4]).unwrap();
    let err = err_of(VectorStore::open_with_delta(&base_path), "truncated mid-body");
    assert!(err.contains("truncated") || err.contains("length mismatch"), "err was: {err}");

    // (b) Truncate inside the header itself: also rejected with a clear message.
    std::fs::write(&delta_path, &full[..40]).unwrap();
    let err = err_of(VectorStore::open_with_delta(&base_path), "torn header");
    assert!(err.contains("truncated") || err.contains("length mismatch"), "err was: {err}");

    // (c) Trailing garbage (a long file) is also rejected — the length must match EXACTLY.
    let mut longer = full.clone();
    longer.extend_from_slice(&[0u8; 8]);
    std::fs::write(&delta_path, &longer).unwrap();
    let err = err_of(VectorStore::open_with_delta(&base_path), "trailing garbage");
    assert!(err.contains("length mismatch"), "err was: {err}");

    // (d) A corrupt magic is rejected (not mistaken for a valid sidecar).
    let mut bad_magic = full.clone();
    bad_magic[0] = b'X';
    std::fs::write(&delta_path, &bad_magic).unwrap();
    let err = err_of(VectorStore::open_with_delta(&base_path), "bad magic");
    assert!(err.contains("bad magic"), "err was: {err}");

    std::fs::remove_file(&base_path).ok();
    std::fs::remove_file(&delta_path).ok();
}

// ============================ gate: base-fingerprint-mismatch rejection ============================

#[test]
fn a_persisted_delta_is_rejected_against_a_mismatched_base() {
    // A .spqd written against generation A must NOT replay onto a base of generation B (its ids
    // would mis-key). The persisted header carries the base generation; open_with_delta routes it
    // through apply_delta, whose sq-32i5 generation tie rejects the mismatch.
    let ga = graph(A);
    let gb = graph(B);
    assert_ne!(Fingerprint::of(&ga), Fingerprint::of(&gb), "A and B must be distinct generations");

    // A store + persisted delta built against generation A.
    let a_path = tmp("mismatch-a");
    let mut store_a = build_finalized(&ga, &a_path);
    store_a.add(id_of(&ga, "http://example.org/dave"), &[0.3, 0.7, 0.0, 0.0]).unwrap();
    let delta_a_path = store_a.save_delta().unwrap();
    drop(store_a);

    // A SEPARATE base built against generation B, at a different path.
    let b_path = tmp("mismatch-b");
    let mut store_b = VectorStore::create(&b_path, 4).unwrap().with_fingerprint(&gb);
    store_b.put(id_of(&gb, "http://example.org/eve"), &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store_b.finalize().unwrap();
    drop(store_b);

    // Replaying the gen-A sidecar onto the gen-B base is a descriptive ERROR, not a silent mis-key.
    let err = err_of(
        VectorStore::open_with_delta_at(&b_path, &delta_a_path),
        "gen-A delta onto a gen-B base",
    );
    assert!(err.contains("generation mismatch"), "err was: {err}");

    // The same sidecar replays cleanly onto its OWN generation (gen A).
    let reopened_a = VectorStore::open_with_delta(&a_path).unwrap();
    assert_eq!(
        reopened_a.get(id_of(&ga, "http://example.org/dave")),
        Some(&[0.3f32, 0.7, 0.0, 0.0][..])
    );

    std::fs::remove_file(&a_path).ok();
    std::fs::remove_file(&delta_a_path).ok();
    std::fs::remove_file(&b_path).ok();
}

#[test]
fn a_dimension_mismatch_between_base_and_sidecar_is_rejected() {
    // Save a 4-d sidecar, then point open_with_delta_at at a base of a DIFFERENT dimension: the
    // dim check rejects it before any mis-shaped replay.
    let g = graph(A);
    let base4 = tmp("dim4");
    let mut store4 = build_finalized(&g, &base4);
    store4.add(id_of(&g, "http://example.org/dave"), &[0.3, 0.7, 0.0, 0.0]).unwrap();
    let delta4 = store4.save_delta().unwrap();
    drop(store4);

    // A 2-d base bound to the SAME generation (so only the dimension differs).
    let base2 = tmp("dim2");
    let mut store2 = VectorStore::create(&base2, 2).unwrap().with_fingerprint(&g);
    store2.put(id_of(&g, "http://example.org/alice"), &[1.0, 0.0]).unwrap();
    store2.finalize().unwrap();
    drop(store2);

    let err = err_of(VectorStore::open_with_delta_at(&base2, &delta4), "dim mismatch");
    assert!(err.contains("dimension"), "err was: {err}");

    std::fs::remove_file(&base4).ok();
    std::fs::remove_file(&delta4).ok();
    std::fs::remove_file(&base2).ok();
}

#[test]
fn save_is_atomic_a_preexisting_sidecar_survives_and_is_replaced_whole() {
    // save_delta writes tmp + fsync + rename, so the live .spqd is only ever a COMPLETE file.
    // Save one delta, then a second (different) delta, and confirm the on-disk sidecar is the
    // second one in full — never a mix.
    let g = graph(A);
    let base_path = tmp("atomic");
    let dave = id_of(&g, "http://example.org/dave");
    let carol = id_of(&g, "http://example.org/carol");

    let mut store = build_finalized(&g, &base_path);
    store.add(dave, &[0.3, 0.7, 0.0, 0.0]).unwrap();
    let delta_path = store.save_delta().unwrap();
    let first_len = std::fs::read(&delta_path).unwrap().len();

    // Mutate further and re-save: a different, longer delta replaces the first whole.
    store.remove(carol);
    store.save_delta().unwrap();
    let second = std::fs::read(&delta_path).unwrap();
    assert_ne!(second.len(), first_len, "the re-saved sidecar reflects the new mutation");

    // No stray tmp file is left behind.
    let mut tmp_path = delta_path.as_os_str().to_os_string();
    tmp_path.push("-tmp");
    assert!(!Path::new(&tmp_path).exists(), "the tmp file is renamed away, not left behind");

    drop(store);
    let reopened = VectorStore::open_with_delta(&base_path).unwrap();
    assert_eq!(reopened.get(dave), Some(&[0.3f32, 0.7, 0.0, 0.0][..]));
    assert_eq!(reopened.get(carol), None, "the second delta's tombstone is in effect");

    std::fs::remove_file(&base_path).ok();
    std::fs::remove_file(&delta_path).ok();
}
