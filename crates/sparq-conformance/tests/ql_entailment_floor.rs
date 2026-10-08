//! [FABLE-5] sq-pbz04.3.4 (epic sq-pbz04.3, design record
//! research/owl2-ql-cq-gate-broadening.md §4+§6) — the PINNED NAMED-CASE floor of
//! the `pr:QL` `sparql11/entailment` arm: the subset that graduates under the
//! SIX-CONDITION soundness predicate, plus the exhaustive hold-reason taxonomy
//! for everything else.
//!
//! ## What graduates — and only under the full conjunctive predicate
//!
//! A `pr:QL` row moves from the experimental bucket to this floor iff ALL of
//! (each condition CHECKED in code by `sparql_entail::run_ql_graduation`, never
//! assumed): (1) the fail-closed CQ-shape gate accepts the query AND it carries
//! no intensional schema-vocabulary atom (the §5 B6 guard, applied here until it
//! lands in the gate itself under sq-pbz04.3.1); (2) the DL-Lite_R TBox is
//! TOTALLY captured (`fully_captured()` from sq-pbz04.3.3 — zero skipped, zero
//! unrecognised schema vocabulary); (3) the consistency condition — ZERO
//! consistency-relevant (negative/disjointness) axioms OR (sq-p6yb7) the
//! DL-Lite_R violation-query consistency check proves the KB CONSISTENT (an
//! INCONSISTENT verdict holds at `inconsistent-kb`, an UNKNOWN one at
//! `pending-consistency` — both fail-closed); (4) default-graph dataset only; (5) the
//! REGIME-COINCIDENCE guard holds — all body terms distinguished, or no
//! existential-generating inclusions (`exists_super` empty) — because the crate
//! computes CERTAIN ANSWERS while W3C entailment-regime solution mappings bind
//! every variable to an RDF term (the fail-closed §4 default, not widened); and
//! (6) the rewritten UCQ's evaluation over the UNMODIFIED data is
//! RESULT-EQUIVALENT to the W3C oracle. Conditions 1–5 are the a-priori
//! soundness argument; 6 is the empirical pin.
//!
//! ## The ratchet shape (named cases, exact set equality)
//!
//! `QL_ENTAILMENT_FLOOR_CASES` is an EXPLICIT list of manifest-entry ids. The
//! runner asserts the measured graduated set EQUALS the pinned set: a pinned
//! case failing to graduate is a hard REGRESSION failure, and a newly-eligible
//! case NOT in the list also fails — an addition must be made deliberately, in a
//! reviewed PR carrying the evidence, so the floor only ever ratchets upward
//! with eyes on it (e.g. as the sq-pbz04.3.1/.3.2 gate broadenings land).
//!
//! ## Honest scope (the load-bearing disclaimer)
//!
//! This floor is a `sparq extension` scoreboard row, tallied SEPARATELY and
//! NEVER summed into the standards-conformance total: sparq implements a
//! FRAGMENT of the QL entailment regime (abstaining elsewhere), so no
//! full-regime or full-profile OWL 2 QL conformance claim is made anywhere. The
//! inference BINARY keeps every QL row (graduated or held) in its out-of-scope
//! bucket (the D-entailment precedent: the dedicated crate-test lane is the
//! enforcement); the existing `QL_DLLITE_FLOOR` certain-answer oracle ratchet is
//! untouched and independent.
//!
//! ## Feature gating (both states)
//!
//! Behind the opt-in `ql-experimental` feature (forwards to
//! `sparq-reason-ql/experimental`); with the feature OFF this file compiles to a
//! single self-SKIP `#[test]` — the lean opt-in posture. The rdf-tests fixtures
//! are fetched by `scripts/fetch-inference-suites.sh` into the gitignored
//! `tests/w3c/rdf-tests/`; when absent the runner SKIPS so a fresh offline
//! checkout stays green (CI fetches them in the `inference-conformance` job).

#[cfg(not(feature = "ql-experimental"))]
#[test]
fn ql_entailment_floor_skipped_without_feature() {
    eprintln!(
        "SKIP: the OWL 2 QL entailment-regime graduated-subset floor is OFF — build with \
         `--features ql-experimental` (and run scripts/fetch-inference-suites.sh) to run it."
    );
}

