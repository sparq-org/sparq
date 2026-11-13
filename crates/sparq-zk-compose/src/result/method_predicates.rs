//! Method-bound integer and date predicates, including private-input constraints.
use super::tests::{alter_input, numeric_fixture};
use super::*;
use crate::manifest::FilterOp;
use crate::planner::PredicateDatatype;
use crate::verifier::InMemorySeenNonces;
use oxrdf::{Literal, NamedNode};
use std::time::Instant;

fn date(lexical: &str) -> Term {
    Literal::new_typed_literal(
        lexical,
        NamedNode::new("http://www.w3.org/2001/XMLSchema#date").unwrap(),
    )
    .into()
}
fn integer(value: u64) -> Term {
    Literal::new_typed_literal(
        value.to_string(),
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}
fn query(op: &str, literal: &str) -> String {
    format!(
        "SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age FILTER(?age {op} {literal}) }}"
    )
}
fn date_query(op: &str, lexical: &str) -> String {
    query(op, &date(lexical).to_string())
}
fn fixture(
    term: Term,
    depth: u32,
) -> (
    ResultCredential,
    ResultPolicy,
    VerifierNonce,
    Vec<BTreeMap<String, Term>>,
) {
    let bytes = (1usize << depth) / 8;
    let (mut c, mut policy, nonce, rows) = numeric_fixture(term, 3, bytes);
    let reference = sig::status_ref_digest(
        &sig::status_list_id_to_field(&c.status_list),
        c.status_index,
        c.status_version,
    );
    let message = sig::presentation_message_with_status_and_scheme(
        &c.graph.commitment,
        &c.graph.salt,
        &reference,
        &sig::scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri()),
    );
    c.signature = sig::sign_deterministic(&sparq_zk::sig::SecretKey::from_seed(1), &message);
    policy.admit_unbound_legacy = false;
    (c, policy, nonce, rows)
}

fn credential_set(
    c: ResultCredential,
    mut policy: ResultPolicy,
    capacity: CredentialCapacity,
    q: String,
) -> (Vec<ResultCredential>, ResultPolicy, String) {
    if capacity == CredentialCapacity::Smallest {
        return (vec![c], policy, q);
    }
    let graph = sparq_zk::commit::commit_triples(
        &[Triple::new(
            NamedNode::new("urn:alice").unwrap(),
            NamedNode::new("urn:licensed").unwrap(),
            NamedNode::new("urn:yes").unwrap(),
        )],
        Fr::from(717u64),
    )
    .unwrap();
    let sk = sparq_zk::sig::SecretKey::from_seed(2);
    let reference = sig::status_ref_digest(
        &sig::status_list_id_to_field(&c.status_list),
        9,
        c.status_version,
    );
    let message = sig::presentation_message_with_status_and_scheme(
        &graph.commitment,
        &graph.salt,
        &reference,
        &sig::scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri()),
    );
    let second = ResultCredential {
        graph,
        issuer: sk.public_key(),
        signature: sig::sign_deterministic(&sk, &message),
        status_list: c.status_list.clone(),
        status_version: c.status_version,
        status_index: 9,
    };
    policy.trusted_issuers.push(second.issuer);
    (
        vec![c, second],
        policy,
        q.replace(" FILTER", " . ?person <urn:licensed> <urn:yes> FILTER"),
    )
}

