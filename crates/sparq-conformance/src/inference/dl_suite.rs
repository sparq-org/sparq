//! [FABLE-5] sq-pbz04.4.5 (epic sq-pbz04.4) — the OWL 2 **Direct-Semantics arm** of the
//! OWL WG test-repository export (`tests/w3c/owl2/all.rdf`), behind the opt-in
//! `dl-direct` feature. 🤖 SPARQ agent.
//!
//! Design record `research/owl2-direct-semantics-scoping.md` §4 (double-counting +
//! profile-lane semantics) / §6 (wiring). Two lanes, both driven through the REAL
//! `sparq-reason-dl` pipeline and both HONESTLY tallied as **sparq-extension** ratchets
//! over the **scoped fragment — NOT full OWL 2 DL**, never folded into
//! standards-conformance totals:
//!
//! 1. **Profile identification** — every `ProfileIdentificationTest` in the DIRECT arm,
//!    checked with the L2 syntactic profile checker (`profiles_from_extraction` — bead
//!    sq-pbz04.4.2), **POSITIVE tags only** (the design record §4 fallback, and here is
//!    the MEASUREMENT that forced it): the record allowed a negative direction only if a
//!    validation held in the L2 work; #1469 performed no such validation, and this
//!    lane's implementation MEASURED the export's EXPLICIT
//!    `owl:NegativePropertyAssertion` profile negations against L2 — on 211 of 322
//!    checkable explicit-negative rows the checker answered `In`, because L2's `In` is
//!    AXIOM-GRAMMAR membership over the extracted ALCH shadow and cannot refute
//!    full-profile membership (the profile specs' non-grammar restrictions — anonymous
//!    individuals, datatype maps, global constraints — are not implemented at L2). So
//!    no negative expectation is checked, and NOTHING is ever inferred from a missing
//!    tag. `test:species` (DL/FULL) is the deferred species check (record §4), not
//!    checked at all.
//! 2. **Direct reasoning** — every `ConsistencyTest` / `InconsistencyTest` /
//!    `PositiveEntailmentTest` / `NegativeEntailmentTest` carrying
//!    `test:semantics test:DIRECT`, decided by the L4 fragment-dispatch
//!    `DirectChecker` (bead sq-pbz04.4.4) under a PINNED deterministic count budget
//!    (wall-clock budgets are banned — floors must be reproducible).
//!
//! # Tri-state accounting (the load-bearing invariant)
//!
//! Every selected check row lands in exactly one [`TriState`] bucket: **Pass** (the
//! checker produced the expected DEFINITIVE verdict), **Fail** (the checker produced the
//! WRONG definitive verdict — a soundness bug; the gating test asserts there are none),
//! or **OutOfFragment** (the checker ABSTAINED with a typed reason — L1 refusal, a
//! dispatch guard, a deferred branch, or budget exhaustion). **An abstention is NEVER a
//! pass**: only a definitive expected verdict increments `pass`. Selection-level
//! exclusions (functional-syntax-only inputs — no `.ofn` parser in-tree; `owl:imports` —
//! no dereferencing in the harness; `test:status test:Rejected`) are OutOfScope rows,
//! outside all three verdict buckets.
//!
//! The dual-tagged (DIRECT ∧ RDF-BASED) tests keep their RDF-Based runs in the existing
//! `owl_suite` (RL) / `el_suite` lanes — one test may legitimately appear in both
//! tallies because the two runs test DIFFERENT semantics (record §4); nothing is
//! double-counted within a single lane.

use super::entail::{self, Row};
use crate::rdf::MiniGraph;
use oxrdf::{NamedOrBlankNode, Term};
use sparq_core::dict::{Dict, Id};
use sparq_reason_dl::check::{ConsistencyVerdict, DirectChecker, EntailmentVerdict, UnknownReason};
use sparq_reason_dl::profile::{profiles_from_extraction, Membership};
use sparq_reason_dl::tableau::Budget;
use std::collections::{BTreeMap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};

const T: &str = "http://www.w3.org/2007/OWL/testOntology#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const OWL_ONTOLOGY: &str = "http://www.w3.org/2002/07/owl#Ontology";
const OWL_IMPORTS: &str = "http://www.w3.org/2002/07/owl#imports";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Pinned deterministic tableau budget for every `DirectChecker` run in this lane —
/// COUNT-based only (nodes / rule applications), so the floors are reproducible on any
/// machine (wall-clock budgets are banned by design). Pinned HERE — smaller than
/// `Budget::default()` — for two reasons: a future default change cannot silently move
/// the conformance floors, and the corpus contains adversarial search spaces (deep
/// `⊔`-backtracking) where a generous cap costs minutes per case in a debug-built
/// harness; a budget exhaustion is an HONEST abstention (`OutOfFragment` in the
/// tri-state), never a verdict. Raising the budget (graduating budget-abstained cases
/// to definitive verdicts) is a deliberate re-pin of the floors in a reviewed PR.
pub const DL_TABLEAU_MAX_NODES: usize = 2_000;
/// See [`DL_TABLEAU_MAX_NODES`] — the companion rule-application count cap.
pub const DL_TABLEAU_MAX_RULE_APPLICATIONS: usize = 20_000;

/// The pinned budget both lanes hand to `DirectChecker::with_budget`.
#[must_use]
pub fn pinned_budget() -> Budget {
    Budget {
        max_nodes: DL_TABLEAU_MAX_NODES,
        max_rule_applications: DL_TABLEAU_MAX_RULE_APPLICATIONS,
    }
}

/// Outcome of one selected check row. An abstention ([`TriState::OutOfFragment`]) is
/// NEVER a pass — the module-level invariant every mapping below enforces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriState {
    /// The checker produced the expected definitive verdict.
    Pass,
    /// The checker produced the WRONG definitive verdict (payload: what was observed).
    /// A soundness bug — the gating test asserts the lane has none.
    Fail(String),
    /// The checker abstained, fail-closed (payload: the stable reason KIND — see
    /// `reason_kind` — so histograms are deterministic across runs).
    OutOfFragment(String),
}

/// Per-lane tally. `pass + fails.len() + out_of_fragment + out_of_scope` counts every
/// selected row exactly once (asserted by the gating test).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaneTally {
    /// Rows where the checker produced the expected definitive verdict.
    pub pass: usize,
    /// Wrong definitive verdicts: `(row key, observed)` — must stay empty.
    pub fails: Vec<(String, String)>,
    /// Abstention histogram keyed by the stable reason kind.
    pub out_of_fragment: BTreeMap<String, usize>,
    /// Selection-level exclusion histogram (fs-only / imports / Rejected status).
    pub out_of_scope: BTreeMap<String, usize>,
}

impl LaneTally {
    /// Record one row's tri-state outcome under `key`.
    fn record(&mut self, key: &str, outcome: TriState) {
        match outcome {
            TriState::Pass => self.pass += 1,
            TriState::Fail(observed) => self.fails.push((key.to_string(), observed)),
            TriState::OutOfFragment(kind) => {
                *self.out_of_fragment.entry(kind).or_default() += 1;
            }
        }
    }

    /// Record one row excluded at selection time (never reached the checker).
    fn record_out_of_scope(&mut self, reason: &str) {
        *self.out_of_scope.entry(reason.to_string()).or_default() += 1;
    }

    /// Total abstained rows.
    #[must_use]
    pub fn out_of_fragment_total(&self) -> usize {
        self.out_of_fragment.values().sum()
    }

    /// Total selection-excluded rows.
    #[must_use]
    pub fn out_of_scope_total(&self) -> usize {
        self.out_of_scope.values().sum()
    }

    /// Every selected row, across all four buckets.
    #[must_use]
    pub fn total(&self) -> usize {
        self.pass + self.fails.len() + self.out_of_fragment_total() + self.out_of_scope_total()
    }
}

