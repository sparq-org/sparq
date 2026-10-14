//! Unit tests for the N3 admissibility reduction (design §4.3) — the
//! transitive `strongerThan` closure, the reflexive `atLeast` lift, the
//! `gteq` discharge rule, and the Rust default-deny universal.
//!
//! 🤖 SPARQ agent — sq-ufsi9 (epic sq-0dksu). The §4.3.3 worked
//! end-to-end example (empty under the strict preference, non-empty under the
//! relaxed one) is the GOLDEN integration test in
//! `tests/secprop_admissibility.rs`; these pin the rule mechanics.

use super::*;

/// The closure of `PREFIXES + body + ruleset()`, as (subject, predicate, object)
/// IRI-string triples (non-IRI terms dropped — the ruleset concludes only over IRIs).
fn closure_iris(body: &str) -> Vec<(String, String, String)> {
    let doc = format!("{PREFIXES}{body}\n{}", ruleset());
    let cl = sparq_reason::reason_n3_terms(&doc, None).expect("reasoning failed");
    cl.facts
        .iter()
        .filter_map(|[s, p, o]| Some((iri(s)?.to_string(), iri(p)?.to_string(), iri(o)?.to_string())))
        .collect()
}

fn has(c: &[(String, String, String)], s: &str, p: &str, o: &str) -> bool {
    c.iter().any(|(cs, cp, co)| cs == s && cp == p && co == o)
}

const CROSS: &str = "https://w3id.org/zkp-sparql/sec-prop#CrossPresentation";
const PER: &str = "https://w3id.org/zkp-sparql/sec-prop#PerPresentation";
const LINKABLE: &str = "https://w3id.org/zkp-sparql/sec-prop#Linkable";

#[test]
fn stronger_than_is_transitively_closed() {
    let c = closure_iris("");
    // CrossPresentation ⊐ PerPresentation ⊐ Linkable ⇒ CrossPresentation ⊐ Linkable.
    assert!(
        has(&c, CROSS, SECX_STRONGER_THAN, LINKABLE),
        "transitive strongerThan: Cross ⊐ Linkable not derived"
    );
}

#[test]
fn at_least_is_reflexive_and_down_closed() {
    let c = closure_iris("");
    // reflexive: every level on an order chain is atLeast itself.
    assert!(has(&c, PER, SECX_AT_LEAST, PER), "reflexive atLeast on PerPresentation");
    assert!(has(&c, CROSS, SECX_AT_LEAST, CROSS), "reflexive atLeast on CrossPresentation");
    // down-closed: a stronger level is atLeast a weaker one (incl. transitively).
    assert!(has(&c, CROSS, SECX_AT_LEAST, PER), "Cross atLeast Per");
    assert!(has(&c, CROSS, SECX_AT_LEAST, LINKABLE), "Cross atLeast Linkable (transitive)");
    // NOT up: a weaker level is never atLeast a stronger one.
    assert!(!has(&c, LINKABLE, SECX_AT_LEAST, CROSS), "Linkable is NOT atLeast Cross");
    assert!(!has(&c, PER, SECX_AT_LEAST, CROSS), "Per is NOT atLeast Cross");
}

const MALICIOUS: &str = "https://w3id.org/zkp-sparql/sec-prop#Malicious";
const COVERT: &str = "https://w3id.org/zkp-sparql/sec-prop#Covert";
const SEMI_HONEST: &str = "https://w3id.org/zkp-sparql/sec-prop#SemiHonest";
const DISHONEST_MAJORITY: &str = "https://w3id.org/zkp-sparql/sec-prop#DishonestMajority";
const HONEST_MAJORITY: &str = "https://w3id.org/zkp-sparql/sec-prop#HonestMajority";
const SUPER_HONEST_MAJORITY: &str = "https://w3id.org/zkp-sparql/sec-prop#SuperHonestMajority";

