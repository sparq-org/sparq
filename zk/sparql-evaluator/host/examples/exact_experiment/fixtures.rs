// [GPT-6] Fixed synthetic contracts; host oracles are tests, never proof substitutes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::v2::{
    Dialect, Policy, PrivateDataset, Request, VERSION, Witness, dataset_commitment,
};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract, RowOrder};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum Fixture {
    DefaultSelect,
    FalseAsk,
    NamedCatalog,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum Authority {
    VerifierAgreed,
    HolderDeclared,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Case {
    pub fixture: Fixture,
    pub authority: Authority,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema_version: u32,
    pub run_id: String,
    pub repetitions: u32,
    pub warmups: u32,
    pub cases: Vec<Case>,
}

impl Manifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.run_id.len() != 32
            || !self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !(1..=3).contains(&self.repetitions)
            || self.warmups > 1
            || self.cases.is_empty()
            || self.cases.len() > 6
        {
            return Err("invalid or over-capacity experiment manifest");
        }
        let mut seen = BTreeSet::new();
        if self
            .cases
            .iter()
            .any(|case| !seen.insert((case.fixture, case.authority)))
        {
            return Err("duplicate experiment contract");
        }
        Ok(())
    }
}

pub(super) fn witness(case: &Case, nonce: [u8; 32]) -> Witness {
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
        // Synthetic, published test salt. It is not a production blinding secret.
        salt: [47; 32],
    };
    let policy = Policy::default();
    let authority = match case.authority {
        Authority::VerifierAgreed => DatasetAuthority::VerifierAgreed {
            commitment: dataset_commitment(&dataset, &policy).expect("fixed valid fixture"),
        },
        Authority::HolderDeclared => DatasetAuthority::HolderDeclared,
    };
    let query = match case.fixture {
        Fixture::DefaultSelect => "SELECT ?s ?o WHERE { ?s ex:p ?o } ORDER BY ?s ?o",
        Fixture::FalseAsk => "ASK { ?s ex:missing ?o }",
        Fixture::NamedCatalog => {
            "SELECT ?g (COUNT(?s) AS ?n) FROM NAMED ex:g1 FROM NAMED ex:g2 FROM NAMED ex:empty WHERE { GRAPH ?g { OPTIONAL { ?s ex:p ?o } GRAPH ex:empty {} } } GROUP BY ?g ORDER BY ?g"
        }
    };
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11DatasetV2,
            query: format!("PREFIX ex: <http://ex/> {query}"),
            authority,
            policy,
            nonce,
        },
        dataset,
    }
}

pub(super) fn expected(fixture: Fixture) -> CanonicalResult {
    match fixture {
        Fixture::DefaultSelect => CanonicalResult::Select {
            variables: vec!["s".into(), "o".into()],
            order: RowOrder::Sequence,
            rows: vec![vec![
                Some("<http://ex/default>".into()),
                Some("<http://ex/d>".into()),
            ]],
        },
        Fixture::FalseAsk => CanonicalResult::Ask(false),
        Fixture::NamedCatalog => CanonicalResult::Select {
            variables: vec!["g".into(), "n".into()],
            order: RowOrder::Sequence,
            rows: ["empty", "g1", "g2"]
                .into_iter()
                .enumerate()
                .map(|(n, graph)| {
                    vec![
                        Some(format!("<http://ex/{graph}>")),
                        Some(format!(
                            "\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>"
                        )),
                    ]
                })
                .collect(),
        },
    }
}

pub(super) fn nonce(run_id: &str, case: usize, ordinal: u32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"sparq-exact-experiment-synthetic-nonce-v1\0");
    hash.update(run_id.as_bytes());
    hash.update((case as u64).to_le_bytes());
    hash.update(ordinal.to_le_bytes());
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_contracts_match_fixed_goldens_and_distinct_authority_hashes() {
        for fixture in [
            Fixture::DefaultSelect,
            Fixture::FalseAsk,
            Fixture::NamedCatalog,
        ] {
            let mut hashes = BTreeSet::new();
            for authority in [Authority::VerifierAgreed, Authority::HolderDeclared] {
                let input = witness(&Case { fixture, authority }, [1; 32]);
                let actual = sparq_proved_evaluator_model::v2::evaluate(&input).unwrap();
                assert_eq!(actual.result, expected(fixture));
                hashes.insert(actual.request_digest);
            }
            assert_eq!(hashes.len(), 2);
        }
    }

    #[test]
    fn manifest_fails_closed_and_nonces_cover_all_permitted_samples() {
        let valid = include_str!("../../../experiments/smoke.json");
        let mut manifest: Manifest = serde_json::from_str(valid).unwrap();
        manifest.validate().unwrap();
        manifest.cases.push(manifest.cases[0].clone());
        assert!(manifest.validate().is_err());
        manifest.cases.pop();
        for repetitions in [0, 4, u32::MAX] {
            manifest.repetitions = repetitions;
            assert!(manifest.validate().is_err());
        }
        let mut unknown: serde_json::Value = serde_json::from_str(valid).unwrap();
        unknown["cases"][0]["fixture"] = "arbitrary_query".into();
        assert!(serde_json::from_value::<Manifest>(unknown).is_err());
        let nonces: BTreeSet<_> = (0..6)
            .flat_map(|case| {
                (0..4).map(move |ordinal| nonce("0123456789abcdef0123456789abcdef", case, ordinal))
            })
            .collect();
        assert_eq!(nonces.len(), 24);
        assert!(!nonces.contains(&[0; 32]));
    }
}
