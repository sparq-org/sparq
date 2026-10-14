// sq-wxaas (QL track, phase Q1): the RL-MATERIALISE-THEN-QUERY acceptance oracle.
//
// The crate's other oracle (`tests/oracle.rs`) checks the rewriter SYNTACTICALLY — it pins the
// exact union of conjunctive queries PerfectRef must emit against hand-derived expectations. That
// is necessary but not sufficient: a UCQ can be the textbook-correct rewriting and still be
// mis-evaluated, and a syntactic oracle can only ever be as right as the hand derivation.
//
// This file closes that gap with an INDEPENDENT, EXECUTABLE baseline: the certain answers the
// rewriter computes are compared against the answers sparq's OWL 2 RL materialiser
// (`sparq_reason::materialize(Profile::OwlRl, …)`) produces for the SAME query over the SAME
// input graph. Two entirely separate reasoning strategies — rewrite-the-query vs
// close-the-data — must agree wherever both are complete.
//
// ## Why the two strategies are comparable at all, and exactly where they are not
//
// OWL 2 RL and OWL 2 QL are INCOMPARABLE profiles; a naive "RL and QL must always agree" oracle
// would be dishonest. The overlap where BOTH are complete for ABox (instance-retrieval) answers
// is the fragment used by the `Agreement::Exact` fixtures below:
//
//   * `A rdfs:subClassOf B` between named classes  — RL `cax-sco`   / QL `A ⊑ B`
//   * `R rdfs:subPropertyOf S`                     — RL `prp-spo1`  / QL `R ⊑ S`
//   * `R owl:inverseOf S`                          — RL `prp-inv1/2`/ QL `R ⊑ S⁻`, `S⁻ ⊑ R`
//   * `R rdfs:domain A`                            — RL `prp-dom`   / QL `∃R ⊑ A`
//   * `R rdfs:range A`                             — RL `prp-rng`   / QL `∃R⁻ ⊑ A`
//   * `A owl:equivalentClass B` between named classes — RL `cax-eqc1/2` / QL `A ⊑ B` + `B ⊑ A`
//
// OUTSIDE that overlap the profiles genuinely diverge, and the divergence has a DIRECTION. The
// `Agreement::QlStrictlyRicher` fixture pins it: an EXISTENTIAL-GENERATING axiom `A ⊑ ∃R` is
// QL-legal but has no OWL 2 RL superclass form at all (`owl:someValuesFrom` is not an RL
// superclass expression), so RL materialisation invents no witness and silently returns fewer
// answers. That is the structural RL incompleteness recorded in
// `research/owl2-el-ql-reasoning-spike.md` §1 — the capability gap QL exists to fill — so the
// test asserts RL ⊊ QL there rather than pretending the two agree.
//
// Both routes are handed the IDENTICAL input graph (TBox triples and ABox triples in one Turtle
// document); the only difference is the reasoning strategy. The rewritten UCQ is evaluated over
// the UNMODIFIED data, which is the FO-rewritability property under test.

#![cfg(feature = "experimental")]

use oxrdf::Triple;
use sparq_core::Graph;
use sparq_engine::QueryResult;
use sparq_reason::Profile;
use sparq_reason_ql::rewrite_production;
use spargebra::SparqlParser;
use std::collections::BTreeSet;

const TURTLE_PREFIXES: &str = "\
@prefix : <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
";

const QUERY_PREFIXES: &str = "\
PREFIX : <http://ex/>
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
";

/// How the QL certain answers must relate to the RL materialise-then-query baseline.
#[derive(Clone, Copy, Debug)]
enum Agreement {
    /// The fixture lies in the RL ∩ QL overlap: both profiles are complete, so the two answer
    /// sets must be EQUAL. Any difference in either direction is a bug.
    Exact,
    /// The fixture uses a QL construct with no RL counterpart. QL must return a STRICT superset:
    /// every RL answer is still certain (soundness of the rewriting), plus at least one answer RL
    /// structurally cannot reach.
    QlStrictlyRicher,
}

struct Case {
    id: &'static str,
    /// Why this fixture is in the stated agreement class — the load-bearing justification.
    rationale: &'static str,
    tbox: &'static str,
    abox: &'static str,
    query: &'static str,
    agreement: Agreement,
}

