//! sq-hmd7l.10 (epic sq-hmd7l) — ORE-style OWL DL harness for sparq-reason-dl.
//!
//! 🤖 SPARQ agent. First benchmark for the DL surface (`tableau.rs` / `profile.rs`).
//! HONEST FRAMING: sparq-reason-dl is a scoped ALCH profile-checker/tableau, NOT a full
//! OWL 2 DL reasoner — on real ORE corpora it is expected to ABSTAIN
//! (`Unknown(OutOfFragment)`) on every ontology outside ALCH, and the harness records
//! that abstention rather than a verdict. The ORE task shapes exercised here are the
//! competition's consistency + class-satisfiability tasks.
//!
//! TWO MODES
//! ---------
//!   * SMOKE (no args, or the sole arg `--smoke`): run the four VENDORED
//!     `examples/data/ore_smoke_*.ttl` fixtures (compiled in via `include_str!`, so the
//!     run is hermetic and cwd-independent) and ASSERT their pinned verdicts — one
//!     consistent, one inconsistent (derived clash through the role hierarchy), one
//!     consistent ontology with a pinned-UNSATISFIABLE class (mad cow), and one
//!     out-of-fragment abstention — plus pinned EL/QL/RL profile memberships. This is
//!     the acceptance gate: `cargo run -p sparq-reason-dl --release --example ore_bench
//!     -- --smoke` exits 0 iff every pinned verdict still holds. The real ORE 2014/2015
//!     corpus files are gather-only downloads (big corpora stay out of git); the
//!     vendored subset is hand-authored IN THE SHAPE of those tasks and hand-verified.
//!
//!   * GATHER (`<path> [format]`): parse one ontology already converted to RDF
//!     (`format` defaults to `turtle`; pass `ntriples` for a riot-converted ORE corpus
//!     file), run extract → profile → tableau consistency, and PRINT
//!     `ontology=… verdict=… profile_el=… profile_ql=… profile_rl=… axioms=…
//!     extract_s=… check_s=…` — NO assertion. The VERDICT fields come before the timing
//!     fields so scripts/bench/reason-dl-same-box.sh can enforce the sq-hmd7l.10
//!     invariant: NO timing row without a verdict-agreement check vs HermiT/Openllet;
//!     a disagreement is a correctness finding, never timed past. `extract_s`/`check_s`
//!     are the sparq-INTERNAL phase breakdown only — the cross-engine metric is the
//!     wrapper's end-to-end process wall clock around this invocation, the same
//!     boundary it applies to the JVM CLIs (startup + parse + reasoning included).
//!
//! Verdicts are produced under a DETERMINISTIC count budget (no wall-clock budgets);
//! gather runs may raise it via ORE_BENCH_MAX_NODES / ORE_BENCH_MAX_RULE_APPS. Budget
//! exhaustion prints `unknown(budget:…)` — an abstention, not a verdict. Work-box
//! wall-clock is non-canonical — printed as a trend only.

use sparq_core::Graph;
use sparq_reason_dl::extract;
use sparq_reason_dl::profile::{profiles_from_extraction, Membership, ProfileSet};
use sparq_reason_dl::tableau::{
    class_satisfiability, consistency_from_extraction, Budget, UnknownReason, Verdict,
};
use sparq_reason_dl::model::ClassExpression;
use std::time::Instant;

// --- the vendored ORE-style smoke subset (hermetic; see each fixture's header) -----------
const UNIV_TTL: &str = include_str!("data/ore_smoke_univ.ttl");
const ROLE_DISJOINT_TTL: &str = include_str!("data/ore_smoke_role_disjoint.ttl");
const MADCOW_TTL: &str = include_str!("data/ore_smoke_madcow.ttl");
const CARDINALITY_TTL: &str = include_str!("data/ore_smoke_cardinality.ttl");

/// The class pinned UNSATISFIABLE w.r.t. the madcow fixture (its ontology stays consistent).
const MADCOW_CLASS: &str = "http://sparq.dev/bench/ore-smoke/madcow#MadCow";

/// Pinned smoke budget — deterministic and generous (the fixtures need only a handful of
/// nodes); pinned explicitly rather than via `Budget::default()` so a future default
/// change cannot silently move the acceptance gate.
const SMOKE_BUDGET: Budget = Budget {
    max_nodes: 10_000,
    max_rule_applications: 200_000,
};

/// What a smoke fixture pins for ontology consistency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Consistent,
    Inconsistent,
    /// Fail-closed abstention: the L1 extractor must refuse the graph (out-of-fragment).
    OutOfFragment,
}

