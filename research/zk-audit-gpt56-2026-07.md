<!-- [FABLE-5] GPT-5.6/codex STAND-IN audit for sq-qhy4 (CR-G1). This is an
     AI-conducted audit driven by GPT-5.6 (codex CLI), cross-checked against the
     code by the orchestrating agent. It is NOT a human accredited-cryptographer
     sign-off and does NOT close sq-qhy4. Every finding below was verified against
     the actual code before recording; codex findings that did not survive
     cross-check are listed as dismissed with the reason. -->

# ZK soundness audit — GPT-5.6 (codex) STAND-IN for the external cryptographer audit (sq-qhy4)

> 🤖 **SPARQ agent** — audit record. This is an **AI-conducted** ZK soundness
> audit driven by **GPT-5.6 (`codex` CLI, codex-cli 0.144.1)** as an explicit
> **stand-in** for the accredited-human-cryptographer audit the maintainer has
> not yet commissioned (bead **sq-qhy4**, gap **CR-G1**). The maintainer asked
> for GPT-5.6 as the stand-in. **Honesty header (do not soften):** an AI audit —
> even a careful, cross-checked one — **materially reduces risk but does NOT
> substitute for human accredited sign-off.** It does not carry professional
> accountability, it inherits the model's blind spots, and it did not run the
> proving toolchain end-to-end (no live `nargo`/`bb` forge execution). Treat this
> as **AI-audited, human sign-off still recommended**. It does **not** close
> sq-qhy4 and does **not** by itself authorise any production ZK security claim.

## 0. Scope + methodology

**Scope (single-prover estate, per the sq-qhy4 charter; `sparq-mpc` excluded):**

- The **Noir circuits** under `zk/compose/compose_core/src/*.nr` (the 13 in-circuit
  relations: `scan`, `filter_int`, `filter_signed`, `filter_float`, `filter_value`,
  `join`, `path`, `revoke`, `holder`, `issuer`, `hashes`) plus the deployed member
  packages under `zk/compose/*/src/main.nr`.
- The **Rust verifier + crypto host** (`crates/sparq-zk-compose/src/verifier.rs` —
  `reconstruct_public_inputs`, `bind_query_correctness`, `bind_issuer_attestations`,
  `bind_joins`, `bind_revocation`/`bind_hidden_revocation`, `prefilter_manifest_structure`,
  `derive_id`; `crates/sparq-zk/src/sig.rs` Schnorr-over-Baby-JubJub; `commit.rs`,
  `dual_leaf.rs`, `poseidon2.rs`, `field.rs`).

**Method.** For every circuit and each verifier component, GPT-5.6 was driven via
`codex exec --skip-git-repo-check` with an adversarial soundness prompt (under-constrained
signals, missing range/bounds checks, Fiat–Shamir/transcript binding, Schnorr flaws,
commitment binding, replay/nullifier, field-arithmetic edge cases, and the dual-leaf
value↔lexical risk). Large files were chunked. **Every codex finding was then
cross-checked against the actual code by the orchestrating agent** — this is the
load-bearing step: codex audits each component *in isolation* and repeatedly flags
gaps that the *host-side binding layer* (or a sibling gate, or a since-landed fix)
already closes. Only findings that survived that cross-check are recorded as REAL;
the rest are listed as dismissed with the concrete reason.

**Prior internal audits consulted** (to distinguish NEW findings from already-tracked
gaps): `research/zk-soundness-audit.md` (v1 BROKEN, 12 findings), `research/zk-verifier-reaudit.md`
(post-remediation "sound as landed"), `research/zk-audit-readiness-dossier.md` (CI inventory +
per-claim status), `research/zk-dual-leaf-issuer-desync-review.md`, and `compliance/cryptoreview/gap-register.md` (CR-G1..G9).

---

## 1. Findings (verified)

