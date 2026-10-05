//! [FABLE-5] sq-u16eq — the pod-backed MCP server (feature `solid`, OFF by default):
//! LDP container/resource CRUD tools over a `sparq_solid::PodStore`, per the MCP-Solid
//! proposal draft (`site/specs/mcp-solid.typ` §6.4 / §7.3 / §9.3, sq-tag1q.1).
//!
//! # What this adds
//!
//! Where [`crate::McpServer`] serves ONE anonymous in-memory [`Graph`] with a
//! SPARQL-only surface, [`SolidMcpServer`] serves a **pod**: a named-graph-per-document
//! dataset with a materialized WAC/ACP authorization view, bound to ONE authenticated
//! session (the Solid-OIDC-derived agent/client/issuer triple fixed at construction —
//! the "local trusted agent" deployment mode of the draft §4.1). Tools:
//!
//! - `query` (Class R) — session-scoped SPARQL over the authorized graph set via the
//!   engine's zero-copy dataset view; the standing default graph is EMPTY and a query
//!   opts into the authorized union with the reserved `FROM` IRI (the
//!   `solid-sparql-query` semantics `sparq-solid` implements).
//! - `resource_get` (Class R) — one pod document, served from the SAME dataset the
//!   `query` tool evaluates over (draft §6.4: the two read surfaces cannot disagree).
//! - `container_list` (Class R) — the direct `ldp:contains` members of one container,
//!   derived ONLY from stored containment triples in the container's own graph — never
//!   from IRI-path guessing (draft §6.4).
//! - `introspect`, `shapes`, `stats` (Class R) — the schema/statistics tools, mined from
//!   the session's authorized projection rather than the whole pod. [SONNET-4.6] sq-8n6iv
//! - `update`, `resource_put`, `resource_delete`, `container_create` (Class U) — all
//!   gated behind the SAME off-by-default write enablement as the base server's
//!   `update` tool ([`SolidServerConfig::allow_update`], draft §7.1).
//!
//! # Session-scoped aggregates — [SONNET-4.6] sq-8n6iv
//!
//! [`crate::McpServer`]'s `introspect` / `shapes` / `stats` tools mine the WHOLE served
//! graph. Handing a whole-pod schema or a whole-pod triple count to one principal is an
//! **aggregate leak**: it discloses the classes, predicates, vocabularies and volume of
//! documents that principal cannot open — a leak that no per-resource check catches,
//! because no resource was read. So this server does not reuse them. Its `introspect` /
//! `shapes` / `stats` are derived from ONE input — the session's **authorized
//! projection** — which is built through the SAME [`sparq_engine::DatasetView`] the
//! `query` tool evaluates under (`PodStore::view_for`). Two consequences follow by
//! construction rather than by a second check:
//!
//! - an unauthorized document contributes to no count, in any of the three tools;
//! - a session with no grants gets an empty schema and zero totals (fail-closed), not
//!   the pod's real totals.
//!
//! The standing default graph is empty here too: the projection ranges over
//! `GRAPH ?g`, so it is the authorized NAMED graphs and nothing else.
//!
//! # The `resources` surface + notifications (Class N, draft §8 / §10)
//!
//! [SONNET-4.6] sq-cmjmr — the pod server declares the MCP **`resources`** capability
//! with **`subscribe: true`** and binds it to Solid Notifications Protocol semantics:
//!
//! - `resources/list` — the pod documents THIS session may read, one MCP resource per
//!   document with `uri` = the resource IRI. A document the session cannot read is
//!   simply absent (never an entry, never an error), so the listing itself is the
//!   authorization boundary.
//! - `resources/read` — the same bytes [`SolidMcpServer`]'s `resource_get` tool serves,
//!   from the same dataset the `query` tool evaluates over.
//! - `resources/subscribe` / `resources/unsubscribe` — a subscription on the topic IRI,
//!   the MCP-side spelling of a Solid Notifications subscription on that topic.
//! - `notifications/resources/updated` — emitted when a subscribed topic's state
//!   changes. The payload is **content-free**: the topic IRI plus an ActivityStreams 2.0
//!   activity type (`Create`/`Update`/`Delete`/`Add`/`Remove`), never the triples. The
//!   subscriber re-reads through the authorized read path.
//!
//! **Authorization is checked twice** (draft §10): once when the subscription is created
//! (no read access ⇒ the same existence-non-disclosure not-found error described below,
//! so subscribing cannot probe for resources) and AGAIN before every
//! single delivery. If the session's read access is revoked after subscribing, deliveries
//! simply stop — the client is **never told**, because a revocation notification would
//! itself disclose that a resource it can no longer read had changed. If access is later
//! restored, deliveries resume; the subscription is not silently torn down.
//!
//! Delivery is pull-based, matching this crate's transport-agnostic dispatch core: a
//! mutating tool call queues the notifications it caused, and the embedder drains them
//! with [`SolidMcpServer::take_notifications`] after each
//! [`SolidMcpServer::handle_message`], writing them to its transport.
//!
//! # Existence non-disclosure (draft §9.3)
//!
//! For every resource-addressed tool, a session **lacking read access to the target
//! receives a tool error byte-identical to the resource-does-not-exist error**
//! ([`not_found_error`]), so resource existence is never disclosed to a session that
//! cannot read the resource. A session *with* read access but without the required
//! write mode gets a distinguishable denied error (permitted by the draft). Writes to
//! an access-control document are gated on `acl:Control` of the governed resource and
//! non-disclose the same way.
//!
//! # ACL write-through (draft §7.3)
//!
//! `resource_put` / `resource_delete` targeting an access-control document (`.acl` /
//! `.acr`) route through `sparq-solid`'s authoritative write-through
//! (`PodStore::put_acl` / `delete_acl`, or the `_acp` twins): the control graph swap
//! and the authorization-view re-materialization succeed or roll back as ONE
//! fail-closed atomic unit — a pod is never left with a policy the enforcement layer
//! could not evaluate. Content-document writes are parse-first + rollback-on-error the
//! same way, and re-materialize the view so a created document becomes visible (and a
//! deleted one invisible) to the very next tool call.
//!
//! # Content negotiation (draft §6.4)
//!
//! [SONNET-4.6] sq-wbsf5 — `resource_get` takes an optional `accept` and serves the
//! representation it names, from the SAME triples in both cases:
//!
//! - `application/n-triples` — the default when `accept` is absent, so an existing caller
//!   sees no change.
//! - `text/turtle` — the same triples through `oxttl`'s `TurtleSerializer` (the oxigraph
//!   writer, so the body is guaranteed well-formed Turtle), with `TURTLE_PREFIXES`
//!   registered so pod vocabularies compact to `prefix:local`.
//!
//! Negotiation is a **pure function of the `accept` string** (`negotiate`) evaluated
//! BEFORE the read gate, so it cannot become an existence oracle. An unsupported `accept`
//! is a tool error naming exactly what IS served — never silent coercion to another
//! syntax. `accept` is ONE media type (`;`-parameters such as `q=` are ignored), not an
//! HTTP `Accept` header list: a comma-separated list matches nothing and is refused
//! rather than silently resolved to a guess.
//!
//! The MCP `resources/read` surface has no per-request `accept` in the protocol, so it
//! stays `application/n-triples` — the same bytes `resource_get` serves by default, from
//! the same `document_triples` read path, so the two cannot disagree.
//!
//! # Non-RDF (binary) resources — SCOPED OUT, deliberately
//!
//! [SONNET-4.6] sq-wbsf5 — LDP has non-RDF sources (an image, a PDF); **this server has
//! none, by decision, not by omission**, and both write and read paths say so:
//! `resource_put` refuses any non-RDF `content_type` naming the scope-out, and no non-RDF
//! resource can exist to be read (nothing else can create one). The reasons:
//!
//! - **There is nowhere to put the bytes.** The pod IS a `sparq_solid::PodStore` — an RDF
//!   dataset of one named graph per document. A binary body is not triples, so supporting
//!   it needs a second, non-RDF store plus its own authorization join; that is a
//!   `sparq-solid` storage design, not an MCP tool-surface change, and inventing a
//!   base64-literal side-channel in the graph would be a fake pod.
//! - **The authorization story would have to be re-derived.** WAC/ACP modes are evaluated
//!   over the dataset; a blob outside it has no graph to anchor its ACL to, and the
//!   existence-non-disclosure guarantee above is only as good as the one gate every read
//!   passes through.
//!
//! Until a pod store with a blob half exists, an honest refusal beats a half-implemented
//! binary path. When it does, the MCP shape is already known: `resources/read` carries a
//! base64 `blob` field alongside `text` for exactly this case.
//!
//! # Honest v1 limits
//!
//! - RDF sources only: `content_type` must be Turtle or N-Triples (or JSON-LD when the
//!   opt-in `jsonld` feature is enabled) — see the non-RDF scope-out above. [GPT-5.6]
//! - The SPARQL `update` tool delegates to the session-checked
//!   `PodStore::update_as_with_budget` / `update_as_acp_with_budget`, so it applies the
//!   per-graph write permission check AND the same wall-clock/row budget the read tools
//!   enforce ([FABLE-5] sq-yhlf0). The budget reaches the two evaluations an update
//!   performs — the authorization check's `GRAPH ?var` binding SELECT and the apply's
//!   `DELETE`/`INSERT … WHERE`. It does NOT bound the remaining operations, and capping the
//!   request body does not bound all of them either: `INSERT`/`DELETE DATA` carry their
//!   triples inline (a body cap DOES bound those, and an embedder serving untrusted callers
//!   should set one), but `CLEAR`/`DROP` cost whatever the targeted graphs already hold —
//!   bounded by this session's write grants, not by the request — and `LOAD` reads a file
//!   whose size is independent of the SPARQL text. This server never installs an allowlisted
//!   LOAD base (`sparq_engine::with_load_base`), so a `LOAD` is refused unless the embedder
//!   wraps the call in one; an embedder that does owes its own file-size limit.
//! - Time-windowed conditional grants fail closed unless [`SolidServerConfig::now`]
//!   supplies a request clock.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use oxrdf::{NamedNode, Term, Triple};
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_introspect::Introspection;
use sparq_solid::{Mode, PodStore, Session};

