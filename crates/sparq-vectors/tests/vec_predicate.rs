//! End-to-end tests for the `vec:` magic predicates (sq-k6ex): real SPARQL
//! queries with `vec:nearest` / `vec:search` evaluated against a tiny in-memory
//! `.spqv` vector store, asserting the expected neighbours. [OPUS-4.8]
#![cfg(feature = "vec-predicate")]

use oxrdf::{NamedNode, Term, Triple};
use sparq_core::Graph;
use sparq_vectors::{query_vec, QueryResult, VectorStore};

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sparq_vec_pred_{}_{}.spqv",
        std::process::id(),
        name
    ));
    p
}

/// Five entities on the unit circle; a-style vectors point along +x, b-style
/// along +y, plus a near-x entity. The store is built (not finalized) in memory
/// — `nearest_exact` scans the build-phase backing directly.
fn fixture(name: &str) -> (Graph, VectorStore) {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/label> "alpha" .
        <http://ex/b> <http://ex/label> "beta" .
        <http://ex/c> <http://ex/label> "gamma" .
        <http://ex/d> <http://ex/label> "delta" .
        <http://ex/e> <http://ex/label> "epsilon" .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path(name), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // near +x
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // near +y
    (g, store)
}

/// Collects the single-projection IRI column of a result, in row order.
fn iris(r: &QueryResult, col: usize) -> Vec<String> {
    r.rows
        .iter()
        .filter_map(|row| match &row[col] {
            Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn nearest_by_query_vector() {
    let (g, store) = fixture("nearest_vec");
    // Query "1,0" → the two most +x-aligned: a (exact) then c.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
        &store,
    )
    .unwrap();
    let got = iris(&r, 0);
    assert_eq!(
        got,
        vec!["http://ex/a".to_string(), "http://ex/c".to_string()],
        "{r:?}"
    );
}

#[test]
fn nearest_by_seed_iri_excludes_self() {
    let (g, store) = fixture("nearest_seed");
    // Neighbours of <a> (which is +x); a itself is excluded → c is nearest.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( <http://ex/a> 1 ) }",
        &store,
    )
    .unwrap();
    let got = iris(&r, 0);
    assert_eq!(
        got,
        vec!["http://ex/c".to_string()],
        "seed must be excluded: {r:?}"
    );
}

#[test]
fn nearest_joins_to_surrounding_bgp() {
    let (g, store) = fixture("nearest_join");
    // The neighbours' labels, joined through the ordinary triple pattern.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?label WHERE {
           ?node vec:nearest ( \"0,1\" 2 ) .
           ?node <http://ex/label> ?label .
         }",
        &store,
    )
    .unwrap();
    let mut labels: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| match &row[0] {
            Some(Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        })
        .collect();
    labels.sort();
    // "0,1" → b (+y) and e (near +y): labels beta, epsilon.
    assert_eq!(
        labels,
        vec!["beta".to_string(), "epsilon".to_string()],
        "{r:?}"
    );
}

/// [GPT-5.6] sq-1ijoc: RDF 1.2 ground triple terms returned by vector search
/// must survive the rewrite's inline VALUES table, including nested terms.
#[test]
fn nearest_returns_nested_ground_triple_term() {
    let g = Graph::load_str(
        r#"
        <http://ex/r> <http://ex/reifies> <<( <http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <http://ex/o> )>> )>> .
        "#,
        "ntriples",
    )
    .unwrap();
    let named = |iri: &str| NamedNode::new(iri).unwrap();
    let inner = Term::Triple(Box::new(Triple::new(
        named("http://ex/a"),
        named("http://ex/q"),
        named("http://ex/o"),
    )));
    let expected = Term::Triple(Box::new(Triple::new(
        named("http://ex/s"),
        named("http://ex/p"),
        inner,
    )));
    let triple_id = g
        .id_of(&expected)
        .expect("fixture triple term must be dictionary encoded");
    let mut store = VectorStore::create(tmp_path("triple_term"), 2).unwrap();
    store.put(triple_id, &[1.0, 0.0]).unwrap();

    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 1 ) }",
        &store,
    )
    .unwrap();

    assert_eq!(r.rows, vec![vec![Some(expected)]], "{r:?}");
}

