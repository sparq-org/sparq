// written while Fable 5 unavailable — re-review when Fable returns.
//! sq-sfsi: the `bind_joins` verifier-gate soundness suite (step 4 of sq-bwwl,
//! the hidden cross-credential JOIN). The adversarial complement to the join
//! manifest schema (sq-fi03): construct a join manifest that violates EXACTLY one
//! of `bind_joins`'s enforcement points, then assert the verifier REJECTS it with
//! the right `CheckError`.
//!
//! The `bind_joins` gate enforces THREE properties (see `verifier::bind_joins`):
//! 1. **Commitment-matching (anti-A2):** the `join_eq` proof's public
//!    `commit_a`/`commit_b` must byte-equal the two referenced scans' bound
//!    `commitments[graph_*]` — a join cannot bind rows from unrelated/forged scans.
//! 2. **Canonical VK (anti-A1):** the `join_eq` proof is verified against the
//!    canonical vk in `verify_manifest`'s per-sub-proof loop (audit-#2). Exercised
//!    on the FULL bb path (toolchain-gated, `#[ignore]`).
//! 3. **UnboundJoin / slot binding:** the `join_eq`'s public `slot_a`/`slot_b` must
//!    equal the query-derived slots a variable SHARED across the two answered
//!    patterns occupies — a join over the wrong column, OR a SPURIOUS join over an
//!    unrelated scan pair, is rejected (`JoinSlotMismatch`).
//!
//! ## Disclosed-vs-hidden scope (load-bearing honesty)
//! `bind_joins` rigorously validates every DECLARED hidden `JoinEdge`. It does NOT
//! *demand* a hidden join for every query cross-scan shared variable: such a
//! variable can ALSO be discharged by the DISCLOSED-row path (the existing
//! `join_obligations`/`verify::recheck` non-bnode obligation gate + the disclosed
//! scan rows), which is the default and is verified SEPARATELY. The hidden
//! `JoinEdge` is the OPT-IN privacy alternative. So "drop the hidden edge" is NOT a
//! soundness hole here — it falls back to the disclosed path (which `recheck` still
//! enforces); demanding a hidden join would WRONGLY break disclosed joins. This
//! suite asserts that boundary explicitly (`drop_edge_falls_back_to_disclosed_path`).
//!
//! These structural checks run inside `prefilter_manifest_structure` (the same
//! structural stage `verify_manifest` runs first), so the reject paths — the
//! security-critical direction — run in default CI WITHOUT the nargo/bb toolchain.
//!
//! ## Accept-proof honesty
//! The `join_eq` PROVING path (`prover_toml_for`'s JoinEq arm) is deferred (sq-fi03
//! left it `Err`), so this suite CANNOT generate a real `join_eq` bb proof in-test.
//! The structural ACCEPT (a well-formed join edge passes `bind_joins` — commitments
//! match, slots match, the demanded join is covered) IS exercised, witness-only,
//! through `prefilter_manifest_structure`. A FULL bb accept (real proof + canonical
//! vk + nonce binding) is marked `#[ignore]` pending the join_eq proving path
//! (tracked: see the join-proving bead referenced in the PR). We do NOT claim the
//! bb accept path works until that proof can be produced.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{public_key_to_hex, SecretKey, SignatureScheme};
// sq-en5dx: the Q6 obligation error surfaced by `recheck` through
// `CheckError::Sparqzk` when a cross-scan join omits its non-bnode obligation.
use sparq_zk::verify::VerifyError;
use sparq_zk_compose::build::{build_join, build_scan, Pattern, Slot};
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingMode, CircuitId, CommitmentAttestation, EntailmentRegime, FieldHex,
    JoinEdge, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::verifier::{
    prefilter_manifest_structure, CheckError, KeySet, RevocationPolicy,
};

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const CHALLENGE_HEX: &str = "0x2a";

/// The STRING-CANONICAL commitment method's `zk:scheme` digest — the method the
/// fixtures below commit under, and the one their issuer now ATTESTS.
///
/// The fixtures sign the METHOD-BOUND presentation message (`ZKSIG_M5`) rather
/// than the method-unbound `ZKSIG_C3`, because `MethodPolicy::current` now
/// refuses an unbound attestation. A test whose SUBJECT is legacy unbound
/// material keeps an unbound fixture and states `MethodPolicy::admit_unbound`
/// explicitly instead.
fn string_canonical_scheme() -> Fr {
    sparq_zk::sig::scheme_iri_to_field(
        sparq_zk::commit::CommitmentMethod::StringCanonicalV1.scheme_iri(),
    )
}

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

/// A salt + status-bound attestation over `commitment` (the scan-verify-path shape).
fn attest_full(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: public_key_to_hex(&sk.public_key()),
        signature: sk.sign_presentation_with_status_and_scheme(
            &commitment,
            &salt,
            &status_ref,
            &string_canonical_scheme(),
        ),
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

// --- Two single-graph credentials sharing a join value `<ex/p>` --------------
//
// Graph A: `<ex/a> <ex/knows> <ex/p>`  (object slot 2 = the join key `<ex/p>`)
// Graph B: `<ex/p> <ex/age> "30"`      (subject slot 0 = the join key `<ex/p>`)
//
// Query: `{ ?a <ex/knows> ?p . ?p <ex/age> ?o }` — the shared variable `?p` is at
// pattern-0 slot 2 (object) and pattern-1 slot 0 (subject). The two patterns carry
// distinct predicate constants, so two DISTINCT scans answer them => a cross-scan
// join the gate demands a JoinEdge for. The join SLOTS are (2, 0).

const JOIN_QUERY: &str =
    "SELECT ?a ?p ?o WHERE { ?a <http://ex/knows> ?p . ?p <http://ex/age> ?o }";
const SLOT_A: u32 = 2; // ?p is the OBJECT of `?a <ex/knows> ?p`
const SLOT_B: u32 = 0; // ?p is the SUBJECT of `?p <ex/age> ?o`

fn graph_a() -> GraphCommitment {
    let g = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/knows"),
        Term::NamedNode(iri("http://ex/p")),
    )];
    commit_triples(&g, salt_from_bytes(&[31u8; 32])).unwrap()
}
fn graph_b() -> GraphCommitment {
    let g = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/p")),
        iri("http://ex/age"),
        int_lit(30),
    )];
    commit_triples(&g, salt_from_bytes(&[32u8; 32])).unwrap()
}

/// A SECOND credential answering pattern A (`?a <ex/knows> ?p`), from a different
/// committed graph (distinct salt => distinct commitment). Used to construct a
/// multi-scan-per-pattern manifest: TWO scans legitimately answer pattern A, the
/// same triple pattern satisfied by two different credentials. sq-sfsi.
fn graph_a2() -> GraphCommitment {
    let g = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a2")),
        iri("http://ex/knows"),
        Term::NamedNode(iri("http://ex/p")),
    )];
    commit_triples(&g, salt_from_bytes(&[33u8; 32])).unwrap()
}

fn scan_inputs(c: &GraphCommitment, pattern: Pattern) -> ProofInputs {
    build_scan(std::slice::from_ref(c), &pattern).expect("scan builds").inputs
}

/// The scan over pattern A (`?a <ex/knows> ?p`).
fn scan_a_inputs(c: &GraphCommitment) -> ProofInputs {
    scan_inputs(c, Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/knows"))),
        o: Slot::Var,
    })
}
/// The scan over pattern B (`?p <ex/age> ?o`).
fn scan_b_inputs(c: &GraphCommitment) -> ProofInputs {
    scan_inputs(c, Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    })
}

fn commitment_hex(inputs: &ProofInputs) -> FieldHex {
    match inputs {
        ProofInputs::Scan { commitments, .. } => commitments[0].clone(),
        _ => unreachable!("scan inputs"),
    }
}

