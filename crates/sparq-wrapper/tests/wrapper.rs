// sq-1rg2q M1: behavior witnesses over real sparq-core mutation paths.

use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::{AccessError, Store, StoreError, TermKind};

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

#[test]
fn out_in_values_and_dataset_follow_the_real_graph() {
    let graph = Graph::load_str(
        "@prefix ex: <http://example.org/> . ex:alice ex:knows ex:bob . ex:bob ex:name \"Bob\" .",
        "turtle",
    )
    .unwrap();
    let store = Store::borrowed(&graph);

    let alice = store.node(iri("alice"));
    let bob = alice.out(&iri("knows")).next().unwrap();
    assert_eq!(bob.focus(), &Term::from(iri("bob")));
    assert_eq!(
        bob.out(&iri("name")).values().collect::<Vec<_>>(),
        vec![Term::from(Literal::new_simple_literal("Bob"))]
    );
    assert_eq!(
        bob.r#in(&iri("knows")).next().unwrap().focus(),
        &Term::from(iri("alice"))
    );
    assert_eq!(bob.dataset().graph().len(), 2);
}

#[test]
fn typed_accessors_check_datatype_and_lexical_form() {
    let store = Store::new();
    let string = store.node(Literal::new_simple_literal("hello"));
    assert_eq!(string.as_str().unwrap(), "hello");

    let integer = store.node(Literal::new_typed_literal("-42", xsd::INTEGER));
    assert_eq!(integer.as_i64().unwrap(), -42);
    let boolean = store.node(Literal::new_typed_literal("1", xsd::BOOLEAN));
    assert!(boolean.as_bool().unwrap());

    let typed = integer.as_typed_literal().unwrap();
    assert_eq!(typed.lexical_form, "-42");
    assert_eq!(typed.datatype, xsd::INTEGER);
    assert_eq!(typed.language, None);

    assert_eq!(
        store.node(iri("alice")).as_bool(),
        Err(AccessError::ExpectedLiteral {
            found: TermKind::NamedNode
        })
    );
    assert!(matches!(
        store
            .node(Literal::new_typed_literal("maybe", xsd::BOOLEAN))
            .as_bool(),
        Err(AccessError::InvalidLexical { .. })
    ));
    assert!(matches!(
        store
            .node(Literal::new_typed_literal("-1", xsd::UNSIGNED_INT))
            .as_i64(),
        Err(AccessError::InvalidLexical { .. })
    ));
}

#[test]
fn bounded_integer_accessors_accept_boundaries_and_reject_one_past() {
    let store = Store::new();
    let signed = [
        (xsd::LONG, i64::MIN, i64::MAX),
        (xsd::INT, i64::from(i32::MIN), i64::from(i32::MAX)),
        (xsd::SHORT, i64::from(i16::MIN), i64::from(i16::MAX)),
        (xsd::BYTE, i64::from(i8::MIN), i64::from(i8::MAX)),
    ];
    for (datatype, min, max) in signed {
        assert_eq!(
            store
                .node(Literal::new_typed_literal(min.to_string(), datatype))
                .as_i64(),
            Ok(min)
        );
        assert_eq!(
            store
                .node(Literal::new_typed_literal(max.to_string(), datatype))
                .as_i64(),
            Ok(max)
        );
        assert!(matches!(
            store
                .node(Literal::new_typed_literal(
                    (i128::from(min) - 1).to_string(),
                    datatype
                ))
                .as_i64(),
            Err(AccessError::InvalidLexical { .. })
        ));
        assert!(matches!(
            store
                .node(Literal::new_typed_literal(
                    (i128::from(max) + 1).to_string(),
                    datatype
                ))
                .as_i64(),
            Err(AccessError::InvalidLexical { .. })
        ));
    }

    let unsigned = [
        (xsd::UNSIGNED_INT, i64::from(u32::MAX)),
        (xsd::UNSIGNED_SHORT, i64::from(u16::MAX)),
        (xsd::UNSIGNED_BYTE, i64::from(u8::MAX)),
    ];
    for (datatype, max) in unsigned {
        assert_eq!(
            store
                .node(Literal::new_typed_literal("0", datatype))
                .as_i64(),
            Ok(0)
        );
        assert_eq!(
            store
                .node(Literal::new_typed_literal(max.to_string(), datatype))
                .as_i64(),
            Ok(max)
        );
        assert!(matches!(
            store
                .node(Literal::new_typed_literal("-1", datatype))
                .as_i64(),
            Err(AccessError::InvalidLexical { .. })
        ));
        assert!(matches!(
            store
                .node(Literal::new_typed_literal(
                    (i128::from(max) + 1).to_string(),
                    datatype
                ))
                .as_i64(),
            Err(AccessError::InvalidLexical { .. })
        ));
    }

    assert_eq!(
        store
            .node(Literal::new_typed_literal("0", xsd::UNSIGNED_LONG))
            .as_i64(),
        Ok(0)
    );
    assert!(matches!(
        store
            .node(Literal::new_typed_literal(
                u64::MAX.to_string(),
                xsd::UNSIGNED_LONG
            ))
            .as_i64(),
        Err(AccessError::InvalidLexical { .. })
    ));
    assert!(matches!(
        store
            .node(Literal::new_typed_literal(
                (i128::from(u64::MAX) + 1).to_string(),
                xsd::UNSIGNED_LONG
            ))
            .as_i64(),
        Err(AccessError::InvalidLexical { .. })
    ));
}

