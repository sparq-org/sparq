//! The static analyser's reasoning THROUGH compound `odrl:LogicalConstraint`s —
//! `odrl:and` / `odrl:or` / `odrl:xone` in both the premise (narrower) and the goal
//! (broader) position — sq-93rgp.
//!
//! Two halves:
//!
//! 1. **The per-operator matrix** — one case per operator per position that must be
//!    PROVEN (`Contains` / `Overlap::Certain`), and one per operator per position that
//!    must stay unproven (`Unknown` / `Overlap::Possible`): an `or` premise whose one
//!    member does not imply, a `xone` goal an `or`/`and` premise cannot establish, a
//!    partly-proven `and` goal.
//! 2. **The brute-force differential check** — random small constraint trees over a
//!    finite request domain. Whenever the analyser says `Contains` or `Certain`, EVERY
//!    request in the domain is evaluated through the real `decide` path and must agree.
//!    A counterexample is an unsoundness (over-claim), which is the failure mode that
//!    matters in a policy engine; a precision gap is not a failure.

use sparq_policy::{
    contains, decide, detect_conflicts, parse_policy_str, Action, Constraint, ConstraintNode,
    Containment, LogicalConstraint, LogicalOperator, Operator, Overlap, Policy, Request, Rule,
    ValidatedPolicy, Value,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ACTION: &str = "http://www.w3.org/ns/odrl/2/read";
const TARGET: &str = "urn:asset/x";
const PARTY: &str = "urn:alice";

// ---------------------------------------------------------------------------
// Builders: atomic constraints and compounds over a tiny two-dimension vocabulary.
// ---------------------------------------------------------------------------

/// The atomic constraint pool, indexed so a test (and the generator) can name one.
/// Two dimensions — `odrl:count` (numeric) and `odrl:purpose` (IRI) — are enough for
/// every inference rule, and small enough to enumerate the whole request domain.
fn atom(i: usize) -> Constraint {
    let count = |op, n: f64| Constraint {
        left: format!("{ODRL}count"),
        operator: op,
        right: Value::Num(n),
    };
    let purpose = |p: &str| Constraint {
        left: format!("{ODRL}purpose"),
        operator: Operator::Eq,
        right: Value::Iri(format!("urn:p/{p}")),
    };
    match i % 6 {
        0 => count(Operator::Eq, 1.0),
        1 => count(Operator::Eq, 2.0),
        2 => count(Operator::Lt, 2.0),
        3 => count(Operator::Gt, 2.0),
        4 => purpose("a"),
        _ => purpose("b"),
    }
}

const POOL: usize = 6;

fn leaf(i: usize) -> ConstraintNode {
    ConstraintNode::Atomic(atom(i))
}

fn compound(op: LogicalOperator, operands: Vec<ConstraintNode>) -> ConstraintNode {
    ConstraintNode::Compound(LogicalConstraint {
        id: format!("_:lc{}", operands.len()),
        operator: op,
        operands,
    })
}

fn and(operands: Vec<ConstraintNode>) -> ConstraintNode {
    compound(LogicalOperator::And, operands)
}
fn or(operands: Vec<ConstraintNode>) -> ConstraintNode {
    compound(LogicalOperator::Or, operands)
}
fn xone(operands: Vec<ConstraintNode>) -> ConstraintNode {
    compound(LogicalOperator::Xone, operands)
}

/// A `read`/`x`/`alice` rule carrying `nodes` as its constraints (atomic nodes land in
/// `constraints`, compound ones in `logical_constraints` — exactly how the parser
/// routes them, and the evaluator ANDs both).
fn rule(id: &str, nodes: Vec<ConstraintNode>) -> Rule {
    let mut r = Rule {
        id: id.to_owned(),
        action: Action(ACTION.to_owned()),
        target: Some(TARGET.to_owned()),
        assignee: Some(PARTY.to_owned()),
        assigner: None,
        constraints: Vec::new(),
        logical_constraints: Vec::new(),
        duties: Vec::new(),
    };
    for n in nodes {
        match n {
            ConstraintNode::Atomic(c) => r.constraints.push(c),
            ConstraintNode::Compound(lc) => r.logical_constraints.push(lc),
        }
    }
    r
}

fn perm_policy(nodes: Vec<ConstraintNode>) -> ValidatedPolicy {
    Policy {
        permissions: vec![rule("_:perm", nodes)],
        ..Policy::default()
    }
    .validate()
    .expect("policy validates")
}

/// `contains(broader ⊇ narrower)` for two single-permission policies differing only in
/// their constraints.
fn verdict(broader: Vec<ConstraintNode>, narrower: Vec<ConstraintNode>) -> Containment {
    contains(&perm_policy(broader), &perm_policy(narrower))
}

/// The conflict verdict for a permission constrained by `perm` and a prohibition — on
/// the same action/target/assignee — constrained by `proh`.
fn conflict(perm: Vec<ConstraintNode>, proh: Vec<ConstraintNode>) -> Overlap {
    let policy = Policy {
        permissions: vec![rule("_:perm", perm)],
        prohibitions: vec![rule("_:proh", proh)],
        ..Policy::default()
    };
    let conflicts = detect_conflicts(&policy);
    assert_eq!(conflicts.len(), 1, "{:?}", conflicts);
    conflicts[0].overlap
}

// ---------------------------------------------------------------------------
// PREMISE (narrower) position — what the narrower rule's compound PROVES.
// ---------------------------------------------------------------------------

/// `and` premise: a satisfied `and` has EVERY operand satisfied, so one operand that
/// is the broader bound proves it.
#[test]
fn and_premise_proves_a_member_bound() {
    assert_eq!(
        verdict(vec![leaf(0)], vec![and(vec![leaf(0), leaf(4)])]),
        Containment::Contains
    );
    // Nested: the `and` member is itself an `and`.
    assert_eq!(
        verdict(vec![leaf(0)], vec![and(vec![and(vec![leaf(0)]), leaf(4)])]),
        Containment::Contains
    );
    // An `and` of OTHER bounds proves nothing about this one (no magnitude reasoning).
    assert_eq!(
        verdict(vec![leaf(0)], vec![and(vec![leaf(1), leaf(4)])]),
        Containment::Unknown
    );
}

/// `or` premise: a satisfied `or` says at least one operand holds but not WHICH, so a
/// broader bound is proven only when EVERY member proves it.
#[test]
fn or_premise_proves_only_when_every_member_does() {
    // Every branch carries bound 0 → proven.
    assert_eq!(
        verdict(
            vec![leaf(0)],
            vec![or(vec![and(vec![leaf(0), leaf(4)]), and(vec![leaf(0), leaf(5)])])]
        ),
        Containment::Contains
    );
    // One branch does NOT carry it → must stay Unknown (the fail-OPEN case).
    assert_eq!(
        verdict(vec![leaf(0)], vec![or(vec![leaf(0), leaf(1)])]),
        Containment::Unknown
    );
}

/// `xone` premise: exactly one operand holds — still "at least one, we do not know
/// which" for implication purposes, so it behaves like `or` on this side.
#[test]
fn xone_premise_behaves_like_or() {
    assert_eq!(
        verdict(
            vec![leaf(0)],
            vec![xone(vec![and(vec![leaf(0), leaf(4)]), and(vec![leaf(0), leaf(5)])])]
        ),
        Containment::Contains
    );
    assert_eq!(
        verdict(vec![leaf(0)], vec![xone(vec![leaf(0), leaf(1)])]),
        Containment::Unknown
    );
}

// ---------------------------------------------------------------------------
// GOAL (broader) position — what the broader rule's compound DEMANDS.
// ---------------------------------------------------------------------------

/// `and` goal: every operand must be proven.
#[test]
fn and_goal_needs_every_operand() {
    assert_eq!(
        verdict(vec![and(vec![leaf(0), leaf(4)])], vec![leaf(0), leaf(4)]),
        Containment::Contains
    );
    // Only one of the two operands is available on the narrower side.
    assert_eq!(
        verdict(vec![and(vec![leaf(0), leaf(4)])], vec![leaf(0)]),
        Containment::Unknown
    );
}

/// `or` goal: one proven operand satisfies the disjunction.
#[test]
fn or_goal_needs_one_operand() {
    assert_eq!(
        verdict(vec![or(vec![leaf(0), leaf(1)])], vec![leaf(0)]),
        Containment::Contains
    );
    // A narrower side with an unrelated bound proves no member.
    assert_eq!(
        verdict(vec![or(vec![leaf(0), leaf(1)])], vec![leaf(4)]),
        Containment::Unknown
    );
    // A narrower `or` whose every branch lands inside the broader `or` proves it.
    assert_eq!(
        verdict(
            vec![or(vec![leaf(0), leaf(1), leaf(2)])],
            vec![or(vec![leaf(0), leaf(1)])]
        ),
        Containment::Contains
    );
    // A narrower `or` with a branch OUTSIDE the broader `or` does not.
    assert_eq!(
        verdict(
            vec![or(vec![leaf(0), leaf(1)])],
            vec![or(vec![leaf(0), leaf(4)])]
        ),
        Containment::Unknown
    );
    // An `or` nested inside the narrower side's `and` is visible as a premise.
    assert_eq!(
        verdict(
            vec![or(vec![leaf(0), leaf(1)])],
            vec![and(vec![or(vec![leaf(0), leaf(1)]), leaf(4)])]
        ),
        Containment::Contains
    );
}

/// `xone` goal: "exactly one operand satisfied AND none unprovable" is not established
/// by positive knowledge, so a multi-operand `xone` goal is never proven — notably NOT
/// by an `or` or an `and` over the same operands. A single-operand `xone` reduces to
/// that operand.
#[test]
fn multi_operand_xone_goal_is_never_proven() {
    // `or` does NOT imply `xone` (two satisfied operands make the `xone` false).
    assert_eq!(
        verdict(vec![xone(vec![leaf(0), leaf(1)])], vec![or(vec![leaf(0), leaf(1)])]),
        Containment::Unknown
    );
    // Nor does an `and` over the same operands — it makes the `xone` definitely FALSE.
    assert_eq!(
        verdict(vec![xone(vec![leaf(0), leaf(4)])], vec![and(vec![leaf(0), leaf(4)])]),
        Containment::Unknown
    );
    // Nor one proven operand on its own: a sibling could also hold, or be unprovable.
    assert_eq!(
        verdict(vec![xone(vec![leaf(0), leaf(4)])], vec![leaf(0)]),
        Containment::Unknown
    );
    // An identical `xone` on both sides IS proven — not by the goal rule, but because
    // the narrower side carries that very compound as a premise, and a compound forces
    // a syntactically identical copy of itself.
    assert_eq!(
        verdict(vec![xone(vec![leaf(0), leaf(1)])], vec![xone(vec![leaf(0), leaf(1)])]),
        Containment::Contains
    );
    // A single-operand `xone` is exactly its operand.
    assert_eq!(
        verdict(vec![xone(vec![leaf(0)])], vec![leaf(0)]),
        Containment::Contains
    );
}

/// A `xone` premise proves an `or` goal over the same operands (exactly one satisfied
/// implies at least one satisfied) — the one direction between the two that holds.
#[test]
fn xone_premise_proves_an_or_goal() {
    assert_eq!(
        verdict(vec![or(vec![leaf(0), leaf(1)])], vec![xone(vec![leaf(0), leaf(1)])]),
        Containment::Contains
    );
}

// ---------------------------------------------------------------------------
// The same rules drive the CONFLICT verdict (the prohibition is the goal side).
// ---------------------------------------------------------------------------

#[test]
fn compound_conflicts_are_certain_only_when_proven() {
    // Prohibition's atomic bound proven by the permission's `and`.
    assert_eq!(
        conflict(vec![and(vec![leaf(0), leaf(4)])], vec![leaf(0)]),
        Overlap::Certain
    );
    // Prohibition's `or` goal proven by the permission's atomic member.
    assert_eq!(
        conflict(vec![leaf(0)], vec![or(vec![leaf(0), leaf(1)])]),
        Overlap::Certain
    );
    // Permission's `or` has a branch that does not force the prohibition's bound.
    assert_eq!(
        conflict(vec![or(vec![leaf(0), leaf(1)])], vec![leaf(0)]),
        Overlap::Possible
    );
    // A multi-operand `xone` prohibition is never proven total.
    assert_eq!(
        conflict(vec![and(vec![leaf(0), leaf(4)])], vec![xone(vec![leaf(0), leaf(4)])]),
        Overlap::Possible
    );
    // An unconstrained permission proves no compound at all.
    assert_eq!(
        conflict(vec![], vec![or(vec![leaf(0), leaf(1)])]),
        Overlap::Possible
    );
}

/// Nesting deeper than the prover's bound degrades to unproven, and terminates.
#[test]
fn nesting_past_the_bound_degrades_conservatively() {
    let mut node = leaf(0);
    for _ in 0..40 {
        node = and(vec![node]);
    }
    // Sound either way; the point is that it answers (bounded cost) and never claims
    // more than it proved.
    assert_ne!(verdict(vec![leaf(1)], vec![node.clone()]), Containment::Contains);
    assert_eq!(verdict(vec![leaf(0)], vec![node]), Containment::Unknown);
}

/// `odrl:andSequence` is NOT a compound the analyser reasons about: `LogicalOperator`
/// parses only `and`/`or`/`xone`, so the parser never routes such a node into
/// `logical_constraints` — it degrades to the unsatisfiable guard, which
/// `Policy::validate` refuses outright on a prohibition. Nothing treats a sequence as
/// an `and`.
#[test]
fn and_sequence_is_not_read_as_a_compound() {
    let doc = |kind: &str| {
        format!(
            "@prefix odrl: <{ODRL}> .\n\
             <urn:pol/p> a odrl:Set ; odrl:{kind} [ odrl:action odrl:read ; \
               odrl:target <{TARGET}> ; odrl:assignee <{PARTY}> ; \
               odrl:constraint [ a odrl:LogicalConstraint ; odrl:andSequence ( \
                 [ odrl:leftOperand odrl:count ; odrl:operator odrl:eq ; \
                   odrl:rightOperand 1 ] ) ] ] ."
        )
    };
    // On a prohibition the guard would fail OPEN, so the whole policy is refused.
    assert!(parse_policy_str(&doc("prohibition"), "turtle").is_err());
    // On a permission it parses, but carries NO compound to reason about — so it can
    // never be the premise that proves anything.
    let p = parse_policy_str(&doc("permission"), "turtle").expect("permission parses");
    assert!(
        p.permissions[0].logical_constraints.is_empty(),
        "andSequence must not become a LogicalConstraint: {:?}",
        p.permissions[0]
    );
}

// ---------------------------------------------------------------------------
// BRUTE-FORCE DIFFERENTIAL CHECK
// ---------------------------------------------------------------------------

/// A small deterministic generator (no extra dev-dependency), matching the one in
/// `odrl_compare_witness.rs`.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> usize {
        (self.next() % n) as usize
    }
}