/// A `join_eq` sub-proof's inputs (witness-only — empty proof_hex) whose public
/// `commit_a`/`commit_b` are the two scans' commitments and whose slots are
/// `(slot_a, slot_b)`. `join_commitment` is an arbitrary hiding value (the gate
/// never opens it; only the bb stage binds it).
fn join_eq_inputs(commit_a: FieldHex, commit_b: FieldHex, slot_a: u32, slot_b: u32) -> ProofInputs {
    ProofInputs::JoinEq {
        id: CircuitId::JoinEq { n_a: 16, n_b: 16 },
        commit_a,
        commit_b,
        join_commitment: FieldHex("0x0c".to_string()),
        slot_a,
        slot_b,
    }
}

/// Build the honest cross-scan join manifest. sub_proofs = [scan_a, scan_b, join_eq];
/// join_edges = [{scan_a:0, graph_a:0, scan_b:1, graph_b:0, join_proof:2}]. The
/// join_eq's commit_a/commit_b match the two scans; its slots are (SLOT_A, SLOT_B).
/// `tweak` lets a forge mutate exactly one field before verification.
fn join_manifest() -> ProofManifest {
    let (ca, sa) = {
        let c = graph_a();
        (c.commitment, c.salt)
    };
    let (cb, sb) = {
        let c = graph_b();
        (c.commitment, c.salt)
    };
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
        // Two patterns; each draws from its own (single) committed graph. The
        // graphs are DISTINCT commitments, so the shared `?p` is a genuine
        // cross-graph join: sq-en5dx makes the Q6 gate require the non-bnode
        // obligation on `?p` (keyed on committed-graph identity, not the collapsing
        // scan-local `[[0],[0]]` index). Declaring it discharges the disclosed-path
        // obligation; the hidden `join_edge` below then additionally binds the join
        // value in ZK. (Before sq-en5dx the Q6 gate was inert for cross-scan joins,
        // so this vec was empty; the `finding_a_*` negative test pins that fix.)
        attributions: vec![vec![0], vec![0]],
        pattern_scans: vec![],
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

/// Run a manifest through the STRUCTURAL prefilter (the stage `verify_manifest`
/// runs first, where `bind_joins` lives). No bb — so the structural join gates are
/// reached without the toolchain.
fn prefilter(m: &ProofManifest) -> Result<(), CheckError> {
    prefilter_manifest_structure(m, &trusted_k(&test_issuer_sk(1)), &fresh_policy()).map(|_| ())
}

// === ACCEPT (structural, witness-only) ======================================

/// A well-formed join edge — commitments match the referenced scans, slots match
/// the query, and the demanded cross-scan join is covered — passes every
/// structural gate, INCLUDING `bind_joins`. (Witness-only: no bb proof, so the
/// FULL `verify_manifest` would then hit MissingProof; the structural prefilter is
/// the layer `bind_joins` runs in, so passing it proves the gate ACCEPTS honest
/// joins — the gate's completeness.)
#[test]
fn honest_join_passes_structural_bind_joins() {
    let m = join_manifest();
    prefilter(&m).expect("honest cross-scan join must pass the structural bind_joins gate");
}

/// sq-y2wy: `ProofManifest::canonicalize` sorts the edge vectors, and
/// the STRUCTURAL gate (`bind_joins` + the binding-edge stage) still ACCEPTS the
/// canonicalised manifest — proving edge-vector sorting preserves the edges' scan
/// references (each edge is self-contained; the verifier resolves by the edge's
/// own indices, not its position). A no-op on a single-edge manifest's CONTENTS,
/// but it exercises the build → canonicalise → verify round-trip end to end.
#[test]
fn canonicalised_join_manifest_still_passes_structural_bind_joins() {
    let mut m = join_manifest();
    m.canonicalize();
    prefilter(&m)
        .expect("a canonicalised honest join manifest must still pass the structural gate");
}

// === ACCEPT (multi-scan-per-pattern): a pattern answered by TWO scans ========
//
// sq-sfsi soundness fix. A query pattern may LEGITIMATELY be answered by more than
// one scan (the same triple pattern satisfied by two different credentials). The
// old `bind_joins` used `position(..)` (FIRST match), which would (a) REJECT a
// valid join whose edge points at the SECOND scan answering the pattern, or worse
// (b) let scan ordering decide which scan the slot binding validates against. The
// fix binds the slot check to the SPECIFIC `edge.scan_a`/`edge.scan_b`, so the join
// is validated against the referenced scan regardless of `sub_proofs` order.

/// Build a multi-scan-per-pattern manifest. sub_proofs =
/// [scan_a (graph_a), scan_a2 (graph_a2, SAME pattern A), scan_b, join_eq].
/// The join edge references scan_a2 (index 1) — the SECOND scan answering pattern
/// A — for `scan_a`, and scan_b (index 2). The join_eq carries scan_a2's commitment.
fn multi_scan_manifest() -> ProofManifest {
    let (ca, sa) = {
        let c = graph_a();
        (c.commitment, c.salt)
    };
    let (ca2, sa2) = {
        let c = graph_a2();
        (c.commitment, c.salt)
    };
    let (cb, sb) = {
        let c = graph_b();
        (c.commitment, c.salt)
    };
    let scan_a = scan_a_inputs(&graph_a());
    let scan_a2 = scan_a_inputs(&graph_a2());
    let scan_b = scan_b_inputs(&graph_b());
    // The join binds scan_a2 (the SECOND scan answering pattern A) to scan_b.
    let commit_a2 = commitment_hex(&scan_a2);
    let commit_b = commitment_hex(&scan_b);
    let join = join_eq_inputs(commit_a2, commit_b, SLOT_A, SLOT_B);
    let sk = test_issuer_sk(1);
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: JOIN_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(ca, sa, &sk),
            attest_full(ca2, sa2, &sk),
            attest_full(cb, sb, &sk),
        ],
        attributions: vec![vec![0], vec![0]],
        pattern_scans: vec![],
        // Cross-graph `?p` join (distinct commitments) ⇒ sq-en5dx Q6 obligation.
        join_obligations: vec![("p".to_string(), 0, 1)],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![
            SubProof { inputs: scan_a, proof_hex: String::new() },
            SubProof { inputs: scan_a2, proof_hex: String::new() },
            SubProof { inputs: scan_b, proof_hex: String::new() },
            SubProof { inputs: join, proof_hex: String::new() },
        ],
        binding_edges: vec![],
        // scan_a -> sub-proof 1 (scan_a2, the SECOND scan answering pattern A),
        // scan_b -> sub-proof 2, join_proof -> sub-proof 3.
        join_edges: vec![JoinEdge { scan_a: 1, graph_a: 0, scan_b: 2, graph_b: 0, join_proof: 3 }],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// A join edge pointing at the SECOND of two scans answering a pattern passes the
/// structural `bind_joins` gate. With the old first-match `position(..)` this was
/// WRONGLY rejected (`scan_for_pattern(pattern_a)` resolved to scan-0, not the
/// referenced scan-1). The membership-based fix accepts it. sq-sfsi.
#[test]
fn multi_scan_join_edge_points_at_second_scan_passes() {
    let m = multi_scan_manifest();
    prefilter(&m).expect(
        "a join edge referencing the SECOND scan answering a pattern must pass bind_joins \
         (multi-scan-per-pattern is a legitimate configuration)",
    );
}

/// Anti-forgery preserved under multi-scan: re-point the edge's `scan_a` at scan_b
/// (sub-proof 2), which does NOT answer pattern A at slot_a. The slot binding has
/// no pattern answered by the referenced scan with `?p` at SLOT_A => REJECT
/// (`JoinSlotMismatch`). Confirms the fix did not weaken the binding: the edge
/// still must reference a scan that genuinely answers the right pattern at the
/// right slot, not merely SOME scan answering the pattern. sq-sfsi.
#[test]
fn multi_scan_join_edge_points_at_non_answering_scan_rejected() {
    let mut m = multi_scan_manifest();
    // sub-proof 2 (scan_b, pattern B) does not answer pattern A at SLOT_A; but its
    // commitment must still match commit_a or we'd trip the commitment gate first.
    // Point scan_a at scan_b AND set the join_eq's commit_a to scan_b's commitment
    // so we reach (and exercise) the slot gate specifically.
    m.join_edges[0].scan_a = 2;
    let commit_b = match &m.sub_proofs[2].inputs {
        ProofInputs::Scan { commitments, .. } => commitments[0].clone(),
        _ => unreachable!("scan_b inputs"),
    };
    if let ProofInputs::JoinEq { commit_a, .. } = &mut m.sub_proofs[3].inputs {
        *commit_a = commit_b;
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!(
            "an edge whose scan_a does not answer pattern A at slot_a must be \
             JoinSlotMismatch, got {other:?}"
        ),
    }
}

// === REJECT 1: commitment-matching (cross-scan forgery, anti-A2) ============

/// Forge: the join_eq's `commit_a` does NOT equal scan A's bound commitment (a
/// join binding a row from an unrelated/forged scan). `bind_joins` must REJECT
/// (`JoinCommitmentMismatch`).
#[test]
fn forge_join_commit_a_not_matching_scan_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { commit_a, .. } = &mut m.sub_proofs[2].inputs {
        // A different (but well-formed) field — not scan A's attested commitment.
        *commit_a = FieldHex("0x0deadbeef".to_string());
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentMismatch { edge: 0 }) => {}
        other => panic!("cross-scan commit_a forgery must be JoinCommitmentMismatch, got {other:?}"),
    }
}

