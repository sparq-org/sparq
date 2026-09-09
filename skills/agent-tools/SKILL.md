---
name: agent-tools
description: Use when an LLM/agent should access a sparq RDF dataset over the Model Context Protocol (MCP) as first-class tools — run SPARQL queries, mine the dataset schema, list class profiles or namespace prefixes, read stats or a VoID descriptor, optionally SHACL-validate against caller-supplied shapes or derive a shape-aware describe_form FormDescription for a focus node, and (gated, off by default) apply SPARQL updates. Covers the opt-in sparq-mcp crate — a JSON-RPC 2.0 MCP server (initialize, tools/list, tools/call, plus the default-build read-only resources/list, resources/read, prompts/list and prompts/get surfaces that expose the dataset VoID descriptor, the default graph and each named graph as N-Triples resources and serve four canned query prompts — explore-dataset, count-by-class, class-overview, predicate-usage — whose IRI arguments are RFC-3987-validated before they reach a SPARQL IRIREF) exposing query (SELECT/ASK to SPARQL-JSON), construct (CONSTRUCT/DESCRIBE to N-Triples), introspect (effective schema as JSON or token-budgeted text), stats, classes (class IRIs with instance and predicate counts), prefixes (namespace declarations and term counts), void (W3C VoID N-Triples), an opt-in read-only validate tool, and a gated update tool that is OFF by default; the transport-agnostic handle_message dispatch core plus the optional stdio feature; and the honest trust model (a local agent-tool server with no built-in auth, read-only by default, queries bounded by a QueryBudget). Also covers the opt-in solid feature — SolidMcpServer, a pod-backed server over sparq-solid's PodStore with WAC/ACP-authorized session-scoped query plus LDP resource tools (resource_get, container_list from stored ldp:contains data, and gated resource_put/resource_delete/container_create with existence non-disclosure and atomic ACL write-through) and session-scoped introspect/shapes/stats mined only from the documents the session may read (closing the whole-graph aggregate leak the base server's versions would open in a multi-principal deployment), plus the MCP resources capability with subscribe:true — resources/list, resources/read, resources/subscribe and content-free notifications/resources/updated bound to Solid Notifications semantics, authorized at subscribe time and again at every delivery. Also covers the opt-in nlq feature's two natural-language tools — ask (NL to SPARQL, validated and executed server-side, returning the executed query plus its real result rows) and nl_query (the translate-only variant — the same grounding and validation, but the query is returned unexecuted with executed:false and no rows, for review before you run it with query). Complements genai-retrieval (sparq-nlq/sparq-introspect) — that is the NL-to-SPARQL loop; this is the MCP front door.
---

# sparq agent-tools (MCP)

The **MCP front door** of the sparq RDF + SPARQL engine: the opt-in **`sparq-mcp`**
crate turns a loaded `sparq_core::Graph` into a **Model Context Protocol** server so an
LLM/agent can use the dataset as first-class tools.

It is **opt-in** (a separate crate; nothing in the workspace depends on it, the default
engine build does not compile it) and a **thin wrapper** over existing surfaces — it adds
no engine capability. JSON-RPC 2.0 framing is hand-rolled over `serde_json`; there is no
heavy MCP-SDK dependency.

## Tools

