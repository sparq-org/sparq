// ─── Crate-level documentation ──────────────────────────────────────────────────────
// [OPUS-4.8] sq-2q1x: the crate doc is split per feature state (mirroring sparq-fedplan's
// sq-gxx7 fix) so `cargo doc` is clean under `-D rustdoc::broken_intra_doc_links` in BOTH
// states: the ON narrative intra-doc-links the gated public items (which exist only when
// the feature is on); the default (feature OFF) build serves a concise, link-free doc.
#![cfg_attr(
    feature = "fedplan-mpc",
    doc = r#"sparq-fedplan-mpc: the opt-in **glue** between cost-based federated source
selection (`sparq-fedplan`) and MPC-over-federated-SPARQL routing (`sparq-mpc`) — the
untrusted-planner → MPC-routing seam (beads sq-2q1x + sq-fix4 + sq-i1wh2 + sq-pwr.2, epics
sq-pwr / sq-0jsc; `research/mpc-untrusted-planner-routing-design.md`).

# What is delivered so far

Phase 1 (sq-2q1x) landed the **seam scaffold**; Phase 2 (sq-fix4) the **source-selection
adapter**; Phase 3 (sq-i1wh2) the **disclosed/hidden routing pass**; Phase 4 (sq-pwr.2) the
**leakage-envelope assembly + dual ratification**; Phase 6 (sq-pwr.3 + sq-xkrt) the **result-aware
source-combination prune**; Phase 5 (sq-1fo4) the **canonicalisation half only** of the
untrusted-plan binding. Phase 5's **soundness** half — the step that would actually discharge
constraint C-A — and the authenticated-input attestation stay deferred and audit-gated.

* [`SourcePrivacyDescriptor`] — the per-source privacy declaration the routing pass reads:
  which predicates a source is **willing to disclose in the clear**, the opaque attestation-key
  id its graph is signed under, and a reserved participation/authorisation field. Its posture is
  **default-DENY**: a predicate is treated as PRIVATE unless the source explicitly marks it
  disclosable. See [`SourcePrivacyDescriptor::may_disclose`] for the load-bearing invariant.