/// Forge: the join_eq's `commit_b` does NOT equal scan B's bound commitment.
#[test]
fn forge_join_commit_b_not_matching_scan_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { commit_b, .. } = &mut m.sub_proofs[2].inputs {
        *commit_b = FieldHex("0x0c0ffee".to_string());
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentMismatch { edge: 0 }) => {}
        other => panic!("cross-scan commit_b forgery must be JoinCommitmentMismatch, got {other:?}"),
    }
}

/// Forge: re-point `graph_a` at scan B's commitment by swapping the EDGE's scan
/// reference so commit_a (scan A's) no longer matches commitments[graph_a] of the
/// pointed scan. Confirms the gate anchors on the SCAN the edge names, not on a
/// prover-chosen commitment read in isolation.
#[test]
fn forge_join_edge_points_at_wrong_scan_rejected() {
    let mut m = join_manifest();
    // Point scan_a at sub-proof 1 (scan B) while the join_eq still carries scan A's
    // commit_a => commit_a != scan_b.commitments[0].
    m.join_edges[0].scan_a = 1;
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentMismatch { edge: 0 }) => {}
        other => panic!("edge pointed at the wrong scan must be JoinCommitmentMismatch, got {other:?}"),
    }
}

/// Hardening: BOTH declared commitments ABSENT from the presentation — two
/// well-formed field values no attestation in the manifest covers, so the
/// authenticated-method map has no entry for either. Neither equals its endpoint
/// scan's commitment, so the edge is REJECTED; the pair's absence from the map is
/// not a licence to let the declared linkage stand.
#[test]
fn forge_join_both_declared_commitments_absent_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { commit_a, commit_b, .. } = &mut m.sub_proofs[2].inputs {
        *commit_a = FieldHex("0x0dead".to_string());
        *commit_b = FieldHex("0x0beef".to_string());
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentMismatch { edge: 0 }) => {}
        other => panic!(
            "a join declaring two commitments the presentation does not attest \
             must be JoinCommitmentMismatch, got {other:?}"
        ),
    }
}

/// Hardening: both declared commitments replaced by an attested commitment of
/// ANOTHER committed graph in the SAME presentation — same issuer, same
/// authenticated commitment method, both present in the authenticated-method map.
/// Every method-agreement rule a verifier could apply to the declared inputs is
/// satisfied, so only equality with the edge's ENDPOINT scans separates this from
/// the honest edge. REJECT.
#[test]
fn forge_join_same_method_decoy_commitment_pair_rejected() {
    let mut m = join_manifest();
    let scan_b_commitment = commitment_hex(&m.sub_proofs[1].inputs);
    if let ProofInputs::JoinEq { commit_a, commit_b, .. } = &mut m.sub_proofs[2].inputs {
        *commit_a = scan_b_commitment.clone();
        *commit_b = scan_b_commitment;
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentMismatch { edge: 0 }) => {}
        other => panic!(
            "a same-method attested decoy pair must still be \
             JoinCommitmentMismatch, got {other:?}"
        ),
    }
}

// === REJECT 2: slot binding (wrong column, §4.4 / audit-#6 analogue) ========

/// Forge: the join_eq's public `slot_a` is NOT the slot `?p` occupies in pattern A
/// (it proves the equality over the wrong column). `bind_joins` must REJECT
/// (`JoinSlotMismatch`).
#[test]
fn forge_join_wrong_slot_a_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { slot_a, .. } = &mut m.sub_proofs[2].inputs {
        *slot_a = 0; // ?p is at slot 2 (object) of pattern A, not slot 0.
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        // The per-edge slot check fails first (the edge's join_eq slots are no
        // longer the shared variable's query positions).
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!("wrong join slot_a must be JoinSlotMismatch, got {other:?}"),
    }
}

/// Forge: the join_eq's public `slot_b` is the wrong column for pattern B.
#[test]
fn forge_join_wrong_slot_b_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { slot_b, .. } = &mut m.sub_proofs[2].inputs {
        *slot_b = 2; // ?p is at slot 0 (subject) of pattern B, not slot 2.
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!("wrong join slot_b must be JoinSlotMismatch, got {other:?}"),
    }
}

// === SCOPE: dropping the hidden edge falls back to the disclosed path ========

/// Honesty/scope assertion (NOT a forge): DROP the JoinEdge but KEEP the disclosed
/// obligation `join_manifest` declares. `bind_joins` does NOT demand a hidden join
/// for a query cross-scan shared variable — that variable is discharged by the
/// DISCLOSED-row path (`recheck`/`join_obligations`), the opt-in hidden join being
/// an alternative for the join VALUE. Post-sq-en5dx the Q6 obligation on the
/// cross-graph `?p` is GENUINELY required (keyed on committed-graph identity), and
/// the manifest passes because it DECLARES that obligation — so this pins the
/// disclosed-vs-hidden boundary with the gate now actually live: a dropped hidden
/// join is not a soundness hole, it reverts to the disclosed mechanism (whose
/// obligation is declared and verified separately). Contrast `finding_a_*`, which
/// drops BOTH the hidden edge AND the obligation and is rejected by Q6.
///
/// Dropping the edge while KEEPING the join_eq sub-proof is a different thing and
/// is now refused (`JoinProofNotBoundToAnEdge`): that sub-proof's declared
/// `commit_a`/`commit_b` would be bound by nothing. The fallback this test pins is
/// therefore the manifest with the hidden join REMOVED, and both halves are
/// asserted here so the boundary is explicit.
#[test]
fn drop_edge_falls_back_to_disclosed_path() {
    let mut m = join_manifest();
    m.join_edges.clear();
    // The disclosed obligation `join_manifest` declares (`[("p", 0, 1)]`) discharges
    // the cross-graph Q6 requirement; `bind_joins` is now a no-op (no hidden edge).
    assert_eq!(m.join_obligations, vec![("p".to_string(), 0, 1)]);
    // Dropping the edge ALONE leaves the join_eq sub-proof behind with nothing
    // binding its declared commitments, which the endpoint gate now refuses.
    match prefilter(&m) {
        Err(CheckError::JoinProofNotBoundToAnEdge { proof: 2 }) => {}
        other => panic!("an orphaned join_eq sub-proof must be refused, got {other:?}"),
    }
    // The disclosed fallback is the manifest WITHOUT the hidden join at all: it
    // passes every structural gate (it would reach MissingProof in the full bb
    // verify).
    m.sub_proofs.truncate(2);
    prefilter(&m).expect(
        "dropping the hidden join and keeping the declared obligation falls back to the disclosed-row path and passes the structural gate",
    );
}

