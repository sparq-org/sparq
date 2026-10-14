//! Mutation-witnessed fuzzy lookup acceptance tests. Removing the deletion
//! signatures, exact-distance verifier, distance ordering, or rewrite arm makes
//! at least one concrete assertion fail. sq-lsp7k.14
#![cfg(feature = "fuzzy")]

use sparq_core::Graph;
use sparq_text::{FuzzyError, TextIndex};

const DATA: &str = r#"
    <http://ex/a> <http://ex/title> "quickly" .
    <http://ex/b> <http://ex/title> "quote" .
    <http://ex/c> <http://ex/title> "quick" .
    <http://ex/d> <http://ex/title> "quaxly" .
"#;

#[test]
fn bounded_candidates_and_zero_distance_exact_invariant() {
    let graph = Graph::load_str(DATA, "ntriples").unwrap();
    let index = TextIndex::build(&graph);

    let one = index.fuzzy("quikly", 1).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!((&*one[0].term, one[0].distance), ("quickly", 1));

    let two = index.fuzzy("quikly", 2).unwrap();
    assert_eq!(
        two.iter()
            .map(|hit| (&*hit.term, hit.distance))
            .collect::<Vec<_>>(),
        [("quickly", 1), ("quaxly", 2)]
    );
    assert!(two
        .iter()
        .all(|hit| &*hit.term != "quote" && &*hit.term != "quick"));

    let fuzzy_exact = index.fuzzy("quick", 0).unwrap();
    let exact = index.search("quick");
    assert_eq!(
        fuzzy_exact
            .iter()
            .map(|hit| (hit.id, hit.score.to_bits()))
            .collect::<Vec<_>>(),
        exact
            .iter()
            .map(|hit| (hit.id, hit.score.to_bits()))
            .collect::<Vec<_>>()
    );
    assert!(index.fuzzy("quikly", 0).unwrap().is_empty());
}

#[test]
fn validates_bound_and_single_term_contract() {
    let graph = Graph::load_str(DATA, "ntriples").unwrap();
    let index = TextIndex::build(&graph);
    assert_eq!(
        index.fuzzy("quick", 3),
        Err(FuzzyError::DistanceTooLarge { requested: 3 })
    );
    assert_eq!(
        index.fuzzy("quick fox", 1),
        Err(FuzzyError::ExpectedSingleToken { found: 2 })
    );
    assert_eq!(index.fuzzy("quick*", 1), Err(FuzzyError::PrefixNotSupported));
}

#[cfg(feature = "engine")]
#[test]
fn fuzzy_magic_predicate_uses_default_and_explicit_distance() {
    use oxrdf::Term;
    use sparq_text::query_text;

    let graph = Graph::load_str(DATA, "ntriples").unwrap();
    let index = TextIndex::build(&graph);
    let query = |distance: Option<u8>| {
        let companion = distance
            .map(|n| format!("?title text:maxDistance {n} ."))
            .unwrap_or_default();
        query_text(
            &graph,
            &format!(
                "PREFIX text: <http://sparq.dev/text#>\n\
                 SELECT ?post WHERE {{\n\
                   ?post <http://ex/title> ?title .\n\
                   ?title text:fuzzy \"quikly\" .\n\
                   {companion}\n\
                 }}"
            ),
            &index,
        )
        .unwrap()
    };

    let default = query(None);
    assert_eq!(default.rows.len(), 1);
    assert!(matches!(
        &default.rows[0][0],
        Some(Term::NamedNode(node)) if node.as_str() == "http://ex/a"
    ));

    let widened = query(Some(2));
    let mut iris: Vec<&str> = widened
        .rows
        .iter()
        .map(|row| match &row[0] {
            Some(Term::NamedNode(node)) => node.as_str(),
            other => panic!("expected IRI, got {other:?}"),
        })
        .collect();
    iris.sort_unstable();
    assert_eq!(iris, ["http://ex/a", "http://ex/d"]);

    let exact = query(Some(0));
    assert!(exact.rows.is_empty());
}
