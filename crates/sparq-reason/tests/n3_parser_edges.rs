//! N3 / Turtle parser edge + error paths — the lowest-covered module in sparq-reason.
//!
//! 🤖 SPARQ agent — sq-qcnn test-quality slice.
//!
//! Drives the public parser API (`parse`, `parse_with_base`, `parse_turtle_with_base`) over the
//! genuinely-dark branches: quantifiers, `@keywords`, the `is…of` / `has` / `<-` predicate
//! sugars, IRI + pname + string + number lexing edge cases, triple-quoted strings, language-tag
//! validation, relative-IRI (RFC 3986) resolution, collections / bnode property lists, and the
//! STRICT-Turtle rejections of N3-only constructs. Each success path asserts the EXACT parsed
//! Term shape; each error path asserts `is_err()` (the parser must REPORT, never panic).

use sparq_reason::n3::parser::{parse, parse_turtle_with_base, parse_with_base};
use sparq_reason::n3::Term;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn iri(s: &str) -> Term {
    Term::Iri(s.into())
}

// ---------- @forAll / @forSome quantifiers ----------

#[test]
fn for_all_quantifier_makes_a_universal_variable() {
    // @forAll <x> declares x as a UNIVERSAL — it becomes a Var in the facts.
    let p = parse("@forAll <http://ex/x> . <http://ex/x> <http://ex/p> <http://ex/o> .")
        .expect("forAll");
    assert!(
        matches!(&p.facts[0][0], Term::Var(_)),
        "the @forAll-quantified subject is a variable; got {:?}",
        p.facts[0][0]
    );
}

#[test]
fn for_some_quantifier_makes_an_existential_blank() {
    // @forSome <y> declares y as an EXISTENTIAL — a blank node in the facts.
    let p = parse("@forSome <http://ex/y> . <http://ex/y> <http://ex/p> <http://ex/o> .")
        .expect("forSome");
    assert!(
        matches!(&p.facts[0][0], Term::Blank(_)),
        "the @forSome-quantified subject is a blank; got {:?}",
        p.facts[0][0]
    );
}

#[test]
fn strict_turtle_rejects_quantifiers() {
    assert!(
        parse_turtle_with_base(
            "@forAll <http://ex/x> . <http://ex/x> <http://ex/p> <http://ex/o> .",
            ""
        )
        .is_err(),
        "quantifiers are not Turtle"
    );
}

// ---------- @prefix / @base / PREFIX / BASE dot discipline ----------

#[test]
fn sparql_style_prefix_and_base_without_dot() {
    // SPARQL-style PREFIX / BASE (no trailing dot) are accepted in N3.
    let p = parse("PREFIX ex: <http://ex/>\nBASE <http://base/>\n<rel> ex:p ex:o .")
        .expect("SPARQL-style directives");
    assert_eq!(
        p.facts[0][0],
        iri("http://base/rel"),
        "BASE resolves the relative subject"
    );
    assert_eq!(
        p.facts[0][1],
        iri("http://ex/p"),
        "PREFIX expands the predicate"
    );
}

#[test]
fn strict_turtle_rejects_at_prefix_without_final_dot() {
    // In strict Turtle, `@prefix` MUST end with a dot.
    assert!(
        parse_turtle_with_base("@prefix ex: <http://ex/> <http://ex/s> ex:p ex:o .", "").is_err(),
        "@prefix needs a final dot in Turtle"
    );
}

// ---------- is…of / has / <- predicate sugar ----------

