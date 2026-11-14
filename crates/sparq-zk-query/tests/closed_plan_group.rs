mod closed_native;
#[path = "support/fixture.rs"]
mod fixture_support;
#[path = "support/mod.rs"]
mod support;
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    codegen::generate_operator_fixture,
    credential::SuiteEncoder,
    interp::{evaluate, EncodeTerm},
    lower::lower,
    witness::prover_toml,
};
const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 4,
    rows_per_table: 4,
};
#[test]
fn suite_constants_for_closed_profile_match() {
    use sparq_zk_query::credential_graphs::{name, ISSUER, VERIFIED_ISSUER};
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk/noir-query/core/src/ops_closed_dataset.nr"),
    )
    .unwrap();
    assert_eq!(
        name(0).unwrap(),
        sparq_zk_query::credential::credential_graph([0; 32], 0).unwrap()
    );
    assert_eq!(VERIFIED_ISSUER, sparq_zk_query::credential::VERIFIED_ISSUER);
    assert!(source.contains("crate::term::credential_graph"));
    assert!(source.contains("crate::ops_core_suite::VERIFIED_ISSUER_ENCODING"));
    let issuer=SuiteEncoder.encode(&fixture_support::iri(ISSUER)).unwrap();
    let suite_source=std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk/noir-query/core/src/suite_constants.nr")).unwrap();
    assert!(suite_source.contains(&format!("0x{}",issuer.enc.iter().map(|b|format!("{b:02x}")).collect::<String>())));
    assert!(source.contains("crate::term::iri_issuer().identity()"));
}
#[test]
fn groups_having_subquery_and_empty_sum_execute() {
    for query in [
        "SELECT (SUM(?x) AS ?s) (COUNT(?x) AS ?c) WHERE {VALUES ?x {1 2 3}}",
        "SELECT (SUM(?x) AS ?s) (COUNT(?x) AS ?c) WHERE {VALUES ?x {}}",
        "SELECT ?g (SUM(?x) AS ?s) WHERE {VALUES (?g ?x) {(1 2) (1 3) (2 1)}} GROUP BY ?g HAVING(SUM(?x)>2)",
        "ASK {{SELECT (SUM(?r) AS ?rec) (SUM(?m) AS ?total) WHERE {VALUES (?r ?m) {(3 5) (1 5)}}} FILTER(?total>0) FILTER(?rec*10>=?total*3)}",
        "SELECT (MIN(?x) AS ?a) (MAX(?x) AS ?b) (AVG(?x) AS ?c) (SAMPLE(?x) AS ?d) WHERE {VALUES ?x {2 2}}",
    ]{
        eprintln!("executing {query}");
        let p=lower(query,B).unwrap();let trace=evaluate(&p,&[],&SuiteEncoder).unwrap();fixture_support::assert_engine(query,"",&trace.result);
        let pkg=generate_operator_fixture(&p).unwrap();let workspace=closed_native::Workspace::new(&pkg);let driver=ProgramProver::new(workspace.0.path(),&pkg.name).unwrap();
        let input=prover_toml(&p,&[],&SuiteEncoder,&trace,[7;32],[3;32]).unwrap();driver.execute(&input,"group").unwrap_or_else(|e|panic!("{query}: {e}"));
        let group=p.nodes().iter().position(|n|matches!(n.operator,sparq_zk_query::plan::Operator::Closed(sparq_zk_query::plan_closed::ClosedOperator::Group{..}))).unwrap();
        let mut bad=trace.clone();bad.tables[group].rows.clear();let input=prover_toml(&p,&[],&SuiteEncoder,&bad,[7;32],[3;32]).unwrap();assert!(driver.execute(&input,"missing_group").is_err());
    }
}
#[test]
fn aggregate_corpus_executes_with_engine_expected_and_altered_controls() {
    use sparq_zk_query::interp::ResultValue;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("../../zk/sparql-evaluator/fixtures/conformance/cases.json")).unwrap()).unwrap();
    for fixture in corpus["cases"].as_array().unwrap().iter().filter(|c| {
        c["id"].as_str().unwrap().starts_with("aggregate-")
            && c["id"] != "aggregate-group-concat-unambiguous"
    }) {
        let query = fixture["query"].as_str().unwrap();
        eprintln!("executing corpus {}", fixture["id"]);
        let rdf = fixture["dataset"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| std::fs::read_to_string(root.join("../../zk/sparql-evaluator/fixtures/conformance/core.nt")).unwrap());
        let (data, _) = fixture_support::dataset(&rdf);
        let p = lower(
            query,
            Bounds {
                quads_per_credential: 16,
                ..B
            },
        )
        .unwrap();
        let trace = evaluate(&p, &data, &SuiteEncoder).unwrap();
        fixture_support::assert_engine(query, &rdf, &trace.result);
        let expected = &fixture["expected"]["result"];
        match &trace.result {
            ResultValue::Ask(v) => assert_eq!(Some(*v), expected["Ask"].as_bool()),
            ResultValue::Select { variables, rows } => {
                let mut expected_rows = vec![];
                for row in expected["Select"]["rows"].as_array().unwrap() {
                    let mut r = vec![];
                    for term in row.as_array().unwrap() {
                        if let Some(text) = term.as_str() {
                            let (_, terms) =
                                fixture_support::dataset(&format!("<urn:s> <urn:p> {text} ."));
                            r.push(Some(SuiteEncoder.encode(&terms[2]).unwrap().enc));
                        } else {
                            r.push(None);
                        }
                    }
                    expected_rows.push(r);
                }
                let mut actual = rows
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|c| c.bound.then_some(c.enc))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                actual.sort();
                expected_rows.sort();
                assert_eq!(actual, expected_rows, "{}", fixture["id"]);
                assert_eq!(
                    variables,
                    &expected["Select"]["variables"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>()
                );
            }
        }
        let pkg = generate_operator_fixture(&p).unwrap();
        let workspace = closed_native::Workspace::new(&pkg);
        let driver = ProgramProver::new(workspace.0.path(), &pkg.name).unwrap();
        let render = |t: &sparq_zk_query::interp::Trace| {
            prover_toml(&p, &data, &SuiteEncoder, t, [7; 32], [3; 32]).unwrap()
        };
        driver.execute(&render(&trace), "aggregate_corpus").unwrap();
        let mut bad = trace.clone();
        let group = p
            .nodes()
            .iter()
            .position(|n| {
                matches!(
                    n.operator,
                    sparq_zk_query::plan::Operator::Closed(
                        sparq_zk_query::plan_closed::ClosedOperator::Group { .. }
                    )
                )
            })
            .unwrap();
        bad.tables[group].rows.clear();
        assert!(driver
            .execute(&render(&bad), "aggregate_hidden_group")
            .is_err());
        let mut bad = trace.clone();
        match &mut bad.result {
            ResultValue::Ask(v) => *v = !*v,
            ResultValue::Select { rows, .. } => rows[0][0].enc[31] ^= 1,
        };
        assert!(driver
            .execute(&render(&bad), "aggregate_wrong_result")
            .is_err());
    }
}
#[test]
fn issuer_key_name_vector_matches_the_suite() {
    let key = sparq_zk::sig::SecretKey::from_seed(19).public_key();
    let name = sparq_zk_query::credential_graphs::key_name(&key).unwrap();
    let c = SuiteEncoder.encode(&name.into()).unwrap();
    let hex = |v: &sparq_merkle_suite::Fr| {
        sparq_merkle_suite::poseidon::field_bytes(*v)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk/noir-query/core/src/ops_closed_issuer_standin.nr"),
    )
    .unwrap();
    for text in [
        hex(&key.0.x),
        hex(&key.0.y),
        c.enc.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    ] {
        assert!(source.contains(&format!("0x{text}")));
    }
}
#[test]
fn correlated_aggregates_refuse_instead_of_reading_erased_capture_cells() {
    let error = lower(
        "ASK {VALUES ?x {1} FILTER EXISTS {{SELECT (COUNT(?x) AS ?c) WHERE {<urn:s> <urn:p> ?x}}}}",
        B,
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("correlated GROUP BY / aggregate substitution"));
}
#[test]
fn credential_profile_codegen_only_wires_the_checked_activation_flags() {
    use sparq_zk_query::credential_graphs as graphs;
    let p = graphs::bind_issuer_registry(
        graphs::lower("ASK {}", B).unwrap(),
        &[(
            oxrdf::NamedNode::new_unchecked("urn:issuer"),
            sparq_zk::sig::SecretKey::from_seed(19).public_key(),
        )],
    )
    .unwrap();
    let package = sparq_zk_query::codegen::generate(&p).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    assert!(source.contains("credential_proof_0.pk, credential_flags[0],"));
    for token in ["assert(", "for ", "if ", " < credential_count"] {
        assert!(!source.contains(token), "{token}");
    }
}

