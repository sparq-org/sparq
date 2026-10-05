//! **SPARQL 1.1 Protocol drivers for external engines** (`protocol-drivers` feature).
//! [FABLE-5] sq-gum8.6
//!
//! The off-CI bug-hunting campaign points these drivers at live endpoints (Jena/Fuseki,
//! Virtuoso, Blazegraph, GraphDB, QLever, Oxigraph, MillenniumDB, and sparq's own
//! server). **CI never opens a socket**: the request-construction layer
//! ([`build_request_parts`]) is a pure function unit-tested without a network, and the
//! HTTP send in [`HttpSparqlEngine`] is a thin dispatch over it.
//!
//! Implemented here:
//!
//! * [`EndpointConfig`] — declarative endpoint description (URL, query method, extra
//!   parameters, headers, timeout), with a preset per campaign engine.
//! * A **quirk layer** for engines that deviate from the plain protocol — a
//!   non-default URL shape ([`EndpointConfig::blazegraph`]'s namespace path,
//!   [`EndpointConfig::graphdb`]'s repository path, [`EndpointConfig::qlever`]'s bare
//!   server root), a non-default transmission method ([`EndpointConfig::qlever`],
//!   [`EndpointConfig::millenniumdb`]), or an extra parameter needed for a comparable
//!   answer ([`EndpointConfig::virtuoso`]'s `format=`, [`EndpointConfig::graphdb`]'s
//!   `infer=false`).
//! * [`PresetEvidence`] — per-config provenance, because an unvalidated preset is a
//!   *silent* misconfiguration source in a campaign. **Every preset constructor returns
//!   [`PresetEvidence::UpstreamDocs`]**: a preset encodes a documented convention, and
//!   building one contacts nothing, so it cannot know whether the URL it was handed
//!   answers. [`PresetEvidence::LiveInstance`] is recorded only by
//!   [`EndpointConfig::confirmed_live`], on the one configuration that just completed an
//!   exchange; `tests/preset_live_conformance.rs` is the opt-in, off-CI probe that does
//!   that promotion.
//!
//! | Preset | URL shape | Method | Quirk |
//! | --- | --- | --- | --- |
//! | [`fuseki`](EndpointConfig::fuseki) | `{base}/{dataset}/query` | form POST | — |
//! | [`oxigraph`](EndpointConfig::oxigraph) | `{base}/query` | form POST | — |
//! | [`virtuoso`](EndpointConfig::virtuoso) | `{base}/sparql` | form POST | `format=` |
//! | [`blazegraph`](EndpointConfig::blazegraph) | `{base}/namespace/{ns}/sparql` | form POST | namespace path |
//! | [`graphdb`](EndpointConfig::graphdb) | `{base}/repositories/{repo}` | form POST | `infer=false` |
//! | [`qlever`](EndpointConfig::qlever) | `{base}` (server root) | GET | no path suffix |
//! | [`millenniumdb`](EndpointConfig::millenniumdb) | `{base}/sparql` | raw-body POST | `application/sparql-query` |
//!
//! [OPUS-5] The Blazegraph / GraphDB / QLever / MillenniumDB presets, their quirk
//! layers, and [`PresetEvidence`] are bead `sq-gum8.10`.

use std::time::Duration;

use sparq_difftest::{parse_results_json, QueryResults};

use crate::engine::SparqlEngine;
use crate::verdict::{EngineFailure, FailureKind};

/// How the query is transmitted (SPARQL 1.1 Protocol §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMethod {
    /// `POST` with `Content-Type: application/sparql-query`, query as the raw body
    /// (§2.1.3 "query via POST directly").
    PostSparqlQuery,
    /// `POST` with `Content-Type: application/x-www-form-urlencoded`, query in a
    /// `query=` field (§2.1.2 — the most broadly supported method; mirrors
    /// sparq-engine's own `SERVICE` transport choice).
    PostForm,
    /// `GET` with the query URL-encoded in the query string (§2.1.1; subject to URL
    /// length limits — the shape some engines are driven with by default, see
    /// [`EndpointConfig::qlever`]).
    Get,
}

