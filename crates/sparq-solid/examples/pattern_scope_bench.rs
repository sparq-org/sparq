//! sq-lrtc3.3 — the pattern-scope measured overhead envelope. Run:
//!     cargo run -p sparq-solid --features pattern-scope --release --example pattern_scope_bench
//!
//! Emits ONE JSON object to stdout (committed under `bench/pattern-scope/`; work-box
//! numbers are NON-canonical — see `research/odrl-pattern-scoped-targets-2026-07.md` §4).
//! Dimensions, per fixture size (`wac_fixture_sized(extra)`):
//!
//! 1. `scoped_build_ms` — the cost this design pays INSTEAD of per-scan filtering:
//!    one `PodStore::scoped_dataset` materialization (decode → filter → rebuild of
//!    every accessible graph) for a scope masking the `title` predicate everywhere.
//! 2. `replica_query_ms` vs `view_query_ms` — the SAME query over the prebuilt
//!    masked replica vs the ordinary graph-granular `query_as` view path on the full
//!    store: the per-query overhead once assembly is done (expected ≈ 0 — after
//!    assembly the engine sees an ordinary dataset).
//! 3. `breakeven_queries` — build cost divided by any per-query saving (the replica
//!    is smaller), when a saving exists: how many queries amortize one assembly.

use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashMap;
use sparq_solid::fixture::{wac_fixture_sized, ALICE};
use sparq_solid::{GraphScope, Mode, PodStore, ScopePattern, Session};
use std::time::Instant;

const TITLE: &str = "https://ex.dev/ns#title";
const Q: &str = "SELECT ?s WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";
const Q_ALL: &str = "SELECT (COUNT(*) AS ?c) WHERE { GRAPH ?g { ?s ?p ?o } }";

fn best_of<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..n {
        let t = Instant::now();
        out = Some(f());
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    (best, out.unwrap())
}

fn main() {
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let mut rows = Vec::new();

    for extra in [0usize, 20, 50] {
        let nq = wac_fixture_sized(extra);
        let quads = nq.lines().filter(|l| !l.trim().is_empty()).count();
        let mut store = PodStore::new(sparq_core::Graph::load_dataset(&nq, "nquads").unwrap());
        store.materialize_wac().unwrap();

        // The scope: mask every `title` triple in every accessible graph (a
        // store-wide prohibition — worst case: every accessible graph is scoped).
        let deny = GraphScope::deny_within(vec![ScopePattern::new(
            None,
            Some(Term::NamedNode(NamedNode::new(TITLE).unwrap())),
            None,
        )]);
        let scopes: FxHashMap<Term, GraphScope> = store
            .graph()
            .named
            .iter()
            .map(|(name, _)| (name.clone(), deny.clone()))
            .collect();

        let (scoped_build_ms, scoped) =
            best_of(3, || store.scoped_dataset(&alice, Mode::Read, &scopes));

        let (view_query_ms, view_rows) =
            best_of(5, || store.query_as(&alice, Mode::Read, Q).unwrap().rows.len());
        let (replica_query_ms, replica_rows) =
            best_of(5, || scoped.query(Q).unwrap().rows.len());
        let (view_scan_ms, _) =
            best_of(5, || store.query_as(&alice, Mode::Read, Q_ALL).unwrap().rows.len());
        let (replica_scan_ms, _) = best_of(5, || scoped.query(Q_ALL).unwrap().rows.len());

        assert_eq!(replica_rows, 0, "masked replica must hold no title triples");
        assert!(view_rows > 0, "the unmasked view path must see title triples");

        let saving = view_scan_ms - replica_scan_ms;
        let breakeven = if saving > 0.0 { (scoped_build_ms / saving).ceil() } else { -1.0 };
        rows.push(format!(
            "    {{\"extra\": {extra}, \"quads\": {quads}, \"scoped_build_ms\": {scoped_build_ms:.3}, \
             \"view_query_ms\": {view_query_ms:.3}, \"replica_query_ms\": {replica_query_ms:.3}, \
             \"view_scan_ms\": {view_scan_ms:.3}, \"replica_scan_ms\": {replica_scan_ms:.3}, \
             \"view_title_rows\": {view_rows}, \"replica_title_rows\": {replica_rows}, \
             \"breakeven_queries\": {breakeven}}}"
        ));
    }

    println!(
        "{{\n  \"bead\": \"sq-lrtc3.3\",\n  \"design\": \"masked-subgraph materialization \
         (research/odrl-pattern-scoped-targets-2026-07.md)\",\n  \"host\": \"work box — NON-canonical\",\n  \
         \"driver\": \"cargo run -p sparq-solid --features pattern-scope --release --example pattern_scope_bench\",\n  \
         \"note\": \"breakeven_queries = -1 means the replica query showed no measurable saving at this size\",\n  \
         \"sizes\": [\n{}\n  ]\n}}",
        rows.join(",\n")
    );
}
