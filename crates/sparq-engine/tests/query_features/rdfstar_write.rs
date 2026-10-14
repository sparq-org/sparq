//! sq-t267 — RDF 1.2 triple-term coverage on the WRITE/INGEST side.
//!
//! `exec.rs`'s `rdf_star_*` cluster already proves strong triple-term *read* coverage
//! (concrete/nested triple terms, triple-term-pattern variable positions, the triple
//! built-ins). The audit found the gaps were on the WRITE side:
//!
//! 1. quoted/triple terms as INSERT DATA / DELETE DATA objects and inside
//!    DELETE/INSERT … WHERE templates;
//! 2. CONSTRUCT templates that BUILD triple terms;
//! 3. `rdf:reifies` / reifier (RDF 1.2 SEP-0006) round-trip through the store and
//!    the N-Triples serializer;
//! 4. dir-langString (base direction) used as data flowing through UPDATE/CONSTRUCT.
//!
//! Each test below was first confirmed empirically against the live engine (probe
//! pass) and pins the ACTUAL behaviour with hand-computed expected results.
//!
//! ## RDF 1.2 data-model reminder (load-bearing for these assertions)
//! - A **triple term** is written `<<( s p o )>>` and is **object-only** — it is a
//!   single RDF term that may appear only in the object slot. spargebra REJECTS it
//!   in subject position (and the CONSTRUCT object-builder drops a template that
//!   puts one in subject position). These tests do not "fake" subject-position
//!   triple terms; they pin that object-position works and that the illegal subject
//!   case is a documented non-feature.
//! - The **annotation/reifier syntax** `<< s p o >>` (and `s p o {| … |}` / `~ :r`)
//!   is sugar that the PARSER desugars to a fresh reifier node `_:r` with
//!   `_:r rdf:reifies <<( s p o )>>`. So an INSERT DATA / template that writes
//!   `<< … >>` produces a `rdf:reifies` triple, not a bare triple term.

use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term, Triple};
use sparq_core::Graph;
use sparq_engine::{construct, construct_ntriples, query, update};

const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

fn nn(s: &str) -> NamedNode {
    NamedNode::new_unchecked(s)
}

/// The triple term `<<( http://ex/{s} http://ex/age "{n}"^^xsd:integer )>>`.
fn age_triple_term(s: &str, n: i64) -> Term {
    Term::Triple(Box::new(Triple::new(
        nn(&format!("http://ex/{s}")),
        nn("http://ex/age"),
        Term::Literal(Literal::new_typed_literal(n.to_string(), xsd::INTEGER)),
    )))
}

/// All `?s ?p ?o` rows of `g` as sorted `"s|p|o"` strings (unbound -> "U").
fn all_rows(g: &Graph) -> Vec<String> {
    let r = query(g, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();
    let mut v: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    c.as_ref()
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "U".into())
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// GAP 1a — triple term as an INSERT DATA / DELETE DATA OBJECT
// ---------------------------------------------------------------------------

#[test]
fn insert_data_triple_term_object_roundtrips() {
    // INSERT DATA with an explicit triple term in OBJECT position stores it as a
    // structural `Term::Triple`, queryable back exactly.
    let g0 = Graph::load_str("", "turtle").unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> INSERT DATA { :s :annotates <<( :alice :age 30 )>> . }",
    )
    .unwrap();
    assert_eq!(g.len(), 1);
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?t WHERE { :s :annotates ?t }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Some(age_triple_term("alice", 30)));
    // The concrete quoted pattern matches the stored term.
    let n = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :annotates <<( :alice :age 30 )>> }",
    )
    .unwrap()
    .rows
    .len();
    assert_eq!(n, 1);
    // A non-matching concrete quoted pattern misses.
    let n = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :annotates <<( :alice :age 31 )>> }",
    )
    .unwrap()
    .rows
    .len();
    assert_eq!(n, 0);
}

