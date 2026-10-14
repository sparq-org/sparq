//! sq-tonhr.6 — write direction: parse → print → re-parse is
//! graph-isomorphic to BOTH the original parse and the independent oxttl
//! `.ttl` oracle, on the whole corpus, in every applicable profile; and
//! non-expressible graphs return the typed residual verdict (all-or-nothing
//! printing, never a lossy document).

mod common;

use common::{dump, fixture_names, is_isomorphic, read_fixture, ttl_oracle};
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_shaclc::{parse, write, Profile, DEFAULT_BASE};

#[test]
fn write_round_trips_the_whole_corpus_in_both_profiles() {
    for (sub, profiles) in [
        ("valid", &[Profile::Strict, Profile::Extended][..]),
        ("rdf12", &[Profile::Strict, Profile::Extended][..]),
        ("extended", &[Profile::Extended][..]),
    ] {
        for name in fixture_names(sub) {
            let doc = read_fixture(sub, &name, "shaclc");
            let expected = ttl_oracle(sub, &name);
            for &profile in profiles {
                let (triples, outcome) = parse(&doc, DEFAULT_BASE, profile)
                    .unwrap_or_else(|e| panic!("{sub}/{name} ({profile:?}): parse: {e}"));
                let text = write(&triples, Some(DEFAULT_BASE), &outcome.prefixes, profile)
                    .unwrap_or_else(|e| panic!("{sub}/{name} ({profile:?}): write refused: {e}"));
                let (re, _) = parse(&text, DEFAULT_BASE, profile).unwrap_or_else(|e| {
                    panic!("{sub}/{name} ({profile:?}): reparse of printed doc: {e}\n--- printed ---\n{text}")
                });
                assert!(
                    is_isomorphic(&re, &triples),
                    "{sub}/{name} ({profile:?}): print round-trip drifted\n--- printed ---\n{text}\n--- reparsed ---\n{}\n--- original ---\n{}",
                    dump(&re),
                    dump(&triples)
                );
                assert!(
                    is_isomorphic(&re, &expected),
                    "{sub}/{name} ({profile:?}): round-trip differs from the .ttl oracle\n--- printed ---\n{text}"
                );
            }
        }
    }
}

fn base_fixture() -> (Vec<Triple>, Vec<(String, String)>) {
    let doc = read_fixture("valid", "basic-shape-with-target", "shaclc");
    let (t, o) = parse(&doc, DEFAULT_BASE, Profile::Strict).expect("parse");
    (t, o.prefixes)
}

#[test]
fn dangling_blank_object_is_a_residual_verdict_in_both_profiles() {
    let (mut triples, prefixes) = base_fixture();
    triples.push(Triple {
        subject: NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://example.org/test#TestShape")),
        predicate: NamedNode::new_unchecked("http://example.org/unprintable#p"),
        object: Term::BlankNode(BlankNode::new_unchecked("z9")),
    });
    for profile in [Profile::Strict, Profile::Extended] {
        let err = write(&triples, Some(DEFAULT_BASE), &prefixes, profile)
            .expect_err("dangling blank must refuse");
        assert!(!err.missing_ontology);
        assert_eq!(err.residual.len(), 1, "{profile:?}: exactly the foreign triple: {err}");
        assert_eq!(err.residual[0].predicate.as_str(), "http://example.org/unprintable#p");
        assert!(err.to_string().contains("not compact-expressible"), "verdict text: {err}");
    }
}

#[test]
fn foreign_iri_predicate_residualizes_in_strict_but_extended_absorbs_it() {
    // partiality as a feature: the extension layers are the declared
    // guard-free fallbacks — strict has no such alternatives, so the same
    // graph residualizes by construction.
    let (mut triples, prefixes) = base_fixture();
    triples.push(Triple {
        subject: NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://example.org/test#TestShape")),
        predicate: NamedNode::new_unchecked("http://example.org/meta#comment"),
        object: Term::NamedNode(NamedNode::new_unchecked("http://example.org/meta#note1")),
    });
    let err = write(&triples, Some(DEFAULT_BASE), &prefixes, Profile::Strict)
        .expect_err("strict must residualize the annotation");
    assert_eq!(err.residual.len(), 1);
    let text = write(&triples, Some(DEFAULT_BASE), &prefixes, Profile::Extended)
        .expect("extended absorbs via the annotation layer");
    let (re, _) = parse(&text, DEFAULT_BASE, Profile::Extended).expect("reparse");
    assert!(is_isomorphic(&re, &triples), "absorbed annotation round-trips\n{text}");
}

#[test]
fn missing_ontology_pattern_is_the_missing_verdict() {
    let (triples, prefixes) = base_fixture();
    let no_onto: Vec<Triple> = triples
        .iter()
        .filter(|t| !matches!(&t.object, Term::NamedNode(n) if n.as_str().ends_with("Ontology")))
        .cloned()
        .collect();
    assert_eq!(no_onto.len(), triples.len() - 1);
    let err = write(&no_onto, Some(DEFAULT_BASE), &prefixes, Profile::Strict)
        .expect_err("no ontology pattern -> no faithful print");
    assert!(err.missing_ontology, "missing verdict: {err}");
}
