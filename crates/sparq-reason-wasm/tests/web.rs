// [OPUS-4.8] sq-6qw3 — headless wasm smoke test of the JS-facing sparq-reason-wasm API.
//
// The crate's `#[cfg(test)] mod tests` in src/ runs on the NATIVE target only, so it
// exercises the engine + reasoner functions the wrappers delegate to — NOT the actual
// `#[wasm_bindgen]` exports (`JsError::new` is a wasm-bindgen import that panics off-wasm).
// This module closes that gap: every test drives the REAL exported `Reasoner` API compiled
// to and executed in a genuine wasm32 runtime via `wasm-pack test --node`, so the closure +
// N-Triples / JSON serialisation across the JS boundary is asserted end to end — exactly the
// surface a `cargo build --target wasm32` never runs.
//
// Tiny, inline datasets. `wasm-pack test --node` runs each `#[wasm_bindgen_test]` in the Node
// executor (no browser, no DOM); Node is the default target, so no
// `wasm_bindgen_test_configure!(run_in_browser)` directive is needed.

#![cfg(target_arch = "wasm32")]

use sparq_reason_wasm::Reasoner;
use wasm_bindgen_test::*;

// The Socrates RDFS example the inference page demonstrates: a subclass axiom + an instance,
// entailing `Socrates a Mortal`.
const SOCRATES_TTL: &str = r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix ex: <http://ex/> .
    ex:Human rdfs:subClassOf ex:Mortal .
    ex:Socrates a ex:Human ."#;

const MORTAL_TYPING: &str =
    "<http://ex/Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Mortal> .";

/// `materialize` returns the FULL closure (base + entailed) as N-Triples, including the
/// entailed Mortal typing.
#[wasm_bindgen_test]
fn materialize_includes_entailed() {
    let nt = Reasoner::materialize(SOCRATES_TTL, "turtle", "rdfs")
        .expect("RDFS materialization must succeed in wasm");
    assert!(
        nt.contains(MORTAL_TYPING),
        "closure must contain the entailed Mortal typing: {nt}"
    );
    // The asserted base is also present (full closure, not just the delta).
    assert!(
        nt.contains("subClassOf"),
        "base subClassOf axiom present: {nt}"
    );
}

/// `entailed` returns ONLY the newly-derived triples: the Mortal typing is present, the
/// asserted base axiom is diffed out.
#[wasm_bindgen_test]
fn entailed_is_the_delta() {
    let nt = Reasoner::entailed(SOCRATES_TTL, "turtle", "rdfs").expect("entailed delta");
    assert!(
        nt.contains(MORTAL_TYPING),
        "delta contains the entailed typing: {nt}"
    );
    assert!(
        !nt.contains("subClassOf"),
        "asserted base must not be in the entailed delta: {nt}"
    );
}

/// `materializeStats` returns a JSON summary whose counts are internally consistent.
#[wasm_bindgen_test]
fn materialize_stats_json() {
    let json = Reasoner::materialize_stats(SOCRATES_TTL, "turtle", "rdfs").expect("stats json");
    assert!(
        json.contains("\"baseTriples\":2"),
        "two distinct base triples: {json}"
    );
    assert!(
        json.contains("\"profile\":\"rdfs\""),
        "profile echoed: {json}"
    );
    // closureTriples > baseTriples (reasoning added at least the Mortal typing).
    assert!(
        json.contains("\"entailed\":"),
        "entailed count present: {json}"
    );
}

/// An N3 rule document entails its conclusion as a ground triple.
#[wasm_bindgen_test]
fn reason_n3_entails_conclusion() {
    let n3 = r#"@prefix ex: <http://ex/> .
        { ?x a ex:Human } => { ?x a ex:Mortal } .
        ex:Socrates a ex:Human ."#;
    let nt = Reasoner::reason_n3(n3).expect("N3 reasoning must succeed in wasm");
    assert!(
        nt.contains("<http://ex/Socrates>") && nt.contains("<http://ex/Mortal>"),
        "N3 rule conclusion missing: {nt}"
    );
}

/// An unknown profile name is a clean error, not a panic.
#[wasm_bindgen_test]
fn unknown_profile_errors() {
    assert!(Reasoner::materialize(SOCRATES_TTL, "turtle", "nope").is_err());
}

