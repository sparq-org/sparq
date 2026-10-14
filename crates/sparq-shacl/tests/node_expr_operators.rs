//! (sq-qcnn) Correctness coverage for the SHACL-AF node-expression
//! **function operators** (`src/func.rs`) — fixture-independent unit tests that
//! drive every built-in operator through the PUBLIC `eval_node_expression` seam
//! and the `apply_rules` value-rule path, asserting hand-derived result node sets
//! from the SHACL 1.2 node-expression algebra.
//!
//! Why a dedicated file: the W3C `node-expr` suite that exercises these operators
//! is fetched into a gitignored `tests/shacl/` dir and SKIPS on a fresh checkout,
//! so on the per-commit coverage gate `func.rs` ran dark for most operators
//! (~56% lines). These tests cover the operators deterministically regardless of
//! the fixtures, and pin the SEMANTICS (not just line-exercise): each assertion is
//! the node set the SHACL-AF algebra specifies for that operator.
//!
//! Whole file is `#[cfg(feature = "shacl-af")]` — the node-expression public API
//! it drives is itself feature-gated, so with the feature off this compiles to
//! nothing (and the AGENTS.md "both feature states" standard is met: the file is a
//! no-op in the default build).

#![cfg(feature = "shacl-af")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::eval_node_expression;
use sparq_shacl::view::GraphView;

const PRELUDE: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
    @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .\n\
    @prefix ex: <http://example.org/> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n";

fn g(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PRELUDE}{ttl}"), "turtle").unwrap()
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(format!("http://example.org/{s}")))
}

/// Evaluate the node expression bound to `ex:<decl_pred>` on `ex:Decl` over `data`.
fn eval(data: &Graph, shapes: &Graph, decl_pred: &str, focus: &Term) -> Vec<Term> {
    let view = GraphView::new(shapes);
    let decl = iri("Decl");
    let e = view
        .object(&decl, &format!("http://example.org/{decl_pred}"))
        .unwrap_or_else(|| panic!("node-expr bnode ex:{decl_pred} present"));
    eval_node_expression(data, shapes, &e, focus)
        .unwrap_or_else(|| panic!("ex:{decl_pred} must be a supported node expression"))
}

/// The lexical values of a result set, sorted (set-equality with dups collapsed
/// by the caller's choice — most assertions use a multiset, so we keep dups).
fn lexes(set: &[Term]) -> Vec<String> {
    let mut v: Vec<String> = set
        .iter()
        .map(|t| match t {
            Term::Literal(l) => l.value().to_string(),
            Term::NamedNode(n) => n.as_str().to_string(),
            Term::BlankNode(b) => b.as_str().to_string(),
            other => other.to_string(),
        })
        .collect();
    v.sort();
    v
}

/// The datatype IRI of a single-literal result (panics if not a single literal).
fn one_dt(set: &[Term]) -> String {
    assert_eq!(set.len(), 1, "expected exactly one term: {set:?}");
    match &set[0] {
        Term::Literal(l) => l.datatype().as_str().to_string(),
        other => panic!("expected a literal, got {other:?}"),
    }
}

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

// ───────────────────────── aggregate / numeric operators ─────────────────────────

