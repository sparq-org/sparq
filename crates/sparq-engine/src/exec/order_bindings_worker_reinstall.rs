use super::*;
use std::sync::Arc;

/// Build an N-row nquads dataset: default-graph rows `<s{i}> <p> <o{i}>` plus one
/// triple in a named graph `<http://ex/hidden>` so EXISTS can find it when no view
/// is restricting access.
fn par_dataset_with_hidden_graph(n: usize) -> sparq_core::Graph {
    let mut nq = String::with_capacity(n * 60 + 80);
    for i in 0..n {
        nq.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
    }
    nq.push_str("<http://ex/hs> <http://ex/hp> <http://ex/ho> <http://ex/hidden> .\n");
    sparq_core::Graph::load_dataset(&nq, "nquads").unwrap()
}

/// ORDER BY with EXISTS under a restricted DatasetView — >= PAR_THRESHOLD rows.
///
/// Setup: the named graph `<http://ex/hidden>` contains a triple, but the installed
/// DatasetView exposes an EMPTY named-graph set, so `GRAPH <http://ex/hidden>` must
/// be invisible.
///
/// The ORDER BY expression is:
///   `IF(EXISTS { GRAPH <http://ex/hidden> { ?a ?b ?c } }, "constant_key", STR(?s))`
///
/// Correct behaviour (view propagated to workers):
///   EXISTS = false → sort key = STR(?s) → ascending lexicographic order by subject IRI.
///
/// Bug behaviour (view NOT propagated to workers, pre-fix):
///   EXISTS = true on worker threads → sort key = "constant_key" for every row →
///   stable sort preserves scan/insertion order, which is numeric (s0,s1,s2,…), NOT
///   lexicographic (s0,s1,s10,s100,…) — observable mismatch for N >= 10.
///
/// NON-VACUITY CONFIRMED: stashing the fix and running produces
///   `subjects != expected_lex_sorted` → assertion fires → RED on the bug.
#[test]
fn order_by_exists_restricted_view_parallel_matches_lex_sort() {
    let n = PAR_THRESHOLD + 100; // triggers both PAR_THRESHOLD parallel paths
    let g = par_dataset_with_hidden_graph(n);

    // View: empty named set → <http://ex/hidden> is NOT accessible.
    let view = crate::DatasetView {
        base: &g,
        named: Arc::new(crate::FxHashSet::default()),
        default: crate::DefaultGraphMode::StoreDefault,
    };

    // ORDER BY: sort by STR(?s) when EXISTS is false (view active), else constant tie.
    let query = "SELECT ?s WHERE { ?s <http://ex/p> ?o } \
                     ORDER BY IF(EXISTS { GRAPH <http://ex/hidden> { ?a ?b ?c } }, \
                                 \"constant_key\", STR(?s))";

    let result = crate::query_view_with_budget(
        &view,
        query,
        &crate::QueryBudget::unlimited(),
    )
    .unwrap();
    assert_eq!(result.len(), n, "all {n} default-graph rows must survive the query");

    // Collect the subject IRIs in the ORDER BY result.
    // `term.to_string()` yields `<http://ex/sN>` (N3 notation, with angle brackets).
    let subjects: Vec<String> = result.rows.iter()
        .map(|row| row[0].clone().unwrap().to_string())
        .collect();

    // Oracle: the expected output is ascending lexicographic order of STR(?s).
    // SPARQL's STR(?s) on a NamedNode strips angle brackets, producing "http://ex/sN".
    // Lex order of "http://ex/s0".."http://ex/s{n-1}" differs from numeric insertion order
    // for any n > 10: "…/s0","…/s1","…/s10","…/s100",… ≠ "…/s0","…/s1","…/s2","…/s3",…
    //
    // IMPORTANT: do NOT use `subjects.sort()` here — Rust's byte-lex sort of the N3
    // representation "<http://ex/sN>" differs from SPARQL's STR(?s) sort because `>`
    // (ASCII 62) > `0`-`9` (48-57): "<http://ex/s10000>" sorts before "<http://ex/s1>"
    // (since `>` at the end of the shorter IRI is a higher byte than `0`).  Instead,
    // sort by the bare IRI value (strip angle brackets) to match STR(?s) collation.
    let iri_key = |s: &str| -> String {
        s.strip_prefix('<').unwrap_or(s).strip_suffix('>').unwrap_or(s).to_owned()
    };
    let mut expected = subjects.clone();
    expected.sort_by_key(|s| iri_key(s));
    assert_eq!(
        subjects, expected,
        "ORDER BY EXISTS under restricted view (>= PAR_THRESHOLD rows) must sort by \
             STR(?s) — worker view reinstall gap lets EXISTS return true on workers, \
             collapsing all sort keys to a constant and exposing the hidden named graph"
    );
}

/// ORDER BY with a custom extension function — >= PAR_THRESHOLD rows.
///
/// Without the fix, rayon workers have no FunctionRegistry installed:
///   `functions::lookup("http://test/noop")` returns `None`
///   → `eval_function` returns `Err("unsupported SPARQL function: …")`
///   → `order_bindings` propagates the `Err`
///   → the query fails with a hard error.
///
/// With the fix, workers get the snapshotted registry:
///   → lookup succeeds → sort key is the custom function's output → query succeeds.
///
/// NON-VACUITY CONFIRMED: stashing the fix produces:
///   `Err("unsupported SPARQL function: Custom(http://test/noop)")` → `.expect()` panics
///   → RED.
#[test]
fn order_by_custom_fn_parallel_no_spurious_registry_error() {
    let n = PAR_THRESHOLD + 100;
    let mut nq = String::with_capacity(n * 60);
    for i in 0..n {
        nq.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
    }
    let g = sparq_core::Graph::load_dataset(&nq, "nquads").unwrap();

    // A simple custom identity function — any custom IRI triggers the registry lookup.
    let mut reg = crate::FunctionRegistry::new();
    reg.register("http://test/noop", |args: &[oxrdf::Term]| {
        Ok(args[0].clone())
    });

    // ORDER BY <custom>(?s): forces workers to call eval_function → registry lookup.
    let query = "SELECT ?s WHERE { ?s <http://ex/p> ?o } ORDER BY <http://test/noop>(?s)";

    let result = crate::query_with_functions_and_budget(
        &g,
        query,
        &reg,
        &crate::QueryBudget::unlimited(),
    )
    .expect(
        "ORDER BY custom-function over >= PAR_THRESHOLD rows must not fail \
             (worker FunctionRegistry reinstall missing?)"
    );

    assert_eq!(
        result.len(),
        n,
        "all {n} rows must survive ORDER BY with custom function on parallel dataset"
    );
}
