// [OPUS-4.8] sq-hbg7 (CI coverage ratchet): coverage for the PROVER-FREE surface
// of `verifier.rs` — the `CheckError` Display impl (every reject reason a relying
// party surfaces) and the external trust-anchor constructors (`KeySet`,
// `HolderRegistry`, `EntailmentPolicy`, `RevocationPolicy`). These run with no
// `nargo`/`bb` toolchain (CI's coverage environment).
//
// WHY THIS IS HONEST COVERAGE, NOT PADDING
// ----------------------------------------
// `CheckError` is the verifier's user-facing rejection vocabulary: each variant's
// Display string is the diagnostic a relying party logs/shows when a proof is
// refused. A wrong or empty message is a real defect (an operator cannot tell WHY
// a credential was rejected — exactly the kind of fail-closed-but-opaque outcome
// this security code must avoid). So the assertions below check each message is
// non-empty AND carries the variant's identifying fields (proof/edge index, the
// offending commitment/key/holder, the audit/feature tag). That is a meaningful
// contract test of the diagnostics, which also happens to cover the ~225-line
// Display match arm that the bb-gated e2e tests reach only on the rare paths a
// live prover drives. The constructors are the public trust-anchor API; their
// fail-closed defaults (empty == trusts nothing) are asserted, not just executed.

use sparq_zk::verify::VerifyError;
// [OPUS-5] sq-rsd3v.7: the two UNBUILT completeness halves, as the refusal message
// and this test's needles must agree on them.
use sparq_zk_compose::derivation::COMPLETENESS_UNDER_ENTAILMENT_UNBUILT;
use sparq_zk_compose::manifest::{CircuitId, EntailmentRegime, StatusListSnapshot};
use sparq_zk_compose::verifier::{
    CheckError, EntailmentPolicy, HolderRegistry, RevocationPolicy,
};
// KeySet is re-used from sparq-zk; the compose crate's verifier re-exports it.
use sparq_zk_compose::verifier::KeySet;

/// Assert a CheckError's Display is non-empty and contains every needle (so the
/// variant's identifying fields actually reach the operator-facing message).
fn assert_display_carries(err: &CheckError, needles: &[&str]) {
    let s = format!("{err}");
    assert!(!s.is_empty(), "empty display for {err:?}");
    for n in needles {
        assert!(
            s.contains(n),
            "display for {err:?} missing needle {n:?}:\n  {s}"
        );
    }
    // The Debug impl is also exercised (derive(Debug)).
    assert!(!format!("{err:?}").is_empty());
}

#[test]
fn check_error_display_covers_structural_reject_reasons() {
    assert_display_carries(
        &CheckError::Sparqzk(VerifyError::Parse("bad }".into())),
        &["sparq-zk", "bad }"],
    );
    assert_display_carries(
        &CheckError::CircuitIdMismatch {
            proof: 3,
            declared: CircuitId::FilterInt { d: 1 },
            derived: Some(CircuitId::FilterInt { d: 2 }),
        },
        &["3", "FilterInt"],
    );
    assert_display_carries(&CheckError::DanglingEdge { edge: 5 }, &["edge", "5"]);
    assert_display_carries(&CheckError::EdgeKindMismatch { edge: 6 }, &["edge", "6"]);
    assert_display_carries(&CheckError::BindingInconsistent { edge: 7 }, &["edge", "7"]);
    assert_display_carries(&CheckError::ProofRejected { proof: 1 }, &["bb", "1"]);
    assert_display_carries(&CheckError::MissingProof { proof: 2 }, &["2"]);
    assert_display_carries(&CheckError::MalformedProof { proof: 4 }, &["4", "malformed"]);
    assert_display_carries(
        &CheckError::MalformedField { proof: 0, what: "bound" },
        &["bound"],
    );
    assert_display_carries(
        &CheckError::PublicInputMismatch { proof: 9 },
        &["9", "public inputs"],
    );
    // [OPUS-4.8] sq-vxq8: distinct-graph strict-ordering reject (duplicate-inclusion).
    assert_display_carries(
        &CheckError::ScanCommitmentsNotStrictlyIncreasing { proof: 0, at: 1 },
        &["strictly increasing", "S2.5"],
    );
    // [OPUS-4.8] Copilot PR #95: Display must be panic-free for EVERY constructible
    // value, including the `at == 0` boundary (the prior `at - 1` would underflow:
    // debug panic / release wrap). `saturating_sub(1)` keeps it total. The current
    // constructor never emits `at == 0`, but Display correctness is independent of
    // that and the assertion documents the contract.
    assert_display_carries(
        &CheckError::ScanCommitmentsNotStrictlyIncreasing { proof: 0, at: 0 },
        &["strictly increasing", "S2.5"],
    );
    // [OPUS-4.8] sq-sfsi: hidden cross-credential JOIN bind_joins reject reasons.
    assert_display_carries(&CheckError::JoinDanglingEdge { edge: 2 }, &["join edge", "2"]);
    assert_display_carries(&CheckError::JoinEdgeKindMismatch { edge: 3 }, &["join edge", "3"]);
    assert_display_carries(
        &CheckError::JoinCommitmentMismatch { edge: 4 },
        &["join edge", "4", "commit_a"],
    );
    assert_display_carries(
        &CheckError::JoinSlotMismatch { edge: 5 },
        &["join edge", "5", "slot"],
    );
    // [OPUS-4.8] sq-r2s8: N-way join chain commitment-equality reject reason.
    assert_display_carries(
        &CheckError::JoinCommitmentChainMismatch { edge: 6 },
        &["join edge", "6", "N-way"],
    );
}