/// The full Direct-arm report both floors are pinned against.
#[derive(Clone, Debug, Default)]
pub struct DlReport {
    /// Profile-identification lane: one row per POSITIVE `test:profile` EL/QL/RL tag on
    /// a selected `ProfileIdentificationTest` (positive-only — module docs).
    pub profile: LaneTally,
    /// EXPLICIT-NEGATIVE profile lane: one row per checkable
    /// `owl:NegativePropertyAssertion(case, test:profile, {EL|QL|RL})` in the DIRECT arm —
    /// the export's assertion that the case is NOT in that profile. Driven with
    /// `expect_in = false`: a `NotIn`/`Unknown` verdict is a Pass/abstention, an `In` is a
    /// Fail (the checker could not refute the full-profile membership). Separated from
    /// [`DlReport::profile`] so the positive lane's pin never moves when the negative lane
    /// grows. [FABLE-5] sq-pbz04.4.16
    pub profile_negative: LaneTally,
    /// Reasoning lane, per declared check kind.
    pub consistency: LaneTally,
    /// See [`DlReport::consistency`].
    pub inconsistency: LaneTally,
    /// See [`DlReport::consistency`].
    pub positive_entailment: LaneTally,
    /// See [`DlReport::consistency`].
    pub negative_entailment: LaneTally,
    /// Selected `ProfileIdentificationTest` cases (DIRECT arm).
    pub profile_cases: usize,
    /// Selected reasoning cases (any of the four check kinds, DIRECT semantics).
    pub reasoning_cases: usize,
    /// Cases dropped for `test:status test:Rejected` (the export ships none — kept
    /// defensively per the bead contract).
    pub rejected_cases: usize,
}

impl DlReport {
    /// The reasoning lane's total pass count (the `DL_DIRECT_FLOOR` measurement).
    #[must_use]
    pub fn reasoning_pass(&self) -> usize {
        self.consistency.pass
            + self.inconsistency.pass
            + self.positive_entailment.pass
            + self.negative_entailment.pass
    }

    /// All wrong-definitive-verdict rows across both lanes (must stay empty).
    #[must_use]
    pub fn all_fails(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for lane in [
            &self.profile,
            &self.profile_negative,
            &self.consistency,
            &self.inconsistency,
            &self.positive_entailment,
            &self.negative_entailment,
        ] {
            out.extend(lane.fails.iter().cloned());
        }
        out
    }

    /// Render the human-readable accounting (counts only — no timings; the two
    /// grep-able ratchet lines are printed by the gating test alongside its floors).
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut md = String::new();
        let _ = writeln!(
            md,
            "OWL 2 Direct-Semantics arm (scoped fragment — NOT full OWL 2 DL; sq-pbz04.4.5)"
        );
        let _ = writeln!(
            md,
            "  profile-identification (positive tags only): {} cases, {} assertion rows — pass {}, fail {}, abstained {}, out-of-scope {}",
            self.profile_cases,
            self.profile.total(),
            self.profile.pass,
            self.profile.fails.len(),
            self.profile.out_of_fragment_total(),
            self.profile.out_of_scope_total(),
        );
        // The explicit-negative profile lane (owl:NegativePropertyAssertion on test:profile).
        // A `fail` HERE is an honest MEASURED gap — L2's In is axiom-grammar membership over
        // the ALCH shadow and cannot always refute full-profile membership — NOT a soundness
        // bug; it is pinned as a gap floor, never asserted to zero. [FABLE-5] sq-pbz04.4.16
        let neg_total = self.profile_negative.total();
        let neg_in_gap = self.profile_negative.fails.len();
        let _ = writeln!(
            md,
            "  profile-identification (explicit-negative NPAs): {} rows — refuted (pass) {}, In-gap {}, abstained {}, out-of-scope {}",
            neg_total,
            self.profile_negative.pass,
            neg_in_gap,
            self.profile_negative.out_of_fragment_total(),
            self.profile_negative.out_of_scope_total(),
        );
        let _ = writeln!(
            md,
            "  In-vs-negative gap (checkable): {} of {}",
            neg_in_gap,
            self.profile_negative.pass + neg_in_gap,
        );
        for (kind, lane) in [
            ("consistency", &self.consistency),
            ("inconsistency", &self.inconsistency),
            ("positive-entailment", &self.positive_entailment),
            ("negative-entailment", &self.negative_entailment),
        ] {
            let _ = writeln!(
                md,
                "  {}: {} rows — pass {}, fail {}, abstained {}, out-of-scope {}",
                kind,
                lane.total(),
                lane.pass,
                lane.fails.len(),
                lane.out_of_fragment_total(),
                lane.out_of_scope_total(),
            );
        }
        let mut abstained: BTreeMap<String, usize> = BTreeMap::new();
        let mut oos: BTreeMap<String, usize> = BTreeMap::new();
        for lane in [
            &self.profile,
            &self.profile_negative,
            &self.consistency,
            &self.inconsistency,
            &self.positive_entailment,
            &self.negative_entailment,
        ] {
            for (k, n) in &lane.out_of_fragment {
                *abstained.entry(k.clone()).or_default() += n;
            }
            for (k, n) in &lane.out_of_scope {
                *oos.entry(k.clone()).or_default() += n;
            }
        }
        if !abstained.is_empty() {
            let _ = writeln!(
                md,
                "  abstention kinds (fail-closed, NEVER counted as pass):"
            );
            for (k, n) in &abstained {
                let _ = writeln!(md, "    - {} × {}", n, k);
            }
        }
        if !oos.is_empty() {
            let _ = writeln!(md, "  out-of-scope (selection exclusions):");
            for (k, n) in &oos {
                let _ = writeln!(md, "    - {} × {}", n, k);
            }
        }
        md
    }
}

/// Map a consistency outcome onto the expected-consistent tri-state.
#[must_use]
pub fn consistency_tri(verdict: &ConsistencyVerdict) -> TriState {
    match verdict {
        ConsistencyVerdict::Consistent => TriState::Pass,
        ConsistencyVerdict::Inconsistent => {
            TriState::Fail("wrongly judged inconsistent".to_string())
        }
        ConsistencyVerdict::Unknown(reason) => TriState::OutOfFragment(reason_kind(reason)),
    }
}

/// Map a consistency outcome onto the expected-INconsistent tri-state.
#[must_use]
pub fn inconsistency_tri(verdict: &ConsistencyVerdict) -> TriState {
    match verdict {
        ConsistencyVerdict::Inconsistent => TriState::Pass,
        ConsistencyVerdict::Consistent => TriState::Fail("wrongly judged consistent".to_string()),
        ConsistencyVerdict::Unknown(reason) => TriState::OutOfFragment(reason_kind(reason)),
    }
}

/// Map an entailment outcome onto the expected-Entailed tri-state.
#[must_use]
pub fn positive_entailment_tri(verdict: &EntailmentVerdict) -> TriState {
    match verdict {
        EntailmentVerdict::Entailed => TriState::Pass,
        EntailmentVerdict::NotEntailed => {
            TriState::Fail("conclusion definitively not entailed".to_string())
        }
        EntailmentVerdict::Unknown(reason) => TriState::OutOfFragment(reason_kind(reason)),
    }
}

/// Map an entailment outcome onto the expected-NOT-Entailed tri-state.
#[must_use]
pub fn negative_entailment_tri(verdict: &EntailmentVerdict) -> TriState {
    match verdict {
        EntailmentVerdict::NotEntailed => TriState::Pass,
        EntailmentVerdict::Entailed => {
            TriState::Fail("non-conclusion wrongly entailed".to_string())
        }
        EntailmentVerdict::Unknown(reason) => TriState::OutOfFragment(reason_kind(reason)),
    }
}

/// Map an L2 membership verdict onto the expected tri-state (`expect_in` = positive tag
/// vs explicit negative assertion). `Membership::Unknown` is an L1 extraction refusal —
/// an abstention, never a pass in EITHER direction.
///
/// The corpus lane drives ONLY `expect_in = true` (module docs: the explicit-negative
/// direction was measured and NOT adopted — L2's `In` is fragment-grammar membership and
/// cannot refute full-profile membership); the `false` arm is the pure documented
/// mapping, kept for the day L2 grows the full-profile restrictions.
#[must_use]
pub fn membership_tri(membership: &Membership, expect_in: bool) -> TriState {
    match (membership, expect_in) {
        (Membership::In, true) | (Membership::NotIn(_), false) => TriState::Pass,
        (Membership::In, false) => {
            TriState::Fail("checker says In despite an explicit negative assertion".to_string())
        }
        (Membership::NotIn(reason), true) => {
            TriState::Fail(format!("checker says NotIn: {}", reason))
        }
        (Membership::Unknown(_), _) => {
            TriState::OutOfFragment("out-of-fragment (L1 extraction refused)".to_string())
        }
    }
}

