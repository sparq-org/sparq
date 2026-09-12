//! SPARQL 1.1 entailment-regime evaluation (`sparql11/entailment` in the same
//! rdf-tests clone the SPARQL harness uses): each test names the regimes
//! under which its expected answers hold (`sd:entailmentRegime` — RDF / RDFS /
//! D / OWL-RDF-Based / OWL-Direct / RIF). The runner picks the strongest
//! regime sparq-reason materializes:
//!
//! - **RDFS** → `materialize_rdfs` (+ rdfD2 predicate typing),
//! - **RDF** (only) → rdfD2 predicate typing,
//! - **OWL-RDF-Based** (without RDF/RDFS tag) → `materialize_owl_rl` — the RL
//!   rules are a sound subset of the OWL 2 RDF-based semantics, so missing
//!   answers are honest fails, not crashes,
//! - **D / OWL-Direct / RIF only** → out of scope with the reason.
//!
//! The query then runs over the materialized closure through the SAME
//! evaluation+comparison path as the W3C SPARQL harness
//! (`run::run_query_test_on` with a data override). Generalized triples the
//! materializer may produce (literal subjects, from range typing) cannot be
//! answers to any SPARQL query and are filtered out of the closure dataset.

use super::report::{Outcome, TestResult};
use crate::manifest::{self, EntryKind, TestEntry};
use crate::run::{self, EntailmentAnswerFilter, Status};
use oxrdf::vocab::rdf;
use oxrdf::Term;
use rustc_hash::FxHashSet;
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};
use sparq_core::dict::Dict;
use std::path::Path;

pub fn notes() -> Vec<String> {
    vec![
        "Source: `sparql11/entailment` from the pinned rdf-tests clone. Each test runs as \
         query-over-materialized-closure through the same evaluation/comparison machinery as \
         the gating SPARQL harness; the regime materialized is the strongest the reasoner \
         supports of the test's `sd:entailmentRegime` set (RDFS ⊃ RDF; OWL-RDF-Based via the \
         OWL RL rules, a sound subset — its incompleteness shows up as listed fails). The \
         OwlRl closure adds a harness-side eq-ref layer (OWL 2 Profiles §4.3 Table 4: \
         reflexive owl:sameAs for every closure term — omitted by the production \
         materializer as store bloat). The regimes' ANSWER RESTRICTION is applied to engine \
         solutions before comparison (SPARQL 1.1 Entailment Regimes): (C1/skolemization, \
         §2/§3.1) bindings to blank nodes not in the queried graph — i.e. introduced by the \
         saturation — are never answers; and for tests whose expectations are sanctioned \
         under OWL-Direct (§7), variables in class/property-NAME positions cannot bind to \
         anonymous class expressions (a bnode is not a name). \
         D-only / OWL-Direct-only / RIF tests are out of scope with their reason."
            .to_string(),
    ]
}

pub fn run_suite(rdf_tests_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let suites_root = rdf_tests_root
        .join("sparql")
        .canonicalize()
        .map_err(|e| format!("rdf-tests sparql dir: {e}"))?;
    let manifest_path = suites_root.join("sparql11/entailment/manifest.ttl");
    let mut entries: Vec<TestEntry> = Vec::new();
    manifest::collect(&manifest_path, &suites_root, &mut entries)?;

    for entry in &entries {
        if entry.kind != EntryKind::QueryEval {
            out.push(result(entry, Outcome::OutOfScope("not a QueryEvaluationTest".into())));
            continue;
        }
        let regimes: FxHashSet<&str> = entry
            .action
            .entailment_regimes
            .iter()
            .filter_map(|r| r.rsplit('/').next())
            .collect();
        let profile = if regimes.contains("RDFS") {
            Profile::Rdfs
        } else if regimes.contains("RDF") {
            Profile::Rdf
        } else if regimes.contains("OWL-RDF-Based") {
            Profile::OwlRl
        } else if d_regime_profile(&regimes).is_some() {
            // [OPUS-4.8] sq-e5atd — a D-only (datatype / value-space) test: route
            // it through the opt-in `Profile::D` materializer when the `d-entail`
            // feature is ON. When OFF, `d_regime_profile` returns None and the test
            // stays OutOfScope (below), so the default inference binary's ratchet is
            // unchanged — the lean opt-in posture.
            d_regime_profile(&regimes).unwrap()
        } else {
            let mut rs: Vec<&str> = regimes.iter().copied().collect();
            rs.sort_unstable();
            out.push(result(
                entry,
                Outcome::OutOfScope(format!(
                    "entailment regime(s) {} not supported (no materialization mapping)",
                    rs.join("+")
                )),
            ));
            continue;
        };
        let direct_sanctioned = regimes.contains("OWL-Direct");
        out.push(result(entry, run_one(entry, profile, direct_sanctioned)));
    }
    Ok(())
}

/// [OPUS-4.8] sq-e5atd — run ONLY the genuinely D-only (`sd:entailmentRegime
/// ent:D` WITHOUT any stronger RDFS/RDF/OWL-RDF-Based regime) tests of the
/// `sparql11/entailment` suite, through the real `Profile::D` materializer. This
/// is the DEDICATED D-regime lane the `tests/d_entail_suite.rs` ratchet asserts a
/// floor over; it is deliberately a SUBSET of `run_suite` (the stronger-regime
/// tests are exercised under their own regime there). Opt-in, `d-entail`.
#[cfg(feature = "d-entail")]
pub fn run_d_regime_suite(rdf_tests_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let suites_root = rdf_tests_root
        .join("sparql")
        .canonicalize()
        .map_err(|e| format!("rdf-tests sparql dir: {e}"))?;
    let manifest_path = suites_root.join("sparql11/entailment/manifest.ttl");
    let mut entries: Vec<TestEntry> = Vec::new();
    manifest::collect(&manifest_path, &suites_root, &mut entries)?;
    for entry in &entries {
        if entry.kind != EntryKind::QueryEval {
            continue;
        }
        let regimes: FxHashSet<&str> = entry
            .action
            .entailment_regimes
            .iter()
            .filter_map(|r| r.rsplit('/').next())
            .collect();
        // A test is D-ONLY when its only materializable regime is D — none of the
        // stronger regimes (which `run_suite` picks first) apply.
        let stronger = regimes.contains("RDFS")
            || regimes.contains("RDF")
            || regimes.contains("OWL-RDF-Based");
        if stronger || !regimes.contains("D") {
            continue;
        }
        let direct_sanctioned = regimes.contains("OWL-Direct");
        out.push(result(entry, run_one(entry, Profile::D, direct_sanctioned)));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Rdf,
    Rdfs,
    OwlRl,
    /// [OPUS-4.8] sq-e5atd — D-entailment (datatype / value-space). Opt-in,
    /// `d-entail`. Only reached for a test whose regime set is D WITHOUT any of
    /// RDFS/RDF/OWL-RDF-Based (those stronger regimes subsume D and are picked
    /// first), so this arm graduates the genuinely D-only tests.
    #[cfg(feature = "d-entail")]
    D,
}

/// The `Profile::D` for a regime set IFF the `d-entail` feature is ON and the set
/// contains the D regime; `None` otherwise (so the caller keeps the test
/// OutOfScope when the feature is off — the default binary is unchanged).
#[cfg(feature = "d-entail")]
fn d_regime_profile(regimes: &FxHashSet<&str>) -> Option<Profile> {
    regimes.contains("D").then_some(Profile::D)
}

/// Feature-OFF stub: D is never a supported profile, so a D-only test stays
/// OutOfScope. (Kept as a function so the call site is identical in both states.)
#[cfg(not(feature = "d-entail"))]
fn d_regime_profile(_regimes: &FxHashSet<&str>) -> Option<Profile> {
    None
}

fn result(entry: &TestEntry, outcome: Outcome) -> TestResult {
    TestResult {
        suite: "sparql11/entailment".into(),
        name: entry.name.clone(),
        outcome,
    }
}

fn run_one(entry: &TestEntry, profile: Profile, direct_sanctioned: bool) -> Outcome {
    // [OPUS-4.8] sq-oy1f — the `sparql11/entailment` manifest at the pinned
    // rdf-tests revision contains NO `qt:graphData` entries (every entailment test
    // uses a single default-graph `qt:data` dataset), so this guard is currently
    // unreachable: there are zero named-graph entailment cases to graduate. It is
    // RETAINED as a fail-closed safeguard — should a future suite bump add a
    // named-graph entailment test, it would be reported OutOfScope (honest) rather
    // than silently materialized through the default-graph-only path below, which
    // would give a WRONG closure (the named-graph dataset would need per-graph
    // entailment semantics in sparq-reason, not just harness wiring). Wiring that
    // properly is tracked as a deferred bead; do NOT drop the guard to "make it run"
    // until the reasoner models named-graph materialization.
    //
    // [SONNET-4.6] sq-6qpyf — that "zero cases at the pin" census is no longer a
    // prose claim: `tests/named_graph_entailment_census.rs` ASSERTS it, and goes red
    // the moment a suite bump introduces one (the tripwire that says "now go do the
    // per-graph materialization work in sparq-reason", epic sq-pbz04). Both arms of
    // this guard — held when named, fall-through when not — are pinned directly by
    // `mod named_graph_guard_tests` at the foot of this file.
    if !entry.action.graph_data.is_empty() {
        return Outcome::OutOfScope("named-graph entailment dataset not wired".into());
    }
    // Load and materialize the default-graph data. The blank-node labels of
    // the ORIGINAL data are recorded before materialization: condition (C1)'s
    // Skolemization function is defined exactly for the blank nodes of the
    // queried graph, so they delimit the bnode bindings that can be answers.
    let mut dict = Dict::new();
    let mut ids: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
    let mut data_bnodes: FxHashSet<String> = FxHashSet::default();
    for d in &entry.action.data {
        match crate::rdf::parse_file(d) {
            Ok(triples) => {
                for t in triples {
                    let row = [
                        Term::from(t.subject),
                        Term::NamedNode(t.predicate),
                        t.object,
                    ];
                    for term in &row {
                        if let Term::BlankNode(b) = term {
                            data_bnodes.insert(b.as_str().to_string());
                        }
                    }
                    ids.push([dict.intern(&row[0]), dict.intern(&row[1]), dict.intern(&row[2])]);
                }
            }
            Err(e) => return Outcome::Fail(format!("data parse error: {e}")),
        }
    }
    match profile {
        Profile::Rdfs => {
            add_declared_reflexives(&mut dict, &mut ids, false);
            sparq_reason::materialize(sparq_reason::Profile::Rdfs, &mut dict, &mut ids);
            add_rdfd2(&mut dict, &mut ids);
        }
        Profile::Rdf => add_rdfd2(&mut dict, &mut ids),
        Profile::OwlRl => {
            add_declared_reflexives(&mut dict, &mut ids, true);
            sparq_reason::materialize(sparq_reason::Profile::OwlRl, &mut dict, &mut ids);
            add_eq_ref(&mut dict, &mut ids);
        }
        // [OPUS-4.8] sq-e5atd — D-entailment: the rdfD1 datatype-typing closure
        // under the STANDARD recognized datatype map (this suite's D tests do not
        // restrict D via a per-test set). The emitted typing triples are
        // GENERALIZED (literal subject `"l"^^d rdf:type d`); the N-Triples
        // serialization below DROPS literal-subject rows (they can never be a
        // SPARQL answer — the regime answer restriction, also why `d-ent-01`
        // correctly returns NO rows), so D-entailment adds no bindable triple but
        // is computed through the real `Profile::D` materializer.
        #[cfg(feature = "d-entail")]
        Profile::D => {
            sparq_reason::materialize(sparq_reason::Profile::D, &mut dict, &mut ids);
        }
    }

    // Serialize the closure as N-Triples (default graph). Generalized triples
    // (literal subjects from rdfs3 range typing) cannot appear in any SPARQL
    // answer and the loader would reject them.
    let mut nquads = String::new();
    let mut seen: FxHashSet<[sparq_core::dict::Id; 3]> = FxHashSet::default();
    for t in &ids {
        if !seen.insert(*t) {
            continue;
        }
        let (s, p, o) = (dict.term(t[0]), dict.term(t[1]), dict.term(t[2]));
        if matches!(s, Term::Literal(_)) {
            continue;
        }
        nquads.push_str(&format!("{s} {p} {o} .\n"));
    }

    // The regimes' answer restriction on the closure's solutions:
    //  - every regime (C1, Entailment Regimes §2/§3.1): bindings to blank
    //    nodes NOT in the queried graph (saturation-introduced) are never
    //    answers — sk is undefined for them, so sk(P(BGP)) is not ground;
    //  - tests whose expected answers are also sanctioned under OWL-Direct
    //    (§7): a variable in a class/property-NAME position cannot bind to an
    //    anonymous class expression (a blank node is not a name).
    let filter = EntailmentAnswerFilter {
        data_bnodes,
        name_position_vars: if direct_sanctioned {
            name_position_vars(entry)
        } else {
            FxHashSet::default()
        },
    };

    match run::run_query_test_filtered(entry, Some(nquads), Some(&filter)) {
        Status::Pass => Outcome::Pass,
        Status::Fail(e) => Outcome::Fail(e),
        Status::Skip(e) => Outcome::OutOfScope(e),
    }
}

/// Variables of the query's BGPs that stand in class-name or property-name
/// positions (Entailment Regimes §7: under the OWL 2 Direct Semantics regime
/// variables stand "in place of class names, object property names, datatype
/// property names, individual names, or literals"; the extended grammar of
/// §7.1.2 substitutes them for the *name* productions, so an anonymous class
/// expression — a blank node — is not a legal binding for them). Detection is
/// positional over the schema vocabulary; property paths don't occur in the
/// entailment suite's queries.
fn name_position_vars(entry: &TestEntry) -> FxHashSet<String> {
    let mut vars = FxHashSet::default();
    let Some(query_path) = &entry.action.query else {
        return vars;
    };
    let Ok(query_text) = std::fs::read_to_string(query_path) else {
        return vars;
    };
    let base = crate::rdf::file_iri(query_path);
    let Ok(parser) = SparqlParser::new().with_base_iri(&base) else {
        return vars;
    };
    let pattern = match parser.parse_query(&query_text) {
        Ok(Query::Select { pattern, .. }) | Ok(Query::Ask { pattern, .. }) => pattern,
        _ => return vars,
    };
    collect_name_position_vars(&pattern, &mut vars);
    vars
}

const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

fn collect_name_position_vars(p: &GraphPattern, vars: &mut FxHashSet<String>) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                triple_name_positions(tp, vars);
            }
        }
        GraphPattern::Path { .. } | GraphPattern::Values { .. } => {}
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            collect_name_position_vars(left, vars);
            collect_name_position_vars(right, vars);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => collect_name_position_vars(inner, vars),
    }
}

