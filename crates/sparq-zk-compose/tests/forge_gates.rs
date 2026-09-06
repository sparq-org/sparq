// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-ajl: forge-and-verify NEGATIVE suite for the soundness binding gates.
//!
//! This is the adversarial complement to the happy-path e2e tests: it is a
//! single gate-indexed map of "construct a FORGED proof/manifest that violates
//! EXACTLY one binding, then assert the verifier REJECTS it with the right
//! `CheckError`". It proves each soundness gate actually *gates*.
//!
//! Bindings covered (one canonical forge each), with the `CheckError` each must
//! return:
//! - commitment binding: `UnattestedCommitment` (unsigned C(G)).
//! - issuer attestation: `InvalidIssuerSignature` (forged sig); `IssuerKeyNotInKeySet` (key not in external K).
//! - attribution: `AttributionMalformed` (omitted attr); `AttributionUnderDeclared` (collapse forge).
//! - revocation snapshot: `CredentialRevoked` (authoritative bit set); `StatusListStale` (version out of window).
//! - nonce: `NonceBindingMismatch` (binding != nonce); `NonceReplay` (nonce reused / single-use).
//! - public-input byte-binding: `PublicInputMismatch` (genuine proof of A presented as statement B — FULL bb path).
//!
//! Structural gates (commitment / attestation / attribution / revocation / the
//! nonce-binding-consistency check) are reached through the SOUND public entry
//! point — they run as the `prefilter_manifest_structure` stage inside
//! `verify_manifest`, but several do not need bb, so those forges run in default
//! CI without the toolchain (they reject before any bb subprocess). The
//! crypto-dependent forges (public-input byte-binding, single-use replay after a
//! genuine accept) require a real `bb prove` and are gated behind the toolchain
//! and `#[ignore]` (run with `-- --ignored`).
//!
//! Where this file overlaps the original e2e suite it deliberately re-states the
//! forge through `verify_manifest` (the full sound path) rather than the
//! structural prefilter alone, so the gate is proven on the real entry point.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{public_key_to_hex, SecretKey, SignatureScheme};
use sparq_zk_compose::build::{build_filter_int, build_scan, encode_int_literal, Pattern, Slot};
use sparq_zk_compose::driver::{CircuitProver, ProofArtifacts};
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingMode, CommitmentAttestation, EntailmentRegime, FieldHex,
    FilterOp, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    encode_artifacts, verify_manifest, CheckError, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
    InMemorySeenNonces, KeySet, RevocationPolicy, VerifierNonce,
};

// --- fixtures (mirrors the e2e plumbing; kept local so this suite is a single
// self-contained adversarial map) ------------------------------------------

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const CHALLENGE_HEX: &str = "0x2a";
/// The salt the single-graph fixtures commit + attest under.
const FIXTURE_SALT_BYTE: u8 = 7;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}
fn fixture_salt() -> Fr {
    salt_from_bytes(&[FIXTURE_SALT_BYTE; 32])
}

/// A tiny named-graph credential: `alice` with an age (25) and a role.
fn credential_graph() -> Vec<Triple> {
    let alice = NamedOrBlankNode::NamedNode(iri("http://ex/alice"));
    vec![
        Triple::new(alice.clone(), iri("http://ex/age"), int_lit(25)),
        Triple::new(alice, iri("http://ex/role"), Term::NamedNode(iri("http://ex/admin"))),
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
    let bits = if revoked { vec![1u8 << STATUS_INDEX] } else { vec![0u8] };
    StatusListSnapshot { status_list: STATUS_LIST.to_string(), version: STATUS_VERSION, bits }
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
/// Policy whose AUTHORITATIVE snapshot has the credential's bit SET (revoked).
fn revoked_policy() -> RevocationPolicy {
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot(true))
}

/// A salt- AND status-bound attestation over `commitment` (the scan-verify-path
/// attestation: a scan-covering commitment MUST be salt+status bound).
fn attest_full(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature: sk.sign_commitment_with_status(&commitment, &salt, &status_ref),
        cryptosuite: SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri().to_string(),
        salt: Some(FieldHex::from_field(&salt)),
        status: Some(AttestedStatusRef {
            ref_commitment: None,
            index: Some(STATUS_INDEX),
            version: Some(STATUS_VERSION),
            index_commitment: None,
        }),
        holder: None, // [OPUS-4.8] sq-h8rg (HolderPoP T2): non-holder-bound (bearer)
    }
}

