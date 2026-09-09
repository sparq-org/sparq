//! Exercise an established native signature and residual circuit composition.
//!
//! [GPT-6] Research executable only. This is not an RDF adapter or an audited
//! Sparq query verifier. See the accompanying README for the exact statement.

use ark_bls12_381::{Bls12_381, Fr};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::rand::{rngs::StdRng, RngCore, SeedableRng};
use bbs_plus::prelude::{KeypairG2, PublicKeyG2, SignatureG1, SignatureParamsG1};
use blake2::Blake2b512;
use legogroth16::{
    circom::{CircomCircuit, R1CS},
    ProvingKey, VerifyingKey,
};
use proof_system::{
    prelude::{EqualWitnesses, MetaStatements, Proof, ProofSpec, Witness, Witnesses},
    statement::{
        bbs_plus::{PoKBBSSignatureG1Prover, PoKBBSSignatureG1Verifier},
        r1cs_legogroth16::{R1CSCircomProver, R1CSCircomVerifier},
        Statements,
    },
    witness::{PoKBBSSignatureG1, R1CSCircomWitness},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, time::Instant};

type Result<T> = std::result::Result<T, String>;
type CompositeProof = Proof<Bls12_381>;

// Signed tuples are [schema, subject, unsigned amount in fixed units].
const SUBJECT: usize = 1;
const AMOUNT: usize = 2;
const INCOME_SCHEMA: u64 = 1001;
const RENT_SCHEMA: u64 = 1002;
const CONTEXT: &[u8] = b"sparq/native-composition/research/v1:income-12*rent>=threshold;u32";
const BACKEND: &str = "dock-legogroth16-bls12-381";

#[derive(Clone)]
struct TrustedIssuer {
    parameters: SignatureParamsG1<Bls12_381>,
    key: PublicKeyG2<Bls12_381>,
}

// Intentionally no Debug implementation: these are holder-private inputs.
#[derive(Clone)]
struct Credential {
    messages: [Fr; 3],
    signature: SignatureG1<Bls12_381>,
}

struct Setup {
    r1cs: R1CS<Bls12_381>,
    wasm: Vec<u8>,
    key: ProvingKey<Bls12_381>,
}

#[derive(Clone)]
struct Request {
    // The verifier supplies these independently of the holder's proof.
    issuers: [TrustedIssuer; 2],
    schemas: [Fr; 2],
    threshold: Fr,
    nonce: Vec<u8>,
    context: Vec<u8>,
}

fn select_backend(name: &str) -> Result<()> {
    if name == BACKEND {
        Ok(())
    } else {
        Err(format!(
            "unsupported backend {name}: no same-witness link to Noir/BN254 is implemented"
        ))
    }
}

fn setup(rng: &mut StdRng) -> Result<Setup> {
    let root = Path::new(env!("OUT_DIR"));
    let circuit = CircomCircuit::<Bls12_381>::from_r1cs_file(root.join("eligibility.r1cs"))
        .map_err(|_| "cannot load the compiled eligibility circuit")?;
    // Commit exactly income and rent. build.rs checks their compiled wire order.
    let key = circuit
        .generate_proving_key(2, rng)
        .map_err(|_| "cannot generate experimental circuit setup")?;
    let r1cs = R1CS::from_file(root.join("eligibility.r1cs")).map_err(|_| "cannot load R1CS")?;
    let wasm = std::fs::read(root.join("eligibility_js/eligibility.wasm"))
        .map_err(|_| "cannot load the generated witness calculator")?;
    Ok(Setup { r1cs, wasm, key })
}

fn issue(
    rng: &mut StdRng,
    schema: u64,
    subject: u64,
    amount: u64,
) -> Result<(TrustedIssuer, Credential)> {
    let parameters =
        SignatureParamsG1::new::<Blake2b512>(b"sparq/native-composition/research/v1/credential", 3);
    let keypair = KeypairG2::generate_using_rng(rng, &parameters);
    let messages = [Fr::from(schema), Fr::from(subject), Fr::from(amount)];
    let signature = SignatureG1::new(rng, &messages, &keypair.secret_key, &parameters)
        .map_err(|_| "issuer signing failed")?;
    signature
        .verify(&messages, keypair.public_key.clone(), parameters.clone())
        .map_err(|_| "issued signature did not verify")?;
    Ok((
        TrustedIssuer {
            parameters,
            key: keypair.public_key.clone(),
        },
        Credential {
            messages,
            signature,
        },
    ))
}

