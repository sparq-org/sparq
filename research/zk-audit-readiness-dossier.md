<!-- [OPUS-4.8] sq-68dee (WS-2, child evidence-assembly for gate sq-qhy4) — authored by
     Opus 4.8 while Fable 5 unavailable; re-review when Fable returns. THE most
     honesty-critical doc class. This is an EVIDENCE-ASSEMBLY record, NOT an audit and
     NOT a soundness claim. sq-qhy4 (the external accredited-cryptographer audit) being
     OPEN is exactly why this dossier exists — to make that audit maximally efficient when
     the maintainer commissions it. Every property below is stated with its VERIFICATION
     STATUS, never as "sound"/"secure"/"audited"/"production-ready". -->

# ZK audit-readiness dossier (WS-2) — circuit inventory + per-claim verification-status map

> 🤖 **SPARQ agent** — research/design-for-review record. Author: Opus 4.8 (Fable
> unavailable — flag for re-review when Fable returns). **Evidence assembly for the OPEN
> external-audit gate `sq-qhy4`.** This document **claims nothing sound, secure, audited,
> or production-ready.** It inventories the ZK estate and maps every security-relevant
> property to *how it is currently verified* (internal re-audit reference / test / static
> analysis / UNVERIFIED / KNOWN-GAP), so the eventual external accredited-cryptographer
> audit is actionable. The audit itself requires an external human cryptographer and is
> **out of scope for any agent**.

## 0. READ FIRST — honesty header (do not soften)

**No external accredited cryptographer has reviewed any part of sparq's bespoke
cryptography.** Every soundness-relevant statement below is phrased as an *obligation to be
verified*, not an established property. The relying-party truth is the published posture in
[`SECURITY.md`](../SECURITY.md) §"`sparq-zk` and `sparq-zk-compose` — ZK verifier:
remediated, but NOT externally audited", and it is **not overridden by anything here**:

- The **original** internal audit ([`research/zk-soundness-audit.md`](./zk-soundness-audit.md),
  2026-06-13) found the v1 verifier **BROKEN** — 12 findings — because the verifier-side
  binding layer did not exist.
- The **`sq-1s2` remediation** (17 commits) has since landed that binding layer, and an
  **internal re-audit** ([`research/zk-verifier-reaudit.md`](./zk-verifier-reaudit.md), bead
  `sq-gbp4`) found the verifier *"SOUND as landed for the threat model the prior audit
  assumed."* That re-audit is **internal, single-model, read-only self-review** — necessary
  but **NOT sufficient** for a production soundness claim.
- **The "BROKEN → sound-as-landed" arc is two snapshots of the same codebase, not a
  contradiction** — the BROKEN verdict describes the pre-remediation tree, the sound-as-landed
  verdict describes the post-`sq-1s2` tree. **Reconciling those against the current code, and
  deciding whether the sound-as-landed verdict survives external adversarial scrutiny, is
  precisely the audit `sq-qhy4` exists to perform.**
- `sparq-mpc` carries **no security guarantee** — semi-honest, honest-majority only; the
  collaborative-proof core is deferred behind `NotYetImplemented` stubs (out of scope for
  `sq-qhy4`; see §7).

**Containment (honest mitigant, NOT a substitute for the audit).** All three bespoke-crypto
crates (`sparq-zk`, `sparq-zk-compose`, `sparq-mpc`) are `publish = false` and are excluded
from `sparq-wasm`; they never ship to crates.io / npm / PyPI and never enter the browser
bundle. A downstream consumer of the published artifacts cannot reach this code by default.
The assurance gap is real but contained to an opt-in research surface.

## 0.1 Relationship to the existing audit-readiness package (honesty about scope)

**A correction to this bead's implicit premise: an audit-readiness package already exists.**
This WS-2 document is a **complementary companion**, NOT a from-scratch artifact and NOT a
replacement:

- [`compliance/cryptoreview/audit-readiness-dossier.md`](../compliance/cryptoreview/audit-readiness-dossier.md)
  (`sq-qhy4.1`) is the **auditor-facing scope + obligation package** — scope, the CR-G1..CR-G9
  obligation set, the must-keep constraint set, the threat model, toolchain + reproduction, and
  "what an external auditor still must do".
- [`compliance/cryptoreview/gap-register.md`](../compliance/cryptoreview/gap-register.md) is
  the authoritative **gap table** (CR-G1..CR-G9 with severities + dispositions + tracking
  beads). It is the single source of truth for the gaps; §3 below **references** those codes
  rather than re-deriving them.

This WS-2 dossier adds the two things those documents summarize but do not tabulate at
circuit granularity: **(1) a stable-identifier CIRCUIT INVENTORY** (§1) and **(2) a
per-security-property VERIFICATION-STATUS MAP** (§2) that names, for each property, the exact
internal-re-audit reference / test / static-analysis pass / UNVERIFIED marker that currently
stands behind it. §4 tabulates the standing **forge-map / adversarial-test coverage** (per the
`sq-l9ulg` template). §5 is the numbered **QUESTIONS FOR THE EXTERNAL AUDITOR**. Where this
document and the two `compliance/cryptoreview/` records overlap, **those records win on scope
and gap-severity; this document is the circuit-level index into them.**

---

## 1. CIRCUIT INVENTORY

The verifier and its circuit family. Stable identifiers use the prefix `CI-` (this document's
circuit-component ids) and reference `CircuitId` (the Rust enum in
`crates/sparq-zk-compose/src/manifest.rs`) where applicable. **Ground truth (`main`,
verified against the code at authoring time):** the family is **13 circuit KINDS** compiled to
**31 on-disk member packages** under `zk/compose/`.

### 1.1 Verifier + host-side components (Rust)