/// Stable, deterministic abstention-kind label for a dispatch [`UnknownReason`] — the
/// variant, not the payload (payload strings can embed hash-ordered diagnostics).
fn reason_kind(reason: &UnknownReason) -> String {
    let kind = match reason {
        UnknownReason::OutOfFragment(_) => "out-of-fragment (L1 extraction refused)",
        UnknownReason::RlPr1Preconditions(_) => "RL PR1 precondition (usage-level punning)",
        UnknownReason::RlDivergenceGuard(_) => "RL divergence guard",
        UnknownReason::ElSkippedAxioms(_) => "EL skipped axioms",
        UnknownReason::ElUnappliedAxioms(_) => "EL unapplied axiom kinds",
        UnknownReason::ElTopGuard => "EL top guard",
        UnknownReason::QlConsistencyPending => "QL consistency pending (sq-pbz04.3.4)",
        // [FABLE-5] sq-fj8lj: the graduated QL branch (dl-direct enables
        // `sparq-reason-dl/dispatch_ql`) abstains with the QL crate's own capture
        // accounting; `QlConsistencyPending` above is unreachable in this build but the
        // variant (and its tri-state mapping test) is kept.
        UnknownReason::QlCaptureGap(_) => "QL capture gap (sparq-reason-ql accounting)",
        UnknownReason::ResourceBudget(_) => "deterministic count budget exhausted",
        UnknownReason::UnencodedConclusion(_) => "unencoded conclusion-axiom kind",
        UnknownReason::ConclusionAnonymousIndividual(_) => {
            "conclusion anonymous individual (non-rollable existential shape)"
        }
        // `UnknownReason` is #[non_exhaustive]; a future variant is still an abstention.
        _ => "other typed abstention",
    };
    kind.to_string()
}

/// One selected test case, as read from the export.
struct Case {
    ident: String,
    checks: Vec<&'static str>,
    profile_test: bool,
    positive_profiles: Vec<String>,
    /// EL/QL/RL profiles the export EXPLICITLY negates for this case via a top-level
    /// `owl:NegativePropertyAssertion(case, test:profile, profile)` — the negative lane's
    /// expectations (checked with `expect_in = false`). [FABLE-5] sq-pbz04.4.16
    negative_profiles: Vec<String>,
    premise: Option<String>,
    input: Option<String>,
    conclusion: Option<String>,
    nonconclusion: Option<String>,
    imports: bool,
}

/// Parse the export text (`tests/w3c/owl2/all.rdf`) into the manifest graph — shared by
/// [`run_direct_arm`] and [`run_render_roundtrip_arm`] so both arms select from the SAME
/// pinned snapshot. [FABLE-5] sq-pbz04.4.17 (extracted verbatim from `run_direct_arm`).
fn parse_export(export_text: &str) -> Result<MiniGraph, String> {
    let fixed = fix_doctype_quotes(export_text);
    let parser = oxrdfxml::RdfXmlParser::new()
        .with_base_iri("http://owl.semanticweb.org/exports/all.rdf")
        .map_err(|e| e.to_string())?;
    let mut triples = Vec::new();
    for t in parser.for_slice(fixed.as_bytes()) {
        triples.push(t.map_err(|e| format!("all.rdf: {}", e))?);
    }
    Ok(MiniGraph { triples })
}

/// Which `test:semantics` sanction selects a case. Every REASONING / PROFILE lane is
/// [`Self::Direct`] and stays so; only the purely-syntactic render round-trip arm also
/// runs the [`Self::RdfBasedOnly`] slice (sq-pbz04.4.18). [SONNET-4.6]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemanticsArm {
    /// `test:semantics test:DIRECT` present — the selection every lane of this module
    /// used before sq-pbz04.4.18, unchanged.
    Direct,
    /// `test:semantics test:RDF-BASED` present AND `test:DIRECT` ABSENT. Deliberately the
    /// COMPLEMENT of [`Self::Direct`] rather than "all RDF-BASED": the two slices are then
    /// DISJOINT, so the two round-trip floors cannot double-count a dual-tagged case's
    /// documents. Note the local name is `RDF-BASED` (hyphenated) in the export.
    RdfBasedOnly,
}

