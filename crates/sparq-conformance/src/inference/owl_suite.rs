//! OWL 2 conformance test cases (the W3C OWL WG test repository export),
//! filtered to the tests applicable to an OWL 2 RL / RDF-rules reasoner:
//! `test:profile test:RL` AND `test:semantics test:RDF-BASED`. Each selected
//! case contributes one scoreboard row per check it declares:
//!
//! - `ConsistencyTest` / `InconsistencyTest` — materialize with
//!   `sparq_reason::materialize_owl_rl`, then `sparq_reason::inconsistencies`.
//! - `PositiveEntailmentTest` — the materialized premise closure must simply
//!   entail the conclusion ontology (bnode homomorphism, D-value literals).
//! - `NegativeEntailmentTest` — the non-conclusion must NOT be entailed.
//!
//! The export inlines every ontology as an RDF/XML literal; ontology-header
//! triples (`owl:Ontology` typing + `owl:imports`) are stripped from both
//! sides, as the official harness compares axioms, not headers.
//!
//! Honesty note (OWL 2 Conformance §2.3): the RL/RDF rules are deliberately
//! INCOMPLETE for the RDF-based semantics — completeness (theorem PR1) is
//! guaranteed only for assertion-style conclusions. Positive entailment tests
//! whose conclusions are TBox axioms may therefore fail under any pure
//! rules-based reasoner; they are reported as fails with that context rather
//! than silently excluded.

