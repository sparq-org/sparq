//! The W3C JSON-LD 1.1 error-code registry.
//!
//! (sq-oy1f.23) Every native pipeline algorithm is fallible and returns
//! [`JsonLdError`], which carries a [`JsonLdErrorCode`] whose string form is the **exact**
//! spec code (e.g. `"invalid @context"`, `"keyword redefinition"`, `"loading document
//! failed"`). Modelling the codes as a closed enum is what makes the W3C
//! `NegativeEvaluationTest`s assertable: the conformance harness compares the raised code
//! against the manifest's `expectErrorCode`, so a *wrong* code is a failure, not a pass
//! (honesty over score — design record §4).
//!
//! The codes are drawn from the JSON-LD 1.1 API error registry
//! (<https://www.w3.org/TR/json-ld11-api/#jsonlderrorcode>) plus the two framing codes
//! from JSON-LD 1.1 Framing (<https://www.w3.org/TR/json-ld11-framing/>).

use std::fmt;

/// A JSON-LD 1.1 error code. The [`JsonLdErrorCode::as_str`] form is the exact string the
/// specification defines, so it can be compared directly against a suite manifest's
/// `expectErrorCode`.
// Deliberately a *closed* enum (no `#[non_exhaustive]`): the JSON-LD 1.1 error
// registry is a fixed, spec-defined set, and closing it lets the conformance harness match
// `expectErrorCode` exhaustively so an unhandled or renamed spec code is a compile-time
// failure rather than a silent wrong-code pass (design record §4). A spec erratum adding a
// code is intentionally a breaking change here — it must be handled everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JsonLdErrorCode {
    /// `colliding keywords`
    CollidingKeywords,
    /// `conflicting indexes`
    ConflictingIndexes,
    /// `context overflow`
    ContextOverflow,
    /// `cyclic IRI mapping`
    CyclicIriMapping,
    /// `invalid @id value`
    InvalidIdValue,
    /// `invalid @import value`
    InvalidImportValue,
    /// `invalid @included value`
    InvalidIncludedValue,
    /// `invalid @index value`
    InvalidIndexValue,
    /// `invalid @nest value`
    InvalidNestValue,
    /// `invalid @prefix value`
    InvalidPrefixValue,
    /// `invalid @propagate value`
    InvalidPropagateValue,
    /// `invalid @protected value`
    InvalidProtectedValue,
    /// `invalid @reverse value`
    InvalidReverseValue,
    /// `invalid @version value`
    InvalidVersionValue,
    /// `invalid base direction`
    InvalidBaseDirection,
    /// `invalid base IRI`
    InvalidBaseIri,
    /// `invalid container mapping`
    InvalidContainerMapping,
    /// `invalid context entry`
    InvalidContextEntry,
    /// `invalid context nullification`
    InvalidContextNullification,
    /// `invalid default language`
    InvalidDefaultLanguage,
    /// `invalid IRI mapping`
    InvalidIriMapping,
    /// `invalid JSON literal`
    InvalidJsonLiteral,
    /// `invalid keyword alias`
    InvalidKeywordAlias,
    /// `invalid language map value`
    InvalidLanguageMapValue,
    /// `invalid language mapping`
    InvalidLanguageMapping,
    /// `invalid language-tagged string`
    InvalidLanguageTaggedString,
    /// `invalid language-tagged value`
    InvalidLanguageTaggedValue,
    /// `invalid local context`
    InvalidLocalContext,
    /// `invalid remote context`
    InvalidRemoteContext,
    /// `invalid reverse property`
    InvalidReverseProperty,
    /// `invalid reverse property map`
    InvalidReversePropertyMap,
    /// `invalid reverse property value`
    InvalidReversePropertyValue,
    /// `invalid scoped context`
    InvalidScopedContext,
    /// `invalid script element`
    InvalidScriptElement,
    /// `invalid set or list object`
    InvalidSetOrListObject,
    /// `invalid term definition`
    InvalidTermDefinition,
    /// `invalid type mapping`
    InvalidTypeMapping,
    /// `invalid type value`
    InvalidTypeValue,
    /// `invalid typed value`
    InvalidTypedValue,
    /// `invalid value object`
    InvalidValueObject,
    /// `invalid value object value`
    InvalidValueObjectValue,
    /// `invalid vocab mapping`
    InvalidVocabMapping,
    /// `IRI confused with prefix`
    IriConfusedWithPrefix,
    /// `keyword redefinition`
    KeywordRedefinition,
    /// `loading document failed`
    LoadingDocumentFailed,
    /// `loading remote context failed`
    LoadingRemoteContextFailed,
    /// `multiple context link headers`
    MultipleContextLinkHeaders,
    /// `processing mode conflict`
    ProcessingModeConflict,
    /// `protected term redefinition`
    ProtectedTermRedefinition,
    // --- JSON-LD 1.1 Framing (json-ld11-framing) ---
    /// `invalid frame` (framing)
    InvalidFrame,
    /// `invalid @embed value` (framing)
    InvalidEmbedValue,
}