const CASES: &[Case] = &[
    Case {
        id: "subclass-hierarchy",
        rationale: "named-class rdfs:subClassOf — RL cax-sco is complete, QL A ⊑ B is complete",
        tbox: ":Platform rdfs:subClassOf :Asset . :Well rdfs:subClassOf :Asset . \
               :Wellbore rdfs:subClassOf :Well .",
        abox: ":p1 a :Platform . :w1 a :Well . :b1 a :Wellbore . :a1 a :Asset .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Asset }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "subproperty-hierarchy",
        rationale: "rdfs:subPropertyOf — RL prp-spo1 is complete, QL R ⊑ S is complete",
        tbox: ":connectedTo rdfs:subPropertyOf :relatedTo . \
               :linkedTo rdfs:subPropertyOf :connectedTo .",
        abox: ":p1 :connectedTo :w1 . :w1 :linkedTo :a1 . :a1 :relatedTo :p1 .",
        query: "SELECT DISTINCT ?x ?y WHERE { ?x :relatedTo ?y }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "inverse-role",
        rationale: "owl:inverseOf — RL prp-inv1/2 is complete, QL captures both R ⊑ S⁻ and S⁻ ⊑ R",
        tbox: ":employs owl:inverseOf :worksFor .",
        abox: ":alice :worksFor :acme . :bob :worksFor :acme . :acme :employs :carol .",
        query: "SELECT DISTINCT ?company ?worker WHERE { ?company :employs ?worker }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "domain-range",
        rationale: "rdfs:domain / rdfs:range — RL prp-dom/prp-rng vs QL ∃R ⊑ A / ∃R⁻ ⊑ A",
        tbox: ":worksFor rdfs:domain :Employee . :worksFor rdfs:range :Company .",
        abox: ":alice :worksFor :acme . :bob :worksFor :globex . :carol a :Employee .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Employee }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "range-position",
        rationale: "the ∃R⁻ ⊑ A direction specifically — a distinct QL rewriting step from ∃R ⊑ A",
        tbox: ":worksFor rdfs:domain :Employee . :worksFor rdfs:range :Company .",
        abox: ":alice :worksFor :acme . :bob :worksFor :globex . :initech a :Company .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Company }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "equivalent-class",
        rationale: "named-class owl:equivalentClass — RL cax-eqc1/2 vs QL's two-inclusion decomposition",
        tbox: ":Person owl:equivalentClass :Human . :Manager rdfs:subClassOf :Person .",
        abox: ":alice a :Person . :bob a :Human . :carol a :Manager .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Human }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "conjunctive-join",
        rationale: "a genuine two-atom CQ — the join must survive rewriting, not just single atoms",
        tbox: ":Manager rdfs:subClassOf :Employee . :contractsWith rdfs:subPropertyOf :worksFor .",
        abox: ":alice a :Manager ; :worksFor :acme . :bob a :Employee ; :contractsWith :acme . \
               :carol a :Employee . :dan a :Manager .",
        query: "SELECT DISTINCT ?x ?c WHERE { ?x a :Employee . ?x :worksFor ?c }",
        agreement: Agreement::Exact,
    },
    Case {
        id: "existential-generation",
        rationale: "A ⊑ ∃R has NO OWL 2 RL superclass form — RL invents no witness, QL rewrites \
                    the existential atom away; the profiles legitimately diverge, QL-upward",
        tbox: ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .",
        abox: ":alice a :Employee . :bob :worksFor :acme .",
        // ?y is NOT projected, so it is an existential (non-distinguished) variable — exactly the
        // shape PerfectRef's reduce/rewrite step collapses against A ⊑ ∃R.
        query: "SELECT DISTINCT ?x WHERE { ?x :worksFor ?y }",
        agreement: Agreement::QlStrictlyRicher,
    },
];

/// One solution row as an order-independent, variable-keyed binding set, so the comparison is
/// immune to projection order and row order.
type Binding = Vec<(String, String)>;

fn answers(result: &QueryResult) -> BTreeSet<Binding> {
    let vars: Vec<&str> = result.vars.iter().map(|v| v.as_str()).collect();
    result
        .rows
        .iter()
        .map(|row| {
            let mut binding: Binding = vars
                .iter()
                .zip(row.iter())
                .filter_map(|(var, term)| {
                    term.as_ref()
                        .map(|t| ((*var).to_string(), t.to_string()))
                })
                .collect();
            binding.sort();
            binding
        })
        .collect()
}

fn document(case: &Case) -> String {
    format!("{TURTLE_PREFIXES}{} {}", case.tbox, case.abox)
}

