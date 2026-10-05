// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! sq-1gir: the STANDING forge-and-verify REGRESSION MAP — one permanent test
//! per historical audit finding (#1–#12), each constructing the SPECIFIC
//! forgery that finding described and asserting the verifier REJECTS it with the
//! mapped error / reject-path.
//!
//! The findings span mixed severities (per `research/zk-verifier-reaudit.md`:
//! CRITICAL = #1–#5 and #8; HIGH = #6, #7, #9, #10, #11; MEDIUM = #12) — each
//! section header below carries its own severity. The map covers the whole #1–#12
//! set, not only the CRITICALs.
//!
//! This is the 1:1 companion to `research/zk-verifier-reaudit.md`: the re-audit
//! FINDS that each finding is closed; THIS file PINS it closed, so a future
//! refactor of the reconstruction / vk / attribution / query-binding path cannot
//! silently re-open a remediated finding without a red test. `forge_gates.rs`
//! organises the same gates by BINDING (one canonical forge per binding); this
//! file re-organises them by FINDING NUMBER and is the gate on the
//! epic-close claim — every finding #1–#12 has exactly one named test here.
//!
//! ## The 1:1 finding → reject map (from `research/zk-verifier-reaudit.md` §385)
//!
//! | Finding | Forge | Reject path / error | Lane |
//! |---|---|---|---|
//! | #1  | honest proof over a different statement      | `PublicInputMismatch`                              | toolchain (`#[ignore]`) |
//! | #2  | prover-supplied / trivial-circuit vk         | positive control: an honest proof verifies via the verifier-recomputed canonical vk (the prover-supplied vk is dead weight). The negative — a proof against a *different* circuit's vk — is the #11 bb relabel (`ProofRejected`). | toolchain (`#[ignore]`) |
//! | #3  | unsigned / untrusted-key commitment          | `UnattestedCommitment` / `IssuerKeyNotInKeySet`   | default |
//! | #4  | replay / JSON-challenge rebind               | `NonceReplay` / `NonceBindingMismatch`            | default |
//! | #5  | `17>=17` proven for a `>=18` query           | `UnboundFilter` (bound mismatch)                  | default |
//! | #6  | wrong-slot operand / failing row disclosed   | `UnboundFilter` (slot / verdict)                  | default |
//! | #7  | filter proof over a different operand        | `BindingInconsistent` (host) / `PublicInputMismatch` (bb) | default + toolchain |
//! | #8  | `[[0],[0]]` collapse / omitted attribution   | `AttributionUnderDeclared` / `AttributionMalformed` | default |
//! | #9  | salt reuse across two scan-referenced graphs | `SaltReused`                                      | default |
//! | #10 | FILTER-add / constant-swap                   | `UnboundFilter` / `UnboundPattern`                | default |
//! | #11 | n/d/r bucket relabel                         | `CircuitIdMismatch` / bb verify fails vs canonical vk | default + toolchain |
//! | #12 | revoked / stale credential                   | `CredentialRevoked` / `StatusListStale`           | default |
//!
//! ## Why some are `#[ignore]`d
//!
//! `verify_manifest` runs stages 1+2 (structural + crypto-HOST: circuit-id
//! re-derivation, binding-edge consistency, query-correctness binding, the
//! cross-graph attribution gate, issuer signatures, salt-uniqueness, revocation)
//! WITHOUT bb, then stage 3 (the public-input byte-reconstruction + `bb verify`)
//! per sub-proof. Findings whose forgery is defeated by a stage-1/2 gate run in
//! the DEFAULT `cargo test` with no prover toolchain. Findings whose forgery
//! requires a GENUINE (forged) `bb` proof to even be CONSTRUCTED — #1 and #2 (and
//! the proof-bound variants of #7/#11) — are gated behind `#[ignore]` and
//! `toolchain_available()`; they are written so the separate toolchain CI lane
//! (bead **sq-f9tl**) can run them with `-- --ignored`. Each `#[ignore]` carries
//! the reason inline.
//!
//! If ANY default-run forge below were to VERIFY (the verifier ACCEPTS the
//! forgery) that is a real soundness regression — the test asserts REJECT, so it
//! goes red; do NOT weaken it to make it pass.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{public_key_to_hex, SecretKey, SignatureScheme};
use sparq_zk_compose::build::{
    build_filter_int, build_scan, encode_int_literal, Pattern, Slot,
};
use sparq_zk_compose::driver::{CircuitProver, ProofArtifacts};
use sparq_zk_compose::manifest::{
    AttestedStatusRef, BindingEdge, BindingMode, CircuitId, CommitmentAttestation, EntailmentRegime,
    FieldHex, FilterOp, ProofInputs, ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::{
    encode_artifacts, verify_manifest, CheckError, EntailmentPolicy, HolderBindingPolicy, HolderRegistry,
    InMemorySeenNonces, KeySet, RevocationPolicy, VerifierNonce,
};

// === shared fixtures (kept local so this map is a single self-contained file) ==

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STATUS_LIST: &str = "http://ex/status/1";
const STATUS_INDEX: u64 = 3;
const STATUS_VERSION: u64 = 1;
const CHALLENGE_HEX: &str = "0x2a";
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
fn fresh_policy() -> RevocationPolicy {
    RevocationPolicy::accept_version(STATUS_VERSION).with_snapshot(fixture_snapshot(false))
}
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
    let dir = std::env::temp_dir().join(format!(
        "sparq_zk_compose_auditmap_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An honest witness-only (empty `proof_hex`) scan-only manifest over the
/// credential graph's `<ex/age>` pattern, with a valid salt+status-bound
/// attestation so every structural gate passes by default. The structural forges
/// break EXACTLY one gate and reach it WITHOUT bb (they reject before the
/// per-sub-proof stage-3 crypto loop). Honest binding challenge is `CHALLENGE_HEX`.
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

/// Sanity anchor: the honest scan-only manifest passes EVERY structural gate of
/// the sound entry point and only fails on the absent bb proof
/// (`MissingProof`). This proves the structural forges below fail EARLIER, at
/// their targeted gate, not on a missing proof.
#[test]
fn honest_baseline_reaches_missing_proof() {
    let (m, _c, _s) = honest_scan_only_manifest();
    match verify_full(&m, "honest_baseline") {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        other => panic!(
            "honest manifest should pass structural gates then hit MissingProof, got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #3 (CRITICAL) — no issuer-signature / key-set membership check.
//   Forge: unsigned commitment, and a real-but-untrusted-key signature.
//   Reject: UnattestedCommitment / IssuerKeyNotInKeySet. LANE: default.
// =====================================================================

/// #3a — drop the issuer attestation so the scan's commitment is unsigned /
/// prover-invented. The commitment-binding gate REJECTS (`UnattestedCommitment`).
#[test]
fn finding_03_unsigned_commitment_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    m.commitment_attestations.clear();
    match verify_full(&m, "f03_unsigned") {
        Err(CheckError::UnattestedCommitment { proof: 0, .. }) => {}
        other => panic!("#3: unsigned commitment must be UnattestedCommitment, got {other:?}"),
    }
}

/// #3b — a cryptographically VALID signature, but by an issuer key the relying
/// party's EXTERNAL trust anchor `K` does not contain. Even a real signature by
/// an untrusted key is no attestation: REJECT (`IssuerKeyNotInKeySet` or the
/// declared-superset rejection `UntrustedDeclaredKey`).
#[test]
fn finding_03_untrusted_issuer_key_rejected() {
    let (mut m, c, salt) = honest_scan_only_manifest();
    // Issuer 2 signs validly; we even list issuer 2 in the manifest key_set...
    let signer = test_issuer_sk(2);
    m.commitment_attestations = vec![attest_full(c, salt, &signer)];
    m.key_set = vec![public_key_to_hex(&signer.public_key())];
    // ...but verify_full's external K trusts ONLY issuer 1.
    match verify_full(&m, "f03_untrusted_key") {
        Err(CheckError::IssuerKeyNotInKeySet { .. })
        | Err(CheckError::UntrustedDeclaredKey { .. }) => {}
        other => panic!(
            "#3: untrusted issuer key must be rejected \
             (IssuerKeyNotInKeySet/UntrustedDeclaredKey), got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #4 (CRITICAL) — no replay / freshness binding.
//   Forge: binding-challenge != verifier nonce; and nonce replay.
//   Reject: NonceBindingMismatch / NonceReplay. LANE: default.
// =====================================================================

/// #4a — the manifest's declared binding challenge does NOT equal the
/// verifier-issued nonce. The nonce-consistency gate REJECTS
/// (`NonceBindingMismatch`).
#[test]
fn finding_04_nonce_binding_mismatch_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest(); // binding declares 0x2a
    let prover = CircuitProver::from_crate_root();
    match verify_manifest(
        &m,
        &prover,
        &scratch("f04_nonce_mismatch"),
        &trusted_k(&test_issuer_sk(1)),
        &fresh_policy(),
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for("0x99"), // verifier issues a DIFFERENT nonce
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::NonceBindingMismatch) => {}
        other => panic!("#4: binding != nonce must be NonceBindingMismatch, got {other:?}"),
    }
}

/// #4b — re-present the SAME (nonce, manifest) to the SAME single-use store. The
/// nonce is BURNED on the first presentation, so the SECOND REJECTS
/// (`NonceReplay`). The first reaches MissingProof (witness-only) — the nonce is
/// still consumed — so the replay defence is proven without bb.
#[test]
fn finding_04_nonce_replay_rejected() {
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
    match run("f04_replay_1") {
        Err(CheckError::MissingProof { proof: 0 }) => {}
        other => panic!("#4: first presentation should reach MissingProof, got {other:?}"),
    }
    match run("f04_replay_2") {
        Err(CheckError::NonceReplay) => {}
        other => panic!("#4: re-presentation under a burnt nonce must be NonceReplay, got {other:?}"),
    }
}

// =====================================================================
// FINDING #5 (CRITICAL) — FILTER operator/bound/verdict never bound to the
//   query's FILTER.
//   Forge: prove `17 >= 17 = true` and present it under a `?age >= 18` query.
//   Reject: UnboundFilter (bound mismatch). LANE: default (bind_query_correctness
//   is a stage-2 host gate).
// =====================================================================

/// Build an honest age scan (witness-only) over `{ ?s <ex/age> ?o }` so the
/// FILTER's pattern is answered. Returns the scan inputs + the salt/commitment so
/// a caller can attest it.
fn age_scan_inputs() -> (ProofInputs, Fr, Fr) {
    let salt = fixture_salt();
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let commitment = commit.commitment;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern).expect("scan builds");
    (scan.inputs, commitment, salt)
}

/// A witness-only `filter_int` sub-proof declaration over a host-chosen
/// `(operand_enc, op, bound, expected)` (no bb proof; the structural gates read
/// only the declared fields).
fn filter_int_inputs(value: u64, op: FilterOp, bound: u64, expected: bool) -> ProofInputs {
    let operand_enc = encode_int_literal(value);
    let (inputs, _digits) =
        build_filter_int(operand_enc, value, op, bound, expected).expect("in-family filter");
    inputs
}

/// Assemble a scan(0) + filter(1) witness-only manifest for the given query,
/// with one binding edge scan[row].slot → filter, attesting the age scan so the
/// structural gates reach the query-binding stage.
fn scan_plus_filter_manifest(
    query: &str,
    filter: ProofInputs,
    edge: Option<BindingEdge>,
) -> ProofManifest {
    let (scan_inputs, commitment, salt) = age_scan_inputs();
    let sk = test_issuer_sk(1);
    ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: query.into(),
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
        sub_proofs: vec![
            SubProof { inputs: scan_inputs, proof_hex: String::new() },
            SubProof { inputs: filter, proof_hex: String::new() },
        ],
        binding_edges: edge.into_iter().collect(),
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    }
}

/// #5 — the headline `age vs >=18` forge: a `>=18` query needs a gating filter
/// with `bound=18`; the prover wires a filter declaring a SMALLER `bound=17`
/// (so the credential's age 25 trivially passes `>=17`), keeping the operand
/// CONSISTENT with the scanned age (so stage-2 binding-consistency passes) but
/// the FILTER's bound does NOT equal the query's. `filter_edge_true` requires
/// `*f_bound == bound`, so no true-verdict edge gates ?o ⇒ REJECT
/// (`UnboundFilter`). This is the comparison-bound-substitution forge: the proof
/// answers a WEAKER predicate than the query demands.
#[test]
fn finding_05_filter_bound_substitution_rejected() {
    // Query: ?o (the age) must be >= 18. Forge a filter declaring bound=17.
    let query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }";
    // Operand = the scanned age (25) so stage-2 binding-consistency PASSES; the
    // forged bound is 17 (not the query's 18). 25 >= 17 = true, but bound != 18.
    let filter = filter_int_inputs(25, FilterOp::Ge, 17, true);
    // Edge points at the age object slot (slot 2) of scan row 0 → filter.
    let edge = BindingEdge { from_proof: 0, from_row: 0, from_slot: 2, to_proof: 1 };
    let m = scan_plus_filter_manifest(query, filter, Some(edge));
    match verify_full(&m, "f05_bound_sub") {
        Err(CheckError::UnboundFilter { variable }) => {
            assert_eq!(variable, "o", "#5: the FILTER variable is ?o");
        }
        other => panic!(
            "#5: a bound=17 filter presented for a >=18 query must be UnboundFilter \
             (bound mismatch), got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #6 (HIGH→CRITICAL slot binding) — binding edge ignored which slot the
//   FILTER constrains / didn't prune by verdict.
//   Forge: a true-verdict filter that is consistent with the query's (op, bound)
//   but is NOT wired (by a binding edge) to the slot ?o actually binds to, so the
//   FILTER variable's row is left ungated.
//   Reject: UnboundFilter (no true-verdict edge at the constrained slot). LANE:
//   default. (Pointing the edge at the WRONG slot whose encoding differs from the
//   operand is caught even EARLIER by stage-2 `BindingInconsistent`; the residual
//   slot-binding gate this pins is `bind_query_correctness`'s requirement that
//   ?o's OWN slot carry a true-verdict edge.)
// =====================================================================

/// #6 — a `?o >= 18` query with a true-verdict filter that matches the query's
/// (op, bound) and whose operand equals the scanned age, BUT no binding edge wires
/// it to ?o's slot (slot 2). `bind_query_correctness` requires every active row of
/// the answering scan to carry a true-verdict edge AT ?o's slot; with none ⇒
/// REJECT (`UnboundFilter`). This pins the verdict-pruning / slot-binding gate: a
/// passing verdict that is not bound to the constrained column cannot discharge the
/// FILTER.
#[test]
fn finding_06_verdict_not_wired_to_filter_slot_rejected() {
    let query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }";
    // The filter is consistent with the query's (op, bound) and true (25 >= 18)...
    let filter = filter_int_inputs(25, FilterOp::Ge, 18, true);
    // ...but it is NOT wired to ?o's slot: no binding edge at all, so ?o's row is
    // ungated. (A wrong-slot edge whose encoding != the operand would be caught
    // even earlier as BindingInconsistent — see finding_07_*; here we isolate the
    // slot-binding requirement of bind_query_correctness.)
    let m = scan_plus_filter_manifest(query, filter, None);
    match verify_full(&m, "f06_unwired") {
        Err(CheckError::UnboundFilter { variable }) => {
            assert_eq!(variable, "o", "#6: the FILTER variable is ?o");
        }
        other => panic!(
            "#6: a true verdict not wired to ?o's slot must leave the FILTER unbound — \
             expected UnboundFilter, got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #7 (HIGH) — composition seam (operand substitution / kind confusion)
//   was JSON-only.
//   Forge (host): a binding edge whose scanned column != the filter's operand_enc
//     (a claimed join that is a lie).
//   Reject (host): BindingInconsistent. LANE: default.
//   Forge (bb): a GENUINE filter proof over a DIFFERENT operand O' — the proof's
//     operand_enc public input cannot byte-match the scan's bound column.
//   Reject (bb): PublicInputMismatch. LANE: toolchain (`#[ignore]`, sq-f9tl).
// =====================================================================

/// #7 (host) — a binding edge claims scan[row=0,slot=2] feeds the filter, but the
/// filter's declared `operand_enc` is a DIFFERENT value. The stage-2
/// binding-consistency gate REJECTS (`BindingInconsistent`) — the join the prover
/// claimed is a lie. (This is the JSON-level seam; the proof-bound variant is the
/// `#[ignore]`d bb test below.)
#[test]
fn finding_07_operand_substitution_host_rejected() {
    let query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }";
    // The filter declares operand_enc for value 99 — NOT the scanned age (25).
    let filter = filter_int_inputs(99, FilterOp::Ge, 18, true);
    // Edge claims the scanned age column (slot 2) feeds this filter.
    let edge = BindingEdge { from_proof: 0, from_row: 0, from_slot: 2, to_proof: 1 };
    let m = scan_plus_filter_manifest(query, filter, Some(edge));
    match verify_full(&m, "f07_operand_sub") {
        Err(CheckError::BindingInconsistent { edge: 0 }) => {}
        other => panic!(
            "#7: a binding edge over a substituted operand must be BindingInconsistent, \
             got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #8 (CRITICAL) — cross-graph bnode join guard driven by prover-declared,
//   proof-unbound attributions.
//   Forge: omit the proof-bound attribution; and the `[[0],[0]]` collapse.
//   Reject: AttributionMalformed / AttributionUnderDeclared. LANE: default.
// =====================================================================

/// #8a — OMIT the proof-bound per-graph attribution vector (empty) so the
/// cross-graph obligation cross-check would be vacuous. REJECT
/// (`AttributionMalformed`: must be present + exactly k bits).
#[test]
fn finding_08_attribution_omitted_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    if let ProofInputs::Scan { attribution, .. } = &mut m.sub_proofs[0].inputs {
        attribution.clear();
    } else {
        unreachable!("sub-proof 0 is a scan");
    }
    match verify_full(&m, "f08_attr_omit") {
        Err(CheckError::AttributionMalformed { proof: 0, expected: 1, got: 0 }) => {}
        other => panic!("#8: omitted attribution must be AttributionMalformed, got {other:?}"),
    }
}

/// #8b — the headline `[[0],[0]]` collapse: a two-graph cross-graph join whose
/// `manifest.attributions` UNDER-declares a graph the scan PROVED a contribution
/// from, to drop a non-bnode obligation. REJECT (`AttributionUnderDeclared`).
#[test]
fn finding_08_attribution_collapse_rejected() {
    let sk = test_issuer_sk(1);
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
        attributions: vec![vec![0]], // FORGE: under-declare (drop graph 1).
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
    match verify_full(&m, "f08_attr_collapse") {
        Err(CheckError::AttributionUnderDeclared { pattern: 0, .. }) => {}
        other => panic!("#8: collapse forge must be AttributionUnderDeclared, got {other:?}"),
    }
}

// =====================================================================
// FINDING #9 (HIGH) — cross-graph bnode salt separation unenforced.
//   Forge: two DISTINCT scan-referenced graphs committed + attested under the
//     SAME salt (the cross-graph bnode-correlation channel).
//   Reject: SaltReused. LANE: default.
// =====================================================================

/// #9 — two distinct committed graphs (g0, g1), both drawn by ONE k=2 scan, both
/// attested under the SAME salt. The verifier's salt-uniqueness check finds the
/// salt reused across two distinct scan-referenced commitments ⇒ REJECT
/// (`SaltReused`).
#[test]
fn finding_09_salt_reused_rejected() {
    let sk = test_issuer_sk(1);
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
    // FORGE: the SAME salt for two distinct graphs.
    let reused = salt_from_bytes(&[42u8; 32]);
    let c0 = commit_triples(&g0, reused).unwrap();
    let c1 = commit_triples(&g1, reused).unwrap();
    assert_ne!(c0.commitment, c1.commitment, "distinct graphs => distinct commitments");
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 scan builds");
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(c0.commitment, reused, &sk),
            attest_full(c1.commitment, reused, &sk),
        ],
        attributions: vec![vec![0, 1]], // honest: both graphs contribute.
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
    match verify_full(&m, "f09_salt_reuse") {
        Err(CheckError::SaltReused { .. }) => {}
        other => panic!("#9: a salt reused across two scan-referenced graphs must be SaltReused, got {other:?}"),
    }
}

// =====================================================================
// FINDING #10 (HIGH) — query text not bound into the proof (FILTER-add /
//   constant-swap replay).
//   Forge (FILTER-add): add a FILTER to the query with no answering true-verdict
//     edge. Reject: UnboundFilter.
//   Forge (constant-swap): present an age scan under a SALARY query (different
//     BGP constant). Reject: UnboundPattern.
//   LANE: default.
// =====================================================================

/// #10a — FILTER-add: the query carries `FILTER(?o >= 18)` but the manifest has
/// no gating filter sub-proof / edge. REJECT (`UnboundFilter`).
#[test]
fn finding_10_filter_add_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    // Add a FILTER to the query the prover never proved (no filter sub-proof / edge).
    m.query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }".into();
    match verify_full(&m, "f10_filter_add") {
        Err(CheckError::UnboundFilter { variable }) => {
            assert_eq!(variable, "o", "#10a: the added FILTER is over ?o");
        }
        other => panic!("#10a: a FILTER-add with no proof must be UnboundFilter, got {other:?}"),
    }
}

/// #10b — constant-swap: the manifest's scan proves `<ex/age>` but the query asks
/// `<ex/salary>` (a different BGP constant). No scan answers the salary pattern ⇒
/// REJECT (`UnboundPattern`).
#[test]
fn finding_10_constant_swap_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    // Swap the query's predicate constant to one no scan proves.
    m.query = "SELECT ?s ?o WHERE { ?s <http://ex/salary> ?o }".into();
    match verify_full(&m, "f10_const_swap") {
        Err(CheckError::UnboundPattern { pattern: 0 }) => {}
        other => panic!(
            "#10b: an age scan presented under a salary query must be UnboundPattern, got {other:?}"
        ),
    }
}

// =====================================================================
// FINDING #11 (HIGH) — circuit-id re-derivation trusts declared n/d; r-bucket
//   relabel slack.
//   Forge (host): declare a scan id with the WRONG r bucket (the derived r from
//     rows.len()/row_count disagrees).
//   Reject (host): CircuitIdMismatch. LANE: default.
//   The n-relabel variant (n=64 proof declaring n=16) is closed by the canonical
//   vk recompute (bb verify rejects) — that is the proof-bound variant, gated
//   behind the toolchain lane below.
// =====================================================================

/// #11 (host) — declare the scan's circuit id with `r = 8` while the disclosed
/// `rows` (length 4, the `scan_k1_n16_r4` member) derive `r = 4`. Stage-1b
/// re-derivation disagrees with the declared id ⇒ REJECT (`CircuitIdMismatch`).
/// This pins the r-bucket relabel closed without bb.
#[test]
fn finding_11_circuit_id_r_relabel_rejected() {
    let (mut m, _c, _s) = honest_scan_only_manifest();
    if let ProofInputs::Scan { id, .. } = &mut m.sub_proofs[0].inputs {
        // FORGE: relabel r to a different valid bucket (8) than the derived (4).
        match id {
            CircuitId::Scan { r, .. } => {
                assert_eq!(*r, 4, "honest scan_k1_n16_r4 declares r=4");
                *r = 8;
            }
            other => unreachable!("scan id, got {other:?}"),
        }
    } else {
        unreachable!("sub-proof 0 is a scan");
    }
    match verify_full(&m, "f11_r_relabel") {
        Err(CheckError::CircuitIdMismatch { proof: 0, declared, derived }) => {
            assert_eq!(declared, CircuitId::Scan { k: 1, n: 16, r: 8 }, "declared the forged r=8");
            assert_eq!(
                derived,
                Some(CircuitId::Scan { k: 1, n: 16, r: 4 }),
                "derived the honest r=4 from rows"
            );
        }
        other => panic!("#11: an r-bucket relabel must be CircuitIdMismatch, got {other:?}"),
    }
}

// =====================================================================
// FINDING #12 (MEDIUM) — revocation unimplemented; status-list index disclosed.
//   Forge: a genuine, fresh, issuer-bound credential whose AUTHORITATIVE status
//     bit is SET (revoked); and a credential whose version is out of the
//     freshness window.
//   Reject: CredentialRevoked / StatusListStale. LANE: default.
// =====================================================================

/// #12a — present a genuine, fresh, issuer-bound credential but whose
/// AUTHORITATIVE status bit (the relying party's OWN snapshot) is SET. The
/// liveness gate reads the authoritative snapshot (never the prover's) ⇒ REJECT
/// (`CredentialRevoked`).
#[test]
fn finding_12_revoked_bit_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest();
    let prover = CircuitProver::from_crate_root();
    match verify_manifest(
        &m,
        &prover,
        &scratch("f12_revoked"),
        &trusted_k(&test_issuer_sk(1)),
        &revoked_policy(), // authoritative bit SET
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::CredentialRevoked { index: STATUS_INDEX, .. }) => {}
        other => panic!("#12a: a revoked credential must be CredentialRevoked, got {other:?}"),
    }
}

/// #12b — the credential's issuer-bound reference is version 1, but the relying
/// party's freshness window only accepts a LATER version (no snapshot for v1 in
/// window). The freshness gate REJECTS (`StatusListStale`).
#[test]
fn finding_12_stale_version_rejected() {
    let (m, _c, _s) = honest_scan_only_manifest();
    let prover = CircuitProver::from_crate_root();
    let policy = RevocationPolicy::accept_version(5).with_snapshot(StatusListSnapshot {
        status_list: STATUS_LIST.to_string(),
        version: 5,
        bits: vec![0u8],
    });
    match verify_manifest(
        &m,
        &prover,
        &scratch("f12_stale"),
        &trusted_k(&test_issuer_sk(1)),
        &policy,
        &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(),
        &EntailmentPolicy::simple_only(),
        &nonce_for(CHALLENGE_HEX),
        &InMemorySeenNonces::new(),
    ) {
        Err(CheckError::StatusListStale { version: STATUS_VERSION, .. }) => {}
        other => panic!("#12b: a stale version must be StatusListStale, got {other:?}"),
    }
}

// =====================================================================
// TOOLCHAIN-GATED LANE (sq-f9tl): findings whose forge requires a GENUINE
// (forged) bb proof to construct. These run only with `-- --ignored` on a box
// with nargo + bb. Each is written so the toolchain CI lane can run it; the
// reject path is the same one the re-audit maps for these findings.
// =====================================================================

/// Prove a real `filter_int_d1` of `value <op> bound = expected`; returns the
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

/// Prove a real scan over `{ ?s <ex/age> ?o }`; returns the honest scan
/// ProofInputs + its encoded artifact hex.
fn honest_age_scan(challenge: &FieldHex, prover: &CircuitProver, tag: &str) -> (ProofInputs, String) {
    let salt = fixture_salt();
    let commit = commit_triples(&credential_graph(), salt).unwrap();
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/age"))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern).expect("scan builds");
    let (id, toml) =
        prover_toml_for(&scan.inputs, challenge, &scan.witness.counts, &scan.witness.enc, &[], None, None).unwrap();
    let out = scratch(tag);
    let art = prover.prove_in(&id, &toml, &out, tag).expect("scan proves");
    (scan.inputs, encode_artifacts(&art))
}

/// Build a composed manifest: an honest age scan (index 0, answers the query) +
/// a possibly-forged FILTER sub-proof (index 1). The scan carries a valid
/// salt+status-bound attestation so the byte-binding stage is reached.
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

/// #1 (CRITICAL) — bb-verified public_inputs reconstructed from / compared to the
/// declared statement. Forge: a GENUINE proof of `5 < 10 = true` presented under a
/// manifest DECLARING `bound = 99`. The reconstruction (bound=99) cannot byte-match
/// the proof's committed inputs (bound=10) ⇒ REJECT (`PublicInputMismatch`).
///
/// `#[ignore]`: requires a real `bb prove` (the toolchain lane, sq-f9tl) — the
/// forge is a genuine proof of statement A re-labelled as statement B, which can
/// only be constructed with the prover toolchain.
#[test]
#[ignore = "toolchain lane (sq-f9tl): needs a real bb prove to forge a genuine-proof-over-different-statement (#1)"]
fn finding_01_public_input_statement_substitution_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping #1 toolchain forge");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "f01_scan");
    let (mut filter, art) = honest_filter_d1(5, FilterOp::Lt, 10, true, &challenge, &prover, "f01");
    // FORGE: declare bound=99 while proof_hex still proves 5 < 10.
    if let ProofInputs::FilterInt { bound, .. } = &mut filter {
        *bound = 99;
    }
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    match verify_full(&m, "f01_verify") {
        Err(CheckError::PublicInputMismatch { proof: 1 }) => {}
        other => panic!("#1: statement substitution must be PublicInputMismatch, got {other:?}"),
    }
}

/// #2 (CRITICAL) — bb verify uses the verifier-recomputed CANONICAL vk, never the
/// prover-supplied vk. Forge: take a genuine proof but corrupt the bundled vk in
/// the artifact hex. The verifier recomputes the canonical vk and runs bb verify
/// against THAT; a tampered bundled vk is simply ignored, so the proof STILL
/// verifies (the bundled vk is dead weight) — this test instead asserts the
/// positive control: an honest composed manifest verifies through the canonical-vk
/// path. The negative — a proof made against a DIFFERENT circuit's vk — is the n/d/r
/// relabel forge (#11 bb variant below): bb verify against the canonical vk
/// REJECTS it (`ProofRejected`).
///
/// `#[ignore]`: requires a real `bb prove` + `bb verify` (toolchain lane, sq-f9tl).
#[test]
#[ignore = "toolchain lane (sq-f9tl): needs a real bb prove/verify to exercise the canonical-vk path (#2)"]
fn finding_02_canonical_vk_used_not_prover_vk() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping #2 toolchain control");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "f02_scan");
    let (filter, art) = honest_filter_d1(5, FilterOp::Lt, 10, true, &challenge, &prover, "f02");
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    // Positive control: the canonical-vk recompute path accepts an honest proof.
    verify_full(&m, "f02_verify").expect(
        "#2: an honest composed manifest must verify through the verifier-recomputed canonical vk \
         (the prover-supplied vk is never trusted)",
    );
}

/// #7 (bb variant) — a GENUINE filter proof over a DIFFERENT operand than the
/// scanned column. The filter proof's `operand_enc` public input cannot byte-match
/// the value bound into the scan proof ⇒ REJECT (`PublicInputMismatch` on the
/// filter sub-proof). Here we forge by declaring a different `operand_enc` than the
/// proof proves: the reconstruction over the declared operand mismatches the
/// proof's committed operand.
///
/// `#[ignore]`: requires a real `bb prove` (toolchain lane, sq-f9tl).
#[test]
#[ignore = "toolchain lane (sq-f9tl): needs a real bb prove to forge a genuine filter proof over a substituted operand (#7)"]
fn finding_07_operand_substitution_bb_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping #7 bb forge");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let (scan_inputs, scan_hex) = honest_age_scan(&challenge, &prover, "f07bb_scan");
    // Genuine proof of operand 5 < 10 = true.
    let (mut filter, art) =
        honest_filter_d1(5, FilterOp::Lt, 10, true, &challenge, &prover, "f07bb");
    // FORGE: declare a DIFFERENT operand_enc (for value 6) while the proof proves 5.
    if let ProofInputs::FilterInt { operand_enc, .. } = &mut filter {
        *operand_enc = encode_int_literal(6);
    }
    let m = composed_manifest(scan_inputs, scan_hex, filter, encode_artifacts(&art));
    match verify_full(&m, "f07bb_verify") {
        Err(CheckError::PublicInputMismatch { proof: 1 }) => {}
        other => panic!("#7 (bb): a substituted operand must be PublicInputMismatch, got {other:?}"),
    }
}

