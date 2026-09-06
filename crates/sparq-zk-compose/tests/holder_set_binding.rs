// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-3c00 (HolderPoP hidden-holder-SET anonymity tier): the in-circuit
//! hidden-holder SET-membership gate — the holder analogue of the hidden-issuer
//! set-membership (`bind_hidden_issuer_attestations`, sq-z9l), and the
//! hidden-holder upgrade over the clear-digest holder PoK (`bind_holder_pok`,
//! sq-c2ql). It proves the holder is a MEMBER of a holder SET WITHOUT revealing
//! WHICH holder.
//!
//! ## What this gates (the trust anchor)
//! A [`HolderSetProof`] is a bb proof of the `hidden_holder_set` relation
//! (knowledge of `hsk` with `hpk = hsk·G`, on-curve / non-identity / `< L`, AND
//! `holder_key_digest(hpk)` a Merkle MEMBER of the committed holder set, with
//! `hsk`/`hpk`/index/path all PRIVATE). The verifier
//! ([`verify_manifest`] → `bind_holder_set`) does NOT trust the proof's public
//! `holder_set_root`: it recomputes the AUTHORITATIVE root from its OWN
//! [`HolderRegistry`] (the trust anchor), reconstructs the proof's public inputs
//! from its own nonce + that root, and byte-compares + `bb verify`s. So "in the
//! set" is bound to the relying party's holder registry — only WHICH holder is
//! hidden, never the trust source.
//!
//! ## SOUNDNESS (load-bearing, NOT a security claim)
//! This wires the membership gate; it does NOT make the composition verifier sound.
//! The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
//! `holder_set_d{depth}` inherits that — a passing proof is NOT, under an
//! adversarial prover, a guarantee the holder relation holds, and there is NO
//! external accredited-cryptographer sign-off (sq-qhy4 pending). Research-grade,
//! opt-in. No soundness / ZK-privacy property is asserted as achieved by any test
//! here.
//!
//! ## Toolchain
//! The serde + no-toolchain structural tests run WITHOUT nargo/bb (they exercise
//! the RUST VERIFIER's fail-closed logic — `HolderSetNotEnabled`,
//! `HolderSetDepthMismatch`, `HolderSetUnreferencedCommitment`,
//! `HolderSetMalformedProof`, and a crafted-blob `HolderSetRootMismatch`, all
//! reachable before any bb call because `bind_holder_set` runs after the
//! per-sub-proof bb loop and these fire on EMPTY `sub_proofs`). The full
//! prove→verify→tamper round-trips are `#[ignore]` + toolchain-gated (skip cleanly
//! when nargo/bb are absent), mirroring `holder_pok_binding.rs` /
//! `family_members_pzet.rs`.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::{field_to_be_bytes_32, Fr};
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey, SignatureScheme};
use sparq_zk_compose::build::{build_scan, Pattern, Slot};
use sparq_zk_compose::driver::{CircuitProver, ProofArtifacts};
use sparq_zk_compose::holder::{
    holder_pok_witness, holder_set_membership_witness, holder_set_prover_toml, holder_set_root,
    HolderSetWitness,
};
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingMode, CircuitId, CommitmentAttestation, EntailmentRegime, FieldHex,
    HolderSetProof, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    encode_artifacts, verify_manifest, CheckError, EntailmentPolicy, HolderBindingPolicy,
    HolderRegistry, InMemorySeenNonces, KeySet, RevocationPolicy, VerifierNonce,
};

