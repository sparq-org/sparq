//! Runtime document scopes are part of exported blank labels.
use sparq_reason::n3::{reason_n3_terms, Term};

#[test]
fn runtime_doc_labels_are_exact_across_repeated_and_distinct_texts() {
    let src = r#"
        @prefix : <http://ex/>.
        @prefix log: <http://www.w3.org/2000/10/swap/log#>.
        :a :text "_:x <http://ex/p> <http://ex/o> .".
        :b :text "_:x <http://ex/p> <http://ex/o> .".
        :c :text "_:x <http://ex/p> <http://ex/other> .".
        { ?d :text ?t. ?t log:parsedAsN3 ?f. ?f log:includes { ?b :p ?o } }
            => { ?d :blank ?b }.
    "#;
    let iri = |s: &str| Term::Iri(format!("http://ex/{s}"));
    // Independent SHA-256 vectors for the specified domain + lengths + text + base.
    let expected = [
        [
            iri("a"),
            iri("blank"),
            Term::Blank("__rp0_36e2d3c73471b49f_x".into()),
        ],
        [
            iri("b"),
            iri("blank"),
            Term::Blank("__rp0_36e2d3c73471b49f_x".into()),
        ],
        [
            iri("c"),
            iri("blank"),
            Term::Blank("__rp0_c457d264bfa52888_x".into()),
        ],
    ];
    for _ in 0..2 {
        let closure = reason_n3_terms(src, None).expect("closure");
        assert_eq!(closure.derived.len(), expected.len());
        for fact in &expected {
            assert!(
                closure.derived.contains(fact),
                "missing {fact:?}: {:?}",
                closure.derived
            );
        }
    }
}
