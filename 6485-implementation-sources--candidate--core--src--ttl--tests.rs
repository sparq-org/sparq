//! [OPUS-4.8] (sq-jocpn) The load-bearing correctness harness for the native Turtle parser: a
//! DIFFERENTIAL oracle that parses each fixture with BOTH the native path and oxttl and asserts the
//! resulting triple SETS are equal up to blank-node isomorphism, plus a malformed-input parity
//! fixture where BOTH parsers must `Err`. Ground terms (IRIs, literals, labelled blank nodes) must
//! be byte-identical; only anonymous blank-node labels may differ, so the comparison is
//! blank-node-isomorphic (the same relation `turtle_suite.rs` and the parallel differential use).

use super::parse;
use crate::dict::Dict;
use oxrdf::Term;
use std::collections::HashMap;

/// Render a `[Id;3]` from our dict into three `Display` strings (IRIs `<…>`, blanks `_:…`,
/// literals via oxrdf `Display`) — the same shapes oxttl's `Display` produces, so the two sides
/// compare directly.
fn native_triples(src: &str, base: Option<&str>) -> Result<Vec<[String; 3]>, String> {
    let mut d = Dict::new();
    let ids = parse(src.as_bytes(), base, &mut d)?;
    Ok(ids
        .iter()
        .map(|&[s, p, o]| [d.term(s).to_string(), d.term(p).to_string(), d.term(o).to_string()])
        .collect())
}

/// Parse via oxttl (the reference), returning `Display`-rendered triples.
fn oxttl_triples(src: &str, base: Option<&str>) -> Result<Vec<[String; 3]>, String> {
    let mut p = oxttl::TurtleParser::new();
    if let Some(b) = base {
        p = p.with_base_iri(b).map_err(|e| e.to_string())?;
    }
    let mut out = Vec::new();
    for t in p.for_slice(src.as_bytes()) {
        let t = t.map_err(|e| e.to_string())?;
        out.push([
            t.subject.to_string(),
            format!("<{}>", t.predicate.as_str()),
            term_str(&t.object),
        ]);
    }
    Ok(out)
}

fn term_str(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        other => other.to_string(),
    }
}

// ---- blank-node graph isomorphism (backtracking, same relation as the conformance suite) ----

fn is_bnode(s: &str) -> bool {
    s.starts_with("_:")
}

fn iso(a: &[[String; 3]], b: &[[String; 3]]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut fwd: HashMap<String, String> = HashMap::new();
    let mut rev: HashMap<String, String> = HashMap::new();
    let mut trail: Vec<String> = Vec::new();
    let mut used = vec![false; b.len()];
    search(0, a, b, &mut used, &mut fwd, &mut rev, &mut trail, &mut 0)
}

#[allow(clippy::too_many_arguments)]
fn search(
    i: usize,
    a: &[[String; 3]],
    b: &[[String; 3]],
    used: &mut [bool],
    fwd: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
    trail: &mut Vec<String>,
    steps: &mut usize,
) -> bool {
    const BUDGET: usize = 2_000_000;
    if i == a.len() {
        return true;
    }
    for j in 0..b.len() {
        if used[j] {
            continue;
        }
        *steps += 1;
        if *steps > BUDGET {
            return false;
        }
        let mark = trail.len();
        if (0..3).all(|k| pos_match(&a[i][k], &b[j][k], fwd, rev, trail)) {
            used[j] = true;
            if search(i + 1, a, b, used, fwd, rev, trail, steps) {
                return true;
            }
            used[j] = false;
        }
        while trail.len() > mark {
            let x = trail.pop().unwrap();
            if let Some(y) = fwd.remove(&x) {
                rev.remove(&y);
            }
        }
    }
    false
}

