//! (`sq-bif.26`) Minimal-input integration pins for deterministic JSON-LD
//! parsing, expansion, compaction, and deny-by-default document loading.

use sparq_jsonld::compact::compact_expanded;
use sparq_jsonld::{expand, DocumentLoader, Json, JsonLdErrorCode, JsonLdOptions, NoopLoader};

#[test]
fn empty_and_whitespace_only_json_report_the_expected_error() {
    let empty = Json::parse("").expect_err("empty input must be rejected");
    assert_eq!(empty.message, "expected a JSON value");
    assert_eq!(empty.position, 0);

    let whitespace = Json::parse(" \t\n\r").expect_err("whitespace-only input must be rejected");
    assert_eq!(whitespace.message, "expected a JSON value");
}

#[test]
fn malformed_fraction_reports_the_specific_error() {
    let error = Json::parse("1.").expect_err("a fraction requires at least one digit");
    assert_eq!(error.message, "invalid number: fraction requires a digit");
}

#[test]
fn unterminated_string_reports_the_specific_error() {
    let error = Json::parse(r#""abc"#).expect_err("an unterminated string must be rejected");
    assert_eq!(error.message, "unterminated string");
}

#[test]
fn null_and_empty_array_expand_to_empty_arrays() {
    for input in ["null", "[]"] {
        let parsed = Json::parse(input).expect("minimal input is valid JSON");
        let expanded = expand(&parsed, &JsonLdOptions::default(), &NoopLoader)
            .expect("minimal input expansion succeeds");

        match expanded {
            Json::Arr(items) => assert!(items.is_empty(), "{input} must expand to []"),
            other => panic!("{input} must expand to an array, got {other:?}"),
        }
    }
}

#[test]
fn empty_object_context_is_not_embedded_by_compact_expanded() {
    let expanded =
        Json::parse(r#"[{"http://ex/p":[{"@value":1}]}]"#).expect("expanded input is valid JSON");
    let empty_context = Json::parse("{}").expect("empty context is valid JSON");
    let compacted = compact_expanded(
        &expanded,
        &empty_context,
        &JsonLdOptions::default(),
        &NoopLoader,
    )
    .expect("compaction against an empty context succeeds");

    assert_eq!(
        compacted,
        Json::parse(r#"{"http://ex/p":1}"#).expect("expected output is valid JSON")
    );
    assert!(compacted.get("@context").is_none());
}

#[test]
fn noop_loader_denies_remote_documents_deterministically() {
    let error = NoopLoader
        .load_document("https://example.test/context.jsonld")
        .expect_err("NoopLoader must reject every remote document");

    assert_eq!(error.code(), JsonLdErrorCode::LoadingDocumentFailed);
    assert!(error
        .detail()
        .expect("NoopLoader denial includes detail")
        .contains("remote document loading is disabled"));
}
