//! [OPUS-5] sq-zgbso.2 — regression pins for the TWO confirmed fail-OPEN defects the
//! review of PR #3458 found by execution in the N3-stratum ODRL path.
//!
//! This is ACCESS CONTROL: a fail-open here GRANTS access that the Rust reference path
//! denies. Each test below reproduces the reviewer's exact scenario and pins the fixed
//! behaviour; each one is red on the pre-fix tree (see the PR body for the recorded
//! before/after and the per-fix mutation runs).
//!
//! **Defect 1 — an expired permission GRANTED.** A constraint node carrying TWO operand
//! triples (`recipient eq alice`, satisfied; `dateTime lteq 2000-01-01`, expired)
//! evaluated to `granted=false` on the Rust reference path but `granted=true` on the N3
//! path. Root cause: `odrlx:atomicSat` is keyed on the CONSTRAINT NODE, so any single
//! satisfiable operand marked the whole node satisfied — "some operand is satisfied"
//! standing in for "every operand is satisfied". Separately
//! `prohibition_atomic_supported` read only `.next()` of leftOperand/operator and was
//! structurally blind to the extra operands.
//!
//! **Defect 2 — the conflict-refusal guard was never called.** `materialize_odrl_n3`
//! never consulted `refuse_unimplementable_conflict`, so a policy carrying
//! `odrl:conflict odrl:perm` (or an unknown strategy IRI) yielded Rust `refused=true`
//! but an N3-materialized grant `urn:alice auth:read urn:t/1`.
//!
//! Gated on the default-OFF `odrl-bridge` feature (requires sparq-policy), and run by
//! the `opt-in sparq-solid (odrl-bridge)` CI feature-matrix leg.
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{parse_policy_str, Request};
use sparq_solid::odrl_bridge::{materialize_odrl_n3, materialize_policy};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const TARGET: &str = "urn:t/1";

fn req(action_local: &str, party: &str, at: Option<&str>) -> Request {
    let mut r = Request::new(format!("{}{}", ODRL, action_local)).on(TARGET).by(party);
    if let Some(t) = at {
        r = r.at(t);
    }
    r
}

/// The Rust reference decision for `policy_ttl` — the path the N3 path must not be more
/// permissive than.
fn rust_outcome(policy_ttl: &str, request: &Request) -> sparq_solid::odrl_bridge::BridgeOutcome {
    let policy = parse_policy_str(policy_ttl, "turtle").expect("policy parses");
    let mut graph = Graph::new();
    materialize_policy(&mut graph, &policy, request)
}

// ── Defect 1: the two-operand expired permission ─────────────────────────────

/// The reviewer's EXACT scenario: ONE constraint node carrying two operand triples —
/// `recipient eq <urn:alice>` (satisfied by the requesting party) and
/// `dateTime lteq 2000-01-01` (long expired).
const POL_TWO_OPERAND_EXPIRED: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/two-operand> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:constraint _:c ] .
_:c odrl:leftOperand odrl:recipient , odrl:dateTime ;
    odrl:operator    odrl:eq , odrl:lteq ;
    odrl:rightOperand <urn:alice> , "2000-01-01T00:00:00Z"^^xsd:dateTime ."#;

#[test]
fn defect1_two_operand_expired_permission_is_not_granted_by_n3() {
    let request = req("read", "urn:alice", Some("2026-07-25T00:00:00Z"));

    // The Rust reference path denies: the two right-operands under a non-set operator
    // fold to an ambiguous (unsatisfiable) guard — fail-closed.
    let rust = rust_outcome(POL_TWO_OPERAND_EXPIRED, &request);
    assert!(!rust.granted, "Rust reference path must NOT grant the expired permission");

    // The N3 path must not be more permissive. A constraint node whose operands it
    // cannot fully evaluate is REFUSED outright (fail-closed), materializing nothing.
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL_TWO_OPERAND_EXPIRED, &request);
    assert!(
        out.is_err(),
        "N3 path must refuse a multi-operand constraint node it cannot fully evaluate, \
         got {:?}",
        out
    );
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

