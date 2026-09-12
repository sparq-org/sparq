//! [GPT-6] Canonical signed-i64 planning, separate from unsigned verification.
//!
//! This is prover-local planning, not a proof or credential authentication. The
//! private inner query uses order-preserving biased bounds, and cannot be passed
//! by callers to an unsigned compiler. RDF terms and committed graphs are never
//! rewritten. Only canonical xsd:integer spellings in the i64 range are admitted.

use super::{
    canonical_integer, optimize, plan_disclosure_numeric, unsupported, DisclosurePlan,
    DisclosureQuery, IntegerFilter, MembershipRef, OptimizationLimits, OptimizationReport,
    PlanError, PlannerLimits, QueryKind, QuerySlot, MAX_DISCLOSURE_AST_NODES,
    MAX_DISCLOSURE_FILTERS,
};
use crate::manifest::FilterOp;
use oxrdf::{Term, Triple};
use spargebra::algebra::{Expression, GraphPattern};
use sparq_zk::commit::GraphCommitment;
use std::collections::BTreeMap;

// The sign-bit flip maps MIN..MAX monotonically onto 0..u64::MAX. Bitwise
// conversion avoids signed addition or negation overflow, including i64::MIN.
const SIGN_BIT: u64 = 1_u64 << 63;

pub(crate) fn biased(value: i64) -> u64 {
    (value as u64) ^ SIGN_BIT
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum NumericProfile {
    Unsigned,
    Signed,
}

impl NumericProfile {
    pub(crate) fn value(self, term: &Term) -> Option<u64> {
        match self {
            Self::Unsigned => canonical_integer(term),
            Self::Signed => canonical_signed_integer(term).map(biased),
        }
    }
}

/// Parses an exact canonical xsd:integer term within the signed-i64 range.
///
/// Rejects leading plus, leading zeros, negative zero, whitespace, other numeric
/// datatypes, and out-of-range values. This does not normalize valid alternative
/// XML Schema spellings into a different committed RDF term.
pub fn canonical_signed_integer(term: &Term) -> Option<i64> {
    let Term::Literal(literal) = term else {
        return None;
    };
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#integer" {
        return None;
    }
    let lexical = literal.value();
    let value: i64 = lexical.parse().ok()?;
    (value.to_string() == lexical).then_some(value)
}

/// A normalized signed comparison with its actual public i64 bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIntegerFilter {
    /// Directly bound variable, without the leading question mark.
    pub variable: String,
    /// Comparison normalized with the variable on the left.
    pub op: FilterOp,
    /// Canonical signed public bound.
    pub bound: i64,
}

/// A separately admitted signed-integer query, never an unsigned query instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedDisclosureQuery {
    pub(crate) inner: DisclosureQuery,
}

impl SignedDisclosureQuery {
    /// Admits the positive disclosure fragment with canonical signed-i64 comparisons.
    ///
    /// # Errors
    /// Rejects unsupported syntax, noncanonical or out-of-range bounds, unsafe
    /// FILTER scope, and the same shape/resource limits as unsigned planning.
    pub fn parse(sparql: &str) -> Result<Self, PlanError> {
        Ok(Self {
            inner: DisclosureQuery::parse_numeric(sparql, NumericProfile::Signed)?,
        })
    }

    /// Returns the admitted answer-support kind.
    pub fn kind(&self) -> QueryKind {
        self.inner.kind
    }

    /// Returns the released variables in projection order.
    pub fn projection(&self) -> &[String] {
        &self.inner.projection
    }

    /// Returns the unchanged RDF pattern constants and variables.
    pub fn patterns(&self) -> &[[QuerySlot; 3]] {
        &self.inner.patterns
    }

    /// Returns comparisons with signed bounds, without exposing the internal bias.
    pub fn filters(&self) -> Vec<SignedIntegerFilter> {
        self.inner
            .filters
            .iter()
            .map(|filter| SignedIntegerFilter {
                variable: filter.variable.clone(),
                op: filter.op,
                bound: (filter.bound ^ SIGN_BIT) as i64,
            })
            .collect()
    }
}

/// Selects signed-integer witnesses without altering committed RDF terms.
///
/// # Errors
/// Rejects unsupported releases, absent witnesses, and exhausted resource bounds.
pub fn plan_signed_disclosure(
    query: &SignedDisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: PlannerLimits,
) -> Result<DisclosurePlan, PlanError> {
    plan_signed_disclosure_admitted(query, credentials, released, limits, |_, _, _| true)
}

/// Restricts eligible signed witnesses while preserving every matching obligation.
///
/// # Errors
/// Returns the same errors as [`plan_signed_disclosure`]; admission can remove
/// candidates only, and cannot waive signed comparisons or RDF identity checks.
pub fn plan_signed_disclosure_admitted<A>(
    query: &SignedDisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: PlannerLimits,
    admit: A,
) -> Result<DisclosurePlan, PlanError>
where
    A: Fn(usize, MembershipRef, &Triple) -> bool,
{
    plan_disclosure_numeric(
        &query.inner,
        credentials,
        released,
        limits,
        admit,
        NumericProfile::Signed,
    )
}