/// Hardening: the coverage rule of stage 2g′ is not short-cut once an edge
/// exists. An honest covered join BESIDE an uncovered `JoinEq` is refused in the
/// FLAT regime, naming the uncovered sub-proof.
///
/// `drop_edge_falls_back_to_disclosed_path` above covers the case where NO edge is
/// declared at all; this is the case where one is, so a gate that returned `Ok`
/// as soon as any edge had validated would accept it and the extra join's
/// prover-chosen `commit_a`/`commit_b` would be read by nothing. The extended-regime
/// counterpart is `an_uncovered_join_is_refused_in_the_extended_regime` in
/// `src/verifier.rs`.
#[test]
fn an_uncovered_join_beside_a_covered_one_is_refused_in_the_flat_regime() {
    let mut m = join_manifest();
    // The honest join at index 2 is covered by the declared edge. Add a SECOND
    // `join_eq` at index 3, declaring the SAME (honest) commitments and slots, that
    // no edge names — so only the coverage rule separates it from index 2.
    let commit_a = commitment_hex(&m.sub_proofs[0].inputs);
    let commit_b = commitment_hex(&m.sub_proofs[1].inputs);
    m.sub_proofs.push(SubProof {
        inputs: join_eq_inputs(commit_a, commit_b, SLOT_A, SLOT_B),
        proof_hex: String::new(),
    });
    match prefilter(&m) {
        Err(CheckError::JoinProofNotBoundToAnEdge { proof: 3 }) => {}
        other => panic!("an uncovered join beside a covered one must be refused, got {other:?}"),
    }
    // Not vacuous: covering it with its own edge makes the same manifest pass.
    m.join_edges.push(JoinEdge { scan_a: 0, graph_a: 0, scan_b: 1, graph_b: 0, join_proof: 3 });
    prefilter(&m).expect("a second join covered by its own edge passes the structural gate");
}

/// sq-en5dx NEGATIVE regression (Finding A of the sq-1s2.6 composition review,
/// `research/zk-bind-composition-review.md`): construct the exact cross-scan
/// attribution-alias forge and assert the Q6 obligation ITSELF rejects it.
///
/// The forge: two DISTINCT `k=1` scans (`graph_a`, `graph_b` — distinct salts ⇒
/// distinct commitments) answer the two `JOIN_QUERY` patterns sharing `?p`, with the
/// collapsed `attributions = [[0], [0]]`, NO declared non-bnode obligation, and NO
/// hidden `join_edge`. Before sq-en5dx the Q6 gate keyed on the scan-LOCAL index, so
/// both `{0}`s collapsed (`|A_0 ∪ A_1| = 1`), the non-bnode obligation was DROPPED
/// for the cross-scan join, and this manifest passed the whole structural prefilter
/// — the gate was inert, commitment-salt separation the sole live backstop. The
/// namespace fix keys the union on the committed-graph IDENTITY, so the two distinct
/// graphs no longer collapse and the obligation on `?p` is REQUIRED; omitting it ⇒
/// rejection by the Q6 obligation itself (`recheck` → `MissingObligation`, surfaced
/// as `CheckError::Sparqzk`).
///
/// This is rejection by Q6, NOT by salt-separation: the two distinct salts are
/// well-formed, so `bind_issuer_attestations` (a different stage) would ACCEPT them;
/// the rejection here is the missing cross-graph obligation, and it fires
/// regardless of the salts. NON-VACUITY: revert the `global_attributions` namespace
/// fix and `[[0], [0]]` collapses again ⇒ no obligation required ⇒ this manifest
/// passes the structural prefilter ⇒ the `MissingObligation` match below fails and
/// this test fails.
#[test]
fn finding_a_cross_scan_alias_forge_rejected_by_q6() {
    let mut m = join_manifest();
    m.join_edges.clear(); // no hidden join to discharge the cross-graph `?p`
    m.join_obligations.clear(); // and NO disclosed obligation either — the forge
    match prefilter(&m) {
        Err(CheckError::Sparqzk(VerifyError::MissingObligation(edge))) => {
            assert_eq!(edge.variable, "p", "the dropped obligation is on the join variable ?p");
            assert_eq!(
                edge.patterns,
                (0, 1),
                "the obligation is the cross-scan join between pattern 0 (graph A) and pattern 1 (graph B)"
            );
        }
        other => panic!(
            "the Finding-A cross-scan [[0],[0]] alias forge (distinct graphs, no obligation, \
             no hidden join) must be rejected by the Q6 obligation ITSELF \
             (MissingObligation on ?p), not merely by salt-separation; got {other:?}"
        ),
    }
}

// === N-WAY CHAIN: shared-commitment composition (design §2.4) ================
//
// sq-r2s8: a query variable joined across MORE THAN TWO patterns is
// composed from several pairwise `join_eq` sub-proofs. The composition is sound
// only if every pairwise proof binds the SAME hiding `join_commitment` (one
// blinder), so `a_val == b_val` per hop + the shared commitment compose into a
// transitive N-way equality. `bind_joins` now enforces this: edges joining the
// SAME query variable must carry byte-equal `join_commitment`s.

// Graph C: `<ex/p> <ex/city> <ex/london>` — a THIRD credential sharing `<ex/p>` at
// the SUBJECT (slot 0). Joins `?p` across a third pattern, forming a 3-way chain.
fn graph_c() -> GraphCommitment {
    let g = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/p")),
        iri("http://ex/city"),
        Term::NamedNode(iri("http://ex/london")),
    )];
    commit_triples(&g, salt_from_bytes(&[34u8; 32])).unwrap()
}

const CHAIN_QUERY: &str = "SELECT ?a ?p ?o ?c WHERE { \
    ?a <http://ex/knows> ?p . ?p <http://ex/age> ?o . ?p <http://ex/city> ?c }";
const SLOT_C: u32 = 0; // ?p is the SUBJECT of `?p <ex/city> ?c`

/// The scan over pattern C (`?p <ex/city> ?c`).
fn scan_c_inputs(c: &GraphCommitment) -> ProofInputs {
    scan_inputs(c, Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/city"))),
        o: Slot::Var,
    })
}