/// What backs **this configuration's** URL shape, transmission method, and negotiation
/// choices — a claim about the endpoint this config actually names, not about the
/// preset convention in the abstract.
///
/// A wrong URL or quirk convention is a *silent* misconfiguration source: a bad path
/// usually surfaces loudly as [`FailureKind::HttpStatus`], but a subtly wrong parameter
/// — a `format=` the engine ignores, an inference flag left at a non-comparable default
/// — instead yields results that differ from every other engine for reasons that are not
/// an engine bug. So a [`crate::ledger`] entry raised through anything other than
/// [`PresetEvidence::LiveInstance`] should have its endpoint configuration ruled out
/// before it is filed upstream.
///
/// Constructing a config opens no socket, so **no constructor in this module ever
/// returns `LiveInstance`** — only [`EndpointConfig::confirmed_live`] does, after a
/// caller has completed an exchange with that very config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetEvidence {
    /// Not a preset — the caller supplied the URL and method
    /// ([`EndpointConfig::generic`]), and nothing here vouches for either.
    CallerSupplied,
    /// This configuration answered an actual request/response exchange. Recorded
    /// **only** by [`EndpointConfig::confirmed_live`]; a config that merely uses a
    /// preset whose convention was once checked elsewhere is `UpstreamDocs`, not this.
    LiveInstance,
    /// The conventions come from the engine's upstream documentation or source, applied
    /// to a URL this crate has not contacted. The state every preset constructor
    /// returns — run the ignored `preset_live_conformance` integration test to promote a
    /// concrete config to [`PresetEvidence::LiveInstance`].
    UpstreamDocs,
}

/// Declarative description of one SPARQL-protocol endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointConfig {
    /// Engine name for verdicts / the ledger.
    pub name: String,
    /// Absolute query endpoint URL.
    pub query_url: String,
    /// Transmission method.
    pub method: QueryMethod,
    /// Extra `key=value` pairs appended to the form body / query string (engine
    /// quirks, e.g. Virtuoso's `format=`).
    pub extra_params: Vec<(String, String)>,
    /// Extra request headers beyond the standard `Accept`.
    pub headers: Vec<(String, String)>,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Maximum response body size in bytes. Requests returning more data are truncated
    /// via `ureq`'s body-read limiter and treated as a transport error. Defaults to
    /// 64 MiB — sufficient for any sane SPARQL result set; raise for unusually large
    /// results in a campaign run (e.g. full-graph dumps). [SONNET-4.6]
    pub max_body_bytes: u64,
    /// What backs this configuration's conventions — see [`PresetEvidence`].
    pub evidence: PresetEvidence,
}

impl EndpointConfig {
    /// A plain SPARQL 1.1 Protocol endpoint: form-encoded POST, no quirks.
    pub fn generic(name: &str, query_url: &str) -> Self {
        EndpointConfig {
            name: name.to_string(),
            query_url: query_url.to_string(),
            method: QueryMethod::PostForm,
            extra_params: Vec::new(),
            headers: Vec::new(),
            timeout: Duration::from_secs(60),
            max_body_bytes: 64 * 1024 * 1024,
            evidence: PresetEvidence::CallerSupplied,
        }
    }

    /// [`EndpointConfig::generic`] carrying a preset's name, URL, and provenance.
    fn preset(name: &str, query_url: String, evidence: PresetEvidence) -> Self {
        EndpointConfig {
            evidence,
            ..EndpointConfig::generic(name, &query_url)
        }
    }

    /// Apache Jena Fuseki: standard protocol at `{base}/{dataset}/query`.
    pub fn fuseki(base_url: &str, dataset: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        EndpointConfig::preset(
            "fuseki",
            format!("{base}/{dataset}/query"),
            PresetEvidence::UpstreamDocs,
        )
    }

    /// Oxigraph server: standard protocol at `{base}/query`.
    pub fn oxigraph(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        EndpointConfig::preset(
            "oxigraph",
            format!("{base}/query"),
            PresetEvidence::UpstreamDocs,
        )
    }

