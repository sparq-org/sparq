#[path = "support/authenticated.rs"]
mod authenticated;
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
fn authenticated_read_witnesses_cover_consumer_inputs_and_public_values() {
    use sparq_zk_query::{
        credential::SuiteEncoder, plan::Operator, witness::prover_toml_authenticated,
    };
    let data = authenticated::fixture();
    let cells = vec![data[0].cells(0).unwrap()];
    for query in [
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x > 1)}",
        "SELECT ?x WHERE {VALUES ?x {7 UNDEF} FILTER(?x > 0)}",
        "ASK {FILTER(?absent > 0)}",
    ] {
        let plan = lower(query, B).unwrap();
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        let render = |trace: &_| {
            prover_toml_authenticated(
                &plan,
                &data,
                trace,
                [9; 32],
                [3; 32],
                authenticated::issuer_root(),
            )
        };
        let text = render(&trace).unwrap();
        assert_eq!(text, render(&trace).unwrap());
        let (filter, input) = plan
            .nodes()
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                if let Operator::Filter(input, _) = n.operator {
                    Some((i, input as usize))
                } else {
                    None
                }
            })
            .unwrap();
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("reads_{filter} =")));
        if query.contains("absent") {
            assert!(line.is_none());
            continue;
        }
        let line = line.unwrap();
        let literals = trace.tables[input]
            .rows
            .iter()
            .flatten()
            .filter(|c| {
                c.bound
                    && c.kind == 3
                    && !plan.parameters().iter().any(|p| {
                        if let sparq_zk_query::plan::Parameter::Term(t) = p {
                            use sparq_zk_query::interp::EncodeTerm;
                            SuiteEncoder.encode(t).unwrap() == **c
                        } else {
                            false
                        }
                    })
            })
            .count();
        assert_eq!(line.matches("bound = true").count(), literals);
        assert_eq!(
            line.matches("bound = false").count(),
            plan.nodes()[input].output.capacity as usize - literals
        );
        let mut forged = trace.clone();
        let literal = forged.tables[input]
            .rows
            .iter_mut()
            .flatten()
            .find(|c| c.bound && c.kind == 3)
            .unwrap();
        literal.enc[31] ^= 1;
        assert_eq!(
            render(&forged).unwrap_err(),
            Error::Invalid("expression read opening".into())
        );
    }
}

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