use super::entail::{self, Recognized, Row};
use super::report::{Outcome, TestResult};
use crate::rdf::MiniGraph;
use oxrdf::{NamedOrBlankNode, Term};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const T: &str = "http://www.w3.org/2007/OWL/testOntology#";
const OWL_ONTOLOGY: &str = "http://www.w3.org/2002/07/owl#Ontology";
const OWL_IMPORTS: &str = "http://www.w3.org/2002/07/owl#imports";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Documented divergences — the per-divergence DISPOSITION pass (sq-pbz04.1.3).
///
/// Each entry was audited from its raw export premise/conclusion against the
/// OWL 2 Profiles REC: the §4.2 RL grammar, every rule HEAD in the §4.3 RL/RDF
/// rule tables, and theorem PR1, whose completeness guarantee (Conformance REC
/// §2.3) covers only assertion conclusions — ClassAssertion /
/// ObjectPropertyAssertion / DataPropertyAssertion / SameIndividual
/// (DifferentIndividuals conclusions are outside PR1 entirely). Criterion: if
/// the RL/RDF rules genuinely derive the expected conclusion, FIX the reasoner
/// and drop the entry; otherwise the divergence is PERMANENT with its
/// rule-level grounding below. Result: 13/13 PERMANENT — no rule head emits
/// owl:TransitiveProperty, owl:complementOf, owl:unionOf, list cells, or a
/// reified owl:AllDifferent; the only differentFrom-emitting rule is dt-diff
/// (unequal-value LITERAL pairs, never individuals); ReflexiveObjectProperty
/// is excluded from the RL grammar. Forcing any of these would be an UNSOUND
/// beyond-RL extension — worse than an honest divergence; the owl.rs
/// soundness guards pin the non-derivations (sq-350ms), and
/// research/inference-completeness-audit.md §2b carries the audit record.
/// Reported distinctly (counts as pass+divergence), never silently. [FABLE-5]
const DOCUMENTED_DIVERGENCES: &[(&str, &str)] = &[
    (
        "chain2trans1",
        "PERMANENT — conclusion is the TBox axiom `p rdf:type owl:TransitiveProperty` (from \
         the self-chain p∘p ⊑ p); prp-spo2 consumes owl:propertyChainAxiom only in its BODY \
         to derive chained assertions, no rule head emits owl:TransitiveProperty, and PR1 \
         completeness is assertion-only",
    ),
    (
        "DisjointClasses-001",
        "PERMANENT — conclusion types Stewie into an INVENTED anonymous owl:complementOf \
         class; complementOf occurs only in the BODY of the clash rule cls-com — no RL/RDF \
         rule head constructs a class expression (PR1 covers named-class assertions only)",
    ),
    (
        "DisjointClasses-003",
        "PERMANENT — as DisjointClasses-001 via owl:AllDisjointClasses: the conclusion \
         invents TWO anonymous owl:complementOf classes; cax-adc consumes the members list \
         to derive `false` only, and no rule head constructs a class expression",
    ),
    (
        "New-Feature-ObjectQCR-002",
        "PERMANENT — needs the CONTRAPOSITIVE of cls-maxqc3: were Stewie a Woman, Peter's \
         maxQC-1-on-Woman restriction would force `Stewie sameAs Meg` against their \
         differentFrom, so full semantics types Stewie into the complement of Woman; \
         cls-maxqc3/4 derive sameAs only from ALREADY-co-typed fillers, and no rule head \
         constructs the required owl:complementOf class",
    ),
    (
        "WebOnt-I5.5-005",
        "PERMANENT — conclusion asserts the EXISTENCE of an anonymous class \
         `[ owl:unionOf (a) ]`; no RL/RDF rule head emits owl:unionOf or rdf:first/rdf:rest \
         list cells (cls-uni/scm-uni consume unionOf in bodies), so the closure can never \
         contain the required structure",
    ),
    (
        "New-Feature-DisjointDataProperties-002",
        "PERMANENT — conclusion is a REIFIED owl:AllDifferent/owl:distinctMembers structure; \
         eq-diff2/3 and prp-adp consume these structures in rule BODIES (clash detection) \
         and no rule head constructs one — and differentFrom between individuals is \
         underivable anyway (dt-diff emits it only between unequal-value literals)",
    ),
    (
        "New-Feature-DisjointObjectProperties-001",
        "PERMANENT — needs the CONTRAPOSITIVE of prp-pdw: were Peter = Lois, one pair would \
         lie in BOTH disjoint properties, so full semantics entails `Peter differentFrom \
         Lois`; prp-pdw derives only `false` from an actual shared pair, no rule emits \
         differentFrom between individuals (dt-diff covers literals only), and \
         DifferentIndividuals conclusions are outside PR1's assertion scope",
    ),
    (
        "New-Feature-DisjointObjectProperties-002",
        "PERMANENT — the prp-adp/prp-pdw contrapositive exactly as in \
         New-Feature-DisjointObjectProperties-001, PLUS the conclusion is a reified \
         owl:AllDifferent/owl:distinctMembers structure that no rule head constructs",
    ),
    (
        "owl2-rl-rules-fp-differentFrom",
        "PERMANENT — needs the CONTRAPOSITIVE of prp-fp: were Y1 = Y2, prp-fp would merge \
         X1/X2 against their differentFrom, so full semantics entails `Y1 differentFrom \
         Y2`; prp-fp requires the SAME subject and derives only sameAs, and no rule emits \
         differentFrom between individuals (dt-diff covers literals only)",
    ),
    (
        "owl2-rl-rules-ifp-differentFrom",
        "PERMANENT — the prp-ifp CONTRAPOSITIVE, symmetric to the fp case: were X1 = X2, \
         prp-ifp would merge Y1/Y2 against their differentFrom; prp-ifp derives only \
         sameAs, and differentFrom between individuals has no producing rule",
    ),
    (
        "New-Feature-ReflexiveProperty-001",
        "PERMANENT — the premise's ReflexiveObjectProperty is EXCLUDED from the RL grammar \
         (Profiles §4.2: all OWL 2 axioms 'apart from disjoint unions of classes and \
         reflexive object property axioms'), the export's RL tag notwithstanding; \
         accordingly no prp-rfx rule exists, so `Peter knows Peter` is underivable",
    ),
    (
        "WebOnt-I5.8-008",
        "PERMANENT — a TBox rdfs:range conclusion needing value-space INTERSECTION \
         (xsd:short ∩ xsd:unsignedInt ⊑ xsd:unsignedShort); scm-rng1 only propagates a \
         range UP existing subClassOf edges, no dt-*/scm-* rule intersects datatype \
         ranges, and PR1 is assertion-only",
    ),
    (
        "WebOnt-I5.8-009",
        "PERMANENT — as WebOnt-I5.8-008 with xsd:nonNegativeInteger ∩ \
         xsd:nonPositiveInteger = {0} ⊑ xsd:short: datatype-range intersection is beyond \
         the RL/RDF dt-* rules, and the rdfs:range conclusion is a TBox axiom outside PR1",
    ),
];

