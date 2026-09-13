// [GPT-6] Original complete synthetic fixtures with an explicit fixed context.
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v4};

pub fn corpus() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../fixtures/now-context.json")).unwrap()
}

pub fn input(case: &serde_json::Value, agreed: bool, nonce: u8) -> v4::Witness {
    let mut witness = v4::Witness {
        request: v4::Request {
            version: v4::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v4::Dialect::SparqSparql11NowContextV4,
            query: case["query"].as_str().unwrap().into(),
            context: v4::ExecutionContext {
                now: v4::NowContext {
                    datetime: corpus()["now"].as_str().unwrap().into(),
                },
            },
            authority: DatasetAuthority::HolderDeclared,
            policy: v4::Policy::default(),
            nonce: [nonce; 32],
        },
        dataset: v4::PrivateDataset {
            nquads: case["nquads"].as_str().unwrap().into(),
            named_graphs: vec![],
            salt: [89; 32],
        },
    };
    if agreed {
        witness.request.authority = DatasetAuthority::VerifierAgreed {
            commitment: v4::dataset_commitment(&witness.dataset, &witness.request.policy).unwrap(),
        };
    }
    witness
}
