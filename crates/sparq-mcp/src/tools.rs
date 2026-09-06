//! MCP tool definitions: the `name` / `description` / `inputSchema` triples an MCP
//! client receives from `tools/list`, plus the dispatch from a `tools/call` to the
//! sparq engine.
//!
//! [OPUS-4.8] (sq-0z43i, gh #909) Every tool is a thin wrapper over an existing
//! sparq read API — there is no new query/engine logic here:
//! - `query` → `sparq_engine::query_json` (SELECT/ASK; SPARQL 1.1 JSON results).
//! - `construct` → `sparq_engine::construct_ntriples` (CONSTRUCT/DESCRIBE; N-Triples).
//! - `introspect` → `sparq_introspect::Introspection::build(...).to_json()` /
//!   `to_text_summary(...)` (classes, predicates, prefixes, characteristic sets).
//! - `shapes` → `crate::shapes::class_shape(...)` (the data-grounded predicate/datatype/
//!   cardinality constraints for one class IRI; structured grounding for a client LLM —
//!   no server-side model). [OPUS-4.8] sq-zak4f
//! - `stats` → graph triple count + introspection totals.
//! - `classes` → per-class instance and predicate counts.
//! - `prefixes` → namespace declarations + per-namespace distinct-IRI term counts.
//! - `void` → `sparq_introspect::Introspection::to_void` (W3C VoID N-Triples).
//! - `validate` (feature `shacl`, OFF by default) → `sparq_shacl::validate` over
//!   caller-supplied shapes; read-only and absent from the default tool surface.
//! - `describe_form` (feature `shacl`, OFF by default) → `sparq_forms::derive_form`
//!   over caller-supplied shapes for one focus node; returns the `FormDescription`
//!   JSON verbatim. Read-only. [FABLE-5] sq-lsp7k.1.6
//! - `ask` (feature `nlq`, OFF by default) → `crate::nlq` (server-side NL→SPARQL→execute
//!   via `sparq-nlq`; embeds a configurable LLM call, degrades cleanly when none is
//!   configured). [OPUS-4.8] sq-jxjgr
//! - `update` (gated, OFF by default) → `sparq_engine::update_in_place_atomic`.

use serde_json::{json, Value};

use crate::server::McpServer;

/// A single MCP tool advertised in `tools/list`.
///
/// The `input_schema` is a JSON-Schema object describing the tool's arguments, exactly
/// as the MCP `Tool.inputSchema` field requires; clients use it to validate / form the
/// `arguments` they pass to `tools/call`.
pub struct ToolSpec {
    /// The tool name (the `name` a `tools/call` references).
    pub name: &'static str,
    /// One-line human/agent-facing description of what the tool does.
    pub description: &'static str,
    /// JSON-Schema (`type: object`) for the tool's `arguments`.
    pub input_schema: fn() -> Value,
}

impl ToolSpec {
    /// Render this spec as the JSON object MCP's `tools/list` returns per tool.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": (self.input_schema)(),
        })
    }
}

/// The `query` tool: run a read-only SELECT or ASK and return SPARQL 1.1 JSON results.
pub const QUERY: ToolSpec = ToolSpec {
    name: "query",
    description: "Run a read-only SPARQL 1.1 SELECT or ASK query against the loaded RDF \
                  graph and return SPARQL 1.1 Query Results JSON \
                  (head.vars + results.bindings, or a boolean for ASK). Use `construct` \
                  for CONSTRUCT/DESCRIBE. This tool never mutates the graph.",
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

/// The `construct` tool: run a CONSTRUCT/DESCRIBE and return N-Triples text.
pub const CONSTRUCT: ToolSpec = ToolSpec {
    name: "construct",
    description: "Run a read-only SPARQL CONSTRUCT or DESCRIBE query against the loaded \
                  RDF graph and return the resulting triples as N-Triples text. This \
                  tool never mutates the graph.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "sparql": {
                    "type": "string",
                    "description": "A SPARQL 1.1 CONSTRUCT or DESCRIBE query string."
                }
            },
            "required": ["sparql"],
            "additionalProperties": false
        })
    },
};