// [FABLE-5] sq-ohnj1 — the eye-js README socrates example (data + query), driving the compat
// bindings in a real wasm runtime.
const SOC_DATA: &str = r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#>.
@prefix : <http://example.org/socrates#>.
:Socrates a :Human.
:Human rdfs:subClassOf :Mortal.
{?A rdfs:subClassOf ?B. ?S a ?A} => {?S a ?B}."#;
const SOC_QUERY: &str = r#"@prefix : <http://example.org/socrates#>.
{:Socrates a ?WHAT} => {:Socrates a ?WHAT}."#;
const SOC_MORTAL_NT: &str = "<http://example.org/socrates#Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/socrates#Mortal> .";
const SOC_HUMAN_NT: &str = "<http://example.org/socrates#Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/socrates#Human> .";

/// `reasonN3New` (eye-js DERIVATIONS default) returns only the newly-derived Mortal typing,
/// not the asserted base.
#[wasm_bindgen_test]
fn reason_n3_new_is_delta() {
    let nt = Reasoner::reason_n3_new(SOC_DATA).expect("derivations delta in wasm");
    assert!(nt.contains(SOC_MORTAL_NT), "derived Mortal present: {nt}");
    assert!(!nt.contains(SOC_HUMAN_NT), "asserted Human excluded: {nt}");
}

/// `reasonN3Query` (eye-js `--query`) selects every `:Socrates a ?WHAT` over the closure —
/// both the asserted Human and the entailed Mortal.
#[wasm_bindgen_test]
fn reason_n3_query_selects_over_closure() {
    let nt = Reasoner::reason_n3_query(SOC_DATA, SOC_QUERY).expect("query filter in wasm");
    assert!(nt.contains(SOC_MORTAL_NT), "entailed Mortal selected: {nt}");
    assert!(nt.contains(SOC_HUMAN_NT), "asserted Human selected: {nt}");
}

/// [OPUS-5] sq-xqchl.1 (GH #3144) — a builtin in the query premise is EVALUATED in wasm too
/// (it used to fail closed), and it FILTERS: `:b` is 12, so only `:a` answers.
#[wasm_bindgen_test]
fn reason_n3_query_evaluates_a_premise_builtin() {
    let q = r#"@prefix math: <http://www.w3.org/2000/10/swap/math#>.
@prefix : <http://ex/>.
{ ?x :age ?n. ?n math:greaterThan 18 } => { ?x a :Adult }."#;
    let nt = Reasoner::reason_n3_query("@prefix : <http://ex/>. :a :age 21 . :b :age 12 .", q)
        .expect("builtin query filter in wasm");
    assert!(
        nt.contains("<http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Adult> ."),
        "{nt}"
    );
    assert!(!nt.contains("<http://ex/b>"), ":b is 12 — the builtin must filter it out: {nt}");
}

/// `why` (only compiled with the `explain` feature) returns a proof tree for the entailed
/// triple, and `null` for a triple that is not entailed.
#[cfg(feature = "explain")]
#[wasm_bindgen_test]
fn why_proof_tree() {
    let json = Reasoner::why(
        SOCRATES_TTL,
        "turtle",
        "rdfs",
        "<http://ex/Socrates>",
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>",
        "<http://ex/Mortal>",
    )
    .expect("why must succeed for an entailed triple");
    assert!(json.contains("\"root\":"), "proof tree JSON: {json}");
    assert!(
        json.contains("\"asserted\""),
        "proof bottoms out in asserted facts: {json}"
    );

    // A triple whose subject is absent from the data yields the JSON literal `null`.
    let absent = Reasoner::why(
        SOCRATES_TTL,
        "turtle",
        "rdfs",
        "<http://ex/NotInData>",
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>",
        "<http://ex/Mortal>",
    )
    .expect("why must not error on an absent triple");
    assert_eq!(absent, "null", "absent triple => null");
}

/// [FABLE-5] sq-ixc3.20 — `whyN3` (only compiled with the `explain` feature) returns a proof
/// tree for an N3-rule-derived triple (naming the fired rule), and `null` for a triple that is
/// not in the closure.
#[cfg(feature = "explain")]
#[wasm_bindgen_test]
fn why_n3_proof_tree() {
    let n3 = r#"@prefix ex: <http://ex/> .
        ex:a ex:knows ex:b .
        { ?x ex:knows ?y . } => { ?x ex:reaches ?y . } ."#;
    let json = Reasoner::why_n3(n3, "<http://ex/a>", "<http://ex/reaches>", "<http://ex/b>")
        .expect("whyN3 must succeed for a derived triple");
    assert!(json.contains("n3-rule-"), "derivation names the rule: {json}");
    assert!(json.contains("\"asserted\""), "leaves are asserted: {json}");

    let absent = Reasoner::why_n3(n3, "<http://ex/b>", "<http://ex/reaches>", "<http://ex/a>")
        .expect("whyN3 must not error on an absent triple");
    assert_eq!(absent, "null", "absent triple => null");
}
