//! [OPUS-5] sq-xqchl.1 (GH #3144) — the EYE `--query` filter: every INSTANTIATED conclusion of
//! a query document's forward rules over the deductive closure of the data document.
//!
//! The load-bearing invariant is that the query premise is evaluated by the SAME matcher the
//! forward chainer uses, so the FULL premise language is available — builtins, quoted `{ … }`
//! formulae and first-class `( … )` lists all evaluate exactly as they do in a document rule.
//! The compat path this replaces translated the premise into a SPARQL BGP, which cannot
//! evaluate any of them and so rejected them fail-closed; these tests pin that they now
//! produce the RIGHT answer, not merely *an* answer.
//!
//! The second invariant: `--query` is a PROJECTION, not a closure step. A conclusion already
//! present in the closure is still an answer (that is what separates `--query` from
//! `--pass-only-new`).

use sparq_reason::n3::Term;
use sparq_reason::{reason_n3_query, reason_n3_query_terms};

const EX: &str = "http://ex/";
const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn iri(local: &str) -> Term {
    Term::Iri(format!("{EX}{local}"))
}

fn int(v: &str) -> Term {
    Term::Lit(v.into(), "http://www.w3.org/2001/XMLSchema#integer".into(), None)
}

/// `[s p o]` as a row, for comparing against an answer set.
fn row(s: Term, p: Term, o: Term) -> [Term; 3] {
    [s, p, o]
}

/// The eye-js README Socrates example, verbatim: the query projects `:Socrates a ?WHAT` over the
/// closure, so BOTH the asserted `Human` typing and the entailed `Mortal` typing are answers —
/// and the non-matching `rdfs:subClassOf` axiom is not.
#[test]
fn query_projects_over_the_closure_including_already_asserted_conclusions() {
    let data = r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#>.
@prefix : <http://ex/>.
:Socrates a :Human.
:Human rdfs:subClassOf :Mortal.
{ ?A rdfs:subClassOf ?B. ?S a ?A } => { ?S a ?B }."#;
    let query = "@prefix : <http://ex/>. { :Socrates a ?WHAT } => { :Socrates a ?WHAT }.";
    let answers = reason_n3_query_terms(data, query).expect("query filter");
    let ty = Term::Iri(TYPE.into());
    assert!(
        answers.contains(&row(iri("Socrates"), ty.clone(), iri("Human"))),
        "the ASSERTED typing is an answer too — a projection, not a `--pass-only-new` delta: {answers:?}"
    );
    assert!(
        answers.contains(&row(iri("Socrates"), ty, iri("Mortal"))),
        "the entailed typing is an answer: {answers:?}"
    );
    assert_eq!(answers.len(), 2, "the subClassOf axiom does not match the query: {answers:?}");
}

/// sq-xqchl.1 — a `math:` COMPARISON builtin in the query premise. Previously rejected
/// fail-closed (a BGP cannot evaluate it); now it filters, so only the over-18 subject answers.
#[test]
fn query_premise_evaluates_a_comparison_builtin() {
    let data = "@prefix : <http://ex/>. :a :age 21 . :b :age 12 .";
    let query = r#"@prefix math: <http://www.w3.org/2000/10/swap/math#>.
@prefix : <http://ex/>.
{ ?x :age ?n. ?n math:greaterThan 18 } => { ?x a :Adult }."#;
    let answers = reason_n3_query_terms(data, query).expect("builtin query");
    assert_eq!(
        answers,
        vec![row(iri("a"), Term::Iri(TYPE.into()), iri("Adult"))],
        "only :a is over 18 — the builtin FILTERS; matching it as data would answer both: {answers:?}"
    );
}

/// A FUNCTIONAL builtin computes a value that the conclusion projects — the answer is a term
/// that appears nowhere in the data, so a BGP translation could not have produced it at all.
#[test]
fn query_premise_evaluates_a_functional_builtin() {
    let data = "@prefix : <http://ex/>. :a :x 2 . :a :y 40 .";
    let query = r#"@prefix math: <http://www.w3.org/2000/10/swap/math#>.
@prefix : <http://ex/>.
{ ?s :x ?a. ?s :y ?b. (?a ?b) math:sum ?t } => { ?s :total ?t }."#;
    let answers = reason_n3_query_terms(data, query).expect("functional builtin query");
    assert_eq!(answers, vec![row(iri("a"), iri("total"), int("42"))], "{answers:?}");
}

