//! RDF 1.2 syntax/eval/C14N floors measured at w3c/rdf-tests
//! 7b5afb91857a9c282c03d2e45d3f76485797c901. Counts may only rise.

/// (suite directory, manifest directory, passing test floor).
pub const LANES: &[(&str, &str, usize)] = &[
    ("rdf-turtle", "syntax", 77),
    ("rdf-turtle", "eval", 32),
    ("rdf-trig", "syntax", 38),
    ("rdf-trig", "eval", 26),
    ("rdf-n-triples", "syntax", 34),
    ("rdf-n-triples", "c14n", 41),
    ("rdf-n-quads", "syntax", 31),
    ("rdf-n-quads", "c14n", 41),
];