#[test]
fn check_error_display_covers_query_correctness_reasons() {
    assert_display_carries(&CheckError::UnboundPattern { pattern: 2 }, &["pattern", "2"]);
    assert_display_carries(
        &CheckError::UnboundFilter { variable: "age".into() },
        &["age", "FILTER"],
    );
    assert_display_carries(
        &CheckError::UnmappableFilterVar { variable: "x".into() },
        &["x", "FILTER"],
    );
    assert_display_carries(
        &CheckError::AttributionUnderDeclared { pattern: 0, proof_graph: 1 },
        &["pattern", "1"],
    );
    assert_display_carries(&CheckError::AttributionUnbound { pattern: 3 }, &["pattern", "3"]);
    assert_display_carries(
        &CheckError::AttributionMalformed { proof: 0, expected: 2, got: 1 },
        &["2", "1"],
    );
    // [OPUS-5] sq-q9r5e follow-up: the explicit pattern→scan mapping's rejections.
    assert_display_carries(
        &CheckError::PatternScanArityMismatch { patterns: 2, declared: 1 },
        &["pattern_scans", "2", "1"],
    );
    assert_display_carries(
        &CheckError::PatternScanUnbound { pattern: 1 },
        &["pattern_scans", "1"],
    );
    assert_display_carries(
        &CheckError::PatternScanMismatch { pattern: 0, proof: 4 },
        &["pattern_scans", "0", "4"],
    );
    assert_display_carries(
        &CheckError::PatternScanUndeclared { proof: 3 },
        &["pattern_scans", "3"],
    );
    // [OPUS-5] #5240: within-pattern repeated-variable slot equality.
    assert_display_carries(
        &CheckError::RepeatedSlotMismatch {
            pattern: 0,
            proof: 1,
            row: 2,
            variable: "v".into(),
            slots: (0, 2),
        },
        &["v", "pattern", "row"],
    );
}

#[test]
fn check_error_display_covers_issuer_and_salt_reasons() {
    assert_display_carries(
        &CheckError::UnattestedCommitment { proof: 0, commitment: "0xc0".into() },
        &["0xc0", "attestation"],
    );
    assert_display_carries(
        &CheckError::InvalidIssuerSignature { commitment: "0xc1".into() },
        &["0xc1", "signature"],
    );
    assert_display_carries(
        &CheckError::IssuerKeyNotInKeySet { commitment: "0xc2".into() },
        &["0xc2", "K"],
    );
    assert_display_carries(
        &CheckError::UntrustedDeclaredKey { key: "0xkey".into() },
        &["0xkey"],
    );
    assert_display_carries(
        &CheckError::AttestationKeyNotInDeclaredSet {
            commitment: "0xc3".into(),
            key: "0xk3".into(),
        },
        &["0xc3", "0xk3"],
    );
    assert_display_carries(&CheckError::SaltReused { salt: "0xs".into() }, &["0xs", "salt"]);
    assert_display_carries(
        &CheckError::ScanCommitmentSaltMissing { proof: 1, commitment: "0xc4".into() },
        &["0xc4", "salt"],
    );
}

