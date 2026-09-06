<!-- [OPUS-4.8] sq-toze / sq-toze.20 — Cryptographic-review control table. Honesty-critical.
     Authored by Opus 4.8 while Fable 5 unavailable — re-review when Fable returns. -->

# Cryptographic review — control table

**Framework:** Cryptographic review of sparq's crypto estate (`sq-toze.20`).
**Scope:** the bespoke ZK/MPC crypto (`sparq-zk`, `sparq-zk-compose`, `sparq-mpc`, `zk/`) +
the standard primitives the estate and the release pipeline use. See
[`README.md`](./README.md) §1 for the Tier-A (sound) / Tier-B (research-only) split that
governs every row below.

## Status legend

- **PASS** — a real, verified control (a sound primitive used correctly, or a review
  artifact that exists and was re-read). Evidence is a file path / test / CI job.
- **AUDIT-READY** — the control + its evidence pack are in place, but the *certificate*
  requires an accredited external body we cannot substitute for. Labeled so.
- **NOT-SOUND (research-only)** — the control models a security property the code does
  **not** yet provide to a production standard. The honest disposition; never upgraded to
  PASS by this framework.
- **EXTERNAL-REQUIRED** — the gating control is an external cryptographer / formal-methods
  review that is out of agent scope. Tracked in the gap register.
