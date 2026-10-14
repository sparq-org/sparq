//! Bounded missing-answer explanation for a single basic graph pattern.
//!
//! sq-lsp7k.17: [`why_not`] substitutes a caller-provided target
//! binding into every triple pattern and reports exactly the resulting triples
//! absent from the graph. More general SPARQL algebra is deliberately rejected.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Write as _};

use oxrdf::vocab::rdf;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple, Variable};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use sparq_core::Graph;

use crate::{prov, PROV, SPARQ_PROV_NS};

// sq-lsp7k: RDF vocabulary for deterministic missing-answer reports.
const ABSENT: &str = "urn:sparq:prov:absent";
const POSITION: &str = "urn:sparq:prov:position";
const TARGET_BINDING: &str = "urn:sparq:prov:targetBinding";

/// One failing conjunct in a missing-answer explanation.
///
/// The original [`TriplePattern`] is retained for attribution to the BGP and
/// [`grounded`](Self::grounded) is the concrete triple proved absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingPattern {
    pattern: TriplePattern,
    grounded: Triple,
}

impl MissingPattern {
    /// The original triple pattern, before target-binding substitution.
    pub fn pattern(&self) -> &TriplePattern {
        &self.pattern
    }

    /// The concrete triple absent from the graph.
    pub fn grounded(&self) -> &Triple {
        &self.grounded
    }
}

/// Why-not explanation or report construction failed closed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WhyNotError {
    /// The supplied algebra node is not a single basic graph pattern.
    UnsupportedAlgebra,
    /// A variable used by the BGP is absent from the target binding.
    MissingBinding(Variable),
    /// Substitution produced a term that cannot occupy an RDF triple's subject.
    InvalidSubject(Term),
    /// Substitution produced a term that is not an IRI in predicate position.
    InvalidPredicate(Term),
    /// A report was requested with a target other than the one that grounded it.
    TargetMismatch,
}

impl fmt::Display for WhyNotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAlgebra => f.write_str(
                "why-not explanation supports one basic graph pattern only; other SPARQL algebra is unsupported",
            ),
            Self::MissingBinding(variable) => {
                write!(f, "target binding does not bind {variable}")
            }
            Self::InvalidSubject(term) => {
                write!(f, "target binding produces an invalid triple subject: {term}")
            }
            Self::InvalidPredicate(term) => {
                write!(f, "target binding produces a non-IRI triple predicate: {term}")
            }
            Self::TargetMismatch => f.write_str(
                "target binding does not reproduce a missing pattern's grounded triple",
            ),
        }
    }
}

impl Error for WhyNotError {}

/// Explains why a fully-ground target binding is absent from one BGP's answers.
///
/// Each BGP triple pattern is grounded with `target`. The returned vector
/// contains a pattern if and only if its concrete triple is absent from `graph`,
/// preserving the BGP's order. An empty vector therefore means every conjunct
/// is present and the target would be an answer to this BGP.
///
/// This bounded API deliberately accepts only [`GraphPattern::Bgp`]. `OPTIONAL`,
/// `UNION`, `FILTER`, property paths, named graphs, and all other algebra return
/// [`WhyNotError::UnsupportedAlgebra`]. A missing variable binding or an RDF-term
/// position error also fails closed instead of inventing an explanation.
pub fn why_not(
    graph: &Graph,
    algebra: &GraphPattern,
    target: &HashMap<Variable, Term>,
) -> Result<Vec<MissingPattern>, WhyNotError> {
    let GraphPattern::Bgp { patterns } = algebra else {
        return Err(WhyNotError::UnsupportedAlgebra);
    };

    let mut missing = Vec::new();
    for pattern in patterns {
        let grounded = ground_triple(pattern, target)?;
        if !contains(graph, &grounded) {
            missing.push(MissingPattern {
                pattern: pattern.clone(),
                grounded,
            });
        }
    }
    Ok(missing)
}

