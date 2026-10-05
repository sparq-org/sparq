use super::*;

#[test]
fn ebv_error_is_none() {
    // SPARQL EBV: error → None (a type error in the expression).
    assert_eq!(ebv(&Value::Error), None, "ebv(Error) must be None");
    assert!(!effective_boolean(&Value::Error), "effective_boolean(Error) must be false (drop row)");
}

#[test]
fn ebv_unbound_is_none() {
    assert_eq!(ebv(&Value::Unbound), None, "ebv(Unbound) must be None");
    assert!(!effective_boolean(&Value::Unbound), "effective_boolean(Unbound) must be false (drop row)");
}

#[test]
fn ebv_bool_true_is_some_true() {
    assert_eq!(ebv(&Value::Bool(true)), Some(true));
    assert!(effective_boolean(&Value::Bool(true)));
}

#[test]
fn ebv_bool_false_is_some_false() {
    assert_eq!(ebv(&Value::Bool(false)), Some(false));
    assert!(!effective_boolean(&Value::Bool(false)));
}

#[test]
fn ebv_nonzero_int_is_true() {
    assert_eq!(ebv(&Value::Num(Num::Int(1))), Some(true));
    assert_eq!(ebv(&Value::Num(Num::Int(-1))), Some(true));
}

#[test]
fn ebv_zero_int_is_false() {
    assert_eq!(ebv(&Value::Num(Num::Int(0))), Some(false));
}

#[test]
fn ebv_iri_term_is_none_type_error() {
    use oxrdf::NamedNode;
    let iri = Value::Term(Term::NamedNode(NamedNode::new_unchecked("http://ex/x")));
    assert_eq!(ebv(&iri), None, "EBV of IRI is a type error → None");
    assert!(!effective_boolean(&iri), "effective_boolean(IRI) = false (drop row)");
}

#[test]
fn ebv_nonempty_string_is_true() {
    let t = Value::Term(Term::Literal(Literal::new_simple_literal("hello")));
    assert_eq!(ebv(&t), Some(true), "non-empty string literal EBV = true");
}

#[test]
fn ebv_empty_string_is_false() {
    let t = Value::Term(Term::Literal(Literal::new_simple_literal("")));
    assert_eq!(ebv(&t), Some(false), "empty string literal EBV = false");
}

#[test]
fn ebv_lang_tagged_literal_is_none_type_error() {
    // rdf:langString is NOT xsd:string: its EBV is a type error.
    let t = Value::Term(Term::Literal(Literal::new_language_tagged_literal_unchecked("hello", "en")));
    assert_eq!(ebv(&t), None, "lang-tagged literal EBV is type error → None");
}

#[test]
fn ebv_typed_boolean_true_false() {
    let tt = Value::Term(Term::Literal(Literal::new_typed_literal("true", xsd::BOOLEAN)));
    let ff = Value::Term(Term::Literal(Literal::new_typed_literal("false", xsd::BOOLEAN)));
    assert_eq!(ebv(&tt), Some(true));
    assert_eq!(ebv(&ff), Some(false));
}

#[test]
fn ebv_ill_formed_boolean_is_type_error() {
    let ill = Value::Term(Term::Literal(Literal::new_typed_literal("yes", xsd::BOOLEAN)));
    assert_eq!(ebv(&ill), None, "ill-formed boolean EBV is type error");
}

#[test]
fn ebv_xsd_string_nonempty_is_true() {
    let s = Value::Term(Term::Literal(Literal::new_typed_literal("abc", xsd::STRING)));
    assert_eq!(ebv(&s), Some(true));
}

#[test]
fn ebv_xsd_string_empty_is_false() {
    let s = Value::Term(Term::Literal(Literal::new_typed_literal("", xsd::STRING)));
    assert_eq!(ebv(&s), Some(false));
}