fn toolchain_available() -> bool {
    fn on_path(tool: &str) -> bool {
        std::process::Command::new(tool)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    on_path("nargo") && on_path("bb")
}
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_zk_compose_forge_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a witness-only (empty proof_hex) scan-only manifest over the credential
/// graph's `<ex/age>` pattern, with a valid salt+status-bound attestation so it
/// passes every structural gate by default. Returns the manifest + the
/// commitment + salt so a forge can selectively break ONE gate. The verifier
/// reaches whichever structural gate the forge targets WITHOUT bb (these reject
/// before the per-sub-proof crypto loop). The honest binding challenge is
/// `CHALLENGE_HEX`.
fn honest_scan_only_manifest() -> (ProofManifest, Fr, Fr) {
    let salt = fixture_salt();
    let sk = test_issuer_sk(1);
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let commitment = commit.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern).expect("scan builds");
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![attest_full(commitment, salt, &sk)],
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof { inputs: scan.inputs, proof_hex: String::new() }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    };
    (m, commitment, salt)
}

/// Run a manifest through the FULL sound entry point with the honest fixtures
/// (trusted K = issuer 1, fresh policy, nonce = the honest challenge, a fresh
/// single-use store). Returns the verdict for the caller to match on.
fn verify_full(m: &ProofManifest, name: &str) -> Result<(), CheckError> {
    let prover = CircuitProver::from_crate_root();
    verify_manifest(
        m,
        &prover,
        &scratch(name),
        &trusted_k(&test_issuer_sk(1)),
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
}

/// Sanity: the honest scan-only manifest passes the STRUCTURAL stage of the
/// sound entry point. (It carries no bb proof, so `verify_manifest` rejects with
/// `MissingProof` *after* the structural gate — proving the forges below fail
/// EARLIER, at their targeted gate, not on the missing proof.)
#[test]
fn honest_scan_only_reaches_missing_proof() {
    let (m, _c, _s) = honest_scan_only_manifest();
    match verify_full(&m, "honest_reaches_missing") {
        // Structural gate passed; the only thing wrong is the absent bb proof.
        Err(CheckError::MissingProof { proof: 0 }) => {}
        other => panic!("honest manifest should pass structural gates then hit MissingProof, got {other:?}"),
    }
}

// === BINDING 1: commitment binding =========================================

/// Forge: drop the issuer attestation so the scan's commitment is unsigned /
/// prover-invented. The commitment-binding gate must REJECT (`UnattestedCommitment`).
#[test]
fn forge_commitment_unsigned_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    m.commitment_attestations.clear();
    match verify_full(&m, "forge_unsigned") {
        Err(CheckError::UnattestedCommitment { proof: 0, .. }) => {}
        other => panic!("unsigned commitment must be UnattestedCommitment, got {other:?}"),
    }
}

// === BINDING 2: issuer attestation =========================================

/// Forge: keep an attestation but corrupt its signature (sign a DIFFERENT
/// commitment, then relabel). The issuer-signature gate must REJECT
/// (`InvalidIssuerSignature`).
#[test]
fn forge_issuer_invalid_signature_rejected() {
    let (mut m, c, salt) = honest_scan_only_manifest();
    let sk = test_issuer_sk(1);
    // Sign c+1 (salt+status bound so it reaches the signature check), relabel to c.
    let wrong = attest_full(c + Fr::from(1u64), salt, &sk);
    m.commitment_attestations = vec![CommitmentAttestation {
        commitment: FieldHex::from_field(&c),
        ..wrong
    }];
    match verify_full(&m, "forge_badsig") {
        Err(CheckError::InvalidIssuerSignature { .. }) => {}
        other => panic!("forged signature must be InvalidIssuerSignature, got {other:?}"),
    }
}