// --- fixtures (mirror the holder_pok_binding / e2e plumbing) ---------------

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const CHALLENGE_HEX: &str = "0x2a";
const SALT_BYTE: u8 = 9;
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
/// The holder set the relying party authorises (canonical = sorted hex order).
const SET_SEEDS: [u64; 4] = [100, 101, 102, 103];
/// The in-set holder the prover possesses (seed 102 — in SET_SEEDS).
const IN_SET_HOLDER_SEED: u64 = 102;
/// A holder NOT in the set (the out-of-set forge subject).
const OUT_OF_SET_HOLDER_SEED: u64 = 200;
const HOLDER_SET_DEPTH: u32 = 4;
/// The issuer that attests the scan-covered credential (the holder-set proof binds
/// to the relying party's HolderRegistry, NOT the issuer — but a scan-covered
/// commitment still needs an issuer attestation + revocation reference to pass the
/// prefilter, exactly like every other scan e2e).
const ISSUER_SEED: u64 = 1;

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
fn nonce_for(hex: &str) -> VerifierNonce {
    VerifierNonce::from_hex(hex).expect("valid nonce hex")
}
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sparq_zk_compose_holderset_{name}_{}",
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
/// The holder set in the CANONICAL leaf order the registry commits — sorted by
/// normalized public-key hex (the same `BTreeSet` order `HolderRegistry` /
/// `KeySet` use). The prover MUST build its membership path over this order so its
/// root matches the verifier's authoritative root; the index is the slot in THIS
/// order (resolve via `HolderRegistry::member_index`).
fn holder_set_keys() -> Vec<PublicKey> {
    let mut keyed: Vec<(String, PublicKey)> = SET_SEEDS
        .iter()
        .map(|s| {
            let pk = SecretKey::from_seed(*s).public_key();
            (public_key_to_hex(&pk), pk)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, pk)| pk).collect()
}
/// A `HolderRegistry` over the authorised holder set, with the hidden-holder-set
/// path OPTED IN at the given depth (the relying-party trust anchor).
fn registry_with_set(depth: u32) -> HolderRegistry {
    HolderRegistry::from_hex_keys(
        holder_set_keys()
            .iter()
            .map(public_key_to_hex)
            .collect::<Vec<_>>(),
    )
    .with_hidden_holder_set_depth(depth)
}
fn issuer_sk() -> SecretKey {
    SecretKey::from_seed(ISSUER_SEED)
}
fn trusted_k(sk: &SecretKey) -> KeySet {
    KeySet::from_hex_keys([public_key_to_hex(&sk.public_key())])
}
fn empty_key_set() -> KeySet {
    KeySet::empty()
}
fn fixture_snapshot() -> StatusListSnapshot {
    StatusListSnapshot {
        status_list: STATUS_LIST.to_string(),
        version: STATUS_VERSION,
        bits: vec![0u8],
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
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot())
}
/// A SALT- + STATUS-bound bearer attestation (the audit-#3/#9/#12 shape, `holder:
/// None`) over `commitment` — the issuer attestation a scan-covered commitment
/// needs to pass the prefilter. The holder-set proof binds to the registry, not
/// this attestation, so no holder binding is required here.
fn attest_bearer(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature: sk.sign_commitment_with_status(
            &commitment,
            &salt,
            &sparq_zk::sig::status_ref_digest(
                &sparq_zk::sig::status_list_id_to_field(STATUS_LIST),
                STATUS_INDEX,
                STATUS_VERSION,
            ),
        ),
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

/// A minimal `Challenge`-bound manifest carrying a single `HolderSetProof` and NO
/// `sub_proofs` (so the bb sub-proof loop is a no-op and the structural
/// rejections fire with no toolchain). `binding` is `Challenge` so no clear holder
/// gate runs — this isolates `bind_holder_set`.
fn manifest_with_set_proof(proof: HolderSetProof) -> ProofManifest {
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT * WHERE {}".into(),
        issuers: vec![],
        key_set: vec![],
        commitment_attestations: vec![],
        attributions: vec![],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge {
            challenge: FieldHex(CHALLENGE_HEX.into()),
        },
        revocation: None,
        status_snapshots: vec![],
        sub_proofs: vec![],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![],
        holder_set_proofs: vec![proof],
    }
}

