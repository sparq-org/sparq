//! UFO-SN3 — the finite-world UFO expressibility suite, wired into the
//! central conformance scoreboard as a sparq-EXTENSION-shaped ratchet. HONESTLY an
//! extension ratchet over sparq's own reference profile — NOT a UFO / OntoUML /
//! gUFO standards-conformance claim (no normative UFO conformance test suite
//! exists; UFO is a research foundational ontology, gUFO its lightweight OWL
//! implementation).
//!
//! ## What UFO-SN3 is
//!
//! UFO-SN3 is an executable, finite-world projection of representative Unified
//! Foundational Ontology concepts (UFO-A endurants/moments/rigidity/identity,
//! UFO-B events/participation, UFO-C agents/commitments/norms, plus situations,
//! worlds, and accessibility) onto sparq's N3 forward reasoner. The vocabulary,
//! ruleset, and per-case fixtures live in `tests/ufo_sn3/` (see its README.md for
//! scope, the decidable-profile argument, and gUFO attribution). The rules are
//! function-free, range-restricted, monotone, and free of fresh-term generation,
//! so positive materialization reaches a finite fixpoint on finite input.
//!
//! ## The reification-node projection (the honest RDF 1.2 caveat)
//!
//! sparq-core stores and queries RDF 1.2 triple terms, but the N3 engine's term
//! model (`sparq_reason::n3::Term`) has NO triple-term variant — rules cannot
//! pattern-match `<<( ?s ?p ?o )>>`. UFO-SN3 v1 therefore encodes propositions as
//! explicit reification nodes (`ufo:Proposition` with `ufo:subj`/`ufo:pred`/
//! `ufo:obj`), and this suite asserts the projection's load-bearing invariant
//! DIRECTLY: a proposition holding in a situation never asserts its encoded
//! subject-predicate-object triple in the enclosing graph. Native quoted-triple
//! matching in N3 rules is a tracked `sparq-reason` feature gap, never faked here.
//!
//! ## Execution model (the eye_cases pattern)
//!
//! Each case concatenates `cases/<name>/input.n3` with `rules/ufo-sn3.n3`, runs
//! the document through the REAL `sparq_reason::reason_n3` forward closure, and
//! asserts (a) every triple of `cases/<name>/answer.n3` is in the closure
//! (superset entailment, exactly `sparq-reason/tests/eye_cases.rs`), and (b) a
//! per-case set of NEGATIVE-entailment guards — triples that must NOT be derived
//! (open-world absence is never falsity; anti-rigid memberships do not propagate;
//! `ufo:sameContinuant` never becomes `owl:sameAs`; the reification projection
//! never leaks the encoded triple) — term-structure checks, never count-based.
//!
//! ## The ratchet
//!
//! `UFO_SN3_FLOOR` is the ACTUAL measured count of expressibility assertions the
//! battery makes (the `UFO-SN3 expressibility assertions N (floor F)` line the
//! runner prints). It may only RISE; a drop is a reasoner or ruleset regression.
//! The floor is MIRRORED in the central scoreboard (`scoreboard::SUITES`) and read
//! TEXTUALLY by `tests/scoreboard_floors.rs`, so the two can never silently drift.
//! The lane is UNGATED (plain `reason_n3`, no opt-in code to link), so it runs in
//! ordinary `cargo test --workspace`.

use sparq_core::dict::Dict;
use sparq_reason::reason_n3;
use std::collections::HashSet;

/// The UFO-SN3 expressibility assertion floor (the RATCHET). The ACTUAL measured
/// count of assertions the battery below makes — every answer.n3 triple checked
/// into the closure plus every negative-entailment guard plus the vocabulary
/// parse/inertness checks. MIRRORED in the central scoreboard
/// (`scoreboard::SUITES`) and read TEXTUALLY by `tests/scoreboard_floors.rs`. It
/// may only RISE (raise it as cases/negatives grow). HONESTLY a sparq
/// EXTENSION-shaped ratchet over the finite-world UFO-SN3 reference profile, NOT
/// a UFO/gUFO/OntoUML standards-conformance claim.
pub const UFO_SN3_FLOOR: usize = 42;

const VOCAB: &str = include_str!("ufo_sn3/vocab/ufo-sn3.ttl");
const RULES: &str = include_str!("ufo_sn3/rules/ufo-sn3.n3");

const UFO: &str = "urn:ufo-sn3:";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";

/// A tally of expressibility assertions made — the quantity ratcheted. Bumped
/// only through `must` / `must_not`, so the count is exactly the number of real
/// term-structure checks performed (no inflation).
#[derive(Default)]
struct Tally {
    n: usize,
}

