//! Engine-level integration: `text:` magic predicates inside full SPARQL,
//! joining text hits to triples through the ordinary engine (`engine`
//! feature — the default).
#![cfg(feature = "engine")]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_text::{prepare_text, query_text, TextIndex};

const DATA: &str = r#"
    <http://ex/post1> <http://ex/title> "Autonomous driving milestones" .
    <http://ex/post1> <http://ex/author> <http://ex/alice> .
    <http://ex/post2> <http://ex/title> "The quick brown fox" .
    <http://ex/post2> <http://ex/author> <http://ex/bob> .
    <http://ex/post3> <http://ex/title> "Fox hunting banned" .
    <http://ex/post3> <http://ex/author> <http://ex/alice> .
    <http://ex/post4> <http://ex/title> "Café culture in ΣΟΦΊΑ"@en .
    <http://ex/post4> <http://ex/author> <http://ex/eve> .
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

#[test]
fn join_text_hits_to_triples() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post ?author WHERE {
             ?post <http://ex/title> ?title .
             ?post <http://ex/author> ?author .
             ?title text:matches "fox" .
           }"#,
        &idx,
    )
    .unwrap();
    let mut posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    posts.sort();
    assert_eq!(posts, ["http://ex/post2", "http://ex/post3"]);
}

#[test]
fn score_variable_and_order_by() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post ?s WHERE {
             ?post <http://ex/title> ?title .
             ?title text:matches "fox" .
             ?title text:score ?s .
           } ORDER BY DESC(?s)"#,
        &idx,
    )
    .unwrap();
    assert_eq!(r.rows.len(), 2);
    // Scores are bound xsd:doubles, descending.
    let scores: Vec<f64> = r
        .rows
        .iter()
        .map(|row| match &row[1] {
            Some(Term::Literal(l)) => {
                assert_eq!(l.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#double");
                l.value().parse().unwrap()
            }
            other => panic!("expected a score literal, got {other:?}"),
        })
        .collect();
    assert!(scores[0] >= scores[1]);
    assert!(scores.iter().all(|s| *s > 0.0));
}

#[test]
fn matches_any_and_prefix() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title .
             ?title text:matchesAny "fox auto*" .
           }"#,
        &idx,
    )
    .unwrap();
    let mut posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    posts.sort();
    assert_eq!(posts, ["http://ex/post1", "http://ex/post2", "http://ex/post3"]);
}

#[test]
fn unicode_and_language_tagged_join() {
    let (g, idx) = setup();
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title .
             ?title text:matches "σοφία café" .
           }"#,
        &idx,
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(iri(&r.rows[0][0]), "http://ex/post4");
}

#[test]
fn magic_pattern_inside_optional_and_no_match() {
    let (g, idx) = setup();
    // OPTIONAL { ... text:matches } — rewrite recurses into sub-patterns.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post ?hit WHERE {
             ?post <http://ex/title> ?title .
             OPTIONAL { ?title text:matches "banned" . BIND(?title AS ?hit) }
           }"#,
        &idx,
    )
    .unwrap();
    assert_eq!(r.rows.len(), 4);
    assert_eq!(r.rows.iter().filter(|row| row[1].is_some()).count(), 1);

    // No token matches -> empty VALUES -> zero rows, not an error.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title . ?title text:matches "zzz" .
           }"#,
        &idx,
    )
    .unwrap();
    assert!(r.rows.is_empty());
}

#[test]
fn passthrough_without_text_patterns() {
    let (g, idx) = setup();
    let q = "SELECT ?s WHERE { ?s <http://ex/author> <http://ex/alice> }";
    let through_text = query_text(&g, q, &idx).unwrap();
    let plain = sparq_engine::query(&g, q).unwrap();
    assert_eq!(through_text.rows.len(), plain.rows.len());
    assert_eq!(through_text.rows.len(), 2);
}

#[test]
fn prepared_composition_ask() {
    let (g, idx) = setup();
    let prepared = prepare_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           ASK { ?post <http://ex/title> ?t . ?t text:matches "milestones" }"#,
        &idx,
    )
    .unwrap();
    assert!(sparq_engine::ask_prepared(&g, &prepared).unwrap());
}

/// `text:phrase` over a positions-enabled fixture: the adjacent-in-order
/// tokens match, the same tokens at non-adjacent positions do not, and order
/// is significant — exercised end-to-end through the engine.
#[test]
fn phrase_matches_adjacent_tokens_only() {
    let data = r#"
        <http://ex/post1> <http://ex/title> "the quick brown fox" .
        <http://ex/post2> <http://ex/title> "quick and brown fox" .
        <http://ex/post3> <http://ex/title> "a slow brown bear" .
    "#;
    let g = Graph::load_str(data, "ntriples").unwrap();
    let idx = TextIndex::build_with_positions(&g);

    // "quick brown" is adjacent+in-order only in post1; post2 has the same two
    // tokens separated by "and" (a near-miss the BM25 AND search would catch).
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title .
             ?title text:phrase "quick brown" .
           }"#,
        &idx,
    )
    .unwrap();
    let mut posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    posts.sort();
    assert_eq!(posts, ["http://ex/post1"]);

    // Order is significant: "brown quick" never occurs adjacently.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title . ?title text:phrase "brown quick" .
           }"#,
        &idx,
    )
    .unwrap();
    assert!(r.rows.is_empty());

    // The analyzer (casefolding) is honoured, same as indexing.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE {
             ?post <http://ex/title> ?title . ?title text:phrase "QUICK Brown" .
           }"#,
        &idx,
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(iri(&r.rows[0][0]), "http://ex/post1");
}

