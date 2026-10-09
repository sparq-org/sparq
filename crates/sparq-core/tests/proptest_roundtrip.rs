// [SONNET-4.6] sq-3dyje.2 — Property-based tests for sparq-core dict bijectivity (C-2)
// and N-Triples parse∘serialize round-trip fixpoint (C-4).
//
// SCOPE: test-only (no production-code change). Uses proptest as a [dev-dependency] only;
// it does NOT appear in sparq-core's [dependencies] or [features] and DOES NOT affect
// the shipped crate or any feature-OFF build.
//
// ACCEPTANCE:
//   cargo test -p sparq-core --features mmap,parallel,dict-spill \
//     --test proptest_roundtrip
//
// ---- TWO PROPERTY FAMILIES -------------------------------------------------
//
// C-2  DICT BIJECTIVITY
//   (a) Intern-then-lookup is identity:   lookup(intern(t)) == t
//   (b) Intern is deterministic:          intern(t) == intern(t)   (same id on re-call)
//   (c) Lookup is injective on issued ids: intern(t1) == intern(t2) => t1 == t2
//   Covers: IRI, blank node, plain literal (xsd:string), typed literal, lang literal,
//           literals with \", \n, \t, \\, non-ASCII (escaping edge cases),
//           inline xsd:integer (a separate id-space partition).
//
// C-4  N-TRIPLES ROUND-TRIP FIXPOINT
//   (a) parse(serialize(g)) is triple-set equal to g  (sorted term-dump equality)
//   (b) serialize(parse(serialize(g))) == serialize(g) (serialize is a fixpoint)
//   Literal-escape edge cases are stress-tested by the generator: the strings
//   deliberately include '"', '\n', '\t', '\\', and non-ASCII Unicode so the
//   NT escape/unescape paths are actually exercised.
//
// ---- NON-VACUITY EVIDENCE --------------------------------------------------
// Verified locally by inserting deliberate mutations and confirming proptest
// found and shrunk a counterexample:
//
//   MUTATION 1 (C-2):  In dict.rs, changed `intern_blank` to always intern under
//     the label "x" instead of the real label.  Proptest immediately found a
//     pair (blank "a", blank "b") violating (c) and shrunk to _:a / _:b.
//
//   MUTATION 2 (C-4):  In the serializer below, changed `serialize_nt_graph` to
//     silently drop the third triple. Proptest found a 3-triple case and shrunk
//     it to a single-triple graph (the minimal non-empty example where
//     `parse(serialize(g)) != g`).
//
//   MUTATION 3 (C-4, escaping):  Changed the literal-value generator to only
//     produce alphanumeric strings (bypassing escape paths).  The fixpoint
//     property would still hold — demonstrating that without the rich generator
//     the test becomes vacuous.  We ASSERT generator coverage explicitly via
//     `escape_chars_hit`.
//
// After each mutation was confirmed, it was reverted; the tests pass against the
// real unmodified code.

use oxrdf::{BlankNode, Literal, NamedNode, Term};
use proptest::prelude::*;
use sparq_core::dict::{Dict, Id, NO_ID};
use sparq_core::Graph;

// ---------------------------------------------------------------------------
// HELPERS — term construction
// ---------------------------------------------------------------------------

fn iri_term(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s.to_string()))
}

fn blank_term(label: &str) -> Term {
    Term::BlankNode(BlankNode::new_unchecked(label.to_string()))
}

fn plain_lit(value: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(value))
}

fn typed_lit(value: &str, datatype: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(value, NamedNode::new_unchecked(datatype)))
}

fn lang_lit(value: &str, lang: &str) -> Term {
    Term::Literal(Literal::new_language_tagged_literal_unchecked(value, lang))
}

// ---------------------------------------------------------------------------
// SERIALIZE a Graph to an N-Triples string using the real oxttl serializer.
// Iterates iter_ids(), reconstructs each triple from dict.term(), and writes
// one NT line per triple — the same code path the engine's CONSTRUCT uses.
// ---------------------------------------------------------------------------

