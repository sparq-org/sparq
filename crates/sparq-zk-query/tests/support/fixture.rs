use oxrdf::{Literal, NamedNode, Term};
use sparq_zk_query::{
    interp::{Cell, CredentialCells, EncodeTerm},
    Error,
};

/// The fixed credential-suite encoder also drives the operator fixtures.
pub struct FixtureEncoder;
impl EncodeTerm for FixtureEncoder {
    fn encode(&self, term: &Term) -> Result<Cell, Error> {
        sparq_zk_query::credential::SuiteEncoder.encode(term)
    }
}

pub fn iri(s: &str) -> Term {
    NamedNode::new(s).unwrap().into()
}
#[allow(dead_code)]
pub fn integer(n: i64) -> Term {
    Literal::new_typed_literal(
        n.to_string(),
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}
pub fn quad(s: &Term, p: &Term, o: &Term) -> [Cell; 4] {
    let enc = FixtureEncoder;
    [
        enc.encode(s).unwrap(),
        enc.encode(p).unwrap(),
        enc.encode(o).unwrap(),
        Cell::default(),
    ]
}
#[allow(dead_code)]
pub fn fixture() -> Vec<CredentialCells> {
    vec![CredentialCells {
        quads: vec![
            quad(&iri("urn:a"), &iri("urn:p"), &integer(1)),
            quad(&iri("urn:b"), &iri("urn:p"), &integer(2)),
        ],
    }]
}

#[allow(dead_code)]
pub fn dataset(rdf: &str) -> (Vec<CredentialCells>, Vec<Term>) {
    let mut quads = vec![];
    let mut vocabulary = vec![];
    for triple in oxttl::NTriplesParser::new().for_reader(rdf.as_bytes()) {
        let triple = triple.unwrap();
        let s: Term = triple.subject.into();
        let p: Term = triple.predicate.into();
        let o = triple.object;
        let mut q = quad(&s, &p, &o);
        for c in &mut q {
            if c.kind == 2 {
                c.scope = 1;
            }
        }
        quads.push(q);
        vocabulary.extend([s, p, o]);
    }
    (vec![CredentialCells { quads }], vocabulary)
}

#[allow(dead_code)]
pub fn generated_dataset(mut seed: u64) -> (String, Vec<CredentialCells>, Vec<Term>) {
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) % 5
    };
    let mut rdf = String::new();
    for i in 0..4 {
        let n = random() as i64 - 2;
        rdf.push_str(&format!("<urn:s{i}> <urn:p> \"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:s{i}> <urn:q> <urn:group{}> .\n",random()%2));
    }
    let (credentials, vocabulary) = dataset(&rdf);
    (rdf, credentials, vocabulary)
}

#[allow(dead_code)]
pub const DIFFERENTIAL_QUERIES: &[&str] = &[
    "ASK {VALUES ?x {0} FILTER(?x)}",
    "ASK {VALUES ?x {1} FILTER(?x)}",
    "ASK {VALUES ?x {false} FILTER(?x)}",
    "ASK {VALUES ?x {true} FILTER(?x)}",
    "ASK {VALUES ?x {\"\"} FILTER(?x)}",
    "ASK {VALUES ?x {\"x\"} FILTER(?x)}",
    "SELECT ?s ?x WHERE {?s <urn:p> ?x}",
    "SELECT ?x WHERE {?s <urn:p> ?x}",
    "SELECT DISTINCT ?x WHERE {?s <urn:p> ?x}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= -1 && ?x < 2)}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x IN (-2,0,2))}",
    "SELECT ?s WHERE {?s <urn:p> ?x VALUES ?x {-1 1 UNDEF}}",
    "SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:q> ?y}",
    "SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:p> ?x}",
    "ASK {?s <urn:p> ?x FILTER(?x > 5)}",
    "ASK {?s <urn:p> ?x}",
    "ASK {?s <urn:p> ?s}",
    "SELECT ?s WHERE {?s <urn:p> ?x} LIMIT 0",
];

#[allow(dead_code)]
pub fn assert_engine(query: &str, rdf: &str, actual: &sparq_zk_query::interp::ResultValue) {
    assert_engine_with_encoder(query, rdf, actual, &FixtureEncoder)
}
#[allow(dead_code)]
pub fn assert_engine_with_encoder(
    query: &str,
    rdf: &str,
    actual: &sparq_zk_query::interp::ResultValue,
    encoder: &impl EncodeTerm,
) {
    use sparq_zk_query::interp::ResultValue;
    let graph = sparq_core::Graph::load_str(rdf, "ntriples").unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&sparq_engine::query_json(&graph, query).unwrap()).unwrap();
    match actual {
        ResultValue::Ask(value) => {
            assert_eq!(Some(*value), expected["boolean"].as_bool(), "{query}")
        }
        ResultValue::Select { variables, rows } => {
            let mut expected_rows = vec![];
            for row in expected["results"]["bindings"].as_array().unwrap() {
                let mut cells = vec![];
                for variable in variables {
                    let b = &row[variable];
                    let term = match b["type"].as_str() {
                        None => None,
                        Some("uri") => Some(iri(b["value"].as_str().unwrap())),
                        Some("bnode") => Some(
                            oxrdf::BlankNode::new(b["value"].as_str().unwrap())
                                .unwrap()
                                .into(),
                        ),
                        Some("literal" | "typed-literal") => {
                            Some(if let Some(lang) = b["xml:lang"].as_str() {
                                Literal::new_language_tagged_literal(
                                    b["value"].as_str().unwrap(),
                                    lang,
                                )
                                .unwrap()
                                .into()
                            } else {
                                Literal::new_typed_literal(
                                    b["value"].as_str().unwrap(),
                                    NamedNode::new(
                                        b["datatype"]
                                            .as_str()
                                            .unwrap_or("http://www.w3.org/2001/XMLSchema#string"),
                                    )
                                    .unwrap(),
                                )
                                .into()
                            })
                        }
                        other => panic!("binding type {other:?}"),
                    };
                    cells.push(term.as_ref().map(|t| encoder.encode(t).unwrap().enc));
                }
                expected_rows.push(cells);
            }
            let mut actual_rows: Vec<Vec<_>> = rows
                .iter()
                .map(|r| r.iter().map(|c| c.bound.then_some(c.enc)).collect())
                .collect();
            actual_rows.sort();
            expected_rows.sort();
            assert_eq!(actual_rows, expected_rows, "{query}");
        }
    }
}