/// A first-class `( … )` LIST in the query premise: `list:member` iterates the list VALUE held
/// by the data. Previously rejected fail-closed.
#[test]
fn query_premise_matches_a_first_class_list() {
    let data = "@prefix : <http://ex/>. :alice :children (:bob :carol) .";
    let query = r#"@prefix list: <http://www.w3.org/2000/10/swap/list#>.
@prefix : <http://ex/>.
{ ?p :children ?l. ?l list:member ?c } => { ?c :parent ?p }."#;
    let answers = reason_n3_query_terms(data, query).expect("list query");
    assert_eq!(answers.len(), 2, "one answer per list member: {answers:?}");
    assert!(answers.contains(&row(iri("bob"), iri("parent"), iri("alice"))), "{answers:?}");
    assert!(answers.contains(&row(iri("carol"), iri("parent"), iri("alice"))), "{answers:?}");
}

/// A literal `( … )` list PATTERN in the query premise unifies STRUCTURALLY against the list
/// VALUE in the data (members pairwise), so a variable inside the pattern binds.
#[test]
fn query_premise_unifies_a_literal_list_pattern() {
    let data = "@prefix : <http://ex/>. :alice :children (:bob :carol) . :dan :children (:eve) .";
    let query = "@prefix : <http://ex/>. { ?p :children (:bob ?second) } => { ?second :sibling :bob }.";
    let answers = reason_n3_query_terms(data, query).expect("list-pattern query");
    assert_eq!(
        answers,
        vec![row(iri("carol"), iri("sibling"), iri("bob"))],
        "only :alice's two-member list unifies; :dan's one-member list must not: {answers:?}"
    );
}

/// A quoted `{ … }` FORMULA in the query premise: `log:includes` tests containment in a
/// formula-valued fact of the closure. Previously rejected fail-closed.
#[test]
fn query_premise_matches_a_quoted_formula() {
    let data = "@prefix : <http://ex/>. :alice :says { :sky :is :blue } . :bob :says { :sky :is :green } .";
    let query = r#"@prefix log: <http://www.w3.org/2000/10/swap/log#>.
@prefix : <http://ex/>.
{ ?who :says ?f. ?f log:includes { :sky :is :blue } } => { ?who a :Correct }."#;
    let answers = reason_n3_query_terms(data, query).expect("formula query");
    assert_eq!(
        answers,
        vec![row(iri("alice"), Term::Iri(TYPE.into()), iri("Correct"))],
        "containment must SELECT alice and reject bob: {answers:?}"
    );
}

/// The query document's FACTS are not loaded as data (EYE reads the query file as a query, not
/// as a second data document), so a premise that only its own facts could satisfy has no answer.
#[test]
fn query_document_facts_are_not_data() {
    let data = "@prefix : <http://ex/>. :a :p :b .";
    let query = "@prefix : <http://ex/>. :c :p :d . { ?s :p ?o } => { ?s :q ?o }.";
    let answers = reason_n3_query_terms(data, query).expect("query");
    assert_eq!(answers, vec![row(iri("a"), iri("q"), iri("b"))], "{answers:?}");
}

/// A BACKWARD (`<=`) rule of the data document is goal-directed — it never fires forward, so it
/// contributes nothing to the closure — but it IS available to the query premise.
#[test]
fn query_premise_resolves_a_backward_rule() {
    let data = "@prefix : <http://ex/>. :a :p :b . { ?s :q ?o } <= { ?s :p ?o }.";
    let query = "@prefix : <http://ex/>. { ?s :q ?o } => { ?s :answer ?o }.";
    let answers = reason_n3_query_terms(data, query).expect("backward query");
    assert_eq!(answers, vec![row(iri("a"), iri("answer"), iri("b"))], "{answers:?}");
}

/// Two query rules that project the same conclusion answer ONCE (answers are deduplicated).
#[test]
fn answers_are_deduplicated_across_query_rules() {
    let data = "@prefix : <http://ex/>. :a :p :b .";
    let query = r#"@prefix : <http://ex/>.
{ ?s :p ?o } => { ?s :hit ?o }.
{ ?s :p ?o } => { ?s :hit ?o }."#;
    let answers = reason_n3_query_terms(data, query).expect("query");
    assert_eq!(answers, vec![row(iri("a"), iri("hit"), iri("b"))], "{answers:?}");
}

