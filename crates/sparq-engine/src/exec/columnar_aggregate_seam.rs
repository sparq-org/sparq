use super::*;

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// Build `n` triples `ex:s{i} ex:score {i}` (integer literal → inline id).
/// Returns (graph, subject ids [dict IRIs], score ids [inline integers]).
fn scores_graph(n: u32) -> (Graph, Vec<Id>, Vec<Id>) {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/score> \"{i}\"^^<{XSD_INT}> .\n"));
    }
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let subj: Vec<Id> = (0..n)
        .map(|i| g.id_of(&Term::NamedNode(oxrdf::NamedNode::new(format!("http://ex/s{i}")).unwrap())).unwrap())
        .collect();
    let score: Vec<Id> = (0..n)
        .map(|i| g.id_of(&Term::Literal(Literal::new_typed_literal(i.to_string(), xsd::INTEGER))).unwrap())
        .collect();
    (g, subj, score)
}

fn var(name: &str) -> Variable {
    Variable::new(name).unwrap()
}

/// TRUE scalar oracle: calls `eval_aggregate` directly, bypassing the columnar seam.
/// Returns `(rows, local_vocab)` — caller uses `local_vocab` for [`term_of`] decoding.
///
/// Under `feature="vectorized"`, calling `group_aggregate` for the "reference" side is
/// vacuous — it takes the columnar path too, making a columnar-vs-columnar comparison that
/// cannot catch a columnar mutation. This helper replicates the sequential scalar fallback
/// of `group_aggregate` WITHOUT the columnar seam.
/// (C2-fix sq-pntvh.4 adversarial review)
fn scalar_oracle(
    g: &Graph,
    b: &Bindings,
    group_vars: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
) -> (Vec<Row>, LocalVocab) {
    let key_cols: Vec<Option<usize>> = group_vars.iter().map(|v| b.col(v)).collect();
    let (mut order, mut members) = build_groups(b, &key_cols);
    if group_vars.is_empty() && order.is_empty() {
        order.push(Key::new());
        members.push(Vec::new());
    }
    let mut local = LocalVocab::default();
    let mut rows = Vec::with_capacity(order.len());
    for (key, mems) in order.iter().zip(&members) {
        let mut row = Row::from_slice(key);
        for (_, agg) in aggregates {
            let v = eval_aggregate(g, &local, b, mems, agg)
                .expect("scalar oracle eval_aggregate must succeed");
            let id = value_to_id(g, &mut local, &v);
            row.push(id);
        }
        rows.push(row);
    }
    (rows, local)
}

/// Decode rows to `Option<Term>` slices for full-term comparison
/// (lexical + datatype, not raw ids or f64 values).
fn rows_to_terms(g: &Graph, local: &LocalVocab, rows: &[Row]) -> Vec<Vec<Option<oxrdf::Term>>> {
    rows.iter()
        .map(|row| row.iter().map(|&id| term_of(g, local, id)).collect())
        .collect()
}

/// Run the columnar path (via `group_aggregate`) and the TRUE scalar oracle, then
/// assert their outputs are FULL-TERM-identical (lexical + datatype).
///
/// `group_aggregate` — under `feature="vectorized"` — tries the columnar seam first.
/// The scalar oracle calls `eval_aggregate` directly, so the comparison is
/// columnar-vs-scalar, not columnar-vs-columnar. A datatype mutation in the columnar
/// path (e.g., SUM returning `Num::Double` instead of `Num::Int`) produces a different
/// term (different xsd: datatype) and fails.
/// (C2-fix sq-pntvh.4 adversarial review)
fn check_agg_byte_identical(
    g: &Graph,
    b: Bindings,
    group_vars: &[Variable],
    aggregates: Vec<(Variable, AggregateExpression)>,
) {
    // TRUE scalar reference: calls eval_aggregate directly.
    let (scalar_rows, scalar_local) = scalar_oracle(g, &b, group_vars, &aggregates);

    // Columnar path: group_aggregate (tries columnar under vectorized, falls back otherwise).
    let b_col = Bindings::unsorted(b.vars.clone(), b.rows.clone());
    let mut local_col = LocalVocab::default();
    let col_out = group_aggregate(g, &mut local_col, b_col, group_vars, &aggregates)
        .expect("columnar group_aggregate must succeed");

    // Compare as full Terms (lexical + datatype), not raw ids.
    // Raw ids across different LocalVocab instances diverge for non-inline terms.
    let scalar_terms = rows_to_terms(g, &scalar_local, &scalar_rows);
    let col_terms = rows_to_terms(g, &local_col, &col_out.rows);
    assert_eq!(
        scalar_terms, col_terms,
        "columnar and scalar group_aggregate must produce FULL-TERM-identical output \
             (lexical + datatype)"
    );
}

