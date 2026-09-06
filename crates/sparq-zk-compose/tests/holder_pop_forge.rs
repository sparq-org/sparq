// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-ncz0 (HolderPoP T4): the comprehensive forge-and-verify REGRESSION SUITE
//! for the issuer-attested credential↔holder-bound HolderPoP closure (T1+T2+T3,
//! all merged on main).
//!
//! ## What this gates
//! T1 (`sparq_zk::sig::commitment_message_with_holder` / `holder_key_digest`),
//! T2 (`AttestedHolderBinding`), and T3 (`bind_holder_binding` + `bind_holder_pop`,
//! the fail-closed bearer policy, the issuer-signature digest anchor) together
//! close the TRUSTED-HOLDER GAP: a HolderPoP presentation must be by the holder
//! the ISSUER bound into THIS credential, not merely *some* trusted holder. This
//! file is the systematic adversarial map that proves the T3 verifier REJECTS
//! every HolderPoP forgery vector with the SPECIFIC `CheckError`, and ACCEPTS only
//! valid presentations, so a future regression in the binding logic fails loudly.
//!
//! `forge_gates.rs` organises the soundness gates by BINDING (commitment / issuer
//! / attribution / revocation / nonce / public-input). `audit_forge_map.rs` pins
//! the historical audit findings #1–#12. NEITHER covers the HolderPoP closure;
//! THIS file is its dedicated forge-and-verify map.
//!
//! ## The forge-vector → verdict map (HolderPoP closure)
//!
//! | Vector | Forge | Expected verdict |
//! |---|---|---|
//! | A-presents-B            | trusted holder A presents B's issuer-bound credential | REJECT `HolderKeyMismatch` |
//! | swapped/wrong key       | presented holder key digest ≠ attested digest          | REJECT `HolderKeyMismatch` |
//! | tampered attested digest| manifest's attested `holder_pk_digest` altered          | REJECT `InvalidIssuerSignature` |
//! | clear-key disagreement  | disclosed clear `holder_public_key` ≠ presented key     | REJECT `HolderKeyMismatch` / `InvalidIssuerSignature` |
//! | unrelated-attestation   | holder binding on an attestation NOT covering the scan  | does NOT satisfy (REJECT `HolderBindingMissing` under require_binding) |
//! | bearer-where-required   | no `AttestedHolderBinding` under `require_binding`      | REJECT `HolderBindingMissing` |
//! | identity holder key     | presented / attested holder key is the curve identity  | REJECT fail-closed (`HolderNotTrusted`/`HolderPopMalformed`/`HolderKeyMismatch`) |
//! | replay / wrong-nonce PoP| PoP minted over a DIFFERENT challenge than the nonce    | REJECT `HolderPopInvalid` |
//! | untrusted issuer anchor | holder binding signed by an issuer key NOT in external K| REJECT `IssuerKeyNotInKeySet` |
//! | positive: bound + req    | correctly issuer-bound presentation under require_binding | ACCEPT (reaches `MissingProof`) |
//! | positive: bearer + allow | bearer credential under allow_bearer                    | ACCEPT (reaches `MissingProof`) |
//!
//! ## No toolchain required
//! Every forge here is a HOLDER-binding violation, which the verifier rejects at
//! the `bind_holder_pop` gate — which runs BEFORE the per-sub-proof bb loop. So
//! the negative cases fire with empty `proof_hex` and NO `nargo`/`bb`. The two
//! POSITIVE controls pass the holder gate and then stop at `MissingProof` (the
//! absent bb proof): reaching `MissingProof` is downstream of and distinct from
//! ANY holder-binding rejection, so it proves the credential↔holder binding was
//! ACCEPTED. (The full bb-prove end-to-end positive is `holder_pop_valid_verifies_
//! end_to_end` in e2e.rs, toolchain-gated.) This suite tests the RUST VERIFIER
//! logic, not the circuits.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{
    holder_key_digest, public_key_from_hex, public_key_to_hex, PublicKey, SecretKey,
    SignatureScheme,
};
use sparq_zk_compose::build::{build_scan, Pattern, Slot};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{
    AttestedHolderBinding, AttestedStatusRef, BindingMode, CommitmentAttestation, EntailmentRegime,
    FieldHex, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::verifier::{
    verify_manifest, CheckError, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
    InMemorySeenNonces, KeySet, RevocationPolicy, VerifierNonce,
};

// --- fixtures (mirror the e2e / forge_gates plumbing; kept local so this suite
// is a single self-contained adversarial map for the HolderPoP closure) -------

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
/// The challenge the manifests declare AND the verifier issues as its nonce.
const CHALLENGE_HEX: &str = "0x2a";
/// The single salt the credential graph is committed + attested under.
const SALT_BYTE: u8 = 9;

/// The Baby-JubJub IDENTITY point in compressed hex. `public_key_from_hex` and
/// `holder_key_digest` both REJECT it fail-closed (the identity has no affine
/// coordinates), so a `HolderPop` presenting it can never be honoured. Hardcoded
/// (stable compressed constant) — the e2e suite uses the same value.
const IDENTITY_HOLDER_HEX: &str =
    "0100000000000000000000000000000000000000000000000000000000000000";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}