fn pos_match(
    x: &str,
    y: &str,
    fwd: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
    trail: &mut Vec<String>,
) -> bool {
    match (is_bnode(x), is_bnode(y)) {
        (true, true) => match (fwd.get(x), rev.get(y)) {
            (Some(mx), Some(my)) => mx == y && my == x,
            (None, None) => {
                fwd.insert(x.to_string(), y.to_string());
                rev.insert(y.to_string(), x.to_string());
                trail.push(x.to_string());
                true
            }
            _ => false,
        },
        (false, false) => x == y,
        _ => false,
    }
}

/// Assert the native parse and the oxttl parse produce isomorphic graphs for `src`.
#[track_caller]
fn assert_same(src: &str) {
    assert_same_base(src, None);
}

#[track_caller]
fn assert_same_base(src: &str, base: Option<&str>) {
    let n = native_triples(src, base).unwrap_or_else(|e| panic!("native parse failed on {src:?}: {e}"));
    let o = oxttl_triples(src, base).unwrap_or_else(|e| panic!("oxttl parse failed on {src:?}: {e}"));
    assert!(
        iso(&n, &o),
        "native != oxttl for {src:?}\n  native ({}): {:#?}\n  oxttl  ({}): {:#?}",
        n.len(),
        n,
        o.len(),
        o
    );
}

/// Assert BOTH parsers reject `src` (malformed-input parity).
#[track_caller]
fn assert_both_reject(src: &str) {
    let n = native_triples(src, None);
    let o = oxttl_triples(src, None);
    assert!(
        o.is_err(),
        "the fixture is not actually rejected by oxttl (fix the fixture): {src:?} -> {o:?}"
    );
    assert!(
        n.is_err(),
        "native parser ACCEPTED input oxttl rejects (unsound): {src:?}\n  native: {n:?}"
    );
}

const PFX: &str = "@prefix ex: <http://ex/> .\n";

#[test]
fn plain_triples_and_prefixed_names() {
    assert_same(&format!("{PFX}ex:s ex:p ex:o ."));
    assert_same("<http://a/s> <http://a/p> <http://a/o> .");
    assert_same("@prefix : <http://d/> . :s :p :o .");
}

#[test]
fn a_keyword_is_rdf_type() {
    assert_same(&format!("{PFX}ex:s a ex:C ."));
}

#[test]
fn predicate_and_object_lists() {
    assert_same(&format!("{PFX}ex:s ex:p 1 , 2 , 3 ; ex:q ex:o ; a ex:C ."));
    assert_same(&format!("{PFX}ex:s ex:p 1 ;; ex:q 2 ; ."));
}

#[test]
fn numeric_literals_keep_lexical_form_and_datatype() {
    for lit in ["1", "-0", "+1", "007", "1.0", ".5", "1.0e0", "1E0", "1.e0", "-3.14", "6.022e23"] {
        assert_same(&format!("{PFX}ex:s ex:p {lit} ."));
    }
}

#[test]
fn boolean_literals() {
    assert_same(&format!("{PFX}ex:s ex:p true ."));
    assert_same(&format!("{PFX}ex:s ex:p false ."));
}

#[test]
fn string_literals_all_quote_forms_and_escapes() {
    assert_same(&format!("{PFX}ex:s ex:p \"hello\" ."));
    assert_same(&format!("{PFX}ex:s ex:p 'single' ."));
    assert_same(&format!("{PFX}ex:s ex:p \"a\\tb\\nc\\\"d\\\\e\" ."));
    assert_same(&format!("{PFX}ex:s ex:p \"\"\"triple\nquoted \"inside\" ok\"\"\" ."));
    assert_same(&format!("{PFX}ex:s ex:p '''single triple''' ."));
    assert_same(&format!("{PFX}ex:s ex:p \"\\u00e9\\U0001F600\" ."));
}

#[test]
fn typed_and_lang_literals() {
    assert_same(&format!("{PFX}ex:s ex:p \"5\"^^ex:myType ."));
    assert_same(&format!("{PFX}ex:s ex:p \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> ."));
    assert_same(&format!("{PFX}ex:s ex:p \"hi\"@en ."));
    assert_same(&format!("{PFX}ex:s ex:p \"hi\"@EN-US ."));
    assert_same(&format!("{PFX}ex:s ex:p \"hi\"@en-Latn-US ."));
}

