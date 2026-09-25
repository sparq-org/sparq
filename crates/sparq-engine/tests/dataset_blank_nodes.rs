// [GPT-6] REC-derived RDF merge identity tests; native execution, not proof evidence.
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{
    DatasetView, DefaultGraphMode, FxHashSet, PreparedQuery, ask, ask_prepared, construct, query,
    query_view,
};
use std::sync::Arc;

fn graph() -> Graph {
    Graph::load_dataset(
        "_:shared <http://ex/p> <http://ex/a> <http://ex/g1> .\n\
         _:shared <http://ex/q> <http://ex/b> <http://ex/g1> .\n\
         _:shared <http://ex/p> <http://ex/a> <http://ex/g2> .\n\
         _:shared <http://ex/r> <http://ex/c> <http://ex/g2> .\n\
         _:fm0_0_shared <http://ex/p> <http://ex/a> <http://ex/reserved> .\n\
         _:fm1_0_shared <http://ex/q> <http://ex/b> <http://ex/reserved> .",
        "nquads",
    )
    .unwrap()
}

#[test]
fn merged_source_graphs_have_disjoint_nodes_but_preserve_each_graphs_identity() {
    let graph = graph();
    let text = "PREFIX ex:<http://ex/> SELECT ?s FROM ex:g1 FROM ex:g2 WHERE { ?s ex:p ex:a }";
    let result = query(&graph, text).unwrap();
    assert_eq!(
        result.rows.len(),
        2,
        "identical triples in distinct source graphs stay distinct"
    );
    assert_ne!(result.rows[0][0], result.rows[1][0]);
    assert!(
        ask(
            &graph,
            "PREFIX ex:<http://ex/> ASK FROM ex:g1 FROM ex:g2 { ?s ex:p ex:a; ex:q ex:b }"
        )
        .unwrap()
    );
    assert!(
        !ask(
            &graph,
            "PREFIX ex:<http://ex/> ASK FROM ex:g1 FROM ex:g2 { ?s ex:q ex:b; ex:r ex:c }"
        )
        .unwrap()
    );
}

#[test]
fn from_named_and_ordinary_graph_access_keep_source_identity() {
    let graph = graph();
    for clauses in ["", "FROM NAMED ex:g1 FROM NAMED ex:g2"] {
        let text = format!(
            "PREFIX ex:<http://ex/> ASK {clauses} {{ GRAPH ex:g1 {{ ?s ex:q ex:b }} GRAPH ex:g2 {{ ?s ex:r ex:c }} }}"
        );
        assert!(ask(&graph, &text).unwrap(), "{text}");
    }
}

#[test]
fn merged_nodes_cannot_collide_with_preserved_named_graph_nodes() {
    let graph = graph();
    let text = "PREFIX ex:<http://ex/> ASK FROM ex:g1 FROM NAMED ex:reserved { ?s ex:p ex:a . GRAPH ex:reserved { ?s ?p ?o } }";
    assert!(!ask(&graph, text).unwrap());
    assert!(!ask_prepared(&graph, &PreparedQuery::parse(text).unwrap()).unwrap());
    // [GPT-6] This implementation acquires each IRI once, but separates its FROM
    // copy from its preserved FROM NAMED copy; §13.2.3 leaves that identity open.
    assert!(!ask(&graph, "PREFIX ex:<http://ex/> ASK FROM ex:g1 FROM NAMED ex:g1 { ?s ex:p ex:a . GRAPH ex:g1 { ?s ex:p ex:a } }").unwrap());
}

#[test]
fn repeated_from_uses_one_snapshot_and_ground_triples_still_deduplicate() {
    let graph = graph();
    let result = query(
        &graph,
        "PREFIX ex:<http://ex/> SELECT ?s FROM ex:g1 FROM ex:g1 { ?s ex:p ex:a }",
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1, "explicit one-snapshot-per-IRI policy");
    let ground = Graph::load_dataset("<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g2> .", "nquads").unwrap();
    assert_eq!(
        query(
            &ground,
            "PREFIX ex:<http://ex/> SELECT * FROM ex:g1 FROM ex:g2 { ?s ?p ?o }"
        )
        .unwrap()
        .rows
        .len(),
        1
    );
}

#[test]
fn dataset_view_restrictions_and_graph_producers_share_the_merge() {
    let graph = graph();
    let named: FxHashSet<_> = [Term::NamedNode(NamedNode::new("http://ex/g1").unwrap())]
        .into_iter()
        .collect();
    let view = DatasetView {
        base: &graph,
        named: Arc::new(named),
        default: DefaultGraphMode::StoreDefault,
    };
    let text = "PREFIX ex:<http://ex/> SELECT ?s FROM ex:g1 FROM ex:g2 { ?s ex:p ex:a }";
    assert_eq!(query_view(&view, text).unwrap().rows.len(), 1);
    let result = construct(&graph, "PREFIX ex:<http://ex/> CONSTRUCT { ?s ex:p ex:a } FROM ex:g1 FROM ex:g2 WHERE { ?s ex:p ex:a }").unwrap();
    assert_eq!(result.len(), 2);
}

#[test]
fn triple_term_nodes_are_renamed_consistently_and_reserve_namespaces() {
    let graph = Graph::load_dataset(
        "_:x <http://ex/p> <<( _:x <http://ex/q> _:x )>> <http://ex/g1> .\n\
         _:x <http://ex/p> <<( _:x <http://ex/q> _:x )>> <http://ex/g2> .\n\
         <http://ex/s> <http://ex/t> <<( _:fm0_0_x <http://ex/q> _:fm1_0_x )>> <http://ex/reserved> .",
        "nquads",
    ).unwrap();
    let rows = query(&graph, "PREFIX ex:<http://ex/> SELECT ?s ?o FROM ex:g1 FROM ex:g2 FROM NAMED ex:reserved { ?s ex:p ?o }").unwrap().rows;
    assert_eq!(rows.len(), 2);
    for row in rows {
        let Term::BlankNode(subject) = row[0].as_ref().unwrap() else {
            panic!("blank source")
        };
        let Term::Triple(triple) = row[1].as_ref().unwrap() else {
            panic!("triple term")
        };
        assert_eq!(triple.subject.to_string(), subject.to_string());
        assert_eq!(triple.object, Term::BlankNode(subject.clone()));
        assert!(!["fm0_0_x", "fm1_0_x"].contains(&subject.as_str()));
    }
}
