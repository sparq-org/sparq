// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-c2ql (HolderPoP T6 / B2): the in-circuit holder Proof-of-Possession binding
//! edge — the HIDDEN-key tier that ties the proven holder key to the ISSUER-ATTESTED
//! credential, the analogue of the clear-key T3/sq-z8s7 B1 closure but WITHOUT
//! disclosing the holder key.
//!
//! ## What this gates (the binding edge)
//! A [`HolderPokProof`] is a bb proof of the `holder_pok` relation (knowledge of a
//! holder secret `hsk` with `hpk = hsk·G` and `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])
//! == holder_pk_digest`, with `hsk`/`hpk` PRIVATE). The verifier
//! ([`verify_manifest`] → `bind_holder_pok`) does NOT trust the proof's public
//! `holder_pk_digest`: it reads the digest from the ISSUER-ATTESTED
//! [`AttestedHolderBinding`] on the attestation covering the PoK's scan-referenced
//! commitment (anchored in the issuer's Schnorr signature, under the external `K`),
//! reconstructs the proof's public inputs from its own nonce + THAT digest, and
//! byte-compares + `bb verify`s. So the proven hidden holder key is bound to the
//! issuer-signed credential.
//!
//! ## SOUNDNESS (load-bearing, NOT a security claim)
//! This wires the binding edge; it does NOT make the composition verifier sound.
//! The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
//! `holder_pok` inherits that — a passing PoK is NOT, under an adversarial prover, a
//! guarantee the holder relation holds, and there is NO external accredited-
//! cryptographer sign-off (sq-qhy4 pending). Research-grade, opt-in. No soundness /
//! ZK-privacy claim is made or implied by any test here.
//!
//! ## Toolchain
//! The serde + no-toolchain structural tests run WITHOUT nargo/bb. The full
//! prove→verify→tamper→invalid-witness round-trips are `#[ignore]` + toolchain-gated
//! (skip cleanly when nargo/bb are absent), mirroring `family_members_pzet.rs` /
//! the `holder_pop_valid_verifies_end_to_end` e2e test.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{
    holder_key_digest, public_key_to_hex, PublicKey, SecretKey, SignatureScheme,
};
use sparq_zk_compose::build::{build_scan, Pattern, Slot};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::holder::holder_pok_witness;
use sparq_zk_compose::manifest::{
    AttestedHolderBinding, AttestedStatusRef, BindingMode, CircuitId, CommitmentAttestation,
    EntailmentRegime, FieldHex, HolderPokProof, ProofInputs, ProofManifest, RevocationStatus,
    StatusListSnapshot, SubProof,
};
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    encode_artifacts, verify_manifest, CheckError, EntailmentPolicy, HolderBindingPolicy,
    HolderRegistry, InMemorySeenNonces, KeySet, RevocationPolicy, VerifierNonce,
};

// --- fixtures (mirror the e2e / holder_pop_forge plumbing) ----------------

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const CHALLENGE_HEX: &str = "0x2a";
const SALT_BYTE: u8 = 9;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}
fn fixture_salt() -> Fr {
    salt_from_bytes(&[SALT_BYTE; 32])
}
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
fn fresh_policy() -> RevocationPolicy {
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot(false))
}
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sparq_zk_compose_holderpok_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn fixture_commitment() -> Fr {
    commit_triples(&credential_graph(), fixture_salt())
        .unwrap()
        .commitment
}

/// A SALT-, STATUS- AND HOLDER-bound attestation (the ZKSIG_C4 variant): the issuer
/// signs `commitment_message_with_holder(C(G), salt, status_ref, holder_key_digest(hpk))`,
/// binding holder `hpk` into THIS credential. `disclose_key` selects the clear tier.
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