#[test]
fn labeled_blank_nodes_scoped_across_statements() {
    assert_same(&format!("{PFX}_:a ex:p _:b . _:b ex:q _:a ."));
}

#[test]
fn anonymous_blank_node_property_lists() {
    assert_same(&format!("{PFX}ex:s ex:p [ ex:q 1 ] ."));
    assert_same(&format!("{PFX}[ ex:q 1 ] ex:p ex:o ."));
    assert_same(&format!("{PFX}ex:s ex:p [] ."));
    assert_same(&format!("{PFX}[ ex:a 1 ; ex:b [ ex:c 2 ] ] ex:p ex:o ."));
}

#[test]
fn collections() {
    assert_same(&format!("{PFX}ex:s ex:p () ."));
    assert_same(&format!("{PFX}ex:s ex:p ( 1 2 3 ) ."));
    assert_same(&format!("{PFX}ex:s ex:p ( ex:a ex:b [ ex:q 1 ] ) ."));
    assert_same(&format!("{PFX}( 1 2 ) ex:p ex:o ."));
}

#[test]
fn pn_local_escapes_and_percent_encoding() {
    assert_same("@prefix : <http://ex/> . :s :p\\.x :o\\#y .");
    assert_same(&format!("{PFX}ex:s ex:p ex:a%20b ."));
    assert_same(&format!("{PFX}ex:foo.bar ex:p ex:o ."));
    assert_same(&format!("{PFX}ex:s ex:p ex: ."));
}

#[test]
fn base_and_relative_iri_resolution() {
    assert_same("@base <http://a/b/c> . <../d> <#p> <e> .");
    assert_same("@base <http://a/b/c> . <> <#p> <//other/x> .");
    assert_same_base(&format!("{PFX}<rel> ex:p <also/rel> ."), Some("http://root/dir/"));
    assert_same_base("<s> <p> <o> .", Some("http://base.example/x/"));
    // SPARQL-style BASE + PREFIX (no dot terminator).
    assert_same("BASE <http://a/> PREFIX p: <http://p/> <s> p:o p:v .");
    // relative BASE resolves against the prior base
    assert_same("@base <http://a/b/> . @base <c/> . <d> <p> <e> .");
}

#[test]
fn mixed_directive_forms_and_comments() {
    assert_same("# a comment\n@prefix ex: <http://ex/> . # trailing\nex:s ex:p ex:o . # end");
    assert_same("PREFIX ex: <http://ex/>\nex:s ex:p ex:o .");
    assert_same("@prefix ex: <http://ex/> .\n@prefix ex: <http://redef/> .\nex:s ex:p ex:o .");
}

#[test]
fn iriref_with_uchar_escape() {
    assert_same("<http://ex/\\u0061> <http://ex/p> <http://ex/o> .");
}

#[test]
fn empty_and_whitespace_documents() {
    assert_same("");
    assert_same("   \n\t  # only a comment\n");
    assert_same("@prefix ex: <http://ex/> .");
}

// ---- malformed-input parity: both parsers must reject ----

#[test]
fn undeclared_prefix_rejected() {
    assert_both_reject("ex:s ex:p ex:o .");
    assert_both_reject(&format!("{PFX}ex:s ex:p nope:o ."));
}

#[test]
fn missing_terminator_rejected() {
    assert_both_reject(&format!("{PFX}ex:s ex:p ex:o"));
    assert_both_reject(&format!("{PFX}ex:s ex:p ex:o ; ex:q"));
}

#[test]
fn unterminated_string_rejected() {
    assert_both_reject(&format!("{PFX}ex:s ex:p \"unterminated ."));
    assert_both_reject(&format!("{PFX}ex:s ex:p \"\"\"unterminated ."));
}

#[test]
fn unterminated_iriref_rejected() {
    assert_both_reject("<http://ex/s <http://ex/p> <http://ex/o> .");
}

