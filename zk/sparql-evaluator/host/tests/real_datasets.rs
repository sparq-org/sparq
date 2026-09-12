// [GPT-6] Real V2 dataset receipts and guest rejection tests; no mock/ignored path.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator::v2::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, Error, Nonces, embedded_artifact, embedded_pin};
use sparq_proved_evaluator_model::v2::*;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, ProofContract, Provenance, RowOrder,
};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Default)]
struct MemoryNonces(BTreeSet<[u8; 32]>);
impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

fn r0vm() -> PathBuf {
    std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required")
}

fn witness(query: &str) -> Witness {
    let dataset = PrivateDataset {
        nquads: concat!(
            "<http://ex/default> <http://ex/p> <http://ex/d> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g2> .\n",
            "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .\n",
        )
        .into(),
        named_graphs: vec![
            "http://ex/g2".into(),
            "http://ex/empty".into(),
            "http://ex/g1".into(),
        ],
        salt: [47; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11DatasetV2,
            query: format!("PREFIX ex: <http://ex/> {query}"),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [41; 32],
        },
        dataset,
    }
}

fn guest() -> AcceptedGuest {
    AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin()).unwrap()
}

#[test]
fn real_v2_catalog_and_nested_graph_result_bind_every_public_expectation() {
    let input = witness(
        "SELECT ?g (COUNT(?s) AS ?n) FROM NAMED ex:g1 FROM NAMED ex:g2 FROM NAMED ex:empty WHERE { GRAPH ?g { OPTIONAL { ?s ex:p ?o } GRAPH ex:empty {} } } GROUP BY ?g ORDER BY ?g",
    );
    let guest = guest();
    let proof = prove_with_artifact(&input, &r0vm(), &guest).expect("real V2 named-dataset proof");
    let expected = evaluate(&input).unwrap();
    let mut nonces = MemoryNonces::default();
    assert_eq!(
        verify_with_artifact(&proof, &input.request, &mut nonces, &guest).unwrap(),
        expected
    );
    assert!(verify_with_artifact(&proof, &input.request, &mut nonces, &guest).is_err());
    let CanonicalResult::Select { order, rows, .. } = &expected.result else {
        panic!("SELECT")
    };
    assert_eq!(*order, RowOrder::Sequence);
    assert_eq!(
        rows,
        &vec![
            vec![
                Some("<http://ex/empty>".into()),
                Some("\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>".into())
            ],
            vec![
                Some("<http://ex/g1>".into()),
                Some("\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>".into())
            ],
            vec![
                Some("<http://ex/g2>".into()),
                Some("\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>".into())
            ],
        ]
    );
    for selector in 0..7 {
        let mut request = input.request.clone();
        match selector {
            0 => request.version = sparq_proved_evaluator_model::VERSION,
            1 => request.query.push(' '),
            2 => request.nonce[0] ^= 1,
            3 => request.policy.max_named_graphs -= 1,
            4 => request.authority = DatasetAuthority::HolderDeclared,
            5 => {
                request.authority = DatasetAuthority::VerifierAgreed {
                    commitment: [53; 32],
                }
            }
            _ => request.contract = ProofContract::SelectedSupport,
        }
        assert!(
            verify_with_artifact(&proof, &request, &mut MemoryNonces::default(), &guest).is_err()
        );
    }
    let v1_request = sparq_proved_evaluator_model::Request {
        version: sparq_proved_evaluator_model::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: sparq_proved_evaluator_model::Dialect::SparqSparql11SnapshotV1,
        query: input.request.query.clone(),
        authority: input.request.authority.clone(),
        policy: sparq_proved_evaluator_model::Policy::default(),
        nonce: input.request.nonce,
    };
    assert!(
        sparq_proved_evaluator::verify_with_artifact(
            &proof,
            &v1_request,
            &mut MemoryNonces::default(),
            &guest,
        )
        .is_err(),
        "a genuine V2 receipt cannot be interpreted as V1"
    );
    let mut tampered = proof.clone();
    tampered.receipt.journal.bytes[0] ^= 1;
    assert!(
        verify_with_artifact(
            &tampered,
            &input.request,
            &mut MemoryNonces::default(),
            &guest
        )
        .is_err()
    );
}

#[test]
fn real_v2_holder_declared_absence_respects_from_named_scope() {
    let mut input = witness(
        "ASK FROM NAMED ex:empty { { GRAPH ex:empty { GRAPH ex:g2 { ?s ex:p ?o } } } UNION { BIND((\"1200\"^^<http://www.w3.org/2001/XMLSchema#byte> + 0) > 5 AS ?invalid) FILTER(?invalid) } UNION { FILTER(\"2024-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> = \"2024-01-01T00:00:00.000000001Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>) } }",
    );
    input.request.authority = DatasetAuthority::HolderDeclared;
    input.request.nonce = [43; 32];
    let guest = guest();
    let proof =
        prove_with_artifact(&input, &r0vm(), &guest).expect("real V2 holder-declared proof");
    let journal =
        verify_with_artifact(&proof, &input.request, &mut MemoryNonces::default(), &guest).unwrap();
    assert_eq!(journal, evaluate(&input).unwrap());
    assert_eq!(journal.result, CanonicalResult::Ask(false));
    assert_eq!(journal.provenance, Provenance::HolderDeclaredOnly);
    let mut stronger = input.request;
    stronger.authority = DatasetAuthority::VerifierAgreed {
        commitment: journal.dataset_commitment,
    };
    assert!(verify_with_artifact(&proof, &stronger, &mut MemoryNonces::default(), &guest).is_err());
}

