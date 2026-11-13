use sparq_jsonld::{
    expand, DocumentLoader, Json, JsonLdError, JsonLdErrorCode as E, JsonLdOptions, RemoteDocument,
};

struct Loader;
impl DocumentLoader for Loader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        let mut document = RemoteDocument::new(
            r#"{"@id":"child", "name":"value"}"#,
            "https://final.example/doc",
        );
        document.content_type = Some("application/json; charset=utf-8".into());
        document.context_url = Some("context".into());
        match url {
            "https://request.example/doc" => {}
            "https://final.example/context" => {
                return Ok(RemoteDocument::new(
                    r#"{"@context":{"name":"https://vocab.example/name"}}"#,
                    url,
                ))
            }
            "https://request.example/ld" => {
                document.content_type = Some("Application/LD+JSON;profile=x".into())
            }
            "https://request.example/vendor" => {
                document.content_type = Some("application/vendor+json".into())
            }
            "https://request.example/bad-type" => document.content_type = Some("text/plain".into()),
            "https://request.example/bad-json" => document.document = "{".into(),
            _ => return Err(JsonLdError::new(E::LoadingDocumentFailed)),
        }
        Ok(document)
    }
}

#[test]
fn remote_context_link_and_redirect_base_are_used() {
    let output = expand(
        &Json::Str("https://request.example/doc".into()),
        &JsonLdOptions::default(),
        &Loader,
    )
    .unwrap();
    assert_eq!(output, Json::parse(r#"[{"@id":"https://final.example/child","https://vocab.example/name":[{"@value":"value"}]}]"#).unwrap());
    let vendor = expand(
        &Json::Str("https://request.example/vendor".into()),
        &JsonLdOptions::default(),
        &Loader,
    )
    .unwrap();
    assert_eq!(vendor, output);
}

#[test]
fn explicit_base_does_not_rebase_remote_context_link() {
    let mut options = JsonLdOptions::default();
    options.base = Some("https://override.example/base/".into());
    let output = expand(
        &Json::Str("https://request.example/doc".into()),
        &options,
        &Loader,
    )
    .unwrap();
    let Json::Arr(nodes) = output else {
        panic!("expanded array")
    };
    assert_eq!(
        nodes[0].get("@id").and_then(Json::as_str),
        Some("https://override.example/base/child")
    );
    assert!(nodes[0].get("https://vocab.example/name").is_some());
}

#[test]
fn jsonld_media_type_ignores_context_link() {
    let output = expand(
        &Json::Str("https://request.example/ld".into()),
        &JsonLdOptions::default(),
        &Loader,
    )
    .unwrap();
    assert_eq!(output, Json::Arr(vec![]));
}

#[test]
fn failed_type_json_and_missing_documents_have_loading_error() {
    for path in ["bad-type", "bad-json", "missing"] {
        let error = expand(
            &Json::Str(format!("https://request.example/{path}")),
            &JsonLdOptions::default(),
            &Loader,
        )
        .unwrap_err();
        assert_eq!(error.code(), E::LoadingDocumentFailed, "{path}");
    }
}

#[test]
fn compaction_prefix_error_is_not_lost_through_the_context_cache() {
    let input = Json::parse(r#"[{"tag:example,2026:p":{"@value":"v"}}]"#).unwrap();
    let context = Json::parse(r#"{"tag":"https://vocab.example/"}"#).unwrap();
    for _ in 0..2 {
        let error =
            sparq_jsonld::compact::compact(&input, &context, &JsonLdOptions::default(), &Loader)
                .unwrap_err();
        assert_eq!(error.code(), E::IriConfusedWithPrefix);
    }
}

#[test]
fn flatten_context_keeps_graph_but_compacts_property_arrays() {
    let input = Json::parse(r#"{"https://vocab.example/p":"v"}"#).unwrap();
    let context = Json::parse(r#"{"p":"https://vocab.example/p"}"#).unwrap();
    let output = sparq_jsonld::flatten::flatten_with_context(
        &input,
        Some(&context),
        &JsonLdOptions::default(),
        &Loader,
    )
    .unwrap();
    assert_eq!(
        output,
        Json::parse(
            r#"{"@context":{"p":"https://vocab.example/p"},"@graph":[{"@id":"_:b0","p":"v"}]}"#
        )
        .unwrap()
    );
}

#[test]
fn conflicting_indexes_error_in_flatten_and_to_rdf() {
    let input =
        Json::parse(r#"[{"@id":"https://ex/s","@index":"a"},{"@id":"https://ex/s","@index":"b"}]"#)
            .unwrap();
    let options = JsonLdOptions::default();
    assert_eq!(
        sparq_jsonld::flatten(&input, &options, &Loader)
            .unwrap_err()
            .code(),
        E::ConflictingIndexes
    );
    assert_eq!(
        sparq_jsonld::to_rdf::to_rdf(&input, &options, &Loader)
            .unwrap_err()
            .code(),
        E::ConflictingIndexes
    );
    assert_eq!(
        sparq_jsonld::frame::frame(
            &input,
            &Json::obj(),
            &options,
            &sparq_jsonld::frame::FrameOptions::default(),
            &Loader
        )
        .unwrap_err()
        .code(),
        E::ConflictingIndexes
    );
}

#[cfg(not(feature = "html"))]
#[test]
fn html_loading_requires_the_opt_in_feature() {
    struct Html;
    impl DocumentLoader for Html {
        fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
            let mut doc =
                RemoteDocument::new("<script type='application/ld+json'>{}</script>", url);
            doc.content_type = Some("text/html".into());
            Ok(doc)
        }
    }
    let error = expand(
        &Json::Str("https://ex/doc".into()),
        &JsonLdOptions::default(),
        &Html,
    )
    .unwrap_err();
    assert_eq!(error.code(), E::LoadingDocumentFailed);
}

#[test]
fn remote_compaction_uses_the_final_url_for_relative_output() {
    let input = Json::Str("https://request.example/doc".into());
    let context = Json::parse(r#"{"p":"https://vocab.example/name"}"#).unwrap();
    let options = JsonLdOptions::default();
    let compacted = sparq_jsonld::compact::compact(&input, &context, &options, &Loader).unwrap();
    assert_eq!(compacted.get("@id").and_then(Json::as_str), Some("child"));
}

#[test]
fn remote_flattening_uses_the_final_url_for_relative_output() {
    let input = Json::Str("https://request.example/doc".into());
    let context = Json::parse(r#"{"p":"https://vocab.example/name"}"#).unwrap();
    let options = JsonLdOptions::default();
    let flattened =
        sparq_jsonld::flatten::flatten_with_context(&input, Some(&context), &options, &Loader)
            .unwrap();
    let Some(Json::Arr(nodes)) = flattened.get("@graph") else {
        panic!("graph array")
    };
    assert_eq!(nodes[0].get("@id").and_then(Json::as_str), Some("child"));
}

#[test]
fn valid_compact_iri_can_keep_the_original_spelling() {
    let input = Json::parse(r#"{"@id":"urn:x","https://ex/p":"v"}"#).unwrap();
    let context = Json::parse(r#"{"@vocab":"urn:","urn":{"@id":"","@prefix":true}}"#).unwrap();
    for _ in 0..2 {
        let output =
            sparq_jsonld::compact::compact(&input, &context, &JsonLdOptions::default(), &Loader)
                .unwrap();
        assert_eq!(output.get("@id").and_then(Json::as_str), Some("urn:x"));
        assert_eq!(
            expand(&output, &JsonLdOptions::default(), &Loader).unwrap(),
            expand(&input, &JsonLdOptions::default(), &Loader).unwrap()
        );
    }
}

#[test]
fn valid_compact_iri_is_kept_when_its_iri_is_in_the_inverse_context() {
    let input = Json::parse(r#"{"@id":"https://ex/s","urn:x":"v"}"#).unwrap();
    let context = Json::parse(
        r#"{"@vocab":"urn:","urn":{"@id":"","@prefix":true},"x":null,"other":{"@id":"urn:x","@type":"@id"}}"#,
    )
    .unwrap();
    // `other` cannot compact a literal, and `x` blocks vocabulary compaction.
    for _ in 0..2 {
        let output =
            sparq_jsonld::compact::compact(&input, &context, &JsonLdOptions::default(), &Loader)
                .unwrap();
        assert_eq!(output.get("urn:x").and_then(Json::as_str), Some("v"));
        assert_eq!(
            expand(&output, &JsonLdOptions::default(), &Loader).unwrap(),
            expand(&input, &JsonLdOptions::default(), &Loader).unwrap()
        );
    }
}

#[test]
fn prefix_confusion_is_detected_before_base_relativization() {
    let input = Json::parse(r#"{"@id":"urn:x","https://ex/p":"v"}"#).unwrap();
    let context =
        Json::parse(r#"{"urn":{"@id":"https://vocab.example/","@prefix":true}}"#).unwrap();
    let mut options = JsonLdOptions::default();
    options.base = Some("urn:base".into());
    for _ in 0..2 {
        assert_eq!(
            sparq_jsonld::compact::compact(&input, &context, &options, &Loader)
                .unwrap_err()
                .code(),
            E::IriConfusedWithPrefix
        );
    }
}

#[test]
fn null_remote_context_restores_original_document_base() {
    struct Remote<'a>(&'a str);
    impl DocumentLoader for Remote<'_> {
        fn load_document(&self, _: &str) -> Result<RemoteDocument, JsonLdError> {
            Ok(RemoteDocument::new(self.0, "https://final.example/doc"))
        }
    }
    let mut options = JsonLdOptions::default();
    options.base = Some("https://override.example/base/".into());
    let url = Json::Str("https://request.example/doc".into());
    for (document, expected) in [
        (
            r#"{"@id":"child","https://ex/p":"v"}"#,
            "https://override.example/base/child",
        ),
        (
            r#"{"@context":null,"@id":"child","https://ex/p":"v"}"#,
            "https://final.example/child",
        ),
        (
            r#"{"@context":[{"@base":"https://local.example/"},null],"@id":"child","https://ex/p":"v"}"#,
            "https://final.example/child",
        ),
    ] {
        let output = expand(&url, &options, &Remote(document)).unwrap();
        let Json::Arr(nodes) = output else {
            panic!("expanded array")
        };
        assert_eq!(nodes[0].get("@id").and_then(Json::as_str), Some(expected));
    }
    let inline = Json::parse(r#"{"@context":null,"@id":"child","https://ex/p":"v"}"#).unwrap();
    let Json::Arr(nodes) = expand(&inline, &options, &Loader).unwrap() else {
        panic!("expanded array")
    };
    assert_eq!(
        nodes[0].get("@id").and_then(Json::as_str),
        Some("https://override.example/base/child")
    );
}
