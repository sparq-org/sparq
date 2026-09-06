# zkSPARQL Estate Recon

> Grounding for the zkSPARQL Proposed Spec (bead sq-rvgr2.2). Read-only recon; not itself normative.

## Summary

The zkSPARQL query-proof estate is substantially BUILT and tested across two `publish=false`,
non-default crates. `sparq-zk` is the off-circuit commitment pipeline (Poseidon2-BN254, RDFC10
canon, per-graph commitments, Schnorr-over-BabyJubJub issuer sigs). `sparq-zk-compose` holds
the `ProofManifest` format, the `CircuitId`/`ProofInputs` Noir circuit family, and a 5577-line
verifier at `crates/sparq-zk-compose/src/verifier.rs`.

The verifier MUST-check list is **exactly 12 `bind_*` obligations** (grep -c confirms 12, not
13) plus stage-1 recheck, the stage-3a public-input byte-compare (audit 1), stage-3b canonical-
vk recompute (audit 2), stage-3c bb-verify, and stage-4 nonce single-use/binding. The security-
properties vocabulary is layered: a VENDORED `sec-prop` ontology (8 properties incl
unlinkability, PQ-forgery/snooping, circuit-audit) extended by sparq `secprop-ext.ttl`
(ZK-type, soundness, completeness, hiding/binding, anonymity, setup, single-use, plus an
orthogonal assurance/audit-status axis). ODRL-driven proof admissibility (sq-0dksu) is BUILT
and MERGED (#1230).

The **biggest spec gap is transport**: NO HTTP media type, no wire protocol, no JSON-LD
context; the manifest is serde-JSON tagged `urn:sparq:zk:ProofManifest`, exchanged out-of-band
via a `VerifierNonce` challenge-response. Everywhere soundness is UNAUDITED (sq-qhy4):
research-grade, hidden-holder tiers labelled NOT-yet-sound, and a passing proof is not a
guarantee under an adversarial prover.

---

## What Is Built and Tested

- **Verifier** `crates/sparq-zk-compose/src/verifier.rs` (5577 lines): 12 `bind_*` obligations
  implemented and tested (`tests/verifier_errors.rs`, `e2e.rs`, `forge_gates.rs`,
  `join_forge.rs`, `holder_pop_forge.rs`, `differential_fuzz.rs`, `audit_forge_map.rs`).
  Names and lines: `bind_query_correctness` 3633, `bind_attributions` 3981,
  `bind_issuer_attestations` 2242, `bind_revocation` 2722, `bind_joins` 3790,
  `bind_entailment` 4433, `bind_holder_pop` 4082, `bind_holder_binding` 4189,
  `bind_hidden_revocation` 2871, `bind_hidden_issuer_attestations` 3034,
  `bind_holder_pok` 3190, `bind_holder_set` 3377.

- **Two entry points**: `prefilter_manifest_structure` near 2040 (stages 1 and 2, NOT sound
  alone) and `verify_manifest` 4578 (sound entry) adding recheck (Q6 bnode guard, attribution
  arity, circuit-id re-derivation, strictly-increasing commitments),
  `reconstruct_public_inputs` byte-compare with verifier nonce field 0 (4685 and 4786, audit
  1), canonical-vk recompute (4697, audit 2), bb verify (4705), nonce single-use `record_fresh`
  and `NonceBindingMismatch` (4620–4644, audit 4). Audit 3 (issuer-signature / key-set
  binding, codex #1) is enforced per-scan by `bind_issuer_attestations` (2242, listed above),
  completing the verifier's audit 1–4 mapping.

- **`ProofManifest` serde-JSON format**: `manifest.rs` 1116, default type
  `urn:sparq:zk:ProofManifest` (1497), `to_json` serde_json pretty (1539), `canonicalize`
  stable hash (1525); `CircuitId` Noir family `manifest.rs:515`
  (Scan, FilterInt, FilterF64, FilterSignedInt, FilterDecimal, FilterValueDl variants,
  RevokeUnset, HiddenIssuer, HolderPok, HolderSet, JoinEq); ~70-variant fail-closed
  `CheckError` `verifier.rs:1020`.

- **External trust anchors**, never manifest-supplied: `KeySet` 164, `HolderRegistry` 299,
  `HolderBindingPolicy` 444, `RevocationPolicy` 630, `VerifierNonce` 757, `SeenNonces` trait
  with durable `FileSeenNonces` 909 and `InMemorySeenNonces` 845.

- **Security-properties vocabulary**: VENDORED `sec-prop` 8 properties
  (`crates/sparq-trust/ontologies/zkp-sparql/vocab/sec-prop.yaml.ld`) EXTENDED by
  `secprop-ext.ttl` (`crates/sparq-secprop-vocab/ontologies/secprop-ext.ttl`):
  `ZeroKnowledgeType`, `Soundness`, `Completeness`, `Hiding`, `Binding`, `Anonymity`, `Setup`,
  `Interactivity`, `SelectiveDisclosure`, `SingleUse` plus `AssuranceLevel`, `AuditStatus`,
  `Assumption`, `PropertyScope` axes; Rust constants `crates/sparq-trust/src/secprop.rs`.

- **Per-method annotation graph and 3 over-claim guards** (opt-in `secprop-annotations`):
  `crates/sparq-zk/src/secprop.rs` (`audit_overclaim_violations`,
  `source_layer_transfer_violations`, `completeness_violations`); data
  `crates/sparq-zk/ontologies/secprop-methods.ttl`.

- **ODRL admissibility** (sq-0dksu) MERGED #1230: ODRL 2.2 profile
  `crates/sparq-policy/ontologies/odrl-secprop-profile.ttl` (13+ `secx:requires`
  leftOperands); N3 reduction `crates/sparq-trust/src/admissibility.rs` (`LEVEL_ORDERS`,
  `CLOSURE_RULES`, `DISCHARGE_RULE` on `sparq_reason`) with default-deny universal admissible
  at line 210.

- **Fail-closed pre-check gate** `crates/sparq-trust/src/admit.rs:493 admit_with_precheck`
  (opt-in `secprop-precheck`): `PrecheckOutcome Admitted, Denied, ReductionError`; reduction
  error is also deny (614); base admit 175 checks issuer Schnorr sig over RDFC10 commitment
  plus SHACL scope plus reserved-predicate guard plus clear-WebID holder binding.

---

## Designed, Not Built

- **VC cryptosuite bridge** (sq-9c5e, PR #1155) IMPLEMENTED but UNMERGED — only on branch
  `feat/sq-9c5e-vc-cryptosuite-bridge` (`crates/sparq-zk/src/vc_bridge.rs`, 699 lines, NOT
  on main); real off-circuit Ed25519 (`ed25519-dalek`) + ECDSA-P256 (`p256`) Data-Integrity
  verify for `eddsa-rdfc-2022` / `ecdsa-rdfc-2019`; opt-in `vc-bridge` feature.

- **bbs-2023 / ecdsa-sd-2023 selective-disclosure VC ingest** — explicitly DEFERRED seam in
  `vc_bridge.rs` (no in-repo BBS verifier); design `zk-configurable-commitment-design.md §5.3`.

- ~~**P-384 ECDSA** (`ecdsa-rdfc-2019` SHA-384 profile) — not implemented, fails closed as
  `VcBridgeError::UnsupportedKeyCurve` (only P-256/SHA-256 built).~~ **BUILT** (sq-txg1y,
  issue #3234): both published curve profiles verify off-circuit, the profile resolved from the
  issuer key by `EcdsaProfile::from_sec1_key` before the DI hash is taken, pinned against the
  vc-di-ecdsa § A.3 published vector. `UnsupportedKeyCurve` now denotes P-521 only. The same
  bead added the additive `vc_bridge_json` JSON-LD envelope entry point (`oxjsonld` expansion
  against a caller-supplied `@context` allowlist; no network, no `did:` resolution).

- **In-circuit VC-proof verification** — deliberately OUT OF SCOPE; the query proof does NOT
  re-verify the source VC Ed25519/ECDSA proof in-circuit; `zk:sourceCryptosuite` is
  provenance only.

- **Hidden-holder soundness tiers** (`bind_holder_pok` sq-c2ql, `bind_holder_set` sq-3c00)
  WIRED but code+docs state they do NOT make the verifier sound: NOT-yet-sound
  (sq-qhy4/sq-9hrn; remediation epic sq-1s2); opt-in only.

- **Live-service / async-proving posture** DESIGNED ONLY in `research/zkp-query-proofs-plan.md
  §7` (Q9 aggregation, Q11 live-service); no server endpoint, job model, or async proving in
  code.

- **RDFS/OWL entailment in-circuit closure proof** — `bind_entailment` enforces only a
  disclosed-base re-check of derivation steps; in-circuit closure deferred
  (`manifest.rs:1170`); only Simple entailment actually proved.

---

## Candidate Normative Surface

1. **MANIFEST FORMAT**: a zkSPARQL proof response is a JSON object with mandatory
   `type = urn:sparq:zk:ProofManifest` (`manifest.rs:1498`), serialised canonically
   (`ProofManifest::canonicalize` before hashing); a spec MUST mint a real media type
   (candidate `application/vc+zksparql+json` or a `+ld` JSON-LD variant) — currently ABSENT.

2. **CHALLENGE-RESPONSE**: the proof request is a verifier-issued fresh `VerifierNonce` (a
   BN254 field element, `as_field_hex`) minted out-of-band and handed to the prover BEFORE
   proving; the prover MUST commit it as public-input field 0 of every sub-proof; the verifier
   MUST record it single-use before the crypto gate and MUST burn-on-mismatch
   (`verifier.rs:4620`). Transport UNSPECIFIED.

3. **SUB-PROOF blob layout**: hex-encoded `len|proof|len|pi|vk` (`SubProof.proof_hex`);
   public_inputs = 32-byte big-endian field elements, structs/arrays flattened row-major,
   bool to `{0,1}`, u32/u64 to integer value, NO header/length-prefix (empirically pinned to
   bb 5.0.0-nightly.20260324; `reconstruct_public_inputs` `verifier.rs:4766`).

4. **VERIFIER MUST-CHECK OBLIGATIONS** (12 `bind_*` + 5 structural), all fail-closed:
   recheck (Q6 bnode cross-graph join guard + attribution arity + strictly-increasing
   commitments); `bind_query_correctness:3633`; `bind_attributions:3981` superset;
   `bind_issuer_attestations:2242` key-in-external-K; `bind_revocation:2722`
   authoritative-snapshot bit; `bind_joins:3790` commitment+slot binding;
   `bind_entailment:4433` regime+grounded-steps; `bind_holder_pop:4082`;
   `bind_holder_binding:4189`; `bind_hidden_revocation:2871`;
   `bind_hidden_issuer_attestations:3034`; `bind_holder_pok:3190`; `bind_holder_set:3377`;
   `reconstruct_public_inputs` byte-compare with verifier-nonce field 0; canonical-vk
   recompute; bb verify; nonce single-use + `NonceBindingMismatch`.

5. **TRUST ANCHORS MUST be external relying-party inputs**, never manifest-supplied: trusted
   `KeySet K` (`manifest.key_set` only accepted as a subset), authoritative
   `StatusListSnapshot` (`RevocationPolicy`), `HolderRegistry`, fresh `VerifierNonce`,
   `SeenNonces` store — codified because trusting `manifest.key_set` was the codex 1
   soundness hole.

6. **ISSUER SIGNATURE binding**: Schnorr over Baby-JubJub with a Poseidon2 challenge;
   cryptosuite IRI `https://sparq.dev/ns/zk#poseidon2-schnorr-v1`
   (`SignatureScheme::POSEIDON2_SCHNORR_V1_IRI`); an unresolved cryptosuite MUST reject
   (fail-closed, no default).

7. **SECURITY-PROPERTIES VOCABULARY** namespace `https://w3id.org/zkp-sparql/sec-prop#`:
   `Unlinkability(Strength/Scope)`, `PostQuantumForgery/Snooping`, `SignatureTypeLeakage`,
   `ProofSizeLeakage`, `CircuitAudit`, `ValidityPeriodLeakage` + `secx` extension
   `ZeroKnowledgeType`, `Soundness`, `Completeness`, `Hiding`, `Binding`, `Anonymity`,
   `Setup`, `Interactivity`, `SelectiveDisclosure`, `SingleUse`; assurance axis
   `Proven>Claimed>Conjectured`; `auditStatus` incl `ExternalSignOffPending`; scope
   `QueryProofLayer` vs `SourceLayerOnly` (source-layer property MUST NOT satisfy a
   query-proof constraint).

8. **ODRL ADMISSIBILITY PROFILE**: a policy using any `secx:requires` leftOperand MUST assert
   `odrl:profile <https://sparq.dev/ns/odrl-secprop-profile#>`; each leftOperand carries one
   `secx:overDimension` fact; only `odrl:gteq` is reduced (other operators to unsatisfied to
   deny); admissibility = method satisfies EVERY constraint (default-deny);
   `requiresAssurance gteq secx:Proven` mechanically denies every sparq ZK method while
   sq-qhy4 is open.

9. **OVER-CLAIM RULE**: NO sparq ZK method MAY be labelled `secx:Proven` on a positive
   privacy/soundness property while sq-qhy4 is open; only settled-NEGATIVE facts
   (`PQForgeable`, `Replayable`, `SchemeRevealed`) may be `Proven` — machine-checkable guard
   (`secprop.rs audit_overclaim_violations`).

---

## Gaps (Spec Must Address)

1. **NO media type / content-type registered** for the proof manifest or the nonce/challenge;
   the manifest is a bare serde-JSON struct tagged with a URN.

2. **NO JSON-LD context for `ProofManifest`** — it does not round-trip as a Verifiable
   Presentation and cannot be consumed by a generic VC/DI processor.

3. **NO wire protocol / endpoint** for the challenge-response — `VerifierNonce` issuance and
   manifest submission transport are entirely out-of-band and unspecified.

4. **Circuit family is version-pinned to an external Noir/bb toolchain** (nargo
   1.0.0-beta.21, bb 5.0.0-nightly.20260324) driven by subprocess; the public-input byte
   layout is EMPIRICALLY determined, not normatively specified.

5. **The SPARQL fragment is bucketed and partial**: BGP scan + integer/xsd:double/signed-int/
   decimal FILTER + a single-prover hidden cross-credential JOIN; no OPTIONAL/UNION/property-
   paths/aggregation/subqueries — the fragment scope needs a normative definition.

6. **`w3id.org/zkp-sparql/` IRIs were minted as placeholder** while the source repo was
   private; a spec must confirm the permanent-identifier redirect is live and stable.

7. **No canonical test-vector / conformance suite** for the manifest format or the `bind_*`
   obligations exposed as portable spec fixtures (forge tests exist but are Rust-internal).

8. **MPC composition surface is referenced by the admissibility vocabulary**
   (HonestMajority/SemiHonest assumptions) but its interaction with the query-proof verifier
   is not part of the built verifier.

---

## Honesty Flags

- **sq-qhy4 (P0 external audit) is OPEN**: the ENTIRE ZK estate is research-grade and NOT
  externally audited; a passing proof is NOT a guarantee the SPARQL statement holds under an
  adversarial prover.

- **`bind_holder_pok` (sq-c2ql) and `bind_holder_set` (sq-3c00)** are EXPLICITLY labelled
  NOT-yet-sound in code and manifest docs (remediation epic sq-1s2).

- **The dual-leaf value-lane** (`sparq-zk` feature `dual-leaf`) carries a documented
  INV-VL downgrade (#769 accepted, CR-G8) and is NOT externally audited.

- **Only Simple entailment is proved**; RDFS/OWL derivation is a disclosed-base host re-check
  (`bind_entailment`), the in-circuit closure proof is deferred.

- **The admissibility reasoner reasons over ANNOTATIONS**, not cryptography; every sparq ZK
  property annotation is at most `secx:Claimed` with `auditStatus ExternalSignOffPending`.

- **PostQuantum posture is a settled NEGATIVE**: all current issuer signature suites
  (Schnorr/EdDSA/BBS+) are Shor-broken; Pedersen binding breaks under a CRQC so
  retro-soundness fails.

- **The VC cryptosuite bridge (PR #1155) is OPEN and UNMERGED** — its code is NOT on main;
  any VC-ingest claim must be caveated as branch-only.

- **The vendored `sec-prop` ontology lineage**: the `do-not-cite` draft caveat is superseded
  by the 2026-06-21 MIT decision (`PROVENANCE.md`) — verify before discounting.

---

> **Empirical-honesty reminder**: ZK and MPC estates are NOT production-sound until the
> external cryptographer audit sq-qhy4 completes. All work-box benchmarks are non-canonical;
> do not hard-code them in documentation or tests.

---

*Recon captured by Sonnet 4.6 under the Fable program; [SONNET-4.6]*
