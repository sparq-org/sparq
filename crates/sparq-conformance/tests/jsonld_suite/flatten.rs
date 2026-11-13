//! Native flattening, including context compaction and exact negative error codes.
use super::common::*;
use sparq_jsonld::{Json, JsonLdOptions};
use std::path::Path;

pub fn run_flatten_native(root: &Path) -> Score {
    let mut score = Score::default();
    let entries = match read_manifest(root, "flatten") {
        Ok(entries) => entries,
        Err(why) => {
            score.fail("flatten-manifest", why);
            return score;
        }
    };
    for entry in entries {
        if entry.spec_version.as_deref() == Some("json-ld-1.0")
            || entry.processing_mode.as_deref() == Some("json-ld-1.0")
        {
            score.skip();
            continue;
        }
        let mut options = JsonLdOptions::default();
        options.base = Some(doc_base(&entry));
        options.compact_arrays = entry.compact_arrays.unwrap_or(true);
        let loader = super::remote::ManifestLoader::new(root, &entry);
        let result = (|| {
            let text =
                std::fs::read_to_string(root.join(&entry.input)).map_err(|e| e.to_string())?;
            let input = Json::parse(&text).map_err(|e| e.to_string())?;
            let outcome = sparq_jsonld::flatten(&input, &options, &loader);
            if let Some(want) = &entry.expect_error_code {
                return match outcome {
                    Err(e) if e.code().as_str() == want => Ok(()),
                    other => Err(format!("expected {want}, got {other:?}")),
                };
            }
            let mut flattened = outcome.map_err(|e| e.to_string())?;
            if let Some(context) = &entry.context {
                let text =
                    std::fs::read_to_string(root.join(context)).map_err(|e| e.to_string())?;
                let context = Json::parse(&text).map_err(|e| e.to_string())?;
                flattened = sparq_jsonld::flatten::flatten_with_context(
                    &input,
                    Some(&context),
                    &options,
                    &loader,
                )
                .map_err(|e| e.to_string())?;
            }
            let got = sparq_json_to_serde(&flattened)?;
            let expect = entry.expect.as_ref().ok_or("missing expected document")?;
            let text = std::fs::read_to_string(root.join(expect)).map_err(|e| e.to_string())?;
            let want = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if json_ld_equal(&got, &want) {
                Ok(())
            } else {
                Err(format!("JSON mismatch: {got} != {want}"))
            }
        })();
        match result {
            Ok(()) => score.pass(&entry.id),
            Err(why) => score.fail(&entry.id, why),
        }
    }
    score
}