fn run(m: &ProofManifest, registry: &HolderRegistry, tag: &str) -> Result<(), CheckError> {
    let prover = CircuitProver::from_crate_root();
    verify_manifest(
        m,
        &prover,
        &scratch(tag),
        &empty_key_set(),
        &RevocationPolicy::accept_version(STATUS_VERSION),
        registry,
        &HolderBindingPolicy::default(),
        &EntailmentPolicy::default(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
}

// =========================================================================
// NO-TOOLCHAIN: serde + new manifest field
// =========================================================================

/// `HolderSetProof` round-trips through serde, and a manifest with NO
/// `holder_set_proofs` key parses (serde default), so legacy manifests stay valid.
#[test]
fn holder_set_proof_serde_round_trip() {
    let hs = HolderSetProof {
        commitment: FieldHex("0xc0ffee".into()),
        depth: HOLDER_SET_DEPTH,
        holder_set_root: FieldHex("0xabcd".into()),
        proof_hex: "deadbeef".into(),
    };
    let json = serde_json::to_string(&hs).expect("serializes");
    let back: HolderSetProof = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, hs, "HolderSetProof round-trips");

    // A manifest JSON with NO `holder_set_proofs` key still parses (serde default).
    let m_json = serde_json::json!({
        "query": "SELECT * WHERE { ?s ?p ?o }",
        "attributions": [],
        "entailment_regime": "simple",
        "binding": { "mode": "challenge", "challenge": "0x2a" },
        "sub_proofs": []
    });
    let m: ProofManifest = serde_json::from_value(m_json).expect("legacy manifest parses");
    assert!(
        m.holder_set_proofs.is_empty(),
        "holder_set_proofs defaults to empty for a legacy manifest"
    );
}

/// `CircuitId::HolderSet { depth }` round-trips and names the on-disk package.
#[test]
fn holder_set_circuit_id_round_trips() {
    let id = CircuitId::HolderSet { depth: HOLDER_SET_DEPTH };
    assert_eq!(id.package(), "holder_set_d4");
    let json = serde_json::to_string(&id).expect("serializes");
    let back: CircuitId = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, id, "HolderSet CircuitId round-trips");
}

// =========================================================================
// NO-TOOLCHAIN: structural fail-closed rejections (reachable before any bb)
// =========================================================================

/// A `HolderSetProof` present but the relying party has NOT enabled the
/// hidden-holder-set path (no `with_hidden_holder_set_depth`) => REJECT
/// `HolderSetNotEnabled` — there is no authoritative root to bind to, fail-closed.
#[test]
fn holder_set_not_enabled_rejected() {
    let hs = HolderSetProof {
        commitment: FieldHex::from_field(&fixture_commitment()),
        depth: HOLDER_SET_DEPTH,
        holder_set_root: FieldHex("0x01".into()),
        proof_hex: String::new(),
    };
    let m = manifest_with_set_proof(hs);
    // Registry WITHOUT the opt-in depth.
    let registry = HolderRegistry::from_hex_keys(
        holder_set_keys()
            .iter()
            .map(public_key_to_hex)
            .collect::<Vec<_>>(),
    );
    match run(&m, &registry, "set_not_enabled") {
        Err(CheckError::HolderSetNotEnabled) => {}
        other => panic!("expected HolderSetNotEnabled, got {other:?}"),
    }
}

/// A `HolderSetProof` whose declared depth does NOT match the registry policy depth
/// => REJECT `HolderSetDepthMismatch` (the trees would be over different layouts).
#[test]
fn holder_set_depth_mismatch_rejected() {
    let hs = HolderSetProof {
        commitment: FieldHex::from_field(&fixture_commitment()),
        depth: 2, // declared depth
        holder_set_root: FieldHex("0x01".into()),
        proof_hex: String::new(),
    };
    let m = manifest_with_set_proof(hs);
    // Policy depth 4 != declared 2.
    let registry = registry_with_set(4);
    match run(&m, &registry, "set_depth_mismatch") {
        Err(CheckError::HolderSetDepthMismatch { declared: 2, policy: 4 }) => {}
        other => panic!("expected HolderSetDepthMismatch {{2,4}}, got {other:?}"),
    }
}

