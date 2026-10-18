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
use sparq_zk_compose::{AuthenticatedMethod, MethodAuthError, MethodPolicy};

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

/// The attestation shape every pre-method-binding manifest carries — a
/// signature over the method-unbound `ZKSIG_C3` message — is now REFUSED under
/// the DEFAULT policy, and passes the pre-filter only under the explicit
/// `MethodPolicy::admit_unbound` opt-in.
///
/// Nothing about the attestation changes: no field is added or removed. What
/// changed is the relying party's default answer to "this issuer attested no
/// commitment method".
#[test]
fn an_existing_method_unbound_attestation_needs_the_explicit_opt_in() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.commitment_attestations.push(attest_unbound(c, salt, &sk));

    // Refused by default, and the refusal names its own cause rather than
    // masquerading as a bad signature.
    match prefilter_manifest_structure(&m, &trusted(&sk), &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { .. }) => {}
        other => panic!("expected a method-not-authenticated refusal, got {:?}", other),
    }

    // Accepted under the written-down downgrade.
    let permissive = trusted(&sk).with_method_policy(MethodPolicy::current().admit_unbound());
    prefilter_manifest_structure(&m, &permissive, &fresh_policy())
        .expect("admit_unbound accepts already-issued method-unbound material");
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
/// never heard of it gets — and that default is now FAIL-CLOSED: an attestation
/// that attests no commitment method is REFUSED unless the relying party says
/// otherwise.
///
/// This is the sq-j506 prerequisite. A commitment whose method is not
/// issuer-attested has no authenticated leaf shape, so there is nothing to pin a
/// lane constant or a pattern-slot encoding against. It opens no value-lane
/// gate: the Stage-0 value-lane refusals are untouched.
#[test]
fn the_key_sets_default_policy_refuses_an_unbound_attestation() {
    assert_eq!(KeySet::empty().method_policy(), MethodPolicy::current());
    assert!(
        !KeySet::empty().method_policy().accepts_unbound(),
        "the default policy must refuse a method-UNBOUND attestation"
    );
    // The refusal is a POLICY decision, so it names its own cause rather than
    // being folded into an invalid-signature refusal.
    assert_eq!(
        MethodPolicy::current().admit(AuthenticatedMethod::Unbound),
        Err(MethodAuthError::MethodNotAuthenticated)
    );
    // `admit_unbound` is the written-down opt-in back to the old behaviour, and
    // `require_method_bound` takes the requirement back again.
    let permissive = MethodPolicy::current().admit_unbound();
    assert!(permissive.accepts_unbound());
    assert_eq!(
        permissive.admit(AuthenticatedMethod::Unbound),
        Ok(AuthenticatedMethod::Unbound)
    );
    assert!(!permissive.require_method_bound().accepts_unbound());
    // Opting in to unbound material does NOT also admit the superseded
    // dual-leaf version: the two dials are independent.
    assert_eq!(
        permissive.admit(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1)),
        Err(MethodAuthError::MethodNotAdmitted(CommitmentMethod::DualLeafV1))
    );
    assert_eq!(
        KeySet::empty()
            .with_method_policy(MethodPolicy::current().admit_unbound())
            .method_policy(),
        MethodPolicy::current().admit_unbound()
    );
    // `with_method_policy` does not disturb the keys or the hidden-issuer depth.
    let sk = SecretKey::from_seed(1);
    let ks = trusted(&sk).with_hidden_issuer_depth(4);
    let with_policy = ks.clone().with_method_policy(MethodPolicy::current().admit_unbound());
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
    let strict = || trusted(&sk).with_hidden_issuer_depth(HIDDEN_DEPTH);
    let permissive =
        || strict().with_method_policy(MethodPolicy::current().admit_unbound());
    let honest = sparq_zk::sig::commitment_message_with_status(&c, &salt, &fixture_status_ref());

    // An unbound hidden message is refused by the DEFAULT policy, and passes
    // only under the explicit `admit_unbound` downgrade.
    match prefilter_manifest_structure(&hidden(honest), &strict(), &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { .. }) => {}
        other => panic!("expected MethodNotAuthenticated for an unbound hidden message, got {other:?}"),
    }
    prefilter_manifest_structure(&hidden(honest), &permissive(), &fresh_policy())
        .expect("admit_unbound accepts the honest method-unbound hidden message");

    // The message-mismatch refusal precedes the method policy either way: a
    // declared message matching no candidate is refused for THAT reason, under
    // the strict default and under the downgrade alike.
    for (label, ks) in [("default", strict()), ("admit_unbound", permissive())] {
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

/// A hidden-issuer entry over `c` declaring `message`, with the salt disclosed on
/// the entry itself. The `proof_hex` is empty: these tests stop at the pre-filter,
/// which is the stage that reads the declared message.
fn hidden_entry(c: Fr, salt: Fr, message: FieldHex) -> HiddenIssuerAttestation {
    HiddenIssuerAttestation {
        commitment: FieldHex::from_field(&c),
        depth: HIDDEN_DEPTH,
        key_set_root: FieldHex("0x0".into()),
        message,
        salt: Some(FieldHex::from_field(&salt)),
        proof_hex: String::new(),
    }
}

/// EVERY hidden-only commitment gets its OWN policy decision, not just the first
/// one the scan loop reaches.
///
/// Two scans over two distinctly-salted committed graphs, both hidden-only: the
/// first declares the method-BOUND `ZKSIG_M5` message, the second the
/// method-UNBOUND one. Under `require_method_bound` the second must be refused —
/// the first commitment's admitted entry must not stand in for it, and the
/// second must not fall through to an absent-key default.
#[test]
fn each_hidden_only_commitment_gets_its_own_policy_decision() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c1, salt1) = scan_manifest();
    let salt2 = salt_from_bytes(&[9u8; 32]);
    let graph2 = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/bob")),
        iri("http://ex/age"),
        Term::Literal(Literal::new_typed_literal(
            "30",
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )];
    let commit2 = commit_triples(&graph2, salt2).unwrap();
    let c2 = commit2.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan2 = build_scan(&[commit2], &pattern).unwrap();
    m.sub_proofs.push(SubProof { inputs: scan2.inputs, proof_hex: String::new() });
    m.attributions.push(vec![0]);
    m.query = "SELECT ?s ?o ?t ?u WHERE { ?s <http://ex/age> ?o . ?t <http://ex/age> ?u }".into();

    let bound1 = sparq_zk::sig::presentation_message_with_status_and_scheme(
        &c1,
        &salt1,
        &fixture_status_ref(),
        &scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri()),
    );
    let unbound2 = sparq_zk::sig::commitment_message_with_status(&c2, &salt2, &fixture_status_ref());
    m.hidden_issuer_attestations = vec![
        hidden_entry(c1, salt1, FieldHex::from_field(&bound1)),
        hidden_entry(c2, salt2, FieldHex::from_field(&unbound2)),
    ];
    let strict = trusted(&sk)
        .with_hidden_issuer_depth(HIDDEN_DEPTH)
        .with_method_policy(MethodPolicy::current().require_method_bound());
    match prefilter_manifest_structure(&m, &strict, &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { commitment }) => assert_eq!(
            FieldHex(commitment).to_field(),
            Some(c2),
            "the refusal must name the second, method-unbound commitment"
        ),
        other => panic!(
            "expected MethodNotAuthenticated for the second, method-unbound hidden-only commitment, got {other:?}"
        ),
    }
}

