<!-- [OPUS-4.8] sq-qhy4.1 (child of sq-qhy4 / CR-G1) — Auditor-readiness dossier.
     Consolidation-only auditor package; authored by Opus 4.8 while Fable 5 unavailable —
     re-review when Fable returns. THE most honesty-critical doc class: every soundness
     sentence is obligation/negation framed. NOT a soundness claim, NOT a substitute for
     the external audit (sq-qhy4). -->

# ZK verifier external-audit readiness dossier

> 🤖 SPARQ agent record. **Audit-readiness PREP for `sq-qhy4`** — the external
> accredited-cryptographer audit of sparq's bespoke ZK estate. This document
> **consolidates existing internal material into one auditor-facing package**. It
> **does not claim soundness, does not certify anything, and does not substitute for
> the external audit.** The audit itself requires an external human cryptographer and
> is **out of scope for any agent**; this dossier exists to make that audit
> *actionable and efficient* by gathering scope, obligations, open questions,
> toolchain, and reproduction into one place.

## 0. NOT-YET-SOUND honesty header (read first; do not soften)

**No external accredited cryptographer has reviewed any part of sparq's bespoke
cryptography.** The ZK verifier-soundness *claim* rests entirely on sparq's own
internal, single-model (Opus 4.8, Fable unavailable) self-audits, and `sparq-mpc`
carries **no security guarantee** (semi-honest, honest-majority only; the
collaborative-proof core is deferred behind `NotYetImplemented` stubs). **An external
cryptographer audit is REQUIRED before any ZK/MPC security, privacy, integrity, or
attestation property may be relied upon in production** (gap `CR-G1`, severity
CRITICAL; `gap-register.md`).

