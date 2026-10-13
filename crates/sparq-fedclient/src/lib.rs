// ─── Crate-level documentation ──────────────────────────────────────────────────────
// [OPUS-4.8] sq-tiq4: the narrative below intra-doc-links the per-phase modules
// (`source`/`discovery`/`planner`/`pushdown`/`operators`/`stream`/`adaptive`) and the
// re-exported items — but EVERY one of those is `#[cfg(feature = "fedclient")]`-gated, so in
// the DEFAULT (fedclient OFF) build none of them are in scope and `cargo doc` under
// `-D rustdoc::broken_intra_doc_links` failed on ~40 unresolved links. The fix keeps the rich
// linked narrative for the build where its targets actually exist (`fedclient` ON) and serves
// a concise, link-free crate doc for the OFF build, so `cargo doc` is clean in BOTH states.
// `deny(rustdoc::broken_intra_doc_links)` below locks the invariant in (rustdoc-only lint — it
// fires under `cargo doc`, never under `cargo build`/`clippy`, so the existing gates are
// unchanged). Mirror this split in any future crate-doc edit.
#![cfg_attr(
    feature = "fedclient",
    doc = r#"sparq-fedclient: a **streaming federation CLIENT** over heterogeneous remote RDF
sources — the query *consumer* half of federation (epic **sq-dnko** / **sq-3183**,
Phase 0 bead **sq-s1uy**). See `research/federation-client-design.md` — the crate's
architecture-of-record (§4 architecture, §7 honest risks).

Given one SPARQL query and a set of heterogeneous remote sources — full SPARQL
endpoints, bindings-restricted (brTPF) servers, plain TPF servers, and the *local*
sparq engine — this crate (when complete) **discovers** each source's capability,
**plans** a federated execution that pushes the most precise sub-query each source
can answer (REUSING the [`sparq-fedplan`](sparq_fedplan) cost-based planner), and
**streams** results back through non-blocking federation operators.

# What has landed, and what is still ahead

