//! Plans disclosures and selects private witnesses for positive query answers.
//!
//! [GPT-6] This module is a host planner, not a proof verifier. It selects one
//! successful witness for every released DISTINCT row (or a true ASK), without
//! claiming that the released set includes every possible answer. Authentication,
//! membership, hidden equality, and hidden FILTER obligations remain for a proof
//! backend to enforce. A public FILTER removes predicate computation only; it
//! never removes the credential or triple authentication obligation.
//!
//! `DisclosurePlan` contains PRIVATE credential/leaf attribution. It deliberately
//! has no serialization implementation and must not be copied into a public
//! manifest. Its counts and explanation can also reveal witness structure and
//! belong in local diagnostics unless the disclosure policy permits them.
//!
//! The admitted fragment is SELECT DISTINCT / true ASK with a nonempty positive
//! BGP, joins, and directly bound canonical nonnegative xsd:integer comparisons.
//! Projected blank nodes and query blank-node syntax are rejected; hidden blank
//! nodes retain per-credential scope during joins. This module is research-grade
//! and has not been externally audited.

use crate::manifest::FilterOp;
use oxrdf::{Term, Triple};
use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern};
use sparq_zk::commit::GraphCommitment;
use sparq_zk::verify::{fragment_filters, FilterCmp};
use std::collections::{BTreeMap, BTreeSet};

mod optimize;
#[doc(inline)]
pub use optimize::{
    optimize_disclosure, optimize_disclosure_admitted, OptimizationCompletion, OptimizationLimits,
    OptimizationReport, OptimizationStats, PlanObjective,
};

/// Maximum query text size before invoking the shared SPARQL parser.
pub const MAX_DISCLOSURE_QUERY_BYTES: usize = 8192;
/// Conservative raw ASCII punctuation budget before parsing, including literals and IRIs.
///
/// This is resource admission, not tokenization. Counting punctuation everywhere
/// bounds flat operator chains before parser-side recursive AST work. A query
/// with punctuation-heavy public literals may be rejected despite simple algebra.
pub const MAX_DISCLOSURE_QUERY_PUNCTUATION: usize = 256;
/// Maximum admitted BGP patterns, including programmatically constructed query shapes.
pub const MAX_DISCLOSURE_PATTERNS: usize = 64;
/// Maximum normalized FILTER comparisons evaluated per candidate binding.
pub const MAX_DISCLOSURE_FILTERS: usize = 32;
/// Maximum supplied credentials, including empty graphs and ineligible candidates.
///
/// Checked using slice length before validation, statistics, or search. This
/// bounds empty-graph traversal, which does not consume candidate-triple fuel.
pub const MAX_DISCLOSURE_CREDENTIALS: usize = 256;
// Bound local AST walks after the preparse fuel check; not a circuit capacity.
const MAX_DISCLOSURE_AST_NODES: usize = 512;

/// The result contract admitted by the planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// Each released mapping has a witness; duplicate mappings are rejected.
    SelectDistinct,
    /// A single private solution supports the released answer `true`.
    Ask,
}

/// An RDF constant or an identity-preserving query variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuerySlot {
    /// A public RDF term written in the query.
    Constant(Term),
    /// A variable, without a leading question mark.
    Variable(String),
}

/// A normalized, directly bound integer comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegerFilter {
    /// Variable on the left side of the normalized comparison.
    pub variable: String,
    /// Comparison, with reversed operands normalized by flipping the operator.
    pub op: FilterOp,
    /// Public canonical nonnegative xsd:integer bound.
    pub bound: u64,
}

/// The supported query shape, independently derivable from public query text.
///
/// Public fields support backend compilation. Verifiers must call [`Self::parse`]
/// on their query text instead of trusting a prover-supplied instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureQuery {
    /// DISTINCT answer-support or true-ASK contract.
    pub kind: QueryKind,
    /// Released variables, in SPARQL projection order.
    pub projection: Vec<String>,
    /// Subject, predicate, object slots, in AST traversal order.
    pub patterns: Vec<[QuerySlot; 3]>,
    /// Conjunctive integer comparisons in AST traversal order.
    pub filters: Vec<IntegerFilter>,
}

