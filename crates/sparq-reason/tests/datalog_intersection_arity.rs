//! [OPUS-5] sq-3xsx0 — `cls-int1` in the bench Datalog encodings must generalise to ANY
//! `owl:intersectionOf` list arity, not just the 1–2 the LUBM TBox happens to use.
//!
//! The competitor-comparison encodings in `bench/reason-encodings/` (Nemo `.rls`, VLog
//! `.dlog`) exist to reproduce sparq's OWL-RL closure set-for-set on a general Datalog
//! engine. sparq's own `cls-int1` decodes the WHOLE intersection list and requires
//! membership in every conjunct (`crates/sparq-reason/src/owl.rs`, `members.iter().all(…)`),
//! but the encodings used to spell the rule out per list length for lengths 1 and 2 — so on
//! any corpus with a 3+-conjunct intersection they would have silently UNDER-derived
//! relative to sparq. The encodings now use a recursive list walk instead.
//!
//! This file pins that change from both ends:
//!
//! 1. **Semantics** — the generic rule shape is run through a real semi-naive Datalog
//!    fixpoint (this crate's own `datalog` evaluator) over intersections of arity 1, 2, 3
//!    and 4, asserting the EXACT derived membership set. The previous arity-capped shape is
//!    run over the same facts and must MISS exactly the arity-3/4 memberships while agreeing
//!    on arity 1–2 — which is simultaneously the bug demonstration and the argument that the
//!    pinned LUBM closure counts (whose lists are all length 1–2) are unchanged.
//! 2. **Artifact** — the two shipped encodings must actually carry the recursive walk and no
//!    arity-capped decode, so reverting either file turns this test red. Neither engine is a
//!    cargo dependency (both are gather-only builds), so the closure cross-check in
//!    `scripts/bench/materialize-same-box.sh` only runs on a box that has them; at PR time
//!    this is the only guard those files have.
//!
//! 🤖 SPARQ agent.

use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};
use sparq_reason::datalog::{eval, parse_program};
use std::path::PathBuf;

const EX: &str = "http://ex/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// An id-level triple, the shape both `datalog::eval`'s input facts and its closure use.
type Triple = [Id; 3];
/// An individual paired with the intersection classes it must be derived to belong to.
type Membership = (Id, Vec<Id>);

/// Prefixes + `scm-int`'s list walk (`intTail`), shared by both rule shapes: the encodings
/// derive the guard relation once and both `cls-int1` forms sit downstream of it.
const PREAMBLE: &str = r#"
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://ex/> .

[?c, ex:intTail, ?l] :- [?c, owl:intersectionOf, ?l] .
[?c, ex:intTail, ?r] :- [?c, ex:intTail, ?l], [?l, rdf:rest, ?r] .
"#;

/// The GENERIC rule shape now shipped in `nemo/owl-rl.rls` + `vlog/owl-rl.dlog`:
/// `intAll(l, x)` = "x is typed with every conjunct from list cell `l` onward".
const GENERIC: &str = r#"
[?l, ex:intAll, ?x] :- [?c, ex:intTail, ?l], [?l, rdf:first, ?k], [?l, rdf:rest, rdf:nil], [?x, a, ?k] .
[?l, ex:intAll, ?x] :- [?c, ex:intTail, ?l], [?l, rdf:first, ?k], [?l, rdf:rest, ?r], [?r, ex:intAll, ?x], [?x, a, ?k] .
[?x, a, ?c] :- [?c, owl:intersectionOf, ?l], [?l, ex:intAll, ?x] .
"#;

/// The SUPERSEDED shape: one rule per list length, spelled out for lengths 1 and 2. (The
/// encodings bound the conjuncts in an n-ary helper relation; this dialect is triple-only,
/// so the length-2 conjuncts are keyed on the list head instead — the arity CAP, which is
/// what this shape exists to demonstrate, is identical either way.)
const ARITY_CAPPED: &str = r#"
[?l, ex:int1a, ?k] :- [?c, ex:intTail, ?l], [?l, rdf:first, ?k], [?l, rdf:rest, rdf:nil] .
[?l, ex:int2a, ?k] :- [?c, ex:intTail, ?l], [?l, rdf:first, ?k], [?l, rdf:rest, ?r], [?r, rdf:rest, rdf:nil] .
[?l, ex:int2b, ?k] :- [?c, ex:intTail, ?l], [?l, rdf:rest, ?r], [?r, rdf:first, ?k], [?r, rdf:rest, rdf:nil] .
[?x, a, ?c] :- [?c, owl:intersectionOf, ?l], [?l, ex:int1a, ?k], [?x, a, ?k] .
[?x, a, ?c] :- [?c, owl:intersectionOf, ?l], [?l, ex:int2a, ?k1], [?l, ex:int2b, ?k2], [?x, a, ?k1], [?x, a, ?k2] .
"#;

fn iri(d: &mut Dict, local: &str) -> Id {
    d.intern_iri(&format!("{}{}", EX, local))
}