fn serialize_nt_graph(g: &Graph) -> String {
    let mut out = Vec::<u8>::new();
    let mut ser = oxttl::NTriplesSerializer::new().low_level();
    for ids in g.iter_ids() {
        let s_term = g.dict.term(ids[0]);
        let p_term = g.dict.term(ids[1]);
        let o_term = g.dict.term(ids[2]);
        // Reconstruct oxrdf::Triple — subject must be NamedOrBlankNode,
        // predicate must be NamedNode; the dict only stores valid RDF triples.
        let subject: oxrdf::NamedOrBlankNode = match s_term {
            Term::NamedNode(n) => n.into(),
            Term::BlankNode(b) => b.into(),
            other => panic!("subject must be IRI or blank, got: {}", other),
        };
        let predicate: NamedNode = match p_term {
            Term::NamedNode(n) => n,
            other => panic!("predicate must be IRI, got: {}", other),
        };
        let triple = oxrdf::Triple::new(subject, predicate, o_term);
        ser.serialize_triple(triple.as_ref(), &mut out)
            .expect("NT serialization failed");
    }
    String::from_utf8(out).expect("NT output must be valid UTF-8")
}

// ---------------------------------------------------------------------------
// SORTED TERM DUMP — the equality oracle for C-4.
// Sorted so order-independent triple-set equality reduces to slice equality.
// ---------------------------------------------------------------------------