/// T5 (byte-identity + non-vacuity): whole-dataset SUM/COUNT/MIN/MAX over an
/// all-inline-integer column: (a) `columnar_aggregate` returns `Some` (path engaged),
/// (b) the aggregate result decodes to the scalar-expected value, and (c) the columnar
/// result is FULL-TERM-identical to the TRUE scalar oracle (catches datatype mutations).
///
/// Scores 0..=299: SUM=44850 (xsd:integer), COUNT=300 (xsd:integer), MIN=0, MAX=299.
/// n=300 ensures the batch exceeds VEC_MIN_BATCH (256) so columnar_aggregate engages.
/// A mutation returning `Num::Double` instead of `Num::Int` for SUM changes the xsd:
/// datatype and fails (c); a MIN→MAX mutation changes the numeric value and fails (b).
/// (C2-fix sq-pntvh.4 adversarial review)
#[test]
fn t5_whole_dataset_agg_byte_identical_to_scalar() {
    let n = 300u32;
    let (g, _subj, score) = scores_graph(n);
    let score_var = var("score");
    let rows: Vec<Row> = score.iter().map(|&s| Row::from_slice(&[s])).collect();
    let b = Bindings::unsorted(vec![score_var.clone()], rows);

    // The key-cols list is empty (whole-dataset = no GROUP BY).
    let key_cols: Vec<Option<usize>> = vec![];
    let (order, members) = build_groups(&b, &key_cols);

    let mk_agg = |name: AggregateFunction| -> Vec<(Variable, AggregateExpression)> {
        vec![(
            var("agg"),
            AggregateExpression::FunctionCall {
                name,
                expr: Expression::Variable(score_var.clone()),
                distinct: false,
            },
        )]
    };

    // (expected_value, AggregateFunction label, aggregates)
    let cases: &[(f64, AggregateFunction)] = &[
        (44850.0, AggregateFunction::Sum),   // 0+1+…+299 = 299*300/2 = 44850
        (300.0,   AggregateFunction::Count),  // 300 rows
        (0.0,     AggregateFunction::Min),    // minimum of 0..=299
        (299.0,   AggregateFunction::Max),    // maximum
    ];

    for &(expected, ref name) in cases {
        let aggregates = mk_agg(name.clone());
        let b2 = Bindings::unsorted(b.vars.clone(), b.rows.clone());

        // (a) Call columnar_aggregate DIRECTLY — assert it returns Some (path engaged).
        let mut local_col = LocalVocab::default();
        let col_rows = columnar_aggregate(
            &g, &mut local_col, &b2, &[], &aggregates, &order, &members, &[var("agg")],
        );
        assert!(
            col_rows.is_some(),
            "columnar_aggregate must engage (return Some) for eligible aggregate {:?}",
            name
        );
        let col_rows = col_rows.unwrap();
        assert_eq!(col_rows.len(), 1, "whole-dataset agg must produce exactly 1 row for {:?}", name);

        // (b) Decode the result id and compare with the scalar expected value (value check).
        let result_id = col_rows[0][0];
        let got = g.numeric_value(result_id).unwrap_or_else(|| {
            // For sums beyond INLINE_MAX the result is in local vocab.
            match term_of(&g, &local_col, result_id) {
                Some(Term::Literal(l)) => l.value().parse::<f64>().unwrap_or(f64::NAN),
                _ => f64::NAN,
            }
        });
        assert_eq!(
            got, expected,
            "aggregate {:?} over scores 0..=299 must equal {}", name, expected
        );

        // (c) FULL-TERM identity against the TRUE scalar oracle (catches datatype mutations).
        // scalar_oracle calls eval_aggregate directly — it never touches the columnar seam,
        // so a SUM Int→Double mutation in columnar is caught here by the xsd: datatype
        // difference.
        let (scalar_rows, scalar_local) = scalar_oracle(&g, &b2, &[], &aggregates);
        let col_terms = rows_to_terms(&g, &local_col, &col_rows);
        let scalar_terms = rows_to_terms(&g, &scalar_local, &scalar_rows);
        assert_eq!(
            col_terms, scalar_terms,
            "columnar {:?} must be FULL-TERM-identical to scalar oracle (lexical + datatype)",
            name
        );
    }
}

