//! (sq-bif.12) End-to-end differential coverage for the opt-in `vectorized`
//! (columnar / vector-at-a-time) execution path.
//!
//! The `chunk` module's in-`src` unit tests already cover the columnar PRIMITIVES in
//! isolation (`from_rows`/`to_rows` round-trip, ragged rejection, `select_numeric`,
//! `apply_selection`, the `NaN`-never-passes rule). What was missing — and what this
//! file adds — is an END-TO-END check that drives the columnar kernels off a column
//! produced by *real SPARQL execution* over a `Graph` and proves the survivors are
//! **multiset-equal** to the rows a row-at-a-time scalar SPARQL `FILTER` keeps over the
//! same data. That differential is the load-bearing invariant the M4 vectorisation
//! roadmap rests on (`research/optimization-techniques.md`, bead `sq-hvfe`): a vector
//! path may only be wired into the evaluator if it returns the *same* result the scalar
//! path does.
//!
//! Only built under the opt-in `vectorized` feature; the whole file is empty otherwise
//! so the default `cargo test` is unaffected (and the `vectorized` matrix leg in
//! `.github/workflows/feature-matrix.yml` is what actually runs it in CI).
#![cfg(feature = "vectorized")]

use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_engine::{query, DataChunk, VecCmp};

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// A graph of `n` `(ex:s{i}, ex:age, i)` integer-literal triples, plus a couple of
/// NON-numeric `ex:name` triples so the scan column can contain cells the numeric
/// kernel must reject (mirroring the row evaluator excluding a non-numeric FILTER
/// operand). Returns the graph.
fn ages_graph(n: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!(
            "<http://ex/s{i}> <http://ex/age> \"{i}\"^^<{XSD_INT}> .\n"
        ));
    }
    // Two non-numeric objects under a different predicate (never selected by an
    // integer FILTER, but they DO live in the dictionary so a column built over them
    // exercises the kernel's `numeric_value(...) == None` reject path).
    nt.push_str("<http://ex/p1> <http://ex/name> \"alice\" .\n");
    nt.push_str("<http://ex/p2> <http://ex/name> \"bob\" .\n");
    Graph::load_str(&nt, "ntriples").unwrap()
}

const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

/// The dictionary id of every `?o` bound by `query`, in row order. The query MUST
/// project exactly one variable. Panics if a bound term is not in the dictionary
/// (impossible here — every projected term was just read out of the same graph).
fn object_ids(graph: &Graph, q: &str) -> Vec<Id> {
    query(graph, &format!("{PFX}{q}"))
        .unwrap()
        .rows
        .iter()
        .map(|row| {
            let term = row[0].as_ref().expect("projected ?o is bound");
            graph.id_of(term).expect("a queried term is interned")
        })
        .collect()
}

/// A multiset (sorted vec) helper so we compare result *bags*, not order — the
/// columnar path and the scalar FILTER path need not enumerate in the same order.
fn bag(mut ids: Vec<Id>) -> Vec<Id> {
    ids.sort_unstable();
    ids
}

/// CORE DIFFERENTIAL: for a range of comparisons, the columnar
/// `select_numeric` → `apply_selection` survivors equal (as a multiset) the object
/// ids a scalar SPARQL `FILTER(?o <cmp> t)` returns over the same column. This drives
/// the gather/selection kernels off a real-execution column AND ties them to the
/// engine's actual scalar FILTER semantics.
#[test]
fn columnar_filter_multiset_equals_scalar_sparql() {
    let n = 200;
    let g = ages_graph(n);

    // The full numeric column, exactly as the scalar engine sees it (row order of an
    // unfiltered scan). This is the input batch the vector path would receive.
    let all_ids = object_ids(&g, "SELECT ?o WHERE { ?s ex:age ?o }");
    assert_eq!(all_ids.len(), n, "every age triple is scanned");
    let chunk = DataChunk::from_columns(vec![all_ids.clone()], n).unwrap();

    // (VecCmp kernel, equivalent SPARQL FILTER tail) pairs.
    let cases: &[(VecCmp, &str)] = &[
        (VecCmp::Gt(30.0), "FILTER(?o > 30)"),
        (VecCmp::Ge(30.0), "FILTER(?o >= 30)"),
        (VecCmp::Lt(10.0), "FILTER(?o < 10)"),
        (VecCmp::Le(10.0), "FILTER(?o <= 10)"),
        (VecCmp::Eq(42.0), "FILTER(?o = 42)"),
        // Boundary: nothing passes (threshold above the max value).
        (VecCmp::Gt((n as f64) + 5.0), "FILTER(?o > 205)"),
        // Boundary: everything passes (threshold below the min value).
        (VecCmp::Ge(0.0), "FILTER(?o >= 0)"),
    ];

    for (cmp, filter) in cases {
        // Vector path: comparison kernel produces a selection vector, gather compacts.
        let sel = chunk.select_numeric(&g, 0, *cmp);
        let gathered = chunk.apply_selection(&sel);
        let vec_ids: Vec<Id> = gathered.column(0).unwrap().to_vec();

        // Scalar path: the engine's own row-at-a-time FILTER over the same triples.
        let scalar_ids = object_ids(&g, &format!("SELECT ?o WHERE {{ ?s ex:age ?o {filter} }}"));

        assert_eq!(
            bag(vec_ids.clone()),
            bag(scalar_ids.clone()),
            "columnar {cmp:?} survivors must be the SAME bag as scalar `{filter}` \
             (vec={} scalar={})",
            vec_ids.len(),
            scalar_ids.len()
        );
        // The selection-vector length must equal the gathered row count (gather is the
        // faithful compaction of the selection, not a re-filter).
        assert_eq!(sel.len(), gathered.len());
        assert_eq!(gathered.len(), scalar_ids.len());
    }
}