The crate began as the **Phase-0 compiling skeleton** (design §6) — the public module
layout the design §4 names ([`source`], [`discovery`], [`planner`], [`pushdown`],
[`operators`], [`stream`]), the **opt-in feature** ([`fedclient`](#opt-in-hard-constraint),
OFF by default), and the **dependency-boundary proof** (below). Landed since, each behind
the same default-OFF `fedclient` feature (with live pattern probing behind the additional
default-OFF `pattern_probe` feature):

* **Phase 1 — capability discovery** ([`discovery`], bead `sq-nfxl`): GET the source's
  Service Description + `/.well-known/void`, parse them to a `Capability` +
  `SourceDescriptor`, with a FedX-style ASK-probe fallback, all behind a default-deny
  SSRF-guarded fetch seam. The extra default-OFF `pattern_probe` feature (bead `sq-fx5id`)
  adds bounded, per-query pattern ASK/capped-SELECT observations through that same fetch seam;
  only definitive ASK `false` prunes, while every failure keeps the source.
* **Phase 2 — source-type abstraction + Endpoint adapter** ([`source`], bead `sq-rsxf`):
  [`SourceType`] (`Endpoint | BrTpf | Tpf | Local`), the [`FederatedSource`] trait, the
  fine-grained [`Capability`](source::Capability) descriptor, and the real [`Endpoint`] SRJ
  adapter over a [`Transport`] seam behind a default-deny [`EgressGuard`].
* **Phase 3 — planner bridge + materialised single-source interpreter** ([`planner`] +
  [`operators`], bead `sq-j27p`): [`SourceResolver`] index→adapter resolution + [`lower_leaf`],
  and [`materialize_single_source`] + [`parse_srj`] + [`solutions_equal`].
* **Phase 4 — capability-aware pushdown** ([`pushdown`], bead `sq-7byx`): per leaf / FedX
  exclusive group, build the most precise sub-query a source can answer (projection +
  common-variable-checked filters + ORDER/LIMIT under capability) — [`exclusive_groups`] +
  [`push_group`] + [`render_values_block`], correctness-preservingly NARROWing each source.
* **Phase 5 — streaming operators** ([`operators`] + [`stream`], bead `sq-vtba`): the
  [`SolutionStream`] boundary the client owns, the bounded blocking [`ScatterPool`], the
  `StreamJoin`-feeder [`StreamingJoin`], and the [`stream_single_source`] streaming
  interpreter — backpressured + bounded (Rust ownership + the reused StreamJoin spill).
* **Phase 6 — brTPF + TPF fragment adapters** ([`source`], bead `sq-2qze`): the real
  Triple-Pattern-Fragments adapters ([`TpfSource`] / [`BrTpfSource`]) over a
  [`FragmentTransport`] seam, complete-by-construction with count-metadata cardinality. The
  [`wire`] module (bead `sq-6ihg`) adds the brTPF binding-block transport encodings — a COMPACT
  self-describing BINARY mapping wire ([`encode_bindings`] / [`decode_bindings`]) alongside the
  line-oriented TEXT wire the server parses ([`encode_bindings_text`]), so a large bind-join
  block travels compactly and losslessly and the client can speak either form. The **native
  HTTP** [`FragmentTransport`](source::FragmentTransport) ([`HttpFragmentTransport`], bead
  `sq-yzca`) is the production seam — ureq + the same default-deny SSRF resolver, Hydra
  URI-template serialisation (`?subject=&predicate=&object=` + the brTPF `values` block),
  `hydra:next` pagination, and a Turtle/TriG fragment-body parse that splits the Hydra/VoID
  control triples from the data triples — and the [`operators`] interpreter is wired to answer
  a `JoinTree` leaf resolving to a TPF/brTPF source through its typed `solutions` path (via
  [`lower_leaf_fragment`]) rather than the SRJ `execute` a fragment server refuses.

* **Phase 7 — adaptive re-planning** (the `adaptive` module, bead `sq-ij5x`, the FINAL
  phase): the opt-in feedback loop that re-invokes [`sparq-fedplan`](sparq_fedplan)'s planner
  on the **unjoined remainder** when observed cardinality diverges from the static estimate,
  bounded to at most once per operator boundary — re-planning changes the plan, never the
  answer. Behind the extra default-OFF `fedclient-adaptive` feature (it pulls
  `sparq-fedplan/adaptive-replan`); when that feature is off the `adaptive` module and its
  `execute_adaptive_single_source` / `execute_adaptive_multi_source` entry points are
  `#[cfg]`-compiled out. The multi-source loop ([FABLE-5] sq-xw8zz) lifts the
  `MultiSource` guard — union-arm leaves with LIVE per-arm latency observations feeding
  the planner's per-source EWMA.

With Phase 7 the **8-phase streaming federation client is feature-complete** (Phases 0–7
all landed; epic sq-dnko closed). The public surface and the dependency boundary were made
visible before the logic landed.

# Opt-in (hard constraint)

The whole client is behind the **`fedclient` cargo feature, OFF by default**, and the
crate is a standalone workspace member with `publish = false`. `sparq-core` and
`sparq-engine` **never** depend on it — the dependency arrow points one-way *into*
the engine (the client reuses the engine's SERVICE transport + SSRF guard + local
eval), so the default engine build and the WASM artifact are byte-identical with or
without `sparq-fedclient`. A build that does not enable `fedclient` compiles an empty
crate (mirrors [`sparq-fedplan`](sparq_fedplan)'s `fedplan` feature).

# The dependency boundary (load-bearing, enforced)

The boundary was **proved before any logic existed** (Phase 0) and is re-checked on every
build. Two complementary checks enforce that neither `sparq-core` nor `sparq-engine` ever
gains a dependency edge *to* `sparq-fedclient`, in both feature states:

* `scripts/fedclient-boundary-guard.sh` — a CI step (in `feature-matrix.yml`) that
  inverts the dependency graph (`cargo tree -i sparq-fedclient`) and fails if
  `sparq-core` or `sparq-engine` appears as a dependent;
* `tests/boundary.rs` — a hermetic `cargo metadata` test asserting the same invariant
  from inside the test suite, so it gates even off the CI script.

Both must FAIL if a future edit introduces such an edge.

[OPUS-4.8] sq-s1uy / sq-73om / sq-tiq4 — flagged for Fable re-review when available."#
)]
// Default (fedclient OFF) build: a concise, link-free crate doc. None of the per-phase
// modules/items linked above exist in this build, so the doc is plain text + code spans
// (matching `README.md`). [OPUS-4.8] sq-tiq4.
#![cfg_attr(
    not(feature = "fedclient"),
    doc = r#"sparq-fedclient: a **streaming federation CLIENT** over heterogeneous remote RDF