/// Forge: a cryptographically VALID signature, but by an issuer key the relying
/// party's EXTERNAL trust anchor `K` does not contain. The issuer-trust gate must
/// REJECT (`IssuerKeyNotInKeySet`) — even a real signature by an untrusted key is
/// no attestation.
#[test]
fn forge_issuer_key_not_in_external_k_rejected() {
    let (mut m, c, salt) = honest_scan_only_manifest();
    // Issuer 2 signs validly; we list issuer 2 in the manifest key_set...
    let signer = test_issuer_sk(2);
    m.commitment_attestations = vec![attest_full(c, salt, &signer)];
    m.key_set = vec![public_key_to_hex(&signer.public_key())];
    // ...but verify_full's external K trusts ONLY issuer 1. So the attestation's
    // key (issuer 2) is not in K. (The declared key_set is also a superset of K,
    // which the verifier separately rejects; whichever fires, it must be a trust
    // rejection, not an accept.)
    match verify_full(&m, "forge_key_not_in_k") {
        Err(CheckError::IssuerKeyNotInKeySet { .. })
        | Err(CheckError::UntrustedDeclaredKey { .. }) => {}
        other => panic!("untrusted issuer key must be rejected (IssuerKeyNotInKeySet/UntrustedDeclaredKey), got {other:?}"),
    }
}

// === BINDING 3: attribution =================================================

/// Forge: OMIT the proof-bound per-graph attribution vector (empty) so the
/// cross-graph obligation cross-check would be vacuous. The attribution gate must
/// REJECT (`AttributionMalformed`).
#[test]
fn forge_attribution_omitted_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    if let ProofInputs::Scan { attribution, .. } = &mut m.sub_proofs[0].inputs {
        attribution.clear();
    } else {
        unreachable!("sub-proof 0 is a scan");
    }
    match verify_full(&m, "forge_attr_omit") {
        Err(CheckError::AttributionMalformed { proof: 0, expected: 1, got: 0 }) => {}
        other => panic!("omitted attribution must be AttributionMalformed, got {other:?}"),
    }
}

/// Forge (the headline `[[0],[0]]` collapse): a two-graph cross-graph join whose
/// `manifest.attributions` under-declares a graph the scan PROVED a contribution
/// from, to drop a non-bnode obligation. The attribution gate must REJECT
/// (`AttributionUnderDeclared`). This is structural (no bb): the proof-bound
/// `attribution` bits disagree with the under-declared `manifest.attributions`.
#[test]
fn forge_attribution_under_declared_rejected() {
    let sk = test_issuer_sk(1);
    // Two distinct single-triple graphs, distinct salts, both contributing to one
    // cross-graph pattern over a shared predicate.
    let g0 = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/p"),
        int_lit(1),
    )];
    let g1 = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/b")),
        iri("http://ex/p"),
        int_lit(2),
    )];
    let salt0 = salt_from_bytes(&[11u8; 32]);
    let salt1 = salt_from_bytes(&[12u8; 32]);
    let c0 = commit_triples(&g0, salt0).unwrap();
    let c1 = commit_triples(&g1, salt1).unwrap();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 scan builds");
    // The scan proves a contribution from BOTH graphs; the forge declares only [0].
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(c0.commitment, salt0, &sk),
            attest_full(c1.commitment, salt1, &sk),
        ],
        // FORGE: under-declare — claim the pattern only draws from graph 0.
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof { inputs: scan.inputs, proof_hex: String::new() }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    };
    match verify_full(&m, "forge_attr_collapse") {
        Err(CheckError::AttributionUnderDeclared { pattern: 0, .. }) => {}
        other => panic!("collapse forge must be AttributionUnderDeclared, got {other:?}"),
    }
}

// === BINDING 4: revocation snapshot ========================================

