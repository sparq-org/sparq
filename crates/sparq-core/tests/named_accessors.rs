use oxrdf::{NamedNode, Term};
use sparq_core::Graph;

fn iri(name: &str) -> Term {
    NamedNode::new(format!("http://ex/{name}")).unwrap().into()
}

fn names_with_prefix(graph: &Graph, prefix: &str) -> Vec<Term> {
    let mut names = Vec::new();
    graph.for_named_graphs_with_prefix(prefix, |name, _| names.push(name.clone()));
    names
}

#[test]
fn named_accessors_preserve_order_content_and_refresh_prefix_queries() {
    let mut graph = Graph::new();
    graph.add_named(iri("z"), Graph::new()).unwrap();
    graph.add_named(iri("a"), Graph::new()).unwrap();
    assert_eq!(names_with_prefix(&graph, "http://ex/a"), [iri("a")]);
    let removed = graph.remove_named(&iri("a")).unwrap().unwrap();
    graph.add_named(iri("zz"), removed).unwrap();
    // The length matches the cached index's length, but the sort order changed.
    assert_eq!(names_with_prefix(&graph, "http://ex/zz"), [iri("zz")]);
    graph
        .apply_delta_to_named(1, &[[iri("s"), iri("p"), iri("o")]], &[])
        .unwrap();
    assert_eq!(graph.named_graph(&iri("zz")).unwrap().len(), 1);
    assert_eq!(
        graph
            .set_named(iri("zz"), Graph::new())
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    let members = graph.take_named().unwrap();
    assert!(graph.named().is_empty());
    graph.extend_named(members).unwrap();
    graph.retain_named(|name, _| *name == iri("zz")).unwrap();
    assert_eq!(graph.named()[0].0, iri("zz"));
    assert!(graph.apply_delta_to_named(1, &[], &[]).is_err());
    assert!(graph.remove_named(&iri("absent")).unwrap().is_none());
}

#[cfg(feature = "mmap")]
#[test]
fn frozen_named_accessors_refuse_changes_and_keep_the_view() {
    let mut graph = Graph::new();
    graph.add_named(iri("g"), Graph::new()).unwrap();
    graph.freeze_for_test("storage unavailable");
    assert!(graph.add_named(iri("new"), Graph::new()).is_err());
    assert!(graph.extend_named([]).is_err());
    assert!(graph.set_named(iri("g"), Graph::new()).is_err());
    assert!(graph.remove_named(&iri("g")).is_err());
    assert!(graph.retain_named(|_, _| false).is_err());
    assert!(graph.take_named().is_err());
    assert!(graph.apply_delta_to_named(0, &[], &[]).is_err());
    assert!(graph.replace_default(Graph::new()).is_err());
    assert!(graph.rebuild_default(|_, triples| triples.clear()).is_err());
    assert_eq!(graph.named().len(), 1);
    assert!(graph.named_graph(&iri("g")).unwrap().is_frozen());
}

#[test]
fn durable_named_methods_refresh_prefix_queries_when_the_count_returns_to_its_old_value() {
    let mut graph = Graph::new();
    graph.ensure_named(&iri("z")).unwrap();
    graph.ensure_named(&iri("a")).unwrap();
    assert_eq!(names_with_prefix(&graph, "http://ex/a"), [iri("a")]);
    graph.drop_named_durable(&iri("a")).unwrap();
    graph.ensure_named(&iri("zz")).unwrap();
    assert_eq!(names_with_prefix(&graph, "http://ex/zz"), [iri("zz")]);
    graph.drop_all_named_durable().unwrap();
    graph.ensure_named(&iri("zz")).unwrap();
    graph
        .apply_delta_nquads(
            "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/z> .",
            "",
        )
        .unwrap();
    assert_eq!(names_with_prefix(&graph, "http://ex/zz"), [iri("zz")]);
}