use crate::jsonrpc::{Request, RpcError, INVALID_PARAMS, METHOD_NOT_FOUND, RESOURCE_NOT_FOUND};
use crate::notifications::{classify, ActivityType, Subscriptions, TopicDigest};
use crate::server::{arg_str, handle_raw, tool_text_result};
use crate::tools::ToolSpec;

/// `ldp:contains` — the containment predicate a container's own graph stores.
const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";
/// `ldp:BasicContainer` — the type asserted on a container `container_create` makes.
const LDP_BASIC_CONTAINER: &str = "http://www.w3.org/ns/ldp#BasicContainer";
/// `ldp:Container` — asserted alongside [`LDP_BASIC_CONTAINER`].
const LDP_CONTAINER: &str = "http://www.w3.org/ns/ldp#Container";
/// `rdf:type`.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The reserved graph-name space a pod write may never target (the rewrite sentinel
/// and the materialized auth view live here; `PodStore::new` strips inbound graphs
/// under it, and this server refuses to re-introduce one).
const RESERVED_PREFIX: &str = "urn:sparq:";

/// Configuration for a [`SolidMcpServer`]: the ONE authenticated session the server is
/// bound to, the pod's access-control model, and the same security-default gates as
/// [`crate::ServerConfig`] (writes OFF, bounded query budget).
#[derive(Debug, Clone)]
pub struct SolidServerConfig {
    /// The authenticated agent's WebID (`None` = anonymous — a fresh pod with no
    /// public grant then fails closed to zero visibility).
    pub agent: Option<String>,
    /// The client identifier (WAC `acl:origin` / ACP `acp:client`); `None` = any.
    pub client: Option<String>,
    /// The OIDC issuer that vouched for the WebID (ACP only); `None` = any.
    pub issuer: Option<String>,
    /// The request clock as an `xsd:dateTime` lexical string, consulted only by
    /// time-windowed conditional grants. `None` ⇒ such grants fail closed.
    pub now: Option<String>,
    /// `false` = WAC pod (`.acl`, [`PodStore::materialize_wac`]); `true` = ACP pod
    /// (`.acr`, [`PodStore::materialize_acp`]). Selects which model every
    /// (re-)materialization this server performs runs — including the ACL
    /// write-through twins.
    pub acp: bool,
    /// Whether the mutating tools (`update`, `resource_put`, `resource_delete`,
    /// `container_create`) are exposed. **OFF by default** — a freshly-built pod
    /// server is strictly read-only, exactly like [`crate::ServerConfig`].
    pub allow_update: bool,
    /// Wall-clock deadline per tool-issued query, in seconds (`None` = unbounded).
    pub query_timeout_secs: Option<u64>,
    /// Row cap on any materialised query result (`None` = uncapped).
    pub max_rows: Option<usize>,
    /// The server name reported in the `initialize` handshake.
    pub server_name: String,
}

impl Default for SolidServerConfig {
    fn default() -> Self {
        SolidServerConfig {
            agent: None,
            client: None,
            issuer: None,
            now: None,
            acp: false,
            // Security default: read-only, exactly like the base server.
            allow_update: false,
            query_timeout_secs: Some(30),
            max_rows: Some(1_000_000),
            server_name: "sparq-mcp-solid".to_string(),
        }
    }
}

/// An MCP server over a `sparq-solid` [`PodStore`], bound to one authenticated
/// session. See the [module docs](self) for the tool surface and its draft-conformance
/// properties (existence non-disclosure, ACL write-through, shared dataset).
///
/// Construction (re-)materializes the authorization view under the configured model
/// ([`SolidServerConfig::acp`]), so the server never starts on a stale or
/// differently-modelled view.
pub struct SolidMcpServer {
    store: PodStore,
    config: SolidServerConfig,
    /// [SONNET-4.6] sq-cmjmr — the topics this session subscribed to plus the queue of
    /// content-free notifications awaiting [`SolidMcpServer::take_notifications`]. Bound
    /// to this server (⇒ to this one authenticated session), so a subscription can never
    /// outlive or escape its authorization context.
    subs: Subscriptions,
}

/// The pod `query` tool (session-scoped — distinct semantics from the base server's).
pub const POD_QUERY: ToolSpec = ToolSpec {
    name: "query",
    description: "Run a read-only SPARQL 1.1 SELECT or ASK query against the pod, \
                  restricted to the named graphs (documents) this session may read, and \
                  return SPARQL 1.1 Query Results JSON. One named graph per pod \
                  document; the standing DEFAULT graph is EMPTY — use GRAPH patterns, \
                  or opt into the authorized union for one query with FROM \
                  <http://www.w3.org/ns/solid/sparql#union-default-graph>. An \
                  unauthorized document contributes nothing and raises no error.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "sparql": {
                    "type": "string",
                    "description": "A SPARQL 1.1 SELECT or ASK query string."
                }
            },
            "required": ["sparql"],
            "additionalProperties": false
        })
    },
};

/// The `resource_get` tool: one pod document from the same dataset `query` evaluates.
pub const RESOURCE_GET: ToolSpec = ToolSpec {
    name: "resource_get",
    description: "Return the RDF representation of ONE pod resource (document), served \
                  from the same named-graph-per-document dataset the `query` tool \
                  evaluates over. Result: the resource content plus the media type it \
                  was served as — N-Triples by default, or Turtle when `accept` asks for \
                  it (same triples, different syntax). A resource this session cannot \
                  read is reported with the SAME error as a resource that does not exist.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The resource IRI (the pod document's named-graph name)."
                },
                "accept": {
                    "type": "string",
                    "description": "Optional media type to serve: \
                                    \"application/n-triples\" (the default when omitted) \
                                    or \"text/turtle\". ONE media type, not an HTTP \
                                    Accept list; any other value is a tool error (no \
                                    silent coercion). Non-RDF (binary) representations \
                                    are out of scope — this pod stores RDF only."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    },
};

/// The `container_list` tool: `ldp:contains` members from stored containment data.
pub const CONTAINER_LIST: ToolSpec = ToolSpec {
    name: "container_list",
    description: "List the DIRECT members of one pod container (the semantics of \
                  ldp:contains), derived ONLY from the containment triples stored in the \
                  container's own document — never guessed from IRI paths. Each member \
                  carries a container flag. A container this session cannot read is \
                  reported with the SAME error as one that does not exist.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The container IRI (conventionally slash-terminated)."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    },
};

/// The pod `introspect` tool: the effective schema of the session's AUTHORIZED
/// documents only (never the whole pod). [SONNET-4.6] sq-8n6iv
pub const POD_INTROSPECT: ToolSpec = ToolSpec {
    name: "introspect",
    description: "Return the effective schema of the pod documents THIS SESSION MAY \
                  READ — classes (by instance count), predicates (by usage), namespace \
                  prefixes, and characteristic sets — mined from exactly the named \
                  graphs the `query` tool can see. A document this session cannot read \
                  contributes to no count, so the schema is not an aggregate view of the \
                  whole pod. Set `format` to \"text\" for a compact token-budgeted \
                  summary suitable for LLM grounding, or \"json\" (the default) for the \
                  full machine-readable structure.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "enum": ["json", "text"],
                    "description": "Output shape: \"json\" (full structure, default) or \
                                    \"text\" (compact prompt-ready summary)."
                },
                "budget_chars": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "For format=\"text\": approximate character budget for \
                                    the summary (default 4000)."
                }
            },
            "additionalProperties": false
        })
    },
};

/// The pod `shapes` tool: one class's data-grounded shape over the AUTHORIZED
/// documents only. [SONNET-4.6] sq-8n6iv
pub const POD_SHAPES: ToolSpec = ToolSpec {
    name: "shapes",
    description: "Given a class/type IRI, return the data-grounded shape of that class \
                  — which predicates its instances actually use, with coverage, observed \
                  datatypes, object-kind split, observed range, and the cardinalities the \
                  data supports — derived ONLY from the pod documents this session may \
                  read. Instances in unauthorized documents are invisible here exactly as \
                  they are to `query`, so a class only this session's unreadable \
                  documents use is reported as absent. No LLM is involved.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "class": {
                    "type": "string",
                    "description": "The full class IRI (e.g. \
                                    \"http://xmlns.com/foaf/0.1/Person\"). Call \
                                    `introspect` first to discover the classes the \
                                    authorized documents use."
                }
            },
            "required": ["class"],
            "additionalProperties": false
        })
    },
};

/// The pod `stats` tool: totals over the AUTHORIZED documents only. [SONNET-4.6] sq-8n6iv
pub const POD_STATS: ToolSpec = ToolSpec {
    name: "stats",
    description: "Return summary statistics — total triples, distinct subjects, typed \
                  entities, and the number of distinct classes / predicates / namespaces \
                  — computed over exactly the pod documents this session may read, not \
                  over the whole pod. Two sessions get different totals; a session with \
                  no grants gets zeros. Counts are over the RDF MERGE of those documents, \
                  so a triple asserted in two of them counts once.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    },
};

/// The pod `update` tool (session-checked writes; gated like every mutating tool).
pub const POD_UPDATE: ToolSpec = ToolSpec {
    name: "update",
    description: "Apply a SPARQL 1.1 Update to the pod, enforcing this session's \
                  per-document WRITE authorization on every graph the update could \
                  touch (fail-closed: one unwritable target rejects the whole request \
                  before anything is applied; the default graph is never writable). \
                  Only available when the server was started with updates enabled.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "sparql": {
                    "type": "string",
                    "description": "A SPARQL 1.1 Update request string."
                }
            },
            "required": ["sparql"],
            "additionalProperties": false
        })
    },
};