fn triple_name_positions(tp: &TriplePattern, vars: &mut FxHashSet<String>) {
    // A variable in predicate position is a property-name position.
    let NamedNodePattern::NamedNode(pred) = &tp.predicate else {
        if let NamedNodePattern::Variable(v) = &tp.predicate {
            vars.insert(v.as_str().to_string());
        }
        return;
    };
    let pred = pred.as_str();
    let mut grab = |t: &TermPattern| {
        if let TermPattern::Variable(v) = t {
            vars.insert(v.as_str().to_string());
        }
    };
    // Object is a class-name position.
    if pred == rdf::TYPE.as_str()
        || pred == format!("{RDFS}domain")
        || pred == format!("{RDFS}range")
        || pred == format!("{OWL}someValuesFrom")
        || pred == format!("{OWL}allValuesFrom")
        || pred == format!("{OWL}onClass")
    {
        grab(&tp.object);
    }
    // Both ends are class-name positions.
    if pred == format!("{RDFS}subClassOf")
        || pred == format!("{OWL}equivalentClass")
        || pred == format!("{OWL}disjointWith")
        || pred == format!("{OWL}complementOf")
    {
        grab(&tp.subject);
        grab(&tp.object);
    }
    // Subject is a property-name position.
    if pred == format!("{RDFS}domain") || pred == format!("{RDFS}range") {
        grab(&tp.subject);
    }
    // Both ends are property-name positions.
    if pred == format!("{RDFS}subPropertyOf")
        || pred == format!("{OWL}equivalentProperty")
        || pred == format!("{OWL}propertyDisjointWith")
        || pred == format!("{OWL}inverseOf")
    {
        grab(&tp.subject);
        grab(&tp.object);
    }
    // Object is a property-name position.
    if pred == format!("{OWL}onProperty") {
        grab(&tp.object);
    }
}

/// eq-ref (OWL 2 Profiles §4.3, Table 4): `T(?s ?p ?o) → T(?s owl:sameAs ?s),
/// T(?p owl:sameAs ?p), T(?o owl:sameAs ?o)`. The production materializer
/// deliberately omits the full reflexive owl:sameAs layer (store bloat, like
/// the declared reflexives above), but the regime's answers need it — e.g.
/// sparqldl-10 walks `?a owl:sameAs ?b` over individuals with NO asserted
/// sameAs. Literal subjects would be generalized triples and are skipped.
fn add_eq_ref(dict: &mut Dict, ids: &mut Vec<[sparq_core::dict::Id; 3]>) {
    let same_as = dict.intern_iri("http://www.w3.org/2002/07/owl#sameAs");
    let mut terms: FxHashSet<sparq_core::dict::Id> = FxHashSet::default();
    for &[s, p, o] in ids.iter() {
        terms.insert(s);
        terms.insert(p);
        terms.insert(o);
    }
    for t in terms {
        if !matches!(dict.term(t), Term::Literal(_)) {
            ids.push([t, same_as, t]);
        }
    }
}

/// The finite-vocabulary reflexive layer the entailment regimes expect of
/// DECLARED terms (the production materializer omits all reflexives as store
/// bloat): every declared class is its own subclass (rdfs10/scm-cls), every
/// declared property its own subproperty (rdfs6/scm-op), and — under the OWL
/// regimes — every declared named individual an `owl:Thing`.
fn add_declared_reflexives(
    dict: &mut Dict,
    ids: &mut Vec<[sparq_core::dict::Id; 3]>,
    owl: bool,
) {
    let ty = dict.intern_iri(rdf::TYPE.as_str());
    let sc = dict.intern_iri(oxrdf::vocab::rdfs::SUB_CLASS_OF.as_str());
    let sp = dict.intern_iri(oxrdf::vocab::rdfs::SUB_PROPERTY_OF.as_str());
    let class_tys = [
        dict.intern_iri("http://www.w3.org/2002/07/owl#Class"),
        dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#Class"),
    ];
    let prop_tys = [
        dict.intern_iri("http://www.w3.org/2002/07/owl#ObjectProperty"),
        dict.intern_iri("http://www.w3.org/2002/07/owl#DatatypeProperty"),
        dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#Property"),
    ];
    let named_ind = dict.intern_iri("http://www.w3.org/2002/07/owl#NamedIndividual");
    let thing = dict.intern_iri("http://www.w3.org/2002/07/owl#Thing");
    let nothing = dict.intern_iri("http://www.w3.org/2002/07/owl#Nothing");
    let mut add: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
    for &[s, p, o] in ids.iter() {
        if p == sc || p == sp {
            // rdfs6/10 via the axiomatic domain/range of subClassOf and
            // subPropertyOf: both endpoints are classes/properties.
            add.push([s, p, s]);
            add.push([o, p, o]);
        }
        if p != ty {
            continue;
        }
        if class_tys.contains(&o) {
            add.push([s, sc, s]);
            if owl {
                // scm-cls: every class is ⊑ owl:Thing and ⊒ owl:Nothing.
                add.push([s, sc, thing]);
                add.push([nothing, sc, s]);
            }
        } else if prop_tys.contains(&o) {
            add.push([s, sp, s]);
        } else if owl && o == named_ind {
            add.push([s, ty, thing]);
        }
    }
    ids.extend(add);
}

/// rdfD2: every asserted predicate is an `rdf:Property` — part of the RDF (and
/// RDFS) regime vocabulary the production materializer omits as store bloat.
fn add_rdfd2(dict: &mut Dict, ids: &mut Vec<[sparq_core::dict::Id; 3]>) {
    let ty = dict.intern_iri(rdf::TYPE.as_str());
    let property = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#Property");
    let preds: FxHashSet<_> = ids.iter().map(|t| t[1]).collect();
    for p in preds {
        ids.push([p, ty, property]);
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-kuvu3 (epic sq-pbz04) — the EXPERIMENTAL OWL 2 QL (DL-Lite_R)
// query-rewriting arm. Opt-in, `ql-experimental`.
// ---------------------------------------------------------------------------

/// The OWL profile IRI a test must list under `sd:EntailmentProfile` for the QL
/// arm to attempt it.
#[cfg(feature = "ql-experimental")]
const PR_QL: &str = "http://www.w3.org/ns/owl-profile/QL";

// ---------------------------------------------------------------------------
// [FABLE-5] sq-pbz04.3.4 (epic sq-pbz04.3, design record
// research/owl2-ql-cq-gate-broadening.md §4+§6) — the SIX-CONDITION GRADUATION
// PREDICATE for the `pr:QL` entailment arm, plus the HONEST REASON TAXONOMY for
// everything it holds back. A row graduates to the pinned named-case floor
// (`tests/ql_entailment_floor.rs`, a `sparq extension` scoreboard row NEVER
// summed into the standards-conformance total) iff ALL SIX conditions below are
// CHECKED — in code, never assumed — and pass. A graduated case that is not
// actually sound is the worst failure mode; an honest hold is always acceptable,
// so every check is fail-closed and every hold carries a specific reason.
// ---------------------------------------------------------------------------

/// [FABLE-5] sq-pbz04.3.4 — why a `pr:QL` entailment row did NOT graduate to the
/// pinned named-case floor. The taxonomy is EXHAUSTIVE by construction: every
/// non-graduated case carries exactly one variant, and the only "we could not
/// classify it" variant ([`QlHoldReason::UnclassifiedAbstain`]) is asserted EMPTY
/// over the current corpus by `tests/ql_entailment_floor.rs`, so a new rewriter
/// abstain class can never hide inside a catch-all.
#[cfg(feature = "ql-experimental")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QlHoldReason {
    /// No sound certain-answer UCQ rewriting for this shape exists in this design
    /// (design record §5): BIND/aggregation, variable predicates, intensional
    /// schema-vocabulary queries, OPTIONAL/MINUS, recursive or negated property
    /// paths, GRAPH, sub-SELECT, SERVICE, RDF-star, CONSTRUCT/DESCRIBE. A
    /// documented divergence — the case will never graduate under this design.
    PermanentlyOutside(String),
    /// Held at the CQ-shape gate on a shape scheduled for sound broadening
    /// (design record §5 B1–B5: UCQ/UNION input, literal-object role atoms,
    /// distinguished-only FILTER, constant VALUES, non-recursive path
    /// desugaring) or a soundly-liftable rewriter-fragment gap (a blank-node
    /// body term is just a non-distinguished variable). May graduate once the
    /// broadening lands AND the remaining five conditions hold.
    PendingGate(String),
    /// Condition (2) failed: the TBox is NOT totally captured —
    /// `TBox::fully_captured()` is false (axioms skipped as outside the DL-Lite_R
    /// fragment, or unrecognised schema vocabulary), so the rewrite may be
    /// silently incomplete for this TBox.
    PendingCapture {
        /// Axioms recognised-but-skipped as outside the DL-Lite_R fragment.
        skipped: usize,
        /// OWL/RDFS constructs the extractor does not model.
        unrecognised_schema: usize,
    },
    /// Condition (3) failed: the TBox carries consistency-relevant (negative /
    /// disjointness) axioms and the DL-Lite_R consistency check (sq-p6yb7,
    /// `sparq_reason_ql::check_consistency_with`, opt-in `ql-consistency`)
    /// returned UNKNOWN — some negative axiom was not structurally captured
    /// (e.g. `owl:complementOf`), so an inconsistency could go unseen and the
    /// UCQ answers may under-approximate. Fail-closed hold. (Before sq-p6yb7
    /// this variant held EVERY negative-axiom TBox; a KB the check proves
    /// CONSISTENT now passes condition (3), and a KB it proves INCONSISTENT
    /// holds as [`QlHoldReason::InconsistentKb`].)
    PendingConsistency {
        /// Count of negative/disjointness axioms in the TBox.
        negative_axioms: usize,
    },
    /// Condition (3) failed: the DL-Lite_R consistency check (sq-p6yb7) proved
    /// the KB INCONSISTENT — a definitive verdict (everything is entailed under
    /// certain-answer semantics), but the W3C entailment-regime spec leaves a
    /// query on an inconsistent graph to the system (it MAY raise an error), so
    /// there is no spec-pinned oracle semantics to graduate against. Fail-closed
    /// hold — never a guessed everything-is-entailed pass. [FABLE-5] sq-p6yb7
    InconsistentKb {
        /// The violated negative inclusion (Display form) — the witness.
        violated: String,
    },
    /// Condition (4) failed: the test uses a named-graph (`qt:graphData`)
    /// dataset, or the query carries its own `FROM`/`FROM NAMED` dataset clause
    /// (which the CQ-shape gate DROPS rather than honours — so it must be held
    /// here, never silently ignored); per-graph rewriting semantics are not
    /// wired (design record §8).
    NamedGraphDataset,
    /// Condition (5) failed: the regime-coincidence guard — the crate computes
    /// CERTAIN ANSWERS (an anonymous existential witness can support an answer)
    /// while W3C entailment-regime solution mappings bind every variable to an
    /// RDF term. Held when a non-distinguished body position could be filled by
    /// an anonymous witness (see `ql_regime_coincides` for the written argument).
    PendingCoincidence(String),
    /// Condition (6) failed: conditions (1)–(5) held a priori, but the rewritten
    /// UCQ's evaluation over the unmodified data was NOT result-equivalent to
    /// the W3C oracle — an honest gap (or a bug), never laundered into a pass.
    OracleDivergent {
        /// Minimised UCQ size of the rewrite that diverged.
        disjuncts: usize,
        /// The observed mismatch.
        detail: String,
    },
    /// The rewriter abstained with a reason string this taxonomy does not yet
    /// classify. Fail-closed (the case is held), but NEVER silently tolerated:
    /// the pinned-floor test asserts this bucket is EMPTY, so a new abstain
    /// class forces an explicit taxonomy decision instead of hiding.
    UnclassifiedAbstain(String),
    /// The harness could not set the case up (parse error, missing result file,
    /// rewriter panic/timeout) — distinct from an abstain, and never a pass.
    Inconclusive(String),
}

#[cfg(feature = "ql-experimental")]
impl QlHoldReason {
    /// The stable taxonomy bucket token (greppable in reports; asserted by the
    /// floor + arm tests). One token per variant — no catch-all.
    pub fn label(&self) -> &'static str {
        match self {
            QlHoldReason::PermanentlyOutside(_) => "permanently-outside",
            QlHoldReason::PendingGate(_) => "pending-gate",
            QlHoldReason::PendingCapture { .. } => "pending-capture",
            QlHoldReason::PendingConsistency { .. } => "pending-consistency",
            QlHoldReason::InconsistentKb { .. } => "inconsistent-kb",
            QlHoldReason::NamedGraphDataset => "named-graph-dataset",
            QlHoldReason::PendingCoincidence(_) => "pending-coincidence",
            QlHoldReason::OracleDivergent { .. } => "oracle-divergent",
            QlHoldReason::UnclassifiedAbstain(_) => "unclassified-abstain",
            QlHoldReason::Inconclusive(_) => "inconclusive",
        }
    }

    /// The single-line `OutOfScope` reason a held row is reported under. The
    /// stable `QL experimental (` prefix keeps every held row in the experimental
    /// histogram bucket (visibly NOT a conformance claim); the taxonomy label
    /// names the specific hold. Positional `format!` args per the CodeQL
    /// `rust/unused-variable` guard in the shared agent contract.
    pub fn reason(&self) -> String {
        let detail = match self {
            QlHoldReason::PermanentlyOutside(why) => {
                format!("no sound rewriting in this design: {}", why)
            }
            QlHoldReason::PendingGate(why) => {
                format!("held at the CQ-shape gate, fail-closed: {}", why)
            }
            QlHoldReason::PendingCapture { skipped, unrecognised_schema } => format!(
                "TBox not totally captured ({} skipped, {} unrecognised schema) — \
                 the rewrite may be incomplete for this TBox",
                skipped, unrecognised_schema
            ),
            QlHoldReason::PendingConsistency { negative_axioms } => format!(
                "TBox carries {} negative/disjointness axiom(s) and the DL-Lite_R \
                 consistency check returned UNKNOWN (structurally-uncaptured negative \
                 axiom) — certain answers may be under-approximated",
                negative_axioms
            ),
            QlHoldReason::InconsistentKb { violated } => format!(
                "KB proven INCONSISTENT (violated negative inclusion: {}) — the \
                 entailment-regime behaviour on an inconsistent graph is \
                 implementation-defined, so the case is held, never an \
                 everything-is-entailed pass",
                violated
            ),
            QlHoldReason::NamedGraphDataset => {
                "named-graph or query-carried (FROM) QL dataset not wired (per-graph \
                 rewriting semantics unsettled)"
                    .to_string()
            }
            QlHoldReason::PendingCoincidence(why) => format!(
                "certain-answer vs entailment-regime semantics may diverge: {}",
                why
            ),
            QlHoldReason::OracleDivergent { disjuncts, detail } => format!(
                "rewritten UCQ ({} disjunct(s)) DIVERGED from the W3C oracle: {}",
                disjuncts, detail
            ),
            QlHoldReason::UnclassifiedAbstain(why) => {
                format!("rewriter abstain not yet classified by the taxonomy: {}", why)
            }
            QlHoldReason::Inconclusive(why) => why.clone(),
        };
        format!("QL experimental ({}): {}", self.label(), detail)
    }
}

