//! Seeded property test for Parquet identity over generated query results.
#![cfg(feature = "parquet")]

use std::collections::BTreeSet;

use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};
use proptest::collection::{btree_set, vec};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sparq_arrow::{from_parquet_bytes, to_parquet_bytes};
use sparq_engine::QueryResult;

// These constructors are independent of the Parquet implementation and cover
// every supported RDF-term arm, including directional literals and triple terms.
fn ground_term() -> impl Strategy<Value = Term> {
    let lexical = "[a-zA-Z0-9 _-]{0,24}";
    prop_oneof![
        any::<u32>().prop_map(|n| Term::NamedNode(
            NamedNode::new(format!("https://example.test/resource/{n}")).unwrap()
        )),
        any::<u32>().prop_map(|n| Term::BlankNode(BlankNode::new(format!("b{n}")).unwrap())),
        lexical.prop_map(|value| Term::Literal(Literal::new_simple_literal(value))),
        any::<i64>().prop_map(|value| Term::Literal(Literal::from(value))),
        lexical.prop_map(|value| Term::Literal(
            Literal::new_language_tagged_literal(value, "en").unwrap()
        )),
        lexical.prop_map(|value| Term::Literal(
            Literal::new_directional_language_tagged_literal(value, "ar", BaseDirection::Rtl)
                .unwrap()
        )),
        any::<u32>().prop_map(|n| Term::Triple(Box::new(Triple::new(
            NamedNode::new(format!("https://example.test/s/{n}")).unwrap(),
            NamedNode::new("https://example.test/p").unwrap(),
            Literal::from(i64::from(n)),
        )))),
    ]
}

fn query_result() -> impl Strategy<Value = QueryResult> {
    btree_set(any::<u16>(), 1..6).prop_flat_map(|ids: BTreeSet<u16>| {
        let vars: Vec<_> = ids
            .into_iter()
            .map(|id| Variable::new(format!("v{id}")).unwrap())
            .collect();
        let width = vars.len();
        vec(vec(prop::option::of(ground_term()), width), 0..24).prop_map(move |rows| QueryResult {
            vars: vars.clone(),
            rows,
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x5A17_2026),
        ..ProptestConfig::default()
    })]

    #[test]
    fn parquet_round_trip_is_identity(result in query_result()) {
        let bytes = to_parquet_bytes(&result).unwrap();
        let restored = from_parquet_bytes(&bytes).unwrap();
        prop_assert_eq!(restored.vars, result.vars);
        prop_assert_eq!(restored.rows, result.rows);
    }
}
