//! Seeded property tests for flattened-CSV identity over generated query
//! results, mirroring the IPC/Parquet suites with CSV-hostile lexical content
//! (delimiters, quotes, newlines, whitespace) in the literal strategy.
#![cfg(feature = "csv")]

use std::collections::BTreeSet;

use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sparq_arrow::{csv_variables_from_bytes, from_csv_bytes, to_csv_bytes};
use sparq_engine::QueryResult;

// Construct every RDF term variant independently of the CSV implementation. The
// lexical class deliberately includes the CSV delimiter, the quote character, and
// newlines so the container's quoting is exercised, plus the empty string so the
// unbound-vs-empty-literal distinction is generated, not hand-picked.
fn ground_term() -> impl Strategy<Value = Term> {
    let lexical = "[a-zA-Z0-9 _,\"\n-]{0,24}";
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
    // exercises header-order fidelity rather than limiting inputs to sorted variables.
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
        rng_seed: RngSeed::Fixed(0x5C4A_2026),
        ..ProptestConfig::default()
    })]

    /// Mutation witness: changing a variable position, term, null cell, or
    /// row in either expected value makes the corresponding exact identity assertion
    /// fail.
    #[test]
    fn csv_roundtrip_and_header_reader_are_exact(result in query_result()) {
        let bytes = to_csv_bytes(&result).unwrap();
        let restored = from_csv_bytes(&bytes).unwrap();
        let variables = csv_variables_from_bytes(&bytes).unwrap();

        prop_assert_eq!(&restored.vars, &result.vars);
        prop_assert_eq!(&restored.rows, &result.rows);
        prop_assert_eq!(&variables, &result.vars);
    }
}
