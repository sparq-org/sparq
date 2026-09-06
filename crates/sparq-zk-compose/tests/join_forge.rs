// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-hlul (step 5 of sq-bwwl, the hidden cross-credential JOIN): the
//! forge-and-verify REGRESSION SUITE for the hidden join. The adversarial
//! complement of `join_gates.rs` (the `bind_joins` structural-gate suite): here
//! we systematically attempt to FORGE a passing join and assert every attempt is
//! REJECTED, AND — the genuinely new capability over `join_gates.rs` — drive a
//! REAL `join_eq` bb proof to forge the CRYPTOGRAPHIC direction (tampered proof,
//! unsatisfiable forged witness, attacker vk, bb-bytes privacy) which the
//! structural-only `join_gates.rs` cannot reach.
//!
//! ## Relationship to `join_gates.rs` (NO duplication — sq-hlul reconciled vs #188)
//! `join_gates.rs` (PR #188) already pins: the structural positive control, the
//! multi-scan second-scan accept + non-answering reject, the commit_a/commit_b
//! mismatch forges, the wrong-scan / wrong-slot-a / wrong-slot-b forges, the
//! dangling-edge / proof-not-join_eq / scan-a-not-scan structural rejects, the
//! N-way `JoinCommitmentChainMismatch` accept+reject pair, AND a full-bb ACCEPT
//! (`full_bb_join_accept_real_proof`, a real 3-proof composed manifest verified
//! end-to-end through `verify_manifest`). This suite therefore keeps ONLY the
//! vectors `join_gates.rs` does NOT cover:
//!   - STRUCTURAL: a spurious edge over a CONST column (slot_b is a predicate
//!     constant, not a variable) and a join across two DISTINCT variables — both
//!     `JoinSlotMismatch` shapes the single-slot-flip cases miss; and the privacy
//!     LAYOUT pin (the join value is absent from the public `ProofInputs::JoinEq`).
//!   - CRYPTOGRAPHIC (real bb): tampered-proof → bb reject, unequal-values →
//!     witness unsatisfiable, forged join_commitment → witness unsatisfiable,
//!     attacker/non-canonical vk → bb reject, and the real-proof PRIVACY PIN (the
//!     join value's 32-byte word is absent from the proof's `public_inputs`).
//!
//! ## Two tiers (which test runs where) — load-bearing honesty
//! - **Per-PR (fast, NO toolchain):** the STRUCTURAL forge vectors that
//!   `bind_joins` (inside `prefilter_manifest_structure`) rejects without
//!   `nargo`/`bb`. They build `bind_joins` rejections with empty `proof_hex`.
//! - **Nightly (`#[ignore]`, REAL bb prove, gated on toolchain):** the
//!   CRYPTOGRAPHIC forge vectors over a real `join_eq_na16_nb16` proof. Same
//!   `#[ignore]`/`toolchain_available()` convention as e2e's `full_prove_verify_*`
//!   and `join_gates::full_bb_join_accept_real_proof`. These drive the HOST
//!   proving path (`build_join` + `prover_toml_for`'s JoinEq arm, both landed by
//!   #188 / sq-r2s8) — this suite does NOT re-implement a private TOML renderer.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::encode::{encode_term, salt_from_bytes};
use sparq_zk::field::{field_from_hex_str, field_to_be_bytes_32, Fr};
use sparq_zk::sig::{public_key_to_hex, SecretKey, SignatureScheme};
use sparq_zk_compose::build::{build_join, build_scan, BuiltJoin, BuiltScan, Pattern, Slot};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingMode, CircuitId, CommitmentAttestation, EntailmentRegime, FieldHex,
    JoinEdge, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    prefilter_manifest_structure, CheckError, KeySet, RevocationPolicy,
};

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const CHALLENGE_HEX: &str = "0x2a";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}
fn test_issuer_sk(seed: u64) -> SecretKey {
    SecretKey::from_seed(seed)
}
fn trusted_k(sk: &SecretKey) -> KeySet {
    KeySet::from_hex_keys([public_key_to_hex(&sk.public_key())])
}
fn fixture_snapshot() -> StatusListSnapshot {
    StatusListSnapshot { status_list: STATUS_LIST.to_string(), version: STATUS_VERSION, bits: vec![0u8] }
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

/// A salt + status-bound attestation over `commitment` (the scan-verify-path shape;
/// mirrors `join_gates::attest_full`).
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
        holder: None,
    }
}

