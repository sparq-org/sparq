use super::*;

fn g() -> Graph {
    Graph::load_str(
        "@prefix : <http://ex/> .\n\
             :a :name \"Alice\" . :b :name \"bob\" . :c :name \"Carol123\" .\n\
             :a :age 30 . :b :age -5 .\n",
        "turtle",
    )
    .unwrap()
}
// Result-set size via the full materialising path (applies FILTER/BIND functions).
fn n(sparql: &str) -> usize {
    crate::query(&g(), sparql).unwrap().len()
}

#[test]
fn string_functions() {
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(STRLEN(?n) > 3) }"), 2); // Alice, Carol123
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(STRSTARTS(?n, \"A\")) }"), 1);
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(STRENDS(?n, \"3\")) }"), 1);
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(CONTAINS(LCASE(?n), \"o\")) }"), 2); // bob, carol
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(UCASE(?n) = \"BOB\") }"), 1);
    // BIND never drops rows.
    assert_eq!(n("SELECT ?g WHERE { ?s <http://ex/name> ?n BIND(CONCAT(?n, \"!\") AS ?g) }"), 3);
    assert_eq!(n("SELECT ?g WHERE { ?s <http://ex/name> ?n BIND(SUBSTR(?n, 1, 2) AS ?g) }"), 3);
}

#[test]
fn numeric_and_type_functions() {
    assert_eq!(n("SELECT ?a WHERE { ?s <http://ex/age> ?a FILTER(ABS(?a) > 10) }"), 1); // |30|
    assert_eq!(n("SELECT ?a WHERE { ?s <http://ex/age> ?a FILTER(isNumeric(?a)) }"), 2);
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(isLiteral(?n)) }"), 3);
    assert_eq!(n("SELECT ?s WHERE { ?s <http://ex/name> ?n FILTER(isIRI(?s)) }"), 3);
    assert_eq!(n("SELECT ?a WHERE { ?s <http://ex/age> ?a FILTER(FLOOR(?a) = ?a) }"), 2);
}

#[test]
fn lang_typed_and_sample() {
    let g = Graph::load_str(
        "@prefix : <http://ex/> . :a :name \"Alice\"@en . :b :name \"Bob\"@fr . :c :name \"X\" .",
        "turtle",
    )
    .unwrap();
    let q = |s: &str| crate::query(&g, s).unwrap().len();
    // LANGMATCHES: "en" matches the en literal; "*" matches any non-empty tag (en, fr; not X).
    assert_eq!(q("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(LANGMATCHES(LANG(?n), \"en\")) }"), 1);
    assert_eq!(q("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(LANGMATCHES(LANG(?n), \"*\")) }"), 2);
    // STRLANG / STRDT construct literals (exercised via BIND — all 3 rows).
    assert_eq!(q("SELECT ?x WHERE { ?s <http://ex/name> ?n BIND(STRLANG(STR(?n), \"de\") AS ?x) }"), 3);
    assert_eq!(
        q("SELECT ?x WHERE { ?s <http://ex/name> ?n BIND(STRDT(STR(?n), <http://www.w3.org/2001/XMLSchema#string>) AS ?x) }"),
        3
    );
    // SAMPLE: one row per group.
    let g2 = Graph::load_str("@prefix : <http://ex/> . :a :p :x . :a :p :y . :b :p :z .", "turtle").unwrap();
    assert_eq!(
        crate::query(&g2, "SELECT ?s (SAMPLE(?o) AS ?v) WHERE { ?s <http://ex/p> ?o } GROUP BY ?s").unwrap().len(),
        2
    );
}

#[test]
fn datetime_accessors() {
    let g = Graph::load_str(
        "@prefix : <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             :e :at \"2024-03-15T13:45:30\"^^xsd:dateTime .",
        "turtle",
    )
    .unwrap();
    let q = |s: &str| crate::query(&g, s).unwrap().len();
    assert_eq!(q("SELECT * WHERE { ?e <http://ex/at> ?d FILTER(YEAR(?d) = 2024 && MONTH(?d) = 3 && DAY(?d) = 15) }"), 1);
    assert_eq!(q("SELECT * WHERE { ?e <http://ex/at> ?d FILTER(HOURS(?d) = 13 && MINUTES(?d) = 45 && SECONDS(?d) = 30) }"), 1);
}

#[cfg(feature = "regex")]
#[test]
fn regex_functions() {
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(REGEX(?n, \"^[A-Z]\")) }"), 2); // Alice, Carol123
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(REGEX(?n, \"BOB\", \"i\")) }"), 1);
    assert_eq!(n("SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(REGEX(?n, \"[0-9]+\")) }"), 1); // Carol123
    assert_eq!(n("SELECT ?g WHERE { ?s <http://ex/name> ?n BIND(REPLACE(?n, \"[0-9]\", \"X\") AS ?g) }"), 3);
}
