//! Manifest-driven virtual HTTP responses. No sockets or network clients.

use super::common::*;
use sparq_jsonld::{
    DocumentLoader, FsLoader, Json, JsonLdError, JsonLdErrorCode as E, JsonLdOptions,
    RemoteDocument,
};
fn resolve_iri(base: &str, reference: &str) -> String {
    sparq_jsonld::ActiveContext::new(Some(base))
        .expand_iri(reference, true, false)
        .unwrap_or_else(|| reference.into())
}
use serde_json::Value;
use std::path::Path;

pub struct ManifestLoader {
    files: FsLoader,
    url: String,
    options: Value,
}

impl ManifestLoader {
    pub fn new(root: &Path, entry: &Entry) -> Self {
        Self {
            files: FsLoader::new().map_prefix(SUITE_BASE, root),
            url: resolve_iri(SUITE_BASE, &entry.input),
            options: entry.options.clone(),
        }
    }

    fn load(&self, url: &str, depth: usize) -> Result<RemoteDocument, JsonLdError> {
        if depth > 16 {
            return Err(JsonLdError::new(E::LoadingDocumentFailed));
        }
        let options = if url == self.url {
            &self.options
        } else {
            &Value::Null
        };
        let status = options
            .get("httpStatus")
            .and_then(Value::as_u64)
            .unwrap_or(200);
        if (300..400).contains(&status) {
            let target = options
                .get("redirectTo")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonLdError::new(E::LoadingDocumentFailed))?;
            // Test-suite redirectTo values are relative to the manifest, not the input.
            return self.load(&resolve_iri(SUITE_BASE, target), depth + 1);
        }
        if !(200..300).contains(&status) {
            return Err(JsonLdError::new(E::LoadingDocumentFailed));
        }
        let mut doc = self.files.load_document(url)?;
        if let Some(media) = options.get("contentType").and_then(Value::as_str) {
            doc.content_type = Some(media.into());
        }
        let media = doc
            .content_type
            .as_deref()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let json = media == "application/json" || media.ends_with("+json");
        let mut contexts = Vec::new();
        let links: Vec<&str> = match options.get("httpLink") {
            Some(Value::String(s)) => vec![s],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        for header in links {
            for link in split_links(header) {
                let Some((target, parameters)) = link
                    .trim()
                    .strip_prefix('<')
                    .and_then(|s| s.split_once('>'))
                else {
                    continue;
                };
                let mut relation = "";
                let mut kind = "";
                for parameter in parameters.split(';').skip(1) {
                    if let Some((key, value)) = parameter.trim().split_once('=') {
                        let value = value.trim().trim_matches('"');
                        if key.eq_ignore_ascii_case("rel") {
                            relation = value;
                        }
                        if key.eq_ignore_ascii_case("type") {
                            kind = value;
                        }
                    }
                }
                let target = resolve_iri(url, target);
                if !json
                    && relation.split_ascii_whitespace().any(|r| r == "alternate")
                    && kind.eq_ignore_ascii_case("application/ld+json")
                {
                    return self.load(&target, depth + 1);
                }
                if json
                    && media != "application/ld+json"
                    && relation
                        .split_ascii_whitespace()
                        .any(|r| r == "http://www.w3.org/ns/json-ld#context")
                {
                    contexts.push(target);
                }
            }
        }
        if contexts.len() > 1 {
            return Err(JsonLdError::new(E::MultipleContextLinkHeaders));
        }
        doc.context_url = contexts.pop();
        Ok(doc)
    }
}

