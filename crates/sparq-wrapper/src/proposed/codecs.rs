//! Symmetric codecs for extended RDF literal values.
//!
//! This is a Rust port of the still-unlanded symmetric literal mappings from
//! rdfjs/wrapper issue #7 and draft PRs #90/#91. Unlike a lossy numeric cast or
//! a string-only mapping, these codecs validate the RDF datatype and retain the
//! language tag where one is required.

// [GPT-5.6] sq-1rg2q.4: exact i128 and language-tagged literal codecs.

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode};
use std::fmt;

/// An owned language-tagged string.
///
/// Both fields participate in equality so a decode/encode cycle cannot erase
/// which language a lexical form belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangString {
    /// The literal's lexical form.
    pub value: String,
    /// Its validated BCP47 language tag, in the normalized form used by RDF.
    pub language: String,
}

/// An error returned by an extended literal codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The literal has a datatype unsupported by the selected decoder.
    Datatype {
        /// The datatype required by the decoder.
        expected: &'static str,
        /// The datatype found on the literal.
        found: NamedNode,
    },
    /// An `xsd:integer` lexical form is malformed or outside `i128`.
    InvalidInteger {
        /// The rejected lexical form.
        lexical_form: String,
    },
    /// A language tag supplied to the encoder is not valid BCP47.
    InvalidLanguage {
        /// The rejected language tag.
        language: String,
    },
    /// An `rdf:langString` literal did not carry its required language tag.
    MissingLanguage,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Datatype { expected, found } => {
                write!(
                    f,
                    "expected {expected}, found datatype <{}>",
                    found.as_str()
                )
            }
            Self::InvalidInteger { lexical_form } => write!(
                f,
                "invalid or out-of-range xsd:integer lexical form {lexical_form:?}"
            ),
            Self::InvalidLanguage { language } => {
                write!(f, "invalid BCP47 language tag {language:?}")
            }
            Self::MissingLanguage => f.write_str("rdf:langString literal has no language tag"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Encodes an `i128` as an exact `xsd:integer` literal.
///
/// Decimal formatting is lossless across the full `i128` range; in particular,
/// this never narrows through `i64` or a floating-point representation.
pub fn encode_i128(value: i128) -> Literal {
    Literal::new_typed_literal(value.to_string(), xsd::INTEGER)
}

/// The whitespace XML Schema's `collapse` facet normalizes: tab, line feed,
/// carriage return, and space. Deliberately narrower than [`str::trim`], which
/// also strips Unicode whitespace the facet leaves in place.
const XSD_WHITESPACE: &[char] = &['\t', '\n', '\r', ' '];

/// Decodes an exact `xsd:integer` literal as `i128`.
///
/// `xsd:integer` fixes XML Schema's `whiteSpace` facet to `collapse`, so
/// boundary whitespace is normalized away before the lexical-to-value mapping
/// and `" 7"` decodes as `7`. Interior whitespace survives the collapse, so
/// `"+ 1"` is still rejected. This keeps the codec agreeing with the query
/// engine, which reads `" 1 "^^xsd:integer` as the value 1.
///
/// Other datatypes, malformed lexical forms, and integers outside the `i128`
/// range are returned as typed errors rather than coerced or truncated. The
/// error reports the literal's original lexical form, not the collapsed one.
pub fn decode_i128(literal: &Literal) -> Result<i128, CodecError> {
    if literal.datatype() != xsd::INTEGER {
        return Err(CodecError::Datatype {
            expected: "xsd:integer",
            found: literal.datatype().into_owned(),
        });
    }
    // `collapse` rewrites tab/LF/CR to a space, folds runs of spaces, then trims
    // both ends. For `xsd:integer` any space surviving the fold is interior and
    // fails the parse regardless, so trimming the facet's four characters off the
    // ends reaches the same decision and the same value.
    literal
        .value()
        .trim_matches(XSD_WHITESPACE)
        .parse()
        .map_err(|_| CodecError::InvalidInteger {
            lexical_form: literal.value().to_owned(),
        })
}

/// Encodes a lexical form and BCP47 language as an `rdf:langString` literal.
///
/// Invalid language tags are rejected. The returned RDF literal retains both
/// the string and the validated language instead of flattening it to
/// `xsd:string`.
pub fn encode_lang_string(value: &str, language: &str) -> Result<Literal, CodecError> {
    Literal::new_language_tagged_literal(value, language).map_err(|_| CodecError::InvalidLanguage {
        language: language.to_owned(),
    })
}

/// Decodes an `rdf:langString` without discarding its language tag.
///
/// Plain and typed strings are unsupported inputs and return a datatype error.
pub fn decode_lang_string(literal: &Literal) -> Result<LangString, CodecError> {
    if literal.datatype() != rdf::LANG_STRING {
        return Err(CodecError::Datatype {
            expected: "rdf:langString",
            found: literal.datatype().into_owned(),
        });
    }
    let language = literal.language().ok_or(CodecError::MissingLanguage)?;
    Ok(LangString {
        value: literal.value().to_owned(),
        language: language.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        decode_i128, decode_lang_string, encode_i128, encode_lang_string, CodecError, LangString,
    };
    use crate::Store;
    use oxrdf::vocab::{rdf, xsd};
    use oxrdf::{Literal, NamedNode};

    #[test]
    fn encode_i128_uses_exact_integer_lexical_form_and_datatype() {
        let value = i128::from(i64::MAX) + 1;
        let literal = encode_i128(value);

        assert_eq!(literal.value(), "9223372036854775808");
        assert_eq!(literal.datatype(), xsd::INTEGER);
        assert_eq!(literal.language(), None);
    }

    #[test]
    fn decode_i128_accepts_full_range_and_rejects_malformed_lexical_forms() {
        let maximum = Literal::new_typed_literal(i128::MAX.to_string(), xsd::INTEGER);
        assert_eq!(decode_i128(&maximum), Ok(i128::MAX));

        let malformed = Literal::new_typed_literal("12.5", xsd::INTEGER);
        assert_eq!(
            decode_i128(&malformed),
            Err(CodecError::InvalidInteger {
                lexical_form: "12.5".to_owned(),
            })
        );

        let out_of_range =
            Literal::new_typed_literal("170141183460469231731687303715884105728", xsd::INTEGER);
        assert!(matches!(
            decode_i128(&out_of_range),
            Err(CodecError::InvalidInteger { .. })
        ));
    }

    #[test]
    fn encode_lang_string_retains_lexical_form_and_validated_language() {
        let literal = encode_lang_string("Y llyfrgellydd", "cy").expect("valid language");

        assert_eq!(literal.value(), "Y llyfrgellydd");
        assert_eq!(literal.language(), Some("cy"));
        assert_eq!(literal.datatype(), rdf::LANG_STRING);
        assert!(matches!(
            encode_lang_string("value", "not_a_language"),
            Err(CodecError::InvalidLanguage { .. })
        ));
    }

    #[test]
    fn decode_lang_string_retains_language_and_rejects_plain_strings() {
        let literal =
            Literal::new_language_tagged_literal("The librarian", "en").expect("valid language");
        assert_eq!(
            decode_lang_string(&literal),
            Ok(LangString {
                value: "The librarian".to_owned(),
                language: "en".to_owned(),
            })
        );

        let plain = Literal::new_simple_literal("The librarian");
        assert!(matches!(
            decode_lang_string(&plain),
            Err(CodecError::Datatype { .. })
        ));
    }

    #[test]
    fn extended_values_round_trip_through_store_mutation_and_readback() {
        let mut store = Store::new();
        let subject = NamedNode::new("http://example.org/book").expect("valid IRI");
        let count = NamedNode::new("http://example.org/count").expect("valid IRI");
        let label = NamedNode::new("http://example.org/label").expect("valid IRI");
        let large = i128::from(i64::MAX) + 1;
        let language_value = LangString {
            value: "Y llyfrgellydd".to_owned(),
            language: "cy".to_owned(),
        };

        store
            .insert(subject.clone(), count.clone(), encode_i128(large))
            .expect("integer insert");
        store
            .insert(
                subject.clone(),
                label.clone(),
                encode_lang_string(&language_value.value, &language_value.language)
                    .expect("language encode"),
            )
            .expect("language insert");

        let integer = store
            .node(subject.clone())
            .out(&count)
            .next()
            .expect("integer readback")
            .into_term();
        let language = store
            .node(subject)
            .out(&label)
            .next()
            .expect("language readback")
            .into_term();
        let oxrdf::Term::Literal(integer) = integer else {
            panic!("integer readback is a literal");
        };
        let oxrdf::Term::Literal(language) = language else {
            panic!("language readback is a literal");
        };

        assert_eq!(decode_i128(&integer), Ok(large));
        assert_eq!(integer.value(), large.to_string());
        assert_eq!(integer.datatype(), xsd::INTEGER);
        assert_eq!(decode_lang_string(&language), Ok(language_value));
        assert_eq!(language.datatype(), rdf::LANG_STRING);
    }
}