/// The `resource_put` tool: create or replace one document (LDP PUT).
pub const RESOURCE_PUT: ToolSpec = ToolSpec {
    name: "resource_put",
    description: "Create or replace ONE pod resource's RDF representation (LDP PUT \
                  semantics): replacement atomically swaps the document's named graph; \
                  creation also links the document into its parent containers' \
                  ldp:contains listings. Writing an access-control document (.acl/.acr) \
                  routes through the authoritative write-through: the policy swap and \
                  the authorization-view rebuild succeed or roll back as one unit. \
                  Requires write access; only available when updates are enabled.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The resource IRI to write." },
                "content": { "type": "string", "description": "The full new RDF body." },
                "content_type": {
                    "type": "string",
                    "description": "\"text/turtle\" or \"application/n-triples\" — RDF \
                                    sources only. Non-RDF (binary) resources are out of \
                                    scope: this pod is an RDF dataset with nowhere to \
                                    store bytes, so a binary body is refused, not stored."
                }
            },
            "required": ["url", "content", "content_type"],
            "additionalProperties": false
        })
    },
};

/// The `resource_delete` tool: delete one document (LDP DELETE).
pub const RESOURCE_DELETE: ToolSpec = ToolSpec {
    name: "resource_delete",
    description: "Delete ONE pod resource (LDP DELETE semantics): removes the \
                  document's named graph and its parent's ldp:contains link. Deleting a \
                  non-empty container is rejected. Deleting an access-control document \
                  (.acl/.acr) routes through the authoritative write-through, so the \
                  removed rules stop granting immediately. Requires write access; only \
                  available when updates are enabled.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The resource IRI to delete." }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    },
};

/// The `container_create` tool: create one empty container.
pub const CONTAINER_CREATE: ToolSpec = ToolSpec {
    name: "container_create",
    description: "Create ONE empty pod container (typed ldp:BasicContainer) and link it \
                  into its parent containers' ldp:contains listings. The IRI must be \
                  slash-terminated. Creating an existing container is an error. Requires \
                  write access; only available when updates are enabled.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The container IRI (must end in \"/\")." }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    },
};

/// The tool names only reachable when [`SolidServerConfig::allow_update`] is on.
const MUTATING: [&str; 4] = ["update", "resource_put", "resource_delete", "container_create"];

/// The existence-non-disclosure error (draft §9.3): used **byte-identically** for a
/// resource that does not exist and for one this session may not read, so no
/// resource-addressed tool discloses existence to a session without read access.
pub fn not_found_error(url: &str) -> String {
    format!("resource not found: <{url}>")
}

/// The distinguishable write-denied error for a session that CAN read the target but
/// lacks the required write mode (the draft permits distinguishing this case).
fn write_denied_error(url: &str) -> String {
    format!("write access denied: <{url}>")
}

/// Map a `content_type` / `accept` value to the engine's RDF format name. Media-type
/// parameters (`; charset=…`) are ignored; only the RDF text formats a pod document
/// can round-trip are accepted.
fn rdf_format(content_type: &str) -> Option<&'static str> {
    let base = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match base.as_str() {
        "text/turtle" | "turtle" | "ttl" => Some("turtle"),
        "application/n-triples" | "ntriples" | "n-triples" | "nt" => Some("ntriples"),
        #[cfg(feature = "jsonld")]
        "application/ld+json" | "jsonld" | "json-ld" => Some("jsonld"), // [GPT-5.6]
        _ => None,
    }
}

/// The media types `resource_put` will INGEST, for the refusal message. RDF sources only
/// — non-RDF (binary) bodies are scoped out, see the module docs. [SONNET-4.6] sq-wbsf5
#[cfg(not(feature = "jsonld"))]
const INGESTED_MEDIA_TYPES: &str = "text/turtle, application/n-triples";
#[cfg(feature = "jsonld")]
const INGESTED_MEDIA_TYPES: &str = "text/turtle, application/n-triples, application/ld+json";

/// The media types `resource_get` will SERVE, for the refusal message. Hand-kept in step
/// with the arms of `negotiate`; `negotiate_refuses_an_unservable_accept_naming_what_is_served`
/// asserts the refusal names both, so a representation added without updating this string
/// is caught by the tests rather than shipped as a wrong advertisement. [SONNET-4.6] sq-wbsf5
const SERVED_MEDIA_TYPES: &str = "application/n-triples, text/turtle";

/// The representation `resource_get` serves, chosen by [`negotiate`] from the caller's
/// `accept`. Both variants render the SAME triples from the SAME read path — the choice
/// is syntax only, never content. [SONNET-4.6] sq-wbsf5
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Representation {
    /// `application/n-triples` — the default when no `accept` is supplied.
    NTriples,
    /// `text/turtle`, written by `oxttl`'s `TurtleSerializer`.
    Turtle,
}

impl Representation {
    /// The media type this representation is reported as in the tool result.
    fn media_type(self) -> &'static str {
        match self {
            Representation::NTriples => "application/n-triples",
            Representation::Turtle => "text/turtle",
        }
    }
}

/// Content negotiation for `resource_get` (draft §6.4): resolve the caller's `accept` to
/// the representation to serve.
///
/// A **pure function of the `accept` value** — it never touches the store, so it cannot
/// become an existence oracle no matter where the caller runs it. Absent `accept` keeps
/// the v1 default (N-Triples). An unsupported value is refused with a message naming
/// exactly what IS served: no silent coercion to a syntax the caller did not ask for.
/// One media type only (`;`-parameters such as `q=` are ignored) — an HTTP-style
/// comma-separated `Accept` list matches nothing and is refused rather than guessed at.
fn negotiate(accept: Option<&str>) -> Result<Representation, String> {
    let Some(accept) = accept else { return Ok(Representation::NTriples) };
    match rdf_format(accept) {
        Some("ntriples") => Ok(Representation::NTriples),
        Some("turtle") => Ok(Representation::Turtle),
        // Includes `Some("jsonld")` under the `jsonld` feature: that parser is INGEST-only
        // (`resource_put`), there is no JSON-LD writer here, so serving it would be a lie.
        _ => Err(format!(
            "unsupported accept `{}`: this server serves {}",
            accept, SERVED_MEDIA_TYPES
        )),
    }
}

/// Prefixes registered on the Turtle writer so the vocabularies that dominate pod
/// documents compact to `prefix:local`. Purely cosmetic and purely additive: a
/// non-matching IRI simply renders in full, so the set can never make output wrong —
/// only more or less compact. Kept short and stable on purpose (there is no per-request
/// prefix input to negotiate against). [SONNET-4.6] sq-wbsf5
const TURTLE_PREFIXES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("ldp", "http://www.w3.org/ns/ldp#"),
    ("acl", "http://www.w3.org/ns/auth/acl#"),
    ("acp", "http://www.w3.org/ns/solid/acp#"),
    ("solid", "http://www.w3.org/ns/solid/terms#"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("schema", "https://schema.org/"),
];

/// Serialize a document's triples as **Turtle** through `oxttl`'s `TurtleSerializer` —
/// the oxigraph writer, so the body is guaranteed well-formed Turtle rather than
/// hand-rolled — with [`TURTLE_PREFIXES`] registered for compaction.
///
/// Writing into an in-memory `Vec<u8>` cannot actually fail, but a serializer-internal
/// error is propagated as `Err` rather than swallowed: a truncated body reported as a
/// successful read would be exactly the kind of silent lie this surface must not tell.
fn triples_to_turtle(triples: &[Triple]) -> Result<String, String> {
    let mut ser = oxttl::TurtleSerializer::new();
    for (name, iri) in TURTLE_PREFIXES {
        // `with_prefix` fails only on a syntactically invalid namespace IRI; ours are
        // constants, so fall back to the un-prefixed writer rather than panic if a future
        // edit breaks one (less compaction, never wrong output).
        ser = ser.with_prefix(*name, *iri).unwrap_or_else(|_| oxttl::TurtleSerializer::new());
    }
    let mut w = ser.for_writer(Vec::new());
    for t in triples {
        w.serialize_triple(t.as_ref())
            .map_err(|e| format!("Turtle serialization failed: {}", e))?;
    }
    let bytes = w.finish().map_err(|e| format!("Turtle serialization failed: {}", e))?;
    String::from_utf8(bytes).map_err(|e| format!("Turtle serialization was not UTF-8: {}", e))
}

/// Whether `iri` names an access-control document by the `.acl`/`.acr` convention
/// (mirrors `sparq-solid`'s write-through targeting rule).
fn is_control_doc(iri: &str) -> bool {
    iri.ends_with(".acl") || iri.ends_with(".acr")
}

/// The parent container of `iri` under LDP slash semantics, or `None` at (or above)
/// the origin root: `https://h/a/b` → `https://h/a/`, `https://h/a/` → `https://h/`,
/// `https://h/` → `None`.
fn parent_iri(iri: &str) -> Option<String> {
    let scheme_end = iri.find("://")? + 3;
    let root_slash = iri[scheme_end..].find('/')? + scheme_end;
    let cut = iri.strip_suffix('/').unwrap_or(iri);
    let pos = cut.rfind('/')?;
    if pos < root_slash {
        return None; // already the origin root (or a scheme slash)
    }
    Some(iri[..=pos].to_string())
}

/// Parse + validate a write-target IRI: syntactically an IRI and outside the reserved
/// `urn:sparq:` space.
fn valid_target(url: &str) -> Result<NamedNode, String> {
    if url.starts_with(RESERVED_PREFIX) {
        return Err(format!("invalid target <{url}>: the `{RESERVED_PREFIX}` graph space is reserved"));
    }
    NamedNode::new(url).map_err(|e| format!("invalid resource IRI <{url}>: {e}"))
}

/// Extract the required string `uri` of a `resources/*` request, or the JSON-RPC
/// invalid-params error naming the method. [SONNET-4.6] sq-cmjmr
fn resource_uri_param<'a>(params: &'a Value, method: &str) -> Result<&'a str, RpcError> {
    params.get("uri").and_then(Value::as_str).ok_or_else(|| {
        RpcError::new(INVALID_PARAMS, format!("{} requires a string `uri`", method))
    })
}

/// Pretty-print a JSON tool result.
fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// One recorded containment-link addition (for rollback): the parent graph's name and
/// the `(parent, ldp:contains, child)` triple inserted into it.
type AddedLink = (Term, [Term; 3]);