#[test]
fn owned_store_mutation_changes_reacquired_traversal() {
    let mut store = Store::new();
    let alice = iri("alice");
    let knows = iri("knows");
    let bob = iri("bob");

    assert_eq!(store.node(alice.clone()).out(&knows).count(), 0);
    store
        .insert(alice.clone(), knows.clone(), bob.clone())
        .unwrap();
    assert_eq!(
        store
            .node(alice.clone())
            .out(&knows)
            .values()
            .collect::<Vec<_>>(),
        vec![Term::from(bob.clone())]
    );
    store.remove(alice.clone(), knows.clone(), bob).unwrap();
    assert_eq!(store.node(alice).out(&knows).count(), 0);
}

#[test]
fn borrowed_store_rejects_mutation_and_literal_subjects() {
    let graph = Graph::new();
    let mut borrowed = Store::borrowed(&graph);
    assert_eq!(
        borrowed.insert(iri("s"), iri("p"), iri("o")),
        Err(StoreError::Borrowed)
    );

    let mut owned = Store::new();
    assert_eq!(
        owned.insert(
            Literal::new_simple_literal("not a subject"),
            iri("p"),
            iri("o")
        ),
        Err(StoreError::LiteralSubject)
    );
}

#[cfg(feature = "proposed-distinct")]
#[test]
fn proposed_dataset_helpers_are_distinct_by_term_value() {
    let graph = Graph::load_str(
        "@prefix ex: <http://example.org/> . ex:s ex:p ex:o1, ex:o2 . ex:t ex:p ex:o2 .",
        "turtle",
    )
    .unwrap();
    let store = Store::borrowed(&graph);
    let dataset = store.dataset();
    assert_eq!(
        dataset.subjects_of(&iri("p")).values().collect::<Vec<_>>(),
        vec![Term::from(iri("s")), Term::from(iri("t"))]
    );
    assert_eq!(
        dataset.objects_of(&iri("p")).values().collect::<Vec<_>>(),
        vec![Term::from(iri("o1")), Term::from(iri("o2"))]
    );
}

#[cfg(feature = "proposed-cardinality")]
#[test]
fn proposed_singular_accessors_report_mutation_witnessed_cardinality() {
    use sparq_wrapper::{Cardinality, CardinalityError};

    let mut store = Store::new();
    let alice = iri("alice");
    let name = iri("name");
    let first = Literal::new_simple_literal("Alice");
    let second = Literal::new_simple_literal("Alicia");

    assert_eq!(
        store.node(alice.clone()).required_out(&name).unwrap_err(),
        CardinalityError {
            focus: Term::from(alice.clone()),
            predicate: name.clone(),
            expected: Cardinality::ExactlyOne,
            found: 0,
        }
    );
    store
        .insert(alice.clone(), name.clone(), first.clone())
        .unwrap();
    assert_eq!(
        store
            .node(alice.clone())
            .required_out(&name)
            .unwrap()
            .as_str()
            .unwrap(),
        "Alice"
    );
    store.insert(alice.clone(), name.clone(), second).unwrap();
    let error = store.node(alice).optional_out(&name).unwrap_err();
    assert_eq!(error.expected, Cardinality::AtMostOne);
    assert_eq!(error.found, 2);
}
