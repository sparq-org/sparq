// [OPUS-5] sq-1rg2q.4: public-surface witnesses for the symmetric extended literal
// codecs. The in-module unit tests pin each function; this file pins the property the
// proposal actually promises across the crate boundary — every codec is bidirectional
// and preserves lexical form, datatype, and language, and unsupported or out-of-range
// values are errors rather than lossy casts.

#![cfg(feature = "proposed-codecs")]

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, Term};
use sparq_wrapper::proposed::codecs::{
    decode_i128, decode_lang_string, encode_i128, encode_lang_string, CodecError, LangString,
};
use sparq_wrapper::{AccessError, Store};

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{}", local)).expect("valid IRI")
}

/// Writes one triple and reads its object back out of the graph as a literal, so every
/// assertion below observes a term that made a real round trip through the store rather
/// than the value the test just constructed.
fn store_round_trip(object: Literal) -> Literal {
    let mut store = Store::new();
    let subject = iri("subject");
    let predicate = iri("predicate");

    store
        .insert(subject.clone(), predicate.clone(), object)
        .expect("insert");

    let term = store
        .node(subject)
        .out(&predicate)
        .next()
        .expect("the inserted triple is readable")
        .into_term();
    let Term::Literal(literal) = term else {
        panic!("the readback term is a literal");
    };
    literal
}

#[test]
fn integer_above_i64_max_survives_mutation_and_readback_exactly() {
    let value = i128::from(i64::MAX) + 1;
    let encoded = encode_i128(value);
    let readback = store_round_trip(encoded.clone());

    // Equality on `Literal` compares lexical form, datatype, and language together, so
    // this single assertion is the "preserves all three" half of the invariant.
    assert_eq!(readback, encoded);
    assert_eq!(readback.value(), "9223372036854775808");
    assert_eq!(readback.datatype(), xsd::INTEGER);
    assert_eq!(readback.language(), None);
    assert_eq!(decode_i128(&readback), Ok(value));
}

#[test]
fn language_tagged_string_survives_mutation_and_readback_exactly() {
    let encoded = encode_lang_string("Y llyfrgellydd", "cy").expect("valid language tag");
    let readback = store_round_trip(encoded.clone());

    assert_eq!(readback, encoded);
    assert_eq!(readback.value(), "Y llyfrgellydd");
    assert_eq!(readback.language(), Some("cy"));
    assert_eq!(readback.datatype(), rdf::LANG_STRING);
    assert_eq!(
        decode_lang_string(&readback),
        Ok(LangString {
            value: "Y llyfrgellydd".to_owned(),
            language: "cy".to_owned(),
        })
    );
}

#[test]
fn malformed_lexical_forms_fail_instead_of_reading_back_a_substituted_value() {
    // A malformed lexical form is still a well-formed RDF literal, so the store accepts
    // it. The decoder is what must refuse it, and it must refuse it after a real
    // readback rather than only on a freshly built term. `"+ 1"` is the interior
    // whitespace witness: unlike the boundary padding collapsed by the whiteSpace facet
    // (see `boundary_whitespace_is_collapsed_before_the_lexical_to_value_mapping`), a
    // space between the sign and the digits survives the collapse and is not an integer.
    for lexical_form in ["12.5", "", "0x10", "1e3", "seven", "+ 1"] {
        let readback = store_round_trip(Literal::new_typed_literal(lexical_form, xsd::INTEGER));
        assert_eq!(
            decode_i128(&readback),
            Err(CodecError::InvalidInteger {
                lexical_form: lexical_form.to_owned(),
            }),
            "{:?} must be rejected, not coerced",
            lexical_form
        );
    }

    assert!(matches!(
        encode_lang_string("value", "not a language"),
        Err(CodecError::InvalidLanguage { .. })
    ));
    assert!(matches!(
        encode_lang_string("value", ""),
        Err(CodecError::InvalidLanguage { .. })
    ));
}

#[test]
fn boundary_whitespace_is_collapsed_before_the_lexical_to_value_mapping() {
    // xsd:integer fixes XML Schema's whiteSpace facet to `collapse`, so a lexical form
    // padded with spaces, tabs, or newlines is still in the lexical space and maps to the
    // ordinary value. Rejecting these would put the codec at odds with the query engine,
    // which reads `" 1 "^^xsd:integer` as the value 1.
    for (lexical_form, value) in [
        (" 7", 7),
        ("7 ", 7),
        ("  7  ", 7),
        ("\t7\n", 7),
        ("\r\n7\t", 7),
        (" +7 ", 7),
        (" -7 ", -7),
    ] {
        let readback = store_round_trip(Literal::new_typed_literal(lexical_form, xsd::INTEGER));
        // The store keeps the padded lexical form verbatim -- the collapse belongs to the
        // lexical-to-value mapping, not to the term -- so this really is the padded input
        // reaching the decoder.
        assert_eq!(readback.value(), lexical_form);
        assert_eq!(
            decode_i128(&readback),
            Ok(value),
            "{:?} is a whitespace-collapsed xsd:integer",
            lexical_form
        );
    }
}

#[test]
fn decode_after_encode_is_the_identity_across_the_full_i128_range() {
    let values = [
        i128::MIN,
        i128::MIN + 1,
        i128::from(i64::MIN) - 1,
        i128::from(i64::MIN),
        -1,
        0,
        1,
        i128::from(i64::MAX),
        i128::from(i64::MAX) + 1,
        i128::from(u64::MAX),
        i128::from(u64::MAX) + 1,
        i128::MAX - 1,
        i128::MAX,
    ];

    for value in values {
        let readback = store_round_trip(encode_i128(value));
        assert_eq!(decode_i128(&readback), Ok(value), "round trip of {}", value);
        // Re-encoding the decoded value must reproduce the identical literal; without
        // this the codec could round trip the number while drifting its lexical form.
        assert_eq!(encode_i128(decode_i128(&readback).expect("decoded")), readback);
    }
}

