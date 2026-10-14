// sq-bif.15 — glue test pinning the collaborative-proof / attestation
// crypto boundary's HONEST DEFERRAL at the public-API level. TEST-COVERAGE bead:
// no production logic changed. The crate exposes the `CollaborativeProof` and
// `Attestation` trait surfaces (`crate::proof`) but ships NO implementor — every
// real construction is gated on the ZK-foundation remediation (#3 issuer-sig /
// #4 replay / #5/#6 FILTER-binding / #8/#9 attribution / #12 revocation) AND the
// open Q1 distributed-attestation spike (architecture §5.1 / §5.2 Q1). This test
// implements the public traits with the same HONEST stub the production module
// documents (`MpcError::not_yet`) and pins that the deferred entrypoints fail
// closed with a typed, gate-naming `NotYetImplemented` — never a panic, never a
// fake success. Runs in BOTH feature states (no RNG / sockets). 🤖 SPARQ agent.
//
// HONESTY: sparq-mpc is honest-majority, SEMI-HONEST only; the collaborative ZK
// proof is a STUB. Nothing here claims the proof is sound, malicious-secure, or
// externally audited (sq-qhy4 pending). The test asserts the OPPOSITE — that the
// path is honestly NOT YET IMPLEMENTED.

use sparq_mpc::backend::{
    AbortKind, AdversaryModel, BackendInfo, CorruptionThreshold, MpcBackend, OutputGuarantee,
    PublicVerifiability, SecurityDescriptor,
};
use sparq_mpc::holder::Holder;
use sparq_mpc::proof::Proof;
use sparq_mpc::{
    Attestation, CollaborativeProof, HolderId, JoinPlan, MpcError, PartialResult, ProofStatement,
};

use oxrdf::Variable;

/// A do-nothing backend used ONLY to instantiate the generic `CollaborativeProof`
/// trait surface in a test. Its crypto methods are honest `NotYetImplemented`
/// stubs, exactly as a real backend's would be until the gates land — mirrors
/// the production module's in-crate `StubBackend`.
struct StubBackend;

impl MpcBackend for StubBackend {
    type Share = ();

    fn info(&self) -> BackendInfo {
        // Honest-majority, semi-honest-only descriptor (semi-honest adversary +
        // no-detection selective abort). This is a TEST stub, not a security
        // claim about any deployed backend.
        BackendInfo::new(
            "test-stub",
            SecurityDescriptor {
                adversary: AdversaryModel::SemiHonest,
                output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
                threshold: CorruptionThreshold::HonestMajority { t: 1 },
                public_verifiability: PublicVerifiability(false),
            },
            0,
        )
    }

    fn share_private_input(&self, _h: &Holder) -> Result<Vec<()>, MpcError> {
        Err(MpcError::not_yet("secret-share private input", "test stub"))
    }

    fn run_secure(&self, _s: &[()]) -> Result<Vec<()>, MpcError> {
        Err(MpcError::not_yet("run secure computation", "test stub"))
    }

    fn reconstruct_disclosed(&self, _s: &[()]) -> Result<PartialResult, MpcError> {
        Err(MpcError::not_yet(
            "reconstruct disclosed output",
            "test stub",
        ))
    }
}

/// The honest deferral of the collaborative proof — the SAME contract the
/// production `proof` module documents: `prove` / `verify` return a typed
/// `NotYetImplemented` naming the ZK-foundation gates and the Q1 spike, never a
/// fabricated proof.
struct DeferredProof;

impl CollaborativeProof<StubBackend> for DeferredProof {
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
        // `verify` takes a `&Proof`, whose type is field-private with no public
        // constructor (an honest "no fake proof object" posture), so this arm
        // cannot be EXERCISED from outside the crate. The in-crate
        // `proof::tests::collaborative_proof_is_honestly_deferred` covers it via
        // the `Proof::test_stub()` it can reach. We still implement it honestly
        // so the trait surface is complete.
        Err(MpcError::not_yet(
            "verify collaborative proof",
            "ZK foundation + M4",
        ))
    }
}

/// The honest deferral of the distributed source attestation (the federated
/// lift of ZK-remediation #3).
struct DeferredAttest;

impl Attestation for DeferredAttest {
    fn attest_source(
        &self,
        _h: &HolderId,
        _k: &[String],
    ) -> Result<sparq_mpc::proof::AttestationShare, MpcError> {
        Err(MpcError::not_yet(
            "distributed issuer-signature attestation over secret-shared witness",
            "ZK foundation #3/#8/#9 + Q1",
        ))
    }
}

/// A minimal, real `ProofStatement` (all fields are public). Building the
/// statement is crypto-free — it is the public shape the proof's inputs WILL
/// bind to once the proof exists; constructing it here exercises the public
/// type, not any crypto.
fn statement() -> ProofStatement {
    ProofStatement {
        query: "SELECT ?x WHERE { ?x ?p ?o }".into(),
        disclosed_key_set: vec!["did:example:dmv#key-1".into()],
        challenge: vec![],
        disclosed_result: PartialResult {
            holder: HolderId::new("federation"),
            vars: vec![Variable::new_unchecked("x")],
            rows: vec![],
        },
    }
}

#[test]
fn prove_is_honestly_deferred_with_a_gate_naming_error() {
    let err = DeferredProof
        .prove(&StubBackend, &statement(), &[])
        .expect_err("the collaborative prover must NOT silently succeed");
    match err {
        MpcError::NotYetImplemented { what, gated_on } => {
            // The deferral names WHAT is deferred and the gates blocking it —
            // the auditable-at-the-call-site contract (#3 issuer-sig + Q1 spike).
            assert!(
                what.contains("collaborative proof"),
                "the deferred capability must be named: {what}"
            );
            assert!(
                gated_on.contains("#3"),
                "must cite the issuer-signature gate #3: {gated_on}"
            );
            assert!(
                gated_on.contains("Q1"),
                "must cite the Q1 distributed-attestation spike: {gated_on}"
            );
        }
        other => panic!("prove must return NotYetImplemented, got {other:?}"),
    }
}

#[test]
fn attest_source_is_honestly_deferred_with_a_gate_naming_error() {
    let err = DeferredAttest
        .attest_source(&HolderId::new("alice"), &[])
        .expect_err("attestation must NOT silently succeed");
    match err {
        MpcError::NotYetImplemented { what, gated_on } => {
            assert!(
                what.contains("attestation"),
                "the deferred capability must be named: {what}"
            );
            assert!(
                gated_on.contains("#3") && gated_on.contains("Q1"),
                "must cite issuer-signature #3 + Q1: {gated_on}"
            );
        }
        other => panic!("attest_source must return NotYetImplemented, got {other:?}"),
    }
}

#[test]
fn not_yet_implemented_is_distinct_from_protocol_and_tampered() {
    // The deferral channel must be the DISTINCT honest stub variant, not a
    // generic protocol error or a tampering abort — a relying party (and the
    // privacy-claims gate) must be able to tell "gated, no crypto yet" apart
    // from "precondition violated" or "active tampering detected".
    let deferred = DeferredProof
        .prove(&StubBackend, &statement(), &[])
        .unwrap_err();
    assert!(matches!(deferred, MpcError::NotYetImplemented { .. }));
    assert!(!matches!(deferred, MpcError::Protocol(_)));
    assert!(!matches!(deferred, MpcError::Tampered { .. }));
}
