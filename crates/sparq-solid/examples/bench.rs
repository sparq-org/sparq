//! The measured baseline (design doc §6: v1 rewrite path + v2 dataset-view path). Run:
//!     cargo run -p sparq-solid --example bench --release
//!
//! Measures, on the ~1.1k-graph WAC fixture (+ ACP variant):
//!   1. auth-view materialization (and re-materialization after an ACL change — v1's
//!      incremental-maintenance story IS a full re-run);
//!   2. per-query cost of the DEFAULT zero-copy dataset-view path (`query_as`) vs the
//!      kept v1 FROM-NAMED-copy path (`query_as_rewrite`) vs the engine on the full
//!      dataset directly — including each path's overhead isolated on a query whose
//!      pattern matches nothing;
//!   3. session graph-set computation, cold vs cached.
//!
//! Default-graph semantics (post PR #1593 / issue #1546): the v2 view path is
//! spec-conformant empty-by-default, so a *bare* default-graph pattern matches nothing
//! there unless the query opts in with
//! `FROM <http://www.w3.org/ns/solid/sparql#union-default-graph>`. The v1 rewrite path is
//! union-always by construction (`query_as_rewrite` always wraps default-graph patterns in
//! `GRAPH ?g` over the authorized set; it does not consume the opt-in IRI). So to keep the
//! rewrite-vs-zero-copy comparison apples-to-apples over the SAME union-default semantics,
//! this bench runs v1 with the bare `TITLES` query and v2 with `V2_UNION_QUERY` — the opt-in
//! `TITLES_UNION` on the default build, or bare `TITLES` under the `legacy-union-default-graph`
//! hatch (where a bare pattern is already union-always). Both then range over exactly the
//! authorized named graphs and return the same row count. On the default build a third
//! measurement records the v2 spec default (bare `TITLES` → 0 rows).

use sparq_core::Graph;
use sparq_solid::fixture::{acp_fixture, wac_fixture, ALICE};
use sparq_solid::{Mode, PodStore, Session};
use std::time::Instant;

/// Bare default-graph pattern. On v1 (`query_as_rewrite`) this is union-always over the
/// authorized graphs; on v2 (`query_as`) it is the spec-conformant empty default (0 rows).
const TITLES: &str = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";
/// The same query with the per-request union-default opt-in — makes v2's default graph the
/// RDF merge of the authorized named graphs, matching v1's union-always semantics
/// like-for-like for the perf comparison (issue #1546 `#union-default-graph`). Only used on
/// the spec-conformant default build (under the legacy hatch v2 is already union-always).
#[cfg(not(feature = "legacy-union-default-graph"))]
const TITLES_UNION: &str = "SELECT ?title FROM <http://www.w3.org/ns/solid/sparql#union-default-graph> \
     WHERE { ?s <https://ex.dev/ns#title> ?title }";

/// The v2 query that ranges over the authorized union, so its row count matches v1's
/// union-always rewrite for the apples-to-apples perf comparison. It is feature-dependent:
/// on the spec-conformant DEFAULT build a bare pattern is empty, so v2 must opt in with the
/// reserved IRI (`TITLES_UNION`); under the `legacy-union-default-graph` escape hatch a bare
/// pattern is already union-always (and the reserved-IRI opt-in is a no-op there — the
/// legacy rewrite does not consume it), so the bare `TITLES` is the union form.
#[cfg(not(feature = "legacy-union-default-graph"))]
const V2_UNION_QUERY: &str = TITLES_UNION;
#[cfg(feature = "legacy-union-default-graph")]
const V2_UNION_QUERY: &str = TITLES;

