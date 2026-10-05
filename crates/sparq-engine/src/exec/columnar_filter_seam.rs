use super::*;

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// `n` triples `ex:s{i} ex:age {i}` (integer literal → inline id). Returns
/// (graph, subject ids [dict IRIs, NON-inline], age ids [inline integers]).
fn ages_graph(n: u32) -> (Graph, Vec<Id>, Vec<Id>) {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{i}\"^^<{XSD_INT}> .\n"));
    }
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let subj = (0..n)
        .map(|i| g.id_of(&Term::NamedNode(oxrdf::NamedNode::new(format!("http://ex/s{i}")).unwrap())).unwrap())
        .collect();
    let age = (0..n)
        .map(|i| g.id_of(&Term::Literal(Literal::new_typed_literal(i.to_string(), xsd::INTEGER))).unwrap())
        .collect();
    (g, subj, age)
}

fn var(name: &str) -> Variable {
    Variable::new(name).unwrap()
}

fn int_lit(s: &str) -> Expression {
    Expression::Literal(Literal::new_typed_literal(s, xsd::INTEGER))
}

/// `?v OP konst`.
fn mk_filter(op: &str, v: &Variable, konst: Expression) -> Expression {
    let a = Box::new(Expression::Variable(v.clone()));
    let k = Box::new(konst);
    match op {
        ">" => Expression::Greater(a, k),
        ">=" => Expression::GreaterOrEqual(a, k),
        "<" => Expression::Less(a, k),
        "<=" => Expression::LessOrEqual(a, k),
        "=" => Expression::Equal(a, k),
        _ => unreachable!(),
    }
}

/// The numeric age values (inline column, index 1) of a result row set, in order.
fn age_values(g: &Graph, rows: &[Row]) -> Vec<f64> {
    rows.iter().map(|r| g.numeric_value(r[1]).unwrap()).collect()
}

#[test]
fn columnar_seam_is_byte_identical_to_scalar_on_inline_column() {
    // n=300 ensures the batch exceeds VEC_MIN_BATCH (256) so columnar_filter engages.
    // (sq-y5ew5) Updated for the new 4-arg signature and Result return type.
    let n = 300u32;
    let (g, subj, age) = ages_graph(n);
    let vars = vec![var("s"), var("age")];
    let rows: Vec<Row> = (0..n as usize).map(|i| Row::from_slice(&[subj[i], age[i]])).collect();
    let local = LocalVocab::default();
    let age_var = var("age");

    for (op, t) in [
        (">", "20"), (">=", "20"), ("<", "10"), ("<=", "10"),
        ("=", "13"), (">", "39"), (">=", "0"), (">", "100"),
    ] {
        let expr = mk_filter(op, &age_var, int_lit(t));

        // The seam ENGAGES for an all-inline column: Result is Ok(Some(rows)).
        let via_columnar = columnar_filter(&g, &local, &Bindings::unsorted(vars.clone(), rows.clone()), &expr);
        assert!(via_columnar.as_ref().unwrap().is_some(), "all-inline column must be columnar-eligible ({op} {t})");

        // Scalar reference (the real row path).
        let mut scalar = Bindings::unsorted(vars.clone(), rows.clone());
        apply_filter_scalar(&g, &local, &mut scalar, &expr).unwrap();

        // Dispatcher (takes the columnar branch under the feature).
        let mut disp = Bindings::unsorted(vars.clone(), rows.clone());
        apply_filter(&g, &local, &mut disp, &expr).unwrap();

        // BYTE-IDENTICAL: same rows (incl. the NON-inline subject column), same order.
        assert_eq!(disp.rows, scalar.rows, "dispatcher rows != scalar for {op} {t}");
        assert_eq!(via_columnar.as_ref().unwrap().as_ref().unwrap(), &scalar.rows, "columnar_filter rows != scalar for {op} {t}");
    }
}

#[test]
fn columnar_seam_pins_exact_survivors_and_order() {
    // Non-vacuous: pin the EXACT surviving age sequence (content + ascending order) so a
    // wrong mask (off-by-one / inverted / column desync) turns this red.
    // n=300 ensures the batch exceeds VEC_MIN_BATCH (256) so columnar_filter engages.
    // (sq-y5ew5) Updated for new signature and Result<Option<Vec<Row>>> return.
    let n = 300u32;
    let (g, subj, age) = ages_graph(n);
    let vars = vec![var("s"), var("age")];
    let rows: Vec<Row> = (0..n as usize).map(|i| Row::from_slice(&[subj[i], age[i]])).collect();
    let local = LocalVocab::default();
    let age_var = var("age");

    let out = columnar_filter(&g, &local, &Bindings::unsorted(vars.clone(), rows.clone()), &mk_filter(">", &age_var, int_lit("20"))).unwrap().unwrap();
    let expected: Vec<f64> = (21..300).map(f64::from).collect();
    assert_eq!(age_values(&g, &out), expected, "age > 20 must keep 21..=299, ascending");

    // A DIFFERENT filter must select a DIFFERENT set (the mask does real work).
    let other = columnar_filter(&g, &local, &Bindings::unsorted(vars, rows), &mk_filter("<", &age_var, int_lit("20"))).unwrap().unwrap();
    assert_ne!(age_values(&g, &other), expected, "distinct filters must select distinct rows");
}