fn fixture_salt() -> Fr {
    salt_from_bytes(&[SALT_BYTE; 32])
}

/// A tiny named-graph credential: `alice` with an age (25) and a role.
fn credential_graph() -> Vec<Triple> {
    let alice = NamedOrBlankNode::NamedNode(iri("http://ex/alice"));
    vec![
        Triple::new(alice.clone(), iri("http://ex/age"), int_lit(25)),
        Triple::new(
            alice,
            iri("http://ex/role"),
            Term::NamedNode(iri("http://ex/admin")),
        ),
    ]
}

fn test_issuer_sk(seed: u64) -> SecretKey {
    SecretKey::from_seed(seed)
}
fn trusted_k(sk: &SecretKey) -> KeySet {
    KeySet::from_hex_keys([public_key_to_hex(&sk.public_key())])
}
fn nonce_for(hex: &str) -> VerifierNonce {
    VerifierNonce::from_hex(hex).expect("valid nonce hex")
}

fn fixture_snapshot(revoked: bool) -> StatusListSnapshot {
    let bits = if revoked {
        vec![1u8 << STATUS_INDEX]
    } else {
        vec![0u8]
    };
    StatusListSnapshot {
        status_list: STATUS_LIST.to_string(),
        version: STATUS_VERSION,
        bits,
    }
}
fn fixture_revocation() -> RevocationStatus {
    RevocationStatus {
        ref_commitment: None,
        status_list: Some(STATUS_LIST.to_string()),
        index: Some(STATUS_INDEX),
        version: Some(STATUS_VERSION),
        index_commitment: None,
    }
}
/// Policy: accept the fixture version with a fresh, NON-revoked authoritative snapshot.
fn fresh_policy() -> RevocationPolicy {
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot(false))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sparq_zk_compose_holderforge_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The scan `ProofInputs` over `{ ?s <ex/age> ?o }` on the credential graph
/// (committed under the fixture salt). The single scan-referenced commitment.
fn age_scan() -> ProofInputs {
    let commit = commit_triples(&credential_graph(), fixture_salt()).unwrap();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    build_scan(std::slice::from_ref(&commit), &pattern)
        .expect("scan builds")
        .inputs
}

/// The fixture credential's commitment (the scan-referenced commitment).
fn fixture_commitment() -> Fr {
    commit_triples(&credential_graph(), fixture_salt())
        .unwrap()
        .commitment
}

/// A SALT- + STATUS-bound BEARER attestation (`holder: None`) over `commitment` —
/// the scan-verify-path attestation for a credential with NO issuer-attested
/// holder binding.
fn attest_bearer(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature: sk.sign_commitment_with_status(&commitment, &salt, &status_ref),
        cryptosuite: SignatureScheme::Poseidon2SchnorrV1
            .cryptosuite_iri()
            .to_string(),
        salt: Some(FieldHex::from_field(&salt)),
        status: Some(AttestedStatusRef {
            ref_commitment: None,
            index: Some(STATUS_INDEX),
            version: Some(STATUS_VERSION),
            index_commitment: None,
        }),
        holder: None,
    }
}