/// A 3-way join manifest over `?p` shared across patterns A, B, C. Two hidden join
/// edges: (scan_a, scan_b) and (scan_b, scan_c), BOTH joining `?p`. With
/// `shared_commitment = true` both join_eq sub-proofs carry the SAME
/// `join_commitment` (the honest N-way construction); with `false` the second hop
/// carries a DIFFERENT commitment (the chain-forge). sub_proofs =
/// [scan_a, scan_b, scan_c, join_ab, join_bc].
fn chain_manifest(shared_commitment: bool) -> ProofManifest {
    let (ga, gb, gc) = (graph_a(), graph_b(), graph_c());
    let scan_a = scan_a_inputs(&ga);
    let scan_b = scan_b_inputs(&gb);
    let scan_c = scan_c_inputs(&gc);
    let commit_a = commitment_hex(&scan_a);
    let commit_b = commitment_hex(&scan_b);
    let commit_c = commitment_hex(&scan_c);

    // Hop 1: A.object(2) == B.subject(0). Hop 2: B.subject(0) == C.subject(0).
    let mut join_ab = join_eq_inputs(commit_a, commit_b.clone(), SLOT_A, SLOT_B);
    let mut join_bc = join_eq_inputs(commit_b, commit_c, SLOT_B, SLOT_C);
    // Honest N-way: both hops bind the SAME hiding commitment. Forge: hop 2 differs.
    let shared = FieldHex("0x0c".to_string());
    if let ProofInputs::JoinEq { join_commitment, .. } = &mut join_ab {
        *join_commitment = shared.clone();
    }
    if let ProofInputs::JoinEq { join_commitment, .. } = &mut join_bc {
        *join_commitment = if shared_commitment {
            shared.clone()
        } else {
            // Valid but DIFFERENT field element from `shared` (0x0c): the
            // rejection is specifically due to divergent commitments, not a
            // malformed-hex parse failure.
            FieldHex("0xdeadbeef".to_string())
        };
    }

    let sk = test_issuer_sk(1);
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: CHAIN_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(ga.commitment, ga.salt, &sk),
            attest_full(gb.commitment, gb.salt, &sk),
            attest_full(gc.commitment, gc.salt, &sk),
        ],
        attributions: vec![vec![0], vec![0], vec![0]],
        pattern_scans: vec![],
        // `?p` is joined cross-graph across ALL THREE distinct-commitment patterns,
        // so sq-en5dx's Q6 gate requires the non-bnode obligation on every pair.
        join_obligations: vec![
            ("p".to_string(), 0, 1),
            ("p".to_string(), 0, 2),
            ("p".to_string(), 1, 2),
        ],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![
            SubProof { inputs: scan_a, proof_hex: String::new() },
            SubProof { inputs: scan_b, proof_hex: String::new() },
            SubProof { inputs: scan_c, proof_hex: String::new() },
            SubProof { inputs: join_ab, proof_hex: String::new() },
            SubProof { inputs: join_bc, proof_hex: String::new() },
        ],
        binding_edges: vec![],
        join_edges: vec![
            JoinEdge { scan_a: 0, graph_a: 0, scan_b: 1, graph_b: 0, join_proof: 3 },
            JoinEdge { scan_a: 1, graph_a: 0, scan_b: 2, graph_b: 0, join_proof: 4 },
        ],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// A 3-way join chain over `?p` whose two pairwise join_eq sub-proofs bind the SAME
/// hiding commitment passes the structural `bind_joins` gate — the honest N-way
/// composition (design §2.4). sq-r2s8.
#[test]
fn nway_chain_shared_commitment_passes() {
    let m = chain_manifest(true);
    prefilter(&m).expect(
        "an N-way join chain whose pairwise hops bind the SAME join_commitment must pass bind_joins",
    );
}

/// A 3-way join chain over `?p` whose second hop binds a DIFFERENT `join_commitment`
/// is REJECTED (`JoinCommitmentChainMismatch`): the hops may have proved equalities
/// over different values, so the claimed N-way join is unproven. This is the
/// soundness obligation N-way composition adds over independent pairwise joins.
/// sq-r2s8.
#[test]
fn nway_chain_divergent_commitment_rejected() {
    let m = chain_manifest(false);
    match prefilter(&m) {
        Err(CheckError::JoinCommitmentChainMismatch { edge: 1 }) => {}
        other => panic!(
            "an N-way chain with a divergent join_commitment must be \
             JoinCommitmentChainMismatch on edge 1, got {other:?}"
        ),
    }
}

// === REJECT 3: spurious / wrong-slot hidden join (anti-spurious, §4.4) =======

/// Forge: the JoinEdge binds the right scans but its join_eq's slots don't match
/// the shared variable's query positions. The per-edge slot check must REJECT
/// (`JoinSlotMismatch`) — a slot-mismatched / spurious hidden join cannot pass.
#[test]
fn forge_join_edge_present_but_wrong_slots_rejected() {
    let mut m = join_manifest();
    if let ProofInputs::JoinEq { slot_a, slot_b, .. } = &mut m.sub_proofs[2].inputs {
        // Swap the slots: (0, 2) instead of (2, 0) — neither maps to ?p's query
        // positions in (pattern A, pattern B) respectively.
        *slot_a = 0;
        *slot_b = 2;
    } else {
        unreachable!("join_eq inputs");
    }
    match prefilter(&m) {
        Err(CheckError::JoinSlotMismatch { edge: 0 }) => {}
        other => panic!("a slot-mismatched join edge must be JoinSlotMismatch, got {other:?}"),
    }
}

// === REJECT 4: structural malformed edges ===================================

/// Forge: the JoinEdge's `join_proof` index is out of range (dangling).
#[test]
fn forge_join_dangling_join_proof_rejected() {
    let mut m = join_manifest();
    m.join_edges[0].join_proof = 99;
    match prefilter(&m) {
        Err(CheckError::JoinDanglingEdge { edge: 0 }) => {}
        other => panic!("a dangling join_proof index must be JoinDanglingEdge, got {other:?}"),
    }
}

/// Forge: the JoinEdge's `graph_a` index is out of the referenced scan's
/// `commitments` range (dangling committed-graph index).
#[test]
fn forge_join_dangling_graph_index_rejected() {
    let mut m = join_manifest();
    m.join_edges[0].graph_a = 5; // scan A has only commitments[0].
    match prefilter(&m) {
        Err(CheckError::JoinDanglingEdge { edge: 0 }) => {}
        other => panic!("a dangling graph_a index must be JoinDanglingEdge, got {other:?}"),
    }
}

/// Forge: the JoinEdge's `join_proof` points at a SCAN sub-proof (not a join_eq).
/// The kind check must REJECT (`JoinEdgeKindMismatch`).
#[test]
fn forge_join_proof_not_join_eq_rejected() {
    let mut m = join_manifest();
    m.join_edges[0].join_proof = 0; // sub-proof 0 is a scan, not a join_eq.
    match prefilter(&m) {
        Err(CheckError::JoinEdgeKindMismatch { edge: 0 }) => {}
        other => panic!("join_proof pointing at a scan must be JoinEdgeKindMismatch, got {other:?}"),
    }
}

/// Forge: the JoinEdge's `scan_a` points at the join_eq sub-proof (not a scan).
#[test]
fn forge_join_scan_a_not_scan_rejected() {
    let mut m = join_manifest();
    m.join_edges[0].scan_a = 2; // sub-proof 2 is the join_eq, not a scan.
    match prefilter(&m) {
        Err(CheckError::JoinEdgeKindMismatch { edge: 0 }) => {}
        other => panic!("scan_a pointing at a join_eq must be JoinEdgeKindMismatch, got {other:?}"),
    }
}

// === ACCEPT (FULL bb path) — REAL join_eq proof end-to-end ===================
//
// sq-r2s8: the join_eq PROVING path (`build_join` + `prover_toml_for`'s
// JoinEq arm) now produces a real bb proof, so this test — previously
// `#[ignore]`'d as `full_bb_join_accept_deferred` because no real join_eq proof
// could be generated — now generates one and asserts it VERIFIES end-to-end
// through `verify_manifest`: public-input reconstruction (audit #1), CANONICAL VK
// by `CircuitId::JoinEq` (audit #2, enforcement point 2 — proved end-to-end with a
// genuine proof), bb verify, the nonce binding (audit #4), AND the `bind_joins`
// structural gate. The two scan sub-proofs ALSO carry real bb proofs (verify_manifest
// verifies every sub-proof), so this is a faithful 3-proof composed manifest.
//
// TIER: this is the heavy toolchain/nightly lane — THREE real bb proves (two scans +
// the join). It is `#[ignore]`'d so it does NOT run on the per-PR fast path, exactly
// like `full_manifest_prove_verify_scan` in `e2e.rs`. It RUNS in the zk-toolchain /
// nightly lane via `cargo nextest run -p sparq-zk-compose --run-ignored all`
// (or `cargo test -- --ignored`) on a box with `nargo`+`bb`. The SECURITY-CRITICAL
// reject paths above run WITHOUT the toolchain on every PR.