impl SolidMcpServer {
    /// Build a read-only pod server over `store` with the default
    /// [`SolidServerConfig`] (anonymous session, WAC, writes disabled), materializing
    /// the WAC authorization view.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the initial materialization fails (e.g. an ACL names a
    /// reserved-encoding principal); no server is built on an unevaluable policy.
    pub fn new(store: PodStore) -> Result<Self, String> {
        Self::with_config(store, SolidServerConfig::default())
    }

    /// Build a pod server over `store` with an explicit [`SolidServerConfig`],
    /// (re-)materializing the authorization view under the configured model so the
    /// served view is never stale and always matches [`SolidServerConfig::acp`].
    ///
    /// # Errors
    ///
    /// Returns `Err` if that materialization fails.
    pub fn with_config(store: PodStore, config: SolidServerConfig) -> Result<Self, String> {
        let mut server = SolidMcpServer { store, config, subs: Subscriptions::default() };
        server.rematerialize()?;
        Ok(server)
    }

    /// Whether the mutating tools are exposed (mirrors
    /// [`SolidServerConfig::allow_update`]).
    pub fn allow_update(&self) -> bool {
        self.config.allow_update
    }

    /// Read-only borrow of the served pod store (for embedders / tests).
    pub fn store(&self) -> &PodStore {
        &self.store
    }

    /// Handle one raw JSON-RPC message — the pod twin of
    /// [`crate::McpServer::handle_message`], sharing the very same framing core
    /// (`Some(response)` for a request, `None` for a notification, and top-level
    /// JSON-RPC batch arrays accepted per JSON-RPC 2.0 §6).
    pub fn handle_message(&mut self, raw: &str) -> Option<String> {
        handle_raw(raw, |req| self.dispatch(req))
    }

