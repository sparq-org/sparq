//! Verifier-side commitment-method AUTHENTICATION, end to end through the
//! manifest gates.
//!
//! Three things are under test here:
//!
//! 1. **Back-compatibility.** An attestation over the method-UNBOUND message
//!    (`ZKSIG_C3`) authenticates as `Unbound`, is accepted under the default
//!    policy, and reaches the same verdict it reached before method
//!    authentication existed — including the same `InvalidIssuerSignature`
//!    refusal when the signature is bad. The whole pre-existing suite passing
//!    unedited is the broader form of this proof; the tests here state it
//!    directly.
//! 2. **The positive path.** An attestation over the METHOD-BOUND presentation
//!    message (`ZKSIG_M5`) authenticates as that method, and is accepted even
//!    under `MethodPolicy::require_method_bound`.
//! 3. **The refusals.** `require_method_bound` refuses an unbound attestation; an
//!    attested superseded method is refused without the written-down opt-in; a
//!    hidden-only entry whose declared message is no candidate is refused; and a
//!    REGISTRY entry's `ZKSIG_M3` signature is not a presentation attestation.
//!
//! Nothing here opens a value-lane gate; see `value_lane_pin.rs` for the test
//! that the gates stay closed.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::{commit_triples, CommitmentMethod};
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::registry::RegistryEntry;
use sparq_zk::sig::{
    public_key_to_hex, scheme_iri_to_field, status_list_id_to_field, status_ref_digest, SecretKey,
    SignatureScheme,
};
use sparq_zk_compose::build::{build_scan, Pattern, Slot};
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingMode, CommitmentAttestation, EntailmentRegime, FieldHex,
    HiddenIssuerAttestation, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::verifier::{
    prefilter_manifest_structure, CheckError, KeySet, RevocationPolicy,
};
use sparq_zk_compose::MethodPolicy;

const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const GRAPH_SALT: [u8; 32] = [7u8; 32];
/// The hidden-issuer Merkle depth the hidden-only fixtures declare.
const HIDDEN_DEPTH: u32 = 4;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn credential_graph() -> Vec<Triple> {
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/alice")),
        iri("http://ex/age"),
        Term::Literal(Literal::new_typed_literal(
            "25",
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )]
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

fn fixture_snapshot() -> StatusListSnapshot {
    StatusListSnapshot {
        status_list: STATUS_LIST.to_string(),
        version: STATUS_VERSION,
        bits: vec![0u8],
    }
}

fn fresh_policy() -> RevocationPolicy {
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot())
}

fn fixture_status_ref() -> Fr {
    status_ref_digest(
        &status_list_id_to_field(STATUS_LIST),
        STATUS_INDEX,
        STATUS_VERSION,
    )
}

/// A one-scan manifest over the fixture credential graph, with NO attestation
/// yet. Returns it alongside the scan commitment and the salt, so a test can
/// attach whichever attestation shape it is probing.
fn scan_manifest() -> (ProofManifest, Fr, Fr) {
    let salt = salt_from_bytes(&GRAPH_SALT);
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let commitment = commit.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[commit], &pattern).expect("scan builds");
    let m = ProofManifest {
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into(),
        issuers: vec![],
        key_set: vec![],
        commitment_attestations: vec![],
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex("0x2a".into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![SubProof { inputs: scan.inputs, proof_hex: String::new() }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![],
        holder_set_proofs: vec![],
        fully_hidden_revocation: None,
    };
    (m, commitment, salt)
}

fn attestation(commitment: Fr, signature: String, sk: &SecretKey) -> CommitmentAttestation {
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature,
        cryptosuite: SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri().to_string(),
        salt: None,
        status: Some(AttestedStatusRef::clear(STATUS_INDEX, STATUS_VERSION)),
        holder: None,
    }
}

/// The method-UNBOUND (`ZKSIG_C3`) attestation — what every credential issued so
/// far carries.
fn attest_unbound(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    let mut a = attestation(
        commitment,
        sk.sign_commitment_with_status(&commitment, &salt, &fixture_status_ref()),
        sk,
    );
    a.salt = Some(FieldHex::from_field(&salt));
    a
}

/// The METHOD-BOUND presentation (`ZKSIG_M5`) attestation for `method`.
fn attest_bound(
    commitment: Fr,
    salt: Fr,
    method: CommitmentMethod,
    sk: &SecretKey,
) -> CommitmentAttestation {
    let scheme = scheme_iri_to_field(method.scheme_iri());
    let mut a = attestation(
        commitment,
        sk.sign_presentation_with_status_and_scheme(
            &commitment,
            &salt,
            &fixture_status_ref(),
            &scheme,
        ),
        sk,
    );
    a.salt = Some(FieldHex::from_field(&salt));
    a
}

