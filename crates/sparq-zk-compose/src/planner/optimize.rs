//! [GPT-6] Bounded joint selection over successful private row witnesses.

use super::{
    assemble_plan, extend_bindings, released_bindings, validate_released, DisclosurePlan,
    DisclosureQuery, MembershipRef, PlanError, PlannerLimits, QueryKind, ScopedTerm,
    MAX_DISCLOSURE_CREDENTIALS,
};
use oxrdf::{Term, Triple};
use sparq_zk::commit::GraphCommitment;
use std::collections::BTreeMap;

// Bound recursive search and live partial bindings independently of caller caps.
// This limits joint selection only; the first-success planner remains separate.
const MAX_JOINT_DEPTH: usize = 128;

/// Structural proof work, ordered by authentication count and then membership count.
///
/// These counts are not calibrated latency estimates. Credential identity and
/// canonical leaf identity remain distinct even when plaintext values coincide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlanObjective {
    /// Unique credential authentication obligations, the primary objective.
    pub authentications: usize,
    /// Unique credential/leaf memberships, the secondary objective.
    pub memberships: usize,
}

/// Completion of local joint witness search, without a cryptographic assurance claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationCompletion {
    /// Search established the minimum structural objective in its admitted space.
    Optimal,
    /// Exhaustive search found no jointly feasible witness assignment.
    Infeasible,
    /// The candidate budget ended before optimality or infeasibility was established.
    BudgetExhausted,
}

/// Host limits for joint search; unrelated to a backend's circuit capacities.
#[derive(Debug, Clone, Copy)]
pub struct OptimizationLimits {
    /// Pattern, result, and global candidate-attempt limits.
    pub planner: PlannerLimits,
    /// Maximum number of result-by-pattern positions retained by joint search.
    ///
    /// The implementation also enforces an absolute depth limit of 128 positions
    /// to bound recursive stack and live partial-binding storage.
    pub max_pattern_occurrences: usize,
    /// Maximum distinct credential authentications permitted by the chosen backend.
    ///
    /// This is a joint feasibility restriction, not an estimate of proving cost.
    pub max_authentications: usize,
}

impl Default for OptimizationLimits {
    fn default() -> Self {
        Self {
            planner: PlannerLimits::default(),
            max_pattern_occurrences: MAX_JOINT_DEPTH,
            max_authentications: MAX_JOINT_DEPTH,
        }
    }
}

/// Deterministic local search statistics, potentially revealing private witness structure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptimizationStats {
    /// Number of credentials in the fixed input slice.
    pub input_credentials: usize,
    /// Total canonical triples across the fixed credential slice.
    pub input_triples: usize,
    /// Number of BGP patterns in the fixed query.
    pub input_patterns: usize,
    /// Number of released mappings in their fixed caller order.
    pub released_rows: usize,
    /// Product of released rows and query patterns.
    pub pattern_occurrences: usize,
    /// Distinct-credential capacity restricting this search's feasible space.
    pub authentication_limit: usize,
    /// Candidate triple attempts, including admission and pattern rejections.
    pub candidate_steps: usize,
    /// Complete feasible assignments recorded as successively better incumbents.
    /// Equal or worse complete assignments may be counted as pruned prefixes.
    pub feasible_assignments: usize,
    /// Prefixes pruned by credential capacity or the incumbent structural objective.
    pub pruned_prefixes: usize,
}

/// A prover-local optimization result, never a public proof or verification verdict.
///
/// When the budget is exhausted, `plan` may contain the best feasible assignment
/// found so far, or be absent if none was reached. Absence under exhaustion does
/// not establish infeasibility. No hidden attribution or search statistics should
/// be serialized into a public presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationReport {
    /// Best complete feasible plan encountered; never a partial row assignment.
    pub plan: Option<DisclosurePlan>,
    /// Whether the structural optimum, infeasibility, or neither was established.
    pub completion: OptimizationCompletion,
    /// Local counters and fixed input sizes.
    pub stats: OptimizationStats,
}

impl OptimizationReport {
    /// Returns the structural cost of the best complete feasible plan, if any.
    pub fn objective(&self) -> Option<PlanObjective> {
        self.plan.as_ref().map(|plan| PlanObjective {
            authentications: plan.authentication.len(),
            memberships: plan.memberships.len(),
        })
    }
}

/// Jointly minimizes structural proof work for fixed query and released rows.
///
/// Search is explicit opt-in: [`super::plan_disclosure`] keeps first-success
/// behavior. This function does not authenticate credentials, run proofs, or
/// claim a runtime improvement. It streams candidates without materializing an
/// exponential per-row solution list. Ties retain the first witness assignment
/// in released-row, pattern, original-credential, and canonical-leaf order.
///
/// # Errors
/// Returns an error for invalid queries/releases or exceeded input/depth bounds.
/// Exhausted candidate budgets are reported in [`OptimizationReport`] instead.
pub fn optimize_disclosure(
    query: &DisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: OptimizationLimits,
) -> Result<OptimizationReport, PlanError> {
    optimize_disclosure_admitted(query, credentials, released, limits, |_, _, _| true)
}