#[test]
fn bad_number_rejected() {
    assert_both_reject(&format!("{PFX}ex:s ex:p 1.2.3 ."));
    assert_both_reject(&format!("{PFX}ex:s ex:p 1e ."));
}

#[test]
fn relative_iri_with_no_base_rejected() {
    // With no base, a relative IRI reference must be rejected by both.
    assert_both_reject("<rel> <p> <o> .");
}

#[test]
fn unclosed_bracket_and_paren_rejected() {
    assert_both_reject(&format!("{PFX}ex:s ex:p [ ex:q 1 ."));
    assert_both_reject(&format!("{PFX}ex:s ex:p ( 1 2 ."));
}

#[test]
fn rdf12_reifiers_triple_terms_and_annotations() {
    // The workspace oxttl is built with `rdf-12`, so `<< … >>` reifiers, `<<( … )>>` triple terms,
    // `~`, and `{| … |}` annotation blocks are all accepted BY DEFAULT and desugar to rdf:reifies.
    // The native parser must reproduce that desugaring (up to anon reifier-blank labels).
    assert_same(&format!("{PFX}<< ex:a ex:b ex:c >> ex:p ex:o ."));
    assert_same(&format!("{PFX}ex:a ex:b << ex:s ex:p ex:o >> ."));
    assert_same(&format!("{PFX}ex:s ex:reifies <<( ex:a ex:b ex:c )>> ."));
    assert_same(&format!("{PFX}ex:s ex:p ex:o ~ex:r ."));
    assert_same(&format!("{PFX}ex:s ex:p ex:o ~ ."));
    assert_same(&format!("{PFX}ex:s ex:p ex:o {{| ex:c 0.9 ; ex:d ex:e |}} ."));
    assert_same(&format!("{PFX}ex:s ex:p ex:o ~ ex:r1 {{| ex:c 1 |}} ."));
    assert_same(&format!("{PFX}<< ex:s ex:p ex:o ~ ex:rid >> ex:x ex:y ."));
    // nested triple term in a triple term's object
    assert_same(&format!("{PFX}ex:s ex:r <<( ex:a ex:b <<( ex:c ex:d ex:e )>> )>> ."));
}

#[test]
fn bad_escape_rejected() {
    assert_both_reject(&format!("{PFX}ex:s ex:p \"bad\\qescape\" ."));
    assert_both_reject(&format!("{PFX}ex:s ex:p \"\\u00\" ."));
}

// ---- an anti-vacuity guard: a deliberately-wrong expectation MUST fail iso ----

#[test]
fn iso_is_not_vacuous() {
    let a = vec![["<http://a>".into(), "<http://p>".into(), "<http://x>".into()]];
    let b = vec![["<http://a>".into(), "<http://p>".into(), "<http://y>".into()]];
    assert!(!iso(&a, &b), "iso must distinguish different ground objects");
    let c = vec![["_:x".into(), "<http://p>".into(), "_:y".into()]];
    let d = vec![["_:m".into(), "<http://p>".into(), "_:n".into()]];
    assert!(iso(&c, &d), "iso must accept a consistent blank-node renaming");
}

// ---- anonymous blank labels are unique across independent parses (parallel-merge safety) ----

#[test]
fn fresh_anon_labels_unique_across_parses() {
    // Two independent parses of a `[]` must NOT mint the same anon label — otherwise the
    // label-based dict merge in the parallel loader would wrongly unify two distinct anon nodes.
    let mut d1 = Dict::new();
    let ids1 = parse(format!("{PFX}ex:s ex:p [] .").as_bytes(), None, &mut d1).unwrap();
    let mut d2 = Dict::new();
    let ids2 = parse(format!("{PFX}ex:s ex:p [] .").as_bytes(), None, &mut d2).unwrap();
    let l1 = d1.term(ids1[0][2]).to_string();
    let l2 = d2.term(ids2[0][2]).to_string();
    assert!(l1.starts_with("_:"), "anon object is a blank node: {l1}");
    assert_ne!(l1, l2, "two independent parses must mint DISTINCT anon labels");
}