/// The fixture: intersection classes of arity 1..=4 over conjuncts `A B C D`, and four
/// individuals typed with a prefix of `A B C D` of length 1..=4.
///
/// Returns the facts plus the interned ids used by the assertions.
fn fixture(d: &mut Dict) -> (Vec<Triple>, Vec<Membership>) {
    let ty = d.intern_iri(RDF_TYPE);
    let inter = d.intern_iri("http://www.w3.org/2002/07/owl#intersectionOf");
    let first = d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#first");
    let rest = d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#rest");
    let nil = d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil");
    let conjuncts: Vec<Id> = ["A", "B", "C", "D"].iter().map(|c| iri(d, c)).collect();

    let mut facts = Vec::new();
    // `I<n> owl:intersectionOf ( A … )`, an n-cell RDF list per n.
    let mut classes = Vec::new();
    for n in 1..=4usize {
        let c = iri(d, &format!("I{}", n));
        let cells: Vec<Id> = (0..n).map(|i| iri(d, &format!("l{}_{}", n, i))).collect();
        facts.push([c, inter, cells[0]]);
        for (i, &cell) in cells.iter().enumerate() {
            facts.push([cell, first, conjuncts[i]]);
            facts.push([cell, rest, if i + 1 < n { cells[i + 1] } else { nil }]);
        }
        classes.push(c);
    }
    // `x<n>` is typed with the first n conjuncts, so it belongs to exactly I1..=In.
    let mut expected = Vec::new();
    for n in 1..=4usize {
        let x = iri(d, &format!("x{}", n));
        for &k in &conjuncts[..n] {
            facts.push([x, ty, k]);
        }
        expected.push((x, classes[..n].to_vec()));
    }
    (facts, expected)
}

/// Derived `rdf:type` facts whose object is one of the intersection classes — i.e. exactly
/// the `cls-int1` memberships, with the fixture's own type assertions and the helper
/// relations filtered out.
fn memberships(d: &mut Dict, facts: &[Triple], src: &str) -> FxHashSet<Triple> {
    let ty = d.intern_iri(RDF_TYPE);
    let classes: FxHashSet<Id> = (1..=4usize).map(|n| iri(d, &format!("I{}", n))).collect();
    let program = parse_program(d, src).expect("program parses");
    let closure = eval(d, facts, &program).expect("program is stratifiable");
    let inputs: FxHashSet<Triple> = facts.iter().copied().collect();
    closure
        .into_iter()
        .filter(|f| !inputs.contains(f) && f[1] == ty && classes.contains(&f[2]))
        .collect()
}

#[test]
fn generic_cls_int1_covers_every_list_arity() {
    let mut d = Dict::new();
    let (facts, expected) = fixture(&mut d);
    let want: FxHashSet<Triple> = {
        let ty = d.intern_iri(RDF_TYPE);
        expected
            .iter()
            .flat_map(|(x, cs)| cs.iter().map(move |&c| [*x, ty, c]))
            .collect()
    };
    // 1 + 2 + 3 + 4 memberships: `x<n>` is an instance of I1..=In and nothing else.
    assert_eq!(want.len(), 10);

    let got = memberships(&mut d, &facts, &format!("{}{}", PREAMBLE, GENERIC));
    assert_eq!(
        got, want,
        "the recursive intAll walk must derive EXACTLY the memberships of every arity — \
         no misses at arity 3/4, no over-derivation at arity 1/2"
    );
}

#[test]
fn arity_capped_cls_int1_misses_three_plus_conjunct_intersections() {
    let mut d = Dict::new();
    let (facts, _) = fixture(&mut d);
    let capped = memberships(&mut d, &facts, &format!("{}{}", PREAMBLE, ARITY_CAPPED));
    let generic = memberships(&mut d, &facts, &format!("{}{}", PREAMBLE, GENERIC));

    // The bug the generalisation fixes: a 3+-conjunct intersection derives NOTHING.
    let ty = d.intern_iri(RDF_TYPE);
    let (i3, i4) = (iri(&mut d, "I3"), iri(&mut d, "I4"));
    let missed: FxHashSet<Triple> = generic.difference(&capped).copied().collect();
    assert!(
        !missed.is_empty() && missed.iter().all(|f| f[2] == i3 || f[2] == i4),
        "the arity-capped decode must miss the arity-3/4 memberships and only those; \
         missed = {:?}",
        missed
    );
    assert!(
        !capped.contains(&[iri(&mut d, "x4"), ty, i4]),
        "x4 is typed with all four conjuncts of I4, yet the capped decode has no rule for a \
         4-cell list — this is the under-derivation vs sparq's whole-list decode"
    );

    // ...and the argument that the pinned LUBM closure counts are unaffected: on the list
    // lengths LUBM actually uses (1 and 2), the two shapes agree fact-for-fact.
    let (i1, i2) = (iri(&mut d, "I1"), iri(&mut d, "I2"));
    let short = |s: &FxHashSet<Triple>| -> FxHashSet<Triple> {
        s.iter()
            .copied()
            .filter(|f| f[2] == i1 || f[2] == i2)
            .collect()
    };
    assert_eq!(
        short(&capped),
        short(&generic),
        "on arity-1/2 lists the recursive walk must derive exactly what the explicit decode \
         did — otherwise the encodings' pinned LUBM closure count would move"
    );
    assert_eq!(capped, short(&capped), "the capped shape derives nothing else");
}

#[test]
fn shipped_encodings_use_the_generic_list_walk() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/reason-encodings");
    // (file, the recursive step rule's marker, the superseded arity-capped decode's marker)
    let cases = [
        ("nemo/owl-rl.rls", "intAll(?r, ?x)", ["int1def(", "int2def("]),
        ("vlog/owl-rl.dlog", "IntAll(R,X)", ["Int1(C,A)", "Int2(C,A,B)"]),
    ];
    for (rel, recursive_step, capped) in cases {
        let path = root.join(rel);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        // Only rule lines: the header prose names the superseded shape on purpose.
        let rules: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with(['%', '#']))
            .collect();
        assert!(
            rules.contains(recursive_step),
            "{}: cls-int1 must be the recursive list walk (`{}` in a rule body), so it \
             generalises past the arity-1/2 lists LUBM happens to use",
            rel,
            recursive_step
        );
        for marker in capped {
            assert!(
                !rules.contains(marker),
                "{}: the arity-capped `{}` decode is superseded by the recursive walk",
                rel,
                marker
            );
        }
    }
}