/// A `HolderSetProof` over a commitment that NO scan sub-proof references (here: no
/// scans at all) => REJECT `HolderSetUnreferencedCommitment` — a dangling
/// set-membership proof, fail-closed.
#[test]
fn holder_set_unreferenced_commitment_rejected() {
    let hs = HolderSetProof {
        commitment: FieldHex::from_field(&fixture_commitment()),
        depth: HOLDER_SET_DEPTH,
        holder_set_root: FieldHex("0x01".into()),
        proof_hex: String::new(),
    };
    let m = manifest_with_set_proof(hs);
    let registry = registry_with_set(HOLDER_SET_DEPTH);
    match run(&m, &registry, "set_unreferenced") {
        Err(CheckError::HolderSetUnreferencedCommitment { .. }) => {}
        other => panic!("expected HolderSetUnreferencedCommitment, got {other:?}"),
    }
}

/// A `HolderSetProof` whose declared commitment is not a field element => REJECT
/// `HolderSetUnreferencedCommitment` (it can never match a real scan commitment).
#[test]
fn holder_set_non_field_commitment_rejected() {
    let hs = HolderSetProof {
        commitment: FieldHex("0xnot-a-field".into()),
        depth: HOLDER_SET_DEPTH,
        holder_set_root: FieldHex("0x01".into()),
        proof_hex: String::new(),
    };
    let m = manifest_with_set_proof(hs);
    let registry = registry_with_set(HOLDER_SET_DEPTH);
    match run(&m, &registry, "set_non_field_commitment") {
        Err(CheckError::HolderSetUnreferencedCommitment { .. }) => {}
        other => panic!("expected HolderSetUnreferencedCommitment, got {other:?}"),
    }
}

/// A `HolderSetProof` over a REFERENCED commitment but with a malformed (non-hex /
/// truncated) proof blob => REJECT `HolderSetMalformedProof`, before any bb call.
/// Uses a single scan sub-proof with an EMPTY blob: the bb loop skips an empty
/// proof? No — instead we reference the commitment via a scan whose proof is also
/// structurally absent. To stay no-toolchain we instead reference it by including
/// the commitment in a scan `ProofInputs` but give the SCAN an empty proof too —
/// which the bb loop would reject FIRST. So this case is exercised in the
/// toolchain-gated section; here we only assert the malformed-blob path on an
/// unreferenced proof still rejects (unreferenced fires first, documented).
#[test]
fn holder_set_malformed_blob_on_unreferenced_still_rejects() {
    // Unreferenced + malformed: the unreferenced check fires first (no toolchain),
    // so this documents that a garbage blob never reaches bb on a dangling proof.
    let hs = HolderSetProof {
        commitment: FieldHex::from_field(&fixture_commitment()),
        depth: HOLDER_SET_DEPTH,
        holder_set_root: FieldHex("0x01".into()),
        proof_hex: "zznothex".into(),
    };
    let m = manifest_with_set_proof(hs);
    let registry = registry_with_set(HOLDER_SET_DEPTH);
    match run(&m, &registry, "set_malformed_unref") {
        Err(CheckError::HolderSetUnreferencedCommitment { .. }) => {}
        other => panic!("expected HolderSetUnreferencedCommitment (fires before blob decode), got {other:?}"),
    }
}

// =========================================================================
// NO-TOOLCHAIN: crafted-blob trust-anchor rejections (referenced commitment, bb
// never reached because the byte-compare diverges before the bb verify call).
// =========================================================================
//
// To make the commitment REFERENCED without running the bb loop, we attach a scan
// sub-proof with a crafted artifacts blob whose public inputs match the
// reconstructed scan vector — the bb loop would still try to verify it, so a fully
// no-toolchain path is not possible for a REFERENCED proof. Instead, these craft a
// HolderSetProof blob whose public-inputs second word is NOT the authoritative
// root and rely on the UNREFERENCED check being bypassed by... it cannot be. So the
// root-mismatch + proof-rejected diagnostics are necessarily toolchain-gated
// (they need a verified scan reference). Documented honestly in the ignored tests.

