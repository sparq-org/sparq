//! The admission boundary every decision entry point takes: a [`ValidatedPolicy`].
//!
//! A [`Policy`] is the raw model, built by the parser or by hand. A [`ValidatedPolicy`]
//! can only come from [`Policy::validate`], which refuses the shapes that would let a
//! prohibition silently stop applying or let a compound constraint decide without
//! evidence. [`crate::parse_policy`] validates before it returns, and
//! [`crate::decide`], [`crate::matched_prohibition`] and [`crate::prohibition_status`]
//! accept only a validated policy, so a typed construction or a ledger replay cannot
//! reach the evaluator without passing the same checks as a parsed document.

use std::ops::Deref;

use crate::model::{ConstraintNode, LogicalConstraint, Policy, Rule};
use crate::parse::{is_unsatisfiable_guard, node_has_guard};

/// A [`Policy`] that passed [`Policy::validate`]. Read it through `Deref`; there is no
/// way to build or mutate one except by validating a [`Policy`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidatedPolicy(Policy);

impl ValidatedPolicy {
    /// The validated policy's model.
    pub fn policy(&self) -> &Policy {
        &self.0
    }

    /// Give the model back, for editing; it must be validated again before use.
    pub fn into_inner(self) -> Policy {
        self.0
    }
}

impl Deref for ValidatedPolicy {
    type Target = Policy;
    fn deref(&self) -> &Policy {
        &self.0
    }
}

impl Policy {
    /// Admit this policy for evaluation, or refuse it (fail-closed).
    ///
    /// # Errors
    ///
    /// Refuses a policy with:
    /// - an empty `and`/`or`/`xone` anywhere (it asserts nothing decidable);
    /// - a prohibition carrying the unsatisfiable guard the parser substitutes for an
    ///   unrepresentable constraint (on a prohibition the guard would fail open);
    /// - a prohibition whose action, target or assignee is a blank node (a refined
    ///   action, an anonymous collection), which could never match a request.
    pub fn validate(self) -> Result<ValidatedPolicy, String> {
        for r in self.permissions.iter().chain(&self.prohibitions) {
            if r.logical_constraints.iter().any(has_empty_compound) {
                return Err(format!(
                    "rule {} has an empty odrl:and/odrl:or/odrl:xone, which asserts nothing \
                     decidable; the policy is refused (fail-closed)",
                    r.id
                ));
            }
        }
        for r in &self.prohibitions {
            refuse_degraded_prohibition(r)?;
            if opaque_head(r) {
                return Err(format!(
                    "prohibition {} has a blank-node action, target or assignee, which can \
                     never match a request; the prohibition would never fire and a sibling \
                     permission would grant, so the policy is refused (fail-closed)",
                    r.id
                ));
            }
        }
        Ok(ValidatedPolicy(self))
    }
}

/// A rule head the request can never be matched against: a blank node.
pub(crate) fn opaque_head(r: &Rule) -> bool {
    let opaque = |s: &str| s.starts_with("_:");
    opaque(&r.action.0)
        || r.target.as_deref().is_some_and(opaque)
        || r.assignee.as_deref().is_some_and(opaque)
}

fn has_empty_compound(lc: &LogicalConstraint) -> bool {
    lc.operands.is_empty()
        || lc.operands.iter().any(|n| match n {
            ConstraintNode::Atomic(_) => false,
            ConstraintNode::Compound(inner) => has_empty_compound(inner),
        })
}

/// The unsatisfiable guard fails closed only on a permission. On a prohibition it fails
/// OPEN: the prohibition can never fire, so a sibling permission grants what the author
/// forbade. A prohibition carrying a degraded constraint (atomic, or any operand of a
/// compound) therefore refuses the whole policy, like a malformed collection operand.
fn refuse_degraded_prohibition(r: &Rule) -> Result<(), String> {
    let degraded = r.constraints.iter().any(is_unsatisfiable_guard)
        || r.logical_constraints
            .iter()
            .any(|lc| lc.operands.iter().any(node_has_guard));
    if degraded {
        return Err(format!(
            "prohibition {} has a constraint that cannot be represented (a missing or \
             unknown operand or operator, a multi-valued rightOperand under a non-set \
             operator, an unencodable or typed set member, an odrl:unit, or a malformed \
             compound operand); dropping it would disable the prohibition and let a sibling \
             permission grant, so the policy is refused (fail-closed)",
            r.id
        ));
    }
    Ok(())
}