#[cfg(feature = "ql-experimental")]
mod gated {
    use sparq_conformance::inference::sparql_entail::{
        run_ql_graduation, QlGraduationVerdict, QlHoldReason,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// The OWL 2 QL entailment-regime GRADUATED-SUBSET floor (the RATCHET
    /// count). MUST equal `QL_ENTAILMENT_FLOOR_CASES.len()` (asserted below);
    /// mirrored in the central scoreboard (`scoreboard::SUITES`) and read
    /// TEXTUALLY by `tests/scoreboard_floors.rs`. It may only RISE — raise it
    /// (with the named list) as gate broadenings make more cases genuinely pass
    /// all six conditions. [FABLE-5] sq-pbz04.3.4
    ///
    /// Raised 11 → 15 by sq-pbz04.3.6 (the body-blank-node lifting): with body
    /// blank nodes lifted to fresh non-distinguished variables — AND the emitter
    /// now preserving the SHARED-existential join (a repeated `Unbound` id maps to
    /// ONE emitted variable; `emit::cq_to_bgp` previously gave each occurrence a
    /// fresh name, an over-approximating cartesian product surfaced by the
    /// differential-oracle test strengthening) — the four undistinguished-variable
    /// `sparqldl` cases now pass all six conditions. Per the CodeQL
    /// positional-`format!` guard, the exact `println!` ratchet line the runner
    /// prints is `QL entailment graduated floor 15 of 31 (pinned 15)`. [OPUS-4.8]
    /// sq-pbz04.3.6
    pub const QL_ENTAILMENT_FLOOR: usize = 15;

    /// The PINNED NAMED-CASE floor: the manifest-entry ids (the stable `#frag`
    /// of each entry IRI — `mf:name` is NOT unique in this suite) of every
    /// `pr:QL` case that graduates under the six-condition predicate at the
    /// pinned rdf-tests revision. Sorted. Removals fail CI (a regression);
    /// additions fail CI too until pinned here with evidence (the ratchet only
    /// moves deliberately).
    ///
    /// What the eleven are (the evidence of record): plain extensional
    /// conjunctive queries over class/role atoms whose TBoxes are fully
    /// captured positive DL-Lite_R inclusions with NO existential generators —
    /// `rdf04` (plain BGP match), `rdfs02`/`rdfs10` (subPropertyOf incl.
    /// transitivity), `rdfs04`/`rdfs09` (subClassOf incl. transitivity),
    /// `rdfs06`/`rdfs07` (domain/range), `sparqldl-01`/`sparqldl-04`
    /// (type + role conjunctions). Plus two new graduates from the B2
    /// (literal-object atoms) broadening in sq-pbz04.3.1:
    ///
    /// - `lang` (`SELECT ?x WHERE { ?x foaf:name "name"@en }`, data
    ///   `:a foaf:name "name" . :b foaf:name "name"@en`, TBox
    ///   `foaf:name a owl:DatatypeProperty` — a QL-legal declaration,
    ///   silently ignored, `fully_captured() == true`, `skipped == 0`,
    ///   `exists_super` empty, `consistency_relevant == 0`; `?x` is
    ///   projected so condition (5) passes; the B2-broadened gate accepts the
    ///   literal-object atom; `rewrite_production` produces 1 disjunct
    ///   (identity rewrite, no applicable TBox inclusions); evaluated over
    ///   the UNMODIFIED data returns exactly `{:b}` — `"name"@en != "name"`
    ///   in SPARQL term equality, so only `:b` matches; W3C oracle is `{:b}`.
    ///   Result-equivalent. All six conditions pass.)
    ///
    /// - `plainLit` (IDENTICAL query, data, TBox, and oracle to `lang` —
    ///   same manifest files, same rewrite, same result; the case name differs
    ///   only in its manifest fragment; the per-case evidence is the same as
    ///   `lang` above. All six conditions pass for the same reasons.)
    ///
    /// Plus two new graduates from the BODY-BLANK-NODE lifting in sq-pbz04.3.6 (a
    /// body blank node is a non-distinguished existential = a fresh non-shared
    /// variable; `emit::cq_to_atoms` now maps each distinct label to a unique
    /// `Term::Unbound`, so the gate accepts these two undistinguished-variable
    /// cases that were previously held at `pending-gate`):
    ///
    /// - `sparqldl-05` (`ASK { _:a rdf:type :Person }`, data `data-03.ttl`:
    ///   `:a/:b/:c a :Person`, TBox = only class/property/individual DECLARATIONS
    ///   (`:Person a owl:Class`, `:name`/`:nick a owl:DatatypeProperty`,
    ///   `owl:NamedIndividual`/`owl:Ontology`). Evidence per condition:
    ///   (1) the gate ACCEPTS after the .3.6 lift (`_:a` → a fresh `Unbound`),
    ///   no intensional atom (`rdf:type :Person`, an extensional class);
    ///   (2) `fully_captured() == true` — every axiom is a recognised QL
    ///   declaration, `skipped == 0`, `unrecognised_schema == 0`;
    ///   (3) `consistency_relevant == 0` — no negative/disjointness axiom;
    ///   (4) default-graph dataset only (no `qt:graphData`, no query `FROM`);
    ///   (5) the regime-coincidence guard passes on the `exists_super`-EMPTY
    ///   branch — the declaration-only TBox has NO existential-generating
    ///   inclusion, so the canonical model adds no anonymous witness and every
    ///   certain answer maps into the data's own terms;
    ///   (6) `rewrite_production` yields 1 disjunct (identity rewrite — no
    ///   applicable inclusions), evaluated over the UNMODIFIED data the `ASK`
    ///   returns `true` (`:a`/`:b`/`:c` are `:Person`); W3C oracle is `true`.
    ///   Result-equivalent. All six pass.
    ///
    /// - `sparqldl-06` (`ASK { :a :p _:aa . _:aa :r _:dd . _:dd :t _:bb .
    ///   _:bb :s :a }`, a 4-hop cycle over three distinct shared body blank
    ///   nodes, data `data-06.ttl`: object-property + individual DECLARATIONS
    ///   plus the ABox `:a :p :aa`, `:dd :t :bb`, `:bb :s :aa`, `:cc :r :dd`,
    ///   `:aa :r :ee`). Same condition evidence as `sparqldl-05`:
    ///   (1) gate accepts — each distinct blank label maps to its own `Unbound`,
    ///   shared labels share an id (none are shared here beyond the pairing the
    ///   cycle spells out); no intensional atom;
    ///   (2) `fully_captured()`; (3) zero consistency-relevant axioms;
    ///   (4) default-graph; (5) `exists_super` EMPTY (declaration-only TBox);
    ///   (6) `rewrite_production` yields 1 disjunct (identity), evaluated over
    ///   the unmodified data the cycle does NOT close (`:a :p :aa`, `:aa :r :ee`,
    ///   but there is no `:ee :t …` triple), so the `ASK` returns `false`; W3C
    ///   oracle is `false`. Result-equivalent. All six pass.
    ///
    /// - `sparqldl-07` (`SELECT * { :a :p _:aa . ?X :t ?Y . ?Y :s _:aa .
    ///   _:aa :r ?Z }`, data `data-06.ttl`, distinguished `?X`/`?Y`/`?Z`, the
    ///   blank node `_:aa` SHARED across three atoms) and `sparqldl-08`
    ///   (`SELECT * { ?X :p _:a . _:a :r ?Y }`, data `data-06.ttl`, `_:a` shared
    ///   across two atoms). These are the SHARED-existential JOIN cases and they
    ///   graduate ONLY because sq-pbz04.3.6 ALSO fixed `emit::cq_to_bgp` to map a
    ///   repeated `Unbound` id to ONE emitted variable (preserving the join);
    ///   before the fix the emitter gave each occurrence a fresh variable, so the
    ///   rewrite computed a CARTESIAN product that condition (6) correctly caught
    ///   as `oracle-divergent` (held, never faked). Condition evidence (both):
    ///   (1) gate accepts the shared body blank node, no intensional atom;
    ///   (2) `fully_captured()` (declaration-only TBox); (3) zero
    ///   consistency-relevant axioms; (4) default-graph; (5) `exists_super` EMPTY;
    ///   (6) `rewrite_production` yields 1 disjunct (identity, join preserved),
    ///   and evaluated over the unmodified data returns EXACTLY the W3C oracle:
    ///   sparqldl-08 → `{(?X=:a, ?Y=:ee)}` (the only closed `:p`/`:r` path through
    ///   `:aa`), sparqldl-07 → `{(?X=:dd, ?Y=:bb, ?Z=:ee)}`. Result-equivalent.
    ///   All six pass. [OPUS-4.8] sq-pbz04.3.6
    ///
    /// Each rewrote through the REAL `rewrite_production` and returned EXACTLY
    /// the W3C oracle's answers over the unmodified data. [SONNET-4.6] [OPUS-4.8]
    pub const QL_ENTAILMENT_FLOOR_CASES: &[&str] = &[
        "lang",
        "plainLit",
        "rdf04",
        "rdfs02",
        "rdfs04",
        "rdfs06",
        "rdfs07",
        "rdfs09",
        "rdfs10",
        "sparqldl-01",
        "sparqldl-04",
        "sparqldl-05",
        "sparqldl-06",
        "sparqldl-07",
        "sparqldl-08",
    ];

    fn rdf_tests_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdf-tests")
    }