fn holder_pop(holder_sk: &SecretKey) -> String {
    let challenge_fr = FieldHex(CHALLENGE_HEX.into()).to_field().unwrap();
    holder_sk.sign_holder_pop(&challenge_fr)
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

// =========================================================================
// NO-TOOLCHAIN: serde + new manifest field
// =========================================================================

/// `HolderPokProof` round-trips through serde, and a manifest with NO
/// `holder_pok_proofs` key parses (serde default), so legacy manifests stay valid.
#[test]
fn holder_pok_proof_serde_round_trip() {
    let pok = HolderPokProof {
        commitment: FieldHex("0xc0ffee".into()),
        proof_hex: "deadbeef".into(),
    };
    let json = serde_json::to_string(&pok).expect("serializes");
    let back: HolderPokProof = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, pok, "HolderPokProof round-trips");

    // A manifest JSON with NO `holder_pok_proofs` key still parses (serde default).
    // `BindingMode` is internally tagged `{ "mode": "challenge", ... }`.
    let m_json = serde_json::json!({
        "query": "SELECT * WHERE { ?s ?p ?o }",
        "attributions": [],
        "entailment_regime": "simple",
        "binding": { "mode": "challenge", "challenge": "0x2a" },
        "sub_proofs": []
    });
    let m: ProofManifest = serde_json::from_value(m_json).expect("legacy manifest parses");
    assert!(
        m.holder_pok_proofs.is_empty(),
        "holder_pok_proofs defaults to empty for a legacy manifest"
    );
}

// =========================================================================
// NO-TOOLCHAIN: structural binding-edge rejections reachable before any bb
// =========================================================================
//
// `bind_holder_pok` runs after the per-sub-proof bb loop in `verify_manifest`. With
// EMPTY `sub_proofs` the loop is a no-op, so these structural rejections fire with
// NO nargo/bb. A HolderPokProof over a commitment no scan references => dangling.

/// A `HolderPop` manifest carrying a `HolderPokProof` over a commitment that NO scan
/// references (here: no scans at all) is rejected `HolderPokUnreferencedCommitment`
/// — a dangling in-circuit PoK, fail-closed. (No toolchain: empty sub_proofs.)
#[test]
fn holder_pok_unreferenced_commitment_rejected() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT * WHERE {}".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&issuer.public_key())],
        commitment_attestations: vec![],
        attributions: vec![],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::HolderPop {
            challenge: FieldHex(CHALLENGE_HEX.into()),
            holder: public_key_to_hex(&holder.public_key()),
            pop: holder_pop(&holder),
            cryptosuite: SignatureScheme::Poseidon2SchnorrV1
                .cryptosuite_iri()
                .to_string(),
        },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        // NO scans => the PoK's commitment is unreferenced.
        sub_proofs: vec![],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![HolderPokProof {
            commitment: FieldHex::from_field(&fixture_commitment()),
            proof_hex: String::new(),
        }],
        holder_set_proofs: vec![],
    };
    let prover = CircuitProver::from_crate_root();
    match verify_manifest(
        &m,
        &prover,
        &scratch("pok_unreferenced"),
        &trusted_k(&issuer),
        &fresh_policy(),
        &registry,
        &HolderBindingPolicy::allow_bearer().require_in_circuit_pok(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::HolderPokUnreferencedCommitment { .. }) => {}
        other => panic!(
            "a holder PoK over a commitment no scan references must be \
             HolderPokUnreferencedCommitment, got {other:?}"
        ),
    }
}

/// A malformed (non-hex) holder-PoK commitment is rejected `HolderPokMalformedProof`
/// before any bb call. (No toolchain: empty sub_proofs.)
#[test]
fn holder_pok_malformed_commitment_rejected() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT * WHERE {}".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&issuer.public_key())],
        commitment_attestations: vec![],
        attributions: vec![],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::HolderPop {
            challenge: FieldHex(CHALLENGE_HEX.into()),
            holder: public_key_to_hex(&holder.public_key()),
            pop: holder_pop(&holder),
            cryptosuite: SignatureScheme::Poseidon2SchnorrV1
                .cryptosuite_iri()
                .to_string(),
        },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![HolderPokProof {
            // Not a field element.
            commitment: FieldHex("0xnot-a-field".into()),
            proof_hex: String::new(),
        }],
        holder_set_proofs: vec![],
    };
    let prover = CircuitProver::from_crate_root();
    match verify_manifest(
        &m,
        &prover,
        &scratch("pok_malformed"),
        &trusted_k(&issuer),
        &fresh_policy(),
        &registry,
        &HolderBindingPolicy::allow_bearer().require_in_circuit_pok(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::HolderPokMalformedProof) => {}
        other => panic!("a malformed PoK commitment must be HolderPokMalformedProof, got {other:?}"),
    }
}