impl Tally {
    fn must(
        &mut self,
        closure: &HashSet<(String, String, String)>,
        t: &(String, String, String),
        ctx: &str,
    ) {
        assert!(
            closure.contains(t),
            "[UFO-SN3 {ctx}] expected triple not derived: {t:?}"
        );
        self.n += 1;
    }
    fn must_not(
        &mut self,
        closure: &HashSet<(String, String, String)>,
        t: &(String, String, String),
        ctx: &str,
    ) {
        assert!(
            !closure.contains(t),
            "[UFO-SN3 {ctx}] triple derived that must NOT be entailed: {t:?}"
        );
        self.n += 1;
    }
}

/// The closure of an N3 document as a set of canonical (s, p, o) strings —
/// exactly the `eye_cases.rs` oracle shape.
fn closure_strings(src: &str) -> HashSet<(String, String, String)> {
    let mut d = Dict::new();
    let triples = reason_n3(&mut d, src).expect("UFO-SN3 reasoning failed");
    triples
        .iter()
        .map(|[s, p, o]| {
            (
                d.term(*s).to_string(),
                d.term(*p).to_string(),
                d.term(*o).to_string(),
            )
        })
        .collect()
}

/// A ground IRI triple in the closure's canonical string form.
fn t(s: &str, p: &str, o: &str) -> (String, String, String) {
    (format!("<{s}>"), format!("<{p}>"), format!("<{o}>"))
}

/// Run one fixture case: closure(input + rules) must contain every answer triple
/// (superset entailment) and none of the negative guards.
fn check_case(
    tally: &mut Tally,
    name: &str,
    input: &str,
    answer: &str,
    negatives: &[(String, String, String)],
) {
    let closure = closure_strings(&format!("{input}\n{RULES}"));
    let expected = closure_strings(answer);
    assert!(
        !expected.is_empty(),
        "[UFO-SN3 {name}] answer.n3 parsed to zero triples"
    );
    for triple in &expected {
        tally.must(&closure, triple, name);
    }
    for triple in negatives {
        tally.must_not(&closure, triple, name);
    }
}

