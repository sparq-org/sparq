//! sq-384j — Tested library quickstart, embedded into the docs guide.
//!
//! This is the canonical "use sparq as a library" example. The region between the
//! `ANCHOR: quickstart` / `ANCHOR_END: quickstart` markers is single-sourced into
//! `book/src/getting-started/install.md` via mdBook's `{{#rustdoc_include}}`, so the
//! snippet a reader sees in the guide is a fragment of THIS file — a file that
//! `cargo test -p sparq-engine --examples` actually compiles and runs (see the
//! `#[test]` below). Keep the anchored region self-contained and dependency-light
//! (only `sparq-core` + `sparq-engine`, both already direct deps of this crate) so
//! the guide example stays honest and never drifts from a passing test.
//!
//! Run it directly with:
//!   cargo run -p sparq-engine --example quickstart

// ANCHOR: quickstart
use sparq_core::Graph;
use sparq_engine::query;

fn count_people(turtle: &str) -> Result<usize, String> {
    // Parse RDF (Turtle/N-Triples/N-Quads/TriG) straight from a string.
    let graph = Graph::load_str(turtle, "turtle")?;

    // Run a SPARQL 1.1 SELECT. The result exposes `vars` (the projected
    // variables) and `rows` (each a `Vec<Option<Term>>`, one cell per var).
    let result = query(
        &graph,
        "SELECT ?person WHERE { ?person a <http://schema.org/Person> }",
    )?;

    Ok(result.rows.len())
}
// ANCHOR_END: quickstart

fn main() {
    let turtle = r#"
        @prefix schema: <http://schema.org/> .
        <http://example.org/alice> a schema:Person .
        <http://example.org/bob>   a schema:Person .
        <http://example.org/acme>  a schema:Organization .
    "#;

    let n = count_people(turtle).expect("query should succeed");
    println!("{n} people");
}

#[cfg(test)]
mod tests {
    use super::count_people;

    // The same code path the docs guide shows, exercised by `cargo test`. If the
    // public API drifts, this test (and therefore the embedded guide snippet)
    // breaks loudly rather than silently going stale.
    #[test]
    fn quickstart_counts_people() {
        let turtle = r#"
            @prefix schema: <http://schema.org/> .
            <http://example.org/alice> a schema:Person .
            <http://example.org/bob>   a schema:Person .
            <http://example.org/acme>  a schema:Organization .
        "#;
        assert_eq!(count_people(turtle).unwrap(), 2);
    }
}