/// Select test cases from the manifest graph for one [`SemanticsArm`] — the shared
/// selection every arm uses (matching semantics, not `test:Rejected`, carrying at least
/// one check kind or a `ProfileIdentificationTest` typing). Returns the selected cases
/// plus the count of `Rejected`-status cases dropped WITHIN that arm. [FABLE-5]
/// sq-pbz04.4.17 (extracted verbatim from `run_direct_arm` so the round-trip arm cannot
/// drift from the L5 selection); parameterised by arm in sq-pbz04.4.18 [SONNET-4.6].
fn collect_cases_for(g: &MiniGraph, arm: SemanticsArm) -> (Vec<Case>, usize) {
    // Pre-scan the top-level `owl:NegativePropertyAssertion` nodes for the EXPLICIT-NEGATIVE
    // profile lane (sq-pbz04.4.16). A manifest-level profile negation is an NPA whose
    // `assertionProperty` is `test:profile`, `sourceIndividual` is a TestCase IRI, and
    // `targetIndividual` is one of {EL, QL, RL} — "case X is NOT in profile Y". NPAs embedded
    // in premise/input ontology LITERALS are test DATA (parsed later, per case) and never seen
    // here, so this only picks the manifest metadata. Keyed by the source (case) IRI.
    let negated_profiles: HashMap<String, Vec<String>> = collect_profile_negations(g);

    let mut cases = Vec::new();
    let mut rejected = 0usize;
    for case_node in g.subjects_with_type(&format!("{}TestCase", T)) {
        let iri_objs = |p: &str| -> Vec<String> {
            g.objects(&case_node, &format!("{}{}", T, p))
                .into_iter()
                .filter_map(|t| match t {
                    Term::NamedNode(n) => Some(n.as_str().to_string()),
                    _ => None,
                })
                .collect()
        };
        // The DIRECT arm: only tests sanctioned under the Direct semantics. The
        // RDF-Based runs of dual-tagged tests stay in the owl_suite / el_suite lanes.
        // The RDF-BASED-only arm (sq-pbz04.4.18) is the disjoint complement, and feeds
        // ONLY the syntactic render round-trip — no reasoning verdict is claimed for it.
        let semantics = iri_objs("semantics");
        let sanctioned = |name: &str| semantics.iter().any(|s| s == &format!("{}{}", T, name));
        let selected = match arm {
            SemanticsArm::Direct => sanctioned("DIRECT"),
            SemanticsArm::RdfBasedOnly => sanctioned("RDF-BASED") && !sanctioned("DIRECT"),
        };
        if !selected {
            continue;
        }
        let status = g
            .object(&case_node, &format!("{}status", T))
            .and_then(|t| match t {
                Term::NamedNode(n) => n.as_str().strip_prefix(T).map(|s| s.to_string()),
                _ => None,
            });
        // Bead contract: `test:status test:Rejected` is EXCLUDED from the arm entirely
        // (the pinned export ships none — the export query already filters them).
        // Proposed / Extracredit / untagged cases RUN, honestly: this is an extension
        // ratchet over what the checker computes, not an Approved-only conformance claim.
        if status.as_deref() == Some("Rejected") {
            rejected += 1;
            continue;
        }

        let types = g.types_of(&case_node);
        let has = |t: &str| types.iter().any(|x| x == &format!("{}{}", T, t));
        let mut checks: Vec<&'static str> = Vec::new();
        if has("ConsistencyTest") {
            checks.push("consistency");
        }
        if has("InconsistencyTest") {
            checks.push("inconsistency");
        }
        if has("PositiveEntailmentTest") {
            checks.push("positive-entailment");
        }
        if has("NegativeEntailmentTest") {
            checks.push("negative-entailment");
        }
        let profile_test = has("ProfileIdentificationTest");
        if checks.is_empty() && !profile_test {
            continue;
        }

        let lit = |p: &str| -> Option<String> { g.str_object(&case_node, &format!("{}{}", T, p)) };
        let case_iri = match &case_node {
            NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        let known = |p: &String| {
            ["EL", "QL", "RL"]
                .iter()
                .any(|k| p == &format!("{}{}", T, k))
        };
        let profile_name = |p: &str| p.strip_prefix(T).unwrap_or(p).to_string();
        let case = Case {
            ident: lit("identifier")
                .unwrap_or_else(|| case_iri.rsplit('/').next().unwrap_or(&case_iri).to_string()),
            checks,
            profile_test,
            positive_profiles: iri_objs("profile")
                .iter()
                .filter(|p| known(p))
                .map(|p| profile_name(p))
                .collect(),
            negative_profiles: negated_profiles.get(&case_iri).cloned().unwrap_or_default(),
            premise: lit("rdfXmlPremiseOntology"),
            input: lit("rdfXmlInputOntology"),
            conclusion: lit("rdfXmlConclusionOntology"),
            nonconclusion: lit("rdfXmlNonConclusionOntology"),
            imports: g
                .object(&case_node, &format!("{}importedOntology", T))
                .is_some()
                || g.object(&case_node, &format!("{}importedOntologyIRI", T))
                    .is_some(),
        };
        cases.push(case);
    }
    (cases, rejected)
}

/// Run the whole Direct arm over the export text (`tests/w3c/owl2/all.rdf`).
///
/// # Errors
/// Returns `Err` only on an export-level failure (the RDF/XML export itself failed to
/// parse) — per-case problems are accounted in the report, never dropped.
pub fn run_direct_arm(export_text: &str) -> Result<DlReport, String> {
    let g = parse_export(export_text)?;
    let (cases, rejected) = collect_cases_for(&g, SemanticsArm::Direct);
    let mut report = DlReport {
        rejected_cases: rejected,
        ..DlReport::default()
    };
    for case in &cases {
        run_case(case, &mut report);
    }
    Ok(report)
}

// -----------------------------------------------------------------------------------------
// Render round-trip arms (sq-pbz04.4.17 DIRECT; sq-pbz04.4.18 RDF-BASED-only)
// -----------------------------------------------------------------------------------------

/// Which corpus slice a [`RenderRoundTripReport`] measures — the two slices are DISJOINT,
/// so their document counts and floors add without double-counting. [SONNET-4.6]
/// sq-pbz04.4.18
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoundTripArm {
    /// The DIRECT-sanctioned cases ([`run_render_roundtrip_arm`], sq-pbz04.4.17).
    #[default]
    Direct,
    /// The RDF-BASED-only cases — `test:RDF-BASED` WITHOUT `test:DIRECT`
    /// ([`run_render_roundtrip_rdf_based_arm`], sq-pbz04.4.18).
    RdfBasedOnly,
}

impl RoundTripArm {
    /// Short human label for the accounting heading.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct-Semantics",
            Self::RdfBasedOnly => "RDF-Based-only",
        }
    }

    /// Stable key segment for violation rows, so a Direct and an RDF-BASED-only row for
    /// the same case identifier never collide if the two reports are concatenated.
    fn key_segment(self) -> &'static str {
        match self {
            Self::Direct => "owl2-dl",
            Self::RdfBasedOnly => "owl2-rdf-based",
        }
    }
}

/// Report of the RENDER ROUND-TRIP arm ([`run_render_roundtrip_arm`], sq-pbz04.4.17;
/// [`run_render_roundtrip_rdf_based_arm`], sq-pbz04.4.18): the L1 forward renderer's
/// `extract → render → re-extract` invariant, checked over every ontology document of one
/// corpus slice (the same case selection as [`run_direct_arm`], or its disjoint
/// RDF-BASED-only complement). A belt-and-suspenders completeness check for the renderer's
/// 13 hand-written round-trip tests — purely syntactic (no tableau, no reasoning), so it
/// carries no verdict semantics: it can only certify that the forward mapping is a
/// faithful inverse of the reverse mapping ON the fragment L1 actually accepts. This is
/// exactly why the RDF-BASED-only slice is admissible here while it is NOT admissible in
/// any reasoning lane: round-tripping a document makes no claim about which semantics
/// sanctions it. [FABLE-5] 🤖 SPARQ agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderRoundTripReport {
    /// Which corpus slice this report measures.
    pub arm: RoundTripArm,
    /// Ontology documents examined: each PRESENT premise / input / conclusion /
    /// non-conclusion RDF/XML literal of each selected case of [`Self::arm`] counts once
    /// (documents are per-slot, not de-duplicated by content — the accounting mirrors
    /// the corpus shape, and the arm is cheap enough not to need dedup).
    pub documents: usize,
    /// Documents where L1 extraction SUCCEEDED and the rendered output re-extracted to
    /// an EQUAL structural model (`Ontology: PartialEq`) — the round-trip invariant.
    pub round_tripped: usize,
    /// Documents the L1 extractor refused (out-of-ALCH-fragment / malformed input). The
    /// render round-trip invariant is scoped to SUCCESSFUL extractions (the `render`
    /// module contract), so a refusal is out of the invariant's scope here — the
    /// extraction boundary itself is what the L5 abstention pins already measure.
    pub extraction_refused: usize,
    /// Documents whose RDF/XML literal oxrdfxml rejects (the M6 mechanism) — they never
    /// reach extraction in ANY lane.
    pub parse_failed: usize,
    /// Round-trip VIOLATIONS `(document key, diagnostic)`: a successful extraction whose
    /// rendered output failed to re-extract, re-extracted to a DIFFERENT model, or
    /// panicked. The gating test asserts this stays EMPTY — a non-empty entry is a REAL
    /// renderer (or extractor) fidelity bug, never pinnable as acceptable.
    pub violations: Vec<(String, String)>,
}

impl RenderRoundTripReport {
    /// `true` iff every examined document landed in exactly one bucket.
    #[must_use]
    pub fn accounting_closed(&self) -> bool {
        self.documents
            == self.round_tripped
                + self.extraction_refused
                + self.parse_failed
                + self.violations.len()
    }

    /// Render the human-readable accounting (counts only — no timings).
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut md = String::new();
        let _ = writeln!(
            md,
            "OWL 2 {} arm — L1 render round-trip (scoped ALCH fragment; sq-pbz04.4.17/.18)",
            self.arm.label()
        );
        let _ = writeln!(
            md,
            "  documents {} — round-tripped {}, extraction-refused {}, parse-failed {}, violations {}",
            self.documents,
            self.round_tripped,
            self.extraction_refused,
            self.parse_failed,
            self.violations.len(),
        );
        md
    }
}

/// Run the L1 render round-trip arm over the export text (`tests/w3c/owl2/all.rdf`):
/// for every ontology document of every DIRECT-arm case (the [`run_direct_arm`]
/// selection — DIRECT semantics, not `Rejected`), parse the RDF/XML literal with the
/// harness convention (ontology header / `owl:imports` edges stripped), run the REAL
/// `sparq_reason_dl::extract`, and — whenever extraction succeeds — assert the renderer
/// contract by re-extracting `sparq_reason_dl::render_to_triples`' output and comparing
/// the structural models for equality. Fresh blank nodes differ; the models must be `==`.
///
/// `owl:imports`-using cases are NOT excluded here (unlike the reasoning/profile lanes):
/// the round-trip invariant is per-DOCUMENT and purely syntactic, so the imports closure
/// is irrelevant — the literal text is round-tripped as-is.
///
/// See [`run_render_roundtrip_rdf_based_arm`] for the disjoint RDF-BASED-only slice
/// (sq-pbz04.4.18).
///
/// # Errors
/// Returns `Err` only on an export-level failure (the RDF/XML export itself failed to
/// parse) — per-document problems are accounted in the report, never dropped.
pub fn run_render_roundtrip_arm(export_text: &str) -> Result<RenderRoundTripReport, String> {
    run_render_roundtrip_for(export_text, SemanticsArm::Direct, RoundTripArm::Direct)
}