/// Optimizes signed witness selection under the existing structural objective.
///
/// # Errors
/// Rejects invalid releases and exceeded input/depth bounds. Candidate-budget
/// exhaustion is reported explicitly without an optimality claim.
pub fn optimize_signed_disclosure(
    query: &SignedDisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: OptimizationLimits,
) -> Result<OptimizationReport, PlanError> {
    optimize_signed_disclosure_admitted(query, credentials, released, limits, |_, _, _| true)
}

/// Optimizes signed witnesses with a backend's additional candidate restriction.
///
/// # Errors
/// Returns the same validation and resource errors as [`optimize_signed_disclosure`].
pub fn optimize_signed_disclosure_admitted<A>(
    query: &SignedDisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: OptimizationLimits,
    admit: A,
) -> Result<OptimizationReport, PlanError>
where
    A: Fn(usize, MembershipRef, &Triple) -> bool,
{
    optimize::optimize_disclosure_numeric(
        &query.inner,
        credentials,
        released,
        limits,
        admit,
        NumericProfile::Signed,
    )
}

pub(super) fn collect_filters(pattern: &GraphPattern) -> Result<Vec<IntegerFilter>, PlanError> {
    let mut patterns = vec![pattern];
    let mut filters = Vec::new();
    let mut nodes = 0;
    while let Some(pattern) = patterns.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("signed FILTER AST nodes"));
        }
        match pattern {
            GraphPattern::Bgp { .. } => {}
            GraphPattern::Join { left, right } => {
                patterns.push(right);
                patterns.push(left);
            }
            GraphPattern::Filter { expr, inner } => {
                collect_comparisons(expr, &mut filters)?;
                patterns.push(inner);
            }
            _ => return Err(unsupported("signed FILTER graph operator")),
        }
    }
    Ok(filters)
}

fn collect_comparisons(
    expr: &Expression,
    filters: &mut Vec<IntegerFilter>,
) -> Result<(), PlanError> {
    use Expression as E;
    let mut pending = vec![expr];
    let mut nodes = 0;
    while let Some(expr) = pending.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("signed comparison AST nodes"));
        }
        let (op, left, right) = match expr {
            E::And(left, right) => {
                pending.push(right);
                pending.push(left);
                continue;
            }
            E::Less(a, b) => (FilterOp::Lt, a, b),
            E::LessOrEqual(a, b) => (FilterOp::Le, a, b),
            E::Greater(a, b) => (FilterOp::Gt, a, b),
            E::GreaterOrEqual(a, b) => (FilterOp::Ge, a, b),
            E::Equal(a, b) => (FilterOp::Eq, a, b),
            E::Not(inner) => match inner.as_ref() {
                E::Equal(a, b) => (FilterOp::Ne, a, b),
                _ => return Err(unsupported("signed FILTER negation")),
            },
            _ => return Err(unsupported("signed FILTER expression")),
        };
        let (variable, constant, op) = match (left.as_ref(), right.as_ref()) {
            (E::Variable(variable), constant) => (variable, constant, op),
            (constant, E::Variable(variable)) => {
                let flipped = match op {
                    FilterOp::Lt => FilterOp::Gt,
                    FilterOp::Le => FilterOp::Ge,
                    FilterOp::Gt => FilterOp::Lt,
                    FilterOp::Ge => FilterOp::Le,
                    FilterOp::Eq => FilterOp::Eq,
                    FilterOp::Ne => FilterOp::Ne,
                };
                (variable, constant, flipped)
            }
            _ => {
                return Err(unsupported(
                    "signed FILTER requires variable and integer literal",
                ))
            }
        };
        let bound = signed_constant(constant)
            .ok_or_else(|| unsupported("canonical signed-i64 FILTER bound"))?;
        if filters.len() >= MAX_DISCLOSURE_FILTERS {
            return Err(PlanError::LimitExceeded("signed FILTER comparisons"));
        }
        filters.push(IntegerFilter {
            variable: variable.as_str().to_owned(),
            op,
            bound: biased(bound),
        });
    }
    Ok(())
}

// SPARQL parses a bare negative numeric token as UnaryMinus(Literal(magnitude)).
// Admit only that exact constant shape, not arithmetic on variables/expressions.
// Building the negative lexical token first also handles MIN's magnitude 2^63
// without attempting to store or negate it in a signed i64.
fn signed_constant(expr: &Expression) -> Option<i64> {
    match expr {
        Expression::Literal(literal) => canonical_signed_integer(&literal.clone().into()),
        Expression::UnaryMinus(inner) => {
            let Expression::Literal(literal) = inner.as_ref() else {
                return None;
            };
            canonical_integer(&literal.clone().into())?;
            let token = oxrdf::Literal::new_typed_literal(
                format!("-{}", literal.value()),
                literal.datatype().into_owned(),
            );
            canonical_signed_integer(&token.into())
        }
        _ => None,
    }
}