fn trusted(sk: &SecretKey) -> KeySet {
    KeySet::from_hex_keys([public_key_to_hex(&sk.public_key())])
}

// ---------------------------------------------------------------- back-compat

/// BACK-COMPAT: the attestation shape every existing manifest carries — a
/// signature over the method-unbound `ZKSIG_C3` message — still passes the
/// pre-filter under the DEFAULT policy, with no attestation field added or
/// changed.
#[test]
fn an_existing_method_unbound_attestation_still_prefilters() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.commitment_attestations.push(attest_unbound(c, salt, &sk));
    prefilter_manifest_structure(&m, &trusted(&sk), &fresh_policy())
        .expect("a method-unbound attestation is accepted under the default policy");
}

/// PRECEDENCE: a genuinely bad signature still yields `InvalidIssuerSignature`.
/// Authenticating the method must not turn a forged signature into one of the
/// new method refusals — that would change the error an existing manifest sees.
#[test]
fn a_forged_signature_is_still_the_invalid_signature_refusal() {
    let sk = SecretKey::from_seed(1);
    let other = SecretKey::from_seed(2);
    let (mut m, c, salt) = scan_manifest();
    // A signature by a DIFFERENT key, presented under `sk`'s public key: no
    // candidate message verifies.
    let mut att = attest_unbound(c, salt, &sk);
    att.signature = other.sign_commitment_with_status(&c, &salt, &fixture_status_ref());
    m.commitment_attestations.push(att);
    match prefilter_manifest_structure(&m, &trusted(&sk), &fresh_policy()) {
        Err(CheckError::InvalidIssuerSignature { .. }) => {}
        other => panic!("expected InvalidIssuerSignature, got {other:?}"),
    }
}

/// The method refusals never fire on an UNTRUSTED key: key-set membership is
/// still reported first, so the gate ordering is unchanged.
#[test]
fn an_untrusted_issuer_is_still_reported_before_any_method_refusal() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.commitment_attestations.push(attest_unbound(c, salt, &sk));
    let strict = KeySet::empty().with_method_policy(MethodPolicy::current().require_method_bound());
    match prefilter_manifest_structure(&m, &strict, &fresh_policy()) {
        Err(CheckError::IssuerKeyNotInKeySet { .. }) => {}
        other => panic!("expected IssuerKeyNotInKeySet, got {other:?}"),
    }
}

// ------------------------------------------------------------- positive path

/// A `ZKSIG_M5` attestation authenticates its method, so the manifest passes
/// even under the STRICT policy that refuses a method-unbound attestation.
#[test]
fn a_method_bound_attestation_passes_the_strict_policy() {
    let sk = SecretKey::from_seed(1);
    for method in [CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV2] {
        let (mut m, c, salt) = scan_manifest();
        m.commitment_attestations.push(attest_bound(c, salt, method, &sk));
        let strict =
            trusted(&sk).with_method_policy(MethodPolicy::current().require_method_bound());
        prefilter_manifest_structure(&m, &strict, &fresh_policy()).unwrap_or_else(|e| {
            panic!("an M5 attestation for {} must pass: {e}", method.scheme_iri())
        });
        // And under the default policy too.
        prefilter_manifest_structure(&m, &trusted(&sk), &fresh_policy())
            .expect("and under the default policy");
    }
}

// ----------------------------------------------------------------- refusals

/// THE HEADLINE REFUSAL: `require_method_bound` refuses an attestation that
/// binds no method.
#[test]
fn require_method_bound_refuses_a_method_unbound_attestation() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.commitment_attestations.push(attest_unbound(c, salt, &sk));
    let strict = trusted(&sk).with_method_policy(MethodPolicy::current().require_method_bound());
    match prefilter_manifest_structure(&m, &strict, &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { .. }) => {}
        other => panic!("expected MethodNotAuthenticated, got {other:?}"),
    }
}

/// An attested SUPERSEDED method is refused by the default policy, and admitted
/// only with the written-down opt-in. The opt-in is the whole difference.
#[test]
fn an_attested_legacy_method_needs_the_written_down_opt_in() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.commitment_attestations
        .push(attest_bound(c, salt, CommitmentMethod::DualLeafV1, &sk));
    match prefilter_manifest_structure(&m, &trusted(&sk), &fresh_policy()) {
        Err(CheckError::MethodNotAdmitted { method_iri, .. }) => {
            assert_eq!(method_iri, CommitmentMethod::DualLeafV1.scheme_iri());
        }
        other => panic!("expected MethodNotAdmitted, got {other:?}"),
    }
    let permissive =
        trusted(&sk).with_method_policy(MethodPolicy::current().admit_legacy_dual_leaf_v1());
    prefilter_manifest_structure(&m, &permissive, &fresh_policy())
        .expect("the legacy opt-in admits it");
}

