//! sq-jwdwi — raw push-parser stress at token and UTF-8 boundaries.

const BASE: &str = "http://example.org/push-boundaries/";

const STRICT_DOCUMENT: &str = include_str!("fixtures/valid/complex1.shaclc");
const RDF12_DOCUMENT: &str = include_str!("fixtures/rdf12/reifier-shape.shaclc");
const TRIPLE_TERM_DOCUMENT: &str = include_str!("fixtures/rdf12/tripleterm-in-array.shaclc");
const EXTENDED_DOCUMENT: &str = include_str!("fixtures/extended/propertyEscape.shaclc");

// Keep literal non-ASCII scalars in this source: escaped Unicode would only
// stress ASCII token boundaries instead of the PushParser's UTF-8 boundaries.
const UNICODE_DIRECTIONAL_DOCUMENT: &str = r#"
PREFIX ex: <http://example.org/unicode#>

shape ex:UnicodeShape {
    message="مرحبا 世界 🦀"@ar--rtl .
    ex:greeting hasValue="שלום"@he--rtl .
}
"#;

const INCOMPLETE_DOCUMENT: &str = "PREFIX ex:";

const STRICT_CASES: &[(&str, &str)] = &[
    ("strict", STRICT_DOCUMENT),
    ("rdf12", RDF12_DOCUMENT),
    ("unicode-directional", UNICODE_DIRECTIONAL_DOCUMENT),
    ("triple-term", TRIPLE_TERM_DOCUMENT),
];

const EXTENDED_CASES: &[(&str, &str)] = &[
    ("strict", STRICT_DOCUMENT),
    ("rdf12", RDF12_DOCUMENT),
    ("unicode-directional", UNICODE_DIRECTIONAL_DOCUMENT),
    ("triple-term", TRIPLE_TERM_DOCUMENT),
    ("extended", EXTENDED_DOCUMENT),
];

// The generated raw modules deliberately expose the same API with distinct
// concrete term types. Generate identical tests so neither profile can drift.
macro_rules! push_parser_suite {
    ($suite:ident, $raw:path, $cases:ident) => {
        mod $suite {
            use $raw as raw;

            type Parsed = (Vec<raw::Triple>, raw::ParseOutcome);

            fn one_shot(case: &str, document: &str) -> Parsed {
                let mut triples = Vec::new();
                let outcome = raw::parse(document, Some(super::BASE), |triple| {
                    triples.push(triple);
                })
                .unwrap_or_else(|error| panic!("{case}: one-shot parse failed: {error}"));
                assert!(
                    !triples.is_empty(),
                    "{case}: representative emitted no triples"
                );
                (triples, outcome)
            }

            fn pushed<'a>(
                case: &str,
                chunking: &str,
                chunks: impl IntoIterator<Item = &'a str>,
            ) -> Parsed {
                let mut triples = Vec::new();
                let outcome = {
                    let mut parser = raw::PushParser::new(Some(super::BASE), |triple| {
                        triples.push(triple);
                    });
                    for chunk in chunks {
                        parser.push(chunk).unwrap_or_else(|error| {
                            panic!("{case}: {chunking} push failed: {error}")
                        });
                    }
                    parser
                        .end()
                        .unwrap_or_else(|error| panic!("{case}: {chunking} end failed: {error}"))
                };
                (triples, outcome)
            }

            fn assert_same(case: &str, chunking: &str, expected: &Parsed, actual: &Parsed) {
                assert_eq!(
                    actual.0, expected.0,
                    "{case}: {chunking} triple output diverged"
                );
                assert_eq!(
                    actual.1.prefixes, expected.1.prefixes,
                    "{case}: {chunking} prefix outcome diverged"
                );
                assert_eq!(
                    actual.1.base, expected.1.base,
                    "{case}: {chunking} base outcome diverged"
                );
            }

            fn assert_every_character_split(case: &str, document: &str, expected: &Parsed) {
                for split in 0..=document.len() {
                    if !document.is_char_boundary(split) {
                        continue;
                    }
                    let chunking = format!("character-boundary split at byte {split}");
                    let actual = pushed(case, &chunking, [&document[..split], &document[split..]]);
                    assert_same(case, &chunking, expected, &actual);
                }
            }

            fn assert_one_byte_chunks(case: &str, document: &str, expected: &Parsed) {
                assert!(document.is_ascii(), "{case}: one-byte chunks require ASCII");
                let actual = pushed(
                    case,
                    "one-byte chunks",
                    (0..document.len()).map(|offset| &document[offset..offset + 1]),
                );
                assert_same(case, "one-byte chunks", expected, &actual);
            }

            fn assert_one_scalar_chunks(case: &str, document: &str, expected: &Parsed) {
                assert!(
                    !document.is_ascii(),
                    "{case}: UTF-8 stress document is ASCII"
                );
                let actual = pushed(
                    case,
                    "one-Unicode-scalar chunks",
                    document
                        .char_indices()
                        .map(|(offset, scalar)| &document[offset..offset + scalar.len_utf8()]),
                );
                assert_same(case, "one-Unicode-scalar chunks", expected, &actual);
            }

            fn incomplete_error<'a>(
                chunking: &str,
                chunks: impl IntoIterator<Item = &'a str>,
            ) -> (String, usize, usize, Option<&'static str>) {
                let mut triples = Vec::new();
                let error = {
                    let mut parser = raw::PushParser::new(Some(super::BASE), |triple| {
                        triples.push(triple);
                    });
                    for chunk in chunks {
                        parser.push(chunk).unwrap_or_else(|error| {
                            panic!("incomplete: {chunking} push failed before end: {error}")
                        });
                    }
                    parser
                        .end()
                        .expect_err("incomplete final input was accepted")
                };
                assert!(
                    triples.is_empty(),
                    "incomplete: {chunking} emitted partial triples: {triples:?}"
                );
                (error.message, error.line, error.column, error.code)
            }

            #[test]
            fn output_and_outcome_match_at_every_valid_boundary() {
                for &(case, document) in super::$cases {
                    let expected = one_shot(case, document);
                    assert_every_character_split(case, document, &expected);
                    if document.is_ascii() {
                        assert_one_byte_chunks(case, document, &expected);
                    } else {
                        assert_one_scalar_chunks(case, document, &expected);
                    }
                }
            }

            #[test]
            fn incomplete_final_input_has_stable_error_without_partial_success() {
                let expected = (
                    "expected IRIREF but got <eof> at line 1:11".to_owned(),
                    1,
                    11,
                    Some("UNEXPECTED_TOKEN"),
                );
                assert_eq!(
                    incomplete_error("single chunk", [super::INCOMPLETE_DOCUMENT]),
                    expected
                );
                assert_eq!(
                    incomplete_error(
                        "one-byte chunks",
                        (0..super::INCOMPLETE_DOCUMENT.len())
                            .map(|offset| { &super::INCOMPLETE_DOCUMENT[offset..offset + 1] }),
                    ),
                    expected
                );
            }
        }
    };
}

push_parser_suite!(strict, sparq_shaclc::raw::shaclc12, STRICT_CASES);
push_parser_suite!(extended, sparq_shaclc::raw::shaclc12ext, EXTENDED_CASES);
