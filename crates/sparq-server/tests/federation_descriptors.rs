//! [OPUS-4.8] sq-d3d8 (epic sq-3183): integration tests for the OPT-IN federation
//! discovery descriptors — the W3C VoID document at `GET /.well-known/void` and the SPARQL
//! 1.1 Service Description served for a `GET /sparql` with no `query` parameter.
//!
//! These tests run ONLY with the `federation-descriptors` cargo feature (the whole test
//! file is gated). They spin the real axum server and assert:
//!   * the OPT-IN posture — both endpoints are off unless the config flag is set;
//!   * valid RDF bodies carrying the expected triples (`void:triples`, `sd:endpoint`, …);
//!   * content negotiation (Turtle default, N-Triples on request).
#![cfg(feature = "federation-descriptors")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice a ex:Person ; ex:knows ex:bob ; ex:name "Alice" .
    ex:bob   a ex:Person ; ex:name "Bob" .
    ex:carol a ex:Person .
"#;

/// [OPUS-4.8] sq-optl: a TriG dataset with TWO IRI-named graphs (plus default-graph triples),
/// for the Service Description's `sd:namedGraph` enumeration test.
const DATA_NAMED: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice a ex:Person .
    ex:people { ex:alice ex:name "Alice" . ex:bob ex:name "Bob" . }
    ex:orgs   { ex:acme a ex:Org . }
"#;

/// Boots a server with the descriptors flag ON (or OFF) and returns its base URL.
async fn spawn(descriptors_on: bool) -> String {
    spawn_with(DATA, "turtle", descriptors_on).await
}

