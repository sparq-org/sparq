//! Behavioural coverage for the [`sparq_text`] `text:` rewrite that the existing
//! `e2e.rs` suite does not reach: the budgeted entry point
//! ([`sparq_text::query_text_with_budget`]), CONSTRUCT/DESCRIBE rewrites (the
//! non-SELECT/ASK arms of `rewrite_query`), magic patterns nested inside the
//! structural algebra operators that `rewrite_pattern` must recurse through
//! (UNION / MINUS / FILTER / ORDER BY / DISTINCT / sub-SELECT projection /
//! property paths), a variable-predicate triple sharing a BGP with a magic
//! pattern (the predicate-passthrough arm), and the "ambiguous"/"duplicate"
//! companion-attachment errors (more than one match on a variable carrying a
//! `text:score`/`text:slop`). Every test asserts a CONCRETE result, not mere
//! line execution. sq-bj7j
#![cfg(feature = "engine")]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine::{construct_prepared, describe_prepared, QueryBudget};
use sparq_text::{prepare_text, query_text, query_text_with_budget, TextIndex};

const DATA: &str = r#"
    <http://ex/post1> <http://ex/title> "The quick brown fox" .
    <http://ex/post1> <http://ex/author> <http://ex/alice> .
    <http://ex/post2> <http://ex/title> "Fox hunting banned" .
    <http://ex/post2> <http://ex/author> <http://ex/bob> .
    <http://ex/post3> <http://ex/title> "Lazy dog sleeping" .
    <http://ex/post3> <http://ex/author> <http://ex/alice> .
"#;

fn setup() -> (Graph, TextIndex) {
    let g = Graph::load_str(DATA, "ntriples").unwrap();
    let idx = TextIndex::build(&g);
    (g, idx)
}

fn iri(t: &Option<Term>) -> String {
    match t {
        Some(Term::NamedNode(n)) => n.as_str().to_string(),
        other => panic!("expected an IRI, got {other:?}"),
    }
}

const PREFIX: &str = "PREFIX text: <http://sparq.dev/text#>";

/// The budgeted entry point under an unlimited budget returns exactly what the
/// unbudgeted one does — same hits, same rewrite path.
#[test]
fn with_budget_unlimited_matches_unbudgeted() {
    let (g, idx) = setup();
    let q = format!(
        "{PREFIX}
         SELECT ?post WHERE {{ ?post <http://ex/title> ?t . ?t text:matches \"fox\" }}"
    );
    let unbudgeted = query_text(&g, &q, &idx).unwrap();
    let budgeted = query_text_with_budget(&g, &q, &idx, &QueryBudget::unlimited()).unwrap();
    let mut a: Vec<String> = unbudgeted.rows.iter().map(|r| iri(&r[0])).collect();
    let mut b: Vec<String> = budgeted.rows.iter().map(|r| iri(&r[0])).collect();
    a.sort();
    b.sort();
    assert_eq!(a, ["http://ex/post1", "http://ex/post2"]);
    assert_eq!(a, b);
}

/// A tight `max_rows` budget aborts the rewritten query: the budget is threaded
/// through the rewrite into the engine, not silently dropped.
#[test]
fn with_budget_max_rows_is_enforced_through_the_rewrite() {
    let (g, idx) = setup();
    // "fox" matches two posts; a 1-row working-set ceiling must refuse it.
    let q = format!(
        "{PREFIX}
         SELECT ?post WHERE {{ ?post <http://ex/title> ?t . ?t text:matches \"fox\" }}"
    );
    let budget = QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() };
    let err = query_text_with_budget(&g, &q, &idx, &budget).unwrap_err();
    assert!(err.to_lowercase().contains("budget"), "error {err:?} should mention the budget");
}

/// CONSTRUCT carrying a `text:` pattern: the non-SELECT arm of `rewrite_query`
/// is exercised, and the produced triples reflect the text hits.
#[test]
fn construct_query_with_text_pattern() {
    let (g, idx) = setup();
    let prepared = prepare_text(
        &g,
        &format!(
            "{PREFIX}
             CONSTRUCT {{ ?post <http://ex/hit> ?author }}
             WHERE {{
               ?post <http://ex/title> ?t .
               ?post <http://ex/author> ?author .
               ?t text:matches \"fox\" .
             }}"
        ),
        &idx,
    )
    .unwrap();
    let triples = construct_prepared(&g, &prepared).unwrap();
    let mut subjects: Vec<String> =
        triples.iter().map(|tr| tr.subject.to_string()).collect();
    subjects.sort();
    // Only post1 (quick brown fox) and post2 (fox hunting) match "fox".
    assert_eq!(subjects, ["<http://ex/post1>", "<http://ex/post2>"]);
    // Every constructed triple uses the CONSTRUCT-template predicate.
    assert!(triples.iter().all(|tr| tr.predicate.as_str() == "http://ex/hit"));
}

/// DESCRIBE carrying a `text:` pattern: the Describe arm of `rewrite_query`
/// rewrites, and DESCRIBE emits triples about the matched resources.
#[test]
fn describe_query_with_text_pattern() {
    let (g, idx) = setup();
    let prepared = prepare_text(
        &g,
        &format!(
            "{PREFIX}
             DESCRIBE ?post WHERE {{
               ?post <http://ex/title> ?t .
               ?t text:matches \"lazy\" .
             }}"
        ),
        &idx,
    )
    .unwrap();
    let triples = describe_prepared(&g, &prepared).unwrap();
    // post3 ("Lazy dog sleeping") is the only match; DESCRIBE returns its
    // outgoing triples (title + author), all subject = post3.
    assert!(!triples.is_empty(), "DESCRIBE of a matched resource yields its triples");
    assert!(
        triples.iter().all(|tr| tr.subject.to_string() == "<http://ex/post3>"),
        "every described triple is about the single matched post: {triples:?}"
    );
}

/// A magic pattern in each branch of a UNION: `rewrite_pattern` must recurse
/// into BOTH the left and the right arm.
#[test]
fn union_rewrites_both_branches() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?post WHERE {{
               {{ ?post <http://ex/title> ?t . ?t text:matches \"quick\" }}
               UNION
               {{ ?post <http://ex/title> ?t . ?t text:matches \"lazy\" }}
             }}"
        ),
        &idx,
    )
    .unwrap();
    let mut posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    posts.sort();
    // "quick" -> post1, "lazy" -> post3; the union of the two rewritten arms.
    assert_eq!(posts, ["http://ex/post1", "http://ex/post3"]);
}

/// A magic pattern under MINUS: the right (subtrahend) arm is rewritten, so the
/// matched rows are removed from the left.
#[test]
fn minus_rewrites_the_subtrahend() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?post WHERE {{
               ?post <http://ex/title> ?t .
               MINUS {{ ?post <http://ex/title> ?t2 . ?t2 text:matches \"fox\" }}
             }}"
        ),
        &idx,
    )
    .unwrap();
    let mut posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    posts.sort();
    // All three titles minus the two "fox" matches (post1, post2) => only post3.
    assert_eq!(posts, ["http://ex/post3"]);
}

/// A magic pattern under FILTER + a sub-SELECT projection + ORDER BY + DISTINCT:
/// the Filter/Project/OrderBy/Distinct arms of `rewrite_pattern` recurse into
/// the inner BGP. The FILTER additionally prunes one hit.
#[test]
fn filter_project_orderby_distinct_recurse_into_inner() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT DISTINCT ?author WHERE {{
               SELECT ?author WHERE {{
                 ?post <http://ex/title> ?t .
                 ?post <http://ex/author> ?author .
                 ?t text:matches \"fox\" .
                 FILTER(?author = <http://ex/alice>)
               }}
               ORDER BY ?author
             }}"
        ),
        &idx,
    )
    .unwrap();
    // post1 (alice) and post2 (bob) match "fox"; the FILTER keeps only alice.
    let authors: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    assert_eq!(authors, ["http://ex/alice"]);
}

/// A property path triple sharing a BGP with a magic pattern: spargebra lifts
/// the path into a `GraphPattern::Path` joined onto the BGP, exercising the
/// Path pass-through arm of `rewrite_pattern` while the BGP's magic pattern is
/// still rewritten.
#[test]
fn property_path_passes_through_while_bgp_is_rewritten() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?author WHERE {{
               ?post <http://ex/title>/<http://ex/title> ?t .
               ?post <http://ex/author> ?author .
               ?post <http://ex/title> ?t2 .
               ?t2 text:matches \"quick\" .
             }}"
        ),
        &idx,
    )
    .unwrap();
    // <title>/<title> is empty (title's object is a literal, not a subject of
    // another title), so the join with the path yields no rows; the query is
    // accepted and rewritten regardless. The point is the rewrite traverses the
    // Path node without error.
    assert!(r.rows.is_empty(), "title/title path has no solutions: {:?}", r.rows);
}