#[test]
fn decode_after_encode_is_the_identity_for_language_tagged_strings() {
    // The second element of each pair is the tag RDF normalizes to. A codec that
    // preserved the input casing instead would not be symmetric with the store.
    let cases = [
        ("The librarian", "en", "en"),
        ("Le bibliothécaire", "fr", "fr"),
        ("Der Bibliothekar", "de-AT", "de-at"),
        ("The librarian", "EN", "en"),
        ("", "en", "en"),
        ("  leading and trailing  ", "en", "en"),
        ("emoji 📚 and \u{200b}zero width", "en", "en"),
    ];

    for (value, language, normalized) in cases {
        let encoded = encode_lang_string(value, language).expect("valid language tag");
        let readback = store_round_trip(encoded.clone());
        let decoded = decode_lang_string(&readback).expect("decodes");

        assert_eq!(
            decoded,
            LangString {
                value: value.to_owned(),
                language: normalized.to_owned(),
            },
            "round trip of {:?}@{}",
            value,
            language
        );
        assert_eq!(
            encode_lang_string(&decoded.value, &decoded.language).expect("re-encodes"),
            readback,
            "re-encoding {:?}@{} must be a fixed point",
            value,
            language
        );
    }
}

#[test]
fn out_of_range_integers_are_errors_rather_than_saturating_or_wrapping() {
    // One past each end of i128. A saturating or wrapping decoder would return
    // i128::MAX / i128::MIN here instead of failing.
    let above = "170141183460469231731687303715884105728";
    let below = "-170141183460469231731687303715884105729";

    for lexical_form in [above, below] {
        let readback = store_round_trip(Literal::new_typed_literal(lexical_form, xsd::INTEGER));
        assert_eq!(
            decode_i128(&readback),
            Err(CodecError::InvalidInteger {
                lexical_form: lexical_form.to_owned(),
            })
        );
    }
}

#[test]
fn unsupported_datatypes_are_datatype_errors_rather_than_reinterpreted() {
    // xsd:long carries an integer lexical form the i128 decoder could happily parse.
    // Refusing it is the point: the decoder validates the datatype, it does not sniff
    // the string.
    let long = store_round_trip(Literal::new_typed_literal("42", xsd::LONG));
    assert_eq!(
        decode_i128(&long),
        Err(CodecError::Datatype {
            expected: "xsd:integer",
            found: xsd::LONG.into_owned(),
        })
    );

    // Likewise a language-tagged literal is not an integer, even though "42"@en parses.
    let tagged = store_round_trip(encode_lang_string("42", "en").expect("valid language tag"));
    assert_eq!(
        decode_i128(&tagged),
        Err(CodecError::Datatype {
            expected: "xsd:integer",
            found: rdf::LANG_STRING.into_owned(),
        })
    );

    // A plain string and an xsd:string are both untagged, so neither decodes as a
    // language-tagged string; flattening either one would silently lose the language.
    for plain in [
        Literal::new_simple_literal("The librarian"),
        Literal::new_typed_literal("The librarian", xsd::STRING),
    ] {
        let readback = store_round_trip(plain);
        assert_eq!(
            decode_lang_string(&readback),
            Err(CodecError::Datatype {
                expected: "rdf:langString",
                found: xsd::STRING.into_owned(),
            })
        );
    }

    let integer = store_round_trip(encode_i128(7));
    assert_eq!(
        decode_lang_string(&integer),
        Err(CodecError::Datatype {
            expected: "rdf:langString",
            found: xsd::INTEGER.into_owned(),
        })
    );
}

#[test]
fn the_codec_reaches_values_the_built_in_i64_accessor_must_reject() {
    // This is why the extended codec exists. The same literal that `as_i64` has to fail
    // on -- it does not fit i64 -- decodes exactly through `decode_i128`.
    let value = i128::from(i64::MAX) + 1;
    let mut store = Store::new();
    let subject = iri("subject");
    let predicate = iri("count");
    store
        .insert(subject.clone(), predicate.clone(), encode_i128(value))
        .expect("insert");

    let node = store
        .node(subject)
        .out(&predicate)
        .next()
        .expect("readback");
    assert!(matches!(
        node.as_i64(),
        Err(AccessError::InvalidLexical { .. })
    ));

    let Term::Literal(literal) = node.into_term() else {
        panic!("the readback term is a literal");
    };
    assert_eq!(decode_i128(&literal), Ok(value));
}

#[test]
fn codec_errors_are_public_std_errors_that_name_what_was_rejected() {
    fn as_std_error(error: CodecError) -> Box<dyn std::error::Error> {
        Box::new(error)
    }

    let datatype = decode_i128(&Literal::new_simple_literal("42")).expect_err("wrong datatype");
    let rendered = as_std_error(datatype).to_string();
    assert!(rendered.contains("xsd:integer"), "{}", rendered);
    assert!(
        rendered.contains("http://www.w3.org/2001/XMLSchema#string"),
        "{}",
        rendered
    );

    let malformed = decode_i128(&Literal::new_typed_literal("12.5", xsd::INTEGER))
        .expect_err("malformed lexical form");
    let rendered = as_std_error(malformed).to_string();
    assert!(rendered.contains("12.5"), "{}", rendered);

    let language = encode_lang_string("value", "not a language").expect_err("invalid language");
    let rendered = as_std_error(language).to_string();
    assert!(rendered.contains("not a language"), "{}", rendered);

    let rendered = as_std_error(CodecError::MissingLanguage).to_string();
    assert!(rendered.contains("language"), "{}", rendered);
}
