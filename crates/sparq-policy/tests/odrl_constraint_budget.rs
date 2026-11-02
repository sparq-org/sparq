//! The constraint-assembly budget of the ODRL parse: nesting depth and total operand
//! expansion.
//!
//! `assemble_logical` walks a rule's `odrl:LogicalConstraint` combinator graph
//! recursively, tracking only the ANCESTOR chain to break cycles. Two shapes therefore
//! cost far more than the document that describes them:
//!
//! * a node reachable by several paths through an ACYCLIC graph is assembled once per
//!   path, so a document whose every level has two operands pointing at the same pair of
//!   next-level nodes describes 2^n operand nodes in O(n) triples;
//! * a chain of n nested compounds costs n stack frames.
//!
//! Both are now bounded, and exceeding either budget refuses the WHOLE policy (never
//! degrades one operand, which would widen one rule kind or the other). Normal nested
//! policies parse exactly as before.

use sparq_policy::{
    constraint_budget_refusal, parse_policy_str, ConstraintBudget, ConstraintNode, LogicalOperator,
    Operator, Value,
};

const PREFIX: &str = "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
                      @prefix ex: <https://ex.dev/> .\n";

/// A policy whose one permission carries the compound `ex:root`, with `body` supplying the
/// constraint nodes.
fn policy(body: &str) -> String {
    format!(
        "{PREFIX}\
         ex:p a odrl:Set ; odrl:permission ex:r .\n\
         ex:r odrl:action odrl:read ; odrl:target ex:asset ; odrl:assignee ex:alice ;\n\
              odrl:constraint ex:root .\n\
         {body}"
    )
}

/// One atomic constraint node that is always satisfiable.
fn atom(name: &str) -> String {
    format!(
        "{name} odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;\n\
         \x20   odrl:rightOperand \"2000-01-01T00:00:00Z\" .\n"
    )
}

/// A DIAMOND combinator graph `levels` deep whose assembled tree has 2^levels operand
/// nodes: each level's two nodes both carry the SAME two next-level operands, so every
/// path through the graph is assembled separately even though the graph itself is acyclic
/// and linear in size.
///
/// `ex:root` is the entry node; the deepest level's two nodes are atomic constraints.
fn diamond(levels: usize) -> String {
    let name = |lvl: usize, side: char| {
        if lvl == 0 && side == 'a' {
            "ex:root".to_owned()
        } else {
            format!("ex:n{}{}", lvl, side)
        }
    };
    let mut out = String::new();
    for lvl in 0..levels {
        for side in ['a', 'b'] {
            out.push_str(&format!(
                "{} a odrl:LogicalConstraint ; odrl:and {}, {} .\n",
                name(lvl, side),
                name(lvl + 1, 'a'),
                name(lvl + 1, 'b'),
            ));
        }
    }
    for side in ['a', 'b'] {
        out.push_str(&atom(&name(levels, side)));
    }
    out
}

/// A CHAIN of `levels` nested compounds, each the single operand of the one above.
fn chain(levels: usize) -> String {
    let name = |lvl: usize| {
        if lvl == 0 {
            "ex:root".to_owned()
        } else {
            format!("ex:c{}", lvl)
        }
    };
    let mut out = String::new();
    for lvl in 0..levels {
        out.push_str(&format!(
            "{} a odrl:LogicalConstraint ; odrl:and {} .\n",
            name(lvl),
            name(lvl + 1),
        ));
    }
    out.push_str(&atom(&name(levels)));
    out
}

/// A shared-subgraph document that doubles per level is REFUSED, by the expansion budget,
/// and returns at all: 30 levels is 2^30 operand nodes, which no machine assembles.
///
/// The refusal is the whole policy's (`Err`), so no rule of it is read with an operand
/// quietly dropped or degraded.
#[test]
fn a_doubling_shared_subgraph_is_refused_by_the_expansion_budget() {
    let err = parse_policy_str(&policy(&diamond(30)), "turtle")
        .expect_err("a 2^30-node expansion must be refused, not assembled");
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Expansion),
        "{}",
        err
    );
}

/// The budget refuses only the shapes that are past it: a diamond small enough to fit is
/// still assembled, and its tree is the full 2^levels nodes it describes.
#[test]
fn a_shared_subgraph_inside_the_budget_still_assembles_every_path() {
    let levels = 5;
    let p = parse_policy_str(&policy(&diamond(levels)), "turtle")
        .expect("a 32-node expansion is inside the budget");
    let lc = &p.permissions[0].logical_constraints[0];
    fn leaves(n: &ConstraintNode) -> usize {
        match n {
            ConstraintNode::Atomic(_) | ConstraintNode::SharedAtomic(_) => 1,
            ConstraintNode::Compound(lc) => lc.operands.iter().map(leaves).sum(),
        }
    }
    let total: usize = lc.operands.iter().map(leaves).sum();
    assert_eq!(total, 1 << levels, "every path is assembled separately");
}