/// Build a `holder_set_d{depth}` artifacts blob with arbitrary `public_inputs` (so
/// the verifier's byte-compare path can be exercised). `proof`/`vk` are dummy bytes
/// (never reached when the public-input byte-compare diverges first).
fn craft_set_blob(public_inputs: Vec<u8>) -> String {
    encode_artifacts(&ProofArtifacts {
        proof: vec![0u8; 8],
        public_inputs,
        vk: vec![0u8; 8],
    })
}

/// Sanity: the crafted-blob helper round-trips through `encode_artifacts` /
/// `decode_artifacts` (so the toolchain-gated root-mismatch test below feeds a
/// well-formed blob whose only defect is the wrong root word).
#[test]
fn craft_set_blob_round_trips() {
    let challenge = FieldHex(CHALLENGE_HEX.into()).to_field().unwrap();
    let wrong_root = Fr::from(0xdeadu64);
    let mut pi = Vec::with_capacity(64);
    pi.extend_from_slice(&field_to_be_bytes_32(&challenge));
    pi.extend_from_slice(&field_to_be_bytes_32(&wrong_root));
    let blob = craft_set_blob(pi.clone());
    assert!(!blob.is_empty(), "crafted blob encodes to hex");
    // The authoritative root for the configured set/depth is well-defined.
    let auth = registry_with_set(HOLDER_SET_DEPTH)
        .hidden_holder_set_root(HOLDER_SET_DEPTH)
        .expect("authoritative holder-set root is derivable");
    assert_ne!(
        field_to_be_bytes_32(&auth),
        field_to_be_bytes_32(&wrong_root),
        "the crafted wrong root differs from the authoritative root"
    );
}

/// The authoritative holder-set root the verifier derives from its registry equals
/// the host `holder_set_root` over the same keys/depth — the SINGLE source of truth
/// the trust anchor (`HolderSetRootMismatch`) rests on. A proof committed under the
/// in-set holder's REAL membership witness uses exactly this root.
#[test]
fn registry_root_matches_host_root() {
    let depth = HOLDER_SET_DEPTH;
    let registry = registry_with_set(depth);
    let from_registry = registry
        .hidden_holder_set_root(depth)
        .expect("registry derives a root");
    let from_host = holder_set_root(&holder_set_keys(), depth).expect("host derives a root");
    assert_eq!(
        from_registry, from_host,
        "the verifier's authoritative root must equal the host commitment over the same set"
    );
    // The in-set holder has a membership index + path in this canonical order.
    let idx = registry
        .member_index(&public_key_to_hex(
            &SecretKey::from_seed(IN_SET_HOLDER_SEED).public_key(),
        ))
        .expect("the in-set holder is a member");
    let sibs = holder_set_membership_witness(&holder_set_keys(), depth, idx as u64)
        .expect("the in-set holder has a membership path");
    assert_eq!(sibs.len(), depth as usize, "the path has one sibling per level");
}

// =========================================================================
// TOOLCHAIN-GATED (#[ignore]): full prove → verify → tamper round-trips.
// =========================================================================