/// Serialises a missing-answer report as deterministic RDF 1.2 N-Triples.
///
/// `missing` is the vector returned by [`why_not`] for `target`. Each entry
/// becomes one `prov:Entity` whose `rdf:reifies` is the exact grounded triple as
/// an RDF 1.2 triple term. The entity also carries
/// `urn:sparq:prov:absent true`, its zero-based BGP position, and a canonical
/// target-binding literal. Entries and their metadata retain `missing` order.
///
/// The target is applied to every retained original pattern again before any
/// RDF is emitted. Invalid bindings return the corresponding [`WhyNotError`],
/// and a target that does not reproduce [`MissingPattern::grounded`] returns
/// [`WhyNotError::TargetMismatch`]. Because `MissingPattern` has no public
/// constructor, report entries can originate only from the bounded, single-BGP
/// [`why_not`] surface.
///
/// sq-lsp7k
pub fn why_not_report_ntriples(
    target: &HashMap<Variable, Term>,
    missing: &[MissingPattern],
) -> Result<String, WhyNotError> {
    Ok(sparq_engine::triples_to_ntriples(&why_not_report_graph(
        target, missing,
    )?))
}

/// Serialises the same missing-answer report as prefix-compacted Turtle.
///
/// The RDF graph and validation rules are identical to
/// [`why_not_report_ntriples`]. Output is deterministic and retains the BGP
/// order preserved by [`why_not`].
///
/// sq-lsp7k
pub fn why_not_report_turtle(
    target: &HashMap<Variable, Term>,
    missing: &[MissingPattern],
) -> Result<String, WhyNotError> {
    let triples = why_not_report_graph(target, missing)?;
    let serializer = oxttl::TurtleSerializer::new()
        .with_prefix("prov", PROV)
        .expect("the PROV namespace must be a valid prefix IRI")
        .with_prefix("spqprov", SPARQ_PROV_NS)
        .expect("the sparq provenance namespace must be a valid prefix IRI")
        .with_prefix("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#")
        .expect("the RDF namespace must be a valid prefix IRI")
        .with_prefix("xsd", "http://www.w3.org/2001/XMLSchema#")
        .expect("the XSD namespace must be a valid prefix IRI");
    let mut writer = serializer.for_writer(Vec::new());
    for triple in &triples {
        writer
            .serialize_triple(triple)
            .expect("serialising a why-not report into memory must succeed");
    }
    let bytes = writer
        .finish()
        .expect("finishing why-not Turtle serialisation into memory must succeed");
    Ok(String::from_utf8(bytes).expect("why-not Turtle output must be valid UTF-8"))
}

fn why_not_report_graph(
    target: &HashMap<Variable, Term>,
    missing: &[MissingPattern],
) -> Result<Vec<Triple>, WhyNotError> {
    for entry in missing {
        if &ground_triple(entry.pattern(), target)? != entry.grounded() {
            return Err(WhyNotError::TargetMismatch);
        }
    }

    let target_binding = canonical_target(target);
    let mut report = Vec::with_capacity(missing.len() * 5);
    for (position, entry) in missing.iter().enumerate() {
        let node = report_node(&target_binding, position, entry.grounded());
        let subject = NamedOrBlankNode::NamedNode(node);
        report.push(Triple::new(
            subject.clone(),
            rdf::TYPE,
            Term::NamedNode(prov("Entity")),
        ));
        report.push(Triple::new(
            subject.clone(),
            rdf::REIFIES,
            Term::Triple(Box::new(entry.grounded().clone())),
        ));
        report.push(Triple::new(
            subject.clone(),
            NamedNode::new_unchecked(ABSENT),
            Term::Literal(Literal::from(true)),
        ));
        report.push(Triple::new(
            subject.clone(),
            NamedNode::new_unchecked(POSITION),
            Term::Literal(Literal::from(
                u64::try_from(position).expect("a report vector must fit in u64"),
            )),
        ));
        report.push(Triple::new(
            subject,
            NamedNode::new_unchecked(TARGET_BINDING),
            Term::Literal(Literal::new_simple_literal(target_binding.clone())),
        ));
    }
    Ok(report)
}

fn canonical_target(target: &HashMap<Variable, Term>) -> String {
    let mut bindings: Vec<_> = target.iter().collect();
    bindings.sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));

    let mut canonical = String::new();
    for (index, (variable, term)) in bindings.into_iter().enumerate() {
        if index != 0 {
            canonical.push_str("; ");
        }
        write!(&mut canonical, "{variable}={term}")
            .expect("writing a target binding into memory must succeed");
    }
    canonical
}

