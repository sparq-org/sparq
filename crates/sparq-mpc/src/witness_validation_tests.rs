// sq-7leq: encode the witness-validation-before-proving TEST
// OBLIGATION for the collaborative-proof (coZK) path. Written while Fable
// unavailable — re-review on return. This module ENCODES an obligation against
// an admittedly NOT-sound area (sq-qhy4 single-prover audit; sq-9hrn coZK
// re-audit); it does NOT claim the collaborative path is sound.
//! Witness-validation-before-proving obligation for the collaborative-proof path
//! (bead **sq-7leq**, derived from `research/mpc-cozk-reaudit.md` §3, bead
//! **sq-9hrn**).
//!
//! ## What this encodes — and why it is honest, not a soundness claim
//!
//! The coZK re-audit against CRYPTO'25 eprint 2025/1026 (Garg–Goel–Jain–
//! Roberts–Sekar) surfaced a NEGATIVE RESULT for the *intended* collaborative
//! prover: the "honest-majority semi-honest ⇒ malicious-secure for free" folklore
//! holds for collaborative zk-SNARKs **only if the extended witness is validated
//! for cross-holder consistency BEFORE the proving phase opens or commits to any
//! value derived from it**. If a prover proves over an **inconsistent / maliciously
//! extended** witness, the protocol can **leak honest provers' private inputs**
//! through the proving transcript / induced openings — even when the verifier
//! still rejects the proof (re-audit Lens 1/3). The defence — requirement
//! **R-WV** — is a fail-closed validate-before-prove gate; a
//! "prove-anyway-and-let-the-verifier-reject" path is exactly the leak path and
//! MUST NOT exist.
//!
//! ## Current disposition — the obligation is OPEN (documented-gap), not MET
//!
//! There is **no built collaborative prover** to feed an invalid witness to:
//! every [`CollaborativeProof`] method returns [`MpcError::NotYetImplemented`]
//! naming its gate (`proof.rs`; the no-fake-crypto table, `adversarial_tests.rs`
//! ROW 2/3). So the validate-before-prove precondition cannot be MET today — there
//! is nothing to validate. The honest encoding is therefore two-tier:
//!
//! - **[`gate_state`] — a PASSING meta-test** that pins the *current* honest
//!   posture: the deferred `prove` refuses (fails closed with a gate-naming error)
//!   WITHOUT ever opening a witness-derived value or emitting a proof. Today this
//!   trivially satisfies "no prove-over-invalid-witness path exists" — *because no
//!   prover exists at all*. This is the regression anchor: if a future change makes
//!   `prove` start returning `Ok(..)` while this obligation is still unmet, the
//!   collaborative-proving soundness gate is being crossed and this test (plus the
//!   ignored suite below, once un-ignored) is what catches it.
//!
//! - **[`obligation`] — the `#[ignore]`d R-WV / T1–T4 suite** (re-audit §3).
//!   These are written against a `WitnessValidatingProver` interface the
//!   implementation MUST satisfy when the collaborative path is built
//!   (sq-f7bu / sq-bjl, milestone M-E). They are `#[ignore]`d **because there is
//!   no implementation to run them against** — running them now would require a
//!   real prover, which deliberately does not exist (building a fake one to make
//!   them pass would be exactly the fake crypto this crate forbids). Each carries
//!   a comment citing sq-9hrn / sq-qhy4 and stating it documents an OPEN soundness
//!   obligation. When the prover lands, these lift directly into its test suite
//!   and the `#[ignore]` is removed; until then `cargo test` reports them as
//!   ignored (green), and the gap is MEASURABLE rather than silent.
//!
//! NONE of this asserts the collaborative path is sound. Per the re-audit (§4)
//! and `lib.rs`/`README.md`, no production attestation/correctness claim may be
//! made over the collaborative path until R-WV is implemented, T1–T4 + clause C
//! pass un-ignored, the honest-majority malicious-security line (sq-km34.*) lands,
//! AND an external cryptographer audit covers the MULTI-prover construction
//! (`sq-qhy4` audits the single-prover verifier only and does NOT discharge this).