struct SmokeCase {
    name: &'static str,
    ttl: &'static str,
    expect: Expect,
    /// Pinned EL/QL/RL `Membership::In` booleans — `None` for the out-of-fragment case,
    /// where all three memberships must be `Unknown`. Hand-derived from the W3C OWL 2
    /// Profiles grammar; the per-fixture rationale lives in `run_smoke`.
    expect_profiles_in: Option<[bool; 3]>,
    /// A class IRI pinned UNSATISFIABLE w.r.t. the (consistent) ontology, if any.
    unsat_class: Option<&'static str>,
}

/// One ontology run through the real fail-closed pipeline: parse → extract → profile →
/// tableau consistency. Timing covers extract-through-verdict work only (parse excluded).
struct Outcome {
    verdict: Verdict,
    profiles: ProfileSet,
    axioms: usize,
    extract_s: f64,
    check_s: f64,
}

fn check_text(text: &str, format: &str, budget: Budget) -> Result<Outcome, String> {
    let (dict, triples) = Graph::parse_to_triples(text, format)?;
    let t = Instant::now();
    let extraction = extract(&dict, &triples);
    let extract_s = t.elapsed().as_secs_f64();
    let profiles = profiles_from_extraction(&extraction);
    let axioms = extraction.as_ref().map(|o| o.axioms.len()).unwrap_or(0);
    let t = Instant::now();
    let verdict = consistency_from_extraction(&extraction, budget);
    let check_s = t.elapsed().as_secs_f64();
    Ok(Outcome {
        verdict,
        profiles,
        axioms,
        extract_s,
        check_s,
    })
}

/// Render a consistency verdict for the harness (`consistent` / `inconsistent` /
/// `unknown(…)`). Stable strings — scripts/bench/reason-dl-same-box.sh matches on them.
fn verdict_str(v: &Verdict) -> String {
    match v {
        Verdict::Satisfiable => "consistent".to_owned(),
        Verdict::Unsatisfiable => "inconsistent".to_owned(),
        Verdict::Unknown(UnknownReason::OutOfFragment(_)) => "unknown(out-of-fragment)".to_owned(),
        Verdict::Unknown(UnknownReason::ResourceBudget(which)) => {
            format!("unknown(budget:{:?})", which)
        }
    }
}

fn membership_str(m: &Membership) -> &'static str {
    match m {
        Membership::In => "in",
        Membership::NotIn(_) => "not-in",
        Membership::Unknown(_) => "unknown",
    }
}

fn budget_from_env(default: Budget) -> Budget {
    let read = |var: &str, fallback: usize| {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(fallback)
    };
    Budget {
        max_nodes: read("ORE_BENCH_MAX_NODES", default.max_nodes),
        max_rule_applications: read("ORE_BENCH_MAX_RULE_APPS", default.max_rule_applications),
    }
}

// --- SMOKE ------------------------------------------------------------------------------

/// Pinned-profile rationale (hand-derived from the W3C OWL 2 Profiles grammar, §2–§4):
///
/// * `univ_consistent` — the ∀involvedWith.(Course ⊔ Person) superclass is outside EL
///   (no allValuesFrom) and QL (no allValuesFrom), and its UNION filler is not an RL
///   superClassExpression; the ∃teaches.Course superclass is also non-RL. → none.
/// * `role_disjoint_inconsistent` — ∀s.B kills EL and QL, but every axiom (named-filler
///   universal superclass, disjointness, role hierarchy, assertions) is RL. → RL only.
/// * `madcow_unsat_class` — ∀eats.¬Meat kills EL/QL; the ∃eats.Meat SUPERclass kills
///   RL. → none.
fn smoke_cases() -> Vec<SmokeCase> {
    vec![
        SmokeCase {
            name: "univ_consistent",
            ttl: UNIV_TTL,
            expect: Expect::Consistent,
            expect_profiles_in: Some([false, false, false]),
            unsat_class: None,
        },
        SmokeCase {
            name: "role_disjoint_inconsistent",
            ttl: ROLE_DISJOINT_TTL,
            expect: Expect::Inconsistent,
            expect_profiles_in: Some([false, false, true]),
            unsat_class: None,
        },
        SmokeCase {
            name: "madcow_unsat_class",
            ttl: MADCOW_TTL,
            expect: Expect::Consistent,
            expect_profiles_in: Some([false, false, false]),
            unsat_class: Some(MADCOW_CLASS),
        },
        SmokeCase {
            name: "cardinality_out_of_fragment",
            ttl: CARDINALITY_TTL,
            expect: Expect::OutOfFragment,
            expect_profiles_in: None,
            unsat_class: None,
        },
    ]
}

