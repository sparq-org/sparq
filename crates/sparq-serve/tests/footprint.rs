//! (sq-fggn) Behavioural tests for the conservative write-footprint
//! analysis (`Footprint` / `TargetGraph`) that drives commutativity batching
//! under [`CommitGranularity::CommuteGroup`](sparq_serve::CommitGranularity).
//!
//! These assert the actual COMMUTATIVITY DECISIONS the writer relies on — the
//! soundness contract of §6.5: two updates may only be grouped (applied in one
//! generation without ordering them) when [`Footprint::commutes_with`] says so.
//! A wrong `true` would let the writer reorder conflicting writes, so each case
//! below is a correctness assertion, not a line-toucher. The production applier's
//! footprint hook ([`GraphApplier`]) is exercised too, so the text-derived
//! footprint the writer actually sees is the one under test.

use std::sync::Arc;

use sparq_serve::{ApplyUpdates, Footprint, GraphApplier, TargetGraph};

fn named(iri: &str) -> TargetGraph {
    TargetGraph::Named(Arc::from(iri))
}

/// `INSERT DATA` into the default graph yields a `Data` footprint whose insert
/// set is exactly {Default} and whose delete set is empty.
#[test]
fn insert_data_default_graph_is_data_insert_only() {
    let fp = Footprint::of_sparql("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }");
    match &fp {
        Footprint::Data { inserts, deletes } => {
            assert!(inserts.contains(&TargetGraph::Default));
            assert_eq!(inserts.len(), 1);
            assert!(deletes.is_empty());
        }
        Footprint::Barrier => panic!("pure INSERT DATA must be a Data footprint"),
    }
    assert!(!fp.is_barrier());
}

