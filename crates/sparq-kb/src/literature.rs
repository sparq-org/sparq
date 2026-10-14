//! # Literature-ingestion scaffolding — on FIXTURES, no live calls (GenAI-KB Phase 5)
//!
//! sq-2489d.5 (epic sq-2489d); design record
//! `research/provenance-driven-genai-kb.md` §4 (the provenance-stamped literature-
//! trawling architecture) and §5 Phase 5. 🤖 SPARQ agent — provenance-driven GenAI KB.
//! Written while Fable unavailable; flag for re-review when Fable returns.
//!
//! OPT-IN behind the default-OFF `literature` cargo feature (pulls in only a JSON reader,
//! NOT the SPARQL engine — the scaffolding is a pure deterministic data-transform). The
//! lean default build of `sparq-kb` carries none of this.
//!
//! ## What this is (and is NOT)
//!
//! This is the **shape** of the literature-ingestion pipeline the design calls for,
//! exercised end-to-end over **committed fixtures**:
//!
//! ```text
//! [connector] -> [normalise] -> [extract (record/replay)] -> [ground] -> [emit TTL]
//!     -> [SHACL gate, run by the caller] -> [sidecar verdict]
//! ```
//!
//! **It makes ZERO network and ZERO live-model calls.** The only LLM-in-the-loop step —
//! extraction — is isolated behind the `extract::Extractor` **record/replay trait** (a
//! `literature`-feature item), and the only adapter shipped here is
//! `extract::RecordedExtractor`, which **replays a
//! committed tape**. The live extractor (a real cheap-model batch) is the separate,
//! credential-gated Phase-6 bead (`sq-t5f3l`, `needs-access`); an agent cannot mint the
//! S2/OpenAlex/Anthropic credentials it needs, so Phases 1–5 are deliberately built and
//! tested entirely on fixtures.
//!
//! ## The load-bearing honesty invariants
//!
//! 1. **Quarantine, never silently drop.** Every candidate Finding that fails grounding
//!    or that the caller's SHACL gate rejects is recorded in the `pipeline::Sidecar`
//!    (`quarantined`), with its reason — never dropped. This is the sidecar-honesty
//!    pattern (`.quarantine`) the design (§4.7) requires.
//! 2. **Propose-then-verify (the deterministic grounding-resolver, §4.3).** A cheap model
//!    hallucinates. So `ground` requires every committed Finding's `justification` to be
//!    an **entailed span of the source abstract** AND every cited DOI to **resolve to a
//!    `pkg:Source` actually in the batch**. A candidate that fails either check is
//!    quarantined, not committed. The SHACL gate checks *structure*; grounding checks that
//!    the claim is *anchored in the text* — neither checks *truth*, and the docs say so.
//! 3. **The machine tier never outranks a human.** Every emitted Finding is
//!    `prov:wasAttributedTo` a `pipeline::MACHINE_AGENT_IRI` (`pkg:MachineAgent`) and
//!    carries `secx:Conjectured` with a bounded confidence. `shapes/literature.shapes.ttl`
//!    enforces this declaratively at the write-gate (a machine Finding may never stamp
//!    `secx:Proven`).
//!
//! ## The metric (Phase-5 acceptance bar)
//!
//! On a frozen fixture batch the pipeline reports a `pipeline::Sidecar` carrying the
//! **citation-grounding rate** (candidates whose grounding passed / total candidates) and
//! the inputs for the **SHACL-conformance rate** (the caller runs the SHACL gate over the
//! emitted TTL with `shapes/literature.shapes.ttl` and records the conformance). No
//! hard-coded performance number lives here: the rates are *computed at run time* from the
//! fixture batch.

#[cfg(feature = "literature")]
pub mod connector;
/// The live CORE API v3 connector (`parse_core_batch` + the `literature-live` HTTP client).
/// Pure parse + retry discipline is available under `literature`; the networked socket layer
/// (`CoreClient`) is behind `literature-live`. sq-tzars.1
#[cfg(feature = "literature")]
pub mod connector_core;
#[cfg(feature = "literature")]
pub mod extract;
/// The LIVE cheap-model batch extractor behind the `extract::Extractor` trait
/// (`extract_live::LiveExtractor` + the JSON-only prompt / transcript parse / defensive
/// machine-tier caps). The pure prompt + parse + cap logic is available under `literature`;
/// the subprocess seam that actually invokes a sub-agent (`extract_live::CommandRunner`,
/// default-unset + configurable) is behind `literature-live`. Never driven in CI —
/// record/replay stays the only test path. sq-tzars.6
#[cfg(feature = "literature")]
pub mod extract_live;
#[cfg(feature = "literature")]
pub mod ground;
/// The hard-capped, dry-run-first pilot ITERATION LOOP (`sq-tzars.9`): pre-registered
/// audit bar (written before extraction, enforced by type-state), append-only sidecar
/// chain, fail-stop caps, and the maintainer-armed live-emit gate (`live_emit_allowed`).
/// Pure — the networked wiring is the `literature-pilot` binary (`literature-live`).
/// sq-tzars.9
#[cfg(feature = "literature")]
pub mod pilot;
#[cfg(feature = "literature")]
pub mod pipeline;

/// The committed OpenAlex-shaped connector fixture (a FABRICATED, illustrative batch —
/// not a captured live response). The pipeline's input under test.
#[cfg(feature = "literature")]
pub const FIXTURE_OPENALEX_BATCH: &str = include_str!("../fixtures/literature/openalex-batch.json");

/// The committed extraction tape replayed by [`extract::RecordedExtractor`] (the frozen
/// output a cheap-model extractor WOULD have produced — replayed so CI makes no live
/// model call).
#[cfg(feature = "literature")]
pub const FIXTURE_EXTRACTIONS: &str = include_str!("../fixtures/literature/extractions.json");

/// The committed CORE API v3 connector fixture — a REAL, SANITIZED `/v3/search/works`
/// response recorded once locally and scrubbed (no key, no copyrighted full-text; see the
/// file's `_comment`). The CORE-path tests parse this so CI replays real data with ZERO
/// network. Consumed by `connector_core::parse_core_batch`.
#[cfg(feature = "literature")]
pub const FIXTURE_CORE_BATCH: &str = include_str!("../fixtures/literature/core-batch.json");

/// The literature-tier SHACL guardrails (`shapes/literature.shapes.ttl`) — the extra
/// write-gate constraints on the machine-extraction tier (assurance ≠ `secx:Proven`,
/// confidence ceiling, no dangling `cito:citesAsEvidence`). Always available as data;
/// the validator that runs them is behind `validate`.
pub const LITERATURE_SHAPES: &str = include_str!("../shapes/literature.shapes.ttl");