/// The BASELINE: close the graph under OWL 2 RL, then run the ORIGINAL query over the closure.
fn rl_baseline(case: &Case) -> BTreeSet<Binding> {
    let (mut dict, mut triples) = Graph::parse_to_triples(&document(case), "turtle")
        .unwrap_or_else(|e| panic!("{}: fixture must parse: {e}", case.id));
    sparq_reason::materialize(Profile::OwlRl, &mut dict, &mut triples);
    let closed = Graph::from_parts(dict, triples);
    let sparql = format!("{QUERY_PREFIXES}{}", case.query);
    let result = sparq_engine::query(&closed, &sparql)
        .unwrap_or_else(|e| panic!("{}: RL-baseline query must execute: {e}", case.id));
    answers(&result)
}

/// The SYSTEM UNDER TEST: rewrite the query against the TBox, then run the UCQ over the
/// UNMODIFIED graph — no materialisation anywhere on this path.
fn ql_certain_answers(case: &Case) -> BTreeSet<Binding> {
    let doc = document(case);
    let tbox: Vec<Triple> = oxttl::TurtleParser::new()
        .for_slice(doc.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("{}: fixture must parse as Turtle: {e}", case.id));
    let parsed = SparqlParser::new()
        .parse_query(&format!("{QUERY_PREFIXES}{}", case.query))
        .unwrap_or_else(|e| panic!("{}: query must parse: {e}", case.id));

    let rewritten = rewrite_production(&parsed, &tbox)
        .unwrap_or_else(|e| panic!("{}: fixture must be in QL rewriting scope: {e}", case.id));

    let data = Graph::load_str(&doc, "turtle")
        .unwrap_or_else(|e| panic!("{}: fixture must load: {e}", case.id));
    let result = sparq_engine::query(&data, &rewritten.query.to_string())
        .unwrap_or_else(|e| panic!("{}: rewritten query must execute: {e}", case.id));
    answers(&result)
}

/// The acceptance oracle: every fixture's QL certain answers agree with the RL
/// materialise-then-query baseline in the way its `Agreement` class declares.
#[test]
fn ql_certain_answers_match_rl_materialisation_baseline() {
    for case in CASES {
        let baseline = rl_baseline(case);
        let certain = ql_certain_answers(case);

        assert!(
            baseline.is_subset(&certain),
            "{}: QL dropped an answer the RL closure entails — the rewriting is UNSOUND or \
             incomplete. rationale: {}\n  RL baseline: {:?}\n  QL certain:  {:?}",
            case.id,
            case.rationale,
            baseline,
            certain
        );

        match case.agreement {
            Agreement::Exact => assert_eq!(
                certain, baseline,
                "{}: fixture is in the RL ∩ QL overlap, so the two strategies must agree exactly. \
                 rationale: {}",
                case.id, case.rationale
            ),
            Agreement::QlStrictlyRicher => assert!(
                certain.len() > baseline.len(),
                "{}: fixture is meant to exhibit the RL incompleteness QL repairs, but QL found \
                 no extra answer — either the fixture stopped exercising the existential or the \
                 rewriter regressed. rationale: {}\n  RL baseline: {:?}\n  QL certain:  {:?}",
                case.id,
                case.rationale,
                baseline,
                certain
            ),
        }
    }
}

/// Guards the `Agreement::Exact` classification itself. If a fixture that claims to be in the
/// RL ∩ QL overlap were also satisfiable WITHOUT any reasoning, `Exact` agreement would be
/// vacuous — both strategies would trivially return the raw asserted answers. Each overlap
/// fixture must therefore yield strictly MORE answers than plain evaluation over the raw graph.
#[test]
fn overlap_fixtures_are_non_vacuous() {
    for case in CASES {
        if !matches!(case.agreement, Agreement::Exact) {
            continue;
        }
        let raw = Graph::load_str(&document(case), "turtle")
            .unwrap_or_else(|e| panic!("{}: fixture must load: {e}", case.id));
        let unreasoned = sparq_engine::query(&raw, &format!("{QUERY_PREFIXES}{}", case.query))
            .unwrap_or_else(|e| panic!("{}: raw query must execute: {e}", case.id));

        assert!(
            answers(&unreasoned).len() < rl_baseline(case).len(),
            "{}: fixture is VACUOUS — plain evaluation already returns every answer, so the \
             RL-vs-QL agreement proves nothing about reasoning",
            case.id
        );
    }
}