#[test]
fn numeric_promotion_and_decimal_sum_bind_execute() {
    let Some(gate) = support::gate("numeric_promotion_and_decimal_sum_bind_execute", &["nargo"])
    else {
        return;
    };
    for query in [
        "ASK {FILTER(1 + 2.5 = 3.5)}",
        "ASK {FILTER(281474976710656 * 1 = 281474976710656)}",
        "SELECT DISTINCT ?result WHERE {{SELECT (SUM(?mass) AS ?total) (COUNT(DISTINCT ?iss) AS ?n) WHERE {VALUES (?iss ?mass) {(<urn:a> 200.0) (<urn:b> 200.0) (<urn:c> 200.0)}}} BIND((?total>=500 && ?n=3) AS ?result)}",
        "SELECT ?lot (SUM(?mass) AS ?total) WHERE {VALUES (?lot ?iss ?mass) {(<urn:lot> <urn:a> 200.0) (<urn:lot> <urn:b> 200.0) (<urn:lot> <urn:c> 200.0)}} GROUP BY ?lot HAVING(SUM(?mass)>=500 && COUNT(DISTINCT ?iss)=3)",
    ]{
        let p=lower(query,B).unwrap();let trace=evaluate(&p,&[],&SuiteEncoder).unwrap();fixture_support::assert_engine(query,"",&trace.result);
        let pkg=generate_operator_fixture(&p).unwrap();let root=closed_native::Workspace::new(&pkg);let driver=ProgramProver::new(root.0.path(),&pkg.name).unwrap();
        let render=|t|prover_toml(&p,&[],&SuiteEncoder,t,[7;32],[3;32]).unwrap();driver.execute(&render(&trace),"decimal_honest").unwrap();
        let mut wrong=trace.clone();match &mut wrong.result{sparq_zk_query::interp::ResultValue::Ask(v)=>*v = !*v,sparq_zk_query::interp::ResultValue::Select{rows,..}=>rows[0][0].enc[31]^=1};
        assert!(driver.execute(&render(&wrong),"decimal_wrong_result").is_err());
    }
    support::complete(gate);
}
