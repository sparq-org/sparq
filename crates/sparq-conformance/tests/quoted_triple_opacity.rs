//! [KERN] RDF 1.2 quoted-triple OPACITY conformance — "quoting never asserts"
//! and closure non-interference, pinned against the REAL reasoning profiles.
//!
//! ## What is pinned
//!
//! RDF 1.2 makes a quoted/reified triple a TERM: `:r rdf:reifies <<( :s :p :o )>>`
//! asserts one triple about `:r` and NOTHING about `:s :p :o` (triple terms are
//! referentially opaque — RDF 1.2 Semantics, triple terms; RDF 1.2 Concepts,
//! "Reifying triples"). sparq-core already stores triple terms structurally
//! (`Dict::intern_triple_ids`; `ttl.rs` / `nt.rs` desugar the `<< … >>` and
//! `{| … |}` forms exactly like oxttl). This lane pins the REASONER side of that
//! contract, per profile (RDFS + OWL 2 RL via `sparq_reason::materialize`;
//! OWL 2 EL classification behind the `el-suite` feature):
//!
//! 1. **Quoting never asserts** — a graph with a reified triple `<< s p o >>`
//!    (any surface form: explicit `rdf:reifies` + triple term, reified-triple
//!    term as subject/object, fresh or named reifier) does NOT entail `s p o`,
//!    nor any consequence of it (domain/range typings, sub-property fan-out),
//!    unless `s p o` is separately asserted. The ANNOTATION form
//!    `s p o {| … |}` — which per RDF 1.2 DOES assert `s p o` — is the fixture's
//!    positive control proving the guarded rules genuinely fire when a triple
//!    IS asserted.
//! 2. **Closure non-interference** — the closure of a base graph is
//!    BYTE-IDENTICAL whether or not triple terms referring to it (a true one,
//!    a false one, an entailed-but-unasserted one, a reversed one) are present:
//!    removing the overlay-only closure delta from `closure(base ∪ overlay)`
//!    yields `closure(base)`, with
//!    `closure(base)` additionally byte-pinned against a committed
//!    expected-answer file (`tests/quoted_opacity/expected/*.nt`, sorted
//!    N-Triples — hand-verified, see `quoted_opacity/README.md`).
//! 3. **Reifier annotations reason normally** — a reifier's own annotations
//!    (`:statedBy` under an `rdfs:domain`, a subclass hop) participate in the
//!    closure like any other triples, WITHOUT leaking the quoted content.
//!
//! ## The ratchet
//!
//! `QUOTED_OPACITY_FLOOR` is the MEASURED count of opacity assertions the
//! battery makes (the `quoted-triple opacity assertions N (floor F)` line the
//! runner prints); it may only RISE. It is MIRRORED in the central scoreboard
//! (`scoreboard::SUITES`) and read TEXTUALLY by `tests/scoreboard_floors.rs`.
//! The lane is UNGATED (plain `sparq_reason::materialize`, default features,
//! no fetched data) so it runs in ordinary `cargo test --workspace`; the EL arm
//! alone is behind the existing opt-in `el-suite` feature (it links the
//! `sparq-reason-el` classifier) and does NOT count toward the floor, so the
//! measured count is feature-independent.

use std::collections::BTreeSet;

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason::{materialize, Profile};

/// The quoted-triple opacity assertion floor (the RATCHET). The ACTUAL measured
/// count of assertions the UNGATED battery below makes — every `must` /
/// `must_not` / byte-identity check, per profile (RDFS + OWL 2 RL). MIRRORED in
/// the central scoreboard (`scoreboard::SUITES`) and read TEXTUALLY by
/// `tests/scoreboard_floors.rs`. It may only RISE. [KERN]
pub const QUOTED_OPACITY_FLOOR: usize = 84;

const NS: &str = "urn:sparq:qto:";
const RDF_TYPE: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
const RDF_REIFIES: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies>";
// Used only by the feature-gated EL arm below; cfg-gated to match so the
// default-feature build stays dead-code-clean (clippy gates -D warnings).
#[cfg(feature = "el-suite")]
const RDFS_SUBCLASS_OF: &str = "<http://www.w3.org/2000/01/rdf-schema#subClassOf>";

const NEVER_ASSERTS: &str = include_str!("quoted_opacity/never_asserts.ttl");
const BASE: &str = include_str!("quoted_opacity/base.ttl");
const QUOTED_OVERLAY: &str = include_str!("quoted_opacity/quoted_overlay.ttl");
const ANNOTATED_REIFIER: &str = include_str!("quoted_opacity/annotated_reifier.ttl");
const EXPECTED_BASE_RDFS: &str = include_str!("quoted_opacity/expected/base_rdfs_closure.nt");
const EXPECTED_BASE_OWLRL: &str = include_str!("quoted_opacity/expected/base_owlrl_closure.nt");