#[test]
fn ufo_sn3_expressibility_ratchet() {
    let mut tally = Tally::default();

    vocab_parses_and_is_inert(&mut tally);

    // -- rigidity: Kind memberships propagate across accessibility; Role/Phase
    //    memberships do not, and an explicit counter-situation witnesses
    //    contingency (never inferred from absence).
    let ns = |l: &str| format!("urn:ufo-sn3:case:rigidity:{l}");
    check_case(
        &mut tally,
        "rigidity-kind-vs-role",
        include_str!("ufo_sn3/cases/rigidity-kind-vs-role/input.n3"),
        include_str!("ufo_sn3/cases/rigidity-kind-vs-role/answer.n3"),
        &[
            // The anti-rigid (Role) membership must NOT propagate to w2.
            t(&ns("aliceEmployee"), &format!("{UFO}holdsIn"), &ns("w2")),
            // No explicit contrary for the Kind membership → no rigidity violation.
            t(
                &ns("alicePerson"),
                &format!("{UFO}rigidityViolationIn"),
                &ns("w2"),
            ),
            // The rigid Kind is never marked contingent for the individual.
            t(&ns("Person"), &format!("{UFO}contingentFor"), &ns("alice")),
        ],
    );

    // -- identity: keys compile from the Kind's criterion and are inherited by
    //    dependent sortals; same-key co-typed instances are marked
    //    ufo:sameContinuant — never collapsed with owl:sameAs.
    let ns = |l: &str| format!("urn:ufo-sn3:case:identity:{l}");
    check_case(
        &mut tally,
        "identity",
        include_str!("ufo_sn3/cases/identity/input.n3"),
        include_str!("ufo_sn3/cases/identity/answer.n3"),
        &[t(&ns("alice"), OWL_SAME_AS, &ns("aliceRecord"))],
    );

    // -- relator: existential dependence on mediated participants, participant
    //    existence import, and truthmaking of the supplied proposition. The
    //    load-bearing reification-projection negative: the proposition HOLDING
    //    never asserts its encoded subject-predicate-object triple.
    let ns = |l: &str| format!("urn:ufo-sn3:case:relator:{l}");
    check_case(
        &mut tally,
        "relator",
        include_str!("ufo_sn3/cases/relator/input.n3"),
        include_str!("ufo_sn3/cases/relator/answer.n3"),
        &[t(&ns("alice"), &ns("employedBy"), &ns("acme"))],
    );

    // -- events: reified Participation records project hasParticipant /
    //    participatesIn and situate everything in the occurrence world.
    let ns = |l: &str| format!("urn:ufo-sn3:case:event:{l}");
    check_case(
        &mut tally,
        "event-participation",
        include_str!("ufo_sn3/cases/event-participation/input.n3"),
        include_str!("ufo_sn3/cases/event-participation/answer.n3"),
        // The participantRole TYPE is not an existent pulled into the world.
        &[t(&ns("Worker"), &format!("{UFO}existsIn"), &ns("w"))],
    );

    // -- dispositions: inherence implies existential dependence; a holding
    //    trigger proposition activates; the supplied manifestation occurs.
    let ns = |l: &str| format!("urn:ufo-sn3:case:disposition:{l}");
    check_case(
        &mut tally,
        "disposition",
        include_str!("ufo_sn3/cases/disposition/input.n3"),
        include_str!("ufo_sn3/cases/disposition/answer.n3"),
        // The trigger proposition's OBJECT is not existentially imported.
        &[t(&ns("hammer"), &format!("{UFO}existsIn"), &ns("w"))],
    );

    // -- commitments/norms: a satisfied norm condition activates the supplied
    //    obligation, which activates the grounded commitment/claim pair.
    let ns = |l: &str| format!("urn:ufo-sn3:case:norm:{l}");
    check_case(
        &mut tally,
        "commitment-norm",
        include_str!("ufo_sn3/cases/commitment-norm/input.n3"),
        include_str!("ufo_sn3/cases/commitment-norm/answer.n3"),
        // Activating the obligation does NOT assert its content proposition holds.
        &[t(&ns("paymentDue"), &format!("{UFO}holdsIn"), &ns("w"))],
    );

    // -- closed validation: explicit ufo:notHoldsIn + an explicit ufo:closedFor
    //    scope refute; open-world absence alone derives nothing.
    let ns = |l: &str| format!("urn:ufo-sn3:case:validation:{l}");
    check_case(
        &mut tally,
        "closed-validation",
        include_str!("ufo_sn3/cases/closed-validation/input.n3"),
        include_str!("ufo_sn3/cases/closed-validation/answer.n3"),
        &[
            // Refutation never flips into the proposition holding.
            t(&ns("aliceEmployed"), &format!("{UFO}holdsIn"), &ns("w")),
            // The reification projection never leaks the encoded triple.
            t(&ns("alice"), &ns("employedBy"), &ns("acme")),
        ],
    );

    // -- accessibility: explicit necessity propagates holdsIn across an edge;
    //    the edge is not symmetric and necessity itself does not propagate.
    let ns = |l: &str| format!("urn:ufo-sn3:case:accessibility:{l}");
    check_case(
        &mut tally,
        "accessibility",
        include_str!("ufo_sn3/cases/accessibility/input.n3"),
        include_str!("ufo_sn3/cases/accessibility/answer.n3"),
        &[
            t(
                &ns("intendsDelivery"),
                &format!("{UFO}necessaryIn"),
                &ns("w2"),
            ),
            t(&ns("w2"), &format!("{UFO}accessible"), &ns("w1")),
        ],
    );

    // The line a CI ratchet can grep (assertion count in field $4), mirroring the
    // RIF-Core / RSP / BM25 EXTENSION ratchet shape.
    println!("\nUFO-SN3 finite-world expressibility ratchet");
    println!(
        "UFO-SN3 expressibility assertions {} (floor {})",
        tally.n, UFO_SN3_FLOOR
    );

    assert!(
        tally.n >= UFO_SN3_FLOOR,
        "UFO-SN3 expressibility assertion count {} regressed below the ratchet floor {}",
        tally.n,
        UFO_SN3_FLOOR
    );
}

/// The committed vocabulary parses through the REAL N3 path, and is INERT under
/// the ruleset: the vocab declares classes/properties (`ufo:Kind a owl:Class`),
/// never instances (`?x a ufo:Kind`), so no UFO-SN3 rule may fire on it alone.
fn vocab_parses_and_is_inert(tally: &mut Tally) {
    let closure = closure_strings(&format!("{VOCAB}\n{RULES}"));
    // A known vocabulary triple survives the parse (the vocab genuinely loaded).
    tally.must(
        &closure,
        &t(
            &format!("{UFO}Endurant"),
            RDFS_SUBCLASS_OF,
            &format!("{UFO}Entity"),
        ),
        "vocab",
    );
    // Inertness: declaring the ufo:Kind CLASS is not an instance of it — the
    // rigidity rules must not fire on the vocabulary.
    tally.must_not(
        &closure,
        &t(
            &format!("{UFO}Kind"),
            &format!("{UFO}rigidity"),
            &format!("{UFO}Rigid"),
        ),
        "vocab",
    );
    // The vocab never types anything RDF-wise as a Kind instance either.
    tally.must_not(
        &closure,
        &t(&format!("{UFO}Kind"), RDF_TYPE, &format!("{UFO}Kind")),
        "vocab",
    );
}