/// The same fail-open shape with only the RIGHT operand duplicated
/// (`recipient eq <urn:alice>, <urn:bob>`): the Rust evaluator folds two right-operands
/// under the non-set `eq` into the unsatisfiable guard, while the pre-fix strata bound
/// `?r` to `<urn:alice>` and marked the node satisfied.
#[test]
fn defect1_multi_right_operand_constraint_is_not_granted_by_n3() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/multi-right> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:constraint _:c ] .
_:c odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
    odrl:rightOperand <urn:alice> , <urn:bob> ."#;
    let request = req("read", "urn:alice", None);
    assert!(!rust_outcome(POL, &request).granted, "Rust reference must NOT grant");
    let mut graph = Graph::new();
    assert!(
        materialize_odrl_n3(&mut graph, POL, &request).is_err(),
        "N3 path must refuse a multi-right-operand constraint node"
    );
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

/// The DENY-side mirror, and the reason the driver-level guard is load-bearing on its
/// own: with only the rules-layer fix a multi-operand PROHIBITION constraint derives no
/// `odrlx:atomicSat`, so its deny would be silently DROPPED — widening access. The
/// driver must refuse the whole call instead. This is also the direct regression pin for
/// `prohibition_atomic_supported`'s `.next()` blindness: pre-fix it inspected only the
/// FIRST leftOperand/operator pair (`recipient`/`eq` — in scope) and waved the node
/// through.
#[test]
fn defect1_multi_operand_prohibition_is_refused_not_silently_dropped() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two-operand-proh> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:constraint _:c ] .
_:c odrl:leftOperand odrl:recipient , odrl:purpose ;
    odrl:operator    odrl:eq ;
    odrl:rightOperand <urn:alice> ."#;
    let request = req("read", "urn:alice", None);
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &request);
    assert!(
        out.is_err(),
        "a prohibition constraint carrying an operand the strata cannot evaluate must be \
         REFUSED, never silently ignored (dropping a deny widens access), got {:?}",
        out
    );
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

// ── Defect 2: the conflict-refusal guard ─────────────────────────────────────

/// `odrl:conflict odrl:perm` — permissions override prohibitions — is unrepresentable by
/// the bridge's `∪ allow ∖ ∪ deny` enforcement. The Rust path returns `refused=true` and
/// materializes nothing; the N3 path materialized the grant `urn:alice auth:read urn:t/1`.
const POL_CONFLICT_PERM: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/conflict-perm> a odrl:Set ; odrl:conflict odrl:perm ;
    odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;

#[test]
fn defect2_conflict_perm_policy_is_refused_by_n3() {
    let request = req("read", "urn:alice", None);

    // Rust reference: a loud, fail-closed refusal — no grant, no deny.
    let rust = rust_outcome(POL_CONFLICT_PERM, &request);
    assert!(rust.refused, "Rust reference path must REFUSE odrl:conflict odrl:perm");
    assert!(!rust.granted && !rust.prohibited, "a refusal materializes nothing");

    // N3 path must refuse too — never materialize the grant.
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL_CONFLICT_PERM, &request);
    assert!(
        out.is_err(),
        "N3 path must refuse an unimplementable odrl:conflict strategy, got {:?}",
        out
    );
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

#[test]
fn defect2_unknown_conflict_strategy_iri_is_refused_by_n3() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/conflict-unknown> a odrl:Set ; odrl:conflict <urn:made-up/strategy> ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;
    let request = req("read", "urn:alice", None);

    let rust = rust_outcome(POL, &request);
    assert!(rust.refused, "Rust reference path must REFUSE an unknown odrl:conflict IRI");
    assert!(!rust.granted, "a refusal materializes nothing");

    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &request);
    assert!(
        out.is_err(),
        "N3 path must refuse an unknown odrl:conflict strategy IRI, got {:?}",
        out
    );
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