#[test]
fn delete_data_triple_term_object_removes_exact_term() {
    // DELETE DATA with a triple-term object removes ONLY the matching reified
    // statement, leaving the other intact.
    let g0 = Graph::load_str(
        "PREFIX : <http://ex/> \
         :s :annotates <<( :alice :age 30 )>> . \
         :s :annotates <<( :bob :age 25 )>> .",
        "turtle",
    )
    .unwrap();
    assert_eq!(g0.len(), 2);
    let g = update(
        &g0,
        "PREFIX : <http://ex/> DELETE DATA { :s :annotates <<( :alice :age 30 )>> . }",
    )
    .unwrap();
    assert_eq!(
        all_rows(&g),
        vec![format!(
        "<http://ex/s>|<http://ex/annotates>|<<( <http://ex/bob> <http://ex/age> \"25\"^^<{}> )>>",
        xsd::INTEGER.as_str()
    )]
    );
    // Deleting a triple term that is not present is a no-op.
    let g2 = update(
        &g,
        "PREFIX : <http://ex/> DELETE DATA { :s :annotates <<( :carol :age 99 )>> . }",
    )
    .unwrap();
    assert_eq!(g2.len(), 1);
}

#[test]
fn insert_data_nested_triple_term_object_roundtrips() {
    // A triple term NESTED one level inside the object round-trips structurally.
    let g0 = Graph::load_str("", "turtle").unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> INSERT DATA { :x :p <<( :a :b <<( :c :d :e )>> )>> . }",
    )
    .unwrap();
    let r = query(&g, "PREFIX : <http://ex/> SELECT ?o WHERE { :x :p ?o }").unwrap();
    let inner = Term::Triple(Box::new(Triple::new(
        nn("http://ex/c"),
        nn("http://ex/d"),
        Term::NamedNode(nn("http://ex/e")),
    )));
    let outer = Term::Triple(Box::new(Triple::new(
        nn("http://ex/a"),
        nn("http://ex/b"),
        inner,
    )));
    assert_eq!(r.rows[0][0], Some(outer));
}

// ---------------------------------------------------------------------------
// GAP 1b — triple terms BUILT inside DELETE/INSERT … WHERE templates
// ---------------------------------------------------------------------------

#[test]
fn insert_where_builds_triple_term_object_from_solution() {
    // The INSERT template builds a triple term in OBJECT position from per-solution
    // bindings: one annotation triple per `?x :age ?a` solution.
    let g0 = Graph::load_str(
        "PREFIX : <http://ex/> :alice :age 30 . :bob :age 25 .",
        "turtle",
    )
    .unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> INSERT { :stmt :reify <<( ?x :age ?a )>> } WHERE { ?x :age ?a }",
    )
    .unwrap();
    // 2 source triples + 2 reify triples.
    assert_eq!(g.len(), 4);
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?t WHERE { :stmt :reify ?t }",
    )
    .unwrap();
    let mut got: Vec<String> = r
        .rows
        .iter()
        .map(|row| row[0].as_ref().unwrap().to_string())
        .collect();
    got.sort();
    let mut want = vec![
        age_triple_term("alice", 30).to_string(),
        age_triple_term("bob", 25).to_string(),
    ];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn delete_where_removes_triple_term_via_template() {
    // The DELETE template matches a triple-term object via the WHERE pattern and
    // removes exactly the FILTER-selected statement.
    let g0 = Graph::load_str(
        "PREFIX : <http://ex/> \
         :s :annotates <<( :alice :age 30 )>> . \
         :s :annotates <<( :bob :age 25 )>> .",
        "turtle",
    )
    .unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> \
         DELETE { :s :annotates <<( ?x :age ?a )>> } \
         WHERE  { :s :annotates <<( ?x :age ?a )>> FILTER(?a = 30) }",
    )
    .unwrap();
    assert_eq!(g.len(), 1);
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?t WHERE { :s :annotates ?t }",
    )
    .unwrap();
    assert_eq!(r.rows[0][0], Some(age_triple_term("bob", 25)));
}

#[test]
fn delete_insert_where_rewrites_a_reified_statement() {
    // Combined DELETE+INSERT: rewrite the reified `:alice :age 30` term to
    // `:alice :age 31`, keeping the annotation predicate.
    let g0 = Graph::load_str(
        "PREFIX : <http://ex/> :s :annotates <<( :alice :age 30 )>> .",
        "turtle",
    )
    .unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> \
         DELETE { :s :annotates <<( ?x :age ?a )>> } \
         INSERT { :s :annotates <<( ?x :age 31 )>> } \
         WHERE  { :s :annotates <<( ?x :age ?a )>> }",
    )
    .unwrap();
    assert_eq!(g.len(), 1);
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?t WHERE { :s :annotates ?t }",
    )
    .unwrap();
    assert_eq!(r.rows[0][0], Some(age_triple_term("alice", 31)));
}

