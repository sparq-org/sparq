<!-- [OPUS-4.8] sq-2q1x / sq-fix4 / sq-i1wh2 / sq-pwr.2 / sq-pwr.3 / sq-xkrt; [OPUS-5] sq-1fo4: internal README for a publish=false crate; full design lives in research/mpc-untrusted-planner-routing-design.md. -->
# sparq-fedplan-mpc

The opt-in seam between cost-based federated source selection (`sparq-fedplan`) and MPC-over-federated-SPARQL routing (`sparq-mpc`) — without coupling the
two upstreams to each other. Behind the **`fedplan-mpc` cargo feature, OFF by default**; the default build compiles an empty crate and pulls in neither
upstream. Per-item statements, recall-safety arguments and error cases live in the module rustdoc; the design record is
[`research/mpc-untrusted-planner-routing-design.md`](../../research/mpc-untrusted-planner-routing-design.md).

## 🚀 Quickstart

Internal crate (`publish = false`); enable the feature in the workspace, select the participating sources (Phase 2), result-aware-prune the source
combinations (Phase 6), partition the operators disclosed-vs-hidden (Phase 3), assemble + dual-ratify the declared leakage envelope (Phase 4), commit to
the resulting plan (Phase 5 — canonicalisation only):

```rust
// behind `--features fedplan-mpc`
let selected = sparq_fedplan_mpc::select_private_sources(&bgp, &descriptors, &privacy)?;
// Phase 6 — result-aware combination prune: is any full-BGP combination feasible?
let combos = sparq_fedplan_mpc::prune_source_combinations(&bgp, &descriptors, &selected)?;
if combos.is_bgp_dead() {
    // no conjunct has a live source — skip the MPC path entirely
}
// ...and skip the individual combinations Rule C2 proved dead (one source index per pattern)
let live = combinations.filter(|assignment| !combos.combination_is_dead(assignment));
let routing = sparq_fedplan_mpc::route_operators(
    &selected, &privacy, &operators, RoutingPolicy::Default,
)?;
// routing.routing — Vec<OperatorRouting> the sparq-mpc pipeline consumes
let envelope = sparq_fedplan_mpc::assemble_leakage_envelope(&routing, &operators, &selected)?;
// dual ratification: each holder fail-closed + the verifier's disclosed-operand budget
match sparq_fedplan_mpc::ratify_envelope(&envelope, &privacy, Some(budget)) {
    RatificationOutcome::Ratified => { /* cleared for execution */ }
    rejected => { /* a holder vetoed a private-term disclosure, or the verifier rejected an over-leak */ }
}
// Phase 5 — commit, then fail-closed re-validate a recomputed plan. CANONICALITY ONLY, not soundness.
let claimed = sparq_fedplan_mpc::commit_plan(&selected, &routing, &envelope)?;
match sparq_fedplan_mpc::revalidate_plan(&claimed, &recomputed) {
    PlanRevalidation::Bound => { /* matches field-for-field and digest-for-digest */ }
    PlanRevalidation::Diverged(d) => { /* names the dropped source / re-routed operator */ }
}
// The step that would make a divergence a PROOF failure is deferred and audit-gated:
assert!(sparq_fedplan_mpc::bind_plan_to_proof(&claimed, &statement).is_err()); // SeamError::Deferred
```

## ✨ Features

- **`SourcePrivacyDescriptor`** (Phase 1, sq-2q1x) — the per-source privacy declaration. Posture is **default-deny**: a predicate is disclosable only when
  the source explicitly marks it `Public`; a planner cannot widen it, the source re-enforces fail-closed.
- **`select_private_sources`** (Phase 2, sq-fix4) — the **source-selection adapter**: runs `sparq-fedplan::select_sources` over the BGP + descriptors, then
  **prunes** any source whose descriptor declares it will not participate (`participates == Some(false)`), surfacing the retained per-pattern candidates
  plus an **audit trail** (`pruned`) of what was dropped and why. Participation/authorisation is the **only** prune rule — a source holding a *private*
  predicate stays a candidate (its in-clear-vs-MPC routing is the Phase-3 decision), so the adapter is recall-safe and loses no answers.