pub fn notes() -> Vec<String> {
    vec![
        "Source: the OWL WG test-repository export (all.rdf, pinned snapshot — see \
         scripts/fetch-inference-suites.sh). Selection: `test:profile test:RL` AND \
         `test:semantics test:RDF-BASED`; each selected case yields one row per declared \
         check (consistency / inconsistency / positive / negative entailment). Cases not \
         RL-profiled or direct-semantics-only are outside the RL applicability rule and \
         are counted in the selection-summary row, not as individual rows."
            .to_string(),
        "Method: premise → `sparq_reason::materialize_owl_rl` → \
         `sparq_reason::inconsistencies` / bnode-homomorphism entailment check; \
         ontology-header triples stripped on both sides. The RL/RDF rules are by design \
         incomplete for arbitrary (TBox) conclusions under the RDF-based semantics \
         (conformance doc §2.3, theorem PR1) — such fails are listed, not hidden."
            .to_string(),
    ]
}

struct Case {
    ident: String,
    status: Option<String>,
    checks: Vec<&'static str>,
    premise: Option<String>,
    conclusion: Option<String>,
    nonconclusion: Option<String>,
    imports: bool,
}

pub fn run_suite(export: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let text = std::fs::read_to_string(export).map_err(|e| {
        format!(
            "read {}: {e} — run scripts/fetch-inference-suites.sh",
            export.display()
        )
    })?;
    // oxrdfxml rejects single-quoted DOCTYPE entity values; normalize them.
    let fixed = fix_doctype_quotes(&text);
    let parser = oxrdfxml::RdfXmlParser::new()
        .with_base_iri("http://owl.semanticweb.org/exports/all.rdf")
        .map_err(|e| e.to_string())?;
    let mut triples = Vec::new();
    for t in parser.for_slice(fixed.as_bytes()) {
        triples.push(t.map_err(|e| format!("all.rdf: {e}"))?);
    }
    let g = MiniGraph { triples };

    let mut total = 0usize;
    let mut not_rl = 0usize;
    let mut not_rdf_based = 0usize;
    for case_node in g.subjects_with_type(&format!("{T}TestCase")) {
        let types = g.types_of(&case_node);
        let has = |t: &str| types.iter().any(|x| x == &format!("{T}{t}"));
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
        if checks.is_empty() {
            continue; // ProfileIdentificationTest-only — not a reasoning check
        }
        total += 1;
        let iri_objs = |p: &str| -> Vec<String> {
            g.objects(&case_node, &format!("{T}{p}"))
                .into_iter()
                .filter_map(|t| match t {
                    Term::NamedNode(n) => Some(n.as_str().to_string()),
                    _ => None,
                })
                .collect()
        };
        if !iri_objs("profile").iter().any(|p| p == &format!("{T}RL")) {
            not_rl += 1;
            continue;
        }
        if !iri_objs("semantics")
            .iter()
            .any(|s| s == &format!("{T}RDF-BASED"))
        {
            not_rdf_based += 1;
            continue;
        }

        let lit = |p: &str| -> Option<String> { g.str_object(&case_node, &format!("{T}{p}")) };
        let case = Case {
            ident: lit("identifier").unwrap_or_else(|| match &case_node {
                NamedOrBlankNode::NamedNode(n) => n
                    .as_str()
                    .rsplit('/')
                    .next()
                    .unwrap_or(n.as_str())
                    .to_string(),
                NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
            }),
            status: g
                .object(&case_node, &format!("{T}status"))
                .and_then(|t| match t {
                    Term::NamedNode(n) => n.as_str().strip_prefix(T).map(|s| s.to_string()),
                    _ => None,
                }),
            checks,
            premise: lit("rdfXmlPremiseOntology").or_else(|| lit("rdfXmlInputOntology")),
            conclusion: lit("rdfXmlConclusionOntology"),
            nonconclusion: lit("rdfXmlNonConclusionOntology"),
            imports: g
                .object(&case_node, &format!("{T}importedOntology"))
                .is_some()
                || g.object(&case_node, &format!("{T}importedOntologyIRI"))
                    .is_some(),
        };
        run_case(case, out);
    }
    out.push(TestResult {
        suite: "owl2-rl/selection".into(),
        name: format!(
            "{total} reasoning test cases in the export; selected = RL-profile ∧ RDF-based \
             semantics (excluded: {not_rl} not in the RL profile, {not_rdf_based} \
             direct-semantics-only)"
        ),
        outcome: Outcome::OutOfScope("selection summary (informational row)".into()),
    });
    Ok(())
}

