use super::*;

#[test]
fn ebv_error_is_none() {
    // SPARQL EBV: error → None (a type error in the expression).
    assert_eq!(ebv(&Value::Error, crate::EbvSemantics::Rec2013), None, "ebv(Error) must be None");
    assert!(!effective_boolean(&Value::Error, crate::EbvSemantics::Rec2013), "effective_boolean(Error) must be false (drop row)");
}

#[test]
fn ebv_unbound_is_none() {
    assert_eq!(ebv(&Value::Unbound, crate::EbvSemantics::Rec2013), None, "ebv(Unbound) must be None");
    assert!(!effective_boolean(&Value::Unbound, crate::EbvSemantics::Rec2013), "effective_boolean(Unbound) must be false (drop row)");
}

#[test]
fn ebv_bool_true_is_some_true() {
    assert_eq!(ebv(&Value::Bool(true), crate::EbvSemantics::Rec2013), Some(true));
    assert!(effective_boolean(&Value::Bool(true), crate::EbvSemantics::Rec2013));
}

#[test]
fn ebv_bool_false_is_some_false() {
    assert_eq!(ebv(&Value::Bool(false), crate::EbvSemantics::Rec2013), Some(false));
    assert!(!effective_boolean(&Value::Bool(false), crate::EbvSemantics::Rec2013));
}

#[test]
fn ebv_nonzero_int_is_true() {
    assert_eq!(ebv(&Value::Num(Num::Int(1)), crate::EbvSemantics::Rec2013), Some(true));
    assert_eq!(ebv(&Value::Num(Num::Int(-1)), crate::EbvSemantics::Rec2013), Some(true));
}

#[test]
fn ebv_zero_int_is_false() {
    assert_eq!(ebv(&Value::Num(Num::Int(0)), crate::EbvSemantics::Rec2013), Some(false));
}

#[test]
fn ebv_iri_term_is_none_type_error() {
    use oxrdf::NamedNode;
    let iri = Value::Term(Term::NamedNode(NamedNode::new_unchecked("http://ex/x")));
    assert_eq!(ebv(&iri, crate::EbvSemantics::Rec2013), None, "EBV of IRI is a type error → None");
    assert!(!effective_boolean(&iri, crate::EbvSemantics::Rec2013), "effective_boolean(IRI) = false (drop row)");
}

#[test]
fn ebv_nonempty_string_is_true() {
    let t = Value::Term(Term::Literal(Literal::new_simple_literal("hello")));
    assert_eq!(ebv(&t, crate::EbvSemantics::Rec2013), Some(true), "non-empty string literal EBV = true");
}

#[test]
fn ebv_empty_string_is_false() {
    let t = Value::Term(Term::Literal(Literal::new_simple_literal("")));
    assert_eq!(ebv(&t, crate::EbvSemantics::Rec2013), Some(false), "empty string literal EBV = false");
}

#[test]
fn ebv_lang_tagged_literal_follows_the_dialect() {
    // SPARQL 1.1 (REC 2013): plain literals, language-tagged ones included, use length.
    // The 1.2 draft: rdf:langString is NOT xsd:string, so its EBV is a type error.
    let t = Value::Term(Term::Literal(Literal::new_language_tagged_literal_unchecked("hello", "en")));
    let empty = Value::Term(Term::Literal(Literal::new_language_tagged_literal_unchecked("", "en")));
    assert_eq!(ebv(&t, crate::EbvSemantics::Rec2013), Some(true));
    assert_eq!(ebv(&empty, crate::EbvSemantics::Rec2013), Some(false));
    assert_eq!(ebv(&t, crate::EbvSemantics::Draft20260912), None, "lang-tagged literal EBV is type error → None");
}

#[test]
fn ebv_typed_boolean_true_false() {
    let tt = Value::Term(Term::Literal(Literal::new_typed_literal("true", xsd::BOOLEAN)));
    let ff = Value::Term(Term::Literal(Literal::new_typed_literal("false", xsd::BOOLEAN)));
    assert_eq!(ebv(&tt, crate::EbvSemantics::Rec2013), Some(true));
    assert_eq!(ebv(&ff, crate::EbvSemantics::Rec2013), Some(false));
}

#[test]
fn ebv_ill_formed_boolean_is_false_per_sparql_11() {
    let ill = Value::Term(Term::Literal(Literal::new_typed_literal("yes", xsd::BOOLEAN)));
    // REC §17.2.2 first bullet explicitly specifies false here.
    assert_eq!(ebv(&ill, crate::EbvSemantics::Rec2013), Some(false));
}

#[test]
fn ebv_xsd_string_nonempty_is_true() {
    let s = Value::Term(Term::Literal(Literal::new_typed_literal("abc", xsd::STRING)));
    assert_eq!(ebv(&s, crate::EbvSemantics::Rec2013), Some(true));
}

#[test]
fn ebv_xsd_string_empty_is_false() {
    let s = Value::Term(Term::Literal(Literal::new_typed_literal("", xsd::STRING)));
    assert_eq!(ebv(&s, crate::EbvSemantics::Rec2013), Some(false));
}