/// The policy default (no `require_in_circuit_pok`, no `holder_pok_proofs`) leaves
/// the B2 gate a no-op — a HolderPop manifest with no PoK and an empty scan set
/// passes `bind_holder_pok` untouched (it does not reach any B2 error). We assert
/// the result is NOT any HolderPok* error (the clear-key path governs). (No toolchain.)
#[test]
fn holder_pok_absent_and_not_required_is_noop() {
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT * WHERE {}".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&issuer.public_key())],
        commitment_attestations: vec![],
        attributions: vec![],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::HolderPop {
            challenge: FieldHex(CHALLENGE_HEX.into()),
            holder: public_key_to_hex(&holder.public_key()),
            pop: holder_pop(&holder),
            cryptosuite: SignatureScheme::Poseidon2SchnorrV1
                .cryptosuite_iri()
                .to_string(),
        },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![],
        holder_set_proofs: vec![],
    };
    let prover = CircuitProver::from_crate_root();
    // allow_bearer (the back-compat default) + no PoK present + no PoK required: the
    // B2 gate must not raise ANY HolderPok* error.
    match verify_manifest(
        &m,
        &prover,
        &scratch("pok_noop"),
        &trusted_k(&issuer),
        &fresh_policy(),
        &registry,
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::HolderPokUnreferencedCommitment { .. })
        | Err(CheckError::HolderPokBindingMissing { .. })
        | Err(CheckError::HolderPokMissing { .. })
        | Err(CheckError::HolderPokDigestMismatch { .. })
        | Err(CheckError::HolderPokProofRejected { .. })
        | Err(CheckError::HolderPokMalformedProof) => {
            panic!("the B2 gate must be a no-op when no PoK is present or required")
        }
        // Any non-B2 outcome (e.g. an unrelated clear-path Ok/Err) is fine: we only
        // assert the B2 gate did not fire.
        _ => {}
    }
}

// =========================================================================
// TOOLCHAIN-GATED: full prove → verify → tamper → invalid-witness round-trips
// =========================================================================

/// Build a real scan proof over `{ ?s <ex/age> ?o }` and the matching ProofInputs.
fn prove_scan(prover: &CircuitProver, salt: Fr, tag: &str) -> (ProofInputs, String) {
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern).expect("scan builds");
    let challenge = FieldHex(CHALLENGE_HEX.into());
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
    let out = scratch(&format!("{tag}_scan"));
    let art = prover.prove_in(&id, &toml, &out, tag).expect("scan prove succeeds");
    (scan.inputs, encode_artifacts(&art))
}

/// Prove the `holder_pok` member for holder `hsk` against `digest` as the PUBLIC
/// `holder_pk_digest`, returning the encoded proof blob. The honest case sets
/// `digest = holder_key_digest(hsk.public_key())`; an invalid-witness case sets a
/// DIFFERENT digest (the proof is then unprovable — `nargo execute` fails).
fn prove_holder_pok(
    prover: &CircuitProver,
    hsk: &SecretKey,
    challenge: &FieldHex,
    tag: &str,
) -> String {
    let witness = holder_pok_witness(hsk).expect("non-identity holder has a witness");
    let toml = sparq_zk_compose::holder::holder_pok_prover_toml(
        &challenge.to_field().unwrap(),
        &witness,
    );
    let out = scratch(&format!("{tag}_pok"));
    let art = prover
        .prove_in(&CircuitId::HolderPok, &toml, &out, tag)
        .expect("holder_pok prove succeeds");
    encode_artifacts(&art)
}

/// Build a full HolderPop manifest: a real scan proof, an issuer-attested-holder-bound
/// attestation to `holder`, the PoP under `holder`, and (optionally) a `HolderPokProof`.
fn full_manifest(
    scan_inputs: ProofInputs,
    scan_hex: String,
    holder: &SecretKey,
    issuer: &SecretKey,
    salt: Fr,
    pok_proofs: Vec<HolderPokProof>,
) -> ProofManifest {
    let commitment = fixture_commitment();
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&issuer.public_key())],
        commitment_attestations: vec![attest_holder(
            commitment,
            salt,
            &holder.public_key(),
            false, // hidden-key tier: only the digest is attested (no clear hpk)
            issuer,
        )],
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::HolderPop {
            challenge: FieldHex(CHALLENGE_HEX.into()),
            holder: public_key_to_hex(&holder.public_key()),
            pop: holder_pop(holder),
            cryptosuite: SignatureScheme::Poseidon2SchnorrV1
                .cryptosuite_iri()
                .to_string(),
        },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof {
            inputs: scan_inputs,
            proof_hex: scan_hex,
        }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: pok_proofs,
        holder_set_proofs: vec![],
    }
}