/// [OPUS-4.8] sq-optl: like [`spawn`] but loads `data` in `format` (so a test can boot a server
/// over a named-graph TriG dataset). Uses `load_dataset` — the only loader that PRESERVES named
/// graphs as separate sub-graphs (`load_str` collapses every quad into the default graph); it
/// falls back to `load_str` for non-dataset formats, so the Turtle path is unchanged. Keeps the
/// single spawn body so the two paths cannot drift.
async fn spawn_with(data: &str, format: &str, descriptors_on: bool) -> String {
    let graph = Graph::load_dataset(data, format).unwrap();
    let config = ServerConfig {
        federation_descriptors: descriptors_on,
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// OPT-IN posture: both endpoints are off unless the flag is set.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn void_404_when_flag_off() {
    let base = spawn(false).await;
    let resp = client()
        .get(format!("{base}/.well-known/void"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn sparql_get_no_query_is_400_when_flag_off() {
    // Historical behaviour preserved: a GET /sparql with no `query` is a 400.
    let base = spawn(false).await;
    let resp = client().get(format!("{base}/sparql")).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

// ---------------------------------------------------------------------------
// VoID document.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn void_served_as_turtle_by_default() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/.well-known/void"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    // The dataset IRI self-describes the host the client used.
    assert!(
        body.contains("/.well-known/void#dataset"),
        "VoID body: {body}"
    );
    // VoID counts present. The source graph has 6 triples (3 rdf:type + 2 ex:name + 1 ex:knows).
    assert!(
        body.contains("triples"),
        "VoID must carry void:triples: {body}"
    );
    assert!(
        body.contains("triples> 6"),
        "void:triples should be 6: {body}"
    );
    // Class partition for ex:Person (3 instances).
    assert!(
        body.contains("http://ex/Person"),
        "VoID must list the class: {body}"
    );
}

#[tokio::test]
async fn void_negotiates_ntriples() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/.well-known/void"))
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("<http://rdfs.org/ns/void#Dataset>"),
        "N-Triples VoID: {body}"
    );
    assert!(
        body.contains("<http://rdfs.org/ns/void#triples>"),
        "N-Triples VoID: {body}"
    );
}

#[tokio::test]
async fn void_carries_characteristic_set_stats() {
    // [OPUS-4.8] sq-mr32 (federation A3/Z2): the served VoID now also carries the
    // characteristic-set source statistics (sparq `scs:` extension), end-to-end over the
    // real HTTP server — so a federation client polling this node gets per-entity-type
    // predicate co-occurrence + multiplicity, not just bare VoID counts.
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/.well-known/void"))
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Standard VoID still present (CS rides alongside, never replaces it).
    assert!(
        body.contains("<http://rdfs.org/ns/void#Dataset>"),
        "VoID still present: {body}"
    );
    // Dataset links to characteristic sets + carries the exact distinct-set count.
    assert!(
        body.contains("<http://sparq.dev/ns/cs#characteristicSet>"),
        "served VoID must link characteristic sets: {body}"
    );
    assert!(
        body.contains("<http://sparq.dev/ns/cs#distinctCharacteristicSets>"),
        "served VoID must carry the distinct-set count: {body}"
    );
    // A typed CS node with subjects + per-predicate stats (property/triples/avgMult).
    assert!(
        body.contains("<http://sparq.dev/ns/cs#CharacteristicSet>"),
        "{body}"
    );
    assert!(body.contains("<http://sparq.dev/ns/cs#subjects>"), "{body}");
    assert!(
        body.contains("<http://sparq.dev/ns/cs#avgMultiplicity>"),
        "{body}"
    );
    // The CS partition reuses void:property and names a real predicate from the graph.
    assert!(
        body.contains("<http://ex/knows>"),
        "CS stat must name the predicate: {body}"
    );
    // The whole body must still be valid RDF (re-parses as N-Triples).
    let n = oxttl::NTriplesParser::new()
        .for_slice(body.as_bytes())
        .filter(|t| t.is_ok())
        .count();
    assert!(
        n >= 10,
        "VoID+CS should re-parse to many triples, got {n}: {body}"
    );
}

// ---------------------------------------------------------------------------
// SPARQL Service Description (GET /sparql with no query).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_description_served_on_get_no_query() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("Accept", "text/turtle")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    // sd:Service + sd:endpoint pointing at this /sparql.
    assert!(
        body.contains("Service"),
        "SD must declare sd:Service: {body}"
    );
    assert!(
        body.contains("/sparql"),
        "SD must carry the endpoint URL: {body}"
    );
    // Advertised query language + a result format.
    assert!(
        body.contains("SPARQL11Query"),
        "SD must advertise SPARQL11Query: {body}"
    );
    assert!(
        body.contains("formats"),
        "SD must advertise result formats: {body}"
    );
    // Link to the VoID document.
    assert!(
        body.contains("/.well-known/void#dataset"),
        "SD must link the VoID dataset: {body}"
    );
}

#[tokio::test]
async fn service_description_ntriples() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("<http://www.w3.org/ns/sparql-service-description#endpoint>"),
        "SD N-Triples must carry sd:endpoint: {body}"
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-qfcb: parse the SD as RDF and assert the MANDATORY structure +
// that the advertised capabilities match what the server genuinely serves.
// ---------------------------------------------------------------------------

const SD: &str = "http://www.w3.org/ns/sparql-service-description#";
const FMT: &str = "http://www.w3.org/ns/formats/";

/// Fetches the SD as N-Triples and parses it into a `(subject, predicate, object)` triple
/// vector (term Display strings), so a test can assert over the parsed graph rather than raw
/// substrings. Asserts the body is well-formed RDF in the process.
async fn fetch_sd_triples(base: &str) -> Vec<(String, String, String)> {
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let mut out = Vec::new();
    for t in oxttl::NTriplesParser::new().for_slice(body.as_bytes()) {
        let t = t.expect("SD must be valid N-Triples (parse-and-assert, not substring)");
        out.push((
            t.subject.to_string(),
            t.predicate.to_string(),
            t.object.to_string(),
        ));
    }
    assert!(!out.is_empty(), "SD must contain triples");
    out
}

/// The set of object IRIs (Display, sans `<>`) for `(p)` on any subject of type `sd:Service`.
fn objects_of(triples: &[(String, String, String)], predicate: &str) -> Vec<String> {
    let pred = format!("<{predicate}>");
    triples
        .iter()
        .filter(|(_, p, _)| *p == pred)
        .map(|(_, _, o)| o.trim_matches(|c| c == '<' || c == '>').to_string())
        .collect()
}

#[tokio::test]
async fn sd_parses_and_carries_mandatory_terms() {
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;

    // There is exactly one sd:Service, typed as such, with an sd:endpoint.
    let rdf_type = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
    let service_subjects: Vec<&String> = triples
        .iter()
        .filter(|(_, p, o)| p == rdf_type && *o == format!("<{SD}Service>"))
        .map(|(s, _, _)| s)
        .collect();
    assert_eq!(
        service_subjects.len(),
        1,
        "exactly one sd:Service: {triples:?}"
    );

    let endpoints = objects_of(&triples, &format!("{SD}endpoint"));
    assert_eq!(endpoints.len(), 1, "exactly one sd:endpoint");
    assert!(
        endpoints[0].ends_with("/sparql"),
        "endpoint is this /sparql: {}",
        endpoints[0]
    );

    // sd:supportedLanguage includes SPARQL11Query (always) AND the SPARQL-1.2-SD
    // version-agnostic sd:SPARQLQuery (sq-2msb) — a 1.1-only client and a 1.2-aware client both
    // recognise the endpoint.
    let langs = objects_of(&triples, &format!("{SD}supportedLanguage"));
    assert!(
        langs.contains(&format!("{SD}SPARQL11Query")),
        "SPARQL11Query advertised: {langs:?}"
    );
    assert!(
        langs.contains(&format!("{SD}SPARQLQuery")),
        "the 1.2-SD version-agnostic sd:SPARQLQuery advertised: {langs:?}"
    );

    // The REAL served SD (through `service_capabilities`) carries sd:supportedVersion 1.0 and
    // 1.1 only: the announced 1.2 semantics are only partly implemented.
    let versions = objects_of(&triples, &format!("{SD}supportedVersion"));
    assert_eq!(
        versions
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "http://www.w3.org/ns/sparql#version-1.0",
            "http://www.w3.org/ns/sparql#version-1.1"
        ]
        .into_iter()
        .collect(),
        "sd:supportedVersion: {versions:?}"
    );

    // sd:resultFormat advertises the four SPARQL-results serialisations + the three RDF ones.
    let result_formats = objects_of(&triples, &format!("{SD}resultFormat"));
    for f in [
        "SPARQL_Results_JSON",
        "SPARQL_Results_XML",
        "SPARQL_Results_CSV",
        "SPARQL_Results_TSV",
        "Turtle",
        "N-Triples",
        "RDF_XML",
    ] {
        assert!(
            result_formats.contains(&format!("{FMT}{f}")),
            "result format {f} advertised: {result_formats:?}"
        );
    }
    // sd:inputFormat advertises the RDF serialisations the GSP write path parses.
    let input_formats = objects_of(&triples, &format!("{SD}inputFormat"));
    for f in ["Turtle", "N-Triples", "RDF_XML"] {
        assert!(
            input_formats.contains(&format!("{FMT}{f}")),
            "input format {f} advertised: {input_formats:?}"
        );
    }
}