The published posture (`SECURITY.md`, §"`sparq-zk` and `sparq-zk-compose` — ZK
verifier: remediated, but NOT externally audited") is the relying-party truth and is
**not overridden** by anything here:

- The *original* internal audit (`research/zk-soundness-audit.md`, 2026-06-13) found
  the v1 verifier **BROKEN** — 12 findings, 6 CRITICAL — because the verifier-side
  binding layer that would make the in-circuit relations mean anything to a third
  party **did not exist**.
- The `sq-1s2` remediation has since **landed** that binding layer, and an internal
  *re-audit* (`research/zk-verifier-reaudit.md`, bead `sq-gbp4`) found the verifier
  *"sound as landed for the threat model the prior audit assumed."*
- That re-audit is **internal, single-model, read-only self-review, with the
  cryptographic-chain forge tests historically `#[ignore]`d out of default CI** — it
  is **necessary but NOT sufficient** to support a production soundness claim.

**The "BROKEN → sound-as-landed" arc is two snapshots of the same codebase, not a
contradiction**: the BROKEN verdict describes the pre-remediation tree; the
sound-as-landed verdict describes the post-`sq-1s2` tree the design records assume is
in force. **Reconciling those two snapshots against the *current* code — and deciding
whether the sound-as-landed verdict survives external adversarial scrutiny — is
precisely the audit `sq-qhy4` exists to perform.** Nothing in this dossier is a
guarantee; every soundness statement below is phrased as an *obligation to be
verified*, not an established property.

> **Containment (honest mitigant, NOT a substitute for the audit).** All three
> bespoke-crypto crates (`sparq-zk`, `sparq-zk-compose`, `sparq-mpc`) are
> `publish = false` and are excluded from `sparq-wasm`; they are never shipped to
> crates.io / npm / PyPI and never enter the browser bundle (CR-13). A downstream
> consumer of the published artifacts cannot reach this code by default. The
> assurance gap is real but contained to an opt-in research surface.

## 1. Scope of the audit

The auditor's target is sparq's **bespoke single-prover ZK query-proof estate**. The
collaborative / multi-prover (MPC) path is **explicitly out of scope for `sq-qhy4`**
(see §7) — it is unbuilt and separately gated.

### 1.1 Crates (Rust)

| Path | Role |
|---|---|
| `crates/sparq-zk` | Primitives: RDFC-1.0 canonicalisation (`canon.rs`), commitment / leaf hashing (`commit.rs`, `poseidon2.rs`), term encoding (`encode.rs`), the BN254 scalar field wrapper (`field.rs`), the Schnorr-over-Baby-JubJub issuer signature (`sig.rs`), salt mint + ingest (`ingest.rs`), the proof trace (`trace.rs`), the issuer-key registry (`registry.rs`). |
| `crates/sparq-zk-compose` | The composition layer and **the verifier under audit**: `verifier.rs` (the binding layer), `driver.rs` (nargo/bb invocation + `canonical_vk`), `manifest.rs`, `derivation.rs`, `holder.rs`, `issuer.rs`, `revocation.rs`, `toml.rs`. |
| `crates/sparq-mpc` | **Out of scope for `sq-qhy4`.** Listed only for boundary clarity (§7). |

### 1.2 The verifier (`crates/sparq-zk-compose/src/verifier.rs`)

This single file (~5.5k lines) is the highest-risk surface — it is the seam between a
prover-supplied manifest JSON and the detached Barretenberg proof. The
audit-relevant symbols (cite **by symbol name**; line numbers move with the tree, so
the auditor should `grep -n` against the checkout rather than trust any number quoted
in the older research docs):

- `reconstruct_public_inputs` — rebuilds the public-input vector from the declared
  manifest and byte-compares it to the proof's; the obligation it discharges is
  audit-finding #1.
- the byte-equality reject site returning `CheckError::PublicInputMismatch`.
- `canonical_vk` (in `driver.rs`) — recomputes the verifying key from the canonical
  compiled circuit; obligation audit-#2 (the prover's `vk` must never be trusted).
- `bind_issuer_attestations` / `bind_hidden_issuer_attestations` — the clear-tier and
  in-circuit-Schnorr issuer attestation gates; obligation audit-#3.
- `bind_query_correctness` — ties op / bound / expected / pattern constants to the
  relying party's query; obligation audit-#5/#6/#10.
- `bind_revocation` / `bind_hidden_revocation` — status-list liveness gates;
  obligation audit-#12.
- `bind_holder_pop` — holder proof-of-possession gate (see §6.2 for its documented
  limit).
- the `SeenNonces` trait + `record_fresh` (declared trait + file-backed impl) — the
  single-use nonce / replay store; obligation audit-#4.
- `derive_id` — re-derives the `CircuitId` (size bucket / parameters); obligation
  audit-#11.

### 1.3 Circuits (Noir; `zk/compose/compose_core/src/*.nr`)

The shared in-circuit relations live in the `compose_core` library; the deployed
binaries are size-bucketed wrappers whose `main` signatures define the public-input
layout the verifier must reconstruct.

| Module | Relation |
|---|---|
| `scan.nr` | Scan completeness + soundness; re-commit to the public commitment; strict-increasing distinct-graph ordering; per-graph attribution bit; row-present. |
| `filter_int.nr` | Non-negative `xsd:integer` value-FILTER: value folded from witnessed canonical digits, token rebuilt + blake3-bound to `operand_enc`, verdict over typed `u64`. |
| `filter_signed.nr` | Signed integer + decimal value-FILTER (same discipline; `-0` rejected). |
| `filter_float.nr` | IEEE-754 (`f64`) value-FILTER over canonical-digit-derived bits. |
| `join.nr` | Cross-credential equality JOIN over full-leaf identity. |
| `issuer.nr` | In-circuit Schnorr-over-Baby-JubJub (on-curve, identity-key rejection, `s < L`, challenge-reduction bind, verify equation, Merkle root) — the hidden-issuer tier. |
| `holder.nr` | In-circuit holder proof-of-knowledge (`holder_pok`: on-curve, identity rejection, `hsk < L`, `hpk == hsk*G`, digest binding). |
| `revoke.nr` | Hidden status-list revocation proof. |
| `hashes.nr` | `commit_fold`, `h2`/`h3` Poseidon2 leaf/commit hashing. |
| `issuer.nr` / `holder.nr` curve gadgets | `scalar_mul`, `point_add`, `assert_on_curve`, `assert_lt_l` (reused across members). |

**Deployed circuit binaries** (size buckets, each with its own `main`):
`scan_k{1,2}_n{16,64}_r{4,8}`, `filter_int_d{1..4}`, `filter_signed_int_d{2,4}`,
`filter_decimal_i3_f2`, `filter_f64`, `filter_f64_d{1..4}`, `join_eq_na{16,64}_nb{16,64}`,
`holder_pok`, `holder_set_d4`, `hidden_issuer_d4`, `revoke_unset_d10`.

### 1.4 The composition seam (the auditor's load-bearing focus)

The audit's central question is **not** "are the in-circuit relations correct?" (the
original audit judged the in-circuit scan/filter relations correct *in-circuit*) but
**"does the verifier-side binding layer make those relations mean what the manifest
claims, against a fully malicious prover?"** Concretely the auditor must adversarially
re-examine that:

1. every prover-supplied manifest field that carries trust is **reconstructed into /
   byte-equalled against** the proof's public inputs (no JSON-only check survives);
2. every trust anchor (verifying key, issuer key set, verifier nonce) is the
   **verifier's**, never the prover's;
3. the `main` `pub` parameter declaration order **is** the reconstruction order, with
   no omitted `pub`, no included private parameter, and no missed `-> pub` return,
   for **every** member including the f64 and k=2 buckets.

## 2. The full obligation set (CR-G1..CR-G8)