#[test]
fn actual_v2_guest_rejects_catalog_omissions_invalid_source_and_cross_version() {
    let base = witness("ASK { GRAPH ex:empty {} }");
    let executor = ExternalProver::new("real-sparq-v2-catalog-negative", r0vm());
    let execute = |input: &Witness| {
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(input)
            .unwrap()
            .build()
            .unwrap();
        executor.execute(env, embedded_artifact())
    };
    execute(&base).expect("positive actual V2 guest control");
    let mut cases = Vec::new();
    let mut omitted = base.clone();
    omitted
        .dataset
        .named_graphs
        .retain(|name| name != "http://ex/empty");
    cases.push(omitted);
    let mut undeclared = base.clone();
    undeclared.request.authority = DatasetAuthority::HolderDeclared;
    undeclared
        .dataset
        .named_graphs
        .retain(|name| name != "http://ex/g2");
    cases.push(undeclared);
    let mut duplicate = base.clone();
    duplicate.request.authority = DatasetAuthority::HolderDeclared;
    duplicate
        .dataset
        .named_graphs
        .push("http://ex/empty".into());
    cases.push(duplicate);
    let mut blank = base.clone();
    blank.request.authority = DatasetAuthority::HolderDeclared;
    blank
        .dataset
        .nquads
        .push_str("<http://ex/a> <http://ex/p> <http://ex/b> _:graph .");
    cases.push(blank);
    let mut exhausted = base.clone();
    exhausted.request.authority = DatasetAuthority::HolderDeclared;
    exhausted.request.policy.max_named_graphs = 2;
    cases.push(exhausted);
    let mut cross_version = base;
    cross_version.request.version = sparq_proved_evaluator_model::VERSION;
    cases.push(cross_version);
    let exclusions: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/v2-admission-rejections.json")).unwrap();
    let excluded_cases = exclusions["cases"].as_array().unwrap();
    assert_eq!(excluded_cases.len(), 5, "execute every V2 query exclusion");
    for case in excluded_cases {
        cases.push(witness(case["query"].as_str().unwrap()));
    }
    for input in cases {
        let error = execute(&input).expect_err("invalid V2 relation must reject");
        let rejection = format!("{error:#}");
        assert!(
            rejection.contains("Guest panicked:")
                && rejection.contains("bounded exact-dataset relation rejected"),
            "unexpected executor failure: {rejection}"
        );
    }
    let mut trailing: Vec<u8> = risc0_zkvm::serde::to_vec(&witness("ASK {}"))
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    trailing.extend_from_slice(&0_u32.to_le_bytes());
    for bytes in [
        7_u32.to_le_bytes().to_vec(),
        vec![0_u8; sparq_proved_evaluator_model::MAX_WITNESS_BYTES + 1],
        vec![1_u8],
        trailing,
    ] {
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write_slice(&bytes)
            .build()
            .unwrap();
        let error = executor
            .execute(env, embedded_artifact())
            .expect_err("unframed, excessive, trailing or unknown-version input must reject");
        let rejection = format!("{error:#}");
        assert!(
            rejection.contains("Guest panicked:")
                && rejection.contains("bounded exact-dataset relation rejected"),
            "unexpected executor failure: {rejection}"
        );
    }
}

#[test]
fn v2_wire_version_and_maximum_input_fit_the_bounded_transport() {
    let mut input = witness("ASK {}");
    input.dataset.named_graphs = (0..MAX_NAMED_GRAPHS)
        .map(|index| format!("http://ex/graph-{index}"))
        .collect();
    let names_bytes: usize = input.dataset.named_graphs.iter().map(String::len).sum();
    input.dataset.nquads =
        " ".repeat(sparq_proved_evaluator_model::MAX_DATASET_BYTES as usize - names_bytes);
    input.request.query = " ".repeat(sparq_proved_evaluator_model::MAX_QUERY_BYTES);
    let encoded = risc0_zkvm::serde::to_vec(&input).unwrap();
    assert_eq!(encoded[0], VERSION);
    assert!(encoded.len() * 4 <= sparq_proved_evaluator_model::MAX_WITNESS_BYTES);
    let decoded: Witness = risc0_zkvm::serde::from_slice(&encoded).unwrap();
    assert_eq!(decoded.request, input.request);
    assert_eq!(decoded.dataset.nquads, input.dataset.nquads);
    assert_eq!(decoded.dataset.named_graphs, input.dataset.named_graphs);
    use sparq_proved_evaluator_model as v1;
    let input_v1 = v1::Witness {
        request: v1::Request {
            version: v1::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v1::Dialect::SparqSparql11SnapshotV1,
            query: input.request.query,
            authority: DatasetAuthority::VerifierAgreed {
                commitment: [53; 32],
            },
            policy: v1::Policy::default(),
            nonce: [43; 32],
        },
        dataset: v1::PrivateDataset {
            ntriples: " ".repeat(v1::MAX_DATASET_BYTES as usize),
            salt: [47; 32],
        },
    };
    let encoded = risc0_zkvm::serde::to_vec(&input_v1).unwrap();
    assert_eq!(
        encoded[0],
        v1::VERSION,
        "no wrapper changes the V1 wire layout"
    );
    assert!(encoded.len() * 4 <= v1::MAX_WITNESS_BYTES);
    let decoded: v1::Witness = risc0_zkvm::serde::from_slice(&encoded).unwrap();
    assert_eq!(decoded.request, input_v1.request);
    assert_eq!(decoded.dataset.ntriples, input_v1.dataset.ntriples);
}