// ---------------------------------------------------------------------------
// GAP 2 — CONSTRUCT templates that BUILD triple terms
// ---------------------------------------------------------------------------

#[test]
fn construct_builds_triple_term_object() {
    // CONSTRUCT instantiates a triple term in OBJECT position from each solution.
    let g = Graph::load_str(
        "PREFIX : <http://ex/> :alice :age 30 . :bob :age 25 .",
        "turtle",
    )
    .unwrap();
    let ts = construct(
        &g,
        "PREFIX : <http://ex/> CONSTRUCT { :stmt :reify <<( ?x :age ?a )>> } WHERE { ?x :age ?a }",
    )
    .unwrap();
    assert_eq!(ts.len(), 2);
    let mut objs: Vec<String> = ts.iter().map(|t| t.object.to_string()).collect();
    objs.sort();
    let mut want = vec![
        age_triple_term("alice", 30).to_string(),
        age_triple_term("bob", 25).to_string(),
    ];
    want.sort();
    assert_eq!(objs, want);
    // Every constructed triple has the expected constant subject/predicate.
    assert!(ts.iter().all(
        |t| t.subject.to_string() == "<http://ex/stmt>" && t.predicate == nn("http://ex/reify")
    ));
}

#[test]
fn construct_builds_nested_triple_term_object() {
    // A triple term nested one level inside the CONSTRUCT object builds correctly.
    let g = Graph::load_str("PREFIX : <http://ex/> :alice :age 30 .", "turtle").unwrap();
    let ts = construct(
        &g,
        "PREFIX : <http://ex/> CONSTRUCT { :s :p <<( ?x :stated <<( ?x :age ?a )>> )>> } WHERE { ?x :age ?a }",
    )
    .unwrap();
    assert_eq!(ts.len(), 1);
    let inner = age_triple_term("alice", 30);
    let outer = Term::Triple(Box::new(Triple::new(
        nn("http://ex/alice"),
        nn("http://ex/stated"),
        inner,
    )));
    assert_eq!(ts[0].object, outer);
}

#[test]
fn construct_triple_term_in_subject_position_is_dropped() {
    // RDF 1.2: a triple term is OBJECT-ONLY. A CONSTRUCT template that puts a triple
    // term in SUBJECT position produces an illegal triple, so it is silently dropped
    // (SPARQL §16.2: template triples with an illegal slot are not emitted). This
    // pins the documented non-feature — it is NOT a bug, and we do not fake it.
    let g = Graph::load_str("PREFIX : <http://ex/> :alice :age 30 .", "turtle").unwrap();
    let ts = construct(
        &g,
        "PREFIX : <http://ex/> CONSTRUCT { <<( ?x :age ?a )>> :asserted true } WHERE { ?x :age ?a }",
    )
    .unwrap();
    assert!(
        ts.is_empty(),
        "triple term in subject position must be dropped, got {ts:?}"
    );
}

// ---------------------------------------------------------------------------
// GAP 3 — rdf:reifies / reifier (SEP-0006) round-trip through store + serializer
// ---------------------------------------------------------------------------

#[test]
fn reifier_syntax_parses_to_rdf_reifies_and_roundtrips() {
    // The three SEP-0006 surface forms all parse to `?r rdf:reifies <<( s p o )>>`.
    // (a) `~ :r` annotation: produces the asserted base triple + the reifies triple.
    let g = Graph::load_str("PREFIX : <http://ex/> :alice :age 30 ~:r1 .", "turtle").unwrap();
    assert_eq!(
        all_rows(&g),
        vec![
            format!("<http://ex/alice>|<http://ex/age>|\"30\"^^<{}>", xsd::INTEGER.as_str()),
            format!("<http://ex/r1>|<{RDF_REIFIES}>|<<( <http://ex/alice> <http://ex/age> \"30\"^^<{}> )>>", xsd::INTEGER.as_str()),
        ]
    );
    // (b) explicit `rdf:reifies` triple with a `<<( )>>` object round-trips identically.
    let g2 = Graph::load_str(
        &format!("PREFIX : <http://ex/> :r1 <{RDF_REIFIES}> <<( :alice :age 30 )>> ."),
        "turtle",
    )
    .unwrap();
    let r = query(&g2, &format!("SELECT ?t WHERE {{ ?r <{RDF_REIFIES}> ?t }}")).unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Some(age_triple_term("alice", 30)));
}

