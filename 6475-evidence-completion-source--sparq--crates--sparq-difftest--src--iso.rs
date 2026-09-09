//! Cross-oracle **blank-node isomorphism**: canonical labelling for `CONSTRUCT`/`DESCRIBE` graph
//! results and for `SELECT` results that project blank nodes. Engine-independent.
//! [OPUS-5] sq-qcnn.7
//!
//! Blank-node **labels are engine-local and arbitrary** — one oracle's `_:b0` and another's
//! `_:c14n2` may denote the same node — so a result carrying a blank node cannot be compared by
//! label. What is well defined is whether a **bijection** between the two engines' blank nodes
//! exists under which the whole result matches: graph (and solution-table) *isomorphism*. That is
//! NP-hard in general and tractable in practice via canonical labelling, which is what
//! [RDFC-1.0](https://www.w3.org/TR/rdf-canon/) does.
//!
//! # Independence (the load-bearing constraint, restated for this module)
//!
//! The canonical labelling here is the third-party [`rdf_canon`] crate (the maintained zkp-ld
//! RDFC-1.0 implementation) driving [`oxrdf`] `0.2` terms purely as a data carrier. It is **not**
//! sparq's `sparq-canon`, and it is not any engine's own canonicaliser — reusing one side's
//! canonicaliser is the independence trap this crate exists to avoid (see the crate docs).
//! `oxrdf` here holds strings and decides nothing: which literals count as equal is settled
//! *before* the conversion, by this crate's own value-canonical literal rule — the same one that
//! backs [`crate::term::canonical_key`].
//!
//! Honest caveats, stated rather than papered over:
//!
//! * `rdf_canon` speaks `oxrdf 0.2`, whose term model has **no RDF-1.2 triple terms**, so a
//!   [`Term::Triple`] anywhere in a result is rejected with [`IsoError::TripleTerm`] rather than
//!   silently dropped or flattened. A caller must route that to triage, not to "equal".
//! * RDFC-1.0 has known worst-case blow-ups on adversarial ("poison") graphs. The HNDQ call limit
//!   is therefore set explicitly ([`HNDQ_CALL_LIMIT`]) and exceeding it returns
//!   [`IsoError::TooComplex`] for the caller to count — never a hang, and never a silent "equal".
//! * Isomorphism is an existence claim about a bijection; it is **not** a claim that the two
//!   engines chose the same labels, and this module makes no soundness claim beyond that.
//!
//! # The two result shapes
//!
//! * **Graph results** (`CONSTRUCT`/`DESCRIBE`) are a *set* of triples: [`canonical_graph`] emits
//!   the RDFC-1.0 canonical N-Triples serialisation and [`graph_isomorphic`] compares two of them.
//! * **`SELECT` results projecting blank nodes** are harder, because blank-node scope is the whole
//!   result: the table must be canonicalised **as one structure**, not row by row, or two rows that
//!   share a node would compare equal to two rows that do not. [`canonical_solutions`] therefore
//!   reifies the table into a graph — one fresh row node per row, carrying a marker triple plus one
//!   triple per bound variable — and canonicalises that. Row nodes are distinguishable from value
//!   blank nodes by the marker triple (only row nodes carry it), and each row gets its own node, so
//!   **duplicate rows are preserved** (SPARQL bags, not sets) and the value-blank-node bijection is
//!   forced to be consistent across rows.
//!
//! Non-blank terms are encoded value-canonically (by the same internal `canonical_lexical` rule
//! that backs [`crate::term::canonical_key`]), matching [`crate::multiset::multiset_equal`]'s
//! regime, so cross-engine canonical-*lexical* variance does not read as a divergence here either.

use std::collections::HashMap;

use oxrdf::{BlankNode, GraphName, Literal, NamedNode, Quad, Subject, Term as OxTerm};
use sha2::Sha256;

use crate::json::Solution;
use crate::term::{canonical_lexical, effective_datatype, Term};

/// The RDFC-1.0 Hash-N-Degree-Quads call limit. RDFC-1.0 is exponential on adversarial graphs, and
/// a differential harness must degrade to a counted triage outcome rather than hang; a result that
/// needs more than this many HNDQ calls is reported as [`IsoError::TooComplex`].
pub const HNDQ_CALL_LIMIT: usize = 100_000;

