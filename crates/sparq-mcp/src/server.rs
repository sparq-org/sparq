//! The MCP server: configuration, the in-memory graph it serves, and the
//! transport-agnostic JSON-RPC dispatch core.
//!
//! [OPUS-4.8] (sq-0z43i, gh #909) [`McpServer::handle_message`] is the single
//! entry point: it takes one raw JSON-RPC message string and returns the raw
//! response string (or `None` for a notification). The stdio transport in the
//! `transport` module (feature `stdio`) is a thin loop around it; the round-trip
//! test calls it directly, so the test exercises the real dispatch path with no
//! process spawn.

use std::time::Duration;

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_introspect::Introspection;
use serde_json::{json, Value};

use crate::jsonrpc::{
    Request, Response, RpcError, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    METHOD_NOT_FOUND, RESOURCE_NOT_FOUND,
};
use crate::prompts;
use crate::resources::{self, ReadError};
use crate::tools;

/// The newest MCP protocol revision this server implements — the version offered in
/// the `initialize` result when the client proposes an unsupported (or no) revision,
/// per the MCP versioning rules. [SONNET-4.6] sq-bvnqm
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// Every published MCP protocol revision this server can speak, newest first
/// (`[0]` is [`PROTOCOL_VERSION`]). The `initialize` / `tools/list` / `tools/call`
/// request-response shapes this tools-only server implements are identical across
/// these revisions — the later additions (structured tool output, elicitation,
/// tasks, icons, resource links) are all optional capabilities a server may simply
/// not declare, and the plain `type`/`properties`/`required` tool input schemas are
/// valid under the JSON Schema 2020-12 dialect that 2025-11-25 makes the default.
///
/// 2025-03-26 is the one revision that REQUIRES receiving JSON-RPC batches (added
/// there, removed again in 2025-06-18). [OPUS-5] (gh #2497) [`McpServer::handle_message`]
/// now accepts a top-level batch array in both dispatch cores, so the revision is claimed here
/// rather than declined; before that it was deliberately absent because claiming it
/// would have overstated conformance. [SONNET-4.6] sq-bvnqm
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// MCP version negotiation over the client's `initialize` params: accept the
/// client's proposed `protocolVersion` when it is one we support, otherwise respond
/// with our latest ([`PROTOCOL_VERSION`]) — the client then decides whether to
/// continue or disconnect. A missing or non-string proposal gets the latest too.
/// [SONNET-4.6] sq-bvnqm
pub(crate) fn negotiate_protocol_version(params: &Value) -> &'static str {
    params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .and_then(|proposed| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| *supported == proposed)
        })
        .unwrap_or(PROTOCOL_VERSION)
}

/// Server configuration. The security-relevant field is [`Self::allow_update`]:
/// it is **`false` by default**, so a freshly-built server is strictly read-only
/// and never advertises or accepts the mutating `update` tool.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Whether the mutating `update` tool is exposed. **OFF by default.** When
    /// `false` the tool is neither advertised in `tools/list` nor callable via
    /// `tools/call` (a call returns a `METHOD_NOT_FOUND`-class tool error), so a
    /// default server cannot mutate the dataset at all. Turn it on only when the
    /// MCP client is trusted to issue writes — see the crate-level trust model.
    pub allow_update: bool,
    /// Wall-clock deadline applied to every tool-issued query/update, in seconds.
    /// Bounds an expensive or adversarial query so one `tools/call` cannot run the
    /// server unbounded. `None` disables the deadline (unbounded — not recommended
    /// for an agent-facing server).
    pub query_timeout_secs: Option<u64>,
    /// Upper bound on the rows of any materialised result, applied to every
    /// tool-issued query. `None` disables the row cap.
    pub max_rows: Option<usize>,
    /// The server name reported in the `initialize` handshake (`serverInfo.name`).
    pub server_name: String,
    /// [FABLE-5] (sq-lsp7k.10, feature `templates`) The named parameterized templates this
    /// server exposes through the `template_list` / `template_invoke` tools. Registered by
    /// the embedder as ALREADY-VALIDATED [`sparq_engine::templates::Template`]s (parse +
    /// declared-parameter validation happen at construction, fail-closed), so every listed
    /// template is invocable. Empty (the default) ⇒ the two tools are not advertised. An
    /// UPDATE template is advertised but only *invocable* when [`Self::allow_update`] is
    /// also `true` — the template layer never widens the read-only posture.
    #[cfg(feature = "templates")]
    pub templates: Vec<sparq_engine::templates::Template>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            // Security default: read-only.
            allow_update: false,
            // A generous-but-bounded default deadline so an agent's runaway query
            // cannot hang the server; opt out explicitly with `None`.
            query_timeout_secs: Some(30),
            max_rows: Some(1_000_000),
            server_name: "sparq-mcp".to_string(),
            // [FABLE-5] sq-lsp7k.10: no templates unless the embedder registers them.
            #[cfg(feature = "templates")]
            templates: Vec::new(),
        }
    }
}

