// [GPT-6] Executes V3 in the actual guest; these executions are not receipts.
#[path = "support/graph_results.rs"]
mod fixtures;
use fixtures::*;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator::embedded_artifact;
use sparq_proved_evaluator_model::{RowOrder, v3};
use std::path::PathBuf;

fn executor() -> ExternalProver {
    ExternalProver::new(
        "sparq-v3-actual-guest",
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("local r0vm required")),
    )
}

fn execute(input: &v3::Witness) -> v3::Journal {
    let env = ExecutorEnv::builder()
        .session_limit(Some(1 << 25))
        .write(input)
        .unwrap()
        .build()
        .unwrap();
    executor()
        .execute(env, embedded_artifact())
        .expect("actual V3 guest execution")
        .journal
        .decode()
        .expect("typed V3 journal")
}

#[test]
fn actual_v3_canonicalization_guest_smoke() {
    let mut input = witness(BAG, SHARED_NODE, &[]);
    accepted(&mut input);
    let journal = execute(&input);
    assert_eq!(journal, v3::evaluate(&input).unwrap());
    let v3::CanonicalResult::Select { order, rows, .. } = &journal.result else {
        panic!("table")
    };
    assert_eq!(*order, RowOrder::Bag);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[2], rows[3]);
    assert!(
        rows.iter()
            .all(|row| row[0] == rows[0][0] && row[2].is_none())
    );
    assert!(!format!("{:?}", journal.result).contains("private"));
    input.dataset.nquads = SHARED_NODE.replace("private", "renamed");
    accepted(&mut input);
    assert_eq!(journal.result, execute(&input).result);
    for (query, source, count) in [
        (CONSTRUCT, CONSTRUCT_SOURCE, 5),
        ("DESCRIBE ex:a", DESCRIBE_SOURCE, 4),
    ] {
        let input = witness(query, source, &[]);
        let actual = execute(&input);
        assert_eq!(actual, v3::evaluate(&input).unwrap());
        let v3::CanonicalResult::Graph { ntriples } = actual.result else {
            panic!("graph")
        };
        assert_eq!(ntriples.lines().count(), count);
        assert!(
            !ntriples.contains("tc0_0_0")
                && !ntriples.contains("inbound")
                && !ntriples.contains("outside")
        );
    }
}

#[test]
fn actual_v3_order_query_nodes_and_dataset_merge_preserve_identity() {
    let correlated = witness(
        "CONSTRUCT { _:fresh ex:p ?s } WHERE { ?s ex:p ?o FILTER EXISTS { ?s ex:p ?x } }",
        "<http://ex/a> <http://ex/p> <http://ex/b> .",
        &[],
    );
    let actual = execute(&correlated);
    assert_eq!(actual, v3::evaluate(&correlated).unwrap());
    let v3::CanonicalResult::Graph { ntriples } = actual.result else {
        panic!("graph")
    };
    assert_eq!(ntriples.lines().count(), 1);
    for query in [
        "SELECT ?s ?n { ?s ex:p ?n } ORDER BY ?n",
        "SELECT ?s ?n { { SELECT ?s ?n { ?s ex:p ?n } ORDER BY DESC(?n) } }",
        "SELECT ?n { _:query ex:p ?n } ORDER BY ?n",
        "CONSTRUCT { ex:s ex:p ex:a } WHERE { VALUES ?v { 1 1 } }",
        "DESCRIBE ?s WHERE { VALUES ?s { ex:a ex:a } }",
    ] {
        let input = witness(query, SHARED_NODE, &[]);
        assert_eq!(execute(&input), v3::evaluate(&input).unwrap());
    }
    let source = "_:same <http://ex/p> <http://ex/a> <http://ex/g1> .\n_:same <http://ex/q> <http://ex/b> <http://ex/g2> .";
    let named = ["http://ex/g1", "http://ex/g2", "http://ex/empty"];
    for (query, expected) in [
        (
            "ASK { GRAPH ex:g1 { ?s ex:p ex:a } GRAPH ex:g2 { ?s ex:q ex:b } GRAPH ex:empty {} }",
            true,
        ),
        (
            "ASK FROM ex:g1 FROM ex:g2 { ?s ex:p ex:a; ex:q ex:b }",
            false,
        ),
    ] {
        let input = witness(query, source, &named);
        assert_eq!(execute(&input).result, v3::CanonicalResult::Ask(expected));
    }
    let input = witness(
        "CONSTRUCT { ?s ?p ?o } FROM ex:g1 FROM ex:g2 WHERE { ?s ?p ?o }",
        source,
        &named,
    );
    assert_eq!(execute(&input), v3::evaluate(&input).unwrap());
}

