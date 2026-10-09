#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! # Build-out status
//!
//! This is the document-level W3C JSON-LD 1.1 program (design record
//! `research/jsonld-1.1-design.md`). The surface that ships today is:
//!
//! - [`json`] — the minimal, dependency-free `Json` AST (moved verbatim out of
//!   `sparq-engine`'s JSON-LD writer; public API preserved via a re-export shim there),
//!   plus a small `Json::parse` used to read remote contexts.
//! - [`error`] — the full JSON-LD 1.1 error-code registry ([`JsonLdErrorCode`]) plus the
//!   fallible [`JsonLdError`] type every algorithm returns.
//! - [`options`] — [`JsonLdOptions`], the processing options honoured across the pipeline.
//! - [`loader`] — the [`DocumentLoader`] trait, **deny-by-default** via [`NoopLoader`]
//!   (no ambient network — a remote fetch raises `loading document failed`), plus the
//!   local-fixture [`FsLoader`].
//! - [`context`] — the **Context Processing** foundation (bead `sq-oy1f.24`): the
//!   `ActiveContext`, Create Term Definition, and IRI Expansion. Bead `sq-90mu3` adds the
//!   compaction-side companions: Inverse Context Creation (§4.3), IRI Compaction (§7.1),
//!   and Term Selection (§7.2), exposed as `ActiveContext::inverse_context` and
//!   `compact_iri`.
//! - [`expand`](mod@expand) — the document-level **Expansion Algorithm** + Value Expansion
//!   (bead `sq-oy1f.25`): [`expand`](expand::expand) turns a compact document into its
//!   canonical expanded form (scoped contexts, container maps, `@nest`, `@reverse`,
//!   `@included`, `@json`, keyword aliases), threading `frameExpansion` for the framing
//!   pipeline.
//! - [`node_map`] — **Node Map Generation** + Generate Blank Node Identifier (bead
//!   `sq-oy1f.26`, JSON-LD 1.1 API §7.2/§7.4): [`node_map::generate_node_map`] indexes an
//!   expanded document's node objects by graph and `@id`, minting deterministic blank-node
//!   labels; the shared intermediate the flattening pipeline projects from.
//! - [`flatten`](mod@flatten) — the document-level **Flattening Algorithm** (bead
//!   `sq-oy1f.26`, §7.1): [`flatten::flatten`] expands, node-maps, folds named graphs under
//!   `@graph`, sorts by `@id`, and drops empty nodes, returning the flattened expanded form.
//! - [`compact`] — the document-level **Compaction Algorithm** + Value Compaction (bead
//!   `sq-oy1f.27`), including scoped contexts, container reshaping, keyword aliasing, and
//!   the `compactArrays` / `compactToRelative` / `ordered` options.
//! - [`from_rdf`] — **Deserialize RDF as JSON-LD** (bead `sq-oy1f.28`), converting the
//!   crate-local RDF dataset model into an expanded document, including native types,
//!   direction handling, JSON literals, and RDF-list reconstruction.
//! - [`frame`] — the document-level **Framing Algorithm** (beads `sq-oy1f.27` / `.29`),
//!   including frame matching, value patterns, embedding, defaults, named graphs, and
//!   framing-specific validation. Its pinned W3C framing lane passes all 92 cases.
//!
//! [GPT-5.6] (`sq-ci15w`) Only [`to_rdf`] and [`api`] remain documented stubs. The
//! implemented pipeline is `todo!()`-free; remote loading and HTML extraction remain
//! separate, opt-in follow-on capabilities.

// Real, shipped surfaces.
pub mod context;
pub mod error;
pub mod json;
pub mod loader;
pub mod options;

// Document-level algorithms. `to_rdf` and `api` retain their documented placeholder
// surfaces; expansion, flattening, compaction, framing, and from-RDF are implemented.
pub mod api;
pub mod compact;
pub mod expand;
pub mod flatten;
pub mod frame;
pub mod from_rdf;
mod fx;
pub mod node_map;
pub mod to_rdf;

pub use context::inverse::{compact_iri, InverseContext};
pub use context::{ActiveContext, Direction, Override, TermDefinition};
pub use error::{JsonLdError, JsonLdErrorCode};
pub use expand::expand;
pub use flatten::{flatten, flatten_expanded};
pub use json::{Json, JsonParseError};
pub use loader::{DocumentLoader, FsLoader, NoopLoader, RemoteDocument};
pub use node_map::{generate_node_map, BlankNodeIssuer, GraphMap, NodeMap};
pub use options::{EmbedFlag, JsonLdOptions, ProcessingMode, RdfDirection};