/// The `introspect` tool: mine and return the dataset's effective schema.
pub const INTROSPECT: ToolSpec = ToolSpec {
    name: "introspect",
    description: "Return the effective schema the loaded graph actually uses — classes \
                  (by instance count), predicates (by usage), namespace prefixes, and \
                  characteristic sets — mined directly from the store indexes (exact \
                  counts, no sampling). Set `format` to \"text\" for a compact \
                  token-budgeted summary suitable for LLM grounding, or \"json\" (the \
                  default) for the full machine-readable structure.",
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

/// The `shapes` tool: the data-grounded predicate/datatype/cardinality constraints for
/// one class IRI, as structured JSON for a CLIENT LLM to ground NL→SPARQL on. No
/// server-side model — this is the lean, structured grounding aid (`sq-zak4f`).
/// [OPUS-4.8]
pub const SHAPES: ToolSpec = ToolSpec {
    name: "shapes",
    description: "Given a class/type IRI, return the data-grounded SHACL-style shape of \
                  that class: which predicates instances actually use, each with its \
                  coverage, observed datatypes, object-kind split (IRI vs literal), \
                  observed range, and the cardinalities the data supports (min_count=1 \
                  for a predicate present on every instance; max_count=1 for a \
                  single-valued one — emitted only when the data proves the bound, never \
                  fabricated). Use this to ground a SPARQL query for instances of the \
                  class: it tells you the valid predicates and whether to bind a literal \
                  or an IRI. Constraints describe the EFFECTIVE schema (what the graph \
                  asserts), not an aspirational contract. No LLM is involved.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "class": {
                    "type": "string",
                    "description": "The full class IRI (e.g. \
                                    \"http://xmlns.com/foaf/0.1/Person\"). Call \
                                    `introspect` first to discover the classes the \
                                    dataset uses."
                }
            },
            "required": ["class"],
            "additionalProperties": false
        })
    },
};

/// The `stats` tool: dataset totals (triples, distinct subjects, typed entities, …).
pub const STATS: ToolSpec = ToolSpec {
    name: "stats",
    description: "Return summary statistics for the loaded graph: total triples, \
                  distinct subjects, typed entities, and the number of distinct \
                  classes / predicates / namespaces. Cheap, structured counts for \
                  deciding how to query the dataset.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    },
};

/// [GPT-5.6] sq-cekgj: class profiles ranked by instance count.
pub const CLASSES: ToolSpec = ToolSpec {
    name: "classes",
    description: "Return the classes detected in the loaded graph: each class IRI, its \
                  instance count, and the number of predicates used by its instances, \
                  sorted by instance count descending then class IRI ascending. Also \
                  returns the total number of classes as distinct_classes. No LLM is \
                  involved.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    },
};

/// [GPT-5.6] sq-kx5b0: namespace declarations ranked by distinct-IRI term count.
pub const PREFIXES: ToolSpec = ToolSpec {
    name: "prefixes",
    description: "Return namespace declarations detected in the loaded graph: each known \
                  prefix, its namespace IRI, and its distinct IRI term count, sorted by \
                  term count descending. Also returns the total number of distinct \
                  namespaces as distinct_prefixes. No LLM is involved.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    },
};

/// [GPT-5.6] sq-2kkym: the `void` tool, a W3C VoID dataset descriptor in N-Triples.
pub const VOID: ToolSpec = ToolSpec {
    name: "void",
    description: "Return a W3C VoID dataset descriptor as N-Triples: exact dataset \
                  totals plus class and property partitions. The optional dataset IRI \
                  defaults to urn:sparq:dataset; characteristic_sets=true appends \
                  sparq's characteristic-set statistics for federated planning.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "dataset": {
                    "type": "string",
                    "description": "IRI naming the described dataset (default \
                                    \"urn:sparq:dataset\")."
                },
                "characteristic_sets": {
                    "type": "boolean",
                    "description": "Append characteristic-set statistics when true \
                                    (default false)."
                }
            },
            "additionalProperties": false
        })
    },
};

