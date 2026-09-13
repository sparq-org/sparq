// [GPT-6] Appended exact keys must reach ordinary FILTER, COUNT, ORDER and aggregates.
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::query;

fn date(fraction: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        format!("2024-01-01T00:00:00.{fraction}Z"),
        NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime"),
    ))
}

#[test]
fn append_then_query_preserves_exact_old_and_new_temporal_values() {
    for storage in 0..3 {
        let old = date("000000001");
        let middle = date("000000002");
        let later = date("000000003");
        let source =
            format!("<http://ex/s> <http://ex/p> {old} .\n<http://ex/s> <http://ex/p> {middle} .");
        let graph = Graph::load_str(&source, "nt").unwrap();
        let mut graph = match storage {
            1 => graph.into_compressed(),
            2 => graph.fork(),
            _ => graph,
        };
        let selected = format!(
            "SELECT ?t {{ <http://ex/s> <http://ex/p> ?t FILTER(?t > {old}) }} ORDER BY ?t"
        );
        assert_eq!(
            query(&graph, &selected).unwrap().rows,
            vec![vec![Some(middle.clone())]]
        );
        let iri = |value| Term::NamedNode(NamedNode::new_unchecked(value));
        graph
            .apply_delta(
                &[[
                    iri("http://ex/unrelated"),
                    iri("http://ex/text"),
                    Term::Literal(Literal::new_simple_literal("new text")),
                ]],
                &[],
            )
            .unwrap();
        assert_eq!(
            query(&graph, &selected).unwrap().rows,
            vec![vec![Some(middle.clone())]]
        );
        let triple = [iri("http://ex/s"), iri("http://ex/p"), later.clone()];
        graph
            .apply_delta(std::slice::from_ref(&triple), &[])
            .unwrap();
        assert_eq!(
            query(&graph, &selected).unwrap().rows,
            vec![vec![Some(middle.clone())], vec![Some(later.clone())]]
        );
        let count = query(
            &graph,
            &format!(
                "SELECT (COUNT(?t) AS ?n) {{ <http://ex/s> <http://ex/p> ?t FILTER(?t > {old}) }}"
            ),
        )
        .unwrap();
        assert_eq!(
            count.rows,
            vec![vec![Some(Term::Literal(Literal::from(2)))]]
        );
        let extrema = query(
            &graph,
            "SELECT (MIN(?t) AS ?lo) (MAX(?t) AS ?hi) { <http://ex/s> <http://ex/p> ?t }",
        )
        .unwrap();
        assert_eq!(extrema.rows, vec![vec![Some(old), Some(later)]]);
        graph.apply_delta(&[], &[triple]).unwrap();
        assert_eq!(
            query(&graph, &selected).unwrap().rows,
            vec![vec![Some(middle)]]
        );
    }
}