/// Run the L1 render round-trip arm over the export's **RDF-BASED-only** slice — the cases
/// carrying `test:semantics test:RDF-BASED` and NOT `test:DIRECT`, which the DIRECT arm
/// ([`run_render_roundtrip_arm`]) never sees. Same invariant, same EMPTY-violations
/// discipline, DISJOINT corpus.
///
/// This extends the renderer-fidelity check, NOT the reasoning claim: nothing here decides
/// consistency or entailment, and no verdict of any kind is attributed to an RDF-BASED
/// test.
///
/// Marginal value is honestly LOW, and SMALLER than the bead estimated. sq-pbz04.4.18
/// expected "several hundred MORE ontology documents"; MEASURED, 479 of the export's 493
/// cases are DUAL-tagged (DIRECT ∧ RDF-BASED) and already covered by
/// [`run_render_roundtrip_arm`], so this slice is only **7 cases / 13 documents** — and the
/// DIRECT arm already exercises every ALCH axiom / class-expression shape the renderer
/// emits. The bead also expected refusals to dominate the slice (OWL-Full-heavy); they do
/// not — these 7 are OWL-1-era WebOnt cases, and only 3 of the 13 documents are refused.
/// It is worth running anyway because it is closed-form and cheap, and because it makes
/// coverage exhaustive over the ELIGIBLE slice — every non-`Rejected` case with a
/// recognised check kind, sanctioned DIRECT and/or RDF-BASED — rather than merely large.
/// That is NOT the whole export: the 1 case tagged with neither semantics, and any case
/// carrying no recognised check kind, is outside BOTH arms and stays unmeasured. Exact
/// counts are pinned in `tests/dl_suite.rs` (`DL_RDF_BASED_ROUNDTRIP_*`).
///
/// # Errors
/// Returns `Err` only on an export-level failure (the RDF/XML export itself failed to
/// parse) — per-document problems are accounted in the report, never dropped.
///
/// [SONNET-4.6] sq-pbz04.4.18
pub fn run_render_roundtrip_rdf_based_arm(
    export_text: &str,
) -> Result<RenderRoundTripReport, String> {
    run_render_roundtrip_for(
        export_text,
        SemanticsArm::RdfBasedOnly,
        RoundTripArm::RdfBasedOnly,
    )
}

/// Shared body of both round-trip arms — identical per-document logic, only the case
/// selection and the violation-key segment differ. [SONNET-4.6] sq-pbz04.4.18
fn run_render_roundtrip_for(
    export_text: &str,
    semantics: SemanticsArm,
    arm: RoundTripArm,
) -> Result<RenderRoundTripReport, String> {
    let g = parse_export(export_text)?;
    let (cases, _rejected) = collect_cases_for(&g, semantics);
    let mut report = RenderRoundTripReport {
        arm,
        ..RenderRoundTripReport::default()
    };
    for case in &cases {
        let base = format!("http://owl.semanticweb.org/id/{}", case.ident);
        for (slot, doc) in [
            ("premise", &case.premise),
            ("input", &case.input),
            ("conclusion", &case.conclusion),
            ("non-conclusion", &case.nonconclusion),
        ] {
            let Some(xml) = doc else {
                continue;
            };
            report.documents += 1;
            let rows = match parse_ontology(xml, &base) {
                Ok(rows) => rows,
                Err(_) => {
                    report.parse_failed += 1;
                    continue;
                }
            };
            // catch_unwind defensiveness (the other lanes' convention): a panic anywhere
            // in extract/render is a violation row, never a torn report.
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let (mut dict, ids) = intern_rows(&rows);
                let onto1 = match sparq_reason_dl::extract(&dict, &ids) {
                    Ok(o) => o,
                    Err(_) => return None,
                };
                let rendered = sparq_reason_dl::render_to_triples(&onto1, &mut dict);
                Some(match sparq_reason_dl::extract(&dict, &rendered) {
                    Err(e) => Err(format!("rendered output failed re-extraction: {:?}", e)),
                    Ok(onto2) if onto2 == onto1 => Ok(()),
                    Ok(onto2) => Err(roundtrip_mismatch_diagnostic(&onto1, &onto2)),
                })
            }));
            let key = format!(
                "{}/render-roundtrip/{}: {}",
                arm.key_segment(),
                slot,
                case.ident
            );
            match outcome {
                Ok(None) => report.extraction_refused += 1,
                Ok(Some(Ok(()))) => report.round_tripped += 1,
                Ok(Some(Err(diag))) => report.violations.push((key, diag)),
                Err(_) => {
                    report
                        .violations
                        .push((key, "render round-trip panicked".to_string()));
                }
            }
        }
    }
    Ok(report)
}

/// Bounded diagnostic for a round-trip model mismatch: the first differing axiom index
/// plus both models' sizes (positional `format!` args — CodeQL guard).
fn roundtrip_mismatch_diagnostic(
    original: &sparq_reason_dl::Ontology,
    reextracted: &sparq_reason_dl::Ontology,
) -> String {
    let first_diff = original
        .axioms
        .iter()
        .zip(reextracted.axioms.iter())
        .position(|(a, b)| a != b);
    match first_diff {
        Some(i) => format!(
            "re-extracted model diverges at axiom {} (original {} axioms, re-extracted {}): {:?} vs {:?}",
            i,
            original.len(),
            reextracted.len(),
            original.axioms[i],
            reextracted.axioms[i]
        ),
        None => format!(
            "re-extracted model is a strict prefix/extension: original {} axioms, re-extracted {}",
            original.len(),
            reextracted.len()
        ),
    }
}

/// Collect the manifest-level EXPLICIT-NEGATIVE profile assertions
/// (`owl:NegativePropertyAssertion(case, test:profile, {EL|QL|RL})`) keyed by the source
/// (case) IRI. [FABLE-5] sq-pbz04.4.16
///
/// Only a top-level NPA whose `assertionProperty` is `test:profile` and whose
/// `targetIndividual` is one of the three tractable profiles is collected — every other NPA
/// (data-level negations in premise/input literals, non-profile assertion properties) is
/// ignored. The result preserves EL/QL/RL encounter order per case for deterministic keys.
fn collect_profile_negations(g: &MiniGraph) -> HashMap<String, Vec<String>> {
    let assertion_prop = format!("{}assertionProperty", OWL);
    let source_ind = format!("{}sourceIndividual", OWL);
    let target_ind = format!("{}targetIndividual", OWL);
    let profile_prop = format!("{}profile", T);
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for npa in g.subjects_with_type(&format!("{}NegativePropertyAssertion", OWL)) {
        // assertionProperty must be exactly test:profile.
        let is_profile = matches!(
            g.object(&npa, &assertion_prop),
            Some(Term::NamedNode(n)) if n.as_str() == profile_prop
        );
        if !is_profile {
            continue;
        }
        let Some(Term::NamedNode(src)) = g.object(&npa, &source_ind) else {
            continue;
        };
        let Some(Term::NamedNode(tgt)) = g.object(&npa, &target_ind) else {
            continue;
        };
        let Some(profile) = tgt.as_str().strip_prefix(T) else {
            continue;
        };
        if !matches!(profile, "EL" | "QL" | "RL") {
            continue;
        }
        let case = src.as_str().to_string();
        let list = out.entry(case).or_default();
        // De-duplicate a repeated (case, profile) negation — one row per pair.
        if !list.iter().any(|p| p == profile) {
            list.push(profile.to_string());
        }
    }
    out
}

/// Run both lanes' checks for one selected case.
fn run_case(case: &Case, report: &mut DlReport) {
    if case.profile_test || !case.negative_profiles.is_empty() {
        run_profile_lane(case, report);
    }
    if !case.checks.is_empty() {
        run_reasoning_lane(case, report);
    }
}

/// Pick the membership verdict for a named profile from a computed [`ProfileSet`].
fn membership_of<'a>(ps: &'a sparq_reason_dl::profile::ProfileSet, name: &str) -> &'a Membership {
    match name {
        "EL" => &ps.el,
        "QL" => &ps.ql,
        _ => &ps.rl,
    }
}