/// The policy rides the `KeySet`, so the default is what an existing caller that
/// never heard of it gets.
#[test]
fn the_key_sets_default_policy_is_the_back_compatible_one() {
    assert_eq!(KeySet::empty().method_policy(), MethodPolicy::current());
    assert!(KeySet::empty().method_policy().accepts_unbound());
    assert_eq!(
        KeySet::empty()
            .with_method_policy(MethodPolicy::current().require_method_bound())
            .method_policy(),
        MethodPolicy::current().require_method_bound()
    );
    // `with_method_policy` does not disturb the keys or the hidden-issuer depth.
    let sk = SecretKey::from_seed(1);
    let ks = trusted(&sk).with_hidden_issuer_depth(4);
    let with_policy = ks.clone().with_method_policy(MethodPolicy::current().require_method_bound());
    assert_eq!(ks.signature_modes(), with_policy.signature_modes());
}

// ----------------------------------------------- the registry/presentation split

/// A REGISTRY entry's signature is over `ZKSIG_M3`, a different signed object
/// from the `ZKSIG_M5` presentation message. Presenting a registry signature as a
/// clear manifest attestation therefore does not authenticate anything, and the
/// verifier reports its pre-existing `InvalidIssuerSignature` refusal — under the
/// default policy and under the strict one alike.
///
/// Why this matters: an issuer records commitments in its registry for reasons
/// unrelated to authorising a bearer presentation (including commitments it
/// holder-bound to a specific key), so "recorded" must not imply "presentable".
/// The two domain tags are what keeps the two statements apart.
#[test]
fn a_registry_signature_is_not_a_presentation_attestation() {
    let sk = SecretKey::from_seed(1);
    let (m, c, salt) = scan_manifest();
    let entry = RegistryEntry::issued_with_status(
        iri("http://ex/vc/1"),
        c,
        salt,
        CommitmentMethod::DualLeafV2,
        iri(STATUS_LIST),
        STATUS_INDEX,
        STATUS_VERSION,
        sparq_zk::registry::Status::Active,
        &sk,
    )
    .expect("DualLeafV2 is selectable for new issuance");
    assert!(
        entry.verify_commitment_signature_with_status(),
        "the fixture entry's own registry signature must verify as a registry one"
    );

    // The same commitment, salt, key and clear status reference the entry records,
    // assembled by hand into a manifest attestation — only the SIGNATURE is the
    // registry's.
    let mut bridged = attestation(
        c,
        entry
            .commitment_signature
            .clone()
            .expect("an issued entry carries a signature"),
        &sk,
    );
    bridged.salt = Some(FieldHex::from_field(&salt));

    for (label, ks) in [
        ("default", trusted(&sk)),
        (
            "require_method_bound",
            trusted(&sk).with_method_policy(MethodPolicy::current().require_method_bound()),
        ),
    ] {
        let mut m = m.clone();
        m.commitment_attestations.push(bridged.clone());
        match prefilter_manifest_structure(&m, &ks, &fresh_policy()) {
            Err(CheckError::InvalidIssuerSignature { .. }) => {}
            other => panic!("[{label}] expected InvalidIssuerSignature, got {other:?}"),
        }
    }

    // And the issuer's own PRESENTATION attestation over the same values passes,
    // so the refusal above is the signed message and nothing else.
    let mut ok = m.clone();
    ok.commitment_attestations.push(attest_bound(
        c,
        salt,
        CommitmentMethod::DualLeafV2,
        &sk,
    ));
    let strict = trusted(&sk).with_method_policy(MethodPolicy::current().require_method_bound());
    prefilter_manifest_structure(&ok, &strict, &fresh_policy())
        .expect("the issuer's presentation attestation over the same values passes");
}

// ------------------------------------------------------ the hidden-only path

