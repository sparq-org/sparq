//! sq-mg1wx: hermetic OWL 2 QL rewrite-then-execute benchmark.
//!
//! The fixture shapes mirror the shallow hierarchy, inverse-role, and conjunctive-query forms
//! found in NPD/Requiem-style OBDA suites. Every answer count is asserted before its timing row is
//! printed, so a semantic regression cannot be hidden by a plausible duration.

use oxrdf::Triple;
use spargebra::{Query, SparqlParser};
use sparq_core::Graph;
use sparq_reason_ql::rewrite_production;
use std::time::Instant;

const PREFIXES: &str = "PREFIX : <http://ex/> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX owl: <http://www.w3.org/2002/07/owl#> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";
const TURTLE_PREFIXES: &str = "@prefix : <http://ex/> . @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . @prefix owl: <http://www.w3.org/2002/07/owl#> . ";

struct Case {
    id: &'static str,
    query: &'static str,
    expected: usize,
}

const CASES: &[Case] = &[
    Case {
        id: "npd-class",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Asset }",
        expected: 3,
    },
    Case {
        id: "npd-role",
        query: "SELECT DISTINCT ?x ?y WHERE { ?x :relatedTo ?y }",
        expected: 3,
    },
    Case {
        id: "requiem-inverse",
        query: "SELECT DISTINCT ?company ?worker WHERE { ?company :employs ?worker }",
        expected: 2,
    },
    Case {
        id: "requiem-join",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Employee . ?x :worksFor ?c }",
        expected: 2,
    },
];

fn tbox() -> Vec<Triple> {
    let ttl = format!(
        "{TURTLE_PREFIXES} :Platform rdfs:subClassOf :Asset . :Well rdfs:subClassOf :Asset . :connectedTo rdfs:subPropertyOf :relatedTo . :linkedTo rdfs:subPropertyOf :relatedTo . :employs owl:inverseOf :worksFor . :Manager rdfs:subClassOf :Employee ."
    );
    oxttl::TurtleParser::new()
        .for_reader(ttl.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("embedded TBox must parse")
}

fn abox() -> Graph {
    Graph::load_str(
        "@prefix : <http://ex/> . :p1 a :Platform . :w1 a :Well . :a1 a :Asset . :p1 :connectedTo :w1 . :w1 :linkedTo :a1 . :a1 :relatedTo :p1 . :alice a :Manager ; :worksFor :acme . :bob a :Employee ; :worksFor :acme . :carol a :Employee .",
        "turtle",
    )
    .expect("embedded ABox must parse")
}

fn parse(body: &str) -> Query {
    SparqlParser::new()
        .parse_query(&format!("{PREFIXES}{body}"))
        .expect("embedded query must parse")
}

fn main() {
    let schema = tbox();
    let data = abox();
    println!("case\tanswers\trewriter_phase_ms\tend_to_end_ms");
    for case in CASES {
        let started = Instant::now();
        let rewrite_started = Instant::now();
        let rewritten = rewrite_production(&parse(case.query), &schema)
            .unwrap_or_else(|error| panic!("{} rewrite failed: {error}", case.id));
        let rewrite_ms = rewrite_started.elapsed().as_secs_f64() * 1_000.0;
        let answers = sparq_engine::count(&data, &rewritten.query.to_string())
            .unwrap_or_else(|error| panic!("{} execution failed: {error}", case.id));

        assert_eq!(
            answers, case.expected,
            "{} answer-set-size mismatch (timing row suppressed)",
            case.id
        );
        let end_to_end_ms = started.elapsed().as_secs_f64() * 1_000.0;
        println!(
            "{}\t{}\t{rewrite_ms:.6}\t{end_to_end_ms:.6}",
            case.id, answers
        );
    }
}