/// [FABLE-5] sq-pbz04.3.4 — the per-case verdict of the six-condition graduation
/// predicate.
#[cfg(feature = "ql-experimental")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QlGraduationVerdict {
    /// ALL SIX conditions passed: (1) the fail-closed CQ-shape gate accepts the
    /// query AND it carries no intensional schema-vocabulary atom; (2) the TBox
    /// is totally captured (`fully_captured()`); (3) zero consistency-relevant
    /// axioms; (4) default-graph dataset only; (5) the regime-coincidence guard
    /// holds; (6) the rewritten UCQ's evaluation over the unmodified data is
    /// result-equivalent to the W3C oracle. Eligible for the pinned named-case
    /// floor (a sparq-extension row, never a standards-conformance claim).
    Graduated {
        /// Minimised UCQ size of the sound rewrite.
        disjuncts: usize,
    },
    /// At least one condition failed; carries the specific hold reason.
    Held(QlHoldReason),
}

/// One `pr:QL` entailment case with its graduation verdict.
#[cfg(feature = "ql-experimental")]
#[derive(Clone, Debug)]
pub struct QlCaseVerdict {
    /// The STABLE case id — the fragment of the manifest entry IRI (unique across
    /// the suite, unlike `name`: the suite reuses e.g. "RDFS inference test
    /// subClassOf" for two distinct entries). The pinned floor is named over these.
    pub id: String,
    /// The human-readable manifest name (NOT unique).
    pub name: String,
    /// The six-condition verdict.
    pub verdict: QlGraduationVerdict,
}

/// The `OutOfScope` reason a GRADUATED row is reported under in the inference
/// binary. The binary keeps graduated rows out of its own pass/fail tally (the
/// D-entailment precedent: graduation lives in the dedicated feature-gated
/// crate-test lane, `tests/ql_entailment_floor.rs`, whose named-case ratchet is
/// the actual enforcement) — so no QL row can ever inflate the binary's
/// standards-conformance ratchet.
#[cfg(feature = "ql-experimental")]
pub fn ql_graduated_reason(disjuncts: usize) -> String {
    format!(
        "QL graduated (six-condition predicate, sq-pbz04.3.4): rewritten UCQ \
         ({} disjunct(s)) result-equivalent to the W3C oracle; pinned by the \
         ql_entailment_floor named-case ratchet as a sparq-extension row, NEVER \
         summed into the standards-conformance total",
        disjuncts
    )
}

/// [FABLE-5] sq-pbz04.3.4 — run the SIX-CONDITION GRADUATION PREDICATE over every
/// `pr:QL`-tagged test of the `sparql11/entailment` suite and return one
/// [`QlCaseVerdict`] per case, in manifest order. This is the single computation
/// both consumers share: `tests/ql_entailment_floor.rs` pins the graduated ids as
/// the named-case ratchet, and [`run_ql_experimental_arm`] renders the verdicts as
/// the inference binary's (still all-`OutOfScope`) QL rows.
#[cfg(feature = "ql-experimental")]
pub fn run_ql_graduation(rdf_tests_root: &Path) -> Result<Vec<QlCaseVerdict>, String> {
    let suites_root = rdf_tests_root
        .join("sparql")
        .canonicalize()
        .map_err(|e| format!("rdf-tests sparql dir: {e}"))?;
    let manifest_path = suites_root.join("sparql11/entailment/manifest.ttl");
    let mut entries: Vec<TestEntry> = Vec::new();
    manifest::collect(&manifest_path, &suites_root, &mut entries)?;

    let mut out = Vec::new();
    for entry in &entries {
        if entry.kind != EntryKind::QueryEval {
            continue;
        }
        // Select only the cases whose expected answers are sanctioned under OWL 2
        // QL (`sd:EntailmentProfile` lists `pr:QL`). Everything else is left to the
        // gating regime arms in `run_suite`.
        if !entry.action.entailment_profiles.iter().any(|p| p == PR_QL) {
            continue;
        }
        out.push(QlCaseVerdict {
            id: case_fragment(&entry.id),
            name: entry.name.clone(),
            verdict: ql_graduation_one(entry),
        });
    }
    Ok(out)
}

/// [OPUS-4.8] sq-kuvu3 / [FABLE-5] sq-pbz04.3.4 — run the OWL 2 QL arm over the
/// `pr:QL`-tagged tests of the `sparql11/entailment` suite and append one
/// `OutOfScope` `TestResult` per case.
///
/// HONESTY CONTRACT (the load-bearing invariant this arm preserves):
/// * NO BINARY-RATCHET GRADUATION — every row (graduated or held) lands in the
///   `Outcome::OutOfScope` bucket of the inference BINARY, so no QL row can ever
///   count toward the binary's standards-conformance pass rate or ratchet floor.
///   The actual graduation lives in the dedicated feature-gated crate-test lane
///   (`tests/ql_entailment_floor.rs`, the D-entailment precedent) whose pinned
///   named-case list is a `sparq extension` scoreboard row, tallied separately.
/// * FAIL-CLOSED PRESERVED — a query the rewriter cannot soundly rewrite is held
///   with its taxonomy reason, never a guessed pass.
/// * NO FAKED PASSES — a row reads `QL graduated` only when ALL SIX graduation
///   conditions genuinely passed (including empirical oracle result-equivalence);
///   every other row carries its specific `QL experimental (<taxonomy>)` hold.
///
/// The suite key is `sparql11/entailment (QL experimental)` so these rows are
/// visibly separate from the gating regime rows.
#[cfg(feature = "ql-experimental")]
pub fn run_ql_experimental_arm(
    rdf_tests_root: &Path,
    out: &mut Vec<TestResult>,
) -> Result<(), String> {
    for case in run_ql_graduation(rdf_tests_root)? {
        let reason = match &case.verdict {
            QlGraduationVerdict::Graduated { disjuncts } => ql_graduated_reason(*disjuncts),
            QlGraduationVerdict::Held(hold) => hold.reason(),
        };
        out.push(TestResult {
            suite: "sparql11/entailment (QL experimental)".into(),
            name: case.name,
            // EVERY QL row is OutOfScope in the binary — never a binary-ratchet pass.
            outcome: Outcome::OutOfScope(reason),
        });
    }
    Ok(())
}

/// Notes rendered under the QL experimental section (when the arm is wired into a
/// report). Honest scoping: only the six-condition-sound subset graduates, and it
/// graduates into a separate sparq-extension ratchet — never a conformance total.
#[cfg(feature = "ql-experimental")]
pub fn ql_experimental_notes() -> Vec<String> {
    vec![
        "OWL 2 QL (DL-Lite_R) query-rewriting arm (sq-kuvu3 + sq-pbz04.3.4, opt-in \
         `ql-experimental`). Each `sd:EntailmentProfile pr:QL` case runs through a \
         SIX-CONDITION graduation predicate — every condition CHECKED in code: (1) the \
         fail-closed CQ-shape gate accepts the query and it carries no intensional \
         schema-vocabulary atom; (2) the DL-Lite_R TBox is totally captured \
         (fully_captured()); (3) the consistency condition — zero consistency-relevant \
         (negative/disjointness) axioms, OR the sq-p6yb7 DL-Lite_R violation-query \
         consistency check proves the KB CONSISTENT (an INCONSISTENT or UNKNOWN verdict \
         holds, fail-closed); (4) default-graph dataset only; (5) the regime-coincidence guard \
         holds (all body terms distinguished, or no existential-generating \
         inclusions — otherwise the crate's CERTAIN-ANSWER semantics may diverge \
         from the W3C entailment-regime solution-mapping semantics); (6) the \
         rewritten UCQ (`sparq_reason_ql::rewrite_production`: PerfectRef ∪ bounded \
         tree-witness ∪ UCQ-containment minimisation), evaluated over the UNMODIFIED \
         data, is result-equivalent to the suite oracle. Cases passing ALL SIX are \
         pinned by name in the `ql_entailment_floor` ratchet — a sparq-EXTENSION row, \
         NEVER summed into the standards-conformance total; every other case is held \
         with an exhaustive taxonomy reason (permanently-outside / pending-gate / \
         pending-capture / pending-consistency / inconsistent-kb / pending-coincidence / \
         oracle-divergent / inconclusive), never a guessed pass. In THIS binary every \
         QL row (graduated or held) stays in the out-of-scope bucket, so no QL row \
         counts toward the binary's conformance ratchet."
            .to_string(),
    ]
}

/// The stable per-case id: the fragment (after `#`, else the last `/` segment) of
/// the manifest entry IRI. Unique across the `pr:QL` corpus (asserted by the
/// floor test), unlike `mf:name`.
#[cfg(feature = "ql-experimental")]
fn case_fragment(id: &str) -> String {
    match id.rsplit_once('#') {
        Some((_, frag)) if !frag.is_empty() => frag.to_string(),
        _ => id.rsplit('/').next().unwrap_or(id).to_string(),
    }
}