#[test]
fn reifier_triple_term_serialises_to_canonical_form_and_reparses_via_turtle() {
    // A stored `rdf:reifies` triple with a triple-term object serialises through the
    // CONSTRUCT N-Triples writer in canonical `<<( … )>>` form. N-Triples is a syntactic
    // subset of Turtle, so the output re-loads via the Turtle loader to the identical
    // structural term. (Re-loading via the "ntriples" loader is a KNOWN GAP — see
    // `ntriples_triple_term_reparse_is_a_known_gap` / bead sq-hxgb.)
    let g = Graph::load_str(
        &format!("PREFIX : <http://ex/> :r1 <{RDF_REIFIES}> <<( :alice :age 30 )>> ."),
        "turtle",
    )
    .unwrap();
    let nt = construct_ntriples(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(
        nt.trim_end(),
        format!(
            "<http://ex/r1> <{RDF_REIFIES}> <<( <http://ex/alice> <http://ex/age> \"30\"^^<{}> )>> .",
            xsd::INTEGER.as_str()
        )
    );
    // The serialised form re-parses (via Turtle) to the identical structural term.
    let g2 = Graph::load_str(&nt, "turtle").unwrap();
    let r = query(&g2, &format!("SELECT ?t WHERE {{ ?r <{RDF_REIFIES}> ?t }}")).unwrap();
    assert_eq!(r.rows[0][0], Some(age_triple_term("alice", 30)));
}

#[test]
fn ntriples_triple_term_object_reparses_via_nt_loader() {
    // (sq-hxgb FIXED) The CONSTRUCT writer EMITS the N-Triples 1.2 triple-term
    // object syntax `<<( … )>>`; sparq-core's fast N-Triples loader (`nt::parse_chunk`) now
    // ACCEPTS it in the object position, so a serialise->reload round-trip through the
    // "ntriples" loader yields the identical structural term as the Turtle reload (which
    // already worked — see `reifier_triple_term_serialises_to_canonical_form_and_reparses_via_turtle`).
    // (Was the `#[ignore]`d `ntriples_triple_term_reparse_is_a_known_gap`.)
    let g = Graph::load_str(
        &format!("PREFIX : <http://ex/> :r1 <{RDF_REIFIES}> <<( :alice :age 30 )>> ."),
        "turtle",
    )
    .unwrap();
    let nt = construct_ntriples(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").unwrap();
    let g2 =
        Graph::load_str(&nt, "ntriples").expect("N-Triples loader should accept <<( )>> (sq-hxgb)");
    let r = query(&g2, &format!("SELECT ?t WHERE {{ ?r <{RDF_REIFIES}> ?t }}")).unwrap();
    assert_eq!(r.rows.len(), 1, "exactly the reifies triple round-trips");
    assert_eq!(r.rows[0][0], Some(age_triple_term("alice", 30)));

    // The N-Triples reload must equal the Turtle reload of the SAME serialised bytes
    // (write/read symmetry: the two loaders agree on the triple-term structure).
    let g_ttl = Graph::load_str(&nt, "turtle").unwrap();
    let r_ttl = query(&g_ttl, &format!("SELECT ?t WHERE {{ ?r <{RDF_REIFIES}> ?t }}")).unwrap();
    assert_eq!(r_ttl.rows, r.rows, "N-Triples reload must match Turtle reload");
}

#[test]
fn insert_data_reifier_annotation_creates_fresh_reifies_node() {
    // INSERT DATA with the `<< … >>` annotation form desugars to a FRESH reifier
    // blank node carrying `rdf:reifies <<( … )>>` plus the annotation triple — the
    // base triple is NOT auto-asserted by the bare annotation object form.
    let g0 = Graph::load_str("", "turtle").unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> INSERT DATA { << :alice :age 30 >> :certainty 0.9 . }",
    )
    .unwrap();
    // Exactly two triples: the reifies edge and the certainty annotation, sharing one bnode.
    assert_eq!(g.len(), 2);
    let r = query(
        &g,
        &format!("PREFIX : <http://ex/> SELECT ?r ?t ?c WHERE {{ ?r <{RDF_REIFIES}> ?t . ?r :certainty ?c }}"),
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][1], Some(age_triple_term("alice", 30)));
    assert_eq!(
        r.rows[0][2],
        Some(Term::Literal(Literal::new_typed_literal(
            "0.9",
            xsd::DECIMAL
        )))
    );
    assert!(
        matches!(r.rows[0][0], Some(Term::BlankNode(_))),
        "reifier must be a fresh blank node"
    );
}