- **`route_operators`** (Phase 3, sq-i1wh2) — the **disclosed/hidden routing pass**: given the Phase-2 selection, the privacy descriptors and the query's
  `QueryOperator`s, it classifies each operator `Disclosed` (every operand disclosable in the clear) or `Hidden(class)` (default-deny otherwise — routes
  through the MPC path), emitting the `Vec<OperatorRouting>` the `sparq-mpc` pipeline today receives hand-written. The `RoutingPolicy` knob picks the cheap
  default (disclose global-IRI operands) or the strict "hide even a public term" route; the most-private contributing source wins per operand.
- **`assemble_leakage_envelope` + `ratify_envelope`** (Phase 4, sq-pwr.2) — the **leakage-envelope assembly + dual-ratification** gate.
  `assemble_leakage_envelope` derives a declared `LeakageEnvelope` that **honestly enumerates** what the plan reveals — the operator structure
  (count/class/label), the disclose/hide partition, the operands each `Disclosed` operator exposes, and which sources participate — **over-counting, never
  under-counting**. `ratify_envelope` then runs the dual gate: each **holder** fail-closed-rejects a plan disclosing one of its own private predicates
  (constraint C-B — the most-private holder wins), AND the **verifier** rejects an over-leaking envelope (distinct disclosed operands exceeding its
  budget), returning a `RatificationOutcome` naming *which* ratification failed and *why*.
- **`prune_source_combinations`** (Phase 6, sq-pwr.3 + sq-xkrt) — the FedUP-style **result-aware source-combination prune** over the Phase-2 selection. The
  seam's secure-join cost grows with the number of source *combinations* (one source per pattern) it considers, so this pass surfaces the combinations that
  **provably contribute no answer** and the seam routes fewer toward MPC. Two recall-safe rules:
  - **Rule C1 — unsatisfiable-conjunct collapse.** An empty Phase-2 candidate list proves that conjunct empty, so the conjunction is unsatisfiable and
    **every** combination is dead (`∅ ⋈ R = ∅`).
  - **Rule C2 — dead same-source star pairing** (the FedUP *quotient-summary/provenance* rule, and the one that fires on the **live** path). If two
    patterns share a **subject**-position join variable and **no** served characteristic set of a source carries both their predicates, assigning *both*
    patterns to that source is dead, along with every combination containing that pairing — while the source stays a valid candidate for each pattern
    **individually** (a cross-source combination is untouched). It fires **only** behind a per-predicate completeness guard (`Σ_{C ∋ p} subjects ==
    void:distinctSubjects` over DISTINCT set keys), so a truncated, repeated-key or unknown summary declines.

  The Phase-2 selection passes through **unchanged** (advisory + auditable), reporting `bgp_satisfiable`, the empty-pattern witnesses, the Rule-C2
  `dead_pairings` + `dead_patterns`, the BGP join components, and a combination-dead audit trail; `combination_is_dead(&assignment)` is the per-combination
  test an enumerator runs, returning `false` ("keep it") for anything it cannot prove. The value-overlap / bound-IRI-propagation prune is **deliberately
  declined** — not recall-safely expressible from the public summary (single-IRI `may_hold_authority`, no authority-set enumerator) — so a future
  contributor does not build an unsound prune. Pattern ids that are not a permutation of the BGP's, an out-of-range candidate index, and a `source_id`
  mismatched to its descriptor all fail closed (`DescriptorMismatch`/`SourceCombination`).
