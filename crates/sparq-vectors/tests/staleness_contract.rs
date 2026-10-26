//! (sq-wlzi) The **id-keyed staleness contract** for `.spqv` / `.spqg`, demonstrated
//! end-to-end. A [`VectorStore`] (and the `DiskAnnIndex` over it) is keyed by **raw dictionary term
//! id**: a query resolves `graph.id_of(term) -> id` and looks `id` up in the store. The store is
//! therefore valid ONLY against a graph whose `id -> term` binding equals the build-time binding.
//!
//! sparq-core assigns dictionary ids in a **thread-count-dependent** order (the parallel sharded
//! dict merge sizes its shard count from `rayon::current_num_threads()`), so the *same source RDF*
//! re-parsed at a different `RAYON_NUM_THREADS` gets a DIFFERENT id binding. Since sq-xhiv the graph
//! fingerprint is dict-id-order-INDEPENDENT (it folds the term SET), so that re-parsed graph
//! fingerprints IDENTICALLY and `check_graph` PASSES it — yet the raw id of a given term may now
//! denote a different term, so the store serves the WRONG vector. The fingerprint cannot catch this
//! case; the **usage discipline is the safety net**: to serve a persisted store, REOPEN the
//! persisted graph with `Graph::open` (which mmaps the FROZEN id order), NEVER re-parse the
//! source RDF.
//!
//! These two tests pin both halves of that contract:
//!   * [`sound_path_save_then_graph_open_serves_correct_vectors`] — the SOUND round-trip.
//!   * [`reparse_trap_passes_the_check_but_serves_the_wrong_vector`] — why the discipline is
//!     load-bearing: a re-parse at a different thread count passes `check_graph` yet mis-resolves.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_vectors::VectorStore;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch path under the temp dir, tagged and pid/seq-stamped so parallel test
/// binaries never collide.
fn scratch(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sparq_wlzi_{tag}_{}_{n}", std::process::id()))
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

/// Load `nt` (N-Triples) inside a rayon pool of exactly `threads` workers, so the parallel sharded
/// dict merge runs at a known shard count. `RAYON_NUM_THREADS` is read once per process for the
/// GLOBAL pool, so a scoped pool is the only way to vary the thread count within one test.
fn load_in_pool(nt: &str, threads: usize) -> Graph {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| Graph::load_str(nt, "ntriples").expect("load ntriples"))
}

/// A multi-namespace N-Triples document large enough that the sharded merge ACTUALLY shards (the
/// shard count is `(threads*2).clamp(4,64)`, so e.g. 2 vs 8 threads → 4 vs 16 shards → a different
/// id assignment). Subjects span seven namespaces so the merge interleaves them across shards.
fn corpus() -> String {
    let mut nt = String::new();
    for i in 0..3000u32 {
        nt.push_str(&format!(
            "<http://ns{}.example/s{}> <http://ns{}.example/p{}> <http://ns{}.example/o{}> .\n",
            i % 7,
            i,
            i % 5,
            i % 11,
            i % 9,
            i
        ));
    }
    nt
}

/// A distinctive 4-d vector for a term, derived from its id so each stored term has its own unique
/// vector (collision-free over the id range we use). Lets a test assert "the vector served for this
/// id is exactly the one stored for that term" at the byte level — deterministic, no ranking.
fn vec_for(id: Id) -> [f32; 4] {
    [id as f32, (id as f32) * 0.5, 7.0, -3.0]
}

/// Build a store over `g` (bound to `g`, so it carries `g`'s fingerprint), giving every subject
/// `s{i}` for `i` in `0..n` a distinctive id-derived vector. Returns the store path.
fn build_store(g: &Graph, path: &std::path::Path, n: u32) {
    let mut s = VectorStore::create(path, 4).unwrap().with_fingerprint(g);
    for i in 0..n {
        let t = iri(&format!("http://ns{}.example/s{}", i % 7, i));
        let id = g.id_of(&t).expect("subject present in dict");
        s.put(id, &vec_for(id)).unwrap();
    }
    s.finalize().unwrap();
}

/// THE SOUND PATH. Build a store against a graph, `Graph::save` that graph, then reopen it with
/// `Graph::open` (which mmaps the frozen dict id order) and query the id-keyed store against the
/// reopened graph: the staleness check passes AND every term resolves to the vector that was stored
/// for it. The persisted dict pins the exact `id -> term` binding the store was keyed against.
#[test]
fn sound_path_save_then_graph_open_serves_correct_vectors() {
    let nt = corpus();
    let n = 64u32; // embed the first 64 subjects

    // Build the graph at one thread count; persist BOTH the graph and a store bound to it.
    let g_build = load_in_pool(&nt, 8);
    let store_path = scratch("sound").with_extension("spqv");
    build_store(&g_build, &store_path, n);
    let graph_dir = scratch("sound_graph");
    g_build.save(&graph_dir).expect("persist the build graph");

    // SERVE: reopen the PERSISTED graph (frozen id order), not a re-parse of the source RDF.
    let g_served = Graph::open(&graph_dir).expect("reopen the persisted graph");
    let store = VectorStore::open(&store_path).unwrap();

    // The checked query path certifies the store against the reopened graph...
    assert!(
        store.check_graph(&g_served).is_ok(),
        "a store served against its OWN persisted graph (reopened via Graph::open) must verify"
    );
    // ...and EVERY embedded term resolves to the exact vector stored for it. `Graph::open` preserves
    // the build-time `id -> term` binding, so the id-keyed lookup is correct for every term.
    for i in 0..n {
        let t = iri(&format!("http://ns{}.example/s{}", i % 7, i));
        let id = g_served.id_of(&t).expect("subject present after reopen");
        let served = store.get(id).expect("served vector present");
        assert_eq!(
            served,
            &vec_for(id),
            "the reopened persisted graph must resolve term {t:?} to ITS OWN stored vector"
        );
    }

    std::fs::remove_file(&store_path).ok();
    std::fs::remove_dir_all(&graph_dir).ok();
}