- **OPEN-gap** — a residual hole (privacy, test-infra) recorded in `gap-register.md` with a
  bead.

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
> **Honesty banner.** Per `SECURITY.md` (§"`sparq-zk` and `sparq-zk-compose` — ZK verifier:
> remediated, but NOT externally audited"), the verifier was originally found unsound, the
> `sq-1s2` remediation has landed, and the internal re-audit found it **"sound as landed for
> the threat model the prior audit assumed"** — but `SECURITY.md` still tells a relying party
> not to present a "verified" result as a production-grade guarantee **until the external
> cryptographer audit (`sq-qhy4`) completes**, and `sparq-mpc` carries **NO guarantee**. No
> row below upgrades that posture. The favourable internal re-audit (CR-3) is recorded as
> *progress evidence*, not as soundness certification — CR-G1 (external audit) is the gate, and
> it is OPEN. [OPUS-4.8]

## Controls

| # | Control | Status | Evidence (file / test / CI / bead) |
|---|---|---|---|
| **CR-1** | **Crypto-estate inventory exists** — every bespoke and standard primitive/library is enumerated with its assurance tier (sound vs research-only). | **PASS** | [`README.md`](./README.md) §1 (Tier-A/Tier-B tables); [`evidence.md`](./evidence.md) §CR-1. Manifests: `crates/sparq-zk/Cargo.toml`, `sparq-zk-compose/Cargo.toml`, `sparq-mpc/Cargo.toml`. |
| **CR-2** | **The original soundness audit is preserved** — the original verdict "v1 verifier is NOT sound (12 findings, 5 CRITICAL)" is kept on record as history and not laundered away (the regression map for the closed findings is bead `sq-1gir`). | **PASS (preserved)** | `research/zk-soundness-audit.md` (full 12-finding report). The original verdict is preserved as history under `SECURITY.md` §"`sparq-zk` and `sparq-zk-compose` — ZK verifier: remediated, but NOT externally audited" (the "**Status — what changed**" paragraph records the predated-BROKEN finding before the remediation). [OPUS-4.8] |
| **CR-3** | **Post-remediation review exists, HONESTLY CALIBRATED** — the remediated verifier was re-audited; its "sound as landed" verdict is recorded **only** as internal, single-model, self-review evidence (necessary, not sufficient), NOT as external certification. | **AUDIT-READY** (internal self-review only; external gate = CR-G1) | `research/zk-verifier-reaudit.md` (bead `sq-gbp4`, closed). **Honesty caveats made explicit:** (i) single-model (Opus 4.8, Fable unavailable, self-flagged "re-review when Fable returns"); (ii) read-only internal pass, not an external accredited audit; (iii) the crypto-chain forge tests (`forge_pubinput_*`) are `#[ignore]`d, so the closure rests on code-reading + one empirical anchor, not on default-CI execution. See [`evidence.md`](./evidence.md) §CR-3. |
| **CR-4** | **ZK verifier-soundness — overall disposition** — what a relying party may rely on TODAY. | **REMEDIATED, NOT EXTERNALLY AUDITED — research-only / EXTERNAL-REQUIRED** | The published posture (`SECURITY.md`) governs: the `sq-1s2` binding layer has landed and the internal re-audit found the verifier "sound as landed for the threat model the prior audit assumed," **but** do **not** present a "verified" result as a production-grade guarantee until CR-G1 (external cryptographer audit, `sq-qhy4`) closes — no production soundness/privacy/integrity guarantee until then. Internal evidence of remediation: `research/zk-verifier-reaudit.md` + the forge suite (CR-6). The 12-finding 1:1 closed-disposition map is in `research/zk-verifier-reaudit.md` §"Recommended beads" table (bead `sq-1gir`). [OPUS-4.8] |
| **CR-5** | **MPC — overall disposition** — `sparq-mpc` provides no production security guarantee. | **NOT-SOUND (research-only)** | `SECURITY.md` §"`sparq-mpc` — cryptography deferred"; `crates/sparq-mpc/Cargo.toml` header ("crypto deferred", `NotYetImplemented` stubs for M4/Q1); `research/mpc-m4-distributed-sig-feasibility.md`; `research/mpc-malicious-security-design.md`. Honest-majority semi-honest Shamir (M0–M3) exists but malicious security / collaborative-proof are DEFERRED. |
| **CR-6** | **Adversarial forge-and-verify negative-test suite exists** — each historical binding gate is proven to *reject* a one-binding-violation forge with the correct `CheckError`. | **PASS** (with `#[ignore]` caveat → CR-G2) | `crates/sparq-zk-compose/tests/forge_gates.rs` (bead `sq-ajl`, closed): `forge_pubinput_statement_substitution_rejected`, `forge_commitment_unsigned_rejected`, `forge_issuer_*`, `forge_nonce_*`, `forge_attribution_*`, `forge_revocation_*`. Plus `join_forge.rs`, `holder_pop_forge.rs`, `audit_forge_map.rs`, `verifier_errors.rs`, `differential_fuzz.rs` (bead `sq-61g`). **Caveat:** the bb-dependent forges are `#[ignore]`d (toolchain-gated) ⇒ OPEN-gap CR-G2. |
| **CR-7** | **Differential prove→verify→cleartext fuzzer** — proven results are differentially checked against a cleartext SPARQL evaluation. | **PASS** | `crates/sparq-zk-compose/tests/differential_fuzz.rs` (bead `sq-61g`, closed). |
| **CR-8** | **Circuit gate-count regression ratchet** — every compiled `zk/compose` member's UltraHonk gate count is pinned in a checked-in snapshot; a recompile that bloats a relation fails. | **PASS** (toolchain-present lane) | `crates/sparq-zk-compose/tests/gate_count.rs` + `tests/gate_count_snapshot.json` (bead `sq-c5f`). Reads real `bb gates -s ultra_honk`; 3% tolerance. Not `#[ignore]`d but skips cleanly when nargo/bb absent. |
| **CR-9** | **Pinned ZK toolchain + forge/anchor CI lane** — the Noir/bb toolchain is version-pinned (never "latest"), and the `#[ignore]`d real-bb forge + empirical-anchor tests run in a dedicated CI lane on zk changes. | **PASS** (path-gated lane) | `.github/workflows/zk-toolchain.yml` (`zk forge + anchor suite (nargo/bb)`); `NARGO_VERSION: 1.0.0-beta.21`, `BB_VERSION` pinned. The check-run always appears (merge_group), real bb steps `if: zk == 'true'`. |
| **CR-10** | **Public-input reconstruction has a NON-CIRCULAR empirical anchor** — the verifier's hand-serialized `reconstruct_public_inputs` (the prior audit's CRITICAL #1) is pinned to bytes captured from a REAL `bb prove`, not to itself. | **PASS** (for `filter_int_d1` + `scan_k1_n16_r4` only → CR-G3) | `crates/sparq-zk-compose/src/verifier.rs` `reconstruct_*_matches_real_bb_public_inputs` tests, anchored by `e2e.rs` `probe_*_public_inputs_hex` (real `bb prove`). **Caveat:** only 2 members anchored; `filter_f64_d*` + k=2 scan members lack a captured golden vector ⇒ OPEN-gap CR-G3. |
| **CR-11** | **Standard primitives used correctly** (Sigstore attestation, SHA-256 digests, RDFC10, OS-CSPRNG). | **PASS** | `.github/workflows/release.yml` (`attest-build-provenance`, `SHA256SUMS`, buildkit `provenance: mode=max`); `crates/sparq-zk/src/canon.rs` + `tests/rdf_canon_suite.rs` (W3C suite); `sparq-zk/src/ingest.rs` (`getrandom` salt mint); `sparq-mpc/src/rng.rs` (ChaCha20 OS-seeded). See [`evidence.md`](./evidence.md) §CR-11. |
| **CR-12** | **Insecure-RNG path is gated OFF by default** — no normal build can construct a predictable masking RNG. | **PASS** | `crates/sparq-mpc/Cargo.toml` `[features] insecure-test-rng = []` (off by default; the feature name is the warning). `sparq-mpc/src/rng.rs` / `shamir.rs` use OS-seeded ChaCha20 in the real protocol (bead `sq-1vt`, resolved). |
| **CR-13** | **Bespoke crypto is NOT shipped / NOT in the default dependency graph** — containment: a downstream consumer cannot reach the Tier-B code via the published artifacts. | **PASS** | `publish = false` in all of `sparq-zk`, `sparq-zk-compose`, `sparq-mpc` (verified); none is a dependency of `sparq-wasm` (absent from `crates/sparq-wasm/Cargo.toml`; verified by the crates' own `cargo tree -p sparq-wasm` invariant notes). |
| **CR-14** | **Constant-time / side-channel posture is documented** — where timing leakage matters and where it is accepted. | **AUDIT-READY** (source-level analysis done; no externally-verified / instrumented CT proof) | A per-component **source-level** side-channel review now exists: [`side-channel-analysis.md`](./side-channel-analysis.md) (bead `sq-egx6`, CR-G5 ADDRESSED-by-analysis). The Schnorr *verify* path (`sparq-zk/src/sig.rs`) is verifier-side over public commitments (no long-term secret in the timing-sensitive path); the MPC paths open only masked products / the verdict bit (operands never reconstructed); Poseidon2 + the MPC bit-circuit are fixed-round with no secret-dependent branch/index; the mask draw is effectively-constant-time-in-expectation (`sparq-mpc/src/rng.rs:45`). Concrete fixes were bead-tracked from that analysis (which itself changed no code): `sq-u8a8` (`subtle`/`zeroize` ABSENT), `sq-7ltf` (latent non-CT `Fp::pow`), `sq-8jv7` (arkworks signing not asserted CT); all have since landed, and the follow-up `sq-j3b9` has replaced the signing path's secret-scalar multiplications with a fixed-width always-add ladder (`sparq-zk/src/ct.rs`) that removes the square-and-multiply control-flow shape. **The documented posture is unchanged by those fixes: signing is still NOT asserted constant-time** — the arkworks field-op residual (the curve/dep swap) remains open. **No instrumented `dudect`/`ctgrind` measurement has been done** (folds into CR-G1's external pass); the arkworks BN254 field ops are not asserted constant-time by sparq. See also [`evidence.md`](./evidence.md) §CR-14. |
| **CR-15** | **FIPS posture is stated honestly** — sparq makes NO FIPS claim and uses no validated module. | **PASS (honest negative claim)** | [`fips-posture.md`](./fips-posture.md) (the standalone posture statement, sq-cu32) + [`evidence.md`](./evidence.md) §CR-15. BN254 / Poseidon2 / Schnorr-over-Baby-JubJub are **not** FIPS-approved algorithms and not intended to be (they are ZK-friendly primitives). SHA-256 (release digests) is FIPS 180-4 but is used for integrity, not as a validated crypto module. No CMVP certificate is claimed (CR-G4). Crate-doc FIPS notes: `crates/sparq-zk-compose/README.md`, `crates/sparq-zk/src/lib.rs`, `crates/sparq-mpc/src/lib.rs`. |
| **CR-16** | **Residual privacy gaps are tracked, not hidden** — the known linkability / binding residuals from the re-audit are on the books. | **OPEN-gap (documented)** | `research/zk-verifier-reaudit.md` NEW-1/NEW-2; beads `sq-hyhj` (in-circuit salt binding + revocation-disclosure residuals), `sq-93h` (per-graph salt disclosed on hidden-only path), `sq-i1dt` (credential-bound HolderPop). Recorded in `gap-register.md` as CR-G6. |

## External / out-of-agent-scope items (label, do NOT claim as PASS)

- **CR-G1 — External accredited-cryptographer audit of the ZK verifier + circuits.** The
  single gating control for any production ZK security claim. An agent cannot substitute for
  it. This slice + the two audit docs + the forge suite are the evidence pack it consumes.
  **CRITICAL, EXTERNAL-REQUIRED.**
- **CR-G4 — FIPS 140-3 CMVP validation.** Not done, not claimed; the estate uses
  non-FIPS-approved ZK primitives by design.
- **Formal verification** of `reconstruct_public_inputs` ↔ circuit-`main` correspondence and
  of the in-circuit relations is not done; assurance is code-reading + empirical anchor +
  forge tests (strong testing, not proof). Folded into CR-G1's scope.

## The honesty test (what would make a row a FINDING)

Any of the following would be an automatic high-severity auditor finding, and none is present
in this table by construction:

1. Marking CR-4 or CR-5 as **PASS** (it would imply the scaffold is secure).
2. Citing the internal re-audit (CR-3) as if it were an **external** certification.
3. A maturity/score statement implying the ZK/MPC estate provides a guarantee `SECURITY.md`
   disclaims.
4. Omitting CR-G1 (the external-audit requirement) or burying it below the PASS rows.