#[tokio::test]
async fn sd_advertised_result_formats_match_what_server_serves() {
    // [OPUS-4.8] sq-qfcb: HONEST advertising — for every SPARQL-results format the SD
    // advertises, a real SELECT with the matching Accept header must come back in that
    // format's Content-Type. (If the SD over-promised, this would fail.)
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let result_formats = objects_of(&triples, &format!("{SD}resultFormat"));

    let pairs = [
        ("SPARQL_Results_JSON", "application/sparql-results+json"),
        ("SPARQL_Results_XML", "application/sparql-results+xml"),
        ("SPARQL_Results_CSV", "text/csv"),
        ("SPARQL_Results_TSV", "text/tab-separated-values"),
    ];
    for (fmt_iri, accept) in pairs {
        assert!(
            result_formats.contains(&format!("{FMT}{fmt_iri}")),
            "{fmt_iri} must be advertised"
        );
        let resp = client()
            .get(format!("{base}/sparql"))
            .query(&[("query", "SELECT ?s WHERE { ?s a <http://ex/Person> }")])
            .header("Accept", accept)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "format {fmt_iri}");
        let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
        assert!(
            ct.starts_with(accept),
            "advertised {fmt_iri} but Accept {accept} returned Content-Type {ct}"
        );
    }
}

/// Objects of `(subject, predicate)` (Display strings, blank nodes kept as `_:b…`). Unlike
/// [`objects_of`] this is subject-scoped, so a test can WALK the SD's blank-node structure
/// (sd:Service → sd:defaultDataset → sd:namedGraph → sd:NamedGraph → sd:name).
fn objects_of_subject(
    triples: &[(String, String, String)],
    subject: &str,
    predicate: &str,
) -> Vec<String> {
    let pred = format!("<{predicate}>");
    triples
        .iter()
        .filter(|(s, p, _)| s == subject && *p == pred)
        .map(|(_, _, o)| o.clone())
        .collect()
}

