mod closed_native;
#[path = "support/mod.rs"]
mod support;
// Public per-table choices and exact overflow refusals.
use sparq_zk_query::{
    capacity::Bounds,
    credential::SuiteEncoder,
    credential_graphs as graphs,
    interp::{CredentialCells, EncodeTerm},
    plan_closed::with_row_bounds,
    Error,
};
fn bounds(rows_per_table: u32) -> Bounds {
    Bounds {
        credentials: 1,
        quads_per_credential: 64,
        rows_per_table,
    }
}
#[test]
fn selected_table_buckets_change_hash_and_source() {
    let p = graphs::lower("SELECT ?x {?s <urn:p> ?x}", bounds(4)).unwrap();
    let small = with_row_bounds(p.clone(), bounds(32), &vec![4; p.nodes().len()]).unwrap();
    let mut rows = vec![4; p.nodes().len()];
    rows[0] = 32;
    let wide = with_row_bounds(p, bounds(32), &rows).unwrap();
    assert_ne!(small.hash(), wide.hash());
    assert_ne!(
        sparq_zk_query::codegen::generate_operator_fixture(&small).unwrap(),
        sparq_zk_query::codegen::generate_operator_fixture(&wide).unwrap()
    );
}
#[test]
fn table_overflow_refuses_below_the_program_row_ceiling() {
    let p = graphs::lower("SELECT ?x {?s <urn:p> ?x}", bounds(4)).unwrap();
    let p = with_row_bounds(p.clone(), bounds(32), &vec![4; p.nodes().len()]).unwrap();
    let data = CredentialCells {
        quads: (0..5)
            .map(|i| {
                [
                    SuiteEncoder
                        .encode(&oxrdf::NamedNode::new_unchecked(format!("urn:s{i}")).into())
                        .unwrap(),
                    SuiteEncoder
                        .encode(&oxrdf::NamedNode::new_unchecked("urn:p").into())
                        .unwrap(),
                    SuiteEncoder
                        .encode(&oxrdf::Literal::from(i).into())
                        .unwrap(),
                    Default::default(),
                ]
            })
            .collect(),
    };
    let error = graphs::evaluate_cells(&p, std::slice::from_ref(&data), &SuiteEncoder).unwrap_err();
    assert!(matches!(error,Error::Capacity(ref why) if why=="per-table operator output"));
    let wide = with_row_bounds(p.clone(), bounds(32), &vec![32; p.nodes().len()]).unwrap();
    assert_eq!(
        graphs::evaluate_cells(&wide, &[data], &SuiteEncoder)
            .unwrap()
            .tables[0]
            .rows
            .len(),
        5
    );
}
#[test]
fn invalid_and_incomplete_bucket_choices_refuse() {
    let p = graphs::lower("ASK {}", bounds(4)).unwrap();
    for rows in [vec![], vec![3; p.nodes().len()], vec![64; p.nodes().len()]] {
        assert!(matches!(
            with_row_bounds(p.clone(), bounds(32), &rows),
            Err(Error::Capacity(_))
        ));
    }
    let p = graphs::lower(
        "ASK { ?s <urn:p> ?x FILTER NOT EXISTS {?s <urn:q> ?x} }",
        bounds(4),
    )
    .unwrap();
    assert!(matches!(
        with_row_bounds(p.clone(), bounds(32), &vec![32; p.nodes().len()]),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn wide_join_executes_and_rejects_a_consistent_missing_pair() {
    use sparq_zk_query::{
        codegen,
        interp::{ResultValue, Trace},
        plan::Operator,
        plan_closed::ClosedOperator as C,
        witness_closed,
    };
    let Some(gate) = support::gate("wide join inverse map execution", &["nargo"]) else {
        return;
    };
    let request = Bounds {
        credentials: 1,
        quads_per_credential: 8,
        rows_per_table: 32,
    };
    let p = graphs::lower(
        "SELECT ?s ?x ?y {?s <urn:p> ?x; <urn:q> ?y}",
        Bounds {
            rows_per_table: 4,
            ..request
        },
    )
    .unwrap();
    let limits = p
        .nodes()
        .iter()
        .map(|n| {
            if matches!(
                n.operator,
                Operator::Join(..) | Operator::Closed(C::DatasetScan { .. })
            ) {
                32
            } else {
                4
            }
        })
        .collect::<Vec<_>>();
    let p = with_row_bounds(p, request, &limits).unwrap();
    let join = p
        .nodes()
        .iter()
        .position(|n| matches!(n.operator, Operator::Closed(C::Join { .. })))
        .unwrap();
    let iri = |s: &str| {
        SuiteEncoder
            .encode(&oxrdf::NamedNode::new_unchecked(s).into())
            .unwrap()
    };
    let number = |n: i32| {
        SuiteEncoder
            .encode(&oxrdf::Literal::from(n).into())
            .unwrap()
    };
    let data = [CredentialCells {
        quads: vec![
            [iri("urn:s1"), iri("urn:p"), number(1), Default::default()],
            [iri("urn:s1"), iri("urn:q"), number(2), Default::default()],
            [iri("urn:s2"), iri("urn:p"), number(3), Default::default()],
            [iri("urn:s2"), iri("urn:q"), number(4), Default::default()],
        ],
    }];
    let trace = graphs::evaluate_cells_for_request(&p, &data, &SuiteEncoder, [7; 32]).unwrap();
    let ResultValue::Select { rows, .. } = &trace.result else {
        panic!("SELECT")
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&vec![iri("urn:s1"), number(1), number(2)]));
    assert!(rows.contains(&vec![iri("urn:s2"), number(3), number(4)]));
    let package = codegen::generate_operator_fixture(&p).unwrap();
    let workspace = closed_native::Workspace::new(&package);
    let render = |t: &Trace| {
        witness_closed::operator_prover_toml(&p, &data, &SuiteEncoder, t, [7; 32], [3; 32]).unwrap()
    };
    let execute = |text: &str, tag: &str| {
        std::fs::write(
            workspace
                .0
                .path()
                .join(&package.name)
                .join(format!("Prover_{tag}.toml")),
            text,
        )
        .unwrap();
        std::process::Command::new("timeout")
            .args([
                "600",
                "nargo",
                "execute",
                "--inliner-aggressiveness",
                "0",
                "--package",
                &package.name,
                "--prover-name",
                &format!("Prover_{tag}"),
                tag,
            ])
            .current_dir(workspace.0.path())
            .output()
            .unwrap()
    };
    let honest = execute(&render(&trace), "wide_honest");
    assert!(
        honest.status.success(),
        "{}{}",
        String::from_utf8_lossy(&honest.stdout),
        String::from_utf8_lossy(&honest.stderr)
    );
    let mut missing = trace.clone();
    missing.tables[join].rows.pop();
    missing.tables[p.result_node() as usize].rows.pop();
    let ResultValue::Select { rows, .. } = &mut missing.result else {
        unreachable!()
    };
    rows.pop();
    let text = render(&missing);
    let prefix = format!("closed_pairs_{join} = ");
    let line = text.lines().find(|l| l.starts_with(&prefix)).unwrap();
    let mut pairs: Vec<[u32; 2]> = serde_json::from_str(&line[prefix.len()..]).unwrap();
    for pair in &mut pairs[1..] {
        *pair = [0, 0];
    }
    let replacement = format!("{prefix}{}", serde_json::to_string(&pairs).unwrap());
    let text = text.replacen(line, &replacement, 1);
    let rejected = execute(&text, "wide_missing_pair");
    assert!(!rejected.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(diagnostic.contains("operator length"), "{diagnostic}");
    support::complete(gate);
}