/// A SALT-, STATUS- AND HOLDER-bound attestation (T3/sq-z8s7 B1): the issuer signs
/// `commitment_message_with_holder(C(G), salt, status_ref, holder_key_digest(hpk))`
/// (the ZKSIG_C4 variant), binding holder `hpk` into THIS credential. `disclose_key`
/// selects the clear-key tier (`Some(holder_public_key)`).
fn attest_holder(
    commitment: Fr,
    salt: Fr,
    hpk: &PublicKey,
    disclose_key: bool,
    sk: &SecretKey,
) -> CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    let holder_digest = holder_key_digest(hpk).expect("non-identity holder key digests");
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature: sk.sign_commitment_with_holder(&commitment, &salt, &status_ref, &holder_digest),
        cryptosuite: SignatureScheme::Poseidon2SchnorrV1
            .cryptosuite_iri()
            .to_string(),
        salt: Some(FieldHex::from_field(&salt)),
        status: Some(AttestedStatusRef {
            ref_commitment: None,
            index: Some(STATUS_INDEX),
            version: Some(STATUS_VERSION),
            index_commitment: None,
        }),
        holder: Some(
            AttestedHolderBinding::from_holder_key(hpk, disclose_key)
                .expect("non-identity holder key binds"),
        ),
    }
}

/// A holder's valid PoP over the issued challenge (`CHALLENGE_HEX`).
fn holder_pop(holder_sk: &SecretKey) -> String {
    let challenge_fr = FieldHex(CHALLENGE_HEX.into()).to_field().unwrap();
    holder_sk.sign_holder_pop(&challenge_fr)
}

/// Build a HolderPop-bound manifest over the age-scan credential. The PRESENTED
/// key + PoP are `presented_holder`'s; `attestations` carry whatever covering
/// attestation the caller wants (bearer / holder-bound / forged). `proof_hex` is
/// empty so the holder gate fires before any bb call. The issuer key is disclosed
/// in `key_set`.
fn holder_manifest(
    presented_holder: &SecretKey,
    pop_hex: String,
    cryptosuite: &str,
    attestations: Vec<CommitmentAttestation>,
    issuer_sk: &SecretKey,
) -> ProofManifest {
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&issuer_sk.public_key())],
        commitment_attestations: attestations,
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::HolderPop {
            challenge: FieldHex(CHALLENGE_HEX.into()),
            holder: public_key_to_hex(&presented_holder.public_key()),
            pop: pop_hex,
            cryptosuite: cryptosuite.to_string(),
        },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof {
            inputs: age_scan(),
            proof_hex: String::new(),
        }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// Run a manifest through the sound entry point with the honest fixtures
/// (trusted K = issuer 1, fresh non-revoked policy, nonce = the issued challenge,
/// a fresh single-use store) under the supplied `registry` + `holder_policy`.
fn verify_under(
    m: &ProofManifest,
    name: &str,
    registry: &HolderRegistry,
    holder_policy: &HolderBindingPolicy,
) -> Result<(), CheckError> {
    let prover = CircuitProver::from_crate_root();
    verify_manifest(
        m,
        &prover,
        &scratch(name),
        &trusted_k(&test_issuer_sk(1)),
        &fresh_policy(),
        registry,
        holder_policy,
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
}

// =========================================================================
// POSITIVE CONTROLS — only valid presentations are accepted past the gate
// =========================================================================

/// POSITIVE: a correctly issuer-bound presentation under `require_binding` — the
/// PRESENTED holder key digest == the issuer-attested digest (and the disclosed
/// clear key matches). It PASSES the holder-binding gate and stops at
/// `MissingProof` (the absent bb proof), which is downstream of and distinct from
/// ANY holder-binding rejection: the credential↔holder binding was ACCEPTED.
#[test]
fn positive_correctly_bound_under_require_binding_accepts() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    // Issuer bound THIS holder; the presenter IS that holder (clear-key tier).
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder.public_key(),
        true,
        &issuer,
    )];
    let m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "pos_bound_require",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        Ok(()) => panic!("unexpected Ok without a bb proof"),
        other => panic!(
            "a correctly issuer-bound presentation under require_binding must PASS \
             the holder gate (reaching MissingProof), got {other:?}"
        ),
    }
}

/// POSITIVE: a correctly issuer-bound presentation in the HIDDEN tier (the
/// attestation carries ONLY the digest, no clear `holder_public_key`) still PASSES
/// the gate — the digest cross-check alone binds the presenter (clear-key
/// disclosure is optional; the digest is mandatory).
#[test]
fn positive_correctly_bound_hidden_tier_accepts() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    // disclose_key = false => no clear `holder_public_key`, only the digest.
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder.public_key(),
        false,
        &issuer,
    )];
    let m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "pos_bound_hidden",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        Ok(()) => panic!("unexpected Ok without a bb proof"),
        other => panic!(
            "a correctly-bound HIDDEN-tier presentation must PASS the holder gate \
             (digest cross-check alone), got {other:?}"
        ),
    }
}