/// An MCP server over an in-memory sparq [`Graph`].
///
/// Construct it with a loaded graph and a [`ServerConfig`], then drive it either via
/// [`McpServer::handle_message`] (embed your own transport) or, with the `stdio`
/// feature, the `serve_stdio` loop (the standard MCP stdio transport).
pub struct McpServer {
    graph: Graph,
    config: ServerConfig,
    /// [FABLE-5] (sq-lsp7k.10, feature `text`) The lazily-built BM25 literal index behind
    /// the `text_search` tool. `None` until the first search; reconciled incrementally
    /// (`O(new dictionary terms)`) before every search so it stays current across
    /// `update` / `template_invoke` mutations without a per-call rebuild.
    #[cfg(feature = "text")]
    text_index: Option<sparq_text::TextIndex>,
}

impl McpServer {
    /// Build a read-only server over `graph` with the default [`ServerConfig`]
    /// (update disabled).
    pub fn new(graph: Graph) -> Self {
        Self::with_config(graph, ServerConfig::default())
    }

    /// Build a server over `graph` with an explicit [`ServerConfig`].
    pub fn with_config(graph: Graph, config: ServerConfig) -> Self {
        McpServer {
            graph,
            config,
            #[cfg(feature = "text")]
            text_index: None,
        }
    }

    /// Whether the mutating `update` tool is exposed (mirrors
    /// [`ServerConfig::allow_update`]).
    pub fn allow_update(&self) -> bool {
        self.config.allow_update
    }

    /// The configured query budget for one tool call.
    fn budget(&self) -> QueryBudget {
        let mut b = QueryBudget::unlimited();
        if let Some(secs) = self.config.query_timeout_secs {
            b.deadline = Some(std::time::Instant::now() + Duration::from_secs(secs));
        }
        b.max_rows = self.config.max_rows;
        b
    }

    /// Read-only borrow of the served graph (for embedders / tests).
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The server's configuration (crate-internal: template-tool advertisement reads it).
    #[cfg(feature = "templates")]
    pub(crate) fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Handle one raw JSON-RPC message. Returns `Some(response_json)` for a request,
    /// `None` for a notification (no response is emitted), and a serialized error
    /// response for a malformed / failed request.
    ///
    /// The string is one complete JSON value; the transport is responsible for
    /// delimiting (the stdio transport uses one value per line). That value may be a
    /// single request object or a top-level **batch array**, in which case the reply is
    /// an array of the non-notification responses: every element is dispatched in order,
    /// notification entries are omitted, an all-notification batch gets no response at
    /// all, an empty array is answered with a single (non-array) `INVALID_REQUEST` error,
    /// and a malformed element gets its own null-id error without voiding the rest of the
    /// batch — the JSON-RPC 2.0 §6 contract the MCP 2025-03-26 revision requires.
    /// [OPUS-5] gh #2497
    pub fn handle_message(&mut self, raw: &str) -> Option<String> {
        handle_raw(raw, |req| self.dispatch(req))
    }

