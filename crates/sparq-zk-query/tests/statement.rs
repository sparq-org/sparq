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
fn explicit_empty_holder_scope_is_refused_while_an_absent_scope_selects_all() {
    let mut all = sparq_zk_query::statement::EnvelopeInputs::issuer_only([7; 32]);
    all.holder_required = true;
    let mut none = all.clone();
    none.holder_issuers_present = true;
    assert!(all.words().is_ok());
    assert_eq!(
        none.words().unwrap_err(),
        sparq_zk_query::Error::Invalid("empty holder issuer list".into())
    );
}

#[test]
fn envelope_instants_use_the_authenticated_timestamp_range() {
    use sparq_zk_query::{statement::EnvelopeInputs, Error};
    let mut policy = EnvelopeInputs::issuer_only([7; 32]);
    policy.validity_required = true;
    policy.freshness_required = true;
    policy.now = i64::MAX as u64;
    policy.earliest_created = i64::MAX as u64;
    assert!(policy.words().is_ok());
    for freshness in [false, true] {
        let mut invalid = policy.clone();
        if freshness {
            invalid.earliest_created += 1;
        } else {
            invalid.now += 1;
        }
        assert_eq!(
            invalid.words().unwrap_err(),
            Error::Invalid("envelope instant range".into())
        );
    }
}

#[test]
fn computed_boolean_answers_do_not_require_a_source_vocabulary_entry() {
    use sparq_zk_query::{answer::Answer, credential::SuiteEncoder, interp::EncodeTerm};
    let plan = lower("SELECT ?result WHERE {VALUES ?result {}}", B).unwrap();
    for value in [true, false] {
        let term: oxrdf::Term = oxrdf::Literal::from(value).into();
        let result = ResultValue::Select {
            variables: vec!["result".into()],
            rows: vec![vec![{
                let mut cell = SuiteEncoder.encode(&term).unwrap();
                cell.key_class = 0;
                cell.key = [0; 32];
                cell
            }]],
        };
        assert_eq!(
            Answer::release(&result, &plan, &[], &SuiteEncoder).unwrap(),
            Answer::Select {
                variables: vec!["result".into()],
                rows: vec![vec![Some(term)]]
            }
        );
    }
}

#[test]
fn revealed_inputs_verify_signatures_and_reconstruct_ordered_padded_words() {
    use sparq_zk::{
        field::{field_to_be_bytes_32, Fr},
        sig,
    };
    use sparq_zk_query::statement::{number, EnvelopeInputs, RevealedInputs, RevealedSignature};
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 2,
            ..B
        },
    )
    .unwrap();
    let secret = sig::SecretKey::from_seed(19);
    let message = Fr::from(7u64);
    let item = RevealedSignature {
        key: secret.public_key(),
        message: field_to_be_bytes_32(&message),
        signature: sig::sign_deterministic(&secret, &message),
    };
    let admitted = RevealedInputs::verify(std::slice::from_ref(&item)).unwrap();
    let envelope = EnvelopeInputs::issuer_only(number(9));
    let layout = Layout::new(&plan);
    let mut statement = Statement {
        request_digest: [0; 32],
        input_commitment: [0; 32],
        signature_mode: 0,
        result: ResultValue::Ask(true),
    };
    let words = layout
        .reconstruct_revealed(&plan, &statement, &envelope, &admitted)
        .unwrap();
    let key = field_to_be_bytes_32(
        &sparq_merkle_suite::metadata::key_cell(&item.key)
            .unwrap()
            .enc,
    );
    assert_eq!(
        &words[words.len() - 6..words.len() - 1],
        &[number(1), item.message, number(0), key, number(0)]
    );
    let mut other = item.clone();
    other.message = number(8);
    other.signature = sig::sign_deterministic(&secret, &Fr::from(8u64));
    let ordered = RevealedInputs::verify(&[item.clone(), other.clone()]).unwrap();
    let reversed = RevealedInputs::verify(&[other.clone(), item.clone()]).unwrap();
    assert_ne!(
        layout
            .reconstruct_revealed(&plan, &statement, &envelope, &ordered)
            .unwrap()
            .last(),
        layout
            .reconstruct_revealed(&plan, &statement, &envelope, &reversed)
            .unwrap()
            .last()
    );
    let over = RevealedInputs::verify(&[item.clone(), item.clone(), item.clone()]).unwrap();
    assert!(layout
        .reconstruct_revealed(&plan, &statement, &envelope, &over)
        .is_err());
    assert!(RevealedInputs::verify(&vec![item.clone(); 5]).is_err());
    for field in ["message", "key", "signature"] {
        let mut changed = item.clone();
        match field {
            "message" => changed.message = number(8),
            "key" => changed.key = sig::SecretKey::from_seed(20).public_key(),
            _ => changed.signature = other.signature,
        }
        assert!(
            RevealedInputs::verify(&[changed]).is_err(),
            "altered revealed {field} accepted"
        );
    }
    let mut noncanonical = item;
    noncanonical.message = [255; 32];
    assert!(RevealedInputs::verify(&[noncanonical]).is_err());
    assert!(layout
        .reconstruct_authenticated_with_envelope(&plan, &statement, &envelope)
        .is_err());
    statement.signature_mode = 1;
    assert!(layout
        .reconstruct_revealed(&plan, &statement, &envelope, &admitted)
        .is_err());
    let hidden = layout
        .reconstruct_authenticated_with_envelope(&plan, &statement, &envelope)
        .unwrap();
    assert_eq!(&hidden[hidden.len() - 6..hidden.len() - 1], &[[0; 32]; 5]);
}
