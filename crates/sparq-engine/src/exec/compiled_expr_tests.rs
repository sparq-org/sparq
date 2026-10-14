use super::*;
use oxrdf::vocab::xsd;
use spargebra::algebra::{Expression, Function, OrderExpression};

fn int_expr(s: &str) -> Expression {
    Expression::Literal(oxrdf::Literal::new_typed_literal(s, xsd::INTEGER))
}

fn str_expr(s: &str) -> Expression {
    Expression::Literal(oxrdf::Literal::new_simple_literal(s))
}

fn var_expr(v: &str) -> Expression {
    Expression::Variable(oxrdf::Variable::new_unchecked(v))
}

/// `eval_expr` == `eval_compiled` for every row of `b` with expression `e`.
fn assert_compiled_matches_original(graph: &Graph, local: &LocalVocab, b: &Bindings, e: &Expression) {
    let compiled = compile_expr(e, b, local);
    for row in &b.rows {
        let expected = eval_expr(graph, local, b, row, e).expect("eval_expr error");
        let got = eval_compiled(graph, local, b, row, &compiled).expect("eval_compiled error");
        // Compare via string representation (Value is not PartialEq, but Terms are).
        let exp_str = format!("{expected:?}");
        let got_str = format!("{got:?}");
        assert_eq!(
            exp_str, got_str,
            "eval_compiled diverged from eval_expr for expr {e:?} on row {row:?}"
        );
    }
}

fn one_row_graph() -> Graph {
    Graph::load_str(
        "<http://ex/s> <http://ex/p> <http://ex/o> .\n\
             <http://ex/a> <http://ex/n> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        "ntriples",
    )
    .unwrap()
}

