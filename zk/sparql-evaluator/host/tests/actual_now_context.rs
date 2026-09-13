// [GPT-6] Actual guest context semantics and typed relation rejections.
#[path = "support/now_context.rs"]
mod fixtures;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator::embedded_artifact;
use sparq_proved_evaluator_model::v4;
use std::path::PathBuf;

fn execute(executor: &ExternalProver, witness: &v4::Witness) -> Result<v4::Journal, String> {
    let env = ExecutorEnv::builder()
        .session_limit(Some(1 << 24))
        .write(witness)
        .unwrap()
        .build()
        .unwrap();
    let session = executor
        .execute(env, embedded_artifact())
        .map_err(|e| format!("{e:#}"))?;
    let journal: v4::Journal = session.journal.decode().unwrap();
    v4::bind_journal(&journal, &witness.request).unwrap();
    Ok(journal)
}

fn reject(executor: &ExternalProver, witness: &v4::Witness) {
    let error = execute(executor, witness).expect_err("invalid context must fail the relation");
    assert!(
        error.contains("Guest panicked:")
            && error.contains("bounded exact-dataset relation rejected"),
        "{error}"
    );
}

#[test]
fn actual_v4_context_goldens_then_invalid_context_and_capacity() {
    let executor = ExternalProver::new(
        "real-sparq-now-context",
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("local r0vm required")),
    );
    let corpus = fixtures::corpus();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        for agreed in [false, true] {
            let witness = fixtures::input(case, agreed, 101);
            let expected: v4::CanonicalResult =
                serde_json::from_value(case["expected_result"].clone()).unwrap();
            assert_eq!(
                execute(&executor, &witness).unwrap().result,
                expected,
                "{} agreed={agreed}",
                case["id"]
            );
        }
    }
    let mut witness = fixtures::input(&cases[0], false, 101);
    for query in ["ASK {}", "SELECT (IF(false,NOW(),1) AS ?n) {}"] {
        witness.request.query = query.into();
        for invalid in corpus["invalid_contexts"].as_array().unwrap() {
            witness.request.context.now.datetime = invalid.as_str().unwrap().into();
            reject(&executor, &witness);
        }
    }
    witness = fixtures::input(&cases[0], false, 101);
    witness.request.context.now.datetime = "x".repeat(v4::MAX_NOW_BYTES + 1);
    reject(&executor, &witness);
    witness.request.context.now.datetime = format!(
        "2026-01-01T00:00:00.{}Z",
        "0".repeat(v4::MAX_NOW_BYTES - 21)
    );
    witness.request.query = format!(
        "SELECT (COALESCE({}) AS ?n) {{}}",
        vec!["NOW()"; 65].join(",")
    );
    reject(&executor, &witness);
    witness = fixtures::input(&cases[0], false, 101);
    for expression in ["RAND()", "UUID()", "STRUUID()", "BNODE()"] {
        witness.request.query = format!("SELECT ({expression} AS ?n) {{}}");
        reject(&executor, &witness);
    }
    witness.request.query = "ASK {FILTER EXISTS {FILTER(NOW()=NOW())}}".into();
    witness.dataset.nquads = "_:source <http://ex/p> <http://ex/o> .".into();
    reject(&executor, &witness);
    witness = fixtures::input(&cases[0], false, 101);
    witness.request.query = "SELECT ?n {VALUES ?n {1} FILTER EXISTS {FILTER(BOUND(?n))}}".into();
    reject(&executor, &witness);
}