/// A hidden-only commitment with NO disclosed status reference builds no
/// candidate set, so the message the bb gate would bind is the method-UNBOUND
/// one — and `require_method_bound` must refuse it, through the whole
/// pre-filter (not only through the helper that classifies the message).
#[test]
fn a_hidden_only_commitment_without_a_status_reference_is_refused_when_a_method_is_required() {
    let sk = SecretKey::from_seed(1);
    let (mut m, c, salt) = scan_manifest();
    m.revocation = None;
    let bound = sparq_zk::sig::presentation_message_with_status_and_scheme(
        &c,
        &salt,
        &fixture_status_ref(),
        &scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri()),
    );
    m.hidden_issuer_attestations = vec![hidden_entry(c, salt, FieldHex::from_field(&bound))];
    let strict = trusted(&sk)
        .with_hidden_issuer_depth(HIDDEN_DEPTH)
        .with_method_policy(MethodPolicy::current().require_method_bound());
    match prefilter_manifest_structure(&m, &strict, &fresh_policy()) {
        Err(CheckError::MethodNotAuthenticated { .. }) => {}
        other => panic!(
            "expected MethodNotAuthenticated with no candidate set to bind a method, got {other:?}"
        ),
    }
    // Control: the refusal is the POLICY's, not something structural about a
    // hidden-only commitment. Under the `admit_unbound` downgrade the same
    // manifest is not refused for its method.
    let permissive = trusted(&sk)
        .with_hidden_issuer_depth(HIDDEN_DEPTH)
        .with_method_policy(MethodPolicy::current().admit_unbound());
    let r = prefilter_manifest_structure(&m, &permissive, &fresh_policy());
    assert!(
        !matches!(r, Err(CheckError::MethodNotAuthenticated { .. })),
        "admit_unbound accepts an unbound method, got {r:?}"
    );
}