impl DisclosureQuery {
    /// Parses and admits a query through the existing SPARQL parser.
    ///
    /// # Errors
    /// Rejects malformed queries, bag SELECT, modifiers, nonpositive operators,
    /// expression projections, blank-node syntax, unsupported FILTERs, and resource
    /// limits. Raw punctuation in public literals and IRIs counts toward the cap.
    pub fn parse(sparql: &str) -> Result<Self, PlanError> {
        if sparql.len() > MAX_DISCLOSURE_QUERY_BYTES {
            return Err(PlanError::LimitExceeded("query text bytes"));
        }
        if sparql
            .bytes()
            .filter(u8::is_ascii_punctuation)
            .take(MAX_DISCLOSURE_QUERY_PUNCTUATION + 1)
            .count()
            > MAX_DISCLOSURE_QUERY_PUNCTUATION
        {
            return Err(PlanError::LimitExceeded("query punctuation fuel"));
        }
        // The vendored parser additionally guards actual syntactic nesting.
        let parsed = spargebra::SparqlParser::new()
            .parse_query(sparql)
            .map_err(|e| PlanError::Parse(e.to_string()))?;
        if parsed.dataset().is_some() {
            return Err(unsupported("FROM / FROM NAMED"));
        }
        let (kind, projection, inner) = match &parsed {
            spargebra::Query::Select { pattern, .. } => {
                let GraphPattern::Distinct { inner } = pattern else {
                    return Err(unsupported(
                        "SELECT must use DISTINCT, without LIMIT/OFFSET",
                    ));
                };
                let GraphPattern::Project { inner, variables } = inner.as_ref() else {
                    return Err(unsupported("SELECT projection"));
                };
                (
                    QueryKind::SelectDistinct,
                    variables.iter().map(|v| v.as_str().to_owned()).collect(),
                    inner.as_ref(),
                )
            }
            spargebra::Query::Ask { pattern, .. } => {
                // The shared parser may produce the implicit star projection
                // around ASK. Its variables are not released by ASK.
                let inner = match pattern {
                    GraphPattern::Project { inner, .. } => inner.as_ref(),
                    other => other,
                };
                (QueryKind::Ask, Vec::new(), inner)
            }
            _ => return Err(unsupported("only SELECT DISTINCT and true ASK")),
        };
        let mut patterns = Vec::new();
        collect_patterns(inner, &mut patterns)?;
        let filters = fragment_filters(sparql)
            .map_err(|e| unsupported(&e.to_string()))?
            .into_iter()
            .map(|f| IntegerFilter {
                variable: f.variable,
                op: match f.op {
                    FilterCmp::Lt => FilterOp::Lt,
                    FilterCmp::Le => FilterOp::Le,
                    FilterCmp::Gt => FilterOp::Gt,
                    FilterCmp::Ge => FilterOp::Ge,
                    FilterCmp::Eq => FilterOp::Eq,
                    FilterCmp::Ne => FilterOp::Ne,
                },
                bound: f.bound,
            })
            .collect();
        let query = Self {
            kind,
            projection,
            patterns,
            filters,
        };
        query.validate()?;
        Ok(query)
    }

    /// Returns stable variable IDs in first-pattern-occurrence order.
    ///
    /// Backends may assign IDs starting at one; zero can denote a constant.
    pub fn variables(&self) -> Vec<String> {
        let mut variables = Vec::new();
        for slot in self.patterns.iter().flatten() {
            if let QuerySlot::Variable(variable) = slot {
                if !variables.contains(variable) {
                    variables.push(variable.clone());
                }
            }
        }
        variables
    }