/// FILTER parity: apply_filter_scalar (which uses eval_compiled internally) produces
/// the same surviving rows as a reference path that applies eval_expr + effective_boolean
/// directly per row.  sq-7d3dj.4
#[test]
fn compiled_filter_is_byte_identical_to_eval_expr() {
    let g = one_row_graph();
    // Build a small set of rows with inline integer ids so we can filter numerically.
    let n = 30u32;
    let rows: Vec<Row> = (0..n)
        .map(|i| Row::from_slice(&[sparq_core::dict::inline_id_of_int(i as i64).unwrap()]))
        .collect();
    let v = oxrdf::Variable::new_unchecked("x");
    let filters: Vec<Expression> = vec![
        Expression::Greater(Box::new(var_expr("x")), Box::new(int_expr("15"))),
        Expression::Less(Box::new(var_expr("x")), Box::new(int_expr("10"))),
        Expression::Equal(Box::new(var_expr("x")), Box::new(int_expr("7"))),
        Expression::GreaterOrEqual(Box::new(var_expr("x")), Box::new(int_expr("20"))),
    ];
    let local = LocalVocab::default();
    for expr in &filters {
        // Reference path: apply eval_expr + effective_boolean directly to each row.
        let b_ref = Bindings::unsorted(vec![v.clone()], rows.clone());
        let reference_rows: Vec<Row> = b_ref
            .rows
            .iter()
            .filter(|row| {
                eval_expr(&g, &local, &b_ref, row, expr)
                    .map(|val| effective_boolean(&val, crate::EbvSemantics::Rec2013))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        // Compiled path: apply_filter_scalar uses eval_compiled internally.
        let mut b_compiled = Bindings::unsorted(vec![v.clone()], rows.clone());
        apply_filter_scalar(&g, &local, &mut b_compiled, expr).unwrap();

        assert_eq!(
            reference_rows, b_compiled.rows,
            "compiled apply_filter_scalar diverged from eval_expr reference for {expr:?}"
        );
        // Sanity-check: the reference must be non-trivial for each filter expression.
        assert!(
            !reference_rows.is_empty(),
            "filter {expr:?} kept no rows — test is vacuous"
        );
        assert!(
            reference_rows.len() < rows.len(),
            "filter {expr:?} kept all rows — test is vacuous"
        );
    }
}

/// BIND parity: extend_bindings (which uses eval_compiled internally) produces values
/// that are term-identical to what eval_expr yields for the same BIND expression on
/// the same input rows.  sq-7d3dj.4
#[test]
fn compiled_bind_is_byte_identical() {
    let g = one_row_graph();
    let local = LocalVocab::default();

    // Input Bindings: ?v bound to the inline integer ids for 10 and 20.
    let v_var = oxrdf::Variable::new_unchecked("v");
    let v10 = sparq_core::dict::inline_id_of_int(10).unwrap();
    let v20 = sparq_core::dict::inline_id_of_int(20).unwrap();

    // The BIND expression: ?v * 2.
    let bind_expr = Expression::Multiply(Box::new(var_expr("v")), Box::new(int_expr("2")));

    // Reference path: eval_expr on each input row, converted to a Term for comparison.
    let b_ref = Bindings::unsorted(
        vec![v_var.clone()],
        vec![Row::from_slice(&[v10]), Row::from_slice(&[v20])],
    );
    let reference: Vec<Option<Term>> = b_ref
        .rows
        .iter()
        .map(|row| value_as_term(&eval_expr(&g, &local, &b_ref, row, &bind_expr).unwrap()))
        .collect();

    // Compiled path: extend_bindings uses eval_compiled internally.
    let doubled_var = oxrdf::Variable::new_unchecked("doubled");
    let mut local2 = LocalVocab::default();
    let b_out = extend_bindings(
        &g,
        &mut local2,
        Bindings::unsorted(
            vec![v_var.clone()],
            vec![Row::from_slice(&[v10]), Row::from_slice(&[v20])],
        ),
        &doubled_var,
        &bind_expr,
    )
    .unwrap();
    let doubled_col = b_out.col(&doubled_var).unwrap();
    let compiled: Vec<Option<Term>> = b_out
        .rows
        .iter()
        .map(|row| term_of(&g, &local2, row[doubled_col]))
        .collect();

    assert_eq!(
        reference, compiled,
        "extend_bindings result diverged from eval_expr reference for BIND ?v * 2"
    );
    assert_eq!(reference.len(), 2, "expected exactly 2 output rows");
}

/// ORDER BY parity: order_bindings (which uses eval_compiled internally) produces the
/// same row ordering as a reference sort that uses eval_expr to compute sort keys.
/// sq-7d3dj.4
#[test]
fn compiled_order_is_byte_identical() {
    let g = one_row_graph();
    let local = LocalVocab::default();

    // Pre-sort Bindings: ?n bound to inline integer ids in arbitrary order.
    let n_var = oxrdf::Variable::new_unchecked("n");
    let n5 = sparq_core::dict::inline_id_of_int(5).unwrap();
    let n2 = sparq_core::dict::inline_id_of_int(2).unwrap();
    let n8 = sparq_core::dict::inline_id_of_int(8).unwrap();
    let n1 = sparq_core::dict::inline_id_of_int(1).unwrap();
    let pre_sort_rows = vec![
        Row::from_slice(&[n5]),
        Row::from_slice(&[n2]),
        Row::from_slice(&[n8]),
        Row::from_slice(&[n1]),
    ];
    let pre_sort = Bindings::unsorted(vec![n_var.clone()], pre_sort_rows.clone());

    // Reference path: sort rows using eval_expr to compute the ?n sort key.
    let n_expr = Expression::Variable(n_var.clone());
    let mut reference_rows = pre_sort_rows.clone();
    reference_rows.sort_by(|a_row, b_row| {
        let va = eval_expr(&g, &local, &pre_sort, a_row, &n_expr).unwrap();
        let vb = eval_expr(&g, &local, &pre_sort, b_row, &n_expr).unwrap();
        compare_values(&va, &vb).unwrap_or(Ordering::Equal)
    });

    // Compiled path: order_bindings uses eval_compiled internally.
    let mut b = Bindings::unsorted(vec![n_var.clone()], pre_sort_rows.clone());
    order_bindings(&g, &local, &mut b, &[OrderExpression::Asc(n_expr.clone())], None).unwrap();

    assert_eq!(
        reference_rows, b.rows,
        "order_bindings result diverged from eval_expr reference for ORDER BY ASC(?n)"
    );
    // Sanity-check: the sort is non-trivial (input was not already sorted).
    assert_ne!(
        pre_sort_rows, b.rows,
        "pre-sort input happened to be sorted — test is vacuous"
    );
}

/// sq-7d3dj.30.23 — INDEX-CARRY top-k byte-identity: the bounded
/// top-k path (`order_bindings(…, Some(k))`) now carries only the row INDEX in the
/// `keyed` vector (not a cloned `Row`) and gathers the k surviving rows from `b.rows`
/// by index. This test pins that the gathered result is BYTE-IDENTICAL to the full
/// stable sort (`order_bindings(…, None)`) truncated to `[..k]`, for a set with
/// deliberate ORDER BY tie groups (duplicate keys) and `k < n`. It also pins the
/// exact surviving key sequence so a wrong gather (wrong index, wrong prefix, dropped
/// tie-break) turns the test RED.
///
/// MUTATION-VERIFICATION (non-vacuous): the `expected_keys` assertion pins the exact
/// ascending survivor sequence `[1,1,2,3,4]`. If the gather used the wrong index
/// (e.g. `b.rows[*i]` → `b.rows[0]`) or the prefix bound `[..k]` were off, or if the
/// stable-tie order were reversed, the surviving sequence changes and this assertion
/// fails. Reversing the pinned first element (say to `2`) also makes it fail — verified
/// by flipping the expected head and observing RED.
#[test]
fn index_carry_topk_is_byte_identical_to_full_sort_truncated() {
    let g = one_row_graph();
    let local = LocalVocab::default();
    let n_var = oxrdf::Variable::new_unchecked("n");
    let n_expr = Expression::Variable(n_var.clone());

    // Inline-int keys with DELIBERATE tie groups: two 1s, two 4s, plus 3,2,3,4.
    // 8 rows in scrambled input order → the full sort is non-trivial and ties exercise
    // the index tie-breaker. The distinct SECOND column value tags each row so a wrong
    // gather (right key, wrong row) is observable, not masked by identical rows.
    let key = |v: i64| sparq_core::dict::inline_id_of_int(v).unwrap();
    let tag = |v: i64| sparq_core::dict::inline_id_of_int(1000 + v).unwrap();
    let tag_var = oxrdf::Variable::new_unchecked("tag");
    let input_rows = vec![
        Row::from_slice(&[key(3), tag(0)]),
        Row::from_slice(&[key(1), tag(1)]),
        Row::from_slice(&[key(4), tag(2)]),
        Row::from_slice(&[key(1), tag(3)]),
        Row::from_slice(&[key(2), tag(4)]),
        Row::from_slice(&[key(4), tag(5)]),
        Row::from_slice(&[key(3), tag(6)]),
        Row::from_slice(&[key(4), tag(7)]),
    ];
    let n = input_rows.len();
    let vars = vec![n_var.clone(), tag_var.clone()];
    let order = [OrderExpression::Asc(n_expr.clone())];

    // For each k in 1..n (all k < n exercise the bounded index-carry path), the
    // top-k output must equal the full sort truncated to [..k], byte-for-byte.
    let mut full = Bindings::unsorted(vars.clone(), input_rows.clone());
    order_bindings(&g, &local, &mut full, &order, None).unwrap();
    // Sanity: the full sort actually reorders (input was scrambled) — not vacuous.
    assert_ne!(full.rows, input_rows, "full sort was a no-op — test is vacuous");

    for k in 1..n {
        let mut topk = Bindings::unsorted(vars.clone(), input_rows.clone());
        order_bindings(&g, &local, &mut topk, &order, Some(k)).unwrap();
        assert_eq!(
            topk.rows,
            full.rows[..k].to_vec(),
            "index-carry top-k (k={}) diverged from full-sort truncated to [..k]",
            k
        );
    }

    // Pin the EXACT surviving ROWS for k=5 so a gather-index bug goes RED. The
    // expected 5 smallest keys (ascending, ties kept) are [1,1,2,3,4]; the two tied
    // `1` rows keep INPUT order (stable tie-break) — row idx 1 (tag 1) before row idx 3
    // (tag 3) — and among the three `3`/`4` boundary the first `3` (tag 0 then tag 6)
    // and first `4` (tag 2) are the picks. The distinct tag column pins the exact rows,
    // not just the keys, so a right-key/wrong-row gather is caught.
    let mut topk5 = Bindings::unsorted(vars.clone(), input_rows.clone());
    order_bindings(&g, &local, &mut topk5, &order, Some(5)).unwrap();
    let expected5 = vec![
        Row::from_slice(&[key(1), tag(1)]), // first tied 1 (input idx 1)
        Row::from_slice(&[key(1), tag(3)]), // second tied 1 (input idx 3)
        Row::from_slice(&[key(2), tag(4)]),
        Row::from_slice(&[key(3), tag(0)]), // first tied 3 (input idx 0)
        Row::from_slice(&[key(3), tag(6)]), // second tied 3 (input idx 6)
    ];
    assert_eq!(
        topk5.rows, expected5,
        "top-5 must gather exactly these rows (keys [1,1,2,3,3], stable tie order)"
    );

    // DESC boundary: k=1 must gather the single MAX-key row. The three `4`s tie; the
    // FIRST in input order (idx 2, tag 2) wins the stable tie-break. Byte-identical to
    // the full DESC sort's head.
    let tag_col = 1usize;
    let desc = [OrderExpression::Desc(n_expr.clone())];
    let mut full_desc = Bindings::unsorted(vars.clone(), input_rows.clone());
    order_bindings(&g, &local, &mut full_desc, &desc, None).unwrap();
    let mut top1_desc = Bindings::unsorted(vars, input_rows);
    order_bindings(&g, &local, &mut top1_desc, &desc, Some(1)).unwrap();
    assert_eq!(top1_desc.rows.len(), 1, "k=1 must yield exactly one row");
    assert_eq!(
        top1_desc.rows,
        full_desc.rows[..1].to_vec(),
        "DESC k=1 must gather the same head row as the full DESC sort"
    );
    assert_eq!(
        top1_desc.rows[0][tag_col],
        tag(2),
        "DESC k=1 head must be the FIRST max-key (4) row in input order (tag 2)"
    );
}

/// `compile_expr` round-trips: `eval_compiled` matches `eval_expr` for each variant.
/// Tests Variable, NamedNode, Literal, And, Or, Not, Equal, Greater, Add, FunctionCall.
/// sq-7d3dj.4
#[test]
fn compile_expr_matches_eval_expr_for_all_variants() {
    let g = one_row_graph();
    let local = LocalVocab::default();

    // Build a 3-column bindings with a bound IRI, a bound integer, and an unbound slot.
    let iri_id = g.id_of(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/s"))).unwrap();
    let num_id = sparq_core::dict::inline_id_of_int(7).unwrap();
    let va = oxrdf::Variable::new_unchecked("a");
    let vb = oxrdf::Variable::new_unchecked("b");
    let vc = oxrdf::Variable::new_unchecked("c"); // unbound
    let rows = vec![
        Row::from_slice(&[iri_id, num_id, NO_ID]),
        Row::from_slice(&[NO_ID, sparq_core::dict::inline_id_of_int(0).unwrap(), NO_ID]),
    ];
    let b = Bindings::unsorted(vec![va.clone(), vb.clone(), vc.clone()], rows);

    let exprs: Vec<Expression> = vec![
        var_expr("a"),
        var_expr("b"),
        var_expr("c"),    // unbound → Value::Unbound
        var_expr("zzz"),  // never in scope → Value::Unbound
        Expression::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/x")),
        int_expr("42"),
        str_expr("hello"),
        Expression::Bound(vb.clone()),
        Expression::Bound(vc.clone()),
        Expression::Greater(Box::new(var_expr("b")), Box::new(int_expr("3"))),
        Expression::Equal(Box::new(var_expr("b")), Box::new(int_expr("7"))),
        Expression::Not(Box::new(Expression::Equal(Box::new(var_expr("b")), Box::new(int_expr("0"))))),
        Expression::Add(Box::new(var_expr("b")), Box::new(int_expr("1"))),
        Expression::And(
            Box::new(Expression::Bound(vb.clone())),
            Box::new(Expression::Greater(Box::new(var_expr("b")), Box::new(int_expr("0")))),
        ),
        Expression::Or(
            Box::new(Expression::Bound(vc.clone())),
            Box::new(Expression::Greater(Box::new(var_expr("b")), Box::new(int_expr("5")))),
        ),
        Expression::FunctionCall(Function::IsNumeric, vec![var_expr("b")]),
        Expression::FunctionCall(Function::IsIri, vec![var_expr("a")]),
    ];

    for e in &exprs {
        assert_compiled_matches_original(&g, &local, &b, e);
    }
    // Compare all expression lanes with captured outer terms too.
    // Inner columns deliberately contain different values (and UNBOUND).
    let mut captured = LocalVocab::default();
    captured.correlation.insert(va, Term::BlankNode(BlankNode::new_unchecked("outer")));
    captured.correlation.insert(vb, Term::Literal(Literal::new_typed_literal("9007199254740993", xsd::INTEGER)));
    captured.correlation.insert(vc, Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/outer")));
    for e in &exprs {
        assert_compiled_matches_original(&g, &captured, &b, e);
    }
}