/// The reification predicate marking a row node of a `SELECT` result table.
const P_ROW: &str = "urn:x-sparq-difftest:row";
/// The object of the row-marker triple.
const O_SOLUTION: &str = "urn:x-sparq-difftest:Solution";
/// Prefix of the per-variable reification predicate; the variable name follows, hex-encoded so any
/// variable name yields a syntactically valid IRI and two distinct names can never collide.
const P_VAR_PREFIX: &str = "urn:x-sparq-difftest:var:";

/// Why a canonical labelling could not be produced. Every variant is a case the caller must route
/// to **counted triage** — none of them may be read as "the two results agree".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsoError {
    /// An RDF-1.2 triple term appeared in the result. The RDFC-1.0 implementation used here speaks
    /// `oxrdf 0.2`, which has no triple-term model, so this is refused rather than approximated.
    TripleTerm,
    /// A literal appeared in subject position, or a non-IRI in predicate position (generalised RDF,
    /// which RDFC-1.0 is not defined over). Carries the position for the triage message.
    NotRdf(String),
    /// A term carried a syntactically invalid IRI or language tag — invalid oracle output, which is
    /// reported rather than repaired.
    Invalid(String),
    /// RDFC-1.0 exceeded [`HNDQ_CALL_LIMIT`], or otherwise declined the input.
    TooComplex(String),
}

impl std::fmt::Display for IsoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TripleTerm => write!(
                f,
                "blank-node canonicalisation: RDF-1.2 triple terms are not supported by the \
                 RDFC-1.0 implementation used here"
            ),
            Self::NotRdf(what) => write!(f, "blank-node canonicalisation: not RDF ({})", what),
            Self::Invalid(what) => write!(f, "blank-node canonicalisation: invalid term ({})", what),
            Self::TooComplex(why) => write!(f, "blank-node canonicalisation declined: {}", why),
        }
    }
}

impl std::error::Error for IsoError {}

/// Does this term contain a blank node (including one nested inside a triple term)?
pub fn term_has_blank_node(term: &Term) -> bool {
    match term {
        Term::Blank(_) => true,
        Term::Triple(t) => t.iter().any(term_has_blank_node),
        Term::Iri(_) | Term::Literal { .. } => false,
    }
}

/// Does any solution in this `SELECT` result bind a variable to a term containing a blank node?
/// This is the routing predicate a harness calls to decide between plain multiset equality
/// ([`crate::multiset::multiset_equal`]) and [`solutions_isomorphic`].
pub fn solutions_have_blank_nodes(solutions: &[Solution]) -> bool {
    solutions
        .iter()
        .any(|sol| sol.values().any(term_has_blank_node))
}

/// The RDFC-1.0 canonical N-Triples serialisation of a graph result (`CONSTRUCT`/`DESCRIBE`).
///
/// Blank nodes are relabelled to the canonical `_:c14nN` identifiers, so two engines that answered
/// the same graph with different blank-node labels produce the **same string**. A graph is a set:
/// duplicate triples in `triples` collapse, as RDF requires.
pub fn canonical_graph(triples: &[[Term; 3]]) -> Result<String, IsoError> {
    let mut labels = Relabeller::default();
    let mut quads = Vec::with_capacity(triples.len());
    for t in triples {
        quads.push(Quad::new(
            to_subject(&t[0], &mut labels)?,
            to_predicate(&t[1])?,
            to_object(&t[2], &mut labels)?,
            GraphName::DefaultGraph,
        ));
    }
    canonicalize(&quads)
}

/// Are two graph results **isomorphic** — equal up to a bijection of their blank nodes?
pub fn graph_isomorphic(a: &[[Term; 3]], b: &[[Term; 3]]) -> Result<bool, IsoError> {
    Ok(canonical_graph(a)? == canonical_graph(b)?)
}