#[test]
fn check_error_display_covers_nonce_and_revocation_reasons() {
    assert_display_carries(&CheckError::NonceReplay, &["nonce", "single-use"]);
    assert_display_carries(&CheckError::NonceBindingMismatch, &["nonce"]);
    assert_display_carries(
        &CheckError::ScanCommitmentStatusMissing { proof: 0, commitment: "0xc5".into() },
        &["0xc5", "status"],
    );
    assert_display_carries(&CheckError::RevocationReferenceMissing { proof: 2 }, &["2"]);
    assert_display_carries(
        &CheckError::RevocationReferenceMismatch { commitment: "0xc6".into() },
        &["0xc6"],
    );
    assert_display_carries(
        &CheckError::RevocationReferenceModeInvalid { commitment: "0xc7".into() },
        &["0xc7"],
    );
    assert_display_carries(&CheckError::HiddenRevocationRequired { proof: 1 }, &["1"]);
    assert_display_carries(&CheckError::HiddenRevocationIndexCommitmentMismatch, &["index commitment"]);
    assert_display_carries(
        &CheckError::StatusSnapshotMissing { status_list: "http://ex/l".into(), version: 3 },
        &["http://ex/l", "3"],
    );
    assert_display_carries(
        &CheckError::StatusSnapshotTampered { status_list: "http://ex/l".into(), version: 4 },
        &["http://ex/l", "4"],
    );
    assert_display_carries(
        &CheckError::CredentialRevoked { status_list: "http://ex/l".into(), index: 5 },
        &["http://ex/l", "5", "REVOKED"],
    );
    assert_display_carries(
        &CheckError::StatusListStale { status_list: "http://ex/l".into(), version: 6 },
        &["http://ex/l", "6"],
    );
}

#[test]
fn check_error_display_covers_hidden_revocation_reasons() {
    assert_display_carries(&CheckError::HiddenRevocationNotEnabled, &["hidden-index"]);
    assert_display_carries(
        &CheckError::HiddenRevocationDepthMismatch { declared: 10, policy: 8 },
        &["10", "8"],
    );
    assert_display_carries(
        &CheckError::HiddenRevocationRootUnavailable {
            status_list: "http://ex/l".into(),
            version: 1,
        },
        &["http://ex/l"],
    );
    assert_display_carries(&CheckError::HiddenRevocationRootMismatch, &["root"]);
    assert_display_carries(&CheckError::HiddenRevocationProofRejected, &["bb"]);
    assert_display_carries(&CheckError::HiddenRevocationMalformedProof, &["malformed"]);
}

#[test]
fn check_error_display_covers_hidden_issuer_reasons() {
    assert_display_carries(&CheckError::HiddenIssuerNotEnabled, &["hidden-issuer"]);
    assert_display_carries(
        &CheckError::HiddenIssuerDepthMismatch { declared: 4, policy: 5 },
        &["4", "5"],
    );
    assert_display_carries(&CheckError::HiddenIssuerRootUnavailable, &["key-set"]);
    assert_display_carries(&CheckError::HiddenIssuerRootMismatch, &["root"]);
    assert_display_carries(
        &CheckError::HiddenIssuerMessageMismatch { commitment: "0xc8".into() },
        &["0xc8", "message"],
    );
    assert_display_carries(
        &CheckError::HiddenIssuerUnreferencedCommitment { commitment: "0xc9".into() },
        &["0xc9"],
    );
    assert_display_carries(&CheckError::HiddenIssuerProofRejected, &["bb"]);
    assert_display_carries(&CheckError::HiddenIssuerMalformedProof, &["malformed"]);
}

// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): the in-circuit holder-PoK binding-edge
// reasons each render their commitment + a stable sq-c2ql phrase, so a verifier
// rejection names the binding-edge cause precisely.
#[test]
fn check_error_display_covers_holder_pok_reasons() {
    assert_display_carries(
        &CheckError::HolderPokUnreferencedCommitment { commitment: "0xd1".into() },
        &["0xd1", "sq-c2ql"],
    );
    assert_display_carries(
        &CheckError::HolderPokBindingMissing { commitment: "0xd2".into() },
        &["0xd2", "sq-c2ql"],
    );
    assert_display_carries(
        &CheckError::HolderPokMissing { commitment: "0xd3".into() },
        &["0xd3", "sq-c2ql"],
    );
    assert_display_carries(
        &CheckError::HolderPokDigestMismatch { commitment: "0xd4".into() },
        &["0xd4", "binding edge"],
    );
    assert_display_carries(
        &CheckError::HolderPokProofRejected { commitment: "0xd5".into() },
        &["0xd5", "bb"],
    );
    assert_display_carries(&CheckError::HolderPokMalformedProof, &["malformed", "sq-c2ql"]);
}

