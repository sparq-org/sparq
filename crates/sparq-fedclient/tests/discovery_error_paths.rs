//! sq-bif.2 — capability-discovery orchestration error-path + edge-case suite.
//!
//! The `discovery` module's inline tests cover the happy SD round-trip, the ASK fallback when
//! nothing is published, and an unreachable endpoint. This file targets the discovery
//! ORCHESTRATION branches the inline suite leaves uncovered, driving the public
//! [`discover`](sparq_fedclient::discovery::discover) against a [`MapFetcher`] double (no
//! network):
//!
//!  * a SD GET that **succeeds but returns malformed N-Triples** (the parse error propagates,
//!    unlike a transport-failed SD GET which falls through to the ASK probe);
//!  * the **VoID-without-SD** path: no Service Description, but a served VoID document AND an
//!    ASK probe — the descriptor is populated from VoID while the capability comes from the
//!    ASK fallback;
//!  * an SD whose `sd:Service` carries **no `sd:supportedLanguage`** (the recall-safe
//!    unknown-version capability);
//!  * the VoID best-effort branch: a **malformed VoID** is silently ignored (descriptor stays
//!    `None`), not fatal;
//!  * [`well_known_void_url`] over query-string / fragment / IPv6-authority / no-path endpoints;
//!  * the `parse_ask_boolean` behaviour through `discover` (the strict scanner accepts only an
//!    exact JSON `true`/`false` literal at a value boundary; sq-2gfe), and the
//!    [`Capability::ask_fallback`] / [`MediaType`] units the round-trip exercises only implicitly.
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-bif.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_fedclient::discovery::{
    discover, parse_service_description, well_known_void_url, Capability, FilterClass, Interface,
    MapFetcher, MediaType, Provenance, SparqlVersion,
};

/// `ASK { ?s ?p ?o }` — the FedX reachability probe `discover` issues. The URL form is
/// `{endpoint}?query={percent-encoded}`; this helper builds the exact key `MapFetcher` matches.
const ASK_PROBE: &str = "ASK { ?s ?p ?o }";
fn ask_url(endpoint: &str) -> String {
    let mut q = String::new();
    for b in ASK_PROBE.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                q.push(b as char)
            }
            _ => q.push_str(&format!("%{:02X}", b)),
        }
    }
    format!("{}?query={}", endpoint, q)
}

/// A minimal served-shape Service Description naming a SPARQL 1.1 endpoint.
const SD_SPARQL11: &str = r#"<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL11Query> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#resultFormat> <http://www.w3.org/ns/formats/SPARQL_Results_JSON> .
"#;

/// A served-shape VoID document (the `from_void_nt` consumer's input).
const VOID_NT: &str = r#"<http://host/.well-known/void#dataset> <http://rdfs.org/ns/void#triples> "300"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://host/.well-known/void#dataset> <http://rdfs.org/ns/void#propertyPartition> _:p1 .
_:p1 <http://rdfs.org/ns/void#property> <http://xmlns.com/foaf/0.1/knows> .
_:p1 <http://rdfs.org/ns/void#triples> "200"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;

// ─── discover() orchestration error / edge paths ───────────────────────────────────────

#[test]
fn discover_propagates_malformed_service_description() {
    // A SD GET that SUCCEEDS (200) but returns malformed N-Triples: the parse error propagates,
    // because a successful body that fails to parse is a hard error (distinct from a *transport*
    // failure on the SD GET, which is non-fatal and falls through to the ASK probe).
    let endpoint = "http://host/sparql";
    let fetcher = MapFetcher::new().with(endpoint, "this is not <ntriples at all");
    let err = discover(endpoint, &fetcher).unwrap_err();
    assert!(
        err.contains("service-description") || err.contains("malformed"),
        "a malformed served SD must surface a parse error, got {}",
        err
    );
}