#[test]
fn columnar_seam_declines_small_batches_regardless_of_column_type() {
    // A batch of 2 rows is below VEC_MIN_BATCH (256), so the seam DECLINES regardless
    // of the column type (non-inline ints, negatives, strings, unbound). The scalar path
    // runs in all cases and produces correct results. (sq-y5ew5) Updated:
    // non-inline columns no longer cause decline for large batches (they use the tri-mask);
    // decline here is due to VEC_MIN_BATCH only.
    let big = 2_000_000_000u32.to_string(); // > INLINE_MAX (2^30-1): a NON-inline dict integer
    let nt = format!(
        "<http://ex/a> <http://ex/age> \"5\"^^<{XSD_INT}> .\n\
             <http://ex/b> <http://ex/age> \"{big}\"^^<{XSD_INT}> .\n\
             <http://ex/c> <http://ex/age> \"-7\"^^<{XSD_INT}> .\n\
             <http://ex/d> <http://ex/age> \"hello\" .\n"
    );
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let lit_id = |s: &str, dt: Option<&str>| -> Id {
        let t = match dt {
            Some(d) => Term::Literal(Literal::new_typed_literal(s, oxrdf::NamedNode::new(d).unwrap())),
            None => Term::Literal(oxrdf::Literal::new_simple_literal(s)),
        };
        g.id_of(&t).unwrap()
    };
    let five = lit_id("5", Some(XSD_INT));
    assert!(dict::is_inline(five));
    let big_id = lit_id(&big, Some(XSD_INT));
    assert!(!dict::is_inline(big_id), "2e9 must be a NON-inline dict integer");
    let neg = lit_id("-7", Some(XSD_INT));
    let hello = lit_id("hello", None);

    let vars = vec![var("s"), var("age")];
    let expr = mk_filter(">", &var("age"), int_lit("0"));
    let local = LocalVocab::default();

    for age_cell in [big_id, neg, hello, NO_ID] {
        // 2 rows < VEC_MIN_BATCH=256 → seam declines.
        let rows: Vec<Row> = vec![Row::from_slice(&[five, five]), Row::from_slice(&[five, age_cell])];
        assert!(
            columnar_filter(&g, &local, &Bindings::unsorted(vars.clone(), rows.clone()), &expr).unwrap().is_none(),
            "batch of 2 rows must decline (VEC_MIN_BATCH)"
        );
        let mut disp = Bindings::unsorted(vars.clone(), rows.clone());
        let mut scalar = Bindings::unsorted(vars.clone(), rows);
        apply_filter(&g, &local, &mut disp, &expr).unwrap();
        apply_filter_scalar(&g, &local, &mut scalar, &expr).unwrap();
        assert_eq!(disp.rows, scalar.rows, "row-path result must be identical on decline");
    }
}

#[test]
fn columnar_seam_declines_non_sargable_and_temporal_filters() {
    // (sq-y5ew5) Updated for new 4-arg signature and Result return.
    let n = 8u32;
    let (g, subj, age) = ages_graph(n);
    let vars = vec![var("s"), var("age")];
    let rows: Vec<Row> = (0..n as usize).map(|i| Row::from_slice(&[subj[i], age[i]])).collect();
    let b = Bindings::unsorted(vars, rows);
    let local = LocalVocab::default();
    let age_var = var("age");

    // var-vs-var: not sargable → decline.
    let var_var = Expression::Greater(
        Box::new(Expression::Variable(age_var.clone())),
        Box::new(Expression::Variable(var("s"))),
    );
    assert!(columnar_filter(&g, &local, &b, &var_var).unwrap().is_none());

    // A temporal comparison is sargable but has NO vector kernel yet → decline.
    let dt = Expression::Literal(Literal::new_typed_literal(
        "2020-01-01T00:00:00Z",
        oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#dateTime").unwrap(),
    ));
    let temporal = Expression::Greater(Box::new(Expression::Variable(age_var)), Box::new(dt));
    assert!(columnar_filter(&g, &local, &b, &temporal).unwrap().is_none(), "temporal comparison has no vector kernel");
}

#[cfg(feature = "zk")]
#[test]
fn columnar_seam_declines_while_zk_trace_armed() {
    // The zk trace must capture the FILTER obligation set, so the columnar seam DECLINES
    // while the recorder is armed and the scalar path records it
    // (research/vector-at-a-time-m4.md §3.2, open question 2 — simplest-safe rule).
    // n=300 to exceed VEC_MIN_BATCH (256) so the sanity check can confirm eligibility.
    // (sq-y5ew5) Updated for new 4-arg signature and Result return.
    let n = 300u32;
    let (g, subj, age) = ages_graph(n);
    let vars = vec![var("s"), var("age")];
    let rows: Vec<Row> = (0..n as usize).map(|i| Row::from_slice(&[subj[i], age[i]])).collect();
    let b = Bindings::unsorted(vars, rows);
    let local = LocalVocab::default();
    let expr = mk_filter(">", &var("age"), int_lit("0"));

    // Sanity: eligible (and firing) when zk is NOT armed.
    assert!(columnar_filter(&g, &local, &b, &expr).unwrap().is_some(), "eligible without zk armed");
    // Armed ⇒ decline (I2 fires before VEC_MIN_BATCH, so batch size is irrelevant here).
    let _guard = crate::zk::install();
    assert!(crate::zk::enabled());
    assert!(columnar_filter(&g, &local, &b, &expr).unwrap().is_none(), "columnar seam must decline while zk is armed");
}