/// A triple with a VARIABLE predicate in the SAME BGP as a magic pattern is
/// left untouched (the predicate-passthrough arm), while the magic pattern is
/// still rewritten and joined.
#[test]
fn variable_predicate_triple_passes_through() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?p WHERE {{
               ?post ?p ?t .
               ?t text:matches \"fox\" .
             }}"
        ),
        &idx,
    )
    .unwrap();
    // ?t binds to the matched literals; ?p is the predicate that points at them
    // — only <http://ex/title> does (authors are IRIs, never text hits). Two
    // matches (post1, post2) both via the title predicate.
    let preds: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    assert_eq!(preds.len(), 2, "two fox titles: {preds:?}");
    assert!(preds.iter().all(|p| p == "http://ex/title"), "{preds:?}");
}

/// `text:score` on a variable that carries TWO match patterns is ambiguous —
/// the scorer cannot pick which match it ranks.
#[test]
fn ambiguous_text_score_over_two_matches_errors() {
    let (g, idx) = setup();
    let err = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?t ?s WHERE {{
               ?t text:matches \"fox\" .
               ?t text:matchesAny \"lazy\" .
               ?t text:score ?s .
             }}"
        ),
        &idx,
    )
    .unwrap_err();
    assert!(err.contains("ambiguous"), "error {err:?} should flag the ambiguity");
}

/// A `text:score` declared TWICE for one match is a duplicate.
#[test]
fn duplicate_text_score_errors() {
    let (g, idx) = setup();
    let err = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?t ?s1 ?s2 WHERE {{
               ?t text:matches \"fox\" .
               ?t text:score ?s1 .
               ?t text:score ?s2 .
             }}"
        ),
        &idx,
    )
    .unwrap_err();
    assert!(err.contains("duplicate text:score"), "error {err:?}");
}

/// `text:slop` on a variable with TWO match patterns is ambiguous (the slop
/// cannot be attached to a single `text:near`).
#[test]
fn ambiguous_text_slop_over_two_matches_errors() {
    let (g, idx) = setup();
    let err = query_text(
        &g,
        &format!(
            "{PREFIX}
             SELECT ?t WHERE {{
               ?t text:near \"fox\" .
               ?t text:matches \"lazy\" .
               ?t text:slop 2 .
             }}"
        ),
        &idx,
    )
    .unwrap_err();
    assert!(err.contains("ambiguous"), "error {err:?} should flag the ambiguity");
}