    /// Route a request to its method handler. Returns the JSON `result` or a JSON-RPC
    /// error object.
    fn dispatch(&mut self, req: &Request) -> Result<Value, RpcError> {
        match req.method.as_str() {
            "initialize" => Ok(self.initialize_result(&req.params)),
            // Lifecycle notification from the client; nothing to do.
            "notifications/initialized" | "initialized" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list_result()),
            "tools/call" => self.tools_call(&req.params),
            // [SONNET-4.6] sq-sjey1: the read-only resources + prompts surfaces.
            "resources/list" => Ok(self.resources_list()),
            "resources/read" => self.resources_read(&req.params),
            "prompts/list" => Ok(self.prompts_list()),
            "prompts/get" => self.prompts_get(&req.params),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {}", other),
            )),
        }
    }

    /// The `initialize` handshake result: the negotiated protocol version, server
    /// info, and the declared capabilities.
    ///
    /// [SONNET-4.6] sq-sjey1: `resources` and `prompts` join `tools`. Both sub-capability
    /// flags are declared FALSE and mean it: this server never pushes an unsolicited
    /// `listChanged` notification, and it implements no `resources/subscribe` (the pod
    /// server — feature `solid` — is the one that does, and declares `subscribe: true`
    /// itself). Declaring a flag whose machinery does not exist would be an overclaim.
    fn initialize_result(&self, params: &Value) -> Value {
        json!({
            "protocolVersion": negotiate_protocol_version(params),
            "capabilities": {
                "tools": {},
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false },
            },
            "serverInfo": {
                "name": self.config.server_name,
                "version": env!("CARGO_PKG_VERSION"),
            }
        })
    }

    /// The `tools/list` result: every advertised tool's `name`/`description`/
    /// `inputSchema`. `update` appears here only when update is enabled.
    fn tools_list_result(&self) -> Value {
        let tools: Vec<Value> = tools::advertised(self).iter().map(|t| t.to_json()).collect();
        json!({ "tools": tools })
    }

    /// `resources/list`: the dataset descriptor, the default graph, and one entry per
    /// named graph. See [`crate::resources`]. [SONNET-4.6] sq-sjey1
    fn resources_list(&self) -> Value {
        json!({ "resources": resources::descriptors(&self.graph) })
    }

    /// `resources/read`: materialise one resource in the MCP `contents` shape, under the
    /// server's [`QueryBudget`].
    ///
    /// A URI this server does not serve is MCP's `RESOURCE_NOT_FOUND`; a served resource
    /// that could not be materialised (a tripped budget) is an INTERNAL error, because
    /// reporting it as "not found" would assert something false about the dataset.
    /// [SONNET-4.6] sq-sjey1
    fn resources_read(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| {
            RpcError::new(INVALID_PARAMS, "resources/read requires a string `uri`")
        })?;
        let text = resources::read(&self.graph, uri, &self.budget()).map_err(|e| match e {
            ReadError::NotFound(m) => RpcError::new(RESOURCE_NOT_FOUND, m),
            ReadError::Failed(m) => RpcError::new(INTERNAL_ERROR, m),
        })?;
        Ok(json!({
            "contents": [ {
                "uri": uri,
                "mimeType": resources::NTRIPLES_MIME,
                "text": text,
            } ]
        }))
    }

    /// `prompts/list`: the static canned-query prompt catalog. See [`crate::prompts`].
    /// [SONNET-4.6] sq-sjey1
    fn prompts_list(&self) -> Value {
        let prompts: Vec<Value> = prompts::PROMPTS.iter().map(|p| p.to_json()).collect();
        json!({ "prompts": prompts })
    }

    /// `prompts/get`: render one prompt into the MCP `messages` shape. An unknown prompt
    /// name and an unusable argument are both `INVALID_PARAMS`, as the MCP prompts spec
    /// prescribes — a prompt is never rendered around an argument that failed validation.
    /// [SONNET-4.6] sq-sjey1
    fn prompts_get(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "prompts/get requires a string `name`"))?;
        let spec = prompts::find(name)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("unknown prompt: {}", name)))?;
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let text = (spec.render)(&args).map_err(|m| RpcError::new(INVALID_PARAMS, m))?;
        Ok(json!({
            "description": spec.description,
            "messages": [ {
                "role": "user",
                "content": { "type": "text", "text": text },
            } ]
        }))
    }

    /// Handle a `tools/call`: validate the requested tool name + arguments, run it,
    /// and wrap the output in the MCP `CallToolResult` shape (a `content` array with a
    /// single text item, plus `isError` on a tool-level failure).
    fn tools_call(&mut self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call requires a string `name`"))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        // Reject the mutating tool up front when it is not enabled, *before* parsing
        // args — so a disabled `update` is never even attempted.
        if name == tools::UPDATE.name && !self.allow_update() {
            return Err(RpcError::new(
                METHOD_NOT_FOUND,
                "tool `update` is not available: this server is read-only \
                 (start it with update enabled to allow writes)",
            ));
        }

        let result = match name {
            "query" => self.tool_query(&args),
            "construct" => self.tool_construct(&args),
            "introspect" => self.tool_introspect(&args),
            "shapes" => self.tool_shapes(&args),
            "stats" => self.tool_stats(),
            "classes" => self.tool_classes(),
            "prefixes" => self.tool_prefixes(),
            "void" => self.tool_void(&args),
            #[cfg(feature = "nlq")]
            "ask" => self.tool_ask(&args),
            #[cfg(feature = "nlq")]
            "nl_query" => self.tool_nl_query(&args),
            #[cfg(feature = "templates")]
            "template_list" => self.tool_template_list(),
            #[cfg(feature = "templates")]
            "template_invoke" => self.tool_template_invoke(&args),
            #[cfg(feature = "text")]
            "text_search" => self.tool_text_search(&args),
            #[cfg(feature = "shacl")]
            "validate" => self.tool_validate(&args),
            #[cfg(feature = "shacl")]
            "describe_form" => self.tool_describe_form(&args),
            "update" => self.tool_update(&args),
            other => {
                return Err(RpcError::new(
                    METHOD_NOT_FOUND,
                    format!("unknown tool: {}", other),
                ))
            }
        };

        // A tool-execution failure (a bad SPARQL string, a budget trip) is reported
        // as an MCP tool error (`isError: true` + a text message), NOT a JSON-RPC
        // protocol error — the client/agent reads it and can retry. A protocol error
        // (bad method, bad params shape) is the JSON-RPC `Err` path above.
        match result {
            Ok(text) => Ok(tool_text_result(text, false)),
            Err(msg) => Ok(tool_text_result(msg, true)),
        }
    }

    /// `query`: SELECT/ASK → SPARQL 1.1 JSON results.
    fn tool_query(&self, args: &Value) -> Result<String, String> {
        let sparql = arg_str(args, "sparql")?;
        sparq_engine::query_json_with_budget(&self.graph, sparql, &self.budget())
    }

    /// `construct`: CONSTRUCT/DESCRIBE → N-Triples text.
    fn tool_construct(&self, args: &Value) -> Result<String, String> {
        let sparql = arg_str(args, "sparql")?;
        sparq_engine::construct_ntriples_with_budget(&self.graph, sparql, &self.budget())
    }

    /// `introspect`: the effective schema as JSON (default) or a text summary.
    fn tool_introspect(&self, args: &Value) -> Result<String, String> {
        let format = args.get("format").and_then(Value::as_str).unwrap_or("json");
        let ix = Introspection::build(&self.graph);
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

    /// `shapes`: the data-grounded shape (valid predicates + datatypes + cardinalities)
    /// of one class IRI, as structured JSON for a client LLM to ground NL→SPARQL on. No
    /// server-side model — reuses the introspection schema miner. [OPUS-4.8] sq-zak4f
    fn tool_shapes(&self, args: &Value) -> Result<String, String> {
        let class_iri = arg_str(args, "class")?;
        let ix = Introspection::build(&self.graph);
        let shape = crate::shapes::class_shape(&ix, class_iri)?;
        Ok(serde_json::to_string_pretty(&shape).unwrap_or_else(|_| shape.to_string()))
    }

    /// `ask` (feature `nlq`): server-side NL→SPARQL→execute via sparq-nlq. Degrades
    /// cleanly — returns a clear "not configured" tool error when no LLM backend is set,
    /// never a fabricated answer or a panic. [OPUS-4.8] sq-jxjgr
    #[cfg(feature = "nlq")]
    fn tool_ask(&self, args: &Value) -> Result<String, String> {
        let question = arg_str(args, "question")?;
        crate::nlq::ask(&self.graph, question, &self.budget())
    }

    /// `nl_query` (feature `nlq`): server-side NL→SPARQL **translation** via sparq-nlq —
    /// the query is validated but never executed, so it takes no [`QueryBudget`]. Degrades
    /// cleanly on the same "not configured" path as `ask`. [SONNET-4.6] sq-sj1f9
    #[cfg(feature = "nlq")]
    fn tool_nl_query(&self, args: &Value) -> Result<String, String> {
        let question = arg_str(args, "question")?;
        crate::nlq::nl_query(&self.graph, question)
    }

    /// `stats`: dataset totals as a small JSON object.
    fn tool_stats(&self) -> Result<String, String> {
        let ix = Introspection::build(&self.graph);
        let stats = json!({
            "triples": self.graph.len(),
            "distinct_subjects": ix.subjects,
            "typed_entities": ix.entities,
            "classes": ix.classes.len(),
            "predicates": ix.predicates.len(),
            "namespaces": ix.vocabularies.distinct,
        });
        Ok(serde_json::to_string_pretty(&stats).unwrap_or_else(|_| stats.to_string()))
    }

    /// [GPT-5.6] sq-cekgj: `classes`, class profiles ranked by instance count.
    fn tool_classes(&self) -> Result<String, String> {
        let ix = Introspection::build(&self.graph);
        let mut profiles = ix.classes;
        let distinct_classes = profiles.len();
        profiles.sort_by(|a, b| b.instances.cmp(&a.instances).then(a.class.cmp(&b.class)));
        let classes: Vec<Value> = profiles
            .into_iter()
            .map(|profile| {
                json!({
                    "class": profile.class,
                    "instances": profile.instances,
                    "predicate_count": profile.predicates.len(),
                })
            })
            .collect();
        let result = json!({
            "distinct_classes": distinct_classes,
            "classes": classes,
        });
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
    }

    /// [GPT-5.6] sq-kx5b0: `prefixes`, namespace declarations ranked by term count.
    fn tool_prefixes(&self) -> Result<String, String> {
        let ix = Introspection::build(&self.graph);
        let distinct_prefixes = ix.vocabularies.distinct;
        let mut namespaces = ix.vocabularies.namespaces;
        namespaces.sort_by(|a, b| b.terms.cmp(&a.terms).then(a.namespace.cmp(&b.namespace)));
        let prefixes: Vec<Value> = namespaces
            .into_iter()
            .map(|vocabulary| {
                json!({
                    "prefix": vocabulary.prefix,
                    "namespace": vocabulary.namespace,
                    "term_count": vocabulary.terms,
                })
            })
            .collect();
        let result = json!({
            "distinct_prefixes": distinct_prefixes,
            "prefixes": prefixes,
        });
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
    }

    /// `void`: W3C VoID dataset descriptor as deterministic N-Triples.
    fn tool_void(&self, args: &Value) -> Result<String, String> {
        let dataset_iri = match args.get("dataset") {
            None => "urn:sparq:dataset",
            Some(value) => value
                .as_str()
                .ok_or_else(|| "argument `dataset` must be a string".to_string())?,
        };
        let characteristic_sets = match args.get("characteristic_sets") {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| "argument `characteristic_sets` must be a boolean".to_string())?,
        };
        let ix = Introspection::build(&self.graph);
        Ok(if characteristic_sets {
            ix.to_void_with_cs(dataset_iri)
        } else {
            ix.to_void(dataset_iri)
        })
    }

    /// `template_list` (feature `templates`): the registered named-template definitions
    /// as JSON — name / kind / text / declared typed parameters / description — so an
    /// agent can ground a `template_invoke` call. [FABLE-5] sq-lsp7k.10
    #[cfg(feature = "templates")]
    fn tool_template_list(&self) -> Result<String, String> {
        let defs: Vec<Value> = self.config.templates.iter().map(|t| t.to_json()).collect();
        let out = Value::Array(defs);
        Ok(serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string()))
    }

    /// `template_invoke` (feature `templates`): bind typed JSON arguments into a
    /// registered template through the injection-safe #901 algebra rewrite and execute
    /// it. FAIL-CLOSED: an unknown template name, an unknown/missing parameter, or a
    /// JSON shape that does not match the declared type is a tool error, never a guess.
    /// An UPDATE template additionally requires `allow_update` — exactly the raw
    /// `update` tool's gate, so the template layer never widens write access.
    /// [FABLE-5] sq-lsp7k.10
    #[cfg(feature = "templates")]
    fn tool_template_invoke(&mut self, args: &Value) -> Result<String, String> {
        let name = arg_str(args, "name")?;
        let template = self
            .config
            .templates
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| format!("no such template `{}` (see template_list)", name))?
            .clone();
        if template.is_update() && !self.allow_update() {
            return Err(format!(
                "template `{}` is a SPARQL UPDATE and this server is read-only                  (start it with update enabled to allow template writes)",
                name
            ));
        }
        let params = args.get("parameters").cloned().unwrap_or_else(|| json!({}));
        let bound = template.bind_json(&params)?;
        // The bound algebra renders to canonical SPARQL (values escaped as data) and runs
        // through the SAME budgeted engine entry points as the raw query/update tools.
        let rendered = bound.render();
        if bound.is_update() {
            let budget = self.budget();
            sparq_engine::update_in_place_atomic_with_budget(&mut self.graph, &rendered, &budget)?;
            Ok(format!("ok; graph now has {} triples", self.graph.len()))
        } else if bound.is_graph_form() {
            sparq_engine::construct_ntriples_with_budget(&self.graph, &rendered, &self.budget())
        } else {
            sparq_engine::query_json_with_budget(&self.graph, &rendered, &self.budget())
        }
    }

    /// `text_search` (feature `text`): BM25 full-text search over the graph's string
    /// literals via `sparq-text`. The index is built lazily on first use and reconciled
    /// incrementally (`O(new dictionary terms)`) before every search, so it stays current
    /// after `update` / `template_invoke` mutations. Returns ranked hits as JSON —
    /// each the matching literal (N-Triples form) plus its BM25 score. [FABLE-5]
    /// sq-lsp7k.10
    #[cfg(feature = "text")]
    fn tool_text_search(&mut self, args: &Value) -> Result<String, String> {
        let query = arg_str(args, "query")?;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("and");
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 1000) as usize;
        // Lazy build / incremental reconcile (a shrunken dictionary — impossible within
        // one live Graph, but guarded anyway — forces a rebuild).
        let rebuild = match &self.text_index {
            None => true,
            Some(ix) => ix.needs_rebuild(&self.graph),
        };
        if rebuild {
            self.text_index = Some(sparq_text::TextIndex::build(&self.graph));
        }
        let index = self.text_index.as_mut().expect("just ensured");
        index.reconcile(&self.graph);
        let hits = match mode {
            "and" => index.search(query),
            "any" => index.search_any(query),
            other => {
                return Err(format!(
                    "unknown mode `{}` (expected \"and\" or \"any\")",
                    other
                ))
            }
        };
        let total = hits.len();
        let rows: Vec<Value> = hits
            .iter()
            .take(limit)
            .map(|h| {
                json!({
                    "literal": self.graph.dict.term(h.id).to_string(),
                    "score": h.score,
                })
            })
            .collect();
        let out = json!({ "total_matches": total, "returned": rows.len(), "hits": rows });
        Ok(serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string()))
    }

    /// `validate` (feature `shacl`): parse a caller-owned shapes graph and run the
    /// existing SHACL validator over a shared borrow of the served data. [GPT-5.6]
    #[cfg(feature = "shacl")]
    fn tool_validate(&self, args: &Value) -> Result<String, String> {
        let source = arg_str(args, "shapes")?;
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("turtle");
        let shapes = Graph::load_str(source, format)
            .map_err(|error| format!("invalid SHACL shapes graph: {error}"))?;

        let report = sparq_shacl::validate(self.graph(), &shapes);
        // ValidationReport retains below-threshold results for diagnostics. The MCP
        // contract returns only results at severities that participate in this shapes
        // graph's conformance decision.
        let model = sparq_shacl::ShapesModel::parse(&shapes);
        let disallowed: Vec<&str> = model
            .conformance_disallows()
            .map(|set| set.iter().map(String::as_str).collect())
            .unwrap_or_else(|| sparq_shacl::DEFAULT_CONFORMANCE_DISALLOWS.to_vec());
        let results: Vec<Value> = report
            .results
            .iter()
            .filter(|result| disallowed.contains(&result.severity.as_str()))
            .map(|result| {
                let message = result
                    .effective_messages()
                    .into_iter()
                    .next()
                    .map(|term| match term {
                        oxrdf::Term::Literal(literal) => literal.value().to_owned(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                json!({
                    "focusNode": result.focus_node.to_string(),
                    "path": result.path.as_ref().map(|path| path.to_turtle()),
                    "severity": result.severity,
                    "message": message,
                })
            })
            .collect();
        let out = json!({ "conforms": report.conforms, "results": results });
        Ok(serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string()))
    }

    /// `describe_form` (feature `shacl`): parse the focus node + a caller-owned
    /// shapes graph (the same parsing path as `validate`), derive the form with
    /// `sparq_forms::derive_form` over a shared borrow of the served data, and
    /// return its `FormDescription` JSON VERBATIM (no key reshaping — agents and
    /// renderers consume the one canonical contract). [FABLE-5] sq-lsp7k.1.6
    #[cfg(feature = "shacl")]
    fn tool_describe_form(&self, args: &Value) -> Result<String, String> {
        let focus = parse_focus_term(arg_str(args, "focus")?)?;
        let source = arg_str(args, "shapes")?;
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("turtle");
        let shapes = Graph::load_str(source, format)
            .map_err(|error| format!("invalid SHACL shapes graph: {}", error))?;

        let mut opts = sparq_forms::FormOptions::default();
        if let Some(mode) = args.get("mode").and_then(Value::as_str) {
            opts.mode = match mode {
                "edit" => sparq_forms::Mode::Edit,
                "view" => sparq_forms::Mode::View,
                other => {
                    // Fail closed on an unknown mode rather than silently editing.
                    return Err(format!(
                        "unknown mode `{}` (expected \"edit\" or \"view\")",
                        other
                    ));
                }
            };
        }
        if let Some(shape) = args.get("shape").and_then(Value::as_str) {
            let iri = oxrdf::NamedNode::new(shape)
                .map_err(|error| format!("invalid shape IRI: {}", error))?;
            opts.shape = Some(oxrdf::Term::NamedNode(iri));
        }

        let form = sparq_forms::derive_form(self.graph(), &shapes, &focus, &opts);
        serde_json::to_string_pretty(&form)
            .map_err(|error| format!("form serialization failed: {}", error))
    }

    /// `update`: apply a SPARQL 1.1 Update atomically (only reachable when enabled).
    fn tool_update(&mut self, args: &Value) -> Result<String, String> {
        let sparql = arg_str(args, "sparql")?;
        let budget = self.budget();
        sparq_engine::update_in_place_atomic_with_budget(&mut self.graph, sparql, &budget)?;
        Ok(format!("ok; graph now has {} triples", self.graph.len()))
    }
}

/// Parse a `describe_form` focus argument: `_:label` denotes a blank node, anything
/// else must be a valid IRI (literals cannot be SHACL focus nodes for a form).
/// [FABLE-5] sq-lsp7k.1.6
#[cfg(feature = "shacl")]
fn parse_focus_term(focus: &str) -> Result<oxrdf::Term, String> {
    if let Some(label) = focus.strip_prefix("_:") {
        oxrdf::BlankNode::new(label)
            .map(oxrdf::Term::BlankNode)
            .map_err(|error| format!("invalid focus blank-node label: {}", error))
    } else {
        oxrdf::NamedNode::new(focus)
            .map(oxrdf::Term::NamedNode)
            .map_err(|error| format!("invalid focus IRI: {}", error))
    }
}

/// Extract a required string argument or produce a tool-level error message.
pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument `{}`", key))
}