* [`select_private_sources`] (**Phase 2, implemented**) — wraps
  [`sparq_fedplan::select_sources`] and **prunes** any source that has declared it will not
  participate (the descriptor's authorisation field), surfacing the retained per-pattern
  candidate set plus an audit trail of what was pruned and why. The **only** prune rule is
  participation/authorisation — a source holding a *private* predicate stays a candidate (its
  in-clear-vs-MPC routing is the Phase-3 decision), so the adapter is recall-safe and loses no
  answers. It is plumbing — **source-selection routing, not a cryptographic guarantee**: it runs
  NO MPC and reveals nothing it was not already given in the clear. See [`selection`].
* [`route_operators`] (**Phase 3, implemented**) — the policy-parameterised disclosed/hidden
  routing pass. Given the Phase-2 selection, the per-source privacy descriptors, and the query's
  [`QueryOperator`]s, it classifies each operator as [`Routing::Disclosed`](sparq_mpc::pipeline::Routing)
  (every operand disclosable in the clear) or [`Routing::Hidden`](sparq_mpc::pipeline::Routing)
  (default-deny otherwise — runs through the MPC path), emitting the
  [`sparq_mpc::pipeline::OperatorRouting`] vector the pipeline today receives hand-written. The
  [`RoutingPolicy`] knob selects the cheap default (disclose global-IRI operands) or the strict
  "hide even a public term" route. It is routing plumbing — **not** a cryptographic guarantee; it
  runs NO MPC and computes only the *proposed* partition. See [`routing`].
* [`assemble_leakage_envelope`] + [`ratify_envelope`] (**Phase 4, implemented**) — the
  leakage-envelope assembly + **dual-ratification** gate. `assemble_leakage_envelope` derives,
  from the Phase-3 [`PrivateRouting`] and the [`QueryOperator`]s it was computed over, a declared
  [`LeakageEnvelope`] that **honestly enumerates** what the plan reveals: the operator structure
  (count, per-operator class + label), the disclose/hide partition, the operands each `Disclosed`
  operator exposes in the clear, and which sources participate — **over-counting, never
  under-counting** the leak. `ratify_envelope` then runs the dual gate: each **holder**
  fail-closed-rejects a plan that would disclose one of its own private predicates (constraint
  C-B), AND the **verifier** rejects an over-leaking envelope (one whose disclosed-operand count
  exceeds its declared budget). It is **leakage-accounting + a plan-time policy gate — not a
  cryptographic enforcement**: it runs NO MPC and makes NO soundness/privacy claim. See
  [`envelope`].
* [`prune_source_combinations`] (**Phase 6, implemented**) — the FedUP-style **result-aware
  source-combination prune** over the Phase-2 selection. A full federated query executes a
  *conjunction* of patterns, and the seam's secure-join cost grows with the number of source
  *combinations* (one source per pattern) it considers. This pass surfaces which combinations
  **provably contribute no answer** so the seam routes fewer toward MPC, under two recall-safe,
  summary-expressible rules. **Rule C1 — unsatisfiable-conjunct collapse**: if any pattern's
  Phase-2 candidate list is empty, that conjunct is a proved-empty relation, so the whole
  conjunction is unsatisfiable and **every** source-combination is dead (`∅ ⋈ R = ∅`).
  **Rule C2 — dead same-source star pairing** (the FedUP quotient-summary rule, the one that
  fires on the *live* path): a source's served characteristic sets are a *quotient summary* that
  partitions its subjects by exact predicate set, so if two patterns share a **subject**-position
  join variable and **no** set of a source carries both their predicates, assigning *both* to
  that source can never yield an answer — that pairing, and every combination containing it, is
  dead, while the source stays a valid candidate for each pattern individually. Rule C2 fires
  only behind a **completeness guard** (`Σ_{C ∋ p} subjects == void:distinctSubjects`), so a
  truncated or unknown summary declines rather than over-prunes. It carries the Phase-2 selection
  through unchanged (advisory + auditable, never silently dropping), reporting `bgp_satisfiable`,
  the empty-pattern witnesses, the Rule-C2 [`DeadPairing`]s (plus
  [`PrunedCombinations::combination_is_dead`] to test one combination), the BGP join components,
  and a combination-dead audit trail. The value-overlap / bound-IRI-propagation prune is
  **declined** as not recall-safely expressible from the public summary (documented in
  [`combination`]). It is selection plumbing — **not** a cryptographic guarantee; runs NO MPC,
  makes NO privacy/soundness claim. See [`combination`].
* [`commit_plan`] + [`revalidate_plan`] (**Phase 5, CANONICALISATION HALF ONLY**, sq-1fo4) — the
  untrusted-plan binding seam. `commit_plan` reduces the produced plan — the Phase-2 per-pattern
  source assignment, the Phase-3 disclosed/hidden route, and the Phase-4 declared participation set
  — to a deterministic, domain-separated [`PlanCommitment`] whose
  [`plan_digest`](PlanCommitment::plan_digest) is a single [`sparq_mpc::FieldWord`], i.e. shaped to
  become a **public input** once a collaborative proof exists to carry it. `revalidate_plan`
  compares a *claimed* commitment against an *independently recomputed* one and **fails closed**,
  reporting a structured [`PlanDivergence`] list that names the dropped source / re-routed operator.
  **This is a canonicality + transcript-integrity check, NOT a soundness mechanism** — a malicious
  planner computes the plan *and* its commitment, so it can always commit to the plan it actually
  ran and pass. It becomes load-bearing only once the digest is bound into a proof, which is
  [`bind_plan_to_proof`] — **deferred**, returning [`SeamError::Deferred`] naming its gates
  (`sparq-mpc::proof` is still `NotYetImplemented`; sq-qhy4 + sq-9hrn pending; the attachment point
  is design-record §9 Q3, still open). See [`binding`].
* [`SeamPhase`] + [`SeamError`] — the shared error/phase types. The [`SeamError::Deferred`] channel
  is returned today by [`bind_plan_to_proof`] (Phase 5's gated soundness half).

# What this crate does NOT do (honest boundary)

It performs **no** MPC, **no** secret-sharing, and runs **no** privacy-bearing cryptographic logic
— the Phase-2 adapter only routes the caller's own (public) descriptors, the Phase-3 routing pass
only computes a *proposed* partition over typed operators, and the Phase-4 envelope+ratification is
a *declaration* of what the plan reveals plus a *plan-time policy check* over it (it opens nothing
and verifies nothing cryptographic). It makes **no** soundness, privacy, or security claim. The
leakage of the routing pass itself (it reveals the query's operator structure and the disclose/hide
partition — **not** operand values or result cardinalities) is enumerated honestly in the Phase-4
[`LeakageEnvelope`]. The dual ratification is a plan-time gate, not a runtime guarantee that a
malicious holder/verifier honours it. The Phase-5 [`PlanCommitment`] is a *canonical encoding* of
plan metadata the envelope already declares — it invokes no cryptography beyond a domain-separated
digest, widens the declared leakage by nothing, and **provides no soundness**: the step that would
(the binding into a collaborative proof) is [`bind_plan_to_proof`], which is deferred. The further
privacy-bearing work (that binding, the authenticated-input attestation) remains **deferred** and
**audit-gated**: the MPC estate is research-grade, honest-majority semi-honest only, and is **not**
externally audited — the external accredited-cryptographer sign-off (sq-qhy4) and the
collaborative-coZK re-audit (sq-9hrn) are pending. Do not present anything here as providing a
privacy or soundness guarantee. See `README.md` and the design record for the full caveat.

# Opt-in (hard constraint)

The whole surface is behind the **`fedplan-mpc` cargo feature, OFF by default**, and the crate
is a standalone `publish = false` workspace member. `sparq-core` / `sparq-engine` never depend
on it, so the default engine build and the WASM artifact are byte-identical with or without
it; a build that does not enable `fedplan-mpc` compiles an empty crate and pulls in neither
`sparq-fedplan` nor `sparq-mpc`. Neither upstream gains a cross-dependency on the other.

[OPUS-4.8] sq-2q1x / sq-fix4 / sq-i1wh2 / sq-pwr.2 / sq-pwr.3 / sq-xkrt — flagged for Fable re-review.
[OPUS-5] sq-1fo4 — Phase 5, canonicalisation half only."#
)]
// Default (feature OFF) build: a concise, link-free crate doc. None of the gated items above
// exist in this build, so the doc is plain text only (no intra-doc links). [OPUS-4.8] sq-2q1x.
#![cfg_attr(
    not(feature = "fedplan-mpc"),
    doc = r#"sparq-fedplan-mpc: the opt-in glue between cost-based federated source selection