fn required_links() -> MetaStatements {
    let mut links = MetaStatements::new();
    for refs in [
        [(0, SUBJECT), (1, SUBJECT)], // Same holder in both credentials.
        [(0, AMOUNT), (2, 0)],        // Signed income = circuit income.
        [(1, AMOUNT), (2, 1)],        // Signed rent = circuit rent.
    ] {
        links.add_witness_equality(EqualWitnesses(refs.into_iter().collect()));
    }
    links
}

fn disclosed(schema: Fr) -> BTreeMap<usize, Fr> {
    BTreeMap::from([(0, schema)])
}

fn prover_spec(
    setup: &Setup,
    request: &Request,
    links: MetaStatements,
) -> Result<ProofSpec<Bls12_381>> {
    let mut statements = Statements::new();
    for (issuer, schema) in request.issuers.iter().zip(request.schemas) {
        statements.add(PoKBBSSignatureG1Prover::new_statement_from_params(
            issuer.parameters.clone(),
            disclosed(schema),
        ));
    }
    statements.add(
        R1CSCircomProver::new_statement_from_params(
            setup.r1cs.clone(),
            setup.wasm.clone(),
            setup.key.clone(),
        )
        .map_err(|_| "invalid prover circuit statement")?,
    );
    let spec = ProofSpec::new(statements, links, vec![], Some(request.context.clone()));
    spec.validate()
        .map_err(|_| "invalid prover specification")?;
    Ok(spec)
}

fn verifier_spec(key: &VerifyingKey<Bls12_381>, request: &Request) -> Result<ProofSpec<Bls12_381>> {
    let mut statements = Statements::new();
    for (issuer, schema) in request.issuers.iter().zip(request.schemas) {
        statements.add(PoKBBSSignatureG1Verifier::new_statement_from_params(
            issuer.parameters.clone(),
            issuer.key.clone(),
            disclosed(schema),
        ));
    }
    statements.add(
        R1CSCircomVerifier::new_statement_from_params(vec![request.threshold], key.clone())
            .map_err(|_| "invalid verifier circuit statement")?,
    );
    // Never accept a holder-supplied proof specification or equality graph.
    let spec = ProofSpec::new(
        statements,
        required_links(),
        vec![],
        Some(request.context.clone()),
    );
    spec.validate()
        .map_err(|_| "invalid verifier specification")?;
    Ok(spec)
}

fn prove(
    rng: &mut StdRng,
    setup: &Setup,
    request: &Request,
    credentials: &[Credential; 2],
    circuit_values: [Fr; 2],
    links: MetaStatements,
) -> Result<CompositeProof> {
    let mut witnesses = Witnesses::new();
    for credential in credentials {
        witnesses.add(PoKBBSSignatureG1::new_as_witness(
            credential.signature.clone(),
            BTreeMap::from([
                (SUBJECT, credential.messages[SUBJECT]),
                (AMOUNT, credential.messages[AMOUNT]),
            ]),
        ));
    }
    let mut residual = R1CSCircomWitness::new();
    residual.set_private("income".to_owned(), vec![circuit_values[0]]);
    residual.set_private("rent".to_owned(), vec![circuit_values[1]]);
    residual.set_public("threshold".to_owned(), vec![request.threshold]);
    witnesses.add(Witness::R1CSLegoGroth16(residual));
    Proof::new::<StdRng, Blake2b512>(
        rng,
        prover_spec(setup, request, links)?,
        witnesses,
        Some(request.nonce.clone()),
        Default::default(),
    )
    .map(|(proof, _)| proof)
    .map_err(|_| "composite proof creation rejected the witness".to_owned())
}

fn verify(
    rng: &mut StdRng,
    key: &VerifyingKey<Bls12_381>,
    request: &Request,
    proof: CompositeProof,
) -> Result<()> {
    proof
        .verify::<StdRng, Blake2b512>(
            rng,
            verifier_spec(key, request)?,
            Some(request.nonce.clone()),
            Default::default(),
        )
        .map_err(|_| "composite verification rejected the presentation".to_owned())
}

fn verify_bytes(
    rng: &mut StdRng,
    key: &VerifyingKey<Bls12_381>,
    request: &Request,
    encoded: &[u8],
) -> Result<()> {
    // A bounded experiment envelope, not a production network parser.
    if encoded.len() > 65_536 {
        return Err("presentation exceeds the experiment envelope limit".to_owned());
    }
    let mut remaining = encoded;
    let proof = CompositeProof::deserialize_compressed(&mut remaining)
        .map_err(|_| "invalid proof encoding")?;
    if !remaining.is_empty() {
        return Err("trailing bytes after proof".to_owned());
    }
    verify(rng, key, request, proof)
}