    fn validate(&self) -> Result<(), PlanError> {
        if self.patterns.len() > MAX_DISCLOSURE_PATTERNS {
            return Err(PlanError::LimitExceeded("admitted BGP patterns"));
        }
        if self.filters.len() > MAX_DISCLOSURE_FILTERS {
            return Err(PlanError::LimitExceeded("admitted FILTER comparisons"));
        }
        if self.patterns.is_empty() {
            return Err(unsupported("empty BGP"));
        }
        let variables = self.variables();
        let mut projected = BTreeSet::new();
        for variable in &self.projection {
            if !variables.contains(variable) || !projected.insert(variable) {
                return Err(unsupported("unbound or repeated projection variable"));
            }
        }
        if self.kind == QueryKind::Ask && !self.projection.is_empty() {
            return Err(unsupported("ASK cannot release bindings"));
        }
        for filter in &self.filters {
            if !variables.contains(&filter.variable) {
                return Err(unsupported("unbound FILTER variable"));
            }
        }
        for pattern in &self.patterns {
            for (position, slot) in pattern.iter().enumerate() {
                if let QuerySlot::Constant(term) = slot {
                    let valid = match position {
                        0 | 1 => matches!(term, Term::NamedNode(_)),
                        _ => matches!(term, Term::NamedNode(_) | Term::Literal(_)),
                    };
                    if !valid {
                        return Err(unsupported("constant RDF term in this triple position"));
                    }
                }
            }
        }
        Ok(())
    }
}

fn unsupported(reason: &str) -> PlanError {
    PlanError::Unsupported(reason.into())
}

fn term_slot(term: &TermPattern) -> Result<QuerySlot, PlanError> {
    match term {
        TermPattern::NamedNode(n) => Ok(QuerySlot::Constant(n.clone().into())),
        TermPattern::Literal(l) => Ok(QuerySlot::Constant(l.clone().into())),
        TermPattern::Variable(v) => Ok(QuerySlot::Variable(v.as_str().to_owned())),
        _ => Err(unsupported("query blank nodes and triple terms")),
    }
}

fn collect_patterns(
    pattern: &GraphPattern,
    out: &mut Vec<[QuerySlot; 3]>,
) -> Result<BTreeSet<String>, PlanError> {
    enum Visit<'a> {
        Pattern(&'a GraphPattern),
        Join,
        Filter(&'a Expression),
    }
    let mut pending = vec![Visit::Pattern(pattern)];
    let mut scopes: Vec<BTreeSet<String>> = Vec::new();
    let mut nodes = 0;
    while let Some(visit) = pending.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("query AST nodes"));
        }
        match visit {
            Visit::Pattern(GraphPattern::Bgp { patterns }) => {
                if patterns.len() > MAX_DISCLOSURE_PATTERNS.saturating_sub(out.len()) {
                    return Err(PlanError::LimitExceeded("admitted BGP patterns"));
                }
                let mut variables = BTreeSet::new();
                for triple in patterns {
                    let predicate = match &triple.predicate {
                        NamedNodePattern::NamedNode(n) => QuerySlot::Constant(n.clone().into()),
                        NamedNodePattern::Variable(v) => QuerySlot::Variable(v.as_str().to_owned()),
                    };
                    let slots = [
                        term_slot(&triple.subject)?,
                        predicate,
                        term_slot(&triple.object)?,
                    ];
                    for slot in &slots {
                        if let QuerySlot::Variable(v) = slot {
                            variables.insert(v.clone());
                        }
                    }
                    out.push(slots);
                }
                scopes.push(variables);
            }
            Visit::Pattern(GraphPattern::Join { left, right }) => {
                pending.push(Visit::Join);
                pending.push(Visit::Pattern(right));
                pending.push(Visit::Pattern(left));
            }
            Visit::Join => {
                let right = scopes
                    .pop()
                    .ok_or_else(|| unsupported("missing right JOIN scope"))?;
                let left = scopes
                    .last_mut()
                    .ok_or_else(|| unsupported("missing left JOIN scope"))?;
                left.extend(right);
            }
            Visit::Pattern(GraphPattern::Filter { expr, inner }) => {
                pending.push(Visit::Filter(expr));
                pending.push(Visit::Pattern(inner));
            }
            Visit::Filter(expr) => {
                let variables = scopes
                    .last()
                    .ok_or_else(|| unsupported("missing FILTER input scope"))?;
                let mut filter_variables = BTreeSet::new();
                collect_filter_variables(expr, &mut filter_variables)?;
                // Flattening FILTERs is only valid when their variables are bound
                // by the FILTER's own input, not by a sibling join added later.
                if !filter_variables.is_subset(variables) {
                    return Err(unsupported("FILTER variable outside its input scope"));
                }
            }
            _ => return Err(unsupported("operator outside positive BGP/JOIN/FILTER")),
        }
    }
    scopes
        .pop()
        .ok_or_else(|| unsupported("missing query scope"))
}