sources — the query *consumer* half of federation (epic **sq-dnko** / **sq-3183**). See
`research/federation-client-design.md` — the crate's architecture-of-record.

**This is the default build with the `fedclient` feature OFF, so the crate is empty** — the
whole client (the `source` / `discovery` / `planner` / `pushdown` / `operators` / `stream` /
`adaptive` modules and every re-exported item) is gated behind the **`fedclient` cargo
feature, OFF by default**. Build the docs with `--features fedclient` to see the federation
API and the per-phase narrative; this crate is a standalone workspace member with
`publish = false`, and `sparq-core` / `sparq-engine` **never** depend on it (the dependency
arrow points one-way *into* the engine), so the default engine build and the WASM artifact
are byte-identical with or without `sparq-fedclient` (mirrors `sparq-fedplan`'s `fedplan`
feature).

[OPUS-4.8] sq-s1uy / sq-73om / sq-tiq4 — flagged for Fable re-review when available."#
)]
#![forbid(unsafe_code)]
// [OPUS-4.8] sq-tiq4: lock the fix in. `rustdoc::broken_intra_doc_links` is a rustdoc-only
// lint — it fires under `cargo doc`, NOT under `cargo build`/`clippy`/`test` — so denying it
// keeps the existing build/clippy/test gates untouched while making a future broken crate-doc
// link a hard `cargo doc` error in EITHER feature state.
#![deny(rustdoc::broken_intra_doc_links)]
// [OPUS-4.8] sq-s1uy: crate has zero `unsafe`.
// When `fedclient` is off the crate is intentionally empty, so its items are unused.
#![cfg_attr(not(feature = "fedclient"), allow(dead_code, unused_imports))]

// ─── Public module layout (design §4) ──────────────────────────────────────────────
// Every module (source, discovery, planner, pushdown, operators, stream) now carries real
// logic. The doc comment on each records WHICH design section it realises and WHICH existing
// sparq seam that phase reuses, so the layout is self-documenting. All are gated behind
// `fedclient`.

/// §4.1 — **source-type abstraction**: the `SourceType` enum (Endpoint | BrTpf | Tpf |
/// Local) and the `FederatedSource` trait (`discover()` + `execute(&SubQuery) ->
/// SolutionStream`), the sparq analogue of Comunica's `IQuerySource`. Phase 2 wires the
/// Endpoint adapter over the engine's existing SRJ transport; Phase 6 adds brTPF/TPF.
#[cfg(feature = "fedclient")]
pub mod source;
// [OPUS-4.8] sq-g2xs: shared ureq-3 SSRF-resolver helpers (URI→host:port, capacity-bounded
// filtered resolve, refusal error) used by every native federation transport. Native-only —
// only built when `fedclient` pulls ureq in.
#[cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]
pub(crate) mod ureq_egress;
// [OPUS-4.8] sq-rsxf: re-export the Phase-2 §4.1 source surface at the crate root.
#[cfg(feature = "fedclient")]
pub use source::{
    is_forbidden_ip, BindJoin, BrTpfSource, Capability, EgressGuard, Endpoint, FedError,
    FederatedSource, FilterClass, Interface, LocalSource, SourceType, SubQuery, TpfSource,
    Transport,
};
// [OPUS-4.8] sq-2qze: re-export the Phase-6 brTPF/TPF fragment surface at the crate root.
#[cfg(feature = "fedclient")]
pub use source::{
    FragBinding, FragPattern, FragTerm, FragTriple, FragmentPage, FragmentTransport, PatternTerm,
};
// [OPUS-4.8] sq-25xk: re-export the native IP-pinning HTTP transport for the Endpoint adapter.
// Native-only — it wraps ureq (gated out of the wasm bundle) and installs an SSRF resolver so
// the guard-vetted IP IS the dialled IP (no DNS-rebinding re-resolve window).
#[cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]
pub use source::HttpTransport;
// [OPUS-4.8] sq-yzca: re-export the native HTTP FragmentTransport for the TPF/brTPF adapters.
// Native-only — ureq + the SAME default-deny SSRF resolver, Hydra URI-template serialisation
// (`?subject=&predicate=&object=` + the brTPF `values` block) and Turtle/TriG fragment-body
// parse (control/data split, `hydra:totalItems`/`void:triples` count, `hydra:next` pagination).
#[cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]
pub use source::HttpFragmentTransport;

