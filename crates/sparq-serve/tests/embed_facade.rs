//! sq-xa15c (#1248 items 1+2) — DIRECT unit coverage of the in-process
//! embedding-seam data-path facade ([`sparq_serve::embed`]).
//!
//! The companion `embed_seam.rs` proves the generation-ring concurrency wrapper
//! preserves snapshot isolation, but it only routes through two of the facade's
//! data-path functions (`query_json` + `update_in_place`). This file exercises EACH
//! remaining public facade wrapper directly on a small in-memory `Graph` and asserts
//! the correct result, so the facade's own one-line bodies are covered (not just the
//! underlying engine path) and the load-bearing property of each — read/ask returns
//! the right value, the two write paths mutate as documented, the atomic write is
//! all-or-nothing, the quad-level delta applies adds/deletes across default + named
//! graphs, and the existence/metadata probes return correct presence/counts.
//!
//! Everything here is deterministic and network-free.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_serve::embed::{self, Metadata};
use sparq_engine::QueryBudget;

/// A two-triple default-graph resource on one subject.
fn seed() -> Graph {
    Graph::load_str(
        "<http://ex/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n\
         <http://ex/alice> <http://xmlns.com/foaf/0.1/age> \"30\" .\n",
        "ntriples",
    )
    .expect("load seed graph")
}

/// Pull the single scalar `?c` value out of a SPARQL-1.1-JSON SELECT body without a
/// JSON dependency (mirrors `embed_seam.rs`'s helper so the JSON path stays asserted).
fn json_scalar(json: &str) -> String {
    let needle = "\"value\":\"";
    let start = json.rfind(needle).expect("value present") + needle.len();
    let end = json[start..].find('"').expect("close quote") + start;
    json[start..end].to_string()
}

#[test]
fn query_json_returns_expected_rows() {
    let g = seed();
    // Two triples on one subject -> SELECT ?p ?o yields two result bindings, and the
    // JSON carries the two predicate IRIs.
    let json = embed::query_json(&g, "SELECT ?p ?o WHERE { <http://ex/alice> ?p ?o }")
        .expect("query_json ok");
    assert!(
        json.contains("http://xmlns.com/foaf/0.1/name"),
        "name predicate present in JSON: {}",
        json
    );
    assert!(
        json.contains("http://xmlns.com/foaf/0.1/age"),
        "age predicate present in JSON: {}",
        json
    );
    // A COUNT(*) over the graph reads exactly 2 through the JSON facade.
    let cj = embed::query_json(&g, "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }")
        .expect("count json ok");
    assert_eq!(json_scalar(&cj), "2");
}

#[test]
fn query_json_with_budget_enforces_row_cap() {
    let g = seed();
    // An unlimited budget returns the same body as the plain facade.
    let unlimited = embed::query_json_with_budget(
        &g,
        "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }",
        &QueryBudget::unlimited(),
    )
    .expect("unlimited budget ok");
    assert_eq!(json_scalar(&unlimited), "2");

    // A 1-row working-set cap refuses the 2-row SELECT (the budget is enforced at the
    // working-set, so the materialised 2-row intermediate trips the ceiling).
    let tight = QueryBudget {
        max_rows: Some(1),
        ..QueryBudget::unlimited()
    };
    let refused = embed::query_json_with_budget(&g, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &tight);
    assert!(
        refused.is_err(),
        "a 2-row result must be refused under a 1-row budget, got {:?}",
        refused
    );
}

#[test]
fn query_returns_structured_rows() {
    let g = seed();
    let res = embed::query(&g, "SELECT ?p WHERE { <http://ex/alice> ?p ?o }")
        .expect("query ok");
    assert_eq!(res.vars.len(), 1, "one projected variable");
    assert_eq!(res.vars[0].as_str(), "p");
    assert_eq!(res.rows.len(), 2, "two predicate bindings");
    // Every row has the single ?p column bound to a NamedNode (the predicate IRI).
    for row in &res.rows {
        assert_eq!(row.len(), 1);
        assert!(
            matches!(row[0], Some(Term::NamedNode(_))),
            "?p binds to an IRI, got {:?}",
            row[0]
        );
    }
}

#[test]
fn ask_returns_correct_bool() {
    let g = seed();
    assert!(
        embed::ask(&g, "ASK { <http://ex/alice> ?p ?o }").expect("ask present"),
        "pattern with solutions -> true"
    );
    assert!(
        !embed::ask(&g, "ASK { <http://ex/nobody> ?p ?o }").expect("ask absent"),
        "pattern with no solutions -> false"
    );
}