/// Optimizes witnesses while preserving a backend's candidate admission restriction.
///
/// `admit(pattern_index, membership, triple)` must be deterministic and only
/// restrict the candidate set. It sees original credential and leaf indices and
/// the original canonical triple; graph contents and commitments are untouched.
/// All ordinary RDF matching, scoped identity, and FILTER checks still apply.
/// A true callback result is not authentication or a proof-verification result.
///
/// # Errors
/// Returns an error for invalid queries/releases or exceeded input/depth bounds.
/// Exhausted candidate budgets are reported without claiming optimality.
pub fn optimize_disclosure_admitted<A>(
    query: &DisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: OptimizationLimits,
    admit: A,
) -> Result<OptimizationReport, PlanError>
where
    A: Fn(usize, MembershipRef, &Triple) -> bool,
{
    // Cheap bounds precede validation of caller-constructed public query shapes.
    if credentials.len() > MAX_DISCLOSURE_CREDENTIALS {
        return Err(PlanError::LimitExceeded("input credentials"));
    }
    if query.patterns.len() > limits.planner.max_patterns {
        return Err(PlanError::LimitExceeded("patterns"));
    }
    if released.len() > limits.planner.max_results {
        return Err(PlanError::LimitExceeded("released rows"));
    }
    let positions = query
        .patterns
        .len()
        .checked_mul(released.len())
        .filter(|count| *count <= limits.max_pattern_occurrences && *count <= MAX_JOINT_DEPTH)
        .ok_or(PlanError::LimitExceeded("joint pattern occurrences"))?;
    query.validate()?;
    if query.kind == QueryKind::Ask && (released.len() != 1 || !released[0].is_empty()) {
        return Err(PlanError::InvalidRelease {
            row: 0,
            reason: "true ASK requires exactly one empty binding; false ASK is unsupported".into(),
        });
    }
    validate_released(query, released)?;
    let input_triples = credentials.iter().try_fold(0_usize, |total, graph| {
        total
            .checked_add(graph.canonical.triples.len())
            .ok_or(PlanError::LimitExceeded("input triple count"))
    })?;
    let mut search = JointSearch {
        query,
        credentials,
        released,
        admit: &admit,
        budget: limits.planner.max_search_steps,
        authentication_limit: limits.max_authentications,
        exhausted: false,
        selected: Vec::with_capacity(positions),
        authentication: BTreeMap::new(),
        membership: BTreeMap::new(),
        best: None,
        stats: OptimizationStats {
            input_credentials: credentials.len(),
            input_triples,
            input_patterns: query.patterns.len(),
            released_rows: released.len(),
            pattern_occurrences: positions,
            authentication_limit: limits.max_authentications,
            ..Default::default()
        },
    };
    if released.is_empty() {
        search.best = Some((
            PlanObjective {
                authentications: 0,
                memberships: 0,
            },
            Vec::new(),
        ));
        search.stats.feasible_assignments = 1;
    } else {
        search.visit(0, &released_bindings(&released[0]));
    }
    let completion = if search.exhausted {
        OptimizationCompletion::BudgetExhausted
    } else if search.best.is_some() {
        OptimizationCompletion::Optimal
    } else {
        OptimizationCompletion::Infeasible
    };
    let plan = search
        .best
        .map(|(_, witnesses)| {
            let rows = witnesses
                .chunks(query.patterns.len())
                .map(<[MembershipRef]>::to_vec)
                .collect();
            assemble_plan(query, credentials.len(), released, rows)
        })
        .transpose()?;
    Ok(OptimizationReport {
        plan,
        completion,
        stats: search.stats,
    })
}

struct JointSearch<'a, A> {
    query: &'a DisclosureQuery,
    credentials: &'a [GraphCommitment],
    released: &'a [BTreeMap<String, Term>],
    admit: &'a A,
    budget: usize,
    authentication_limit: usize,
    exhausted: bool,
    selected: Vec<MembershipRef>,
    authentication: BTreeMap<usize, usize>,
    membership: BTreeMap<MembershipRef, usize>,
    best: Option<(PlanObjective, Vec<MembershipRef>)>,
    stats: OptimizationStats,
}

impl<A: Fn(usize, MembershipRef, &Triple) -> bool> JointSearch<'_, A> {
    fn objective(&self) -> PlanObjective {
        PlanObjective {
            authentications: self.authentication.len(),
            memberships: self.membership.len(),
        }
    }

    fn visit(&mut self, position: usize, bindings: &BTreeMap<String, ScopedTerm>) {
        let pattern_count = self.query.patterns.len();
        let pattern = position % pattern_count;
        let row = position / pattern_count;
        for credential in 0..self.credentials.len() {
            for leaf in 0..self.credentials[credential].canonical.triples.len() {
                if self.stats.candidate_steps >= self.budget {
                    self.exhausted = true;
                    return;
                }
                self.stats.candidate_steps += 1;
                let witness = MembershipRef { credential, leaf };
                let triple = &self.credentials[credential].canonical.triples[leaf];
                if !(self.admit)(pattern, witness, triple) {
                    continue;
                }
                let Some(next) = extend_bindings(self.query, pattern, credential, triple, bindings)
                else {
                    continue;
                };
                self.selected.push(witness);
                *self.authentication.entry(credential).or_default() += 1;
                *self.membership.entry(witness).or_default() += 1;
                let cost = self.objective();
                // Authentication/member sets can only grow along a prefix.
                // Equal-cost ties are safe to prune because DFS visits complete
                // assignments in their deterministic lexicographic tie order.
                if cost.authentications > self.authentication_limit
                    || self.best.as_ref().is_some_and(|(best, _)| cost >= *best)
                {
                    self.stats.pruned_prefixes += 1;
                } else if position + 1 == self.stats.pattern_occurrences {
                    self.stats.feasible_assignments += 1;
                    self.best = Some((cost, self.selected.clone()));
                } else if pattern + 1 == pattern_count {
                    self.visit(position + 1, &released_bindings(&self.released[row + 1]));
                } else {
                    self.visit(position + 1, &next);
                }
                decrement(&mut self.authentication, credential);
                decrement(&mut self.membership, witness);
                self.selected.pop();
                if self.exhausted {
                    return;
                }
            }
        }
    }
}

fn decrement<K: Ord>(counts: &mut BTreeMap<K, usize>, key: K) {
    if let Some(count) = counts.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            counts.remove(&key);
        }
    }
}