fn run_smoke() {
    for case in smoke_cases() {
        let out = check_text(case.ttl, "turtle", SMOKE_BUDGET)
            .unwrap_or_else(|e| panic!("smoke fixture {} must parse: {}", case.name, e));
        // Verdict + profile fields print BEFORE the timing fields (the harness invariant).
        println!(
            "smoke[{}]: verdict={} profile_el={} profile_ql={} profile_rl={} axioms={} \
             extract_s={:.6} check_s={:.6}",
            case.name,
            verdict_str(&out.verdict),
            membership_str(&out.profiles.el),
            membership_str(&out.profiles.ql),
            membership_str(&out.profiles.rl),
            out.axioms,
            out.extract_s,
            out.check_s
        );

        match case.expect {
            Expect::Consistent => assert!(
                out.verdict.is_satisfiable(),
                "{}: pinned CONSISTENT, got {:?}",
                case.name,
                out.verdict
            ),
            Expect::Inconsistent => assert!(
                out.verdict.is_unsatisfiable(),
                "{}: pinned INCONSISTENT (the derived role-hierarchy clash no longer \
                 derives?), got {:?}",
                case.name,
                out.verdict
            ),
            Expect::OutOfFragment => assert!(
                matches!(
                    out.verdict,
                    Verdict::Unknown(UnknownReason::OutOfFragment(_))
                ),
                "{}: pinned Unknown(OutOfFragment) — the fail-closed abstention the \
                 harness depends on — got {:?}",
                case.name,
                out.verdict
            ),
        }

        match case.expect_profiles_in {
            Some([el, ql, rl]) => {
                let got = [
                    out.profiles.el.is_in(),
                    out.profiles.ql.is_in(),
                    out.profiles.rl.is_in(),
                ];
                assert_eq!(
                    got,
                    [el, ql, rl],
                    "{}: pinned EL/QL/RL membership {:?} != got {:?} ({:?})",
                    case.name,
                    [el, ql, rl],
                    got,
                    out.profiles
                );
            }
            None => assert!(
                out.profiles.el.is_unknown()
                    && out.profiles.ql.is_unknown()
                    && out.profiles.rl.is_unknown(),
                "{}: out-of-fragment fixture must yield Unknown for all three profiles, \
                 got {:?}",
                case.name,
                out.profiles
            ),
        }

        if let Some(class_iri) = case.unsat_class {
            // Re-parse to reach the Ontology + Dict (the fixture is tiny); the class
            // satisfiability task is the ORE task shape the DL tableau uniquely serves.
            let (dict, triples) =
                Graph::parse_to_triples(case.ttl, "turtle").expect("fixture parses");
            let onto = extract(&dict, &triples).expect("fixture is in-fragment");
            let class_id = dict.lookup(&oxrdf::Term::NamedNode(
                oxrdf::NamedNode::new_unchecked(class_iri),
            ));
            let v = class_satisfiability(&ClassExpression::Class(class_id), &onto, SMOKE_BUDGET);
            println!(
                "smoke[{}]: class=<{}> satisfiability={}",
                case.name,
                class_iri,
                match &v {
                    Verdict::Satisfiable => "satisfiable".to_owned(),
                    Verdict::Unsatisfiable => "unsatisfiable".to_owned(),
                    other => verdict_str(other),
                }
            );
            assert!(
                v.is_unsatisfiable(),
                "{}: class <{}> pinned UNSATISFIABLE w.r.t. the ontology, got {:?}",
                case.name,
                class_iri,
                v
            );
        }
    }
    println!(
        "OK: all pinned ORE-smoke verdicts held (consistent / derived-inconsistent / \
         unsat-class / out-of-fragment abstention + EL/QL/RL memberships). Wall-clock is \
         trend-only (non-canonical shared box)."
    );
}

// --- GATHER -----------------------------------------------------------------------------

fn run_gather(path: &str, format: &str) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read ontology {}: {}", path, e));
    let budget = budget_from_env(SMOKE_BUDGET);
    let out = check_text(&text, format, budget)
        .unwrap_or_else(|e| panic!("cannot parse {} as {}: {}", path, format, e));
    // Verdict + profile fields FIRST, timing after — the harness records the verdict and
    // cross-checks it vs HermiT/Openllet BEFORE it trusts any wall-clock (sq-hmd7l.10).
    println!(
        "ontology={} verdict={} profile_el={} profile_ql={} profile_rl={} axioms={} \
         extract_s={:.6} check_s={:.6}",
        path,
        verdict_str(&out.verdict),
        membership_str(&out.profiles.el),
        membership_str(&out.profiles.ql),
        membership_str(&out.profiles.rl),
        out.axioms,
        out.extract_s,
        out.check_s
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--smoke") => run_smoke(),
        Some(path) => {
            let format = args.get(1).map(String::as_str).unwrap_or("turtle");
            run_gather(path, format);
        }
    }
}