fn reject(checks: &mut BTreeMap<String, bool>, name: &str, result: Result<()>) -> Result<()> {
    if result.is_ok() {
        return Err(format!("negative case unexpectedly accepted: {name}"));
    }
    checks.insert(name.to_owned(), true);
    Ok(())
}

fn exercise(rng: &mut StdRng) -> Result<Value> {
    let start = Instant::now();
    let setup = setup(rng)?;
    let setup_ms = start.elapsed().as_secs_f64() * 1000.0;
    let (income_issuer, income) = issue(rng, INCOME_SCHEMA, 7, 60_000)?;
    let (rent_issuer, rent) = issue(rng, RENT_SCHEMA, 7, 2_000)?;
    let credentials = [income, rent];
    let values = [
        credentials[0].messages[AMOUNT],
        credentials[1].messages[AMOUNT],
    ];
    let mut nonce = vec![0; 32];
    rng.fill_bytes(&mut nonce);
    let request = Request {
        issuers: [income_issuer, rent_issuer],
        schemas: [Fr::from(INCOME_SCHEMA), Fr::from(RENT_SCHEMA)],
        threshold: Fr::from(10_000u64),
        nonce,
        context: CONTEXT.to_vec(),
    };
    let start = Instant::now();
    let proof = prove(
        rng,
        &setup,
        &request,
        &credentials,
        values,
        required_links(),
    )?;
    let prove_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut encoded = Vec::new();
    proof
        .serialize_compressed(&mut encoded)
        .map_err(|_| "proof serialization failed")?;
    let start = Instant::now();
    verify_bytes(rng, &setup.key.vk, &request, &encoded)?;
    let verify_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut checks = BTreeMap::new();
    checks.insert("valid_two_issuer_composition".to_owned(), true);

    for (name, changed) in [
        ("changed_threshold", {
            let mut r = request.clone();
            r.threshold += Fr::from(1u64);
            r
        }),
        ("changed_disclosure", {
            let mut r = request.clone();
            r.schemas[0] += Fr::from(1u64);
            r
        }),
        ("changed_nonce", {
            let mut r = request.clone();
            r.nonce[0] ^= 1;
            r
        }),
        ("changed_context", {
            let mut r = request.clone();
            r.context.push(0);
            r
        }),
        ("wrong_issuer", {
            let mut r = request.clone();
            r.issuers.swap(0, 1);
            r
        }),
    ] {
        reject(
            &mut checks,
            name,
            verify(rng, &setup.key.vk, &changed, proof.clone()),
        )?;
    }
    for (name, circuit_values) in [
        ("spliced_income", [values[0] + Fr::from(1u64), values[1]]),
        ("spliced_rent", [values[0], values[1] + Fr::from(1u64)]),
    ] {
        let result = prove(
            rng,
            &setup,
            &request,
            &credentials,
            circuit_values,
            required_links(),
        )
        .and_then(|p| verify(rng, &setup.key.vk, &request, p));
        reject(&mut checks, name, result)?;
    }
    let unlinked = prove(
        rng,
        &setup,
        &request,
        &credentials,
        [values[0] + Fr::from(1u64), values[1]],
        MetaStatements::new(),
    )?;
    // Attack control: the independent statements really do verify under a weak
    // specification. The normal verifier must not accept that specification.
    let mut weak_spec = verifier_spec(&setup.key.vk, &request)?;
    weak_spec.meta_statements = MetaStatements::new();
    unlinked
        .clone()
        .verify::<StdRng, Blake2b512>(
            rng,
            weak_spec,
            Some(request.nonce.clone()),
            Default::default(),
        )
        .map_err(|_| "unlinked attack control should verify its weaker statement")?;
    checks.insert(
        "unlinked_control_verifies_only_the_weaker_statement".to_owned(),
        true,
    );
    reject(
        &mut checks,
        "same_nonce_without_witness_links",
        verify(rng, &setup.key.vk, &request, unlinked),
    )?;
    let second = prove(
        rng,
        &setup,
        &request,
        &credentials,
        values,
        required_links(),
    )?;
    verify(rng, &setup.key.vk, &request, second.clone())?;
    let mut spliced_proof = proof.clone();
    spliced_proof.statement_proofs[2] = second.statement_proofs[2].clone();
    reject(
        &mut checks,
        "spliced_valid_residual_proof",
        verify(rng, &setup.key.vk, &request, spliced_proof),
    )?;

    // Each credential is authentic, but the holder subjects do not match.
    let (other_issuer, other_rent) = issue(rng, RENT_SCHEMA, 8, 2_000)?;
    let mut different_subject = request.clone();
    different_subject.issuers[1] = other_issuer;
    let result = prove(
        rng,
        &setup,
        &different_subject,
        &[credentials[0].clone(), other_rent],
        values,
        required_links(),
    )
    .and_then(|p| verify(rng, &setup.key.vk, &different_subject, p));
    reject(&mut checks, "different_authenticated_subjects", result)?;

    let mut invalid_signature = credentials.clone();
    invalid_signature[0].signature = credentials[1].signature.clone();
    let result = prove(
        rng,
        &setup,
        &request,
        &invalid_signature,
        values,
        required_links(),
    )
    .and_then(|p| verify(rng, &setup.key.vk, &request, p));
    reject(&mut checks, "swapped_issuer_signature", result)?;

    let mut boundary = request.clone();
    boundary.threshold = Fr::from(36_000u64);
    let boundary_proof = prove(
        rng,
        &setup,
        &boundary,
        &credentials,
        values,
        required_links(),
    )?;
    verify(rng, &setup.key.vk, &boundary, boundary_proof)?;
    checks.insert("exact_eligibility_boundary".to_owned(), true);
    boundary.threshold += Fr::from(1u64);
    let result = prove(
        rng,
        &setup,
        &boundary,
        &credentials,
        values,
        required_links(),
    )
    .and_then(|p| verify(rng, &setup.key.vk, &boundary, p));
    reject(&mut checks, "negative_residual_does_not_wrap", result)?;

    // An authentic signature is not permission to bypass the circuit's domain.
    let (large_issuer, large_income) = issue(rng, INCOME_SCHEMA, 7, u64::from(u32::MAX) + 1)?;
    let mut outside_domain = request.clone();
    outside_domain.issuers[0] = large_issuer;
    let large_values = [large_income.messages[AMOUNT], values[1]];
    let result = prove(
        rng,
        &setup,
        &outside_domain,
        &[large_income, credentials[1].clone()],
        large_values,
        required_links(),
    )
    .and_then(|p| verify(rng, &setup.key.vk, &outside_domain, p));
    reject(&mut checks, "signed_income_exceeds_u32", result)?;

    let mut appended = encoded.clone();
    appended.push(0);
    reject(
        &mut checks,
        "trailing_proof_bytes",
        verify_bytes(rng, &setup.key.vk, &request, &appended),
    )?;
    reject(
        &mut checks,
        "truncated_proof",
        verify_bytes(rng, &setup.key.vk, &request, &encoded[..encoded.len() / 2]),
    )?;
    reject(
        &mut checks,
        "unsupported_noir_link",
        select_backend("noir-bn254"),
    )?;

    Ok(json!({
        "experiment": "native-composition-v1", "externally_audited": false,
        "backend": BACKEND, "proof_system_version": "0.34.0", "circom_version": "2.2.2",
        "relation": "income - 12 * rent >= threshold, unsigned 32-bit inputs",
        "provenance": "local correctness smoke; not canonical performance evidence",
        "host": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "debug_assertions": cfg!(debug_assertions),
            "rayon_threads_environment": std::env::var("RAYON_NUM_THREADS").ok() },
        "proof_bytes": encoded.len(), "r1cs_constraints": setup.r1cs.constraints.len(),
        "local_smoke_timings_ms": { "setup": setup_ms, "prove": prove_ms, "verify": verify_ms },
        "checks": checks,
    }))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let backend = match args.as_slice() {
        [] => BACKEND,
        [flag, backend] if flag == "--backend" => backend,
        _ => {
            return Err(format!(
                "usage: sparq-native-composition-spike [--backend {BACKEND}]"
            ))
        }
    };
    select_backend(backend)?;
    let report = exercise(&mut StdRng::from_entropy())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|_| "report serialization failed")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_composition_accepts_valid_and_rejects_tampered_presentations() {
        let report = exercise(&mut StdRng::seed_from_u64(19)).expect("cryptographic checks");
        let checks = report["checks"].as_object().expect("named checks");
        assert!(checks.values().all(|value| value == &Value::Bool(true)));
    }

    #[test]
    fn backend_selection_rejects_unknown_and_cross_field_linkage() {
        assert!(select_backend(BACKEND).is_ok());
        for unsupported in ["noir-bn254", "legogroth16-bn254", "", "unknown"] {
            assert!(select_backend(unsupported).is_err());
        }
    }
}