// --- Two single-graph credentials sharing the join value `<ex/p>` ------------
//
// Graph A: `<ex/a> <ex/knows> <ex/p>`  (object slot 2 = the join key `<ex/p>`)
// Graph B: `<ex/p> <ex/age> "30"`      (subject slot 0 = the join key `<ex/p>`)
//
// `<ex/p>` is a NAMED node, so `encode_term` is salt-INDEPENDENT (it hashes the
// IRI only): the two graphs' join-slot encodings are GENUINELY equal, so the
// in-circuit `a_val == b_val` is satisfiable for the HONEST manifest. The query's
// shared variable `?p` is at pattern-0 slot 2 and pattern-1 slot 0 => slots (2, 0).

const JOIN_QUERY: &str =
    "SELECT ?a ?p ?o WHERE { ?a <http://ex/knows> ?p . ?p <http://ex/age> ?o }";
const SLOT_A: u32 = 2;
const SLOT_B: u32 = 0;
const SALT_A: [u8; 32] = [31u8; 32];
const SALT_B: [u8; 32] = [32u8; 32];

fn triple_a() -> Triple {
    Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/knows"),
        Term::NamedNode(iri("http://ex/p")),
    )
}
fn triple_b() -> Triple {
    Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/p")),
        iri("http://ex/age"),
        int_lit(30),
    )
}
fn graph_a() -> GraphCommitment {
    commit_triples(&[triple_a()], salt_from_bytes(&SALT_A)).unwrap()
}
fn graph_b() -> GraphCommitment {
    commit_triples(&[triple_b()], salt_from_bytes(&SALT_B)).unwrap()
}

/// The encoding of the shared join value `<ex/p>` (salt-independent, named node).
fn join_value_enc() -> Fr {
    encode_term(&Term::NamedNode(iri("http://ex/p")), &Fr::from(0u64)).unwrap()
}

fn scan_artifact(c: &GraphCommitment, pattern: Pattern) -> BuiltScan {
    build_scan(std::slice::from_ref(c), &pattern).expect("scan builds")
}
fn pattern_a() -> Pattern {
    Pattern { s: Slot::Var, p: Slot::Const(Term::NamedNode(iri("http://ex/knows"))), o: Slot::Var }
}
fn pattern_b() -> Pattern {
    Pattern { s: Slot::Var, p: Slot::Const(Term::NamedNode(iri("http://ex/age"))), o: Slot::Var }
}
fn scan_a_inputs(c: &GraphCommitment) -> ProofInputs {
    scan_artifact(c, pattern_a()).inputs
}
fn scan_b_inputs(c: &GraphCommitment) -> ProofInputs {
    scan_artifact(c, pattern_b()).inputs
}
fn commitment_hex(inputs: &ProofInputs) -> FieldHex {
    match inputs {
        ProofInputs::Scan { commitments, .. } => commitments[0].clone(),
        _ => unreachable!("scan inputs"),
    }
}

/// A `join_eq` sub-proof's public inputs (witness-only — empty `proof_hex`), built
/// directly so the STRUCTURAL forge tests can tamper individual public fields.
///
/// [OPUS-4.8] `join_commitment` is the REAL hiding commitment
/// `h3(DOMAIN, join_value_enc(), blinding())` (via the host `join_value_commitment`),
/// NOT a placeholder. The structural privacy pin asserts this field is the hiding
/// IMAGE of the join value (and not a bare encoding of it); a constant here would
/// make that assertion vacuous. Toolchain-free (no nargo/bb) — just the host hash.
fn join_eq_inputs(commit_a: FieldHex, commit_b: FieldHex, slot_a: u32, slot_b: u32) -> ProofInputs {
    let join_commitment =
        FieldHex::from_field(&sparq_zk::sig::join_value_commitment(&join_value_enc(), &blinding()));
    ProofInputs::JoinEq {
        id: CircuitId::JoinEq { n_a: 16, n_b: 16 },
        commit_a,
        commit_b,
        join_commitment,
        slot_a,
        slot_b,
    }
}