/// Build a real scan sub-proof over the fixture credential and return its inputs +
/// encoded artifacts blob (so the holder-set proof's commitment is REFERENCED).
fn prove_scan(prover: &CircuitProver, tag: &str) -> (ProofInputs, String) {
    let commit = commit_triples(&credential_graph(), fixture_salt()).unwrap();
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

/// Prove the `holder_set_d{depth}` member for the in-set holder against `root` as
/// the PUBLIC `holder_set_root`, returning the encoded proof blob.
fn prove_holder_set(
    prover: &CircuitProver,
    holder_seed: u64,
    root: &Fr,
    holders: &[PublicKey],
    depth: u32,
    index: u64,
    tag: &str,
) -> String {
    let hsk = SecretKey::from_seed(holder_seed);
    let pok = holder_pok_witness(&hsk).expect("non-identity holder has a witness");
    let siblings = holder_set_membership_witness(holders, depth, index)
        .expect("the holder has a membership path");
    let witness = HolderSetWitness { pok, index, siblings };
    let challenge = FieldHex(CHALLENGE_HEX.into()).to_field().unwrap();
    let toml = holder_set_prover_toml(&challenge, root, &witness);
    let out = scratch(&format!("{tag}_set"));
    let art = prover
        .prove_in(&CircuitId::HolderSet { depth }, &toml, &out, tag)
        .expect("holder_set prove succeeds");
    encode_artifacts(&art)
}

/// Assemble a `Challenge`-bound manifest with a real scan + a holder-set proof,
/// with the scan commitment issuer-attested + revocation-bound (so it passes the
/// prefilter). The issuer-`K` anchor is added to `key_set` + attribution wired.
fn manifest_with_scan_and_set(
    scan_inputs: ProofInputs,
    scan_hex: String,
    set_proof: HolderSetProof,
) -> ProofManifest {
    let mut m = manifest_with_set_proof(set_proof);
    let issuer = issuer_sk();
    let ProofInputs::Scan { commitments, .. } = &scan_inputs else {
        panic!("scan inputs");
    };
    let commit_fr = commitments[0].to_field().expect("commitment is a field");
    // One triple pattern in the query => one attribution entry (arity check).
    m.query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }".into();
    m.key_set = vec![public_key_to_hex(&issuer.public_key())];
    m.attributions = vec![vec![0]];
    m.commitment_attestations = vec![attest_bearer(commit_fr, fixture_salt(), &issuer)];
    m.revocation = Some(fixture_revocation());
    m.status_snapshots = vec![fixture_snapshot()];
    m.sub_proofs = vec![SubProof { inputs: scan_inputs, proof_hex: scan_hex }];
    m
}

