//! The `oxrdf` → `sparq-difftest` **neutral-term** bridge for the value-level differential
//! comparators. [OPUS-5] sq-qcnn.5 (epic `sq-qcnn`; DAG node B of
//! `research/differential-testing-value-level.md`).
//!
//! [`sparq_difftest`] is the ENGINE-INDEPENDENT comparator library (node A, bead `sq-qcnn.4`): it
//! depends on **no** sparq crate and owns every value decision the differential makes — exact
//! arbitrary-precision integer / decimal equality (`num-bigint` / `bigdecimal`), canonical
//! `xsd:double` handling incl. `INF` / `NaN`, UTC-normalised temporals, and RDFC-1.0 blank-node
//! canonical labelling. That independence is load-bearing: canonicalising **both** sides of a
//! differential through sparq's OWN numeric tower or term comparator would apply any bug there
//! identically to both sides, where it **cancels** — the differential would go blind exactly where
//! it must see.
//!
//! Both engines in the query fuzzer already MATERIALISE their answers as the same `oxrdf 0.3`
//! [`Term`] (sparq through `sparq_engine::query`, Oxigraph through its `QuerySolution`), so this
//! bridge is a purely STRUCTURAL re-shaping of that shared carrier into the neutral model: it moves
//! `(kind, lexical, datatype, language)` across and stops. It decides no equality — *which* terms
//! count as equal is settled downstream, by `sparq-difftest`. The single exception is
//! [`fold_derived_integer`], a HARNESS-forced datatype fold documented on that function.
//!
//! # Known blind spot, stated rather than papered over
//!
//! The neutral literal model carries lexical + datatype + language tag, and has **no field for the
//! RDF-1.2 base DIRECTION** (`oxrdf`'s `Literal::direction`). Two literals differing only in their
//! base direction therefore compare EQUAL through this bridge. The query fuzzer's generator emits
//! no directional literals, so nothing is being lost silently today — but a generator that grows
//! them needs the direction added to `sparq-difftest`'s term model first, or that difference will
//! not be seen.

use oxrdf::{NamedOrBlankNode, Term, Triple};
use sparq_difftest::term::XSD;
use sparq_difftest::{Solution, Term as NeutralTerm};

/// `xsd:integer` — the base of the derived-integer family [`fold_derived_integer`] collapses onto.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// The XSD datatypes whose value space is exactly `xsd:integer`'s (SPARQL 1.1 §17.1 / XSD 1.1
/// §3.4): the bounded `long`/`int`/`short`/`byte` chain and the signed / unsigned restrictions.
/// `xsd:integer` itself is deliberately absent — it is the fold TARGET, not a member.
fn is_integer_derived(local: &str) -> bool {
    matches!(
        local,
        "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "positiveInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    )
}

/// Collapse an `xsd:integer`-**derived** datatype IRI onto `xsd:integer`; every other datatype is
/// returned unchanged.
///
/// This is the one HARNESS-forced concession in the bridge, and it is forced by the ORACLE's
/// storage layer rather than by a softening of the comparison: Oxigraph's store re-canonicalises
/// numeric literals when it LOADS a graph, including re-typing every integer-derived datatype down
/// to its `xsd:integer` base, while sparq keeps the term exactly as written. So one and the same
/// stored triple comes back as `"116"^^xsd:integer` from Oxigraph and `"116"^^xsd:int` from sparq —
/// a single RDF value reported through a datatype one side no longer has. Without the fold, every
/// answer over the generator's deliberately mixed `ex:val` column would read as a divergence.
///
/// It is deliberately NARROW. `xsd:integer` / `xsd:decimal` / `xsd:double` / `xsd:float` stay
/// DISTINCT, so an `integer`-vs-`decimal`-vs-`double` disagreement on a computed value is still a
/// mismatch; and the VALUE is never touched — `sparq-difftest` compares it exactly, so the
/// generator's `>2^53` integers and 18-significant-digit decimals (which a shared `f64` would
/// wrongly equate) still separate.
pub fn fold_derived_integer(datatype: &str) -> &str {
    match datatype.strip_prefix(XSD) {
        Some(local) if is_integer_derived(local) => XSD_INTEGER,
        _ => datatype,
    }
}