/// A query document with no forward rule fails LOUDLY rather than returning an empty answer
/// that reads like "the query matched nothing".
#[test]
fn a_query_document_without_a_forward_rule_is_an_error() {
    let err = reason_n3_query_terms("@prefix : <http://ex/>. :a :p :b .", "@prefix : <http://ex/>. :a :p :b .")
        .unwrap_err();
    assert!(err.contains("no `{ … } => { … }` forward rule"), "{err}");
}

/// The dictionary-interning entry point returns the answer in RDF shape: a list-VALUED answer
/// term is expanded into its rdf:first/rest chain, whose structure triples come back with it.
#[test]
fn interning_entry_point_expands_a_list_valued_answer() {
    let mut dict = sparq_core::dict::Dict::default();
    let data = "@prefix : <http://ex/>. :a :p 1 . :a :p 2 .";
    let query = r#"@prefix log: <http://www.w3.org/2000/10/swap/log#>.
@prefix : <http://ex/>.
{ ?s :p ?x } => { ?s :ps ?x }."#;
    let ids = reason_n3_query(&mut dict, data, query).expect("interned query");
    assert_eq!(ids.len(), 2, "two answers, no list structure to expand: {ids:?}");

    // Now a query whose conclusion IS a list value: the chain structure is part of the answer.
    let list_query = "@prefix : <http://ex/>. { ?s :p 1 } => { ?s :pair (1 2) }.";
    let ids = reason_n3_query(&mut dict, data, list_query).expect("interned list answer");
    assert_eq!(
        ids.len(),
        5,
        "the `:pair` answer plus the two-cell rdf:first/rest chain (2 triples per cell): {ids:?}"
    );
}

/// Generated rdf:first/rest CELLS must never be the same RDF node as a blank the answer
/// already carries. The list expander used to mint `_l1`, `_l2`, … unconditionally, so a query
/// projecting a data blank literally spelled `_:_l1` alongside a list value MERGED that blank
/// with a chain cell — two distinct RDF nodes collapsing into one, i.e. a semantically wrong
/// answer graph. Cell labels are now allocated under a prefix proven fresh against every blank
/// reachable in the rows being expanded.
#[test]
fn generated_list_cells_never_merge_with_a_projected_blank() {
    const FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
    const REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";

    let mut dict = sparq_core::dict::Dict::default();
    // Cells were minted tail-first, so the old expander gave `(1 2)` the labels `_l1` (the `2`
    // cell) and `_l2` (the head) — `_:_l1` is therefore a label the data can collide with.
    let data = "@prefix : <http://ex/>. :a :seeAlso _:_l1 . _:_l1 :name \"n\" .";
    let query = "@prefix : <http://ex/>. { ?s :seeAlso ?b } => { ?b :pair (1 2) }.";
    let ids = reason_n3_query(&mut dict, data, query).expect("interned list answer");

    // Interning is idempotent, so this is the id the PROJECTED data blank already has.
    let projected = dict.intern_blank("_l1");
    let pair = dict.intern_iri("http://ex/pair");
    let first = dict.intern_iri(FIRST);
    let rest = dict.intern_iri(REST);

    let answer = ids.iter().find(|r| r[1] == pair).expect("the `:pair` answer row");
    assert_eq!(
        answer[0], projected,
        "the projected blank keeps its own identity — it IS `_:_l1` from the data: {ids:?}"
    );
    let cells: Vec<_> = ids.iter().filter(|r| r[1] == first).map(|r| r[0]).collect();
    assert_eq!(cells.len(), 2, "a two-member list expands to two cells: {ids:?}");
    assert!(
        !cells.contains(&projected),
        "a generated list cell MERGED with the projected `_:_l1` blank — two distinct RDF \
         nodes collapsed into one: {ids:?}"
    );
    assert!(
        !ids.iter().any(|r| r[1] == rest && r[0] == projected),
        "the projected blank must not carry chain structure either: {ids:?}"
    );
    assert_eq!(ids.len(), 5, "the `:pair` answer plus 2 triples per cell: {ids:?}");
}