#[test]
fn check_error_display_covers_holder_pop_and_entailment_reasons() {
    assert_display_carries(&CheckError::HolderRegistryEmpty, &["HolderPop", "registry"]);
    assert_display_carries(
        &CheckError::HolderNotTrusted { holder: "0xh".into() },
        &["0xh", "registry"],
    );
    assert_display_carries(&CheckError::HolderPopMalformed, &["HolderPop"]);
    assert_display_carries(
        &CheckError::HolderPopInvalid { holder: "0xh2".into() },
        &["0xh2"],
    );
    assert_display_carries(
        &CheckError::EntailmentRegimeNotAccepted { regime: "rdfs" },
        &["rdfs", "EntailmentPolicy"],
    );
    assert_display_carries(&CheckError::UnexpectedDerivationSteps, &["Simple"]);
    assert_display_carries(
        &CheckError::MissingDerivationSteps { regime: "owl" },
        &["owl"],
    );
    assert_display_carries(&CheckError::MalformedDerivationStep { step: 2 }, &["2"]);
    assert_display_carries(
        &CheckError::UngroundedDerivationAntecedent { step: 1, antecedent: 0 },
        &["1", "0"],
    );
    // [OPUS-5] sq-rsd3v.7: the completeness-under-entailment refusal must name the
    // regime AND BOTH unbuilt halves (the closure-sweep and the saturation proof) —
    // a relying party that asked for completeness has to be able to read WHY it
    // cannot have it, and that the accept it did not get would only ever have been
    // soundness of derivation. The needles come from the single source of truth so
    // the message cannot drift away from the documented obligation.
    assert_display_carries(
        &CheckError::CompletenessUnderEntailmentUnavailable { regime: "rdfs" },
        &[
            "rdfs",
            "sq-rsd3v.7",
            COMPLETENESS_UNDER_ENTAILMENT_UNBUILT[0],
            COMPLETENESS_UNDER_ENTAILMENT_UNBUILT[1],
        ],
    );
    // sq-rsd3v.6: the message must name owl:sameAs and the bead, so an operator
    // reading a rejection knows the refusal is deliberate (equality reasoning
    // needs the separate canonicalisation member) rather than a shape bug.
    assert_display_carries(
        &CheckError::EqualityReasoningUnsupported { step: 3 },
        &["3", "owl:sameAs", "sq-rsd3v.6"],
    );
}

#[test]
fn check_error_is_a_std_error() {
    // The std::error::Error impl is wired (Display + Debug). A caller can box it.
    let e = CheckError::NonceReplay;
    let boxed: Box<dyn std::error::Error> = Box::new(e);
    assert!(boxed.source().is_none());
    assert!(!boxed.to_string().is_empty());
}

// =========================================================================
// External trust-anchor constructors (the public verifier API)
// =========================================================================

#[test]
fn keyset_empty_trusts_nothing_and_overflow_root_is_none() {
    let k = KeySet::empty();
    assert!(k.is_empty(), "an empty KeySet trusts no issuer (fail-closed default)");
    // No keys => no authoritative hidden-issuer root member index.
    assert_eq!(k.member_index("00"), None);
    // A hidden-issuer root over an empty set at depth 0 is defined (single padding
    // leaf), but at an implausible depth it is None (overflow / fail-closed).
    assert_eq!(k.hidden_issuer_root(32), None);
}

#[test]
fn keyset_from_hex_drops_unparseable_keys_failclosed() {
    // Unparseable / non-curve hex entries are DROPPED (they can never match a real
    // attestation key, so dropping is fail-closed — never a silent trust widening).
    let k = KeySet::from_hex_keys(["not-a-key", "zzzz", ""]);
    assert!(k.is_empty(), "garbage keys are dropped, leaving an empty (no-trust) set");

    // A KeySet builder enabling the hidden-issuer path keeps the trust set intact.
    let enabled = KeySet::empty().with_hidden_issuer_depth(4);
    assert!(enabled.is_empty());
}

#[test]
fn holder_registry_empty_and_garbage_are_failclosed() {
    let r = HolderRegistry::empty();
    assert!(r.is_empty(), "an empty registry trusts no holder (HolderPop rejected)");
    let g = HolderRegistry::from_hex_keys(["nope", "1g"]);
    assert!(g.is_empty(), "unparseable holder keys are dropped fail-closed");
    // Default == empty.
    assert_eq!(HolderRegistry::default(), HolderRegistry::empty());
}

