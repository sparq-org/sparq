#[path = "support/fixture.rs"]
mod fixture_support;

use fixture_support::{fixture, FixtureEncoder};
use sparq_zk_query::{
    capacity::Bounds,
    interp::{evaluate, ResultValue},
    lower::lower,
    witness::prover_toml,
    Error,
};

const B: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};

#[test]
fn supplied_trace_renders_all_tables_and_keeps_the_public_request() {
    let p = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x IN (1,2))} LIMIT 1 OFFSET 1",
        B,
    )
    .unwrap();
    let data = fixture();
    let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
    let a = prover_toml(&p, &data, &FixtureEncoder, &trace, [9; 32], [3; 32]).unwrap();
    assert_eq!(
        a,
        prover_toml(&p, &data, &FixtureEncoder, &trace, [9; 32], [3; 32]).unwrap()
    );
    assert!(a.contains("counts = [2, 0]\ncredential_count = 1\n"));
    assert!(a.contains("indices = [2, 1, 1]"));
    assert!(a.contains("request_digest = [9, 9, 9,"));
    for i in 0..p.nodes().len() {
        assert!(a
            .lines()
            .any(|l| l.starts_with(&format!("t{i} = {{ len = "))));
    }
    let mut wrong = trace.clone();
    wrong.tables.pop();
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("trace node count".into())
    );
    let mut wrong = trace.clone();
    wrong.tables[0].rows[0].pop();
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("witness row width".into())
    );
    let mut wrong = trace.clone();
    wrong.tables[0].rows.resize(5, vec![]);
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Capacity("witness table".into())
    );
    let mut wrong = trace.clone();
    wrong.result = ResultValue::Ask(true);
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("result form".into())
    );
    let mut wrong = trace.clone();
    if let ResultValue::Select { variables, .. } = &mut wrong.result {
        variables[0] = "other".into();
    }
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("result variables".into())
    );
    assert!(matches!(
        prover_toml(
            &p,
            &vec![data[0].clone(); 3],
            &FixtureEncoder,
            &trace,
            [9; 32],
            [3; 32],
        ),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn false_ask_is_an_explicit_result_with_an_empty_private_table() {
    let p = lower("ASK {?s <urn:p> ?x FILTER(?x > 8)}", B).unwrap();
    let data = fixture();
    let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
    let input = prover_toml(&p, &data, &FixtureEncoder, &trace, [1; 32], [3; 32]).unwrap();
    assert!(input.lines().any(|l| l == "result = false"));
    assert_eq!(trace.tables.last().unwrap().rows.len(), 0);
}