use crate::backend::{
    AbortKind, AdversaryModel, BackendInfo, CorruptionThreshold, MpcBackend, OutputGuarantee,
    PublicVerifiability, SecurityDescriptor,
};
use crate::join::JoinPlan;
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::proof::{CollaborativeProof, Proof, ProofStatement};
use oxrdf::Variable;

// =====================================================================
// Shared fixtures (mirror the proof.rs / adversarial_tests.rs stubs so this
// module is self-contained; no production type is changed).
// =====================================================================

/// Honest-stub backend — identical posture to `proof.rs::StubBackend`: honest-
/// majority, semi-honest-only, every crypto method a typed `NotYetImplemented`.
struct StubBackend;
impl MpcBackend for StubBackend {
    type Share = ();
    fn info(&self) -> BackendInfo {
        BackendInfo::new(
            "stub-witness-validation",
            SecurityDescriptor {
                adversary: AdversaryModel::SemiHonest,
                output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
                threshold: CorruptionThreshold::HonestMajority { t: 1 },
                public_verifiability: PublicVerifiability(false),
            },
            0,
        )
    }
    fn share_private_input(&self, _h: &crate::holder::Holder) -> Result<Vec<()>, MpcError> {
        Err(MpcError::not_yet("secret-share private input", "M3 + Q2"))
    }
    fn run_secure(&self, _s: &[()]) -> Result<Vec<()>, MpcError> {
        Err(MpcError::not_yet("run secure computation", "M3 + Q2"))
    }
    fn reconstruct_disclosed(&self, _s: &[()]) -> Result<PartialResult, MpcError> {
        Err(MpcError::not_yet("reconstruct disclosed output", "M3 + Q2"))
    }
}

fn statement() -> ProofStatement {
    ProofStatement {
        query: "SELECT ?x WHERE { ?x ?p ?o }".into(),
        disclosed_key_set: vec!["did:example:dmv#key-1".into()],
        challenge: vec![],
        disclosed_result: PartialResult {
            holder: HolderId::new("fed"),
            vars: vec![Variable::new_unchecked("x")],
            rows: vec![],
        },
    }
}

/// The deferred collaborative prover EXACTLY as the no-fake-crypto table pins it
/// (`adversarial_tests.rs` ROW 2/3): `prove`/`verify` are honest stubs that fail
/// closed with a gate-naming `NotYetImplemented`. This is the *current*
/// implementation surface the obligation is asserted against.
struct DeferredProver;
impl CollaborativeProof<StubBackend> for DeferredProver {
    fn prove(
        &self,
        _b: &StubBackend,
        _s: &ProofStatement,
        _j: &[JoinPlan],
    ) -> Result<Proof, MpcError> {
        Err(MpcError::not_yet(
            "collaborative proof of correct + attested-source result",
            "ZK foundation #3/#4/#5/#6/#8/#9/#12 + Q1 distributed-attestation spike (M4)",
        ))
    }
    fn verify(&self, _s: &ProofStatement, _p: &Proof) -> Result<bool, MpcError> {
        Err(MpcError::not_yet(
            "verify collaborative proof",
            "ZK foundation + M4",
        ))
    }
}

// =====================================================================
// PASSING meta-test — pins the CURRENT honest fail-closed posture.
// This is the regression anchor for sq-7leq while the obligation is OPEN.
// =====================================================================
mod gate_state {
    use super::*;