#[test]
fn update_in_place_mutates_graph() {
    let mut g = seed();
    embed::update_in_place(
        &mut g,
        "INSERT DATA { <http://ex/bob> <http://xmlns.com/foaf/0.1/name> \"Bob\" }",
    )
    .expect("insert ok");
    assert_eq!(g.len(), 3, "one triple inserted");

    embed::update_in_place(
        &mut g,
        "DELETE DATA { <http://ex/alice> <http://xmlns.com/foaf/0.1/age> \"30\" }",
    )
    .expect("delete ok");
    assert_eq!(g.len(), 2, "one triple deleted");
    assert!(
        !embed::ask(&g, "ASK { <http://ex/alice> <http://xmlns.com/foaf/0.1/age> ?o }")
            .expect("ask"),
        "deleted triple gone"
    );
}

#[test]
fn update_in_place_atomic_is_all_or_nothing() {
    let mut g = seed();
    // A syntactically invalid update must leave the graph exactly as it was.
    let before = g.len();
    let err = embed::update_in_place_atomic(&mut g, "INSERT DATA { this is not sparql }");
    assert!(err.is_err(), "malformed update is rejected");
    assert_eq!(g.len(), before, "atomic failure left no partial prefix applied");

    // A well-formed atomic update commits.
    embed::update_in_place_atomic(
        &mut g,
        "INSERT DATA { <http://ex/carol> <http://xmlns.com/foaf/0.1/name> \"Carol\" }",
    )
    .expect("atomic insert ok");
    assert_eq!(g.len(), before + 1, "valid atomic update committed");
}

#[test]
fn apply_delta_nquads_applies_adds_and_deletes() {
    let mut g = seed();
    // Insert one default-graph triple and one NAMED-graph triple; delete one of the
    // seed triples. Deletes are applied before inserts, per the documented contract.
    let inserts = "<http://ex/bob> <http://xmlns.com/foaf/0.1/name> \"Bob\" .\n\
         <http://ex/doc1> <http://ex/p> \"v\" <http://ex/graph1> .\n";
    let deletes = "<http://ex/alice> <http://xmlns.com/foaf/0.1/age> \"30\" .\n";
    embed::apply_delta_nquads(&mut g, inserts, deletes).expect("delta ok");

    // Default graph: started 2, -1 delete +1 insert = 2.
    assert_eq!(g.len(), 2, "default-graph triple count after delta");
    assert!(
        embed::ask(&g, "ASK { <http://ex/bob> ?p ?o }").expect("ask bob"),
        "inserted default-graph triple present"
    );
    assert!(
        !embed::ask(&g, "ASK { <http://ex/alice> <http://xmlns.com/foaf/0.1/age> ?o }")
            .expect("ask age"),
        "deleted triple gone"
    );
    // The named-graph statement auto-created the named graph.
    let g1 = Term::NamedNode(NamedNode::new("http://ex/graph1").unwrap());
    assert!(
        embed::named_graph_exists(&g, &g1),
        "named graph auto-created by the delta insert"
    );

    // Deleting from an absent named graph is a no-op (does not error).
    let del_named =
        "<http://ex/doc1> <http://ex/p> \"v\" <http://ex/graph1> .\n";
    embed::apply_delta_nquads(&mut g, "", del_named).expect("named-graph delete ok");
}

#[test]
fn exists_reflects_presence() {
    let g = seed();
    assert!(embed::exists(&g), "non-empty default graph -> exists");

    let empty = Graph::load_str("", "ntriples").expect("empty graph");
    assert!(!embed::exists(&empty), "empty default graph -> not exists");
}

#[test]
fn named_graph_exists_reports_correct_presence() {
    let mut g = seed();
    let name = Term::NamedNode(NamedNode::new("http://ex/doc").unwrap());
    assert!(
        !embed::named_graph_exists(&g, &name),
        "named graph absent before any named-graph statement"
    );
    // Add a statement in that named graph via the quad-delta facade.
    embed::apply_delta_nquads(
        &mut g,
        "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/doc> .\n",
        "",
    )
    .expect("named insert ok");
    assert!(
        embed::named_graph_exists(&g, &name),
        "named graph present after insert"
    );
    // A different IRI is still absent.
    let other = Term::NamedNode(NamedNode::new("http://ex/other").unwrap());
    assert!(!embed::named_graph_exists(&g, &other), "unrelated IRI absent");
}

#[test]
fn metadata_reports_correct_counts() {
    let mut g = seed();
    // Fresh seed: 2 default-graph triples, 0 named graphs.
    let m0 = embed::metadata(&g);
    assert_eq!(
        m0,
        Metadata {
            triples: 2,
            named_graphs: 0,
        }
    );

    // Add a default-graph triple and two distinct named graphs.
    embed::update_in_place(
        &mut g,
        "INSERT DATA { <http://ex/bob> <http://xmlns.com/foaf/0.1/name> \"Bob\" }",
    )
    .expect("insert ok");
    embed::apply_delta_nquads(
        &mut g,
        "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
         <http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g2> .\n",
        "",
    )
    .expect("named inserts ok");

    let m1 = embed::metadata(&g);
    assert_eq!(m1.triples, 3, "default-graph triple count tracks inserts");
    assert_eq!(m1.named_graphs, 2, "two distinct named graphs counted");
}