/// The `update` tool: apply a SPARQL 1.1 Update. ONLY advertised / callable when the
/// server is started with update enabled (see [`crate::ServerConfig::allow_update`]).
pub const UPDATE: ToolSpec = ToolSpec {
    name: "update",
    description: "Apply a SPARQL 1.1 Update (INSERT/DELETE/LOAD/CLEAR/…) to the loaded \
                  graph, atomically (all-or-nothing per request). This MUTATES the \
                  dataset and is only available when the server was started with \
                  update explicitly enabled. Returns the new triple count on success.",
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

/// The `ask` tool: server-side natural-language → SPARQL → execute, behind the opt-in
/// `nlq` feature (`sq-jxjgr`). HONEST framing in the description: it embeds a configurable
/// LLM call, so cost/quality depend on the user's model, and it returns the executed
/// SPARQL + an honest rendering of the result rows (and, with `nlq/citations`, in-graph
/// citations) — NOT a free-form prose paragraph the model could fabricate. The structured
/// `shapes` / `introspect` tools are the no-LLM default; this trades a model call for the
/// convenience of not writing SPARQL yourself. [OPUS-4.8]
#[cfg(feature = "nlq")]
pub const ASK: ToolSpec = ToolSpec {
    name: "ask",
    description: "Answer a natural-language question by generating, validating, and \
                  running a SPARQL query SERVER-SIDE, then returning the executed SPARQL \
                  plus the result rows it produced (and in-graph citations when available). \
                  HONEST FRAMING: this embeds a configurable LLM call — cost, latency, and \
                  answer quality depend entirely on the model/endpoint YOU configure (via \
                  ANTHROPIC_API_KEY, or an OpenAI-compatible SPARQ_NLQ_ENDPOINT_URL + \
                  _MODEL); the server ships no default model and never phones home. If no \
                  model is configured the tool returns a clear 'not configured' error — \
                  it NEVER fabricates an answer. The returned answer is grounded in the \
                  executed query's real result rows, not a free-form paragraph. For a \
                  no-LLM path, use `shapes` + `introspect` and write the query yourself.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "A natural-language question about the loaded dataset."
                }
            },
            "required": ["question"],
            "additionalProperties": false
        })
    },
};

/// The `nl_query` tool: server-side natural-language → SPARQL **translation**, behind the
/// same opt-in `nlq` feature (`sq-sj1f9`). The description's load-bearing job is to stop an
/// agent reading the output as an ANSWER: nothing was executed, so no rows are implied.
/// [SONNET-4.6]
#[cfg(feature = "nlq")]
pub const NL_QUERY: ToolSpec = ToolSpec {
    name: "nl_query",
    description: "Translate a natural-language question into a SPARQL query and return the \
                  query WITHOUT running it. The query is validated (it parses, and it uses \
                  no construct the server refuses, such as SERVICE federation) but NOT \
                  executed: the response carries `executed: false` and NO result rows, so \
                  do not read it as an answer. Review the query, then run it with `query` \
                  (or use `ask` to translate and execute in one step). HONEST FRAMING: like \
                  `ask` this embeds a configurable LLM call — cost, latency, and query \
                  quality depend entirely on the model/endpoint YOU configure (via \
                  ANTHROPIC_API_KEY, or an OpenAI-compatible SPARQ_NLQ_ENDPOINT_URL + \
                  _MODEL); the server ships no default model. If no model is configured \
                  the tool returns a clear 'not configured' error — it NEVER fabricates a \
                  query. Because nothing was executed, a returned query may still fail at \
                  runtime or match nothing.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "A natural-language question about the loaded dataset."
                }
            },
            "required": ["question"],
            "additionalProperties": false
        })
    },
};