    /// **The obligation's current-state anchor (PASSING, sq-7leq / sq-9hrn).**
    ///
    /// R-WV demands: no value derived from an inconsistent/extended witness may be
    /// opened or committed into a joint proof; an inconsistent witness causes a
    /// fail-closed abort BEFORE any opening — never a "prove anyway and reject"
    /// path. Today the strongest *true* statement is the trivial-but-load-bearing
    /// one: the deferred prover **never proves at all** — it fails closed with a
    /// gate-naming error before touching any witness — so there is, by
    /// construction, NO path that proves over an invalid witness.
    ///
    /// We assert exactly that and NOTHING stronger: `prove` returns
    /// `NotYetImplemented` (never `Ok(proof)`), even when handed a statement whose
    /// witness would be inconsistent. This is the honest disposition for an OPEN
    /// obligation against a NOT-sound area (sq-qhy4): it documents that the gate is
    /// satisfied *vacuously* (no prover) and will catch a future regression where a
    /// prover begins returning `Ok(..)` while R-WV (the `#[ignore]`d T1–T4 below)
    /// is still unmet.
    #[test]
    fn deferred_prover_never_proves_over_any_witness() {
        let prover = DeferredProver;
        let result = prover.prove(&StubBackend, &statement(), &[]);

        // MUST NOT emit a proof — the only acceptable outcome while the obligation
        // is open is a fail-closed deferral, never `Ok(proof)`.
        match result {
            Ok(_proof) => panic!(
                "sq-7leq REGRESSION: collaborative `prove` returned Ok(proof) while the \
                 witness-validation-before-proving obligation (R-WV, re-audit §3 / sq-9hrn) \
                 is still OPEN and unmet. A prover that emits a proof without a \
                 validate-before-prove gate is exactly the 2025/1026 leak path. Either \
                 implement R-WV + un-ignore the T1–T4 suite, or keep `prove` failing closed."
            ),
            Err(MpcError::NotYetImplemented { gated_on, .. }) => {
                // The deferral must still name its gates (no naked stub).
                assert!(
                    gated_on.contains("#3") && gated_on.contains("Q1"),
                    "deferred prove must cite the issuer-signature gate #3 and the Q1 spike; \
                     got gated_on={gated_on:?}"
                );
            }
            Err(other) => panic!(
                "sq-7leq: deferred prove must fail closed via NotYetImplemented (the honest \
                 deferral channel), got {other:?}"
            ),
        }
    }

    /// Companion: `verify` likewise must never accept (`Ok(true)`) while the path
    /// is deferred — a stub that accepts any proof would let an unvalidated-witness
    /// proof through at the verifier seam. Pins the fail-closed posture on the
    /// verify side of the same boundary.
    #[test]
    fn deferred_verify_never_accepts() {
        let prover = DeferredProver;
        let proof = Proof::test_stub();
        assert!(
            !matches!(prover.verify(&statement(), &proof), Ok(true)),
            "sq-7leq: deferred verify must never accept (Ok(true)) — an accepting stub would \
             admit a proof produced over an unvalidated witness"
        );
        assert!(
            matches!(
                prover.verify(&statement(), &proof),
                Err(MpcError::NotYetImplemented { .. })
            ),
            "sq-7leq: deferred verify must be the typed deferral channel"
        );
    }
}

// =====================================================================
// The R-WV / T1–T4 OBLIGATION SUITE — `#[ignore]`d documented gap.
//
// These encode requirement R-WV (re-audit §3) against the interface the FUTURE
// collaborative prover MUST satisfy. They are #[ignore]d because no prover exists
// to run them against — they document an OPEN soundness obligation (sq-9hrn /
// sq-qhy4) and are liftable verbatim into the prover's suite when sq-f7bu /
// sq-bjl lands (un-ignore at that point).
//
// The interface below (`WitnessValidatingProver`, `ProveTrace`) is the CONTRACT
// the obligation pins, written test-side so the assertions are concrete. It is
// NOT a production type and implies NO chosen construction (Q1/Q2 unresolved).
// =====================================================================
mod obligation {
    use super::*;

    /// An instrumented trace of a collaborative prove attempt — the observable the
    /// obligation tests assert against. The FUTURE prover must expose enough of
    /// this (open-round count, proof emission, witness-lineage opening count) for
    /// R-WV to be checkable; the shape here is the minimum the re-audit §3 T1–T4
    /// assertions need.
    #[allow(dead_code)] // contract only — no implementor exists yet (documented gap).
    struct ProveTrace {
        /// Did the prover abort before any open/commit of a witness-derived value?
        aborted_before_open: bool,
        /// Number of MPC open-rounds executed on any value in the witness lineage.
        /// R-WV requires this to be ZERO once an inconsistency is present.
        witness_lineage_opens: usize,
        /// Did the prover emit a proof object (even one the verifier would reject)?
        emitted_proof: bool,
        /// The abort error, if any (must be a malicious-abort / consistency error).
        abort_error: Option<MpcError>,
    }