| Id | Component | Location | Purpose | Verification status pointer |
|---|---|---|---|---|
| **CI-V** | `verify_manifest` | `crates/sparq-zk-compose/src/verifier.rs` (~5.7k LoC) | The relying-party entry point. Runs stage 1 (structural), stage 2 (crypto-HOST: circuit-id re-derivation, binding-edge consistency, query-correctness binding, cross-graph attribution, issuer Schnorr signatures, salt-uniqueness, revocation), then stage 3 (`reconstruct_public_inputs` byte-reconstruction + `bb verify`) per sub-proof. | §2 rows PI-BIND, VK-CANON, ISSUER-SIG, REPLAY, FILTER-BIND, ATTRIB, SALT, REVOKE |
| **CI-COMMIT** | Commitment scheme | `crates/sparq-zk/src/commit.rs` | `C(G)` = Poseidon2 fold over per-triple leaves. Three `CommitmentMethod`s: `StringCanonicalV1` (default, only method implemented end-to-end; blake3 over canonical N-Triples token), `DualLeafV1` (config-only, leaf encoding `sq-j506` NOT implemented), `ValueOnlyV1` (feature-gated research dial). | §2 row COMMIT-BIND; §3 CR-G8/CR-G9 |
| **CI-POS2** | Poseidon2 host port | `crates/sparq-zk/src/poseidon2.rs` (+ `poseidon2_constants.rs`) | Bit-compatible Rust port of `noir-lang/poseidon` **v0.3.0** `Poseidon2::hash` (BN254), cross-tested against `nargo`. | §2 row POS2-XVEC |
| **CI-SIG** | Schnorr-over-Baby-JubJub | `crates/sparq-zk/src/sig.rs` | Issuer / holder signature scheme; verifier checks issuer signatures over `C(G)` against an external trusted key set. | §2 rows ISSUER-SIG, SIG-CT; §3 CR-G5 |
| **CI-REG** | `<urn:sparq:zk>` registry | `crates/sparq-zk/src/registry.rs` | Per-credential entry (`zk:commitment`, `zk:issuerPublicKey`, `zk:commitmentSignature`, `zk:statusList[Index/Version]`, `zk:rdfc10Salt`, ...) the prover reads; fail-closed loader. | §2 row REG-FAILCLOSED |
| **CI-DUAL** | Dual-leaf handles | `crates/sparq-zk/src/dual_leaf.rs` | Value-handle + lexical-identity encoding helpers for the `DualLeafV1` method. Config-only today. | §3 CR-G8 |

### 1.2 In-circuit relations (Noir; `zk/compose/compose_core/src/*.nr`)

The 13 KINDS (the `CircuitId` variants). Each KIND compiles to one or more members; a proof
verifies **only** against the member whose const-generic parameters its witnesses fit
(EXACT-match discipline, `sq-wto`). **I/O convention:** the first public input of every member
is `challenge: pub Field` (a verifier-supplied tag, byte-bound to the proof by the host-side
`reconstruct_public_inputs`); the remaining public inputs are the `ProofInputs` variant fields
in `main` declaration order. **Commitment scheme:** Poseidon2 (`noir-lang/poseidon` v0.3.0) for
commitment folds and Merkle trees; `blake3` for canonical-token binding on the string-canonical
FILTER lanes.

| Id | KIND (`CircuitId`) | Members on disk | Purpose (in-circuit relation) | Public inputs (beyond `challenge`) |
|---|---|---|---|---|
| **CI-SCAN** | `Scan { k, n, r }` | `scan_k{1,2}_n{16,64}_r{4,8}` (8) | BGP triple-pattern scan over `k` graphs, `n` slots/graph, `r` disclosed rows: commitment recompute + scan completeness/soundness. | commitments, pattern (const flags + const enc), rows, row_count |
| **CI-FINT** | `FilterInt { d }` | `filter_int_d{1..4}` (4) | Hidden non-negative `xsd:integer` FILTER, `d` digits; operand bound to committed literal via canonical blake3 token. | operand_enc, op, bound, expected |
| **CI-FF64** | `FilterF64 { d }` | `filter_f64_d{1..4}` (4) + raw `filter_f64` (1) | Hidden `xsd:double` FILTER over the INTEGER-VALUED double fragment; IEEE bits derived in-circuit from the bound value (no prover-free `a_bits`). Raw `filter_f64` is the free-bits building block (non-composable). | operand_enc, op, bound, expected |
| **CI-FSINT** | `FilterSignedInt { md }` | `filter_signed_int_d{2,4}` (2) | Hidden SIGNED `xsd:integer` FILTER, `md` magnitude digits; sign-aware compare over `u64` magnitude. | operand_enc, op, bound, expected |
| **CI-FDEC** | `FilterDecimal { id, fd }` | `filter_decimal_i3_f2` (1) | Hidden `xsd:decimal` FILTER, `id` integer + `fd` fraction digits; fixed-point compare against a HOST-PRESCALED public bound. | operand_enc, op, bound_scaled, expected |
| **CI-FVDL-I** | `FilterValueDl` *(feature `dual-leaf`)* | `filter_value_dl_int` (1) | Dual-leaf value-lane FILTER over committed non-negative `xsd:integer`; binds operand via two Poseidon2 permutations over `VALUE_HOOK`, `lexical_component` a FREE witness (no in-circuit blake3). | operand_enc, op, bound, expected |
| **CI-FVDL-F** | `FilterValueDlF64` *(feature `dual-leaf`)* | `filter_value_dl_f64` (1) | Dual-leaf value-lane FILTER over committed `xsd:double`; canonicalises IEEE bits in-circuit (−0.0→+0.0, NaN→canonical qNaN) before forming `value_component`. | operand_enc, op, bound, expected |
| **CI-FVDL-D** | `FilterValueDlDecimal` *(feature `dual-leaf`)* | `filter_value_dl_decimal` (1) | Dual-leaf value-lane FILTER over committed `xsd:decimal` at canonical scale; scale folded into public `datatype_const`. | operand_enc, op, bound, expected, datatype_const |
| **CI-REVOKE** | `RevokeUnset { depth }` | `revoke_unset_d10` (1) | Hidden-index status-list inclusion + bit-unset over a depth-`depth` Poseidon2 Merkle tree; index/leaf-bit/path PRIVATE. | status-list Merkle root |
| **CI-HISS** | `HiddenIssuer { depth }` | `hidden_issuer_d4` (1) | In-circuit Schnorr-over-Baby-JubJub verify + hidden-key set membership over a depth-`depth` Poseidon2 Merkle tree of key set K; proves "signed by SOME key in K" without disclosing which. | commitment message `m`, key-set Merkle root |
| **CI-HPOK** | `HolderPok` | `holder_pok` (1) | In-circuit holder Proof-of-Possession: `hpk = hsk·G` (Baby-JubJub, one scalar-mul) AND `Poseidon2([ZKSIG_HK, hpk.x, hpk.y]) == holder_pk_digest`; hsk + key PRIVATE. | issuer-attested `holder_pk_digest` |
| **CI-HSET** | `HolderSet { depth }` | `holder_set_d4` (1) | Hidden-holder SET membership: `hpk = hsk·G` AND depth-`depth` Poseidon2 Merkle membership of the holder-key digest; proves "SOME holder in the set" without disclosing which. | holder-set Merkle root |
| **CI-JOIN** | `JoinEq { n_a, n_b }` | `join_eq_na{16,64}_nb{16,64}` (4) | Hidden cross-credential JOIN: `row_a[slot_a] == row_b[slot_b]` without disclosing the joined term. **SCHEMA ONLY** — the verifier gate `bind_joins` (step 4, `sq-sfsi`) is NOT wired. | commit_a, commit_b, join_commitment, slot_a, slot_b |