/// brTPF binding-block **transport encodings** (bead `sq-6ihg`, follow-up to the server's
/// `sq-dxhb`): a COMPACT, self-describing BINARY mapping wire ([`wire::encode_bindings`] /
/// [`wire::decode_bindings`]) alongside the line-oriented TEXT wire the server already parses
/// ([`wire::encode_bindings_text`]). The binary form drops the per-pair position key and the
/// N-Triples framing (a one-byte kind tag + length-prefixed bare lexical bytes) so a large
/// [`FragBinding`] block travels compactly and losslessly; the text form matches the server's
/// `position=term` grammar so a client can speak either over the same `FragBinding` model. A
/// codec only — the native HTTP [`FragmentTransport`](source::FragmentTransport)
/// ([`HttpFragmentTransport`], bead `sq-yzca`) attaches the TEXT form on the `values` query
/// parameter; the binary wire here is the alternative a body-carrying transport emits.
#[cfg(feature = "fedclient")]
pub mod wire;
// [OPUS-4.8] sq-6ihg: re-export the brTPF binding-block wire codec at the crate root.
#[cfg(feature = "fedclient")]
pub use wire::{
    decode_bindings, encode_bindings, encode_bindings_text, WireError, BINARY_MAGIC, BINARY_VERSION,
};

/// §4.1 — **capability discovery** (Phase 1, bead sq-nfxl): GET `/.well-known/void` + the
/// Service Description per endpoint, parse VoID+`scs:` via the existing
/// [`SourceDescriptor::from_void_nt`](sparq_fedplan::SourceDescriptor) seam (REUSED), parse
/// SD into a [`Capability`](discovery::Capability) (the one genuinely-new client-side parser),
/// with a FedX-style ASK-probe fallback when nothing is published. Every fetch is behind an
/// SSRF-guarded [`Fetcher`](discovery::Fetcher) seam (default-deny private/internal IPs).
#[cfg(feature = "fedclient")]
pub mod discovery;
// [GPT-5.6] sq-fx5id: the live pattern-probe surface is separately default-OFF even though it
// reuses the always-fedclient discovery module and Fetcher policy seam.
#[cfg(feature = "pattern_probe")]
pub use discovery::{
    PatternProbeConfig, PatternProbeOutcome, PatternProbeSession, PatternProbeStats,
};

/// §4.2 — **planner bridge**: lower the parsed query's BGP into `sparq-fedplan`'s light
/// `Bgp`/`TriplePattern`/`Term`/`Var`, build one `SourceDescriptor` per discovered source,
/// call [`select_sources`](sparq_fedplan::select_sources) +
/// [`plan_bgp`](sparq_fedplan::plan_bgp), and resolve plan pattern/source *indices* to
/// patterns / endpoint adapters (Phase 3). The client does NOT write a new planner.
#[cfg(feature = "fedclient")]
pub mod planner;
// [OPUS-4.8] sq-j27p: re-export the Phase-3 planner-bridge surface at the crate root.
// [OPUS-4.8] sq-yzca: `lower_leaf_fragment` lowers a leaf to a `FragPattern` for a TPF/brTPF
// source (the fragment-source twin of `lower_leaf`'s SPARQL `SubQuery`).
#[cfg(feature = "fedclient")]
pub use planner::{lower_leaf, lower_leaf_fragment, pattern_vars, ResolveError, SourceResolver};
#[cfg(feature = "pattern_probe")]
pub use planner::{select_sources_with_pattern_probes, ProbeSource};

/// §4.3 — **capability-aware pushdown**: per leaf / FedX exclusive group, build the
/// MOST PRECISE sub-query a source can answer (projection + common-variable-checked
/// filters + ORDER/LIMIT when the capability covers them), reusing
/// `render_values_block` for VALUES bind-join. Anything a source cannot evaluate is kept
/// local (Phase 4). Pushdown only ever NARROWS a source's result — correctness-preserving.
#[cfg(feature = "fedclient")]
pub mod pushdown;
// [OPUS-4.8] sq-7byx: re-export the Phase-4 capability-aware pushdown surface at the crate root.
#[cfg(feature = "fedclient")]
pub use pushdown::{
    bind_block_size, common_variable_check, exclusive_groups, group_vars, push_group,
    render_values_block, ExclusiveGroup, Filter, PushedGroup, DEFAULT_BIND_BLOCK,
};