This consolidates the gap register (`gap-register.md`, gaps CR-G1..CR-G7) **plus a
new entry CR-G8** derived from the dual-leaf adversarial review (`#793`/`#794`,
`research/zk-dual-leaf-issuer-desync-review.md`). **CR-G8 is introduced here for the
auditor's benefit and is NOT yet reflected in `gap-register.md`** — registering it
there is a tracked follow-up (§8, the §9.3 bead from the review). The auditor should
treat CR-G8 as a design-decision-with-named-cost the dossier surfaces, not an
established register entry.

| ID | Obligation (framed as "what an auditor must verify / what is NOT yet externally established") | Severity | Status |
|---|---|---|---|
| **CR-G1** | The entire estate has **no external accredited-cryptographer review**. All soundness assurance is internal self-review. The auditor must re-run the adversarial pass and (ideally) a machine-checked formal verification of `reconstruct_public_inputs` to circuit-`main` correspondence. | CRITICAL | EXTERNAL-REQUIRED (this dossier is the evidence pack) |
| **CR-G2** | The crypto-chain forge tests were historically `#[ignore]`d out of default CI; closure rested on code-reading + anchors. A toolchain-gated lane (`.github/workflows/zk-toolchain.yml`) now runs them — the auditor must confirm it is green and required-gated. | HIGH | ADDRESSED — verify before relying (`sq-f9tl` closed) |
| **CR-G3** | Public-input empirical anchors were missing for `filter_f64_d*` + k=2 scan members; without them a toolchain bump could silently drift serialization. Probe-captured anchors landed — verify they exist and the lane recaptures-and-compares them. | HIGH | ADDRESSED — verify before relying (`sq-f9tl` closed) |
| **CR-G4** | No FIPS 140-3 validated module; the negative claim is stated (`fips-posture.md`). The auditor need not act, but should note the ZK primitives (BN254 / Poseidon2 / Schnorr-over-Baby-JubJub) are **not** FIPS-approved. | LOW | ADDRESSED → audit-ready negative claim (`sq-cu32` landed) |
| **CR-G5** | No instrumented constant-time / side-channel measurement; a **source-level** review exists (`side-channel-analysis.md`). Present exposure is argued LOW by *architectural placement* (verify path carries no secret; signing is issuance-side), **not** by constant-time primitives. `sq-j3b9` has since removed the square-and-multiply control-flow shape from the two secret-scalar multiplications on the signing path (a sparq-owned fixed-width always-add ladder replaces arkworks' bit-branching `mul_bigint`), but the arkworks **field-op** residual — the actual curve/dep swap — remains open and the "not asserted constant-time" posture is deliberately UNCHANGED. The auditor must judge whether an instrumented `dudect`/`ctgrind` pass is required for the threat model. | P2 | ADDRESSED-by-analysis (`sq-egx6`; hardening beads `sq-u8a8`/`sq-19ej`/`sq-it50`/`sq-7ltf`/`sq-8jv7` landed; `sq-j3b9` PARTIAL — scalar-mul shape only) |
| **CR-G6** | Residual ZK privacy / binding **deferrals** (not soundness breaks): in-circuit salt binding not done (salt verified verifier-side-clear); per-graph salt + list-IRI/version disclosed in the clear (linkability handles); `bind_holder_pop` is not credential-bound (see §6.2). The auditor must confirm these are deferrals, not soundness re-openings. | MEDIUM (privacy) | OPEN — tracked (`sq-hyhj`, `sq-93h`, `sq-i1dt`, `sq-42e3` under `sq-1s2`) |
| **CR-G7** | `sparq-mpc` malicious security / collaborative-proof (M4) is **deferred**: no malicious-security, no distributed-signature-over-secret-shared-witness. Out of scope for `sq-qhy4` (single-prover only). | HIGH (for any MPC reliance) | NOT-SOUND (research-only) / deferred (`sq-bjl`, `sq-34ml`, `sq-ox16`; coZK re-audit `sq-9hrn`) |
| **CR-G8** | **NEW (from `#793`/`#794`).** A *proposed* dual-leaf term encoding (issue `#769`) would **REMOVE an in-circuit invariant the current circuit enforces** — INV-VL (value = parse(committed lexical), enforced against arbitrary/malicious committers) — replacing it with an honest-issuer-for-value assumption. This is a **trust-model regression for the value-FILTER lane**, not a free gate win. The auditor must (a) confirm INV-VL **holds today** in `filter_int.nr`/`filter_signed.nr`/`filter_float.nr`, and (b) decide whether the dual-leaf's honest-issuer-only value lane is acceptable for the target deployments. See §3 and §5.1. | HIGH (if dual-leaf adopted) | DESIGN-DECISION (proposed; not implemented). Register-entry follow-up tracked (§8). |

### 2.1 Cross-document map (so the auditor sees the chain)

- `research/zk-soundness-audit.md` (pre-remediation) — 12 findings (#1-#12), 6
  CRITICAL (#1,2,3,4,5,8), 5 HIGH (#6,7,9,10,11), 1 MEDIUM (#12) + 7 hardening
  recommendations + 7 refuted/guarded attacks. **Verdict: BROKEN.**
- `research/zk-verifier-reaudit.md` (post-`sq-1s2`, bead `sq-gbp4`) — all 12 findings
  dispositioned **CLOSED** with a per-finding reject-path + forge-test map; two **LOW
  deferrals** added: NEW-1 (test/CI anchors — folded into CR-G2/CR-G3) and NEW-2
  (privacy/holder-binding — folded into CR-G6). **Verdict: "sound as landed for the
  assumed threat model" — internal, single-model.**
- `research/mpc-cozk-reaudit.md` (bead `sq-9hrn`) — collaborative path **RE-OPEN /
  gated**; encodes requirement R-WV + test obligations T1-T4 + clause C; states
  explicitly that **`sq-qhy4` audits the single-prover verifier only and does NOT
  discharge the multi-prover obligation** (CR-G7 / §7).
- `research/zk-dual-leaf-issuer-desync-review.md` (`#793`/`#794`) — source of CR-G8.

## 3. The must-keep soundness constraint set

This is the constraint set §3.4 of `research/zk-age-gatecount-reduction.md` enumerates
as **must survive any reduction** — reproduced as the auditor's invariant checklist.
**These are obligations the auditor must verify hold (and continue to hold under any
proposed optimisation), NOT properties this dossier asserts as proven.** File:line
citations in the source doc were verified against the checkout *at that time*; the
auditor should re-confirm by symbol.

### A. Operand bound to the signed/committed credential (not attacker-chosen)

- **A1 — operand binding.** The compared value is a *constrained function* of the
  committed literal bytes, asserted equal to `operand_enc`
  (`filter_int.nr` `assert_eq(h2(LITERAL, hs), operand_enc, …)`), not a free witness.
- **A2 — `operand_enc` is the field the scan proof bound to a committed triple.** The
  verifier's `binding_consistency` edge rejects (`BindingInconsistent`) unless
  `scanned == operand`. Removing this edge would let a FILTER prove `>=18` about a
  literal from no credential.
- **A3-A7 — scan binds the witnessed graph to the public commitment.** Re-commit
  (`commit_fold(leaves, counts[g]) == commitments[g]`), disclosed-row-present,
  completeness, strictly-increasing `commitments[g-1] < commitments[g]` (anti
  duplicate-inclusion / COUNT forgery, mirrored verifier-side), `attribution[g] ==
  graph_matches`.
- **A8 — commitment signed by a trusted issuer (clear tier).**
  `bind_issuer_attestations(manifest, trusted_key_set, hidden_covered)` against the
  **external** `trusted_key_set` (never `manifest.key_set`); fail-closed "neither
  clear nor hidden ⇒ reject".
- **A9 — hidden-issuer tier in-circuit Schnorr.** `issuer.nr`: on-curve, identity-key
  rejection, `s < L` (`assert_lt_l`), challenge-reduction bind, verify equation,
  Merkle root.

### B. The comparison `value >= bound` is correct over the field (no wraparound)

- **B1 — operands in a range-checked integer domain; no signed-field wrap.**
  Magnitudes accumulate into `u64`; static overflow guards `D<=19` / `MD<=19` /
  `ID+FD<=19` are must-keep. A reduction must **not** collapse this into a single
  signed `Field`.
- **B2 — verdict correct and constant-shape.** All six predicates evaluated
  unconditionally over typed `u64`, no secret-dependent branch. Lowering to `Field`
  arithmetic reintroduces modular wrap and is **rejected**.
- **B3 — verdict asserted equal to public `expected`.**
- **B4 — canonical-lexical discipline.** digit-range, no-leading-zero, `-0` rejected,
  op-in-range — these protect A1 by forbidding a non-canonical token that re-encodes
  the same value.

### C. Public inputs bind correctly (no malleability)

- **C1 — verifier-side public-input reconstruction byte-matches the proof.**
  `if reconstructed != art.public_inputs { return Err(PublicInputMismatch) }`;
  per-variant serialization order must match `main`'s `pub` order. **The load-bearing
  tie between the JSON statement and the detached proof.**
- **C2 — canonical verifier-side vk, never the prover's** (`canonical_vk(id, …)`
  recomputed from the re-derived `CircuitId`).
- **C3 — field 0 is the verifier nonce, and the declared binding equals it**
  (`NonceBindingMismatch` otherwise).
- **C4 — query-correctness binding** (`bind_query_correctness` +
  `bind_attributions`): op / bound / expected / pattern constants match the relying
  party's query.

### D. Nullifier / uniqueness / replay

> **Naming note for the auditor.** The codebase has **no construction named
> "nullifier."** The functional analogue is a **single-use verifier-nonce store**;
> the obligation below is what an auditor would check a nullifier scheme for.

- **D1 — single-use nonce, burn-on-present.** The `SeenNonces` / `record_fresh` store
  records the nonce before the crypto gate; a rejection is never a free retry. The
  nonce is public-input field 0 on **every** member and byte-bound (C3).
- **D2 — holder possession.** clear tier `bind_holder_pop`; in-circuit hidden tier
  `holder.nr` `holder_pok` (on-curve, identity rejection, `hsk < L`, `hpk == hsk*G`,
  digest binding) — **NOT-yet-sound / opt-in** (see §6.2).

### Reject list (constraints a mis-implemented reduction would TRADE)

A reduction that (i) lowers the `u64` comparison to `Field`-arith (re-introduces
modular wrap — violates B1/B2); or (ii) drops the canonical-form binds when moving to
a numeric lane (lets a prover bind a second encoding of the same value — violates
A1/B4); or (iii) takes the operand as a free witness/public input without anchoring
it to the scan-bound commitment (violates A1/A2); or (iv) feeds Baby-JubJub
coordinates to a Grumpkin native MSM black-box (verifies on the wrong curve —
violates A9) — each **trades soundness for gates and is REJECTED.**

## 4. The threat model

> **Important reconciliation for the auditor.** `research/threat-model.md` is the
> **production-core STRIDE model only** (trust boundaries B1-B5 are *infrastructure*
> boundaries — RDF bytes, SPARQL string, HTTP, SERVICE federation, mmap loader) and it
> **explicitly places the ZK/MPC estate OUT OF SCOPE**, deferring to the ZK estate's
> own adversarial model. **There is no consolidated B-numbered ZK/MPC trust-boundary
> document.** The ZK trust model below is synthesised from the design records' own
> `## 1` sections and the soundness audits; ZK obligations are keyed to
> **audit-finding numbers (audit-#1..#12)**, not B-numbers.

### 4.1 Parties

| Party | Trusted for | NOT trusted for / adversary capability |
|---|---|---|
| **Issuer** | Honest at issuance. Signs credentials offline (one-time); trusted to bind the correct subject and (today) the value-to-lexical agreement is *machine-enforced*, not merely trusted (INV-VL, §5.1). Its Schnorr key is in the verifier's external `KeySet K`, **never prover-supplied**. | A *malicious / compromised* issuer is the locus of CR-G8: under the proposed dual-leaf it could desync value from lexical. A canonicalisation bug at sign time is a residual. |
| **Holder / prover** | Holds the long-lived holder secret `hsk` (`hpk = hsk*G`). In the single-prover join model holds *both* credentials. | **Fully adversarial / malicious-prover model.** Writes the manifest JSON, chooses public inputs, runs the prover; goal: `verify_manifest` returns `Ok(())` for a *false* statement. Includes a holder A who obtained a valid presentation for a different subject B (device seizure, collusion, wire capture). |
| **Verifier / relying party** | Honest, **fail-closed**. Anchors trusted issuer keys (`KeySet K`), issues a fresh per-presentation nonce, runs `verify_manifest`. | Must learn *that* a statement holds, not the hidden value; for a deterministic encoding may be able to dictionary-attack (a documented residual). |

### 4.2 Standing assumption (load-bearing, NOT externally validated)

Every design record assumes the audit-#1/#2 cryptographic gate (public-input
reconstruction + canonical-vk pinning) and the audit-#3/#4/#9/#12 anchors are "in
force post-`sq-1s2`." **That assumption is exactly what `sq-qhy4` must independently
validate; it is not yet externally established.**

## 5. Open questions an auditor must answer

These are the genuinely-open questions, framed as obligations. None is asserted
resolved here.

### 5.1 The INV-VL value-to-lexical regression (CR-G8) — the headline open question

- **Confirm INV-VL holds today.** In `filter_int.nr`/`filter_signed.nr`/
  `filter_float.nr` the compared value and the committed lexical form are derived from
  **one witness** (the canonical digits): the value is folded from the digits *and*
  the same digits are rebuilt into the token, blake3-hashed, and asserted equal to the
  issuer-signed `operand_enc`. The reviewer must verify that this means **today, even
  a fully malicious issuer cannot produce a leaf that compares as 18 under a
  value-FILTER while its lexical/term identity is "5"** — there is no desync state
  reachable today. (Call this INV-VL: *value = parse(committed lexical), enforced
  in-circuit against an arbitrary committer.*)
- **Confirm the dual-leaf would DELETE INV-VL.** The proposed dual-leaf FILTER member
  introduces two *independent* witnesses (a value hook and a lexical-component
  witness); nothing constrains `value_hook == parse(preimage(lexical_component))`
  in-circuit without re-introducing the lexical parse the change exists to delete. The
  reviewer must judge that this is a **strict trust-model regression for the
  value-FILTER lane** (value becomes sound *only* under an honest-issuer-for-value
  assumption), not "no new capability."
- **Decide acceptability.** Is the value-FILTER lane acceptable as
  honest-issuer-only? Or must it be gated on canonical-issuance conformance from the
  start? Are there target deployments that trust an issuer for *identity* but not for
  *value*, or that worry about issuer-key compromise / insider risk? Those are exactly
  the models INV-VL protects and the dual-leaf weakens.
- **Note the broadened attack surface.** The disclosed scan rows are PUBLIC, so a
  desynced lexical side can be consumed *anywhere* in the pipeline (audit log,
  downstream non-ZK consumer), not just inside a second in-circuit identity gadget;
  and the join lane (`join.nr`, full-leaf equality) makes a one-malicious-issuer +
  one-honest-issuer cross-credential desync directly exploitable.
- **Confirm the proposed mitigations bound — but do not eliminate — the regression.**
  Host-side same-leaf co-binding at ingest makes *honest* sparq ingest desync-free and
  desync a detectable protocol violation, but **cannot** defend against a malicious
  issuer running a patched committer (that is INV-VL's irreducible loss);
  canonical-issuance conformance is the only mechanism that *recovers* value-to-lexical
  agreement as an issuance invariant.

### 5.2 Holder-PoK / hidden-issuer in-circuit Schnorr

- **Credential-binding gap (the headline).** The currently-shipped `bind_holder_pop`
  checks only that the holder is in the registry and proves a Schnorr PoP over the
  nonce — it reads **no commitment, attestation, or issuer signature**, so it binds
  the *nonce*, not the *credential*. The auditor must confirm this means **among
  trusted holders, one could present another's credential**, and that closing it
  (the `zk-holder-pop-design.md` design, bead `sq-c2ql`) requires the
  `holder_pk_digest` to be folded **into the issuer signature** (a new domain tag),
  never read from a prover JSON field — a JSON-only check is **audit-#1 reopened**.
- **In-circuit hidden-key tier (`holder.nr` `holder_pok`).** The auditor must verify
  the circuit asserts: on-curve `hpk`; identity-key rejection
  (`!((hpk.x == 0) & (hpk.y == 1))`); `assert_lt_l(hsk)` (canonical scalar, `s < L`);
  `derived == hpk` from the scalar-mul; and `h2(hpk) == holder_pk_digest`; and that
  `holder_pk_digest` + `challenge` are folded into the reconstructed public-input
  vector and byte-equalled (audit-#1), vk selected by re-derived `CircuitId`
  (audit-#2).
- **Honest limits the auditor must not over-read.** It does **not** stop a holder
  sharing its own secret (non-transferability is out of scope); does **not** prove the
  holder is a human; the clear-key tier does **not** hide *which* holder (linkability);
  it does **not** retroactively fix bearer credentials.
- **Status.** The credential-bound holder PoK is **design-for-review / not yet
  implemented**; opt-in by policy when built. Gate count unmeasured (must be measured
  + regression-gated before any figure is published).

### 5.3 Nullifier / replay

- The single-use property is delegated to the audit-#4 nonce store inside
  `verify_manifest`; the auditor must verify that store actually enforces single-use
  (burn-on-present, fail-closed) and that field-0 byte-equality holds for **every**
  member including any new ones (`holder_pok`, `join_eq`, value-lane FILTER).
- No explicit nonce-expiry / replay-window policy is specified; no cross-verifier
  domain separation of the challenge is discussed. The auditor must judge whether a
  single nonce accepted by two distinct relying parties is a concern for the threat
  model.

### 5.4 Public-input binding (the recurring failure class)

- The auditor must verify that **no** trust-relevant field (issuer attestation,
  commitments, FILTER op/bound/expected, attributions, join slots, holder digest,
  any value-lane encoding) is checked **only** against the prover JSON without being
  byte-bound into the reconstructed public inputs — this is the precise failure class
  the re-audit (bead `sq-gbp4`) exists to catch.
- The auditor should confirm the `main` `pub` declaration order **is** the
  reconstruction order for every member, and that the empirical anchors
  (captured from real bb) for `filter_f64_d*` and the k=2 scan members exist and are
  recaptured-and-compared by the toolchain lane (CR-G3).

### 5.5 Issuer Schnorr-over-Baby-JubJub + side channels

- The auditor must independently judge the in-circuit Schnorr relation in `issuer.nr`
  (on-curve, identity rejection, `s < L`, challenge reduction, verify equation, Merkle
  root) and the host signing path (`sparq-zk/src/sig.rs`).
- The side-channel posture is **source-level only** (`side-channel-analysis.md`): the
  host `derive_nonce` degenerate-`k` guard was made branchless, but the Baby-JubJub
  scalar multiplication and scalar arithmetic use arkworks ops sparq does **not**
  assert constant-time. The auditor must decide whether an instrumented timing study
  is required (signing is issuance-side / trusted environment today, but becomes
  load-bearing if signing moves to an exposed online surface).

## 6. What is already in place (cite as evidence, do not re-raise as gaps)

To prevent re-litigation, the auditor should treat these as existing controls (their
*sufficiency* is exactly the open question CR-G1 closes):

- The per-finding **CLOSED** dispositions for all 12 historical findings, each with a
  reject path + (mostly) a forge test — `research/zk-verifier-reaudit.md`
  §"Per-finding dispositions" + the 1:1 finding-to-forge-to-reject-path map.
- The negative-test estate: `crates/sparq-zk-compose/tests/forge_gates.rs`
  (forge-and-verify), `differential_fuzz.rs` (cleartext-vs-circuit differential),
  `gate_count.rs` + `gate_count_snapshot.json` (circuit-size ratchet), `e2e.rs`,
  `audit_forge_map.rs`, `holder_pok_binding.rs`, `holder_pop_forge.rs`,
  `join_forge.rs`, `verifier_errors.rs`, and `crates/sparq-zk/tests/`
  (Poseidon2 / RDFC10 cross-vectors).
- The toolchain-gated CI lane `.github/workflows/zk-toolchain.yml` that runs the
  forge + real-bb anchor suite on zk changes.
- Tier-A sound primitives (out of the unsound surface): Sigstore build-provenance,
  SHA-256 release digests, RDFC-1.0 canonicalisation, OS-CSPRNG sourcing.
- Containment: `publish = false` + wasm exclusion (CR-13).

### 6.2 The documented holder-binding limit (cite as a KNOWN deferral, CR-G6 / NEW-2a)

`bind_holder_pop` proves possession of a *trusted holder key* over the nonce but does
**not** bind it to the specific credential. This is a documented deferral (re-audit
NEW-2a), not a soundness re-opening — a revoked/forged credential still fails, an
untrusted holder still fails. Closing it is bead `sq-c2ql` (§5.2).

## 7. The MPC / collaborative boundary (out of scope for `sq-qhy4`)

`sq-qhy4` audits the **single-prover verifier only**. The collaborative / multi-prover
path is separately gated and **must not be folded into this audit**:

- `sparq-mpc` provides **no confidentiality, correctness, attestation, or
  malicious-security guarantee** today; it is honest-majority **semi-honest** only.
  Every collaborative-proof method returns `MpcError::NotYetImplemented` (contract-
  tested) — the honest fail-closed state.
- The coZK re-audit (`research/mpc-cozk-reaudit.md`, bead `sq-9hrn`) holds all four
  CRYPTO'25-eprint-2025/1026 lenses **RE-OPEN / gated**, encodes requirement R-WV
  (validate-the-shared-extended-witness-before-any-open) + test obligations T1-T4 +
  clause C, and states **explicitly that `sq-qhy4` does NOT discharge the multi-prover
  obligation.** A collaborative-proving soundness claim is BLOCKED until R-WV + T1-T4 +
  C all pass, the honest-majority malicious-security MPC line lands, **and** a separate
  external cryptographer audit covers the multi-prover construction.

## 8. The proposed CR-G8 register follow-up (tracked, doc-only)

The dual-leaf review recommends (its §6/§8, doc-only, no code) that **before the
auditor sees the dual-leaf draft**, the design's threat-model framing be corrected to
state the INV-VL **removal** explicitly (not "no new capability / bounded residual"),
and that a CR-G8 entry be added to `gap-register.md`. This dossier introduces CR-G8
for the auditor; the register edit + the host-side co-binding / desync-detection
implementation work are **future beads under `sq-1s2`**, ordered:

1. Amend the dual-leaf draft's framing + add the CR-G8 entry to `gap-register.md`
   (state the INV-VL removal explicitly; obligation/negation framing so the
   privacy-claims gate passes). Doc-only; land before the auditor reviews the draft.
2. Host same-leaf co-binding at ingest (`encode.rs`/`commit.rs` compute and assert
   `value == parse(canonical(lexical))`, fail-closed; tested).
3. Desync-detection + irreducibility tests (ingest rejects a desynced leaf; documented
   doc-test that the verifier cannot detect post-commit desync). Depends on 2.
4. Elevate canonical-issuance conformance to a named value-lane precondition (the
   value-FILTER lane is honest-issuer-only until a conformance mechanism exists).

All four are audit-gated behind `sq-qhy4` for any production reliance.

## 9. Toolchain + reproduction

**Toolchain pins** (the baselined snapshot toolchain, from
`.github/workflows/zk-toolchain.yml`): `nargo 1.0.0-beta.21`,
`bb 5.0.0-nightly.20260324` (Barretenberg / UltraHonk). The auditor must reproduce on
these pins; a version bump can silently drift the public-input serialization, which is
exactly what the empirical anchors guard against.

> **Reproduction-environment caveat (honesty).** Any wall-clock timing the auditor
> observes on a developer/work-box is **NON-canonical** and must not be recorded as a
> canonical result. The circuit-size figures the repo gates (`gate_count_snapshot.json`)
> are a deterministic *circuit metric*, not a performance number; this dossier does not
> reproduce any figure inline (the snapshot file is the source of truth).

### 9.1 Build + verify the verifier (Rust)

- Build/test the two crates in **both feature states**:
  `cargo test -p sparq-zk -p sparq-zk-compose` (default), and with the
  toolchain-gated features that enable the real-bb e2e + forge suites (see the lane
  env in `zk-toolchain.yml`; the crypto-chain tests were historically `#[ignore]`d and
  require nargo/bb on `PATH`).
- The load-bearing reject-path tests:
  `crates/sparq-zk-compose/tests/forge_gates.rs` — one forge-and-verify negative per
  historical CRITICAL (statement substitution, verdict substitution, unsigned/invalid
  issuer signature, issuer-key-not-in-external-K, nonce-binding-mismatch, nonce-replay,
  attribution-omitted / under-declared, revocation revoked-bit / stale-version).

### 9.2 Compile circuits + measure gates (Noir / bb)

- Per circuit binary under `zk/compose/<member>/`: `nargo compile`, then
  `bb gates -s ultra_honk` to read the `circuit_size`. The repo ratchets these via
  `crates/sparq-zk-compose/tests/gate_count.rs` against
  `gate_count_snapshot.json`. **Always run `bb gates` before trusting any gate
  figure** (`nargo info` alone is misleading; see the `noir-optimisation` skill).

### 9.3 The differential prove-to-verify-to-cleartext fuzzer

- `crates/sparq-zk-compose/tests/differential_fuzz.rs` is the cross-check that the
  in-circuit verdict agrees with an independent cleartext evaluation over randomized
  inputs — i.e. honest inputs prove-and-verify, lying inputs are rejected, and the
  circuit's accept/reject matches the cleartext oracle. Combined with the real-bb e2e
  cases (`e2e.rs`) this exercises the full prove-to-verify path on the pinned
  toolchain. The auditor should run it on the pinned `nargo`/`bb` and confirm the
  fuzzer's honest/lie corpora (`zk/compose/<member>/Prover_*.toml`) still partition
  accept/reject as expected.

## 10. What an external auditor still must do (this is external; an agent cannot)

This dossier is the **evidence pack**, not the audit. The audit `sq-qhy4` requires an
external, accredited human cryptographer to:

1. **Re-run the adversarial soundness pass** against the *current* tree, treating the
   internal "sound as landed" verdict as an unverified hypothesis — not a baseline to
   trust.
2. **Independently judge the verifier binding layer** — that `reconstruct_public_inputs`
   to each circuit `main` correspondence is exact and complete, the byte-equality is the
   only acceptance path, and `canonical_vk` / `KeySet K` / the nonce store are
   verifier-anchored (audit-#1..#4, the must-keep §3 constraints).
3. **Judge the Schnorr-over-Baby-JubJub scheme** (host `sig.rs` + in-circuit
   `issuer.nr` / `holder.nr`) — domain separation, nonce derivation, `s < L`, on-curve,
   identity rejection, challenge reduction — and the constant-time posture.
4. **Resolve the CR-G8 / INV-VL question** (§5.1): confirm INV-VL holds today, confirm
   the dual-leaf would remove it, and rule on whether an honest-issuer-only value lane
   is acceptable for the target deployments.
5. **Rule on the holder-binding, nullifier/replay, and public-input-binding open
   questions** (§5.2-§5.4).
6. **Ideally**, deliver a machine-checked formal verification of the
   `reconstruct_public_inputs` to circuit-`main` correspondence (the current assurance is
   code-reading + empirical golden-vector anchors + the forge suite — strong testing,
   not proof).
7. **Sign off (or not).** Only on external sign-off may any ZK soundness / privacy /
   integrity / attestation property be relied upon in production; until then
   `SECURITY.md`'s conservative posture — remediated but externally unaudited, no
   production guarantee — stands.

The MPC / collaborative path (§7) is a **separate** external obligation and is not
discharged by `sq-qhy4`.

---

> **Closing honesty statement (load-bearing).** Nothing in this dossier is a security
> guarantee. sparq's v1 ZK verifier is remediated and internally re-audited but **NOT
> externally audited**, documented **NOT-yet-sound** for production reliance
> (`sq-qhy4`, `sq-9hrn`, `sq-1s2`; `SECURITY.md`; `gap-register.md` CR-G1). External
> accredited-cryptographer sign-off (`sq-qhy4`, P0) is **REQUIRED** before any ZK
> soundness / privacy / integrity / attestation property may be relied upon. The MPC
> estate is semi-honest-only and out of scope for `sq-qhy4`. Every soundness sentence
> above is an **obligation to be verified**, not an established property. This dossier
> **consolidates** existing internal material into an auditor package; it does **not**
> claim soundness and does **not** substitute for the audit. [OPUS-4.8]