#[test]
fn actual_v3_limits_and_admission_fail_with_the_relation_panic() {
    let base = witness(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        "_:a <http://ex/p> _:b .\n_:b <http://ex/p> _:a .",
        &[],
    );
    execute(&base);
    let mut rejected = Vec::new();
    for index in 0..6 {
        let mut input = base.clone();
        let policy = &mut input.request.policy;
        match index {
            0 => policy.canonicalization.max_quads = 1,
            1 => policy.canonicalization.max_input_bytes = 1,
            2 => policy.canonicalization.max_output_bytes = 1,
            3 => policy.canonicalization.max_hndq_calls = 1,
            4 => policy.canonicalization.max_permutation_steps = 1,
            _ => policy.dataset.max_rows = 1,
        }
        rejected.push(input);
    }
    let mut output = witness("ASK {}", "", &[]);
    output.request.policy.canonicalization.max_output_bytes = 1;
    rejected.push(output);
    for query in [
        "SELECT (BNODE() AS ?x) {}",
        "SELECT (RAND() AS ?x) {}",
        "SELECT * { SERVICE ex:service {} }",
        "SELECT * { FILTER EXISTS { GRAPH ex:g {} } }",
    ] {
        rejected.push(witness(query, "", &[]));
    }
    for expression in [
        "(\"9223372036854775808\"^^<http://www.w3.org/2001/XMLSchema#integer> + 0)",
        "STRDT(CONCAT(\"0000\",\"-01-01T00:00:00Z\"), <http://www.w3.org/2001/XMLSchema#dateTime>)",
    ] {
        for form in ["SELECT ?v", "ASK", "CONSTRUCT {}", "DESCRIBE ex:a"] {
            rejected.push(witness(
                &format!("{form} WHERE {{ BIND({expression} AS ?v) }}"),
                "",
                &[],
            ));
        }
    }
    rejected.push(witness(
        "ASK {}",
        "_:s <http://ex/p> <http://ex/o> _:g .",
        &[],
    ));
    let mut omitted = witness("ASK {}", "", &["http://ex/empty"]);
    accepted(&mut omitted);
    omitted.dataset.named_graphs.clear();
    rejected.push(omitted);
    for query in [
        "ASK { ?s ex:p ?o FILTER EXISTS { ?s ex:p ?x } }",
        "SELECT ?s { ?s ex:p ?o FILTER NOT EXISTS { ?s ex:p ?x } }",
    ] {
        rejected.push(witness(query, SHARED_NODE, &[]));
    }
    for (source, named) in [
        ("<http://ex/a> <http://ex/p> _:b .", vec![]),
        (
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n_:unselected <http://ex/p> <http://ex/b> <http://ex/g> .",
            vec!["http://ex/g"],
        ),
    ] {
        rejected.push(witness(
            "ASK { ?s ex:p ?o FILTER EXISTS { ?s ex:p ?x } }",
            source,
            &named,
        ));
    }
    assert_eq!(rejected.len(), 25);
    for input in rejected {
        assert!(v3::evaluate(&input).is_err());
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 25))
            .write(&input)
            .unwrap()
            .build()
            .unwrap();
        let error = executor()
            .execute(env, embedded_artifact())
            .expect_err("invalid V3 relation");
        let text = format!("{error:#}");
        assert!(
            text.contains("Guest panicked:")
                && text.contains("bounded exact-dataset relation rejected"),
            "unexpected executor failure: {text}"
        );
    }
}

#[test]
fn actual_v3_wire_aliases_and_trailing_words_reject_without_journals() {
    let original = witness("ASK {}", "", &[]);
    execute(&original);
    let mut words = risc0_zkvm::serde::to_vec(&original).unwrap();
    let mut invalid = Vec::new();
    for version in [1, 2, 4] {
        words[0] = version;
        invalid.push(
            words
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        );
    }
    words[0] = v3::VERSION;
    words.push(0);
    invalid.push(words.iter().flat_map(|word| word.to_le_bytes()).collect());
    for bytes in invalid {
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 25))
            .write_slice(&bytes)
            .build()
            .unwrap();
        let error = executor()
            .execute(env, embedded_artifact())
            .expect_err("V3 transport alias rejected");
        let text = format!("{error:#}");
        assert!(
            text.contains("Guest panicked:")
                && text.contains("bounded exact-dataset relation rejected"),
            "unexpected executor failure: {text}"
        );
    }
}
