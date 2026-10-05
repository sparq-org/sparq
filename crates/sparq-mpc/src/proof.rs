// [OPUS-4.8] CollaborativeProof / Attestation — the ZKP boundary (deferred).
//! The collaborative ZK proof + distributed attestation boundary.
//!
//! Architecture refs: §4.3 step 5 (the provers jointly emit ONE proof binding
//! correctness AND attestation AND freshness), §4.4 (reuse of the RQ1 ZK
//! estate), §5.1 (the HARD DEPENDENCY — gating, not optional), and the
//! principal RESEARCH RISK **§5.2 Q1** (distributed signature/commitment-opening
//! inside the collaborative proof — "the join nobody has built").
//!
//! ## The two halves of the guarantee (§4.3 step 5)
//!
//! The proof the federation emits must bind BOTH:
//! - **(a) Correctness:** `Disclosed(π) ⊆ Eval_PAG(Q, D)` — the disclosed
//!   result is the correct PAG-semantics evaluation. Excludes
//!   disclosed-property aggregates/ordering, which the verifier recomputes
//!   (convention #4).
//! - **(b) Attested source:** each contributing commitment is issuer-signed
//!   under a key in the disclosed key-set `K`, revealing *membership* not
//!   *identity* ("derived from facts issued by SOME key in K", convention #3).
//!
//! Plus the binding inputs that close the RQ1 replay/binding gaps: the query
//! digest, FILTER operator/operand/verdict, per-row source-graph attribution,
//! and the verifier's fresh challenge `N`.
//!
//! ## HARD DEPENDENCY — why every method here is a stub (§5.1)
//!
//! No MPC+ZK proof can mean anything to a relying party until the RQ1
//! verifier-soundness remediation lands. The MPC attestation step is literally
//! ZK-remediation **#3 generalised to multiple sources** and cannot exist
//! before #3 lands single-source. The full gating set
//! (`research/zk-soundness-audit.md`):
//! - **#3** issuer-signature + key-set membership — without it the prover is the
//!   issuer of every fact; the entire "attested sources" guarantee is absent.
//! - **#4** replay/freshness binding — without it every proof is replayable.
//! - **#5 / #6** FILTER operator/bound/verdict + operand-slot binding — the
//!   £25k/£100k threshold IS the FILTER; directly load-bearing for the use case.
//! - **#8 / #9** per-row source-graph attribution in-circuit + salt separation —
//!   the inter-source disclosure control RQ2a depends on.
//! - **#12** revocation — a revoked credential must not feed a federated
//!   aggregate.
//!
//! (#1/#2 — public-input reconstruction + canonical-vk pinning — are already
//! fixed on main; this layer plugs into that hardened binding seam, §4.4.)
//!
//! ## Q1 — THE RESEARCH-RISK MILESTONE (§5.2 Q1)
//!
//! Verifying a holder's BBS+/EdDSA signature over a *secret-shared* witness in
//! an MPC-friendly way is UNSOLVED in the literature; the ~4.2K-constraint
//! EdDSA-Poseidon figure is single-prover. Composing collaborative proving
//! (Ozdemir–Boneh, and note the 2025/1026 soundness pitfalls — patched
//! constructions + a fresh audit are mandatory, §5.2 Q4) with authenticated-
//! input MPC (Dutta, honest-majority only) over global-IRI joins has ZERO
//! published instances. This is the contribution AND the risk. The honest plan
//! (`PLAN.md` M4) is to SPIKE it, not to schedule it as routine engineering.
//!
//! Accordingly every method here returns [`MpcError::NotYetImplemented`] naming
//! the gate. No method fakes a signature, a commitment opening, or a proof.
//!
//! ## The buildable M4-v1 interim vs the Q1 spike [OPUS-5]
//!
//! `research/mpc-m4-distributed-sig-feasibility.md` §2 separates the *unsolved*
//! Q1 join above from an honestly BUILDABLE interim — verifier-side attestation
//! (Artemis commit-and-prove anchor + Dutta authenticated input), where the
//! issuer signature is checked OUTSIDE the circuit over each already-public
//! commitment. Its two non-research prerequisites (§4 item 4) live in
//! [`crate::federated_binding`]: the explicit freshness/replay binding that
//! out-of-circuit signature checking makes necessary (#4 stops being automatic),
//! and the federated/multi-source public-input layout spanning all holders'
//! commitments/rows/attribution.
//!
//! That module is **layout + out-of-circuit binding only**. It does not weaken
//! the gate stated above, and nothing in this module changes: `prove`/`verify`
//! remain [`MpcError::NotYetImplemented`] until the ZK foundation lands and a
//! collaborative proof exists to emit those public inputs.