/// A HIDDEN-ONLY commitment declares the message its hidden-issuer proof is over.
/// That declared value must EQUAL one of the verifier-built candidates: it is what
/// selects the method the bb gate is then held to, so an unmatched value is
/// refused in the pre-filter (`HiddenIssuerMessageMismatch`) rather than passed
/// over — under the default policy and under `require_method_bound` alike.
///
/// The honest method-unbound message over the SAME manifest still passes by
/// default (back-compatibility) and is refused once a method is required, which is
/// what shows the refusal above is the unmatched message and not the fixture.
#[test]
fn an_unmatched_hidden_only_message_is_refused() {
    let sk = SecretKey::from_seed(1);
    let (m, c, salt) = scan_manifest();
    let hidden = |message: Fr| {
        let mut m = m.clone();
        m.commitment_attestations = vec![];
        m.hidden_issuer_attestations = vec![HiddenIssuerAttestation {
            commitment: FieldHex::from_field(&c),
            depth: HIDDEN_DEPTH,
            key_set_root: FieldHex("0x0".into()),
            message: FieldHex::from_field(&message),
            salt: Some(FieldHex::from_field(&salt)),
            proof_hex: String::new(),
        }];
        m
    };
    let hidden_keys = || trusted(&sk).with_hidden_issuer_depth(HIDDEN_DEPTH);
    let strict =
        || hidden_keys().with_method_policy(MethodPolicy::current().require_method_bound());
    let honest = sparq_zk::sig::commitment_message_with_status(&c, &salt, &fixture_status_ref());

    prefilter_manifest_structure(&hidden(honest), &hidden_keys(), &fresh_policy())
        .expect("the honest method-unbound hidden message still passes by default");
    match prefilter_manifest_structure(&hidden(honest), &strict(), &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { .. }) => {}
        other => panic!("expected MethodNotAuthenticated for an unbound hidden message, got {other:?}"),
    }

    for (label, ks) in [("default", hidden_keys()), ("require_method_bound", strict())] {
        match prefilter_manifest_structure(&hidden(Fr::from(1u64)), &ks, &fresh_policy()) {
            Err(CheckError::HiddenIssuerMessageMismatch { .. }) => {}
            other => panic!("[{label}] expected HiddenIssuerMessageMismatch, got {other:?}"),
        }
    }

    // A method-BOUND hidden message names its method and passes the strict policy.
    let bound = sparq_zk::sig::presentation_message_with_status_and_scheme(
        &c,
        &salt,
        &fixture_status_ref(),
        &scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri()),
    );
    prefilter_manifest_structure(&hidden(bound), &strict(), &fresh_policy())
        .expect("a method-bound hidden message passes the strict policy");
}

