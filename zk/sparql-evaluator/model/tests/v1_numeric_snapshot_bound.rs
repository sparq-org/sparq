// Native checks of the versioned numeric consumers, not proof evidence.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, EvaluationCapacity, EvaluationError, Policy,
    PrivateDataset, ProofContract, Request, VERSION, Witness, admit, dataset_commitment,
    evaluate_detailed,
};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const INTEGER_MAX: &str = "9223372036854775807";
const INTEGER_NEXT: &str = "9223372036854775808";
const DECIMAL_MAX: &str = "170141183460469231731687303715884105727";
const DECIMAL_NEXT: &str = "170141183460469231731687303715884105728";

fn check(query: &str, data: &str, accepted: bool) {
    let dataset = PrivateDataset {
        ntriples: data.into(),
        salt: [7; 32],
    };
    let policy = Policy::default();
    let commitment = dataset_commitment(&dataset, &policy).unwrap();
    for authority in [
        DatasetAuthority::VerifierAgreed { commitment },
        DatasetAuthority::HolderDeclared,
    ] {
        let input = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11SnapshotV1,
                query: format!("PREFIX xsd:<{XSD}> {query}"),
                authority,
                policy: policy.clone(),
                nonce: [11; 32],
            },
            dataset: dataset.clone(),
        };
        admit(&input.request).expect("capacity is an evaluation bound");
        let actual = evaluate_detailed(&input);
        if accepted {
            let result = actual.unwrap_or_else(|error| panic!("{query}, {data}: {error}"));
            // A successful SELECT must retain a result, including unbound/error cells.
            if let CanonicalResult::Select { rows, .. } = result.result {
                assert!(!rows.is_empty(), "{query}, {data}");
            }
        } else {
            assert_eq!(
                actual.map(|journal| journal.result),
                Err(EvaluationError::Capacity(
                    EvaluationCapacity::NumericRepresentation
                )),
                "{query}, {data}",
            );
        }
    }
}