/// The profile-identification lane — POSITIVE `test:profile` tags AND EXPLICIT-NEGATIVE
/// `owl:NegativePropertyAssertion` negations. The positive tags feed [`DlReport::profile`]
/// (checked `expect_in = true`); the negations feed [`DlReport::profile_negative`] (checked
/// `expect_in = false`). Both directions share ONE extraction of the input ontology. The
/// negative lane (sq-pbz04.4.16) unlocks the 322-row explicit-negative direction the module
/// docs' MEASUREMENT deferred; an `In` verdict there is an honest gap (L2 cannot refute
/// full-profile membership from axiom-grammar membership over the ALCH shadow alone), pinned
/// as a gap floor, never asserted to zero.
fn run_profile_lane(case: &Case, report: &mut DlReport) {
    let pos = &case.positive_profiles;
    let neg = &case.negative_profiles;
    if case.profile_test {
        report.profile_cases += 1;
    }
    if case.profile_test && pos.is_empty() && neg.is_empty() {
        // A ProfileIdentificationTest with no positive tag AND no explicit negation: either a
        // species-only case (test:species DL/FULL — the deferred species check) or otherwise
        // untagged — nothing to check in either direction.
        report.profile.record_out_of_scope(
            "no positive EL/QL/RL tag and no explicit negation (species check deferred)",
        );
        return;
    }
    if case.imports {
        for _ in pos {
            report
                .profile
                .record_out_of_scope("uses owl:imports (no dereferencing in the harness)");
        }
        for _ in neg {
            report
                .profile_negative
                .record_out_of_scope("uses owl:imports (no dereferencing in the harness)");
        }
        return;
    }
    let Some(input_xml) = case.input.clone().or_else(|| case.premise.clone()) else {
        for _ in pos {
            report
                .profile
                .record_out_of_scope("input only available in functional syntax");
        }
        for _ in neg {
            report
                .profile_negative
                .record_out_of_scope("input only available in functional syntax");
        }
        return;
    };
    let base = format!("http://owl.semanticweb.org/id/{}", case.ident);
    let profile_set = catch_unwind(AssertUnwindSafe(
        || -> Result<sparq_reason_dl::profile::ProfileSet, String> {
            let rows = parse_ontology(&input_xml, &base)?;
            let (dict, ids) = intern_rows(&rows);
            Ok(profiles_from_extraction(&sparq_reason_dl::extract(
                &dict, &ids,
            )))
        },
    ));
    let profile_set = match profile_set {
        Ok(Ok(ps)) => ps,
        Ok(Err(e)) => {
            let observed = format!("input RDF/XML parse error: {}", e);
            for name in pos {
                let key = format!("owl2-dl/profile-{}: {}", name, case.ident);
                report
                    .profile
                    .record(&key, TriState::Fail(observed.clone()));
            }
            for name in neg {
                let key = format!("owl2-dl/profile-neg-{}: {}", name, case.ident);
                report
                    .profile_negative
                    .record(&key, TriState::Fail(observed.clone()));
            }
            return;
        }
        Err(_) => {
            let observed = "profile pipeline panicked".to_string();
            for name in pos {
                let key = format!("owl2-dl/profile-{}: {}", name, case.ident);
                report
                    .profile
                    .record(&key, TriState::Fail(observed.clone()));
            }
            for name in neg {
                let key = format!("owl2-dl/profile-neg-{}: {}", name, case.ident);
                report
                    .profile_negative
                    .record(&key, TriState::Fail(observed.clone()));
            }
            return;
        }
    };
    // Positive tags (expect In).
    for name in pos {
        let key = format!("owl2-dl/profile-{}: {}", name, case.ident);
        report.profile.record(
            &key,
            membership_tri(membership_of(&profile_set, name), true),
        );
    }
    // Explicit negations (expect NotIn). An `In` here is the honest measured gap.
    for name in neg {
        let key = format!("owl2-dl/profile-neg-{}: {}", name, case.ident);
        report.profile_negative.record(
            &key,
            membership_tri(membership_of(&profile_set, name), false),
        );
    }
}

/// The Direct consistency / entailment lane, through the L4 `DirectChecker`.
fn run_reasoning_lane(case: &Case, report: &mut DlReport) {
    report.reasoning_cases += 1;
    // Record OutOfScope on every declared check of this case.
    let all_out_of_scope = |report: &mut DlReport, reason: &str| {
        for kind in &case.checks {
            record_kind_oos(report, kind, reason);
        }
    };
    if case.imports {
        all_out_of_scope(report, "uses owl:imports (no dereferencing in the harness)");
        return;
    }
    let Some(premise_xml) = case.premise.clone().or_else(|| case.input.clone()) else {
        all_out_of_scope(report, "premise only available in functional syntax");
        return;
    };
    let base = format!("http://owl.semanticweb.org/id/{}", case.ident);
    let premise = match parse_ontology(&premise_xml, &base) {
        Ok(rows) => rows,
        Err(e) => {
            let observed = format!("premise RDF/XML parse error: {}", e);
            for kind in &case.checks {
                record_kind(report, kind, &case.ident, TriState::Fail(observed.clone()));
            }
            return;
        }
    };
    let checker = DirectChecker::with_budget(pinned_budget());
    for kind in &case.checks {
        let outcome = match *kind {
            "consistency" | "inconsistency" => {
                let verdict = catch_unwind(AssertUnwindSafe(|| {
                    let (mut dict, ids) = intern_rows(&premise);
                    checker.consistency(&mut dict, &ids).verdict
                }));
                match verdict {
                    Ok(v) if *kind == "consistency" => consistency_tri(&v),
                    Ok(v) => inconsistency_tri(&v),
                    Err(_) => TriState::Fail("checker panicked".to_string()),
                }
            }
            "positive-entailment" | "negative-entailment" => {
                let (doc, expect_positive) = if *kind == "positive-entailment" {
                    (&case.conclusion, true)
                } else {
                    (&case.nonconclusion, false)
                };
                let Some(xml) = doc else {
                    let side = if expect_positive {
                        "conclusion"
                    } else {
                        "non-conclusion"
                    };
                    record_kind_oos(
                        report,
                        kind,
                        &format!("{} only available in functional syntax", side),
                    );
                    continue;
                };
                match parse_ontology(xml, &base) {
                    Err(e) => TriState::Fail(format!("conclusion RDF/XML parse error: {}", e)),
                    Ok(conclusion_rows) => {
                        let verdict = catch_unwind(AssertUnwindSafe(|| {
                            let (mut dict, prem_ids) = intern_rows(&premise);
                            let concl_ids: Vec<[Id; 3]> = conclusion_rows
                                .iter()
                                .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
                                .collect();
                            checker.entailment(&mut dict, &prem_ids, &concl_ids).verdict
                        }));
                        match verdict {
                            Ok(v) if expect_positive => positive_entailment_tri(&v),
                            Ok(v) => negative_entailment_tri(&v),
                            Err(_) => TriState::Fail("checker panicked".to_string()),
                        }
                    }
                }
            }
            _ => unreachable!(),
        };
        record_kind(report, kind, &case.ident, outcome);
    }
}

/// Record one row outcome into the lane for `kind`.
fn record_kind(report: &mut DlReport, kind: &str, ident: &str, outcome: TriState) {
    let key = format!("owl2-dl/{}: {}", kind, ident);
    match kind {
        "consistency" => report.consistency.record(&key, outcome),
        "inconsistency" => report.inconsistency.record(&key, outcome),
        "positive-entailment" => report.positive_entailment.record(&key, outcome),
        _ => report.negative_entailment.record(&key, outcome),
    }
}

/// Record one selection-excluded row into the lane for `kind`.
fn record_kind_oos(report: &mut DlReport, kind: &str, reason: &str) {
    match kind {
        "consistency" => report.consistency.record_out_of_scope(reason),
        "inconsistency" => report.inconsistency.record_out_of_scope(reason),
        "positive-entailment" => report.positive_entailment.record_out_of_scope(reason),
        _ => report.negative_entailment.record_out_of_scope(reason),
    }
}