    /// OpenLink Virtuoso quirk layer: `{base}/sparql`, form-encoded POST, plus an
    /// explicit `format=application/sparql-results+json` output parameter (Virtuoso's
    /// documented output-selection parameter; bare-`Accept` negotiation is unreliable
    /// on some deployments).
    pub fn virtuoso(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        let mut config = EndpointConfig::preset(
            "virtuoso",
            format!("{base}/sparql"),
            PresetEvidence::UpstreamDocs,
        );
        config.extra_params.push((
            "format".to_string(),
            "application/sparql-results+json".to_string(),
        ));
        config
    }

    /// Blazegraph (NanoSparqlServer) quirk layer: the **namespace-scoped** endpoint
    /// `{base}/namespace/{namespace}/sparql`, where `base_url` includes the web
    /// application's context path — `/blazegraph` for the 2.x standalone jar,
    /// `/bigdata` for the older WAR deployments — and `namespace` names the KB instance
    /// (Blazegraph's own default is `kb`). Pointing at `{base}/sparql` instead silently
    /// targets whichever namespace the deployment made default, which is why the
    /// namespace is a required argument here rather than an optional suffix.
    ///
    /// Form-encoded POST with a plain `Accept` header is sufficient. Blazegraph's
    /// *default* output is SPARQL-Results-XML, but it honours `Accept`, so the REST
    /// API's Accept-overriding `format=` parameter is deliberately **not** set — one
    /// less quirk to keep in sync.
    ///
    /// Evidence: [`PresetEvidence::UpstreamDocs`], like every preset — the returned
    /// config names a `base_url`/`namespace` this crate has not contacted. The
    /// *convention* above was additionally checked once against the Wikidata Query
    /// Service, a public Blazegraph deployment, at
    /// `https://query.wikidata.org/bigdata/namespace/wdq/sparql` — `ASK {}` over
    /// form-encoded POST, GET, and raw `application/sparql-query` POST each returned
    /// HTTP 200 with `Content-Type: application/sparql-results+json` when `Accept`
    /// asked for it, and `application/sparql-results+xml` when it did not. That says
    /// nothing about *your* deployment; use [`EndpointConfig::confirmed_live`] after a
    /// successful exchange to record evidence for a concrete config.
    pub fn blazegraph(base_url: &str, namespace: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        EndpointConfig::preset(
            "blazegraph",
            format!("{base}/namespace/{namespace}/sparql"),
            PresetEvidence::UpstreamDocs,
        )
    }

    /// Ontotext GraphDB quirk layer (RDF4J REST protocol): the repository URL *is* the
    /// query endpoint, `{base}/repositories/{repository}` with no `/query` suffix
    /// (updates go to `{base}/repositories/{repository}/statements`). GraphDB's default
    /// `graphdb.connector.port` is 7200.
    ///
    /// Quirk: RDF4J's `infer` parameter defaults to **true**, so a repository with a
    /// ruleset answers over entailed statements as well as asserted ones. That is not a
    /// wrong answer, but it makes GraphDB incomparable with engines that do no
    /// inference — [`crate::differential`] would report every entailed row as a
    /// mismatch. The preset pins `infer=false` so the endpoint answers over explicit
    /// statements only; drop it from `extra_params` to hunt bugs in the entailment path
    /// instead (self-referential oracles like [`crate::tlp`] stay valid either way).
    pub fn graphdb(base_url: &str, repository: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        let mut config = EndpointConfig::preset(
            "graphdb",
            format!("{base}/repositories/{repository}"),
            PresetEvidence::UpstreamDocs,
        );
        config
            .extra_params
            .push(("infer".to_string(), "false".to_string()));
        config
    }