/// A tally of opacity assertions made — the quantity ratcheted. Bumped only
/// through `must` / `must_not` / `must_reify` / `byte_identical`, so the count
/// is exactly the number of real checks performed (no inflation).
#[derive(Default)]
struct Tally {
    n: usize,
}

impl Tally {
    fn must(&mut self, closure: &BTreeSet<String>, line: &str, ctx: &str) {
        assert!(
            closure.contains(line),
            "[quoted-opacity {ctx}] required triple missing from the closure: {line}"
        );
        self.n += 1;
    }

    fn must_not(&mut self, closure: &BTreeSet<String>, line: &str, ctx: &str) {
        assert!(
            !closure.contains(line),
            "[quoted-opacity {ctx}] OPACITY VIOLATION — quoted content leaked into the \
             closure: {line}"
        );
        self.n += 1;
    }

    /// The closure contains SOME triple with the given subject + predicate —
    /// used where the object is a structurally-interned triple term or a fresh
    /// blank, whose exact rendering this lane deliberately does not pin.
    fn must_edge(&mut self, closure: &BTreeSet<String>, s: &str, p: &str, ctx: &str) {
        let prefix = format!("{s} {p} ");
        assert!(
            closure.iter().any(|l| l.starts_with(&prefix)),
            "[quoted-opacity {ctx}] required triple missing from the closure: {prefix}…"
        );
        self.n += 1;
    }

    /// Byte-identity of two sorted-line closures (rendered as one document, so
    /// a mismatch shows the full diff context).
    fn byte_identical(&mut self, actual: &BTreeSet<String>, expected: &str, ctx: &str) {
        let actual_doc = to_doc(actual);
        assert_eq!(
            actual_doc, expected,
            "[quoted-opacity {ctx}] closure is not byte-identical"
        );
        self.n += 1;
    }
}

/// Parse a Turtle 1.2 fixture through the REAL default ingest path.
fn parse(src: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(src, "ttl").expect("fixture parses")
}

/// Render interned triples as canonical sorted N-Triples-style lines. Dict ids
/// are run-local; the string rendering is the stable comparison currency.
fn lines_of(dict: &Dict, triples: &[[Id; 3]]) -> BTreeSet<String> {
    triples
        .iter()
        .map(|[s, p, o]| format!("{} {} {} .", dict.term(*s), dict.term(*p), dict.term(*o)))
        .collect()
}

/// The full materialized closure of a fixture under `profile`, as sorted lines.
fn closure_lines(profile: Profile, src: &str) -> BTreeSet<String> {
    let (mut dict, mut triples) = parse(src);
    materialize(profile, &mut dict, &mut triples);
    lines_of(&dict, &triples)
}

/// The PARSED (un-reasoned) triples of a fixture, as sorted lines.
fn parsed_lines(src: &str) -> BTreeSet<String> {
    let (dict, triples) = parse(src);
    lines_of(&dict, &triples)
}

fn to_doc(lines: &BTreeSet<String>) -> String {
    let mut doc: String = lines.iter().map(|l| format!("{l}\n")).collect();
    if doc.is_empty() {
        doc.push('\n');
    }
    doc
}

fn iri(local: &str) -> String {
    format!("<{NS}{local}>")
}

fn spo(s: &str, p: &str, o: &str) -> String {
    format!("{s} {p} {o} .")
}

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Rdfs => "rdfs",
        Profile::OwlRl => "owl-rl",
        #[allow(unreachable_patterns)] // `d-entail` adds Profile::D — not exercised here
        _ => "other",
    }
}