| # | Component | Finding | Severity | Exploitability | Status |
|---|---|---|---|---|---|
| **H-1** | `zk/.../issuer.nr` `schnorr_verify` / `key_set_membership` + `sig.rs::public_key_from_hex` | **In-circuit issuer Schnorr verify has NO prime-order subgroup check on the free-witness issuer `pk`** — only `assert_on_curve` + reject-identity `(0,1)`. The host `public_key_from_hex` likewise rejects only the identity, not non-subgroup points. A small-order (torsion) issuer key such as `(0,-1)` (order 2) is on-curve, is not `(0,1)`, and passes both. | **HIGH** | If a torsion point is present in the relying party's trusted KeySet K, a prover forges a valid Schnorr proof under it with **no secret key** (pick `s`; set `R=sG` when the Poseidon challenge is even, else `R=sG+pk`; `e·pk` depends only on challenge parity for an order-2 key), then satisfies the hidden-issuer key-set Merkle membership. Gated by K-curation — but K-curation has no subgroup check either, so the guarantee "only a holder of a secret key for a key in K can attest" is broken in-circuit. | **NEW / CONFIRMED** |
| **M-1** | `commit.rs`, `dual_leaf.rs`, `filter_value.nr` | **`DualLeafV1.is_production_selectable() == true` while `removes_inv_vl() == true`** — the dual-leaf value lane does not bind the compared value to `parse(committed lexical)`; a malicious *trusted* issuer can commit lexical `"5"` yet a value handle `18` and pass a `>= 18` FILTER. | **MEDIUM** | Bounded to a **malicious trusted issuer** (an untrusted party cannot exploit it; `StringCanonicalV1` — the only end-to-end method — is not affected, and the `dual-leaf` cargo feature is OFF by default; the leaf encoding `sq-j506` is not implemented end-to-end). | **CONFIRMED — already tracked (CR-G8 / sq-j506)** |
| **L-1** | `verifier.rs::bind_query_correctness` (~3869) | **FILTER gating uses `find_map` to pick only the FIRST slot** a filtered variable occupies within a scan. A single scan whose bound pattern answers two query patterns that place the filtered variable at different slots (e.g. scan `(?,P,?)` answering both `(?s P ?v)` and `(?v P ?s)`) has only one of those slots gated. | **LOW** | **CONFIRMED reachable at the structural gate** (sq-q9r5e, below): a fully-attested, revocation-fresh manifest with an HONEST filter proof over the first slot and the second slot entirely ungated returned `Ok`. Still not shown to yield a *useful* forged answer — the second slot is a subject/predicate position, so the ungated `?v` binding is an IRI a numeric FILTER could not legitimately satisfy anyway. | **CONFIRMED + FIXED (sq-q9r5e)** |
| **L-2** | `verifier.rs` holder-PoP path | **Declared `zk:cryptosuite` IRI is validated as an allowlist but the resolved `SignatureScheme` is discarded** — verification always calls the fixed Schnorr `sig_verify` regardless of the declared suite. | **LOW** | Benign today (exactly one signature scheme is implemented, and it *is* Schnorr), but it is not fail-closed against a future second scheme / downgrade. Overlaps the documented CR-G9 (method×circuit×signature matrix) obligation. | **CONFIRMED (defense-in-depth) — overlaps CR-G9** |

### H-1 detail (the one NEW soundness gap)

The host clear-issuer path (`bind_issuer_attestations` → `sig::verify`) is safe: `sig::verify`
(sig.rs:1076-1079) checks `is_in_correct_subgroup_assuming_on_curve()` for both `R` and `pk`.
That check is the **only** prime-order subgroup guard in the entire estate. The **hidden-issuer**
path (`bind_hidden_issuer_attestations` → the in-circuit `issuer.nr::schnorr_verify`) does **not**
reproduce it: it does `assert_on_curve(pk)`, `assert_on_curve(r)`, rejects the neutral `(0,1)`,
range-binds `s`/`e`, and checks `s·G == R + e·pk`. None of these rejects an order-2/4/8 point.
Baby-JubJub has cofactor 8, so torsion points exist; `(0,-1)` is the order-2 point, on-curve,
`≠ (0,1)`. The comment at `issuer.nr:158-161` **falsely** claims `assert_on_curve` rejects a
"small-subgroup / twist point" — it does not (a small-subgroup point *is* on the curve). This is
exactly the false-assurance-comment hazard the prior audit flagged, plus the in-circuit analogue
of the host-side identity-key fix already applied (`sig.rs` "codex #3").

