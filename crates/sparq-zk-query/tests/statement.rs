#[path = "support/fixture.rs"]
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

#[test]
fn envelope_policy_has_canonical_optional_values_and_binds_wide_times() {
    use sparq_zk_query::statement::EnvelopeInputs;
    let mut policy = EnvelopeInputs::issuer_only([7; 32]);
    let mut invalid = policy.clone();
    invalid.now = 1;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.earliest_created = 1;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.snapshot_root[31] = 1;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.snapshot_depth = 1;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.allow_missing_status = true;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.holder_issuers[0][31] = 1;
    assert!(invalid.words().is_err());
    policy.holder_required = true;
    policy.holder_issuers_present = true;
    policy.holder_issuer_count = 2;
    policy.holder_issuers[0][31] = 3;
    policy.holder_issuers[1][31] = 5;
    policy.status_required = true;
    policy.allow_missing_status = true;
    policy.snapshot_root[31] = 7;
    policy.snapshot_depth = 3;
    policy.validity_required = true;
    policy.now = 1u64 << 40;
    policy.freshness_required = true;
    policy.earliest_created = (1u64 << 40) - 1;
    let words = policy.words().unwrap();
    let word = |n: u64| {
        let mut w = [0; 32];
        w[24..].copy_from_slice(&n.to_be_bytes());
        w
    };
    let mut expected = vec![[7; 32]];
    expected.extend(
        [
            4,
            1,
            1,
            2,
            3,
            5,
            0,
            0,
            1,
            1,
            7,
            3,
            1,
            1u64 << 40,
            1,
            (1u64 << 40) - 1,
        ]
        .map(word),
    );
    assert_eq!(words, expected);
    for count in [0, 5] {
        let mut invalid = policy.clone();
        invalid.holder_issuer_count = count;
        assert!(invalid.words().is_err());
    }
    let mut invalid = policy.clone();
    invalid.holder_issuers.swap(0, 1);
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.holder_issuers[1] = invalid.holder_issuers[0];
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.holder_issuers[0] = [0; 32];
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.holder_issuers_present = false;
    assert!(invalid.words().is_err());
    let mut invalid = policy.clone();
    invalid.holder_required = false;
    assert!(invalid.words().is_err());
    let plan = lower("ASK {}", B).unwrap();
    let statement = Statement {
        request_digest: [0; 32],
        input_commitment: [0; 32],
        signature_mode: 1,
        result: ResultValue::Ask(true),
    };
    let layout = Layout::new(&plan);
    let before = layout
        .reconstruct_authenticated_with_envelope(&plan, &statement, &policy)
        .unwrap();
    policy.now += 1;
    let after = layout
        .reconstruct_authenticated_with_envelope(&plan, &statement, &policy)
        .unwrap();
    assert_ne!(before.last(), after.last());
}

#[test]
fn explicit_empty_holder_scope_is_distinct_from_an_absent_scope() {
    let mut all = sparq_zk_query::statement::EnvelopeInputs::issuer_only([7; 32]);
    all.holder_required = true;
    let mut none = all.clone();
    none.holder_issuers_present = true;
    assert!(all.words().is_ok());
    assert!(none.words().is_ok());
    assert_ne!(all.words().unwrap(), none.words().unwrap());
}
