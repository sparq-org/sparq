//! The BGP / triple-pattern model the planner plans over.
//!
//! A federated planner reasons about a Basic Graph Pattern (a conjunction of triple
//! patterns) before any source is contacted, so the pattern model here is a *light*,
//! self-contained one (an IRI/literal term or a named variable per position) rather
//! than the engine's full algebra term — this slice plans over descriptors, never over
//! a live `Graph`, and pulling in the engine's term type would couple the lean planner
//! to the execution stack. sq-a35t.

/// A query variable, identified by name (without the leading `?`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Var(pub String);

impl Var {
    pub fn new(name: impl Into<String>) -> Var {
        Var(name.into())
    }
}

/// One position of a triple pattern: a bound IRI/literal *term* (stored as its lexical
/// string — IRIs bare, literals already rendered) or an unbound *variable*.
///
/// Only the *shape* matters to the planner: whether a position is bound, and — for
/// bound IRIs — what authority/namespace it carries (so HiBISCuS pruning can compare it
/// against a source's capability set). Literals are bound terms with no authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// A bound IRI (stored bare, e.g. `http://dbpedia.org/resource/Berlin`).
    Iri(String),
    /// A bound literal (any non-IRI constant — its authority is irrelevant to pruning).
    Literal(String),
    /// An unbound variable.
    Var(Var),
}

impl Term {
    /// The variable at this position, if it is unbound.
    pub fn as_var(&self) -> Option<&Var> {
        match self {
            Term::Var(v) => Some(v),
            _ => None,
        }
    }

    /// The bound IRI at this position, if any (used for authority pruning).
    pub fn as_iri(&self) -> Option<&str> {
        match self {
            Term::Iri(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_bound(&self) -> bool {
        !matches!(self, Term::Var(_))
    }
}

/// A triple pattern `(subject, predicate, object)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriplePattern {
    pub subject: Term,
    pub predicate: Term,
    pub object: Term,
}

impl TriplePattern {
    pub fn new(subject: Term, predicate: Term, object: Term) -> TriplePattern {
        TriplePattern {
            subject,
            predicate,
            object,
        }
    }

    /// The bound predicate IRI of this pattern, if the predicate position is a bound
    /// IRI. CostFed/CS cardinality and HiBISCuS predicate pruning both key on this.
    pub fn predicate_iri(&self) -> Option<&str> {
        self.predicate.as_iri()
    }

    /// The variables this pattern *produces* (its unbound positions), de-duplicated in
    /// position order (subject, predicate, object). The join graph connects two
    /// patterns iff they share at least one such variable.
    pub fn vars(&self) -> Vec<&Var> {
        let mut out = Vec::new();
        for t in [&self.subject, &self.predicate, &self.object] {
            if let Some(v) = t.as_var() {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        out
    }
}

/// A Basic Graph Pattern: an ordered list of triple patterns. Order is the caller's
/// query order; the planner chooses its own join order over the selected sources.
#[derive(Debug, Clone, Default)]
pub struct Bgp {
    pub patterns: Vec<TriplePattern>,
}

impl Bgp {
    pub fn new(patterns: Vec<TriplePattern>) -> Bgp {
        Bgp { patterns }
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether patterns `i` and `j` share a variable (i.e. they join). Two patterns
    /// with no shared variable form a Cartesian product, which the planner avoids
    /// while any connected candidate remains.
    pub fn shares_var(&self, i: usize, j: usize) -> bool {
        let vi = self.patterns[i].vars();
        self.patterns[j].vars().iter().any(|v| vi.contains(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(Var::new(s))
    }

    #[test]
    fn vars_dedup_and_position_order() {
        // ?s :p ?s  — same var in subject and object: produced once.
        let tp = TriplePattern::new(var("s"), iri("http://ex/p"), var("s"));
        assert_eq!(tp.vars(), vec![&Var::new("s")]);
        let tp2 = TriplePattern::new(var("s"), var("p"), var("o"));
        assert_eq!(
            tp2.vars(),
            vec![&Var::new("s"), &Var::new("p"), &Var::new("o")]
        );
    }

    #[test]
    fn predicate_iri_only_when_bound() {
        let bound = TriplePattern::new(var("s"), iri("http://ex/p"), var("o"));
        assert_eq!(bound.predicate_iri(), Some("http://ex/p"));
        let unbound = TriplePattern::new(var("s"), var("p"), var("o"));
        assert_eq!(unbound.predicate_iri(), None);
    }

    #[test]
    fn shares_var_detects_join_edge() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
            TriplePattern::new(var("o"), iri("http://ex/q"), var("z")), // joins on ?o
            TriplePattern::new(var("a"), iri("http://ex/r"), var("b")), // disjoint
        ]);
        assert!(bgp.shares_var(0, 1));
        assert!(!bgp.shares_var(0, 2));
        assert!(!bgp.shares_var(1, 2));
    }
}