/// The honest cross-scan join manifest (mirrors `join_gates::join_manifest`).
fn join_manifest() -> ProofManifest {
    let (ca, sa) = { let c = graph_a(); (c.commitment, c.salt) };
    let (cb, sb) = { let c = graph_b(); (c.commitment, c.salt) };
    let scan_a = scan_a_inputs(&graph_a());
    let scan_b = scan_b_inputs(&graph_b());
    let commit_a = commitment_hex(&scan_a);
    let commit_b = commitment_hex(&scan_b);
    let join = join_eq_inputs(commit_a, commit_b, SLOT_A, SLOT_B);
    let sk = test_issuer_sk(1);
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: JOIN_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![attest_full(ca, sa, &sk), attest_full(cb, sb, &sk)],
        attributions: vec![vec![0], vec![0]],
        pattern_scans: vec![],
        // [OPUS-4.8] sq-en5dx: `graph_a`/`graph_b` are DISTINCT commitments, so the
        // shared `?p` is a genuine cross-graph join whose non-bnode obligation the Q6
        // gate now requires (keyed on committed-graph identity). Declare it so these
        // slot/edge forge vectors reach `bind_joins` (the gate under test) instead of
        // failing earlier on the missing obligation; the omitted-obligation case is
        // pinned by `join_gates::finding_a_cross_scan_alias_forge_rejected_by_q6`.
        join_obligations: vec![("p".to_string(), 0, 1)],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![
            SubProof { inputs: scan_a, proof_hex: String::new() },
            SubProof { inputs: scan_b, proof_hex: String::new() },
            SubProof { inputs: join, proof_hex: String::new() },
        ],
        binding_edges: vec![],
        join_edges: vec![JoinEdge { scan_a: 0, graph_a: 0, scan_b: 1, graph_b: 0, join_proof: 2 }],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

fn prefilter(m: &ProofManifest) -> Result<(), CheckError> {
    prefilter_manifest_structure(m, &trusted_k(&test_issuer_sk(1)), &fresh_policy()).map(|_| ())
}

// ============================================================================
// TIER 1 — STRUCTURAL forge vectors (per-PR, no toolchain). ONLY the vectors
// join_gates.rs does NOT already pin (the positive control, multi-scan, single-
// slot-flip, commit-mismatch, dangling, and N-way cases all live in join_gates).
// ============================================================================

// --- SPURIOUS / wrong-slot hidden join (anti-A: forged unrelated join) -------

/// Forge: a SPURIOUS join edge whose `join_eq` slots `(0, 1)` correspond to NO
/// shared query variable's positions over the referenced scans (slot 0 of pattern
/// A is `?a`, slot 1 of pattern B is the constant `<ex/age>` — they are neither
/// the same variable nor both variables). `bind_joins` must REJECT
/// (`JoinSlotMismatch`): a prover cannot inject a join over an unrelated column
/// pair. NOVEL vs join_gates' single-slot flips: this targets a CONST slot (the
/// predicate), not just the wrong variable position.
#[test]
fn structural_forge_spurious_edge_unrelated_columns_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { slot_a, slot_b, .. } = &mut m.sub_proofs[2].inputs {
        *slot_a = 0; // ?a in pattern A
        *slot_b = 1; // the predicate CONSTANT <ex/age> in pattern B (not a var)
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!("a spurious join over unrelated columns must be JoinSlotMismatch, got {other:?}"),
    }
}

/// Forge: the join edge's two scans share a variable, but the declared join is on
/// a slot pair where the variables DIFFER (`slot_a = 0 = ?a`, `slot_b = 0 = ?p`:
/// `?a != ?p`). The slot binding requires the SAME variable at both slots =>
/// REJECT (`JoinSlotMismatch`). NOVEL vs join_gates: both slots map to a (distinct)
/// VARIABLE rather than one slot being the right variable's wrong position.
#[test]
fn structural_forge_join_on_distinct_variables_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { slot_a, slot_b, .. } = &mut m.sub_proofs[2].inputs {
        *slot_a = 0; // ?a
        *slot_b = 0; // ?p — a DIFFERENT variable
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!("a join across two DISTINCT variables must be JoinSlotMismatch, got {other:?}"),
    }
}

// --- PRIVACY PIN (structural, over the public-input LAYOUT) ------------------