/// Intern term rows into a fresh dict (one dict per checker run — the RL branch
/// materializes into it, so runs must not share state).
fn intern_rows(rows: &[Row]) -> (Dict, Vec<[Id; 3]>) {
    let mut dict = Dict::new();
    let ids = rows
        .iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    (dict, ids)
}

/// Parses an inline RDF/XML ontology literal, dropping the ontology header
/// (`?x rdf:type owl:Ontology` typings and `owl:imports` edges) — the harness compares
/// axioms, not headers, mirroring the official OWLWG harness (and the RL / EL lanes).
fn parse_ontology(xml: &str, base: &str) -> Result<Vec<Row>, String> {
    let parser = oxrdfxml::RdfXmlParser::new()
        .with_base_iri(base)
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for t in parser.for_slice(xml.as_bytes()) {
        let t = t.map_err(|e| e.to_string())?;
        rows.push(entail::triple_row(&t));
    }
    rows.retain(|[_, p, o]| {
        let is_type = matches!(p, Term::NamedNode(n) if n.as_str() == RDF_TYPE);
        let is_imports = matches!(p, Term::NamedNode(n) if n.as_str() == OWL_IMPORTS);
        let is_ontology = matches!(o, Term::NamedNode(n) if n.as_str() == OWL_ONTOLOGY);
        !(is_imports || (is_type && is_ontology))
    });
    Ok(rows)
}