/// Forge: present a genuine, fresh, issuer-bound credential but whose
/// AUTHORITATIVE status bit (the relying party's own snapshot) is SET. The
/// revocation gate must REJECT (`CredentialRevoked`). Here the relying party
/// supplies a `revoked_policy`; the prover cannot un-revoke it (the bit decision
/// reads the authoritative snapshot, never the prover's).
#[test]
fn forge_revocation_revoked_bit_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest();
    let prover = CircuitProver::from_crate_root();
    match verify_manifest(
        &m,
        &prover,
        &scratch("forge_revoked"),
        &trusted_k(&test_issuer_sk(1)),
        &revoked_policy(), // authoritative bit SET
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::CredentialRevoked { index: STATUS_INDEX, .. }) => {}
        other => panic!("revoked credential must be CredentialRevoked, got {other:?}"),
    }
}

/// Forge: the credential's issuer-bound reference is for version 1, but the
/// relying party's freshness window only accepts a LATER version (no snapshot for
/// v1 in window). The freshness gate must REJECT (`StatusListStale`).
#[test]
fn forge_revocation_stale_version_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest();
    let prover = CircuitProver::from_crate_root();
    // Accept only version 5 (window 0) — version 1 is stale / out of window.
    let policy = RevocationPolicy::accept_version(5).with_snapshot(StatusListSnapshot {
        status_list: STATUS_LIST.to_string(),
        version: 5,
        bits: vec![0u8],
    });
    match verify_manifest(
        &m,
        &prover,
        &scratch("forge_stale"),
        &trusted_k(&test_issuer_sk(1)),
        &policy,
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::StatusListStale { version: STATUS_VERSION, .. }) => {}
        other => panic!("stale version must be StatusListStale, got {other:?}"),
    }
}

// === BINDING 5: nonce =======================================================

/// Forge: the manifest's declared binding challenge does NOT equal the
/// verifier-issued nonce. The nonce-consistency gate must REJECT
/// (`NonceBindingMismatch`). No bb needed (checked before the crypto loop).
#[test]
fn forge_nonce_binding_mismatch_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest(); // binding declares 0x2a
    let prover = CircuitProver::from_crate_root();
    // ...but the verifier issues a DIFFERENT nonce.
    match verify_manifest(
        &m,
        &prover,
        &scratch("forge_nonce_mismatch"),
        &trusted_k(&test_issuer_sk(1)),
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for("0x99"),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::NonceBindingMismatch) => {}
        other => panic!("binding≠nonce must be NonceBindingMismatch, got {other:?}"),
    }
}

/// Forge: re-present the SAME (nonce, manifest) to the SAME single-use store. The
/// nonce is recorded on the FIRST presentation (burn-on-present policy), so the
/// SECOND is REJECTED (`NonceReplay`). The first presentation here reaches
/// MissingProof (witness-only manifest) — the nonce is still consumed — so the
/// replay defence is proven without bb.
#[test]
fn forge_nonce_replay_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest();
    let prover = CircuitProver::from_crate_root();
    let seen = InMemorySeenNonces::new();
    let nonce = nonce_for(CHALLENGE_HEX);
    let run = |tag: &str| {
        verify_manifest(
            &m,
            &prover,
            &scratch(tag),
            &trusted_k(&test_issuer_sk(1)),
            &fresh_policy(),
            &HolderRegistry::empty(),
            &HolderBindingPolicy::allow_bearer(),
            &EntailmentPolicy::simple_only(),
            &nonce,
            &seen,
        )
    };
    // First presentation: nonce fresh, structural gates pass, then MissingProof
    // (witness-only) — but the nonce is BURNED.
    match run("forge_replay_1") {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        other => panic!("first presentation should reach MissingProof, got {other:?}"),
    }
    // Second presentation under the burnt nonce: flat NonceReplay (recorded first).
    match run("forge_replay_2") {
        Err(CheckError::NonceReplay) => {}
        other => panic!("re-presentation under a burnt nonce must be NonceReplay, got {other:?}"),
    }
}

// === BINDING 6: public-input byte-binding (FULL bb prove) ===================
//
// The load-bearing soundness binding: a GENUINE bb proof of statement A,
// presented under a manifest DECLARING statement B, must be rejected because the
// verifier reconstructs the public-input vector from the DECLARED statement and
// byte-compares it to the proof's committed public inputs. This needs a real
// proof, so it is toolchain-gated + `#[ignore]` (run with `-- --ignored`).

