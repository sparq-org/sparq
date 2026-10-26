// sq-pzet (car-hire family completion, sq-8dx2): round-trip + family-
// completeness tests for the REMAINING circuit-family members wired in this bead —
// the scan (k,n,r)-lattice members that had no compiled package, and the N=64
// hidden-join buckets so a join composes with an n=64 scan.
//
// ## NOT a soundness claim (load-bearing honesty)
// This bead BUILDS circuits; it does NOT make the verifier sound. The composition
// verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2). These
// tests assert only the OPERATIONAL properties of the new members: that they
// compile, that an honest witness is satisfiable (and produces a bb proof that bb
// verifies), and that a dishonest witness is unsatisfiable / a tampered proof is
// rejected by bb. They make NO claim that a passing proof implies the SPARQL
// statement is true under an adversarial prover — that is the (open) verifier
// soundness question, untouched here.
//
// Most cryptographic round-trips are `#[ignore]` (a full bb prove is slow) and
// LIVE (they fail loud without nargo + bb unless the lane declares itself
// toolchain-free), mirroring the existing
// e2e / join_forge full-prove suites. The FAMILY-COMPLETENESS gate
// (`every_derivable_id_has_a_compiled_member`) runs WITHOUT the toolchain — it is
// the regression guard for the silent-unprovability gap this bead closed (a
// derived id with no compiled package, the sq-wto class).

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::{field_from_hex_str, Fr};
use sparq_zk_compose::build::{
    build_join, build_scan, derive_join_eq_id, derive_scan_id, Pattern, Slot, JOIN_EQ_N_BUCKETS,
    SCAN_K_VALUES, SCAN_N_BUCKETS, SCAN_R_BUCKETS,
};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{CircuitId, FieldHex, ProofInputs};
use sparq_zk_compose::toml::prover_toml_for;

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const CHALLENGE_HEX: &str = "0x2a";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}
fn challenge() -> FieldHex {
    FieldHex(CHALLENGE_HEX.into())
}
fn blinding() -> Fr {
    field_from_hex_str("0x1234abcd").expect("blinder parses")
}

mod support;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_zk_family_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// On-disk package directory for a circuit id, relative to `zk/compose/`.
fn member_dir(id: &CircuitId) -> std::path::PathBuf {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .map(|root| root.join("zk").join("compose").join(id.package().unwrap()))
        .expect("workspace layout")
}

// ===========================================================================
// FAMILY-COMPLETENESS GATE (no toolchain) — the regression guard for sq-pzet.
//
// `derive_scan_id` / `derive_join_eq_id` may return ONLY ids whose Noir package
// is compiled. A derived id with no `zk/compose/<package>/` directory is the
// silent-unprovability bug this bead closed (the sq-wto class: an honest input
// derives an id, but the prover has no member to prove it against). This gate
// fails the build if any reachable (k,n,r) / (n_a,n_b) combination lacks a
// package — so a future bucket addition cannot reintroduce the gap.
// ===========================================================================

