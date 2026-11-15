mod closed_support;
use closed_support::*;
use sparq_zk_query::{interp::Cell, interp_closed_order::*, Error};

fn order_by(
    input: &sparq_zk_query::interp::Table,
    keys: &[OrderKey],
    capacity: usize,
) -> Result<sparq_zk_query::interp::Table, Error> {
    let width = input.rows.first().map_or(1, Vec::len);
    sparq_zk_query::interp_closed_order::order_by(input, width, keys, capacity)
}

fn key(column: usize, descending: bool) -> OrderKey {
    OrderKey { column, descending }
}

#[test]
fn order_preserves_multiplicity_and_breaks_ties_canonically() {
    let input = canonical(vec![
        vec![int(2), int(1)],
        vec![int(1), int(9)],
        vec![int(2), int(3)],
        vec![int(2), int(3)],
    ]);
    assert_eq!(
        order_by(&input, &[key(0, false), key(1, true)], 4)
            .unwrap()
            .rows,
        vec![
            vec![int(1), int(9)],
            vec![int(2), int(3)],
            vec![int(2), int(3)],
            vec![int(2), int(1)]
        ]
    );
    let a = order_by(&input, &[key(0, false)], 4).unwrap();
    assert_eq!(a.rows[0][0], int(1));
    assert_eq!(a.rows[1..], canonical(a.rows[1..].to_vec()).rows);
    assert_eq!(order_by(&input, &[], 4).unwrap(), input);
    assert!(matches!(order_by(&input, &[], 3), Err(Error::Capacity(_))));
}

#[test]
fn order_across_term_kinds_and_lexical_iris() {
    let mut b = iri("urn:b");
    b.kind = 2;
    b.scope = 1;
    let input = canonical(vec![
        vec![int(2)],
        vec![iri("urn:z")],
        vec![b],
        vec![Cell::default()],
        vec![iri("urn:a")],
    ]);
    let out = vec![
        vec![Cell::default()],
        vec![b],
        vec![iri("urn:a")],
        vec![iri("urn:z")],
        vec![int(2)],
    ];
    assert_eq!(order_by(&input, &[key(0, false)], 5).unwrap().rows, out);
    assert_eq!(
        order_by(&input, &[key(0, true)], 5).unwrap().rows,
        out.into_iter().rev().collect::<Vec<_>>()
    );
}

#[test]
fn undefined_literal_pairs_cannot_create_a_comparator_cycle() {
    let mut high = int(4);
    high.enc = [0; 32];
    high.enc[31] = 1;
    let mut unknown = int(2);
    unknown.key_class = 0;
    unknown.enc = [0; 32];
    unknown.enc[31] = 2;
    let mut low = int(1);
    low.enc = [0; 32];
    low.enc[31] = 3;
    let input = canonical(vec![vec![high], vec![unknown], vec![low]]);
    assert_eq!(
        order_by(&input, &[key(0, false)], 3).unwrap().rows,
        vec![vec![unknown], vec![low], vec![high]]
    );
}

#[test]
fn order_named_refusals_and_malformed_tables() {
    for (class, expected) in [
        (2, "double arithmetic pending"),
        (7, "lexical opening required"),
        (127, "comparison capacity"),
    ] {
        let mut c = int(1);
        c.key_class = class;
        let err = order_by(&canonical(vec![vec![c]]), &[key(0, false)], 1).unwrap_err();
        assert!(err.to_string().contains(expected));
    }
    let mut c = iri("urn:a");
    c.key_class = 0;
    assert!(order_by(&canonical(vec![vec![c]]), &[key(0, false)], 1)
        .unwrap_err()
        .to_string()
        .contains("IRI lexical key"));
    assert!(matches!(
        order_by(&canonical(vec![vec![int(1)]]), &[key(1, false)], 1),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn lexical_prefix_collisions_require_an_opening() {
    let a = iri("urn:a");
    let mut b = iri("urn:b");
    b.key = a.key;
    assert!(
        order_by(&canonical(vec![vec![a], vec![b]]), &[key(0, false)], 2)
            .unwrap_err()
            .to_string()
            .contains("lexical key collision")
    );
}

#[test]
fn mixed_timezone_comparisons_refuse_by_name() {
    let mut a = int(1);
    a.key_class = 3;
    let mut b = int(2);
    b.key_class = 4;
    assert!(
        order_by(&canonical(vec![vec![a], vec![b]]), &[key(0, false)], 2)
            .unwrap_err()
            .to_string()
            .contains("mixed timezone comparison")
    );
}

#[test]
fn differential_order_on_the_engine_input_table() {
    for seed in [0i64, 1, 7, 41, 893, 1201] {
        let mut rdf = String::new();
        for i in 0..8 {
            rdf.push_str(&format!("<urn:s{i}> <urn:n> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:s{i}> <urn:m> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", (seed + i * 7) % 11 - 5, i));
        }
        let pattern = "{?s <urn:n> ?x; <urn:m> ?y}";
        let input =
            canonical(engine(&rdf, &format!("SELECT ?x ?y WHERE {pattern}"), &["x", "y"]).rows);
        for (query_keys, keys) in [
            ("ASC(?x) ASC(?y)", vec![key(0, false), key(1, false)]),
            ("DESC(?x) DESC(?y)", vec![key(0, true), key(1, true)]),
            ("ASC(?x) DESC(?y)", vec![key(0, false), key(1, true)]),
        ] {
            let expected = engine(
                &rdf,
                &format!("SELECT ?x ?y WHERE {pattern} ORDER BY {query_keys}"),
                &["x", "y"],
            );
            assert_eq!(
                order_by(&input, &keys, 8).unwrap(),
                expected,
                "seed {seed}: {query_keys}"
            );
        }
    }
    let pattern = "{ VALUES ?x { <urn:z> <urn:a> <urn:m> } }";
    let input = canonical(engine("", &format!("SELECT ?x WHERE {pattern}"), &["x"]).rows);
    let expected = engine(
        "",
        &format!("SELECT ?x WHERE {pattern} ORDER BY ?x"),
        &["x"],
    );
    assert_eq!(order_by(&input, &[key(0, false)], 3).unwrap(), expected);
}