/// Prove a real filter_int_d1 of `value <op> bound = expected`, returning the
/// honest ProofInputs + the bb artifacts.
fn honest_filter_d1(
    value: u64,
    op: FilterOp,
    bound: u64,
    expected: bool,
    challenge: &FieldHex,
    prover: &CircuitProver,
    tag: &str,
) -> (ProofInputs, ProofArtifacts) {
    let operand_enc = encode_int_literal(value);
    let (filter, digits) = build_filter_int(operand_enc, value, op, bound, expected).unwrap();
    let (id, toml) = prover_toml_for(&filter, challenge, &[], &[], &digits, None, None).unwrap();
    let out = scratch(tag);
    let art = prover.prove_in(&id, &toml, &out, tag).expect("filter proves");
    (filter, art)
}

/// Prove a real scan over `{ ?s <ex/age> ?o }` on the credential graph; returns
/// the honest scan ProofInputs + its encoded artifact hex (the manifest's query
/// is a 1-pattern scan, so this honest scan answers it).
fn honest_age_scan(challenge: &FieldHex, prover: &CircuitProver, tag: &str) -> (ProofInputs, String) {
    let salt = fixture_salt();
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern).expect("scan builds");
    let (id, toml) = prover_toml_for(&scan.inputs, challenge, &scan.witness.counts, &scan.witness.enc, &[], None, None).unwrap();
    let out = scratch(tag);
    let art = prover.prove_in(&id, &toml, &out, tag).expect("scan proves");
    (scan.inputs, encode_artifacts(&art))
}

/// Build a composed manifest: an honest age scan (index 0, answers the query) +
/// a possibly-forged FILTER sub-proof (index 1, unreferenced by the query). The
/// scan carries a valid salt+status-bound attestation so the #6 test reaches the
/// crypto byte-compare (not an earlier structural gate).
fn composed_manifest(
    scan_inputs: ProofInputs,
    scan_hex: String,
    filter_inputs: ProofInputs,
    filter_hex: String,
) -> ProofManifest {
    let sk = test_issuer_sk(1);
    let salt = fixture_salt();
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![attest_full(commit.commitment, salt, &sk)],
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![
            SubProof { inputs: scan_inputs, proof_hex: scan_hex },
            SubProof { inputs: filter_inputs, proof_hex: filter_hex },
        ],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// Positive control: an honest composed manifest verifies through the full
/// `verify_manifest` path (the byte-binding holds for the honest statement).
#[test]
#[ignore = "slow: full bb prove of a scan + filter member (sq-ajl byte-binding control)"]
fn forge_pubinput_positive_control_verifies() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "fg_pub_pos_scan");
    let (filter, art) = honest_filter_d1(5, FilterOp::Lt, 10, true, &challenge, &prover, "fg_pub_pos");
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    verify_full(&m, "fg_pub_pos_verify").expect("honest composed manifest verifies");
}

/// BINDING 6 forge: a GENUINE proof of `5 < 10 = true` presented under a manifest
/// DECLARING `bound = 99`. The reconstructed public inputs (bound=99) cannot
/// byte-match the proof's (bound=10) ⇒ `PublicInputMismatch`.
#[test]
#[ignore = "slow: full bb prove of a scan + filter member (sq-ajl byte-binding forge)"]
fn forge_pubinput_statement_substitution_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "fg_pub_sub_scan");
    let (mut filter, art) =
        honest_filter_d1(5, FilterOp::Lt, 10, true, &challenge, &prover, "fg_pub_sub");
    // FORGE: declare bound=99 while proof_hex still proves 5 < 10.
    if let ProofInputs::FilterInt { bound, .. } = &mut filter {
        *bound = 99;
    }
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    match verify_full(&m, "fg_pub_sub_verify") {
        Err(CheckError::PublicInputMismatch { proof: 1 }) => {}
        other => panic!("statement substitution must be PublicInputMismatch, got {other:?}"),
    }
}