/// `odrl:conflict odrl:invalid` (the ODRL default) voids a CONFLICTING policy as a whole —
/// which the bridge cannot represent. The admissibility rule is conditional: refused iff a
/// permission/prohibition conflict is actually detected. Both halves are pinned so the N3
/// path tracks `conflict_admissibility` exactly rather than a coarser approximation.
#[test]
fn defect2_conflict_invalid_tracks_rust_admissibility_both_ways() {
    const CONFLICTING: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inv-c> a odrl:Set ; odrl:conflict odrl:invalid ;
    odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;
    const UNCONFLICTED: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inv-u> a odrl:Set ; odrl:conflict odrl:invalid ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;
    let request = req("read", "urn:alice", None);

    // Conflicting → Rust refuses → N3 must refuse.
    assert!(rust_outcome(CONFLICTING, &request).refused, "Rust refuses the conflicting policy");
    let mut g1 = Graph::new();
    assert!(
        materialize_odrl_n3(&mut g1, CONFLICTING, &request).is_err(),
        "N3 must refuse odrl:conflict odrl:invalid with a detected conflict"
    );
    assert!(g1.named.is_empty(), "nothing may be materialized on a refusal");

    // No conflict → admissible on BOTH paths, and the grant is still emitted (the guard
    // must not degenerate into "refuse every odrl:conflict policy").
    let rust = rust_outcome(UNCONFLICTED, &request);
    assert!(!rust.refused && rust.granted, "Rust admits the unconflicted invalid-policy");
    let mut g2 = Graph::new();
    let out = materialize_odrl_n3(&mut g2, UNCONFLICTED, &request)
        .expect("N3 admits the unconflicted invalid-policy");
    assert!(out.granted, "the unconflicted policy still grants on the N3 path");
}

// ── Multi-valued rule attributes (action/target/assignee) ────────────────────
//
// The Rust reference evaluator selects exactly ONE object per rule attribute
// (`first_str` — the first bound value), whereas the N3 strata bind
// odrl:action/target/assignee as triple patterns and so match a request that agrees
// with ANY asserted value. A rule asserting two assignees/targets/actions could
// therefore fire an N3 grant (or a prohibition match) for a value the Rust path never
// selected — widening. `validate_policy_for_n3` refuses such a rule outright.

/// A permission naming TWO assignees. The Rust path selects one; the N3 grant stratum
/// would grant BOTH — the widening this guard closes.
const POL_TWO_ASSIGNEE_PERM: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two-assignee> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ;
    odrl:assignee <urn:alice> , <urn:bob> ] ."#;

#[test]
fn multi_valued_assignee_permission_is_refused_by_n3() {
    // The Rust reference path selects exactly ONE assignee, so exactly one of the two
    // parties is granted — demonstrating the single-value selection the N3 pattern match
    // does not share.
    let alice = rust_outcome(POL_TWO_ASSIGNEE_PERM, &req("read", "urn:alice", None)).granted;
    let bob = rust_outcome(POL_TWO_ASSIGNEE_PERM, &req("read", "urn:bob", None)).granted;
    assert!(
        alice ^ bob,
        "the Rust reference path selects exactly one assignee (alice={alice}, bob={bob})"
    );

    // Whichever party the Rust path did NOT select is the grant the N3 path must not
    // widen into. The driver refuses the multi-valued rule outright, materializing nothing
    // for EITHER party — the graph is left unchanged.
    for party in ["urn:alice", "urn:bob"] {
        let mut graph = Graph::new();
        let out = materialize_odrl_n3(&mut graph, POL_TWO_ASSIGNEE_PERM, &req("read", party, None));
        assert!(
            out.is_err(),
            "N3 path must refuse a multi-assignee permission (party {party}), got {:?}",
            out
        );
        assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
    }
}

#[test]
fn multi_valued_target_permission_is_refused_by_n3() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two-target> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:assignee <urn:alice> ;
    odrl:target <urn:t/1> , <urn:t/2> ] ."#;
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &req("read", "urn:alice", None));
    assert!(out.is_err(), "N3 path must refuse a multi-target permission, got {:?}", out);
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

#[test]
fn multi_valued_action_permission_is_refused_by_n3() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two-action> a odrl:Set ; odrl:permission [
    odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:action odrl:read , odrl:write ] ."#;
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &req("write", "urn:alice", None));
    assert!(out.is_err(), "N3 path must refuse a multi-action permission, got {:?}", out);
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

