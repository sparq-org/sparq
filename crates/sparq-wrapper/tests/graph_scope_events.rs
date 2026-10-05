// sq-1rg2q.7: projected change events fire only at the first/last in-scope
// copy of a triple and stay silent for graphs outside the projection.

#![cfg(feature = "proposed-graph-scope-events")]

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::proposed::graph_scope::{GraphScope, Projection};
use sparq_wrapper::proposed::graph_scope_events::ObservableDataset;
use sparq_wrapper::proposed::observe::{ChangeKind, ObserveError};
use std::cell::RefCell;
use std::rc::Rc;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn graph_term(local: &str) -> Term {
    Term::NamedNode(iri(local))
}

type Seen = Rc<RefCell<Vec<(ChangeKind, Projection, usize)>>>;

fn record(dataset: &mut ObservableDataset, projection: Projection) -> Seen {
    let seen = Seen::default();
    let seen_by_observer = Rc::clone(&seen);
    dataset.subscribe(projection, move |event, projection, committed| {
        seen_by_observer.borrow_mut().push((
            event.kind,
            projection.clone(),
            committed.named.iter().map(|(_, graph)| graph.len()).sum(),
        ));
    });
    seen
}

#[test]
fn projected_events_fire_only_at_first_and_last_in_scope_copies() {
    let (g1, g2, g3) = (graph_term("g1"), graph_term("g2"), graph_term("g3"));
    let (alice, tag) = (iri("alice"), iri("tag"));
    let value = Literal::new_simple_literal("shared");
    let projection = Projection::new([g1.clone(), g2.clone()]);
    let mut dataset = ObservableDataset::new();
    let seen = record(&mut dataset, projection.clone());

    let mut step = |insert: bool, graph: &Term| {
        let (s, p, o) = (alice.clone(), tag.clone(), value.clone());
        let changed = if insert {
            dataset.insert(Some(graph), s, p, o)
        } else {
            dataset.remove(Some(graph), s, p, o)
        }
        .unwrap();
        (
            changed,
            seen.borrow()
                .iter()
                .map(|(kind, ..)| *kind)
                .collect::<Vec<_>>(),
        )
    };

    use ChangeKind::{Add, Delete};
    assert_eq!(step(true, &g3), (true, vec![]), "excluded graph is silent");
    assert_eq!(step(true, &g1), (true, vec![Add]), "first in-scope copy");
    assert_eq!(
        step(true, &g2),
        (true, vec![Add]),
        "second copy is invisible"
    );
    assert_eq!(step(true, &g2), (false, vec![Add]), "duplicate is a no-op");
    assert_eq!(step(false, &g1), (true, vec![Add]), "g2 still holds a copy");
    assert_eq!(
        step(false, &g3),
        (true, vec![Add]),
        "excluded graph is silent"
    );
    assert_eq!(step(false, &g2), (true, vec![Add, Delete]), "last copy");
    assert_eq!(
        step(false, &g2),
        (false, vec![Add, Delete]),
        "absent delete"
    );

    // Each event reports the configured projection and sees the committed quads.
    let seen = seen.borrow();
    assert!(seen.iter().all(|(_, reported, _)| *reported == projection));
    assert_eq!(
        seen.iter()
            .map(|(kind, _, quads)| (*kind, *quads))
            .collect::<Vec<_>>(),
        [(Add, 2), (Delete, 0)]
    );
}

#[test]
fn default_graph_membership_and_scope_projection_are_explicit() {
    let g1 = graph_term("g1");
    let (alice, tag) = (iri("alice"), iri("tag"));
    let mut scratch = Graph::new();
    let scope =
        GraphScope::new(&mut scratch, [g1.clone(), g1.clone()], g1.clone()).with_default_graph();
    let with_default = scope.projection().clone();
    assert_eq!(
        with_default,
        Projection::new([g1.clone()]).with_default_graph()
    );
    assert!(with_default.includes(None) && with_default.includes(Some(&g1)));

    let mut dataset = ObservableDataset::new();
    let default_seen = record(&mut dataset, with_default);
    let named_only = record(&mut dataset, Projection::new([g1.clone()]));

    let value = Literal::new_simple_literal("v");
    assert!(dataset
        .insert(None, alice.clone(), tag.clone(), value.clone())
        .unwrap());
    assert_eq!(default_seen.borrow().len(), 1);
    assert!(named_only.borrow().is_empty());

    // The default copy already makes the triple visible, so g1's copy is silent.
    assert!(dataset
        .insert(Some(&g1), alice.clone(), tag.clone(), value.clone())
        .unwrap());
    assert_eq!(default_seen.borrow().len(), 1);
    assert_eq!(named_only.borrow().len(), 1);
}

#[test]
fn absent_graphs_are_not_created_and_literal_subjects_are_rejected() {
    let mut dataset = ObservableDataset::default();
    let seen = record(&mut dataset, Projection::new([graph_term("g1")]));
    let value = Literal::new_simple_literal("v");

    assert!(!dataset
        .remove(
            Some(&graph_term("g1")),
            iri("alice"),
            iri("tag"),
            value.clone()
        )
        .unwrap());
    assert!(dataset.graph().named.is_empty());
    assert_eq!(
        dataset.insert(Some(&graph_term("g1")), value.clone(), iri("tag"), value),
        Err(ObserveError::LiteralSubject)
    );
    assert!(seen.borrow().is_empty());
    assert!(dataset.into_graph().named.is_empty());
}