    /// QLever quirk layer: the SPARQL endpoint is the server **root** — there is no
    /// `/sparql` path suffix (QLever names only `/ping`, `/metrics`, and its Graph Store
    /// route), and the port is whatever the server was started with, as QLever's
    /// `--port` is a required argument with no default. So `base_url` is used verbatim,
    /// minus any trailing slash.
    ///
    /// Quirk: [`QueryMethod::Get`]. QLever accepts all three SPARQL 1.1 Protocol forms;
    /// GET is the shape its own UI and public API endpoints are driven with, and it
    /// keeps the query in the request line for endpoint-side logging. A long generated
    /// case can outgrow a deployment's URL-length limit — set
    /// `.method = QueryMethod::PostForm` for those runs. Note that QLever enforces
    /// §2.1.2 for a form-encoded POST (the URL query string must then be empty), so do
    /// not hand this preset a `base_url` that already carries parameters.
    ///
    /// No `format=`/`action=` override is set: QLever honours `Accept` and already
    /// defaults to `application/sparql-results+json` for SELECT and ASK.
    pub fn qlever(base_url: &str) -> Self {
        let mut config = EndpointConfig::preset(
            "qlever",
            base_url.trim_end_matches('/').to_string(),
            PresetEvidence::UpstreamDocs,
        );
        config.method = QueryMethod::Get;
        config
    }

    /// MillenniumDB quirk layer: `{base}/sparql` on the `mdb server` protocol port
    /// (1234 by default — the separate 4321 port serves the web interface, not the
    /// protocol endpoint).
    ///
    /// Quirk: [`QueryMethod::PostSparqlQuery`]. The documented request form is a POST
    /// with `Content-Type: application/sparql-query` carrying the query as the raw
    /// body; support for a form-encoded `query=` field is not documented, so the preset
    /// does not assume it. `Accept` is honoured for output selection.
    pub fn millenniumdb(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        let mut config = EndpointConfig::preset(
            "millenniumdb",
            format!("{base}/sparql"),
            PresetEvidence::UpstreamDocs,
        );
        config.method = QueryMethod::PostSparqlQuery;
        config
    }

    /// Record that **this** configuration completed a real request/response exchange:
    /// sets [`evidence`](EndpointConfig::evidence) to [`PresetEvidence::LiveInstance`].
    ///
    /// The only way that variant is ever produced. Call it *after* a successful,
    /// parseable protocol exchange with the endpoint this config names — that is what
    /// `tests/preset_live_conformance.rs` does — never on the strength of a preset whose
    /// convention was checked against some other deployment.
    #[must_use]
    pub fn confirmed_live(mut self) -> Self {
        self.evidence = PresetEvidence::LiveInstance;
        self
    }
}

/// The fully constructed request, before any socket is opened. Pure data — this is the
/// seam the no-network CI tests check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestParts {
    /// Final URL (including the query string for [`QueryMethod::Get`]).
    pub url: String,
    /// `"GET"` or `"POST"`.
    pub http_method: &'static str,
    /// `Content-Type` for POST bodies (`None` for GET).
    pub content_type: Option<&'static str>,
    /// Request body (`None` for GET).
    pub body: Option<String>,
    /// Extra headers from the config (the standard `Accept` is added at send time).
    pub headers: Vec<(String, String)>,
}

/// Percent-encode one value for `application/x-www-form-urlencoded` / a URL query
/// string: unreserved characters (RFC 3986 §2.3) pass through, space becomes `+`,
/// everything else becomes `%XX` per UTF-8 byte.
pub fn form_urlencode(value: &str) -> String {
    // The RFC 3986 unreserved keep-set, with ' ' ALSO kept so the one-pass encoder leaves it
    // for the `+` swap below (`percent-encoding` has no native `+` mode). A literal '+' in the
    // input is itself encoded (`%2B`), so the swap is unambiguous (#3712).
    const FORM: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~')
        .remove(b' ');
    percent_encoding::utf8_percent_encode(value, FORM)
        .to_string()
        .replace(' ', "+")
}

/// Append an already-encoded query-string `params` to `base_url`, using `?` when
/// `base_url` has no query string yet and `&` when it already has one.  A pre-existing
/// trailing `?` is treated the same as having a query string (we just append). [SONNET-4.6]
fn append_query_string(base_url: &str, params: &str) -> String {
    if base_url.contains('?') {
        format!("{}&{}", base_url, params)
    } else {
        format!("{}?{}", base_url, params)
    }
}