/// The two MPC trust-model chains (issue #6186) are real `secx:atLeast` orders, in
/// the ASYMMETRIC direction: `Covert atLeast SemiHonest` but NOT the reverse, and
/// `DishonestMajority atLeast HonestMajority` but NOT the reverse (the threshold axis
/// is ordered by how much corruption is TOLERATED). The asymmetry is the whole point:
/// it is what makes a strict preference EXCLUDE a weaker protocol instead of
/// vacuously admitting it.
#[test]
fn mpc_trust_model_orders_are_asymmetric() {
    let c = closure_iris("");

    // AdversaryModel: Malicious ⊐ Covert ⊐ SemiHonest.
    assert!(has(&c, COVERT, SECX_AT_LEAST, SEMI_HONEST), "Covert atLeast SemiHonest");
    assert!(
        !has(&c, SEMI_HONEST, SECX_AT_LEAST, COVERT),
        "SemiHonest must NOT be atLeast Covert"
    );
    assert!(
        has(&c, MALICIOUS, SECX_AT_LEAST, SEMI_HONEST),
        "Malicious atLeast SemiHonest (transitive)"
    );
    assert!(
        !has(&c, SEMI_HONEST, SECX_AT_LEAST, MALICIOUS),
        "SemiHonest must NOT be atLeast Malicious — a semi-honest protocol cannot \
         discharge `requiresAdversaryModel gteq Malicious`"
    );
    assert!(has(&c, SEMI_HONEST, SECX_AT_LEAST, SEMI_HONEST), "reflexive on SemiHonest");

    // CorruptionThreshold: DishonestMajority ⊐ HonestMajority ⊐ SuperHonestMajority.
    assert!(
        has(&c, DISHONEST_MAJORITY, SECX_AT_LEAST, HONEST_MAJORITY),
        "DishonestMajority atLeast HonestMajority"
    );
    assert!(
        !has(&c, HONEST_MAJORITY, SECX_AT_LEAST, DISHONEST_MAJORITY),
        "HonestMajority must NOT be atLeast DishonestMajority — an honest-majority \
         protocol cannot discharge `requiresCorruptionThreshold gteq DishonestMajority`"
    );
    assert!(
        has(&c, DISHONEST_MAJORITY, SECX_AT_LEAST, SUPER_HONEST_MAJORITY),
        "DishonestMajority atLeast SuperHonestMajority (transitive)"
    );
    assert!(has(&c, HONEST_MAJORITY, SECX_AT_LEAST, HONEST_MAJORITY), "reflexive");
}

/// The discharge rule fires exactly when the held level is `atLeast` the
/// required one — equal level (reflexive) and stronger level both satisfy; a
/// weaker held level does NOT.
#[test]
fn discharge_rule_respects_the_order() {
    // method m1 holds PerPresentation; m2 holds CrossPresentation.
    let data = "\
zk:m1 secx:hasProperty [ secx:property secx:UnlinkabilityScope ; secx:level secx:PerPresentation ] .\n\
zk:m2 secx:hasProperty [ secx:property secx:UnlinkabilityScope ; secx:level secx:CrossPresentation ] .\n\
# constraint cReq: UnlinkabilityScope gteq PerPresentation\n\
zk:cReqPer odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:PerPresentation .\n\
# constraint cReqCross: UnlinkabilityScope gteq CrossPresentation\n\
zk:cReqCross odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:CrossPresentation .\n";
    let c = closure_iris(data);
    let sat = "https://w3id.org/zkp-sparql/sec-prop#satisfies";
    let m1 = "https://sparq.dev/ns/zk#m1";
    let m2 = "https://sparq.dev/ns/zk#m2";
    let creq_per = "https://sparq.dev/ns/zk#cReqPer";
    let creq_cross = "https://sparq.dev/ns/zk#cReqCross";
    // equal level satisfies (reflexive atLeast):
    assert!(has(&c, m1, sat, creq_per), "Per satisfies gteq Per (reflexive)");
    // weaker held level does NOT satisfy a stronger requirement:
    assert!(!has(&c, m1, sat, creq_cross), "Per does NOT satisfy gteq Cross");
    // stronger held level satisfies both:
    assert!(has(&c, m2, sat, creq_per), "Cross satisfies gteq Per (down-closed)");
    assert!(has(&c, m2, sat, creq_cross), "Cross satisfies gteq Cross (reflexive)");
}