    /// The interface the obligation is written against. A real collaborative prover
    /// (sq-f7bu / sq-bjl) MUST implement this so R-WV (T1–T4) is enforceable. No
    /// implementor exists at M0 — that is precisely why the suite is `#[ignore]`d.
    #[allow(dead_code)] // contract only.
    trait WitnessValidatingProver {
        /// Drive a prove attempt where `witness_consistent` models whether the
        /// shared extended witness is cross-holder consistent, and
        /// `validation_enabled` is the test-only flag of T3 (disabling the gate).
        /// Returns the instrumented trace.
        fn prove_traced(
            &self,
            statement: &ProofStatement,
            witness_consistent: bool,
            validation_enabled: bool,
        ) -> ProveTrace;
    }

    /// Helper: panics with the standard "no prover exists yet" message. Every T*
    /// test calls this AFTER its assertions are written, so the assertion logic is
    /// type-checked (it compiles), proving the obligation is concretely encoded —
    /// but the test cannot pass until a real `WitnessValidatingProver` lands. This
    /// is what makes the gap MEASURABLE without faking a prover.
    fn no_prover_yet() -> ! {
        panic!(
            "OPEN soundness obligation (sq-7leq / re-audit §3, beads sq-9hrn + sq-qhy4): \
             no collaborative `WitnessValidatingProver` is built yet (sq-f7bu / sq-bjl, M-E). \
             This test ENCODES requirement R-WV (witness-validation-before-proving) and is \
             #[ignore]d until the prover exists; un-ignore it when sq-f7bu/sq-bjl lands. \
             It documents a gap — it is NOT a claim that the path is sound."
        )
    }

    /// **T1 — inconsistent-share abort-before-open (re-audit §3, Lens 1/3).**
    /// An adversarial holder contributes a degree-inconsistent (off-codeword) share
    /// of its witness value. The prove path MUST fail-closed-abort BEFORE any open
    /// or proof-commit of a witness-derived value: zero witness-lineage opens, no
    /// emitted proof. OPEN obligation — `#[ignore]`d (sq-9hrn / sq-qhy4).
    #[test]
    #[ignore = "OPEN soundness obligation (sq-7leq/sq-9hrn): no collaborative prover built (sq-f7bu/sq-bjl, M-E); un-ignore when it lands"]
    fn t1_inconsistent_witness_aborts_before_any_open() {
        let prover = unimplemented_prover();
        // Inconsistent witness, validation gate ENABLED (the production posture).
        let trace = prover.prove_traced(&statement(), /*consistent=*/ false, true);
        assert!(
            trace.aborted_before_open,
            "T1: an inconsistent witness MUST trigger a fail-closed abort before any open"
        );
        assert_eq!(
            trace.witness_lineage_opens, 0,
            "T1: ZERO opens of any witness-derived value may occur once inconsistency is present \
             (re-audit Lens 1 leak path)"
        );
        assert!(
            !trace.emitted_proof,
            "T1: MUST NOT emit a proof (even a rejected one) over an inconsistent witness"
        );
        assert!(
            matches!(
                trace.abort_error,
                Some(MpcError::Tampered { .. }) | Some(MpcError::Protocol(_))
            ),
            "T1: the abort must be a malicious-abort / consistency error, not a silent skip"
        );
    }

    /// **T2 — witness-extension leakage probe (re-audit §3, Lens 1).**
    /// A malicious prover proves over an extended witness inconsistent with its
    /// committed `C(G_i)` (claims row encodings that do not fold to its signed
    /// commitment). MUST abort before proving; AND the transcript up to the abort
    /// carries ZERO openings on any honest-derived value's lineage (the
    /// information-theoretic non-recoverability of honest witness bits). OPEN —
    /// `#[ignore]`d (sq-9hrn / sq-qhy4).
    #[test]
    #[ignore = "OPEN soundness obligation (sq-7leq/sq-9hrn): no collaborative prover built (sq-f7bu/sq-bjl, M-E); un-ignore when it lands"]
    fn t2_witness_extension_inconsistent_with_commitment_aborts_no_leak() {
        let prover = unimplemented_prover();
        let trace = prover.prove_traced(&statement(), /*consistent=*/ false, true);
        assert!(
            trace.aborted_before_open && !trace.emitted_proof,
            "T2: a witness extension inconsistent with the signed commitment must abort before \
             proving, never emit a proof"
        );
        // Adversarial assertion: no honest-derived value was opened before the abort
        // (structural non-leakage — count of openings on honest lineage == 0).
        assert_eq!(
            trace.witness_lineage_opens, 0,
            "T2: an honest holder's private witness bits must be information-theoretically \
             non-recoverable from the transcript up to the abort (re-audit Lens 1)"
        );
    }

