//! [GPT-6] Genuine synthetic public-preimage RDF proof and typed tamper controls.
use ark_std::rand::{rngs::StdRng, RngCore, SeedableRng};
use serde_json::json;
use sparq_native_composition_spike::rdf::{
    issue_rdf, prove_public_bgp, public_context, verify_public_bgp, AcceptedStatus, ConsumedNonces,
    Error, Issuer, Request, RolePolicy, StatusReference, TrustedIssuer,
};
use std::{collections::BTreeMap, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err(
            "native-rdf takes no arguments; stdout is the synthetic experiment JSON".into(),
        );
    }
    let mut rng = StdRng::from_entropy();
    let setup_start = Instant::now();
    let issuer = Issuer::generate(&mut rng, "urn:synthetic:issuer:names")?;
    let setup_ms = setup_start.elapsed().as_secs_f64() * 1000.0;
    let status = StatusReference {
        list: "urn:synthetic:status:names".into(),
        epoch: 1,
        index: 3,
    };
    let document = "<urn:alice> <urn:name> \"Alice\" .\n<urn:alice> <urn:private> \"undisclosed synthetic value\" .";
    let issue_start = Instant::now();
    let credential = issue_rdf(&mut rng, &issuer, document, &status)?;
    let issue_ms = issue_start.elapsed().as_secs_f64() * 1000.0;
    let mut nonce = [0; 32];
    rng.fill_bytes(&mut nonce);
    let request = Request {
        query: "SELECT DISTINCT ?name WHERE { <urn:alice> <urn:name> ?name }".into(),
        rows: vec![BTreeMap::from([("name".into(), "\"Alice\"".into())])],
        roles: vec![RolePolicy {
            issuer: TrustedIssuer::from_bytes(
                "urn:synthetic:issuer:names",
                &issuer.public().to_bytes()?,
            )?,
            status: AcceptedStatus {
                reference: status.clone(),
                bits: vec![0],
            },
        }],
        nonce,
    };
    let prove_start = Instant::now();
    let proof = prove_public_bgp(&mut rng, &request, &[credential])?;
    let prove_ms = prove_start.elapsed().as_secs_f64() * 1000.0;
    let mut consumed = ConsumedNonces::default();
    let verify_start = Instant::now();
    verify_public_bgp(&mut rng, &request, &proof, &mut consumed)?;
    let verify_ms = verify_start.elapsed().as_secs_f64() * 1000.0;
    let mut rejections = BTreeMap::new();
    for case in [
        "query",
        "result",
        "nonce",
        "issuer",
        "status_epoch",
        "revoked",
    ] {
        let mut changed = request.clone();
        match case {
            "query" => changed.query = changed.query.replace("urn:alice", "urn:mallory"),
            "result" => {
                changed.rows[0].insert("name".into(), "\"Mallory\"".into());
            }
            "nonce" => changed.nonce[0] ^= 1,
            "issuer" => {
                changed.roles[0].issuer =
                    Issuer::generate(&mut rng, "urn:synthetic:issuer:names")?.public()
            }
            "status_epoch" => changed.roles[0].status.reference.epoch += 1,
            "revoked" => changed.roles[0].status.bits[0] = 8,
            _ => unreachable!(),
        }
        let actual = verify_public_bgp(&mut rng, &changed, &proof, &mut ConsumedNonces::default());
        let expected = if case == "revoked" {
            Error::Status
        } else {
            Error::Verification
        };
        if actual != Err(expected) {
            return Err(format!("unexpected typed outcome for {case}: {actual:?}").into());
        }
        rejections.insert(case, format!("{expected:?}"));
    }
    if verify_public_bgp(&mut rng, &request, &proof, &mut consumed) != Err(Error::Replay) {
        return Err("replay accepted".into());
    }
    rejections.insert("replay", "Replay".into());
    let public = json!({
        "context": serde_json::from_slice::<serde_json::Value>(&public_context(&request, &proof.support)?)?,
        "nonce": nonce,
        "triple_slots": sparq_native_composition_spike::rdf::TRIPLE_SLOTS
    });
    let public_bytes = serde_json::to_vec(&public)?.len();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "status": "experimental_not_externally_audited",
            "protocol": "sparq/native-rdf/public-bgp/v1",
            "signature_encoding": "experimental Dock BBS+ BLS12-381 canonical RDF message vector",
            "authentication": "fresh synthetic issuer key independently selected in verifier request",
            "semantics": "nonempty SELECT DISTINCT successful support; no completeness claim",
            "disclosure": "query, mappings, issuer roles, status list/epoch/index, canonical signed-slot indices and fixed capacity",
            "original_vc_authentication": false, "w3c_bbs_2023": false,
            "hidden_rdf_predicate_link": false, "residual_circuit_used": false,
            "canonical_timing": false, "timing_scope": "inclusive local API wall times; no speedup claim",
            "timings_ms": {"key_generation": setup_ms, "issue_including_canonicalization_and_signature_check": issue_ms, "prove_including_reconstruction": prove_ms, "verify_including_reconstruction_status_and_signature": verify_ms, "internal_signature_only": null, "internal_canonicalization_only": null},
            "rss_bytes": null, "build_attestation": null,
            "public_transcript": public, "public_transcript_json_bytes": public_bytes,
            "proof_bytes": proof.proof.len(), "proof": proof.proof,
            "independent_verifier": "library rebuilds expected statements from caller request",
            "positive_verification": true, "typed_rejections": rejections
        }))?
    );
    Ok(())
}