#[test]
fn date_query_admission_preserves_datatype_and_scope() {
    for lexical in [
        "0001-01-01",
        "9999-12-31",
        "2000-02-29",
        "2024-02-29",
        "2100-02-28",
        "2023-01-31",
        "2023-02-28",
        "2023-03-31",
        "2023-04-30",
        "2023-05-31",
        "2023-06-30",
        "2023-07-31",
        "2023-08-31",
        "2023-09-30",
        "2023-10-31",
        "2023-11-30",
        "2023-12-31",
    ] {
        let parsed = DisclosureQuery::parse(&date_query("<=", lexical)).unwrap();
        assert_eq!(
            parsed.filters[0].bound,
            lexical.replace('-', "").parse::<u64>().unwrap()
        );
    }
    let q = DisclosureQuery::parse(&date_query("<=", "2024-02-29")).unwrap();
    assert_eq!(q.filters[0].bound, 20240229);
    assert_eq!(q.filters[0].op, FilterOp::Le);
    assert_eq!(q.filters[0].datatype, PredicateDatatype::Date);
    assert_eq!(
        q.filters[0].value(NumericProfile::Unsigned, &date("2024-02-29")),
        Some(20240229)
    );
    assert_eq!(
        q.filters[0].value(NumericProfile::Unsigned, &integer(20240229)),
        None
    );
    let q = DisclosureQuery::parse(&query(">=", "20240229")).unwrap();
    assert_eq!(
        q.filters[0].value(NumericProfile::Unsigned, &date("2024-02-29")),
        None
    );
    for lexical in [
        "2024-02-29Z",
        "2024-02-29+01:00",
        "2024-2-29",
        "0000-01-01",
        "2100-02-29",
        "2023-04-31",
    ] {
        let error = DisclosureQuery::parse(&date_query("<=", lexical))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("xsd:date requires valid YYYY-MM-DD without timezone"),
            "{error}"
        );
    }
    let error = DisclosureQuery::parse(&query(
        "<=",
        "\"2024-02-29T00:00:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("xsd:dateTime FILTER"), "{error}");
    let q = "SELECT DISTINCT ?person WHERE { { ?person <urn:p> ?x FILTER(?age <= \"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>) } ?person <urn:age> ?age }";
    assert!(DisclosureQuery::parse(q)
        .unwrap_err()
        .to_string()
        .contains("FILTER variable outside its input scope"));
}

#[test]
fn method_bound_predicate_selection_and_named_refusals() {
    for depth in [10, 17, 20] {
        for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
            let (c, policy, nonce, rows) = fixture(integer(42), depth);
            let prepared = prepare_result_with_options(
                &query(">=", "18"),
                &[c],
                &rows,
                &policy,
                &nonce,
                ResultOptions {
                    credential_capacity: capacity,
                    integer_capacity: IntegerCapacityPolicy::HideInU64,
                    ..ResultOptions::default()
                },
            )
            .unwrap();
            let k = if capacity == CredentialCapacity::Smallest {
                1
            } else {
                2
            };
            assert_eq!(
                prepared.package,
                format!("result_v5_k{k}_n16_p3_r4_f2_i64_d{depth}")
            );
            assert_eq!(
                prepared.presentation.integer_capacity,
                PrivateIntegerCapacity::FullU64
            );
            assert_eq!(
                public_statement(&prepared.presentation, &policy, &nonce)
                    .unwrap()
                    .package,
                prepared.package
            );
        }
    }
    let (c, policy, nonce, rows) = fixture(date("2024-02-29"), 10);
    let p = prepare_result(
        &date_query("<=", "2024-02-29"),
        &[c],
        &rows,
        &policy,
        &nonce,
    )
    .unwrap();
    assert_eq!(p.package, "result_v5_k1_n16_p3_r4_f1_date_d10");
    assert_eq!(
        public_statement(&p.presentation, &policy, &nonce)
            .unwrap()
            .package,
        p.package
    );
    let mut different = p.presentation.clone();
    different.query = query("<=", "20240229");
    assert_ne!(
        public_statement(&different, &policy, &nonce)
            .unwrap()
            .package,
        p.package
    );
    let (c, policy, nonce, rows) = fixture(date("2024-02-29"), 17);
    assert!(prepare_result(
        &date_query("<=", "2024-02-29"),
        &[c],
        &rows,
        &policy,
        &nonce
    )
    .err()
    .unwrap()
    .to_string()
    .contains("no method-bound xsd:date member"));
    let (c, mut policy, nonce, rows) = numeric_fixture(integer(42), 3, 128);
    policy.admit_unbound_legacy = false;
    let reference = sig::status_ref_digest(
        &sig::status_list_id_to_field(&c.status_list),
        c.status_index,
        c.status_version,
    );
    let error = authenticate_string_canonical(
        FoldedMessage::MethodBound,
        false,
        &c.issuer,
        &c.signature,
        &c.graph.commitment,
        &c.graph.salt,
        &reference,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("attests no commitment method"), "{error}");
    let error = prepare_result(&query(">=", "18"), &[c], &rows, &policy, &nonce)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("attests no commitment method"), "{error}");
}

#[test]
fn full_width_legacy_presentations_retain_a_named_refusal() {
    let (c, mut policy, nonce, rows) = numeric_fixture(integer(100), 3, 128);
    let q = query(">=", "18");
    let p = prepare_result(&q, &[c], &rows, &policy, &nonce).unwrap();
    assert_eq!(p.presentation.version, 2);
    assert!(p.presentation.proof.is_empty());
    policy.admit_unbound_legacy = false;
    let driver = CircuitProver::from_crate_root();
    let scratch = std::env::temp_dir().join("sparq_legacy_refusal_no_proof");
    let Err(ResultError::Rejected(error)) = verify_result(
        &q,
        &p.presentation,
        &policy,
        &nonce,
        &InMemorySeenNonces::default(),
        &driver,
        &scratch,
    ) else {
        panic!("expected a legacy refusal before proof verification");
    };
    assert!(
        error.contains("method-unbound legacy presentation"),
        "{error}"
    );
    assert!(
        error.contains("ResultPolicy::admit_unbound_legacy"),
        "{error}"
    );
    policy.admit_unbound_legacy = true;
    let Err(ResultError::Rejected(error)) = verify_result(
        &q,
        &p.presentation,
        &policy,
        &nonce,
        &InMemorySeenNonces::default(),
        &driver,
        &scratch,
    ) else {
        panic!("expected the missing-proof refusal before any subprocess");
    };
    assert_eq!(error, "missing proof");
}

