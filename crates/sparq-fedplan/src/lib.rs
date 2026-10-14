// ─── Crate-level documentation ──────────────────────────────────────────────────────
// sq-gxx7 (sibling of sq-tiq4 / PR #470): the narrative below intra-doc-links
// items that are EVERY one `#[cfg(feature = "fedplan")]`-gated — the re-exports
// (`SourceDescriptor`/`select_sources`/`plan_bgp`/`JoinTree`/`StreamJoin`/…) and the private
// `selection`/`plan`/`stream` modules. In the DEFAULT (fedplan OFF) build none of them are in
// scope, so `cargo doc` under `-D rustdoc::broken_intra_doc_links` failed on 13 unresolved
// links. The fix mirrors sq-tiq4: keep the rich linked narrative only for the build where its
// targets exist (`fedplan` ON) and serve a concise, link-free crate doc for the OFF build, so
// `cargo doc` is clean in BOTH states. In the ON narrative the links point at PUBLIC re-exported
// items (never the private `selection`/`plan`/`stream` modules), so the ON doc is also clean
// under the deny (no `private_intra_doc_links` warning). `deny(rustdoc::broken_intra_doc_links)`
// below locks the invariant in (rustdoc-only lint — it fires under `cargo doc`, never under
// `cargo build`/`clippy`/`test`, so the existing gates are unchanged). Mirror this split in any
// future crate-doc edit.
#![cfg_attr(
    feature = "fedplan",
    doc = r#"sparq-fedplan: **cost-based federated source selection + bind-vs-hash join
planning** over already-fetched source *descriptors* — the first opt-in slice of
cost-based federation (bead sq-a35t, epic sq-3183).

This crate plans a federated SPARQL Basic Graph Pattern (BGP) across a set of
remote SPARQL endpoints whose **statistics are already in hand** — each source is
described by a [`SourceDescriptor`] carrying the W3C VoID property/class partitions
(`void:propertyPartition` / `void:classPartition`) that a `sparq-server` already
serves at `/.well-known/void`, *plus* the mined **characteristic sets** the server
also serves under the `scs:` vocab (`http://sparq.dev/ns/cs#`). The planner is:

* **Pure** — it consumes descriptors, never the network. No I/O, no live `Graph`.
  A caller fetches descriptors once (or builds them programmatically /
  [`SourceDescriptor::from_void_nt`] from the served N-Triples) and hands them in.
* **Deterministic** — same descriptors + same BGP ⇒ same plan, every time. Every
  internal ordering breaks ties on a stable key (source id, then pattern index).

## What it produces (this slice)

1. [`select_sources`] — **source selection**: for each triple pattern, which
   sources *can* contribute, with a per-(pattern, source) cardinality estimate.
   Two complementary techniques:
   * **HiBISCuS-style authority/prefix pruning** — a source is
     pruned for a pattern only when its capability set (the IRI authorities /
     predicates / classes it is known to hold) makes a contribution *impossible*.
     This is **recall-safe**: a source is dropped only on positive evidence of
     non-contribution; *any* uncertainty keeps it. See [`select_sources`] for the
     invariant and its proof obligations (tested in the `selection::tests` module).
   * **CostFed-style skew-aware cardinality** — the per-pattern, per-source
     cardinality is estimated from the served per-predicate stats (triples, average
     multiplicity) carried in the characteristic sets, *not* a uniform-distribution
     guess; predicate skew is preserved.
2. [`plan_bgp`] — a **bind-vs-hash join planner**: a greedy join-order
   heuristic over the selected sources that, for each join, chooses a **bind join**
   (probe the right side with the left side's bindings — cheap when the left is
   small and the bound variable is selective) or a **hash/symmetric join** (scan
   both sides once — cheap when both sides are large), using characteristic-set
   star-join cardinality to estimate intermediate-result sizes. The output is a
   [`JoinTree`] with per-node estimated cardinality and cost.

## Opt-in (hard constraint)

The whole planner is behind the **`fedplan` cargo feature, OFF by default**, and the
crate is a standalone workspace member published on crates.io — `sparq-core` /
`sparq-engine` never depend on it, so the default engine and the WASM artifact are
byte-identical with or without it. A build that does not enable `fedplan` compiles
an empty crate.

## Streaming execution (sq-vf7q)

3. [`StreamJoin`] — an **ANAPSID-style non-blocking streaming join with bounded
   operator spill**: the execution-side operator the planner's
   [`JoinAlgo::Streaming`] choice corresponds to. It consumes two incrementally-arriving
   tuple streams (federated sub-results returning at different rates), builds *and*
   probes both sides, and emits matches without first materialising either full input.
   Memory is bounded by [`StreamJoinOptions::mem_budget_tuples`]: over-budget join-key
   partitions spill to a backing store (a temp file by default — std only) and are
   reconciled on probe. The load-bearing invariant — proven by tests — is that the
   streamed + spilled result is **multiset-equal** to the equivalent blocking hash join
   ([`blocking_hash_join`]) for any interleaving and any budget.

## Live adaptive re-planning (sq-7s4z) — opt-in

4. `AdaptiveExecutor` — **mid-execution plan switching** behind the separate
   `adaptive-replan` cargo feature (OFF by default; implies `fedplan`). It captures the
   *observed* runtime statistics (`RuntimeStats`: real per-pattern cardinality +
   per-source latency) as execution proceeds, and at **stage boundaries** — between two
   leaf joins of the left-deep plan — re-invokes the planner on the **remaining** (not-
   yet-executed) patterns when an observation diverges from its estimate past a policy
   factor (`ReplanPolicy::divergence_factor`), adopting the new suffix order only when
   it wins by a hysteresis margin (`ReplanPolicy::improvement_margin`) so stable-but-
   noisy stats never thrash. **Soundness boundary**: re-planning reorders only the
   not-yet-started suffix (never a mid-operator swap); because BGP join is
   commutative/associative, any order over the same patterns yields the *same* result
   multiset — proven by `adaptive::tests::replan_result_equals_static` against an
   order-driven reference executor. Mid-operator adaptivity + live source failover are
   deferred (beaded under epic sq-3183).