/// Apply the six-condition graduation predicate to ONE `pr:QL` test. Conditions
/// are checked fail-closed and first-fail names the hold reason; the check order
/// is (4) dataset — it determines how the rest load — then (1) gate + intensional
/// guard, (2) TBox capture, (3) consistency, (5) regime coincidence, (6) oracle
/// result-equivalence. The watchdog mirrors the sibling runners so a rewriter
/// hang/panic becomes a recorded hold, not a dead harness.
#[cfg(feature = "ql-experimental")]
fn ql_graduation_one(entry: &TestEntry) -> QlGraduationVerdict {
    use QlGraduationVerdict::Held;

    // CONDITION (4) — default-graph dataset only. A named-graph dataset would
    // need per-graph rewriting semantics (design record §8), so hold rather than
    // silently rewrite over the wrong TBox.
    if !entry.action.graph_data.is_empty() {
        return Held(QlHoldReason::NamedGraphDataset);
    }
    let Some(query_path) = &entry.action.query else {
        return Held(QlHoldReason::Inconclusive("manifest entry has no qt:query".into()));
    };
    let Some(result_path) = &entry.result_file else {
        return Held(QlHoldReason::Inconclusive("manifest entry has no mf:result".into()));
    };

    let query_text = match std::fs::read_to_string(query_path) {
        Ok(t) => t,
        // Positional `format!` args (not inline `{e}`) per the CodeQL
        // `rust/unused-variable` false-positive guard in the shared agent contract.
        Err(e) => return Held(QlHoldReason::Inconclusive(format!("read query: {}", e))),
    };
    let base = crate::rdf::file_iri(query_path);
    let parser = match SparqlParser::new().with_base_iri(&base) {
        Ok(p) => p,
        Err(e) => return Held(QlHoldReason::Inconclusive(format!("bad base IRI: {}", e))),
    };
    let query = match parser.parse_query(&query_text) {
        Ok(q) => q,
        Err(e) => return Held(QlHoldReason::Inconclusive(format!("query parse error: {}", e))),
    };
    // CONDITION (4, continued) — the query must not carry its own `FROM`/`FROM
    // NAMED` dataset clause either: the CQ-shape gate DROPS `dataset` rather
    // than rejecting it, so an overriding dataset would otherwise be silently
    // ignored and the rewrite evaluated over the wrong graph. No current
    // `pr:QL` case carries one (verified over the pinned rdf-tests revision);
    // this keeps condition (4) complete against future re-pins. [FABLE-5]
    if query_carries_dataset(&query) {
        return Held(QlHoldReason::NamedGraphDataset);
    }

    // CONDITION (1a) — the REAL fail-closed CQ-shape gate (never re-implemented
    // here: graduation calls the same `as_conjunctive_query` every rewriting
    // entry point runs). A rejection is classified into the taxonomy.
    let cq = match sparq_reason_ql::as_conjunctive_query(&query) {
        Ok(cq) => cq,
        Err(sparq_reason_ql::CqError::OutOfScope(reason)) => {
            return Held(classify_gate_rejection(&query, &reason));
        }
    };
    // CONDITION (1b) — the intensional-atom guard (design record §5 B6). The
    // gate currently ADMITS a role atom whose predicate is semantics-bearing
    // schema vocabulary (e.g. `?c rdfs:subClassOf ex:Student`), and the rewriter
    // then evaluates it over ASSERTED triples only — silently missing
    // TBox-entailed schema facts. Until the gate itself carries the guard
    // (sq-pbz04.3.1), graduation checks it here: an intensional case is
    // permanently outside this design's sound rewriting.
    if let Some(what) = intensional_atom(&cq) {
        return Held(QlHoldReason::PermanentlyOutside(format!(
            "intensional/schema-vocabulary atom ({}) — the rewriter would evaluate \
             it over asserted triples only, missing TBox-entailed schema facts",
            what
        )));
    }

    // Load the test's data triples — these carry BOTH the ABox and the DL-Lite_R
    // TBox the rewriter extracts from.
    let mut data: Vec<oxrdf::Triple> = Vec::new();
    for d in &entry.action.data {
        match crate::rdf::parse_file(d) {
            Ok(triples) => data.extend(triples),
            Err(e) => {
                return Held(QlHoldReason::Inconclusive(format!("data parse error: {}", e)))
            }
        }
    }

    // CONDITIONS (2) + (3) — total TBox capture, then the consistency condition,
    // on the SAME extraction the rewriter performs (sq-pbz04.3.3's accounting:
    // `skipped == 0 && unrecognised_schema == 0` is the decisive "nothing was
    // missed" signal). Condition (3) was UPGRADED by sq-p6yb7: a TBox with
    // negative/disjointness axioms no longer holds unconditionally — the
    // DL-Lite_R consistency check (violation-query composition) runs, and only
    // an Unknown (uncaptured negative axiom) or Inconsistent verdict holds.
    let tbox = sparq_reason_ql::TBox::extract(&data);
    if !tbox.fully_captured() {
        return Held(QlHoldReason::PendingCapture {
            skipped: tbox.skipped,
            unrecognised_schema: tbox.unrecognised_schema,
        });
    }
    if let Err(hold) = ql_condition3_consistency(&tbox, &data) {
        return Held(hold);
    }

    // CONDITION (5) — the regime-coincidence guard (design record §4).
    if let Err(why) = ql_regime_coincides(&cq, &tbox) {
        return Held(QlHoldReason::PendingCoincidence(why));
    }

    // CONDITION (6) — the empirical pin: rewrite under a watchdog, evaluate the
    // UCQ over the UNMODIFIED data through the real engine, compare to the W3C
    // oracle. A rewriter abstain here (the gate passed but the atom mapping is
    // outside the fragment, e.g. a literal-object or blank-node term) is
    // classified into the taxonomy.
    let q_for_thread = query.clone();
    let tbox_triples = data;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(move || {
            sparq_reason_ql::rewrite_production(&q_for_thread, &tbox_triples)
        });
        let _ = tx.send(result);
    });
    let rewritten = match rx.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(Ok(Ok(r))) => r,
        // FAIL-CLOSED: the rewriter rejected the query past the gate — hold,
        // never a guessed answer.
        Ok(Ok(Err(sparq_reason_ql::CqError::OutOfScope(reason)))) => {
            return Held(classify_rewrite_abstain(&reason));
        }
        Ok(Err(_)) => return Held(QlHoldReason::Inconclusive("rewriter panicked".into())),
        Err(_) => return Held(QlHoldReason::Inconclusive("rewriter timeout (20s)".into())),
    };
    let disjuncts = rewritten.report.disjuncts;
    match ql_eval_matches_oracle(entry, &rewritten.query, result_path) {
        Ok(true) => QlGraduationVerdict::Graduated { disjuncts },
        Ok(false) => Held(QlHoldReason::OracleDivergent {
            disjuncts,
            detail: "rewritten-UCQ answers differ from the oracle".into(),
        }),
        Err(e) => Held(QlHoldReason::Inconclusive(e)),
    }
}

/// CONDITION (3) — the consistency condition, UPGRADED by sq-p6yb7 with its written argument.
///
/// The hold this condition exists for: an INCONSISTENT KB certain-answers EVERYTHING, so the
/// positive UCQ rewriting's answers under-approximate the certain answers, and blind graduation
/// against the W3C oracle would compare the wrong quantity. With zero consistency-relevant
/// axioms a DL-Lite_R KB is always satisfiable (the canonical model of the positive inclusions
/// is a model) — condition (3) passes trivially, as before.
///
/// With negative axioms present, the DL-Lite_R consistency check
/// (`sparq_reason_ql::check_consistency_with`, sq-p6yb7 — violation-query composition rewritten
/// through the SAME PerfectRef machinery the rewriter uses) now decides:
///
/// * `Consistent` — condition (3) PASSES. Written argument: the verdict is complete over the
///   fully-captured fragment (condition (2) already holds here, and `Consistent` is only
///   returned when every negative axiom was structurally captured), and for a SATISFIABLE
///   DL-Lite_R KB the certain answers of a positive (U)CQ are determined by the positive
///   inclusions alone (the canonical model is universal), which is exactly what the rewriting
///   computes — no under-approximation remains.
/// * `Inconsistent` — held as [`QlHoldReason::InconsistentKb`]. The verdict is definitive, but
///   the SPARQL 1.1 Entailment Regimes spec leaves querying an inconsistent graph to the
///   system (it MAY raise an error), so there is no spec-pinned oracle semantics to graduate
///   an everything-is-entailed answer against. Fail-closed (and empirically vacuous at the
///   pinned rdf-tests revision: the pending-consistency bucket measured ZERO before this
///   upgrade, so no current case reaches this arm either).
/// * `Unknown` — held as [`QlHoldReason::PendingConsistency`] (fail-closed: a
///   structurally-uncaptured negative axiom, e.g. `owl:complementOf`, could hide an
///   inconsistency).
#[cfg(feature = "ql-experimental")]
fn ql_condition3_consistency(
    tbox: &sparq_reason_ql::TBox,
    data: &[oxrdf::Triple],
) -> Result<(), QlHoldReason> {
    if tbox.consistency_relevant == 0 {
        return Ok(());
    }
    match sparq_reason_ql::check_consistency_with(tbox, data) {
        sparq_reason_ql::QlConsistency::Consistent => Ok(()),
        sparq_reason_ql::QlConsistency::Inconsistent(v) => Err(QlHoldReason::InconsistentKb {
            violated: v.axiom.to_string(),
        }),
        sparq_reason_ql::QlConsistency::Unknown(_) => Err(QlHoldReason::PendingConsistency {
            negative_axioms: tbox.consistency_relevant,
        }),
    }
}

/// CONDITION (5) — the regime-coincidence guard, with its written argument.
///
/// The crate computes CERTAIN ANSWERS of the CQ: non-distinguished variables are
/// existentially quantified, so an ANONYMOUS TBox-generated witness can support
/// an answer (`Person ⊑ ∃hasParent`, data `:a a :Person`, query
/// `SELECT ?x WHERE { ?x :hasParent ?y }` certain-answers `{a}` with no `?y`
/// binding in the data). The W3C SPARQL 1.1 Entailment Regimes, by contrast,
/// extend BGP matching but keep SPARQL solution mappings: the BGP's solutions
/// bind EVERY BGP variable to an RDF term (projection happens above BGP
/// matching), and condition (C1) restricts blank-node bindings to the blank
/// nodes of the queried graph — an anonymous existential witness is not a term
/// of the queried graph, so it supports NO regime solution.
///
/// The two semantics therefore provably coincide when NO answer can depend on an
/// anonymous witness, i.e. when EITHER:
/// * every body term is distinguished (a variable in the projection; a blank
///   node in the body is a non-distinguished variable and fails this arm) — then
///   every certain answer instantiates the whole BGP with real terms of the
///   data, and conversely every regime solution is a certain answer (an entailed
///   ground BGP holds in every model); OR
/// * the TBox has no existential-generating inclusion (`exists_super` empty) —
///   in DL-Lite_R anonymous individuals arise ONLY from `B ⊑ ∃R` inclusions, so
///   the canonical model adds no anonymous individual and every certain answer's
///   supporting homomorphism maps into the data's own terms.
///
/// This is the fail-closed DEFAULT of the design record §4, deliberately NOT
/// widened: any widening needs a new written argument here.
#[cfg(feature = "ql-experimental")]
fn ql_regime_coincides(
    cq: &sparq_reason_ql::ConjunctiveQuery,
    tbox: &sparq_reason_ql::TBox,
) -> Result<(), String> {
    if tbox.exists_super.is_empty() {
        return Ok(());
    }
    let distinguished: FxHashSet<&str> =
        cq.distinguished.iter().map(|v| v.as_str()).collect();
    let check = |t: &TermPattern| -> Result<(), String> {
        match t {
            TermPattern::Variable(v) if !distinguished.contains(v.as_str()) => Err(format!(
                "non-distinguished body variable ?{} with {} existential-generating \
                 inclusion(s) in the TBox — an anonymous witness could support a \
                 certain answer that is not a regime solution",
                v.as_str(),
                tbox.exists_super.len()
            )),
            // A blank node in a query body is a non-distinguished variable.
            TermPattern::BlankNode(_) => Err(format!(
                "blank-node body term (a non-distinguished variable) with {} \
                 existential-generating inclusion(s) in the TBox",
                tbox.exists_super.len()
            )),
            _ => Ok(()),
        }
    };
    for atom in &cq.atoms {
        check(&atom.subject)?;
        check(&atom.object)?;
    }
    Ok(())
}

/// CONDITION (1b) — detect an INTENSIONAL (schema-vocabulary) atom the current
/// gate still admits (design record §5 B6). Semantics-bearing positions:
/// * a constant predicate in the `rdfs:` namespace that is not one of the four
///   annotation predicates (`rdfs:label` / `comment` / `seeAlso` /
///   `isDefinedBy`, whose extensions no QL TBox axiom changes), or ANY constant
///   predicate in the `owl:` namespace (all of it is axiom/semantics-bearing
///   vocabulary under the regime — including `owl:sameAs`, which the rewriter
///   does not reason over; fail-closed);
/// * `rdf:type` whose object is a VARIABLE (a class-name-position variable — a
///   query FOR schema) or a constant class in the `rdfs:`/`owl:` namespace.
#[cfg(feature = "ql-experimental")]
fn intensional_atom(cq: &sparq_reason_ql::ConjunctiveQuery) -> Option<String> {
    const ANNOTATION: [&str; 4] = [
        "http://www.w3.org/2000/01/rdf-schema#label",
        "http://www.w3.org/2000/01/rdf-schema#comment",
        "http://www.w3.org/2000/01/rdf-schema#seeAlso",
        "http://www.w3.org/2000/01/rdf-schema#isDefinedBy",
    ];
    for atom in &cq.atoms {
        let NamedNodePattern::NamedNode(pred) = &atom.predicate else {
            // The gate already rejects variable predicates; unreachable here.
            continue;
        };
        let p = pred.as_str();
        if p == rdf::TYPE.as_str() {
            match &atom.object {
                TermPattern::Variable(v) => {
                    return Some(format!(
                        "rdf:type with class-name-position variable ?{}",
                        v.as_str()
                    ));
                }
                TermPattern::NamedNode(c)
                    if c.as_str().starts_with(RDFS) || c.as_str().starts_with(OWL) =>
                {
                    return Some(format!("rdf:type with schema-vocabulary class {}", c));
                }
                _ => {}
            }
        } else if (p.starts_with(RDFS) && !ANNOTATION.contains(&p)) || p.starts_with(OWL) {
            return Some(format!("schema-vocabulary predicate {}", pred));
        }
    }
    None
}

