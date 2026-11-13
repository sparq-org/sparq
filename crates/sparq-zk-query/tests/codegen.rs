use sparq_zk_query::{capacity::Bounds, codegen::generate, lower::lower};
const B: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};

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
    for op in [
        "open_dataset",
        "scan",
        "filter",
        "project",
        "distinct",
        "slice",
        "select",
        "request_binding",
    ] {
        assert!(
            source.contains(&format!("ops_core::{op}("))
                || source.contains(&format!("ops_core::{op}::<")),
            "{op}"
        );
    }
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
    assert!(source.contains("ops_core_expr::evaluate("));
    assert_eq!(p.files.len(), 3);
}