fn encode_params(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_urlencode(k), form_urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the request for `sparql` against `config` — pure, no network.
pub fn build_request_parts(config: &EndpointConfig, sparql: &str) -> RequestParts {
    match config.method {
        QueryMethod::PostSparqlQuery => {
            // Extra params ride on the URL for a raw-body POST.
            let url = if config.extra_params.is_empty() {
                config.query_url.clone()
            } else {
                append_query_string(&config.query_url, &encode_params(&config.extra_params))
            };
            RequestParts {
                url,
                http_method: "POST",
                content_type: Some("application/sparql-query"),
                body: Some(sparql.to_string()),
                headers: config.headers.clone(),
            }
        }
        QueryMethod::PostForm => {
            let mut pairs = vec![("query".to_string(), sparql.to_string())];
            pairs.extend(config.extra_params.iter().cloned());
            RequestParts {
                url: config.query_url.clone(),
                http_method: "POST",
                content_type: Some("application/x-www-form-urlencoded"),
                body: Some(encode_params(&pairs)),
                headers: config.headers.clone(),
            }
        }
        QueryMethod::Get => {
            let mut pairs = vec![("query".to_string(), sparql.to_string())];
            pairs.extend(config.extra_params.iter().cloned());
            RequestParts {
                url: append_query_string(&config.query_url, &encode_params(&pairs)),
                http_method: "GET",
                content_type: None,
                body: None,
                headers: config.headers.clone(),
            }
        }
    }
}

/// A live SPARQL-protocol engine. Campaign use only — CI self-tests never construct a
/// request over the network (the pure layer above carries the tested logic).
pub struct HttpSparqlEngine {
    config: EndpointConfig,
    agent: ureq::Agent,
}

impl HttpSparqlEngine {
    /// Build a driver over `config` with its timeout applied to the whole round trip.
    pub fn new(config: EndpointConfig) -> Self {
        let agent_config = ureq::Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .user_agent(concat!("sparq-metamorph/", env!("CARGO_PKG_VERSION")))
            .build();
        HttpSparqlEngine {
            config,
            agent: ureq::Agent::new_with_config(agent_config),
        }
    }

    fn failure(&self, query: &str, kind: FailureKind, message: String) -> EngineFailure {
        EngineFailure {
            engine: self.config.name.clone(),
            query: query.to_string(),
            kind,
            message,
        }
    }
}

const ACCEPT: &str = "application/sparql-results+json";

impl SparqlEngine for HttpSparqlEngine {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, EngineFailure> {
        let parts = build_request_parts(&self.config, sparql);
        let response = if parts.http_method == "GET" {
            let mut request = self.agent.get(&parts.url).header("Accept", ACCEPT);
            for (key, value) in &parts.headers {
                request = request.header(key, value);
            }
            request.call()
        } else {
            let mut request = self.agent.post(&parts.url).header("Accept", ACCEPT);
            if let Some(content_type) = parts.content_type {
                request = request.header("Content-Type", content_type);
            }
            for (key, value) in &parts.headers {
                request = request.header(key, value);
            }
            request.send(parts.body.as_deref().unwrap_or(""))
        };
        let body = match response {
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(self.config.max_body_bytes)
                .read_to_string()
                .map_err(|e| {
                    self.failure(sparql, FailureKind::Transport, format!("reading response: {e}"))
                })?,
            Err(ureq::Error::StatusCode(code)) => {
                return Err(self.failure(
                    sparql,
                    FailureKind::HttpStatus(code),
                    format!("endpoint returned HTTP {code}"),
                ))
            }
            Err(e) => {
                return Err(self.failure(sparql, FailureKind::Transport, e.to_string()));
            }
        };
        parse_results_json(&body)
            .map_err(|e| self.failure(sparql, FailureKind::InvalidResults, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_urlencode_covers_unreserved_space_and_utf8() {
        assert_eq!(form_urlencode("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
        assert_eq!(form_urlencode("a b"), "a+b");
        assert_eq!(form_urlencode("?v < 5 && ?w = \"é\""), "%3Fv+%3C+5+%26%26+%3Fw+%3D+%22%C3%A9%22");
    }

    /// #3712 tripwire: every printable ASCII byte + a control + 2-/4-byte UTF-8 — exactly the
    /// RFC 3986 unreserved set passes through, space becomes `+`, the rest is uppercase `%XX`.
    #[test]
    fn form_urlencode_pins_the_unreserved_set() {
        let all: String = (0x20u8..0x7f)
            .map(char::from)
            .chain("\t\né😀".chars())
            .collect();
        assert_eq!(
            form_urlencode(&all),
            "+%21%22%23%24%25%26%27%28%29%2A%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%09%0A%C3%A9%F0%9F%98%80"
        );
    }

    #[test]
    fn build_request_parts_post_form_encodes_query_and_extras() {
        let mut config = EndpointConfig::generic("g", "http://localhost:1234/sparql");
        config
            .extra_params
            .push(("format".to_string(), "application/sparql-results+json".to_string()));
        let parts = build_request_parts(&config, "SELECT * WHERE { ?s ?p ?o }");
        assert_eq!(parts.http_method, "POST");
        assert_eq!(parts.url, "http://localhost:1234/sparql");
        assert_eq!(parts.content_type, Some("application/x-www-form-urlencoded"));
        let body = parts.body.unwrap();
        assert!(body.starts_with("query=SELECT"));
        assert!(body.contains("&format=application%2Fsparql-results%2Bjson"));
    }

    #[test]
    fn build_request_parts_raw_post_keeps_the_query_verbatim() {
        let mut config = EndpointConfig::generic("g", "http://localhost:1234/sparql");
        config.method = QueryMethod::PostSparqlQuery;
        let parts = build_request_parts(&config, "ASK { ?s ?p ?o }");
        assert_eq!(parts.content_type, Some("application/sparql-query"));
        assert_eq!(parts.body.as_deref(), Some("ASK { ?s ?p ?o }"));
        assert_eq!(parts.url, "http://localhost:1234/sparql");
    }

    #[test]
    fn build_request_parts_get_puts_the_query_in_the_url() {
        let mut config = EndpointConfig::generic("g", "http://localhost:1234/sparql");
        config.method = QueryMethod::Get;
        let parts = build_request_parts(&config, "ASK { }");
        assert_eq!(parts.http_method, "GET");
        assert!(parts.url.starts_with("http://localhost:1234/sparql?query=ASK"));
        assert_eq!(parts.body, None);
        assert_eq!(parts.content_type, None);
    }

    /// A `query_url` that already carries a query string must get `&` not a second `?`.
    /// [SONNET-4.6]
    #[test]
    fn build_request_parts_no_double_question_mark_on_pre_existing_query_string() {
        // PostSparqlQuery arm: extra_params appended to a URL that already has '?'
        let mut config = EndpointConfig::generic("g", "http://localhost:1234/sparql?default-graph-uri=urn:x");
        config.method = QueryMethod::PostSparqlQuery;
        config.extra_params.push(("format".to_string(), "json".to_string()));
        let parts = build_request_parts(&config, "ASK { }");
        let url = &parts.url;
        assert!(!url.contains("??"), "double '?' in URL: {url}");
        assert!(url.contains("default-graph-uri=urn%3Ax&format=json") || url.contains("default-graph-uri=urn:x&format=json"),
            "expected '&' separator, got: {url}");

        // Get arm: query string appended to a URL that already has '?'
        let mut config2 = EndpointConfig::generic("g", "http://localhost:1234/sparql?service=default");
        config2.method = QueryMethod::Get;
        let parts2 = build_request_parts(&config2, "ASK { }");
        let url2 = &parts2.url;
        assert!(!url2.contains("??"), "double '?' in URL: {url2}");
        assert!(url2.contains("service=default&query="), "expected '&' separator, got: {url2}");
    }

    #[test]
    fn presets_encode_the_documented_conventions() {
        let fuseki = EndpointConfig::fuseki("http://localhost:3030/", "ds");
        assert_eq!(fuseki.query_url, "http://localhost:3030/ds/query");
        assert_eq!(fuseki.method, QueryMethod::PostForm);

        let oxigraph = EndpointConfig::oxigraph("http://localhost:7878");
        assert_eq!(oxigraph.query_url, "http://localhost:7878/query");

        let virtuoso = EndpointConfig::virtuoso("http://localhost:8890");
        assert_eq!(virtuoso.query_url, "http://localhost:8890/sparql");
        assert!(virtuoso
            .extra_params
            .contains(&("format".to_string(), "application/sparql-results+json".to_string())));
    }

    /// Blazegraph's namespace-scoped path — `{base}/namespace/{ns}/sparql`, NOT
    /// `{base}/sparql`, which would silently target the deployment's default namespace.
    #[test]
    fn blazegraph_preset_targets_the_namespace_scoped_endpoint() {
        let config = EndpointConfig::blazegraph("http://localhost:9999/blazegraph/", "kb");
        assert_eq!(
            config.query_url,
            "http://localhost:9999/blazegraph/namespace/kb/sparql"
        );
        assert_eq!(config.method, QueryMethod::PostForm);
        // `Accept` alone drives negotiation; no `format=` override is set.
        assert!(config.extra_params.is_empty(), "{:?}", config.extra_params);

        let parts = build_request_parts(&config, "ASK { }");
        assert_eq!(parts.http_method, "POST");
        assert_eq!(
            parts.url,
            "http://localhost:9999/blazegraph/namespace/kb/sparql"
        );
        assert_eq!(parts.content_type, Some("application/x-www-form-urlencoded"));
        assert_eq!(parts.body.as_deref(), Some("query=ASK+%7B+%7D"));
    }

    /// GraphDB: the repository URL *is* the query endpoint (no `/query` suffix), and
    /// inference is pinned off so the endpoint stays comparable with non-entailing
    /// engines under the differential oracle.
    #[test]
    fn graphdb_preset_targets_the_repository_url_and_disables_inference() {
        let config = EndpointConfig::graphdb("http://localhost:7200", "campaign");
        assert_eq!(config.query_url, "http://localhost:7200/repositories/campaign");
        assert_eq!(config.method, QueryMethod::PostForm);

        let parts = build_request_parts(&config, "SELECT * WHERE { ?s ?p ?o }");
        assert_eq!(parts.url, "http://localhost:7200/repositories/campaign");
        let body = parts.body.unwrap();
        assert!(body.starts_with("query=SELECT"), "{body}");
        assert!(body.ends_with("&infer=false"), "{body}");
    }

    /// QLever: the bare server root is the endpoint, driven by GET.
    #[test]
    fn qlever_preset_uses_get_against_the_bare_server_root() {
        let config = EndpointConfig::qlever("http://localhost:7001/");
        assert_eq!(config.query_url, "http://localhost:7001");
        assert_eq!(config.method, QueryMethod::Get);

        let parts = build_request_parts(&config, "ASK { }");
        assert_eq!(parts.http_method, "GET");
        assert_eq!(parts.url, "http://localhost:7001?query=ASK+%7B+%7D");
        assert_eq!(parts.body, None);
        assert_eq!(parts.content_type, None);
    }

    /// MillenniumDB: raw-body POST under `application/sparql-query`, query verbatim.
    #[test]
    fn millenniumdb_preset_posts_the_raw_query_to_the_sparql_path() {
        let config = EndpointConfig::millenniumdb("http://localhost:1234");
        assert_eq!(config.query_url, "http://localhost:1234/sparql");
        assert_eq!(config.method, QueryMethod::PostSparqlQuery);

        let parts = build_request_parts(&config, "SELECT * WHERE { ?s ?p ?o } LIMIT 10");
        assert_eq!(parts.http_method, "POST");
        assert_eq!(parts.url, "http://localhost:1234/sparql");
        assert_eq!(parts.content_type, Some("application/sparql-query"));
        assert_eq!(
            parts.body.as_deref(),
            Some("SELECT * WHERE { ?s ?p ?o } LIMIT 10")
        );
    }

    /// Every preset must declare its provenance: a preset left at
    /// [`PresetEvidence::CallerSupplied`] would read as "the caller chose this URL" and
    /// hide the fact that the crate is asserting a convention on the caller's behalf.
    #[test]
    fn every_preset_declares_its_evidence() {
        let presets = [
            EndpointConfig::fuseki("http://h:3030", "ds"),
            EndpointConfig::oxigraph("http://h:7878"),
            EndpointConfig::virtuoso("http://h:8890"),
            EndpointConfig::blazegraph("http://h:9999/blazegraph", "kb"),
            EndpointConfig::graphdb("http://h:7200", "r"),
            EndpointConfig::qlever("http://h:7001"),
            EndpointConfig::millenniumdb("http://h:1234"),
        ];
        for config in &presets {
            // Documentation-derived, and never `LiveInstance`: building a config
            // contacts nothing, so no constructor may claim this endpoint answered.
            assert_eq!(
                config.evidence,
                PresetEvidence::UpstreamDocs,
                "preset {} did not declare its evidence",
                config.name
            );
        }
        assert_eq!(
            EndpointConfig::generic("g", "http://h/sparql").evidence,
            PresetEvidence::CallerSupplied
        );
    }

    /// `LiveInstance` is a claim about one concrete config, so it is reachable only
    /// through the explicit promotion a caller makes after an actual exchange.
    #[test]
    fn confirmed_live_promotes_only_the_configuration_it_is_called_on() {
        let documented = EndpointConfig::blazegraph("http://h:9999/blazegraph", "kb");
        assert_eq!(documented.evidence, PresetEvidence::UpstreamDocs);

        let probed = documented.clone().confirmed_live();
        assert_eq!(probed.evidence, PresetEvidence::LiveInstance);
        // Promotion touches evidence and nothing else.
        assert_eq!(
            probed,
            EndpointConfig {
                evidence: PresetEvidence::LiveInstance,
                ..documented.clone()
            }
        );
        // A sibling config built from the same preset is untouched by that promotion.
        assert_eq!(
            EndpointConfig::blazegraph("http://other:9999/blazegraph", "kb").evidence,
            PresetEvidence::UpstreamDocs
        );
        assert_eq!(
            EndpointConfig::generic("g", "http://h/sparql")
                .confirmed_live()
                .evidence,
            PresetEvidence::LiveInstance
        );
    }

    /// Serve exactly one canned HTTP response on an ephemeral **loopback** port (no
    /// external network) and return the endpoint URL. Reads the request headers plus a
    /// `Content-Length` body before answering, so the client's write never races the
    /// response.
    fn serve_once(response: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Read until the header terminator, then drain the declared body length.
            let header_end = loop {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
                if n == 0 {
                    return;
                }
            };
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
            let content_length: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() < header_end + content_length {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://127.0.0.1:{port}/sparql")
    }

    #[test]
    fn http_engine_parses_a_valid_json_response() {
        let body = r#"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/x"}}]}}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/sparql-results+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let engine = HttpSparqlEngine::new(EndpointConfig::generic("loopback", &serve_once(response)));
        assert_eq!(engine.name(), "loopback");
        match engine.select("SELECT * WHERE { ?s ?p ?o }").unwrap() {
            QueryResults::Solutions { solutions, .. } => assert_eq!(solutions.len(), 1),
            QueryResults::Boolean(_) => panic!("SELECT returned a boolean"),
        }
    }

    #[test]
    fn http_engine_maps_a_500_to_http_status_failure() {
        let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let engine = HttpSparqlEngine::new(EndpointConfig::generic("loopback", &serve_once(response)));
        let err = engine.select("ASK { }").unwrap_err();
        assert_eq!(err.kind, FailureKind::HttpStatus(500), "fail-closed: {err:?}");
    }

    #[test]
    fn http_engine_maps_garbage_body_to_invalid_results() {
        let response =
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot json!";
        let engine = HttpSparqlEngine::new(EndpointConfig::generic("loopback", &serve_once(response)));
        let err = engine.select("ASK { }").unwrap_err();
        assert_eq!(err.kind, FailureKind::InvalidResults, "fail-closed: {err:?}");
    }
}