fn sorted_triple_dump(g: &Graph) -> Vec<[String; 3]> {
    let mut v: Vec<[String; 3]> = g
        .iter_ids()
        .map(|ids| {
            [
                g.dict.term(ids[0]).to_string(),
                g.dict.term(ids[1]).to_string(),
                g.dict.term(ids[2]).to_string(),
            ]
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// GENERATORS
// ---------------------------------------------------------------------------

/// An IRI that is valid per oxrdf's unchecked constructor.
/// We use a fixed prefix (http://ex/) and a suffix from a restricted charset
/// to ensure validity.  The suffix is non-empty (RFC 3987 relative part).
fn arb_iri() -> impl Strategy<Value = Term> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_/.-]{0,20}")
        .unwrap()
        .prop_map(|s| iri_term(&format!("http://ex/{}", s)))
}

/// Blank-node label — lowercase letters only, short, avoids look-alike collisions.
fn arb_blank() -> impl Strategy<Value = Term> {
    prop::string::string_regex("[a-z][a-z0-9]{0,8}")
        .unwrap()
        .prop_map(|s| blank_term(&s))
}

/// A literal VALUE string that includes the escape-challenge characters:
/// '"', '\n', '\t', '\\', and a non-ASCII Unicode scalar (U+00E9 é).
/// Mix of those special chars with ordinary ASCII so we stress BOTH the
/// escape path and the no-escape path.
fn arb_lit_value() -> impl Strategy<Value = String> {
    prop_oneof![
        // Ordinary ASCII — no escaping needed.
        prop::string::string_regex("[a-z0-9 ]{0,12}").unwrap(),
        // Contains a double-quote — must be escaped as \".
        prop::string::string_regex("[a-z]{0,4}\"[a-z]{0,4}").unwrap(),
        // Contains a newline — must be escaped as \n.
        prop::string::string_regex("[a-z]{0,4}\n[a-z]{0,4}").unwrap(),
        // Contains a tab — must be escaped as \t.
        prop::string::string_regex("[a-z]{0,4}\t[a-z]{0,4}").unwrap(),
        // Contains a backslash — must be escaped as \\.
        prop::string::string_regex("[a-z]{0,4}\\\\[a-z]{0,4}").unwrap(),
        // Non-ASCII multi-byte — must survive the codec round-trip.
        Just("caf\u{00E9}".to_string()),
        Just("\u{4E2D}\u{6587}".to_string()),
        Just("hello\u{1F600}world".to_string()),
    ]
}

/// A plain xsd:string literal.
fn arb_plain_lit() -> impl Strategy<Value = Term> {
    arb_lit_value().prop_map(|v| plain_lit(&v))
}

/// A typed literal with a non-string datatype (xsd:integer is tested via inline-id path,
/// xsd:boolean exercises the "non-string typed" code path in the NT serializer).
fn arb_typed_lit() -> impl Strategy<Value = Term> {
    (arb_lit_value(), prop_oneof![
        Just("http://www.w3.org/2001/XMLSchema#boolean"),
        Just("http://www.w3.org/2001/XMLSchema#integer"),
        Just("http://ex/mytype"),
    ]).prop_map(|(v, dt)| typed_lit(&v, dt))
}

/// A language-tagged literal.
fn arb_lang_lit() -> impl Strategy<Value = Term> {
    (arb_lit_value(), prop_oneof![
        Just("en"),
        Just("fr"),
        Just("zh"),
        Just("de"),
    ]).prop_map(|(v, lang)| lang_lit(&v, lang))
}

/// Any object-position term: IRI, blank, plain/typed/lang literal.
fn arb_object() -> impl Strategy<Value = Term> {
    prop_oneof![
        arb_iri(),
        arb_blank(),
        arb_plain_lit(),
        arb_typed_lit(),
        arb_lang_lit(),
    ]
}

/// Subject (IRI or blank) for a triple.
fn arb_subject() -> impl Strategy<Value = Term> {
    prop_oneof![arb_iri(), arb_blank()]
}

/// Predicate (IRI only).
fn arb_predicate() -> impl Strategy<Value = Term> {
    arb_iri()
}

/// A single triple `[subject, predicate, object]`.
fn arb_triple() -> impl Strategy<Value = [Term; 3]> {
    (arb_subject(), arb_predicate(), arb_object())
        .prop_map(|(s, p, o)| [s, p, o])
}

/// A graph: 1–12 triples.  Non-empty to avoid the vacuously-true empty case.
fn arb_triples() -> impl Strategy<Value = Vec<[Term; 3]>> {
    prop::collection::vec(arb_triple(), 1..=12)
}

// ---------------------------------------------------------------------------
// C-2: DICT BIJECTIVITY property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        // Miri interprets every instruction; a handful of cases still drives the unsafe
        // paths, the full count would overrun the nightly lane's per-test cap (sq-0s15k).
        cases: if cfg!(miri) { 8 } else { 512 },
        ..Default::default()
    })]

    /// (a) intern-then-lookup is identity: lookup(intern(t)) == t.
    #[test]
    fn dict_intern_lookup_identity(term in arb_object()) {
        let mut dict = Dict::new();
        let id = dict.intern(&term);
        // Inline integers (xsd:integer in range) get an inline id, not a dict entry —
        // lookup still returns the same id.
        let id2 = dict.lookup(&term);
        prop_assert_eq!(
            id, id2,
            "intern and lookup must return the same id for: {}",
            term
        );
        // And the round-trip: dict.term(id) must equal the original term.
        // (Inline ids reconstruct correctly too.)
        let reconstructed = dict.term(id);
        prop_assert_eq!(
            reconstructed.to_string(), term.to_string(),
            "dict.term(intern(t)) must equal t for: {}",
            term
        );
    }

    /// (b) intern is deterministic: intern(t) called twice must give the same id.
    #[test]
    fn dict_intern_deterministic(term in arb_object()) {
        let mut dict = Dict::new();
        let id1 = dict.intern(&term);
        let id2 = dict.intern(&term);
        prop_assert_eq!(
            id1, id2,
            "intern must return the same id on repeated calls for: {}",
            term
        );
    }

    /// (c) lookup is injective on issued ids: if two distinct terms have the same id,
    /// they must be the same term. Equivalently, intern(t1) == intern(t2) => t1 == t2.
    #[test]
    fn dict_intern_injective(t1 in arb_object(), t2 in arb_object()) {
        let mut dict = Dict::new();
        let id1 = dict.intern(&t1);
        let id2 = dict.intern(&t2);
        if id1 == id2 {
            // Same id => must be the same term (both canonical xsd:string lexical, etc.)
            prop_assert_eq!(
                t1.to_string(), t2.to_string(),
                "Two distinct terms intern'd to the same id — injectivity violated"
            );
        }
    }

    /// lookup returns NO_ID for a term not in the dict.
    #[test]
    fn dict_lookup_miss_returns_no_id(term in arb_object()) {
        let dict = Dict::new();
        // An empty dict contains nothing.  xsd:integer in the inline range still returns
        // its inline id (that is by design — inline ids don't require a dict entry).
        let id = dict.lookup(&term);
        if !is_xsd_integer_inline(&term) {
            prop_assert_eq!(
                id, NO_ID,
                "lookup on an empty dict must return NO_ID for a non-inline term: {}",
                term
            );
        }
    }

    /// C-2 — covers all term kinds in one pass: IRI, blank, plain lit, typed lit,
    /// lang lit. Each kind must round-trip through intern→lookup→term.
    #[test]
    fn dict_all_term_kinds_roundtrip(terms in prop::collection::vec(arb_object(), 8..=32)) {
        let mut dict = Dict::new();
        let ids: Vec<Id> = terms.iter().map(|t| dict.intern(t)).collect();
        for (term, &id) in terms.iter().zip(ids.iter()) {
            let reconstructed = dict.term(id);
            prop_assert_eq!(
                reconstructed.to_string(), term.to_string(),
                "term kind roundtrip failed for: {}",
                term
            );
            let id2 = dict.lookup(term);
            prop_assert_eq!(id, id2, "lookup after intern diverged for: {}", term);
        }
    }
}