use sparq_zk::field::field_from_hex_str;
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    verify_manifest, encode_artifacts, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
    InMemorySeenNonces, VerifierNonce,
};

mod support;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sparq_zk_compose_join_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nonce_for(hex: &str) -> VerifierNonce {
    VerifierNonce::from_hex(hex).expect("parseable nonce")
}

/// REAL full-bb ACCEPT: a genuine `join_eq` proof (plus the two genuine scan
/// proofs) verifies end-to-end through `verify_manifest`. Replaces the previously
/// `#[ignore]`d-EMPTY `full_bb_join_accept_deferred` with an actual proof. Proves
/// enforcement point 2 (canonical VK, audit #2) end-to-end for the join member.
#[test]
#[ignore = "heavy toolchain/nightly lane: THREE real bb proves (two scans + the join_eq). \
            Runs via `cargo nextest run -p sparq-zk-compose --run-ignored all` (or \
            `cargo test -- --ignored`) on a box with nargo+bb; sq-r2s8."]
fn full_bb_join_accept_real_proof() {
    if !support::require_toolchain("full_bb_join_accept_real_proof", &["nargo", "bb"]) {
        return;
    }

    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();

    // The two committed credentials (same fixtures the structural suite uses): A
    // holds `<ex/a> <ex/knows> <ex/p>` (join key at OBJECT, slot 2); B holds
    // `<ex/p> <ex/age> "30"` (join key at SUBJECT, slot 0).
    let ga = graph_a();
    let gb = graph_b();

    // --- scan A (real proof) ---
    let pattern_a = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/knows"))),
        o: Slot::Var,
    };
    let scan_a = build_scan(std::slice::from_ref(&ga), &pattern_a).expect("scan A builds");
    let (sa_id, sa_toml) = prover_toml_for(
        &scan_a.inputs,
        &challenge,
        &scan_a.witness.counts,
        &scan_a.witness.enc,
        &[],
        None,
        None,
    )
    .expect("scan A toml");
    let sa_art = prover
        .prove_in(&sa_id, &sa_toml, &scratch("scan_a"), "join_scan_a")
        .expect("scan A proves");

    // --- scan B (real proof) ---
    let pattern_b = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan_b = build_scan(std::slice::from_ref(&gb), &pattern_b).expect("scan B builds");
    let (sb_id, sb_toml) = prover_toml_for(
        &scan_b.inputs,
        &challenge,
        &scan_b.witness.counts,
        &scan_b.witness.enc,
        &[],
        None,
        None,
    )
    .expect("scan B toml");
    let sb_art = prover
        .prove_in(&sb_id, &sb_toml, &scratch("scan_b"), "join_scan_b")
        .expect("scan B proves");

    // --- join_eq (real proof) — the headline path under test. ---
    // A fixed (test-only) blinder; production draws it per-presentation.
    let blinding = field_from_hex_str("0x1234abcd").expect("blinder parses");
    let built_join =
        build_join(&ga, SLOT_A, &gb, SLOT_B, blinding).expect("honest join builds (shared <ex/p>)");
    let (j_id, j_toml) = prover_toml_for(
        &built_join.inputs,
        &challenge,
        &[],
        &[],
        &[],
        Some(&built_join.witness),
        None,
    )
    .expect("join toml emits with the witness");
    assert_eq!(j_id, CircuitId::JoinEq { n_a: 16, n_b: 16 });
    let j_art = prover
        .prove_in(&j_id, &j_toml, &scratch("join_eq"), "join_eq")
        .expect("join_eq proves (the relation is satisfiable for a real join)");
    assert!(!j_art.proof.is_empty(), "join_eq produced a non-empty proof");

    // The join's public commit_a/commit_b MUST equal the two scans' commitments
    // (the bind_joins anti-A2 binding) — they do, because build_join read them from
    // the SAME GraphCommitments build_scan committed.
    assert_eq!(commitment_hex(&scan_a.inputs), match &built_join.inputs {
        ProofInputs::JoinEq { commit_a, .. } => commit_a.clone(),
        _ => unreachable!(),
    });

    // --- assemble the full manifest with REAL proofs on every sub-proof. ---
    let sk = test_issuer_sk(1);
    let manifest = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: JOIN_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(ga.commitment, ga.salt, &sk),
            attest_full(gb.commitment, gb.salt, &sk),
        ],
        attributions: vec![vec![0], vec![0]],
        pattern_scans: vec![],
        // Cross-graph `?p` join (distinct commitments) ⇒ sq-en5dx Q6 obligation.
        join_obligations: vec![("p".to_string(), 0, 1)],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: challenge.clone() },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![
            SubProof { inputs: scan_a.inputs, proof_hex: encode_artifacts(&sa_art) },
            SubProof { inputs: scan_b.inputs, proof_hex: encode_artifacts(&sb_art) },
            SubProof { inputs: built_join.inputs, proof_hex: encode_artifacts(&j_art) },
        ],
        binding_edges: vec![],
        join_edges: vec![JoinEdge { scan_a: 0, graph_a: 0, scan_b: 1, graph_b: 0, join_proof: 2 }],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    };
    // The verifier nonce that all three proofs committed (CHALLENGE_HEX) is bound
    // as field 0 of every sub-proof by the audit-#1 reconstruction.
    verify_manifest(
        &manifest,
        &prover,
        &scratch("verify"),
        &trusted_k(&sk),
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
    .expect("a real join_eq proof (canonical VK + pubinput + bb verify + bind_joins) must verify");
}

// === The per-sub-proof member resolution, through `verify_manifest` ==========
//
// `verify_manifest`'s per-sub-proof loop resolves each sub-proof's member from
// the AUTHENTICATED commitment method of the commitments that sub-proof is bound
// to, and selects its verification key from the RESOLVED member. The resolution
// sits between the audit-#1 public-input byte-compare and the canonical-vk
// recomputation, so a refusal there is reached with NO nargo/bb involved — the
// cases below hand-assemble a `join_eq` artifact blob whose public inputs
// byte-match the verifier's own reconstruction and never get as far as a prover
// invocation.
//
// The feature gate matches the resolution's own: it is `dual-leaf`-only, and the
// feature-matrix `sparq-zk-compose (dual-leaf …)` leg runs this crate's whole
// test suite with the feature on, so these cases execute in CI.

/// The `join_eq` member's bb public inputs, in its `main` declaration order:
/// `[challenge, commit_a, commit_b, join_commitment, slot_a, slot_b]`, each a
/// 32-byte big-endian word. Cross-reference `verifier::reconstruct_public_inputs`'s
/// `JoinEq` arm — a divergence makes the byte-compare fail and the case stops
/// BEFORE the resolution, which the control below detects.
#[cfg(feature = "dual-leaf")]
fn join_eq_public_inputs(inputs: &ProofInputs, challenge: &FieldHex) -> Vec<u8> {
    let ProofInputs::JoinEq { commit_a, commit_b, join_commitment, slot_a, slot_b, .. } = inputs
    else {
        unreachable!("join_eq inputs");
    };
    let mut out = Vec::new();
    for f in [challenge, commit_a, commit_b, join_commitment] {
        let v = f.to_field().expect("parseable field");
        out.extend_from_slice(&sparq_zk::field::field_to_be_bytes_32(&v));
    }
    for n in [*slot_a, *slot_b] {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&u64::from(n).to_be_bytes());
        out.extend_from_slice(&word);
    }
    out
}