/// §4.4 — **physical federation operators**: interpret a `JoinTree` into operators —
/// `Bind` → VALUES/brTPF bind-join, `Hash`/`Streaming` → the reused
/// [`StreamJoin`](sparq_fedplan::StreamJoin), `Local` → `sparq-engine` local eval — over
/// the async `SolutionStream` boundary (the Phase-5 [`stream`] item, not yet defined),
/// with concurrent fan-out (Phase 3/5).
#[cfg(feature = "fedclient")]
pub mod operators;
// [OPUS-4.8] sq-j27p: re-export the Phase-3 materialised single-source interpreter surface.
#[cfg(feature = "fedclient")]
pub use operators::{materialize_single_source, parse_srj, solutions_equal, InterpError, Relation};
// [OPUS-4.8] sq-vtba: re-export the Phase-5 streaming-operator surface — the bounded blocking
// thread-pool, the `StreamJoin`-feeder streaming join, and the streaming interpreter.
#[cfg(feature = "fedclient")]
pub use operators::{stream_single_source, ScatterPool, StreamOptions, StreamingJoin};
// [OPUS-4.8] sq-7yf0: re-export the multi-source UNION-per-leaf interpreters — a leaf the
// planner retained >1 source for is fanned out as a per-source bag-union rather than rejected
// with `InterpError::MultiSource` (the materialised + streaming counterparts of the
// single-source entry points above).
#[cfg(feature = "fedclient")]
pub use operators::{materialize_multi_source, stream_multi_source};

/// §4.4 — the **`SolutionStream`** abstraction the client owns at its boundary (the
/// engine stays materialised, §7). Backpressured + bounded (Rust ownership + explicit
/// buffer bounds + the reused StreamJoin spill — no GC heap). Adapters yield it;
/// operators consume and produce it (Phase 5).
#[cfg(feature = "fedclient")]
pub mod stream;
// [OPUS-4.8] sq-vtba: re-export the Phase-5 `SolutionStream` boundary surface at the crate root.
#[cfg(feature = "fedclient")]
pub use stream::{Solution, SolutionSink, SolutionStream, StreamItem};

/// §4.5 — **adaptive re-planning** (Phase 7, bead sq-ij5x, the FINAL phase of epic sq-dnko):
/// the opt-in ANAPSID feedback loop. As streaming execution proceeds, each completed operator
/// reports its OBSERVED leaf cardinality; when that diverges materially from the static
/// estimate, the client re-invokes `sparq-fedplan`'s planner (its `AdaptiveExecutor`, which
/// owns the divergence trigger + hysteresis + soundness proof) on the UNJOINED REMAINDER to
/// pick a cheaper suffix order — bounded to at most ONCE per operator boundary (no thrashing).
/// Re-planning changes the plan, never the answer (result-multiset-preserving). Behind the
/// extra default-OFF `fedclient-adaptive` feature (which pulls `sparq-fedplan/adaptive-replan`).
/// Two entry points: single-source (the `MultiSource`-guarded loop) and multi-source
/// ([FABLE-5] sq-xw8zz) — the latter answers a leaf as the bag-union of its retained arms
/// and feeds LIVE per-arm fetch latencies into the planner's per-source EWMA, making the
/// latency-aware cost bias (incl. the opt-in `LatencyAggregation::CardinalityWeighted`,
/// sq-s5kd) consumable from a real execution.
#[cfg(feature = "fedclient-adaptive")]
pub mod adaptive;
// [OPUS-4.8] sq-ij5x: re-export the Phase-7 adaptive surface at the crate root.
// [FABLE-5] sq-xw8zz: + the multi-source (union-arm) entry point.
#[cfg(feature = "fedclient-adaptive")]
pub use adaptive::{
    execute_adaptive_multi_source, execute_adaptive_single_source, static_plan, AdaptiveOutcome,
    ReplanEvent,
};
