//! sq-tonhr.6 COEXISTENCE differential — the generated parser vs
//! `sparq-shacl`'s hand-rolled standard-SCS parser (`scs` feature) on every
//! fixture BOTH accept: identical triple sets up to bnode relabelling.
//!
//! Scope is the standard `valid/` corpus — the hand-rolled parser covers the
//! W3C CG surface only (no rdf12 layer, no extensions), so those fixtures
//! are exactly the shared language. A fixture the hand-rolled parser
//! REJECTS is skipped loudly (reported in the assertion message if the
//! count drifts), never silently.

mod common;

use common::{dump, fixture_names, is_isomorphic, read_fixture};
use sparq_shaclc::{parse_strict, DEFAULT_BASE};

/// Root-caused RESULT divergences in the hand-rolled parser, each with a
/// tracking bead. An entry here asserts the divergence STILL EXISTS, so
/// fixing sparq-shacl turns this test red until the entry is deleted —
/// the list can only shrink, never rot.
const KNOWN_DIVERGENCES: &[(&str, &str)] = &[];

#[test]
fn generated_parser_matches_the_hand_rolled_scs_parser_on_the_shared_corpus() {
    let names = fixture_names("valid");
    let mut compared = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for name in &names {
        let doc = read_fixture("valid", name, "shaclc");
        let generated = parse_strict(&doc, DEFAULT_BASE)
            .unwrap_or_else(|e| panic!("valid/{name}: generated parser rejected: {e}"))
            .0;
        // The hand-rolled parser's coverage is a subset; divergence on
        // ACCEPTANCE is allowed (tracked below), divergence on RESULT is not.
        let hand_rolled = match sparq_shacl::scs::parse(&doc, DEFAULT_BASE) {
            Ok(triples) => triples,
            Err(e) => {
                skipped.push(format!("{name} ({e})"));
                continue;
            }
        };
        if let Some((_, why)) = KNOWN_DIVERGENCES.iter().find(|(n, _)| n == name) {
            assert!(
                !is_isomorphic(&generated, &hand_rolled),
                "valid/{name}: recorded divergence no longer reproduces — the sparq-shacl bug \
                 was fixed; DELETE this KNOWN_DIVERGENCES entry ({why})"
            );
            continue;
        }
        assert!(
            is_isomorphic(&generated, &hand_rolled),
            "valid/{name}: generated vs hand-rolled scs triple sets differ\n--- generated ---\n{}\n--- hand-rolled ---\n{}",
            dump(&generated),
            dump(&hand_rolled)
        );
        compared += 1;
    }
    // Ratchet: the shared corpus must stay non-trivial. If the hand-rolled
    // parser starts rejecting more fixtures, this fails loudly with the list
    // instead of the differential silently shrinking.
    assert!(
        compared >= 40,
        "differential shrank: only {compared}/{} fixtures compared; hand-rolled rejected: {skipped:?}",
        names.len()
    );
}