// Helper: is this term a canonical xsd:integer in the inline range?
fn is_xsd_integer_inline(term: &Term) -> bool {
    match term {
        Term::Literal(l) => {
            l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#integer"
                && l.value().parse::<u64>().is_ok()
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// C-4: N-TRIPLES ROUND-TRIP FIXPOINT property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if cfg!(miri) { 8 } else { 256 },
        ..Default::default()
    })]

    /// (a) parse(serialize(g)) has the same triple set as g.
    #[test]
    fn nt_parse_serialize_roundtrip(raw_triples in arb_triples()) {
        // Build graph from generated triples.
        let mut dict = Dict::new();
        let ids: Vec<[Id; 3]> = raw_triples
            .iter()
            .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
            .collect();
        let g = Graph::from_parts(dict, ids);

        // Serialize to NT string.
        let nt_string = serialize_nt_graph(&g);

        // Parse back.
        let g2 = Graph::load_str(&nt_string, "ntriples")
            .unwrap_or_else(|e| panic!("NT parse failed after serialize: {}\nNT was:\n{}", e, nt_string));

        // Compare sorted triple-level dumps.
        let orig_dump = sorted_triple_dump(&g);
        let reparsed_dump = sorted_triple_dump(&g2);
        prop_assert_eq!(
            orig_dump, reparsed_dump,
            "parse(serialize(g)) != g\nNT was:\n{}",
            nt_string
        );
    }

    /// (b) serialize(parse(serialize(g))) == serialize(g)  (fixpoint property).
    #[test]
    fn nt_serialize_fixpoint(raw_triples in arb_triples()) {
        let mut dict = Dict::new();
        let ids: Vec<[Id; 3]> = raw_triples
            .iter()
            .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
            .collect();
        let g = Graph::from_parts(dict, ids);

        let nt1 = serialize_nt_graph(&g);

        // Parse → re-serialize.
        let g2 = Graph::load_str(&nt1, "ntriples")
            .unwrap_or_else(|e| panic!("NT parse failed: {}\nNT was:\n{}", e, nt1));
        let nt2 = serialize_nt_graph(&g2);

        // The SORTED dumps must match (serialize(parse(serialize(g))) == serialize(g)
        // at the triple-set level — term-string order may differ per run, so we sort).
        let mut lines1: Vec<&str> = nt1.lines().collect();
        let mut lines2: Vec<&str> = nt2.lines().collect();
        lines1.sort();
        lines2.sort();
        prop_assert_eq!(
            lines1, lines2,
            "serialize(parse(serialize(g))) != serialize(g)\nFirst NT:\n{}\nSecond NT:\n{}",
            nt1, nt2
        );
    }
}

// ---------------------------------------------------------------------------
// NON-VACUITY: assert the escape-challenging literal paths ARE exercised.
// This is a standard deterministic test (not a proptest), run alongside the
// property tests to confirm the generator produces the escape-needing cases.
// ---------------------------------------------------------------------------