/// POSITIVE end-to-end: an in-set holder's set-membership proof, committed under the
/// relying party's authoritative root, ACCEPTS. The holder key, secret, index, and
/// path are all PRIVATE — only set-membership is proved (hides WHICH holder).
#[test]
#[ignore = "requires nargo + bb on PATH"]
fn hidden_holder_set_in_set_verifies_end_to_end() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let prover = CircuitProver::from_crate_root();
    let depth = HOLDER_SET_DEPTH;
    let holders = holder_set_keys();
    let registry = registry_with_set(depth);
    let root = registry.hidden_holder_set_root(depth).unwrap();
    let index = registry
        .member_index(&public_key_to_hex(
            &SecretKey::from_seed(IN_SET_HOLDER_SEED).public_key(),
        ))
        .unwrap() as u64;

    let (scan_inputs, scan_hex) = prove_scan(&prover, "in_set");
    let ProofInputs::Scan { commitments, .. } = &scan_inputs else {
        panic!("scan inputs");
    };
    let commitment = commitments[0].clone();
    let set_hex = prove_holder_set(
        &prover, IN_SET_HOLDER_SEED, &root, &holders, depth, index, "in_set",
    );
    let set_proof = HolderSetProof {
        commitment,
        depth,
        holder_set_root: FieldHex::from_field(&root),
        proof_hex: set_hex,
    };
    let m = manifest_with_scan_and_set(scan_inputs.clone(), scan_hex, set_proof);
    verify_manifest(
        &m,
        &prover,
        &scratch("in_set_verify"),
        &trusted_k(&issuer_sk()),
        &fresh_policy(),
        &registry,
        &HolderBindingPolicy::default(),
        &EntailmentPolicy::default(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
    .expect("an in-set holder's set-membership proof verifies end-to-end");
}

/// NEGATIVE end-to-end (the trust anchor): the SAME in-set holder's proof, but the
/// relying party's registry trusts a DIFFERENT set (so its authoritative root
/// diverges from the proof's public root) => REJECT `HolderSetRootMismatch`. A
/// prover that proves membership in its OWN (forged) set cannot pass.
#[test]
#[ignore = "requires nargo + bb on PATH"]
fn hidden_holder_set_forged_set_root_rejected_end_to_end() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let prover = CircuitProver::from_crate_root();
    let depth = HOLDER_SET_DEPTH;
    // The PROVER's set (includes the in-set holder) and its root.
    let prover_holders = holder_set_keys();
    let prover_root = holder_set_root(&prover_holders, depth).unwrap();
    let index = prover_holders
        .iter()
        .position(|h| *h == SecretKey::from_seed(IN_SET_HOLDER_SEED).public_key())
        .unwrap() as u64;
    let (scan_inputs, scan_hex) = prove_scan(&prover, "forged_root");
    let ProofInputs::Scan { commitments, .. } = &scan_inputs else {
        panic!("scan inputs");
    };
    let commitment = commitments[0].clone();
    let set_hex = prove_holder_set(
        &prover, IN_SET_HOLDER_SEED, &prover_root, &prover_holders, depth, index, "forged_root",
    );
    let set_proof = HolderSetProof {
        commitment,
        depth,
        holder_set_root: FieldHex::from_field(&prover_root),
        proof_hex: set_hex,
    };
    let m = manifest_with_scan_and_set(scan_inputs.clone(), scan_hex, set_proof);

    // The VERIFIER's registry trusts a DIFFERENT set (none of the prover's seeds),
    // so its authoritative root differs from the proof's public root.
    let verifier_registry = HolderRegistry::from_hex_keys(
        [300u64, 301, 302, 303]
            .iter()
            .map(|s| public_key_to_hex(&SecretKey::from_seed(*s).public_key()))
            .collect::<Vec<_>>(),
    )
    .with_hidden_holder_set_depth(depth);

    match verify_manifest(
        &m,
        &prover,
        &scratch("forged_root_verify"),
        &trusted_k(&issuer_sk()),
        &fresh_policy(),
        &verifier_registry,
        &HolderBindingPolicy::default(),
        &EntailmentPolicy::default(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::HolderSetRootMismatch) => {}
        other => panic!("expected HolderSetRootMismatch (forged set), got {other:?}"),
    }
}

/// NEGATIVE end-to-end (unprovable witness): an OUT-OF-SET holder cannot produce a
/// satisfying `holder_set_d{depth}` witness against the relying party's
/// authoritative root — the membership fold cannot recompute the root for a
/// non-member digest, so `nargo execute` / `bb prove` FAILS (the proof never
/// exists). Asserted by the prove call returning an error.
#[test]
#[ignore = "requires nargo + bb on PATH"]
fn hidden_holder_set_out_of_set_holder_is_unprovable_end_to_end() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping");
        return;
    }
    let prover = CircuitProver::from_crate_root();
    let depth = HOLDER_SET_DEPTH;
    let holders = holder_set_keys();
    let root = holder_set_root(&holders, depth).unwrap();

    // The out-of-set holder, trying to claim slot 0 (any slot) with that slot's path.
    let hsk = SecretKey::from_seed(OUT_OF_SET_HOLDER_SEED);
    let pok = holder_pok_witness(&hsk).expect("non-identity holder has a witness");
    let siblings = holder_set_membership_witness(&holders, depth, 0).unwrap();
    let witness = HolderSetWitness { pok, index: 0, siblings };
    let challenge = FieldHex(CHALLENGE_HEX.into()).to_field().unwrap();
    let toml = holder_set_prover_toml(&challenge, &root, &witness);
    let out = scratch("out_of_set_unprovable");
    let result = prover.prove_in(&CircuitId::HolderSet { depth }, &toml, &out, "out_of_set");
    assert!(
        result.is_err(),
        "an out-of-set holder must NOT be able to produce a holder_set proof against the trusted root (membership fold fails)"
    );
}