#[test]
fn every_derivable_scan_id_has_a_compiled_member() {
    let mut missing = Vec::new();
    for &k in SCAN_K_VALUES {
        for &n in SCAN_N_BUCKETS {
            for &r in SCAN_R_BUCKETS {
                // A graph of `n` triples disclosing `r` rows derives exactly
                // `Scan { k, n, r }` (n/r are the smallest buckets >= need).
                let id = derive_scan_id(k, n, r).expect("in-lattice scan id derives");
                assert_eq!(id, CircuitId::Scan { k, n, r });
                if !member_dir(&id).join("Nargo.toml").exists() {
                    missing.push(id.package().unwrap());
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "scan family is INCOMPLETE — derivable id(s) with no compiled package (silent-unprovability gap, sq-pzet/sq-wto class): {missing:?}"
    );
}

#[test]
fn every_derivable_join_id_has_a_compiled_member() {
    let mut missing = Vec::new();
    for &n_a in JOIN_EQ_N_BUCKETS {
        for &n_b in JOIN_EQ_N_BUCKETS {
            let id = derive_join_eq_id(n_a, n_b).expect("in-lattice join id derives");
            assert_eq!(id, CircuitId::JoinEq { n_a, n_b });
            if !member_dir(&id).join("Nargo.toml").exists() {
                missing.push(id.package().unwrap());
            }
        }
    }
    assert!(
        missing.is_empty(),
        "join family is INCOMPLETE — derivable id(s) with no compiled package (sq-pzet): {missing:?}"
    );
}

// ===========================================================================
// Helpers to build graphs that land on the NEW members.
// ===========================================================================

/// A single-graph credential of `n` triples `<ex/sN> <ex/age> "N"` so a
/// `?s <ex/age> ?o` scan discloses all `n` rows (variable s + variable o), letting
/// us steer the disclosed-row bucket `r` (e.g. 5..=8 rows => the r=8 member).
fn age_graph(n: u64, salt_byte: u8) -> GraphCommitment {
    let triples: Vec<Triple> = (0..n)
        .map(|i| {
            Triple::new(
                NamedOrBlankNode::NamedNode(iri(&format!("http://ex/s{i}"))),
                iri("http://ex/age"),
                int_lit(i),
            )
        })
        .collect();
    commit_triples(&triples, salt_from_bytes(&[salt_byte; 32])).unwrap()
}

fn age_pattern() -> Pattern {
    Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    }
}

// ===========================================================================
// SCAN: NEW member `scan_k1_n16_r8` (k=1, 5 rows => r=8 bucket, n=16).
// The smallest pre-existing single-graph member is scan_k1_n16_r4 (<=4 rows);
// disclosing 5 rows derives the r=8 member this bead added.
// ===========================================================================

#[test]
fn build_scan_derives_new_k1_r8_member() {
    // 5 matching rows => row bucket r = 8 (the new member).
    let commit = age_graph(5, 7);
    let scan = build_scan(&[commit], &age_pattern()).expect("scan builds");
    assert_eq!(
        scan.inputs.circuit_id(),
        &CircuitId::Scan { k: 1, n: 16, r: 8 },
        "5 disclosed rows must derive the new scan_k1_n16_r8 member"
    );
}

/// Valid witness => the new scan member's relation is satisfiable (fast: nargo
/// execute only, no proving). Toolchain-gated.
#[test]
fn scan_k1_n16_r8_witness_satisfiable() {
    if !support::require_toolchain("scan_k1_n16_r8_witness_satisfiable", &["nargo", "bb"]) {
        return;
    }
    let commit = age_graph(5, 7);
    let scan = build_scan(&[commit], &age_pattern()).expect("scan builds");
    let (id, toml) = prover_toml_for(
        &scan.inputs,
        &challenge(),
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(id, CircuitId::Scan { k: 1, n: 16, r: 8 });
    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("scan_k1_n16_r8 compiles");
    prover
        .gen_witness_tagged(&id, &toml, "scan_k1_r8")
        .expect("honest scan_k1_n16_r8 witness is satisfiable");
    support::complete_required();
}

/// Invalid witness (a disclosed row NOT present in the committed graph) => the
/// new scan member's relation is UNSATISFIABLE: `nargo execute` yields no witness.
/// Toolchain-gated.
#[test]
fn scan_k1_n16_r8_rejects_fabricated_row() {
    if !support::require_toolchain("scan_k1_n16_r8_rejects_fabricated_row", &["nargo", "bb"]) {
        return;
    }
    let commit = age_graph(5, 7);
    let scan = build_scan(&[commit], &age_pattern()).expect("scan builds");
    // Tamper: replace a disclosed row's subject with a value that is NOT in the
    // committed graph. The in-circuit "row present in graph" check then fails.
    let ProofInputs::Scan {
        id,
        commitments,
        pattern_is_const,
        pattern_const_enc,
        mut rows,
        row_count,
        attribution,
    } = scan.inputs.clone()
    else {
        unreachable!("scan inputs");
    };
    // Forge the first disclosed row's subject to an unseen encoding.
    rows[0][0] = FieldHex("0xdeadbeef".to_string());
    let forged = ProofInputs::Scan {
        id: id.clone(),
        commitments,
        pattern_is_const,
        pattern_const_enc,
        rows,
        row_count,
        attribution,
    };
    let (id, toml) = prover_toml_for(
        &forged,
        &challenge(),
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .unwrap();
    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("compiles");
    let res = prover.gen_witness_tagged(&id, &toml, "scan_k1_r8_forge");
    assert!(
        res.is_err(),
        "a disclosed row absent from the committed graph must yield NO satisfying witness"
    );
    support::complete_required();
}

/// Full bb prove -> verify round trip on the new scan member; then a flipped proof
/// byte is rejected. Slow (`#[ignore]`) + toolchain-gated.
#[test]
#[ignore = "slow: full bb prove of the new scan_k1_n16_r8 member"]
fn scan_k1_n16_r8_full_prove_verify_round_trip() {
    if !support::require_toolchain("scan_k1_n16_r8_full_prove_verify_round_trip", &["nargo", "bb"]) {
        return;
    }
    let commit = age_graph(5, 7);
    let scan = build_scan(&[commit], &age_pattern()).expect("scan builds");
    let (id, toml) = prover_toml_for(
        &scan.inputs,
        &challenge(),
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(id, CircuitId::Scan { k: 1, n: 16, r: 8 });
    let prover = CircuitProver::from_crate_root();
    let out = scratch("scan_k1_r8_full");
    let art = prover.prove_in(&id, &toml, &out, "scan_k1_r8_full").expect("prove succeeds");
    assert!(!art.proof.is_empty());
    assert!(
        prover.verify(&art, &out.join("verify")).expect("verify runs"),
        "valid scan_k1_n16_r8 proof must verify"
    );
    // Tamper: flip a proof byte -> bb must reject.
    let mut bad = art.clone();
    let mid = bad.proof.len() / 2;
    bad.proof[mid] ^= 0xff;
    assert!(
        !prover.verify(&bad, &out.join("verify_bad")).expect("verify runs"),
        "tampered scan_k1_n16_r8 proof must be rejected"
    );
    support::complete_required();
}

// ===========================================================================
// JOIN: NEW asymmetric / large member `join_eq_na64_nb64` (a graph with >16
// triples requires the N=64 bucket this bead added). Two credentials sharing
// the join key `<ex/p>`, each holding >16 triples.
// ===========================================================================

/// Graph A: 20 filler triples + one `<ex/a> <ex/knows> <ex/p>` (object slot 2).
fn big_graph_a() -> GraphCommitment {
    let mut triples: Vec<Triple> = (0..20)
        .map(|i| {
            Triple::new(
                NamedOrBlankNode::NamedNode(iri(&format!("http://ex/fa{i}"))),
                iri("http://ex/pad"),
                int_lit(i),
            )
        })
        .collect();
    triples.push(Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/knows"),
        Term::NamedNode(iri("http://ex/p")),
    ));
    commit_triples(&triples, salt_from_bytes(&[71u8; 32])).unwrap()
}

/// Graph B: 20 filler triples + one `<ex/p> <ex/age> "30"` (subject slot 0).
fn big_graph_b() -> GraphCommitment {
    let mut triples: Vec<Triple> = (0..20)
        .map(|i| {
            Triple::new(
                NamedOrBlankNode::NamedNode(iri(&format!("http://ex/fb{i}"))),
                iri("http://ex/pad"),
                int_lit(i),
            )
        })
        .collect();
    triples.push(Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/p")),
        iri("http://ex/age"),
        int_lit(30),
    ));
    commit_triples(&triples, salt_from_bytes(&[72u8; 32])).unwrap()
}

#[test]
fn build_join_derives_new_n64_member() {
    // Both graphs hold 21 triples (> 16) => the N=64 bucket on both sides.
    let built =
        build_join(&big_graph_a(), 2, &big_graph_b(), 0, blinding()).expect("big join builds");
    assert_eq!(
        built.inputs.circuit_id(),
        &CircuitId::JoinEq { n_a: 64, n_b: 64 },
        "two >16-triple graphs must derive the new join_eq_na64_nb64 member"
    );
}

/// Valid witness => the new join member's relation is satisfiable (fast: nargo
/// execute only). Toolchain-gated.
#[test]
fn join_eq_na64_nb64_witness_satisfiable() {
    if !support::require_toolchain("join_eq_na64_nb64_witness_satisfiable", &["nargo", "bb"]) {
        return;
    }
    let built =
        build_join(&big_graph_a(), 2, &big_graph_b(), 0, blinding()).expect("big join builds");
    let (id, toml) =
        prover_toml_for(&built.inputs, &challenge(), &[], &[], &[], Some(&built.witness), None).unwrap();
    assert_eq!(id, CircuitId::JoinEq { n_a: 64, n_b: 64 });
    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("join_eq_na64_nb64 compiles");
    prover
        .gen_witness_tagged(&id, &toml, "join_n64")
        .expect("honest join_eq_na64_nb64 witness is satisfiable");
    support::complete_required();
}

/// Invalid witness (the two graphs share NO value at the chosen slots) => the
/// in-circuit `assert(a_val == b_val)` has no satisfying witness; `build_join`
/// itself refuses to forge one (returns `None`). Toolchain-gated for symmetry
/// with the satisfiable test, but the host-refusal half needs no toolchain.
#[test]
fn join_eq_na64_nb64_rejects_unequal_values() {
    // The host refuses to build a join over graphs sharing no value at the slots.
    // big_graph_a's object-slot key is <ex/p>; big_graph_b's SUBJECT slot of its
    // pad rows is <ex/fbN>, never <ex/p> — so joining slot 2 of A to slot 1
    // (predicate) of B shares no value.
    let none = build_join(&big_graph_a(), 2, &big_graph_b(), 1, blinding());
    assert!(
        none.is_none(),
        "build_join must refuse a join over graphs that share NO value at the chosen slots"
    );
}

/// Full bb prove -> verify round trip on the new join member; then a flipped proof
/// byte is rejected. Slow (`#[ignore]`) + toolchain-gated.
#[test]
#[ignore = "slow: full bb prove of the new join_eq_na64_nb64 member"]
fn join_eq_na64_nb64_full_prove_verify_round_trip() {
    if !support::require_toolchain("join_eq_na64_nb64_full_prove_verify_round_trip", &["nargo", "bb"]) {
        return;
    }
    let built =
        build_join(&big_graph_a(), 2, &big_graph_b(), 0, blinding()).expect("big join builds");
    let (id, toml) =
        prover_toml_for(&built.inputs, &challenge(), &[], &[], &[], Some(&built.witness), None).unwrap();
    assert_eq!(id, CircuitId::JoinEq { n_a: 64, n_b: 64 });
    let prover = CircuitProver::from_crate_root();
    let out = scratch("join_n64_full");
    let art = prover.prove_in(&id, &toml, &out, "join_n64_full").expect("prove succeeds");
    let vk = prover.canonical_vk(&id, &out.join("cvk")).expect("canonical vk");
    assert!(
        prover
            .verify_with(&art.proof, &art.public_inputs, &vk, &out.join("verify"))
            .expect("verify runs"),
        "valid join_eq_na64_nb64 proof must verify"
    );
    // Tamper: flip a proof byte -> bb must reject.
    let mut bad = art.proof.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xff;
    assert!(
        !prover
            .verify_with(&bad, &art.public_inputs, &vk, &out.join("verify_bad"))
            .expect("verify runs"),
        "tampered join_eq_na64_nb64 proof must be rejected"
    );
    support::complete_required();
}