#[test]
fn is_of_inverse_swaps_subject_and_object() {
    // `B is P of A` asserts (A P B).
    let p = parse("<http://ex/b> is <http://ex/p> of <http://ex/a> .").expect("is..of");
    assert_eq!(
        p.facts[0],
        [iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")],
        "`B is P of A` ⊢ (A P B)"
    );
}

#[test]
fn is_predicate_without_of_is_an_error() {
    // `is P` must be followed by `of`.
    assert!(
        parse("<http://ex/b> is <http://ex/p> <http://ex/a> .").is_err(),
        "`is P` without `of` is a parse error"
    );
}

#[test]
fn has_keyword_is_explicit_forward_predicate() {
    // `S has P O` is plain (S P O).
    let p = parse("<http://ex/s> has <http://ex/p> <http://ex/o> .").expect("has");
    assert_eq!(
        p.facts[0],
        [iri("http://ex/s"), iri("http://ex/p"), iri("http://ex/o")],
        "`S has P O` ⊢ (S P O)"
    );
}

#[test]
fn strict_turtle_rejects_is_of_and_has() {
    assert!(
        parse_turtle_with_base("<http://ex/b> is <http://ex/p> of <http://ex/a> .", "").is_err(),
        "is..of is not Turtle"
    );
    assert!(
        parse_turtle_with_base("<http://ex/s> has <http://ex/p> <http://ex/o> .", "").is_err(),
        "has is not Turtle"
    );
}

// ---------- IRI + pname lexing edge cases ----------

#[test]
fn iri_unicode_escape_is_decoded() {
    // A inside an IRIREF decodes to 'A'.
    let p = parse("<http://ex/\\u0041> <http://ex/p> <http://ex/o> .").expect("iri escape");
    assert_eq!(p.facts[0][0], iri("http://ex/A"), "\\u0041 -> A in the IRI");
}

#[test]
fn iri_with_bad_unicode_escape_is_an_error() {
    assert!(
        parse("<http://ex/\\uXYZW> <http://ex/p> <http://ex/o> .").is_err(),
        "non-hex \\u escape in an IRI is an error"
    );
}

#[test]
fn unterminated_iri_is_an_error() {
    assert!(
        parse("<http://ex/unclosed").is_err(),
        "an unterminated IRI is reported"
    );
}

#[test]
fn prefixed_name_with_escaped_local_punctuation() {
    // PN_LOCAL_ESC: a backslash-escaped reserved punctuation char (here `!`) is part of the
    // local name literally. (`:` is NOT in the escape set — it is a literal pname char.)
    let p = parse("@prefix ex: <http://ex/> . ex:a\\!b <http://ex/p> <http://ex/o> .")
        .expect("escaped pname");
    assert_eq!(
        p.facts[0][0],
        iri("http://ex/a!b"),
        "escaped '!' joins the local name"
    );
    // An ILLEGAL escape (a letter is not a reserved punctuation char) is a parse error.
    assert!(
        parse("@prefix ex: <http://ex/> . ex:a\\zb <http://ex/p> <http://ex/o> .").is_err(),
        "an illegal pname escape is reported"
    );
}

#[test]
fn strict_turtle_rejects_local_name_starting_with_hyphen() {
    assert!(
        parse_turtle_with_base(
            "@prefix ex: <http://ex/> . ex:-x <http://ex/p> <http://ex/o> .",
            ""
        )
        .is_err(),
        "a Turtle local name cannot start with '-'"
    );
}

// ---------- string-literal lexing ----------

#[test]
fn triple_quoted_string_preserves_newlines() {
    let p = parse("<http://ex/s> <http://ex/p> \"\"\"line1\nline2\"\"\" .").expect("triple-quoted");
    assert_eq!(
        p.facts[0][2],
        Term::Lit("line1\nline2".into(), format!("{}string", XSD), None),
        "triple-quoted literal keeps the embedded newline"
    );
}

#[test]
fn string_escapes_are_decoded() {
    let p = parse("<http://ex/s> <http://ex/p> \"a\\tb\\nc\" .").expect("escapes");
    assert_eq!(
        p.facts[0][2],
        Term::Lit("a\tb\nc".into(), format!("{}string", XSD), None),
        "\\t and \\n decode to tab + newline"
    );
}

#[test]
fn unterminated_string_literal_is_an_error() {
    assert!(
        parse("<http://ex/s> <http://ex/p> \"unterminated .").is_err(),
        "unterminated literal"
    );
}

// ---------- language tags ----------

#[test]
fn language_tag_is_lowercased_and_carries_lang_string() {
    let rdf_lang = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
    let p = parse("<http://ex/s> <http://ex/p> \"txt\"@EN-US .").expect("lang tag");
    assert_eq!(
        p.facts[0][2],
        Term::Lit("txt".into(), rdf_lang.into(), Some("en-us".into())),
        "language tag is lowercased and typed rdf:langString"
    );
}

#[test]
fn digit_leading_language_tag_is_an_error() {
    assert!(
        parse("<http://ex/s> <http://ex/p> \"txt\"@1bad .").is_err(),
        "a language tag must start with a letter"
    );
}

// ---------- number lexing ----------

#[test]
fn signed_integer_decimal_and_double_literals() {
    let p = parse("<http://ex/s> <http://ex/p> -42, +3.5, 1.0e10 .").expect("numbers");
    let objs: Vec<&Term> = p.facts.iter().map(|t| &t[2]).collect();
    assert!(
        objs.contains(&&Term::Lit("-42".into(), format!("{}integer", XSD), None)),
        "signed int"
    );
    assert!(
        objs.contains(&&Term::Lit("+3.5".into(), format!("{}decimal", XSD), None)),
        "signed dec"
    );
    assert!(
        objs.contains(&&Term::Lit("1.0e10".into(), format!("{}double", XSD), None)),
        "double"
    );
}

// ---------- collections + bnode property lists ----------

#[test]
fn empty_collection_is_the_empty_list_term() {
    let p = parse("<http://ex/s> <http://ex/p> ( ) .").expect("empty collection");
    assert_eq!(
        p.facts[0][2],
        Term::List(vec![]),
        "() is the empty List term"
    );
}

#[test]
fn nonempty_collection_is_a_list_term() {
    let p =
        parse("<http://ex/s> <http://ex/p> ( <http://ex/a> <http://ex/b> ) .").expect("collection");
    assert_eq!(
        p.facts[0][2],
        Term::List(vec![iri("http://ex/a"), iri("http://ex/b")]),
        "( a b ) is a first-class List term"
    );
}

#[test]
fn unterminated_collection_is_an_error() {
    assert!(
        parse("<http://ex/s> <http://ex/p> ( <http://ex/a>").is_err(),
        "an unterminated collection is reported"
    );
}

#[test]
fn bnode_property_list_creates_a_blank_subject() {
    // `[ :q :o ]` as object: emits (s p _:b) plus (_:b q o).
    let p = parse("<http://ex/s> <http://ex/p> [ <http://ex/q> <http://ex/o> ] .")
        .expect("bnode propertylist");
    assert_eq!(
        p.facts.len(),
        2,
        "the outer triple + the bnode's own triple"
    );
    // One fact has a Blank object (the bnode), one has the Blank as subject of (q o).
    assert!(
        p.facts
            .iter()
            .any(|t| matches!(t[2], Term::Blank(_)) && t[1] == iri("http://ex/p")),
        "outer (s p _:b) with a blank object"
    );
    assert!(
        p.facts
            .iter()
            .any(|t| matches!(t[0], Term::Blank(_)) && t[1] == iri("http://ex/q")),
        "inner (_:b q o)"
    );
}

// ---------- relative-IRI (RFC 3986) resolution ----------

#[test]
fn relative_iri_resolution_handles_fragment_dotdot_and_dot() {
    // Resolve `#frag`, `../x`, and `./x` against a base with a path.
    let p = parse_with_base(
        "<#frag> <http://ex/p> <http://ex/o> .",
        "http://base/dir/file",
    )
    .expect("fragment");
    assert_eq!(
        p.facts[0][0],
        iri("http://base/dir/file#frag"),
        "fragment appends to base"
    );

    let p2 = parse_with_base(
        "<../x> <http://ex/p> <http://ex/o> .",
        "http://base/dir1/file",
    )
    .expect("dotdot");
    assert_eq!(
        p2.facts[0][0],
        iri("http://base/x"),
        "../ pops one path segment"
    );

    let p3 = parse_with_base(
        "<./x> <http://ex/p> <http://ex/o> .",
        "http://base/dir/file",
    )
    .expect("dot");
    assert_eq!(
        p3.facts[0][0],
        iri("http://base/dir/x"),
        "./ stays in the current directory"
    );
}

// ---------- rules + sameAs sugar ----------

#[test]
fn forward_backward_rules_and_sameas_sugar() {
    let p = parse(
        "{ ?x <http://ex/p> ?y } => { ?x <http://ex/q> ?y } .\n\
         { <http://ex/a> <http://ex/r> <http://ex/b> } <= true .\n\
         <http://ex/a> = <http://ex/b> .",
    )
    .expect("rules + sameAs");
    assert_eq!(p.rules.len(), 1, "one forward rule");
    assert_eq!(p.backward_rules.len(), 1, "one backward rule");
    assert!(
        p.backward_rules[0].premise.is_empty(),
        "`<= true` is an empty premise"
    );
    let owl_same = "http://www.w3.org/2002/07/owl#sameAs";
    assert!(
        p.facts.iter().any(|t| t[1] == iri(owl_same)),
        "`=` desugars to owl:sameAs"
    );
}

// ---------- generic malformed input ----------

#[test]
fn assorted_malformed_documents_report_errors() {
    for bad in [
        "<http://ex/s> <http://ex/p>", // missing object + terminator
        "{ ?x <http://ex/p> ?y => .",  // unbalanced formula brace
        "@prefix ex <http://ex/> . ex:s ex:p ex:o .", // missing ':' after prefix name
        "<http://ex/s> <http://ex/p> \"v\"^^ .", // missing datatype after ^^
    ] {
        assert!(parse(bad).is_err(), "expected a parse error for: {:?}", bad);
    }
}

#[test]
fn type_keyword_a_expands_to_rdf_type() {
    let p = parse("<http://ex/s> a <http://ex/T> .").expect("a keyword");
    assert_eq!(p.facts[0][1], iri(RDF_TYPE), "`a` is rdf:type");
}

// ---------- multibyte pname bytes (sq-t8z0r fuzz regression, GH #1903) ----------
//
// 🤖 SPARQ agent. The randomized fuzz lane (parse_rif_n3) found the
// byte-wise lexer splitting a UTF-8 character in half: continuation bytes 0x85 /
// 0xA0 read as U+0085 NEL / U+00A0 NBSP through `(byte as char).is_whitespace()`,
// stopping a pname scan MID-character, and the subsequent
// `str::from_utf8(..).unwrap()` panicked (parser.rs read_pname_prefix). In valid
// UTF-8 those byte values only ever occur as continuation bytes, so the fix makes
// byte-level whitespace strictly ASCII. These inputs must parse (or Err) — never panic.

#[test]
fn pname_prefix_with_multibyte_char_containing_0xa0_continuation_byte_does_not_panic() {
    // U+0460 'Ѡ' encodes as D1 A0 — the A0 continuation byte used to read as NBSP
    // whitespace and split the prefix scan mid-character (the fuzz crash).
    let p = parse("@prefix p\u{0460}x: <http://example.org/> . p\u{0460}x:s p\u{0460}x:p p\u{0460}x:o .")
        .expect("a pname prefix containing U+0460 is legal PN_CHARS");
    assert_eq!(p.facts.len(), 1);
    assert_eq!(p.facts[0][0], iri("http://example.org/s"));
}

#[test]
fn pname_with_char_containing_0x85_continuation_byte_does_not_panic() {
    // U+0085 NEL encodes as C2 85 — the 85 continuation byte used to split the scan.
    // Whether this parses or errors is secondary; it must not panic.
    let _ = parse("@prefix p\u{0085}: <http://example.org/> . p\u{0085}:s p\u{0085}:p p\u{0085}:o .");
    // And in subject position via a prefixed-name token.
    let _ = parse("@prefix e: <http://example.org/> . e:a\u{0460}b e:p e:o .");
}