fn verify_full(
    m: &ProofManifest,
    prover: &CircuitProver,
    issuer: &SecretKey,
    registry: &HolderRegistry,
    name: &str,
) -> Result<(), CheckError> {
    verify_manifest(
        m,
        prover,
        &scratch(name),
        &trusted_k(issuer),
        &fresh_policy(),
        registry,
        &HolderBindingPolicy::require_binding().require_in_circuit_pok(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
}

/// POSITIVE (end-to-end, the binding edge): a real `holder_pok` proof over the
/// holder the issuer bound, atop a real scan proof, under `require_in_circuit_pok`,
/// verifies. The proven (hidden) holder key is the issuer-attested one — the
/// hidden-key analogue of the clear-key B1 closure. (NOT a soundness claim;
/// sq-qhy4.)
#[test]
#[ignore = "slow: full bb prove of scan + holder_pok members (sq-c2ql)"]
fn holder_pok_valid_binding_verifies_end_to_end() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping sq-c2ql holder PoK happy path");
        return;
    }
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let salt = fixture_salt();
    let prover = CircuitProver::from_crate_root();

    let (scan_inputs, scan_hex) = prove_scan(&prover, salt, "pok_pos");
    let pok_hex = prove_holder_pok(&prover, &holder, &FieldHex(CHALLENGE_HEX.into()), "pok_pos");
    let m = full_manifest(
        scan_inputs,
        scan_hex,
        &holder,
        &issuer,
        salt,
        vec![HolderPokProof {
            commitment: FieldHex::from_field(&fixture_commitment()),
            proof_hex: pok_hex,
        }],
    );
    verify_full(&m, &prover, &issuer, &registry, "pok_pos_verify")
        .expect("a valid issuer-bound holder PoK must verify end-to-end");
}

/// INVALID WITNESS (the binding edge rejection): a `holder_pok` proof for a
/// DIFFERENT holder (B') than the issuer bound (B). The proof's public digest is
/// B''s, but the issuer-attested digest is B's, so the reconstructed public inputs
/// (verifier nonce + B's issuer-signed digest) do NOT byte-match the proof's
/// (nonce + B''s digest) => REJECT `HolderPokDigestMismatch`. This is the
/// A-presents-B closure for the hidden-key tier: a holder who is not the
/// issuer-bound one cannot bind a PoK to the credential.
#[test]
#[ignore = "slow: full bb prove of scan + holder_pok members (sq-c2ql)"]
fn holder_pok_wrong_holder_rejected_digest_mismatch() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping sq-c2ql wrong-holder PoK");
        return;
    }
    let issuer = test_issuer_sk(1);
    let bound_holder = SecretKey::from_seed(777); // issuer bound THIS holder
    let other_holder = SecretKey::from_seed(888); // ...but the PoK is for THIS one
    let registry = HolderRegistry::from_hex_keys([
        public_key_to_hex(&bound_holder.public_key()),
        public_key_to_hex(&other_holder.public_key()),
    ]);
    let salt = fixture_salt();
    let prover = CircuitProver::from_crate_root();

    let (scan_inputs, scan_hex) = prove_scan(&prover, salt, "pok_wrong");
    // A genuinely VALID holder_pok proof, but for the WRONG holder (other_holder):
    // its public digest is holder_key_digest(other_holder), not the issuer-attested
    // holder_key_digest(bound_holder).
    let pok_hex =
        prove_holder_pok(&prover, &other_holder, &FieldHex(CHALLENGE_HEX.into()), "pok_wrong");
    // The presenter is the bound holder (so the B1 clear gate + PoP pass), but the
    // in-circuit PoK proves possession of the wrong key.
    let m = full_manifest(
        scan_inputs,
        scan_hex,
        &bound_holder,
        &issuer,
        salt,
        vec![HolderPokProof {
            commitment: FieldHex::from_field(&fixture_commitment()),
            proof_hex: pok_hex,
        }],
    );
    match verify_full(&m, &prover, &issuer, &registry, "pok_wrong_verify") {
        Err(CheckError::HolderPokDigestMismatch { .. }) => {}
        other => panic!(
            "a holder PoK for a key the issuer did NOT bind must be \
             HolderPokDigestMismatch (the hidden-key A-presents-B closure), got {other:?}"
        ),
    }
}

