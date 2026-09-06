// [OPUS-4.8] (sq-0z43i, gh #909) Render the crate README as the docs.rs / rustdoc
// front page so crates.io and docs.rs show the same content.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! ## Crate layout
//!
//! - [`McpServer`] — the server: hold a loaded [`sparq_core::Graph`] plus a
//!   [`ServerConfig`], then drive it via [`McpServer::handle_message`] (embed your own
//!   transport) or, with the `stdio` feature, the `serve_stdio` loop.
//! - [`jsonrpc`] — the minimal JSON-RPC 2.0 framing the MCP wire format uses.
//! - [`tools`] — the advertised tool specs (`name` / `description` / `inputSchema`).
//! - [`resources`] — the MCP **resources** surface (`resources/list` / `resources/read`):
//!   the served dataset as a VoID descriptor, the default graph, and one resource per
//!   named graph, each as N-Triples through the same budgeted engine path the
//!   `construct` tool uses. Read-only; adds no crate to the build. [SONNET-4.6] sq-sjey1
//! - [`prompts`] — the MCP **prompts** surface (`prompts/list` / `prompts/get`): a static
//!   catalog of canned query prompts (`explore-dataset`, `count-by-class`,
//!   `class-overview`, `predicate-usage`) over the tools that already exist. An IRI
//!   argument is RFC-3987-validated before it is interpolated into a SPARQL `IRIREF`.
//!   [SONNET-4.6] sq-sjey1
//! - [`shapes`] — the structured `shapes` grounding tool (the data-grounded
//!   predicate/datatype/cardinality constraints for one class IRI; no server-side model).
//! - `nlq` (feature `nlq`) — the server-side natural-language tools: `ask`
//!   (NL→SPARQL→execute via `sparq-nlq`) and `nl_query`, which runs the same grounding
//!   and the same pre-execution checks (question guard, `spargebra` parse,
//!   forbidden-construct refusal) but returns the query **unexecuted** for review
//!   ([SONNET-4.6] sq-sj1f9). Both embed a configurable LLM call and degrade cleanly
//!   (unadvertised, "not configured" error) when no backend is set.
//! - `transport` (feature `stdio`) — the line-delimited stdio serve loop.
//! - `cli` (feature `stdio`) — the argument parsing + dataset loading behind the shipped
//!   `sparq-mcp` binary (`--allow-update` / `--format` / `--query-timeout` / `--max-rows`
//!   + a positional data file). [SONNET-4.6] sq-5xgxe
//! - `solid` (feature `solid`) — the pod-backed server (`SolidMcpServer`): LDP
//!   container/resource CRUD tools (`resource_get`/`container_list`, plus the gated
//!   `resource_put`/`resource_delete`/`container_create`) with WAC/ACP-authorized,
//!   session-scoped semantics over a `sparq_solid::PodStore`. [FABLE-5] sq-u16eq
//!   It also serves the MCP **`resources`** surface (`resources/list`, `resources/read`,
//!   and `resources/subscribe` / `resources/unsubscribe` with `subscribe: true`), bound
//!   to Solid Notifications semantics: content-free `notifications/resources/updated`
//!   messages, authorized at subscribe time AND at every delivery. [SONNET-4.6] sq-cmjmr
//!   Its `introspect` / `shapes` / `stats` tools are mined from the session's AUTHORIZED
//!   projection rather than the whole pod: [`McpServer`]'s whole-graph versions would
//!   disclose the schema and volume of documents the principal cannot open — an
//!   aggregate leak no per-resource check catches. [SONNET-4.6] sq-8n6iv
//! - `templates` (feature `templates`) — the `template_list` / `template_invoke` tools:
//!   named parameterized SPARQL templates registered on `ServerConfig::templates` and
//!   invoked with typed JSON arguments through the injection-safe algebra binding
//!   (fail-closed on unknown/missing/mistyped parameters); an UPDATE template stays
//!   behind the same `allow_update` gate as the raw `update` tool. [FABLE-5] sq-lsp7k.10
//! - `text` (feature `text`) — the `text_search` tool: BM25 full-text search over the
//!   graph's string literals via a lazily-built, incrementally-reconciled `sparq-text`
//!   index. [FABLE-5] sq-lsp7k.10
//! - `shacl` (feature `shacl`) — the read-only `validate` tool: validate the served
//!   graph against caller-supplied shapes via `sparq-shacl`. [GPT-5.6] sq-lsp7k.22
//!   The same feature lights the read-only `describe_form` tool: derive a shape-aware
//!   form for one focus node against caller-supplied shapes via `sparq-forms`,
//!   returning the `FormDescription` JSON verbatim. [FABLE-5] sq-lsp7k.1.6

pub mod jsonrpc;
// [SONNET-4.6] sq-sjey1: the canned-query prompt catalog behind `prompts/list`/`prompts/get`.
pub mod prompts;
// [SONNET-4.6] sq-sjey1: the dataset/named-graph projection behind `resources/list`/`resources/read`.
pub mod resources;
pub mod server;
pub mod shapes;
pub mod tools;

// [OPUS-4.8] sq-jxjgr: the server-side NL `ask` tool, behind the opt-in `nlq` feature.
#[cfg(feature = "nlq")]
#[cfg_attr(docsrs, doc(cfg(feature = "nlq")))]
pub mod nlq;

#[cfg(feature = "stdio")]
#[cfg_attr(docsrs, doc(cfg(feature = "stdio")))]
pub mod transport;

// [SONNET-4.6] sq-5xgxe: the shipped `sparq-mcp` binary's argument parsing + dataset
// loading, kept in the library so the startup contract is unit-testable.
#[cfg(feature = "stdio")]
#[cfg_attr(docsrs, doc(cfg(feature = "stdio")))]
pub mod cli;

// [FABLE-5] sq-u16eq: the pod-backed server with LDP resource tools, behind the opt-in
// `solid` feature.
#[cfg(feature = "solid")]
#[cfg_attr(docsrs, doc(cfg(feature = "solid")))]
pub mod solid;

// [SONNET-4.6] sq-cmjmr: the subscription registry + notification machinery the pod
// server's `resources/subscribe` surface is built on. Crate-internal — the public seam
// is `SolidMcpServer::take_notifications`.
#[cfg(feature = "solid")]
pub(crate) mod notifications;

pub use server::{McpServer, ServerConfig, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};
pub use tools::ToolSpec;

/// The pod-backed MCP server + its configuration (feature `solid`). See `solid`.
#[cfg(feature = "solid")]
#[cfg_attr(docsrs, doc(cfg(feature = "solid")))]
pub use solid::{SolidMcpServer, SolidServerConfig};

/// Serve the MCP protocol over this process's stdin/stdout (the standard MCP stdio
/// transport). Available with the `stdio` feature. See `transport::serve_stdio`.
#[cfg(feature = "stdio")]
#[cfg_attr(docsrs, doc(cfg(feature = "stdio")))]
pub use transport::{serve, serve_stdio};