use crate::backend::MpcBackend;
use crate::join::JoinPlan;
use crate::partial::{HolderId, MpcError, PartialResult};

/// The public statement a [`CollaborativeProof`] is about — everything the
/// verifier reconstructs and checks the proof against (architecture §4.3 step
/// 6). At M0 this is the *shape* of the statement, carried as plain data; it is
/// what the proof's public inputs must bind to once the proof exists. Building
/// it is crypto-free, but it has no meaning until the binding layer (#1–#12)
/// can enforce it, so it is documented here rather than constructed.
#[derive(Debug, Clone)]
pub struct ProofStatement {
    /// The full normalized SPARQL query (its canonical digest is bound into the
    /// proof's public inputs — closes #10 query-text binding).
    pub query: String,
    /// The disclosed key-set `K`: the issuer keys under which contributing
    /// commitments must be signed (attestation half, convention #3 / #3-issue).
    /// Modelled as opaque key identifiers at M0 — the key/signature crypto is
    /// gated on #3.
    pub disclosed_key_set: Vec<String>,
    /// The verifier-issued fresh challenge `N` (freshness binding, #4). The
    /// verifier injects its OWN value when checking; a prover-chosen value here
    /// is only a placeholder for the statement shape.
    pub challenge: Vec<u8>,
    /// The disclosed result the proof attests is `⊆ Eval_PAG(Q, D)`.
    pub disclosed_result: PartialResult,
}

/// The attestation half (§4.3 step 5(ii)): proof that a contributing
/// commitment is issuer-signed under a key in `K`, **distributed** across the
/// holders because no single holder knows the others' witnesses.
///
/// This is ZK-remediation **#3 lifted to the federated setting**, and the
/// distributed-signature-over-secret-shared-witness inside it is exactly Q1.
/// No construction exists at M0.
pub trait Attestation {
    /// Produce the attestation that `holder`'s contribution derives from a
    /// commitment signed by a key in `key_set`, in a form that composes into a
    /// collaborative proof over secret-shared witnesses.
    ///
    /// Gated on ZK-remediation #3 (issuer-signature) / #8–#9 (attribution) and
    /// open Q1; returns [`MpcError::NotYetImplemented`].
    fn attest_source(
        &self,
        holder: &HolderId,
        key_set: &[String],
    ) -> Result<AttestationShare, MpcError>;
}

/// Opaque placeholder for one holder's contribution to the distributed
/// attestation. Intentionally carries no data at M0 — its representation is
/// dictated by the (unchosen) collaborative-proof construction (Q1) and the
/// (unchosen) backend (Q2), so committing to fields now would be fake crypto.
#[derive(Debug, Clone)]
pub struct AttestationShare {
    /// Marker so the type is constructible in tests without implying structure.
    _private: (),
}

/// The collaborative ZK proof boundary: the holders (as provers) jointly emit
/// ONE proof binding correctness (a) AND attested source (b) AND the freshness/
/// query/FILTER/attribution inputs, verifiable by the relying party with an
/// unchanged single-prover verifier (architecture §4.3 step 5, §3.4 coZK).
///
/// ## Status — entirely deferred
/// Generic over the chosen [`MpcBackend`] (Q2) because a collaborative proof
/// over secret-shared witnesses is parameterised by the sharing scheme. The
/// `B: MpcBackend` bound documents that coupling without choosing a scheme. No
/// implementor exists; both methods return [`MpcError::NotYetImplemented`].
pub trait CollaborativeProof<B: MpcBackend> {
    /// Jointly produce one proof for `statement` over the holders' secret-shared
    /// witnesses, given the cross-holder `joins` that shaped the result.
    ///
    /// HARD-blocked on the ZK foundation (#3/#4/#5/#6/#8/#9/#12) AND the Q1
    /// distributed-attestation spike (M4). Returns
    /// [`MpcError::NotYetImplemented`].
    fn prove(
        &self,
        backend: &B,
        statement: &ProofStatement,
        joins: &[JoinPlan],
    ) -> Result<Proof, MpcError>;