impl JsonLdErrorCode {
    /// The exact spec string for this code (e.g. `"loading document failed"`).
    pub fn as_str(self) -> &'static str {
        match self {
            JsonLdErrorCode::CollidingKeywords => "colliding keywords",
            JsonLdErrorCode::ConflictingIndexes => "conflicting indexes",
            JsonLdErrorCode::ContextOverflow => "context overflow",
            JsonLdErrorCode::CyclicIriMapping => "cyclic IRI mapping",
            JsonLdErrorCode::InvalidIdValue => "invalid @id value",
            JsonLdErrorCode::InvalidImportValue => "invalid @import value",
            JsonLdErrorCode::InvalidIncludedValue => "invalid @included value",
            JsonLdErrorCode::InvalidIndexValue => "invalid @index value",
            JsonLdErrorCode::InvalidNestValue => "invalid @nest value",
            JsonLdErrorCode::InvalidPrefixValue => "invalid @prefix value",
            JsonLdErrorCode::InvalidPropagateValue => "invalid @propagate value",
            JsonLdErrorCode::InvalidProtectedValue => "invalid @protected value",
            JsonLdErrorCode::InvalidReverseValue => "invalid @reverse value",
            JsonLdErrorCode::InvalidVersionValue => "invalid @version value",
            JsonLdErrorCode::InvalidBaseDirection => "invalid base direction",
            JsonLdErrorCode::InvalidBaseIri => "invalid base IRI",
            JsonLdErrorCode::InvalidContainerMapping => "invalid container mapping",
            JsonLdErrorCode::InvalidContextEntry => "invalid context entry",
            JsonLdErrorCode::InvalidContextNullification => "invalid context nullification",
            JsonLdErrorCode::InvalidDefaultLanguage => "invalid default language",
            JsonLdErrorCode::InvalidIriMapping => "invalid IRI mapping",
            JsonLdErrorCode::InvalidJsonLiteral => "invalid JSON literal",
            JsonLdErrorCode::InvalidKeywordAlias => "invalid keyword alias",
            JsonLdErrorCode::InvalidLanguageMapValue => "invalid language map value",
            JsonLdErrorCode::InvalidLanguageMapping => "invalid language mapping",
            JsonLdErrorCode::InvalidLanguageTaggedString => "invalid language-tagged string",
            JsonLdErrorCode::InvalidLanguageTaggedValue => "invalid language-tagged value",
            JsonLdErrorCode::InvalidLocalContext => "invalid local context",
            JsonLdErrorCode::InvalidRemoteContext => "invalid remote context",
            JsonLdErrorCode::InvalidReverseProperty => "invalid reverse property",
            JsonLdErrorCode::InvalidReversePropertyMap => "invalid reverse property map",
            JsonLdErrorCode::InvalidReversePropertyValue => "invalid reverse property value",
            JsonLdErrorCode::InvalidScopedContext => "invalid scoped context",
            JsonLdErrorCode::InvalidScriptElement => "invalid script element",
            JsonLdErrorCode::InvalidSetOrListObject => "invalid set or list object",
            JsonLdErrorCode::InvalidTermDefinition => "invalid term definition",
            JsonLdErrorCode::InvalidTypeMapping => "invalid type mapping",
            JsonLdErrorCode::InvalidTypeValue => "invalid type value",
            JsonLdErrorCode::InvalidTypedValue => "invalid typed value",
            JsonLdErrorCode::InvalidValueObject => "invalid value object",
            JsonLdErrorCode::InvalidValueObjectValue => "invalid value object value",
            JsonLdErrorCode::InvalidVocabMapping => "invalid vocab mapping",
            JsonLdErrorCode::IriConfusedWithPrefix => "IRI confused with prefix",
            JsonLdErrorCode::KeywordRedefinition => "keyword redefinition",
            JsonLdErrorCode::LoadingDocumentFailed => "loading document failed",
            JsonLdErrorCode::LoadingRemoteContextFailed => "loading remote context failed",
            JsonLdErrorCode::MultipleContextLinkHeaders => "multiple context link headers",
            JsonLdErrorCode::ProcessingModeConflict => "processing mode conflict",
            JsonLdErrorCode::ProtectedTermRedefinition => "protected term redefinition",
            JsonLdErrorCode::InvalidFrame => "invalid frame",
            JsonLdErrorCode::InvalidEmbedValue => "invalid @embed value",
        }
    }
}