/// T5b: COUNT(*) byte-identical.
#[test]
fn t5b_count_star_byte_identical() {
    let n = 10u32;
    let (g, _subj, score) = scores_graph(n);
    let score_var = var("score");
    let rows: Vec<Row> = score.iter().map(|&s| Row::from_slice(&[s])).collect();
    let b = Bindings::unsorted(vec![score_var], rows);
    let count_star = vec![(
        var("cnt"),
        AggregateExpression::CountSolutions { distinct: false },
    )];
    check_agg_byte_identical(&g, b, &[], count_star);
}

/// T6 (mutation check): SUM output pins the EXACT aggregate value so an implementation
/// that returns MIN or MAX instead of SUM would fail this test. The aggregate value of
/// scores 0..19 is: SUM = 190, MIN = 0, MAX = 19 — all distinct.
#[test]
fn t6_mutation_check_sum_pins_exact_value() {
    let n = 20u32;
    let (g, _subj, score) = scores_graph(n);
    let score_var = var("score");
    let rows: Vec<Row> = score.iter().map(|&s| Row::from_slice(&[s])).collect();
    let b = Bindings::unsorted(vec![score_var.clone()], rows);

    let aggregates = vec![(
        var("total"),
        AggregateExpression::FunctionCall {
            name: AggregateFunction::Sum,
            expr: Expression::Variable(score_var),
            distinct: false,
        },
    )];

    let mut local = LocalVocab::default();
    let out = group_aggregate(&g, &mut local, b, &[], &aggregates).unwrap();
    assert_eq!(out.rows.len(), 1, "whole-dataset aggregate must produce exactly 1 row");

    // The aggregate result id must decode to the integer 190 (SUM 0..=19).
    let result_id = out.rows[0][0];
    let num_val = g.numeric_value(result_id)
        .unwrap_or_else(|| {
            // Might be a local-vocab term (for sums > INLINE_MAX) — materialise it.
            let term = term_of(&g, &local, result_id).unwrap();
            match &term {
                Term::Literal(l) => l.value().parse::<f64>().unwrap_or(f64::NAN),
                _ => f64::NAN,
            }
        });
    assert_eq!(num_val, 190.0_f64, "SUM(0..=19) must equal 190, not MIN=0 or MAX=19");
}

/// T5c: AVG over inline-integer column — columnar path engaged, result FULL-TERM-identical
/// to the TRUE scalar oracle. AVG(0..=299) = 44850/300 = 149.5 (xsd:decimal).
/// n=300 ensures the batch exceeds VEC_MIN_BATCH (256) so columnar_aggregate engages.
/// (C2-fix sq-pntvh.4 adversarial review)
#[test]
fn t5c_avg_byte_identical_to_scalar() {
    let n = 300u32;
    let (g, _subj, score) = scores_graph(n);
    let score_var = var("score");
    let rows: Vec<Row> = score.iter().map(|&s| Row::from_slice(&[s])).collect();
    let b = Bindings::unsorted(vec![score_var.clone()], rows);
    let key_cols: Vec<Option<usize>> = vec![];
    let (order, members) = build_groups(&b, &key_cols);

    let aggregates = vec![(
        var("avg"),
        AggregateExpression::FunctionCall {
            name: AggregateFunction::Avg,
            expr: Expression::Variable(score_var),
            distinct: false,
        },
    )];

    let mut local_col = LocalVocab::default();
    let col_rows = columnar_aggregate(
        &g, &mut local_col, &b, &[], &aggregates, &order, &members, &[var("avg")],
    ).expect("AVG over inline-integer column must engage (return Some)");

    // TRUE scalar oracle: calls eval_aggregate directly, bypassing the columnar seam.
    // Previously this called group_aggregate which under vectorized also went through
    // the columnar path — producing a vacuous columnar-vs-columnar comparison.
    let (scalar_rows, scalar_local) = scalar_oracle(&g, &b, &[], &aggregates);

    // (a) FULL-TERM-identical: compare lexical + datatype, not raw ids.
    let col_terms = rows_to_terms(&g, &local_col, &col_rows);
    let scalar_terms = rows_to_terms(&g, &scalar_local, &scalar_rows);
    assert_eq!(
        col_terms, scalar_terms,
        "columnar AVG must be FULL-TERM-identical to the scalar oracle (lexical + datatype)"
    );

    // (b) Non-vacuous: verify the result is 149.5 (44850/300), not 0 or some wrong value.
    //     AVG(0..=299) = 149.5 (xsd:decimal, stored as a local-vocab Dec term).
    let result_id = col_rows[0][0];
    let term = term_of(&g, &local_col, result_id).expect("AVG result must be a term");
    let avg_str = match &term {
        Term::Literal(l) => l.value().to_string(),
        _ => panic!("AVG result must be a literal, got {:?}", term),
    };
    // The decimal 149.5 serialises as "149.5" (or equivalent exact decimal lexical).
    let avg_f64: f64 = avg_str.parse().expect("AVG lexical must be numeric");
    assert!(
        (avg_f64 - 149.5_f64).abs() < 1e-9,
        "AVG(0..=299) = 44850/300 = 149.5, got {} (term = {:?})", avg_f64, term
    );
}