/// The declared message of EVERY hidden entry over a hidden-only commitment is
/// checked, so the message check does not depend on the order the prover
/// listed them in: `[honest, unmatched]` and `[unmatched, honest]` are both refused.
///
/// Two entries that each DO match a candidate but match DIFFERENT ones are
/// refused the same way, in either order: there is then no single method for the
/// relying party's policy to decide on, so neither is taken.
#[test]
fn every_hidden_entry_declared_message_is_checked_in_either_order() {
    let sk = SecretKey::from_seed(1);
    let (m, c, salt) = scan_manifest();
    let honest = FieldHex::from_field(&sparq_zk::sig::commitment_message_with_status(
        &c,
        &salt,
        &fixture_status_ref(),
    ));
    let bound = FieldHex::from_field(
        &sparq_zk::sig::presentation_message_with_status_and_scheme(
            &c,
            &salt,
            &fixture_status_ref(),
            &scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri()),
        ),
    );
    let unmatched = FieldHex::from_field(&Fr::from(1u64));
    let keys = trusted(&sk).with_hidden_issuer_depth(HIDDEN_DEPTH);
    let orders = [
        ("honest first", vec![honest.clone(), unmatched.clone()]),
        ("unmatched first", vec![unmatched, honest.clone()]),
        ("unbound then bound", vec![honest.clone(), bound.clone()]),
        ("bound then unbound", vec![bound, honest]),
    ];
    for (label, messages) in orders {
        let mut mm = m.clone();
        mm.hidden_issuer_attestations = messages
            .into_iter()
            .map(|message| hidden_entry(c, salt, message))
            .collect();
        match prefilter_manifest_structure(&mm, &keys, &fresh_policy()) {
            Err(CheckError::HiddenIssuerMessageMismatch { .. }) => {}
            other => panic!("[{}] expected HiddenIssuerMessageMismatch, got {:?}", label, other),
        }
    }
}

// --------------------------------------------------------- live toolchain lane

mod support;

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
/// executes it; the default lane has no `nargo`/`bb` and declares that with
/// `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN=1`.
#[test]
#[ignore = "slow: full bb prove + verify of a scan member"]
fn live_verify_manifest_with_an_m5_attestation() {
    use sparq_zk_compose::driver::CircuitProver;
    use sparq_zk_compose::toml::prover_toml_for;
    use sparq_zk_compose::verifier::{
        encode_artifacts, verify_manifest, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
        InMemorySeenNonces, VerifierNonce,
    };

    if !support::require_toolchain("live_verify_manifest_with_an_m5_attestation", &["nargo", "bb"]) {
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

    if !support::require_toolchain("live_hidden_issuer_proof_over_the_m5_message", &["nargo", "bb"]) {
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

/// LIVE, ADDITIVE mode: a commitment carrying BOTH a clear `ZKSIG_M5`
/// attestation AND a hidden-issuer proof over that same M5 message, under
/// `require_method_bound`.
///
/// This is the shape the declared-message rule must NOT disturb. With a clear
/// attestation present the method comes from the clear signature, and the
/// hidden-issuer stage then byte-compares the message it recomputes from that
/// authenticated method — so the clear-authenticated method is what the hidden
/// proof is held to, and the manifest verifies. Recording the method-unbound
/// default for the clear attestation instead would make the recomputed message
/// disagree with the proof's public input and fail this test.
///
/// `#[ignore]`: it proves both a scan member and the depth-4 hidden-issuer
/// member, the same cost as the live tests above.
#[test]
#[ignore = "slow: full bb prove of a scan + the hidden-issuer member"]
fn live_additive_clear_m5_attestation_plus_a_hidden_proof_over_the_m5_message() {
    use sparq_zk_compose::driver::CircuitProver;
    use sparq_zk_compose::issuer::{
        hidden_issuer_prover_toml, key_membership_witness, key_set_root, HiddenIssuerWitness,
    };
    use sparq_zk_compose::manifest::CircuitId;
    use sparq_zk_compose::toml::prover_toml_for;
    use sparq_zk_compose::verifier::{
        encode_artifacts, verify_manifest, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
        InMemorySeenNonces, VerifierNonce,
    };

    if !support::require_toolchain("live_additive_clear_m5_attestation_plus_a_hidden_proof_over_the_m5_message", &["nargo", "bb"]) {
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
        .prove_in(&scan_id, &scan_toml, &scratch("add_m5_scan"), "add_m5_scan")
        .expect("the scan member proves");

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
            &scratch("add_m5_issuer"),
            "add_m5_issuer",
        )
        .expect("the hidden-issuer member proves");

    let (mut m, _, _) = scan_manifest();
    m.sub_proofs = vec![SubProof { inputs: scan.inputs, proof_hex: encode_artifacts(&scan_art) }];
    // ADDITIVE: a clear M5 attestation AND a hidden proof over the same message.
    m.commitment_attestations = vec![attest_bound(commitment, salt, method, &sk)];
    m.hidden_issuer_attestations = vec![HiddenIssuerAttestation {
        commitment: FieldHex::from_field(&commitment),
        depth: DEPTH,
        key_set_root: FieldHex::from_field(
            &keyset.hidden_issuer_root(DEPTH).expect("authoritative root"),
        ),
        message: FieldHex::from_field(&message),
        salt: None,
        proof_hex: encode_artifacts(&hi_art),
    }];
    verify_manifest(
        &m,
        &prover,
        &scratch("add_m5_verify"),
        &keyset,
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &VerifierNonce::from_hex("0x2a").unwrap(),
        &InMemorySeenNonces::new(),
    )
    .expect("a clear M5 attestation alongside a hidden proof over that same M5 message verifies");
}