    /// The session this server is bound to (borrowing the config's owned strings).
    fn session(&self) -> Session<'_> {
        Session {
            agent: self.config.agent.as_deref(),
            client: self.config.client.as_deref(),
            issuer: self.config.issuer.as_deref(),
            now: self.config.now.as_deref(),
        }
    }

    /// The per-call query budget (same defaults as the base server).
    fn budget(&self) -> QueryBudget {
        let mut b = QueryBudget::unlimited();
        if let Some(secs) = self.config.query_timeout_secs {
            b.deadline = Some(std::time::Instant::now() + Duration::from_secs(secs));
        }
        b.max_rows = self.config.max_rows;
        b
    }

    /// Fail-closed point decision: may this session do `mode` on `url`?
    fn allowed(&self, url: &str, mode: Mode) -> bool {
        self.store.decide(&self.session(), url, mode).allow
    }

    /// (Re-)materialize the authorization view under the configured model.
    fn rematerialize(&mut self) -> Result<(), String> {
        if self.config.acp {
            self.store.materialize_acp().map(|_| ())
        } else {
            self.store.materialize_wac().map(|_| ())
        }
    }

    /// The advertised tool list: read tools always; mutating tools only when enabled.
    pub fn advertised(&self) -> Vec<&'static ToolSpec> {
        let mut tools: Vec<&'static ToolSpec> = vec![
            &POD_QUERY,
            &RESOURCE_GET,
            &CONTAINER_LIST,
            &POD_INTROSPECT,
            &POD_SHAPES,
            &POD_STATS,
        ];
        if self.allow_update() {
            tools.extend([&POD_UPDATE, &RESOURCE_PUT, &RESOURCE_DELETE, &CONTAINER_CREATE]);
        }
        tools
    }

    /// Route a request to its method handler (same protocol surface as the base
    /// server: `initialize`, `ping`, `tools/list`, `tools/call`).
    fn dispatch(&mut self, req: &Request) -> Result<Value, RpcError> {
        match req.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": crate::server::negotiate_protocol_version(&req.params),
                // [SONNET-4.6] sq-cmjmr: `resources` with `subscribe: true` — pod
                // documents are MCP resources and a client may subscribe to one.
                // `listChanged` stays FALSE: this server never pushes an unsolicited
                // list-changed notification, and declaring a capability it does not
                // implement would be an overclaim.
                "capabilities": {
                    "tools": {},
                    "resources": { "subscribe": true, "listChanged": false },
                },
                "serverInfo": {
                    "name": self.config.server_name,
                    "version": env!("CARGO_PKG_VERSION"),
                }
            })),
            "notifications/initialized" | "initialized" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let tools: Vec<Value> = self.advertised().iter().map(|t| t.to_json()).collect();
                Ok(json!({ "tools": tools }))
            }
            "tools/call" => self.tools_call(&req.params),
            "resources/list" => Ok(self.resources_list()),
            "resources/read" => self.resources_read(&req.params),
            "resources/subscribe" => self.resources_subscribe(&req.params),
            "resources/unsubscribe" => self.resources_unsubscribe(&req.params),
            other => {
                Err(RpcError::new(METHOD_NOT_FOUND, format!("method not found: {other}")))
            }
        }
    }

    /// Handle a `tools/call`: gate the mutating tools, run, wrap as `CallToolResult`.
    fn tools_call(&mut self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call requires a string `name`"))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        let mutating = MUTATING.contains(&name);

        // Reject every mutating tool up front when writes are disabled — before any
        // argument is even parsed (the same fail-closed shape as the base server).
        if mutating && !self.allow_update() {
            return Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!(
                    "tool `{name}` is not available: this server is read-only \
                     (start it with updates enabled to allow writes)"
                ),
            ));
        }

        // [SONNET-4.6] sq-cmjmr: snapshot every subscribed topic BEFORE a mutating tool
        // runs, so the change it causes is detected uniformly — including the SPARQL
        // `update` tool, which does not report which documents it touched. Costs nothing
        // when nothing is subscribed. The read-authorization snapshot is taken at the same
        // instant: a topic the mutation DELETES no longer has a policy afterwards, so its
        // pre-mutation readability is the only honest record of whether this session was
        // still allowed to learn about it (see `queue_topic_changes`).
        let (before, readable_before) = if mutating {
            (self.digest_subscribed(), self.readable_subscribed())
        } else {
            (BTreeMap::new(), BTreeSet::new())
        };

        let result = match name {
            "query" => self.tool_query(&args),
            "resource_get" => self.tool_resource_get(&args),
            "container_list" => self.tool_container_list(&args),
            "introspect" => self.tool_introspect(&args),
            "shapes" => self.tool_shapes(&args),
            "stats" => self.tool_stats(),
            "update" => self.tool_update(&args),
            "resource_put" => self.tool_resource_put(&args),
            "resource_delete" => self.tool_resource_delete(&args),
            "container_create" => self.tool_container_create(&args),
            other => {
                return Err(RpcError::new(METHOD_NOT_FOUND, format!("unknown tool: {other}")))
            }
        };

        if mutating {
            // A tool that failed (or that rolled back) leaves every digest equal, so this
            // emits nothing — no bespoke per-tool success plumbing needed.
            self.queue_topic_changes(&before, &readable_before);
        }

        match result {
            Ok(text) => Ok(tool_text_result(text, false)),
            Err(msg) => Ok(tool_text_result(msg, true)),
        }
    }

    /// `query`: session-scoped SELECT/ASK through the zero-copy dataset view, with the
    /// spec-conformant empty-default-graph / union-opt-in rewrite and the configured
    /// budget. Authorization never errors: unauthorized graphs are invisible.
    fn tool_query(&self, args: &Value) -> Result<String, String> {
        let sparql = arg_str(args, "sparql")?;
        let wrapped = sparq_solid::wrap_for_view_opt_in(sparql)?;
        let view = self.store.view_for(&self.session(), Mode::Read);
        sparq_engine::query_json_view_with_budget(&view, &wrapped, &self.budget())
    }

    /// Borrow the named graph stored under `url`, if any.
    fn find_doc(&self, url: &str) -> Option<&Graph> {
        let name = NamedNode::new(url).ok()?;
        let term = Term::NamedNode(name);
        self.store.graph.named.iter().find(|(n, _)| *n == term).map(|(_, g)| g)
    }

    /// The index of the named-graph slot for `term`, if present.
    fn slot_index(&self, term: &Term) -> Option<usize> {
        self.store.graph.named.iter().position(|(n, _)| n == term)
    }

    /// `resource_get`: serve one document from the shared dataset (draft §6.4), with
    /// existence non-disclosure (draft §9.3).
    fn tool_resource_get(&self, args: &Value) -> Result<String, String> {
        let url = arg_str(args, "url")?;
        NamedNode::new(url).map_err(|e| format!("invalid resource IRI <{url}>: {e}"))?;
        // Negotiate BEFORE the read gate: the choice depends only on `accept`, so an
        // unsupported media type answers identically whether or not the resource exists.
        let representation = negotiate(args.get("accept").and_then(Value::as_str))?;
        let content = self.document_as(url, representation)?;
        Ok(pretty(&json!({
            "url": url,
            "content_type": representation.media_type(),
            "content": content,
        })))
    }

    /// One pod document rendered in the negotiated `representation`, behind the read gate.
    /// Both syntaxes come from the SAME `document_triples` call, so negotiation can only
    /// change the spelling, never the content. [SONNET-4.6] sq-wbsf5
    fn document_as(&self, url: &str, representation: Representation) -> Result<String, String> {
        let triples = self.document_triples(url)?;
        match representation {
            Representation::NTriples => Ok(sparq_engine::triples_to_ntriples(&triples)),
            Representation::Turtle => triples_to_turtle(&triples),
        }
    }

    /// The N-Triples representation of one pod document — what the `resources/read`
    /// RESOURCE surface serves (MCP has no per-request `accept`, so it is not negotiable
    /// there). It is the SAME rendering of the SAME triples the `resource_get` TOOL
    /// serves by default, so the two surfaces cannot disagree (draft §6.4/§8).
    fn document_ntriples(&self, url: &str) -> Result<String, String> {
        self.document_as(url, Representation::NTriples)
    }

    /// The triples of one pod document — THE read gate every representation passes
    /// through, under the configured budget.
    ///
    /// NON-DISCLOSURE (draft §9.3): unauthorized-read and nonexistent produce the SAME
    /// error, so no surface built on this reveals that a resource exists.
    fn document_triples(&self, url: &str) -> Result<Vec<Triple>, String> {
        if !self.allowed(url, Mode::Read) {
            return Err(not_found_error(url));
        }
        let Some(doc) = self.find_doc(url) else {
            return Err(not_found_error(url));
        };
        sparq_engine::construct_with_budget(
            doc,
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
            &self.budget(),
        )
    }

    /// `container_list`: direct `ldp:contains` members from the container's OWN stored
    /// graph — data-derived, never IRI-path guessing (draft §6.4).
    fn tool_container_list(&self, args: &Value) -> Result<String, String> {
        let url = arg_str(args, "url")?;
        NamedNode::new(url).map_err(|e| format!("invalid container IRI <{url}>: {e}"))?;
        if !self.allowed(url, Mode::Read) {
            return Err(not_found_error(url));
        }
        let Some(doc) = self.find_doc(url) else {
            return Err(not_found_error(url));
        };
        let q = format!("SELECT ?m WHERE {{ <{url}> <{LDP_CONTAINS}> ?m }}");
        let res = sparq_engine::query_with_budget(doc, &q, &self.budget())?;
        let mut members: Vec<Value> = Vec::with_capacity(res.rows.len());
        for row in &res.rows {
            if let Some(Some(Term::NamedNode(m))) = row.first() {
                members.push(json!({
                    "url": m.as_str(),
                    "container": m.as_str().ends_with('/'),
                }));
            }
        }
        members.sort_by(|a, b| a["url"].as_str().cmp(&b["url"].as_str()));
        Ok(pretty(&json!({ "url": url, "members": members })))
    }

    // ───────── session-scoped aggregates (schema / statistics), sq-8n6iv ─────────

    /// The session's authorized data as ONE standalone graph — the single input every
    /// aggregate tool below is derived from.
    ///
    /// It is built by CONSTRUCTing `GRAPH ?g { ?s ?p ?o }` **through the same
    /// [`DatasetView`](sparq_engine::DatasetView) the `query` tool evaluates under**
    /// ([`PodStore::view_for`]), so the set of documents it can possibly contain is
    /// exactly the set `query` can read: a document this session may not read is not
    /// enumerated by `GRAPH ?g` at all, and therefore contributes to no count. That
    /// shared view is the whole point — an aggregate derived from `store.graph` directly
    /// would be a whole-pod summary handed to one principal, which in a multi-principal
    /// deployment leaks the shape (and the size) of documents it cannot open.
    ///
    /// The projection is a COPY (the authorized sub-graphs each own a private
    /// dictionary, so there is no zero-copy union to introspect over) and is rebuilt per
    /// call rather than cached, because a mutating tool or an `.acl` write can change
    /// either the data or the authorized set between calls and a stale schema would be a
    /// silent lie. Cost is linear in authorized data — the same order as the
    /// `Introspection::build` it feeds.
    ///
    /// It runs under the configured [budget](Self::budget), whose `max_rows` **refuses
    /// rather than truncates**, so an over-budget pod yields an error, never a quietly
    /// undercounted schema.
    fn authorized_projection(&self) -> Result<Graph, String> {
        let view = self.store.view_for(&self.session(), Mode::Read);
        let budget = self.budget();
        let triples = sparq_engine::with_view(&view, || {
            sparq_engine::construct_with_budget(
                view.base,
                "CONSTRUCT { ?s ?p ?o } WHERE { GRAPH ?g { ?s ?p ?o } }",
                &budget,
            )
        })?;
        Graph::load_str(&sparq_engine::triples_to_ntriples(&triples), "ntriples")
            .map_err(|e| format!("authorized projection did not round-trip: {}", e))
    }

    /// The mined schema of [the authorized projection](Self::authorized_projection).
    fn introspection(&self) -> Result<Introspection, String> {
        Ok(Introspection::build(&self.authorized_projection()?))
    }

    /// `introspect`: the effective schema of the documents THIS session may read, as
    /// JSON (default) or a token-budgeted text summary.
    fn tool_introspect(&self, args: &Value) -> Result<String, String> {
        let format = args.get("format").and_then(Value::as_str).unwrap_or("json");
        let ix = self.introspection()?;
        match format {
            "json" => Ok(ix.to_json()),
            "text" => {
                let budget = args
                    .get("budget_chars")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or(4000);
                Ok(ix.to_text_summary(budget))
            }
            other => Err(format!("unknown format `{}` (expected \"json\" or \"text\")", other)),
        }
    }

    /// `shapes`: one class's data-grounded shape, mined from the authorized projection
    /// only — the same miner the base server uses, over a session-scoped graph.
    fn tool_shapes(&self, args: &Value) -> Result<String, String> {
        let class_iri = arg_str(args, "class")?;
        let ix = self.introspection()?;
        let shape = crate::shapes::class_shape(&ix, class_iri)?;
        Ok(pretty(&shape))
    }

    /// `stats`: dataset totals over the authorized projection. A session with no read
    /// grants gets zeros — the fail-closed answer, not the pod's totals.
    ///
    /// `triples` is the size of the projection, i.e. of the RDF **merge** of the readable
    /// documents: a triple asserted in two of them is one triple here, not two. That is
    /// the number every other figure in this object is consistent with, which is why it is
    /// reported rather than the sum of the per-document sizes.
    fn tool_stats(&self) -> Result<String, String> {
        let projection = self.authorized_projection()?;
        let ix = Introspection::build(&projection);
        Ok(pretty(&json!({
            "triples": projection.len(),
            "distinct_subjects": ix.subjects,
            "typed_entities": ix.entities,
            "classes": ix.classes.len(),
            "predicates": ix.predicates.len(),
            "namespaces": ix.vocabularies.distinct,
        })))
    }

    // ───────── the MCP `resources` surface + notifications (draft §8/§10) ─────────

    /// Drain every notification queued since the last call, each a complete JSON-RPC
    /// notification message (`notifications/resources/updated`) ready to write to the
    /// transport verbatim.
    ///
    /// Delivery is PULL-based because this crate's dispatch core owns no transport: call
    /// this after each [`SolidMcpServer::handle_message`] and write whatever it returns
    /// alongside the response. Each message carries only the topic IRI and the
    /// ActivityStreams 2.0 activity type — the payload is content-free by construction
    /// (draft §10). Only topics that passed the read check at delivery time appear.
    /// [SONNET-4.6] sq-cmjmr
    pub fn take_notifications(&mut self) -> Vec<String> {
        self.subs.take()
    }

    /// The topic IRIs this session currently subscribes to, sorted (for embedders and
    /// tests). [SONNET-4.6] sq-cmjmr
    pub fn subscribed_topics(&self) -> Vec<String> {
        self.subs.topics()
    }

    /// Digest every subscribed topic that currently exists. Topics absent from the map
    /// do not exist right now — [`classify`] turns an absent→present transition into
    /// `Create` and present→absent into `Delete`.
    fn digest_subscribed(&self) -> BTreeMap<String, TopicDigest> {
        let mut digests = BTreeMap::new();
        if self.subs.is_empty() {
            return digests;
        }
        for topic in self.subs.topics() {
            if let Some(doc) = self.find_doc(&topic) {
                digests.insert(topic, TopicDigest::of(doc));
            }
        }
        digests
    }

    /// The subscribed topics this session may read RIGHT NOW, checked against each topic
    /// ITSELF (never an ancestor). Snapshotted alongside [`Self::digest_subscribed`] so a
    /// `Delete` delivery can be held to the topic's own pre-deletion policy — the policy
    /// that vanishes with the resource. [SONNET-4.6] sq-cmjmr
    fn readable_subscribed(&self) -> BTreeSet<String> {
        let mut readable = BTreeSet::new();
        if self.subs.is_empty() {
            return readable;
        }
        for topic in self.subs.topics() {
            if self.allowed(&topic, Mode::Read) {
                readable.insert(topic);
            }
        }
        readable
    }

    /// Compare the current topic digests against `before` and queue one notification per
    /// changed topic — after RE-CHECKING read access against the post-mutation
    /// authorization view.
    ///
    /// A session whose read access was revoked (possibly by this very write, e.g. an
    /// `.acl` replacement) is dropped SILENTLY: no notification, no error, no "you were
    /// unsubscribed" message. Telling it would itself disclose that a resource it may no
    /// longer read had changed (draft §10). The subscription survives, so restoring
    /// access resumes deliveries.
    ///
    /// The check runs at the topic's [authorization anchor](Self::auth_anchor), which is
    /// the topic itself whenever the topic still exists. That matters for a `Delete`: a
    /// just-deleted resource has no policy of its own left to evaluate, so a literal
    /// check against it would fail closed and make deletion permanently unnotifiable.
    /// The anchor — the nearest existing ancestor container, the same rule that
    /// authorizes creating and deleting that IRI — is the policy that governs it.
    ///
    /// The ancestor fallback must not RE-GRANT what a resource-specific policy took away.
    /// A session whose read access to the topic itself had been revoked (by an ACL on the
    /// topic) can typically still read the parent container, so anchoring a `Delete` at
    /// the parent alone would deliver — disclosing the deletion of a resource the session
    /// was no longer allowed to read. So a `Delete` needs BOTH: the topic was readable
    /// immediately BEFORE the mutation (`readable_before`, from
    /// [`Self::readable_subscribed`] — the topic's own vanished policy), AND the surviving
    /// governing policy at the anchor still permits disclosure.
    fn queue_topic_changes(
        &mut self,
        before: &BTreeMap<String, TopicDigest>,
        readable_before: &BTreeSet<String>,
    ) {
        if self.subs.is_empty() {
            return;
        }
        let after = self.digest_subscribed();
        for topic in self.subs.topics() {
            let Some(activity) = classify(before.get(&topic), after.get(&topic)) else {
                continue;
            };
            // A deleted topic's OWN pre-deletion policy still has to have allowed the
            // read: the anchor stands in for a missing policy, never for a revoked one.
            if activity == ActivityType::Delete && !readable_before.contains(&topic) {
                continue;
            }
            // DELIVERY-TIME authorization — the second of the two checks (draft §10).
            if !self.allowed(&self.auth_anchor(&topic), Mode::Read) {
                continue;
            }
            self.subs.enqueue(&topic, activity);
        }
    }

    /// `resources/list`: the pod documents THIS session may read, as MCP resources whose
    /// `uri` IS the resource IRI. A document the session cannot read is simply absent —
    /// the listing never errors and never hints that something was filtered.
    fn resources_list(&self) -> Value {
        let mut resources: Vec<Value> = Vec::new();
        for (name, _) in &self.store.graph.named {
            let Term::NamedNode(node) = name else { continue };
            let uri = node.as_str();
            // The reserved space holds the materialized auth view, not pod content.
            if uri.starts_with(RESERVED_PREFIX) || !self.allowed(uri, Mode::Read) {
                continue;
            }
            resources.push(json!({
                "uri": uri,
                // The IRI is the name: deriving a prettier label would be guessing, and
                // the pod is the only authority on what a document is called.
                "name": uri,
                "mimeType": "application/n-triples",
            }));
        }
        resources.sort_by(|a, b| a["uri"].as_str().cmp(&b["uri"].as_str()));
        json!({ "resources": resources })
    }

    /// `resources/read`: the same representation the `resource_get` tool serves, in the
    /// MCP `contents` shape. Unreadable and nonexistent are the same error (draft §9.3).
    fn resources_read(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = resource_uri_param(params, "resources/read")?;
        let text = self
            .document_ntriples(uri)
            .map_err(|message| RpcError::new(RESOURCE_NOT_FOUND, message))?;
        Ok(json!({
            "contents": [ {
                "uri": uri,
                "mimeType": "application/n-triples",
                "text": text,
            } ]
        }))
    }

    /// `resources/subscribe`: bind an MCP subscription to a Solid Notifications
    /// subscription on `uri` as the topic.
    ///
    /// SUBSCRIBE-TIME authorization (the first of the two checks, draft §10): a session
    /// that cannot read the topic gets the existence-non-disclosure not-found error, so
    /// subscribing cannot be used to probe which resources exist. v1 additionally
    /// requires the topic to exist NOW — the fail-closed reading, since a decision on an
    /// absent resource has no policy to evaluate.
    fn resources_subscribe(&mut self, params: &Value) -> Result<Value, RpcError> {
        let uri = resource_uri_param(params, "resources/subscribe")?;
        NamedNode::new(uri).map_err(|e| {
            RpcError::new(INVALID_PARAMS, format!("invalid resource IRI <{}>: {}", uri, e))
        })?;
        if uri.starts_with(RESERVED_PREFIX)
            || !self.allowed(uri, Mode::Read)
            || self.find_doc(uri).is_none()
        {
            return Err(RpcError::new(RESOURCE_NOT_FOUND, not_found_error(uri)));
        }
        self.subs.subscribe(uri);
        Ok(json!({}))
    }

    /// `resources/unsubscribe`: drop the subscription. Idempotent and uniform — an
    /// unknown topic gets the SAME empty result as a known one, so an unsubscribe never
    /// discloses what this session was watching (or whether the topic exists).
    fn resources_unsubscribe(&mut self, params: &Value) -> Result<Value, RpcError> {
        let uri = resource_uri_param(params, "resources/unsubscribe")?;
        self.subs.unsubscribe(uri);
        Ok(json!({}))
    }

    /// `update`: session-checked SPARQL Update (`PodStore::update_as_with_budget` — every
    /// touched graph needs this session's write permission, fail-closed) under the SAME
    /// per-call [budget](Self::budget) the read tools enforce, so a tool-issued update's
    /// SPARQL *evaluation* is bounded exactly as a tool-issued query is (draft §9.4).
    /// [FABLE-5] sq-yhlf0.
    ///
    /// The budget bounds the two places an update evaluates SPARQL: the authorization
    /// check's `GRAPH ?var` binding SELECT and the apply's `DELETE`/`INSERT … WHERE`. It
    /// does NOT make the whole call bounded — see the module-level *Honest v1 limits*: only
    /// the inline `INSERT`/`DELETE DATA` forms are bounded by the accepted request size,
    /// `CLEAR`/`DROP` cost whatever the session may write, and `LOAD` is refused unless the
    /// embedder installs an allowlisted base.
    fn tool_update(&mut self, args: &Value) -> Result<String, String> {
        let sparql = arg_str(args, "sparql")?.to_string();
        let config = self.config.clone();
        let session = Session {
            agent: config.agent.as_deref(),
            client: config.client.as_deref(),
            issuer: config.issuer.as_deref(),
            now: config.now.as_deref(),
        };
        let budget = self.budget();
        if self.config.acp {
            self.store.update_as_acp_with_budget(&session, &sparql, &budget)?;
        } else {
            self.store.update_as_with_budget(&session, &sparql, &budget)?;
        }
        Ok("ok".to_string())
    }

    /// The authorization anchor for `url`: `url` itself when the store knows it (a
    /// stored graph, or a structural container prefix of one — those carry
    /// materialized grants), else the NEAREST ancestor container the store knows —
    /// the Solid creation rule: a `PUT` to a nonexistent resource is authorized by
    /// the closest existing parent container's policy. When no ancestor is known the
    /// url itself is returned and the decision fails closed (`NoAcl`).
    fn auth_anchor(&self, url: &str) -> String {
        let known = |iri: &str| {
            self.store.graph.named.iter().any(|(n, _)| match n {
                Term::NamedNode(nn) => {
                    nn.as_str() == iri || (iri.ends_with('/') && nn.as_str().starts_with(iri))
                }
                _ => false,
            })
        };
        if known(url) {
            return url.to_string();
        }
        let mut cur = url.to_string();
        while let Some(p) = parent_iri(&cur) {
            if known(&p) {
                return p;
            }
            cur = p;
        }
        url.to_string()
    }

    /// The uniform read/write gate for a mutating resource tool, evaluated at the
    /// target's [authorization anchor](Self::auth_anchor): no read ⇒ the
    /// non-disclosure not-found error (named for the TARGET, never the anchor); read
    /// but not `write_mode` ⇒ the distinguishable denied error.
    fn gate_write(&self, url: &str, write_mode: Mode) -> Result<(), String> {
        let anchor = self.auth_anchor(url);
        if !self.allowed(&anchor, Mode::Read) {
            return Err(not_found_error(url));
        }
        if !self.allowed(&anchor, write_mode) {
            return Err(write_denied_error(url));
        }
        Ok(())
    }

    /// The gate for writing an access-control document: `acl:Control` of the GOVERNED
    /// resource (the WAC/ACP rule — Control gates the control document), anchored the
    /// same way for a not-yet-existing subtree, with the same non-disclosure shape
    /// (no Control ⇒ not-found, never "denied").
    fn gate_acl_write(&self, acl_url: &str) -> Result<(), String> {
        let governed = &acl_url[..acl_url.len() - 4]; // strip ".acl"/".acr" (both len 4)
        let anchor = self.auth_anchor(governed);
        if !self.allowed(&anchor, Mode::Control) {
            return Err(not_found_error(acl_url));
        }
        Ok(())
    }

    /// `resource_put`: create/replace one document (LDP PUT). ACL documents route
    /// through the authoritative write-through; content documents are parse-first,
    /// containment-linked on create, and re-materialized with rollback-on-error.
    fn tool_resource_put(&mut self, args: &Value) -> Result<String, String> {
        let url = arg_str(args, "url")?.to_string();
        let content = arg_str(args, "content")?.to_string();
        let content_type = arg_str(args, "content_type")?.to_string();
        let name = valid_target(&url)?;
        // RDF sources only. The refusal NAMES the scope-out (module docs: non-RDF
        // resources are a `sparq-solid` storage decision, not an MCP surface one) so a
        // caller learns the shape of the limit, not just that its body was rejected.
        let fmt = rdf_format(&content_type).ok_or_else(|| {
            format!(
                "unsupported content_type `{}`: this server stores RDF sources only ({}); \
                 non-RDF (binary) resources are out of scope",
                content_type, INGESTED_MEDIA_TYPES
            )
        })?;

        if is_control_doc(&url) {
            // ACL write-through (draft §7.3): authorize on Control of the governed
            // resource, then let sparq-solid swap + re-materialize atomically
            // (fail-closed rollback on an unevaluable policy).
            self.gate_acl_write(&url)?;
            let out = if self.config.acp {
                self.store.put_acl_acp(&url, &content, fmt)?
            } else {
                self.store.put_acl(&url, &content, fmt)?
            };
            return Ok(pretty(&json!({
                "url": url,
                "created": !out.existed,
                "triples": out.triples,
                "access_control": true,
            })));
        }

        if url.ends_with('/') {
            return Err(format!(
                "<{url}> is a container IRI — use `container_create` to create containers"
            ));
        }
        self.gate_write(&url, Mode::Write)?;

        // PARSE FIRST — a malformed body is rejected before anything mutates.
        let new_graph = Graph::load_str(&content, fmt)
            .map_err(|e| format!("content for <{url}> did not parse as {fmt}: {e}"))?;
        let triples = new_graph.len();

        // Swap the document slot (capturing the prior content for rollback).
        let term = Term::NamedNode(name);
        let prior = self.take_slot(&term);
        let existed = prior.is_some();
        self.store.graph.named.push((term.clone(), new_graph));

        // A CREATED document becomes a member of its parent containers (draft §7.3).
        let links = if existed { Ok((Vec::new(), Vec::new())) } else { self.link_containment(&url) };
        let (created_graphs, added_links) = match links {
            Ok(x) => x,
            Err(e) => {
                self.restore_slot(&term, prior);
                return Err(e);
            }
        };

        // Re-materialize so the write is visible (a created doc enters the authorized
        // sets; a replaced group document takes effect) — rollback on error.
        if let Err(e) = self.rematerialize() {
            self.unlink_containment(&created_graphs, &added_links);
            self.restore_slot(&term, prior);
            let _ = self.rematerialize(); // restore the prior (already-valid) view
            return Err(format!("resource_put rolled back for <{url}>: {e}"));
        }
        Ok(pretty(&json!({ "url": url, "created": !existed, "triples": triples })))
    }

    /// `resource_delete`: delete one document (LDP DELETE). Rejects non-empty
    /// containers; ACL documents route through the authoritative write-through.
    fn tool_resource_delete(&mut self, args: &Value) -> Result<String, String> {
        let url = arg_str(args, "url")?.to_string();
        let name = valid_target(&url)?;

        if is_control_doc(&url) {
            self.gate_acl_write(&url)?;
            let out = if self.config.acp {
                self.store.delete_acl_acp(&url)?
            } else {
                self.store.delete_acl(&url)?
            };
            return Ok(pretty(&json!({
                "url": url,
                "deleted": out.existed,
                "access_control": true,
            })));
        }

        if !self.allowed(&url, Mode::Read) {
            return Err(not_found_error(&url));
        }
        let term = Term::NamedNode(name);
        if self.slot_index(&term).is_none() {
            return Err(not_found_error(&url));
        }
        if !self.allowed(&url, Mode::Write) {
            return Err(write_denied_error(&url));
        }
        if url.ends_with('/') {
            // A container: reject when its STORED containment listing is non-empty.
            let doc = self.find_doc(&url).expect("slot checked above");
            let q = format!("ASK {{ <{url}> <{LDP_CONTAINS}> ?m }}");
            if sparq_engine::ask(doc, &q)? {
                return Err(format!("container <{url}> is not empty — delete its members first"));
            }
        }

        // Remove the document and its parent's containment link (rollback-capable).
        let prior = self.take_slot(&term).expect("slot checked above");
        let removed_link = self.unlink_parent(&url);
        if let Err(e) = self.rematerialize() {
            if let Some((pterm, t)) = &removed_link {
                if let Some(i) = self.slot_index(pterm) {
                    let _ = self.store.graph.named[i].1.insert_triple(
                        t[0].clone(),
                        t[1].clone(),
                        t[2].clone(),
                    );
                }
            }
            self.restore_slot(&term, Some(prior));
            let _ = self.rematerialize();
            return Err(format!("resource_delete rolled back for <{url}>: {e}"));
        }
        Ok(pretty(&json!({ "url": url, "deleted": true })))
    }

    /// `container_create`: create one empty, typed container and link it into its
    /// parent containers.
    fn tool_container_create(&mut self, args: &Value) -> Result<String, String> {
        let url = arg_str(args, "url")?.to_string();
        let name = valid_target(&url)?;
        if !url.ends_with('/') {
            return Err(format!("container IRI must be slash-terminated: <{url}>"));
        }
        self.gate_write(&url, Mode::Write)?;
        let term = Term::NamedNode(name);
        if self.slot_index(&term).is_some() {
            return Err(format!("container <{url}> already exists"));
        }

        let container = container_graph(&url)?;
        self.store.graph.named.push((term.clone(), container));
        let (created_graphs, added_links) = match self.link_containment(&url) {
            Ok(x) => x,
            Err(e) => {
                self.restore_slot(&term, None);
                return Err(e);
            }
        };
        if let Err(e) = self.rematerialize() {
            self.unlink_containment(&created_graphs, &added_links);
            self.restore_slot(&term, None);
            let _ = self.rematerialize();
            return Err(format!("container_create rolled back for <{url}>: {e}"));
        }
        Ok(pretty(&json!({ "url": url, "created": true })))
    }

    /// Remove and return the named-graph slot for `term`, if present.
    fn take_slot(&mut self, term: &Term) -> Option<Graph> {
        let pos = self.slot_index(term)?;
        Some(self.store.graph.named.swap_remove(pos).1)
    }

    /// Put `prior` back into the slot for `term` (dropping whatever is there now).
    fn restore_slot(&mut self, term: &Term, prior: Option<Graph>) {
        let _ = self.take_slot(term);
        if let Some(g) = prior {
            self.store.graph.named.push((term.clone(), g));
        }
    }

    /// Walk the parent chain of a CREATED resource up to the origin root, inserting
    /// each missing `(parent, ldp:contains, child)` triple into the parent container's
    /// stored graph (creating a typed container graph for a missing parent). Returns
    /// what was added — `(created container-graph names, inserted links)` — so a
    /// failed re-materialization can roll every addition back precisely.
    fn link_containment(&mut self, url: &str) -> Result<(Vec<Term>, Vec<AddedLink>), String> {
        let mut created: Vec<Term> = Vec::new();
        let mut added: Vec<AddedLink> = Vec::new();
        let contains = NamedNode::new(LDP_CONTAINS).expect("valid constant IRI");
        let mut child = url.to_string();
        while let Some(parent) = parent_iri(&child) {
            let pnode = NamedNode::new(parent.as_str())
                .map_err(|e| format!("invalid parent container IRI <{parent}>: {e}"))?;
            let pterm = Term::NamedNode(pnode.clone());
            let pos = match self.slot_index(&pterm) {
                Some(i) => i,
                None => {
                    self.store.graph.named.push((pterm.clone(), container_graph(&parent)?));
                    created.push(pterm.clone());
                    self.store.graph.named.len() - 1
                }
            };
            let q = format!("ASK {{ <{parent}> <{LDP_CONTAINS}> <{child}> }}");
            let present = sparq_engine::ask(&self.store.graph.named[pos].1, &q)?;
            if !present {
                let cnode = NamedNode::new(child.as_str())
                    .map_err(|e| format!("invalid member IRI <{child}>: {e}"))?;
                let triple = [
                    Term::NamedNode(pnode.clone()),
                    Term::NamedNode(contains.clone()),
                    Term::NamedNode(cnode),
                ];
                self.store.graph.named[pos].1.insert_triple(
                    triple[0].clone(),
                    triple[1].clone(),
                    triple[2].clone(),
                )?;
                added.push((pterm, triple));
            }
            child = parent;
        }
        Ok((created, added))
    }

    /// Roll back exactly what [`SolidMcpServer::link_containment`] added.
    fn unlink_containment(&mut self, created: &[Term], added: &[AddedLink]) {
        for (pterm, t) in added.iter().rev() {
            if let Some(i) = self.slot_index(pterm) {
                let _ =
                    self.store.graph.named[i].1.remove_triple(t[0].clone(), t[1].clone(), t[2].clone());
            }
        }
        for term in created.iter().rev() {
            let _ = self.take_slot(term);
        }
    }

    /// Remove the `(parent, ldp:contains, url)` link from the parent container's
    /// stored graph, returning it for rollback (`None` when absent).
    fn unlink_parent(&mut self, url: &str) -> Option<AddedLink> {
        let parent = parent_iri(url)?;
        let pterm = Term::NamedNode(NamedNode::new(parent.as_str()).ok()?);
        let pos = self.slot_index(&pterm)?;
        let q = format!("ASK {{ <{parent}> <{LDP_CONTAINS}> <{url}> }}");
        if !sparq_engine::ask(&self.store.graph.named[pos].1, &q).unwrap_or(false) {
            return None;
        }
        let triple = [
            Term::NamedNode(NamedNode::new(parent.as_str()).ok()?),
            Term::NamedNode(NamedNode::new(LDP_CONTAINS).expect("valid constant IRI")),
            Term::NamedNode(NamedNode::new(url).ok()?),
        ];
        let _ = self.store.graph.named[pos].1.remove_triple(
            triple[0].clone(),
            triple[1].clone(),
            triple[2].clone(),
        );
        Some((pterm, triple))
    }
}