fn timed<T>(label: &str, mut f: impl FnMut() -> T) -> T {
    // best-of-3 for the multi-ms operations
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..3 {
        let t = Instant::now();
        out = Some(f());
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("{label:64} {best:10.2} ms");
    out.unwrap()
}

fn main() {
    let nq = wac_fixture();
    let graph = Graph::load_dataset(&nq, "nquads").unwrap();
    let n_graphs = graph.named.len();
    let n_quads: usize = graph.named.iter().map(|(_, g)| g.len()).sum();
    println!("WAC fixture: {n_graphs} named graphs, {n_quads} quads\n");

    let mut store = PodStore::new(graph);
    // 1. materialization + re-materialization (≈ the ACL-change cost)
    let stats = timed("WAC auth-view materialization (full pipeline)", || {
        store.materialize_wac().unwrap()
    });
    println!("    auth view: {} triples, closure {:?}\n", stats.auth_triples, stats.strata_facts);

    let acp_graph = Graph::load_dataset(&acp_fixture(), "nquads").unwrap();
    let mut acp_store = PodStore::new(acp_graph);
    let astats = timed("ACP auth-view materialization (3 strata)", || {
        acp_store.materialize_acp().unwrap()
    });
    println!("    auth view: {} triples, closure per stratum {:?}\n", astats.auth_triples, astats.strata_facts);

    // 2. per-query: v1 rewrite+copy vs full-dataset direct query
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let allowed = store.accessible(&alice, Mode::Read);
    println!("alice readable graphs: {}", allowed.len());
    let direct = timed("engine::query, FULL dataset (GRAPH ?g over all graphs)", || {
        sparq_engine::query(
            store.graph(),
            "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }",
        )
        .unwrap()
    });
    println!("    rows: {}", direct.rows.len());
    let v1 = timed("query_as_rewrite (v1: FROM NAMED build_active copy)", || {
        store.query_as_rewrite(&alice, Mode::Read, TITLES).unwrap()
    });
    println!("    rows: {} (authorized subset, union-always)", v1.rows.len());
    // v2 ranges over the same authorized union as v1's union-always rewrite — the
    // apples-to-apples perf comparison. On the default build that means opting in with the
    // reserved IRI; under the legacy hatch a bare pattern is already union-always.
    let v2 = timed("query_as (v2: zero-copy DatasetView, authorized union)", || {
        store.query_as(&alice, Mode::Read, V2_UNION_QUERY).unwrap()
    });
    println!("    rows: {} (authorized subset, union)", v2.rows.len());
    assert_eq!(v1.rows.len(), v2.rows.len(), "v1 union-always == v2 authorized union");
    // v2 spec default (bare default-graph pattern, no opt-in) is EMPTY per the Editor's Draft
    // (PR #1593): strictly more conservative than v1's union-always. Only holds on the
    // spec-conformant default build — the legacy hatch makes a bare pattern union-always.
    #[cfg(not(feature = "legacy-union-default-graph"))]
    {
        let v2_default = timed("query_as (v2 spec default: bare pattern, empty default graph)", || {
            store.query_as(&alice, Mode::Read, TITLES).unwrap()
        });
        println!("    rows: {} (empty default graph, spec-conformant)", v2_default.rows.len());
        assert_eq!(v2_default.rows.len(), 0, "spec-conformant empty default graph (#1546)");
    }
    // each path's overhead alone, isolated: same query, pattern matched nothing
    const NOTHING: &str = "SELECT ?x WHERE { ?x <urn:none> <urn:none> }";
    let nothing = timed("v1 copy cost isolated (rewritten query, empty pattern)", || {
        store.query_as_rewrite(&alice, Mode::Read, NOTHING).unwrap()
    });
    assert_eq!(nothing.rows.len(), 0);
    let vnothing = timed("v2 view overhead isolated (same empty pattern)", || {
        store.query_as(&alice, Mode::Read, NOTHING).unwrap()
    });
    assert_eq!(vnothing.rows.len(), 0);

    // 2b. copy-cost SCALING: same tree, 50 filler triples per document (~46k quads).
    //     build_active is O(authorized data) PER QUERY — this is the number the L1
    //     zero-copy view deletes.
    let fat = Graph::load_dataset(&sparq_solid::fixture::wac_fixture_sized(50), "nquads").unwrap();
    let fat_quads: usize = fat.named.iter().map(|(_, g)| g.len()).sum();
    let mut fat_store = PodStore::new(fat);
    fat_store.materialize_wac().unwrap();
    println!("\nfat fixture: {fat_quads} quads");
    let fat_direct = timed("FULL dataset query, fat fixture", || {
        sparq_engine::query(
            fat_store.graph(),
            "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }",
        )
        .unwrap()
    });
    println!("    rows: {}", fat_direct.rows.len());
    let fat_v1 = timed("query_as_rewrite (v1 copy path), fat fixture", || {
        fat_store.query_as_rewrite(&alice, Mode::Read, TITLES).unwrap()
    });
    println!("    rows: {} (union-always)", fat_v1.rows.len());
    // like-for-like: v2 over the authorized union matches v1's union-always semantics.
    let fat_v2 = timed("query_as (v2 view path, authorized union), fat fixture", || {
        fat_store.query_as(&alice, Mode::Read, V2_UNION_QUERY).unwrap()
    });
    println!("    rows: {} (union)", fat_v2.rows.len());
    assert_eq!(fat_v1.rows.len(), fat_v2.rows.len(), "v1 union-always == v2 authorized union (fat)");
    let fat_nothing = timed("v1 copy cost isolated, fat fixture", || {
        fat_store.query_as_rewrite(&alice, Mode::Read, NOTHING).unwrap()
    });
    assert_eq!(fat_nothing.rows.len(), 0);
    let fat_vnothing = timed("v2 view overhead isolated, fat fixture", || {
        fat_store.query_as(&alice, Mode::Read, NOTHING).unwrap()
    });
    assert_eq!(fat_vnothing.rows.len(), 0);

    // 3. session graph-set: cold vs cached
    let bob = Session { agent: Some(sparq_solid::fixture::BOB), client: None, issuer: None, now: None };
    let t = Instant::now();
    let cold = store.accessible(&bob, Mode::Read);
    println!("{:64} {:10.3} ms", "session graph-set, cold (AuthIndex walk)", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let cached = store.accessible(&bob, Mode::Read);
    println!("{:64} {:10.4} ms", "session graph-set, cached", t.elapsed().as_secs_f64() * 1e3);
    assert_eq!(cold.len(), cached.len());
}