/// Classify a CQ-SHAPE-GATE rejection into the hold taxonomy. Shapes on the
/// design record's B1–B5 broadening path are `pending-gate`; shapes on its
/// PERMANENT-REJECT list are `permanently-outside`; anything unmatched is the
/// loud `unclassified-abstain` bucket (asserted EMPTY by the floor test).
#[cfg(feature = "ql-experimental")]
fn classify_gate_rejection(query: &Query, reason: &str) -> QlHoldReason {
    // B1 UCQ input / B3 distinguished-only FILTER / B4 constant VALUES.
    if reason.contains("UNION") {
        return QlHoldReason::PendingGate(format!("{} (B1 UCQ input not yet landed)", reason));
    }
    if reason.contains("FILTER") {
        return QlHoldReason::PendingGate(format!(
            "{} (B3 distinguished-only FILTER not yet landed)",
            reason
        ));
    }
    if reason.contains("VALUES") {
        return QlHoldReason::PendingGate(format!(
            "{} (B4 constant VALUES not yet landed)",
            reason
        ));
    }
    // B5 — only the NON-recursive path forms (`/`, `^`, `|`) are on the
    // broadening path; `+`/`*`/`?`/negated sets reintroduce the
    // recursion/reflexivity QL deliberately excludes and stay permanent.
    if reason.contains("property path") {
        return if query_paths_all_nonrecursive(query) {
            QlHoldReason::PendingGate(format!(
                "{} (B5 non-recursive path desugaring not yet landed)",
                reason
            ))
        } else {
            QlHoldReason::PermanentlyOutside(format!(
                "{} (recursive/zero-length/negated path form)",
                reason
            ))
        };
    }
    // B6 gate-side intensional-atom guard (sq-pbz04.3.1): `check_atom_shape` now emits
    // `"intensional/schema-vocabulary atom: predicate <...> ..."` for rdfs: schema
    // vocabulary and owl: predicates used as role atoms.  This is a PERMANENT-REJECT
    // (the rewriter evaluates over ABox data only; using a schema-vocabulary predicate
    // as a data atom has no sound certain-answer rewriting in this design). [SONNET-4.6]
    if reason.contains("intensional") {
        return QlHoldReason::PermanentlyOutside(reason.to_string());
    }
    // The design record §5 PERMANENT-REJECT list (+ degenerate shapes).
    const PERMANENT: [&str; 15] = [
        "OPTIONAL",
        "MINUS",
        "BIND",
        "aggregation",
        "GROUP",
        "variable predicate",
        "CONSTRUCT",
        "DESCRIBE",
        "SERVICE",
        "GRAPH",
        "LATERAL",
        "sub-SELECT",
        "DISTINCT/REDUCED",
        "RDF-star",
        "ORDER BY",
    ];
    if PERMANENT.iter().any(|k| reason.contains(k))
        || reason.contains("LIMIT/OFFSET")
        || reason.contains("empty query body")
        || reason.contains("projection")
    {
        return QlHoldReason::PermanentlyOutside(reason.to_string());
    }
    QlHoldReason::UnclassifiedAbstain(reason.to_string())
}

/// Classify a REWRITE-TIME abstain (the gate passed, the DL-Lite atom mapping
/// rejected) into the hold taxonomy.
#[cfg(feature = "ql-experimental")]
fn classify_rewrite_abstain(reason: &str) -> QlHoldReason {
    if reason.contains("literal in a class/role atom position") {
        // B2 — a constant is never an unbound position; soundly liftable.
        return QlHoldReason::PendingGate(format!(
            "{} (B2 literal-object role atoms not yet landed)",
            reason
        ));
    }
    if reason.contains("blank-node term") {
        // A blank node in a CQ body is just a fresh non-distinguished variable —
        // soundly liftable, not yet implemented by the rewriter's atom mapping.
        return QlHoldReason::PendingGate(format!(
            "{} (soundly liftable as a fresh non-distinguished variable; not yet \
             implemented)",
            reason
        ));
    }
    if reason.contains("rdf:type object must be a named class") {
        // A class-name-position variable is an intensional/schema query.
        return QlHoldReason::PermanentlyOutside(format!(
            "{} (class-name-position variable — an intensional schema query)",
            reason
        ));
    }
    // Multi-branch UCQ with per-branch FILTER/VALUES (sq-pbz04.3.1 fail-closed guard):
    // the branch-aware emitter is deferred to sq-pbz04.3.2, so this is pending-gate. The
    // reason string starts with "multi-branch UCQ with per-branch FILTER or VALUES". [SONNET-4.6]
    if reason.contains("multi-branch") && (reason.contains("FILTER") || reason.contains("VALUES")) {
        return QlHoldReason::PendingGate(format!(
            "{} (branch-aware emitter deferred to sq-pbz04.3.2)",
            reason
        ));
    }
    if reason.contains("variable predicate") || reason.contains("RDF-star") {
        return QlHoldReason::PermanentlyOutside(reason.to_string());
    }
    QlHoldReason::UnclassifiedAbstain(reason.to_string())
}

/// True iff the query carries its own `FROM`/`FROM NAMED` dataset clause
/// (condition (4): the CQ-shape gate drops `dataset` rather than honouring it,
/// so a dataset-carrying query must be HELD, never rewritten over the wrong
/// graph). [FABLE-5] sq-pbz04.3.4
#[cfg(feature = "ql-experimental")]
fn query_carries_dataset(query: &Query) -> bool {
    match query {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset.is_some(),
    }
}

/// True iff every property path in the query uses only the NON-recursive forms
/// (`NamedNode` / `^` Reverse / `/` Sequence / `|` Alternative) the B5
/// desugaring covers. `*`/`+`/`?`/negated property sets return false.
#[cfg(feature = "ql-experimental")]
fn query_paths_all_nonrecursive(query: &Query) -> bool {
    use spargebra::algebra::PropertyPathExpression as P;
    fn path_ok(p: &P) -> bool {
        match p {
            P::NamedNode(_) => true,
            P::Reverse(inner) => path_ok(inner),
            P::Sequence(a, b) | P::Alternative(a, b) => path_ok(a) && path_ok(b),
            P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) | P::NegatedPropertySet(_) => {
                false
            }
        }
    }
    fn walk(g: &GraphPattern) -> bool {
        match g {
            GraphPattern::Path { path, .. } => path_ok(path),
            GraphPattern::Bgp { .. } | GraphPattern::Values { .. } => true,
            GraphPattern::Join { left, right }
            | GraphPattern::LeftJoin { left, right, .. }
            | GraphPattern::Lateral { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => walk(left) && walk(right),
            GraphPattern::Filter { inner, .. }
            | GraphPattern::Graph { inner, .. }
            | GraphPattern::Extend { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::Group { inner, .. }
            | GraphPattern::Service { inner, .. } => walk(inner),
        }
    }
    match query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => walk(pattern),
        Query::Construct { pattern, .. } | Query::Describe { pattern, .. } => walk(pattern),
    }
}

/// Evaluate a REWRITTEN UCQ over the test's UNMODIFIED default-graph data and
/// report whether the answers are result-equivalent to the suite oracle. SELECT
/// and ASK only (the entailment suite is all SELECT/ASK). Runs the engine on a
/// watchdog thread, like the sibling runners.
#[cfg(feature = "ql-experimental")]
fn ql_eval_matches_oracle(
    entry: &TestEntry,
    rewritten: &spargebra::Query,
    result_path: &Path,
) -> Result<bool, String> {
    use crate::compare::Row;
    use crate::results::Expected;
    use std::collections::BTreeSet;

    let expected = crate::results::parse_expected(result_path)?;

    // Build the default-graph dataset from the UNMODIFIED data files.
    let mut nquads = String::new();
    for d in &entry.action.data {
        // Positional `format!` args per the CodeQL `rust/unused-variable` guard.
        for t in crate::rdf::parse_file(d).map_err(|e| format!("data parse: {}", e))? {
            nquads.push_str(&format!("{} {} {} .\n", t.subject, t.predicate, t.object));
        }
    }
    // The rewritten query serialises to standard SPARQL (a UNION-folded UCQ under
    // the original projection); run it through the engine unchanged.
    let query_str = rewritten.to_string();

    let is_ask = matches!(rewritten, spargebra::Query::Ask { .. });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let graph = sparq_core::Graph::load_dataset(&nquads, "nquads")?;
            if is_ask {
                let b = sparq_engine::ask(&graph, &query_str)?;
                Ok::<_, String>((true, b, Vec::new(), Vec::new()))
            } else {
                let res = sparq_engine::query(&graph, &query_str)?;
                let vars: Vec<String> = res.vars.iter().map(|v| v.as_str().to_string()).collect();
                Ok((false, false, vars, res.rows))
            }
        })();
        let _ = tx.send(result);
    });
    let (was_ask, ask_bool, actual_vars, actual_rows) =
        match rx.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(format!("engine error: {}", e)),
            Err(_) => return Err("engine timeout/panic (20s)".into()),
        };

    if was_ask {
        return match expected {
            Expected::Boolean(b) => Ok(ask_bool == b),
            Expected::Bindings { .. } => Err("expected bindings, rewritten query is ASK".into()),
        };
    }
    let (exp_vars, exp_rows) = match expected {
        Expected::Bindings { vars, rows, .. } => (vars, rows),
        Expected::Boolean(_) => return Err("expected boolean, rewritten query is SELECT".into()),
    };

    // Align both sides on a shared variable order, then compare as multisets (the
    // QL entailment queries are unordered).
    let mut all_vars: BTreeSet<String> = actual_vars.iter().cloned().collect();
    all_vars.extend(exp_vars.iter().cloned());
    for row in &exp_rows {
        all_vars.extend(row.iter().map(|(v, _)| v.clone()));
    }
    let order: Vec<String> = all_vars.into_iter().collect();
    let exp: Vec<Row> = exp_rows
        .iter()
        .map(|r| {
            order
                .iter()
                .map(|v| r.iter().find(|(name, _)| name == v).map(|(_, t)| t.clone()))
                .collect()
        })
        .collect();
    let act: Vec<Row> = actual_rows
        .iter()
        .map(|r| {
            order
                .iter()
                .map(|v| {
                    actual_vars
                        .iter()
                        .position(|av| av == v)
                        .and_then(|i| r.get(i).cloned().flatten())
                })
                .collect()
        })
        .collect();
    crate::compare::rows_equal(&exp, &act, false)
}

// ===========================================================================
// [OPUS-4.8] sq-qo1a9 (epic sq-pbz04) — the GRADUATED DL-Lite_R CERTAIN-ANSWER
// oracle. This is the FORMAL OWL 2 QL conformance arm that earns a pinned floor.
//
// The broad `pr:QL` `sparql11/entailment` set (run by `run_ql_experimental_arm`
// above) is NOT the formal OWL2-QL / DL-Lite_R suite: it mixes intensional /
// non-DL-Lite certain-answer cases the sound query-rewriting fragment cannot
// answer (so it stays EXPERIMENTAL / OutOfScope, with its 1 documented
// divergence). The FORMAL suite is the HAND-DERIVED DL-Lite_R oracle from
// sq-g19x0 — every case is a conjunctive query within sound rewriting, with a
// hand-checked EXACT certain-answer set. On that suite the rewrite is sound AND
// complete case by case, so it graduates.
//
// What "matches the oracle on a case" MEANS here (the load-bearing definition):
// given the case's (TBox, ABox, CQ), `rewrite_production` produces a UCQ that,
// evaluated over the UNMODIFIED ABox through the REAL engine, returns EXACTLY the
// hand-derived certain-answer set — no missing answer (completeness) and no extra
// answer (soundness). Each case's certain answers are derived by hand from the
// DL-Lite_R semantics (the same derivations the rewrite-shape oracle in
// `sparq-reason-ql/tests/oracle.rs` re-checks), so the comparison is against an
// INDEPENDENT ground truth, not the rewriter's own output.
//
// HONEST SCOPE: this is a faithful DL-Lite_R certain-answer oracle, NOT the full
// normative OWL 2 QL conformance suite (there is no runnable W3C answer-comparison
// QL suite — the W3C QL material is structural). So the graduated ratchet is a
// sparq EXTENSION row (like RIF-Core / RSP / BM25), tallied SEPARATELY and NEVER
// folded into the standards-conformance total. The pinned floor is exactly the
// count of cases on which the rewrite is sound AND complete.
// ===========================================================================