/// PRIVACY REGRESSION PIN (the hiding property). The `join_eq` public-input
/// LAYOUT (documented as `[challenge, commit_a, commit_b, join_commitment,
/// slot_a, slot_b]` and emitted by `reconstruct_public_inputs`'s JoinEq arm) does
/// NOT carry the join VALUE — only the two graph commitments, the HIDING
/// `join_commitment`, and the two query-bound slots. We pin this at the manifest
/// level: the public `ProofInputs::JoinEq` fields a verifier reconstructs are
/// EXACTLY {commit_a, commit_b, join_commitment, slot_a, slot_b}; the joined term
/// encoding (`<ex/p>`) appears in NONE of the FIELD-valued public inputs. A future
/// change that leaked the value into a public input (e.g. adding an `enc`/`value`
/// public field) would flip this assertion. The real-proof analogue
/// (`bb_join_value_absent_from_public_inputs`, nightly) pins the SAME property at
/// the bb-bytes level.
#[test]
fn structural_privacy_join_value_absent_from_public_inputs() {
    let m = join_manifest();
    let ProofInputs::JoinEq { commit_a, commit_b, join_commitment, .. } =
        &m.sub_proofs[2].inputs
    else {
        unreachable!("join_eq inputs");
    };
    let value = join_value_enc();
    let value_hex = FieldHex::from_field(&value);
    // The join value's canonical-field encoding must not equal ANY field-valued
    // public input. (`join_commitment` is `h3(DOMAIN, value, blinding)` — a hiding
    // image of the value, never the value itself.)
    for (name, public) in [
        ("commit_a", commit_a),
        ("commit_b", commit_b),
        ("join_commitment", join_commitment),
    ] {
        assert_ne!(
            public.to_field(),
            Some(value),
            "PRIVACY REGRESSION: the join value leaked into the public input `{name}`",
        );
    }
    // And the join_commitment is specifically NOT a bare encoding of the value —
    // it is the hiding image, which differs from `value_hex`.
    assert_ne!(
        join_commitment, &value_hex,
        "PRIVACY REGRESSION: join_commitment must be hiding, not a bare value encoding",
    );
    // Sanity: the value really is the shared term (so the pin is meaningful) — the
    // host build_join locates and binds exactly this value.
    let built = honest_built_join();
    let value_word = field_to_be_bytes_32(&value);
    let row_a_join = field_from_hex_str(&built.witness.row_a[SLOT_A as usize].0).unwrap();
    assert_eq!(
        field_to_be_bytes_32(&row_a_join),
        value_word,
        "fixture: join slot A of build_join's witness is <ex/p>",
    );
}

// ============================================================================
// TIER 2 — CRYPTOGRAPHIC forge vectors (nightly, #[ignore], REAL bb prove).
// These drive the HOST proving path (build_join + prover_toml_for's JoinEq arm,
// landed by #188 / sq-r2s8), then assert the CRYPTOGRAPHIC direction. They do
// NOT re-implement a private TOML renderer, and do NOT duplicate join_gates'
// `full_bb_join_accept_real_proof` (which is the manifest-level ACCEPT control).
// ============================================================================

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
    let dir = std::env::temp_dir().join(format!("sparq_zk_join_forge_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const JOIN_ID: CircuitId = CircuitId::JoinEq { n_a: 16, n_b: 16 };
/// A fixed test-only blinder; production draws one per-presentation.
const BLINDING_HEX: &str = "0x1234abcd";

fn blinding() -> Fr {
    field_from_hex_str(BLINDING_HEX).expect("blinder parses")
}
fn challenge() -> FieldHex {
    FieldHex(CHALLENGE_HEX.into())
}

/// The HOST-built honest join (the same `build_join` path `join_gates`'s full-bb
/// accept uses). Single source of the witness for every cryptographic forge.
fn honest_built_join() -> BuiltJoin {
    build_join(&graph_a(), SLOT_A, &graph_b(), SLOT_B, blinding())
        .expect("honest join builds (shared <ex/p>)")
}

/// Render the honest join's `Prover.toml` via the HOST emitter (`prover_toml_for`'s
/// JoinEq arm + `build_join`'s witness) — NOT a private renderer.
fn honest_join_toml(built: &BuiltJoin) -> (CircuitId, String) {
    prover_toml_for(&built.inputs, &challenge(), &[], &[], &[], Some(&built.witness), None)
        .expect("join toml emits with the host witness")
}

/// FORGE — TAMPERED PROOF: flip a byte in a valid `join_eq` proof => bb REJECTS.
#[test]
#[ignore = "nightly: full bb prove of a join_eq member (nargo + bb)"]
fn bb_forge_tampered_join_proof_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping tampered-proof reject");
        return;
    }
    let built = honest_built_join();
    let (id, toml) = honest_join_toml(&built);
    assert_eq!(id, JOIN_ID);
    let prover = CircuitProver::from_crate_root();
    let out = scratch("join_tamper");
    let art = prover.prove_in(&id, &toml, &out, "join_tamper").expect("prove succeeds");
    let vk = prover.canonical_vk(&id, &out.join("cvk")).expect("canonical vk");
    let mut bad = art.proof.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xff;
    let rejected = prover
        .verify_with(&bad, &art.public_inputs, &vk, &out.join("verify_bad"))
        .expect("verify runs");
    assert!(!rejected, "a tampered join proof must be rejected by bb");
}