#[allow(dead_code)]
pub struct Workspace(pub std::path::PathBuf);
#[allow(dead_code)]
impl Workspace {
    pub fn new(package: &sparq_zk_query::codegen::Package) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sparq_query_execution_{}_{}",
            std::process::id(),
            package.name
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, source) in &package.files {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, source).unwrap();
        }
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap();
        std::os::unix::fs::symlink(core, root.join("zk")).unwrap();
        Self(root)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[allow(dead_code)]
pub struct N0Case {
    pub name: &'static str,
    pub query: &'static str,
    pub sources: Vec<String>,
}
#[allow(dead_code)]
pub fn n0_cases() -> Vec<N0Case> {
    let numeric="<urn:a> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:b> <urn:p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
    let linked = "<urn:a> <urn:q> <urn:g> .\n<urn:b> <urn:q> <urn:h> .\n";
    let decimals="<urn:a> <urn:p> \"1.25\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n<urn:b> <urn:p> \"2.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n";
    let dates="<urn:a> <urn:p> \"2024-01-01\"^^<http://www.w3.org/2001/XMLSchema#date> .\n<urn:b> <urn:p> \"2024-02-01\"^^<http://www.w3.org/2001/XMLSchema#date> .\n";
    vec![
        N0Case{name:"bgp", query:"SELECT ?s ?x WHERE {?s <urn:p> ?x}",sources:vec![numeric.into()]},
        N0Case{name:"join-one-credential",query:"SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:q> ?y}",sources:vec![format!("{numeric}{linked}")]},
        N0Case{name:"join-two-credentials",query:"SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:q> ?y}",sources:vec![numeric.into(),linked.into()]},
        N0Case{name:"integer-range",query:"SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 2)}",sources:vec![numeric.into()]},
        N0Case{name:"decimal-range",query:"SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1.25 && ?x < 2.5)}",sources:vec![decimals.into()]},
        N0Case{name:"date-range",query:"PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= \"2024-01-01\"^^xsd:date && ?x < \"2024-02-01\"^^xsd:date)}",sources:vec![dates.into()]},
        N0Case{name:"membership",query:"SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x IN (1,3))}",sources:vec![numeric.into()]},
        N0Case{name:"values",query:"SELECT ?s WHERE {?s <urn:p> ?x VALUES ?x {1 UNDEF}}",sources:vec![numeric.into()]},
        N0Case{name:"projection-bag",query:"SELECT ?p WHERE {?s ?p ?o}",sources:vec![numeric.into()]},
        N0Case{name:"distinct",query:"SELECT DISTINCT ?p WHERE {?s ?p ?o}",sources:vec![numeric.into()]},
        N0Case{name:"slice",query:"SELECT ?s WHERE {?s <urn:p> ?x} LIMIT 1 OFFSET 1",sources:vec![numeric.into()]},
        N0Case{name:"ask-true",query:"ASK {?s <urn:p> ?x}",sources:vec![numeric.into()]},
        N0Case{name:"ask-false",query:"ASK {?s <urn:p> ?x FILTER(?x > 8)}",sources:vec![numeric.into()]},
    ]
}

#[allow(dead_code)]
pub fn mixed_string_queries() -> Vec<(String, bool, bool)> {
    let mut cases = vec![];
    for lhs in [
        "\"a\"",
        "\"a-long-string-with-the-same-prefix-and-an-extra-suffix\"",
    ] {
        for (rhs, expected) in [
            ("\"a\"@en", true),
            ("1", false),
            (
                "\"2026-11-13\"^^<http://www.w3.org/2001/XMLSchema#date>",
                false,
            ),
            ("true", false),
            ("<urn:a>", true),
        ] {
            for (left, right) in [(lhs, rhs), (rhs, lhs)] {
                cases.push((
                    format!("ASK {{ FILTER(!({left} = {right})) }}"),
                    expected && rhs != "\"a\"@en",
                    expected,
                ));
            }
        }
    }
    for (lhs, rhs, expected) in [
        (r#""a\u0000""#, r#""a""#, true),
        (r#""\u0000""#, r#""\u0000""#, false),
        (r#""\u0000""#, r#""""#, true),
        (r#""a\u0000""#, r#""a\u0000b""#, true),
        (
            "\"same-prefix-that-is-long-enough-to-have-a-suffix-a\"",
            "\"same-prefix-that-is-long-enough-to-have-a-suffix-b\"",
            true,
        ),
        ("\"a\"@en", "\"a\"@en", false),
        ("\"a\"@en", "\"a\"@fr", true),
    ] {
        cases.push((
            format!("ASK {{ FILTER(!({lhs} = {rhs})) }}"),
            expected && !lhs.ends_with("@en"),
            expected,
        ));
    }
    cases
}