/// The export's internal DTD uses single-quoted ENTITY values, which oxrdfxml rejects;
/// rewrite just the DOCTYPE block to double quotes (same fix the RL / EL lanes apply).
fn fix_doctype_quotes(text: &str) -> String {
    let Some(start) = text.find("<!DOCTYPE") else {
        return text.to_string();
    };
    let Some(end) = text[start..].find("]>").map(|e| start + e + 2) else {
        return text.to_string();
    };
    let mut fixed = String::with_capacity(text.len());
    fixed.push_str(&text[..start]);
    fixed.push_str(&text[start..end].replace('\'', "\""));
    fixed.push_str(&text[end..]);
    fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the fail-closed tri-state mappings (the load-bearing invariant) -------------

    #[test]
    fn abstention_is_never_a_pass() {
        let unknown = ConsistencyVerdict::Unknown(UnknownReason::QlConsistencyPending);
        assert!(matches!(
            consistency_tri(&unknown),
            TriState::OutOfFragment(_)
        ));
        assert!(matches!(
            inconsistency_tri(&unknown),
            TriState::OutOfFragment(_)
        ));
        let eu = EntailmentVerdict::Unknown(UnknownReason::UnencodedConclusion("x".into()));
        assert!(matches!(
            positive_entailment_tri(&eu),
            TriState::OutOfFragment(_)
        ));
        assert!(matches!(
            negative_entailment_tri(&eu),
            TriState::OutOfFragment(_)
        ));
        let mu = Membership::Unknown("refused".into());
        assert!(matches!(
            membership_tri(&mu, true),
            TriState::OutOfFragment(_)
        ));
        assert!(matches!(
            membership_tri(&mu, false),
            TriState::OutOfFragment(_)
        ));
    }

    #[test]
    fn definitive_verdicts_map_both_ways() {
        assert_eq!(
            consistency_tri(&ConsistencyVerdict::Consistent),
            TriState::Pass
        );
        assert!(matches!(
            consistency_tri(&ConsistencyVerdict::Inconsistent),
            TriState::Fail(_)
        ));
        assert_eq!(
            inconsistency_tri(&ConsistencyVerdict::Inconsistent),
            TriState::Pass
        );
        assert!(matches!(
            inconsistency_tri(&ConsistencyVerdict::Consistent),
            TriState::Fail(_)
        ));
        assert_eq!(
            positive_entailment_tri(&EntailmentVerdict::Entailed),
            TriState::Pass
        );
        assert!(matches!(
            positive_entailment_tri(&EntailmentVerdict::NotEntailed),
            TriState::Fail(_)
        ));
        assert_eq!(
            negative_entailment_tri(&EntailmentVerdict::NotEntailed),
            TriState::Pass
        );
        assert!(matches!(
            negative_entailment_tri(&EntailmentVerdict::Entailed),
            TriState::Fail(_)
        ));
        assert_eq!(membership_tri(&Membership::In, true), TriState::Pass);
        assert!(matches!(
            membership_tri(&Membership::In, false),
            TriState::Fail(_)
        ));
        assert_eq!(
            membership_tri(&Membership::NotIn("r".into()), false),
            TriState::Pass
        );
        assert!(matches!(
            membership_tri(&Membership::NotIn("r".into()), true),
            TriState::Fail(_)
        ));
    }

    #[test]
    fn lane_tally_accounting_is_total() {
        let mut lane = LaneTally::default();
        lane.record("a", TriState::Pass);
        lane.record("b", TriState::Fail("wrong".into()));
        lane.record("c", TriState::OutOfFragment("kind".into()));
        lane.record_out_of_scope("fs-only");
        assert_eq!(lane.pass, 1);
        assert_eq!(lane.fails.len(), 1);
        assert_eq!(lane.out_of_fragment_total(), 1);
        assert_eq!(lane.out_of_scope_total(), 1);
        assert_eq!(lane.total(), 4);
    }

    #[test]
    fn pinned_budget_is_the_named_counts() {
        let b = pinned_budget();
        assert_eq!(b.max_nodes, DL_TABLEAU_MAX_NODES);
        assert_eq!(b.max_rule_applications, DL_TABLEAU_MAX_RULE_APPLICATIONS);
    }

    // ---- end-to-end over a tiny synthetic export --------------------------------------

    /// A minimal all.rdf-shaped export: one in-fragment consistency case (Pass), one
    /// out-of-fragment consistency case (abstains — the fail-closed path), one
    /// inconsistency case (Pass via the ALCH tableau), and one profile case with a
    /// positive RL tag + an explicit negative EL assertion.
    const MINI_EXPORT: &str = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:test="http://www.w3.org/2007/OWL/testOntology#">
  <test:TestCase rdf:about="http://ex/case-consistent">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ConsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">case-consistent</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="B"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
  <test:TestCase rdf:about="http://ex/case-out-of-fragment">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ConsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">case-out-of-fragment</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;rdf:Description rdf:about="a"&gt;&lt;owl:sameAs rdf:resource="b"/&gt;&lt;/rdf:Description&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
  <test:TestCase rdf:about="http://ex/case-inconsistent">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#InconsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">case-inconsistent</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;owl:equivalentClass&gt;&lt;owl:Class&gt;&lt;owl:complementOf rdf:resource="A"/&gt;&lt;/owl:Class&gt;&lt;/owl:equivalentClass&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
  <test:TestCase rdf:about="http://ex/case-profile">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ProfileIdentificationTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">case-profile</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:profile rdf:resource="http://www.w3.org/2007/OWL/testOntology#RL"/>
    <test:rdfXmlInputOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Restriction&gt;&lt;owl:onProperty&gt;&lt;owl:ObjectProperty rdf:about="r"/&gt;&lt;/owl:onProperty&gt;&lt;owl:allValuesFrom rdf:resource="B"/&gt;&lt;/owl:Restriction&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlInputOntology>
  </test:TestCase>
  <owl:NegativePropertyAssertion>
    <owl:sourceIndividual rdf:resource="http://ex/case-profile"/>
    <owl:assertionProperty rdf:resource="http://www.w3.org/2007/OWL/testOntology#profile"/>
    <owl:targetIndividual rdf:resource="http://www.w3.org/2007/OWL/testOntology#EL"/>
  </owl:NegativePropertyAssertion>
</rdf:RDF>"#;

    #[test]
    fn mini_export_end_to_end() {
        let report = run_direct_arm(MINI_EXPORT).expect("mini export parses");
        // Reasoning lane: the in-fragment consistent case passes; the owl:sameAs case
        // abstains (L1 refuses — fail-closed, NOT a pass); the ¬A ≡ A case is
        // inconsistent via the real ALCH tableau.
        assert_eq!(report.reasoning_cases, 3);
        assert_eq!(report.consistency.pass, 1);
        assert_eq!(report.consistency.out_of_fragment_total(), 1);
        assert_eq!(report.inconsistency.pass, 1);
        assert!(report.all_fails().is_empty(), "{:?}", report.all_fails());
        assert_eq!(report.reasoning_pass(), 2);
        // Positive profile lane: the ∀-restriction TBox is RL, so the positive RL tag passes
        // — exactly one positive row.
        assert_eq!(report.profile_cases, 1);
        assert_eq!(report.profile.total(), 1);
        assert_eq!(report.profile.pass, 1);
        // Explicit-negative profile lane (sq-pbz04.4.16): the mini export's
        // `owl:NegativePropertyAssertion(case-profile, test:profile, EL)` asserts the case is
        // NOT in EL. The input is an ∀R.B TBox — genuinely NotIn EL (EL forbids
        // ObjectAllValuesFrom), so L2 REFUTES the membership and the negative row is a Pass
        // (refuted). Exactly one negative row, no In-gap.
        assert_eq!(report.profile_negative.total(), 1);
        assert_eq!(report.profile_negative.pass, 1);
        assert_eq!(report.profile_negative.fails.len(), 0);
        // The render is counts-only, names the honest scope label, and prints the negative
        // lane's In-vs-negative gap line.
        let md = report.render();
        assert!(md.contains("NOT full OWL 2 DL"));
        assert!(md.contains("NEVER counted as pass"));
        assert!(md.contains("In-vs-negative gap"));
    }

    #[test]
    fn rejected_status_is_excluded() {
        let rejected = MINI_EXPORT.replace(
            "http://www.w3.org/2007/OWL/testOntology#Approved",
            "http://www.w3.org/2007/OWL/testOntology#Rejected",
        );
        let report = run_direct_arm(&rejected).expect("mini export parses");
        assert_eq!(report.rejected_cases, 4);
        assert_eq!(report.reasoning_cases, 0);
        assert_eq!(report.profile_cases, 0);
        assert_eq!(report.reasoning_pass() + report.profile.pass, 0);
    }

    #[test]
    fn fs_only_and_imports_are_out_of_scope_not_passes() {
        // Strip the premise literal from the consistent case → fs-only OutOfScope.
        let fs_only =
            MINI_EXPORT.replacen("test:rdfXmlPremiseOntology", "test:fsPremiseOntology", 2);
        let report = run_direct_arm(&fs_only).expect("mini export parses");
        assert_eq!(report.consistency.out_of_scope_total(), 1);
        assert_eq!(report.consistency.pass, 0);
    }

    // ---- render round-trip arm (sq-pbz04.4.17) -----------------------------------------

    /// Direct unit test of `run_render_roundtrip_arm` over the same mini export the
    /// Direct-arm end-to-end test uses: 4 documents (three premises + one input); the
    /// three in-fragment documents round-trip through the REAL
    /// `extract → render_to_triples → extract` pipeline; the `owl:sameAs` premise is
    /// refused by L1 — and a refusal is NEVER counted as a round-trip.
    #[test]
    fn render_roundtrip_arm_mini_export() {
        let report = run_render_roundtrip_arm(MINI_EXPORT).expect("mini export parses");
        assert_eq!(report.documents, 4);
        assert_eq!(report.round_tripped, 3);
        assert_eq!(report.extraction_refused, 1);
        assert_eq!(report.parse_failed, 0);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
        assert!(report.accounting_closed());
        // The counts-only render names the arm and carries every bucket.
        let md = report.render();
        assert!(md.contains("render round-trip"));
        assert!(md.contains("round-tripped 3"));
        assert!(md.contains("extraction-refused 1"));
    }

    /// Direct unit test of `RenderRoundTripReport::accounting_closed`: closed for a
    /// consistent report, open when a bucket is dropped.
    #[test]
    fn render_roundtrip_accounting_closed_detects_drops() {
        let mut report = RenderRoundTripReport {
            arm: RoundTripArm::Direct,
            documents: 3,
            round_tripped: 1,
            extraction_refused: 1,
            parse_failed: 0,
            violations: vec![("k".to_string(), "diag".to_string())],
        };
        assert!(report.accounting_closed());
        report.documents = 4; // one document unaccounted → open
        assert!(!report.accounting_closed());
    }

    /// A malformed inline ontology literal (bad XML) lands in `parse_failed`, never in a
    /// round-trip or refusal bucket.
    #[test]
    fn render_roundtrip_bad_literal_is_parse_failed() {
        let export = MINI_EXPORT.replace(
            "&lt;rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" \
             xmlns:owl=\"http://www.w3.org/2002/07/owl#\" xml:base=\"http://example.org/\"&gt;\
             &lt;owl:Ontology/&gt;&lt;rdf:Description rdf:about=\"a\"&gt;\
             &lt;owl:sameAs rdf:resource=\"b\"/&gt;&lt;/rdf:Description&gt;&lt;/rdf:RDF&gt;",
            "not xml at all",
        );
        let report = run_render_roundtrip_arm(&export).expect("mini export parses");
        assert_eq!(report.documents, 4);
        assert_eq!(report.parse_failed, 1);
        assert_eq!(report.extraction_refused, 0);
        assert_eq!(report.round_tripped, 3);
        assert!(report.violations.is_empty());
        assert!(report.accounting_closed());
    }

    /// Direct unit test of `collect_profile_negations` (sq-pbz04.4.16): a top-level
    /// `owl:NegativePropertyAssertion(case, test:profile, EL)` is collected keyed by the
    /// source case IRI; a NON-profile assertion property and a non-{EL,QL,RL} target are
    /// ignored; a repeated (case, profile) pair is de-duplicated to one row.
    #[test]
    fn collect_profile_negations_filters_and_dedups() {
        let export = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:test="http://www.w3.org/2007/OWL/testOntology#">
  <owl:NegativePropertyAssertion>
    <owl:sourceIndividual rdf:resource="http://ex/case-a"/>
    <owl:assertionProperty rdf:resource="http://www.w3.org/2007/OWL/testOntology#profile"/>
    <owl:targetIndividual rdf:resource="http://www.w3.org/2007/OWL/testOntology#EL"/>
  </owl:NegativePropertyAssertion>
  <owl:NegativePropertyAssertion>
    <owl:sourceIndividual rdf:resource="http://ex/case-a"/>
    <owl:assertionProperty rdf:resource="http://www.w3.org/2007/OWL/testOntology#profile"/>
    <owl:targetIndividual rdf:resource="http://www.w3.org/2007/OWL/testOntology#EL"/>
  </owl:NegativePropertyAssertion>
  <owl:NegativePropertyAssertion>
    <owl:sourceIndividual rdf:resource="http://ex/case-a"/>
    <owl:assertionProperty rdf:resource="http://www.w3.org/2007/OWL/testOntology#profile"/>
    <owl:targetIndividual rdf:resource="http://www.w3.org/2007/OWL/testOntology#DL"/>
  </owl:NegativePropertyAssertion>
  <owl:NegativePropertyAssertion>
    <owl:sourceIndividual rdf:resource="http://ex/case-b"/>
    <owl:assertionProperty rdf:resource="http://ex/notProfile"/>
    <owl:targetIndividual rdf:resource="http://www.w3.org/2007/OWL/testOntology#RL"/>
  </owl:NegativePropertyAssertion>
</rdf:RDF>"#;
        let parser = oxrdfxml::RdfXmlParser::new()
            .with_base_iri("http://ex/")
            .unwrap();
        let triples: Vec<_> = parser
            .for_slice(export.as_bytes())
            .map(|t| t.unwrap())
            .collect();
        let g = MiniGraph { triples };
        let map = collect_profile_negations(&g);
        // case-a: EL (de-duplicated to one), DL ignored (not EL/QL/RL).
        assert_eq!(map.get("http://ex/case-a"), Some(&vec!["EL".to_string()]));
        // case-b: the assertion property is not test:profile → ignored entirely.
        assert!(!map.contains_key("http://ex/case-b"));
    }
}
