# Manifest recipes

These recipes share the [main skill's prerequisites and scope](../SKILL.md).
The proof estate remains experimental and externally unaudited (sq-qhy4).

<!-- zkp-10.1: separate exact-dataset contract, not a selected-result upgrade. -->
The opt-in detached [exact evaluator](exact-evaluator.md) proves bounded default-graph evaluation with explicit verifier-agreed or holder-declared scope. It is experimental and not externally audited; its first adapter carries no issuer-signature or credential-status claim.
The separately versioned [complete named-dataset API](exact-datasets-v2.md) adds a committed catalog including empty named graphs, GRAPH and local snapshot FROM/FROM NAMED selection. V1 requests and commitments retain their meaning.
The opt-in [V3 graph-result API](graph-results-v3.md) adds blank-node identity, canonical tables and bounded graph forms through typed host/guest APIs; consult artifact-bound evidence for actual execution status.
Issuer-authenticated RDF (V5): every feature is off by default and experimental, and each layer keeps its own evidence level (native model tests, native host tests, direct guest execution, adapter native tests, one genuine adapter case; the low-level driver has no receipt):

- **Model** (model feature `authenticated-rdf`): the [native V5 model](authenticated-rdf-model.md) checks W3C `eddsa-rdfc-2022` credentials against a verifier-pinned key table, then reuses V3 evaluation.
- **Guest** (host feature `authenticated-rdf`): a [separately pinned V5 guest and low-level host API](authenticated-rdf-guest.md) with a committed guest lock. An independently audited scoped gate at `42d13fed` passed its native tests and executed the guest directly, without proving. Its ignored genuine-proof driver proves only the one to six case IDs a v2 job declares; a subset run claims nothing about undeclared cases.
- **Adapter** (host feature `vcq-authenticated` = `vcq` + `authenticated-rdf`): the generic [V5 vcq adapter](vcq-authenticated-rdf-adapter.md) `vcq_authenticated::Risc0AuthenticatedRdfV5` declares six tuples (bag SELECT, ASK, CONSTRUCT; `IssuerAuthenticated`; both authorities). An independently verified scoped native gate at `7fe88955` passed its tests and Clippy without proving. Separately, at that frozen source, one independently audited genuine receipt of the approved V5 guest proved one public synthetic case, `select-bag-verifier-agreed`, with 28 controls. The other five tuples and the row-bound case have not run, so the registry keeps its `adapter_available` false.

None processes JSON-LD or full Data Integrity or checks status, holder binding, validity against a clock or wallet/world completeness. The campaign's V5 and vcq commands have not run as a full campaign. Not externally audited (`sq-qhy4`). <!-- privacy-claims-allow: native tests, direct execution and one synthetic adapter receipt; not audited -->
The [campaign evidence guide](evaluator-evidence.md) explains exact artifact export, actual receipt evidence and source/HAL provenance.
The [exact-evaluator experiment adapter](../../../zk/sparql-evaluator/experiments/README.md) runs fixed synthetic V2 contracts against an independently accepted caller artifact and pin, with real local receipts, typed tamper controls and scoped measurement.

The [query-coverage corpus](evaluator-conformance.md) separates executed host semantics, explicit unsupported features, and guest proof evidence. Its opt-in NPS comparison recipe records other engines' observations without installing dependencies or treating their output as a conformance oracle.

### 1. Commit a credential graph and attest it as an issuer
```rust
use sparq_zk::commit::{commit_triples, CommitmentMethod};
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{SecretKey, status_list_id_to_field, status_ref_digest, public_key_to_hex, scheme_iri_to_field, SignatureScheme};
use sparq_zk_compose::manifest::{AttestedStatusRef, CommitmentAttestation, FieldHex};

let salt = salt_from_bytes(&[7u8; 32]);                 // production: use IngestedDataset (OS-random)
let commit = commit_triples(&credential_triples, salt).unwrap();

let issuer = SecretKey::from_seed(1);                   // production: generate from OS entropy, NOT a seed
let (list, index, version) = ("http://ex/status/1", 3u64, 1u64);
let status_ref = status_ref_digest(&status_list_id_to_field(list), index, version);

// A scan-covering attestation MUST bind salt AND status reference (fail-closed otherwise).
let attestation = CommitmentAttestation {
    commitment: FieldHex::from_field(&commit.commitment),
    issuer_public_key: public_key_to_hex(&issuer.public_key()),
    signature: issuer.sign_presentation_with_status_and_scheme(
        &commit.commitment, &salt, &status_ref,
        &scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri()),
    ),
    cryptosuite: SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri().to_string(),
    salt: Some(FieldHex::from_field(&salt)),
    status: Some(AttestedStatusRef::clear(index, version)),
    holder: None,                                       // bearer; see HolderBindingPolicy
};
```

### 2. Build a BGP scan proof for `{ ?s <http://ex/age> ?o }`
```rust
use oxrdf::{NamedNode, Term};
use sparq_zk_compose::build::{build_scan, Pattern, Slot};
use sparq_zk_compose::manifest::ProofInputs;

let pattern = Pattern {
    s: Slot::Var,
    p: Slot::Const(Term::NamedNode(NamedNode::new("http://ex/age").unwrap())),
    o: Slot::Var,
};
let scan = build_scan(&[commit /* GraphCommitment */], &pattern).expect("a compiled scan member fits");
// scan.inputs is ProofInputs::Scan { id, commitments, rows, attribution, .. };
// scan.witness has the private per-graph encodings prover_toml_for needs.
let operand_enc = match &scan.inputs {
    ProofInputs::Scan { rows, .. } => rows[0][2].clone(), // the object column to feed a FILTER
    _ => unreachable!(),
};
```

### 3. Assemble + prove a full manifest (scan + bound integer FILTER)
```rust
use sparq_zk_compose::build::build_filter_int;
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::toml::prover_toml_for;
use sparq_zk_compose::verifier::encode_artifacts;
use sparq_zk_compose::manifest::*;

let challenge = FieldHex("0x2a".into());                 // == the verifier's nonce
let prover = CircuitProver::from_crate_root();
let out = std::env::temp_dir().join("sparq_zk_manifest");

// scan proof
let (scan_id, scan_toml) = prover_toml_for(&scan.inputs, &challenge,
    &scan.witness.counts, &scan.witness.enc, &[], None, None).unwrap();
let scan_art = prover.prove_in(&scan_id, &scan_toml, &out, "scan").unwrap();

// filter proof `?o >= 18`, verdict true
let (filter_inputs, digits) = build_filter_int(operand_enc, 25, FilterOp::Ge, 18, true).unwrap();
let (f_id, f_toml) = prover_toml_for(&filter_inputs, &challenge, &[], &[], &digits, None, None).unwrap();
let filter_art = prover.prove_in(&f_id, &f_toml, &out, "filter").unwrap();

let manifest = ProofManifest {
    r#type: "urn:sparq:zk:ProofManifest".into(),
    query: "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }".into(),
    issuers: vec!["did:key:zIssuer".into()],
    key_set: vec![public_key_to_hex(&issuer.public_key())], // declared (narrowing) — NOT the trust anchor
    commitment_attestations: vec![attestation],
    attributions: vec![vec![0]],                            // one BGP pattern, from graph 0
    pattern_scans: vec![vec![0]],                           // recorded reading; carries NO verification weight
    join_obligations: vec![],
    entailment_regime: EntailmentRegime::Simple,
    derivation_steps: vec![],                               // Simple regime: no inference steps
    binding: BindingMode::Challenge { challenge: FieldHex("0x2a".into()) },
    revocation: Some(RevocationStatus::clear("http://ex/status/1", 3, 1)),  // ::committed / ::fully_hidden for the hidden modes
    status_snapshots: vec![],                               // prover copy is only a tripwire; see recipe 4
    sub_proofs: vec![
        SubProof { inputs: scan.inputs,   proof_hex: encode_artifacts(&scan_art) },
        SubProof { inputs: filter_inputs, proof_hex: encode_artifacts(&filter_art) },
    ],
    binding_edges: vec![BindingEdge { from_proof: 0, from_row: 0, from_slot: 2, to_proof: 1 }],
    join_edges: vec![],
    hidden_revocation: None,
    fully_hidden_revocation: None,
    hidden_issuer_attestations: vec![],
    holder_pok_proofs: vec![],
    holder_set_proofs: vec![],
};
let json = manifest.to_json().unwrap(); // ships to the verifier
```

### 4. Verify a manifest as a relying party (the full-binding path)
```rust
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::verifier::{
    verify_manifest, EntailmentPolicy, FileSeenNonces, HolderBindingPolicy, HolderRegistry, KeySet,
    RevocationPolicy, VerifierNonce,
};
use sparq_zk_compose::manifest::{ProofManifest, StatusListSnapshot};
use sparq_zk::sig::public_key_to_hex;

let manifest = ProofManifest::from_json(&json).unwrap();

// External trust anchors — resolved + authenticated OUT OF BAND, never from the manifest:
let trusted = KeySet::from_hex_keys([public_key_to_hex(&issuer.public_key())]); // issuers you trust
let policy = RevocationPolicy::accept_version(1)        // freshness window; or ::up_to(now, window)
    .with_snapshot(StatusListSnapshot {                // AUTHORITATIVE bitstring (bit decision reads HERE)
        status_list: "http://ex/status/1".into(), version: 1, bits: vec![0u8] /* index 3 unset = active */,
    });

let nonce = VerifierNonce::from_hex("0x2a").unwrap();  // mint fresh per session, hand to prover BEFORE proving
let seen = FileSeenNonces::open("/var/lib/sparq/seen_nonces").unwrap(); // durable single-use store

let prover = CircuitProver::from_crate_root();
match verify_manifest(&manifest, &prover, std::path::Path::new("/tmp/verify"),
                      &trusted, &policy,
                      &HolderRegistry::empty(),            // Challenge binding: no holder PoP in use
                      &HolderBindingPolicy::allow_bearer(), // or ::require_binding()
                      &EntailmentPolicy::simple_only(),     // reject RDFS/OWL manifests
                      &nonce, &seen) {
    Ok(())  => { /* result is correct, issuer-attested, live, fresh */ }
    Err(e)  => eprintln!("rejected: {e:?}"), // CheckError variant pinpoints the failed gate
}
```
The nonce is **burned on presentation** (consumed even if verification then fails) — a rejection is never a free retry; mint a new nonce for the next attempt.

### 5. Fast structural pre-check without the toolchain
```rust
use sparq_zk_compose::verifier::{prefilter_manifest_structure, KeySet, RevocationPolicy};
// Re-parses the query (Q6 cross-graph bnode-join guard + arity), re-derives circuit ids,
// checks binding edges + issuer/key-set + revocation reference. NO bb, NO freshness.
// sq-en5dx: the Q6 guard keys each pattern's attribution on committed-graph
// IDENTITY (the answering scan's commitments[g]), NOT the scan-LOCAL index — so a
// genuine cross-scan join over two DISTINCT committed graphs (e.g. two k=1 scans, the
// `[[0],[0]]` alias) requires its non-bnode `join_obligations` entry to be DECLARED,
// while two scans over the SAME graph stay obligation-free. Still under sq-qhy4: NOT sound.
// NOT a sound verifier on its own — always follow with verify_manifest. privacy-claims-allow: negative usage; sq-toze.35
let _required = prefilter_manifest_structure(&manifest, &KeySet::empty(), &RevocationPolicy::accept_version(1))?;
```

### 6. Hidden-index revocation proof (privacy upgrade — index never disclosed)
```rust
use sparq_zk_compose::revocation::{merkle_root, merkle_witness, revoke_prover_toml};
use sparq_zk_compose::manifest::{CircuitId, HiddenIndexRevocation, FieldHex};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::verifier::encode_artifacts;

let depth = 10;                                  // compiled member: revoke_unset_d10 (<= 1024 indices)
let root = merkle_root(&authoritative_snapshot, depth).unwrap();
let witness = merkle_witness(&authoritative_snapshot, depth, /*hidden index*/ 3).unwrap(); // witness.bit must be 0
// The issuer signs a HIDING index commitment; the circuit recomputes it from index + blinding.
let index_commitment = sparq_zk::sig::status_index_commitment(3, &blinding); // blinding: per-credential, OS-random
let toml = revoke_prover_toml(&nonce_field, &root, &index_commitment, 3, &blinding, &witness);
let prover = CircuitProver::from_crate_root();
let art = prover.prove_in(&CircuitId::RevokeUnset { depth }, &toml, std::path::Path::new("/tmp/rev"), "rev").unwrap();
// attach to manifest.hidden_revocation; enable on the verifier with
// RevocationPolicy::...with_hidden_index_depth(depth). (Hidden-issuer attestations mirror this
// with sparq_zk_compose::issuer::* + KeySet::with_hidden_issuer_depth.)
```