- **`commit_plan` + `revalidate_plan`** (Phase 5, sq-1fo4) — the **untrusted-plan binding seam**, and **only its canonicalisation half**. `commit_plan`
  reduces the produced plan (Phase-2 per-pattern source assignment + Phase-3 disclosed/hidden route + Phase-4 participation set) to a deterministic,
  domain-separated `PlanCommitment` whose `plan_digest()` is one `sparq_mpc::FieldWord` — shaped to become a public input once a proof exists to carry it.
  It binds plan **content**, not the planner's enumeration order, and excludes `estimated_cardinality` (a cost estimate, not plan semantics).
  `revalidate_plan` compares a *claimed* commitment against an *independently recomputed* one and **fails closed**, naming the **dropped source** /
  **re-routed operator** via `PlanDivergence`; a digest mismatch is never `Bound`, and a routing/envelope disagreement — including a hidden operator
  carrying disclosed operands, or a participation set that is not exactly the distinct sources the selection assigns — is refused at commit time
  (`DescriptorMismatch`/`PlanBinding`).
  - **A canonicality + transcript-integrity check, NOT a soundness mechanism.** A malicious planner computes the plan *and* its commitment, so one that
    drops a source simply commits to the plan it actually ran and passes. It catches *accidental* divergence (a plan mutated after ratification, a
    planner/verifier disagreement, a truncating transport).
  - The **soundness half — `bind_plan_to_proof`, which would discharge C-A — is DEFERRED**, returning `SeamError::Deferred` naming its gates: no
    collaborative proof exists (`sparq-mpc::proof` is an honest `NotYetImplemented` stub), `sq-qhy4` + `sq-9hrn` are pending, and the attachment point is
    design-record **§9 Q3, still open** (the `FederatedStatement` parameter is a *proposal*, not a decision).

> **Internal crate — not published** (`publish = false`). **No soundness or privacy claim.** Phases 2–6 are **plumbing — source-selection + result-aware combination pruning + disclosed/hidden routing +
> leakage-accounting/ratification + a canonical plan encoding, not a cryptographic guarantee**: they perform **no** MPC, run **no** privacy-bearing cryptographic logic, open nothing, and verify nothing
> cryptographic — the descriptors and operators are the caller's own inputs, the combination prune reads only the already-public Phase-2 selection, the envelope is a *declaration* of what the plan
> reveals, the dual ratification is a *plan-time policy gate* (not a runtime guarantee a malicious holder/verifier honours it), and the Phase-5 plan commitment is a domain-separated *digest of plan
> metadata the envelope already declares* which **provides no soundness on its own**. The MPC estate (`sparq-mpc`) is **research-grade, honest-majority semi-honest only, and NOT externally audited** —
> the cryptographer sign-off (`sq-qhy4`) and coZK re-audit (`sq-9hrn`) are pending. The further privacy-bearing work (Phase 5's `bind_plan_to_proof` soundness half, authenticated-input attestation) is
> **deferred and audit-gated**; nothing here is a working protocol.
> <!-- privacy-claims-allow: NEGATIVE/scoped — denies any privacy/soundness property; audits sq-qhy4 / sq-9hrn pending -->

**Threat-model / leakage note (honest).** Phase 2 reads each source's *own* declared `SourcePrivacyDescriptor` to decide participation; Phase 6 reads only the (public) Phase-2 selection to decide
which combinations are infeasible (revealing nothing beyond what the caller's own descriptors already imply, and making **no** value-overlap inference); Phase 3 reads `may_disclose` per operand to
decide the route. Per constraint C-B (§2.2) the descriptor is the source's declaration — Phase 4's `ratify_envelope` has each holder **re-enforce** it fail-closed and the verifier ratify the leakage
envelope, so a lying planner that over-discloses is **rejected here, not honoured**. The plan **itself reveals** the **operator structure** (count, class, order), the **disclose/hide partition**, the
**disclosed operands** and the **participating sources** — enumerated honestly (over-counting) in the envelope — but **not** hidden-operand values or result cardinalities; nothing here claims what is
learned once a query executes, because no query executes here. Phase 5's `PlanCommitment` carries **plan metadata only** — source ids, operator labels, operator classes and disclosed predicate ids,
every one of which the Phase-4 envelope already declares as disclosed by the plan — so it widens the declared leakage envelope by nothing, and it carries no operand values, result rows or
cardinalities.

## 📚 Learn more

- Design: [`research/mpc-untrusted-planner-routing-design.md`](../../research/mpc-untrusted-planner-routing-design.md).
- Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