#[test]
fn entailment_policy_default_is_simple_only_and_owl_subsumes_rdfs() {
    // Default / simple_only accept ONLY Simple (the conservative anchor). We assert
    // via the public verifier behaviour: bind_entailment is private, but accepts()
    // is exercised end-to-end by the e2e entailment tests; here we assert the
    // builder identities the policy guarantees.
    assert_eq!(EntailmentPolicy::default(), EntailmentPolicy::simple_only());
    // with_owl subsumes rdfs (Owl ⊇ Rdfs): the owl-enabled policy equals the
    // rdfs-then-owl chain, demonstrating the subsumption the builder documents.
    let owl = EntailmentPolicy::simple_only().with_owl();
    let rdfs_then_owl = EntailmentPolicy::simple_only().with_rdfs().with_owl();
    assert_eq!(owl, rdfs_then_owl, "with_owl implies with_rdfs (Owl subsumes RDFS)");
    // rdfs-only differs from owl-enabled.
    assert_ne!(EntailmentPolicy::simple_only().with_rdfs(), owl);
    // EntailmentRegime is Copy/Eq — touch it so the import is load-bearing.
    assert_eq!(EntailmentRegime::Simple, EntailmentRegime::Simple);
}

/// [OPUS-5] sq-rsd3v.7: the completeness-under-entailment dial is a DISTINCT
/// dimension of the policy, not a re-spelling of regime acceptance — demanding
/// completeness must not be satisfiable by opting into a regime. Asserted on the
/// builder identities (the refusal behaviour itself is asserted end-to-end in
/// `e2e.rs`, where a manifest actually reaches `bind_entailment`).
#[test]
fn completeness_demand_is_orthogonal_to_regime_acceptance() {
    let base = EntailmentPolicy::simple_only().with_owl();
    let demanding = base.clone().require_completeness_under_entailment();
    assert_ne!(
        base, demanding,
        "requiring completeness must be observable in the policy — accepting Owl \
         does not supply completeness (sq-rsd3v.7 is UNBUILT)"
    );
    // Idempotent, and it never widens regime acceptance.
    assert_eq!(
        demanding.clone().require_completeness_under_entailment(),
        demanding,
        "the dial is a set-once requirement, not a counter"
    );
    assert_ne!(
        EntailmentPolicy::simple_only().require_completeness_under_entailment(),
        demanding,
        "the dial must not imply with_owl (it accepts no regime it did not already)"
    );
    // The default policy demands nothing (so no existing relying party changes
    // behaviour) — the honest default stays "Simple-only, no completeness demand".
    assert_eq!(EntailmentPolicy::default(), EntailmentPolicy::simple_only());
    assert_ne!(
        EntailmentPolicy::default(),
        EntailmentPolicy::simple_only().require_completeness_under_entailment()
    );
    // Both unbuilt halves are named, and the saturation half — the one with no
    // precedent anywhere in sparq — is explicitly among them.
    assert_eq!(COMPLETENESS_UNDER_ENTAILMENT_UNBUILT.len(), 2);
    assert!(
        COMPLETENESS_UNDER_ENTAILMENT_UNBUILT.iter().any(|h| h.contains("saturation")),
        "the fixpoint-saturation obligation must be named, never elided"
    );
}

#[test]
fn revocation_policy_constructors_and_hidden_depth_builder() {
    // accept_version pins a single version (window 0); up_to sets a window. We can
    // attach an authoritative snapshot and enable the hidden-index depth — all
    // builder operations that don't need a prover.
    let snap = StatusListSnapshot {
        status_list: "http://ex/list".into(),
        version: 7,
        bits: vec![0x00],
    };
    let pinned = RevocationPolicy::accept_version(7).with_snapshot(snap.clone());
    let windowed = RevocationPolicy::up_to(7, 3)
        .with_snapshots([snap.clone()])
        .with_hidden_index_depth(10);
    // Distinct policies (different freshness model + hidden depth) are not equal.
    assert_ne!(pinned, windowed);
    // Re-building the same windowed policy is deterministic / Eq.
    let windowed2 = RevocationPolicy::up_to(7, 3)
        .with_snapshots([snap])
        .with_hidden_index_depth(10);
    assert_eq!(windowed, windowed2);
}