/// A deep chain is refused by the DEPTH budget — on a thread with a 1 MiB stack, which an
/// unbounded 20,000-frame recursion exhausts.
#[test]
fn a_deep_chain_is_refused_without_exhausting_the_stack() {
    let doc = policy(&chain(20_000));
    let err = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || parse_policy_str(&doc, "turtle").err())
        .expect("spawn")
        .join()
        .expect("the parse must return an error, not overflow the stack")
        .expect("a 20,000-level chain must be refused");
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Depth),
        "{}",
        err
    );
}

/// A chain inside the depth budget still parses, nested to exactly its authored depth.
#[test]
fn a_chain_inside_the_depth_budget_keeps_its_authored_nesting() {
    let levels = 8;
    let p = parse_policy_str(&policy(&chain(levels)), "turtle")
        .expect("an 8-level chain is inside the budget");
    let mut node = &p.permissions[0].logical_constraints[0].operands[0];
    let mut depth = 1;
    while let ConstraintNode::Compound(lc) = node {
        node = &lc.operands[0];
        depth += 1;
    }
    assert!(matches!(
        node,
        ConstraintNode::Atomic(_) | ConstraintNode::SharedAtomic(_)
    ));
    assert_eq!(depth, levels, "the authored nesting is preserved exactly");
}

/// A NORMAL nested policy — the ODRL test suite's 062 shape, an `odrl:or` of two
/// `odrl:and`s — parses to exactly the tree it authored, unchanged by the budget.
#[test]
fn a_normal_nested_policy_parses_identically() {
    let doc = format!(
        "{PREFIX}\
         ex:p a odrl:Set ; odrl:permission ex:r .\n\
         ex:r odrl:action odrl:read ; odrl:target ex:asset ; odrl:assignee ex:alice ;\n\
              odrl:constraint ex:root .\n\
         ex:root a odrl:LogicalConstraint ; odrl:or ex:l, ex:r2 .\n\
         ex:l a odrl:LogicalConstraint ; odrl:and ex:c1, ex:c2 .\n\
         ex:r2 a odrl:LogicalConstraint ; odrl:and ex:c3 .\n\
         ex:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand ex:research .\n\
         ex:c2 odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand ex:bob .\n\
         ex:c3 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand ex:teaching .\n"
    );
    let p = parse_policy_str(&doc, "turtle").expect("the 062 shape parses");
    let root = &p.permissions[0].logical_constraints[0];
    assert_eq!(root.operator, LogicalOperator::Or);
    assert_eq!(root.operands.len(), 2);
    let ConstraintNode::Compound(left) = &root.operands[0] else {
        panic!(
            "first operand must be the nested odrl:and, got {:?}",
            root.operands[0]
        );
    };
    assert_eq!(left.operator, LogicalOperator::And);
    assert_eq!(left.operands.len(), 2);
    let c1 = match &left.operands[0] {
        ConstraintNode::Atomic(c) => c,
        ConstraintNode::SharedAtomic(c) => c,
        other => panic!("expected an atomic constraint, got {other:?}"),
    };
    assert_eq!(c1.operator, Operator::Eq);
    assert_eq!(c1.right, Value::Iri("https://ex.dev/research".to_owned()));
    let ConstraintNode::Compound(right) = &root.operands[1] else {
        panic!(
            "second operand must be the nested odrl:and, got {:?}",
            root.operands[1]
        );
    };
    assert_eq!(right.operator, LogicalOperator::And);
    assert_eq!(right.operands.len(), 1);
}

/// `constraint_budget_refusal` reads the kind tag out of a refusal wherever it appears,
/// including from the middle of a sentence a caller wrapped it in, and returns `None` for
/// any other error.
#[test]
fn the_refusal_kind_is_readable_from_a_wrapped_message() {
    let depth = parse_policy_str(&policy(&chain(20_000)), "turtle").unwrap_err();
    let expansion = parse_policy_str(&policy(&diamond(30)), "turtle").unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&depth),
        Some(ConstraintBudget::Depth)
    );
    assert_eq!(
        constraint_budget_refusal(&expansion),
        Some(ConstraintBudget::Expansion)
    );
    let wrapped = format!("could not load the policy document ({}); refused", depth);
    assert_eq!(
        constraint_budget_refusal(&wrapped),
        Some(ConstraintBudget::Depth),
        "a wrapped refusal must still be recognisable"
    );
    assert_eq!(
        constraint_budget_refusal("not a budget refusal at all"),
        None
    );
    assert_eq!(
        constraint_budget_refusal("policy declares multiple conflicting strategies"),
        None
    );
}
