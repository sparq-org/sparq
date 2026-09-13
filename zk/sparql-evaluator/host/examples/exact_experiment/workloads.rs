// [GPT-6] Materialized named-organization data with independently derived count goldens.
use serde::{Deserialize, Serialize};
use sparq_proved_evaluator_model::{CanonicalResult, RowOrder};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Organization {
    pub seed: String,
    pub scale: usize,
    pub nquads: String,
    pub named_graphs: Vec<String>,
    pub query: String,
    pub expected: CanonicalResult,
}

impl Organization {
    pub fn generated(seed: &str, scale: usize) -> Result<Self, &'static str> {
        if seed.len() != 16
            || !seed
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(2..=128).contains(&scale)
        {
            return Err("organization seed or source-quad capacity rejected");
        }
        let namespace = format!("urn:sparq:bench:{seed}:");
        let named_graphs: Vec<_> = ["department-a", "department-b", "empty"]
            .iter()
            .map(|name| format!("{namespace}{name}"))
            .collect();
        let mut nquads = String::new();
        for (department, graph) in named_graphs.iter().take(2).enumerate() {
            for index in 0..scale {
                nquads.push_str(&format!(
                    "<{namespace}member-{department}-{index}> <urn:member> <urn:Employee> <{graph}> .\n"
                ));
            }
        }
        let clauses = named_graphs
            .iter()
            .map(|graph| format!("FROM NAMED <{graph}> "))
            .collect::<String>();
        let query = format!(
            "SELECT ?g (COUNT(?s) AS ?n) {clauses}WHERE {{ GRAPH ?g {{ OPTIONAL {{ ?s <urn:member> ?kind }} }} }} GROUP BY ?g ORDER BY ?g"
        );
        let rows = named_graphs
            .iter()
            .enumerate()
            .map(|(index, graph)| {
                vec![
                    Some(format!("<{graph}>")),
                    Some(format!(
                        "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                        if index == 2 { 0 } else { scale }
                    )),
                ]
            })
            .collect();
        let expected = CanonicalResult::Select {
            variables: vec!["g".into(), "n".into()],
            order: RowOrder::Sequence,
            rows,
        };
        Ok(Self {
            seed: seed.into(),
            scale,
            nquads,
            named_graphs,
            query,
            expected,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if *self != Self::generated(&self.seed, self.scale)? {
            return Err("materialized organization differs from the declared synthetic profile");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_form_counts_preserve_empty_graph_and_full_scale() {
        for scale in [2, 8, 32, 128] {
            let organization = Organization::generated("0123456789abcdef", scale).unwrap();
            organization.validate().unwrap();
            assert_eq!(organization.nquads.lines().count(), scale * 2);
            let mut truncated = organization.clone();
            truncated.nquads.pop();
            assert!(truncated.validate().is_err());
            let mut wrong = organization;
            wrong.expected = CanonicalResult::Ask(false);
            assert!(wrong.validate().is_err());
        }
        assert!(Organization::generated("0123456789abcdef", 129).is_err());
    }
}