/// `sum` over mixed integer args stays `xsd:integer`; a decimal arg makes it
/// `xsd:decimal`; non-numeric members are ignored; the empty set sums to 0.
#[test]
fn sum_integer_decimal_and_empty() {
    let data = g("ex:a a ex:N .");
    let shapes = g(r#"
        ex:Decl
          ex:allInt   [ shnex:sum ( 4 5 3 ) ] ;
          ex:withDec  [ shnex:sum ( 1 2.5 ) ] ;
          ex:withText [ shnex:sum ( 4 "not a number" 6 ) ] ;
          ex:empty    [ shnex:sum [ sh:path ex:nope ] ] .
    "#);
    let f = iri("a");
    assert_eq!(lexes(&eval(&data, &shapes, "allInt", &f)), vec!["12"]);
    assert_eq!(
        one_dt(&eval(&data, &shapes, "allInt", &f)),
        format!("{XSD}integer")
    );
    assert_eq!(lexes(&eval(&data, &shapes, "withDec", &f)), vec!["3.5"]);
    assert_eq!(
        one_dt(&eval(&data, &shapes, "withDec", &f)),
        format!("{XSD}decimal")
    );
    // Non-numeric members ignored: 4 + 6 = 10.
    assert_eq!(lexes(&eval(&data, &shapes, "withText", &f)), vec!["10"]);
    // Empty input sums to integer 0.
    assert_eq!(lexes(&eval(&data, &shapes, "empty", &f)), vec!["0"]);
}

/// `min` / `max` pick the numerically least / greatest member; a set with no
/// numeric member yields the empty set (NOT a zero).
#[test]
fn min_max_pick_extremes_and_empty_on_non_numeric() {
    let data = g("ex:a a ex:N .");
    let shapes = g(r#"
        ex:Decl
          ex:mn   [ shnex:min ( 7 2 9 4 ) ] ;
          ex:mx   [ shnex:max ( 7 2 9 4 ) ] ;
          ex:mnNN [ shnex:min ( "x" "y" ) ] .
    "#);
    let f = iri("a");
    assert_eq!(lexes(&eval(&data, &shapes, "mn", &f)), vec!["2"]);
    assert_eq!(lexes(&eval(&data, &shapes, "mx", &f)), vec!["9"]);
    // No numeric member -> empty (min_max returns None).
    assert!(eval(&data, &shapes, "mnNN", &f).is_empty());
}

/// `count` returns the cardinality of its argument set as an `xsd:integer`
/// (duplicates counted; `count` does NOT dedup).
#[test]
fn count_counts_with_duplicates() {
    let data = g("ex:a a ex:N .");
    let shapes = g(r#"ex:Decl ex:c [ shnex:count ( 4 3 3 5 ) ] ."#);
    let r = eval(&data, &shapes, "c", &iri("a"));
    assert_eq!(lexes(&r), vec!["4"]);
    assert_eq!(one_dt(&r), format!("{XSD}integer"));
}

/// `distinct` deduplicates by RDF term equality, preserving one of each.
#[test]
fn distinct_dedups_by_term() {
    let data = g("ex:a a ex:N .");
    let shapes = g(r#"ex:Decl ex:d [ shnex:distinct ( 4 2 4 2 9 ) ] ."#);
    assert_eq!(
        lexes(&eval(&data, &shapes, "d", &iri("a"))),
        vec!["2", "4", "9"]
    );
}

// ───────────────────────── boolean / conditional operators ─────────────────────────

/// `exists` is `{ true }` when its argument set is non-empty, `{ false }` when
/// empty — and the result is an `xsd:boolean`.
#[test]
fn exists_reflects_nonemptiness() {
    let data = g("ex:a ex:has ex:x .");
    let shapes = g(r#"
        ex:Decl
          ex:hasSome [ shnex:exists [ sh:path ex:has ] ] ;
          ex:hasNone [ shnex:exists [ sh:path ex:missing ] ] .
    "#);
    let f = iri("a");
    assert_eq!(lexes(&eval(&data, &shapes, "hasSome", &f)), vec!["true"]);
    assert_eq!(
        one_dt(&eval(&data, &shapes, "hasSome", &f)),
        format!("{XSD}boolean")
    );
    assert_eq!(lexes(&eval(&data, &shapes, "hasNone", &f)), vec!["false"]);
}

/// `if … then … else?`: the `then` branch when the condition is `{ true }`, the
/// `else` branch otherwise; with no `else` and a false condition the result is
/// the empty set.
#[test]
fn if_then_else_branches() {
    let data = g("ex:a a ex:N .");
    let shapes = g(r#"
        ex:Decl
          ex:tBranch    [ shnex:if true  ; shnex:then 1 ; shnex:else 2 ] ;
          ex:fBranch    [ shnex:if false ; shnex:then 1 ; shnex:else 2 ] ;
          ex:fNoElse    [ shnex:if false ; shnex:then 1 ] ;
          ex:nonBoolIsF [ shnex:if "yes" ; shnex:then 1 ; shnex:else 2 ] .
    "#);
    let f = iri("a");
    assert_eq!(lexes(&eval(&data, &shapes, "tBranch", &f)), vec!["1"]);
    assert_eq!(lexes(&eval(&data, &shapes, "fBranch", &f)), vec!["2"]);
    // No else + false condition -> empty set.
    assert!(eval(&data, &shapes, "fNoElse", &f).is_empty());
    // A condition that is not exactly { true } takes the else branch.
    assert_eq!(lexes(&eval(&data, &shapes, "nonBoolIsF", &f)), vec!["2"]);
}

// ───────────────────────── set / list operators ─────────────────────────

/// `concat ( E1 E2 … )` concatenates members in order WITHOUT dedup (duplicates
/// preserved across arguments).
#[test]
fn concat_preserves_order_and_duplicates() {
    let data = g(r#"
        ex:a ex:x ex:p1 ; ex:y ex:p1 , ex:p2 .
    "#);
    let shapes = g(r#"
        ex:Decl ex:cat [ shnex:concat ( [ sh:path ex:x ] [ sh:path ex:y ] ) ] .
    "#);
    // ex:x = {p1}; ex:y = {p1, p2}. concat = [p1, p1, p2] (p1 appears twice).
    let r = eval(&data, &shapes, "cat", &iri("a"));
    assert_eq!(r.len(), 3, "concat keeps duplicates: {r:?}");
    let mut vals = lexes(&r);
    vals.sort();
    assert_eq!(
        vals,
        vec![
            "http://example.org/p1".to_string(),
            "http://example.org/p1".to_string(),
            "http://example.org/p2".to_string()
        ]
    );
}

/// `nodes N ; limit k` keeps the first `k`; `offset k` drops the first `k`; they
/// compose as offset-then-limit. The input order is the data-graph order, which
/// is deterministic for a single subject's objects here, so we assert as a set.
#[test]
fn limit_and_offset_slice() {
    // Five distinct objects on a single predicate.
    let data = g(r#"ex:a ex:p ex:n1 , ex:n2 , ex:n3 , ex:n4 , ex:n5 ."#);
    let shapes = g(r#"
        ex:Decl
          ex:lim2 [ shnex:nodes [ sh:path ex:p ] ; shnex:limit 2 ] ;
          ex:off3 [ shnex:nodes [ sh:path ex:p ] ; shnex:offset 3 ] .
    "#);
    let f = iri("a");
    // limit 2 -> exactly 2 of the 5.
    assert_eq!(eval(&data, &shapes, "lim2", &f).len(), 2);
    // offset 3 -> the remaining 2 of the 5.
    assert_eq!(eval(&data, &shapes, "off3", &f).len(), 2);
}

/// `nodes N ; remove R`: `Eval(N)` minus the members of `Eval(R)` (term
/// equality), order-preserving.
#[test]
fn remove_subtracts_members() {
    let data = g(r#"
        ex:a ex:p ex:n1 , ex:n2 , ex:n3 .
        ex:a ex:drop ex:n2 .
    "#);
    let shapes = g(r#"
        ex:Decl ex:r [ shnex:nodes [ sh:path ex:p ] ; shnex:remove [ sh:path ex:drop ] ] .
    "#);
    // {n1,n2,n3} minus {n2} = {n1, n3}.
    assert_eq!(
        lexes(&eval(&data, &shapes, "r", &iri("a"))),
        vec!["http://example.org/n1", "http://example.org/n3"]
    );
}

/// `nodes N ; flatMap E`: for each member, evaluate `E` with the focus rebound,
/// concatenated (no dedup). Here flatMap over each child collects each child's
/// `ex:name`.
#[test]
fn flat_map_rebinds_focus_per_member() {
    let data = g(r#"
        ex:a ex:child ex:b , ex:c .
        ex:b ex:name "Bob" .
        ex:c ex:name "Carol" .
    "#);
    let shapes = g(r#"
        ex:Decl ex:fm [ shnex:nodes [ sh:path ex:child ] ; shnex:flatMap [ sh:path ex:name ] ] .
    "#);
    assert_eq!(
        lexes(&eval(&data, &shapes, "fm", &iri("a"))),
        vec!["Bob", "Carol"]
    );
}

/// `nodes N ; orderBy K` sorts by the least key value of each node, ascending by
/// default and descending with `shnex:desc true`. A node with NO key sorts first
/// (ascending).
#[test]
fn order_by_ascending_descending_and_missing_key() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 , ex:i3 .
        ex:i1 ex:h 30 .
        ex:i2 ex:h 10 .
        ex:i3 ex:h 20 .
    "#);
    let shapes = g(r#"
        ex:Decl
          ex:asc  [ shnex:nodes [ sh:path ex:item ] ; shnex:orderBy [ sh:path ex:h ] ] ;
          ex:desc [ shnex:nodes [ sh:path ex:item ] ; shnex:orderBy [ sh:path ex:h ] ; shnex:desc true ] .
    "#);
    let f = iri("a");
    // ascending by height: i2(10), i3(20), i1(30) — order is observable here.
    let asc: Vec<String> = eval(&data, &shapes, "asc", &f)
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(
        asc,
        vec![
            "<http://example.org/i2>".to_string(),
            "<http://example.org/i3>".to_string(),
            "<http://example.org/i1>".to_string()
        ]
    );
    let desc: Vec<String> = eval(&data, &shapes, "desc", &f)
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(
        desc,
        vec![
            "<http://example.org/i1>".to_string(),
            "<http://example.org/i3>".to_string(),
            "<http://example.org/i2>".to_string()
        ]
    );
}

/// orderBy where one node has NO key sorts that node first (ascending).
#[test]
fn order_by_missing_key_sorts_first() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 .
        ex:i1 ex:h 50 .
        # ex:i2 has no ex:h key.
    "#);
    let shapes = g(r#"
        ex:Decl ex:asc [ shnex:nodes [ sh:path ex:item ] ; shnex:orderBy [ sh:path ex:h ] ] .
    "#);
    let asc: Vec<String> = eval(&data, &shapes, "asc", &iri("a"))
        .iter()
        .map(|t| t.to_string())
        .collect();
    // i2 (no key) sorts before i1 (key 50).
    assert_eq!(
        asc,
        vec![
            "<http://example.org/i2>".to_string(),
            "<http://example.org/i1>".to_string()
        ]
    );
}

// ───────────────────────── filter-shape operators ─────────────────────────

/// `findFirst S ; nodes N` returns the first node of `Eval(N)` conforming to the
/// inline filter shape `S` (empty if none conform).
#[test]
fn find_first_returns_first_conforming() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 , ex:i3 .
        ex:i2 ex:tag "ok" .
        ex:i3 ex:tag "ok" .
    "#);
    let shapes = g(r#"
        ex:HasTag a sh:NodeShape ; sh:property [ sh:path ex:tag ; sh:minCount 1 ] .
        ex:Decl ex:ff [
          shnex:nodes [ sh:path ex:item ] ;
          shnex:findFirst ex:HasTag
        ] .
    "#);
    // i1 has no ex:tag (fails minCount 1). The first conforming is i2 OR i3 (the
    // data-graph iteration order over the three objects is deterministic for a
    // single run); assert it is exactly one of the conforming pair, never i1.
    let r = eval(&data, &shapes, "ff", &iri("a"));
    assert_eq!(r.len(), 1, "findFirst yields a single node: {r:?}");
    let got = r[0].to_string();
    assert!(
        got == "<http://example.org/i2>" || got == "<http://example.org/i3>",
        "must be a conforming node (i2 or i3), never the non-conforming i1: {got}"
    );
    assert_ne!(got, "<http://example.org/i1>");
}

/// `findFirst` over a set where NO node conforms yields the empty set.
#[test]
fn find_first_none_conforming_is_empty() {
    let data = g(r#"ex:a ex:item ex:i1 , ex:i2 ."#);
    let shapes = g(r#"
        ex:HasTag a sh:NodeShape ; sh:property [ sh:path ex:tag ; sh:minCount 1 ] .
        ex:Decl ex:ff [
          shnex:nodes [ sh:path ex:item ] ;
          shnex:findFirst ex:HasTag
        ] .
    "#);
    assert!(eval(&data, &shapes, "ff", &iri("a")).is_empty());
}

/// `matchAll S ; nodes N` is `{ true }` iff EVERY node of `Eval(N)` conforms to
/// `S` (empty input ⇒ `{ true }`), else `{ false }`.
#[test]
fn match_all_universal_quantifier() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 .
        ex:i1 ex:tag "x" .
        ex:i2 ex:tag "y" .
        ex:b ex:item ex:j1 , ex:j2 .
        ex:j1 ex:tag "x" .
        # ex:j2 has no tag -> not all conform.
    "#);
    let shapes = g(r#"
        ex:HasTag a sh:NodeShape ; sh:property [ sh:path ex:tag ; sh:minCount 1 ] .
        ex:Decl ex:ma [
          shnex:nodes [ sh:path ex:item ] ;
          shnex:matchAll ex:HasTag
        ] .
    "#);
    // All items of ex:a conform -> { true }.
    assert_eq!(lexes(&eval(&data, &shapes, "ma", &iri("a"))), vec!["true"]);
    // Not all items of ex:b conform -> { false }.
    assert_eq!(lexes(&eval(&data, &shapes, "ma", &iri("b"))), vec!["false"]);
}

/// `matchAll` must use the SPECIFIC named filter shape, not just "a" shape: with
/// two distinct shapes registered, the discriminating one flips the boolean. (This
/// pins `inline_or_named_shape` to the real `by_node` lookup — a constant shape-id
/// would mis-route to the wrong shape and give the opposite answer.)
#[test]
fn match_all_uses_the_named_filter_shape_not_an_arbitrary_one() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 .
        ex:i1 ex:tag "x" .
        ex:i2 ex:tag "y" .
        # Neither item carries ex:other, so the OtherShape filter fails for all.
    "#);
    let shapes = g(r#"
        # Two distinct named shapes. HasTag conforms for both items; HasOther for
        # neither. matchAll over the SAME nodes must give opposite booleans.
        ex:HasTag   a sh:NodeShape ; sh:property [ sh:path ex:tag   ; sh:minCount 1 ] .
        ex:HasOther a sh:NodeShape ; sh:property [ sh:path ex:other ; sh:minCount 1 ] .
        ex:Decl
          ex:maTag   [ shnex:nodes [ sh:path ex:item ] ; shnex:matchAll ex:HasTag ] ;
          ex:maOther [ shnex:nodes [ sh:path ex:item ] ; shnex:matchAll ex:HasOther ] .
    "#);
    let f = iri("a");
    // All items have a tag -> { true }; none have ex:other -> { false }. If the
    // filter-shape id were a constant (the survivor), both would collapse to the
    // SAME boolean — so the two assertions together kill that mutant.
    assert_eq!(lexes(&eval(&data, &shapes, "maTag", &f)), vec!["true"]);
    assert_eq!(lexes(&eval(&data, &shapes, "maOther", &f)), vec!["false"]);
}

/// `matchAll` over an EMPTY input set is vacuously `{ true }`.
#[test]
fn match_all_empty_input_is_vacuously_true() {
    let data = g(r#"ex:a a ex:N ."#);
    let shapes = g(r#"
        ex:HasTag a sh:NodeShape ; sh:property [ sh:path ex:tag ; sh:minCount 1 ] .
        ex:Decl ex:ma [
          shnex:nodes [ sh:path ex:nope ] ;
          shnex:matchAll ex:HasTag
        ] .
    "#);
    assert_eq!(lexes(&eval(&data, &shapes, "ma", &iri("a"))), vec!["true"]);
}

// ───────────────────────── class / shape membership ─────────────────────────

/// `instancesOf C` yields the SHACL instances of class `C` including the subclass
/// closure (an `rdfs:subClassOf` member is an instance of the superclass).
#[test]
fn instances_of_includes_subclass_closure() {
    let data = g(r#"
        ex:Animal a rdfs:Class .
        ex:Dog rdfs:subClassOf ex:Animal .
        ex:rex a ex:Dog .
        ex:generic a ex:Animal .
        ex:other a ex:Plant .
    "#);
    let shapes = g(r#"ex:Decl ex:io [ shnex:instancesOf ex:Animal ] ."#);
    // rex (a Dog ⊑ Animal) and generic (a Animal) are instances; other is not.
    assert_eq!(
        lexes(&eval(&data, &shapes, "io", &iri("rex"))),
        vec!["http://example.org/generic", "http://example.org/rex"]
    );
}

/// `nodesMatching S` yields every node of the data graph conforming to shape `S`.
#[test]
fn nodes_matching_selects_conforming_nodes() {
    let data = g(r#"
        ex:p1 ex:tag "x" .
        ex:p2 ex:tag "y" .
        ex:p3 ex:other "z" .
    "#);
    let shapes = g(r#"
        ex:Decl ex:nm [ shnex:nodesMatching ex:TagShape ] .
        ex:TagShape a sh:NodeShape ; sh:property [ sh:path ex:tag ; sh:minCount 1 ] .
    "#);
    // p1 and p2 have an ex:tag; p3 does not. (Literal objects "x"/"y"/"z" are not
    // candidate nodes; only subjects/non-literal objects are.)
    assert_eq!(
        lexes(&eval(&data, &shapes, "nm", &iri("p1"))),
        vec!["http://example.org/p1", "http://example.org/p2"]
    );
}

// ───────────────────────── var binding ─────────────────────────

/// `var "focusNode"` is bound to the focus; any other variable name is unbound
/// (the empty set), since there is no SPARQL variable scope at node-expr eval.
#[test]
fn var_focus_node_and_unbound() {
    let data = g(r#"ex:a a ex:N ."#);
    let shapes = g(r#"
        ex:Decl
          ex:vf [ shnex:var "focusNode" ] ;
          ex:vx [ shnex:var "somethingElse" ] .
    "#);
    let f = iri("a");
    assert_eq!(
        eval(&data, &shapes, "vf", &f)[0].to_string(),
        "<http://example.org/a>"
    );
    assert!(eval(&data, &shapes, "vx", &f).is_empty());
}

// ───────────────────────── custom sh:SPARQLFunction ─────────────────────────

/// A declared `sh:SPARQLFunction` is dispatched through the SPARQL engine with the
/// argument pre-bound; its `?result` column is the function output.
#[test]
fn custom_sparql_function_dispatches() {
    let data = g(r#"ex:a ex:val 21 ."#);
    let shapes = g(r#"
        ex:double a sh:SPARQLFunction ;
          sh:parameter [ sh:path ex:arg ] ;
          sh:select "SELECT ((?arg * 2) AS ?result) WHERE { }" .
        ex:Decl ex:dbl [ ex:double ( [ sh:path ex:val ] ) ] .
    "#);
    // ex:val = 21; double -> 42.
    assert_eq!(lexes(&eval(&data, &shapes, "dbl", &iri("a"))), vec!["42"]);
}

/// A custom SPARQL function whose argument evaluates to MORE than one node (a
/// non-scalar) is undefined ⇒ the empty set (fail-closed, no panic).
#[test]
fn custom_sparql_function_non_scalar_arg_is_empty() {
    let data = g(r#"ex:a ex:val 1 , 2 , 3 ."#);
    let shapes = g(r#"
        ex:double a sh:SPARQLFunction ;
          sh:parameter [ sh:path ex:arg ] ;
          sh:select "SELECT ((?arg * 2) AS ?result) WHERE { }" .
        ex:Decl ex:dbl [ ex:double ( [ sh:path ex:val ] ) ] .
    "#);
    // ex:val resolves to {1,2,3} (3 nodes) -> scalar binding fails -> empty.
    assert!(eval(&data, &shapes, "dbl", &iri("a")).is_empty());
}

/// An undeclared function IRI (not a `sh:SPARQLFunction`) is NOT a node
/// expression: `eval_node_expression` returns `None` (the lenient-drop contract).
#[test]
fn unknown_function_iri_is_not_a_node_expr() {
    let data = g(r#"ex:a a ex:N ."#);
    let shapes = g(r#"ex:Decl ex:f [ ex:notAFunction ( ex:a ) ] ."#);
    let view = GraphView::new(&shapes);
    let decl = iri("Decl");
    let e = view.object(&decl, "http://example.org/f").unwrap();
    assert!(
        eval_node_expression(&data, &shapes, &e, &iri("a")).is_none(),
        "an undeclared function IRI is not a supported node expression"
    );
}

/// A custom SPARQL function whose argument evaluates to a BLANK NODE has no
/// SPARQL ground form, so the VALUES row cannot be built ⇒ empty set (the
/// `ground` blank-node arm + `build_function_query` `None` path).
#[test]
fn custom_sparql_function_blank_arg_is_empty() {
    // ex:a's ex:ref points at a blank node; double() over a blank arg is undefined.
    let data = g(r#"ex:a ex:ref [ ex:dummy 1 ] ."#);
    let shapes = g(r#"
        ex:double a sh:SPARQLFunction ;
          sh:parameter [ sh:path ex:arg ] ;
          sh:select "SELECT ((?arg * 2) AS ?result) WHERE { }" .
        ex:Decl ex:dbl [ ex:double ( [ sh:path ex:ref ] ) ] .
    "#);
    assert!(eval(&data, &shapes, "dbl", &iri("a")).is_empty());
}

/// A custom SPARQL function whose SELECT body has NO `?result` column yields the
/// empty set (the `position("result")` None arm) — fail-closed, no panic.
#[test]
fn custom_sparql_function_no_result_column_is_empty() {
    let data = g(r#"ex:a ex:val 5 ."#);
    let shapes = g(r#"
        ex:noResult a sh:SPARQLFunction ;
          sh:parameter [ sh:path ex:arg ] ;
          sh:select "SELECT ((?arg * 2) AS ?other) WHERE { }" .
        ex:Decl ex:nr [ ex:noResult ( [ sh:path ex:val ] ) ] .
    "#);
    assert!(eval(&data, &shapes, "nr", &iri("a")).is_empty());
}

/// `orderBy` over a STRING (non-numeric) sort key sorts lexically (the `cmp_term`
/// lexical fallback + `lexical` literal arm), ascending by default.
#[test]
fn order_by_lexical_string_keys() {
    let data = g(r#"
        ex:a ex:item ex:i1 , ex:i2 , ex:i3 .
        ex:i1 ex:name "charlie" .
        ex:i2 ex:name "alice" .
        ex:i3 ex:name "bob" .
    "#);
    let shapes = g(r#"
        ex:Decl ex:ord [ shnex:nodes [ sh:path ex:item ] ; shnex:orderBy [ sh:path ex:name ] ] .
    "#);
    // Lexical ascending by name: alice(i2), bob(i3), charlie(i1).
    let got: Vec<String> = eval(&data, &shapes, "ord", &iri("a"))
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(
        got,
        vec![
            "<http://example.org/i2>".to_string(),
            "<http://example.org/i3>".to_string(),
            "<http://example.org/i1>".to_string()
        ]
    );
}

/// `concat` whose ANY argument is an unsupported sub-expression makes the whole
/// `concat` parse fail ⇒ not a node expression (`None`), the lenient-drop contract
/// at the operand level.
#[test]
fn concat_with_unsupported_operand_is_none() {
    let data = g(r#"ex:a a ex:N ."#);
    // ex:notAFn is not a declared function, so the inner expr is unsupported, so the
    // whole concat list fails to parse.
    let shapes = g(r#"ex:Decl ex:c [ shnex:concat ( [ ex:notAFn ( ex:a ) ] 3 ) ] ."#);
    let view = GraphView::new(&shapes);
    let decl = iri("Decl");
    let e = view.object(&decl, "http://example.org/c").unwrap();
    assert!(
        eval_node_expression(&data, &shapes, &e, &iri("a")).is_none(),
        "a concat over an unsupported operand must not parse"
    );
}

/// A `sh:SPARQLFunction` applied with the WRONG argument count (more or fewer than
/// the declared `sh:parameter` order) is not parsed as a function expression
/// (`None`), matching the declared-arity contract.
#[test]
fn custom_sparql_function_arity_mismatch_is_none() {
    let data = g(r#"ex:a ex:val 1 ."#);
    let shapes = g(r#"
        ex:double a sh:SPARQLFunction ;
          sh:parameter [ sh:path ex:arg ] ;
          sh:select "SELECT ((?arg * 2) AS ?result) WHERE { }" .
        ex:Decl ex:dbl [ ex:double ( [ sh:path ex:val ] [ sh:path ex:val ] ) ] .
    "#);
    let view = GraphView::new(&shapes);
    let decl = iri("Decl");
    let e = view.object(&decl, "http://example.org/dbl").unwrap();
    // Two args against a one-parameter declaration -> arity mismatch -> None.
    assert!(
        eval_node_expression(&data, &shapes, &e, &iri("a")).is_none(),
        "arity mismatch must not parse as a function expression"
    );
}
