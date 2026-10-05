// [GPT-6] Synthetic V3 fixtures shared by actual guest and genuine receipt tests.
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3};

pub fn witness(query: &str, source: &str, named: &[&str]) -> v3::Witness {
    v3::Witness {
        request: v3::Request {
            version: v3::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: format!("PREFIX ex:<http://ex/> {query}"),
            authority: DatasetAuthority::HolderDeclared,
            policy: v3::Policy::default(),
            nonce: [67; 32],
        },
        dataset: v3::PrivateDataset {
            nquads: source.into(),
            named_graphs: named.iter().map(|name| (*name).into()).collect(),
            salt: [71; 32],
        },
    }
}

pub fn accepted(input: &mut v3::Witness) {
    input.request.authority = DatasetAuthority::VerifierAgreed {
        commitment: v3::dataset_commitment(&input.dataset, &input.request.policy).unwrap(),
    };
}

pub const SHARED_NODE: &str = "_:private <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n_:private <http://ex/p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
pub const BAG: &str = "SELECT ?s ?n ?u { ?s ex:p ?n VALUES ?repeat { 1 1 } }";
pub const CONSTRUCT_SOURCE: &str = "_:tc0_0_0 <http://ex/p> <http://ex/a> .";
pub const CONSTRUCT: &str = "CONSTRUCT { _:new ex:p ?s; ex:q ?s . ?s ex:r ex:a . ?missing ex:p ex:a . ?illegal ex:p ex:a } WHERE { ?s ex:p ex:a VALUES ?repeat { 1 1 } BIND(1 AS ?illegal) }";
pub const DESCRIBE_SOURCE: &str = "<http://ex/a> <http://ex/p> _:x .\n_:x <http://ex/p> _:y .\n_:y <http://ex/p> _:x .\n_:y <http://ex/q> <http://ex/b> .\n<http://ex/inbound> <http://ex/p> <http://ex/a> .\n<http://ex/b> <http://ex/q> <http://ex/outside> .";