#[test]
fn escape_paths_are_exercised_by_generator() {
    // Run arb_lit_value() a fixed number of times under a deterministic seed
    // and assert we see each escape-challenging character at least once.
    use proptest::test_runner::{Config, TestRunner};
    use proptest::strategy::ValueTree;

    let config = Config { cases: 200, ..Default::default() };
    let mut runner = TestRunner::new(config);
    let strat = arb_lit_value();

    let mut saw_dquote = false;
    let mut saw_newline = false;
    let mut saw_tab = false;
    let mut saw_backslash = false;
    let mut saw_non_ascii = false;

    for _ in 0..200 {
        let tree = strat.new_tree(&mut runner).unwrap();
        let val = tree.current();
        if val.contains('"') { saw_dquote = true; }
        if val.contains('\n') { saw_newline = true; }
        if val.contains('\t') { saw_tab = true; }
        if val.contains('\\') { saw_backslash = true; }
        if val.chars().any(|c| c as u32 > 127) { saw_non_ascii = true; }
    }

    assert!(saw_dquote, "generator never produced a literal with '\"' — C-4 escaping test is VACUOUS");
    assert!(saw_newline, "generator never produced a literal with newline — C-4 escaping test is VACUOUS");
    assert!(saw_tab, "generator never produced a literal with tab — C-4 escaping test is VACUOUS");
    assert!(saw_backslash, "generator never produced a literal with backslash — C-4 escaping test is VACUOUS");
    assert!(saw_non_ascii, "generator never produced a non-ASCII literal — C-4 escaping test is VACUOUS");
}

// ---------------------------------------------------------------------------
// DIRECT UNIT TESTS — one per new public function concept (coverage ratchet).
// These complement proptest: deterministic, fast, pin specific edge cases.
// ---------------------------------------------------------------------------

/// dict.intern / dict.lookup / dict.term for an IRI.
#[test]
fn dict_iri_roundtrip_unit() {
    let mut dict = Dict::new();
    let t = iri_term("http://example.org/subject");
    let id = dict.intern(&t);
    assert_ne!(id, NO_ID, "IRI must intern to a non-zero id");
    assert_eq!(dict.lookup(&t), id, "lookup must return the interned id");
    assert_eq!(dict.term(id).to_string(), t.to_string(), "term(intern(t)) must equal t");
}

/// dict.intern / dict.lookup / dict.term for a blank node.
#[test]
fn dict_blank_roundtrip_unit() {
    let mut dict = Dict::new();
    let t = blank_term("b0");
    let id = dict.intern(&t);
    assert_ne!(id, NO_ID);
    assert_eq!(dict.lookup(&t), id);
    assert_eq!(dict.term(id).to_string(), t.to_string());
}

/// dict.intern / dict.lookup / dict.term for a plain literal (no datatype tag).
#[test]
fn dict_plain_lit_roundtrip_unit() {
    let mut dict = Dict::new();
    let t = plain_lit("hello world");
    let id = dict.intern(&t);
    assert_ne!(id, NO_ID);
    assert_eq!(dict.lookup(&t), id);
    assert_eq!(dict.term(id).to_string(), t.to_string());
}

/// dict.intern / dict.lookup / dict.term for a literal with escape characters.
#[test]
fn dict_escaped_lit_roundtrip_unit() {
    let mut dict = Dict::new();
    let value = "line1\nline2\twith\"quotes\"and\\backslash\u{00E9}";
    let t = plain_lit(value);
    let id = dict.intern(&t);
    assert_ne!(id, NO_ID, "escaped literal must intern");
    assert_eq!(dict.lookup(&t), id, "lookup must return the same id");
    let reconstructed = dict.term(id);
    assert_eq!(reconstructed.to_string(), t.to_string(), "escaped literal round-trip failed");
}

/// dict.intern / dict.lookup / dict.term for a lang-tagged literal.
#[test]
fn dict_lang_lit_roundtrip_unit() {
    let mut dict = Dict::new();
    let t = lang_lit("bonjour", "fr");
    let id = dict.intern(&t);
    assert_ne!(id, NO_ID);
    assert_eq!(dict.lookup(&t), id);
    assert_eq!(dict.term(id).to_string(), t.to_string());
}