(`sparq-fedplan`) and MPC-over-federated-SPARQL routing (`sparq-mpc`) — the untrusted-planner →
MPC-routing seam (beads sq-2q1x + sq-fix4 + sq-i1wh2 + sq-pwr.2 + sq-pwr.3 + sq-xkrt + sq-1fo4). See `README.md`
and `skills/mpc/SKILL.md` for the full design.

**This is the default build with the `fedplan-mpc` feature OFF, so the crate is empty** — the
whole surface (the `SourcePrivacyDescriptor`, the Phase-2 `select_private_sources` adapter, the
Phase-3 `route_operators` routing pass, the Phase-4 `assemble_leakage_envelope` +
`ratify_envelope` gate, the Phase-6 `prune_source_combinations` result-aware combination
prune, and the Phase-5 `commit_plan` / `revalidate_plan` plan-binding pair) is gated behind the
**`fedplan-mpc` cargo feature, OFF by default**. Build with `--features fedplan-mpc` to see the
seam API. The crate is a standalone `publish = false` workspace member; `sparq-core` /
`sparq-engine` never depend on it, and a feature-off build pulls in neither `sparq-fedplan` nor
`sparq-mpc`. The crate performs no MPC and makes no privacy/soundness claim; the remaining
privacy-bearing phases are deferred and audit-gated (sq-9hrn / sq-qhy4).

[OPUS-4.8] sq-2q1x / sq-fix4 / sq-i1wh2 / sq-pwr.2 / sq-pwr.3 / sq-xkrt — flagged for Fable re-review.
[OPUS-5] sq-1fo4 — Phase 5, canonicalisation half only."#
)]
#![forbid(unsafe_code)]
// [OPUS-4.8] sq-2q1x: crate has zero `unsafe`.
// [OPUS-4.8] sq-2q1x: lock the crate-doc split in — `broken_intra_doc_links` is a rustdoc-only
// lint (fires under `cargo doc`, never `cargo build`/`clippy`/`test`), so denying it keeps the
// existing build/clippy/test gates untouched while making a future broken link a hard error.
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]
#![cfg_attr(not(feature = "fedplan-mpc"), allow(dead_code, unused_imports))]