/// A fresh, typed container document: `<url> a ldp:BasicContainer, ldp:Container .`
fn container_graph(url: &str) -> Result<Graph, String> {
    let nt = format!(
        "<{url}> <{RDF_TYPE}> <{LDP_BASIC_CONTAINER}> .\n<{url}> <{RDF_TYPE}> <{LDP_CONTAINER}> .\n"
    );
    Graph::load_str(&nt, "ntriples")
}

#[cfg(test)]
mod unit {
    use super::*;

    // Direct unit tests for the new public functions/consts (coverage-floor rule:
    // one direct test per new public fn), plus the private path helpers.

    #[test]
    fn parent_iri_walks_slash_semantics_to_the_origin_root() {
        assert_eq!(parent_iri("https://h.ex/a/b/doc.ttl").as_deref(), Some("https://h.ex/a/b/"));
        assert_eq!(parent_iri("https://h.ex/a/b/").as_deref(), Some("https://h.ex/a/"));
        assert_eq!(parent_iri("https://h.ex/a/").as_deref(), Some("https://h.ex/"));
        assert_eq!(parent_iri("https://h.ex/doc"), Some("https://h.ex/".to_string()));
        assert_eq!(parent_iri("https://h.ex/"), None);
        assert_eq!(parent_iri("urn:sparq:auth"), None); // no slash hierarchy
    }