    /// Verifier side: check a [`Proof`] against a freshly-reconstructed
    /// statement (the verifier's OWN challenge, canonical query digest, and key
    /// set), recompute disclosed-property aggregates outside the proof, and
    /// accept the minimal answer (§4.3 step 6).
    ///
    /// Meaningless until the binding layer exists; returns
    /// [`MpcError::NotYetImplemented`].
    fn verify(&self, statement: &ProofStatement, proof: &Proof) -> Result<bool, MpcError>;
}

/// Opaque placeholder for the emitted collaborative proof. No fields at M0: the
/// proof system (Noir/UltraHonk vs collaborative-SNARK) is unchosen, and a
/// concrete byte layout now would be fake crypto.
#[derive(Debug, Clone)]
pub struct Proof {
    _private: (),
}

impl Proof {
    /// Test-only constructor for the opaque proof placeholder. The deferred
    /// `CollaborativeProof::verify` stub ignores its `&Proof` argument (it returns
    /// the typed `NotYetImplemented` before reading it), but the trait still needs
    /// a value to pass. This exists ONLY so the `no_fake_crypto` adversarial table
    /// (sq-nuok) can call `verify` and assert it fails closed. It implies NO byte
    /// layout — the proof type is deliberately field-less at M0. [OPUS-4.8]
    #[cfg(test)]
    pub(crate) fn test_stub() -> Self {
        Proof { _private: () }
    }
}

#[cfg(test)]
mod tests {
    //! M0 contract tests: the crypto boundary must COMPILE and every deferred
    //! method must fail with a clear, gate-naming [`MpcError::NotYetImplemented`]
    //! — never a panic, never a fake success.
    use super::*;
    use crate::backend::{
        AbortKind, AdversaryModel, BackendInfo, CorruptionThreshold, MpcBackend, OutputGuarantee,
        PublicVerifiability, SecurityDescriptor,
    };
    use oxrdf::Variable;

    /// A do-nothing backend used ONLY to instantiate the generic trait surface
    /// in tests. Its crypto methods are honest stubs, exactly like a real
    /// backend will be until M3.
    struct StubBackend;
    impl MpcBackend for StubBackend {
        type Share = ();
        fn info(&self) -> BackendInfo {
            // [OPUS-4.8] sq-mq8q — honest-majority, semi-honest-only stub via the
            // three-axis descriptor; `BackendInfo::new` derives the back-compat
            // `trust_model = HonestMajority` / `malicious_security = SemiHonestOnly`
            // projection (semi-honest adversary + no-detection selective abort).
            BackendInfo::new(
                "stub",
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

    fn empty_statement() -> ProofStatement {
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

    /// The hard dependency is honest: proving fails with a gate-naming error
    /// that cites the ZK-remediation issues and the Q1 spike — it does NOT
    /// silently succeed.
    #[test]
    fn collaborative_proof_is_honestly_deferred() {
        struct StubProof;
        impl CollaborativeProof<StubBackend> for StubProof {
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

        let err = StubProof
            .prove(&StubBackend, &empty_statement(), &[])
            .unwrap_err();
        match err {
            MpcError::NotYetImplemented { gated_on, .. } => {
                assert!(
                    gated_on.contains("#3"),
                    "must cite issuer-signature gate #3"
                );
                assert!(gated_on.contains("Q1"), "must cite the Q1 spike");
            }
            other => panic!("expected NotYetImplemented, got {other:?}"),
        }
    }

    /// Attestation (the distributed #3) is likewise an honest stub naming its
    /// gate.
    #[test]
    fn attestation_is_honestly_deferred() {
        struct StubAttest;
        impl Attestation for StubAttest {
            fn attest_source(
                &self,
                _h: &HolderId,
                _k: &[String],
            ) -> Result<AttestationShare, MpcError> {
                Err(MpcError::not_yet(
                    "distributed issuer-signature attestation over secret-shared witness",
                    "ZK foundation #3/#8/#9 + Q1",
                ))
            }
        }
        let err = StubAttest
            .attest_source(&HolderId::new("alice"), &[])
            .unwrap_err();
        assert!(matches!(err, MpcError::NotYetImplemented { .. }));
    }
}