/// One hand-derived DL-Lite_R certain-answer oracle case: a TBox + ABox + a
/// conjunctive `SELECT` query, with the EXACT certain-answer set derived by hand
/// from the DL-Lite_R semantics. The runner asserts `rewrite_production`'s UCQ,
/// evaluated over the UNMODIFIED ABox, returns exactly `certain`.
#[cfg(feature = "ql-experimental")]
pub struct QlOracleCase {
    /// Short stable id (for the report + the per-case outcome).
    pub id: &'static str,
    /// The DL-Lite_R TBox, as Turtle (prefixes are prepended by the runner).
    pub tbox: &'static str,
    /// The ABox (the unmodified data the rewrite is evaluated over), as Turtle.
    pub abox: &'static str,
    /// The conjunctive `SELECT` query (a single answer variable `?x`, or two
    /// `?x ?y` — the runner reads the projected variables from the parse).
    pub query: &'static str,
    /// The EXACT certain answers, each a row of local-name bindings aligned to the
    /// query's projected variables in order (e.g. `&[&["alice"]]` for one row
    /// binding `?x` to `:alice`). The empty slice means "no certain answers".
    /// Local names resolve against the `http://ex/` prefix.
    pub certain: &'static [&'static [&'static str]],
}

/// The result of running ONE [`QlOracleCase`] through the graduated arm.
#[cfg(feature = "ql-experimental")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QlOracleOutcome {
    /// The rewrite's UCQ, evaluated over the unmodified ABox, returned EXACTLY the
    /// hand-derived certain answers — sound AND complete on this case. This is the
    /// only outcome that counts toward the pinned floor.
    SoundAndComplete,
    /// The rewrite produced answers that differ from the certain-answer oracle (a
    /// missing or an extra answer). NEVER counted into the floor — a divergence is
    /// an honest gap (or a bug), reported with the mismatch.
    Diverged(String),
    /// The rewriter ABSTAINED (the fail-closed CQ-shape / non-DL-Lite gate). A
    /// formal-suite case is constructed to be within sound rewriting, so an abstain
    /// here is reported (never counted as a pass), but means the case is outside the
    /// rewriter's current fragment — kept honest, not summed into the floor.
    Abstained(String),
    /// The harness could not set the case up (parse error). Never a pass.
    Inconclusive(String),
}

/// The HAND-DERIVED DL-Lite_R certain-answer oracle corpus (sq-qo1a9). Each case
/// is within the SOUND query-rewriting fragment of DL-Lite_R; the `certain` field
/// is derived by hand from the DL-Lite_R semantics, INDEPENDENTLY of the rewriter.
///
/// The corpus exercises the DL-Lite_R rewriting axes the formal oracle proves:
/// class subsumption (incl. a transitive chain), the existential domain/range
/// inclusions (`∃R ⊑ A`, `∃R⁻ ⊑ A`), role inclusion, `owl:inverseOf`, the
/// unqualified `∃R` super-class generator with its applicability condition (must
/// NOT fire on a bound/projected filler), a two-distinguished-variable product
/// (no over-minimisation), and the empty-TBox identity. Every certain-answer set
/// is the set of bindings that hold in EVERY model of (TBox ∪ ABox).
#[cfg(feature = "ql-experimental")]
pub const QL_DLLITE_ORACLE: &[QlOracleCase] = &[
    // C1 — class subsumption: Manager ⊑ Employee. A Manager is certainly an
    // Employee, so { ?x a :Employee } over { :a a :Manager . :b a :Employee }
    // certainly answers both :a and :b.
    QlOracleCase {
        id: "ql-class-subsumption",
        tbox: ":Manager rdfs:subClassOf :Employee .",
        abox: ":a a :Manager . :b a :Employee .",
        query: "SELECT ?x WHERE { ?x a :Employee }",
        certain: &[&["a"], &["b"]],
    },
    // C2 — transitive subclass chain: A ⊑ B ⊑ C. Any A or B is certainly a C.
    QlOracleCase {
        id: "ql-transitive-subclass",
        tbox: ":A rdfs:subClassOf :B . :B rdfs:subClassOf :C .",
        abox: ":a a :A . :b a :B . :c a :C . :d a :Other .",
        query: "SELECT ?x WHERE { ?x a :C }",
        certain: &[&["a"], &["b"], &["c"]],
    },
    // C3 — domain inclusion (∃worksFor ⊑ Employee): anyone who worksFor anything is
    // certainly an Employee. { ?x a :Employee } answers :a (asserted) and :p (has a
    // worksFor edge).
    QlOracleCase {
        id: "ql-domain-introduces-role",
        tbox: ":worksFor rdfs:domain :Employee .",
        abox: ":a a :Employee . :p :worksFor :acme .",
        query: "SELECT ?x WHERE { ?x a :Employee }",
        certain: &[&["a"], &["p"]],
    },
    // C4 — range inclusion (∃worksFor⁻ ⊑ Company): anything something worksFor is
    // certainly a Company. :acme is the object of a worksFor edge.
    QlOracleCase {
        id: "ql-range-introduces-inverse-role",
        tbox: ":worksFor rdfs:range :Company .",
        abox: ":acme a :Company . :p :worksFor :globex .",
        query: "SELECT ?y WHERE { ?y a :Company }",
        certain: &[&["acme"], &["globex"]],
    },
    // C5 — role inclusion (manages ⊑ worksFor): a manages edge is certainly a
    // worksFor edge. Two distinguished variables — the answer is the union of the
    // asserted worksFor pairs and the manages pairs.
    QlOracleCase {
        id: "ql-role-inclusion",
        tbox: ":manages rdfs:subPropertyOf :worksFor .",
        abox: ":p :worksFor :acme . :q :manages :globex .",
        query: "SELECT ?x ?y WHERE { ?x :worksFor ?y }",
        certain: &[&["p", "acme"], &["q", "globex"]],
    },
    // C6 — owl:inverseOf (employs ≡ worksFor⁻) composed with a role inclusion
    // (manages ⊑ employs). A :manages b ⇒ a employs b ⇒ b worksFor a. So
    // { ?x :worksFor ?y } certainly answers the asserted worksFor pairs PLUS the
    // inverse of every manages pair.
    QlOracleCase {
        id: "ql-inverse-of-role-chain",
        tbox: ":employs owl:inverseOf :worksFor . :manages rdfs:subPropertyOf :employs .",
        abox: ":p :worksFor :acme . :boss :manages :emp .",
        query: "SELECT ?x ?y WHERE { ?x :worksFor ?y }",
        certain: &[&["p", "acme"], &["emp", "boss"]],
    },
    // C7 — the unqualified ∃R super-class generator (Employee ⊑ ∃worksFor) FIRES on
    // an UNBOUND (non-distinguished) filler. { ?x :worksFor ?y } with ?y not
    // projected: anyone asserted Employee certainly has SOME worksFor witness, so
    // :e (an Employee with no asserted edge) IS a certain answer, alongside :p (an
    // asserted worksFor subject).
    QlOracleCase {
        id: "ql-exists-super-fires-unbound",
        tbox: ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .",
        abox: ":e a :Employee . :p :worksFor :acme .",
        query: "SELECT ?x WHERE { ?x :worksFor ?y }",
        certain: &[&["e"], &["p"]],
    },
    // C8 — the APPLICABILITY CONDITION (the #1 unsoundness trap): the SAME
    // Employee ⊑ ∃worksFor generator MUST NOT fire when the filler ?y is PROJECTED
    // (bound). With ?y distinguished, { ?x :worksFor ?y } answers ONLY the asserted
    // edges — :e (the witness-only Employee) is NOT a certain answer because its
    // worksFor witness is anonymous (not a named binding for ?y).
    QlOracleCase {
        id: "ql-exists-super-blocked-bound",
        tbox: ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .",
        abox: ":e a :Employee . :p :worksFor :acme .",
        query: "SELECT ?x ?y WHERE { ?x :worksFor ?y }",
        certain: &[&["p", "acme"]],
    },
    // C9 — two distinguished variables under A ⊑ B: { ?x a :B . ?y a :B } over an
    // ABox with an A and a B. The certain answers are the full 2×2 product over the
    // set { :a (an A, hence a B), :b (a B) } — the anti-over-minimisation case
    // (minimisation must not collapse the disjuncts and lose a pair).
    QlOracleCase {
        id: "ql-two-var-product",
        tbox: ":A rdfs:subClassOf :B .",
        abox: ":a a :A . :b a :B .",
        query: "SELECT ?x ?y WHERE { ?x a :B . ?y a :B }",
        certain: &[&["a", "a"], &["a", "b"], &["b", "a"], &["b", "b"]],
    },
    // C10 — A∧C under A⊑B⊑C minimises to { A(?x) }: the conjunctive query
    // { ?x a :A . ?x a :C } has the same certain answers as { ?x a :A } (A ⇒ C), so
    // only the asserted/derivable A individuals answer. :a is an A; :c is only a C
    // (not certainly an A), so it is NOT an answer.
    QlOracleCase {
        id: "ql-conjunction-minimises",
        tbox: ":A rdfs:subClassOf :B . :B rdfs:subClassOf :C .",
        abox: ":a a :A . :c a :C .",
        query: "SELECT ?x WHERE { ?x a :A . ?x a :C }",
        certain: &[&["a"]],
    },
    // C11 — empty TBox is the identity: no schema ⇒ the certain answers are exactly
    // the asserted matches (a regression guard that the rewrite adds nothing).
    QlOracleCase {
        id: "ql-empty-tbox-identity",
        tbox: "",
        abox: ":a a :Employee . :b a :Manager .",
        query: "SELECT ?x WHERE { ?x a :Employee }",
        certain: &[&["a"]],
    },
];

/// [OPUS-4.8] sq-qo1a9 — run the GRADUATED DL-Lite_R certain-answer oracle: for
/// every [`QlOracleCase`], rewrite the CQ with `rewrite_production`, evaluate the
/// UCQ over the UNMODIFIED ABox through the REAL engine, and compare to the
/// hand-derived certain answers EXACTLY. Returns the per-case outcomes in corpus
/// order. Pure (no fetched fixtures — the corpus is in-source), so it runs on any
/// checkout. The caller (the ratchet test) counts `SoundAndComplete` outcomes as
/// the pinned floor and FAILS on any divergence.
#[cfg(feature = "ql-experimental")]
pub fn run_ql_dllite_oracle() -> Vec<(&'static str, QlOracleOutcome)> {
    QL_DLLITE_ORACLE
        .iter()
        .map(|case| (case.id, ql_oracle_one(case)))
        .collect()
}

/// The `http://ex/` prefix the oracle corpus's local names resolve against.
#[cfg(feature = "ql-experimental")]
const QL_EX: &str = "http://ex/";

