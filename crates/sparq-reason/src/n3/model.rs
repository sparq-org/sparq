//! N3 term model — richer than RDF (adds variables and formulae/graph-terms).

/// An N3 term. Beyond RDF's IRI/literal/blank it has `Var` (universally-quantified `?x`),
/// `Formula` (a `{ … }` graph term, used as rule premise/conclusion), and `List`
/// (a first-class `( … )` collection value — N3 lists are terms, not rdf:first/rest
/// triple structure; `rdf:nil` is the empty list).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Term {
    Iri(String),
    /// lexical value, datatype IRI, optional language tag.
    Lit(String, String, Option<String>),
    Blank(String),
    Var(String),
    Formula(Vec<[Term; 3]>),
    List(Vec<Term>),
    /// An RDF-star / RDF 1.2 quoted-triple TERM (`<< s p o >>` / `<<( s p o )>>`,
    /// GH #2012): a first-class term whose value IS the `(s, p, o)` triple.
    ///
    /// Unlike a [`Term::Formula`] (a graph VALUE whose inner variables are
    /// formula-scoped), a quoted triple is TRANSPARENT structure, exactly like
    /// [`Term::List`]: variables inside it are ordinary rule variables that
    /// match and bind against the corresponding component of a quoted triple
    /// in the data (SPARQL-star / RDF-star triple-pattern semantics).
    Triple(Box<[Term; 3]>),
}

impl Term {
    pub fn is_ground(&self) -> bool {
        match self {
            Term::Var(_) => false,
            // A quoted formula is a VALUE — ground when its contents are
            // (so rules may conclude formula-valued facts: log:conjunction,
            // log:conclusion, log:parsedAsN3 results).
            Term::Formula(ts) => ts.iter().all(|r| r.iter().all(Term::is_ground)),
            Term::List(ms) => ms.iter().all(Term::is_ground),
            // A quoted triple is ground when its three components are (its
            // inner variables are RULE variables, not term-scoped — see the
            // variant docs).
            Term::Triple(t) => t.iter().all(Term::is_ground),
            _ => true,
        }
    }
}

/// A `{ premise } => { conclusion }` rule (log:implies).
#[derive(Clone, Debug)]
pub struct Rule {
    pub premise: Vec<[Term; 3]>,
    pub conclusion: Vec<[Term; 3]>,
}