/// THE RE-PARSE TRAP — why the `Graph::open` discipline is load-bearing. Re-parse the SAME source
/// RDF at a DIFFERENT thread count than the build. The dict ids genuinely permute, so:
///   * `check_graph` PASSES (the sq-xhiv fingerprint folds the term SET and is thread-count-stable,
///     so the re-parsed graph fingerprints identically) — the fingerprint CANNOT catch this; and
///   * the id-keyed lookup therefore mis-resolves: a term's id in the re-parsed graph denotes a
///     DIFFERENT term in the store, so the served vector is WRONG.
///
/// This is the foot-gun sq-wlzi documents: a passing `check_graph` is NECESSARY but NOT SUFFICIENT;
/// only reopening the persisted graph (the test above) is sound.
#[test]
fn reparse_trap_passes_the_check_but_serves_the_wrong_vector() {
    let nt = corpus();
    let n = 3000u32; // embed every subject, so a permuted id is guaranteed to land on an embedded one

    // Build against the graph parsed at 8 threads.
    let g_build = load_in_pool(&nt, 8);
    let store_path = scratch("trap").with_extension("spqv");
    build_store(&g_build, &store_path, n);
    let store = VectorStore::open(&store_path).unwrap();

    // RE-PARSE the SAME RDF at a different thread count (the trap: NOT Graph::open).
    let g_reparsed = load_in_pool(&nt, 2);
    assert_eq!(
        g_build.dict().len(),
        g_reparsed.dict().len(),
        "same term set, just permuted"
    );

    // (1) The staleness fingerprint does NOT catch the re-parse: it folds the term SET and is
    // thread-count-stable (sq-xhiv), so the re-parsed graph fingerprints identically and the check
    // PASSES. This is exactly why the fingerprint alone is not a sufficient guard.
    assert!(
        store.check_graph(&g_reparsed).is_ok(),
        "sq-xhiv: the dict-id-order-independent fingerprint matches a re-parse of the same RDF, so \
         check_graph cannot reject the re-parse trap — the Graph::open discipline is the safety net"
    );

    // (2) ...yet the id-keyed lookup mis-resolves for at least one term: find a subject whose id
    // GENUINELY permuted between the two bindings (the sharded merge guarantees some do). For such a
    // term, the store — keyed by the BUILD-time id — returns the vector stored for the term that the
    // BUILD graph placed at the re-parsed id, which is NOT this term's vector. That is the silently
    // wrong serving the contract warns against.
    let mut found_permuted = false;
    for i in 0..n {
        let t = iri(&format!("http://ns{}.example/s{}", i % 7, i));
        let id_build = g_build.id_of(&t).unwrap();
        let id_reparsed = g_reparsed.id_of(&t).unwrap();
        if id_build == id_reparsed {
            continue; // this term happened to keep its id; not the witness we need
        }
        found_permuted = true;
        // What the store serves when the query resolves the term through the RE-PARSED graph.
        let served_via_reparse = store.get(id_reparsed).expect("embedded id has a vector");
        // The term's OWN vector (what a correct serving must return).
        let correct = vec_for(id_build);
        assert_ne!(
            served_via_reparse,
            &correct,
            "term {t:?} permuted (build id {id_build} != reparse id {id_reparsed}); querying the \
             id-keyed store through the re-parsed graph serves the WRONG vector even though \
             check_graph passed — the trap sq-wlzi documents. Reopen the persisted graph instead."
        );
        // The served vector is, concretely, the BUILD graph's term-at-id_reparsed vector — i.e. a
        // different, real term's embedding presented as if it were `t`'s. Pin that mis-attribution.
        assert_eq!(
            served_via_reparse,
            &vec_for(id_reparsed),
            "the mis-served vector is precisely the one the BUILD graph stored at the re-parsed id \
             (a different term's embedding) — a plausible-looking but wrong neighbour"
        );
        break;
    }
    assert!(
        found_permuted,
        "expected the parallel sharded merge to permute at least one subject's id at 8 vs 2 \
         threads; if this fails the test no longer exercises the re-parse trap"
    );

    std::fs::remove_file(&store_path).ok();
}