fn report_node(target_binding: &str, position: usize, grounded: &Triple) -> NamedNode {
    let identity = format!("{target_binding}\n{position}\n{grounded}");
    NamedNode::new_unchecked(format!(
        "{SPARQ_PROV_NS}missing:{position}:{:016x}",
        crate::fnv1a(identity.as_bytes())
    ))
}

fn ground_triple(
    pattern: &TriplePattern,
    target: &HashMap<Variable, Term>,
) -> Result<Triple, WhyNotError> {
    let subject_term = ground_term(&pattern.subject, target)?;
    let subject = match subject_term {
        Term::NamedNode(node) => NamedOrBlankNode::NamedNode(node),
        Term::BlankNode(node) => NamedOrBlankNode::BlankNode(node),
        invalid => return Err(WhyNotError::InvalidSubject(invalid)),
    };

    let predicate = match &pattern.predicate {
        NamedNodePattern::NamedNode(node) => node.clone(),
        NamedNodePattern::Variable(variable) => match binding(target, variable)? {
            Term::NamedNode(node) => node,
            invalid => return Err(WhyNotError::InvalidPredicate(invalid)),
        },
    };

    Ok(Triple::new(
        subject,
        predicate,
        ground_term(&pattern.object, target)?,
    ))
}

fn ground_term(
    pattern: &TermPattern,
    target: &HashMap<Variable, Term>,
) -> Result<Term, WhyNotError> {
    match pattern {
        TermPattern::NamedNode(node) => Ok(Term::NamedNode(node.clone())),
        TermPattern::BlankNode(node) => Ok(Term::BlankNode(node.clone())),
        TermPattern::Literal(literal) => Ok(Term::Literal(literal.clone())),
        TermPattern::Triple(triple) => Ok(Term::Triple(Box::new(ground_triple(triple, target)?))),
        TermPattern::Variable(variable) => binding(target, variable),
    }
}

fn binding(target: &HashMap<Variable, Term>, variable: &Variable) -> Result<Term, WhyNotError> {
    target
        .get(variable)
        .cloned()
        .ok_or_else(|| WhyNotError::MissingBinding(variable.clone()))
}

