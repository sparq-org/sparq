//! The **connector adapter** (one, per the Phase-5 scope): turn a recorded, OpenAlex-
//! shaped works batch into normalised, DOI-keyed [`SourceStub`]s. NO network — the input
//! is a committed JSON string ([`super::FIXTURE_OPENALEX_BATCH`]).
//!
//! sq-2489d.5. The live connector (a real OpenAlex/S2 HTTP client) is the
//! gated Phase-6 work; this adapter reads the *same record shape* off a fixture so the
//! downstream pipeline is exercised identically whether the records came from a tape or a
//! live pull. The DOI is the content-addressed join key (§4.2): the same paper from two
//! sources stitches to one `pkg:Source` IRI.

use serde_json::Value;

/// A normalised, DOI-keyed bibliographic stub — the connector's output and the unit the
/// rest of the pipeline consumes. Mirrors the subset of OpenAlex `/works` fields the
/// adapter reads; a live connector for another source (S2, Crossref) would produce the
/// same struct, so the downstream is source-agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStub {
    /// The DOI (the content-addressed join key). Normalised to a bare `doi:`-less DOI
    /// string, e.g. `10.5555/example.x` (the `https://doi.org/` prefix is stripped).
    pub doi: String,
    /// The work title.
    pub title: String,
    /// The abstract (or TLDR) text — the span the grounding-resolver checks justifications
    /// against (§4.3). Empty when the record carried none.
    pub abstract_text: String,
    /// The publication year, when present.
    pub year: Option<i64>,
    /// The source's redistribution licence, captured verbatim from the record when it
    /// carries one (e.g. `cc-by`, `cc-by-nc`), else `None` for an UNKNOWN/absent licence.
    ///
    /// This field drives the downstream dump tiering and is **fail-closed**: `None`
    /// (unknown) is treated as NON-REDISTRIBUTABLE by the tier partition (`sq-tzars.7`),
    /// so a public projection of an unknown-licence source is metadata-only. Captured by
    /// both connectors — `parse_openalex_batch` here and `parse_core_batch` in the
    /// `connector_core` module. Many bibliographic APIs omit licence metadata (CORE v3's
    /// search schema does), so `None` is the common — and deliberately safe — default.
    /// sq-tzars.1
    pub license: Option<String>,
}

impl SourceStub {
    /// The content-addressed `pkg:Source` IRI for this stub: the DOI, percent-safe, under
    /// the project `doi:` resolver namespace. Same DOI ⇒ same IRI ⇒ one node (§4.2 dedup).
    pub fn source_iri(&self) -> String {
        format!("https://doi.org/{}", self.doi)
    }
}

/// Normalise the `https://doi.org/` (or `doi:`/`DOI:`) prefix off a raw DOI string,
/// lower-casing the DOI (DOIs are case-insensitive). Returns `None` for an empty/whitespace
/// DOI — a record with no DOI cannot be content-addressed, so the adapter skips it.
pub fn normalise_doi(raw: &str) -> Option<String> {
    let t = raw.trim();
    let stripped = t
        .strip_prefix("https://doi.org/")
        .or_else(|| t.strip_prefix("http://doi.org/"))
        .or_else(|| t.strip_prefix("doi:"))
        .or_else(|| t.strip_prefix("DOI:"))
        .unwrap_or(t)
        .trim();
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.to_ascii_lowercase())
}

/// Parse a recorded OpenAlex-shaped works batch (`{ "results": [ { doi, title, abstract,
/// publication_year, … }, … ] }`) into normalised [`SourceStub`]s. Records with no DOI or
/// no title are skipped (they cannot be content-addressed / catalogued); the count of
/// skipped records is returned alongside so the caller can report it (never silently lost).
///
/// Pure + deterministic; reads the committed JSON string only — NO network.
pub fn parse_openalex_batch(json: &str) -> Result<(Vec<SourceStub>, usize), String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("connector JSON: {}", e))?;
    let results = root
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "connector JSON: missing `results` array".to_string())?;

    let mut stubs = Vec::new();
    let mut skipped = 0usize;
    for rec in results {
        let doi = rec
            .get("doi")
            .and_then(Value::as_str)
            .and_then(normalise_doi);
        let title = rec.get("title").and_then(Value::as_str).map(str::to_string);
        let (Some(doi), Some(title)) = (doi, title) else {
            skipped += 1;
            continue;
        };
        if title.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let abstract_text = rec
            .get("abstract")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let year = rec.get("publication_year").and_then(Value::as_i64);
        let license = openalex_license(rec);
        stubs.push(SourceStub {
            doi,
            title,
            abstract_text,
            year,
            license,
        });
    }
    Ok((stubs, skipped))
}