/// The `template_list` tool (feature `templates`): the registered named-template
/// definitions, so an agent can ground a `template_invoke` call. [FABLE-5] sq-lsp7k.10
#[cfg(feature = "templates")]
pub const TEMPLATE_LIST: ToolSpec = ToolSpec {
    name: "template_list",
    description: "List the named parameterized SPARQL templates this server exposes: for \
                  each, its name, kind (query or update), SPARQL text, declared typed \
                  parameters (name -> type: auto|iri|string|boolean|integer|decimal|double \
                  or a datatype IRI) and description. Call this before template_invoke to \
                  learn the exact parameter names and types. Read-only.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    },
};

/// The `template_invoke` tool (feature `templates`): typed, fail-closed invocation of a
/// registered template — the safe alternative to free-form UPDATE for agents. [FABLE-5]
/// sq-lsp7k.10
#[cfg(feature = "templates")]
pub const TEMPLATE_INVOKE: ToolSpec = ToolSpec {
    name: "template_invoke",
    description: "Invoke a named parameterized SPARQL template with typed arguments. The \
                  arguments are bound as typed RDF terms into the template's parsed algebra \
                  (never string concatenation), so values cannot inject SPARQL syntax. \
                  FAIL-CLOSED: an unknown template name, an unknown or missing parameter, \
                  or an argument whose JSON shape does not match the declared type is an \
                  error, never a silent no-op. A SELECT/ASK template returns SPARQL 1.1 \
                  JSON results; CONSTRUCT/DESCRIBE returns N-Triples; an UPDATE template \
                  mutates the graph and is only invocable when the server was started with \
                  update enabled (the same gate as the raw update tool).",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The template name, exactly as listed by template_list."
                },
                "parameters": {
                    "type": "object",
                    "description": "The template's declared parameters. Every declared \
                                    parameter is required; JSON shapes must match the \
                                    declared types (string for iri/string/datatype-IRI \
                                    lexical forms, number for integer/decimal/double, \
                                    boolean for boolean; an `auto` parameter also accepts \
                                    {\"iri\": ...}, {\"value\": ..., \"datatype\": ...} or \
                                    {\"value\": ..., \"lang\": ...})."
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    },
};

/// The `text_search` tool (feature `text`): BM25 full-text search over the graph's
/// string literals via the sparq-text index. [FABLE-5] sq-lsp7k.10
#[cfg(feature = "text")]
pub const TEXT_SEARCH: ToolSpec = ToolSpec {
    name: "text_search",
    description: "Full-text search over the RDF graph's string literals (BM25-ranked, \
                  best first). Returns the matching literals (N-Triples form) with their \
                  relevance scores; to reach the subjects/predicates carrying a matched \
                  literal, follow up with a query tool call that filters on the literal \
                  value. mode \"and\" (default) requires every token; \"any\" ranks the \
                  union. A trailing * on a token is a prefix match (autocomplete-style \
                  IRI/label discovery). Read-only; the index is maintained server-side \
                  and stays current across updates.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search tokens, e.g. \"quick fox\" or \"auto*\"."
                },
                "mode": {
                    "type": "string",
                    "enum": ["and", "any"],
                    "description": "Token combination: \"and\" (all tokens, default) or \
                                    \"any\" (union)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Maximum hits returned (default 20)."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    },
};

/// The `validate` tool (feature `shacl`): validate the served graph against a
/// caller-supplied SHACL shapes graph without mutating either graph. [GPT-5.6]
#[cfg(feature = "shacl")]
pub const VALIDATE: ToolSpec = ToolSpec {
    name: "validate",
    description: "Validate the loaded RDF graph against a caller-supplied SHACL shapes \
                  graph. Returns JSON containing conforms and the disallowed-severity \
                  validation results (focusNode, path, severity, message). The shapes \
                  string is parsed independently and this tool never mutates the loaded \
                  graph.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "shapes": {
                    "type": "string",
                    "description": "The SHACL shapes graph, as Turtle by default."
                },
                "format": {
                    "type": "string",
                    "description": "RDF syntax for shapes (default \"turtle\"; for \
                                    example \"ntriples\")."
                }
            },
            "required": ["shapes"],
            "additionalProperties": false
        })
    },
};

