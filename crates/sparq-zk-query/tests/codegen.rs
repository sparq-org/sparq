use sparq_zk_query::{
    capacity::{Bounds, OpeningProfile},
    codegen::generate,
    lower::lower,
};
const B: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};

#[test]
fn canonical_generation_keeps_fixture_issuer_profiles_explicit() {
    let plan = sparq_zk_query::credential_graphs::lower("ASK {}", B).unwrap();
    let plan = sparq_zk_query::credential_graphs::bind_issuer_registry(
        plan,
        &[(
            oxrdf::NamedNode::new_unchecked("urn:issuer"),
            sparq_zk::sig::SecretKey::from_seed(19).public_key(),
        )],
    )
    .unwrap();
    assert!(matches!(
        generate(&plan),
        Err(sparq_zk_query::Error::Unsupported(_))
    ));
    assert!(sparq_zk_query::codegen::generate_operator_fixture(&plan).is_ok());
}

#[test]
fn exact_targets_use_the_authenticated_named_view() {
    for (query, credentials, quads, rows, slots) in [
        (
            include_str!("fixtures/solid_paths/insurance-count.rq"),
            2,
            64,
            4,
            vec![16, 64],
        ),
        (
            include_str!("fixtures/solid_paths/insurance-closed.rq"),
            2,
            64,
            16,
            vec![16, 64],
        ),
        (
            include_str!("fixtures/solid_paths/trial-eligibility.rq"),
            1,
            64,
            32,
            vec![64],
        ),
        (
            include_str!("fixtures/solid_paths/recycled-min.rq"),
            4,
            16,
            4,
            vec![16; 4],
        ),
        (
            include_str!("fixtures/solid_paths/recycled-sum.rq"),
            4,
            16,
            4,
            vec![16; 4],
        ),
        (
            include_str!("fixtures/solid_paths/recycled-having.rq"),
            4,
            16,
            4,
            vec![16; 4],
        ),
    ] {
        let plan = lower(
            query,
            Bounds {
                credentials,
                quads_per_credential: quads,
                rows_per_table: rows,
            },
        )
        .unwrap()
        .with_credential_quad_bounds(&slots)
        .unwrap();
        let package = generate(&plan).unwrap();
        let source = &package.files[&format!("{}/src/main.nr", package.name)];
        assert!(source.contains("let closed_named_dataset = ops_core::named_view("));
        assert!(source.contains("ops_closed_dataset::graph_scan_bound("));
        assert!(!source.contains("ops_closed_issuer_standin"));
        assert!(!source.contains("ops_closed_dataset::request_graphs("));
        assert!(!source.contains("ops_closed_dataset::default_graph"));
        assert!(!source.contains("assert("));
        assert!(!source.contains("for "));
    }
}

#[test]
fn generation_is_byte_identical_and_constants_are_inputs() {
    let a = lower(
        "SELECT DISTINCT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 3)} LIMIT 1",
        B,
    )
    .unwrap();
    let b = lower(
        "SELECT DISTINCT ?v WHERE {?v <urn:q> ?y FILTER(?y >= 2 && ?y < 4)} LIMIT 2",
        B,
    )
    .unwrap();
    assert_eq!(generate(&a).unwrap(), generate(&a).unwrap());
    assert_eq!(generate(&a).unwrap(), generate(&b).unwrap());
    let package = generate(&a).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    for op in ["scan", "filter", "project", "distinct", "slice", "select"] {
        assert!(
            source.contains(&format!("ops_core::{op}("))
                || source.contains(&format!("ops_core::{op}::<")),
            "{op}"
        );
    }
    assert!(source.contains("ops_core_statement::bind_select_authenticated_modes("));
    assert!(source.contains("core::ops_core_envelope::open_slot"));
    assert!(!source.contains("urn:p"));
    assert!(!source.contains("assert("));
    assert!(!source.contains("for "));
    assert!(!source.contains("if "));
}

#[test]
fn join_values_membership_and_ask_are_gadget_calls() {
    let p = lower(
        "ASK {?s <urn:p> ?x . ?s <urn:q> ?y VALUES ?x {1 UNDEF} FILTER(?x IN (1,2))}",
        B,
    )
    .unwrap();
    let p = generate(&p).unwrap();
    let source = &p.files[&format!("{}/src/main.nr", p.name)];
    for op in ["join", "values", "ask"] {
        assert!(source.contains(&format!("ops_core::{op}(")));
    }
    assert!(source.contains("ops_core_expr::evaluate_indexed("));
    assert_eq!(p.files.len(), 3);
}

#[test]
fn committed_smoke_package_is_exactly_generated_with_its_original_manifest() {
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let package = generate(&plan).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../zk/noir-query/fixtures/generated_smoke");
    for (path, expected) in package.files {
        assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), expected);
    }
    assert_eq!(
        root.join("zk").canonicalize().unwrap(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap()
    );
}

#[test]
fn source_identity_binding_uses_authenticated_identity_and_consumer_checks() {
    for query in [
        "ASK {?s <urn:p> 1}",
        "SELECT ?s WHERE {?s <urn:p> ?x}",
        "ASK {?s <urn:p> ?x . ?s <urn:q> ?x}",
    ] {
        let package = generate(&lower(query, B).unwrap()).unwrap();
        let source = &package.files[&format!("{}/src/main.nr", package.name)];
        assert!(source.contains("open_slot::<4,64,65,32,2>"));
        assert!(!source.contains("ops_core_reads::check_with_parameters("));
    }
    let package = generate(&lower("ASK {}", B).unwrap()).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    assert!(source.contains("open_slot::<4,64,65,32,2>"));
}

