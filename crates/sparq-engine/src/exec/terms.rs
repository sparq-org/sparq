use super::*;

// ---- spargebra term helpers --------------------------------------------------

pub(super) fn ground_to_term(g: &GroundTerm) -> Term {
    match g {
        GroundTerm::NamedNode(n) => Term::NamedNode(n.clone()),
        GroundTerm::Literal(l) => Term::Literal(l.clone()),
        // A ground RDF 1.2 triple term (e.g. in VALUES): fully concrete, so it maps
        // straight to a structural `Term::Triple` (the object may nest another).
        GroundTerm::Triple(t) => Term::Triple(Box::new(oxrdf::Triple::new(
            t.subject.clone(),
            t.predicate.clone(),
            ground_to_term(&t.object),
        ))),
    }
}

pub(super) fn term_pattern_to_term(tp: &TermPattern) -> Result<Term, String> {
    match tp {
        TermPattern::NamedNode(n) => Ok(Term::NamedNode(n.clone())),
        TermPattern::BlankNode(b) => Ok(Term::BlankNode(b.clone())),
        TermPattern::Literal(l) => Ok(Term::Literal(l.clone())),
        TermPattern::Variable(_) => Err("variable where a term was expected".into()),
        // GROUND triple term `<<( s p o )>>` (RDF 1.2): build the structural
        // `Term::Triple`, which the dictionary interns/looks up by its component ids.
        // A variable INSIDE a triple-term pattern (sq-kbs / T6) is handled UPSTREAM by
        // BGP decomposition (`extract_quoted_constraints` -> `quoted_relation`), which
        // rewrites any variable-carrying quoted slot into a synthetic variable before this
        // resolver runs. So in a parsed query this function only ever sees a GROUND triple
        // term — the variable-predicate arm below is a defensive backstop, not a feature gap.
        TermPattern::Triple(t) => {
            let subject: oxrdf::NamedOrBlankNode = match term_pattern_to_term(&t.subject)? {
                Term::NamedNode(n) => n.into(),
                Term::BlankNode(b) => b.into(),
                _ => return Err("RDF 1.2 triple-term subject must be an IRI or blank node".into()),
            };
            let predicate = match &t.predicate {
                NamedNodePattern::NamedNode(n) => n.clone(),
                // Unreachable for parsed queries (see above); kept as a non-panicking backstop.
                NamedNodePattern::Variable(_) => {
                    return Err("variable inside a quoted-triple term must be decomposed by the BGP planner (T6)".into())
                }
            };
            let object = term_pattern_to_term(&t.object)?;
            Ok(Term::Triple(Box::new(oxrdf::Triple::new(subject, predicate, object))))
        }
    }
}

/// Prefix for the synthetic variables that stand in for blank nodes (which are
/// existential variables in a query). `#` cannot appear in a SPARQL `VARNAME`,
/// so these can never collide with a user variable, and the `SELECT *` filter on
/// this prefix can never hide a real one.
pub(super) const BNODE_VAR_PREFIX: &str = "#bn#";

pub(super) fn bnode_var(b: &oxrdf::BlankNode) -> Variable {
    Variable::new_unchecked(format!("{BNODE_VAR_PREFIX}{}", b.as_str()))
}

pub(super) fn tp_var(tp: &TermPattern) -> Option<Variable> {
    match tp {
        TermPattern::Variable(v) => Some(v.clone()),
        TermPattern::BlankNode(b) => Some(bnode_var(b)),
        _ => None,
    }
}

pub(super) fn nnp_var(p: &NamedNodePattern) -> Option<Variable> {
    match p {
        NamedNodePattern::Variable(v) => Some(v.clone()),
        _ => None,
    }
}