impl fmt::Display for JsonLdErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A JSON-LD 1.1 processing error: a spec [`JsonLdErrorCode`] plus optional human-readable
/// context. The code is the load-bearing, machine-checkable part; `detail` is advisory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonLdError {
    code: JsonLdErrorCode,
    detail: Option<String>,
}

impl JsonLdError {
    /// A bare error carrying only its spec code.
    pub fn new(code: JsonLdErrorCode) -> Self {
        JsonLdError { code, detail: None }
    }

    /// An error carrying its spec code plus a human-readable detail string.
    pub fn with_detail(code: JsonLdErrorCode, detail: impl Into<String>) -> Self {
        JsonLdError {
            code,
            detail: Some(detail.into()),
        }
    }

    /// The spec error code.
    pub fn code(&self) -> JsonLdErrorCode {
        self.code
    }

    /// The advisory detail string, if any.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for JsonLdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(d) => write!(f, "{}: {}", self.code.as_str(), d),
            None => f.write_str(self.code.as_str()),
        }
    }
}

impl std::error::Error for JsonLdError {}

impl From<JsonLdErrorCode> for JsonLdError {
    fn from(code: JsonLdErrorCode) -> Self {
        JsonLdError::new(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_as_str_is_the_exact_spec_string() {
        assert_eq!(
            JsonLdErrorCode::LoadingDocumentFailed.as_str(),
            "loading document failed"
        );
        assert_eq!(
            JsonLdErrorCode::ProtectedTermRedefinition.as_str(),
            "protected term redefinition"
        );
        assert_eq!(JsonLdErrorCode::InvalidBaseIri.as_str(), "invalid base IRI");
        assert_eq!(
            JsonLdErrorCode::IriConfusedWithPrefix.as_str(),
            "IRI confused with prefix"
        );
        assert_eq!(JsonLdErrorCode::InvalidEmbedValue.as_str(), "invalid @embed value");
    }

    #[test]
    fn code_display_matches_as_str() {
        assert_eq!(
            JsonLdErrorCode::ContextOverflow.to_string(),
            JsonLdErrorCode::ContextOverflow.as_str()
        );
    }

    #[test]
    fn new_carries_code_and_no_detail() {
        let e = JsonLdError::new(JsonLdErrorCode::InvalidLocalContext);
        assert_eq!(e.code(), JsonLdErrorCode::InvalidLocalContext);
        assert_eq!(e.detail(), None);
    }

    #[test]
    fn with_detail_carries_code_and_detail() {
        let e = JsonLdError::with_detail(JsonLdErrorCode::KeywordRedefinition, "@type");
        assert_eq!(e.code(), JsonLdErrorCode::KeywordRedefinition);
        assert_eq!(e.detail(), Some("@type"));
    }

    #[test]
    fn display_includes_detail_when_present() {
        let bare = JsonLdError::new(JsonLdErrorCode::LoadingDocumentFailed);
        assert_eq!(bare.to_string(), "loading document failed");
        let detailed =
            JsonLdError::with_detail(JsonLdErrorCode::LoadingDocumentFailed, "remote fetch denied");
        assert_eq!(
            detailed.to_string(),
            "loading document failed: remote fetch denied"
        );
    }

    #[test]
    fn from_code_builds_bare_error() {
        let e: JsonLdError = JsonLdErrorCode::InvalidFrame.into();
        assert_eq!(e.code(), JsonLdErrorCode::InvalidFrame);
        assert_eq!(e.detail(), None);
    }
}