// The whole seam surface is feature-gated; a feature-off build compiles an empty crate.
// [OPUS-4.8] sq-pwr.3: Phase 6 result-aware source-combination prune lives in its own module
// (mirroring the Phase-2 `selection` / Phase-3 `routing` / Phase-4 `envelope` splits).
#[cfg(feature = "fedplan-mpc")]
pub mod combination;
// [OPUS-5] sq-1fo4: Phase 5 untrusted-plan binding. CANONICALISATION HALF ONLY — the commitment
// + fail-closed re-validation. The soundness half (`bind_plan_to_proof`) stays deferred: there is
// no collaborative proof to bind to, and sq-qhy4 / sq-9hrn are pending.
#[cfg(feature = "fedplan-mpc")]
pub mod binding;
// [OPUS-4.8] sq-pwr.2: Phase 4 leakage-envelope assembly + dual ratification lives in its own
// module (mirroring the Phase-2 `selection` / Phase-3 `routing` splits).
#[cfg(feature = "fedplan-mpc")]
pub mod envelope;
#[cfg(feature = "fedplan-mpc")]
mod privacy;
// [OPUS-4.8] sq-i1wh2: Phase 3 disclosed/hidden routing pass lives in its own module (mirroring
// the Phase-2 `selection` split).
#[cfg(feature = "fedplan-mpc")]
pub mod routing;
// [OPUS-4.8] sq-fix4: Phase 2 source-selection adapter lives in its own module (mirroring the
// upstream `sparq-fedplan::selection` split).
#[cfg(feature = "fedplan-mpc")]
mod seam;
#[cfg(feature = "fedplan-mpc")]
pub mod selection;

#[cfg(feature = "fedplan-mpc")]
pub use privacy::{Disclosability, SourcePrivacyDescriptor, SourcePrivacyDescriptorBuilder};
// [OPUS-4.8] sq-pwr.3 / sq-xkrt: Phase 6 — the result-aware source-combination prune + its
// result types (including the Rule-C2 `DeadPairing`).
#[cfg(feature = "fedplan-mpc")]
pub use combination::{
    prune_source_combinations, BgpComponent, CombinationPruneReason, DeadPairing,
    PrunedCombination, PrunedCombinations,
};
// [OPUS-5] sq-1fo4: Phase 5 — the plan commitment + fail-closed re-validation, and the DEFERRED
// plan→proof binding.
#[cfg(feature = "fedplan-mpc")]
pub use binding::{
    bind_plan_to_proof, commit_plan, revalidate_plan, CommittedOperator, CommittedPattern,
    PlanCommitment, PlanDivergence, PlanRevalidation, PARTICIPATION_DOMAIN_TAG, PLAN_DOMAIN_TAG,
    ROUTING_DOMAIN_TAG, SELECTION_DOMAIN_TAG,
};
// [OPUS-4.8] sq-pwr.2: Phase 4 — the implemented leakage-envelope assembly + dual-ratification gate.
#[cfg(feature = "fedplan-mpc")]
pub use envelope::{assemble_leakage_envelope, ratify_envelope};
// [OPUS-4.8] sq-i1wh2: Phase 3 — the implemented disclosed/hidden routing pass + its input types.
#[cfg(feature = "fedplan-mpc")]
pub use routing::{route_operators, Operand, QueryOperator, RoutingPolicy};
// [OPUS-4.8] sq-fix4 / sq-pwr.2: the shared seam error/phase types + the Phase-4 envelope/outcome types.
#[cfg(feature = "fedplan-mpc")]
pub use seam::{
    LeakageEnvelope, OperatorDisclosure, PrivateRouting, RatificationOutcome, SeamError, SeamPhase,
};
// [OPUS-4.8] sq-fix4: Phase 2 — the implemented privacy/authorisation-aware source-selection
// adapter and its result types.
#[cfg(feature = "fedplan-mpc")]
pub use selection::{
    select_private_sources, PrivateCandidate, PrivatePatternSources, PruneReason, PrunedCandidate,
    SelectedPrivateSources,
};