/// BINDING 6 forge (verdict): a genuine proof of the FALSE verdict
/// `5 >= 10 = false`, presented declaring `expected = true`. The byte-compare on
/// the verdict word rejects ⇒ `PublicInputMismatch`.
#[test]
#[ignore = "slow: full bb prove (sq-ajl byte-binding verdict forge)"]
fn forge_pubinput_verdict_substitution_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "fg_pub_verdict_scan");
    // Honest: 5 >= 10 is FALSE.
    let (mut filter, art) =
        honest_filter_d1(5, FilterOp::Ge, 10, false, &challenge, &prover, "fg_pub_verdict");
    // FORGE: flip the declared verdict.
    if let ProofInputs::FilterInt { expected, .. } = &mut filter {
        *expected = true;
    }
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    match verify_full(&m, "fg_pub_verdict_verify") {
        Err(CheckError::PublicInputMismatch { proof: 1 }) => {}
        other => panic!("verdict substitution must be PublicInputMismatch, got {other:?}"),
    }
}

// === BINDING 7: distinct-graph strict ordering (duplicate-inclusion / COUNT) ===
//
// [OPUS-4.8] sq-vxq8 / plan S2.5. `scan_check` step 1b enforces
// `commitments[0] < commitments[1] < ...` (strict, on the field representative) so
// the K committed graphs are pairwise DISTINCT. This closes the
// duplicate-inclusion / COUNT-forgery class: including the SAME credential twice
// (its commitment repeated at two block indices) to forge a COUNT-style "I hold
// >=2 tickets" claim. Aggregates are out of the v1 fragment so the class is latent
// today, but the guard lands before any aggregate/COUNT support. Two layers gate
// it: (a) the host-side STRUCTURAL mirror in `prefilter_manifest_structure`
// (rejects a non-increasing commitment vector before any bb proof — runs without
// the toolchain), and (b) the in-circuit `<` itself (the authoritative gate — a
// duplicate-commitment witness is UNSATISFIABLE, so no proof can exist; gated
// behind the toolchain + `#[ignore]`).

/// Two DISTINCT named-graph credentials over a shared predicate `<ex/p>`, each
/// issuer-attested salt+status-bound, so a k=2 scan over `<ex/p>` builds and
/// (with honest distinct graphs) verifies. Each graph carries THREE `<ex/p>`
/// triples (distinct subjects => distinct commitments), so the k=2 scan discloses
/// 6 matched rows => derives the COMPILED `scan_k2_n16_r8` member (n=16, r=8).
/// Returns the two `GraphCommitment`s + their salts.
fn two_distinct_p_graphs(
) -> (sparq_zk::commit::GraphCommitment, Fr, sparq_zk::commit::GraphCommitment, Fr) {
    let g0 = vec![
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/a0")), iri("http://ex/p"), int_lit(1)),
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/a1")), iri("http://ex/p"), int_lit(2)),
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/a2")), iri("http://ex/p"), int_lit(3)),
    ];
    let g1 = vec![
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/b0")), iri("http://ex/p"), int_lit(4)),
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/b1")), iri("http://ex/p"), int_lit(5)),
        Triple::new(NamedOrBlankNode::NamedNode(iri("http://ex/b2")), iri("http://ex/p"), int_lit(6)),
    ];
    let salt0 = salt_from_bytes(&[21u8; 32]);
    let salt1 = salt_from_bytes(&[22u8; 32]);
    let c0 = commit_triples(&g0, salt0).unwrap();
    let c1 = commit_triples(&g1, salt1).unwrap();
    (c0, salt0, c1, salt1)
}