/// T5d (GROUP BY, first-seen order): two-group GROUP BY with first-seen order
/// deliberately different from sorted order. Columnar path must preserve first-seen order
/// and produce FULL-TERM-identical GROUP BY results to the TRUE scalar oracle.
/// (C2-fix sq-pntvh.4 adversarial review)
#[test]
fn t5d_group_by_first_seen_order_preserved() {
    // Two groups: group "2" appears FIRST, group "1" appears SECOND.
    // score column: group "2"→[10,20], group "1"→[5,15].
    let nt = format!(
        "<http://ex/r1> <http://ex/g> \"b\"^^<{XSD_INT}> .\n\
             <http://ex/r1> <http://ex/score> \"10\"^^<{XSD_INT}> .\n\
             <http://ex/r2> <http://ex/g> \"2\"^^<{XSD_INT}> .\n\
             <http://ex/r2> <http://ex/score> \"20\"^^<{XSD_INT}> .\n\
             <http://ex/r3> <http://ex/g> \"1\"^^<{XSD_INT}> .\n\
             <http://ex/r3> <http://ex/score> \"5\"^^<{XSD_INT}> .\n\
             <http://ex/r4> <http://ex/g> \"1\"^^<{XSD_INT}> .\n\
             <http://ex/r4> <http://ex/score> \"15\"^^<{XSD_INT}> .\n"
    );
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let lit = |v: &str| {
        g.id_of(&Term::Literal(Literal::new_typed_literal(v, xsd::INTEGER))).unwrap()
    };
    // Bindings: two columns [g_val, score], rows in order [group2→10, group2→20, group1→5, group1→15].
    let group_var = var("g");
    let score_var = var("score");
    let rows: Vec<Row> = vec![
        Row::from_slice(&[lit("2"), lit("10")]),  // group "2" first
        Row::from_slice(&[lit("2"), lit("20")]),
        Row::from_slice(&[lit("1"), lit("5")]),   // group "1" second
        Row::from_slice(&[lit("1"), lit("15")]),
    ];
    let b = Bindings::unsorted(vec![group_var.clone(), score_var.clone()], rows);

    let aggregates = vec![(
        var("s"),
        AggregateExpression::FunctionCall {
            name: AggregateFunction::Sum,
            expr: Expression::Variable(score_var),
            distinct: false,
        },
    )];
    let group_vars = vec![group_var];

    // TRUE scalar reference: calls eval_aggregate directly, never the columnar seam.
    // Previously called group_aggregate for both sides — under vectorized both took the
    // columnar path, making the comparison vacuous.
    let (scalar_rows, scalar_local) = scalar_oracle(&g, &b, &group_vars, &aggregates);

    // Columnar path (via group_aggregate under vectorized).
    let b_col = Bindings::unsorted(b.vars.clone(), b.rows.clone());
    let mut local_c = LocalVocab::default();
    let col_out = group_aggregate(&g, &mut local_c, b_col, &group_vars, &aggregates).unwrap();

    // FULL-TERM-identical comparison: lexical + datatype, not raw ids or f64.
    let scalar_terms = rows_to_terms(&g, &scalar_local, &scalar_rows);
    let col_terms = rows_to_terms(&g, &local_c, &col_out.rows);
    assert_eq!(
        col_terms, scalar_terms,
        "GROUP BY columnar must preserve first-seen order and be FULL-TERM-identical to scalar oracle"
    );
    assert_eq!(col_out.rows.len(), 2, "must have 2 groups");
    // Pin: first row = group "2" (SUM=30), second = group "1" (SUM=20).
    let sum_val = |rows: &[Row], idx: usize| {
        let id = rows[idx][1]; // [g_key, sum_val]
        g.numeric_value(id).unwrap_or_else(|| {
            let t = term_of(&g, &local_c, id).unwrap();
            match &t { Term::Literal(l) => l.value().parse().unwrap(), _ => f64::NAN }
        })
    };
    assert_eq!(sum_val(&col_out.rows, 0), 30.0, "first group (key=2) SUM must be 30");
    assert_eq!(sum_val(&col_out.rows, 1), 20.0, "second group (key=1) SUM must be 20");
}