**Member count check:** 8 + 4 + (4+1) + 2 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 4 = **31 packages**
(the `dual-leaf` members are compiled but the corresponding `CircuitId` variants are
feature-gated in Rust). This matches the on-disk `zk/compose/` listing.

### 1.3 Externalized Noir dependencies (out-of-tree face repos)

Per `sq-5reoy` / #1599 the former in-tree `zk/ieee754` and `zk/xpath` Noir trees were split to
face repos and REMOVED from this repo. Ground truth from
`zk/compose/compose_core/Nargo.toml` (verified at authoring time):

- **`poseidon`** = `noir-lang/poseidon` tag **v0.3.0** — the commitment hash + all Merkle folds.
  Pinned as a Nargo git dependency.
- **`sparq_ieee754`** = `sparq-org/noir_IEEE754` tag **v0.11.0** — IEEE-754 bit-level compare
  for the `xsd:double` FILTER lanes. Pinned as a Nargo git dependency.
- **`noir_XPath`** = `sparq-org/noir_XPath` — the XPath 2.0 F&O library. **`zk/compose` does NOT
  consume `noir_XPath`** (it is not in the compose dependency chain); it is a separate face repo
  with its own CI and releases. Latest release at authoring time: **v0.3.0**. A read-only
  completeness audit of that face repo at `v0.3.0` — the qt3-derived `test_packages/`
  REAL/STUB partition, and the disposition of the two pre-externalization beads `sq-t9d` /
  `sq-t9v` — is recorded in
  [`noir-xpath-phase0-and-streams-audit.md`](noir-xpath-phase0-and-streams-audit.md). It is
  a *coverage* audit and makes no soundness claim.