/// EVERY request over the finite domain the constraint pool ranges over: `odrl:count`
/// ∈ {absent, 1, 2, 3} × `odrl:purpose` ∈ {absent, a, b} — 12 requests, each on the
/// one action/target/party every generated rule names. Absent values matter: they are
/// the `Unprovable` cases the compound fold must fail closed on.
fn domain() -> Vec<Request> {
    let mut out = Vec::new();
    for count in [None, Some(1.0), Some(2.0), Some(3.0)] {
        for purpose in [None, Some("a"), Some("b")] {
            let mut r = Request::new(ACTION).on(TARGET).by(PARTY);
            if let Some(c) = count {
                r = r.with(format!("{ODRL}count"), Value::Num(c));
            }
            if let Some(p) = purpose {
                r = r.with(format!("{ODRL}purpose"), Value::Iri(format!("urn:p/{p}")));
            }
            out.push(r);
        }
    }
    out
}

/// A random constraint node: an atomic leaf, or a compound over 1–3 sub-nodes.
fn gen_node(g: &mut Lcg, depth: u32) -> ConstraintNode {
    if depth == 0 || g.below(3) == 0 {
        return leaf(g.below(POOL as u64));
    }
    let op = match g.below(3) {
        0 => LogicalOperator::And,
        1 => LogicalOperator::Or,
        _ => LogicalOperator::Xone,
    };
    let n = 1 + g.below(3);
    let operands = (0..n).map(|_| gen_node(g, depth - 1)).collect();
    compound(op, operands)
}

