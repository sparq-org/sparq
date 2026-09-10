// [GPT-6 ASTRA] Fixed synthetic UPDATE comparison through the public reference API.
use oxigraph::{sparql::QueryResults, store::Store};
use std::{collections::BTreeSet, error::Error};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn quads(store: &Store) -> TestResult<Vec<String>> {
    let mut result = Vec::new();
    for quad in store.iter() {
        let quad = quad?;
        result.push(format!("{} {} {} .", quad.subject, quad.predicate, quad.object));
    }
    result.sort();
    Ok(result)
}

#[expect(deprecated, reason = "Match the existing UPDATE differential query API")]
fn rows(store: &Store, query: &str, variables: &[&str]) -> TestResult<Vec<Vec<String>>> {
    let QueryResults::Solutions(solutions) = store.query(query)? else {
        return Err("Expected SELECT solutions".into());
    };
    let mut result = Vec::new();
    for solution in solutions {
        let solution = solution?;
        let mut row = Vec::new();
        for variable in variables {
            row.push(solution.get(*variable).ok_or("Missing binding")?.to_string());
        }
        result.push(row);
    }
    result.sort();
    Ok(result)
}

fn main() -> TestResult<()> {
    let eight = "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let padded = "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let nine = "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let cases: [(&str, &[&str]); 3] = [
        ("lexical-pair", &[eight, padded]),
        ("canonical-eight", &[eight]),
        ("different-values", &[eight, nine]),
    ];
    for (case, terms) in cases {
        let store = Store::new()?;
        let data: String = terms.iter().map(|term| format!("<http://ex/s2> <http://ex/p1> {term} . ")).collect();
        let first = format!("INSERT DATA {{ {data}}}");
        let second = "INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }";
        store.update(first.as_str())?;
        let before = quads(&store)?;
        let where_rows = rows(&store, "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }", &["s", "o"])?;
        store.update(second)?;
        let after = quads(&store)?;
        let blank_rows = rows(&store, "SELECT ?b WHERE { <http://ex/s2> <http://ex/p0> ?b }", &["b"])?;
        let distinct: BTreeSet<_> = blank_rows.iter().map(|row| &row[0]).collect();
        println!("{{\"case\":{case:?},\"terms\":{terms:?},\"first\":{first:?},\"second\":{second:?},\"before\":{before:?},\"where_rows\":{where_rows:?},\"after\":{after:?},\"blank_rows\":{blank_rows:?},\"distinct_blank_nodes\":{}}}", distinct.len());
        if !distinct.iter().all(|value| value.starts_with("_:")) || distinct.len() != where_rows.len() {
            return Err("Fresh blank-node count does not match observed WHERE solutions".into());
        }
    }
    Ok(())
}