#[test]
fn search_binds_score_and_orders() {
    let (g, store) = fixture("search_score");
    // vec:search binds the cosine score; ORDER BY DESC recovers best-first.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score WHERE {
           ( ?node ?score ) vec:search ( \"1,0\" 3 )
         } ORDER BY DESC(?score)",
        &store,
    )
    .unwrap();
    let order = iris(&r, 0);
    // a (cos 1.0) > c (cos ~0.994) > b/e/d further; top-3 best-first.
    assert_eq!(
        order,
        vec![
            "http://ex/a".to_string(),
            "http://ex/c".to_string(),
            "http://ex/e".to_string()
        ],
        "{r:?}"
    );
    // a's score is ~1.0 (exact alignment).
    let a_score = match &r.rows[0][1] {
        Some(Term::Literal(l)) => l.value().parse::<f64>().unwrap(),
        other => panic!("expected score literal, got {other:?}"),
    };
    assert!(
        (a_score - 1.0).abs() < 1e-5,
        "a should be cosine 1.0, got {a_score}"
    );
}

#[test]
fn unembedded_seed_yields_no_rows() {
    let (g, store) = fixture("missing_seed");
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( <http://ex/nope> 5 ) }",
        &store,
    )
    .unwrap();
    assert!(
        r.is_empty(),
        "an unembedded/absent seed has no neighbours: {r:?}"
    );
}

#[test]
fn dimension_mismatch_is_an_error() {
    let (g, store) = fixture("dim_mismatch");
    let err = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0,0\" 2 ) }",
        &store,
    )
    .unwrap_err();
    assert!(
        err.contains("dims"),
        "expected a dimension error, got: {err}"
    );
}

#[test]
fn unknown_vec_predicate_is_rejected() {
    let (g, store) = fixture("unknown_pred");
    let err = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:teleport ( \"1,0\" 2 ) }",
        &store,
    )
    .unwrap_err();
    assert!(err.contains("unknown magic predicate"), "got: {err}");
}

#[test]
fn no_vec_predicate_passes_through() {
    let (g, store) = fixture("passthrough");
    // A query with no vec: predicate must evaluate exactly as the plain engine would.
    let r = query_vec(
        &g,
        "SELECT ?node WHERE { ?node <http://ex/label> \"alpha\" }",
        &store,
    )
    .unwrap();
    assert_eq!(iris(&r, 0), vec!["http://ex/a".to_string()], "{r:?}");
}