fn collect_filter_variables(
    expr: &Expression,
    out: &mut BTreeSet<String>,
) -> Result<(), PlanError> {
    use Expression as E;
    let mut pending = vec![expr];
    let mut nodes = 0;
    while let Some(expr) = pending.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("FILTER AST nodes"));
        }
        match expr {
            E::Variable(v) => {
                out.insert(v.as_str().to_owned());
            }
            E::Literal(_) => {}
            E::And(a, b)
            | E::Less(a, b)
            | E::LessOrEqual(a, b)
            | E::Greater(a, b)
            | E::GreaterOrEqual(a, b)
            | E::Equal(a, b) => {
                pending.push(b);
                pending.push(a);
            }
            E::Not(inner) => pending.push(inner),
            _ => return Err(unsupported("FILTER expression")),
        }
    }
    Ok(())
}

/// A private membership witness location in the supplied credential ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MembershipRef {
    /// Index into the planner's credential slice; not a public graph identifier.
    pub credential: usize,
    /// Canonical triple/leaf index within that credential.
    pub leaf: usize,
}

/// A slot occurrence within a row's query patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRef {
    /// Query pattern index.
    pub pattern: usize,
    /// Subject, predicate, or object position (zero, one, or two).
    pub slot: usize,
}

/// An identity obligation connecting two hidden-variable occurrences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityObligation {
    /// Shared variable, without a leading question mark.
    pub variable: String,
    /// First occurrence of this variable.
    pub from: SlotRef,
    /// Another occurrence which must have the same RDF identity.
    pub to: SlotRef,
}

/// Predicate computation required for an authenticated variable binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterObligation {
    /// The verifier can evaluate this FILTER on the authenticated released term.
    PublicCheck {
        /// Index into the query's normalized FILTER list.
        filter: usize,
    },
    /// A backend must prove the FILTER against its hidden, authenticated operand.
    HiddenPredicate {
        /// Index into the query's normalized FILTER list.
        filter: usize,
        /// A triple slot carrying the operand, linked by identity obligations.
        operand: SlotRef,
    },
}

/// Required work for one released mapping, including PRIVATE witness locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRow {
    /// Released terms only, with exactly the query's projected variables.
    pub released: BTreeMap<String, Term>,
    /// PRIVATE selected membership witness for each pattern, in query order.
    pub witnesses: Vec<MembershipRef>,
    /// Public slots determined by the query and released result alone.
    pub public_slots: Vec<[Option<Term>; 3]>,
    /// A triple is present only when all three slots are publicly determined.
    /// Its membership and authentication obligations are retained separately.
    pub disclosed_triples: Vec<Option<Triple>>,
    /// Predicate obligations, including public checks requiring authentication.
    pub filters: Vec<FilterObligation>,
    /// A deterministic spanning star over each hidden variable's occurrences.
    pub identities: Vec<IdentityObligation>,
}

/// Deterministic obligation counts; these are not performance measurements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanMetrics {
    /// Released DISTINCT mappings (one witness for true ASK).
    pub released_rows: usize,
    /// Pattern occurrences before membership sharing.
    pub membership_occurrences: usize,
    /// Unique selected credential/leaf pairs after sharing.
    pub unique_memberships: usize,
    /// Credentials whose authentication is required, once each.
    pub authentication_obligations: usize,
    /// Supplied credentials which support no selected witness.
    pub unused_credentials: usize,
    /// Fully public triple occurrences, still requiring authenticated membership.
    pub disclosed_triples: usize,
    /// FILTER occurrences computed from released, authenticated terms.
    pub public_filter_checks: usize,
    /// FILTER occurrences requiring hidden predicate proofs.
    pub hidden_filter_obligations: usize,
    /// Hidden identity edges required across selected slots.
    pub hidden_identity_obligations: usize,
}

