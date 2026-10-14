//! Seeded property tests for Arrow IPC identity over generated query results.
#![cfg(feature = "ipc")]

use std::collections::BTreeSet;

use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sparq_arrow::{from_ipc_bytes, ipc_variables_from_bytes, to_ipc_bytes};
use sparq_engine::QueryResult;

// Construct every RDF term variant independently of the IPC implementation,
// including the literal forms and RDF-star triple terms supported by the Arrow schema.
fn ground_term() -> impl Strategy<Value = Term> {
    let lexical = "[a-zA-Z0-9 _-]{0,24}";
    prop_oneof![
        any::<u32>().prop_map(|n| Term::NamedNode(
            NamedNode::new(format!("https://example.test/resource/{n}")).unwrap()
        )),
        any::<u32>().prop_map(|n| Term::BlankNode(BlankNode::new(format!("b{n}")).unwrap())),
        prop_oneof![
            lexical.prop_map(|value| Term::Literal(Literal::new_simple_literal(value))),
            any::<i64>().prop_map(|value| Term::Literal(Literal::from(value))),
            lexical.prop_map(|value| Term::Literal(
                Literal::new_language_tagged_literal(value, "en").unwrap()
            )),
            lexical.prop_map(|value| Term::Literal(
                Literal::new_directional_language_tagged_literal(value, "ar", BaseDirection::Rtl)
                    .unwrap()
            )),
        ],
        any::<u32>().prop_map(|n| Term::Triple(Box::new(Triple::new(
            NamedNode::new(format!("https://example.test/s/{n}")).unwrap(),
            NamedNode::new("https://example.test/p").unwrap(),
            Literal::from(i64::from(n)),
        )))),
    ]
}

fn query_result() -> impl Strategy<Value = QueryResult> {
    // Preserve generated insertion order while removing duplicate variable names. This
    // exercises schema-order fidelity rather than limiting inputs to sorted variables.
    vec(any::<u16>(), 1..6)
        .prop_map(|ids| {
            let mut seen = BTreeSet::new();
            ids.into_iter()
                .filter(|id| seen.insert(*id))
                .map(|id| Variable::new(format!("v{id}")).unwrap())
                .collect::<Vec<_>>()
        })
        .prop_flat_map(|vars| {
            let width = vars.len();
            vec(vec(prop::option::of(ground_term()), width), 0..24).prop_map(move |rows| {
                QueryResult {
                    vars: vars.clone(),
                    rows,
                }
            })
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x1FC0_2026),
        ..ProptestConfig::default()
    })]

    /// Mutation witness: changing a variable position, term, null cell, or row
    /// in either expected value makes the corresponding exact identity assertion fail.
    #[test]
    fn ipc_round_trip_and_schema_reader_are_exact(result in query_result()) {
        let bytes = to_ipc_bytes(&result).unwrap();
        let restored = from_ipc_bytes(&bytes).unwrap();
        let variables = ipc_variables_from_bytes(&bytes).unwrap();

        prop_assert_eq!(&restored.vars, &result.vars);
        prop_assert_eq!(&restored.rows, &result.rows);
        prop_assert_eq!(&variables, &result.vars);
    }
}