The holder path is safe by construction: the holder key `hpk = hsk·G` is derived by `scalar_mul`,
so it lies in `⟨G⟩` (the prime-order subgroup) — only the **free-witness issuer `pk`** needs the
guard. Recommended fix (both layers): (a) add an in-circuit prime-order check to `issuer.nr`
(assert `[L]·pk == identity`, or a cofactor-cleared equality) for the free issuer `pk` (and,
defense-in-depth, `R`); (b) reject non-prime-order-subgroup points in `sig::public_key_from_hex`
so a torsion key can never enter a KeySet K. Tracked as the fix bead below.

---

## 2. Dismissed codex findings (cross-checked to false-positive / already-closed)

codex, auditing each component in isolation, raised these; each was **verified against the code
and dismissed** — recorded here for the human auditor's efficiency (do not re-chase):

- **`join.nr` "challenge absent" / `reconstruct` "JOIN not bound to scans/query" (codex: critical).**
  FALSE-POSITIVE / **dossier was stale**: `bind_joins` **is** wired (`verify_manifest_impl:2350`,
  sq-h732x) — it enforces commitment-matching (join `commit_a/b` byte-equal the referenced scans'
  attested commitments), canonical-VK via the sub-proof loop, and query slot binding; and
  `reconstruct_public_inputs` **does** rebuild JoinEq public inputs `[challenge, commit_a, commit_b,
  join_commitment, slot_a, slot_b]` and byte-compare them. The `challenge` is byte-bound as public
  field 0 by the host for **every** member (unconstrained in-circuit by design), so join replay is
  closed at the host, same as all other members.
- **`revoke.nr` "hidden index not bound to credential" (codex: critical).** FALSE-POSITIVE against
  the **deployed** member: `revoke_unset_d10/main.nr` calls `revoke_unset_check_committed` (not the
  raw `revoke_unset_check` that codex read in the module), which binds a hiding `index_commitment`
  that `bind_hidden_revocation` byte-matches against the **issuer-signed** index commitment (sq-ayv).
- **`filter_float` raw `filter_f64_check` "operand unbound" (codex: high).** FALSE-POSITIVE for the
  composed system: the raw `filter_f64` member has public inputs `(challenge, op, b_bits, expected)`
  with **no `operand_enc`** and **no `CircuitId::FilterF64` mapping**, so it cannot be presented as a
  composable `ProofInputs::FilterF64` (arity → `PublicInputMismatch`; no canonical member to compile a
  vk from). The composable `filter_f64_d{1..4}` derive the IEEE bits from the bound value. The raw
  member is a documented gate-count building block only.
- **`recon` "scan rows beyond capacity `r` silently omitted" (codex: high).** FALSE-POSITIVE:
  `derive_scan_id` sets `r = smallest_bucket([4,8], max(rows.len(), row_count))` and stage-1b requires
  `derived == declared`, so `r >= rows.len()` on every accepted path (else `CircuitIdMismatch`). Every
  disclosed row is byte-bound.
- **`bindquery` "non-canonical FieldHex alias" (codex: medium).** NOT A BUG: `field_from_hex_str`
  uses `from_be_bytes_mod_order` (canonicalising) and `push_field` re-serialises the reduced element
  canonically before byte-comparing against bb's canonical output — aliases collapse to the same
  canonical field element the circuit constrains.
- **`bindquery` "empty scan vacuously discharges FILTER" (codex: medium).** NOT A BREAK: a scan with
  `row_count = 0` discloses no rows, so it asserts no passing row — an honestly-empty result makes no
  false claim. Non-empty scans over the same pattern are still gated per-row.
- **`scan.nr` duplicate-row multiplicity / `hashes.nr` `commit_fold` count truncation (codex: high).**
  Already-documented hardening notes, **not forges** under the system's declared RDF-merge/union set
  semantics and the issuer-signed commitment (the commit-fold length IV gives length separation; a
  short-count view has no valid issuer signature). See `zk-soundness-audit.md` hardening §.
- **`filter_int` / `filter_signed` / `holder` / `path` / `poseidon2` / `sig` (chunks).** codex
  returned **SOUND** (with the usual "can't see the host/omitted paths" caveat) — consistent with the
  internal re-audit. Notably codex independently re-derived that the host Schnorr `verify` correctly
  rejects identity, off-curve, and non-subgroup points and binds the challenge to `(R, pk, m)`.

---

## 3. OVERALL VERDICT