#[test]
fn multi_valued_assignee_prohibition_is_refused_by_n3() {
    // A prohibition with two assignees: refused for the same reason (and the deny side is
    // where a subset reading is most dangerous — a dropped deny widens access).
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two-assignee-proh> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:t/1> ;
    odrl:assignee <urn:alice> , <urn:bob> ] ."#;
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &req("read", "urn:alice", None));
    assert!(out.is_err(), "N3 path must refuse a multi-assignee prohibition, got {:?}", out);
    assert!(graph.named.is_empty(), "nothing may be materialized on a refusal");
}

/// The guard must suppress ONLY multi-valued attributes: a single-valued permission
/// still materializes its grant on the N3 path (else the tests above would pass on a
/// path that refuses everything).
#[test]
fn single_valued_rule_attributes_still_grant_on_n3() {
    const POL: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/single> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;
    let mut graph = Graph::new();
    let out = materialize_odrl_n3(&mut graph, POL, &req("read", "urn:alice", None))
        .expect("N3 admits a single-valued permission");
    assert!(out.granted, "a single-valued permission still grants on the N3 path");
}

// ── The RULES layer on its own ───────────────────────────────────────────────
//
// The driver refuses an ambiguous constraint node before reasoning, so the tests above
// cannot tell whether `rules/odrl-a0.n3` does anything. These drive the five strata
// DIRECTLY, bypassing the driver's validator, and pin the rule-layer half of the fix:
// with the A0 guard removed they derive the fail-OPEN grant again.
//
// The two layers deliberately overlap and neither subsumes the other: A0 closes the
// GRANT direction inside the rules, while the driver's refusal is the only disposition
// that is also safe on the DENY side (suppressing an ambiguous prohibition's
// satisfaction would drop its deny and widen access).

use sparq_core::dict::Dict;
use sparq_reason::{reason_n3_with_cycles, NegationCycles};

const ODRL_A0: &str = include_str!("../rules/odrl-a0.n3");
const ODRL_A: &str = include_str!("../rules/odrl-a.n3");
const ODRL_B: &str = include_str!("../rules/odrl-b.n3");
const ODRL_C: &str = include_str!("../rules/odrl-c.n3");
const ODRL_D: &str = include_str!("../rules/odrl-d.n3");

/// Run the five strata over `facts` exactly as `materialize_odrl_n3` chains them, and
/// return the `auth:`-namespace triples of the final closure as `"s p o"` strings.
fn strata_auth_triples(facts: &str) -> Vec<String> {
    let mut src = facts.to_owned();
    let mut out = Vec::new();
    // odrl-b/c/d.n3 conclude a variable predicate next to store-scoped negation, so they
    // cannot be stratified yet: they keep single-pass semantics by explicit opt-in, as in
    // `odrl_bridge`. Follow-up: rewrite them with one concrete conclusion per mode and drop
    // the opt-in.
    let (strict, legacy) = (NegationCycles::Reject, NegationCycles::SinglePass);
    for (rules, cycles) in
        [(ODRL_A0, strict), (ODRL_A, strict), (ODRL_B, legacy), (ODRL_C, legacy), (ODRL_D, legacy)]
    {
        let mut dict = Dict::new();
        let closure = reason_n3_with_cycles(&mut dict, &format!("{}\n{}", src, rules), cycles)
            .unwrap_or_else(|e| panic!("stratum must reason: {}", e));
        let mut next = String::new();
        out.clear();
        for t in &closure {
            let (s, p, o) = (dict.term(t[0]), dict.term(t[1]), dict.term(t[2]));
            next.push_str(&format!("{} {} {} .\n", s, p, o));
            if format!("{}", p).contains("https://sparq.dev/ns/auth#") {
                out.push(format!("{} {} {}", s, p, o));
            }
        }
        src = next;
    }
    out
}

/// The request facts `serialize_request_n3` emits for alice reading `urn:t/1` at a time
/// long after the policy's `dateTime lteq 2000-01-01` bound has lapsed.
const REQ_FACTS: &str = r#"<urn:odrl-req> a <http://www.w3.org/ns/odrl/2/Request> ;
  <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> ;
  <http://www.w3.org/ns/odrl/2/target> <urn:t/1> ;
  <http://www.w3.org/ns/odrl/2/assignee> <urn:alice> ;
  <http://www.w3.org/ns/odrl/2/dateTime> "2026-07-25T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