fn run_case(case: Case, out: &mut Vec<TestResult>) {
    let ident = case.ident.clone();
    let mut push = |kind: &str, outcome: Outcome| {
        // A fail on a documented-divergence test is reported as the divergence,
        // with the observed reason attached. A divergence-listed test that
        // PASSES is reported as a pass (flagging the stale entry).
        let outcome = match outcome {
            Outcome::Fail(observed) => match DOCUMENTED_DIVERGENCES
                .iter()
                .find(|(name, _)| *name == ident)
            {
                Some((_, rationale)) => Outcome::Divergence(rationale, observed),
                None => Outcome::Fail(observed),
            },
            other => other,
        };
        out.push(TestResult {
            suite: format!("owl2-rl/{kind}"),
            name: ident.clone(),
            outcome,
        });
    };
    if case.status.as_deref() != Some("Approved") {
        let why = format!(
            "status {} (only Approved cases are conformance tests)",
            case.status.as_deref().unwrap_or("absent")
        );
        for kind in &case.checks {
            push(kind, Outcome::OutOfScope(why.clone()));
        }
        return;
    }
    if case.imports {
        for kind in &case.checks {
            push(
                kind,
                Outcome::OutOfScope("uses owl:imports (no dereferencing in the harness)".into()),
            );
        }
        return;
    }
    let Some(premise_xml) = case.premise.clone() else {
        for kind in &case.checks {
            push(
                kind,
                Outcome::OutOfScope("premise only available in functional syntax".into()),
            );
        }
        return;
    };
    let base = format!("http://owl.semanticweb.org/id/{}", case.ident);
    let premise = match parse_ontology(&premise_xml, &base) {
        Ok(rows) => rows,
        Err(e) => {
            for kind in &case.checks {
                push(
                    kind,
                    Outcome::Fail(format!("premise RDF/XML parse error: {e}")),
                );
            }
            return;
        }
    };

    // Materialize once per case, under a watchdog (a hang/panic in the
    // reasoner is a recorded FAIL, not a dead harness).
    let premise_for_thread = premise.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(move || {
            let mut dict = sparq_core::dict::Dict::new();
            let mut ids: Vec<[sparq_core::dict::Id; 3]> = premise_for_thread
                .iter()
                .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
                .collect();
            sparq_reason::materialize_owl_rl(&mut dict, &mut ids);
            let clashes = sparq_reason::inconsistencies(&dict, &ids);
            let rows: Vec<Row> = ids
                .into_iter()
                .map(|[s, p, o]| [dict.term(s), dict.term(p), dict.term(o)])
                .collect();
            (rows, clashes)
        });
        let _ = tx.send(result);
    });
    let (closure, clashes) = match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            for kind in &case.checks {
                push(kind, Outcome::Fail("reasoner panicked".into()));
            }
            return;
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            for kind in &case.checks {
                push(
                    kind,
                    Outcome::Fail("timeout (20s) in materialization".into()),
                );
            }
            return;
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            for kind in &case.checks {
                push(kind, Outcome::Fail("reasoner panicked".into()));
            }
            return;
        }
    };

    let d = Recognized::standard();
    for kind in case.checks.clone() {
        let outcome = match kind {
            "consistency" => {
                if clashes.is_empty() {
                    Outcome::Pass
                } else {
                    Outcome::Fail(format!(
                        "wrongly judged inconsistent: {}",
                        clashes.join("; ")
                    ))
                }
            }
            "inconsistency" => {
                if clashes.is_empty() {
                    Outcome::Fail("inconsistency not detected".into())
                } else {
                    Outcome::Pass
                }
            }
            "positive-entailment" => match &case.conclusion {
                None => {
                    Outcome::OutOfScope("conclusion only available in functional syntax".into())
                }
                Some(xml) => match parse_ontology(xml, &base) {
                    Err(e) => Outcome::Fail(format!("conclusion RDF/XML parse error: {e}")),
                    Ok(conclusion) => {
                        // Finite-restriction datatype axioms: every recognized
                        // datatype is an rdfs:Datatype in any OWL 2 datatype
                        // map; add the typing for the datatypes the CONCLUSION
                        // mentions (the conclusion-vocabulary device rdf-mt
                        // uses for its infinite axiomatic sets).
                        let mut closure = closure.clone();
                        for row in &conclusion {
                            for t in row {
                                if let Term::NamedNode(n) = t {
                                    if d.contains(n.as_str()) {
                                        closure.push([
                                            t.clone(),
                                            Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                                                RDF_TYPE,
                                            )),
                                            Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                                                oxrdf::vocab::rdfs::DATATYPE.as_str(),
                                            )),
                                        ]);
                                    }
                                }
                            }
                        }
                        if !clashes.is_empty() || entail::entails(&closure, &conclusion, &d) {
                            Outcome::Pass
                        } else {
                            Outcome::Fail(
                                "conclusion not entailed by the RL/RDF-rules closure".into(),
                            )
                        }
                    }
                },
            },
            "negative-entailment" => match &case.nonconclusion {
                None => {
                    Outcome::OutOfScope("non-conclusion only available in functional syntax".into())
                }
                Some(xml) => match parse_ontology(xml, &base) {
                    Err(e) => Outcome::Fail(format!("non-conclusion RDF/XML parse error: {e}")),
                    Ok(nonconclusion) => {
                        if !clashes.is_empty() {
                            Outcome::Fail(
                                "premise wrongly judged inconsistent (entails everything)".into(),
                            )
                        } else if entail::entails(&closure, &nonconclusion, &d) {
                            Outcome::Fail("non-conclusion wrongly entailed".into())
                        } else {
                            Outcome::Pass
                        }
                    }
                },
            },
            _ => unreachable!(),
        };
        push(kind, outcome);
    }
}