/// (1) Quoting never asserts: none of the three quoted surface forms entails
/// the reified triple or any consequence of it; the annotation-syntax control
/// (which RDF 1.2 DOES assert) fires every guarded rule.
fn quoting_never_asserts(tally: &mut Tally, profile: Profile) {
    let ctx = format!("never-asserts/{}", profile_name(profile));
    let closure = closure_lines(profile, NEVER_ASSERTS);

    // The reification triples themselves are present and inert.
    tally.must_edge(&closure, &iri("r1"), RDF_REIFIES, &ctx);
    tally.must_edge(&closure, &iri("r2"), RDF_REIFIES, &ctx);
    tally.must(
        &closure,
        &spo(&iri("r2"), &iri("statedBy"), &iri("carol")),
        &ctx,
    );
    tally.must_edge(&closure, &iri("carol"), &iri("claims"), &ctx);

    // POSITIVE CONTROL — the annotation form asserts its base triple, so every
    // guarded rule fires on :frank / :initech.
    tally.must(
        &closure,
        &spo(&iri("frank"), &iri("worksFor"), &iri("initech")),
        &ctx,
    );
    tally.must(
        &closure,
        &spo(&iri("frank"), RDF_TYPE, &iri("Employee")),
        &ctx,
    );
    tally.must(
        &closure,
        &spo(&iri("frank"), RDF_TYPE, &iri("Person")),
        &ctx,
    );
    tally.must(
        &closure,
        &spo(&iri("initech"), RDF_TYPE, &iri("Company")),
        &ctx,
    );
    tally.must(
        &closure,
        &spo(&iri("frank"), &iri("affiliatedWith"), &iri("initech")),
        &ctx,
    );

    // The triples represented by the triple terms are NOT asserted…
    for who in ["alice", "bob", "eve"] {
        tally.must_not(
            &closure,
            &spo(&iri(who), &iri("worksFor"), &iri("acme")),
            &ctx,
        );
        // …and none of their consequences exists either: no domain typing,
        tally.must_not(&closure, &spo(&iri(who), RDF_TYPE, &iri("Employee")), &ctx);
        // no subclass hop above it,
        tally.must_not(&closure, &spo(&iri(who), RDF_TYPE, &iri("Person")), &ctx);
        // no sub-property fan-out.
        tally.must_not(
            &closure,
            &spo(&iri(who), &iri("affiliatedWith"), &iri("acme")),
            &ctx,
        );
    }
    // No range typing from any quoted `… :worksFor :acme` either.
    tally.must_not(
        &closure,
        &spo(&iri("acme"), RDF_TYPE, &iri("Company")),
        &ctx,
    );
}

/// (2) Closure non-interference: the base closure is byte-identical with or
/// without the triple-term overlay: removing the overlay-only closure delta
/// from `closure(base ∪ overlay)` yields `closure(base)`, which also matches
/// the committed expected-answer file.
fn closure_non_interference(tally: &mut Tally, profile: Profile) {
    let ctx = format!("non-interference/{}", profile_name(profile));
    let base_closure = closure_lines(profile, BASE);
    let combined_closure = closure_lines(profile, &format!("{BASE}\n{QUOTED_OVERLAY}"));
    let overlay = parsed_lines(QUOTED_OVERLAY);
    let overlay_closure = closure_lines(profile, QUOTED_OVERLAY);
    // Preserve rows shared with the base while accounting for
    // feature-gated reasoning that derives additional rows from the overlay.
    let overlay_delta: BTreeSet<String> =
        overlay_closure.difference(&base_closure).cloned().collect();

    // The committed expected answer pins closure(base) byte-exactly.
    let expected = match profile {
        Profile::Rdfs => EXPECTED_BASE_RDFS,
        _ => EXPECTED_BASE_OWLRL,
    };
    tally.byte_identical(&base_closure, expected, &ctx);

    // Every overlay triple survives the closure verbatim (nothing rewrites or
    // drops the reification edges)…
    for line in &overlay {
        tally.must(&combined_closure, line, &ctx);
    }
    // …and removing everything derived from the overlay alone yields
    // closure(base), byte-identical: the overlay interfered with nothing.
    let stripped: BTreeSet<String> = combined_closure
        .difference(&overlay_delta)
        .cloned()
        .collect();
    tally.byte_identical(&stripped, expected, &ctx);

    // Negative guards spelled out (also implied by the byte-identity above):
    // the false reified triple, its range consequence, the reversed part-of and
    // its transitive consequence never enter the closure.
    tally.must_not(
        &combined_closure,
        &spo(&iri("alice"), &iri("worksFor"), &iri("globex")),
        &ctx,
    );
    tally.must_not(
        &combined_closure,
        &spo(&iri("globex"), RDF_TYPE, &iri("Company")),
        &ctx,
    );
    tally.must_not(
        &combined_closure,
        &spo(&iri("megacorp"), &iri("partOf"), &iri("acme")),
        &ctx,
    );
    tally.must_not(
        &combined_closure,
        &spo(&iri("megacorp"), &iri("partOf"), &iri("megacorp")),
        &ctx,
    );
}