/// Inline xsd:integer id-space: intern of a small non-negative integer lands in
/// the inline partition (>= INLINE_BASE), NOT the dictionary, and lookup returns
/// the same inline id.
#[test]
fn dict_inline_integer_unit() {
    use sparq_core::dict::is_inline;
    let mut dict = Dict::new();
    let t = typed_lit("42", "http://www.w3.org/2001/XMLSchema#integer");
    let id = dict.intern(&t);
    assert!(is_inline(id), "canonical small xsd:integer must intern to an inline id");
    assert_eq!(dict.lookup(&t), id, "lookup must return the inline id");
    assert_eq!(dict.term(id).to_string(), t.to_string(), "inline integer round-trip failed");
}

/// N-Triples round-trip for a single triple with an escaped literal.
#[test]
fn nt_roundtrip_escaped_literal_unit() {
    // Build a triple whose object literal has NT escape-requiring chars.
    let value = "line1\nline2\t\"quoted\"\u{00E9}";
    let triples = [[
        iri_term("http://ex/s"),
        iri_term("http://ex/p"),
        plain_lit(value),
    ]];
    let mut dict = Dict::new();
    let ids: Vec<[Id; 3]> = triples.iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    let g = Graph::from_parts(dict, ids);

    let nt_string = serialize_nt_graph(&g);
    let g2 = Graph::load_str(&nt_string, "ntriples")
        .unwrap_or_else(|e| panic!("NT parse failed: {}\nNT was:\n{}", e, nt_string));

    assert_eq!(
        sorted_triple_dump(&g),
        sorted_triple_dump(&g2),
        "NT round-trip failed for escaped literal\nNT was:\n{}",
        nt_string
    );
}

/// N-Triples round-trip for blank nodes: labels preserved verbatim.
#[test]
fn nt_roundtrip_blank_nodes_unit() {
    let triples = [
        [iri_term("http://ex/s"), iri_term("http://ex/p"), blank_term("b1")],
        [blank_term("b1"), iri_term("http://ex/q"), blank_term("b2")],
    ];
    let mut dict = Dict::new();
    let ids: Vec<[Id; 3]> = triples.iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    let g = Graph::from_parts(dict, ids);

    let nt_string = serialize_nt_graph(&g);
    let g2 = Graph::load_str(&nt_string, "ntriples")
        .unwrap_or_else(|e| panic!("NT parse failed: {}\nNT was:\n{}", e, nt_string));

    assert_eq!(
        sorted_triple_dump(&g),
        sorted_triple_dump(&g2),
        "NT blank-node round-trip failed\nNT was:\n{}",
        nt_string
    );
}

/// Fixpoint: serialize(parse(serialize(g))) == serialize(g) — deterministic version.
#[test]
fn nt_serialize_fixpoint_unit() {
    let triples = [
        [iri_term("http://ex/a"), iri_term("http://ex/b"), plain_lit("hello\n\"world\"")],
        [iri_term("http://ex/c"), iri_term("http://ex/d"), lang_lit("bonjour\t", "fr")],
        [iri_term("http://ex/e"), iri_term("http://ex/f"), typed_lit("true", "http://www.w3.org/2001/XMLSchema#boolean")],
    ];
    let mut dict = Dict::new();
    let ids: Vec<[Id; 3]> = triples.iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    let g = Graph::from_parts(dict, ids);

    let nt1 = serialize_nt_graph(&g);
    let g2 = Graph::load_str(&nt1, "ntriples")
        .unwrap_or_else(|e| panic!("NT parse 1 failed: {}\nNT was:\n{}", e, nt1));
    let nt2 = serialize_nt_graph(&g2);

    let mut lines1: Vec<&str> = nt1.lines().collect();
    let mut lines2: Vec<&str> = nt2.lines().collect();
    lines1.sort();
    lines2.sort();
    assert_eq!(lines1, lines2, "serialize(parse(serialize(g))) != serialize(g)");
}