    /// The floor const and the named list can never drift apart.
    #[test]
    fn floor_const_matches_the_named_case_list() {
        assert_eq!(
            QL_ENTAILMENT_FLOOR,
            QL_ENTAILMENT_FLOOR_CASES.len(),
            "QL_ENTAILMENT_FLOOR must equal the pinned named-case count"
        );
        // The list is sorted + duplicate-free (set semantics, reviewable diffs).
        let mut sorted = QL_ENTAILMENT_FLOOR_CASES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted, QL_ENTAILMENT_FLOOR_CASES,
            "pinned list must be sorted + unique"
        );
    }

    /// The RATCHET: the measured graduated set equals the pinned named-case set
    /// EXACTLY, and every held case carries a specific (non-catch-all) taxonomy
    /// reason.
    #[test]
    fn ql_entailment_graduated_floor_is_pinned() {
        let root = rdf_tests_root();
        if !root
            .join("sparql/sparql11/entailment/manifest.ttl")
            .exists()
        {
            eprintln!(
                "SKIP: rdf-tests `sparql11/entailment` not present under {} — run \
                 scripts/fetch-inference-suites.sh",
                root.display()
            );
            return;
        }

        let verdicts = run_ql_graduation(&root)
            .unwrap_or_else(|e| panic!("QL graduation predicate error: {e}"));
        assert!(
            !verdicts.is_empty(),
            "the entailment suite has pr:QL-tagged tests"
        );

        // Case ids are the floor's identity — they must be unique.
        let mut ids: Vec<&str> = verdicts.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            n,
            "pr:QL case ids must be unique (the pinned floor is named over them)"
        );

        let graduated: BTreeSet<&str> = verdicts
            .iter()
            .filter(|c| matches!(c.verdict, QlGraduationVerdict::Graduated { .. }))
            .map(|c| c.id.as_str())
            .collect();
        let pinned: BTreeSet<&str> = QL_ENTAILMENT_FLOOR_CASES.iter().copied().collect();

        // Taxonomy tally (and the §8-mandated pending-consistency report).
        let mut tally: std::collections::BTreeMap<&str, usize> = Default::default();
        let mut pending_consistency = 0usize;
        for c in &verdicts {
            if let QlGraduationVerdict::Held(hold) = &c.verdict {
                *tally.entry(hold.label()).or_default() += 1;
                if let QlHoldReason::PendingConsistency { .. } = hold {
                    pending_consistency += 1;
                }
                // EXHAUSTIVE taxonomy: a new rewriter abstain class must be
                // classified explicitly, never absorbed by the loud fallback
                // bucket; and a harness setup failure would silently shrink the
                // measured set, so it is a hard failure too.
                assert_ne!(
                    hold.label(),
                    "unclassified-abstain",
                    "case {} hit the unclassified-abstain bucket ({}) — extend the \
                     taxonomy classification explicitly",
                    c.id,
                    hold.reason()
                );
                assert_ne!(
                    hold.label(),
                    "inconclusive",
                    "case {} was inconclusive ({}) — a setup failure must be fixed, \
                     not tolerated (it would mask a floor regression)",
                    c.id,
                    hold.reason()
                );
            }
        }

        // The greppable ratchet line (CI enforces the count via this line too).
        // Positional args per the CodeQL guard in the shared agent contract.
        println!(
            "QL entailment graduated floor {} of {} (pinned {})",
            graduated.len(),
            verdicts.len(),
            QL_ENTAILMENT_FLOOR
        );
        for (label, count) in &tally {
            println!("  held {}: {}", label, count);
        }
        println!(
            "  pending-consistency bucket size: {} (the DL-Lite_R consistency check \
             landed in sq-p6yb7 — this bucket now holds ONLY structurally-uncaptured \
             negative axioms, e.g. owl:complementOf; it measured ZERO before the check \
             landed, so the sq-p6yb7 upgrade graduates no case at this rdf-tests pin)",
            pending_consistency
        );

        // REGRESSION arm: every pinned case must still graduate.
        for id in &pinned {
            assert!(
                graduated.contains(id),
                "REGRESSION: pinned QL entailment case {:?} no longer graduates — a \
                 previously-sound case failed the six-condition predicate (find its \
                 hold reason in the run output above; never delete it from the pin to \
                 make CI pass)",
                id
            );
        }
        // RATCHET arm: a newly-eligible case must be pinned deliberately.
        for id in &graduated {
            assert!(
                pinned.contains(id),
                "case {:?} newly passes all six graduation conditions but is not in \
                 QL_ENTAILMENT_FLOOR_CASES — add it (and raise QL_ENTAILMENT_FLOOR + \
                 the scoreboard mirror) in a reviewed PR carrying the evidence",
                id
            );
        }
    }
}
