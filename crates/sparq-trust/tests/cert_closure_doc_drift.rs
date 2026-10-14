//! Drift lock between the certificate-closure semantics note and `EdgeRejection`.
//!
//! sq-ry0by. 🤖 SPARQ agent — certificate-closure documentation guard.
#![cfg(feature = "cert-graph")]

use std::collections::BTreeSet;

const GRAPH_RS: &str = include_str!("../src/graph.rs");
const DOC: &str = include_str!("../ontologies/trust/CERT-CLOSURE.md");

fn code_variants() -> BTreeSet<String> {
    let body = GRAPH_RS
        .split_once("pub enum EdgeRejection {")
        .expect("graph.rs must define EdgeRejection")
        .1
        .split_once("\n}")
        .expect("EdgeRejection must have a closing brace")
        .0;

    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("///") && !line.starts_with("#"))
        .map(|line| {
            line.strip_suffix(',')
                .expect("EdgeRejection variants must use one unit variant per line")
        })
        .map(str::to_owned)
        .collect()
}

fn documented_variants() -> BTreeSet<String> {
    const PREFIX: &str = "`EdgeRejection::";
    DOC.match_indices(PREFIX)
        .map(|(start, _)| &DOC[start + PREFIX.len()..])
        .map(|tail| {
            tail.split_once('`')
                .expect("documented EdgeRejection reference must close its code span")
                .0
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn cert_closure_doc_names_exactly_the_edge_rejection_variants() {
    let code = code_variants();
    let documented = documented_variants();
    assert_eq!(
        documented, code,
        "CERT-CLOSURE.md and EdgeRejection have drifted; update the gate table with the enum"
    );
}