#[test]
fn date_predicates_keep_their_datatype_in_both_planners_and_public_checks() {
    use crate::planner::{plan_disclosure_admitted, optimize_disclosure_admitted};
    let (c, policy, nonce, rows) = fixture(date("2024-02-29"), 10);
    let q = date_query("<=", "2024-02-29");
    let parsed = DisclosureQuery::parse(&q).unwrap();
    assert!(plan_disclosure_admitted(
        &parsed,
        std::slice::from_ref(&c.graph),
        &rows,
        PlannerLimits::default(),
        |_, _, _| true
    )
    .is_ok());
    assert!(optimize_disclosure_admitted(
        &parsed,
        std::slice::from_ref(&c.graph),
        &rows,
        OptimizationLimits::default(),
        |_, _, _| true
    )
    .unwrap()
    .plan
    .is_some());
    let reversed = q.replace(
        &format!("?age <= {}", date("2024-02-29")),
        &format!("{} >= ?age", date("2024-02-29")),
    );
    assert_eq!(
        DisclosureQuery::parse(&reversed).unwrap().filters,
        parsed.filters
    );
    let public = q.replace("SELECT DISTINCT ?person", "SELECT DISTINCT ?age");
    let public_rows = vec![BTreeMap::from([("age".into(), date("2024-02-29"))])];
    let p = prepare_result(
        &public,
        std::slice::from_ref(&c),
        &public_rows,
        &policy,
        &nonce,
    )
    .unwrap();
    assert_eq!(p.package, "result_v5_k1_n16_p3_r4_f0_d10");
    assert!(!p.toml.contains("date_literals"));
    let mut bad = p.presentation.clone();
    bad.rows[0].insert("age".into(), disclosed(&date("2024-03-01")).unwrap());
    assert!(public_statement(&bad, &policy, &nonce)
        .err()
        .unwrap()
        .to_string()
        .contains("public FILTER is false"));
    for q in [
        format!("SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age FILTER(?age >= {} && ?age <= {}) }}", date("2024-02-28"), date("2024-03-01")),
        format!("SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age . ?person <urn:funds> ?funds FILTER(?age <= {} && ?funds >= 18) }}", date("2024-03-01")),
    ] {
        let error = prepare_result(&q, std::slice::from_ref(&c), &rows, &policy, &nonce).err().unwrap().to_string();
        assert!(error.contains("hidden xsd:date requires a single date predicate"), "{error}");
    }
    let int_query = query(">=", "20240229");
    let int_parsed = DisclosureQuery::parse(&int_query).unwrap();
    assert!(plan_disclosure_admitted(
        &int_parsed,
        std::slice::from_ref(&c.graph),
        &rows,
        PlannerLimits::default(),
        |_, _, _| true
    )
    .is_err());
    assert!(optimize_disclosure_admitted(
        &int_parsed,
        std::slice::from_ref(&c.graph),
        &rows,
        OptimizationLimits::default(),
        |_, _, _| true
    )
    .unwrap()
    .plan
    .is_none());
}

fn mismatch(driver: &CircuitProver, p: &PreparedResult, input: &str, expected: &str, tag: &str) {
    match driver.private_package_witness(p.package, input, tag) {
        Err(DriverError::Tool { tool, stderr }) => assert!(
            tool.starts_with("nargo") && stderr.contains(expected),
            "{tag}: {tool}: {stderr}"
        ),
        Err(other) => panic!("{tag}: tool failure: {other}"),
        Ok(_) => panic!("{tag}: inconsistent witness satisfied the relation"),
    }
}