/// FORGE — UNEQUAL HIDDEN VALUES: claim a join between two graphs whose join-slot
/// values are GENUINELY UNEQUAL. The in-circuit `assert(a_val == b_val)` has no
/// satisfying witness => `nargo execute` produces NO witness (the relation is
/// UNSATISFIABLE). A passing join over a false equality is UNCONSTRUCTIBLE.
///
/// `build_join` itself REFUSES to build a join over two graphs that share no value
/// (it returns `None` — the prover has no honest witness), so to drive a forged
/// `Prover.toml` we splice graph-B'`s (unequal) witness onto graph-A's, mirroring
/// the host layout, and assert `nargo execute` finds no satisfying witness.
#[test]
#[ignore = "nightly: nargo execute of a join_eq member"]
fn bb_forge_unequal_values_witness_unsatisfiable() {
    if !toolchain_available() {
        eprintln!("nargo absent; skipping unequal-values unsat");
        return;
    }
    // build_join on two graphs that share NO value returns None — there is no honest
    // witness. Confirm that first (the host refuses to forge), then drive the circuit
    // with a hand-spliced unequal-value Prover.toml and assert it is unsatisfiable.
    let gb_prime = commit_triples(
        &[Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/q")),
            iri("http://ex/age"),
            int_lit(30),
        )],
        salt_from_bytes(&[44u8; 32]),
    )
    .unwrap();
    assert!(
        build_join(&graph_a(), SLOT_A, &gb_prime, SLOT_B, blinding()).is_none(),
        "build_join must refuse a join over graphs that share NO value at the slots",
    );

    // Splice: take the honest A side (commit_a / enc_a / row_a) but graph-B'`s
    // UNEQUAL side (commit_b' / enc_b' / row_b'), keeping the honest A-bound
    // join_commitment. The in-circuit a_val == b_val then fails.
    let honest = honest_built_join();
    let built_b = build_join(&gb_prime, SLOT_B, &gb_prime, SLOT_B, blinding())
        .expect("self-join of gb' builds (locates its own row)");
    let ProofInputs::JoinEq { commit_a, join_commitment, slot_a, slot_b, id, .. } =
        honest.inputs.clone()
    else {
        unreachable!("join_eq inputs");
    };
    let ProofInputs::JoinEq { commit_a: commit_b_prime, .. } = built_b.inputs.clone() else {
        unreachable!("join_eq inputs");
    };
    let forged_inputs = ProofInputs::JoinEq {
        id,
        commit_a,
        commit_b: commit_b_prime,
        join_commitment,
        slot_a,
        slot_b,
    };
    let mut forged_witness = honest.witness.clone();
    forged_witness.enc_b = built_b.witness.enc_a.clone();
    forged_witness.counts_b = built_b.witness.counts_a;
    forged_witness.row_b = built_b.witness.row_a.clone();
    let (id, toml) = prover_toml_for(
        &forged_inputs,
        &challenge(),
        &[],
        &[],
        &[],
        Some(&forged_witness),
        None,
    )
    .expect("forged join toml emits");

    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("compiles");
    let res = prover.gen_witness_tagged(&id, &toml, "join_unequal");
    assert!(
        res.is_err(),
        "a join over genuinely-unequal hidden values must be UNSATISFIABLE (no witness)",
    );
}