/// Wrap a tool's textual output in the MCP `CallToolResult` shape.
pub(crate) fn tool_text_result(text: String, is_error: bool) -> Value {
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": is_error,
    })
}

/// Serialize a response, falling back to a hand-built internal-error object if
/// serialization itself somehow fails (it cannot for our concrete types, but we never
/// panic on the I/O path).
pub(crate) fn serialize(resp: &Response) -> String {
    serde_json::to_string(resp).unwrap_or_else(|_| {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":{},\"message\":\"response serialization failed\"}}}}",
            INTERNAL_ERROR
        )
    })
}

/// Serialize a batch of responses as a JSON array, with the same never-panics
/// fallback as [`serialize`].
fn serialize_batch(responses: &[Response]) -> String {
    serde_json::to_string(responses).unwrap_or_else(|_| {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":{},\"message\":\"response serialization failed\"}}}}",
            INTERNAL_ERROR
        )
    })
}

/// The response to an uncorrelatable parse failure: JSON-RPC gives us no id to echo
/// back, so the id is JSON null.
fn parse_error(e: serde_json::Error) -> Response {
    Response::err(
        Value::Null,
        RpcError::new(INVALID_REQUEST, format!("invalid JSON-RPC request: {}", e)),
    )
}

/// Dispatch one already-parsed request, returning its response — or `None` when it is
/// a notification, which never gets a response, even on error.
fn dispatch_request<F>(req: &Request, dispatch: &mut F) -> Option<Response>
where
    F: FnMut(&Request) -> Result<Value, RpcError>,
{
    let is_notification = req.is_notification();
    let id = req.id.clone().unwrap_or(Value::Null);
    let outcome = dispatch(req);
    if is_notification {
        return None;
    }
    Some(match outcome {
        Ok(result) => Response::ok(id, result),
        Err(e) => Response::err(id, e),
    })
}

