//! End-to-end quick start (matches the README): load a pod as a dataset, materialize
//! the WAC auth view, open sessions, and run the SAME query as different principals.
//!
//!     cargo run -p sparq-solid --example quickstart --release
//!
//! (debug works too; the N3 materialization just takes ~10x longer)

use sparq_core::Graph;
use sparq_solid::fixture::{wac_fixture, ALICE, BOB, CAROL, APP};
use sparq_solid::{Mode, PodStore, Session};

fn main() -> Result<(), String> {
    // 1. Load a pod: named graph per document (the bundled fixture is a synthetic
    //    ~1.1k-graph pod — swap in your own N-Quads/TriG dump).
    let graph = Graph::load_dataset(&wac_fixture(), "nquads")?;
    println!("loaded {} named graphs", graph.named.len());

    // 2. Wrap it (fail-closed: nobody sees anything yet) and materialize the WAC
    //    auth view — the N3 rules run and install <urn:sparq:auth>.
    let mut store = PodStore::new(graph);
    let stats = store.materialize_wac()?;
    println!("materialized {} auth triples in {:.0} ms", stats.auth_triples, stats.millis);

    // 3. Open sessions and run the same query as each of them. `query_as` is the
    //    fast path: it evaluates through the engine's zero-copy DatasetView — graph
    //    visibility is an O(1) hash check, nothing is copied per query.
    let query = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";
    for (label, session) in [
        ("alice (pod owner)", Session { agent: Some(ALICE), client: None, issuer: None, now: None }),
        ("carol (team member)", Session { agent: Some(CAROL), client: None, issuer: None, now: None }),
        (
            "bob via app.ex (acl:origin pair)",
            Session { agent: Some(BOB), client: Some(APP), issuer: None, now: None },
        ),
        ("anonymous", Session::default()),
    ] {
        let readable = store.accessible(&session, Mode::Read).len();
        let rows = store.query_as(&session, Mode::Read, query)?.rows.len();
        println!("{label:36} {readable:4} readable graphs, {rows:4} rows");
    }

    // 4. The auth view is plain triples — query it directly.
    let who = sparq_engine::query(
        store.graph(),
        "SELECT (COUNT(*) AS ?grants) WHERE { GRAPH <urn:sparq:auth> {
           ?principal <https://sparq.dev/ns/auth#read> ?graph } }",
    )?;
    println!("auth view: {:?} read grants", who.rows[0][0].as_ref().map(|t| t.to_string()));

    // sanity: the fixture's hand-derived counts (same as tests/e2e.rs)
    let alice =
        store.query_as(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, query)?;
    let anon = store.query_as(&Session::default(), Mode::Read, query)?;
    assert_eq!(alice.rows.len(), 599);
    assert_eq!(anon.rows.len(), 144);
    Ok(())
}