/// Parses an inline RDF/XML ontology literal, dropping the ontology header
/// (`?x rdf:type owl:Ontology` typings and `owl:imports` edges) — the harness
/// compares axiom triples, mirroring the official OWLWG harness.
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

/// The export's internal DTD uses single-quoted ENTITY values, which oxrdfxml
/// rejects; rewrite just the DOCTYPE block to double quotes.
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
    use super::DOCUMENTED_DIVERGENCES;

    /// [FABLE-5] sq-pbz04.1.3 — the per-divergence disposition pass. Every
    /// documented divergence must carry an explicit PERMANENT disposition
    /// grounded in the OWL 2 Profiles REC (a named rule from the §4.3 tables,
    /// the §4.2 grammar exclusion, or theorem PR1's assertion-only completeness
    /// scope). Growing the list, or replacing a grounding with a hand-wave,
    /// must be a conscious reviewed act: the count, uniqueness, and grounding
    /// are pinned here so a divergence can never be added (hiding a real fail)
    /// or reworded into vagueness silently.
    #[test]
    fn divergence_dispositions_are_permanent_and_rule_grounded() {
        assert_eq!(
            DOCUMENTED_DIVERGENCES.len(),
            13,
            "the divergence list changes only via a reviewed disposition pass"
        );
        let mut seen = std::collections::BTreeSet::new();
        for (name, rationale) in DOCUMENTED_DIVERGENCES {
            assert!(seen.insert(*name), "duplicate divergence entry: {}", name);
            assert!(
                rationale.starts_with("PERMANENT — "),
                "{}: rationale must open with its disposition tag",
                name
            );
            // A checkable spec anchor: a rule-table rule name, the RL-grammar
            // section, or the PR1 completeness theorem.
            let anchors = [
                "prp-", "cls-", "cax-", "scm-", "eq-diff", "dt-", "§4.2", "PR1",
            ];
            assert!(
                anchors.iter().any(|tok| rationale.contains(tok)),
                "{}: rationale must cite a rule-table / grammar / PR1 grounding",
                name
            );
        }
    }
}