/// The `describe_form` tool (feature `shacl`): derive a shape-aware form description
/// for one focus node of the served graph against a caller-supplied SHACL shapes
/// graph, returning `sparq_forms::derive_form`'s `FormDescription` JSON verbatim.
/// [FABLE-5] sq-lsp7k.1.6
#[cfg(feature = "shacl")]
pub const DESCRIBE_FORM: ToolSpec = ToolSpec {
    name: "describe_form",
    description: "Derive a shape-aware form for one focus node of the loaded RDF graph \
                  against a caller-supplied SHACL shapes graph, returning the \
                  sparq-forms FormDescription JSON verbatim (applicable shapes, \
                  property groups, fields with widget choices, constraints, and \
                  current values). Read-only: neither graph is mutated.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "focus": {
                    "type": "string",
                    "description": "The focus node: an IRI, or a blank-node label \
                                    written as \"_:label\"."
                },
                "shapes": {
                    "type": "string",
                    "description": "The SHACL shapes graph, as Turtle by default."
                },
                "format": {
                    "type": "string",
                    "description": "RDF syntax for shapes (default \"turtle\"; for \
                                    example \"ntriples\")."
                },
                "mode": {
                    "type": "string",
                    "enum": ["edit", "view"],
                    "description": "\"edit\" (default) resolves editor widgets; \
                                    \"view\" derives a read-only form."
                },
                "shape": {
                    "type": "string",
                    "description": "Optional node-shape IRI to derive against, \
                                    instead of the first applicable shape."
                }
            },
            "required": ["focus", "shapes"],
            "additionalProperties": false
        })
    },
};

/// The read-only tool set, always advertised in the default build. `ask` / `nl_query`
/// (feature `nlq`) are appended by [`advertised`] only when a model backend is configured.
pub const READ_ONLY: &[&ToolSpec] = &[
    &QUERY,
    &CONSTRUCT,
    &INTROSPECT,
    &SHAPES,
    &STATS,
    &CLASSES,
    &PREFIXES,
    &VOID,
];

/// The full list of tools this server advertises, given its config — `UPDATE` is
/// appended only when [`McpServer`] was built with update enabled, and the NL tools
/// (`ask` / `nl_query`, feature `nlq`) only when an LLM backend is configured (so a
/// feature-on server with no key configured does not advertise an unusable tool — it
/// degrades cleanly).
pub fn advertised(server: &McpServer) -> Vec<&'static ToolSpec> {
    let mut tools: Vec<&'static ToolSpec> = READ_ONLY.to_vec();
    if server.allow_update() {
        tools.push(&UPDATE);
    }
    // Both NL tools share the one backend, so they are advertised (or withheld) together.
    #[cfg(feature = "nlq")]
    if crate::nlq::backend_configured() {
        tools.push(&ASK);
        tools.push(&NL_QUERY);
    }
    // [FABLE-5] sq-lsp7k.10: the template tools are advertised only when the embedder
    // registered at least one template (a template-less server does not advertise an
    // unusable tool — the `ask` degrade-cleanly pattern). An UPDATE template is listed
    // even on a read-only server (the agent can see it exists) but its invocation is
    // gated on `allow_update` at call time.
    #[cfg(feature = "templates")]
    if !server.config().templates.is_empty() {
        tools.push(&TEMPLATE_LIST);
        tools.push(&TEMPLATE_INVOKE);
    }
    // [FABLE-5] sq-lsp7k.10: full-text search is self-contained (the index builds lazily
    // from the loaded graph), so the feature alone advertises it.
    #[cfg(feature = "text")]
    tools.push(&TEXT_SEARCH);
    // [GPT-5.6] sq-lsp7k.22: validation is self-contained and read-only, so the
    // opt-in feature alone is the complete advertisement gate.
    #[cfg(feature = "shacl")]
    tools.push(&VALIDATE);
    // [FABLE-5] sq-lsp7k.1.6: form derivation shares the validator's caller-supplied
    // shapes surface, so the same opt-in feature advertises both read-only tools.
    #[cfg(feature = "shacl")]
    tools.push(&DESCRIBE_FORM);
    tools
}