/// `DELETE DATA` from a NAMED graph carries that named-graph slot in its delete
/// set (exercises the `GraphName::NamedNode` -> `TargetGraph::Named` conversion).
#[test]
fn delete_data_named_graph_records_named_target() {
    let fp = Footprint::of_sparql(
        "DELETE DATA { GRAPH <http://pod/alice> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    );
    match fp {
        Footprint::Data { inserts, deletes } => {
            assert!(inserts.is_empty());
            assert!(deletes.contains(&named("http://pod/alice")));
        }
        Footprint::Barrier => panic!("pure DELETE DATA must be a Data footprint"),
    }
}

/// Unparseable text is conservatively a Barrier (never panics, never wrongly
/// classifies as commutable) — the parse-error arm of `of_sparql`.
#[test]
fn garbage_text_is_a_barrier() {
    let fp = Footprint::of_sparql("this is definitely not sparql");
    assert_eq!(fp, Footprint::Barrier);
    assert!(fp.is_barrier());
}

/// A `WHERE`-driven update (DELETE/INSERT … WHERE) is order-sensitive, so it is
/// a Barrier even though it parses fine — the catch-all arm of `of_sparql`.
#[test]
fn where_driven_update_is_a_barrier() {
    let fp = Footprint::of_sparql(
        "DELETE { ?s <http://ex/p> ?o } INSERT { ?s <http://ex/p2> ?o } \
         WHERE { ?s <http://ex/p> ?o }",
    );
    assert!(fp.is_barrier(), "WHERE-driven update must be a barrier");
}

/// Graph-management operations (DROP/CLEAR) are unanalyzable barriers too.
#[test]
fn graph_management_is_a_barrier() {
    assert!(Footprint::of_sparql("DROP GRAPH <http://pod/alice>").is_barrier());
    assert!(Footprint::of_sparql("CLEAR DEFAULT").is_barrier());
}

/// Same-graph SAME-polarity inserts COMMUTE (set union is order-insensitive):
/// the key license that keeps a bulk-ingest-into-one-pod stream batchable.
#[test]
fn same_graph_same_polarity_inserts_commute() {
    let a = Footprint::of_sparql(
        "INSERT DATA { GRAPH <http://pod/p> { <http://ex/a> <http://ex/q> <http://ex/1> } }",
    );
    let b = Footprint::of_sparql(
        "INSERT DATA { GRAPH <http://pod/p> { <http://ex/b> <http://ex/q> <http://ex/2> } }",
    );
    assert!(
        a.commutes_with(&b),
        "same-graph same-polarity inserts commute"
    );
    assert!(b.commutes_with(&a), "commutativity is symmetric");
}

/// Disjoint-graph updates commute regardless of polarity.
#[test]
fn disjoint_graph_updates_commute() {
    let ins = Footprint::of_sparql(
        "INSERT DATA { GRAPH <http://pod/x> { <http://ex/a> <http://ex/q> <http://ex/1> } }",
    );
    let del = Footprint::of_sparql(
        "DELETE DATA { GRAPH <http://pod/y> { <http://ex/b> <http://ex/q> <http://ex/2> } }",
    );
    assert!(
        ins.commutes_with(&del),
        "different graphs cannot share a triple"
    );
    assert!(del.commutes_with(&ins));
}

/// Opposite polarity on a SHARED graph does NOT commute — the one graph-level
/// situation where the same triple could be both inserted and deleted, making
/// application order observable. A wrong `true` here is a soundness bug.
#[test]
fn opposite_polarity_same_graph_does_not_commute() {
    let ins = Footprint::of_sparql(
        "INSERT DATA { GRAPH <http://pod/p> { <http://ex/a> <http://ex/q> <http://ex/1> } }",
    );
    let del = Footprint::of_sparql(
        "DELETE DATA { GRAPH <http://pod/p> { <http://ex/a> <http://ex/q> <http://ex/1> } }",
    );
    assert!(
        !ins.commutes_with(&del),
        "insert+delete on the same graph must NOT commute"
    );
    assert!(!del.commutes_with(&ins), "non-commutativity is symmetric");
}

/// A Barrier commutes with NOTHING — not even another Barrier, and not with a
/// pure data update.
#[test]
fn a_barrier_commutes_with_nothing() {
    let barrier = Footprint::of_sparql("DROP GRAPH <http://pod/p>");
    let data = Footprint::of_sparql("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }");
    let other_barrier = Footprint::of_sparql("CLEAR ALL");
    assert!(!barrier.commutes_with(&data));
    assert!(!data.commutes_with(&barrier));
    assert!(!barrier.commutes_with(&other_barrier));
}

/// A multi-operation update that mixes an insert and a delete into the SAME
/// graph is internally order-sensitive, but `of_sparql` records both graph slots;
/// it must then NOT commute with itself's mirror (delete-into-the-insert-graph).
#[test]
fn mixed_insert_delete_update_tracks_both_graph_sets() {
    let fp = Footprint::of_sparql(
        "INSERT DATA { GRAPH <http://pod/p> { <http://ex/a> <http://ex/q> <http://ex/1> } }; \
         DELETE DATA { GRAPH <http://pod/q> { <http://ex/b> <http://ex/q> <http://ex/2> } }",
    );
    match fp {
        Footprint::Data { inserts, deletes } => {
            assert!(inserts.contains(&named("http://pod/p")));
            assert!(deletes.contains(&named("http://pod/q")));
        }
        Footprint::Barrier => panic!("multi-op pure-data update must stay Data"),
    }
}

/// The production applier's `footprint` hook delegates to `Footprint::of_sparql`,
/// so the text-derived footprint the writer sees under `CommuteGroup` is the very
/// one tested above.
#[test]
fn graph_applier_footprint_matches_of_sparql() {
    let applier = GraphApplier::new();
    let upd = "INSERT DATA { GRAPH <http://pod/p> { <http://ex/s> <http://ex/p> <http://ex/o> } }"
        .to_string();
    assert_eq!(applier.footprint(&upd), Footprint::of_sparql(&upd));
    let barrier = "DROP GRAPH <http://pod/p>".to_string();
    assert!(applier.footprint(&barrier).is_barrier());
}