"#;

#[test]
fn rules_layer_does_not_satisfy_a_multi_operand_constraint_node() {
    // The reviewer's exact node, fed straight to the strata: `recipient eq alice`
    // (satisfied) AND `dateTime lteq 2000-01-01` (expired) on ONE constraint node.
    let facts = format!(
        r#"{}<urn:pol/1> <http://www.w3.org/ns/odrl/2/permission> <urn:perm/1> .
<urn:perm/1> <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> ;
  <http://www.w3.org/ns/odrl/2/target> <urn:t/1> ;
  <http://www.w3.org/ns/odrl/2/assignee> <urn:alice> ;
  <http://www.w3.org/ns/odrl/2/constraint> <urn:c/1> .
<urn:c/1> <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/recipient> ,
    <http://www.w3.org/ns/odrl/2/dateTime> ;
  <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> ,
    <http://www.w3.org/ns/odrl/2/lteq> ;
  <http://www.w3.org/ns/odrl/2/rightOperand> <urn:alice> ,
    "2000-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
"#,
        REQ_FACTS
    );
    let auth = strata_auth_triples(&facts);
    assert!(
        auth.is_empty(),
        "the strata must not satisfy a constraint node whose operands they cannot fully \
         evaluate — one satisfiable cross-product combination is NOT 'every operand is \
         satisfied'; derived: {:?}",
        auth
    );
}

#[test]
fn rules_layer_still_satisfies_a_single_operand_constraint_node() {
    // The guard must suppress ONLY ambiguous nodes: the same permission with a single,
    // satisfied operand tuple still grants. Without this the test above would pass just
    // as well on strata that grant nothing at all.
    let facts = format!(
        r#"{}<urn:pol/1> <http://www.w3.org/ns/odrl/2/permission> <urn:perm/1> .
<urn:perm/1> <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> ;
  <http://www.w3.org/ns/odrl/2/target> <urn:t/1> ;
  <http://www.w3.org/ns/odrl/2/assignee> <urn:alice> ;
  <http://www.w3.org/ns/odrl/2/constraint> <urn:c/1> .
<urn:c/1> <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/recipient> ;
  <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> ;
  <http://www.w3.org/ns/odrl/2/rightOperand> <urn:alice> .
"#,
        REQ_FACTS
    );
    let auth = strata_auth_triples(&facts);
    assert_eq!(
        auth,
        vec!["<urn:alice> <https://sparq.dev/ns/auth#read> <urn:t/1>".to_owned()],
        "an unambiguous, satisfied constraint must still grant"
    );
}

#[test]
fn rules_layer_does_not_satisfy_a_mixed_combinator_logical_constraint() {
    // One node carrying BOTH `odrl:or` (satisfied member) and `odrl:and` (unsatisfied
    // member): the or-rule would satisfy it from the one member while the and-rule reads
    // the SAME node as a conjunction — the defect-1 quantifier hole one level up.
    let facts = format!(
        r#"{}<urn:pol/1> <http://www.w3.org/ns/odrl/2/permission> <urn:perm/1> .
<urn:perm/1> <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> ;
  <http://www.w3.org/ns/odrl/2/target> <urn:t/1> ;
  <http://www.w3.org/ns/odrl/2/assignee> <urn:alice> ;
  <http://www.w3.org/ns/odrl/2/constraint> <urn:lc/1> .
<urn:lc/1> <http://www.w3.org/ns/odrl/2/or> <urn:c/sat> ;
  <http://www.w3.org/ns/odrl/2/and> <urn:c/unsat> .
<urn:c/sat> <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/recipient> ;
  <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> ;
  <http://www.w3.org/ns/odrl/2/rightOperand> <urn:alice> .
<urn:c/unsat> <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/dateTime> ;
  <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/lteq> ;
  <http://www.w3.org/ns/odrl/2/rightOperand> "2000-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
"#,
        REQ_FACTS
    );
    let auth = strata_auth_triples(&facts);
    assert!(
        auth.is_empty(),
        "a logical-constraint node with two different combinators has no single reading; \
         neither lcSat rule may fire; derived: {:?}",
        auth
    );
}