/// Run ONE [`QlOracleCase`]: parse the TBox + ABox, rewrite the CQ, evaluate the
/// UCQ over the unmodified ABox, compare to the hand-derived certain answers.
#[cfg(feature = "ql-experimental")]
fn ql_oracle_one(case: &QlOracleCase) -> QlOracleOutcome {
    use crate::compare::Row;
    use oxrdf::{NamedNode, Term};

    // The Turtle prefix preamble shared by the TBox + ABox (and the query base).
    const TTL_PRE: &str = "\
@prefix : <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
";
    const Q_PRE: &str = "\
PREFIX : <http://ex/>
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
";

    // Parse the TBox triples (the schema the rewriter extracts DL-Lite_R axioms
    // from). Positional `format!` args per the CodeQL `rust/unused-variable` guard.
    let tbox_src = format!("{}{}", TTL_PRE, case.tbox);
    let mut tbox: Vec<oxrdf::Triple> = Vec::new();
    for t in oxttl::TurtleParser::new().for_slice(tbox_src.as_bytes()) {
        match t {
            Ok(tr) => tbox.push(tr),
            Err(e) => return QlOracleOutcome::Inconclusive(format!("TBox parse: {}", e)),
        }
    }

    // Parse + serialise the ABox as N-Triples for the engine (the UNMODIFIED data).
    let abox_src = format!("{}{}", TTL_PRE, case.abox);
    let mut nt = String::new();
    for t in oxttl::TurtleParser::new().for_slice(abox_src.as_bytes()) {
        match t {
            Ok(tr) => nt.push_str(&format!("{} {} {} .\n", tr.subject, tr.predicate, tr.object)),
            Err(e) => return QlOracleOutcome::Inconclusive(format!("ABox parse: {}", e)),
        }
    }

    // Parse the conjunctive query and rewrite it (fail-closed: an abstain is
    // reported, never a guessed pass).
    let q_src = format!("{}{}", Q_PRE, case.query);
    let query = match SparqlParser::new().parse_query(&q_src) {
        Ok(q) => q,
        Err(e) => return QlOracleOutcome::Inconclusive(format!("query parse: {}", e)),
    };
    let rewritten = match sparq_reason_ql::rewrite_production(&query, &tbox) {
        Ok(r) => r,
        Err(e) => return QlOracleOutcome::Abstained(e.to_string()),
    };

    // The projected variables, in order — defines the column alignment for both the
    // expected certain answers and the engine's actual rows.
    let proj: Vec<String> = match &rewritten.query {
        spargebra::Query::Select { pattern, .. } => projected_vars(pattern),
        _ => return QlOracleOutcome::Inconclusive("oracle case query is not SELECT".into()),
    };

    // Evaluate the rewritten UCQ over the unmodified ABox through the REAL engine.
    let query_str = rewritten.query.to_string();
    let graph = match sparq_core::Graph::load_dataset(&nt, "nquads") {
        Ok(g) => g,
        Err(e) => return QlOracleOutcome::Inconclusive(format!("load ABox: {}", e)),
    };
    let res = match sparq_engine::query(&graph, &query_str) {
        Ok(r) => r,
        Err(e) => return QlOracleOutcome::Inconclusive(format!("engine: {}", e)),
    };
    let actual_vars: Vec<String> = res.vars.iter().map(|v| v.as_str().to_string()).collect();

    // Build the EXPECTED rows from the hand-derived certain answers (local names →
    // `http://ex/<name>` IRIs), aligned to the projected variable order.
    let exp: Vec<Row> = case
        .certain
        .iter()
        .map(|binds| {
            binds
                .iter()
                .map(|name| {
                    Some(Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", QL_EX, name))))
                })
                .collect::<Row>()
        })
        .collect();

    // Align the engine's actual rows to the SAME projected-variable order.
    let act: Vec<Row> = res
        .rows
        .iter()
        .map(|r| {
            proj.iter()
                .map(|v| {
                    actual_vars
                        .iter()
                        .position(|av| av == v)
                        .and_then(|i| r.get(i).cloned().flatten())
                })
                .collect::<Row>()
        })
        .collect();

    // EXACT multiset comparison: sound AND complete iff the actual rows equal the
    // hand-derived certain answers (no missing, no extra).
    match crate::compare::rows_equal(&exp, &act, false) {
        Ok(true) => QlOracleOutcome::SoundAndComplete,
        Ok(false) => QlOracleOutcome::Diverged(format!(
            "expected {} certain row(s), engine returned {} row(s) over the rewritten UCQ",
            exp.len(),
            act.len()
        )),
        Err(e) => QlOracleOutcome::Inconclusive(format!("compare: {}", e)),
    }
}

/// The projected variable names (in order) of a `SELECT` pattern — the answer
/// columns the oracle aligns expected vs actual rows on. Peels the
/// Project/Distinct/Reduced wrappers the rewrite re-wraps the UCQ in.
#[cfg(feature = "ql-experimental")]
fn projected_vars(p: &GraphPattern) -> Vec<String> {
    match p {
        GraphPattern::Project { variables, .. } => {
            variables.iter().map(|v| v.as_str().to_string()).collect()
        }
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => projected_vars(inner),
        _ => Vec::new(),
    }
}

// [OPUS-4.8] sq-kuvu3 / [FABLE-5] sq-pbz04.3.4 — DIRECT unit tests for the public
// QL-arm surface (the coverage-ratchet rule: one direct test per new public fn).
// Hermetic (no fetched fixtures) except the arm smoke test, which self-skips.
#[cfg(all(test, feature = "ql-experimental"))]
mod ql_tests {
    use super::*;

    fn parse(q: &str) -> Query {
        SparqlParser::new().parse_query(q).expect("parse")
    }

    fn cq_of(q: &str) -> sparq_reason_ql::ConjunctiveQuery {
        sparq_reason_ql::as_conjunctive_query(&parse(q)).expect("CQ")
    }

    const PRE: &str = "PREFIX : <http://ex/> \
                       PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                       PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
                       PREFIX owl: <http://www.w3.org/2002/07/owl#> ";

    #[test]
    fn hold_reason_taxonomy_is_specific_and_never_claims_a_pass() {
        // Every variant carries a distinct stable label, and every rendered reason
        // keeps the `QL experimental (` marker + its label — no hold can ever read
        // as a graduated pass or a conformance claim.
        let all: [QlHoldReason; 10] = [
            QlHoldReason::PermanentlyOutside("BIND (Extend) is not conjunctive".into()),
            QlHoldReason::PendingGate("FILTER is not conjunctive".into()),
            QlHoldReason::PendingCapture { skipped: 1, unrecognised_schema: 2 },
            QlHoldReason::PendingConsistency { negative_axioms: 3 },
            QlHoldReason::InconsistentKb {
                violated: "<http://ex/A> ⊑ ¬<http://ex/B>".into(),
            },
            QlHoldReason::NamedGraphDataset,
            QlHoldReason::PendingCoincidence("non-distinguished body variable ?y".into()),
            QlHoldReason::OracleDivergent { disjuncts: 1, detail: "differ".into() },
            QlHoldReason::UnclassifiedAbstain("new abstain class".into()),
            QlHoldReason::Inconclusive("data parse error".into()),
        ];
        let mut labels: Vec<&str> = all.iter().map(|r| r.label()).collect();
        labels.sort_unstable();
        let n = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n, "taxonomy labels must be distinct (no catch-all)");
        for r in &all {
            let s = r.reason();
            assert!(s.starts_with("QL experimental ("), "reason: {s}");
            assert!(s.contains(r.label()), "reason must carry its label: {s}");
            assert!(!s.contains("graduated"), "a hold must never read graduated: {s}");
        }
        // The consistency bucket carries its measured count (the §8 report input).
        assert!(QlHoldReason::PendingConsistency { negative_axioms: 3 }
            .reason()
            .contains("3 negative/disjointness"));
    }

    #[test]
    fn graduated_reason_is_scoped_as_an_extension_row() {
        let r = ql_graduated_reason(4);
        assert!(r.starts_with("QL graduated"));
        assert!(r.contains("4 disjunct"));
        // The load-bearing honesty disclaimer.
        assert!(r.contains("NEVER summed into the standards-conformance total"));
        // And the verdict enum distinguishes the two arms.
        assert_ne!(
            QlGraduationVerdict::Graduated { disjuncts: 1 },
            QlGraduationVerdict::Held(QlHoldReason::NamedGraphDataset)
        );
    }

    #[test]
    fn ql_experimental_notes_state_the_six_conditions_honestly() {
        let notes = ql_experimental_notes();
        assert_eq!(notes.len(), 1);
        let n = &notes[0];
        assert!(n.contains("SIX-CONDITION"));
        assert!(n.contains("NEVER summed into the standards-conformance total"));
        assert!(n.contains("fully_captured()"));
        assert!(n.contains("regime-coincidence"));
        assert!(n.contains("never a guessed pass"));
    }

    #[test]
    fn case_fragment_extracts_stable_ids() {
        assert_eq!(case_fragment("http://ex/manifest#rdfs02"), "rdfs02");
        assert_eq!(case_fragment("http://ex/path/leaf"), "leaf");
        assert_eq!(case_fragment("bare"), "bare");
    }

    #[test]
    fn intensional_guard_flags_schema_atoms_and_admits_extensional_ones() {
        // B6 is now in the GATE (sq-pbz04.3.1): `check_atom_shape` rejects schema-vocabulary
        // predicates directly, so `as_conjunctive_query` returns Err for those. We verify the
        // gate rejects them, and that `intensional_atom()` still catches the remaining cases
        // that slip past the gate (rdf:type with class-position variable or schema-ns constant).

        // Schema-vocabulary predicate (paper-sparqldl-Q1 shape): gate now rejects. [SONNET-4.6]
        let q_sub = parse(&format!("{PRE} SELECT ?c WHERE {{ ?c rdfs:subClassOf :Student }}"));
        assert!(
            sparq_reason_ql::as_conjunctive_query(&q_sub).is_err(),
            "rdfs:subClassOf as predicate: gate must reject (B6 now in gate)"
        );
        // owl: predicates are all semantics-bearing — gate rejects. [SONNET-4.6]
        let q_same = parse(&format!("{PRE} SELECT ?x WHERE {{ ?x owl:sameAs :a }}"));
        assert!(
            sparq_reason_ql::as_conjunctive_query(&q_same).is_err(),
            "owl:sameAs as predicate: gate must reject (B6)"
        );

        // rdf:type with a class-position VARIABLE: gate admits (predicate rdf:type is not
        // schema-vocabulary), but intensional_atom() flags it. [SONNET-4.6]
        let cq = cq_of(&format!("{PRE} SELECT ?c WHERE {{ :a rdf:type ?c }}"));
        assert!(intensional_atom(&cq).unwrap().contains("class-name-position"));
        // rdf:type with a schema-namespace class constant: likewise gate admits, harness catches.
        let cq = cq_of(&format!("{PRE} SELECT ?x WHERE {{ ?x rdf:type owl:Class }}"));
        assert!(intensional_atom(&cq).is_some());
        // Extensional class + role atoms are admitted by both gate and intensional_atom().
        let cq = cq_of(&format!("{PRE} SELECT ?x WHERE {{ ?x rdf:type :A . ?x :r ?y }}"));
        assert!(intensional_atom(&cq).is_none());
        // Annotation predicates stay admitted (no QL axiom changes their extension).
        let cq = cq_of(&format!("{PRE} SELECT ?x ?l WHERE {{ ?x rdfs:label ?l }}"));
        assert!(intensional_atom(&cq).is_none());
    }

    #[test]
    fn regime_coincidence_guard_is_fail_closed() {
        use sparq_reason_ql::{Basic, Role, TBox};
        let mut tbox_with_exists = TBox::default();
        tbox_with_exists
            .exists_super
            .push((Basic::Class("http://ex/Employee".into()), Role::named("http://ex/worksFor")));
        let empty_tbox = TBox::default();

        // Non-distinguished ?y + an existential generator: MAY diverge — held.
        let cq = cq_of(&format!("{PRE} SELECT ?x WHERE {{ ?x :worksFor ?y }}"));
        assert!(ql_regime_coincides(&cq, &tbox_with_exists)
            .unwrap_err()
            .contains("non-distinguished body variable ?y"));
        // Same query, NO existential generator: coincides.
        assert!(ql_regime_coincides(&cq, &empty_tbox).is_ok());
        // All body variables distinguished: coincides even WITH the generator.
        let cq = cq_of(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :worksFor ?y }}"));
        assert!(ql_regime_coincides(&cq, &tbox_with_exists).is_ok());
        // A blank-node body term is a non-distinguished variable: held.
        let cq = cq_of(&format!("{PRE} SELECT ?x WHERE {{ ?x :worksFor [] }}"));
        assert!(ql_regime_coincides(&cq, &tbox_with_exists)
            .unwrap_err()
            .contains("blank-node body term"));
    }

    // [FABLE-5] sq-p6yb7 — CONDITION (3) upgraded: the consistency check decides, fail-closed.
    // MUTATION WITNESS (verified during development): making `ql_condition3_consistency`
    // return `Ok(())` unconditionally flips the `Inconsistent`/`Unknown` arms below red.
    #[test]
    fn condition3_runs_the_consistency_check_fail_closed() {
        use oxrdf::{NamedNode, Triple};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let t = |s: &str, p: &str, o: &str| Triple::new(iri(s), iri(p), iri(o));
        const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        const DISJOINT: &str = "http://www.w3.org/2002/07/owl#disjointWith";
        const COMPLEMENT: &str = "http://www.w3.org/2002/07/owl#complementOf";

        // No negative axioms: passes trivially (the pre-sq-p6yb7 behaviour, unchanged).
        let data = vec![t("http://ex/i", RDF_TYPE, "http://ex/A")];
        let tbox = sparq_reason_ql::TBox::extract(&data);
        assert!(ql_condition3_consistency(&tbox, &data).is_ok());

        // Negative axioms + satisfiable KB: the check proves Consistent → condition 3 PASSES
        // (previously this held at pending-consistency unconditionally).
        let data = vec![
            t("http://ex/A", DISJOINT, "http://ex/B"),
            t("http://ex/i", RDF_TYPE, "http://ex/A"),
            t("http://ex/j", RDF_TYPE, "http://ex/B"),
        ];
        let tbox = sparq_reason_ql::TBox::extract(&data);
        assert!(ql_condition3_consistency(&tbox, &data).is_ok());

        // Violated disjointness: proven INCONSISTENT → held (fail-closed, with the witness).
        let data = vec![
            t("http://ex/A", DISJOINT, "http://ex/B"),
            t("http://ex/i", RDF_TYPE, "http://ex/A"),
            t("http://ex/i", RDF_TYPE, "http://ex/B"),
        ];
        let tbox = sparq_reason_ql::TBox::extract(&data);
        match ql_condition3_consistency(&tbox, &data) {
            Err(QlHoldReason::InconsistentKb { violated }) => {
                assert!(violated.contains("http://ex/A"), "witness: {violated}");
            }
            other => panic!("expected InconsistentKb, got {:?}", other),
        }

        // complementOf: structurally uncaptured → the check returns Unknown → held at
        // pending-consistency (fail-closed).
        let data = vec![
            t("http://ex/A", COMPLEMENT, "http://ex/B"),
            t("http://ex/i", RDF_TYPE, "http://ex/A"),
        ];
        let tbox = sparq_reason_ql::TBox::extract(&data);
        assert!(matches!(
            ql_condition3_consistency(&tbox, &data),
            Err(QlHoldReason::PendingConsistency { negative_axioms: 1 })
        ));
    }

    #[test]
    fn gate_rejections_classify_into_the_taxonomy() {
        let classify = |q: &str| {
            let query = parse(q);
            match sparq_reason_ql::as_conjunctive_query(&query) {
                Err(sparq_reason_ql::CqError::OutOfScope(reason)) => {
                    classify_gate_rejection(&query, &reason)
                }
                Ok(_) => panic!("expected a gate rejection for: {}", q),
            }
        };
        // B1 broadening shape → pending-gate. A UNION at the top level is a UCQ —
        // `as_conjunctive_query` rejects multi-branch UCQs (use `as_ucq` for those), so
        // it still generates a gate rejection classifiable as pending-gate (B1). [SONNET-4.6]
        let r = classify(&format!(
            "{PRE} SELECT ?x WHERE {{ {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B }} }}"
        ));
        assert_eq!(r.label(), "pending-gate");
        assert!(r.reason().contains("B1"));
        // B6 gate-side intensional-atom rejection → permanently-outside. [SONNET-4.6]
        let r = classify(&format!(
            "{PRE} SELECT ?c WHERE {{ ?c rdfs:subClassOf :A }}"
        ));
        assert_eq!(r.label(), "permanently-outside");
        assert!(r.reason().contains("intensional"));
        // NOTE: B3 (distinguished-only FILTER) has LANDED (sq-pbz04.3.1) — a FILTER on a
        // projected variable is now ACCEPTED by the gate, not a gate rejection. The
        // classify_gate_rejection FILTER arm handles the legacy pending-gate case for
        // older gate-side rejections; we verify it still classifies those correctly via a
        // non-distinguished FILTER (which the gate still rejects B3-fail-closed).
        let r = classify(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A . ?x :r ?y FILTER(?y = :b) }}"
        ));
        assert_eq!(r.label(), "pending-gate");
        // Non-recursive path → pending-gate; recursive path → permanent.
        // (Since sq-pbz04.3.2 the gate DESUGARS an alternation into a UCQ (B5 landed), so
        // the single-CQ gate rejects it with the UNION/B1 message — still an honest
        // pending-gate hold, because `ql_graduation_one` runs `as_conjunctive_query`, not
        // `as_ucq`. The stale `contains("B5")` expectation was fixed by sq-p6yb7 — this lib
        // test lane is not in any CI leg, so the drift went unnoticed; bead filed.)
        let r = classify(&format!("{PRE} SELECT ?x WHERE {{ ?x (:p|:q) ?y }}"));
        assert_eq!(r.label(), "pending-gate");
        assert!(r.reason().contains("B1 UCQ input"), "reason: {}", r.reason());
        let r = classify(&format!("{PRE} SELECT ?x WHERE {{ ?x :p+ ?y }}"));
        assert_eq!(r.label(), "permanently-outside");
        // Design-permanent shapes → permanently-outside.
        let r = classify(&format!("{PRE} SELECT ?x WHERE {{ ?x rdf:type :A BIND(:b AS ?y) }}"));
        assert_eq!(r.label(), "permanently-outside");
        let r = classify(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A MINUS {{ ?x rdf:type :B }} }}"
        ));
        assert_eq!(r.label(), "permanently-outside");
        let r = classify(&format!("{PRE} SELECT ?x WHERE {{ ?x ?p :A }}"));
        assert_eq!(r.label(), "permanently-outside");
        // An unmatched reason lands in the LOUD unclassified bucket, never silently.
        assert_eq!(
            classify_gate_rejection(&parse(&format!("{PRE} SELECT ?x WHERE {{ ?x :p :a }}")), "some future rejection"),
            QlHoldReason::UnclassifiedAbstain("some future rejection".into())
        );
    }

    #[test]
    fn rewrite_abstains_classify_into_the_taxonomy() {
        let r = classify_rewrite_abstain("literal in a class/role atom position is out of DL-Lite scope");
        assert_eq!(r.label(), "pending-gate");
        assert!(r.reason().contains("B2"));
        let r = classify_rewrite_abstain("blank-node term is out of DL-Lite atom scope");
        assert_eq!(r.label(), "pending-gate");
        let r = classify_rewrite_abstain("rdf:type object must be a named class for DL-Lite rewriting");
        assert_eq!(r.label(), "permanently-outside");
        let r = classify_rewrite_abstain("something the taxonomy has never seen");
        assert_eq!(r.label(), "unclassified-abstain");
    }

    #[test]
    fn query_carried_dataset_clauses_are_held_not_dropped() {
        // Condition (4, continued): the CQ-shape gate DROPS a query's own
        // FROM/FROM NAMED dataset, so the graduation predicate must detect and
        // hold it — a dataset-carrying query must never be rewritten over the
        // default graph as if the clause were not there.
        assert!(query_carries_dataset(&parse(&format!(
            "{PRE} SELECT ?x FROM <http://ex/g> WHERE {{ ?x rdf:type :A }}"
        ))));
        assert!(query_carries_dataset(&parse(&format!(
            "{PRE} SELECT ?x FROM NAMED <http://ex/g> WHERE {{ ?x rdf:type :A }}"
        ))));
        assert!(query_carries_dataset(&parse(&format!(
            "{PRE} ASK FROM <http://ex/g> {{ ?x rdf:type :A }}"
        ))));
        assert!(!query_carries_dataset(&parse(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A }}"
        ))));
    }

    #[test]
    fn nonrecursive_path_detector_covers_the_b5_forms() {
        let ok = |q: &str| query_paths_all_nonrecursive(&parse(q));
        assert!(ok(&format!("{PRE} SELECT ?x WHERE {{ ?x :p/:q ?y }}")));
        assert!(ok(&format!("{PRE} SELECT ?x WHERE {{ ?x ^:p ?y }}")));
        assert!(ok(&format!("{PRE} SELECT ?x WHERE {{ ?x (:p|:q) ?y }}")));
        assert!(!ok(&format!("{PRE} SELECT ?x WHERE {{ ?x :p* ?y }}")));
        assert!(!ok(&format!("{PRE} SELECT ?x WHERE {{ ?x :p? ?y }}")));
        assert!(!ok(&format!("{PRE} SELECT ?x WHERE {{ ?x !(:p) ?y }}")));
    }

    #[test]
    fn arm_reports_only_out_of_scope_rows() {
        // The arm appends ONLY OutOfScope rows — never a Pass / Fail that would
        // count toward the inference binary's conformance rate or ratchet floor.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/w3c/rdf-tests");
        if !root.join("sparql/sparql11/entailment/manifest.ttl").exists() {
            eprintln!("SKIP: rdf-tests entailment fixtures absent");
            return;
        }
        let mut out: Vec<TestResult> = Vec::new();
        run_ql_experimental_arm(&root, &mut out).expect("QL arm runs");
        assert!(!out.is_empty(), "the suite has pr:QL-tagged tests");
        for r in &out {
            assert!(
                matches!(r.outcome, Outcome::OutOfScope(_)),
                "QL arm emitted a non-OutOfScope row ({:?}) — that would risk a faked \
                 conformance pass in the binary ratchet",
                r.outcome
            );
            // Every reason carries one of the two stable markers.
            if let Outcome::OutOfScope(reason) = &r.outcome {
                assert!(
                    reason.starts_with("QL experimental") || reason.starts_with("QL graduated"),
                    "reason: {reason}"
                );
            }
        }
    }

    // [OPUS-4.8] sq-qo1a9 — DIRECT unit tests for the GRADUATED DL-Lite_R
    // certain-answer oracle surface (the coverage-ratchet rule: one direct test
    // per new public fn). Hermetic — the corpus is in-source, no fetched fixtures.
    #[test]
    fn dllite_oracle_is_sound_and_complete_on_every_formal_case() {
        // The load-bearing graduation claim: on the FORMAL DL-Lite_R suite the
        // rewrite is sound AND complete case by case — every case's rewritten UCQ,
        // evaluated over the unmodified ABox, returns EXACTLY the hand-derived
        // certain answers. ANY divergence/abstain/inconclusive must fail here.
        let results = run_ql_dllite_oracle();
        assert_eq!(
            results.len(),
            QL_DLLITE_ORACLE.len(),
            "the runner must report one outcome per oracle case"
        );
        for (id, outcome) in &results {
            assert_eq!(
                *outcome,
                QlOracleOutcome::SoundAndComplete,
                "DL-Lite_R oracle case {} is not sound-and-complete: {:?}",
                id,
                outcome
            );
        }
    }

    #[test]
    fn dllite_oracle_corpus_is_non_trivial_and_well_formed() {
        // The corpus must be a meaningful conformance claim: enough cases, every id
        // unique, and at least one case with NO certain answers (the applicability
        // condition C8 — the existential generator must not fire on a bound filler).
        assert!(
            QL_DLLITE_ORACLE.len() >= 10,
            "too few formal cases to be a meaningful conformance claim"
        );
        let mut ids: Vec<&str> = QL_DLLITE_ORACLE.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "oracle case ids must be unique");
        // A genuine certain-answer oracle must include a case whose answer set is a
        // STRICT subset of the naive asserted matches (the soundness direction).
        assert!(
            QL_DLLITE_ORACLE.iter().any(|c| c.id == "ql-exists-super-blocked-bound"),
            "the applicability-condition (bound-filler) soundness case must be present"
        );
    }

    #[test]
    fn dllite_oracle_outcome_variants_are_distinct() {
        // Direct surface coverage of the public QlOracleOutcome enum: the floor
        // only ever counts SoundAndComplete; the other three are honest non-passes.
        assert_ne!(
            QlOracleOutcome::SoundAndComplete,
            QlOracleOutcome::Diverged("x".into())
        );
        assert_ne!(
            QlOracleOutcome::Abstained("y".into()),
            QlOracleOutcome::Inconclusive("z".into())
        );
        // The corpus's projected-var helper handles the re-wrapped Project/Distinct.
        let q = SparqlParser::new()
            .parse_query("SELECT DISTINCT ?x WHERE { ?x <http://ex/p> ?y }")
            .unwrap();
        let spargebra::Query::Select { pattern, .. } = &q else {
            panic!("select");
        };
        assert_eq!(projected_vars(pattern), vec!["x".to_string()]);
    }
}