| tool | wraps | returns |
| --- | --- | --- |
| `query` | `sparq_engine::query_json` (SELECT/ASK) | SPARQL 1.1 Query Results JSON |
| `construct` | `sparq_engine::construct_ntriples` (CONSTRUCT/DESCRIBE) | N-Triples text |
| `introspect` | `sparq_introspect::Introspection` | effective schema — JSON or token-budgeted text |
| `shapes` | `sparq_introspect::Introspection` (per-class) | data-grounded SHACL-style shape for one class IRI |
| `stats` | graph count + introspection totals | small JSON object |
| `classes` | `sparq_introspect::Introspection` | class IRIs + instance and predicate counts, largest first |
| `prefixes` | `sparq_introspect::Introspection` | namespace declarations + distinct IRI term counts, largest first |
| `void` | `sparq_introspect::Introspection` | W3C VoID N-Triples; optional characteristic sets |
| `ask` *(feature `nlq`, OFF by default)* | `sparq_nlq` NL→SPARQL loop | executed SPARQL + real result rows (+ citations) |
| `nl_query` *(feature `nlq`, OFF by default)* | `sparq_nlq` grounding + `spargebra` validation, **no execution** | validated SPARQL, `executed: false`, **no rows** |
| `update` *(gated, OFF by default)* | `sparq_engine::update_in_place_atomic` | new triple count |
| `template_list` / `template_invoke` *(feature `templates`, OFF by default)* | `sparq_engine::templates` (#901 injection-safe binding) | definitions / typed fail-closed invocation |
| `text_search` *(feature `text`, OFF by default)* | `sparq_text::TextIndex` (BM25, lazily built + reconciled) | ranked literal hits JSON |
| `validate` *(feature `shacl`, OFF by default)* | `sparq_shacl::validate` | `{conforms, results}` JSON |
| `describe_form` *(feature `shacl`, OFF by default)* | `sparq_forms::derive_form` | `FormDescription` JSON, verbatim |

### SHACL validation (feature `shacl`, OFF by default) — [GPT-5.6] sq-lsp7k.22

Call `validate` with `{"shapes": "...Turtle..."}` and optionally `format` (defaults to
`turtle`). The tool parses the shapes as a separate graph, validates the server's data
graph, and returns `conforms` plus disallowed-severity results containing `focusNode`,
`path`, `severity`, and `message`. It never mutates the served graph. A malformed shapes
string returns an MCP tool result with `isError: true`, not a JSON-RPC protocol error.

### Shape-aware forms (feature `shacl`, OFF by default) — [FABLE-5] sq-lsp7k.1.6

Call `describe_form` with `{"focus": "<IRI or _:label>", "shapes": "...Turtle..."}`,
optionally `format` (defaults to `turtle`), `mode` (`"edit"`, the default, or `"view"`),
and `shape` (an explicit node-shape IRI instead of the first applicable one). The tool
parses the shapes with the same path `validate` uses, runs `sparq_forms::derive_form`
over the served data graph, and returns the **`FormDescription` JSON verbatim** — the
same contract the GUI renderer consumes (see `skills/shacl-forms/SKILL.md`), so an agent
gets a shape-directed, widget-scored view/editor description instead of guessing
predicates. Read-only: neither graph is mutated (form *editing* — applying a diff back —
is a separate gated surface, sq-lsp7k.1.4/F6b, not shipped here). Bad focus IRIs,
malformed shapes, and unknown modes are tool errors (`isError: true`), fail-closed.

### Named templates + full-text search (features `templates` / `text`, OFF by default) — [FABLE-5] sq-lsp7k.10

Register validated `sparq_engine::templates::Template`s on `ServerConfig::templates`; the
server then advertises `template_list` (definitions: name/kind/text/typed parameters) and
`template_invoke` (bind typed JSON arguments through the #901 **algebra rewrite** — never
string concatenation — and execute; **fail-closed** on an unknown template or an
unknown/missing/mistyped parameter). An **UPDATE template stays behind the same
`allow_update` gate** as the raw `update` tool: listed on a read-only server, refused at
invocation. `text_search` (feature `text`) BM25-ranks the graph's string literals via a
lazily-built `sparq-text` index, reconciled incrementally (`O(new dict terms)`) before every
search so it stays current across updates; modes `and`/`any`, `tok*` prefix matching for
autocomplete-style discovery. The HTTP server exposes the same template layer at
`/templates` (see the `http-server` skill §5g). Facet-count and richer IRI-autocomplete
tools are deliberately deferred to their own beads (the facet/autocomplete engine features).

### Resources + prompts (default build, read-only) — [SONNET-4.6] sq-sjey1

Beyond `tools`, `initialize` declares the MCP **`resources`** and **`prompts`**
capabilities. Both sub-capability flags are `false` and mean it: this server implements no
`resources/subscribe` and pushes no `notifications/*/list_changed` (pod mode, below, is the
one that declares `subscribe: true`). Neither surface adds a crate to the build or an
engine capability, and neither can mutate.

**`resources/list` / `resources/read`** project the served dataset:

| `uri` | content |
| --- | --- |
| `urn:sparq:dataset` | the W3C VoID descriptor (the `void` tool's default output) |
| `urn:sparq:graph:default` | every triple of the default graph, as N-Triples |
| *the graph IRI* | every triple of that **named graph**, as N-Triples |

Named graphs are listed sorted by IRI (deterministic), and a graph named by a blank node
is omitted rather than given an invented URI. A read materialises the graph through the
**same budgeted `sparq_engine` CONSTRUCT path** the `construct` tool uses, so the server's
deadline and row cap apply. Both `urn:sparq:` URIs are **reserved**. A URI the server does
not serve is `-32002` (`RESOURCE_NOT_FOUND`); a served resource that could not be
materialised (a tripped budget) is `-32603` — reporting that as "not found" would assert
something false about the dataset. `resources/read` on a graph hands the agent the whole
document; for anything selective use `query` / `construct`.

**`prompts/list` / `prompts/get`** serve a static, dataset-independent catalog of canned
query prompts, each rendering to one `user` text message: `explore-dataset` (which
introspection tools to call, in what order, plus a first probe), `count-by-class` (the
ready-to-run class census), `class-overview` (argument `class`) and `predicate-usage`
(argument `predicate`). Nothing here queries the graph, so a `prompts/get` is free.

The `class`/`predicate` arguments are interpolated into a SPARQL `IRIREF` (`<…>`), so they
are **parsed as absolute RFC-3987 IRIs first** — such an IRI cannot contain `<`, `>`, `"`,
`{`, `}`, `|`, `\`, `^`, a backtick, or any character below `0x21`, so a validated
argument provably cannot close the `IRIREF` and append clauses of its own. A hostile or
missing argument, and an unknown prompt name, are all `-32602` (`INVALID_PARAMS`) — the
prompt is refused, never rendered around unvalidated text.

### Pod mode (feature `solid`, OFF by default) — [FABLE-5] sq-u16eq

`SolidMcpServer` serves a **`sparq_solid::PodStore`** (one named graph per pod document
+ a materialized WAC/ACP authorization view), bound to ONE authenticated session fixed
at construction — the MCP-Solid proposal draft's local-trusted-agent deployment mode
(`site/specs/mcp-solid.typ` §6.4/§7.3/§9.3). Tools:

| tool | wraps | notes |
| --- | --- | --- |
| `query` | `wrap_for_view_opt_in` + `query_json_view_with_budget` | session-scoped; empty default graph, union opt-in via the reserved `FROM` IRI |
| `resource_get` | the document's named graph, serialized | same dataset `query` reads — the two surfaces cannot disagree; optional `accept` picks `application/n-triples` (default) or `text/turtle` |
| `container_list` | `ldp:contains` triples in the container's OWN graph | data-derived, never IRI-path guessing |
| `introspect` | `sparq_introspect::Introspection` over the session's authorized projection | schema of the readable documents only — never the whole pod |
| `shapes` | the same miner, one class | a class only unreadable documents use reports as absent |
| `stats` | totals over the same projection | two sessions get different totals; no grants ⇒ zeros |
| `update` *(gated)* | `PodStore::update_as_with_budget` / `update_as_acp_with_budget` | per-graph session write check, fail-closed; its SPARQL *evaluation* is bounded by the SAME per-call budget the read tools use (draft §9.4) — the check's `GRAPH ?var` binding SELECT and the apply's `… WHERE`. The rest is NOT budgeted, and a request cap covers only the inline `INSERT`/`DELETE DATA`: `CLEAR`/`DROP` cost whatever the session may write, and `LOAD` is refused unless the embedder installs an allowlisted base |
| `resource_put` *(gated)* | atomic named-graph swap (+ containment link on create) | `.acl`/`.acr` route through `put_acl`/`put_acl_acp` |
| `resource_delete` *(gated)* | slot removal + containment unlink | non-empty containers rejected; `.acl` via `delete_acl` |
| `container_create` *(gated)* | typed `ldp:BasicContainer` graph + containment | slash-terminated IRIs only |

Key contracts: a resource the session cannot read errors **byte-identically** to a
nonexistent one (existence non-disclosure, draft §9.3); ACL writes are gated on
`acl:Control` of the governed resource and re-derive authorization **atomically with
fail-closed rollback**; creation is authorized at the closest existing parent container
(the Solid creation rule); every content write re-materializes the view so the next
tool call sees it. RDF sources only (Turtle / N-Triples, plus JSON-LD under the `jsonld`
feature) — see the content-negotiation and non-RDF notes below.

**Session-scoped aggregates — [SONNET-4.6] sq-8n6iv.** The base server's `introspect` /
`shapes` / `stats` mine the WHOLE served graph. In a multi-principal deployment that is
an **aggregate leak**: it discloses the classes, predicates, vocabularies and volume of
documents the caller cannot open, and no per-resource check catches it because no
resource was read. Pod mode therefore does not reuse them. All three are mined from one
input — the session's *authorized projection*, CONSTRUCTed through the SAME
`sparq_engine::DatasetView` the `query` tool evaluates under — so an unauthorized
document contributes to no count in any of them, and a session with no grants gets an
empty schema and zero totals rather than the pod's real ones. The projection is rebuilt
per call (a write or an `.acl` change must not leave a stale schema behind) and runs
under the configured `QueryBudget`, whose `max_rows` **refuses rather than truncates** —
so an over-budget pod yields an error, never a quietly undercounted schema. What the
projection *does* legitimately include is data in readable documents that happens to name
an unreadable one — e.g. a readable container's `ldp:contains` link to a container the
session cannot read, the same disclosure `container_list` already makes.

**Content negotiation + the non-RDF story — [SONNET-4.6] sq-wbsf5.** `resource_get`
takes an optional `accept` and serves either `application/n-triples` (the default when
`accept` is absent, so existing callers are unchanged) or `text/turtle` — the SAME
triples from the SAME read gate, written by `oxttl`'s `TurtleSerializer` with the pod
vocabularies (`ldp`/`acl`/`acp`/`solid`/…) registered for compaction. It is ONE media
type, not an HTTP `Accept` list; `;`-parameters (`q=`) are ignored; anything else is a
tool error naming what IS served, never a silent coercion. Negotiation is a pure
function of the `accept` string evaluated BEFORE the read gate, so it cannot become an
existence oracle. JSON-LD is INGEST-only (`resource_put` under the `jsonld` feature) —
there is no JSON-LD writer, so `accept: application/ld+json` is refused.
`resources/read` has no per-request `accept` in MCP, so it stays N-Triples.

**Non-RDF (binary) resources are SCOPED OUT — by decision, not omission.** The pod IS an
RDF dataset (one named graph per document); a binary body has nowhere to live, and a
base64-in-a-literal side-channel would be a fake pod whose ACL has no graph to anchor
to. `resource_put` therefore refuses a non-RDF `content_type` with a message that names
the scope-out, and no non-RDF resource can exist to be read. Adding one is a
`sparq-solid` STORAGE design (blob half + its authorization join), not an MCP
tool-surface change; if it lands, the MCP shape is already known — `resources/read`
carries a base64 `blob` field alongside `text`.

#### The `resources` surface + notifications (draft §8/§10) — [SONNET-4.6] sq-cmjmr

Pod mode declares the MCP **`resources`** capability with **`subscribe: true`** and
binds it to Solid Notifications Protocol semantics:

| method | behaviour |
| --- | --- |
| `resources/list` | the pod documents this session may READ, one resource per document, `uri` = the resource IRI. An unreadable document is simply ABSENT — never an entry, never an error |
| `resources/read` | the same N-Triples bytes `resource_get` serves, in the MCP `contents` shape; unreadable and nonexistent share one error (`-32002`) |
| `resources/subscribe` | subscribes to the topic IRI (the MCP spelling of a Solid Notifications subscription on that topic) |
| `resources/unsubscribe` | idempotent and uniform — an unknown topic gets the same empty result, so it discloses nothing |

**Authorization is checked twice** (draft §10): at subscribe time (no read access ⇒ the
existence-non-disclosure not-found error, so subscribing cannot probe for resources) and
**again before every delivery**. When a session's read access is revoked mid-stream,
deliveries stop **silently** — no notification at all, least of all a "your access was
revoked" one, which would itself disclose that a resource it may no longer read had
changed. The subscription survives, so restoring access resumes deliveries. The
delivery check runs at the target's authorization anchor, so a `Delete` (whose resource
no longer has a policy of its own) is authorized by the container that governs its IRI —
and additionally requires that the topic itself was readable immediately BEFORE the
deletion, so the ancestor fallback cannot re-grant read that a resource-specific ACL had
revoked.

Emission is **pull-based**, matching the transport-agnostic dispatch core: a mutating
tool call queues what it changed and the embedder drains it with
`SolidMcpServer::take_notifications()` after each `handle_message`, writing the messages
to its transport. Payloads are **content-free** — `notifications/resources/updated`
carries the topic IRI and an ActivityStreams 2.0 verb (`Create`/`Update`/`Delete`, and
`Add`/`Remove` when a container's stored `ldp:contains` membership grew/shrank), never
the changed triples; the subscriber re-reads through the authorized read path. Change
detection digests each subscribed topic before/after a mutation, so the SPARQL `update`
tool — which does not report the documents it touched — is covered like the LDP tools.
Honest limit: the digest is a 64-bit order-independent content digest, not a
cryptographic hash, so a collision could suppress one signal (a missed update, never a
leak — the same class the Solid Notifications reconnect contract asks clients to
reconcile by re-reading). v1 requires a subscribed topic to exist at subscribe time.

Every tool ships a proper MCP `inputSchema` (JSON-Schema). `tools/list` returns
`name` / `description` / `inputSchema` per tool. A `tools/call` result is the MCP
`CallToolResult` shape (`content` text item + `isError`); a bad SPARQL string is an MCP
**tool error** (`isError: true`), not a JSON-RPC protocol error, so the agent can read it
and retry.

### Grounding tools (2026-06-23 design call — `shapes` + `ask`; later `nl_query`)

Complementary ways to turn a natural-language question into SPARQL. `shapes` and `ask`
were chosen **both, opt-in** on the 2026-06-23 call; `nl_query` was added later under the
same `nlq` feature:

- **`shapes`** (structured, **no LLM**, default build) — give it a **class IRI** and it
  returns the predicates instances of that class actually use, each with coverage,
  observed datatypes / object-kind (IRI vs literal), observed range, and the
  cardinalities the data proves (`min_count`/`max_count` emitted **only** when the data
  establishes the bound). The **client's own** LLM grounds NL→SPARQL on this. Reuses the
  introspection miner; describes the **effective** schema (what the graph asserts), not an
  aspirational contract.
- **`ask`** (NL, **opt-in `nlq` feature**) — runs the whole NL→SPARQL→validate→execute
  loop **server-side** via `sparq-nlq` and returns the **executed SPARQL** + the **real
  result rows** (+ in-graph citations). It embeds a **configurable** LLM call: cost and
  quality depend on the model **you** configure — `ANTHROPIC_API_KEY`, or an
  OpenAI-compatible `SPARQ_NLQ_ENDPOINT_URL` + `SPARQ_NLQ_ENDPOINT_MODEL` (+ optional
  `_KEY`). **No model is bundled**, nothing phones home. With the feature ON but **no**
  backend configured, `ask` is unadvertised and a direct call returns a clear *"not
  configured"* error — **never** a fabricated answer, never a panic. The answer is the
  query's real rows, not a free-form paragraph.

- **`nl_query`** (NL, same **opt-in `nlq` feature**, `sq-sj1f9`) — the **translate-only**
  middle ground: the same grounding as `ask` and the same **pre-execution** checks — the
  question guard, a `spargebra` parse, and the forbidden-construct refusal — but it stops
  before execution and hands back the query. The response carries `executed: false` and
  **no rows** — it is a query to review, not an answer. A query that parses is still
  refused if it uses a construct the loop will not run (`SERVICE` federation), so
  translation cannot be used to route around `ask`'s refusal; run the returned query with
  `query`. The honest cost of skipping execution is that validation is only **syntactic**:
  the query may still fail at runtime or match nothing. Note that sparq-nlq's
  dictionary-grounding constraint (`NlqConfig::check_dictionary`) is opt-in and **off** in
  the default config, for `ask` as well as `nl_query` — so an ungrounded predicate/class
  IRI is accepted by **both** (`ask` just executes it to zero rows). Same backend, same
  fail-closed "not configured" error.

These are **ergonomics / grounding aids pending measurement** — *not* a token-saving
claim (the project measured representation/token tricks as duds). `shapes` is the lean
no-model default; `ask` trades a model call for not writing SPARQL yourself, and
`nl_query` trades it for the query alone when you want to read it before it runs. The
first two overlap deliberately (`shapes` puts the model on the client; `ask` on the
server).

## Use it

```rust,no_run
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;
use sparq_mcp::{McpServer, ServerConfig};

let graph = Graph::load_str("<a> <b> <c> .", "ntriples")?;

// Embed: feed one JSON-RPC line, get one line back (no transport needed).
let mut server = McpServer::new(graph); // read-only by default
let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
let _reply = server.handle_message(init).unwrap();

// Allow writes EXPLICITLY:
let cfg = ServerConfig { allow_update: true, ..ServerConfig::default() };
# let _ = cfg;
# Ok(()) }
```

`initialize` negotiates the MCP protocol revision: a client-proposed `protocolVersion`
in `sparq_mcp::SUPPORTED_PROTOCOL_VERSIONS` is accepted verbatim; otherwise the server
answers with its latest (`sparq_mcp::PROTOCOL_VERSION`) and the client decides whether
to proceed. The tools flow is identical across the supported revisions.

`handle_message` takes one complete JSON **value**, which may be a single request object
or a top-level **JSON-RPC 2.0 §6 batch array**. Batches are what the `2025-03-26`
revision requires (batching was added there and removed again in `2025-06-18`), and
receiving them is what lets that revision sit in `SUPPORTED_PROTOCOL_VERSIONS`. A batch
is dispatched element by element, in order, against the same server, and answered with
an array of the non-notification responses; a batch of nothing but notifications gets no
response at all, an empty array is answered with a single `-32600` error object, and a
malformed element gets its own null-id error without voiding the rest of the batch.
Batches are accepted whatever revision was negotiated — an array is only ever emitted in
reply to an array, so a client on a batch-free revision is never handed one it did not
ask for.

### Which engine plans a tool call runs (default-on, [SONNET-4.6] sq-mc06h)

`sparq-mcp` is a **native** engine-embedding surface with no bundle-size floor, so — like
`sparq-cli`, `sparq-server` and the `sparq-rdf` Python wheel — its **default** feature set
lights two `sparq-engine` planner opt-ins that the engine LIBRARY leaves off:

- **`algebra-rewrite`** — the pre-execution algebra rewrite pass: `FILTER(?v = <iri>)`
  IRI-constant substitution plus `FILTER(!bound)` → anti-join, applied before evaluation.
  IRI constants **only** — a literal equality is never rewritten (the `sq-lr2ii` avoidance
  contract).
- **`dp-planner`** — the DPccp join-order planner. A connected BGP of 3 or more patterns
  that fits the connected-subgraph budget is planned as a cost-optimal **bushy** join tree
  instead of by greedy GOO. It is default-on once compiled, so a `tools/call` gets it with
  no explicit install; `sparq_engine::without_dp_planner` opts out for one scope.

Both are **result-equivalent** — they change which plan runs, never the answer — and pull
**zero** new dependencies, so the default dependency graph is unchanged. The point is that
an agent's `query` / `construct` call executes the same plans the CLI and the canonical
benchmarks measure, rather than a rewrite-dark, greedy-GOO variant of the same engine.
Build `sparq-mcp` with `--no-default-features` for the unoptimised engine.

This is a **per-surface** decision, not a blanket one: `sparq-wasm` deliberately keeps both
off, because that surface has a bundle-size floor the native ones do not.
`crates/sparq-mcp/tests/planner_features_default.rs` is the tripwire that the default set
keeps them lit and that the forwarding actually reaches the engine.

The standard MCP stdio transport is behind the **`stdio`** feature:
`sparq_mcp::serve_stdio(&mut server)` runs the line-delimited JSON-RPC loop over this
process's stdin/stdout. For an arbitrary reader/writer pair use `sparq_mcp::serve`.

## Running the server binary

The same feature ships a ready-made server, so an MCP client can launch sparq as a
subprocess without you writing a `main`:

```bash
cargo run -p sparq-mcp --features stdio -- [--allow-update] [--format FMT] \
    [--query-timeout SECS] [--max-rows N] [DATA_FILE]
```

- `DATA_FILE` is loaded into the served graph (omit it for an empty graph); the format is
  taken from `--format`, else inferred from the extension (`.nt` → ntriples, `.nq` →
  nquads, `.trig` → trig, anything else turtle). N-Quads/TriG keep their named graphs.
- `--allow-update` is the write switch — **without it the process is read-only** and
  `update` is neither advertised nor callable. `--query-timeout 0` / `--max-rows 0`
  disable the corresponding bound.
- Startup lines go to **stderr**; stdout carries nothing but JSON-RPC responses. A bad
  flag or an unreadable/malformed data file exits non-zero instead of serving.

`sparq_mcp::cli` (`parse_args` / `load_graph` / `format_for` / `USAGE`) is the same
library code the binary runs, for a host that wants the flags but its own transport.
[SONNET-4.6] sq-5xgxe

## Trust model (read this — no overclaim)

This is a **local agent-tool server, not a hardened multi-tenant endpoint**. There is
**no built-in authentication or authorization**: the MCP transport is the trust boundary
you, the operator, establish, and whoever can speak to the server has exactly the access it
was configured with.

- **Read-only by default** — a default `McpServer` advertises and accepts only
  `query` / `construct` / `introspect` / `shapes` / `stats` / `classes` / `prefixes` / `void`; it
  cannot mutate the dataset.
- **`update` is a mutation surface**, exposed **only** when `ServerConfig::allow_update`
  is set (the `sparq-mcp` binary's `--allow-update` flag). It is the single write switch;
  there is no finer per-tool ACL.
- **Queries are bounded** by a `QueryBudget` (deadline + row cap; default 30 s / 1M rows)
  so one `tools/call` cannot run the server unbounded — a blunt anti-DoS ceiling, not a
  fairness quota.

## Status / scope

Opt-in crate at workspace v0.1.0; verified against branch `main` (default `classes` tool
2026-07-13 [GPT-5.6], sq-cekgj; default `prefixes` tool 2026-07-13 [GPT-5.6], sq-kx5b0;
default `void` tool 2026-07-12 [GPT-5.6], sq-2kkym;
default resources + prompts surfaces 2026-07-28 [SONNET-4.6], sq-sjey1;
default-on `algebra-rewrite` + `dp-planner` 2026-07-30 [SONNET-4.6], sq-mc06h;
pod mode 2026-07-11 [FABLE-5]).
Tested by a real in-memory MCP round-trip (default features), a real stdio serve-loop
round-trip, and a spawned-process session against the shipped binary (feature `stdio`,
2026-07-28 [SONNET-4.6], sq-5xgxe). Only the **stdio** transport plus the embeddable
`handle_message` ship today; SSE/HTTP transports are **not implemented** (follow-up beads).
The read-only default is proven fail-closed (a disabled `update` returns `-32601` and does
not mutate the graph).

## Learn more

- **Crate** — [`sparq-mcp`](../../crates/sparq-mcp) (README + rustdoc).
- **MCP spec** — <https://modelcontextprotocol.io>.
- **NL→SPARQL & schema grounding** — [`genai-retrieval`](../genai-retrieval/SKILL.md).
- **Underlying query API** — [`sparql-query`](../sparql-query/SKILL.md).