**`AI-AUDITED: issues-found` — one HIGH soundness gap (H-1), no CRITICAL.**

- **No CRITICAL soundness issue** was found or survived cross-check. Every historical CRITICAL from
  `zk-soundness-audit.md` remains closed on `main`; the binding layer (`reconstruct_public_inputs` +
  the `bind_*` gates) holds up under this independent adversarial pass, and several dossier items the
  auditor would otherwise chase are in fact **already wired** (join binding) or **hardened past the
  raw module** (committed revocation).
- **One HIGH gap is NEW and genuine (H-1):** the in-circuit issuer Schnorr and the host key parse
  lack the prime-order subgroup check the clear-path `sig::verify` has. It is bounded by KeySet
  curation but is a real weakening of the hidden-issuer soundness guarantee and should be fixed before
  any production reliance on the hidden-issuer path.
- **M-1 (dual-leaf INV-VL removal) is real and already tracked (CR-G8 / sq-j506);** the auditor should
  decide whether `DualLeafV1` should be `is_production_selectable() == false` until sq-j506 lands and
  is audited.
- **L-1/L-2** are low-severity items for the human auditor to confirm. **L-1 has since been
  confirmed reachable and fixed** (`sq-q9r5e`, see §4); L-2 remains open under CR-G9.

**Release #1084 ZK gate — recommendation.** With **no critical** and the single **high** gap H-1
scoped to the (feature/opt-in) hidden-issuer path plus a fixable host parse, the maintainer *may*, at
their discretion, treat this stand-in as **provisionally discharging the sq-qhy4 gate for the
`StringCanonicalV1` single-prover clear-issuer path** — **pending** (i) landing the H-1 fix and (ii)
an explicit maintainer decision to accept an AI audit in lieu of human sign-off. **This document does
not close sq-qhy4.** The conservative `SECURITY.md` posture ("NOT externally audited") remains correct
until a human accredited cryptographer signs off; that engagement is still recommended and is what
`sq-qhy4` exists to commission.

---

## 4. Follow-up beads

- **H-1 → new P1 bug `sq-l15mi`**: add the in-circuit prime-order
  subgroup check to `issuer.nr` for the free issuer `pk` (+ `R` defense-in-depth) and reject
  non-subgroup keys in `sig::public_key_from_hex`; add a forge-and-verify regression (torsion-key
  attestation → reject) and fix the false-assurance comment at `issuer.nr:158-161`.