/// 1–2 constraint nodes for one rule.
fn gen_nodes(g: &mut Lcg) -> Vec<ConstraintNode> {
    (0..1 + g.below(2)).map(|_| gen_node(g, 2)).collect()
}

/// Whenever the analyser reports `Contains` or `Overlap::Certain` for randomly
/// generated compound constraint sets, brute-force runtime evaluation over the WHOLE
/// finite request domain must agree. Any counterexample is an over-claim — the
/// soundness bug this issue exists to avoid.
#[test]
fn definite_compound_verdicts_agree_with_every_request_in_the_domain() {
    let reqs = domain();
    let mut g = Lcg(0x93_7a9b_c0de);
    let (mut contains_n, mut certain_n) = (0u32, 0u32);
    for _ in 0..20_000 {
        let broader = gen_nodes(&mut g);
        let narrower = gen_nodes(&mut g);

        // Containment: no request the narrower policy grants may be refused by the
        // broader one.
        let outer = perm_policy(broader.clone());
        let inner = perm_policy(narrower.clone());
        if contains(&outer, &inner) == Containment::Contains {
            contains_n += 1;
            for r in &reqs {
                assert!(
                    !decide(&inner, r).allow || decide(&outer, r).allow,
                    "Contains over-claimed\nbroader {:?}\nnarrower {:?}\nreq {:?}",
                    broader,
                    narrower,
                    r
                );
            }
        }

        // Conflict: a Certain carve-out must leave no request the permission grants
        // alone surviving the prohibition.
        let perm = rule("_:perm", narrower.clone());
        let proh = rule("_:proh", broader.clone());
        let pair = Policy {
            permissions: vec![perm.clone()],
            prohibitions: vec![proh.clone()],
            ..Policy::default()
        };
        if detect_conflicts(&pair)
            .iter()
            .any(|c| c.overlap == Overlap::Certain)
        {
            certain_n += 1;
            let alone = Policy {
                permissions: vec![perm],
                ..Policy::default()
            }
            .validate()
            .expect("permission-only policy validates");
            let both = pair.validate().expect("pair validates");
            for r in &reqs {
                assert!(
                    !decide(&alone, r).allow || !decide(&both, r).allow,
                    "Certain over-claimed\nperm {:?}\nproh {:?}\nreq {:?}",
                    narrower,
                    broader,
                    r
                );
            }
        }
    }
    // The generator must actually reach the definite verdicts, or the check is vacuous
    // (this seed reaches a few thousand of each).
    assert!(contains_n >= 50, "only {} Contains verdicts", contains_n);
    assert!(certain_n >= 50, "only {} Certain verdicts", certain_n);
}