/// TAMPER: a single flipped byte in the `holder_pok` proof blob is rejected. The
/// public inputs still byte-match (the digest is the issuer-attested one), so this
/// reaches `bb verify`, which rejects the corrupted proof => `HolderPokProofRejected`.
#[test]
#[ignore = "slow: full bb prove of scan + holder_pok members (sq-c2ql)"]
fn holder_pok_tampered_proof_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping sq-c2ql tampered PoK");
        return;
    }
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let salt = fixture_salt();
    let prover = CircuitProver::from_crate_root();

    let (scan_inputs, scan_hex) = prove_scan(&prover, salt, "pok_tamper");
    let pok_hex = prove_holder_pok(&prover, &holder, &FieldHex(CHALLENGE_HEX.into()), "pok_tamper");
    // Flip one hex nibble INSIDE the proof bytes (the blob layout is
    // `len(4) | proof | len(4) | pi | vk`; the verifier ignores the prover vk and
    // recomputes the canonical one, so a tail/vk byte would NOT change the verdict).
    // Hex offset 40 = byte 20, well past the 4-byte proof-length prefix and inside
    // the proof body, so decode + the public-input byte-compare still pass and the
    // corruption is caught by `bb verify`.
    let mut bytes: Vec<u8> = pok_hex.into_bytes();
    let at = 40usize.min(bytes.len() - 1);
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    let tampered = String::from_utf8(bytes).unwrap();
    let m = full_manifest(
        scan_inputs,
        scan_hex,
        &holder,
        &issuer,
        salt,
        vec![HolderPokProof {
            commitment: FieldHex::from_field(&fixture_commitment()),
            proof_hex: tampered,
        }],
    );
    match verify_full(&m, &prover, &issuer, &registry, "pok_tamper_verify") {
        // A tampered tail byte either corrupts the proof (bb rejects =>
        // HolderPokProofRejected) or, if it lands in a length/structure region,
        // fails decode (HolderPokMalformedProof). Both are correct fail-closed
        // rejections; never an accept.
        Err(CheckError::HolderPokProofRejected { .. })
        | Err(CheckError::HolderPokMalformedProof) => {}
        other => panic!("a tampered holder PoK proof must be rejected, got {other:?}"),
    }
}

/// MISSING (mandated PoK): under `require_in_circuit_pok`, a holder-bound credential
/// presented with NO `HolderPokProof` is rejected `HolderPokMissing` — the hidden-key
/// possession proof is mandated, never silently waived. (Needs a real scan proof to
/// populate the covering map, hence toolchain-gated.)
#[test]
#[ignore = "slow: full bb prove of a scan member (sq-c2ql)"]
fn holder_pok_required_but_absent_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping sq-c2ql mandated-PoK-absent");
        return;
    }
    let issuer = test_issuer_sk(1);
    let holder = SecretKey::from_seed(777);
    let registry = HolderRegistry::from_hex_keys([public_key_to_hex(&holder.public_key())]);
    let salt = fixture_salt();
    let prover = CircuitProver::from_crate_root();

    let (scan_inputs, scan_hex) = prove_scan(&prover, salt, "pok_missing");
    // Holder-bound credential, valid presentation + PoP, but NO HolderPokProof.
    let m = full_manifest(scan_inputs, scan_hex, &holder, &issuer, salt, vec![]);
    match verify_full(&m, &prover, &issuer, &registry, "pok_missing_verify") {
        Err(CheckError::HolderPokMissing { .. }) => {}
        other => panic!(
            "a holder-bound credential with no in-circuit PoK under require_in_circuit_pok \
             must be HolderPokMissing, got {other:?}"
        ),
    }
}