/// T7 (decline gate): a non-inline column (negative integer) must make the seam DECLINE
/// and the scalar path run; both produce the same result.
#[test]
fn t7_non_inline_column_declines() {
    // A negative integer is not inline (inline range is [0, INLINE_MAX]).
    let nt = format!(
        "<http://ex/a> <http://ex/score> \"-1\"^^<{XSD_INT}> .\n\
             <http://ex/b> <http://ex/score> \"2\"^^<{XSD_INT}> .\n"
    );
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let neg_one = g.id_of(&Term::Literal(Literal::new_typed_literal("-1", xsd::INTEGER))).unwrap();
    let two = g.id_of(&Term::Literal(Literal::new_typed_literal("2", xsd::INTEGER))).unwrap();
    assert!(!dict::is_inline(neg_one), "-1 must be a non-inline dict id");
    assert!(dict::is_inline(two), "2 must be inline");

    let score_var = var("score");
    let rows = vec![Row::from_slice(&[neg_one]), Row::from_slice(&[two])];
    let b = Bindings::unsorted(vec![score_var.clone()], rows);
    let aggregates = vec![(
        var("s"),
        AggregateExpression::FunctionCall {
            name: AggregateFunction::Sum,
            expr: Expression::Variable(score_var),
            distinct: false,
        },
    )];
    // Verify columnar_aggregate declines for the mixed (non-inline) column.
    let key_cols: Vec<Option<usize>> = vec![];
    let (order, members) = build_groups(&b, &key_cols);
    let mut local = LocalVocab::default();
    let out_vars = vec![var("s")];
    let result = columnar_aggregate(
        &g, &mut local, &b, &[], &aggregates, &order, &members, &out_vars
    );
    assert!(result.is_none(), "non-inline column must make columnar_aggregate decline");
}

/// T8 (DISTINCT decline): DISTINCT aggregate must always decline.
#[test]
fn t8_distinct_aggregate_declines() {
    let n = 5u32;
    let (g, _subj, score) = scores_graph(n);
    let score_var = var("score");
    let rows: Vec<Row> = score.iter().map(|&s| Row::from_slice(&[s])).collect();
    let b = Bindings::unsorted(vec![score_var.clone()], rows);
    let aggregates = vec![(
        var("c"),
        AggregateExpression::FunctionCall {
            name: AggregateFunction::Count,
            expr: Expression::Variable(score_var),
            distinct: true, // DISTINCT → must decline
        },
    )];
    let key_cols: Vec<Option<usize>> = vec![];
    let (order, members) = build_groups(&b, &key_cols);
    let mut local = LocalVocab::default();
    let out_vars = vec![var("c")];
    let result = columnar_aggregate(
        &g, &mut local, &b, &[], &aggregates, &order, &members, &out_vars
    );
    assert!(result.is_none(), "DISTINCT aggregate must make columnar_aggregate decline");
}
