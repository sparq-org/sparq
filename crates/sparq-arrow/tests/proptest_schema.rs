//! Property tests for Arrow batch schema, shape, and cell validity.
#![cfg(feature = "arrow")]

use std::collections::BTreeSet;

use arrow_array::Array;
use oxrdf::{BlankNode, Literal, NamedNode, Term, Variable};
use proptest::collection::{btree_set, vec};
use proptest::prelude::*;
use sparq_arrow::{term_schema, to_record_batch};
use sparq_engine::QueryResult;

// Keep generation independent of the exporter: each variant is built through
// oxrdf's checked constructors, while validity expectations come from the input cells.
fn ground_term() -> impl Strategy<Value = Term> {
    prop_oneof![
        any::<u32>().prop_map(|n| Term::NamedNode(
            NamedNode::new(format!("https://example.test/resource/{n}")).unwrap()
        )),
        any::<u32>().prop_map(|n| Term::BlankNode(BlankNode::new(format!("b{n}")).unwrap())),
        ".{0,24}".prop_map(|value| Term::Literal(Literal::new_simple_literal(value))),
        any::<i64>().prop_map(|value| Term::Literal(Literal::from(value))),
    ]
}

fn query_result() -> impl Strategy<Value = QueryResult> {
    btree_set(any::<u16>(), 0..6).prop_flat_map(|ids: BTreeSet<u16>| {
        let vars: Vec<_> = ids
            .into_iter()
            .map(|id| Variable::new(format!("v{id}")).unwrap())
            .collect();
        let width = vars.len();
        vec(vec(prop::option::of(ground_term()), width), 0..20).prop_map(move |rows| QueryResult {
            vars: vars.clone(),
            rows,
        })
    })
}

proptest! {
    #[test]
    fn record_batch_preserves_schema_shape_and_null_mask(result in query_result()) {
        let expected_schema = term_schema(&result.vars).unwrap();
        let expected_rows = result.rows.len();
        let batch = to_record_batch(&result).unwrap();
        let batch_schema = batch.schema();

        prop_assert_eq!(batch_schema.as_ref(), &expected_schema);
        prop_assert_eq!(batch.num_rows(), expected_rows);
        prop_assert_eq!(batch.num_columns(), result.vars.len());

        for (column_index, var) in result.vars.iter().enumerate() {
            prop_assert_eq!(batch_schema.field(column_index).name(), var.as_str());
            let column = batch.column(column_index);
            for (row_index, row) in result.rows.iter().enumerate() {
                prop_assert_eq!(
                    column.is_null(row_index),
                    row[column_index].is_none(),
                    "validity mismatch at row {}, column {}",
                    row_index,
                    column_index
                );
            }
        }
    }
}