    #[test]
    fn rdf_format_maps_media_types_and_rejects_non_rdf() {
        assert_eq!(rdf_format("text/turtle"), Some("turtle"));
        assert_eq!(rdf_format("text/turtle; charset=utf-8"), Some("turtle"));
        assert_eq!(rdf_format("application/n-triples"), Some("ntriples"));
        assert_eq!(rdf_format("application/octet-stream"), None);
        #[cfg(not(feature = "jsonld"))]
        assert_eq!(rdf_format("application/ld+json"), None); // fail closed without parser
    }

    #[test]
    fn negotiate_defaults_to_ntriples_and_maps_turtle() {
        // [SONNET-4.6] sq-wbsf5 — absent `accept` must keep the v1 default, or every
        // existing caller silently changes syntax.
        assert_eq!(negotiate(None), Ok(Representation::NTriples));
        assert_eq!(negotiate(Some("application/n-triples")), Ok(Representation::NTriples));
        assert_eq!(negotiate(Some("text/turtle")), Ok(Representation::Turtle));
        // Media-type parameters are ignored, so a `q=` weight still resolves.
        assert_eq!(negotiate(Some("text/turtle; q=0.9")), Ok(Representation::Turtle));
    }

    #[test]
    fn negotiate_refuses_an_unservable_accept_naming_what_is_served() {
        // No silent coercion, and the message must enumerate the real served set.
        for accept in ["application/rdf+xml", "image/png", "application/n-triples, text/turtle"] {
            let err = negotiate(Some(accept)).expect_err("unservable accept is an error");
            assert!(err.contains("unsupported accept"), "honest error: {err}");
            assert!(err.contains("application/n-triples") && err.contains("text/turtle"));
        }
    }