/// Dispatch one *batch element*. An element that is not a well-formed request object
/// gets its own `INVALID_REQUEST` error rather than voiding the whole batch, which is
/// what JSON-RPC 2.0 §6 requires.
fn dispatch_element<F>(elem: Value, dispatch: &mut F) -> Option<Response>
where
    F: FnMut(&Request) -> Result<Value, RpcError>,
{
    match serde_json::from_value::<Request>(elem) {
        Ok(req) => dispatch_request(&req, dispatch),
        Err(e) => Some(parse_error(e)),
    }
}

/// The shared framing core behind [`McpServer::handle_message`] and its pod twin
/// `SolidMcpServer::handle_message`: take one raw JSON-RPC message — a single request
/// object **or a top-level batch array** — and route each request through `dispatch`.
///
/// [OPUS-5] (gh #2497) Batch *receipt* is the one thing the MCP **2025-03-26** revision
/// requires that the other revisions do not (batching was added there and removed again
/// in 2025-06-18), so it is what lets `2025-03-26` sit in
/// [`SUPPORTED_PROTOCOL_VERSIONS`]. Per JSON-RPC 2.0 §6:
///
/// - every element is dispatched in order and the responses come back as a JSON array;
/// - notification entries are omitted from that array;
/// - a batch of nothing but notifications gets no response at all (`None`), exactly
///   like a single notification;
/// - an EMPTY array is itself an invalid request, answered with a single *non-array*
///   error object;
/// - a malformed element is answered with its own null-id error and does not void the
///   rest of the batch.
///
/// A batch is accepted whatever revision was negotiated: receiving one is a strict
/// superset of the revisions that dropped batching, the framing is unambiguous, and
/// these servers keep no negotiated-version state to reject it against. Note the
/// direction — an array is only ever emitted in reply to an array, never unsolicited,
/// so a client speaking a batch-free revision is never handed one it did not ask for.
pub(crate) fn handle_raw<F>(raw: &str, mut dispatch: F) -> Option<String>
where
    F: FnMut(&Request) -> Result<Value, RpcError>,
{
    // A top-level JSON array can only begin with `[` once insignificant leading
    // whitespace is skipped, so this one-byte probe keeps the single-object path (the
    // overwhelmingly common one) on exactly the parse it always had: straight to
    // `Request`, with no intermediate `Value`.
    if raw.trim_start().starts_with('[') {
        let elements: Vec<Value> = match serde_json::from_str(raw) {
            Ok(elements) => elements,
            Err(e) => return Some(serialize(&parse_error(e))),
        };
        if elements.is_empty() {
            return Some(serialize(&Response::err(
                Value::Null,
                RpcError::new(INVALID_REQUEST, "invalid JSON-RPC request: empty batch"),
            )));
        }
        let responses: Vec<Response> = elements
            .into_iter()
            .filter_map(|elem| dispatch_element(elem, &mut dispatch))
            .collect();
        if responses.is_empty() {
            return None;
        }
        return Some(serialize_batch(&responses));
    }

    let req: Request = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => return Some(serialize(&parse_error(e))),
    };
    dispatch_request(&req, &mut dispatch).map(|resp| serialize(&resp))
}
