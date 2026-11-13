#![cfg(feature = "html")]

use sparq_jsonld::{
    expand, DocumentLoader, Json, JsonLdError, JsonLdErrorCode as E, JsonLdOptions, RemoteDocument,
};

struct Html<'a> {
    text: &'a str,
    media: &'a str,
}
impl DocumentLoader for Html<'_> {
    fn load_document(&self, _: &str) -> Result<RemoteDocument, JsonLdError> {
        let mut doc = RemoteDocument::new(self.text, "https://final.example/dir/doc");
        doc.content_type = Some(self.media.into());
        doc.context_url = Some("https://must-not-load.example/".into());
        Ok(doc)
    }
}
fn extract(text: &str, fragment: &str, all: bool) -> Result<Json, JsonLdError> {
    let mut options = JsonLdOptions::default();
    options.extract_all_scripts = all;
    expand(
        &Json::Str(format!("https://request.example/doc{fragment}")),
        &options,
        &Html {
            text,
            media: "Text/HTML; charset=utf-8",
        },
    )
}
fn node(value: &str) -> Json {
    Json::parse(&format!(r#"[{{"https://ex/p":[{{"@value":{value}}}]}}]"#)).unwrap()
}

#[test]
fn comments_and_raw_text_do_not_create_scripts() {
    let text = r#"<!-- <script type="application/ld+json">invalid</script> -->
      <textarea><script type="application/ld+json">also invalid</script></textarea>
      <style><script type="application/ld+json">invalid</script></style>
      <SCRIPT TYPE='application/ld+json'>{"https://ex/p":"&quot; <!-- -->"}</SCRIPT>"#;
    assert_eq!(
        extract(text, "", false).unwrap(),
        node(r#""&quot; <!-- -->""#)
    );
}

#[test]
fn first_all_and_fragment_have_distinct_selection_rules() {
    let text = r#"<script id="one" type="application/ld+json">{"https://ex/p":1}</script>
      <script id="é two" type="application/ld+json">[{"https://ex/p":2},{"https://ex/p":3}]</script>"#;
    assert_eq!(extract(text, "", false).unwrap(), node("1"));
    let Json::Arr(nodes) = extract(text, "", true).unwrap() else {
        panic!("array")
    };
    assert_eq!(nodes.len(), 3);
    let selected = extract(text, "#%C3%A9%20two", true).unwrap();
    let Json::Arr(nodes) = selected else {
        panic!("array")
    };
    assert_eq!(nodes.len(), 2);
    assert_eq!(
        extract("<script type='text/plain'>bad</script>", "", true).unwrap(),
        Json::Arr(vec![])
    );
}

#[test]
fn missing_invalid_and_duplicate_fragment_targets_fail() {
    for (text, fragment) in [
        ("<html></html>", ""),
        ("<html></html>", "#missing"),
        ("<div id='x'>{}</div>", "#x"),
        ("<script id='x' type='text/plain'>{}</script>", "#x"),
        (
            "<div id='x'></div><script id='x' type='application/ld+json'>{}</script>",
            "#x",
        ),
    ] {
        assert_eq!(
            extract(text, fragment, false).unwrap_err().code(),
            E::LoadingDocumentFailed
        );
    }
}

#[test]
fn invalid_json_and_legacy_comment_wrappers_are_invalid_scripts() {
    for source in [
        "{",
        "<!-- {} -->",
        "<!-- {}",
        "{} -->",
        r#"{"https://ex/p":"</script>"}"#,
    ] {
        let text = format!("<script type='application/ld+json'>{source}</script>");
        assert_eq!(
            extract(&text, "", false).unwrap_err().code(),
            E::InvalidScriptElement
        );
    }
    let text = "<script type='application/ld+json'>{}</script><script type='application/ld+json'>bad</script>";
    assert!(extract(text, "", false).is_ok());
    assert_eq!(
        extract(text, "", true).unwrap_err().code(),
        E::InvalidScriptElement
    );
}

#[test]
fn first_base_resolves_against_final_url_or_explicit_fallback() {
    let text = r#"<BASE HREF='../base/'><base href='https://ignored.example/'>
      <script type='application/ld+json'>{"@id":"child","https://ex/p":"v"}</script>"#;
    let output = extract(text, "", false).unwrap();
    let Json::Arr(nodes) = output else {
        panic!("array")
    };
    assert_eq!(
        nodes[0].get("@id").and_then(Json::as_str),
        Some("https://final.example/base/child")
    );
    let mut options = JsonLdOptions::default();
    options.base = Some("https://override.example/dir/doc".into());
    let output = expand(
        &Json::Str("https://request.example/doc".into()),
        &options,
        &Html {
            text,
            media: "text/html",
        },
    )
    .unwrap();
    let Json::Arr(nodes) = output else {
        panic!("array")
    };
    assert_eq!(
        nodes[0].get("@id").and_then(Json::as_str),
        Some("https://override.example/base/child")
    );
}

#[test]
fn xhtml_uses_xml_cdata_entities_and_namespaces() {
    let text = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><script type="application/ld+json"><![CDATA[{"https://ex/p":"<value>"}]]></script></head></html>"#;
    let output = expand(
        &Json::Str("https://request.example/doc".into()),
        &JsonLdOptions::default(),
        &Html {
            text,
            media: "application/xhtml+xml",
        },
    )
    .unwrap();
    assert_eq!(output, node(r#""<value>""#));
    let text =
        r#"<html><script type="application/ld+json">{"https://ex/p":"&amp;"}</script></html>"#;
    let output = expand(
        &Json::Str("https://request.example/doc".into()),
        &JsonLdOptions::default(),
        &Html {
            text,
            media: "application/xhtml+xml",
        },
    )
    .unwrap();
    assert_eq!(output, node(r#""&""#));
}

#[test]
fn template_contents_do_not_supply_document_scripts_ids_or_base() {
    let text = r#"<template><base href='https://ignored.example/'><script id='hidden' type='application/ld+json'>bad</script></template>
      <script type='application/ld+json'>{"@id":"child","https://ex/p":"v"}</script>"#;
    let output = extract(text, "", false).unwrap();
    let Json::Arr(nodes) = output else {
        panic!("array")
    };
    assert_eq!(
        nodes[0].get("@id").and_then(Json::as_str),
        Some("https://final.example/dir/child")
    );
    assert_eq!(
        extract(text, "#hidden", true).unwrap_err().code(),
        E::LoadingDocumentFailed
    );
}

#[test]
fn foreign_script_elements_are_not_html_script_elements() {
    let text = r#"<svg><script type='application/ld+json'>bad</script></svg>
      <script type='application/ld+json'>{"https://ex/p":"v"}</script>"#;
    assert_eq!(extract(text, "", false).unwrap(), node(r#""v""#));
}

#[test]
fn xhtml_fragment_lookup_keeps_foreign_elements_in_document_order() {
    let text = r#"<html xmlns="http://www.w3.org/1999/xhtml"><svg xmlns="http://www.w3.org/2000/svg"><g id="x"/></svg><script id="x" type="application/ld+json">{"https://ex/p":"v"}</script></html>"#;
    let error = expand(
        &Json::Str("https://request.example/doc#x".into()),
        &JsonLdOptions::default(),
        &Html {
            text,
            media: "application/xhtml+xml",
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), E::LoadingDocumentFailed);
}