- **L-1 → new P2 `sq-q9r5e`** (analysis/hardening): confirm or refute the `bind_query_correctness` single-slot
  gating gap; if reachable, gate **every** slot a filtered variable occupies within a scan.
  **RESOLVED — CONFIRMED reachable, then fixed.** The empirical witness
  (`crates/sparq-zk-compose/tests/e2e.rs::filter_reject_ungated_second_slot_within_scan`)
  builds the audit's own shape — query `{ ?s <age> ?v . ?v <age> ?o } FILTER(?v >= 18)`, whose two
  patterns share the constant layout `(?, <age>, ?)` so ONE scan sub-proof answers both, placing `?v`
  at slot 2 (pattern 0) and slot 0 (pattern 1). With the scan issuer-attested, the revocation fresh,
  and an *honest* `25 >= 18` `filter_int` proof wired to slot 2 only, the pre-fix
  `prefilter_manifest_structure` returned **`Ok`** — slot 0 was never gated. Note the reachability is
  at the *structural gate*, not a demonstrated useful forge: the un-gated position is a subject slot,
  so the ungated `?v` binding is an IRI, which a numeric FILTER could not honestly satisfy in any
  case. The fix replaces the `find_map` with the full `BTreeSet` of slots `?v` occupies across every
  pattern that scan answers and requires a true-verdict edge per `(row, slot)` — matching the
  all-matching-scans discipline `bind_attributions` already uses. It is deliberately FAIL-CLOSED on
  the pattern→scan ambiguity: where two patterns share a constant layout the verifier cannot tell
  which one a scan was meant to answer, so it now demands the FILTER be discharged at every slot that
  scan could be read at, and REJECTS a manifest that cannot. Manifests over queries whose patterns
  have distinct constant layouts (the ordinary case, incl. every other test in the suite) are
  unaffected.

  **Structural follow-up — the over-demand STANDS; the explicit pattern→scan mapping does NOT
  relax it.** The fail-closed direction above is an over-demand. Take `{ ?x <age> ?v . ?x <age> ?c
  FILTER(?v >= 18) }` — two same-layout patterns joined on `?x`, with the FILTER on `?v`, which
  occurs only in pattern 0. An honest prover answering pattern 0 from `{alice age 25}` and pattern 1
  from `{alice age 5}` presents a joined solution (`?x = alice`, `?v = 25`, `?c = 5`) satisfying the
  FILTER — yet membership also matches the second scan to pattern 0 and demands a true-verdict
  `?v >= 18` proof over its `5`, which no honest prover can supply, so the manifest is rejected.
  (Note the *pre*-fix code could not serve that shape either — `find_map` gated the FIRST-matching
  pattern's slot on EVERY matching scan.)

  The obvious fix — let the prover DECLARE which scan answers which pattern and demand only the
  declared scan's slots — was attempted and **rejected as unsound** in review. Ordinary SPARQL
  evaluates each BGP pattern over EVERY compatible committed row, and the query text contains no
  source partition authorising the prover to redefine membership. A prover free to exclude a
  constant-compatible scan from a pattern can therefore drop that scan's rows out of the pattern's
  FILTER and attribution obligations by fiat while still disclosing them; well-formedness rules on
  the declaration (no empty entry, no dangling scan, no pair contradicting the bb-bound constants)
  establish only that the declaration is a TOTAL map of scans to labels, never that an excluded scan
  cannot contribute to the claimed result. Shifting the correct reading onto the consumer does not
  prove the query result, so the flat verifier keeps FULL constant-membership obligations.

  What DID land is the schema slot plus a fail-closed well-formedness gate, deliberately carrying no
  verification weight. `ProofManifest::pattern_scans` records, per query BGP pattern (query order,
  exactly like `attributions`), the `sub_proofs` indices of the scans the prover says answer it, and
  `verifier::check_pattern_scans` rejects a declaration that is mis-sized
  (`PatternScanArityMismatch`), leaves a pattern unanswered (`PatternScanUnbound`), names a sub-proof
  that is out of range / not a scan / whose bb-bound `pattern_is_const`/`pattern_const_enc`
  contradict the pattern's constants (`PatternScanMismatch`, audit #10), or leaves a scan DANGLING
  (`PatternScanUndeclared`). Those are ADDITIONAL rejections: `bind_query_correctness` (FILTER
  slots), `bind_attributions` (audit #8), `global_attributions` (the Q6 cross-graph namespace) and
  `bind_joins` all resolve pattern→scan by `scan_matches_pattern` membership and never read the
  field, so a manifest carrying a declaration is never accepted where the same manifest without one
  is rejected — it can only fail additionally. An EMPTY `pattern_scans` means "not declared" and skips the checks; obligations are identical
  either way.

  **What narrowing would need before it can be revisited:** a claimed result row bound to the
  selected scan rows, with all shared-variable joins enforced, so "this scan does not contribute to
  the answer" is a VERIFIED property and not a prover assertion. The flat `ProofManifest` carries no
  claimed result row, so it cannot express that witness today. Witnesses:
  `crates/sparq-zk-compose/tests/e2e.rs::pattern_scans_*` — the same-layout manifest stays REJECTED
  under every declaration a prover could write (including the intended one-scan-per-pattern
  assignment and its cross), the cross-slot shape stays rejected when the declaration is engineered
  to hide each scan's opposite failing slot, the L-1 single-scan witness stays rejected, and each of
  the four well-formedness rejections is pinned. Both non-narrowing witnesses were mutation-checked:
  making `bind_query_correctness` read `pattern_scans` turns them red. NOT externally audited
  (sq-qhy4).

  **Sibling finding raised while fixing L-1 — WITHIN-pattern repeated variable (#5240).
  CONFIRMED reachable, then fixed.** L-1 is about one variable at two slots across two PATTERNS.
  The question it raised is the degenerate case: one variable at two slots of ONE pattern,
  `{ ?v <p> ?v }`. SPARQL matches a BGP pattern under a single substitution, so such a pattern
  constrains the two disclosed columns to be EQUAL — and nothing enforced that.
  `scan_matches_pattern` compares only per-slot const-ness and the constant encodings (both slots
  are variables, so any `(?, <p>, ?)` scan matches); the scan circuit binds only the CONSTANT slots
  to `pattern_const_enc`, so the in-circuit statement is silent; and every shared-variable gate
  works CROSS-pattern and is structurally blind to the repeat —
  `sparq_zk::verify::cross_graph_join_obligations` (the disclosed path, via `recheck`) iterates
  pattern PAIRS `i < j` over per-pattern variable SETS, where a repeated variable collapses to one
  element and `i == j` is never considered, and `bind_joins` (the hidden path) likewise requires the
  shared variable to sit in two DISTINCT patterns (`pj != pi`). That combination — the cross-pattern
  and cross-scan analogues both covered, the within-pattern one not — is what made it look like a
  gap rather than a deliberate omission.

  Confirmed by the same method as L-1. A fully-attested, revocation-fresh manifest whose single scan
  discloses the row `(alice, knows, bob)` was presented under `SELECT ?v WHERE { ?v <knows> ?v }`;
  `prefilter_manifest_structure` returned **`Ok([])`**, so a relying party would read a solution
  binding `?v` to two DIFFERENT terms at once — a disclosed row that does not satisfy the pattern it
  is disclosed under. Unlike L-1 this needs no numeric-type coincidence to be a useful wrong answer:
  both slots hold ordinary IRIs, so the accepted row is a plain false "these are the same entity"
  claim (self-loop / same-as / reflexive-relation queries are exactly where `{ ?v <p> ?v }` is
  written).

  The fix is `verifier::bind_repeated_pattern_slots`, run as stage 2b′ of
  `prefilter_manifest_structure_impl` immediately after `bind_query_correctness` (placed after so the
  existing gates' already-pinned error precedence is unchanged). For each query BGP pattern it groups
  the pattern's variable slots by NAME, and for each variable at more than one slot demands, for
  EVERY active disclosed row of EVERY scan that matches the pattern by constants, that those slots
  carry equal values — compared on canonical big-endian field bytes (`field_hex_eq`), so hex
  spelling/padding differences do not spuriously diverge and a malformed row hex fails CLOSED.
  Rejection is `CheckError::RepeatedSlotMismatch { pattern, proof, row, variable, slots }`. It
  inherits the sq-q9r5e discipline in both directions: per-ROW (the disclosed result IS the scans'
  rows, so one bad row makes the pattern unproven for the disclosed set) and over EVERY
  constant-MATCHING scan rather than the prover's `pattern_scans` declaration — so a query mixing a
  repeated-variable pattern with a distinct-variable pattern at the same constant layout
  (`{ ?v <p> ?v . ?a <p> ?b }`) is over-demanded and REJECTED fail-closed, the same trade L-1 made.
  Queries whose patterns carry no repeated variable (the ordinary case, incl. every other test in the
  suite) are unaffected.

  The `extended-fragment` regime did NOT have this gap and needs no new gate: `bind_fragment_scans`
  binds each variable slot of the selected row either to the disclosed solution (projected vars, via
  `mu`) or to a per-branch existential-coherence map, both keyed by variable NAME across slots, so a
  repeated variable's two slots are compared there by construction.

  Witnesses (`crates/sparq-zk-compose/tests/e2e.rs`):
  `repeated_pattern_var_rejects_a_row_whose_slots_disagree` (the forge — RED before the gate, with
  the pre-fix `Ok([])` recorded above), `repeated_pattern_var_accepts_a_genuine_self_loop_row`
  (positive control: `(alice, knows, alice)` still verifies) and
  `distinct_pattern_vars_are_unaffected_by_the_repeated_slot_gate` (scope control). Mutation-checked
  BOTH ways: making the equality vacuous reds the forge, and inverting it reds the positive control.
  Research-grade, NOT externally audited (sq-qhy4).
- **M-1 / L-2** are already covered by **CR-G8 / sq-j506** and **CR-G9** respectively — no new bead
  (the auditor should weigh gating `DualLeafV1` out of production until sq-j506 is audited).

*This is an AI (GPT-5.6/codex) audit, cross-checked by the orchestrating agent. It reduces risk but is
not a human accredited-cryptographer sign-off and does not close `sq-qhy4`.*