/// Extract the OpenAlex licence for a `/works` record, fail-closed to `None` when absent.
///
/// OpenAlex records the licence per open-access LOCATION, not once per work, so we read the
/// `primary_location.license` (the canonical host copy), falling back to
/// `best_oa_location.license` and finally a top-level `license`. A blank string is treated
/// as absent. `None` (the common case) is captured as an UNKNOWN licence and treated as
/// non-redistributable downstream. sq-tzars.1
fn openalex_license(rec: &Value) -> Option<String> {
    let from = |key: &str| {
        rec.get(key)
            .and_then(|loc| loc.get("license"))
            .and_then(Value::as_str)
    };
    from("primary_location")
        .or_else(|| from("best_oa_location"))
        .or_else(|| rec.get("license").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_doi_strips_resolver_prefixes_and_lowercases() {
        assert_eq!(
            normalise_doi("https://doi.org/10.5555/Example.X"),
            Some("10.5555/example.x".to_string())
        );
        assert_eq!(normalise_doi("doi:10.1/AbC"), Some("10.1/abc".to_string()));
        assert_eq!(normalise_doi("   "), None);
        assert_eq!(normalise_doi(""), None);
    }

    #[test]
    fn parse_fixture_batch_yields_three_stubs() {
        let (stubs, skipped) =
            parse_openalex_batch(super::super::FIXTURE_OPENALEX_BATCH).expect("fixture parses");
        assert_eq!(stubs.len(), 3, "the fixture has three well-formed works");
        assert_eq!(skipped, 0, "no records are skipped in the clean fixture");
        // DOIs are normalised + content-addressed.
        assert!(stubs
            .iter()
            .any(|s| s.doi == "10.5555/example.rdf-parallel-parse"));
        let zk = stubs
            .iter()
            .find(|s| s.doi == "10.5555/example.zk-sparql-soundness")
            .expect("zk stub present");
        assert!(zk.abstract_text.contains("zero-knowledge protocol"));
        assert_eq!(zk.year, Some(2025));
        assert_eq!(
            zk.source_iri(),
            "https://doi.org/10.5555/example.zk-sparql-soundness"
        );
        // The fabricated OpenAlex fixture carries no licence metadata, so every stub is
        // captured with an UNKNOWN licence (fail-closed = non-redistributable downstream).
        assert!(
            stubs.iter().all(|s| s.license.is_none()),
            "unknown licence is captured as None (fail-closed)"
        );
    }

    #[test]
    fn openalex_license_is_read_from_primary_location_when_present() {
        let json = r#"{ "results": [
            { "doi": "10.1/a", "title": "cc-by work", "abstract": "x",
              "primary_location": { "license": "cc-by" } },
            { "doi": "10.1/b", "title": "best-oa fallback", "abstract": "y",
              "best_oa_location": { "license": "cc-by-nc" } },
            { "doi": "10.1/c", "title": "blank licence is unknown", "abstract": "z",
              "primary_location": { "license": "  " } }
        ] }"#;
        let (stubs, _) = parse_openalex_batch(json).expect("parses");
        let by_doi = |d: &str| stubs.iter().find(|s| s.doi == d).unwrap();
        assert_eq!(by_doi("10.1/a").license.as_deref(), Some("cc-by"));
        assert_eq!(by_doi("10.1/b").license.as_deref(), Some("cc-by-nc"));
        // A blank licence string is treated as absent (fail-closed), not the empty string.
        assert_eq!(by_doi("10.1/c").license, None);
    }

    #[test]
    fn record_without_doi_is_skipped_not_dropped_silently() {
        let json = r#"{ "results": [
            { "title": "no doi", "abstract": "x" },
            { "doi": "10.1/ok", "title": "ok", "abstract": "y" }
        ] }"#;
        let (stubs, skipped) = parse_openalex_batch(json).expect("parses");
        assert_eq!(stubs.len(), 1);
        assert_eq!(skipped, 1, "the DOI-less record is counted as skipped");
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse_openalex_batch("{ not json").is_err());
        assert!(parse_openalex_batch(r#"{ "no_results": 1 }"#).is_err());
    }
}