/// POSITIVE: a BEARER credential (no `AttestedHolderBinding`) under the
/// back-compatible `allow_bearer` policy PASSES the holder gate (the sq-cwq
/// registry + nonce-PoP guarantee stands) and stops at `MissingProof`.
#[test]
fn positive_bearer_under_allow_bearer_accepts() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let attestations = vec![attest_bearer(fixture_commitment(), fixture_salt(), &issuer)];
    let m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "pos_bearer_allow",
        &registry,
        &HolderBindingPolicy::allow_bearer(),
    ) {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        Ok(()) => panic!("unexpected Ok without a bb proof"),
        other => panic!(
            "a bearer credential under allow_bearer must PASS the holder gate \
             (reaching MissingProof), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 1: A-presents-B (the core trusted-holder-gap closure)
// =========================================================================

/// FORGE (THE LOAD-BEARING VECTOR): trusted holder A presents trusted holder B's
/// issuer-bound credential. The credential is issuer-attested-holder-bound to B,
/// but A presents (A's key + A's VALID PoP over the nonce). A is a registry member
/// and A's PoP verifies — exactly the gap the nonce-only PoP left open — yet the
/// presentation is REJECTED because `holder_key_digest(A)` != B's issuer-attested
/// digest. Even the back-compatible `allow_bearer` policy MUST reject (when a
/// binding is PRESENT the cross-check always runs; the policy only governs the
/// bearer-absent case).
#[test]
fn forge_a_presents_b_rejected_holder_key_mismatch() {
    let issuer = test_issuer_sk(1);
    let holder_a = SecretKey::from_seed(777); // the presenter (trusted)
    let holder_b = SecretKey::from_seed(888); // the credential's true subject
                                              // BOTH A and B are authorised holders — registry membership is NOT the gap.
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&holder_a.public_key()),
        public_key_to_hex(&holder_b.public_key()),
    ]);

    // Issuer bound B; A presents (A's key + A's valid PoP over the nonce).
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder_b.public_key(),
        true,
        &issuer,
    )];
    let m = holder_manifest(
        &holder_a,
        holder_pop(&holder_a),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    for policy in [
        HolderBindingPolicy::allow_bearer(),
        HolderBindingPolicy::require_binding(),
    ] {
        match verify_under(&m, "forge_a_presents_b", &registry, &policy) {
            Err(CheckError::HolderKeyMismatch) => {}
            other => panic!(
                "trusted holder A presenting trusted holder B's credential MUST be \
                 rejected HolderKeyMismatch (the trusted-holder gap, policy={policy:?}), got {other:?}"
            ),
        }
    }
}

// =========================================================================
// FORGE 2: swapped / wrong presented key (digest ≠ attested)
// =========================================================================

/// FORGE: the presented holder key (and its valid PoP) belong to a holder whose
/// digest is NOT the attested one — a generalisation of A-presents-B where the
/// presenter is a DIFFERENT trusted holder than the issuer bound, with no clear
/// `holder_public_key` disclosed (hidden tier). The digest cross-check alone must
/// reject (`HolderKeyMismatch`).
#[test]
fn forge_swapped_presented_key_rejected_holder_key_mismatch() {
    let issuer = test_issuer_sk(1);
    let bound_holder = SecretKey::from_seed(1234); // issuer bound this one
    let presenter = SecretKey::from_seed(5678); // a different trusted holder presents
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&bound_holder.public_key()),
        public_key_to_hex(&presenter.public_key()),
    ]);
    // HIDDEN tier (disclose_key = false): only the attested digest is public, so
    // the cross-check is purely digest equality — and the presenter's digest differs.
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &bound_holder.public_key(),
        false,
        &issuer,
    )];
    let m = holder_manifest(
        &presenter,
        holder_pop(&presenter),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "forge_swapped_key",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::HolderKeyMismatch) => {}
        other => panic!(
            "a presented key whose digest != the attested digest must be \
             HolderKeyMismatch, got {other:?}"
        ),
    }
}