[OPUS-4.8] sq-a35t / sq-vf7q / sq-7s4z / sq-gxx7 — flagged for Fable re-review."#
)]
// Default (fedplan OFF) build: a concise, link-free crate doc. None of the per-phase modules /
// re-exported items linked above exist in this build, so the doc is plain text + code spans
// (matching `README.md`). sq-gxx7.
#![cfg_attr(
    not(feature = "fedplan"),
    doc = r#"sparq-fedplan: **cost-based federated source selection + bind-vs-hash join
planning** over already-fetched source *descriptors* — the first opt-in slice of
cost-based federation (bead sq-a35t, epic sq-3183). See `README.md` and
`skills/federated-planning/SKILL.md` for the full design.

**This is the default build with the `fedplan` feature OFF, so the crate is empty** — the
whole planner (source selection, the bind-vs-hash join planner, the `StreamJoin` streaming
operator, and the opt-in `AdaptiveExecutor`) and every re-exported item is gated behind the
**`fedplan` cargo feature, OFF by default**. Build the docs with `--features fedplan` to see the
planner API and the per-phase narrative; this crate is a standalone workspace member published
on crates.io, and `sparq-core` / `sparq-engine` **never** depend on it, so the default
engine build and the WASM artifact are byte-identical with or without `sparq-fedplan`. Live
adaptive re-planning (`AdaptiveExecutor`) is behind the further `adaptive-replan` feature
(which implies `fedplan`).

[OPUS-4.8] sq-a35t / sq-vf7q / sq-7s4z / sq-gxx7 — flagged for Fable re-review."#
)]
#![forbid(unsafe_code)]
// sq-a35t: crate has zero `unsafe`.
// sq-gxx7: lock the crate-doc fix in. `rustdoc::broken_intra_doc_links` is a
// rustdoc-only lint — it fires under `cargo doc`, NOT under `cargo build`/`clippy`/`test` — so
// denying it keeps the existing build/clippy/test gates untouched while making a future broken
// crate-doc link a hard `cargo doc` error in EITHER feature state.
#![deny(rustdoc::broken_intra_doc_links)]
// sq-qik4: also lock in the *private*-link invariant. This is a SEPARATE rustdoc lint
// from `broken_intra_doc_links` (#474) — it fires when a PUBLIC item's doc links to a private
// item. `from_void_nt`'s doc links to `SourceDescriptorBuilder::authorities_complete`; once the
// builder is re-exported (above), that target is public, so this deny is clean. Denying it makes a
// future public→private doc link a hard `cargo doc` error rather than a silently-residual warning.
#![deny(rustdoc::private_intra_doc_links)]
#![cfg_attr(not(feature = "fedplan"), allow(dead_code, unused_imports))]

// sq-7s4z: live adaptive mid-execution re-planning. Gated behind the
// `adaptive-replan` feature (which implies `fedplan`); compiled out entirely otherwise.
#[cfg(feature = "adaptive-replan")]
mod adaptive;
#[cfg(feature = "fedplan")]
mod descriptor;
#[cfg(feature = "fedplan")]
mod pattern;
#[cfg(feature = "fedplan")]
mod plan;
#[cfg(feature = "fedplan")]
mod selection;
// sq-vf7q: the execution-side non-blocking streaming join + spill operator.
#[cfg(feature = "fedplan")]
mod stream;

// sq-qik4: re-export `SourceDescriptorBuilder` too. It is the return type of the
// PUBLIC `SourceDescriptor::builder` and the link target of `from_void_nt`'s doc, so leaving it
// un-exported (a) leaked a public method returning an unnameable type and (b) tripped the residual
// `rustdoc::private_intra_doc_links` warning (the link resolved to a `pub` item reachable only
// through the private `descriptor` module). Re-exporting it closes both: the link now points at a
// genuinely public item. Separate from the #474 `broken_intra_doc_links` gate (a method-doc lint).
#[cfg(feature = "fedplan")]
pub use descriptor::{
    CharSet, ClassPartition, PredPartition, RetrievalCapability, SourceDescriptor,
    SourceDescriptorBuilder, SourceId, CS_NS, RETRIEVAL_NS, VOID_NS,
};
#[cfg(feature = "fedplan")]
pub use pattern::{Bgp, Term, TriplePattern, Var};
#[cfg(feature = "fedplan")]
pub use plan::{plan_bgp, JoinAlgo, JoinNode, JoinTree, PlanOptions};
#[cfg(feature = "fedplan")]
pub use selection::{select_sources, PatternSources, SourceCandidate};
// sq-vf7q.
#[cfg(feature = "fedplan")]
pub use stream::{
    blocking_hash_join, run_streaming, SpillStore, StreamJoin, StreamJoinOptions, Tuple,
};
// sq-7s4z: live adaptive mid-execution re-planning surface.
// sq-s5kd: + LatencyAggregation (per-source vs slowest-arm latency in cost).
#[cfg(feature = "adaptive-replan")]
pub use adaptive::{
    exec_oracle, AdaptiveExecutor, LatencyAggregation, ReplanOutcome, ReplanPolicy, RuntimeStats,
};