/// [OPUS-4.8] Regression: a query that legitimately uses the RDF collection
/// vocabulary (`rdf:first`/`rdf:rest`) but NO `vec:` predicate must pass through
/// unchanged — the rewrite must not strip those triples (it previously did,
/// unconditionally). Here we query an actual `rdf:first` cell and expect the
/// element back.
#[test]
fn rdf_collection_without_vec_predicate_is_preserved() {
    let g = Graph::load_str(
        r#"
        <http://ex/list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <http://ex/elem> .
        <http://ex/list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .
        "#,
        "ntriples",
    )
    .unwrap();
    // An empty store (no fingerprint) — exercises the unbound/legacy path too.
    let store = VectorStore::create(tmp_path("collection_passthrough"), 2).unwrap();
    let r = query_vec(
        &g,
        "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
         SELECT ?elem WHERE { <http://ex/list> rdf:first ?elem }",
        &store,
    )
    .unwrap();
    assert_eq!(iris(&r, 0), vec!["http://ex/elem".to_string()], "{r:?}");
}

/// [SONNET-4.6] (sq-tb9p0) VG-GOV-3: the VG-VOC-1 unrecognised-predicate hard error states the
/// highest `vec:` vocabulary revision this build implements, so a query written against a newer
/// spec revision fails with the version gap named.
#[test]
fn unknown_vec_predicate_error_reports_the_vocabulary_revision() {
    let (g, store) = fixture("unknown_pred_rev");
    let err = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:teleport ( \"1,0\" 2 ) }",
        &store,
    )
    .unwrap_err();
    assert!(
        err.contains(&format!(
            "vocabulary revision {}",
            sparq_vectors::vocab::VOCAB_REVISION
        )),
        "the VG-VOC-1 error must report the implemented vocabulary revision, got: {err}"
    );
}

/// [SONNET-4.6] (sq-tb9p0) VG-TIE-1 through the mainline `vec:` surface: when candidates tie on
/// score at the k boundary, top-k MEMBERSHIP is decided by ascending Unicode-codepoint order of
/// the candidates' canonical N-Triples serialisations — reproducible across implementations,
/// unlike the internal dictionary-id order. The tie group mixes an IRI with two inlined numeric
/// literals, whose dict ids order NUMERICALLY (`"2"` below `"10"`, both above every IRI id) while
/// the N-Triples keys order `"10"^^…` < `"2"^^…` < `<http://ex/z-tied>` — so an id-order
/// tie-break would admit `z-tied` where the VG-TIE-1 rule admits `"10"^^xsd:integer`.
#[test]
fn boundary_score_tie_membership_is_by_ntriples_codepoint_order() {
    let g = Graph::load_str(
        r#"
        <http://ex/top> <http://ex/num> "10"^^<http://www.w3.org/2001/XMLSchema#integer> .
        <http://ex/top> <http://ex/num> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
        <http://ex/z-tied> <http://ex/label> "z" .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let num = |n: &str| {
        g.id_of(&Term::Literal(oxrdf::Literal::new_typed_literal(
            n,
            oxrdf::vocab::xsd::INTEGER,
        )))
        .unwrap()
    };
    let mut store = VectorStore::create(tmp_path("tie_break"), 2).unwrap();
    store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0 to the query
    store.put(id("http://ex/z-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6
    store.put(num("10"), &[0.6, -0.8]).unwrap(); // cos 0.6
    store.put(num("2"), &[3.0, 4.0]).unwrap(); // cos 0.6 (scaled twin)
    // k=2: one slot above the boundary (`top`), one slot for the three-way 0.6 tie — the
    // smallest N-Triples key `"10"^^xsd:integer` must be the admitted member.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
        &store,
    )
    .unwrap();
    let mut got: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| row[0].as_ref().map(|t| t.to_string()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
            "<http://ex/top>".to_string()
        ],
        "{r:?}"
    );
}

/// [SONNET-4.6] (#2445 review) The VG-TIE-1 embeddable-domain guard through the mainline `vec:`
/// surface: a blank-node candidate has a document-local N-Triples label — no tie-break key that
/// is stable across implementations — so when its term (not its score alone) would decide top-k
/// membership, the query MUST fail closed with a hard error, never rank the label. The blank
/// node ties an IRI at the k boundary, exactly where the key would become load-bearing.
#[test]
fn blank_node_candidate_at_the_boundary_is_a_hard_query_error() {
    let g = Graph::load_str(
        r#"
        <http://ex/top> <http://ex/p> "t" .
        <http://ex/a-tied> <http://ex/p> "a" .
        _:tied <http://ex/p> "b" .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let bnode = g
        .id_of(&Term::BlankNode(oxrdf::BlankNode::new_unchecked("tied")))
        .unwrap();
    let mut store = VectorStore::create(tmp_path("bnode_tie"), 2).unwrap();
    store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0
    store.put(id("http://ex/a-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6
    store.put(bnode, &[0.6, -0.8]).unwrap(); // cos 0.6 — tied blank node
    // k=2: one slot for the two-way 0.6 boundary tie the blank node sits in → hard error.
    let err = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
        &store,
    )
    .unwrap_err();
    assert!(err.contains("blank node"), "expected the fail-closed domain error, got: {err}");
    // k=1: the unique winner is an IRI; the blank node stays strictly below the boundary and
    // is rejected by score alone — the query succeeds.
    let r = query_vec(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 1 ) }",
        &store,
    )
    .unwrap();
    assert_eq!(iris(&r, 0), vec!["http://ex/top".to_string()], "{r:?}");
}

/// [SONNET-4.6] (sq-vv2l0) VG-DEG-3 of `site/specs/sparql-vector-genai.typ`: `k = 0` yields ZERO
/// solutions and is NOT an error. `k` is typed as a non-negative integer (VG-TYP-2), so `0` parses
/// and reaches the search; both answer-exact entry points truncate/return empty at `k = 0`. The
/// spec's degenerate cases are only meaningful if the empty answer is a normal answer.
#[test]
fn k_zero_yields_zero_solutions_and_is_not_an_error() {
    let (g, store) = fixture("k_zero");
    for q in [
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 0 ) }",
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( <http://ex/a> 0 ) }",
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score WHERE { ( ?node ?score ) vec:search ( \"1,0\" 0 ) }",
    ] {
        let r = query_vec(&g, q, &store)
            .unwrap_or_else(|e| panic!("k = 0 must not be a query error, got: {e}\nquery: {q}"));
        assert!(r.is_empty(), "k = 0 must yield zero solutions: {r:?}");
    }
}

/// [SONNET-4.6] (sq-vv2l0) VG-GRM-2/VG-ERR-2: a non-variable SCORE position in `vec:search` is a
/// hard query error, exactly as a non-variable neighbour position is — both subject elements go
/// through the same bare-variable requirement. Pins the branch the spec previously left
/// unconfirmed: an author who writes a constant where the score binding belongs gets a loud
/// failure, never an empty or partial result.
#[test]
fn non_variable_score_position_is_a_hard_query_error() {
    let (g, store) = fixture("score_not_var");
    for (label, q) in [
        (
            "literal score position",
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE { ( ?node \"0.9\" ) vec:search ( \"1,0\" 2 ) }",
        ),
        (
            "IRI score position",
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE { ( ?node <http://ex/a> ) vec:search ( \"1,0\" 2 ) }",
        ),
    ] {
        let err = query_vec(&g, q, &store)
            .err()
            .unwrap_or_else(|| panic!("a non-variable score position ({label}) must be an error"));
        assert!(
            err.contains("second vec:search subject element") && err.contains("must be a variable"),
            "expected the score-position variable error ({label}), got: {err}"
        );
    }
}

/// [SONNET-4.6] (sq-tb9p0) VG-TIE-1 through the FILTERED `vec:` execution path — the branch the
/// unfiltered tie test above cannot reach (a constrained neighbour variable returns through the
/// cost-model'd filtered search, not `search_tied`). VG-FILT-2 makes this load-bearing: in
/// answer-exact mode the filtered top-k MUST equal post-filtering the unfiltered (tie-broken)
/// retrieval, so boundary-tie membership in the ADMITTED pool must follow the same N-Triples
/// codepoint rule, with the seed excluded before the boundary is determined.
#[cfg(feature = "filtered-ann")]
mod filtered_boundary_tiebreak {
    use super::tmp_path;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;
    use sparq_vectors::{query_vec, QueryResult, VectorStore};

    /// `<sel> <admits> …` marks the BGP-admitted candidates: `top` plus a three-way cosine-0.6
    /// tie group whose dictionary-id order and N-Triples key order genuinely DISAGREE (the dict
    /// inlines numeric literals into a high-id numeric-order range, so ids order `z-tied` <
    /// `"2"` < `"10"` while the N-Triples keys order `"10"^^…` < `"2"^^…` < `<http://ex/z-tied>`).
    /// `near-but-out` (cos ~0.99 — closer than the whole tie group) is NOT admitted, so the mask
    /// genuinely restricts the pool. An id-order tie-break admits `z-tied`; VG-TIE-1 admits
    /// `"10"^^xsd:integer` — the tests discriminate the two.
    fn fixture(name: &str) -> (Graph, VectorStore) {
        let g = Graph::load_str(
            r#"
            <http://ex/sel> <http://ex/admits> <http://ex/top> .
            <http://ex/sel> <http://ex/admits> <http://ex/z-tied> .
            <http://ex/sel> <http://ex/admits> "10"^^<http://www.w3.org/2001/XMLSchema#integer> .
            <http://ex/sel> <http://ex/admits> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
            <http://ex/near-but-out> <http://ex/label> "out" .
            "#,
            "ntriples",
        )
        .unwrap();
        let id = |s: &str| {
            g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
                .unwrap()
        };
        let num = |n: &str| {
            g.id_of(&Term::Literal(oxrdf::Literal::new_typed_literal(
                n,
                oxrdf::vocab::xsd::INTEGER,
            )))
            .unwrap()
        };
        let mut store = VectorStore::create(tmp_path(name), 2).unwrap();
        store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0, admitted
        store.put(id("http://ex/z-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6, admitted
        store.put(num("10"), &[0.6, -0.8]).unwrap(); // cos 0.6, admitted
        store.put(num("2"), &[3.0, 4.0]).unwrap(); // cos 0.6 (scaled twin), admitted
        store.put(id("http://ex/near-but-out"), &[0.9, 0.1]).unwrap(); // cos ~0.99, NOT admitted
        (g, store)
    }

    fn sorted_terms(r: &QueryResult) -> Vec<String> {
        let mut got: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| row[0].as_ref().map(|t| t.to_string()))
            .collect();
        got.sort();
        got
    }

    const TEN: &str = "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>";

    /// Vector-query form, k=2 restricted to the admitted set: `top` is strictly above the
    /// boundary; ONE slot remains for the three-way 0.6 tie → the smallest N-Triples key
    /// `"10"^^xsd:integer` must be the admitted member (an id-order tie-break — the previous
    /// filtered early-return behaviour — would have admitted `z-tied` instead), and the closer
    /// non-admitted `near-but-out` must stay excluded by the mask.
    #[test]
    fn vector_query_boundary_tie_membership_is_by_ntriples_key() {
        let (g, store) = fixture("filt_tie_vec");
        let r = query_vec(
            &g,
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE {
               <http://ex/sel> <http://ex/admits> ?node .
               ?node vec:nearest ( \"1,0\" 2 ) .
             }",
            &store,
        )
        .unwrap();
        assert_eq!(
            sorted_terms(&r),
            vec![TEN.to_string(), "<http://ex/top>".to_string()],
            "filtered boundary-tie membership must follow the N-Triples key (VG-TIE-1) and the \
             mask must exclude near-but-out (VG-FILT-2/3): {r:?}"
        );
    }

    /// Node-seed form, k=1: the seed `top` sits in the mask and must leave the admitted pool
    /// BEFORE the boundary is determined, so the boundary IS the three-way 0.6 tie and the
    /// smallest N-Triples key `"10"^^xsd:integer` wins (the previous over-fetch-then-drop-seed
    /// behaviour tie-broke by id and returned `z-tied`).
    #[test]
    fn node_seed_boundary_tie_excludes_seed_before_the_boundary() {
        let (g, store) = fixture("filt_tie_seed");
        let r = query_vec(
            &g,
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE {
               <http://ex/sel> <http://ex/admits> ?node .
               ?node vec:nearest ( <http://ex/top> 1 ) .
             }",
            &store,
        )
        .unwrap();
        assert_eq!(
            sorted_terms(&r),
            vec![TEN.to_string()],
            "the seed must be excluded before the boundary; the tie must break by N-Triples key: {r:?}"
        );
    }
}

/// [SONNET-4.6] (sq-tb9p0) VG-MET-4 through the mainline `vec:` surface: a store whose persisted
/// provenance declares a non-cosine metric is REJECTED by the cosine-only query surface — a hard
/// error, not well-formed meaningless scores. Needs `spqv-provenance` to bind a provenance.
#[cfg(feature = "spqv-provenance")]
mod declared_metric {
    use super::{fixture, iris, tmp_path};
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;
    use sparq_vectors::{
        query_vec, EmbeddingMetric as Metric, EmbeddingProvenance, Normalization, VectorStore,
    };

    #[test]
    fn non_cosine_declared_metric_is_rejected() {
        let g = Graph::load_str(
            "<http://ex/a> <http://ex/label> \"alpha\" .",
            "ntriples",
        )
        .unwrap();
        let id = g
            .id_of(&Term::NamedNode(NamedNode::new("http://ex/a").unwrap()))
            .unwrap();
        let mut store = VectorStore::create(tmp_path("euclid_metric"), 2)
            .unwrap()
            .with_provenance(EmbeddingProvenance::new(
                "m",
                Metric::Euclidean,
                Normalization::None,
            ));
        store.put(id, &[1.0, 0.0]).unwrap();
        let err = query_vec(
            &g,
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 1 ) }",
            &store,
        )
        .unwrap_err();
        assert!(
            err.contains("euclidean") && err.contains("cosine"),
            "expected the cross-metric rejection to name both metrics, got: {err}"
        );
    }

    #[test]
    fn declared_cosine_metric_is_accepted() {
        let (g, store) = fixture("cosine_metric_base");
        // Rebind the same vectors under a declared-cosine provenance: the mainline accepts it.
        let mut prov_store = VectorStore::create(tmp_path("cosine_metric"), 2)
            .unwrap()
            .with_provenance(EmbeddingProvenance::new(
                "m",
                Metric::Cosine,
                Normalization::L2,
            ));
        for (id, v) in store.iter() {
            prov_store.put(id, v).unwrap();
        }
        let r = query_vec(
            &g,
            "PREFIX vec: <http://sparq.dev/vec#>
             SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 1 ) }",
            &prov_store,
        )
        .unwrap();
        assert_eq!(iris(&r, 0), vec!["http://ex/a".to_string()], "{r:?}");
    }
}