/// The RDFC-1.0 canonical form of a `SELECT` result table, computed over the **whole table** so
/// blank-node identity shared between rows is part of what is canonicalised.
///
/// The table is reified: row `i` becomes a fresh blank node carrying `<urn:x-sparq-difftest:row>
/// <urn:x-sparq-difftest:Solution>` plus one triple per bound variable. One node per row keeps
/// duplicate rows distinct (SPARQL solution sequences are bags), an unbound variable simply emits
/// no triple, and the marker triple is what stops a row node from being confused with a projected
/// blank node.
pub fn canonical_solutions(solutions: &[Solution]) -> Result<String, IsoError> {
    let mut labels = Relabeller::default();
    let p_row = NamedNode::new_unchecked(P_ROW);
    let o_solution = NamedNode::new_unchecked(O_SOLUTION);
    let mut quads = Vec::new();
    for (i, sol) in solutions.iter().enumerate() {
        // `r`-prefixed row labels cannot collide with the `v`-prefixed value labels the
        // relabeller issues, so a projected blank node can never be mistaken for a row node.
        let row = BlankNode::new_unchecked(format!("r{}", i));
        quads.push(Quad::new(
            row.clone(),
            p_row.clone(),
            o_solution.clone(),
            GraphName::DefaultGraph,
        ));
        for (var, term) in sol {
            quads.push(Quad::new(
                row.clone(),
                NamedNode::new_unchecked(format!("{}{}", P_VAR_PREFIX, hex(var))),
                to_object(term, &mut labels)?,
                GraphName::DefaultGraph,
            ));
        }
    }
    canonicalize(&quads)
}

/// Are two `SELECT` result tables equal up to a **consistent bijection of their blank nodes**?
///
/// This is the blank-node-aware counterpart of [`crate::multiset::multiset_equal`]: row order does
/// not matter, row multiplicity does, and a blank node shared between two rows on one side must be
/// shared between the corresponding two rows on the other. On a table with no blank nodes it
/// decides exactly what `multiset_equal` decides.
pub fn solutions_isomorphic(a: &[Solution], b: &[Solution]) -> Result<bool, IsoError> {
    if a.len() != b.len() {
        return Ok(false);
    }
    Ok(canonical_solutions(a)? == canonical_solutions(b)?)
}

/// Issues a fresh, syntactically valid, per-side blank-node label for each distinct input label.
/// RDFC-1.0 relabels everything anyway, so the only properties required are injectivity and
/// validity — which lets an oracle emit any label at all (including one that is not a legal
/// N-Triples blank-node id) without breaking the comparison.
#[derive(Default)]
struct Relabeller {
    seen: HashMap<String, usize>,
}

impl Relabeller {
    fn issue(&mut self, label: &str) -> BlankNode {
        let next = self.seen.len();
        let n = *self.seen.entry(label.to_string()).or_insert(next);
        BlankNode::new_unchecked(format!("v{}", n))
    }
}

fn to_subject(term: &Term, labels: &mut Relabeller) -> Result<Subject, IsoError> {
    match term {
        Term::Iri(iri) => Ok(named_node(iri)?.into()),
        Term::Blank(label) => Ok(labels.issue(label).into()),
        Term::Literal { .. } => Err(IsoError::NotRdf("literal in subject position".into())),
        Term::Triple(_) => Err(IsoError::TripleTerm),
    }
}

fn to_predicate(term: &Term) -> Result<NamedNode, IsoError> {
    match term {
        Term::Iri(iri) => named_node(iri),
        Term::Triple(_) => Err(IsoError::TripleTerm),
        _ => Err(IsoError::NotRdf("non-IRI in predicate position".into())),
    }
}

fn to_object(term: &Term, labels: &mut Relabeller) -> Result<OxTerm, IsoError> {
    match term {
        Term::Iri(iri) => Ok(named_node(iri)?.into()),
        Term::Blank(label) => Ok(labels.issue(label).into()),
        Term::Triple(_) => Err(IsoError::TripleTerm),
        Term::Literal {
            lexical,
            datatype,
            lang,
        } => {
            if let Some(tag) = lang {
                // The language tag is lowercased here for the same reason `canonical_key`
                // lowercases it: RDF 1.1 compares tags case-insensitively.
                return Literal::new_language_tagged_literal(lexical, tag.to_ascii_lowercase())
                    .map(Into::into)
                    .map_err(|e| IsoError::Invalid(format!("language tag {}: {}", tag, e)));
            }
            let dt = effective_datatype(datatype, lang);
            Ok(Literal::new_typed_literal(canonical_lexical(lexical, dt), named_node(dt)?).into())
        }
    }
}