/// Build a k=2 scan-only manifest over `{ ?s <ex/p> ?o }` from scan inputs + their
/// attestations. Passes every structural gate except whatever the caller forged
/// (or, when honest, reaches MissingProof with no bb proof). `attributions` is
/// `[[0,1]]` (both graphs contribute — the honest truth the in-circuit attribution
/// proves).
fn k2_scan_only_manifest(
    scan_inputs: ProofInputs,
    attests: Vec<CommitmentAttestation>,
) -> ProofManifest {
    let sk = test_issuer_sk(1);
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: attests,
        attributions: vec![vec![0, 1]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof { inputs: scan_inputs, proof_hex: String::new() }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// Structural forge (NO bb): a k=2 scan whose two per-graph commitments are EQUAL
/// (the same credential's commitment included twice — the duplicate-inclusion
/// forge). The strict-ordering structural mirror must REJECT
/// (`ScanCommitmentsNotStrictlyIncreasing`) BEFORE any bb proof. Reaches the gate
/// through the SOUND `verify_manifest` entry point.
#[test]
fn forge_scan_duplicate_commitment_structural_rejected() {
    let (c0, salt0, c1b, salt1b) = two_distinct_p_graphs();
    let sk = test_issuer_sk(1);
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let mut scan = build_scan(&[c0.clone(), c1b.clone()], &pattern).expect("k=2 scan builds");
    if let ProofInputs::Scan { commitments, .. } = &mut scan.inputs {
        // FORGE: both slots carry c0's commitment (equal => not strictly increasing).
        let dup = FieldHex::from_field(&c0.commitment);
        commitments[0] = dup.clone();
        commitments[1] = dup;
    } else {
        unreachable!("scan inputs");
    }
    // Attest the (single distinct) commitment value so the forge reaches the
    // ordering gate, not the attestation gate.
    let attests = vec![attest_full(c0.commitment, salt0, &sk)];
    let _ = (salt1b, c1b);
    let m = k2_scan_only_manifest(scan.inputs, attests);
    match verify_full(&m, "forge_dup_commit_struct") {
        Err(CheckError::ScanCommitmentsNotStrictlyIncreasing { proof: 0, at: 1 }) => {}
        other => panic!(
            "duplicate (equal) commitments must be ScanCommitmentsNotStrictlyIncreasing, got {other:?}"
        ),
    }
}

/// Structural forge (NO bb): a k=2 scan whose commitments are DESCENDING
/// (out-of-order, distinct values). Strict ordering must REJECT
/// (`ScanCommitmentsNotStrictlyIncreasing`) — only the canonical ascending order
/// is accepted, so a prover cannot reorder graphs to dodge the gate.
#[test]
fn forge_scan_out_of_order_commitment_structural_rejected() {
    let (c0, salt0, c1, salt1) = two_distinct_p_graphs();
    let sk = test_issuer_sk(1);
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let mut scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 scan builds");
    // build_scan emits ASCENDING; FORGE by swapping to descending order.
    if let ProofInputs::Scan { commitments, attribution, .. } = &mut scan.inputs {
        commitments.reverse();
        attribution.reverse();
    } else {
        unreachable!("scan inputs");
    }
    let attests = vec![
        attest_full(c0.commitment, salt0, &sk),
        attest_full(c1.commitment, salt1, &sk),
    ];
    let m = k2_scan_only_manifest(scan.inputs, attests);
    match verify_full(&m, "forge_ooo_commit_struct") {
        Err(CheckError::ScanCommitmentsNotStrictlyIncreasing { proof: 0, at: 1 }) => {}
        other => panic!(
            "descending commitments must be ScanCommitmentsNotStrictlyIncreasing, got {other:?}"
        ),
    }
}

/// Positive control (sanity): the honest builder canonicalises graph order
/// ascending, so an honest k=2 scan-only manifest passes the strict-ordering
/// structural gate (and every other structural gate), reaching MissingProof
/// (witness-only). Proves the gate does not reject honest distinct graphs.
#[test]
fn honest_k2_scan_passes_strict_ordering_structural() {
    let (c0, salt0, c1, salt1) = two_distinct_p_graphs();
    let sk = test_issuer_sk(1);
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 scan builds");
    // build_scan emits commitments ascending; confirm that here so the test
    // documents the canonicalisation contract the gate depends on.
    if let ProofInputs::Scan { commitments, .. } = &scan.inputs {
        let a = commitments[0].to_field().unwrap();
        let b = commitments[1].to_field().unwrap();
        assert!(
            sparq_zk::field::field_to_be_bytes_32(&a) < sparq_zk::field::field_to_be_bytes_32(&b),
            "build_scan must emit strictly-increasing commitments"
        );
    } else {
        unreachable!("scan inputs");
    }
    let attests = vec![
        attest_full(c0.commitment, salt0, &sk),
        attest_full(c1.commitment, salt1, &sk),
    ];
    let m = k2_scan_only_manifest(scan.inputs, attests);
    match verify_full(&m, "honest_k2_strict_struct") {
        // Structural stage (incl. strict ordering) passed; only the bb proof is absent.
        Err(CheckError::MissingProof { proof: 0 }) => {}
        other => panic!(
            "honest k=2 scan should pass strict-ordering then hit MissingProof, got {other:?}"
        ),
    }
}

/// CIRCUIT-LEVEL forge (FULL toolchain): a k=2 scan witness whose two per-graph
/// commitments are EQUAL is UNSATISFIABLE under the in-circuit strict-ordering
/// `<` — `nargo execute` produces NO witness, so no proof can exist. This is the
/// AUTHORITATIVE gate (the structural mirror above is defence in depth). Uses the
/// fast witness-generation path (`gen_witness_tagged`); no `bb prove` needed to
/// demonstrate unsatisfiability.
#[test]
#[ignore = "slow/toolchain: real nargo witness-gen of a k=2 scan (sq-vxq8 strict-ordering circuit forge)"]
fn forge_scan_duplicate_commitment_circuit_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (c0, _s0, _c1, _s1) = two_distinct_p_graphs();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    // Build a k=2 scan over the SAME graph twice — the duplicate-inclusion forge.
    // Both graphs' encodings derive c0, so the commitment-recompute (step 1) holds
    // for both, and the ONLY violated constraint is the strict ordering
    // `commitments[0] < commitments[1]` (= `c0 < c0`, false). This isolates step
    // 1b: the witness is well-formed except for the duplicate the gate forbids.
    let scan = build_scan(&[c0.clone(), c0.clone()], &pattern).expect("k=2 dup scan builds");
    if let ProofInputs::Scan { commitments, .. } = &scan.inputs {
        assert_eq!(commitments[0], commitments[1], "forged duplicate commitments");
    } else {
        unreachable!("scan inputs");
    }
    let (id, toml) = prover_toml_for(
        &scan.inputs,
        &challenge,
        &scan.witness.counts,
        &scan.witness.enc,
        &[], None, None
    ).unwrap();
    // The in-circuit `commitments[0] < commitments[1]` (= `c0 < c0`) is FALSE, so
    // the relation is unsatisfiable: nargo writes no witness => Err.
    let res = prover.gen_witness_tagged(&id, &toml, "fg_dup_commit");
    assert!(
        res.is_err(),
        "a duplicate-commitment k=2 scan witness must be UNSATISFIABLE (strict-ordering gate), but nargo produced a witness"
    );
}

/// CIRCUIT-LEVEL positive control (FULL toolchain): an HONEST k=2 scan over two
/// DISTINCT graphs (ascending commitments, as build_scan emits) is SATISFIABLE
/// and PROVES + VERIFIES through the full `verify_manifest` path. Proves the
/// strict-ordering gate did NOT break honest k=2 proofs (the gate's completeness).
#[test]
#[ignore = "slow/toolchain: real bb prove+verify of an honest k=2 scan (sq-vxq8 strict-ordering positive control)"]
fn honest_k2_distinct_scan_proves_and_verifies() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (c0, salt0, c1, salt1) = two_distinct_p_graphs();
    let sk = test_issuer_sk(1);
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 scan builds");
    let (id, toml) = prover_toml_for(
        &scan.inputs,
        &challenge,
        &scan.witness.counts,
        &scan.witness.enc,
        &[], None, None
    ).unwrap();
    let out = scratch("honest_k2_distinct_prove");
    let art = prover
        .prove_in(&id, &toml, &out, "honest_k2_distinct")
        .expect("honest distinct k=2 scan PROVES (strict ordering holds)");
    let attests = vec![
        attest_full(c0.commitment, salt0, &sk),
        attest_full(c1.commitment, salt1, &sk),
    ];
    let mut m = k2_scan_only_manifest(scan.inputs, attests);
    m.sub_proofs[0].proof_hex = encode_artifacts(&art);
    verify_full(&m, "honest_k2_distinct_verify")
        .expect("honest distinct k=2 scan VERIFIES through verify_manifest");
}