/// Re-shape one `oxrdf` term into the neutral model. Structural only: no equality is decided here.
pub fn neutral_term(t: &Term) -> NeutralTerm {
    match t {
        Term::NamedNode(n) => NeutralTerm::Iri(n.as_str().to_string()),
        Term::BlankNode(b) => NeutralTerm::Blank(b.as_str().to_string()),
        Term::Literal(l) => NeutralTerm::Literal {
            lexical: l.value().to_string(),
            datatype: fold_derived_integer(l.datatype().as_str()).to_string(),
            lang: l.language().map(str::to_string),
        },
        // RDF-1.2 triple term. The neutral model carries it; `sparq-difftest`'s RDFC-1.0 path
        // fails CLOSED on one (`IsoError::TripleTerm`) rather than approximating it, which the
        // harness routes to counted triage.
        Term::Triple(inner) => NeutralTerm::Triple(Box::new(neutral_triple(inner))),
    }
}

/// Re-shape one `oxrdf` triple into the neutral model's `[subject, predicate, object]` array (the
/// form `sparq_difftest::iso`'s graph comparators take).
pub fn neutral_triple(t: &Triple) -> [NeutralTerm; 3] {
    [
        neutral_subject(&t.subject),
        NeutralTerm::Iri(t.predicate.as_str().to_string()),
        neutral_term(&t.object),
    ]
}

fn neutral_subject(s: &NamedOrBlankNode) -> NeutralTerm {
    match s {
        NamedOrBlankNode::NamedNode(n) => NeutralTerm::Iri(n.as_str().to_string()),
        NamedOrBlankNode::BlankNode(b) => NeutralTerm::Blank(b.as_str().to_string()),
    }
}