/// The honest join manifest with the `join_eq` sub-proof FIRST, so
/// `verify_manifest`'s loop reaches it before either scan, carrying a
/// hand-assembled artifact blob whose public inputs byte-match the verifier's
/// reconstruction. `graph_a_unbound` selects whether graph A's attestation attests
/// the string-canonical METHOD (`ZKSIG_M5`) or attests NO method (the legacy
/// `ZKSIG_C3` shape) — the latter makes the join's two ENDPOINTS a mixture of
/// attested and unattested.
#[cfg(feature = "dual-leaf")]
fn join_first_manifest(graph_a_unbound: bool) -> ProofManifest {
    let ga = graph_a();
    let gb = graph_b();
    let scan_a = scan_a_inputs(&ga);
    let scan_b = scan_b_inputs(&gb);
    let join = join_eq_inputs(commitment_hex(&scan_a), commitment_hex(&scan_b), SLOT_A, SLOT_B);
    let challenge = FieldHex(CHALLENGE_HEX.to_string());
    let art = sparq_zk_compose::driver::ProofArtifacts {
        proof: vec![0u8; 8],
        public_inputs: join_eq_public_inputs(&join, &challenge),
        vk: vec![0u8; 8],
    };
    let sk = test_issuer_sk(1);
    let att_a = if graph_a_unbound {
        attest_method_unbound(ga.commitment, ga.salt, &sk)
    } else {
        attest_full(ga.commitment, ga.salt, &sk)
    };
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: JOIN_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![att_a, attest_full(gb.commitment, gb.salt, &sk)],
        attributions: vec![vec![0], vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![("p".to_string(), 0, 1)],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        // The JOIN first: the loop resolves its member before any scan is read.
        sub_proofs: vec![
            SubProof { inputs: join, proof_hex: encode_artifacts(&art) },
            SubProof { inputs: scan_a, proof_hex: String::new() },
            SubProof { inputs: scan_b, proof_hex: String::new() },
        ],
        binding_edges: vec![],
        join_edges: vec![JoinEdge { scan_a: 1, graph_a: 0, scan_b: 2, graph_b: 0, join_proof: 0 }],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![],
        holder_set_proofs: vec![],
    }
}

/// A salt + status-bound attestation that attests NO commitment method — the
/// legacy `ZKSIG_C3` message shape. Accepted only under the written-down
/// [`sparq_zk_compose::MethodPolicy::admit_unbound`] downgrade.
#[cfg(feature = "dual-leaf")]
fn attest_method_unbound(commitment: Fr, salt: Fr, sk: &SecretKey) -> CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    let mut att = attest_full(commitment, salt, sk);
    att.signature = sk.sign_commitment_with_status(&commitment, &salt, &status_ref);
    att
}

/// `verify_manifest` with a prover that is never invoked: every case below is
/// refused inside the loop, before the canonical vk is recomputed.
#[cfg(feature = "dual-leaf")]
fn run_verify(m: &ProofManifest, k: &KeySet) -> Result<(), CheckError> {
    verify_manifest(
        m,
        &CircuitProver::new(std::env::temp_dir()),
        &scratch("method_resolution"),
        k,
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    )
}

/// INTEGRATION: through the public `verify_manifest`, a `join_eq` sub-proof whose
/// two ENDPOINT commitments are a mixture of attested and unattested methods has
/// no single authenticated method to select a member against, and the manifest is
/// REFUSED by name inside the per-sub-proof loop.
///
/// This is the loop's own resolution, not a structural pre-filter gate: the
/// control below shows the SAME manifest with both endpoints attested to one
/// method gets past it, and the `proof` field names the join sub-proof the loop
/// was on. Removing the loop's resolver (selecting the key from the declared id
/// instead) makes this case fall through to the canonical-vk recomputation, so
/// the named refusal never arrives.
#[cfg(feature = "dual-leaf")]
#[test]
fn a_joins_member_is_resolved_in_the_verify_loop_from_its_endpoints_methods() {
    use sparq_zk_compose::MethodPolicy;
    let sk = test_issuer_sk(1);
    // The downgrade is REQUIRED for the unattested endpoint to get past stage 2d
    // at all; it is what makes the mixture reachable here.
    let k = trusted_k(&sk).with_method_policy(MethodPolicy::current().admit_unbound());

    match run_verify(&join_first_manifest(true), &k) {
        Err(CheckError::SubProofMethodUnresolved { proof, reason }) => {
            assert_eq!(proof, "sub_proofs[0]", "the join sub-proof the loop was on");
            assert!(
                reason.contains("different"),
                "the reason must name the disagreement, got {reason:?}"
            );
        }
        other => panic!(
            "a join over a mixture of attested and unattested endpoints must be \
             refused in the verify loop, got {other:?}"
        ),
    }

    // CONTROL: both endpoints attested to the SAME method — the resolution
    // succeeds and the manifest proceeds past the loop's resolver into the
    // canonical-vk recomputation, which this toolchain-free prover cannot do. So
    // the refusal above is the resolution's, and the fixture reaches it.
    match run_verify(&join_first_manifest(false), &k) {
        Err(CheckError::Driver(_)) => {}
        other => panic!(
            "with both endpoints attested to one method the resolution must \
             succeed and the loop must proceed to the canonical vk, got {other:?}"
        ),
    }
}

/// The FILTER query for the two-edge inheritance case: ONE pattern, answered by
/// TWO credentials, whose shared object `<ex/p>` the FILTER is discharged at.
#[cfg(feature = "dual-leaf")]
const FILTER_QUERY: &str = "SELECT ?a ?p WHERE { ?a <http://ex/knows> ?p FILTER(?p > 1) }";

/// A `filter_int` sub-proof's public inputs: `[challenge, operand_enc, op, bound,
/// expected]` as 32-byte big-endian words (`reconstruct_public_inputs`' FilterInt
/// arm).
#[cfg(feature = "dual-leaf")]
fn filter_int_public_inputs(inputs: &ProofInputs, challenge: &FieldHex) -> Vec<u8> {
    let ProofInputs::FilterInt { operand_enc, op, bound, expected, .. } = inputs else {
        unreachable!("filter_int inputs");
    };
    let mut out = Vec::new();
    for f in [challenge, operand_enc] {
        let v = f.to_field().expect("parseable field");
        out.extend_from_slice(&sparq_zk::field::field_to_be_bytes_32(&v));
    }
    for n in [u64::from(op.code()), *bound, u64::from(*expected)] {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&word);
    }
    out
}