/// #11 (bb variant) — the n-relabel forge: a proof made against the `n=64` scan
/// member, presented declaring `n=16`. Per the re-audit (#11), the ONLY compiled
/// pair differing solely in `n` is `scan_k2_n16_r8` vs `scan_k2_n64_r8` (k=2; `n`
/// sizes the PRIVATE slot array, so the two members have DIFFERENT constraint
/// systems and different vks). The verifier recomputes the canonical vk for the
/// DECLARED (n=16) member and `bb verify` REJECTS the n=64 proof ⇒ `ProofRejected`
/// (or a public-input length mismatch ⇒ `PublicInputMismatch`).
///
/// `#[ignore]`: requires a real `bb prove` against the n=64 member (toolchain
/// lane, sq-f9tl). Constructing the forge needs the prover to make a genuine
/// proof against the n=64 member, which only the toolchain can do.
#[test]
#[ignore = "toolchain lane (sq-f9tl): needs a real bb prove against the scan_k2_n64_r8 member to forge the n-relabel (#11 bb variant)"]
fn finding_11_n_relabel_bb_rejected() {
    if !toolchain_available() {
        eprintln!("nargo/bb absent; skipping #11 bb n-relabel forge");
        return;
    }
    let challenge = FieldHex(CHALLENGE_HEX.into());
    let prover = CircuitProver::from_crate_root();
    let sk = test_issuer_sk(1);
    // Graph 0: >16 triples so max_graph forces n=64, with 5 matching <ex/p0> rows
    // (distinct objects) so the disclosed match count (5 + graph 1's 1 = 6) lands
    // in the r=8 bucket — the ONLY compiled k=2/n=64 member is scan_k2_n64_r8.
    let subj0 = NamedOrBlankNode::NamedNode(iri("http://ex/big"));
    let mut g0 = Vec::new();
    for i in 0..5u64 {
        // 5 matching <ex/p0> triples with distinct objects.
        g0.push(Triple::new(subj0.clone(), iri("http://ex/p0"), int_lit(100 + i)));
    }
    for i in 0..15u64 {
        // 15 non-matching filler triples → 20 total (forces n=64).
        g0.push(Triple::new(subj0.clone(), iri(&format!("http://ex/q{i}")), int_lit(i)));
    }
    let g1 = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/other")),
        iri("http://ex/p0"),
        int_lit(7),
    )];
    let salt0 = salt_from_bytes(&[71u8; 32]);
    let salt1 = salt_from_bytes(&[72u8; 32]);
    let c0 = commit_triples(&g0, salt0).expect("big graph commits");
    let c1 = commit_triples(&g1, salt1).expect("small graph commits");
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri("http://ex/p0"))),
        o: Slot::Var,
    };
    let scan = build_scan(&[c0.clone(), c1.clone()], &pattern).expect("k=2 n=64 scan builds");
    // Confirm the data really derived the n=64 member; otherwise this is not the
    // forge it claims to be.
    assert_eq!(
        scan.inputs.circuit_id(),
        &CircuitId::Scan { k: 2, n: 64, r: 8 },
        "#11 bb: the >16-slot graph must derive the scan_k2_n64_r8 member"
    );
    let (id, toml) =
        prover_toml_for(&scan.inputs, &challenge, &scan.witness.counts, &scan.witness.enc, &[], None, None).unwrap();
    let out = scratch("f11bb");
    let art = prover.prove_in(&id, &toml, &out, "f11bb").expect("scan_k2_n64_r8 proves");

    // FORGE: relabel the declared id to the n=16 member (scan_k2_n16_r8).
    let mut forged = scan.inputs.clone();
    if let ProofInputs::Scan { id, .. } = &mut forged {
        *id = CircuitId::Scan { k: 2, n: 16, r: 8 };
    }
    let m = ProofManifest {
        fully_hidden_revocation: None,
        r#type: "urn:sparq:zk:ProofManifest".into(),
        query: "SELECT ?s ?o WHERE { ?s <http://ex/p0> ?o }".into(),
        issuers: vec![],
        key_set: vec![public_key_to_hex(&sk.public_key())],
        commitment_attestations: vec![
            attest_full(c0.commitment, salt0, &sk),
            attest_full(c1.commitment, salt1, &sk),
        ],
        attributions: vec![vec![0, 1]],
        pattern_scans: vec![],
        join_obligations: vec![],
        entailment_regime: EntailmentRegime::Simple,
        derivation_steps: vec![],
        binding: BindingMode::Challenge { challenge: FieldHex(CHALLENGE_HEX.into()) },
        revocation: Some(fixture_revocation()),
        status_snapshots: vec![fixture_snapshot(false)],
        sub_proofs: vec![SubProof { inputs: forged, proof_hex: encode_artifacts(&art) }],
        binding_edges: vec![],
        join_edges: vec![],
        hidden_revocation: None,
        hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
    };
    // The n-relabel is closed by the canonical-vk recompute (bb verify rejects
    // the n=64 proof against the n=16 vk) and/or the public-input length mismatch.
    match verify_full(&m, "f11bb_verify") {
        Err(CheckError::ProofRejected { proof: 0 })
        | Err(CheckError::PublicInputMismatch { proof: 0 }) => {}
        other => panic!(
            "#11 (bb): an n=64 proof declaring n=16 must be rejected \
             (ProofRejected / PublicInputMismatch), got {other:?}"
        ),
    }
}