/// FORGE (clear-key-disagreement variant): the attestation discloses a clear
/// `holder_public_key` that disagrees with the digest the issuer signed (the
/// attestation is internally malformed — clear key swapped post-sign). The
/// presenter matches the swapped clear key. The verifier rejects: either the
/// recomputed `holder_key_digest(clear) != attested digest` fails the (5)
/// belt-and-braces clear-key check (`HolderKeyMismatch`), or — since the swapped
/// clear key changes neither the issuer-signed digest — the digest cross-check
/// against the presenter fails first. Either way: rejected, never accepted.
#[test]
fn forge_clear_key_disagrees_with_digest_rejected() {
    let issuer = test_issuer_sk(1);
    let bound_holder = SecretKey::from_seed(1234); // issuer signed THIS digest
    let other = SecretKey::from_seed(4321); // the swapped clear key + presenter
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&bound_holder.public_key()),
        public_key_to_hex(&other.public_key()),
    ]);
    // Issuer signs the digest of `bound_holder` (clear-key tier), then we FORGE the
    // disclosed clear `holder_public_key` to `other` (≠ the signed digest's key).
    let mut att = attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &bound_holder.public_key(),
        true,
        &issuer,
    );
    if let Some(binding) = att.holder.as_mut() {
        binding.holder_public_key = Some(public_key_to_hex(&other.public_key()));
    }
    let m = holder_manifest(
        &other,
        holder_pop(&other),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        vec![att],
        &issuer,
    );
    match verify_under(
        &m,
        "forge_clear_disagree",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        // The presenter (`other`) digest != the issuer-signed digest (bound_holder),
        // so the (4) digest cross-check rejects. (The disclosed clear key, swapped to
        // `other`, would also fail the (5) clear-key check.)
        Err(CheckError::HolderKeyMismatch) => {}
        // If the implementation reaches the issuer-signature anchor first, a swapped
        // clear key that breaks signature consistency is also a correct rejection.
        Err(CheckError::InvalidIssuerSignature { .. }) => {}
        other => panic!(
            "a disclosed clear key disagreeing with the issuer-signed digest must be \
             rejected (HolderKeyMismatch / InvalidIssuerSignature), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 3: tampered attested digest (manifest digest altered)
// =========================================================================

/// FORGE: the manifest's attested `holder_pk_digest` is ALTERED after the issuer
/// signed (a prover swaps in its OWN holder digest hoping to bind its own key).
/// The issuer signed `commitment_message_with_holder(..., ORIGINAL_digest)`, so the
/// recomputed message over the TAMPERED digest yields a different message and the
/// issuer signature no longer verifies (EUF-CMA) — REJECT `InvalidIssuerSignature`.
/// This is the issuer-signature ANCHOR: the digest the verifier cross-checks is the
/// one the ISSUER bound, never a free prover JSON field.
#[test]
fn forge_tampered_attested_digest_rejected_invalid_signature() {
    let issuer = test_issuer_sk(1);
    let bound_holder = SecretKey::from_seed(777); // issuer bound this holder
    let attacker = SecretKey::from_seed(999); // the prover's own holder key
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&bound_holder.public_key()),
        public_key_to_hex(&attacker.public_key()),
    ]);
    // Issuer-bind `bound_holder` (clear-key tier), then TAMPER the attested digest
    // to the attacker's own holder digest (the manifest digest altered post-sign).
    let mut att = attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &bound_holder.public_key(),
        true,
        &issuer,
    );
    let attacker_digest = holder_key_digest(&attacker.public_key()).unwrap();
    if let Some(binding) = att.holder.as_mut() {
        binding.holder_pk_digest = FieldHex::from_field(&attacker_digest);
        // Also disclose the attacker's clear key, so the digest is internally
        // "consistent" — yet the ISSUER signature (over the original digest) breaks.
        binding.holder_public_key = Some(public_key_to_hex(&attacker.public_key()));
    }
    // The attacker presents its OWN key + a valid PoP (so the only thing wrong is
    // the tampered, no-longer-issuer-signed digest).
    let m = holder_manifest(
        &attacker,
        holder_pop(&attacker),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        vec![att],
        &issuer,
    );
    match verify_under(
        &m,
        "forge_tampered_digest",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::InvalidIssuerSignature { .. }) => {}
        other => panic!(
            "a tampered attested holder digest (no longer issuer-signed) must be \
             InvalidIssuerSignature (the issuer-signature anchor), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 4: unrelated-attestation bypass (the T3 scoping fix)
// =========================================================================

/// FORGE (the scoping bug, Copilot review on #142): a holder binding on an
/// UNRELATED attestation (one covering NO scan-referenced commitment) must NOT
/// satisfy the holder-binding check for the credential actually presented. The
/// scan's covering attestation is BEARER; an UNRELATED holder-bound attestation
/// over a DIFFERENT commitment value (bound to the presenter's own key) is added.
/// Under the buggy "ANY attestation has a holder" path the unrelated binding would
/// PASS; under the T3 scoped fix the scan's covering attestation is bearer, so under
/// `require_binding` the presentation is REJECTED `HolderBindingMissing`.
#[test]
fn forge_unrelated_attestation_does_not_satisfy_scoped_check() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777); // the presenter (trusted)
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);

    let scan_commitment = fixture_commitment();
    // The scan's COVERING attestation is BEARER (status-only, holder: None).
    let bearer = attest_bearer(scan_commitment, fixture_salt(), &issuer);
    // An UNRELATED holder-bound attestation over a DIFFERENT commitment value
    // (scan + 1), bound to the PRESENTER'S OWN key — the exact shape that would
    // falsely satisfy the buggy "any attestation" shortcut.
    let unrelated_commitment = scan_commitment + Fr::from(1u64);
    let unrelated = attest_holder(
        unrelated_commitment,
        fixture_salt(),
        &holder.public_key(),
        true,
        &issuer,
    );
    let m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        vec![bearer, unrelated],
        &issuer,
    );
    // Under require_binding: the SCAN-REFERENCED commitment's covering attestation is
    // bearer, so the presentation is bearer-where-binding-required and REJECTED — the
    // unrelated holder binding does NOT rescue it (the scoping bug is closed).
    match verify_under(
        &m,
        "forge_unrelated_scope",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::HolderBindingMissing) => {}
        other => panic!(
            "a holder binding on an UNRELATED attestation must NOT satisfy the scoped \
             check for a bearer scan-referenced credential under require_binding \
             (expected HolderBindingMissing — the scoping bug is closed), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 5: bearer-where-binding-required
// =========================================================================

/// FORGE: a BEARER credential (no `AttestedHolderBinding` on any covering
/// attestation) presented under `require_binding`. The relying party mandates an
/// issuer-attested holder binding, so the bearer credential is REJECTED fail-closed
/// (`HolderBindingMissing`) — the design's "bearer must be rejectable" posture.
/// The presenter is a trusted holder with a valid PoP (so only the missing binding
/// is at issue).
#[test]
fn forge_bearer_where_binding_required_rejected() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let attestations = vec![attest_bearer(fixture_commitment(), fixture_salt(), &issuer)];
    let m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "forge_bearer_required",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::HolderBindingMissing) => {}
        other => panic!(
            "a bearer credential under require_binding must be HolderBindingMissing, got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 6: identity holder key
// =========================================================================

/// FORGE: the PRESENTED holder key is the curve IDENTITY. The identity has no
/// affine coordinates, so it can never be a trusted holder (a `HolderRegistry`
/// drops it at construction), never parses to a usable key for the PoP, and could
/// never match an issuer-attested digest. ANY of these is a correct fail-closed
/// rejection; there is NO path on which the identity is honoured.
#[test]
fn forge_identity_presented_key_rejected_fail_closed() {
    // Sanity: the identity hex is the rejected identity key.
    assert!(
        public_key_from_hex(IDENTITY_HOLDER_HEX).is_none(),
        "the identity key must NOT parse to a usable holder key (fail-closed)"
    );
    assert!(
        HolderRegistry::from_hex_keys([IDENTITY_HOLDER_HEX.to_string()]).is_empty(),
        "a HolderRegistry must drop the identity key (it can never be a real holder)"
    );

    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    // The registry trusts the real holder; the identity is dropped (not storable).
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&holder.public_key()),
        IDENTITY_HOLDER_HEX.to_string(),
    ]);
    // A holder-bound credential (to the real holder), then SWAP the PRESENTED key to
    // the identity.
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder.public_key(),
        true,
        &issuer,
    )];
    let mut m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    if let BindingMode::HolderPop { holder, .. } = &mut m.binding {
        *holder = IDENTITY_HOLDER_HEX.to_string();
    }
    match verify_under(
        &m,
        "forge_identity_key",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::HolderNotTrusted { .. })
        | Err(CheckError::HolderPopMalformed)
        | Err(CheckError::HolderKeyMismatch) => {}
        other => panic!(
            "an identity holder key must be rejected fail-closed \
             (HolderNotTrusted / HolderPopMalformed / HolderKeyMismatch), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 7: replay / wrong-nonce PoP (freshness)
// =========================================================================

/// FORGE: a PoP minted over a DIFFERENT challenge than the verifier's issued nonce
/// (a captured/replayed PoP). The PoP is bound to the verifier's fresh nonce, so a
/// PoP over another challenge cannot pass under the issued nonce — REJECT
/// `HolderPopInvalid` even though the holder IS trusted and the credential IS
/// correctly bound to it.
#[test]
fn forge_replay_wrong_nonce_pop_rejected() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    // A correctly-bound credential — so the ONLY thing wrong is the stale PoP.
    let attestations = vec![attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder.public_key(),
        true,
        &issuer,
    )];
    // A PoP over a DIFFERENT challenge (0xdead), NOT the issued nonce 0x2a.
    let stale_challenge = FieldHex("0xdead".into()).to_field().unwrap();
    let stale_pop = holder.sign_holder_pop(&stale_challenge);
    let m = holder_manifest(
        &holder,
        stale_pop,
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        attestations,
        &issuer,
    );
    match verify_under(
        &m,
        "forge_replay_nonce",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        Err(CheckError::HolderPopInvalid { .. }) => {}
        other => panic!(
            "a PoP over a different challenge (replay / wrong nonce) must be \
             HolderPopInvalid (freshness), got {other:?}"
        ),
    }
}