/// The kernel rejects NON-numeric cells exactly as the scalar FILTER does: a column
/// containing the two `ex:name` string literals yields ZERO survivors for any numeric
/// comparison, matching a scalar `FILTER(?o > -1)` over those same string objects
/// (which is a type error per row → row excluded → empty result).
#[test]
fn non_numeric_column_cells_are_rejected_like_scalar() {
    let g = ages_graph(8);
    let name_ids = object_ids(&g, "SELECT ?o WHERE { ?s ex:name ?o }");
    assert_eq!(name_ids.len(), 2, "two string-literal objects");
    let chunk = DataChunk::from_columns(vec![name_ids], 2).unwrap();

    for cmp in [
        VecCmp::Gt(-1.0),
        VecCmp::Ge(-1.0),
        VecCmp::Lt(1e9),
        VecCmp::Le(1e9),
        VecCmp::Eq(0.0),
    ] {
        let sel = chunk.select_numeric(&g, 0, cmp);
        assert!(
            sel.is_empty(),
            "{cmp:?} must reject every non-numeric cell (kernel)"
        );
    }

    // Scalar reference: a numeric comparison over the string objects returns nothing.
    let scalar = object_ids(&g, "SELECT ?o WHERE { ?s ex:name ?o FILTER(?o > -1) }");
    assert!(scalar.is_empty(), "scalar FILTER also excludes string objects");
}

/// A two-column chunk (subject id + age id) gathered by a numeric selection on the
/// AGE column keeps the matching SUBJECT ids aligned — the gather is row-preserving
/// across all columns, so the surviving `(subject, age)` pairs are exactly the pairs a
/// scalar `SELECT ?s ?o WHERE { ?s ex:age ?o FILTER(?o >= t) }` binds. Proves the
/// selection kernel does not desynchronise columns.
#[test]
fn gather_keeps_columns_row_aligned_vs_scalar_pairs() {
    let n = 50;
    let g = ages_graph(n);
    let pfx = PFX;

    // Build a (subject, age) two-column batch from one ordered scan so the columns are
    // row-aligned by construction.
    let rows = query(&g, &format!("{pfx}SELECT ?s ?o WHERE {{ ?s ex:age ?o }}"))
        .unwrap()
        .rows;
    let subj: Vec<Id> = rows
        .iter()
        .map(|r| g.id_of(r[0].as_ref().unwrap()).unwrap())
        .collect();
    let age: Vec<Id> = rows
        .iter()
        .map(|r| g.id_of(r[1].as_ref().unwrap()).unwrap())
        .collect();
    let chunk = DataChunk::from_columns(vec![subj, age], rows.len()).unwrap();

    // Filter on the AGE column (index 1), keep both columns.
    let t = 25.0;
    let sel = chunk.select_numeric(&g, 1, VecCmp::Ge(t));
    let gathered = chunk.apply_selection(&sel);

    // The surviving (subject, age) id pairs from the vector path.
    let mut vec_pairs: Vec<(Id, Id)> = gathered
        .column(0)
        .unwrap()
        .iter()
        .copied()
        .zip(gathered.column(1).unwrap().iter().copied())
        .collect();
    vec_pairs.sort_unstable();

    // The scalar engine's (subject, age) id pairs for the same FILTER.
    let mut scalar_pairs: Vec<(Id, Id)> = query(
        &g,
        &format!("{pfx}SELECT ?s ?o WHERE {{ ?s ex:age ?o FILTER(?o >= {t}) }}"),
    )
    .unwrap()
    .rows
    .iter()
    .map(|r| {
        (
            g.id_of(r[0].as_ref().unwrap()).unwrap(),
            g.id_of(r[1].as_ref().unwrap()).unwrap(),
        )
    })
    .collect();
    scalar_pairs.sort_unstable();

    assert_eq!(
        vec_pairs, scalar_pairs,
        "gathered (subject, age) pairs must match the scalar FILTER's pairs exactly"
    );
    assert!(!vec_pairs.is_empty(), "the FILTER must keep some rows (non-vacuous)");
}
