//! Boot-time service registration shared by the Solid and LWS descriptions.
//!
//! Register entries alongside mounted routes, then retain the catalog in server state.
//! Extension types use absolute IRIs; standard LWS types may use their context terms.

use crate::error::ServerError;
use crate::ldp::content::{parse_to_triples, serialize_triples, RdfFormat};
use serde_json::{json, Map, Value};

/// Draft Solid service-description extension namespace.
pub const SSD: &str = "https://jeswr.org/ns/solid-sd#";
/// Access-controlled query discovery relation.
pub const SPARQL_ENDPOINT: &str = "http://www.w3.org/ns/solid/sparql#endpoint";
const LWS: &str = "https://www.w3.org/ns/lws#";

/// A deployed service and its description properties.
#[derive(Clone, Debug)]
pub struct ServiceEntry {
    /// Stable fragment within the storage description.
    pub fragment: String,
    /// Absolute type IRI, or a standard LWS context term.
    pub type_term: String,
    /// Absolute URL of the mounted endpoint.
    pub endpoint: String,
    /// Additional JSON-LD properties, using context terms or absolute IRIs.
    pub extras: Map<String, Value>,
}

/// A supported operation and its description properties.
#[derive(Clone, Debug)]
pub struct Capability {
    /// Absolute type IRI, or a standard LWS context term.
    pub type_term: String,
    /// Additional JSON-LD properties, using context terms or absolute IRIs.
    pub extras: Map<String, Value>,
}

/// An extensible catalog built once from the routes mounted at boot.
#[derive(Clone, Debug, Default)]
pub struct ServiceCatalog {
    services: Vec<ServiceEntry>,
    capabilities: Vec<Capability>,
}

impl ServiceCatalog {
    /// Register a service when mounting its route.
    pub fn service(&mut self, entry: ServiceEntry) -> &mut Self {
        self.services.push(entry);
        self
    }

    /// Register a capability when enabling its handler.
    pub fn capability(&mut self, entry: Capability) -> &mut Self {
        self.capabilities.push(entry);
        self
    }

    /// Append entries for additional mounted routes.
    pub fn extend(&mut self, catalog: Self) -> &mut Self {
        self.services.extend(catalog.services);
        self.capabilities.extend(catalog.capabilities);
        self
    }

    /// Inspect the deployed services.
    pub fn services(&self) -> &[ServiceEntry] {
        &self.services
    }

    /// Inspect the supported capabilities.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub(crate) fn services_json(&self, storage: &str) -> Value {
        self.services
            .iter()
            .map(|s| {
                let mut entry = s.extras.clone();
                entry.insert("id".into(), json!(format!("{storage}#{}", s.fragment)));
                entry.insert("type".into(), json!(s.type_term));
                entry.insert("serviceEndpoint".into(), json!(s.endpoint));
                Value::Object(entry)
            })
            .collect()
    }

    pub(crate) fn capabilities_json(&self) -> Value {
        self.capabilities
            .iter()
            .map(|c| {
                let mut entry = c.extras.clone();
                entry.insert("type".into(), json!(c.type_term));
                Value::Object(entry)
            })
            .collect()
    }

    /// Render the Solid graph through the crate's RDF writers.
    ///
    /// # Errors
    /// Returns an error for invalid JSON-LD properties or RDF serialization failures.
    pub fn solid(&self, storage: &str, format: RdfFormat) -> Result<Vec<u8>, ServerError> {
        let mut document = json!({
            "@context": local_context(),
            "id": storage,
            "type": ["http://www.w3.org/ns/pim/space#Storage", format!("{LWS}Storage")],
            "service": self.services_json(storage),
            "capability": self.capabilities_json(),
        });
        for service in &self.services {
            if service.type_term == format!("{SSD}SparqlQueryService") {
                document[SPARQL_ENDPOINT] = json!({"@id": service.endpoint});
            }
        }
        let mut triples = parse_to_triples(
            RdfFormat::JsonLd,
            &serde_json::to_vec(&document).map_err(|e| ServerError::Storage(e.to_string()))?,
            storage,
        )?;
        crate::ldp::content::label_blank_nodes_by_position(&mut triples);
        serialize_triples(format, &triples)
    }
}

/// Inline mappings for draft extension terms, without an external context dependency.
pub fn extension_context() -> Value {
    let mut context = Map::new();
    for term in [
        "SolidNotificationService",
        "SparqlQueryService",
        "CrdtSyncService",
        "PatchSupport",
        "VersioningService",
        "TrashService",
        "AccessControl",
        "RecursiveDelete",
        "WAC",
        "ACP",
        "ODRLGrants",
        "WebSocketSubscription",
        "SseSubscription",
        "channelType",
        "feature",
        "roomTemplate",
        "retention",
        "format",
        "source",
        "patchType",
        "model",
        "protocol",
        "target",
    ] {
        context.insert(term.into(), json!(format!("{SSD}{term}")));
    }
    context.insert(
        "profile".into(),
        json!({"@id": "http://purl.org/dc/terms/conformsTo", "@type": "@id"}),
    );
    context.into()
}

fn local_context() -> Value {
    let mut context = extension_context();
    let c = context
        .as_object_mut()
        .expect("extension context is an object");
    c.insert("@vocab".into(), json!(LWS));
    c.insert("id".into(), json!("@id"));
    c.insert("type".into(), json!("@type"));
    for (term, iri, iri_value) in [
        ("service", "https://www.w3.org/ns/did#service", false),
        (
            "serviceEndpoint",
            "https://www.w3.org/ns/did#serviceEndpoint",
            true,
        ),
        ("capability", "https://www.w3.org/ns/lws#capability", false),
        ("conformsTo", "http://purl.org/dc/terms/conformsTo", true),
    ] {
        c.insert(
            term.into(),
            if iri_value {
                json!({"@id": iri, "@type": "@id"})
            } else {
                json!(iri)
            },
        );
    }
    context
}

pub(crate) fn entry(fragment: &str, type_term: &str, endpoint: String) -> ServiceEntry {
    ServiceEntry {
        fragment: fragment.into(),
        type_term: type_term.into(),
        endpoint,
        extras: Map::new(),
    }
}

pub(crate) fn capability(type_term: &str, extras: Value) -> Capability {
    Capability {
        type_term: type_term.into(),
        extras: extras.as_object().cloned().unwrap_or_default(),
    }
}

// Applied outside authentication, so root discovery is identical on allowed and denied reads.
pub(crate) async fn root_discovery(
    axum::extract::State(endpoint): axum::extract::State<Option<axum::http::HeaderValue>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let root_read = request.uri().path() == "/"
        && matches!(
            *request.method(),
            axum::http::Method::GET | axum::http::Method::HEAD
        );
    let mut response = next.run(request).await;
    if let (true, Some(endpoint)) = (root_read, endpoint) {
        response
            .headers_mut()
            .append(axum::http::header::LINK, endpoint);
    }
    response
}