// ---------------------------------------------------------------------------
// GAP 4 — dir-langString (base direction) flowing through UPDATE / CONSTRUCT
// ---------------------------------------------------------------------------

#[test]
fn dir_langstring_survives_insert_data_and_construct() {
    // A base-direction literal survives INSERT DATA verbatim (datatype rdf:dirLangString
    // preserved) and re-emits unchanged through CONSTRUCT.
    let g0 = Graph::load_str("", "turtle").unwrap();
    let g = update(
        &g0,
        "PREFIX : <http://ex/> INSERT DATA { :a :p \"hello\"@en--ltr . :b :p \"مرحبا\"@ar--rtl . }",
    )
    .unwrap();
    assert_eq!(
        all_rows(&g),
        vec![
            "<http://ex/a>|<http://ex/p>|\"hello\"@en--ltr".to_string(),
            "<http://ex/b>|<http://ex/p>|\"مرحبا\"@ar--rtl".to_string(),
        ]
    );
    // CONSTRUCT re-emits the dir-langString object unchanged.
    let ts = construct(
        &g,
        "PREFIX : <http://ex/> CONSTRUCT { ?s :q ?o } WHERE { ?s :p ?o }",
    )
    .unwrap();
    let mut objs: Vec<String> = ts.iter().map(|t| t.object.to_string()).collect();
    objs.sort();
    assert_eq!(
        objs,
        vec![
            "\"hello\"@en--ltr".to_string(),
            "\"مرحبا\"@ar--rtl".to_string()
        ]
    );
}

#[test]
fn dir_langstring_filter_and_order_by() {
    // FILTER on LANG / LANGDIR and ORDER BY over dir-langString literals (beyond the
    // single core `dir_literal_roundtrip` parse test). ORDER BY follows SPARQL's term
    // ordering: literal lexical value is the sort key here (all share datatype family).
    let g = Graph::load_str(
        "PREFIX : <http://ex/> \
         :a :p \"zzz\"@en--ltr . \
         :b :p \"aaa\"@ar--rtl . \
         :c :p \"mmm\"@en .",
        "turtle",
    )
    .unwrap();
    // FILTER by LANG (direction-tagged literals still report their language subtag).
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p ?o FILTER(LANG(?o) = \"en\") } ORDER BY ?s",
    )
    .unwrap();
    let langs: Vec<String> = r
        .rows
        .iter()
        .map(|row| row[0].as_ref().unwrap().to_string())
        .collect();
    assert_eq!(
        langs,
        vec!["<http://ex/a>".to_string(), "<http://ex/c>".to_string()]
    );
    // LANGDIR returns the base direction (and "" for a plain lang literal).
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?s ?d WHERE { ?s :p ?o BIND(LANGDIR(?o) AS ?d) } ORDER BY ?s",
    )
    .unwrap();
    let dirs: Vec<(String, String)> = r
        .rows
        .iter()
        .map(|row| {
            (
                row[0].as_ref().unwrap().to_string(),
                row[1].as_ref().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        dirs,
        vec![
            ("<http://ex/a>".to_string(), "\"ltr\"".to_string()),
            ("<http://ex/b>".to_string(), "\"rtl\"".to_string()),
            ("<http://ex/c>".to_string(), "\"\"".to_string()),
        ]
    );
    // ORDER BY over the literals sorts by lexical value (aaa < mmm < zzz), direction tags intact.
    let r = query(
        &g,
        "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :p ?o } ORDER BY ?o",
    )
    .unwrap();
    let objs: Vec<String> = r
        .rows
        .iter()
        .map(|row| row[0].as_ref().unwrap().to_string())
        .collect();
    assert_eq!(
        objs,
        vec![
            "\"aaa\"@ar--rtl".to_string(),
            "\"mmm\"@en".to_string(),
            "\"zzz\"@en--ltr".to_string(),
        ]
    );
}
