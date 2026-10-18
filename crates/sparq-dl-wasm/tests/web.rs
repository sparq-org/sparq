// Headless wasm test of the JS-facing export. The crate's native tests exercise
// `check_owl_dl_json`; `JsError::new` is a wasm-bindgen import that panics off-wasm, so the
// `#[wasm_bindgen]` wrapper itself is only reachable here, in a real wasm32 runtime
// (`wasm-pack test --node`).

#![cfg(target_arch = "wasm32")]

use sparq_dl_wasm::check_owl_dl;
use wasm_bindgen_test::*;

const TBOX: &str = r#"
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:A a owl:Class . :B a owl:Class . :C a owl:Class .
:A rdfs:subClassOf :B . :B rdfs:subClassOf :C . :B owl:disjointWith :D . :D a owl:Class .
"#;

#[wasm_bindgen_test]
fn check_owl_dl_returns_the_json_result() {
    let out = check_owl_dl(TBOX, None, "turtle", None).expect("checked");
    assert!(
        out.starts_with(r#"{"consistent":true,"reason":null,"#),
        "{out}"
    );
    assert!(
        out.contains(r#""inferredSubClassOf":[["http://ex/A","http://ex/C"]]"#),
        "{out}"
    );
    assert!(out.ends_with(r#""classification":"complete"}"#), "{out}");
}

#[wasm_bindgen_test]
fn check_owl_dl_merges_the_abox() {
    let abox = "<http://ex/x> a <http://ex/A> , <http://ex/D> .";
    let out = check_owl_dl(TBOX, Some(abox.into()), "turtle", None).expect("checked");
    assert!(out.starts_with(r#"{"consistent":false,"#), "{out}");
}

#[wasm_bindgen_test]
fn check_owl_dl_throws_on_parse_errors_and_bad_options() {
    assert!(check_owl_dl("<a> <b> .", None, "ntriples", None).is_err());
    assert!(check_owl_dl(
        TBOX,
        None,
        "turtle",
        Some(r#"{"budget":{"steps":1}}"#.into())
    )
    .is_err());
}
