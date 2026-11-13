//! Term observations for the shared SPARQL total-order algorithm.
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term};
use sparq_core::temporal::ExactTemporal;
use sparq_substrate::compare::{compare_terms, CompareTerm, LiteralKind, TermClass};
use sparq_substrate::numeric::{cmp_plain_decimal, f64_exact_decimal};
use std::cmp::Ordering;

#[derive(Clone, Copy)]
enum Node<'a> {
    Term(&'a Term),
    Subject(&'a NamedOrBlankNode),
    Predicate(&'a NamedNode),
}

struct Ordered<'a> {
    node: Node<'a>,
    numeric: Option<f64>,
    kind: LiteralKind,
}

impl<'a> Ordered<'a> {
    fn new(term: &'a Term) -> Self {
        let mut result = Self {
            node: Node::Term(term),
            numeric: None,
            kind: LiteralKind::Other,
        };
        if let Term::Literal(literal) = term {
            result.numeric =
                sparq_core::numeric_literal_valid(literal.value(), literal.datatype().as_str())
                    .then(|| {
                        sparq_core::numeric_lexical_f64(
                            literal.value(),
                            literal.datatype().as_str(),
                        )
                    })
                    .flatten();
            result.kind = if result.numeric.is_some() {
                LiteralKind::Numeric
            } else if boolean(literal).is_some() {
                LiteralKind::Boolean
            } else if ExactTemporal::of_lit(literal.value(), literal.datatype().as_str()).is_some()
            {
                if literal.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#date" {
                    LiteralKind::Date
                } else {
                    LiteralKind::DateTime
                }
            } else if literal.language().is_some() {
                LiteralKind::Lang
            } else if literal.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string" {
                LiteralKind::String
            } else {
                LiteralKind::Other
            };
        }
        result
    }

    fn literal(&self) -> Option<&'a Literal> {
        match self.node {
            Node::Term(Term::Literal(l)) => Some(l),
            _ => None,
        }
    }

    fn exact_lexical(&self) -> Option<&str> {
        let literal = self.literal()?;
        (self.numeric.is_some()
            && (sparq_core::is_integer_datatype(literal.datatype().as_str())
                || literal.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#decimal"))
            .then_some(literal.value())
    }
}

fn boolean(literal: &Literal) -> Option<bool> {
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#boolean" {
        return None;
    }
    match literal.value() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn exact_float(lexical: &str, float: f64) -> Option<Ordering> {
    if float == f64::INFINITY {
        return Some(Ordering::Less);
    }
    if float == f64::NEG_INFINITY {
        return Some(Ordering::Greater);
    }
    cmp_plain_decimal(lexical, &f64_exact_decimal(float)?)
}

impl CompareTerm for Ordered<'_> {
    fn term_class(&self) -> TermClass {
        match self.node {
            Node::Term(Term::BlankNode(_)) | Node::Subject(NamedOrBlankNode::BlankNode(_)) => {
                TermClass::Blank
            }
            Node::Term(Term::NamedNode(_))
            | Node::Subject(NamedOrBlankNode::NamedNode(_))
            | Node::Predicate(_) => TermClass::Iri,
            Node::Term(Term::Literal(_)) => TermClass::Literal,
            Node::Term(Term::Triple(_)) => TermClass::Triple,
        }
    }
    fn literal_kind(&self) -> LiteralKind {
        self.kind
    }
    fn value_str(&self) -> Option<String> {
        Some(match self.node {
            Node::Term(Term::BlankNode(b)) | Node::Subject(NamedOrBlankNode::BlankNode(b)) => {
                b.as_str().to_string()
            }
            Node::Term(Term::NamedNode(n))
            | Node::Subject(NamedOrBlankNode::NamedNode(n))
            | Node::Predicate(n) => n.as_str().to_string(),
            Node::Term(Term::Literal(l)) => l.value().to_string(),
            Node::Term(Term::Triple(_)) => return None,
        })
    }
    fn as_f64(&self) -> Option<f64> {
        self.numeric
    }
    fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self.exact_lexical(), other.exact_lexical()) {
            (Some(a), Some(b)) => cmp_plain_decimal(a, b),
            (Some(a), None) => exact_float(a, other.numeric?),
            (None, Some(b)) => exact_float(b, self.numeric?).map(Ordering::reverse),
            (None, None) => Some(Ordering::Equal),
        }
    }
    fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
        let (a, b) = (self.literal()?, other.literal()?);
        if let (Some(a), Some(b)) = (boolean(a), boolean(b)) {
            return Some(a.cmp(&b));
        }
        let a = ExactTemporal::of_lit(a.value(), a.datatype().as_str())?;
        let b = ExactTemporal::of_lit(b.value(), b.datatype().as_str())?;
        Some(a.compare_total(b))
    }
    fn triple_parts(&self) -> Option<[Self; 3]> {
        let Node::Term(Term::Triple(triple)) = self.node else {
            return None;
        };
        Some([
            Self {
                node: Node::Subject(&triple.subject),
                numeric: None,
                kind: LiteralKind::Other,
            },
            Self {
                node: Node::Predicate(&triple.predicate),
                numeric: None,
                kind: LiteralKind::Other,
            },
            Self::new(&triple.object),
        ])
    }
}

pub(super) fn compare(a: &Term, b: &Term) -> Ordering {
    compare_terms(&Ordered::new(a), &Ordered::new(b)).unwrap_or(Ordering::Equal)
}