/// A prover-local disclosure plan, never a public proof or acceptance verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosurePlan {
    /// Rows retain the caller's released order; DISTINCT checks use RDF identity.
    pub rows: Vec<PlannedRow>,
    /// PRIVATE credential indices requiring authentication, sorted and deduplicated.
    pub authentication: Vec<usize>,
    /// PRIVATE membership witnesses, sorted and deduplicated across all results.
    pub memberships: Vec<MembershipRef>,
    /// Deterministic local metrics, which may reveal private witness structure.
    pub metrics: PlanMetrics,
}

impl DisclosurePlan {
    /// Explains selected work without printing hidden RDF terms or credential IDs.
    ///
    /// Counts themselves can reveal witness structure; keep this report local.
    pub fn explanation(&self) -> String {
        let m = &self.metrics;
        format!(
            "{} released rows; {} authenticated credentials; {} unique memberships from {} pattern occurrences; {} unused credentials; {} fully public triples; {} public FILTER checks; {} hidden FILTER obligations; {} hidden identity edges. Authentication remains required for every selected membership. Host planning is not cryptographic verification.",
            m.released_rows, m.authentication_obligations, m.unique_memberships,
            m.membership_occurrences, m.unused_credentials, m.disclosed_triples,
            m.public_filter_checks, m.hidden_filter_obligations, m.hidden_identity_obligations,
        )
    }
}

/// Resource bounds for deterministic witness search, independent of circuit capacities.
#[derive(Debug, Clone, Copy)]
pub struct PlannerLimits {
    /// Maximum BGP pattern count, bounding recursive join depth.
    pub max_patterns: usize,
    /// Maximum released mappings.
    pub max_results: usize,
    /// Maximum candidate triple attempts across the entire request.
    pub max_search_steps: usize,
}

impl Default for PlannerLimits {
    fn default() -> Self {
        // Host protection only, not a statement about supported circuit sizes.
        Self {
            max_patterns: 64,
            max_results: 1024,
            max_search_steps: 1_000_000,
        }
    }
}