> **Honesty note on versions (a small correction to the brief).** The brief cited
> "v0.3.0 / v0.11.0". Reconciled against ground truth: **v0.3.0 is `noir-lang/poseidon`** (the
> commitment hash the compose tree pins) **and, coincidentally, is also the current latest
> `noir_XPath` release** cut the same day; **v0.11.0 is `sparq_ieee754`** (the float lane
> `zk/compose` pins). `noir_XPath` is not on `zk/compose`'s dependency path at all. Separately,
> `AGENTS.md` recorded `noir_XPath` as `v0.2.0` in its externalized-deps note, which went stale
> when v0.3.0 was released 2026-07-06; that note was **synced to v0.3.0** by the doc-sync
> follow-up (`sq-6mhcd` / #3138). This is a version-provenance note, not a soundness statement.
>
> **Pin vs. release.** The face repos' latest release is not automatically what a lane here
> verifies. At the time of writing, the XPath differential lane pinned `XPATH_TAG: "v0.2.0"`
> (`.github/workflows/xpath-differential.yml`, `zk/xpath/scripts/run_differential_harness.sh`),
> so its evidence was about **v0.2.0**, not v0.3.0. That pin was **bumped to v0.3.0** by #5456,
> so the lane's evidence now tracks the current release. What the bump did NOT change: the
> committed golden (`zk/xpath/tests/differential_oracle/src/lib.nr`) is generated from sparq's
> own Rust evaluator and is therefore tag-independent — it regenerated byte-identical, and the
> circuit-side modules the golden exercises (`numeric_types.nr`, `numeric.nr`, and the vendored
> `ieee754`/`json_parser`) are byte-identical between the two tags, with `string.nr`/`sequence.nr`
> purely additive (SHA-2 digests, `langMatches`, `TZ()`, `GROUP_CONCAT`, `SAMPLE` — face-repo
> `sq-3kd2g.4`). Both tags declare the same `nargo 1.0.0-beta.21` compatibility as
> `NARGO_VERSION`. The load-bearing check remains the lane's own green `nargo test` +
> per-test-function fault injection, which only CI can run. Read the pin, not this note.

**Cross-repo audit consequence:** an external auditor auditing the FILTER float lanes must audit
`sparq_ieee754 @ v0.11.0` in its own repo (its `nargo test` + differential oracle now live in the
face repo's CI, not in `zk-toolchain.yml`). The commitment-hash bit-compat chain rests on
`poseidon @ v0.3.0`.

---

## 2. PER-CLAIM VERIFICATION-STATUS MAP

Every security-relevant property the estate relies on, mapped to **how it is currently
verified**. Legend for the *Status* column:

- **RE-AUDIT-CLOSED** — the internal re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`)
  reports it CLOSED with code evidence. **This is an internal, single-model, read-only
  self-review — NOT external sign-off.**
- **TEST-PINNED** — a standing regression test asserts the property (forge → reject). Lane noted
  (`default` = runs in plain `cargo test`; `toolchain` = `#[ignore]`d, needs nargo/bb).
- **STATIC-ANALYSIS** — a hand-traced forge-map / source review (e.g. `sq-l9ulg`), explicitly
  NOT a mechanized proof.
- **UNVERIFIED** — no test, no proof; correctness rests on code-reading and/or an external
  assumption. An honest "not established here."
- **KNOWN-GAP** — a documented deferral or downgrade; see §3.

**In every case the property is a CLAIM/OBLIGATION whose external verification is `sq-qhy4`.**

| Property id | Security-relevant claim (intended) | Where it lives | Status | Evidence pointer |
|---|---|---|---|---|
| **PI-BIND** | The bb-verified public inputs are reconstructed from the *declared* statement under the verifier's OWN nonce and byte-compared to each proof (no statement/verdict substitution). | `verifier.rs reconstruct_public_inputs` | RE-AUDIT-CLOSED (finding #1) + TEST-PINNED (`forge_pubinput_statement_substitution_rejected`, `_verdict_substitution_rejected`; `finding_01_*`) | `zk-verifier-reaudit.md` §"binding gate"; `forge_gates.rs`; `audit_forge_map.rs` |
| **VK-CANON** | `bb verify` uses a verifier-RECOMPUTED canonical vk from the re-derived `CircuitId`, never the prover-supplied vk. | `verifier.rs` / `driver.rs` | RE-AUDIT-CLOSED (finding #2) + TEST-PINNED (positive control + #11 relabel) `toolchain` | `zk-verifier-reaudit.md`; `audit_forge_map.rs` finding #2/#11 |
| **COMMIT-BIND** | Commitments are ISSUER-SIGNED, not unsigned prover-chosen field elements (no arbitrary-graph forgery / triple suppression). | `commit.rs`, `verifier.rs` issuer gate | RE-AUDIT-CLOSED (finding #3) + TEST-PINNED (`forge_commitment_unsigned_rejected`, `forge_issuer_*`) `default` | `zk-verifier-reaudit.md`; `forge_gates.rs`; `registry.rs` `ZK_COMMITMENT_SIGNATURE` |
| **ISSUER-SIG** | The issuer Schnorr-over-Baby-JubJub signature over `C(G)` verifies under a key in the EXTERNAL trusted key set K. | `sig.rs`, `verifier.rs bind_issuer_attestations` | RE-AUDIT-CLOSED (finding #3) + TEST-PINNED (`forge_issuer_invalid_signature_rejected`, `forge_issuer_key_not_in_external_k_rejected`) `default` | `forge_gates.rs`; `sig.rs` |
| **REPLAY** | Single-use replay/freshness binding: the challenge is the verifier's fresh nonce, byte-bound into the proof; a captured manifest is not infinitely replayable, and a JSON-challenge rebind fails. | `verifier.rs` nonce path, `InMemorySeenNonces` | RE-AUDIT-CLOSED (finding #4) + TEST-PINNED (`forge_nonce_binding_mismatch_rejected`, `forge_nonce_replay_rejected`) `default` | `forge_gates.rs`; `audit_forge_map.rs` finding #4 |
| **FILTER-BIND** | The FILTER operator / bound / verdict AND the operand slot are bound to the query's FILTER expression (no comparison substitution, no wrong-slot operand, no `17>=17`-for-`>=18`). | `verifier.rs` query-binding | RE-AUDIT-CLOSED (findings #5, #6, #7, #10) + TEST-PINNED (`UnboundFilter`, `BindingInconsistent`, `UnboundPattern`) `default`+`toolchain` | `zk-verifier-reaudit.md`; `audit_forge_map.rs` findings #5/#6/#7/#10; `filter_signed_binding.rs` |
| **ATTRIB** | Cross-graph attribution cannot be collapsed/omitted (`[[0],[0]]` collapse rejected). | `verifier.rs` attribution gate | RE-AUDIT-CLOSED (finding #8) + TEST-PINNED (`forge_attribution_omitted_rejected`, `_under_declared_rejected`) `default` | `forge_gates.rs` |
| **SALT** | Per-graph RDFC-1.0 salt is not reused across two scan-referenced graphs (`SaltReused`). | `verifier.rs` salt gate | RE-AUDIT-CLOSED (finding #9) + TEST-PINNED (`audit_forge_map.rs` finding #9) `default` | `audit_forge_map.rs` |
| **CID-REDERIVE** | The `CircuitId` (n/d/r/depth/na/nb buckets) is re-derived host-side; a bucket relabel is rejected (`CircuitIdMismatch`) and the proof fails against the canonical vk. | `verifier.rs` id re-derivation | RE-AUDIT-CLOSED (finding #11) + TEST-PINNED (`bb_forge_wrong_vk_rejected`) `default`+`toolchain` | `join_forge.rs`; `audit_forge_map.rs` finding #11 |
| **REVOKE** | A revoked or stale-version credential is rejected (`CredentialRevoked` / `StatusListStale`). | `verifier.rs` revocation gate | RE-AUDIT-CLOSED (finding #12) + TEST-PINNED (`forge_revocation_revoked_bit_rejected`, `_stale_version_rejected`) `default` | `forge_gates.rs` |
| **POS2-XVEC** | The Rust Poseidon2 port is bit-identical to `noir-lang/poseidon` v0.3.0 (commitment cross-vector chain holds). | `poseidon2.rs` + `tests/poseidon2_noir_cross.rs` | TEST-PINNED (`hash_is_deterministic_and_length_separated`, cross-vector tests) `default`(host)/`toolchain`(nargo side) | `poseidon2.rs`; `compose_core/src/lib.nr` bit-compat contract |
| **SCAN-COMPLETE** | In-circuit scan completeness/soundness: the disclosed rows are exactly the pattern matches over the committed graph. | `scan.nr` | RE-AUDIT-noted correct IN-CIRCUIT (prior + re-audit) + TEST-PINNED (`forge_scan_duplicate_commitment_*`, out-of-order ordering) `default`+`toolchain` | `zk-soundness-audit.md` (in-circuit scan correct); `forge_gates.rs` |
| **FILTER-INCIRCUIT** | In-circuit filter comparison over the field is correct (no wraparound; `value >= bound`). | `filter_int.nr`, `filter_signed.nr`, `filter_float.nr` | RE-AUDIT-noted correct IN-CIRCUIT + STATIC-ANALYSIS (ieee754 forge-map `sq-l9ulg`) | `zk-soundness-audit.md`; `sq-l9ulg` audit (ieee754 hint/from_parts) |
| **IEEE754-CONSTR** | Every `unconstrained` hint + private `from_parts` in the ieee754 kernels is fully constrained at proving level (no non-canonical witness forge on {f16,f32,f64,f128}). | `sparq_ieee754 @ v0.11.0` (face repo) | STATIC-ANALYSIS (`sq-l9ulg`, hand-traced forge-map @ a pinned commit) — **explicitly NOT a soundness proof; BN254-assumed** | `sq-l9ulg` close report (4 parts); 2 LATENT gaps → `sq-9zofs` |
| **SIG-CT** | Secret-bearing paths (issuer signing) do not leak via timing. | `sig.rs` scalar-mul | STATIC-ANALYSIS (source-level side-channel review) + KNOWN residual (arkworks scalar-mul not asserted CT) | `compliance/cryptoreview/side-channel-analysis.md`; §3 CR-G5 |
| **REG-FAILCLOSED** | The registry loader is fail-closed (malformed → dropped; absent → no entries; pre-existing copy stripped); unknown `zk:scheme` IRI rejects, never defaults. | `registry.rs`, `commit.rs from_scheme_iri` | TEST-PINNED (`method_parse_is_fail_closed_on_unknown`, `value_only_iri_is_rejected_without_the_feature`) `default` | `registry.rs`; `commit.rs`; §3 CR-G9 |
| **INV-VL** | Value↔lexical agreement on the value-FILTER lane ("compared value equals parse(committed lexical)"). | `filter_int.nr` etc. (string-canonical); dual-leaf REMOVES it | RE-AUDIT-noted machine-enforced for `StringCanonicalV1`; **KNOWN-GAP** (downgraded to issuer-honesty) for `DualLeafV1` | §3 CR-G8; `commit.rs` per-method honesty; `zk-dual-leaf-issuer-desync-review.md` |
| **JOIN-BIND** | The hidden-join query binding (`bind_joins` / `UnboundJoin`) ties the join proof to the query. | (would live in `verifier.rs`) | **UNVERIFIED / KNOWN-GAP** — `JoinEq` is SCHEMA ONLY; the step-4 gate `bind_joins` (`sq-sfsi`) is NOT wired. Structural forge tests exist but the crypto-chain host gate does not. | `manifest.rs` `JoinEq` doc; `join_forge.rs` (structural only) |
| **HOLDER-CRED-BIND** | `HolderPoP` is bound to the specific credential (trusted holder A cannot present trusted holder B's credential). | `verifier.rs bind_holder_pop` / `bind_holder_binding` / `bind_holder_pok` | **RE-AUDIT-CLOSED-SINCE + TEST-PINNED** — the credential-binding UPGRADE the re-audit filed as future work (NEW-2a) has LANDED (`sq-c2ql`/`sq-z8s7`/`sq-i1dt`/`sq-42e3`, all CLOSED) and is forge-pinned. **Correction to prior docs:** `zk-verifier-reaudit.md` NEW-2 and `gap-register.md` CR-G6 still describe this as an open deferral — that text is now STALE against code (see §3, doc-sync phase). `default` | `crates/sparq-zk-compose/tests/holder_pop_forge.rs` (A-presents-B → `HolderKeyMismatch`); `verifier.rs bind_holder_pop`; §3 CR-G6 note |
| **SALT-INCIRCUIT** | Salt binding enforced IN-CIRCUIT (rather than verifier-side-clear). | (deferred) | **KNOWN-GAP** (privacy deferral) | §3 CR-G6 |

---

## 3. KNOWN GAPS (the honest list)

The authoritative gap table is
[`compliance/cryptoreview/gap-register.md`](../compliance/cryptoreview/gap-register.md)
(CR-G1..CR-G9). This section is the **circuit-level index** into it plus the coZK negative
result; it does not restate severities (read the register for those) or re-raise anything the
register lists as "explicitly NOT a gap".

- **CR-G1 — no external accredited-cryptographer audit.** The master gate. All soundness
  assurance is internal self-review. `sq-qhy4` (P0). *This dossier is part of CR-G1's evidence
  pack.*
- **CR-G2 / CR-G3 — crypto-chain forge tests `#[ignore]`d + missing empirical anchors.** A
  toolchain-gated CI lane (`.github/workflows/zk-toolchain.yml`) now runs the `#[ignore]`d
  `forge_pubinput_*` + real-bb e2e suite and recaptures anchors (`sq-f9tl`, closed). The
  auditor should confirm the lane is green and required-gated.
- **CR-G4 — no FIPS 140-3 validated module.** Honest negative claim stated in
  `compliance/cryptoreview/fips-posture.md`: BN254 / Poseidon2 / Schnorr-over-Baby-JubJub are
  not FIPS-approved primitives; no CMVP claim.
- **CR-G5 — no instrumented constant-time / side-channel measurement.** Source-level review
  only (`side-channel-analysis.md`); several hygiene hardenings landed (zeroize/subtle,
  branchless nonce guard, owned ChaCha CSPRNG). Residual: arkworks BN254 / Baby-JubJub scalar
  ops not asserted constant-time. Present exposure LOW by architectural placement (signing is
  issuance-side; verify carries no secret), NOT by constant-time primitives.
- **CR-G6 — residual ZK privacy deferrals (NOT soundness breaks).** In-circuit salt binding not
  done (salt verified verifier-side-clear); per-graph salt + list-IRI/version disclosed in the
  clear (linkability handles). GENUINELY-OPEN: `sq-hyhj` (in-circuit salt binding, P3), `sq-93h`
  (per-graph-salt disclosure on the hidden-only path, P3). A revoked/forged credential still
  fails; an untrusted holder still fails.
  **Doc-sync correction (verified against code, do not take the register on faith here):** the
  register's CR-G6 row and `zk-verifier-reaudit.md` NEW-2 also list "`HolderPop` not yet
  credential-bound (trusted holder A could present B's credential)" as OPEN — that item is
  **CLOSED on `main`**: the issuer-attested credential-bound HolderPoP upgrade landed
  (`sq-c2ql`/`sq-z8s7`/`sq-i1dt`/`sq-42e3`, all CLOSED), the verifier gate `bind_holder_pop` /
  `bind_holder_binding` is WIRED, and `holder_pop_forge.rs` (`sq-ncz0`) pins the A-presents-B
  rejection (`HolderKeyMismatch`). The register text predates the landing; syncing it is a phase
  below. This is a correction of a stale gap-line, not a new soundness claim (see §2
  HOLDER-CRED-BIND).
- **CR-G7 — `sparq-mpc` collaborative path unbuilt / no malicious security.** Out of scope for
  `sq-qhy4`; see §7. The coZK negative result governs this (below).
- **CR-G8 — dual-leaf value+lexical encoding UNAUDITED and REMOVES INV-VL.** For `DualLeafV1`,
  value↔lexical agreement on the value-FILTER lane regresses from machine-enforced to
  trusted-issuer-honesty; a malicious *trusted* issuer could commit one credential answering a
  value question as one number and an identity/join question as another (impossible today under
  `StringCanonicalV1`; no untrusted party can exploit it). Leaf encoding NOT implemented
  (`sq-j506`, audit-gated). `VALUE_HOOK` is many-to-one for double/decimal → term-identity
  hazard requiring structural reject-list enforcement.
- **CR-G9 — commitment-method × circuit × signature compatibility matrix.** A new fail-closed
  soundness surface: a verifier MUST refuse any `(zk:scheme, CircuitId)` / `(zk:scheme,
  zk:cryptosuite)` pair outside the legal matrix. The registry half is fail-closed and tested
  (see REG-FAILCLOSED); the `(method, circuit)` dispatch (`sq-cfmv`) and value-bearing members
  are NOT fully wired. OPEN obligation.
- **JOIN-BIND (schema-only join).** `JoinEq` circuits are compiled but the host query-binding
  gate `bind_joins` (`sq-sfsi`) is NOT wired — the hidden-join path is not end-to-end. Tracked
  as a distinct build gap, surfaced here because it is at circuit granularity.
- **coZK / collaborative-proof negative result.** The multi-prover path (`sparq-mpc/src/proof.rs`)
  returns `NotYetImplemented` for every method; the in-circuit
  distributed-signature-over-secret-shared-witness join ("the join nobody has built") is a
  deferred spike (`sq-bjl`). The adversarial coZK re-audit
  ([`research/mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md), `sq-9hrn`) vs CRYPTO'25 eprint
  2025/1026 returns **RE-OPEN (gating)** — a forward-looking design-governance verdict on an
  unbuilt path, NOT a code-soundness verdict on a running prover (none exists).
- **PQ boundary (honest statement).** The estate is built on BN254 (a pairing-friendly curve,
  128-bit classical security target) with Poseidon2 and Schnorr-over-Baby-JubJub. **None of these
  primitives is post-quantum.** No PQ security is claimed anywhere; a cryptographically-relevant
  quantum adversary breaks the discrete-log-based signatures and the SNARK's assumptions. This
  is stated as a KNOWN boundary, not a mitigated one.

---

## 4. FORGE-MAP / ADVERSARIAL-TEST COVERAGE (per the `sq-l9ulg` template)

Two standing forge suites pin the 12 historical findings closed; they are the 1:1 companion to
`research/zk-verifier-reaudit.md`. **They are regression pins, NOT a soundness proof** — if any
`default`-lane forge below ever VERIFIES (the verifier ACCEPTS the forgery) that is a real
soundness regression and the test goes red.

- `crates/sparq-zk-compose/tests/audit_forge_map.rs` (`sq-1gir`) — **one permanent test per
  historical finding #1–#12**, organized BY FINDING NUMBER; the gate on the epic-close claim.
- `crates/sparq-zk-compose/tests/forge_gates.rs` — the same gates organized BY BINDING (one
  canonical forge per binding).
- `crates/sparq-zk-compose/tests/join_forge.rs` — hidden-join structural + bb forge cases
  (`join` schema-level, plus `bb_forge_tampered_join_proof_rejected`,
  `bb_forge_unequal_values_witness_unsatisfiable`, `bb_forge_wrong_vk_rejected`).
- `crates/sparq-zk-compose/tests/holder_pop_forge.rs` (`sq-ncz0`) — the dedicated forge-and-verify
  map for the issuer-attested credential-bound HolderPoP closure (`bind_holder_pop`, a gate that
  runs BEFORE the per-sub-proof bb loop, so its negatives fire on the `default` lane). Vectors →
  verdict: `A-presents-B` → `HolderKeyMismatch`; `tampered attested digest` →
  `InvalidIssuerSignature`; `untrusted issuer anchor` → `IssuerKeyNotInKeySet`; `replay/wrong-nonce
  PoP` → `HolderPopInvalid`; `bearer-where-required` → `HolderBindingMissing`; positive
  bound/bearer cases ACCEPT (reach `MissingProof`). This closes the item the re-audit's NEW-2 filed
  as future work — see §2 HOLDER-CRED-BIND and the §3 CR-G6 doc-sync note.

The 1:1 finding → reject-path map (transcribed from the `audit_forge_map.rs` header, which
itself cites `research/zk-verifier-reaudit.md`):

| Finding | Forge attempted | Reject path / error | Lane |
|---|---|---|---|
| #1 | honest proof over a DIFFERENT statement | `PublicInputMismatch` | toolchain |
| #2 | prover-supplied / trivial-circuit vk | positive control (honest proof verifies via recomputed canonical vk; the negative is #11) | toolchain |
| #3 | unsigned / untrusted-key commitment | `UnattestedCommitment` / `IssuerKeyNotInKeySet` | default |
| #4 | replay / JSON-challenge rebind | `NonceReplay` / `NonceBindingMismatch` | default |
| #5 | `17>=17` proven for a `>=18` query | `UnboundFilter` (bound mismatch) | default |
| #6 | wrong-slot operand / failing row disclosed | `UnboundFilter` (slot / verdict) | default |
| #7 | filter proof over a different operand | `BindingInconsistent` (host) / `PublicInputMismatch` (bb) | default + toolchain |
| #8 | `[[0],[0]]` collapse / omitted attribution | `AttributionUnderDeclared` / `AttributionMalformed` | default |
| #9 | salt reuse across two scan-referenced graphs | `SaltReused` | default |
| #10 | FILTER-add / constant-swap | `UnboundFilter` / `UnboundPattern` | default |
| #11 | n/d/r bucket relabel | `CircuitIdMismatch` / bb fails vs canonical vk | default + toolchain |
| #12 | revoked / stale credential | `CredentialRevoked` / `StatusListStale` | default |

**Additional STATIC forge-map (ieee754, `sq-l9ulg`, the format/rigor TEMPLATE for this section):**
a hand-traced forge-map over every `unconstrained fn` / `unsafe { hint }` / private `from_parts`
site in the ieee754 kernels found all reachable sites FULLY-CONSTRAINED per the audit (kernel ops
reduce to check-the-hint over `new()`-canonicalised inputs), with two LATENT (unreachable-today)
hardening gaps captured as `sq-9zofs`. **That audit is explicit that "fully constrained per this
audit" is NOT a soundness certification** — it is static analysis, BN254-assumed, and `sq-qhy4`
remains the sole soundness gate.

**Coverage honesty:**
- The findings whose forgery needs a GENUINE bb proof to even construct (#1, #2, the proof-bound
  variants of #7/#11) are `#[ignore]`d and run only in the toolchain-gated lane
  (`zk-toolchain.yml`, bead `sq-f9tl`). Closure of those rests on that lane running + code-reading
  + empirical anchors, NOT on default `cargo test`.
- The forge suites cover the SINGLE-PROVER verifier. There is **no forge suite for the hidden-join
  end-to-end path** (schema-only, `bind_joins` unwired) and **no adversarial suite for the
  collaborative/MPC path** (unbuilt).

---

## 5. QUESTIONS FOR THE EXTERNAL AUDITOR (numbered)

The exact things an accredited cryptographer should scrutinize. These are the audit's work
items; each maps to a §2 property and/or a §3 gap. **Nothing here is answered by this dossier —
answering them IS the audit (`sq-qhy4`).**

1. **`reconstruct_public_inputs` ↔ circuit `main` correspondence.** Verify that the ~5.7k-LoC
   hand-serialized public-input reconstruction in `verifier.rs` produces, for every one of the 31
   members, the EXACT public-input field vector (field order, endianness, arity, no omitted
   input) that `bb verify` expects for that member's `main`. A single field-order / endianness /
   arity slip silently re-opens the whole estate (this was the prior audit's CRITICAL #1). Is a
   machine-checked correspondence feasible?
2. **Commitment-scheme binding.** Is `C(G)` = Poseidon2 fold binding (no length/second-preimage
   forgery beyond the signature gate)? Is the `commit_fold` length IV sufficient for
   length-separation? Does the string-canonical blake3 token bind the operand to the committed
   literal soundly for every FILTER member?
3. **Issuer Schnorr-over-Baby-JubJub scheme.** Audit the signature scheme (nonce derivation,
   challenge reduction, on-curve / `<L` / identity guards) both host-side (`sig.rs`) and
   in-circuit (`issuer.nr`, `holder.nr`). Is the in-circuit Schnorr verify sound and complete?
4. **Fiat–Shamir / nonce discipline.** The `challenge` is a verifier-supplied public tag,
   unconstrained in-circuit by design, byte-bound by the host reconstruction. Confirm this design
   is sound: is there any path where the nonce is NOT byte-bound into `bb`'s public inputs (the
   #4 JSON-rebind class)? Is single-use enforcement (`InMemorySeenNonces`) adequate for the
   intended deployment, and is there a domain-separation gap across circuit kinds?
5. **Canonical-vk recomputation (VK-CANON).** Confirm the verifier recomputes the vk from the
   canonical compiled member named by the re-derived `CircuitId` and never trusts the
   prover-supplied vk — and that the re-derivation cannot be steered to a weaker member.
6. **Scan completeness/soundness relation (`scan.nr`).** Is the in-circuit relation exactly "the
   disclosed rows are all and only the pattern matches over the committed graph" for every
   `(k, n, r)` bucket? Are duplicate/out-of-order commitment forgeries closed in-circuit (not only
   structurally)?
7. **FILTER comparison correctness over the field.** For `filter_int` / `filter_signed` /
   `filter_decimal` / `filter_float`: confirm no modular wraparound, correct sign handling, and
   correct fixed-point scaling; confirm the operand is bound to the committed literal and the
   verdict to the query.
8. **ieee754 kernel constraints (`sparq_ieee754 @ v0.11.0`, in the face repo).** Independently
   verify the `sq-l9ulg` static forge-map: that every `unconstrained` hint / private `from_parts`
   is fully constrained at proving level, that the two LATENT gaps (`sq-9zofs`) are genuinely
   unreachable for {f16,f32,f64,f128}, and that the in-circuit IEEE canonicalisation
   (−0.0/+0.0/NaN) is total.
9. **Dual-leaf INV-VL removal (CR-G8).** If/when `DualLeafV1` is implemented (`sq-j506`): confirm
   the value↔lexical residual is bounded to a malicious *trusted* issuer, that B1
   range-decomposition + B4 canonical-form bind are IN-CIRCUIT, and that reject-list (v) (no
   identity operator reads `value_component`) is STRUCTURALLY enforced.
10. **Commitment-method × circuit × signature matrix (CR-G9).** Confirm the `(method, circuit,
    signature)` compatibility matrix is enforced fail-closed end-to-end — no proof verifies under
    a `CircuitId` illegal for its recorded `zk:scheme`; unknown/mismatched IRIs reject rather than
    default.
11. **Hidden-join binding (JOIN-BIND).** The `JoinEq` circuits are compiled but `bind_joins`
    (`sq-sfsi`) is unwired. Before any reliance: does the intended host gate soundly bind the join
    proof to the query, and is the `join_commitment` binding (not disclosing the joined term)
    sound?
12. **Trusted-setup posture.** UltraHonk (Barretenberg) is the proving backend. Confirm the
    trusted-setup / SRS posture (which universal SRS, its provenance, whether any per-circuit
    setup is needed) and that the deployment's SRS is authentic — this dossier does not establish
    it.
13. **Toolchain reproducibility.** Confirm the `NARGO_VERSION` / `BB_VERSION` pins reproduce the
    committed vk / gate-count snapshot, and that the externalized `sparq_ieee754 @ v0.11.0` /
    `poseidon @ v0.3.0` git deps are pinned to auditable, immutable tags.
14. **PQ boundary.** Confirm and document that the whole estate is classically-secure only
    (BN254 / discrete-log); there is no PQ claim, and the crossover point (SNARK assumption vs
    signature) should be stated for the relying party.
15. **Side-channel residual (CR-G5).** Decide whether the source-level side-channel review is
    sufficient for the intended deployment or whether an instrumented `dudect`/`ctgrind` pass on
    the issuer-signing / arkworks scalar-mul paths is required before any online-signing surface.

**Out of scope for `sq-qhy4` but flagged:** the collaborative/MPC path (§7) — its own future
audit governed by the coZK negative result (`research/mpc-cozk-reaudit.md`).

---

## 6. The MPC / collaborative boundary (out of scope for `sq-qhy4`)

`sq-qhy4` is the SINGLE-PROVER verifier + circuits audit. The MPC / collaborative-proof layer
(`sparq-mpc`) is a separate track (epic `sq-pwr`, navigator [#1180]). Its honest status: **no
security guarantee** — semi-honest, honest-majority only; the collaborative-proof core returns
`NotYetImplemented`; the malicious-security + distributed-signature work is deferred (`sq-bjl`,
`sq-34ml`). The adversarial coZK re-audit (`research/mpc-cozk-reaudit.md`) returns RE-OPEN
(gating) on the *design*. Any mention of MPC must carry that statement. See CR-G7.

---

## 7. Phased plan (each phase = a future bead for the orchestrator)

This dossier is evidence assembly; the following are the *tracked* follow-ups that would raise
audit-readiness further. Each is a candidate future bead (none is a soundness claim; none is
implemented here).

1. ~~**Sync `AGENTS.md` externalized-deps note to `noir_XPath v0.3.0`**~~ — DONE (`sq-6mhcd` /
   #3138); the note records v0.3.0 and distinguishes the face-repo release from the `XPATH_TAG`
   pin the differential lane actually verifies. ~~**The pin bump itself**~~ — also DONE (#5456):
   both `.github/workflows/xpath-differential.yml` and `zk/xpath/scripts/run_differential_harness.sh`
   now pin `XPATH_TAG` **v0.3.0**, so the lane's evidence tracks the current release. The
   resulting evidence is still only as good as a **green CI run of that lane** (`nargo test` +
   per-test-function fault injection), which only CI can produce — read the pin and the run, not
   this note. See §1.3.
2. **Sync `compliance/cryptoreview/gap-register.md` CR-G6 + `research/zk-verifier-reaudit.md`
   NEW-2** to drop the "`HolderPop` not yet credential-bound" line — that item CLOSED on `main`
   (`sq-c2ql`/`sq-z8s7`/`sq-i1dt`/`sq-42e3`, forge-pinned by `holder_pop_forge.rs`). Leave the
   genuinely-open CR-G6 residuals (in-circuit salt binding `sq-hyhj`; salt/list-IRI linkability
   `sq-93h`). Doc-only; keeps the register from mis-stating the estate to the external auditor.
3. **Confirm the `zk-toolchain.yml` forge + anchor lane is required-gated and green** on the
   current `main` (CR-G2/CR-G3 close-verify), and cross-link its run into this dossier's §4.
4. **Wire the hidden-join host gate `bind_joins` (`sq-sfsi`) OR mark `JoinEq` explicitly
   NOT-END-TO-END in the crate README** so no consumer mistakes schema-only join circuits for a
   verifiable path (JOIN-BIND gap).
5. **Assemble a one-file auditor reproduction bundle** (a script that compiles the 31 members,
   runs the `#[ignore]`d forge lane, and recaptures the empirical public-input anchors) so the
   external auditor reproduces §4 in one command — extends
   `compliance/cryptoreview/audit-readiness-dossier.md` §9.
6. **Independent second-model / adversarial re-run of the `sq-l9ulg` ieee754 forge-map against
   `sparq_ieee754 @ v0.11.0` in the face repo** (the static analysis was single-model; a second
   pass reduces single-model risk before the external audit).
7. **Commission the external accredited-cryptographer audit (`sq-qhy4`, CR-G1)** — the terminal
   phase; external, out of agent scope. This dossier + `compliance/cryptoreview/*` +
   `research/zk-soundness-audit.md` + `research/zk-verifier-reaudit.md` + the forge suites are the
   evidence pack.

---

## 8. Open questions that genuinely need the maintainer

1. **Commission timing.** When does the maintainer intend to commission `sq-qhy4`? The
   audit-readiness work (this dossier, the `compliance/cryptoreview/` package) is assembled;
   phases 1–5 above are the remaining agent-doable prep.
2. **Dual-leaf go/no-go before audit.** Should `DualLeafV1` (`sq-j506`) be implemented BEFORE the
   external audit (so the auditor reviews it once) or DEFERRED (audit `StringCanonicalV1` only,
   re-audit the dual-leaf delta later)? This changes the audit's scope materially (CR-G8/CR-G9).
3. **Auditor scope for the MPC boundary.** Should the external cryptographer be asked to review
   the coZK *design* (not code) as part of the engagement, or is that a strictly separate future
   engagement once the collaborative path is built?

---

*This document asserts NO soundness, security, privacy, attestation, or production-readiness
property. Every property is a claim/obligation whose external verification is `sq-qhy4`. It is an
evidence-assembly companion to `compliance/cryptoreview/audit-readiness-dossier.md` and
`gap-register.md`, and an index into `research/zk-soundness-audit.md`,
`research/zk-verifier-reaudit.md`, `research/mpc-cozk-reaudit.md`, and the forge suites.*

[#1180]: https://github.com/sparq-org/sparq/pull/1180