/// Build one neutral [`Solution`] from an engine's BOUND `(variable, term)` pairs.
///
/// The caller passes only the bound pairs: the neutral model represents a projected-but-UNBOUND
/// variable by its ABSENCE from the map, which is what makes bound-vs-unbound a distinct solution
/// (and a distinct `ORDER BY` sort key) rather than a collision with an empty string.
pub fn neutral_solution<'a>(bound: impl Iterator<Item = (&'a str, &'a Term)>) -> Solution {
    bound
        .map(|(var, term)| (var.to_string(), neutral_term(term)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};
    use sparq_difftest::canonical_key;

    fn xsd(local: &str) -> NamedNode {
        NamedNode::new(format!("{XSD}{local}")).unwrap()
    }
    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }
    fn typed(lex: &str, dt: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(lex, xsd(dt)))
    }

    /// The fold collapses exactly the integer-DERIVED family and nothing else — in particular the
    /// primitive numeric types stay distinct, so an `integer`-vs-`decimal`-vs-`double`
    /// disagreement on a computed value is still visible.
    #[test]
    fn fold_derived_integer_is_narrow() {
        for derived in [
            "int",
            "long",
            "short",
            "byte",
            "unsignedInt",
            "unsignedLong",
            "unsignedShort",
            "unsignedByte",
            "nonNegativeInteger",
            "positiveInteger",
            "nonPositiveInteger",
            "negativeInteger",
        ] {
            assert_eq!(
                fold_derived_integer(&format!("{XSD}{derived}")),
                XSD_INTEGER,
                "xsd:{derived} must fold onto xsd:integer"
            );
        }
        // The fold target itself, the other primitive numerics, and non-XSD datatypes: unchanged.
        for kept in ["integer", "decimal", "double", "float", "string", "dateTime", "boolean"] {
            let dt = format!("{XSD}{kept}");
            assert_eq!(fold_derived_integer(&dt), dt, "xsd:{kept} must NOT fold");
        }
        assert_eq!(
            fold_derived_integer("http://example.org/myInt"),
            "http://example.org/myInt"
        );
    }

    /// The bridge must absorb ONLY the lexical / derived-datatype normalisation Oxigraph's store
    /// performs on load — never a difference in VALUE, and never across primitive numeric types.
    /// The `>2^53` integers and 18-digit decimals the generator emits specifically to defeat an
    /// `f64` oracle must still separate.
    #[test]
    fn neutral_term_keys_absorb_lexical_form_but_not_value() {
        let key = |lex: &str, dt: &str| canonical_key(&neutral_term(&typed(lex, dt)));
        // lexical / derived-type normalisation → SAME key
        assert_eq!(key("-0", "integer"), key("0", "integer"));
        assert_eq!(key("116", "int"), key("116", "integer"));
        assert_eq!(key("07", "integer"), key("7", "integer"));
        assert_eq!(key("113.0", "decimal"), key("113", "decimal"));
        // value differences → DIFFERENT key, including beyond f64 resolution
        assert_ne!(
            key("9007199254740992", "integer"),
            key("9007199254740993", "integer")
        );
        assert_ne!(
            key("0.123456789012345678", "decimal"),
            key("0.123456789012345679", "decimal")
        );
        assert_ne!(key("5", "integer"), key("6", "integer"));
        // primitive numeric types stay distinct
        assert_ne!(key("5", "integer"), key("5", "decimal"));
        assert_ne!(key("5", "decimal"), key("5", "double"));
        // a numeric-looking STRING is not a number
        assert_ne!(key("5", "integer"), key("5", "string"));
    }

    /// Every term kind crosses structurally, and the kinds stay distinguishable from each other.
    #[test]
    fn neutral_term_carries_every_kind() {
        assert!(matches!(neutral_term(&iri("http://ex/n0")), NeutralTerm::Iri(s) if s == "http://ex/n0"));
        assert!(
            matches!(neutral_term(&Term::BlankNode(BlankNode::new("b0").unwrap())), NeutralTerm::Blank(s) if s == "b0")
        );
        // A language-tagged literal keeps its tag (which the neutral key lowercases), and is NOT
        // the same term as the plain string of the same lexical.
        let lang = Term::Literal(Literal::new_language_tagged_literal("nm0", "en").unwrap());
        assert!(matches!(
            neutral_term(&lang),
            NeutralTerm::Literal { lang: Some(t), .. } if t == "en"
        ));
        assert_ne!(
            canonical_key(&neutral_term(&lang)),
            canonical_key(&neutral_term(&Term::Literal(Literal::new_simple_literal("nm0"))))
        );
        // A simple literal folds to xsd:string in the neutral key (RDF 1.1), so it keys the same
        // as the explicitly typed form.
        assert_eq!(
            canonical_key(&neutral_term(&Term::Literal(Literal::new_simple_literal("s1")))),
            canonical_key(&neutral_term(&typed("s1", "string")))
        );
        // An IRI and a blank node are different KINDS of term, never interchangeable.
        assert_ne!(
            canonical_key(&neutral_term(&iri("http://ex/b0"))),
            canonical_key(&neutral_term(&Term::BlankNode(BlankNode::new("b0").unwrap())))
        );
    }

    /// A triple's three positions cross in order, and an RDF-1.2 triple term nests rather than
    /// being flattened or dropped.
    #[test]
    fn neutral_triple_preserves_positions_and_nesting() {
        let t = Triple::new(
            NamedNode::new("http://ex/s").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            typed("1", "int"),
        );
        let n = neutral_triple(&t);
        assert!(matches!(&n[0], NeutralTerm::Iri(s) if s == "http://ex/s"));
        assert!(matches!(&n[1], NeutralTerm::Iri(s) if s == "http://ex/p"));
        // The object's derived integer datatype folded, so it keys as xsd:integer.
        assert_eq!(canonical_key(&n[2]), canonical_key(&neutral_term(&typed("1", "integer"))));
        // A blank-node subject stays a blank node (not an IRI), and a nested triple term nests.
        let bnode_subject = Triple::new(
            BlankNode::new("b1").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Triple(Box::new(t.clone())),
        );
        let n = neutral_triple(&bnode_subject);
        assert!(matches!(&n[0], NeutralTerm::Blank(s) if s == "b1"));
        assert!(matches!(&n[2], NeutralTerm::Triple(_)));
    }

    /// A solution carries its bound pairs, and an unbound variable is represented by ABSENCE —
    /// which is what keeps bound-vs-unbound a distinct solution downstream.
    #[test]
    fn neutral_solution_represents_unbound_by_absence() {
        let a = iri("http://ex/n0");
        let sol = neutral_solution([("s", &a)].into_iter());
        assert_eq!(sol.len(), 1);
        assert!(sol.contains_key("s"));
        assert!(!sol.contains_key("n"), "an unbound variable must be ABSENT");
        // Two pairs land under their own variable names (the map is variable-keyed, order-free).
        let b = typed("5", "integer");
        let sol = neutral_solution([("s", &a), ("a", &b)].into_iter());
        assert_eq!(sol.len(), 2);
        assert_eq!(
            canonical_key(sol.get("a").unwrap()),
            canonical_key(&neutral_term(&b))
        );
    }
}