/// `text:phrase` against the cheap (positionless) default index is a hard,
/// clearly-worded query error — not a silent empty result.
#[test]
fn phrase_without_positions_errors() {
    let (g, idx) = setup(); // TextIndex::build — no positions.
    let err = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE { ?post <http://ex/title> ?t . ?t text:phrase "brown fox" }"#,
        &idx,
    )
    .unwrap_err();
    assert!(err.contains("positions-enabled index"), "error {err:?}");
    assert!(err.contains("build_with_positions"), "error {err:?}");
}

/// `text:near` end-to-end: the proximity/slop variant joins ranked hits to
/// triples; `text:slop N` widens the gap budget; `text:score` binds the
/// proximity score; tighter matches rank first under ORDER BY.
#[test]
fn near_with_slop_and_score() {
    let data = r#"
        <http://ex/post1> <http://ex/title> "quick brown fox" .
        <http://ex/post2> <http://ex/title> "quick lazy brown dog" .
        <http://ex/post3> <http://ex/title> "quick the lazy brown bird" .
        <http://ex/post4> <http://ex/title> "brown quick reversed" .
    "#;
    let g = Graph::load_str(data, "ntriples").unwrap();
    let idx = TextIndex::build_with_positions(&g);

    // Default slop (0) == exact adjacency: only post1.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE { ?post <http://ex/title> ?t . ?t text:near "quick brown" }"#,
        &idx,
    )
    .unwrap();
    let posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    assert_eq!(posts, ["http://ex/post1"], "default slop is exact adjacency");

    // text:slop 2 admits post1 (gap 0), post2 (gap 1), post3 (gap 2); the
    // reversed post4 never matches. text:score binds the proximity score; ORDER
    // BY DESC ranks tightest first.
    let r = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post ?s WHERE {
             ?post <http://ex/title> ?t .
             ?t text:near "quick brown" .
             ?t text:slop 2 .
             ?t text:score ?s .
           } ORDER BY DESC(?s)"#,
        &idx,
    )
    .unwrap();
    let posts: Vec<String> = r.rows.iter().map(|row| iri(&row[0])).collect();
    assert_eq!(
        posts,
        ["http://ex/post1", "http://ex/post2", "http://ex/post3"],
        "ranked tightest-first; reversed excluded"
    );
    let scores: Vec<f64> = r
        .rows
        .iter()
        .map(|row| match &row[1] {
            Some(Term::Literal(l)) => {
                assert_eq!(l.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#double");
                l.value().parse().unwrap()
            }
            other => panic!("expected a score literal, got {other:?}"),
        })
        .collect();
    // 1/(1+gap) for gaps 0,1,2 — strictly decreasing. The score is computed in
    // f32 then widened to the xsd:double, so compare against the f32 value.
    let want = |gap: u32| f64::from(1.0f32 / (1.0 + gap as f32));
    assert_eq!(scores, [want(0), want(1), want(2)]);
    assert!(scores.windows(2).all(|w| w[0] > w[1]));
}

/// `text:near` against the cheap (positionless) default index is a hard query
/// error, like `text:phrase`.
#[test]
fn near_without_positions_errors() {
    let (g, idx) = setup(); // TextIndex::build — no positions.
    let err = query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?post WHERE { ?post <http://ex/title> ?t . ?t text:near "brown fox" }"#,
        &idx,
    )
    .unwrap_err();
    assert!(err.contains("positions-enabled index"), "error {err:?}");
    assert!(err.contains("build_with_positions"), "error {err:?}");
}

#[test]
fn rewrite_errors() {
    let (g, idx) = setup();
    let cases = [
        // Query string must be a constant literal.
        (
            "SELECT ?t WHERE { ?t <http://sparq.dev/text#matches> ?q }",
            "constant query-string literal",
        ),
        // Unknown text: predicate is a typo guard (`fuzzy` is now feature-gated).
        // sq-lsp7k.14
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#fuzzzy> "fox" }"#,
            "unknown magic predicate",
        ),
        // text:score needs a match pattern on the same variable.
        (
            "SELECT ?t WHERE { ?t <http://sparq.dev/text#score> ?s }",
            "no text:matches",
        ),
        // text:score object must be a variable.
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#matches> "fox" . ?t <http://sparq.dev/text#score> "1.0" }"#,
            "must be a variable",
        ),
        // Subject must be a variable.
        (
            r#"SELECT ?t WHERE { <http://ex/post1> <http://sparq.dev/text#matches> "fox" }"#,
            "must be a variable",
        ),
        // text:score is not valid for text:phrase (boolean adjacency, unranked).
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#phrase> "fox" . ?t <http://sparq.dev/text#score> ?s }"#,
            "not valid for text:phrase",
        ),
        // text:slop must be a non-negative integer.
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#near> "fox" . ?t <http://sparq.dev/text#slop> "wide" }"#,
            "non-negative integer",
        ),
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#near> "fox" . ?t <http://sparq.dev/text#slop> "-1" }"#,
            "non-negative integer",
        ),
        // text:slop with no text:near on the same variable.
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#matches> "fox" . ?t <http://sparq.dev/text#slop> 2 }"#,
            "only valid alongside text:near",
        ),
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#slop> 2 }"#,
            "no text:near",
        ),
        // Duplicate text:slop on one text:near.
        (
            r#"SELECT ?t WHERE { ?t <http://sparq.dev/text#near> "fox" . ?t <http://sparq.dev/text#slop> 1 . ?t <http://sparq.dev/text#slop> 2 }"#,
            "duplicate text:slop",
        ),
    ];
    for (q, needle) in cases {
        let err = query_text(&g, q, &idx).unwrap_err();
        assert!(err.contains(needle), "query {q:?}: error {err:?} should contain {needle:?}");
    }
}