#[test]
fn live_method_bound_integer_and_date_predicates() {
    let Some(live) = crate::support::gate(
        "live_method_bound_integer_and_date_predicates",
        &["nargo", "bb"],
    ) else {
        return;
    };
    pinned_toolchain().unwrap();
    let driver = CircuitProver::from_crate_root();
    let dir = std::env::temp_dir().join(format!("sparq_method_predicates_{}", std::process::id()));
    for depth in [10, 17, 20] {
        for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
            let (c, policy, nonce, rows) = fixture(integer(u64::MAX), depth);
            let (credentials, policy, q) =
                credential_set(c, policy, capacity, query(">=", "18446744073709551615"));
            let p = prepare_result_with_options(
                &q,
                &credentials,
                &rows,
                &policy,
                &nonce,
                ResultOptions {
                    credential_capacity: capacity,
                    integer_capacity: IntegerCapacityPolicy::HideInU64,
                    ..ResultOptions::default()
                },
            )
            .unwrap();
            assert_eq!(p.work.selected_credentials, credentials.len());
            driver
                .private_package_witness(p.package, &p.toml, "integer_honest")
                .unwrap();
            mismatch(
                &driver,
                &p,
                &alter_input(&p.toml, "filter_ops", |v| v[0] = json!(2)),
                "filter verdict mismatch",
                "integer_verdict",
            );
            mismatch(
                &driver,
                &p,
                &alter_input(&p.toml, "filter_values", |v| {
                    v[0][0] = json!((u64::MAX - 1).to_string())
                }),
                "operand encoding mismatch",
                "integer_opening",
            );
            prove_and_check(p, &q, &policy, &nonce, &rows, &driver, &dir);
        }
    }
    for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
        for op in ["<=", ">="] {
            for lexical in ["2024-02-28", "2024-02-29", "2024-03-01"] {
                let (c, policy, nonce, rows) = fixture(date(lexical), 10);
                let (credentials, policy, q) =
                    credential_set(c, policy, capacity, date_query(op, "2024-02-29"));
                let result = prepare_result_with_options(
                    &q,
                    &credentials,
                    &rows,
                    &policy,
                    &nonce,
                    ResultOptions {
                        credential_capacity: capacity,
                        ..ResultOptions::default()
                    },
                );
                let satisfies = if op == "<=" {
                    lexical <= "2024-02-29"
                } else {
                    lexical >= "2024-02-29"
                };
                if !satisfies {
                    assert!(result.is_err());
                    let broad = q.replace(
                        "2024-02-29",
                        if op == "<=" {
                            "9999-12-31"
                        } else {
                            "0001-01-01"
                        },
                    );
                    let p = prepare_result_with_options(
                        &broad,
                        &credentials,
                        &rows,
                        &policy,
                        &nonce,
                        ResultOptions {
                            credential_capacity: capacity,
                            ..ResultOptions::default()
                        },
                    )
                    .unwrap();
                    driver
                        .private_package_witness(p.package, &p.toml, "date_outside_control")
                        .unwrap();
                    mismatch(
                        &driver,
                        &p,
                        &alter_input(&p.toml, "filter_bounds", |v| v[0] = json!(20240229)),
                        "filter verdict mismatch",
                        "date_outside_boundary",
                    );
                    continue;
                }
                let p = result.unwrap();
                assert_eq!(p.work.selected_credentials, credentials.len());
                driver
                    .private_package_witness(p.package, &p.toml, "date_honest")
                    .unwrap();
                if lexical == "2024-02-29" {
                    let false_op = if op == "<=" { 0 } else { 2 };
                    mismatch(
                        &driver,
                        &p,
                        &alter_input(&p.toml, "filter_ops", |v| v[0] = json!(false_op)),
                        "filter verdict mismatch",
                        "date_verdict",
                    );
                    mismatch(
                        &driver,
                        &p,
                        &alter_input(&p.toml, "date_literals", |v| v[0][0][10] = json!(56)),
                        "date key mismatch",
                        "date_literal",
                    );
                    mismatch(
                        &driver,
                        &p,
                        &alter_input(&p.toml, "filter_values", |v| v[0][0] = json!(20240228)),
                        "date key mismatch",
                        "date_key",
                    );
                    let changed =
                        alter_input(&p.toml, "date_literals", |v| v[0][0][10] = json!(56));
                    let changed =
                        alter_input(&changed, "filter_values", |v| v[0][0] = json!(20240228));
                    mismatch(
                        &driver,
                        &p,
                        &changed,
                        "operand encoding mismatch",
                        "date_opening",
                    );
                }
                prove_and_check(p, &q, &policy, &nonce, &rows, &driver, &dir);
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
    crate::support::complete(live);
}

fn prove_and_check(
    p: PreparedResult,
    q: &str,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    rows: &[BTreeMap<String, Term>],
    driver: &CircuitProver,
    dir: &Path,
) {
    driver.compile_package(p.package).unwrap();
    let member = p.package;
    let start = Instant::now();
    let proof = p.prove(driver, dir, "method_predicates").unwrap();
    let prove = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let checked = verify_result(
        q,
        &proof,
        policy,
        nonce,
        &InMemorySeenNonces::default(),
        driver,
        dir,
    )
    .unwrap();
    let verify = start.elapsed().as_secs_f64();
    assert_eq!(checked.rows, rows);
    eprintln!("RESULT_MEASUREMENT {member} prove_seconds={prove} verify_seconds={verify}");
    let mut changed = proof.clone();
    changed.query = q.replace(">=", ">").replace("<=", "<");
    assert!(verify_result(
        &changed.query,
        &changed,
        policy,
        nonce,
        &InMemorySeenNonces::default(),
        driver,
        dir
    )
    .is_err());
}