fn named_node(iri: &str) -> Result<NamedNode, IsoError> {
    NamedNode::new(iri).map_err(|e| IsoError::Invalid(format!("IRI {}: {}", iri, e)))
}

/// Lowercase hex of a variable name, so every variable maps to a distinct, syntactically valid IRI
/// suffix regardless of the characters it contains.
fn hex(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn canonicalize(quads: &[Quad]) -> Result<String, IsoError> {
    let options = rdf_canon::CanonicalizationOptions {
        hndq_call_limit: Some(HNDQ_CALL_LIMIT),
    };
    rdf_canon::canonicalize_quads_with::<Sha256>(quads, &options)
        .map_err(|e| IsoError::TooComplex(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::XSD_STRING;

    const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

    fn iri(s: &str) -> Term {
        Term::Iri(format!("http://ex/{}", s))
    }
    fn bnode(s: &str) -> Term {
        Term::Blank(s.to_string())
    }
    fn lit(lexical: &str, datatype: &str) -> Term {
        Term::Literal {
            lexical: lexical.to_string(),
            datatype: datatype.to_string(),
            lang: None,
        }
    }
    fn sol(pairs: &[(&str, Term)]) -> Solution {
        pairs.iter().map(|(k, t)| (k.to_string(), t.clone())).collect()
    }

    #[test]
    fn term_has_blank_node_sees_through_triple_terms() {
        assert!(term_has_blank_node(&bnode("b0")));
        assert!(!term_has_blank_node(&iri("s")));
        assert!(!term_has_blank_node(&lit("1", XSD_INT)));
        // nested inside a triple term (the case a shallow check would miss).
        assert!(term_has_blank_node(&Term::Triple(Box::new([
            iri("s"),
            iri("p"),
            bnode("b0"),
        ]))));
        assert!(!term_has_blank_node(&Term::Triple(Box::new([
            iri("s"),
            iri("p"),
            lit("1", XSD_INT),
        ]))));
    }

    #[test]
    fn solutions_have_blank_nodes_is_the_routing_predicate() {
        assert!(!solutions_have_blank_nodes(&[]));
        assert!(!solutions_have_blank_nodes(&[sol(&[("x", iri("n0"))])]));
        assert!(solutions_have_blank_nodes(&[
            sol(&[("x", iri("n0"))]),
            sol(&[("x", bnode("b0"))]),
        ]));
    }

    #[test]
    fn canonical_graph_relabels_blank_nodes_and_is_n_triples() {
        // Same graph, different engine-local labels → identical canonical form.
        let a = [
            [bnode("b0"), iri("p"), bnode("b1")],
            [bnode("b1"), iri("q"), iri("o")],
        ];
        let b = [
            [bnode("zz"), iri("q"), iri("o")],
            [bnode("_ILLEGAL LABEL"), iri("p"), bnode("zz")],
        ];
        let ca = canonical_graph(&a).unwrap();
        assert_eq!(ca, canonical_graph(&b).unwrap());
        // It really is the canonical labelling, not the input labels.
        assert!(ca.contains("_:c14n"), "expected canonical labels, got:\n{}", ca);
        assert!(!ca.contains("_:b0"));
        // N-Triples, not N-Quads: every line is exactly `s p o .` with no graph component.
        assert_eq!(ca.lines().count(), 2, "{}", ca);
        for line in ca.lines() {
            assert_eq!(
                line.split(' ').count(),
                4,
                "expected a 3-term N-Triples line, got: {}",
                line
            );
            assert!(line.ends_with(" ."), "{}", line);
        }
        // A graph is a SET: a duplicated triple does not change the canonical form.
        let dup = [
            [bnode("b0"), iri("p"), bnode("b1")],
            [bnode("b1"), iri("q"), iri("o")],
            [bnode("b0"), iri("p"), bnode("b1")],
        ];
        assert_eq!(ca, canonical_graph(&dup).unwrap());
    }

    #[test]
    fn graph_isomorphic_separates_genuinely_different_shapes() {
        let a = [
            [bnode("b0"), iri("p"), bnode("b1")],
            [bnode("b1"), iri("q"), iri("o")],
        ];
        // Same triples up to relabelling → isomorphic.
        let relabelled = [
            [bnode("x"), iri("p"), bnode("y")],
            [bnode("y"), iri("q"), iri("o")],
        ];
        assert!(graph_isomorphic(&a, &relabelled).unwrap());
        // The `q` edge now hangs off the OTHER node — no bijection can fix that.
        let rewired = [
            [bnode("x"), iri("p"), bnode("y")],
            [bnode("x"), iri("q"), iri("o")],
        ];
        assert!(!graph_isomorphic(&a, &rewired).unwrap());
        // Value-canonical literals: lexical variance of the same value is not a divergence,
        // a different value is.
        let l1 = [[iri("s"), iri("p"), lit("01", XSD_INT)]];
        let l2 = [[iri("s"), iri("p"), lit("1", XSD_INT)]];
        let l3 = [[iri("s"), iri("p"), lit("2", XSD_INT)]];
        assert!(graph_isomorphic(&l1, &l2).unwrap());
        assert!(!graph_isomorphic(&l1, &l3).unwrap());
    }

    #[test]
    fn canonical_graph_refuses_rather_than_approximates() {
        let triple_term = Term::Triple(Box::new([iri("s"), iri("p"), iri("o")]));
        assert_eq!(
            canonical_graph(&[[triple_term.clone(), iri("p"), iri("o")]]),
            Err(IsoError::TripleTerm)
        );
        assert_eq!(
            canonical_graph(&[[iri("s"), iri("p"), triple_term]]),
            Err(IsoError::TripleTerm)
        );
        assert!(matches!(
            canonical_graph(&[[lit("x", XSD_STRING), iri("p"), iri("o")]]),
            Err(IsoError::NotRdf(_))
        ));
        assert!(matches!(
            canonical_graph(&[[iri("s"), lit("p", XSD_STRING), iri("o")]]),
            Err(IsoError::NotRdf(_))
        ));
        assert!(matches!(
            canonical_graph(&[[Term::Iri("not an iri".into()), iri("p"), iri("o")]]),
            Err(IsoError::Invalid(_))
        ));
        // The error is a real std::error::Error with a message naming the problem.
        assert!(IsoError::TripleTerm.to_string().contains("triple term"));
    }

    #[test]
    fn canonical_solutions_keys_the_whole_table_not_each_row() {
        // One shared blank node across two rows vs two distinct ones: the canonical form of the
        // TABLE must distinguish them, even though row-by-row the two tables look alike.
        let shared = [sol(&[("x", bnode("a"))]), sol(&[("x", bnode("a"))])];
        let distinct = [sol(&[("x", bnode("a"))]), sol(&[("x", bnode("c"))])];
        assert_ne!(
            canonical_solutions(&shared).unwrap(),
            canonical_solutions(&distinct).unwrap()
        );
        // Row ORDER is not part of the canonical form (a solution sequence is a bag).
        let reordered = [sol(&[("x", bnode("c"))]), sol(&[("x", bnode("a"))])];
        assert_eq!(
            canonical_solutions(&distinct).unwrap(),
            canonical_solutions(&reordered).unwrap()
        );
        // An empty table is canonicalisable (and empty).
        assert_eq!(canonical_solutions(&[]).unwrap(), "");
    }

    #[test]
    fn solutions_isomorphic_accepts_relabelling_and_rejects_wrong_answers() {
        let a = [
            sol(&[("s", bnode("b0")), ("o", lit("1", XSD_INT))]),
            sol(&[("s", bnode("b1")), ("o", lit("2", XSD_INT))]),
        ];
        // The other engine chose different labels for the same answer.
        let relabelled = [
            sol(&[("s", bnode("c14n7")), ("o", lit("2", XSD_INT))]),
            sol(&[("s", bnode("c14n3")), ("o", lit("01", XSD_INT))]),
        ];
        assert!(solutions_isomorphic(&a, &relabelled).unwrap());
        // A WRONG VALUE on the same shape is still caught — the whole point of the harness.
        let wrong_value = [
            sol(&[("s", bnode("c14n7")), ("o", lit("3", XSD_INT))]),
            sol(&[("s", bnode("c14n3")), ("o", lit("1", XSD_INT))]),
        ];
        assert!(!solutions_isomorphic(&a, &wrong_value).unwrap());
        // A different SHARING structure is caught: here both rows share one node.
        let over_shared = [
            sol(&[("s", bnode("z")), ("o", lit("1", XSD_INT))]),
            sol(&[("s", bnode("z")), ("o", lit("2", XSD_INT))]),
        ];
        assert!(!solutions_isomorphic(&a, &over_shared).unwrap());
        // Multiplicity is significant (bag, not set).
        let dup = [
            sol(&[("s", bnode("q")), ("o", lit("1", XSD_INT))]),
            sol(&[("s", bnode("q")), ("o", lit("1", XSD_INT))]),
        ];
        assert!(!solutions_isomorphic(&dup, &a).unwrap());
        // Different lengths short-circuit to false.
        assert!(!solutions_isomorphic(&a, &a[..1]).unwrap());
    }

    #[test]
    fn solutions_isomorphic_agrees_with_multiset_equality_on_ground_tables() {
        // With no blank nodes anywhere this must decide exactly what `multiset_equal` decides,
        // so a harness can route on `solutions_have_blank_nodes` without changing verdicts.
        let cases: [(Vec<Solution>, Vec<Solution>); 4] = [
            (
                vec![sol(&[("x", iri("n0"))]), sol(&[("x", iri("n1"))])],
                vec![sol(&[("x", iri("n1"))]), sol(&[("x", iri("n0"))])],
            ),
            (
                vec![sol(&[("x", iri("n0"))])],
                vec![sol(&[("x", iri("n1"))])],
            ),
            (
                vec![sol(&[("x", iri("n0"))]), sol(&[("x", iri("n0"))])],
                vec![sol(&[("x", iri("n0"))]), sol(&[("x", iri("n1"))])],
            ),
            (
                // bound-vs-unbound is a distinct solution under both comparators.
                vec![sol(&[("x", iri("n0")), ("y", lit("a", XSD_STRING))])],
                vec![sol(&[("x", iri("n0"))])],
            ),
        ];
        for (a, b) in &cases {
            assert_eq!(
                solutions_isomorphic(a, b).unwrap(),
                crate::multiset::multiset_equal(a, b),
                "iso and multiset_equal disagreed on {:?} vs {:?}",
                a,
                b
            );
        }
    }

    #[test]
    fn the_row_marker_triple_is_what_makes_all_unbound_rows_countable() {
        // A solution with no bound variable emits no variable triple at all, so WITHOUT the row
        // marker an empty table, one such row, and two such rows would every one canonicalise to
        // the same (empty) string — a `SELECT` whose projection is entirely unbound would then
        // compare equal at ANY cardinality. Delete the marker triple from `canonical_solutions`
        // and these assertions go red. (`solutions_isomorphic` is not used for the first pair:
        // it short-circuits on length, which would mask the collision this pins.)
        let none: Vec<Solution> = vec![];
        let one = vec![Solution::new()];
        let two = vec![Solution::new(), Solution::new()];
        assert_ne!(
            canonical_solutions(&none).unwrap(),
            canonical_solutions(&one).unwrap()
        );
        assert_ne!(
            canonical_solutions(&one).unwrap(),
            canonical_solutions(&two).unwrap()
        );
        // A binding whose VALUE mimics the marker object does not acquire a row's role.
        let mimic = [sol(&[("row", Term::Iri(O_SOLUTION.to_string()))])];
        assert_ne!(
            canonical_solutions(&mimic).unwrap(),
            canonical_solutions(&one).unwrap()
        );
        // One row with a blank node is NOT isomorphic to one row with an IRI.
        let with_bnode = [sol(&[("x", bnode("b"))])];
        let with_iri = [sol(&[("x", iri("b"))])];
        assert!(!solutions_isomorphic(&with_bnode, &with_iri).unwrap());
    }

    #[test]
    fn hex_variable_names_cannot_collide_or_produce_an_invalid_iri() {
        assert_eq!(hex("x"), "78");
        assert_ne!(hex("ab"), hex("a"));
        // A variable name with characters that are illegal in an IRI still canonicalises.
        let weird = [sol(&[("a b<>\"", iri("n0"))])];
        assert!(canonical_solutions(&weird).is_ok());
        // ...and stays distinct from a differently-named variable.
        let other = [sol(&[("a b<>", iri("n0"))])];
        assert_ne!(
            canonical_solutions(&weird).unwrap(),
            canonical_solutions(&other).unwrap()
        );
    }
}