#[tokio::test]
async fn sd_enumerates_named_graphs_end_to_end() {
    // [OPUS-4.8] sq-optl: boot a server over a TWO-named-graph TriG dataset and walk the SD
    // structure: sd:Service → sd:defaultDataset → sd:namedGraph → sd:NamedGraph → sd:name. The
    // two IRI graph names (ex:people, ex:orgs) must BOTH surface as FROM-NAMED-referenceable
    // names, each with an sd:graph carrying a per-graph void:triples count (people=2, orgs=1).
    let base = spawn_with(DATA_NAMED, "trig", true).await;
    let triples = fetch_sd_triples(&base).await;
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let void_triples = "http://rdfs.org/ns/void#triples";

    // sd:Service → sd:defaultDataset (a blank node).
    let service: &String = &triples
        .iter()
        .find(|(_, p, o)| p == &format!("<{rdf_type}>") && *o == format!("<{SD}Service>"))
        .expect("an sd:Service")
        .0;
    let datasets = objects_of_subject(&triples, service, &format!("{SD}defaultDataset"));
    assert_eq!(
        datasets.len(),
        1,
        "exactly one sd:defaultDataset: {triples:?}"
    );
    let dataset = &datasets[0];

    // sd:defaultDataset → sd:namedGraph → sd:NamedGraph nodes, one per named graph.
    let ng_nodes = objects_of_subject(&triples, dataset, &format!("{SD}namedGraph"));
    assert_eq!(
        ng_nodes.len(),
        2,
        "two named graphs must be advertised: {triples:?}"
    );

    // For each sd:NamedGraph: it is typed, carries an sd:name IRI, and an sd:graph whose
    // sd:Graph carries a void:triples count. Collect (name → count).
    let mut by_name: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for ng in &ng_nodes {
        let types = objects_of_subject(&triples, ng, rdf_type);
        assert!(
            types.contains(&format!("<{SD}NamedGraph>")),
            "node must be typed sd:NamedGraph: {types:?}"
        );
        let names = objects_of_subject(&triples, ng, &format!("{SD}name"));
        assert_eq!(names.len(), 1, "exactly one sd:name per NamedGraph");
        let graphs = objects_of_subject(&triples, ng, &format!("{SD}graph"));
        assert_eq!(graphs.len(), 1, "exactly one sd:graph per NamedGraph");
        let graph_types = objects_of_subject(&triples, &graphs[0], rdf_type);
        assert!(
            graph_types.contains(&format!("<{SD}Graph>")),
            "sd:graph target must be an sd:Graph: {graph_types:?}"
        );
        let counts = objects_of_subject(&triples, &graphs[0], void_triples);
        assert_eq!(counts.len(), 1, "exactly one void:triples per graph");
        by_name.insert(names[0].clone(), counts[0].clone());
    }

    // Both IRI graph names are present (FROM-NAMED-referenceable), with the right counts.
    assert!(
        by_name.contains_key("<http://ex/people>") && by_name.contains_key("<http://ex/orgs>"),
        "both named graph IRIs must be advertised: {by_name:?}"
    );
    let int = |n: u32| format!("\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>");
    assert_eq!(
        by_name["<http://ex/people>"],
        int(2),
        "people has 2 triples"
    );
    assert_eq!(by_name["<http://ex/orgs>"], int(1), "orgs has 1 triple");
}

#[tokio::test]
async fn sd_default_only_dataset_advertises_no_named_graphs() {
    // [OPUS-4.8] sq-optl: the historical default-only dataset (no named graphs) must advertise
    // NO sd:namedGraph — the enumeration is purely additive and never invents a graph.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let named = triples
        .iter()
        .filter(|(_, p, _)| *p == format!("<{SD}namedGraph>"))
        .count();
    assert_eq!(
        named, 0,
        "default-only dataset has no sd:namedGraph: {triples:?}"
    );
}

