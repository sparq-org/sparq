<!-- [OPUS-4.8] sq-0z43i (gh #909): new opt-in crate. -->
# sparq-mcp

<p>
  <a href="https://crates.io/crates/sparq-mcp"><img src="https://img.shields.io/crates/v/sparq-mcp.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-mcp"><img src="https://docs.rs/sparq-mcp/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

A **Model Context Protocol (MCP)** server exposing a sparq RDF graph as **agent tools** —
an **opt-in** crate. An LLM/agent can use `query`, `introspect`, `stats`, `classes`, `prefixes`, and `void` over a
dataset as first-class MCP tools; SPARQL **`update` is OFF by default**. It is a thin
wrapper over the existing engine read API (`sparq-engine` query path + `sparq-introspect`
schema mining): nothing in the workspace depends on it, the default engine build does not
compile it, and it adds **zero engine capability** and **no heavy dependency** (JSON-RPC
2.0 framing is hand-rolled over `serde_json`). Its **default** feature set does light two sparq-engine planner opt-ins — `algebra-rewrite` and `dp-planner` — so a tool call executes the same plans the CLI and the canonical benchmarks measure; both are result-equivalent and pull zero new dependencies, and `--no-default-features` opts out. <!-- [SONNET-4.6] sq-mc06h -->

## 🚀 Quickstart

```rust,no_run
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;
use sparq_mcp::{McpServer, ServerConfig};

let graph = Graph::load_str("<a> <b> <c> .", "ntriples")?;

// Read-only by default (update tool not advertised or callable).
let mut server = McpServer::new(graph);

// Drive it with your own transport: feed one JSON-RPC line, get one line back.
let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
let reply = server.handle_message(init).unwrap();
assert!(reply.contains("serverInfo"));

// To allow writes, opt in EXPLICITLY:
let _writable = ServerConfig { allow_update: true, ..ServerConfig::default() };
# Ok(()) }
```

The `stdio` feature ships the **`sparq-mcp` binary**: `cargo run -p sparq-mcp --features
stdio -- [--allow-update] [--format FMT] [--query-timeout SECS] [--max-rows N] [DATA_FILE]`
loads the file (format inferred from `.nt`/`.nq`/`.trig`, else turtle) and serves it over the
standard MCP stdio transport — the same loop `serve_stdio(&mut server)` runs for an embedder.

## ✨ Tools

- **`query`** — run a read-only SELECT/ASK; returns SPARQL 1.1 Query Results JSON. Bounded by a configurable `QueryBudget` (default 30 s / 1M rows).
- **`construct`** — run a read-only CONSTRUCT/DESCRIBE; returns N-Triples text.
- **`introspect`** — the effective schema the graph actually uses (classes, predicates, prefixes, characteristic sets) as JSON or token-budgeted text; exact counts, no sampling.
- **`shapes`** — given a class IRI, the data-grounded SHACL-style shape of that class: valid predicates, datatypes/object-kinds, observed range, and only the cardinalities the data proves. Structured grounding for a **client** LLM — no server-side model.
- **`stats`** — dataset totals (triples, distinct subjects, typed entities, class / predicate / namespace counts).
- **`classes`** — list class IRIs with instance and predicate counts, largest class first. <!-- [GPT-5.6] sq-cekgj -->
- **`prefixes`** — list detected namespace declarations and distinct IRI term counts, largest namespace first. <!-- [GPT-5.6] sq-kx5b0 -->
- **`void`** — emit a W3C VoID dataset descriptor as N-Triples, optionally including characteristic-set statistics; `dataset` defaults to `urn:sparq:dataset`.
- **`ask`** / **`nl_query`** *(feature `nlq`, OFF by default)* — NL via `sparq-nlq`: `ask` runs NL→SPARQL→validate→**execute** server-side (executed SPARQL + real result rows + citations); `nl_query` **translates only**, returning a validated (parses, no `SERVICE`) but **unexecuted** query to review and run via `query`. Both embed a **configurable** LLM call (`ANTHROPIC_API_KEY`, or OpenAI-compatible `SPARQ_NLQ_ENDPOINT_URL`+`_MODEL`); no model is bundled, and unconfigured they are unadvertised and error "not configured" — never fabricate.
- **`update`** *(gated, OFF by default)* — apply an atomic SPARQL 1.1 Update; neither advertised in `tools/list` nor callable unless `ServerConfig::allow_update` is set.
- **`template_list` / `template_invoke`** *(feature `templates`, OFF by default)* — named parameterized templates (registered on `ServerConfig::templates`) invoked with **typed, fail-closed** JSON arguments through the #901 injection-safe algebra binding; an UPDATE template stays behind the same `allow_update` gate (sq-lsp7k.10).
- **`text_search`** *(feature `text`, OFF by default)* — BM25 full-text search over the graph's string literals (`sparq-text`; lazily built, incrementally reconciled).
- **`validate`** *(feature `shacl`, OFF by default)* — read-only validation against caller-supplied shapes; returns `{conforms, results}`, with parse failures as tool errors. [GPT-5.6] sq-lsp7k.22
- **`describe_form`** *(feature `shacl`, OFF by default)* — derive a shape-aware form for one focus node against caller-supplied shapes via `sparq-forms`; returns the `FormDescription` JSON **verbatim** (fields, widget choices, constraints, current values). Read-only; `mode` `edit`/`view`, optional explicit `shape` IRI. [FABLE-5] sq-lsp7k.1.6

## ✨ Resources, prompts, and pod mode

Beyond `tools`, the default build declares the MCP **`resources`** and **`prompts`**
capabilities — read-only, adding no crate to the build, both `listChanged: false`/no `subscribe`
(nothing pushes unsolicited notifications). `resources/list` exposes `urn:sparq:dataset`
(the VoID descriptor), `urn:sparq:graph:default`, and one resource per **named graph**
(`uri` = the graph IRI); `resources/read` returns N-Triples through the same budgeted
engine path as `construct`. `prompts/list` / `prompts/get` serve four canned query
prompts — `explore-dataset`, `count-by-class`, `class-overview`, `predicate-usage` —
whose IRI arguments are RFC-3987-validated before interpolation into a SPARQL `IRIREF`,
so a hostile argument is refused rather than rendered. <!-- [SONNET-4.6] sq-sjey1 -->

**Pod mode** *(feature `solid`, OFF by default)*: `SolidMcpServer` serves a `sparq-solid`
`PodStore` (named graph per document, WAC/ACP-authorized, bound to one session) with LDP
tools per the MCP-Solid proposal draft — session-scoped `query`, `resource_get`,
`container_list` (containment from stored `ldp:contains` data, never IRI-path guessing),
`introspect` / `shapes` / `stats` mined from **only the documents the session may read** (all three derive from the same authorized `DatasetView` `query` runs under, so no grants means an empty schema and zero totals — the base server's whole-graph versions would instead hand one principal the classes, predicates and volume of documents it cannot open, an aggregate leak no per-resource check catches) <!-- [SONNET-4.6] sq-8n6iv -->, and gated `update` / `resource_put` / `resource_delete` / `container_create`. A resource the session cannot read errors **identically to one that does not exist**; `.acl`/`.acr` writes route
through the pod store's atomic fail-closed ACL write-through. `resource_get` serves
N-Triples or `text/turtle` on `accept` (anything else refused, never coerced), and
**non-RDF binaries are scoped out by decision** (an RDF pod has nowhere to put the bytes). <!-- [SONNET-4.6] sq-wbsf5 --> Its own `resources` surface adds **`subscribe: true`**:
`resources/subscribe` binds a Solid Notifications subscription, a change queues a
**content-free** `notifications/resources/updated` (topic + ActivityStreams verb, never
the triples) drained via `take_notifications()`, and read access is re-checked **at every
delivery** — a revoked session silently stops receiving, since a revocation notice would
itself disclose the change. <!-- [SONNET-4.6] sq-cmjmr -->

Each tool ships a proper MCP `inputSchema` (JSON-Schema); `tools/call` wraps output in
the MCP `CallToolResult` shape, and a bad query is an MCP tool error (`isError: true`),
not a protocol error, so the agent can read it and retry.

## 🔒 Trust model — read this

This is a **local agent-tool server, not a hardened multi-tenant endpoint**. It has **no
built-in authentication or authorization**: the MCP transport (stdio) is a trust boundary
you, the operator, establish — whoever can speak to the server has exactly the access the
server was configured with. Run it only against a client you trust.

- **Read-only by default.** Default tools cannot mutate; the feature-gated `validate` and `describe_form` tools are read-only.
- **`update` is a mutation surface** and is exposed **only** when you set
  `ServerConfig::allow_update = true` (the binary's `--allow-update` flag). Turn it on only
  when the client is trusted to issue writes; there is no per-tool ACL beyond this switch.
- **Queries are bounded** by a `QueryBudget` (deadline + row cap) so one tool call cannot
  run the server unbounded — a blunt anti-DoS ceiling, not a fairness quota.

No overclaim: it adds no isolation, sandboxing, or auth the host process does not provide.

## 📚 Learn more

- **How-to** — [`skills/agent-tools/SKILL.md`](../../skills/agent-tools/SKILL.md).
- **API reference** — [docs.rs/sparq-mcp](https://docs.rs/sparq-mcp).
- **MCP spec** — <https://modelcontextprotocol.io>.
- **Underlying engine** — [`sparq-engine`](../sparq-engine) (query path) and
  [`sparq-introspect`](../sparq-introspect) (schema mining).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
