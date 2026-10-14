//! `BNODE()` / `BNODE(str)` mint blank nodes distinct from every blank node in the
//! dataset (SPARQL 1.1 §17.4.2.9), even where a stored label equals the minted one.

use sparq_core::Graph;
use sparq_engine::query;

const EX: &str = "PREFIX ex: <http://example.org/>\n";

fn label(g: &Graph, q: &str) -> String {
    let r = query(g, &format!("{EX}{q}")).expect("query");
    r.rows[0][0].as_ref().expect("bound").to_string()
}

fn rows(g: &Graph, q: &str) -> usize {
    query(g, &format!("{EX}{q}")).expect("query").len()
}

/// One test: `BNODE()` draws from a process-wide counter, so parallel tests in this
/// binary would race for the labels the dataset pre-occupies.
#[test]
fn minted_blank_nodes_never_equal_stored_ones() {
    let empty = Graph::load_str("", "turtle").expect("graph");
    // BNODE(): occupy the next labels the counter will produce.
    let seen = label(&empty, "SELECT (BNODE() AS ?b) WHERE {}");
    let n: u64 = seen
        .trim_start_matches("_:fnb")
        .parse()
        .expect("fnb<n> label");
    let mut ttl = String::from("@prefix ex: <http://example.org/> .\nex:d ex:rank 5 .\n");
    for i in n + 1..n + 2000 {
        ttl.push_str(&format!("_:fnb{i} ex:rank 1 .\n"));
    }
    let g = Graph::load_str(&ttl, "turtle").expect("graph");
    let q = "SELECT ?s WHERE { ?s ex:rank ?r BIND(BNODE() AS ?b)
             FILTER EXISTS { ?b2 ex:rank 1 FILTER(sameTerm(?b, ?b2)) } }";
    assert_eq!(rows(&g, q), 0, "a minted node matched a stored blank node");
    let q = "SELECT ?s WHERE { ?s ex:rank ?r BIND(BNODE() AS ?b)
             OPTIONAL { ?k2 ex:rank ?r2 FILTER(sameTerm(?b, ?k2) && ?r2 < ?r) }
             FILTER(!bound(?k2)) }";
    assert_eq!(rows(&g, q), 2000, "every row survives the anti-join");

    // BNODE(str): equal arguments in one solution still share a node.
    let g = Graph::load_str(
        "@prefix ex: <http://example.org/> . ex:d ex:rank 5 .",
        "turtle",
    )
    .expect("graph");
    let same_row = "SELECT ?s WHERE { ex:d ex:rank ?r
                    BIND(BNODE(\"k\") AS ?a) BIND(BNODE(\"k\") AS ?b) FILTER(sameTerm(?a, ?b)) }";
    assert_eq!(
        rows(&g, same_row),
        1,
        "equal arguments in one solution share a node"
    );
}