fn contains(graph: &Graph, triple: &Triple) -> bool {
    let subject = match &triple.subject {
        NamedOrBlankNode::NamedNode(node) => Term::NamedNode(node.clone()),
        NamedOrBlankNode::BlankNode(node) => Term::BlankNode(node.clone()),
    };
    let Some([Some(subject), Some(predicate), Some(object)]) = graph.pattern(
        Some(&subject),
        Some(&triple.predicate),
        Some(&triple.object),
    ) else {
        return false;
    };
    graph.store.contains([subject, predicate, object])
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode};
    use std::collections::HashSet;

    fn iri(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.com/{local}"))
    }

    fn variable(name: &str) -> Variable {
        Variable::new_unchecked(name)
    }

    fn variable_term(name: &str) -> TermPattern {
        TermPattern::Variable(variable(name))
    }

    fn iri_term(local: &str) -> TermPattern {
        TermPattern::NamedNode(iri(local))
    }

    fn pattern(subject: TermPattern, predicate: &str, object: TermPattern) -> TriplePattern {
        TriplePattern {
            subject,
            predicate: NamedNodePattern::NamedNode(iri(predicate)),
            object,
        }
    }

    #[test]
    fn mutation_witness_reports_exact_failing_conjunct_set() {
        let graph =
            Graph::load_str("@prefix : <http://example.com/> . :a :p :b .", "turtle").unwrap();
        let present = pattern(variable_term("x"), "p", variable_term("y"));
        let absent = pattern(variable_term("x"), "q", variable_term("z"));
        let algebra = GraphPattern::Bgp {
            patterns: vec![present.clone(), absent.clone()],
        };
        let target = HashMap::from([
            (variable("x"), Term::NamedNode(iri("a"))),
            (variable("y"), Term::NamedNode(iri("b"))),
            (variable("z"), Term::NamedNode(iri("c"))),
        ]);

        let explanation = why_not(&graph, &algebra, &target).unwrap();

        assert_eq!(explanation.len(), 1);
        assert_eq!(explanation[0].pattern(), &absent);
        assert_ne!(explanation[0].pattern(), &present);
        assert_eq!(
            explanation[0].grounded(),
            &Triple::new(iri("a"), iri("q"), iri("c"))
        );
    }

    #[test]
    fn all_present_returns_empty() {
        let graph = Graph::load_str(
            "@prefix : <http://example.com/> . :a :p :b; :q :c .",
            "turtle",
        )
        .unwrap();
        let algebra = GraphPattern::Bgp {
            patterns: vec![
                pattern(variable_term("x"), "p", variable_term("y")),
                pattern(variable_term("x"), "q", variable_term("z")),
            ],
        };
        let target = HashMap::from([
            (variable("x"), Term::NamedNode(iri("a"))),
            (variable("y"), Term::NamedNode(iri("b"))),
            (variable("z"), Term::NamedNode(iri("c"))),
        ]);

        assert!(why_not(&graph, &algebra, &target).unwrap().is_empty());
    }

    #[test]
    fn reports_every_absent_pattern_in_bgp_order() {
        let graph = Graph::default();
        let first = pattern(iri_term("a"), "p", iri_term("b"));
        let second = pattern(iri_term("a"), "q", iri_term("c"));
        let algebra = GraphPattern::Bgp {
            patterns: vec![first.clone(), second.clone()],
        };

        let explanation = why_not(&graph, &algebra, &HashMap::new()).unwrap();

        assert_eq!(
            explanation
                .iter()
                .map(MissingPattern::pattern)
                .collect::<Vec<_>>(),
            vec![&first, &second]
        );
    }

    /// sq-lsp7k: a parsed report exposes exactly the grounded missing
    /// triples, while a predicate mutation is observably rejected by the oracle.
    #[test]
    fn why_not_report_ntriples_round_trips_exact_grounded_triples() {
        let algebra = GraphPattern::Bgp {
            patterns: vec![
                pattern(variable_term("x"), "p", variable_term("y")),
                pattern(variable_term("x"), "q", variable_term("z")),
            ],
        };
        let mut target = HashMap::new();
        target.insert(variable("z"), Term::NamedNode(iri("c")));
        target.insert(variable("x"), Term::NamedNode(iri("a")));
        target.insert(variable("y"), Term::NamedNode(iri("b")));
        let missing = why_not(&Graph::default(), &algebra, &target).unwrap();

        let report = why_not_report_ntriples(&target, &missing).unwrap();
        let parsed: Vec<_> = oxttl::NTriplesParser::new()
            .for_reader(report.as_bytes())
            .collect::<Result<_, _>>()
            .expect("why-not report must be valid RDF 1.2 N-Triples");

        assert_eq!(parsed.len(), missing.len() * 5);
        assert_eq!(
            parsed
                .iter()
                .map(|triple| triple.subject.clone())
                .collect::<HashSet<_>>()
                .len(),
            missing.len(),
            "each missing conjunct must have exactly one report node"
        );
        let grounded: Vec<_> = parsed
            .iter()
            .filter(|triple| triple.predicate == rdf::REIFIES)
            .map(|triple| match &triple.object {
                Term::Triple(grounded) => grounded.as_ref().clone(),
                other => panic!("rdf:reifies must carry a triple term, got {other}"),
            })
            .collect();
        let expected: Vec<_> = missing
            .iter()
            .map(|entry| entry.grounded().clone())
            .collect();
        assert_eq!(grounded, expected);

        let mut mutated = expected;
        mutated[0] = Triple::new(iri("a"), iri("mutated"), iri("b"));
        assert_ne!(
            grounded, mutated,
            "mutation witness: an invented grounded triple must fail the oracle"
        );

        let absent_subjects: HashSet<_> = parsed
            .iter()
            .filter(|triple| {
                triple.predicate.as_str() == ABSENT
                    && triple.object == Term::Literal(Literal::from(true))
            })
            .map(|triple| triple.subject.clone())
            .collect();
        assert_eq!(absent_subjects.len(), missing.len());
    }

    /// sq-lsp7k: HashMap iteration does not influence bytes, and the
    /// Turtle surface denotes the exact same report graph as N-Triples.
    #[test]
    fn why_not_report_is_deterministic_and_turtle_round_trips() {
        let algebra = GraphPattern::Bgp {
            patterns: vec![pattern(variable_term("x"), "p", variable_term("y"))],
        };
        let target = HashMap::from([
            (variable("x"), Term::NamedNode(iri("a"))),
            (variable("y"), Term::NamedNode(iri("b"))),
        ]);
        let reverse_target = HashMap::from([
            (variable("y"), Term::NamedNode(iri("b"))),
            (variable("x"), Term::NamedNode(iri("a"))),
        ]);
        let missing = why_not(&Graph::default(), &algebra, &target).unwrap();

        let ntriples = why_not_report_ntriples(&target, &missing).unwrap();
        assert_eq!(
            ntriples,
            why_not_report_ntriples(&reverse_target, &missing).unwrap()
        );
        let turtle = why_not_report_turtle(&target, &missing).unwrap();
        assert!(turtle.contains("@prefix prov:"));
        assert!(turtle.contains("@prefix spqprov:"));

        let nt_graph: HashSet<_> = oxttl::NTriplesParser::new()
            .for_reader(ntriples.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        let turtle_graph: HashSet<_> = oxttl::TurtleParser::new()
            .for_slice(turtle.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(turtle_graph, nt_graph);
    }

    /// sq-lsp7k: a report cannot silently relabel an explanation with
    /// a different target binding.
    #[test]
    fn why_not_report_rejects_target_mismatch() {
        let algebra = GraphPattern::Bgp {
            patterns: vec![pattern(variable_term("x"), "p", variable_term("y"))],
        };
        let target = HashMap::from([
            (variable("x"), Term::NamedNode(iri("a"))),
            (variable("y"), Term::NamedNode(iri("b"))),
        ]);
        let missing = why_not(&Graph::default(), &algebra, &target).unwrap();
        let mismatched = HashMap::from([
            (variable("x"), Term::NamedNode(iri("a"))),
            (variable("y"), Term::NamedNode(iri("other"))),
        ]);

        assert_eq!(
            why_not_report_ntriples(&mismatched, &missing),
            Err(WhyNotError::TargetMismatch)
        );
    }

    #[test]
    fn unsupported_algebra_fails_closed() {
        let algebra = GraphPattern::Union {
            left: Box::new(GraphPattern::Bgp { patterns: vec![] }),
            right: Box::new(GraphPattern::Bgp { patterns: vec![] }),
        };

        assert_eq!(
            why_not(&Graph::default(), &algebra, &HashMap::new()),
            Err(WhyNotError::UnsupportedAlgebra)
        );
    }

    #[test]
    fn incomplete_or_invalid_target_fails_closed() {
        let subject_variable = variable("s");
        let predicate_variable = variable("p");
        let algebra = GraphPattern::Bgp {
            patterns: vec![TriplePattern {
                subject: TermPattern::Variable(subject_variable.clone()),
                predicate: NamedNodePattern::Variable(predicate_variable.clone()),
                object: iri_term("o"),
            }],
        };

        assert_eq!(
            why_not(&Graph::default(), &algebra, &HashMap::new()),
            Err(WhyNotError::MissingBinding(subject_variable.clone()))
        );

        let invalid_subject = HashMap::from([
            (
                subject_variable.clone(),
                Term::Literal(Literal::new_simple_literal("not-a-subject")),
            ),
            (predicate_variable.clone(), Term::NamedNode(iri("p"))),
        ]);
        assert!(matches!(
            why_not(&Graph::default(), &algebra, &invalid_subject),
            Err(WhyNotError::InvalidSubject(Term::Literal(_)))
        ));

        let target = HashMap::from([
            (subject_variable, Term::NamedNode(iri("s"))),
            (
                predicate_variable,
                Term::Literal(Literal::new_simple_literal("not-an-iri")),
            ),
        ]);
        assert!(matches!(
            why_not(&Graph::default(), &algebra, &target),
            Err(WhyNotError::InvalidPredicate(Term::Literal(_)))
        ));
    }
}