/// `[filter_int, scan_a, scan_a2]` for `FILTER_QUERY`: the FILTER is at index 0,
/// so the loop resolves its member first, and it is discharged by TWO binding
/// edges — one from each scan answering the single pattern. `graph_a2_unbound`
/// makes the SECOND edge's source scan the method-unattested one.
#[cfg(feature = "dual-leaf")]
fn filter_two_edge_manifest(graph_a2_unbound: bool) -> ProofManifest {
    let ga = graph_a();
    let ga2 = graph_a2();
    let scan_a = scan_a_inputs(&ga);
    let scan_a2 = scan_a_inputs(&ga2);
    // `<ex/p>` is a NAMED node, so its encoding is salt-INDEPENDENT: the two
    // scans disclose the SAME object encoding, which is what lets one FILTER
    // sub-proof be discharged against both of them.
    let operand = match (&scan_a, &scan_a2) {
        (ProofInputs::Scan { rows: r1, .. }, ProofInputs::Scan { rows: r2, .. }) => {
            assert_eq!(r1[0][2], r2[0][2], "both scans disclose the same object");
            r1[0][2].clone()
        }
        _ => unreachable!("scan inputs"),
    };
    let filter = ProofInputs::FilterInt {
        id: CircuitId::FilterInt { d: 1 },
        operand_enc: operand,
        op: sparq_zk_compose::manifest::FilterOp::Gt,
        bound: 1,
        expected: true,
    };
    let challenge = FieldHex(CHALLENGE_HEX.to_string());
    let art = sparq_zk_compose::driver::ProofArtifacts {
        proof: vec![0u8; 8],
        public_inputs: filter_int_public_inputs(&filter, &challenge),
        vk: vec![0u8; 8],
    };
    let sk = test_issuer_sk(1);
    let att_a2 = if graph_a2_unbound {
        attest_method_unbound(ga2.commitment, ga2.salt, &sk)
    } else {
        attest_full(ga2.commitment, ga2.salt, &sk)
    };
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: FILTER_QUERY.into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![attest_full(ga.commitment, ga.salt, &sk), att_a2],
        attributions: vec![vec![0]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot()],
        sub_proofs: vec![
            SubProof { inputs: filter, proof_hex: encode_artifacts(&art) },
            SubProof { inputs: scan_a, proof_hex: String::new() },
            SubProof { inputs: scan_a2, proof_hex: String::new() },
        ],
        binding_edges: vec![
            sparq_zk_compose::manifest::BindingEdge {
                filter: None, from_proof: 1,
                from_row: 0,
                from_slot: 2,
                to_proof: 0,
            },
            sparq_zk_compose::manifest::BindingEdge {
                filter: None, from_proof: 2,
                from_row: 0,
                from_slot: 2,
                to_proof: 0,
            },
        ],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
        holder_pok_proofs: vec![],
        holder_set_proofs: vec![],
    }
}

/// INTEGRATION: through the public `verify_manifest`, a FILTER discharged by TWO
/// binding edges inherits the method of EVERY edge's source scan, not just the
/// first one's.
///
/// One pattern, two credentials answering it, one FILTER sub-proof discharged
/// against both. The SECOND edge's scan attests no method, the first's attests
/// string-canonical, so there is no single method to select a member against and
/// the manifest is REFUSED. Inheriting from the first edge alone would resolve to
/// string-canonical and proceed, which is what the control below shows happening
/// when the two genuinely agree.
#[cfg(feature = "dual-leaf")]
#[test]
fn a_filter_inherits_the_method_of_every_edges_source_scan() {
    use sparq_zk_compose::MethodPolicy;
    let sk = test_issuer_sk(1);
    let k = trusted_k(&sk).with_method_policy(MethodPolicy::current().admit_unbound());

    match run_verify(&filter_two_edge_manifest(true), &k) {
        Err(CheckError::SubProofMethodUnresolved { proof, .. }) => {
            assert_eq!(proof, "sub_proofs[0]", "the FILTER sub-proof the loop was on");
        }
        other => panic!(
            "a FILTER whose two edges' scans disagree on the method must be \
             refused, got {other:?}"
        ),
    }

    // CONTROL: both edges' scans attested to the SAME method — the resolution
    // succeeds and the loop proceeds to the canonical-vk recomputation, which
    // this toolchain-free prover cannot do.
    match run_verify(&filter_two_edge_manifest(false), &k) {
        Err(CheckError::Driver(_)) => {}
        other => panic!(
            "with both edges' scans attesting one method the resolution must \
             succeed, got {other:?}"
        ),
    }
}

// === The DEFERRAL of a hidden-only commitment's resolution ===================

/// The hidden-issuer key-set depth this fixture's relying party enables.
#[cfg(feature = "dual-leaf")]
const HI_DEPTH: u32 = 4;

/// The METHOD-BOUND presentation message over `(commitment, salt)` and this
/// fixture's status reference — the candidate a hidden-only commitment's declared
/// hidden-issuer `message` selects.
#[cfg(feature = "dual-leaf")]
fn hi_message(commitment: &Fr, salt: &Fr) -> Fr {
    let list_id = sparq_zk::sig::status_list_id_to_field(STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, STATUS_INDEX, STATUS_VERSION);
    sparq_zk::sig::presentation_message_with_status_and_scheme(
        commitment,
        salt,
        &status_ref,
        &string_canonical_scheme(),
    )
}

/// `join_first_manifest`'s shape with graph A covered ONLY by a hidden-issuer
/// attestation (no clear attestation), whose declared `message` selects the
/// STRING-CANONICAL method, while graph B's clear attestation attests NO method.
///
/// The hidden-issuer PROOF is junk: the hidden-issuer gate runs AFTER the
/// per-sub-proof loop, which is exactly what makes this fixture able to observe
/// the loop's behaviour on a hidden-only candidate without a toolchain.
#[cfg(feature = "dual-leaf")]
fn hidden_only_endpoint_manifest() -> (ProofManifest, KeySet) {
    use sparq_zk_compose::MethodPolicy;
    let sk = test_issuer_sk(1);
    let key_set = trusted_k(&sk)
        .with_hidden_issuer_depth(HI_DEPTH)
        .with_method_policy(MethodPolicy::current().admit_unbound());
    let ga = graph_a();
    let gb = graph_b();
    let mut m = join_first_manifest(false);
    // Graph A loses its clear attestation and gains a hidden-issuer one.
    m.commitment_attestations = vec![attest_method_unbound(gb.commitment, gb.salt, &sk)];
    let junk = sparq_zk_compose::driver::ProofArtifacts {
        proof: vec![0u8; 8],
        public_inputs: vec![0u8; 32],
        vk: vec![0u8; 8],
    };
    m.hidden_issuer_attestations = vec![sparq_zk_compose::manifest::HiddenIssuerAttestation {
        commitment: FieldHex::from_field(&ga.commitment),
        depth: HI_DEPTH,
        key_set_root: FieldHex::from_field(
            &key_set.hidden_issuer_root(HI_DEPTH).expect("authoritative root"),
        ),
        message: FieldHex::from_field(&hi_message(&ga.commitment, &ga.salt)),
        salt: Some(FieldHex::from_field(&ga.salt)),
        proof_hex: encode_artifacts(&junk),
    }];
    (m, key_set)
}

/// INTEGRATION: the loop DEFERS a sub-proof bound to a hidden-only commitment
/// rather than reading that commitment's candidate method as authenticated.
///
/// Graph A is hidden-only and its declared message selects the string-canonical
/// method; graph B's clear attestation attests none. Read as authenticated, that
/// is a mixture, and the loop would refuse it on the spot
/// (`SubProofMethodUnresolved` on `sub_proofs[0]`) — reporting a method refusal
/// BEFORE the hidden-issuer gate has byte-bound any proof to that candidate. With
/// the deferral in place the loop reports no method verdict at all and the
/// manifest proceeds, failing instead on the toolchain this fixture does not
/// have.
///
/// That the deferred verdict DOES arrive, after the hidden-issuer gate, needs a
/// real `scan_k2` proof and a real hidden-issuer proof, so it is pinned by the
/// toolchain-gated `e2e::a_deferred_member_resolution_is_refused_after_the_hidden_issuer_gate`.
#[cfg(feature = "dual-leaf")]
#[test]
fn a_hidden_only_commitments_candidate_method_is_not_read_in_the_loop() {
    let (m, k) = hidden_only_endpoint_manifest();
    match run_verify(&m, &k) {
        Err(CheckError::Driver(_)) => {}
        Err(CheckError::SubProofMethodUnresolved { proof, .. }) => panic!(
            "the loop must DEFER a hidden-only commitment's candidate method, not \
             refuse {proof} on it before the hidden-issuer gate has run"
        ),
        other => panic!("expected the loop to defer and proceed, got {other:?}"),
    }
}