#[test]
fn v1_snapshot_numeric_representation_bound_checks_each_consuming_path() {
    // Stored FILTER/identity equality and full/top-k ordering consume numerics.
    // The decimal lexical parser accumulates magnitude before applying the sign,
    // so its negative lexical endpoint is -i128::MAX, not i128::MIN.
    for (datatype, inside, outside) in [
        ("integer", INTEGER_MAX.to_owned(), INTEGER_NEXT.to_owned()),
        (
            "integer",
            "-9223372036854775808".into(),
            "-9223372036854775809".into(),
        ),
        ("decimal", DECIMAL_MAX.to_owned(), DECIMAL_NEXT.to_owned()),
        (
            "decimal",
            format!("-{DECIMAL_MAX}"),
            format!("-{DECIMAL_NEXT}"),
        ),
    ] {
        for query in [
            "ASK {?s ?p ?n FILTER(?n < 0)}",
            "SELECT (COUNT(*) AS ?v) {?s ?p ?n FILTER(?n = 0)}",
            "SELECT ?n {?s ?p ?n FILTER(?n = ?n)}",
            "SELECT ?n {?s ?p ?n} ORDER BY ?n",
            "SELECT ?n {?s ?p ?n} ORDER BY ?n LIMIT 1",
        ] {
            for (lexical, accepted) in [(&inside, true), (&outside, false)] {
                let data =
                    format!("<http://ex/s> <http://ex/p> \"{lexical}\"^^<{XSD}{datatype}> .");
                check(query, &data, accepted);
            }
        }
    }
    // Arithmetic intermediates and string constructors retain their own endpoints.
    for (expression, accepted) in [
        (format!("{INTEGER_MAX} + 0"), true),
        (format!("{INTEGER_MAX} + 1"), false),
        (format!("\"{DECIMAL_MAX}\"^^xsd:decimal + 0"), true),
        (format!("\"{DECIMAL_MAX}\"^^xsd:decimal + 1"), false),
        (format!("xsd:integer(\"{INTEGER_MAX}\")"), true),
        (format!("xsd:integer(\"{INTEGER_NEXT}\")"), false),
        ("xsd:integer(\"-9223372036854775808\")".into(), true),
        ("xsd:integer(\"-9223372036854775809\")".into(), false),
        (format!("xsd:decimal(\"{DECIMAL_MAX}\")"), true),
        (format!("xsd:decimal(\"{DECIMAL_NEXT}\")"), false),
        // Comparing endpoints must not require their sum to fit.
        (
            format!("\"{DECIMAL_MAX}\"^^xsd:decimal = \"{DECIMAL_MAX}\"^^xsd:decimal"),
            true,
        ),
        // Computed i128::MIN does not pass through the lexical parser.
        (
            format!(
                "(-\"{DECIMAL_MAX}\"^^xsd:decimal - 1) = (-\"{DECIMAL_MAX}\"^^xsd:decimal - 1)"
            ),
            true,
        ),
    ] {
        check(
            &format!("SELECT (COALESCE({expression}, 0) AS ?v) {{}}"),
            "",
            accepted,
        );
    }
    // Adjacent scales around checked 10^scale alignment; even zero must align.
    // SECONDS creates a decimal at runtime, so scanning source numeric literals
    // would miss the same boundary in its intermediate result.
    for (scale, accepted) in [(38, true), (39, false)] {
        let tiny = format!("0.{}1", "0".repeat(scale - 1));
        for expression in [
            format!("\"{tiny}\"^^xsd:decimal < 1"),
            format!("\"{tiny}\"^^xsd:decimal = 0"),
            format!("SECONDS(\"2024-01-01T00:00:0{tiny}Z\"^^xsd:dateTime) = 0"),
            format!("xsd:decimal(\"{tiny}\") < 1"),
        ] {
            check(
                &format!("SELECT (COALESCE({expression}, 0) AS ?v) {{}}"),
                "",
                accepted,
            );
        }
        let data = format!(
            "<http://ex/a> <http://ex/p> \"{tiny}\"^^<{XSD}decimal> .\n\
             <http://ex/b> <http://ex/p> \"1\"^^<{XSD}integer> ."
        );
        for query in [
            "SELECT ?n {?s ?p ?n} ORDER BY ?n",
            "SELECT ?n {?s ?p ?n} ORDER BY ?n LIMIT 1",
        ] {
            check(query, &data, accepted);
        }
        // Scale is relative to the actual comparison partner, not a lexical cap.
        check(
            &format!("SELECT (\"{tiny}\"^^xsd:decimal = \"{tiny}\"^^xsd:decimal AS ?v) {{}}"),
            "",
            true,
        );
    }
}

#[test]
fn v1_snapshot_bound_preserves_unconsumed_values_and_expression_errors() {
    let huge = "100000000000000000000000000000000000000000000";
    let data = format!("<http://ex/s> <http://ex/p> \"{huge}\"^^<{XSD}integer> .");
    for query in [
        "SELECT ?n {?s ?p ?n}",
        "SELECT (STR(?n) AS ?v) {?s ?p ?n}",
        "SELECT (sameTerm(?n, ?n) AS ?v) {?s ?p ?n}",
        "SELECT (IF(false, ?n = 0, true) AS ?v) {?s ?p ?n}",
        "ASK {?s <http://ex/absent> ?n FILTER(?n < 0)}",
    ] {
        check(query, &data, true);
    }
    for query in [
        "SELECT (COALESCE(1/0, 42) AS ?v) {}",
        "SELECT (\"invalid\"^^xsd:integer = 0 AS ?v) {}",
        "SELECT (IF(false, xsd:integer(\"9223372036854775808\"), 42) AS ?v) {}",
        "SELECT (\"1E39\"^^xsd:decimal = 0 AS ?v) {}",
        // Large written scale alone is allowed until an alignment consumer runs.
        "SELECT (xsd:decimal(\"0.000000000000000000000000000000000000001\") AS ?v) {}",
    ] {
        check(query, "", true);
    }
}
