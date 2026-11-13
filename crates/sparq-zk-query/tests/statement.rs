#[path = "support/mod.rs"]
mod fixture_support;
use fixture_support::FixtureEncoder;
use sparq_zk_query::{
    capacity::Bounds,
    interp::{evaluate, ResultValue},
    lower::lower,
    statement::{dataset_commitment, Layout, Statement},
};
const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 4,
    rows_per_table: 2,
};
#[test]
fn public_reconstruction_is_derived_from_the_plan_and_checks_result_shape() {
    let plan = lower("ASK {}", B).unwrap();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: [0; 32],
        signature_mode: 0,
        result: ResultValue::Ask(false),
    };
    let layout = Layout::new(&plan);
    let words = layout
        .reconstruct(&plan, &statement, &FixtureEncoder)
        .unwrap();
    assert_eq!(
        words,
        layout
            .reconstruct(&plan, &statement, &FixtureEncoder)
            .unwrap()
    );
    let mut altered = statement.clone();
    altered.request_digest[0] ^= 1;
    let changed = layout
        .reconstruct(&plan, &altered, &FixtureEncoder)
        .unwrap();
    assert_ne!(words.last(), changed.last());
    let mut altered = statement.clone();
    altered.signature_mode = 1;
    assert!(layout
        .reconstruct(&plan, &altered, &FixtureEncoder)
        .is_err());
    let mut altered = statement;
    altered.result = ResultValue::Select {
        variables: vec![],
        rows: vec![],
    };
    assert!(layout
        .reconstruct(&plan, &altered, &FixtureEncoder)
        .is_err());
    let mut wrong = layout;
    wrong.rows += 1;
    assert!(wrong.reconstruct(&plan, &altered, &FixtureEncoder).is_err());
}
#[test]
fn commitment_covers_private_salt_cells_and_credential_boundaries() {
    let data = fixture_support::fixture();
    let original = dataset_commitment(&data, [3; 32]).unwrap();
    assert_ne!(original, dataset_commitment(&data, [4; 32]).unwrap());
    let mut altered = data.clone();
    altered[0].quads[0][0].enc[31] ^= 1;
    assert_ne!(original, dataset_commitment(&altered, [3; 32]).unwrap());
    let mut split = data.clone();
    split[0].quads.pop();
    split.push(sparq_zk_query::interp::CredentialCells {
        quads: vec![data[0].quads[1]],
    });
    assert_ne!(original, dataset_commitment(&split, [3; 32]).unwrap());
    let plan = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x}",
        Bounds {
            rows_per_table: 4,
            ..B
        },
    )
    .unwrap();
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: original,
        signature_mode: 0,
        result: trace.result.clone(),
    };
    let layout = Layout::new(&plan);
    layout
        .reconstruct(&plan, &statement, &FixtureEncoder)
        .unwrap();
    let mut bad = statement;
    if let ResultValue::Select { rows, .. } = &mut bad.result {
        rows.reverse();
    }
    assert!(layout.reconstruct(&plan, &bad, &FixtureEncoder).is_err());
}