// =========================================================================
// FORGE 8: untrusted issuer anchor for the holder binding
// =========================================================================

/// FORGE: the holder binding's covering attestation is signed by an issuer key
/// that is NOT in the relying party's EXTERNAL trusted `K`. Even a cryptographically
/// VALID holder-bound signature by an untrusted issuer is no attestation: the digest
/// the verifier would cross-check is not anchored in a trusted issuer's signature.
/// REJECT (`IssuerKeyNotInKeySet`) — the audit-#3 codex-#1 external-anchor precedent
/// applied to the holder-binding signature.
#[test]
fn forge_holder_binding_untrusted_issuer_rejected() {
    let trusted_issuer = test_issuer_sk(1); // the relying party's external K trusts THIS
    let rogue_issuer = test_issuer_sk(2); // ...but the holder binding is signed by THIS
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    // The holder-bound attestation is signed by the rogue issuer (key 2), and the
    // manifest key_set advertises key 2 — but verify_under's external K trusts ONLY
    // key 1.
    let mut att = attest_holder(
        fixture_commitment(),
        fixture_salt(),
        &holder.public_key(),
        true,
        &rogue_issuer,
    );
    // Keep the (valid, rogue-signed) signature; the attestation is well-formed but
    // its issuer is untrusted.
    att.issuer_public_key = public_key_to_hex(&rogue_issuer.public_key());
    let mut m = holder_manifest(
        &holder,
        holder_pop(&holder),
        SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        vec![att],
        &rogue_issuer,
    );
    // The manifest declares the rogue key in key_set (a prover trying to self-anchor).
    m.key_set = vec![public_key_to_hex(&rogue_issuer.public_key())];
    let _ = &trusted_issuer; // verify_under's external K is issuer 1 (the trusted one).
    match verify_under(
        &m,
        "forge_untrusted_issuer",
        &registry,
        &HolderBindingPolicy::require_binding(),
    ) {
        // The covering attestation's issuer (key 2) is not in the external K (key 1).
        // It is rejected as untrusted — either at the issuer-attestation gate
        // (IssuerKeyNotInKeySet) or at the prover-declared-key-set subset check
        // (UntrustedDeclaredKey). Both are correct trust rejections, never an accept.
        Err(CheckError::IssuerKeyNotInKeySet { .. })
        | Err(CheckError::UntrustedDeclaredKey { .. }) => {}
        other => panic!(
            "a holder binding signed by an issuer key NOT in the external K must be \
             rejected as untrusted (IssuerKeyNotInKeySet / UntrustedDeclaredKey), got {other:?}"
        ),
    }
}