/// The Rust default-deny universal: admissible iff EVERY constraint is satisfied;
/// the unsatisfied list names the failures.
#[test]
fn admissible_is_default_deny_over_all_constraints() {
    // m holds Per; policy needs both gteq-Per (ok) and gteq-Cross (fails).
    let annotations =
        "zk:m secx:hasProperty [ secx:property secx:UnlinkabilityScope ; secx:level secx:PerPresentation ] .";
    let policy = "\
zk:cPer   odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:PerPresentation .\n\
zk:cCross odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:CrossPresentation .\n";
    let m = "https://sparq.dev/ns/zk#m";
    let c_per = "https://sparq.dev/ns/zk#cPer";
    let c_cross = "https://sparq.dev/ns/zk#cCross";

    // Both constraints: NOT admissible (the gteq-Cross one fails).
    let a = admissible(m, &[c_per, c_cross], policy, annotations).expect("reason");
    assert!(!a.admissible, "must be inadmissible: one constraint unsatisfied");
    assert_eq!(a.unsatisfied, vec![c_cross.to_string()], "the gteq-Cross constraint is named");

    // Only the satisfiable constraint: admissible.
    let a2 = admissible(m, &[c_per], policy, annotations).expect("reason");
    assert!(a2.admissible, "must be admissible: the only constraint is satisfied");
    assert!(a2.unsatisfied.is_empty());
}

/// A method annotated for a DIFFERENT dimension than the constraint asks about
/// does not accidentally satisfy it (the overDimension mapping is load-bearing).
#[test]
fn wrong_dimension_does_not_satisfy() {
    let annotations =
        "zk:m secx:hasProperty [ secx:property secx:ZeroKnowledgeType ; secx:level secx:PerfectZK ] .";
    let policy = "zk:c odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:Linkable .";
    let a = admissible(
        "https://sparq.dev/ns/zk#m",
        &["https://sparq.dev/ns/zk#c"],
        policy,
        annotations,
    )
    .expect("reason");
    assert!(!a.admissible, "a ZK annotation must not discharge an unlinkability constraint");
}

/// FAIL-CLOSED on an unreduced operator: only `odrl:gteq` is discharged. A
/// constraint with `odrl:lt` (or any other operator) yields no `secx:satisfies`
/// fact, so the method is DENIED — never accidentally admitted.
#[test]
fn non_gteq_operator_is_fail_closed() {
    // The method amply exceeds the requirement, but the operator is `lt`, not `gteq`.
    let annotations =
        "zk:m secx:hasProperty [ secx:property secx:UnlinkabilityScope ; secx:level secx:CrossPresentation ] .";
    let policy = "zk:c odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:lt ; odrl:rightOperand secx:Linkable .";
    let a = admissible(
        "https://sparq.dev/ns/zk#m",
        &["https://sparq.dev/ns/zk#c"],
        policy,
        annotations,
    )
    .expect("reason");
    assert!(!a.admissible, "an unreduced operator (odrl:lt) must fail closed, not admit");
}

/// `ruleset()` is the documented concatenation, and the order/closure/discharge
/// constants are all present (a drift guard for the published rule text).
#[test]
fn ruleset_concatenates_the_three_parts() {
    let rs = ruleset();
    assert!(rs.contains("secx:strongerThan secx:PerPresentation"), "level orders present");
    assert!(rs.contains("=> { ?a secx:strongerThan ?c }"), "transitive closure rule present");
    assert!(rs.contains("secx:satisfies ?c"), "discharge rule present");
    assert!(rs.contains("secx:overDimension"), "leftOperand→dimension map present");
}