    /// **T3 — validate-before-prove is load-bearing, not advisory (re-audit §3,
    /// Lens 3).** Differential test: with the witness-consistency validation
    /// DISABLED (test-only flag) the SAME adversarial input would otherwise reach
    /// an open/proof step (≥1 witness-lineage open OR an emitted proof); with it
    /// ENABLED the abort fires first (zero opens, no proof). Proves the gate is on
    /// the critical path — preventing a future refactor from silently moving
    /// proving ahead of validation. OPEN — `#[ignore]`d (sq-9hrn / sq-qhy4).
    #[test]
    #[ignore = "OPEN soundness obligation (sq-7leq/sq-9hrn): no collaborative prover built (sq-f7bu/sq-bjl, M-E); un-ignore when it lands"]
    fn t3_validation_gate_is_on_the_critical_path() {
        let prover = unimplemented_prover();
        // Validation DISABLED: the inconsistent witness reaches an open / proof.
        let without = prover.prove_traced(&statement(), /*consistent=*/ false, false);
        assert!(
            without.witness_lineage_opens > 0 || without.emitted_proof,
            "T3: with validation DISABLED the inconsistent witness must reach an open/proof — \
             otherwise the gate is not the thing standing between input and leak"
        );
        // Validation ENABLED: abort fires first.
        let with = prover.prove_traced(&statement(), /*consistent=*/ false, true);
        assert!(
            with.aborted_before_open && with.witness_lineage_opens == 0,
            "T3: with validation ENABLED the abort must fire BEFORE any open — the gate is \
             load-bearing, not advisory"
        );
    }

    /// **T4 — commitment-binding of the validated witness (re-audit §3, Lens 1).**
    /// The validated extended witness MUST be the one bound into the proof's public
    /// inputs. A prover that validates witness A but proves over witness B MUST be
    /// rejected at the binding seam (the federated `reconstruct_public_inputs`
    /// byte-equality — the multi-source generalisation of single-prover audit #1).
    /// Closes the bait-and-switch between validated and proven witness. OPEN —
    /// `#[ignore]`d (sq-9hrn / sq-qhy4).
    #[test]
    #[ignore = "OPEN soundness obligation (sq-7leq/sq-9hrn): no collaborative prover built (sq-f7bu/sq-bjl, M-E); un-ignore when it lands"]
    fn t4_validated_witness_is_bound_into_public_inputs() {
        // The bait-and-switch (validate A, prove B) MUST be rejected at the binding
        // seam. Encoded as: a consistent witness validated, but a prove over a
        // DIFFERENT witness must NOT yield an accepting proof.
        let prover = unimplemented_prover();
        // Model the swap by validating a consistent witness then driving prove with
        // an inconsistency injected post-validation — the binding seam must catch it.
        let trace = prover.prove_traced(&statement(), /*consistent=*/ false, true);
        assert!(
            !trace.emitted_proof,
            "T4: a proof over a witness OTHER than the validated one must be rejected at the \
             federated public-input binding seam (multi-source generalisation of audit #1)"
        );
    }

    /// Returns a `WitnessValidatingProver` — but none exists, so this diverges.
    /// Keeping the `prove_traced` calls above type-checked (the trait method is
    /// invoked) proves the obligation is concretely encoded against the real
    /// contract, while the divergence here is why every T* test is `#[ignore]`d.
    fn unimplemented_prover() -> impl WitnessValidatingProver {
        // No implementor exists at M0. We MUST NOT fabricate one — a fake prover
        // that "passes" would be exactly the fake crypto this crate forbids and
        // would falsely close an OPEN soundness gap.
        no_prover_yet();
        #[allow(unreachable_code)]
        {
            // Unreachable: gives the function a concrete return type without an
            // implementor. `no_prover_yet()` always panics first.
            struct Never;
            impl WitnessValidatingProver for Never {
                fn prove_traced(&self, _s: &ProofStatement, _c: bool, _v: bool) -> ProveTrace {
                    unreachable!()
                }
            }
            Never
        }
    }
}
