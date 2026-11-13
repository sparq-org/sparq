use sparq_zk_query::{
    capacity::Bounds,
    lower::lower,
    plan::{Operator, Parameter},
    Error,
};

const BOUNDS: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};

#[test]
fn shape_hash_ignores_constant_values_and_variable_names() {
    let a = lower(
        "SELECT ?s WHERE { ?s <urn:p> ?x FILTER(?x >= 3 && ?x < 8) } LIMIT 2",
        BOUNDS,
    )
    .unwrap();
    let b = lower(
        "SELECT ?v WHERE { ?v <urn:q> ?y FILTER(?y >= 4 && ?y < 9) } LIMIT 1",
        BOUNDS,
    )
    .unwrap();
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    assert_ne!(a.parameters(), b.parameters());
    assert_eq!(a.canonical_bytes(), a.clone().canonical_bytes());
    let c = lower(
        "SELECT DISTINCT ?s WHERE { ?s <urn:p> ?x FILTER(?x >= 3 && ?x < 8) } LIMIT 2",
        BOUNDS,
    )
    .unwrap();
    assert_ne!(a.hash(), c.hash());
    assert_ne!(
        a.hash(),
        lower(
            "SELECT ?s WHERE { ?s <urn:p> ?x FILTER(?x >= 3 && ?x < 8) } LIMIT 2",
            Bounds {
                rows_per_table: 8,
                ..BOUNDS
            }
        )
        .unwrap()
        .hash()
    );
}

#[test]
fn plan_has_topological_inputs_and_parameterized_values_and_slice() {
    let p=lower("SELECT DISTINCT ?s WHERE { ?s <urn:p> ?x . ?s <urn:q> ?y VALUES ?x {1 2} FILTER(?x IN (1,2)) } LIMIT 1 OFFSET 2",BOUNDS).unwrap();
    for (i, n) in p.nodes().iter().enumerate() {
        for id in n.operator.inputs() {
            assert!((id as usize) < i);
        }
    }
    assert!(p
        .nodes()
        .iter()
        .any(|n| matches!(n.operator, Operator::Join(..))));
    assert!(p.parameters().contains(&Parameter::Index(2)));
    assert!(p.parameters().contains(&Parameter::Index(1)));
    assert_eq!(p.result_schema().variables.len(), 1);
}

#[test]
fn public_bounds_select_buckets_and_private_counts_only_refuse() {
    assert_eq!(
        Bounds {
            rows_per_table: 3,
            ..BOUNDS
        }
        .capacity()
        .unwrap(),
        BOUNDS.capacity().unwrap()
    );
    assert!(BOUNDS.check_credentials(&[0, 4]).is_ok());
    assert!(matches!(
        BOUNDS.check_credentials(&[5]),
        Err(Error::Capacity(_))
    ));
    assert!(matches!(
        BOUNDS.check_credentials(&[0, 0, 0]),
        Err(Error::Capacity(_))
    ));
    for b in [
        Bounds {
            credentials: 0,
            ..BOUNDS
        },
        Bounds {
            credentials: 5,
            ..BOUNDS
        },
        Bounds {
            rows_per_table: 33,
            ..BOUNDS
        },
        Bounds {
            quads_per_credential: 257,
            ..BOUNDS
        },
    ] {
        assert!(matches!(b.capacity(), Err(Error::Capacity(_))));
    }
}

#[test]
fn exclusions_and_stage_refusals_are_named() {
    // Exercise check_exclusions through the public lowering path, including nested calls.
    for (query, name, permanent) in [
        ("SELECT * FROM <urn:g> WHERE {?s ?p ?o}", "FROM", true),
        (
            "SELECT * FROM NAMED <urn:g> WHERE {?s ?p ?o}",
            "FROM NAMED",
            true,
        ),
        ("DESCRIBE <urn:s>", "DESCRIBE", true),
        (
            "INSERT DATA { <urn:s> <urn:p> <urn:o> }",
            "SPARQL Update",
            true,
        ),
        ("ASK { SERVICE <urn:s> {?s ?p ?o} }", "SERVICE", true),
        ("SELECT (NOW() AS ?x) WHERE {}", "NOW", true),
        ("SELECT (RAND() AS ?x) WHERE {}", "RAND", true),
        ("SELECT (UUID() AS ?x) WHERE {}", "UUID", true),
        ("SELECT (STRUUID() AS ?x) WHERE {}", "STRUUID", true),
        ("SELECT (BNODE() AS ?x) WHERE {}", "BNODE", true),
        ("ASK {FILTER(ABS(1)=1)}", "ABS", false),
        ("SELECT * WHERE {?s ?p ?o} ORDER BY ?s", "ORDER BY", false),
    ] {
        let e = lower(query, BOUNDS).unwrap_err();
        assert_eq!(
            e,
            if permanent {
                Error::Excluded(name.into())
            } else {
                Error::Unsupported(name.into())
            },
            "{query}"
        );
        assert!(e.to_string().contains(name));
    }
    assert!(matches!(lower("SELECT", BOUNDS), Err(Error::Parse(_))));
}

#[test]
fn rdf12_terms_are_refused_in_values_patterns_and_expressions() {
    for query in [
        "SELECT ?x WHERE {VALUES ?x {<<( <urn:s> <urn:p> <urn:o> )>>}}",
        "SELECT * WHERE {<urn:s> <urn:p> <<( <urn:a> <urn:b> <urn:c> )>>}",
        "SELECT * WHERE {{} MINUS {VALUES ?x {<<( <urn:s> <urn:p> <urn:o> )>>}}}",
        "SELECT * WHERE {{} MINUS {<urn:s> <urn:p> <<( <urn:a> <urn:b> <urn:c> )>>}}",
    ] {
        assert_eq!(
            lower(query, BOUNDS).unwrap_err(),
            Error::Excluded("triple term".into()),
            "{query}"
        );
    }
    assert_eq!(
        lower(
            "SELECT (TRIPLE(<urn:s>,<urn:p>,<urn:o>) AS ?x) WHERE {}",
            BOUNDS
        )
        .unwrap_err(),
        Error::Excluded("TRIPLE".into())
    );
    for query in [
        "SELECT ?x WHERE {VALUES ?x {\"abc\"@en--ltr}}",
        "ASK {FILTER(\"abc\"@en--rtl)}",
        "SELECT * WHERE {?s <urn:p> \"abc\"@en--ltr}",
    ] {
        assert_eq!(
            lower(query, BOUNDS).unwrap_err(),
            Error::Unsupported("directional literal".into()),
            "{query}"
        );
    }
}

#[test]
fn closed_optional_and_subqueries_are_admitted() {
    for query in [
        "ASK {OPTIONAL {?s ?p ?o}}",
        "SELECT ?x WHERE {{SELECT ?x WHERE {VALUES ?x {1}}}}",
    ] {
        lower(query, BOUNDS).unwrap();
    }
}