#[test]
fn discover_void_without_sd_uses_ask_fallback_but_keeps_descriptor() {
    // No SD (the endpoint 400s a no-query GET, modelled as an unregistered URL), but a served
    // VoID AND an ASK probe. The capability is the conservative ASK fallback; the descriptor
    // IS populated from the served VoID — VoID stats and SD capability are independent.
    let endpoint = "http://host/sparql";
    let fetcher = MapFetcher::new()
        .with("http://host/.well-known/void", VOID_NT)
        .with(ask_url(endpoint), r#"{"head":{},"boolean":true}"#);
    let d = discover(endpoint, &fetcher).expect("VoID + reachable ASK ⇒ discovers");
    assert_eq!(
        d.provenance,
        Provenance::AskProbe,
        "no SD ⇒ the capability comes from the ASK fallback"
    );
    // …but the descriptor is the served VoID's statistics, not invented.
    let desc = d
        .descriptor
        .expect("a served VoID ⇒ a descriptor even on the ASK-fallback path");
    let p = desc
        .predicate("http://xmlns.com/foaf/0.1/knows")
        .expect("served property partition");
    assert_eq!(p.triples, 200);
}

#[test]
fn discover_malformed_void_is_silently_ignored() {
    // A served SD (capability from SD) but a MALFORMED VoID: the VoID fetch is best-effort, so
    // the descriptor is silently None — discovery still succeeds with the SD capability.
    let endpoint = "http://host/sparql";
    let fetcher = MapFetcher::new()
        .with(endpoint, SD_SPARQL11)
        .with("http://host/.well-known/void", "garbage <not void");
    let d = discover(endpoint, &fetcher).expect("SD present ⇒ discovers even with bad VoID");
    assert_eq!(d.provenance, Provenance::ServiceDescription);
    assert!(
        d.descriptor.is_none(),
        "a malformed VoID must be silently ignored, not fatal, not invented"
    );
}

#[test]
fn discover_sd_present_but_no_void_leaves_descriptor_none() {
    // A served SD, no VoID registered at all (404). The capability is the SD; the descriptor is
    // None (no statistics published).
    let endpoint = "http://host/sparql";
    let fetcher = MapFetcher::new().with(endpoint, SD_SPARQL11);
    let d = discover(endpoint, &fetcher).expect("SD-only endpoint discovers");
    assert_eq!(d.provenance, Provenance::ServiceDescription);
    assert_eq!(d.capability.sparql_version, Some(SparqlVersion::Sparql11));
    assert!(d.descriptor.is_none(), "no VoID served ⇒ no descriptor");
}

#[test]
fn discover_unreachable_when_nothing_answers() {
    // No SD, no VoID, no ASK probe registered (every GET 404s) ⇒ not a reachable SPARQL service.
    let fetcher = MapFetcher::new();
    assert!(
        discover("http://dead/sparql", &fetcher).is_err(),
        "an endpoint that answers nothing must be a discovery error"
    );
}

#[test]
fn discover_ask_false_still_proves_reachability() {
    // A false ASK answer STILL proves a live SPARQL service (it evaluated the query).
    let endpoint = "http://empty/sparql";
    let fetcher = MapFetcher::new().with(ask_url(endpoint), r#"{"boolean": false}"#);
    let d = discover(endpoint, &fetcher).expect("a false ASK proves reachability");
    assert_eq!(d.provenance, Provenance::AskProbe);
}

#[test]
fn discover_ask_response_without_boolean_is_unreachable() {
    // An ASK URL that returns a body with NO `boolean` field is not a valid ASK answer, so the
    // endpoint is treated as unreachable (the parse_ask_boolean failure path through discover).
    let endpoint = "http://confused/sparql";
    let fetcher = MapFetcher::new().with(ask_url(endpoint), r#"{"results":{"bindings":[]}}"#);
    assert!(
        discover(endpoint, &fetcher).is_err(),
        "an ASK response with no boolean field ⇒ not a reachable SPARQL endpoint"
    );
}

// ─── parse_service_description — partial / unknown-version SD shapes ─────────────────────

#[test]
fn sd_with_service_but_no_language_is_recall_safe_unknown_version() {
    // An sd:Service typed node with NO sd:supportedLanguage: version is unknown (None), and the
    // recall-safe capability keeps FILTER pushable (Full) but does NOT claim aggregate / path /
    // order-limit push (those are only granted to a declared SPARQL 1.1 service).
    let nt = "<http://h/s> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n";
    let cap = parse_service_description(nt)
        .expect("well-formed N-Triples")
        .expect("an sd:Service is present");
    assert_eq!(cap.interface, Interface::Endpoint);
    assert_eq!(cap.sparql_version, None, "no supportedLanguage ⇒ unknown version");
    assert_eq!(cap.pushable_filters, FilterClass::Full);
    assert!(
        !cap.aggregates && !cap.property_paths && !cap.order_limit,
        "unknown version ⇒ no aggregate/path/order-limit push claim (recall-safe)"
    );
}

#[test]
fn sd_with_no_result_formats_does_not_claim_srj() {
    // An sd:Service with no sd:resultFormat: result_formats is empty, so returns_sparql_results_json
    // is false (the parser does not fabricate a format the SD did not advertise).
    let nt = "<http://h/s> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n";
    let cap = parse_service_description(nt).unwrap().unwrap();
    assert!(cap.result_formats.is_empty());
    assert!(
        !cap.returns_sparql_results_json(),
        "no advertised resultFormat ⇒ returns_sparql_results_json is false"
    );
}

#[test]
fn sd_ignores_unknown_feature_iri() {
    // An unknown sd:feature IRI (neither BasicFederatedQuery nor the PROV lineage feature) is
    // silently ignored — the recognised flags stay at their recall-safe defaults.
    let nt = format!(
        "{SD_SPARQL11}<http://host/sparql> \
         <http://www.w3.org/ns/sparql-service-description#feature> \
         <http://example.org/some-unknown-feature> .\n"
    );
    let cap = parse_service_description(&nt).unwrap().unwrap();
    assert!(
        !cap.federated_query,
        "an unknown feature IRI must NOT set federated_query"
    );
    assert!(
        !cap.provenance_lineage,
        "an unknown feature IRI must NOT set provenance_lineage"
    );
}

#[test]
fn parse_sd_over_a_void_document_returns_none() {
    // A pure VoID document (no sd:Service) is NOT a Service Description ⇒ Ok(None), so the caller
    // falls back to the ASK probe rather than treating VoID as a capability source.
    let cap = parse_service_description(VOID_NT).expect("well-formed N-Triples");
    assert!(cap.is_none(), "no sd:Service ⇒ not an SD ⇒ None");
}

// ─── well_known_void_url — authority extraction edge cases ──────────────────────────────

#[test]
fn well_known_void_url_handles_authority_edge_cases() {
    // Query string and fragment after the path do not pollute the authority.
    assert_eq!(
        well_known_void_url("http://host/sparql?default-graph-uri=x").as_deref(),
        Some("http://host/.well-known/void")
    );
    assert_eq!(
        well_known_void_url("http://host/sparql#frag").as_deref(),
        Some("http://host/.well-known/void")
    );
    // A non-standard port is preserved in the authority.
    assert_eq!(
        well_known_void_url("https://host:8080/db/sparql").as_deref(),
        Some("https://host:8080/.well-known/void")
    );
    // An IPv6-literal authority keeps its brackets.
    assert_eq!(
        well_known_void_url("http://[::1]/sparql").as_deref(),
        Some("http://[::1]/.well-known/void")
    );
    // An authority with no path still derives the well-known URL.
    assert_eq!(
        well_known_void_url("http://host").as_deref(),
        Some("http://host/.well-known/void")
    );
    // No scheme / authority ⇒ None.
    assert_eq!(well_known_void_url("not-a-url"), None);
    assert_eq!(well_known_void_url("http:///nohost"), None);
}

// ─── Capability::ask_fallback + MediaType units ─────────────────────────────────────────

#[test]
fn ask_fallback_capability_is_conservative() {
    // The FedX-style fallback: full SPARQL 1.1 + VALUES bind-join + SRJ, but NO aggregate /
    // path / order-limit / update / federated-query / provenance push claim (recall-safe — an
    // ASK-only endpoint published no SD).
    let c = Capability::ask_fallback();
    assert_eq!(c.interface, Interface::Endpoint);
    assert_eq!(c.sparql_version, Some(SparqlVersion::Sparql11));
    assert_eq!(c.pushable_filters, FilterClass::Full);
    assert!(c.returns_sparql_results_json(), "SRJ is the mandatory format");
    assert!(
        !c.aggregates && !c.property_paths && !c.order_limit,
        "the ASK fallback makes NO push claim beyond core evaluation"
    );
    assert!(!c.update && !c.federated_query && !c.provenance_lineage);
}

#[test]
fn media_type_recognises_sparql_results_json_only() {
    let srj = MediaType("http://www.w3.org/ns/formats/SPARQL_Results_JSON".to_string());
    assert_eq!(srj.iri(), "http://www.w3.org/ns/formats/SPARQL_Results_JSON");
    assert!(srj.is_sparql_results_json());
    let turtle = MediaType("http://www.w3.org/ns/formats/Turtle".to_string());
    assert!(
        !turtle.is_sparql_results_json(),
        "a non-SRJ format must not be recognised as SRJ"
    );
}