/// FORGE — FORGED JOIN COMMITMENT: the public `join_commitment` does NOT bind the
/// proven join value (it commits to a DIFFERENT value). The in-circuit
/// `assert_eq(join_commitment, join_value_commitment(a_val, blinding))` fails =>
/// witness UNSATISFIABLE. Locks the anti-equivocation binding (design §2.4/§4.3).
#[test]
#[ignore = "nightly: nargo execute of a join_eq member"]
fn bb_forge_join_commitment_not_binding_value_unsatisfiable() {
    if !toolchain_available() {
        eprintln!("nargo absent; skipping forged-commitment unsat");
        return;
    }
    let honest = honest_built_join();
    // Replace the public join_commitment with a hiding commitment to a DIFFERENT
    // value (<ex/OTHER>) under the same blinder. The witness still proves the join
    // over <ex/p>, so the in-circuit recompute no longer matches the public field.
    let wrong_value = encode_term(&Term::NamedNode(iri("http://ex/OTHER")), &Fr::from(0u64)).unwrap();
    let forged_jc = sparq_zk::sig::join_value_commitment(&wrong_value, &blinding());
    let ProofInputs::JoinEq { id, commit_a, commit_b, slot_a, slot_b, .. } = honest.inputs.clone()
    else {
        unreachable!("join_eq inputs");
    };
    let forged_inputs = ProofInputs::JoinEq {
        id,
        commit_a,
        commit_b,
        join_commitment: FieldHex::from_field(&forged_jc),
        slot_a,
        slot_b,
    };
    let (id, toml) = prover_toml_for(
        &forged_inputs,
        &challenge(),
        &[],
        &[],
        &[],
        Some(&honest.witness),
        None,
    )
    .expect("forged-commitment join toml emits");

    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("compiles");
    let res = prover.gen_witness_tagged(&id, &toml, "join_forged_commit");
    assert!(
        res.is_err(),
        "a join_commitment that does not bind the proven value must be UNSATISFIABLE",
    );
}

/// FORGE — ATTACKER VK: verify a valid join proof against a NON-canonical
/// (attacker-chosen) vk => bb REJECTS. Pins audit #2 (the verifier recomputes the
/// canonical vk; a prover-supplied / wrong vk does not verify the proof).
#[test]
#[ignore = "nightly: full bb prove of a join_eq member (nargo + bb)"]
fn bb_forge_wrong_vk_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping wrong-vk reject");
        return;
    }
    let built = honest_built_join();
    let (id, toml) = honest_join_toml(&built);
    let prover = CircuitProver::from_crate_root();
    let out = scratch("join_wrongvk");
    let art = prover.prove_in(&id, &toml, &out, "join_wrongvk").expect("prove succeeds");
    // An ATTACKER vk = the vk of a DIFFERENT circuit member (a scan). Verifying the
    // join proof against it must fail.
    let scan_id = derive_smallest_scan_id();
    let wrong_vk = prover
        .canonical_vk(&scan_id, &out.join("wrong_vk"))
        .expect("a different member's vk");
    let rejected = prover
        .verify_with(&art.proof, &art.public_inputs, &wrong_vk, &out.join("verify_wrongvk"))
        .expect("verify runs");
    assert!(!rejected, "a join proof verified against a non-canonical (attacker) vk must be rejected");
}

/// The smallest compiled scan member id (for the wrong-vk attacker case).
fn derive_smallest_scan_id() -> CircuitId {
    sparq_zk_compose::build::derive_scan_id(1, 1, 1).expect("a smallest scan member exists")
}

/// CRYPTOGRAPHIC PRIVACY PIN: the join VALUE's encoding must be ABSENT from the
/// REAL proof's `public_inputs` bytes. The public inputs are exactly
/// `[challenge, commit_a, commit_b, join_commitment, slot_a, slot_b]` (32-byte
/// big-endian field words); the join value `Enc(<ex/p>)` is a PRIVATE witness and
/// its 32-byte word must not appear among them. Locks the hiding property at the
/// bb-bytes level (the headline privacy win).
#[test]
#[ignore = "nightly: full bb prove of a join_eq member (nargo + bb)"]
fn bb_join_value_absent_from_public_inputs() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping real-proof privacy pin");
        return;
    }
    let built = honest_built_join();
    let (id, toml) = honest_join_toml(&built);
    let prover = CircuitProver::from_crate_root();
    let out = scratch("join_privacy");
    let art = prover.prove_in(&id, &toml, &out, "join_privacy").expect("prove succeeds");
    // The 32-byte big-endian word of the join value must not appear in the public
    // inputs blob. (bb serialises each public field as a 32-byte BE word.) We scan
    // on ALIGNED 32-byte word boundaries — not a sliding `windows(32)` — so a match
    // means the value occupies a genuine public-input WORD, never a spurious hit
    // straddling two adjacent words.
    let value_word = field_to_be_bytes_32(&join_value_enc());
    let present = art
        .public_inputs
        .chunks_exact(32)
        .any(|w32| w32 == value_word.as_slice());
    assert!(
        !present,
        "PRIVACY REGRESSION: the join value's 32-byte field word appears in the bb public inputs",
    );
}