/// A rejected query, release, or bounded witness search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// SPARQL parser failure.
    Parse(String),
    /// Query or term outside the explicitly admitted fragment.
    Unsupported(String),
    /// Released row has extra, missing, or otherwise unsupported bindings.
    InvalidRelease { row: usize, reason: String },
    /// Two released mappings have the same RDF-term bindings under DISTINCT.
    DuplicateRelease { first: usize, second: usize },
    /// No compatible full BGP/FILTER witness supports this released mapping.
    NoWitness { row: usize },
    /// A host resource bound was exceeded; no partial plan is returned.
    LimitExceeded(&'static str),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(reason) => write!(f, "query does not parse: {reason}"),
            Self::Unsupported(reason) => write!(f, "outside disclosure fragment: {reason}"),
            Self::InvalidRelease { row, reason } => {
                write!(f, "invalid released row {row}: {reason}")
            }
            Self::DuplicateRelease { first, second } => write!(
                f,
                "released rows {first} and {second} duplicate a DISTINCT mapping"
            ),
            Self::NoWitness { row } => write!(f, "no successful witness for released row {row}"),
            Self::LimitExceeded(limit) => write!(f, "disclosure planning limit exceeded: {limit}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Selects successful witnesses and retains every remaining proof obligation.
///
/// This is local witness preparation only. It does not authenticate the supplied
/// graph commitments, verify proofs, or establish answer completeness. Each
/// caller-supplied released row must be supported; other valid rows need not be
/// released. An empty SELECT release carries no positive assertion; ASK accepts
/// exactly one empty binding, representing `true`, and never accepts `false`.
///
/// # Errors
/// Rejects unsupported shapes, incorrect projection domains, duplicate rows,
/// projected blank nodes, unsupported numeric witnesses, unsupported rows, or
/// exhausted host resource bounds. No partial plan is returned on failure.
pub fn plan_disclosure(
    query: &DisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: PlannerLimits,
) -> Result<DisclosurePlan, PlanError> {
    plan_disclosure_admitted(query, credentials, released, limits, |_, _, _| true)
}

/// Selects witnesses after a backend restricts eligible candidate triples.
///
/// [GPT-6] `admit` receives the pattern index, original credential/leaf reference
/// and candidate triple. Returning false only removes a candidate; it cannot
/// waive query matching, FILTER, join, projection or resource checks. This is
/// prover-local eligibility, never a verifier trust or authentication decision.
///
/// # Errors
/// Returns the same validation and resource errors as [`plan_disclosure`], or
/// `NoWitness` when the admitted candidates cannot support a released answer.
pub fn plan_disclosure_admitted<A>(
    query: &DisclosureQuery,
    credentials: &[GraphCommitment],
    released: &[BTreeMap<String, Term>],
    limits: PlannerLimits,
    admit: A,
) -> Result<DisclosurePlan, PlanError>
where
    A: Fn(usize, MembershipRef, &Triple) -> bool,
{
    if credentials.len() > MAX_DISCLOSURE_CREDENTIALS {
        return Err(PlanError::LimitExceeded("input credentials"));
    }
    if query.patterns.len() > limits.max_patterns {
        return Err(PlanError::LimitExceeded("patterns"));
    }
    if released.len() > limits.max_results {
        return Err(PlanError::LimitExceeded("released rows"));
    }
    query.validate()?;
    if query.kind == QueryKind::Ask && (released.len() != 1 || !released[0].is_empty()) {
        return Err(PlanError::InvalidRelease {
            row: 0,
            reason: "true ASK requires exactly one empty binding; false ASK is unsupported".into(),
        });
    }
    validate_released(query, released)?;
    let mut budget = SearchBudget {
        used: 0,
        limit: limits.max_search_steps,
    };
    let mut selected = Vec::with_capacity(released.len());
    for (row_index, release) in released.iter().enumerate() {
        let bindings = released_bindings(release);
        let mut witnesses = Vec::with_capacity(query.patterns.len());
        if !find_witness(
            query,
            credentials,
            0,
            &bindings,
            &mut witnesses,
            &mut budget,
            &admit,
        )? {
            return Err(PlanError::NoWitness { row: row_index });
        }
        selected.push(witnesses);
    }
    assemble_plan(query, credentials.len(), released, selected)
}

fn assemble_plan(
    query: &DisclosureQuery,
    credential_count: usize,
    released: &[BTreeMap<String, Term>],
    selected: Vec<Vec<MembershipRef>>,
) -> Result<DisclosurePlan, PlanError> {
    let mut rows = Vec::with_capacity(released.len());
    let mut memberships = BTreeSet::new();
    let mut authentication = BTreeSet::new();
    let mut metrics = PlanMetrics {
        released_rows: released.len(),
        ..Default::default()
    };
    for (release, witnesses) in released.iter().zip(selected) {
        for witness in &witnesses {
            memberships.insert(*witness);
            authentication.insert(witness.credential);
        }
        let row = row_obligations(query, release, witnesses)?;
        metrics.membership_occurrences += row.witnesses.len();
        metrics.disclosed_triples += row.disclosed_triples.iter().filter(|t| t.is_some()).count();
        metrics.public_filter_checks += row
            .filters
            .iter()
            .filter(|f| matches!(f, FilterObligation::PublicCheck { .. }))
            .count();
        metrics.hidden_filter_obligations += row
            .filters
            .iter()
            .filter(|f| matches!(f, FilterObligation::HiddenPredicate { .. }))
            .count();
        metrics.hidden_identity_obligations += row.identities.len();
        rows.push(row);
    }
    metrics.unique_memberships = memberships.len();
    metrics.authentication_obligations = authentication.len();
    metrics.unused_credentials = credential_count - authentication.len();
    Ok(DisclosurePlan {
        rows,
        authentication: authentication.into_iter().collect(),
        memberships: memberships.into_iter().collect(),
        metrics,
    })
}

fn validate_released(
    query: &DisclosureQuery,
    released: &[BTreeMap<String, Term>],
) -> Result<(), PlanError> {
    let expected: BTreeSet<_> = query.projection.iter().collect();
    // N-Triples term spellings distinguish RDF-term identity (including lexical
    // variants), unlike numeric value equality used by FILTER.
    let mut seen = BTreeMap::<Vec<String>, usize>::new();
    for (row, release) in released.iter().enumerate() {
        if release.keys().collect::<BTreeSet<_>>() != expected {
            return Err(PlanError::InvalidRelease {
                row,
                reason: "bindings must exactly match the projection".into(),
            });
        }
        if release
            .values()
            .any(|t| !matches!(t, Term::NamedNode(_) | Term::Literal(_)))
        {
            return Err(PlanError::InvalidRelease {
                row,
                reason: "projected blank nodes and triple terms are unsupported".into(),
            });
        }
        let key: Vec<_> = release.values().map(ToString::to_string).collect();
        if let Some(first) = seen.insert(key, row) {
            return Err(PlanError::DuplicateRelease { first, second: row });
        }
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct ScopedTerm {
    term: Term,
    // Only blank nodes carry scope; equal IRIs/literals can join across graphs.
    credential: Option<usize>,
}

fn released_bindings(release: &BTreeMap<String, Term>) -> BTreeMap<String, ScopedTerm> {
    release
        .iter()
        .map(|(variable, term)| {
            (
                variable.clone(),
                ScopedTerm {
                    term: term.clone(),
                    credential: None,
                },
            )
        })
        .collect()
}

fn triple_terms(triple: &Triple) -> [Term; 3] {
    [
        triple.subject.clone().into(),
        triple.predicate.clone().into(),
        triple.object.clone(),
    ]
}

struct SearchBudget {
    used: usize,
    limit: usize,
}

fn find_witness<A: Fn(usize, MembershipRef, &Triple) -> bool>(
    query: &DisclosureQuery,
    credentials: &[GraphCommitment],
    pattern_index: usize,
    bindings: &BTreeMap<String, ScopedTerm>,
    witnesses: &mut Vec<MembershipRef>,
    budget: &mut SearchBudget,
    admit: &A,
) -> Result<bool, PlanError> {
    if pattern_index == query.patterns.len() {
        return Ok(query.filters.iter().all(|filter| {
            bindings
                .get(&filter.variable)
                .and_then(|v| canonical_integer(&v.term))
                .is_some_and(|value| integer_comparison(value, filter.op, filter.bound))
        }));
    }
    for (credential, graph) in credentials.iter().enumerate() {
        for (leaf, triple) in graph.canonical.triples.iter().enumerate() {
            if budget.used >= budget.limit {
                return Err(PlanError::LimitExceeded("candidate triple attempts"));
            }
            budget.used += 1;
            if !admit(pattern_index, MembershipRef { credential, leaf }, triple) {
                continue;
            }
            if let Some(next) = extend_bindings(query, pattern_index, credential, triple, bindings)
            {
                witnesses.push(MembershipRef { credential, leaf });
                if find_witness(
                    query,
                    credentials,
                    pattern_index + 1,
                    &next,
                    witnesses,
                    budget,
                    admit,
                )? {
                    return Ok(true);
                }
                witnesses.pop();
            }
        }
    }
    Ok(false)
}

// Shared by first-success and optimizing selection: admission can only restrict
// candidates; RDF identity and early predicates always use this same relation.
fn extend_bindings(
    query: &DisclosureQuery,
    pattern_index: usize,
    credential: usize,
    triple: &Triple,
    bindings: &BTreeMap<String, ScopedTerm>,
) -> Option<BTreeMap<String, ScopedTerm>> {
    let mut next = bindings.clone();
    for (slot, term) in query.patterns[pattern_index]
        .iter()
        .zip(triple_terms(triple))
    {
        match slot {
            QuerySlot::Constant(expected) if term != *expected => return None,
            QuerySlot::Constant(_) => {}
            QuerySlot::Variable(variable) => {
                let scope = matches!(&term, Term::BlankNode(_)).then_some(credential);
                let value = ScopedTerm {
                    term,
                    credential: scope,
                };
                match next.get(variable) {
                    Some(expected) if value != *expected => return None,
                    Some(_) => {}
                    None => {
                        next.insert(variable.clone(), value);
                    }
                }
            }
        }
    }
    // Parsing required every FILTER variable to be directly bound in its own
    // positive input scope, so rejecting an already-bound failing value is safe.
    query
        .filters
        .iter()
        .all(|filter| {
            next.get(&filter.variable).is_none_or(|value| {
                canonical_integer(&value.term)
                    .is_some_and(|v| integer_comparison(v, filter.op, filter.bound))
            })
        })
        .then_some(next)
}

/// Parses the exact canonical nonnegative xsd:integer witness representation.
///
/// Other valid SPARQL numeric types or lexical forms belong to different proof
/// relations; this helper deliberately does not coerce them into this one.
pub fn canonical_integer(term: &Term) -> Option<u64> {
    let Term::Literal(literal) = term else {
        return None;
    };
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#integer" {
        return None;
    }
    let lexical = literal.value();
    if lexical.is_empty()
        || !lexical.bytes().all(|b| b.is_ascii_digit())
        || (lexical.len() > 1 && lexical.starts_with('0'))
    {
        return None;
    }
    lexical.parse().ok()
}

/// Evaluates a normalized comparison on public integers or local witness values.
pub fn integer_comparison(value: u64, op: FilterOp, bound: u64) -> bool {
    match op {
        FilterOp::Lt => value < bound,
        FilterOp::Le => value <= bound,
        FilterOp::Gt => value > bound,
        FilterOp::Ge => value >= bound,
        FilterOp::Eq => value == bound,
        FilterOp::Ne => value != bound,
    }
}

fn row_obligations(
    query: &DisclosureQuery,
    released: &BTreeMap<String, Term>,
    witnesses: Vec<MembershipRef>,
) -> Result<PlannedRow, PlanError> {
    let mut public_slots = Vec::new();
    let mut disclosed_triples = Vec::new();
    let mut hidden_occurrences: BTreeMap<String, Vec<SlotRef>> = BTreeMap::new();
    for (pattern_index, pattern) in query.patterns.iter().enumerate() {
        let mut slots: [Option<Term>; 3] = [None, None, None];
        for (position, slot) in pattern.iter().enumerate() {
            slots[position] = match slot {
                QuerySlot::Constant(term) => Some(term.clone()),
                QuerySlot::Variable(variable) => match released.get(variable) {
                    Some(term) => Some(term.clone()),
                    None => {
                        hidden_occurrences
                            .entry(variable.clone())
                            .or_default()
                            .push(SlotRef {
                                pattern: pattern_index,
                                slot: position,
                            });
                        None
                    }
                },
            };
        }
        let disclosed = match (&slots[0], &slots[1], &slots[2]) {
            (Some(Term::NamedNode(s)), Some(Term::NamedNode(p)), Some(o)) => {
                Some(Triple::new(s.clone(), p.clone(), o.clone()))
            }
            _ => None,
        };
        public_slots.push(slots);
        disclosed_triples.push(disclosed);
    }
    let mut identities = Vec::new();
    for (variable, occurrences) in &hidden_occurrences {
        for to in &occurrences[1..] {
            identities.push(IdentityObligation {
                variable: variable.clone(),
                from: occurrences[0],
                to: *to,
            });
        }
    }
    let mut filters = Vec::new();
    for (index, filter) in query.filters.iter().enumerate() {
        if released.contains_key(&filter.variable) {
            filters.push(FilterObligation::PublicCheck { filter: index });
        } else {
            let operand = hidden_occurrences
                .get(&filter.variable)
                .and_then(|slots| slots.first())
                .copied()
                .ok_or_else(|| unsupported("FILTER variable has no authenticated slot"))?;
            filters.push(FilterObligation::HiddenPredicate {
                filter: index,
                operand,
            });
        }
    }
    Ok(PlannedRow {
        released: released.clone(),
        witnesses,
        public_slots,
        disclosed_triples,
        filters,
        identities,
    })
}