// [SONNET-4.6] sq-6qpyf (epic sq-pbz04) — DIRECT tests for the fail-closed
// named-graph (`qt:graphData`) guard in `run_one`. The guard is UNREACHABLE from
// the pinned rdf-tests manifest (zero such cases — asserted by
// `tests/named_graph_entailment_census.rs`), so without these it carries no
// coverage at all and could be deleted or inverted silently. They exercise the
// guard through the real `run_one` entry point on a synthetic manifest entry, and
// pin BOTH arms so either mutation goes red:
//   - delete the guard  → the named-graph case falls through to the query path and
//     reports `Fail("manifest entry has no qt:query")` instead of the held reason;
//   - invert the guard  → the default-graph case is held instead of falling through.
// Default feature state (no `d-entail` / `ql-experimental` needed), so this runs in
// the bare `cargo test -p sparq-conformance` as well as every gated lane.
#[cfg(test)]
mod named_graph_guard_tests {
    use super::*;
    use std::path::PathBuf;

    /// The exact single-line `OutOfScope` reason a named-graph entailment case is
    /// held under. Spelled out here rather than shared with the guard via a const,
    /// so a reword of the reported reason is a deliberate, visible edit.
    const HELD_REASON: &str = "named-graph entailment dataset not wired";

    /// A synthetic `sparql11/entailment`-shaped QueryEvaluationTest with the given
    /// named-graph dataset and no `qt:query` — enough to reach the guard, and the
    /// missing query makes the fall-through path distinguishable from the held one.
    fn entry(graph_data: Vec<(String, PathBuf)>) -> TestEntry {
        TestEntry {
            id: "http://example.org/entailment-manifest#synthetic".into(),
            name: "synthetic-named-graph-entailment".into(),
            suite: "sparql11/entailment".into(),
            kind: EntryKind::QueryEval,
            withdrawn: false,
            action: manifest::QueryAction { graph_data, ..Default::default() },
            result_file: None,
            update_request: None,
            update_pre: manifest::UpdateState::default(),
            update_post: manifest::UpdateState::default(),
        }
    }

    #[test]
    fn named_graph_dataset_is_held_out_of_scope_under_every_profile() {
        let e = entry(vec![(
            "http://example.org/g".into(),
            PathBuf::from("/nonexistent/named-graph.ttl"),
        )]);
        // Every regime the runner can pick must hold — the default-graph-only
        // materialization path is wrong for a named-graph dataset regardless of
        // which closure it would have computed.
        for profile in [Profile::Rdf, Profile::Rdfs, Profile::OwlRl] {
            match run_one(&e, profile, false) {
                Outcome::OutOfScope(reason) => assert_eq!(
                    reason, HELD_REASON,
                    "a named-graph entailment case must be held under the documented \
                     fail-closed reason, not laundered under some other skip"
                ),
                // Positional `format!`/assert args per the CodeQL
                // `rust/unused-variable` false-positive guard.
                other => panic!(
                    "named-graph entailment case was NOT held fail-closed: {:?} — the \
                     default-graph materialization path gives a WRONG closure for a \
                     named-graph dataset (per-graph semantics are unimplemented, \
                     sq-6qpyf / epic sq-pbz04)",
                    other
                ),
            }
        }
    }

    #[test]
    fn default_graph_dataset_falls_through_the_guard() {
        // The guard must be conditioned on the dataset, not an unconditional hold:
        // an ordinary default-graph entailment case reaches the real query path
        // (and, having no `qt:query`, fails there rather than being held).
        match run_one(&entry(Vec::new()), Profile::Rdfs, false) {
            Outcome::Fail(reason) => assert!(
                reason.contains("no qt:query"),
                "expected the query-path failure, got {:?}",
                reason
            ),
            other => panic!(
                "a default-graph entailment case must fall THROUGH the named-graph \
                 guard into the query path, got {:?}",
                other
            ),
        }
    }
}