/// (3) A reifier's own annotations are reasoned over normally — domain typing
/// and a subclass hop fire ON THE REIFIER — without leaking the reified triple.
fn annotated_reifier(tally: &mut Tally, profile: Profile) {
    let ctx = format!("annotated-reifier/{}", profile_name(profile));
    let closure = closure_lines(profile, ANNOTATED_REIFIER);

    // Normal reasoning over the reifier node :r5.
    tally.must_edge(&closure, &iri("r5"), RDF_REIFIES, &ctx);
    tally.must(
        &closure,
        &spo(&iri("r5"), &iri("statedBy"), &iri("carol")),
        &ctx,
    );
    tally.must(
        &closure,
        &spo(&iri("r5"), &iri("certainty"), "\"0.9\""),
        &ctx,
    );
    tally.must(&closure, &spo(&iri("r5"), RDF_TYPE, &iri("Claim")), &ctx);
    tally.must(
        &closure,
        &spo(&iri("r5"), RDF_TYPE, &iri("Statement")),
        &ctx,
    );

    // No leak: the reified triple and its domain consequence stay out, and the
    // reifier is never conflated with the quoted subject.
    tally.must_not(
        &closure,
        &spo(&iri("alice"), &iri("worksFor"), &iri("globex")),
        &ctx,
    );
    tally.must_not(
        &closure,
        &spo(&iri("alice"), RDF_TYPE, &iri("Employee")),
        &ctx,
    );
    tally.must_not(&closure, &spo(&iri("r5"), RDF_TYPE, &iri("Employee")), &ctx);
}

#[test]
fn quoted_triple_opacity_ratchet() {
    let mut tally = Tally::default();
    for profile in [Profile::Rdfs, Profile::OwlRl] {
        quoting_never_asserts(&mut tally, profile);
        closure_non_interference(&mut tally, profile);
        annotated_reifier(&mut tally, profile);
    }
    println!(
        "quoted-triple opacity assertions {} (floor {})",
        tally.n, QUOTED_OPACITY_FLOOR
    );
    assert!(
        tally.n >= QUOTED_OPACITY_FLOOR,
        "quoted-triple opacity assertion count regressed: {} < floor {}",
        tally.n,
        QUOTED_OPACITY_FLOOR
    );
}

// ---------------------------------------------------------------------------
// The EL arm — behind the existing opt-in `el-suite` feature (it links the
// `sparq-reason-el` consequence-based classifier). Deliberately OUTSIDE the
// ratcheted tally so `QUOTED_OPACITY_FLOOR` stays feature-independent.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "el-suite"))]
#[test]
fn quoted_opacity_el_arm_skipped_without_feature() {
    eprintln!(
        "SKIP: the quoted-triple opacity EL arm is OFF — build with `--features el-suite` \
         to run it (it links the sparq-reason-el classifier)."
    );
}

/// A quoted `rdfs:subClassOf` AXIOM never enters the EL TBox: the classified
/// subsumption lattice is byte-identical with or without the quoted overlay,
/// and the quoted axiom is never derived.
#[cfg(feature = "el-suite")]
#[test]
fn quoted_axiom_does_not_interfere_with_el_classification() {
    use sparq_reason_el::classify_graph;

    let el_base = include_str!("quoted_opacity/el_base.ttl");
    let el_overlay = include_str!("quoted_opacity/el_overlay.ttl");

    let classified = |src: &str| -> BTreeSet<String> {
        let (mut dict, mut triples) = parse(src);
        let report = classify_graph(&mut dict, &mut triples);
        assert_eq!(
            report.unsatisfiable_classes, 0,
            "EL fixture is satisfiable; classifier reported unsatisfiable classes"
        );
        lines_of(&dict, &triples)
    };

    let base_closure = classified(el_base);
    let combined_closure = classified(&format!("{el_base}\n{el_overlay}"));
    let overlay = parsed_lines(el_overlay);

    // The completed chain is classified in both runs.
    let a_sub_c = spo(&iri("A"), RDFS_SUBCLASS_OF, &iri("C"));
    assert!(
        base_closure.contains(&a_sub_c),
        "EL classifier must complete :A ⊑ :C"
    );
    assert!(
        combined_closure.contains(&a_sub_c),
        "EL classifier must complete :A ⊑ :C"
    );

    // The QUOTED axiom never enters the TBox.
    let c_sub_d = spo(&iri("C"), RDFS_SUBCLASS_OF, &iri("D"));
    assert!(
        !combined_closure.contains(&c_sub_d),
        "OPACITY VIOLATION — the quoted rdfs:subClassOf axiom leaked into the EL TBox"
    );

    // Non-interference, byte-identical: closure(base ∪ overlay) \ overlay ==
    // closure(base).
    let stripped: BTreeSet<String> = combined_closure.difference(&overlay).cloned().collect();
    assert_eq!(
        to_doc(&stripped),
        to_doc(&base_closure),
        "EL classification is not byte-identical once the quoted overlay is removed"
    );
}