// A comma inside a quoted parameter or URI reference is not a link separator.
fn split_links(header: &str) -> Vec<&str> {
    let (mut quoted, mut angle, mut escaped) = (false, false, false);
    let mut start = 0;
    let mut links = Vec::new();
    for (i, ch) in header.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' if !angle => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' if !quoted && !angle => {
                links.push(&header[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    links.push(&header[start..]);
    links
}

impl DocumentLoader for ManifestLoader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        self.load(url, 0)
    }
}

pub fn run(root: &Path, category: &str) -> Score {
    let mut score = Score::default();
    let entries = match read_manifest(root, category) {
        Ok(entries) => entries,
        Err(error) => {
            score.fail(category, error);
            return score;
        }
    };
    for entry in entries {
        let mut options = JsonLdOptions::default();
        options.base = entry.base_override.clone();
        options.extract_all_scripts = entry
            .options
            .get("extractAllScripts")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let loader = ManifestLoader::new(root, &entry);
        let input = Json::Str(resolve_iri(SUITE_BASE, &entry.input));
        let outcome = (|| {
            if entry.types.iter().any(|t| t.ends_with(":ToRDFTest")) {
                return super::to_rdf::native_dataset(&input, &options, &loader)
                    .map(|dataset| Output::Rdf(Box::new(dataset)));
            }
            let context = entry
                .context
                .as_ref()
                .map(|p| {
                    let text = std::fs::read_to_string(root.join(p)).map_err(|e| {
                        JsonLdError::with_detail(E::LoadingDocumentFailed, e.to_string())
                    })?;
                    Json::parse(&text).map_err(|e| {
                        JsonLdError::with_detail(E::LoadingDocumentFailed, e.to_string())
                    })
                })
                .transpose()?;
            let output = if entry.types.iter().any(|t| t.ends_with(":CompactTest")) {
                sparq_jsonld::compact::compact(
                    &input,
                    context.as_ref().unwrap_or(&Json::Raw("null".into())),
                    &options,
                    &loader,
                )?
            } else if entry.types.iter().any(|t| t.ends_with(":FlattenTest")) {
                sparq_jsonld::flatten::flatten_with_context(
                    &input,
                    context.as_ref(),
                    &options,
                    &loader,
                )?
            } else {
                sparq_jsonld::expand(&input, &options, &loader)?
            };
            Ok(Output::Json(output))
        })();
        if let Some(want) = &entry.expect_error_code {
            match outcome {
                Err(e) if e.code().as_str() == want => score.pass(&entry.id),
                other => score.fail(&entry.id, format!("expected {want}, got {other:?}")),
            }
            continue;
        }
        let result = outcome.map_err(|e| e.to_string()).and_then(|output| {
            let expect = entry.expect.as_ref().ok_or("missing expected output")?;
            let text = std::fs::read_to_string(root.join(expect)).map_err(|e| e.to_string())?;
            match output {
                Output::Json(json) => {
                    let got = sparq_json_to_serde(&json)?;
                    let want: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    if json_ld_output_equal(&got, &want, &options, &loader).is_ok() {
                        Ok(())
                    } else {
                        Err(format!("JSON mismatch: {got} != {want}"))
                    }
                }
                Output::Rdf(got) => {
                    if *got == nquads_to_canonical_dataset(&text)? {
                        Ok(())
                    } else {
                        Err("RDF mismatch".into())
                    }
                }
            }
        });
        match result {
            Ok(()) => score.pass(&entry.id),
            Err(why) => score.fail(&entry.id, why),
        }
    }
    score
}

#[derive(Debug)]
enum Output {
    Json(Json),
    Rdf(Box<oxrdf::Dataset>),
}

#[test]
fn virtual_responses_apply_redirect_status_media_and_context_links() {
    let fixture = crate::earl::Fixture::new();
    fixture.write("input.json", "{}");
    fixture.write("target.jsonld", "{\"@id\":\"\"}");
    let loader = |options: Value| {
        fixture.write("remote-doc-manifest.jsonld", serde_json::to_vec(&serde_json::json!({"sequence":[{"@id":"#case","input":"input.json","option":options}]})).unwrap());
        let entry = read_manifest(fixture.root(), "remote-doc")
            .unwrap()
            .remove(0);
        ManifestLoader::new(fixture.root(), &entry)
    };
    for status in [301, 303, 307] {
        let doc = loader(serde_json::json!({"httpStatus":status,"redirectTo":"target.jsonld"}))
            .load_document(&format!("{SUITE_BASE}input.json"))
            .unwrap();
        assert_eq!(doc.document_url, format!("{SUITE_BASE}target.jsonld"));
        assert_eq!(doc.content_type.as_deref(), Some("application/ld+json"));
    }
    for status in [404, 500] {
        assert_eq!(
            loader(serde_json::json!({"httpStatus":status}))
                .load_document(&format!("{SUITE_BASE}input.json"))
                .unwrap_err()
                .code(),
            E::LoadingDocumentFailed
        );
    }
    let header = r#"<context.jsonld>; rel="http://www.w3.org/ns/json-ld#context"; title="a,b", <other>; rel="alternate"; type="application/json""#;
    let doc = loader(serde_json::json!({"httpLink":header}))
        .load_document(&format!("{SUITE_BASE}input.json"))
        .unwrap();
    assert_eq!(doc.context_url, Some(format!("{SUITE_BASE}context.jsonld")));
    let double = [
        header,
        r#"<second.jsonld>; rel="http://www.w3.org/ns/json-ld#context""#,
    ];
    assert_eq!(
        loader(serde_json::json!({"httpLink":double}))
            .load_document(&format!("{SUITE_BASE}input.json"))
            .unwrap_err()
            .code(),
        E::MultipleContextLinkHeaders
    );
    for media in ["application/ld+json", "text/html", "application/xhtml+xml"] {
        let doc = loader(serde_json::json!({"contentType":media,"httpLink":double}))
            .load_document(&format!("{SUITE_BASE}input.json"))
            .unwrap();
        assert_eq!(doc.context_url, None);
    }
    let doc = loader(serde_json::json!({"contentType":"text/plain","httpLink":"<target.jsonld>; rel=alternate; type=application/ld+json"})).load_document(&format!("{SUITE_BASE}input.json")).unwrap();
    assert_eq!(doc.document_url, format!("{SUITE_BASE}target.jsonld"));
    assert_eq!(
        loader(Value::Null)
            .load_document("https://unmapped.example/doc")
            .unwrap_err()
            .code(),
        E::LoadingDocumentFailed
    );
}
