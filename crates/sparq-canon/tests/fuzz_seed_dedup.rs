//! RDFC-1.0 dataset **set semantics** on the committed `canonicalize_nquads` fuzz
//! reproducer (sq-40apk, CI run 29152138161). [OPUS-5]
//!
//! An RDF dataset is a SET of quads, so canonicalization collapses a repeated input
//! line. The fuzz target's round-trip invariant therefore compares DISTINCT quad
//! counts, not raw ones — a raw `input_quads == canon_quads` assert fires on any
//! N-Quads document with a duplicate (here: 5 parsed -> 4 distinct -> 4 canonical).
//!
//! The seed itself is replayed by the `fuzz` lane (`cargo fuzz run … seeds/<target>`
//! in `.github/workflows/fuzz.yml`), and `fuzz/Cargo.toml` sets
//! `debug-assertions = true`, so that replay does exercise the target's
//! `debug_assert`. But that lane needs a nightly toolchain and its randomized/replay
//! legs are push- and schedule-driven, so this test additionally pins the SEMANTICS
//! in the stable workspace `cargo test` gate. Reading the seed via `include_str!`
//! keeps one source of truth and makes deleting the reproducer a build error; a
//! workspace test reaching into `fuzz/seeds` is existing practice
//! (`sparq-conformance`'s `differential_fuzz_seeds_nquads_corpus` sweeps the same
//! directory at runtime).

use std::collections::HashSet;

/// The exact CI reproducer from run 29152138161 (crash-24694f1d7701250710a0fec…).
const REPRODUCER: &str =
    include_str!("../../../fuzz/seeds/canonicalize_nquads/24694f1d7701250710a0fec7432dc1b8c562ee99");

fn distinct<T: ToString>(quads: &[T]) -> HashSet<String> {
    quads.iter().map(ToString::to_string).collect()
}

#[test]
fn duplicate_quad_reproducer_preserves_distinct_count() {
    let input_quads = sparq_canon::parse_nquads(REPRODUCER).expect("reproducer must parse");
    let distinct_input = distinct(&input_quads);

    assert_eq!(
        input_quads.len(),
        5,
        "reproducer must retain its duplicate on parse"
    );
    assert_eq!(
        distinct_input.len(),
        4,
        "one repeated quad must collapse under dataset set semantics"
    );

    let canonical =
        sparq_canon::canonicalize_nquads(REPRODUCER).expect("reproducer must canonicalize");
    let canonical_quads =
        sparq_canon::parse_nquads(&canonical).expect("canonical output must parse");

    // The RAW canonical count equals the DISTINCT input count: canonicalization
    // deduplicated exactly the one repeated quad and dropped nothing else.
    assert_eq!(
        canonical_quads.len(),
        distinct_input.len(),
        "canonicalization must preserve the deduplicated quad count"
    );
    // Canonical blank-node relabelling is injective, so distinct quads stay distinct
    // — the invariant the fuzz target asserts.
    assert_eq!(
        distinct(&canonical_quads).len(),
        distinct_input.len(),
        "distinct quad count changed after canonicalization"
    );
}