// --------------------------------------------------------- live toolchain lane

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
    let dir = std::env::temp_dir().join(format!("sparq_method_auth_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// LIVE: a full `verify_manifest` — real `bb` scan proof, public-input
/// reconstruction, `bb verify` — over a manifest whose attestation is the
/// METHOD-BOUND `ZKSIG_M5` one, under the strict policy.
///
/// The structural tests above stop at the pre-filter. This one runs the whole
/// entry point, so it also proves method authentication does not disturb the
/// nonce binding, the reconstruction byte-compare or the proof verification.
///
/// `#[ignore]` so the toolchain lane's `cargo test -- --ignored` step actually
/// executes it; the default lane has no `nargo`/`bb` and would only skip.
#[test]
#[ignore = "slow: full bb prove + verify of a scan member"]
fn live_verify_manifest_with_an_m5_attestation() {
    use sparq_zk_compose::driver::CircuitProver;
    use sparq_zk_compose::toml::prover_toml_for;
    use sparq_zk_compose::verifier::{
        encode_artifacts, verify_manifest, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
        InMemorySeenNonces, VerifierNonce,
    };

    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping the live M5 verify_manifest");
        return;
    }
    let sk = SecretKey::from_seed(1);
    let salt = salt_from_bytes(&GRAPH_SALT);
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let commitment = commit.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[commit], &pattern).unwrap();
    let challenge = FieldHex("0x2a".into());
    let (id, toml) = prover_toml_for(
        &scan.inputs,
        &challenge,
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .unwrap();
    let prover = CircuitProver::from_crate_root();
    let art = prover
        .prove_in(&id, &toml, &scratch("m5_prove"), "m5_prove")
        .expect("the scan member proves");

    let (mut m, _, _) = scan_manifest();
    m.sub_proofs = vec![SubProof { inputs: scan.inputs, proof_hex: encode_artifacts(&art) }];
    m.commitment_attestations.push(attest_bound(
        commitment,
        salt,
        CommitmentMethod::DualLeafV2,
        &sk,
    ));
    let strict = trusted(&sk).with_method_policy(MethodPolicy::current().require_method_bound());
    verify_manifest(
        &m,
        &prover,
        &scratch("m5_verify"),
        &strict,
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &VerifierNonce::from_hex("0x2a").unwrap(),
        &InMemorySeenNonces::new(),
    )
    .expect("a manifest with an M5 attestation verifies end to end");
}

/// LIVE: a HIDDEN-ONLY commitment whose hidden-issuer proof is over the
/// METHOD-BOUND `ZKSIG_M5` presentation message.
///
/// The hidden path has no clear signature, so the method is recovered by
/// matching the hidden entry's public `m` against the same candidate set — and
/// the verifier must then recompute THAT message for the public-input
/// byte-compare. If it recomputed the method-unbound one instead, this test
/// would fail on `HiddenIssuerMessageMismatch`.
///
/// `#[ignore]`: it proves both a scan member and the depth-4 hidden-issuer
/// member, which is the same cost as the existing hidden-issuer live tests.
#[test]
#[ignore = "slow: full bb prove of a scan + the hidden-issuer member"]
fn live_hidden_issuer_proof_over_the_m5_message() {
    use sparq_zk_compose::driver::CircuitProver;
    use sparq_zk_compose::issuer::{
        hidden_issuer_prover_toml, key_membership_witness, key_set_root, HiddenIssuerWitness,
    };
    use sparq_zk_compose::manifest::{CircuitId, HiddenIssuerAttestation};
    use sparq_zk_compose::toml::prover_toml_for;
    use sparq_zk_compose::verifier::{
        encode_artifacts, verify_manifest, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
        InMemorySeenNonces, VerifierNonce,
    };

    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping the live hidden-issuer M5 test");
        return;
    }
    const DEPTH: u32 = 4;
    let method = CommitmentMethod::DualLeafV2;
    // ONE trusted issuer, so its canonical index is 0 and the order is trivial.
    let sk = SecretKey::from_seed(1);
    let keyset = trusted(&sk)
        .with_hidden_issuer_depth(DEPTH)
        .with_method_policy(MethodPolicy::current().require_method_bound());

    let salt = salt_from_bytes(&GRAPH_SALT);
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let commitment = commit.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[commit], &pattern).unwrap();
    let challenge = FieldHex("0x2a".into());
    let challenge_fr = challenge.to_field().unwrap();
    let prover = CircuitProver::from_crate_root();
    let (scan_id, scan_toml) = prover_toml_for(
        &scan.inputs,
        &challenge,
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .unwrap();
    let scan_art = prover
        .prove_in(&scan_id, &scan_toml, &scratch("hi_m5_scan"), "hi_m5_scan")
        .expect("the scan member proves");

    // The MESSAGE is the method-bound one.
    let scheme = scheme_iri_to_field(method.scheme_iri());
    let message = sparq_zk::sig::presentation_message_with_status_and_scheme(
        &commitment,
        &salt,
        &fixture_status_ref(),
        &scheme,
    );
    let sig_hex = sk.sign_presentation_with_status_and_scheme(
        &commitment,
        &salt,
        &fixture_status_ref(),
        &scheme,
    );
    let sig = sparq_zk::sig::signature_from_hex(&sig_hex).unwrap();
    let schnorr = sparq_zk::sig::in_circuit_witness(&sk.public_key(), &message, &sig).unwrap();
    let pks = vec![sk.public_key()];
    let root = key_set_root(&pks, DEPTH).expect("root");
    let siblings = key_membership_witness(&pks, DEPTH, 0).expect("path");
    let hi_toml = hidden_issuer_prover_toml(
        &challenge_fr,
        &message,
        &root,
        &HiddenIssuerWitness { schnorr, index: 0, siblings },
    );
    let hi_art = prover
        .prove_in(
            &CircuitId::HiddenIssuer { depth: DEPTH },
            &hi_toml,
            &scratch("hi_m5_issuer"),
            "hi_m5_issuer",
        )
        .expect("the hidden-issuer member proves");

    let (mut m, _, _) = scan_manifest();
    m.sub_proofs = vec![SubProof { inputs: scan.inputs, proof_hex: encode_artifacts(&scan_art) }];
    // HIDDEN-ONLY: no clear attestation at all.
    m.commitment_attestations = vec![];
    m.hidden_issuer_attestations = vec![HiddenIssuerAttestation {
        commitment: FieldHex::from_field(&commitment),
        depth: DEPTH,
        key_set_root: FieldHex::from_field(
            &keyset.hidden_issuer_root(DEPTH).expect("authoritative root"),
        ),
        message: FieldHex::from_field(&message),
        salt: Some(FieldHex::from_field(&salt)),
        proof_hex: encode_artifacts(&hi_art),
    }];
    verify_manifest(
        &m,
        &prover,
        &scratch("hi_m5_verify"),
        &keyset,
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &VerifierNonce::from_hex("0x2a").unwrap(),
        &InMemorySeenNonces::new(),
    )
    .expect("a hidden-issuer proof over the M5 message verifies end to end");
}