#[tokio::test]
async fn sd_update_language_suppressed_when_write_gated() {
    // [OPUS-4.8] sq-qfcb: when a write-token gates the write surface, an anonymous client
    // CANNOT run an Update, so the SD must NOT advertise sd:SPARQL11Update.
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        federation_descriptors: true,
        auth_token: Some("s3cret".to_string()),
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");

    let triples = fetch_sd_triples(&base).await;
    let langs = objects_of(&triples, &format!("{SD}supportedLanguage"));
    assert!(
        langs.contains(&format!("{SD}SPARQL11Query")),
        "Query still advertised: {langs:?}"
    );
    assert!(
        !langs.contains(&format!("{SD}SPARQL11Update")),
        "Update must NOT be advertised when write-gated: {langs:?}"
    );
}

#[tokio::test]
async fn sd_update_language_advertised_when_open() {
    // The default (no write token) server lets anyone Update, so it advertises SPARQL11Update.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let langs = objects_of(&triples, &format!("{SD}supportedLanguage"));
    assert!(
        langs.contains(&format!("{SD}SPARQL11Update")),
        "open server must advertise SPARQL11Update: {langs:?}"
    );
}

#[tokio::test]
#[cfg(feature = "geo")]
async fn sd_advertises_registered_geo_extension_functions() {
    // [OPUS-4.8] sq-qfcb: with the `geo` feature, the SD must advertise EXACTLY the geof:
    // functions the engine actually registered — sourced from the live registry, so the
    // advertisement cannot drift from what runs. Spot-check a few real ones.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let fns = objects_of(&triples, &format!("{SD}extensionFunction"));
    let registered: Vec<String> = sparq_geo::geof_registry()
        .iris()
        .map(str::to_string)
        .collect();
    assert!(
        !registered.is_empty(),
        "geo registry must register functions"
    );
    // Every advertised function is genuinely registered (no fabrication)...
    for f in &fns {
        assert!(
            registered.contains(f),
            "advertised extension function {f} is NOT in the live registry"
        );
    }
    // ...and every registered function is advertised (no omission).
    for f in &registered {
        assert!(
            fns.contains(f),
            "registered function {f} was NOT advertised in the SD"
        );
    }
    assert!(fns
        .iter()
        .any(|f| f == "http://www.opengis.net/def/function/geosparql/distance"));
}

#[tokio::test]
#[cfg(feature = "service")]
async fn sd_advertises_basic_federated_query_when_service_feature_on() {
    // [OPUS-4.8] sq-qfcb: with the `service` feature the SERVICE clause is compiled in, so
    // the SD honestly advertises sd:feature sd:BasicFederatedQuery.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let features = objects_of(&triples, &format!("{SD}feature"));
    assert!(
        features.contains(&format!("{SD}BasicFederatedQuery")),
        "service feature on => BasicFederatedQuery advertised: {features:?}"
    );
}

#[tokio::test]
#[cfg(not(feature = "service"))]
async fn sd_omits_basic_federated_query_without_service_feature() {
    // Without the `service` feature a SERVICE clause errors at execution, so advertising the
    // feature would be a fiction — it must be absent.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let features = objects_of(&triples, &format!("{SD}feature"));
    assert!(
        !features.contains(&format!("{SD}BasicFederatedQuery")),
        "no service feature => BasicFederatedQuery must be absent: {features:?}"
    );
}

#[tokio::test]
async fn sd_omits_prov_lineage_feature_until_node_serves_it() {
    // [OPUS-4.8] sq-yyy3: `sparq-server` exposes no PROV-O lineage-serving endpoint today, so it
    // must NOT advertise the sparq PROV-O lineage feature — advertising it would over-promise.
    // The descriptor SUPPORTS the feature (Capabilities::provenance ⇒ sd:feature <…/prov#lineage>),
    // but the server keeps the flag false, so the served SD never carries it. This guards the
    // honesty boundary at the integration level.
    let base = spawn(true).await;
    let triples = fetch_sd_triples(&base).await;
    let features = objects_of(&triples, &format!("{SD}feature"));
    assert!(
        !features.contains(&"http://sparq.dev/ns/prov#lineage".to_string()),
        "server does not serve lineage => PROV-O lineage feature must be absent: {features:?}"
    );
}

#[tokio::test]
async fn sparql_query_still_works_with_descriptors_on() {
    // The SD only intercepts the no-query GET; a real query is unaffected.
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s a <http://ex/Person> }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
    let body = resp.text().await.unwrap();
    assert_eq!(body.matches("\"type\":\"uri\"").count(), 3);
}
