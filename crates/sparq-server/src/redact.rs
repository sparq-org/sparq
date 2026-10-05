//! [OPUS-4.8] (sq-toze.34, epic sq-toze) **Request-log redaction** — keep SPARQL query / update
//! text and other request-body content OUT of the `--verbose` request log.
//!
//! ## The exposure
//!
//! With `--verbose` the server installs a `tower_http::trace::TraceLayer`, whose default span
//! records the **request URI** at DEBUG. For the SPARQL Protocol's `GET /sparql?query=…` form
//! (and the url-encoded `POST` form, whose `query=`/`update=` field is likewise a URI-shaped
//! component when echoed), the *full SPARQL query text lives in the URI query string*. SPARQL
//! query/update text can carry PII or otherwise-sensitive content — a patient IRI, an email in
//! a `FILTER`, a literal in an `INSERT DATA`. Logging it verbatim writes that content into the
//! operator's logs, where it is retained, shipped to a SIEM, and read by whoever can read logs:
//! a privacy / PII exposure distinct from, and complementary to, the #241 / sq-kfel
//! *error-body* sanitisation (which governs what a CLIENT sees on an error).
//!
//! This is the same lesson the audit log (`crate::audit`) already applies to its records: it
//! logs a non-reversible *fingerprint* of the query, never the text. Redaction extends that
//! discipline to the `--verbose` request log.
//!
//! ## What this does
//!
//! When redaction is on (the default — see `ServerConfig::redact_logs`),
//! the request log's URI has its **query string replaced** by a placeholder that preserves the
//! length and a stable non-reversible fingerprint of the original — `?<redacted len=N
//! fp=xxxxxxxxxxxxxxxx>` — so logs stay useful for correlation (same query => same `fp`; a size
//! signal from `len`) **without exposing the content**. The path is left intact (it is a route,
//! not user content). When redaction is off (`--log-full-requests` / `SPARQ_LOG_FULL_REQUESTS`
//! truthy), the URI is logged verbatim, exactly as the bare `TraceLayer` did before — the escape
//! hatch for operators who deliberately want full request text in a debug session.
//!
//! ## Honest boundary — log-CONTENT redaction, not anonymity
//!
//! This redacts the *content* of the request as it appears in the log. It does **not** anonymise
//! the request: the log still records — necessarily, to be a request log at all — the **method,
//! the path / endpoint, the response status, the size signal, and the timing**. An adversary with
//! the redacted log still learns *that* a request of roughly-this-size hit *this* endpoint at
//! *this* time, and (via `fp`) that the same query recurred. That is metadata, and it is not
//! erased here. This is also NOT the ZK/MPC privacy story (which concerns what a *remote party*
//! can learn from a *computation*); it is purely operator-log hygiene. The fingerprint is the
//! same FNV-1a construction the audit log uses (`crate::audit::query_fingerprint`) — fixed,
//! one-way, dependency-free, stable across builds and restarts.

/// A 64-bit FNV-1a hash, hex-encoded. Kept independent of the (feature-gated) audit module so
/// redaction works in every build: the construction is identical (same offset basis / prime), so
/// a query's redaction fingerprint and its audit fingerprint agree. One-way: the fingerprint
/// cannot be reversed to the query text.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Redact the query string of a request `path?query` target, leaving the path intact.
///
/// The path is a route, not user content, so it is preserved verbatim (an operator must be able
/// to see *which endpoint* was hit). The query string — where `GET /sparql?query=…` carries the
/// SPARQL text — is replaced by a `<redacted len=N fp=…>` placeholder that preserves the byte
/// length and a stable non-reversible fingerprint, so the log stays correlation-useful without
/// exposing content. A target with no `?` is returned unchanged (nothing to redact). A target
/// with an empty query (`path?`) records `len=0`.
pub fn redact_target(target: &str) -> String {
    match target.split_once('?') {
        Some((path, query)) => {
            format!(
                "{path}?<redacted len={} fp={}>",
                query.len(),
                fnv1a_hex(query.as_bytes())
            )
        }
        None => target.to_string(),
    }
}

#[cfg(feature = "server")]
mod span {
    use super::redact_target;
    use tower_http::trace::MakeSpan;

    /// [OPUS-4.8] sq-toze.34: a [`MakeSpan`] for the `--verbose` request log that records a
    /// **redacted** request target — the path verbatim, the query string replaced by the
    /// length+fingerprint placeholder ([`redact_target`]). Used in place of tower-http's
    /// `DefaultMakeSpan` (which records the raw URI) when [`ServerConfig::redact_logs`] is on.
    ///
    /// [`ServerConfig::redact_logs`]: crate::ServerConfig
    #[derive(Clone, Copy, Debug)]
    pub struct RedactingMakeSpan;

    impl<B> MakeSpan<B> for RedactingMakeSpan {
        fn make_span(&mut self, request: &axum::http::Request<B>) -> tracing::Span {
            // path_and_query() is what the bare TraceLayer's DefaultMakeSpan exposes via
            // request.uri(); we record the redacted form under the same `uri` field name so
            // downstream log parsing is unchanged in shape, only in content.
            let target = request
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_else(|| request.uri().path().to_string());
            tracing::debug_span!(
                "request",
                method = %request.method(),
                uri = %redact_target(&target),
                version = ?request.version(),
            )
        }
    }
}

#[cfg(feature = "server")]
pub use span::RedactingMakeSpan;

#[cfg(test)]
mod tests {
    use super::{fnv1a_hex, redact_target};

    const SECRET: &str = "SELECT ?s WHERE { ?s <http://ex/email> \"alice@example.com\" }";

    #[test]
    fn redacts_query_string_content() {
        let target = format!("/sparql?query={SECRET}");
        let out = redact_target(&target);
        // The path survives; the sensitive query text does NOT appear.
        assert!(out.starts_with("/sparql?"), "path preserved: {out}");
        assert!(
            !out.contains("alice@example.com"),
            "PII literal must be absent from redacted target: {out}"
        );
        assert!(
            !out.contains("SELECT"),
            "query text must be absent from redacted target: {out}"
        );
        assert!(out.contains("len="), "length signal retained: {out}");
        assert!(out.contains("fp="), "fingerprint retained: {out}");
    }

    #[test]
    fn fingerprint_is_stable_and_correlating() {
        let a = redact_target("/sparql?query=ASK%7B%7D");
        let b = redact_target("/sparql?query=ASK%7B%7D");
        let c = redact_target("/sparql?query=ASK%20%7B%7D");
        assert_eq!(a, b, "same query => same redaction (correlatable)");
        assert_ne!(a, c, "different query => different fingerprint");
    }

    #[test]
    fn no_query_string_is_unchanged() {
        assert_eq!(redact_target("/metrics"), "/metrics");
        assert_eq!(redact_target("/sparql"), "/sparql");
    }

    #[test]
    fn empty_query_records_zero_length() {
        assert_eq!(redact_target("/sparql?"), "/sparql?<redacted len=0 fp=cbf29ce484222325>");
    }

    #[test]
    fn fingerprint_matches_fnv1a() {
        // Sanity: the redaction fp is the FNV-1a of the raw query string bytes.
        let q = "query=ASK%7B%7D";
        let out = redact_target(&format!("/sparql?{q}"));
        assert!(out.ends_with(&format!("fp={}>", fnv1a_hex(q.as_bytes()))));
    }
}