#[test]
fn expression_reads_cover_the_entire_consumer_input_before_filtering() {
    let plan = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x <= 9)}",
        Bounds {
            credentials: 1,
            quads_per_credential: 64,
            rows_per_table: 32,
        },
    )
    .unwrap();
    let package = generate(&plan).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    let (i, input) = plan
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(i, node)| {
            if let sparq_zk_query::plan::Operator::Filter(input, _) = node.operator {
                Some((i, input))
            } else {
                None
            }
        })
        .unwrap();
    assert!(source.contains(&format!(
        "reads_{i}: [[core::suite_entry::BoundedTermOpening<65, 41, 1>; 1]; 32]"
    )));
    let check = source
        .find(&format!(
            "core::ops_core_reads::check_with_parameters(t{input}, reads_{i}, [1], parameters);"
        ))
        .unwrap();
    let eval = source.find(&format!("let predicate_{i} =")).unwrap();
    let filter = source.find(&format!("ops_core::filter(t{input},")).unwrap();
    assert!(check < eval && eval < filter);
    assert!(source.contains("open_slot::<64,64,65,32,1>"));
    assert!(!source.contains("source_index"));
}

#[test]
fn absent_expression_variables_stay_unbound_without_read_witnesses() {
    let plan = lower("ASK { FILTER(?absent > 1) }", B).unwrap();
    let package = generate(&plan).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    assert!(!source.contains("ops_core_reads::check_with_parameters("));
    assert!(!source.contains("reads_"));
    assert!(source.contains("ops_core_expr::evaluate_indexed("));
}

#[test]
fn heterogeneous_credential_buckets_are_shape_and_their_limits_are_public_inputs() {
    let b = Bounds {
        credentials: 4,
        quads_per_credential: 64,
        rows_per_table: 4,
    };
    let a = lower("ASK {}", b)
        .unwrap()
        .with_credential_quad_bounds(&[8, 16, 64, 8])
        .unwrap();
    let same = lower("ASK {}", b)
        .unwrap()
        .with_credential_quad_bounds(&[7, 15, 63, 5])
        .unwrap();
    let other = lower("ASK {}", b)
        .unwrap()
        .with_credential_quad_bounds(&[16, 8, 64, 8])
        .unwrap();
    assert_eq!(a.hash(), same.hash());
    assert_eq!(generate(&a).unwrap(), generate(&same).unwrap());
    assert_ne!(a.hash(), other.hash());
    assert_eq!(a.credential_quad_capacities(), vec![8, 16, 64, 8]);
    let p = generate(&a).unwrap();
    let source = &p.files[&format!("{}/src/main.nr", p.name)];
    assert!(source.contains("QuadOpening<64, 65>; 8]"));
    assert!(source.contains("BoundedCredentialProof<16>"));
    assert!(source.contains("BoundedCredentialProof<64>"));
    for token in ["assert(", "for ", "if "] {
        assert!(!source.contains(token));
    }
    assert!(a.clone().with_credential_quad_bounds(&[8, 16]).is_err());
    assert!(a.with_credential_quad_bounds(&[0, 16, 64, 8]).is_err());
}

#[test]
fn public_constant_lists_and_rule_values_do_not_change_the_program_shape() {
    let mut reference = None;
    for length in 0..=sparq_zk_query::plan::IN_CAPACITY {
        let members = (0..length)
            .map(|i| format!("\"medicine-{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let query=format!("SELECT ?s WHERE {{ ?g <urn:issuer-predicate:{length}> <urn:issuer:{length}> . ?s <urn:n:{length}> ?n . ?s <urn:date:{length}> ?d . ?s <urn:med:{length}> ?m . FILTER(?n >= {length} && ?d >= \"2026-11-13\"^^<http://www.w3.org/2001/XMLSchema#date> && ?m IN ({members})) }}");
        let plan = lower(&query, B).unwrap();
        let current = (plan.hash(), generate(&plan).unwrap());
        if let Some(reference) = &reference {
            assert_eq!(&current, reference);
        } else {
            reference = Some(current);
        }
    }
    let members = (0..=sparq_zk_query::plan::IN_CAPACITY)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        lower(
            &format!("ASK {{?s <urn:p> ?x FILTER(?x IN ({members}))}}"),
            B
        )
        .unwrap_err(),
        sparq_zk_query::Error::Capacity("IN list".into())
    );
}

#[test]
fn verifier_selects_opening_profile_and_one_index_binding_per_slot() {
    let compact = lower("ASK {}", B).unwrap();
    let long = compact
        .clone()
        .with_opening_profile(OpeningProfile::LongObjects);
    assert_ne!(compact.hash(), long.hash());
    let p = generate(&long).unwrap();
    assert!(p.files[&format!("{}/src/main.nr", p.name)].contains("open_slot::<4,64,137,32,2>"));
    let wide = compact
        .clone()
        .with_opening_profile(OpeningProfile::Extended);
    assert_ne!(compact.hash(), wide.hash());
    let package = generate(&wide).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    assert!(source.contains("open_slot::<4,128,160,128,2>"));
    for i in 0..2 {
        assert_eq!(
            source
                .matches(&format!("let credential_index_{i}: u32 = {i};"))
                .count(),
            1
        );
        assert_eq!(
            source
                .matches(&format!("let (opened_{i}, issuer_{i}, used_{i})="))
                .count(),
            1
        );
        assert!(source.contains(&format!(
            "credential_quad_bounds[{i}],credential_index_{i},request_digest"
        )));
    }
    let changed = lower("ASK {}", B)
        .unwrap()
        .with_opening_profile(OpeningProfile::Extended);
    assert_eq!(wide.hash(), changed.hash());
    assert_eq!(generate(&wide).unwrap(), generate(&changed).unwrap());
}