    #[cfg(feature = "jsonld")]
    #[test]
    fn negotiate_refuses_jsonld_even_though_it_can_be_ingested() {
        // The `jsonld` feature adds a PARSER, not a writer: serving it would be a lie.
        assert!(negotiate(Some("application/ld+json")).is_err());
    }

    #[test]
    fn representation_media_types_are_the_wire_spellings() {
        assert_eq!(Representation::NTriples.media_type(), "application/n-triples");
        assert_eq!(Representation::Turtle.media_type(), "text/turtle");
    }

    #[test]
    fn triples_to_turtle_writes_well_formed_prefix_compacted_turtle() {
        let graph = Graph::load_str(
            "<https://pod.ex/a> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/a/b> .\n",
            "ntriples",
        )
        .expect("fixture parses");
        let triples =
            sparq_engine::construct(&graph, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
                .expect("CONSTRUCT evaluates");
        let ttl = triples_to_turtle(&triples).expect("in-memory serialization succeeds");
        assert!(ttl.contains("ldp:contains"), "registered prefixes compact: {ttl}");
        // Well-formed: it parses back to the same one triple.
        let back = Graph::load_str(&ttl, "turtle").expect("output is valid Turtle");
        assert_eq!(back.len(), 1);
        assert!(sparq_engine::ask(
            &back,
            "ASK { <https://pod.ex/a> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/a/b> }"
        )
        .expect("ASK evaluates"));
    }

    #[test]
    fn triples_to_turtle_of_no_triples_is_an_empty_document() {
        assert_eq!(triples_to_turtle(&[]).expect("empty serializes"), "");
    }

    #[cfg(feature = "jsonld")]
    #[test]
    fn rdf_format_accepts_jsonld_when_feature_on() {
        assert_eq!(rdf_format("application/ld+json"), Some("jsonld"));
        assert_eq!(rdf_format("application/ld+json; charset=utf-8"), Some("jsonld"));
        assert_eq!(rdf_format("jsonld"), Some("jsonld"));
        assert_eq!(rdf_format("json-ld"), Some("jsonld"));
    }

    #[cfg(feature = "jsonld")]
    #[test]
    fn resource_put_ingests_jsonld_into_the_named_resource_graph() {
        // [GPT-5.6] The root ACL grants Alice write access inherited by the new
        // resource, so this exercises resource_put's parse-first path rather than
        // calling Graph::load_str in isolation.
        let fixture = r#"
<https://pod.ex/> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Container> <https://pod.ex/> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .
"#;
        let store = PodStore::new(Graph::load_dataset(fixture, "nquads").expect("fixture parses"));
        let config = SolidServerConfig {
            agent: Some("https://alice.ex/card#me".to_string()),
            allow_update: true,
            ..SolidServerConfig::default()
        };
        let mut server = SolidMcpServer::with_config(store, config).expect("materializes");
        let result = server
            .tool_resource_put(&json!({
                "url": "https://pod.ex/profile",
                "content_type": "application/ld+json",
                "content": r#"{
                    "@id": "https://pod.ex/profile#me",
                    "https://schema.org/name": "Alice"
                }"#
            }))
            .expect("JSON-LD resource_put succeeds");

        assert_eq!(serde_json::from_str::<Value>(&result).unwrap()["triples"], 1);
        let graph = server
            .store()
            .graph
            .named
            .iter()
            .find(|(name, _)| name.to_string() == "<https://pod.ex/profile>")
            .map(|(_, graph)| graph)
            .expect("resource graph was stored");
        assert!(
            sparq_engine::ask(
                graph,
                "ASK { <https://pod.ex/profile#me> <https://schema.org/name> \"Alice\" }"
            )
            .expect("ASK evaluates")
        );
    }

    #[test]
    fn not_found_error_is_a_pure_function_of_the_url() {
        // Load-bearing for §9.3: the SAME template must serve both the nonexistent
        // and the unauthorized case, so it may depend on nothing but the URL.
        assert_eq!(not_found_error("https://p.ex/x"), "resource not found: <https://p.ex/x>");
    }

    #[test]
    fn valid_target_rejects_the_reserved_graph_space() {
        assert!(valid_target("urn:sparq:auth").is_err());
        assert!(valid_target("not an iri").is_err());
        assert!(valid_target("https://p.ex/ok").is_ok());
    }

    #[test]
    fn default_config_is_read_only_wac_and_bounded() {
        let c = SolidServerConfig::default();
        assert!(!c.allow_update);
        assert!(!c.acp);
        assert_eq!(c.query_timeout_secs, Some(30));
        assert_eq!(c.max_rows, Some(1_000_000));
        assert_eq!(c.server_name, "sparq-mcp-solid");
        assert!(c.agent.is_none() && c.client.is_none() && c.issuer.is_none() && c.now.is_none());
    }

    /// A two-principal pod: alice reads `pub/`, bob reads `priv/` — and neither can
    /// read the other's document. Both documents use a DIFFERENT class and predicate,
    /// so any aggregate that spilled across the boundary is visible as a name, not just
    /// as a count. [SONNET-4.6] sq-8n6iv
    fn split_pod() -> PodStore {
        let nq = r#"
<https://pod.ex/pub/d#it> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://ex.dev/ns#Public> <https://pod.ex/pub/d> .
<https://pod.ex/pub/d#it> <https://ex.dev/ns#openField> "open" <https://pod.ex/pub/d> .
<https://pod.ex/priv/d#it> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://ex.dev/ns#Secret> <https://pod.ex/priv/d> .
<https://pod.ex/priv/d#it> <https://ex.dev/ns#secretField> "shh" <https://pod.ex/priv/d> .
<https://pod.ex/pub/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/pub/.acl> .
<https://pod.ex/pub/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/pub/> <https://pod.ex/pub/.acl> .
<https://pod.ex/pub/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/pub/.acl> .
<https://pod.ex/pub/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/pub/.acl> .
<https://pod.ex/priv/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/priv/.acl> .
<https://pod.ex/priv/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/priv/> <https://pod.ex/priv/.acl> .
<https://pod.ex/priv/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/priv/.acl> .
<https://pod.ex/priv/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/priv/.acl> .
"#;
        PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture parses"))
    }

    fn split_server(agent: Option<&str>) -> SolidMcpServer {
        let config = SolidServerConfig {
            agent: agent.map(str::to_string),
            ..SolidServerConfig::default()
        };
        SolidMcpServer::with_config(split_pod(), config).expect("materializes")
    }

    #[test]
    fn authorized_projection_holds_exactly_the_readable_documents() {
        // [SONNET-4.6] sq-8n6iv — THE aggregate boundary: the projection every schema
        // and statistics tool is mined from must contain alice's document and nothing
        // of bob's. Widening it to `store.graph` flips this red.
        let alice = split_server(Some("https://alice.ex/card#me"));
        let projection = alice.authorized_projection().expect("projection builds");
        assert_eq!(projection.len(), 2, "exactly the two triples of pub/d");
        assert!(sparq_engine::ask(
            &projection,
            "ASK { <https://pod.ex/pub/d#it> <https://ex.dev/ns#openField> \"open\" }"
        )
        .expect("ASK evaluates"));
        assert!(
            !sparq_engine::ask(&projection, "ASK { ?s <https://ex.dev/ns#secretField> ?o }")
                .expect("ASK evaluates"),
            "the unreadable document must contribute nothing"
        );
    }

    #[test]
    fn authorized_projection_is_empty_for_a_grant_less_session() {
        // Fail-closed: no grants ⇒ no aggregate at all, not the pod's totals.
        let anon = split_server(None);
        assert_eq!(anon.authorized_projection().expect("projection builds").len(), 0);
    }

    #[test]
    fn stats_and_introspect_count_only_the_session_view() {
        let alice = split_server(Some("https://alice.ex/card#me"));
        let bob = split_server(Some("https://bob.ex/card#me"));

        let stats: Value =
            serde_json::from_str(&alice.tool_stats().expect("stats")).expect("stats is JSON");
        assert_eq!(stats["triples"], 2);
        assert_eq!(stats["classes"], 1);

        let ix = alice.tool_introspect(&json!({})).expect("introspect");
        assert!(ix.contains("ns#Public"), "the readable class is present: {ix}");
        assert!(!ix.contains("ns#Secret"), "the unreadable class must not appear: {ix}");
        assert!(!ix.contains("secretField"), "nor its predicate: {ix}");

        // The mirror image: bob sees his own document and none of alice's.
        let ix = bob.tool_introspect(&json!({})).expect("introspect");
        assert!(ix.contains("ns#Secret") && !ix.contains("ns#Public"), "{ix}");
    }

    #[test]
    fn shapes_reports_a_class_only_unreadable_documents_use_as_absent() {
        let alice = split_server(Some("https://alice.ex/card#me"));
        // Alice's own class resolves…
        let shape = alice
            .tool_shapes(&json!({ "class": "https://ex.dev/ns#Public" }))
            .expect("the readable class has a shape");
        assert!(shape.contains("openField"), "{shape}");
        // …and bob's does not, exactly as if the instances were absent.
        let err = alice
            .tool_shapes(&json!({ "class": "https://ex.dev/ns#Secret" }))
            .expect_err("an unreadable class is not describable");
        assert!(!err.contains("secretField"), "the error must not leak the shape: {err}");
    }

    #[test]
    fn tool_spec_consts_serialize_with_schema() {
        for spec in [
            &POD_QUERY,
            &RESOURCE_GET,
            &CONTAINER_LIST,
            &POD_INTROSPECT,
            &POD_SHAPES,
            &POD_STATS,
            &POD_INTROSPECT,
            &POD_SHAPES,
            &POD_STATS,
            &POD_UPDATE,
            &RESOURCE_PUT,
            &RESOURCE_DELETE,
            &CONTAINER_CREATE,
        ] {
            let j = spec.to_json();
            assert_eq!(j["name"].as_str(), Some(spec.name));
            assert_eq!(j["inputSchema"]["type"], "object");
        }
    }
}
