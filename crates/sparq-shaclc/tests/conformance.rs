//! sq-tonhr.6 — the vendored shaclc conformance corpus against an
//! INDEPENDENT oracle: every `.shaclc` fixture parsed by the generated
//! parsers must be graph-isomorphic to its `.ttl` side parsed by oxttl.
//!
//! Also locks the profile split (strict REJECTS every extended fixture —
//! the shaclc-js enforcement-leak fix), the negative cases, and one-shot vs
//! chunked-push agreement of the raw modules.

mod common;

use common::{dump, fixture_names, is_isomorphic, read_fixture, ttl_oracle, BASE};
use sparq_shaclc::{parse, parse_extended, parse_strict, Profile, DEFAULT_BASE};

#[test]
fn corpus_shape_is_the_vendored_one() {
    assert!(fixture_names("valid").len() >= 44);
    assert!(fixture_names("rdf12").len() >= 8);
    assert!(fixture_names("extended").len() >= 14);
    assert!(fixture_names("negative").len() >= 6);
    assert_eq!(BASE, DEFAULT_BASE);
}

#[test]
fn valid_and_rdf12_match_the_oxttl_oracle_in_both_profiles() {
    for sub in ["valid", "rdf12"] {
        for name in fixture_names(sub) {
            let doc = read_fixture(sub, &name, "shaclc");
            let expected = ttl_oracle(sub, &name);
            for profile in [Profile::Strict, Profile::Extended] {
                let (got, _) = parse(&doc, DEFAULT_BASE, profile)
                    .unwrap_or_else(|e| panic!("{sub}/{name} ({profile:?}): {e}"));
                assert!(
                    is_isomorphic(&got, &expected),
                    "{sub}/{name} ({profile:?}) differs from the .ttl oracle\n--- got ---\n{}\n--- expected ---\n{}",
                    dump(&got),
                    dump(&expected)
                );
            }
        }
    }
}

#[test]
fn extended_fixtures_match_the_oracle_and_strict_rejects_them_all() {
    for name in fixture_names("extended") {
        let doc = read_fixture("extended", &name, "shaclc");
        let expected = ttl_oracle("extended", &name);
        let (got, _) = parse_extended(&doc, DEFAULT_BASE)
            .unwrap_or_else(|e| panic!("extended/{name}: {e}"));
        assert!(
            is_isomorphic(&got, &expected),
            "extended/{name} differs from the .ttl oracle\n--- got ---\n{}\n--- expected ---\n{}",
            dump(&got),
            dump(&expected)
        );
        // The enforcement-leak fix: the strict parser's tables simply do not
        // contain the extension alternatives.
        let err = parse_strict(&doc, DEFAULT_BASE)
            .expect_err(&format!("extended/{name}: STRICT accepted an extended fixture"));
        assert!(err.code.is_some(), "extended/{name}: stable reject code expected");
    }
}

#[test]
fn negative_fixtures_are_rejected_by_both_profiles() {
    for name in fixture_names("negative") {
        let doc = read_fixture("negative", &name, "shaclc");
        for profile in [Profile::Strict, Profile::Extended] {
            assert!(
                parse(&doc, DEFAULT_BASE, profile).is_err(),
                "negative/{name} accepted by {profile:?}"
            );
        }
    }
}

/// One-shot and chunked push parsing agree on the raw extended module for the
/// whole corpus (the whole-buffer fallback driver for a document-shaped
/// start production must be chunking-invariant).
#[test]
fn raw_push_parser_agrees_with_one_shot_on_the_whole_corpus() {
    use sparq_shaclc::raw::shaclc12ext as m;
    for sub in ["valid", "rdf12", "extended"] {
        for name in fixture_names(sub) {
            let doc = read_fixture(sub, &name, "shaclc");
            let mut one: Vec<m::Triple> = Vec::new();
            m::parse(&doc, Some(BASE), |t| one.push(t))
                .unwrap_or_else(|e| panic!("{sub}/{name}: one-shot: {e}"));
            let mut pushed: Vec<m::Triple> = Vec::new();
            {
                let mut p = m::PushParser::new(Some(BASE), |t| pushed.push(t));
                let mut i = 0;
                while i < doc.len() {
                    let mut j = (i + 7).min(doc.len());
                    while j < doc.len() && !doc.is_char_boundary(j) {
                        j += 1;
                    }
                    p.push(&doc[i..j]).unwrap_or_else(|e| panic!("{sub}/{name}: push: {e}"));
                    i = j;
                }
                p.end().unwrap_or_else(|e| panic!("{sub}/{name}: end: {e}"));
            }
            let fmt = |v: &[m::Triple]| {
                let mut ls: Vec<String> = v.iter().map(|t| format!("{t:?}")).collect();
                ls.sort();
                ls.join("\n")
            };
            assert_eq!(fmt(&one), fmt(&pushed), "{sub}/{name}: push/one-shot divergence");
        }
    }
}
