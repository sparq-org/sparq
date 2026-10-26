// Shared value types crossing the MPC module boundaries (M0).
//! Shared, crypto-free value types used across the MPC module boundaries.
//!
//! Architecture refs: §4.1 (parties — holders are identified), §4.3 step 3
//! (disclosure minimisation — a partial carries only disclosed bindings), and
//! the empirical-honesty discipline (every fork-dependent path surfaces a
//! single, named [`MpcError::NotYetImplemented`] rather than fake crypto).

use oxrdf::{Term, Variable};

/// Identifies one holder (data source / MPC compute party) within a federation.
///
/// Architecture §4.1: holders are the four Pods — mutually distrusting,
/// possibly malicious, acting as MPC compute parties and collaborative provers.
/// At M0 this is just a stable label; the cryptographic identity (the holder's
/// signing key / proof-of-possession) is gated on the ZK foundation and lives
/// behind [`crate::proof`], not here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HolderId(pub String);

impl HolderId {
    pub fn new(id: impl Into<String>) -> Self {
        HolderId(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HolderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One holder's *local partial result*: the disclosed bindings that holder
/// computed over ITS OWN graphs, ready to be joined on global IRIs.
///
/// Architecture §4.2 / §4.3 step 1: this is the unit of inter-node sharing, and
/// the whole point of the design is to MINIMISE it — a holder ships these
/// partials, **never** its raw named graphs. The row shape mirrors
/// `sparq_engine::QueryResult` (`vars` + nullable `rows`) so a holder never has
/// to re-encode its own engine output to produce a partial.
///
/// What is and is NOT disclosed in a partial is the RQ2a question (§4.3 step 3,
/// §5.2 Q3). At M0 the local evaluation discloses its full local SELECT result
/// (the "disclosed-property" regime of convention #4); the *hidden*-intermediate
/// regime (values that must never leave a source, fed into the MPC core) is
/// future work behind [`crate::backend`] and is NOT represented here yet.
#[derive(Debug, Clone, PartialEq)]
pub struct PartialResult {
    /// The holder that produced this partial.
    pub holder: HolderId,
    /// The projected variables, unique and in column order (matches `rows`).
    pub vars: Vec<Variable>,
    /// Disclosed solution rows; one entry per `vars` column, `None` = unbound.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl PartialResult {
    /// Check that projected variables are unique and every disclosed row has
    /// exactly one slot per variable. Refusals identify only public shape
    /// metadata (holder, expected arity, row or column index), never term values.
    pub fn check_shape(&self) -> Result<(), MpcError> {
        let expected = self.vars.len();
        let mut seen = std::collections::BTreeSet::new();
        for (column, variable) in self.vars.iter().enumerate() {
            if !seen.insert(variable.as_str()) {
                return Err(MpcError::Protocol(format!(
                    "holder {} repeats a projected variable at column {column} (expected arity {expected})",
                    self.holder,
                )));
            }
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.len() != expected {
                return Err(MpcError::Protocol(format!(
                    "holder {} expected arity {expected} at row {index}",
                    self.holder,
                )));
            }
        }
        Ok(())
    }

    /// Number of disclosed rows in this partial.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// `true` if this holder contributed no rows for the fragment.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The crate's error type. Crucially, [`MpcError::NotYetImplemented`] is the
/// SINGLE honest channel for everything that is fork-dependent or gated on the
/// ZK foundation: no method in this crate fakes crypto — it either does real
/// work (local eval) or returns this with the gating milestone/issue named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpcError {
    /// A holder's local SPARQL sub-evaluation failed (parse / eval error from
    /// `sparq-engine`, surfaced verbatim). This is a REAL error path — local
    /// evaluation is the one piece implemented at M0.
    LocalEval { holder: HolderId, message: String },
    /// A federation/protocol precondition was violated before any crypto would
    /// run (e.g. holders disagree on the projected variable list of a fragment,
    /// or an empty federation). Real, M0-checkable.
    Protocol(String),
    /// The single honest stub channel: this path is gated on a milestone and/or
    /// an open design fork. `what` names the capability; `gated_on` names the
    /// milestone(s), ZK-remediation issue(s), and/or open question(s) that must
    /// land first. Carrying the gate in the value (not just a doc-comment) keeps
    /// the deferral auditable at the call site.
    NotYetImplemented { what: String, gated_on: String },
    /// **Active tampering detected** during a consistency-checked / robust
    /// reconstruction (honest-majority tamper-evident detect-and-abort toward
    /// guarantee (D) — NOT malicious security; sq-m34i).
    /// Returned when redundant Shamir shares (`n > t+1`) do NOT all lie on one
    /// degree-`t` polynomial and the inconsistency exceeds the correctable error
    /// budget `e = ⌊(n−t−1)/2⌋`. This is **not** a precondition bug
    /// ([`MpcError::Protocol`]) nor a deferred stub ([`MpcError::NotYetImplemented`]):
    /// it means a party fed an inconsistent share value, so the result is
    /// REFUSED rather than silently corrupted. `cheaters` lists the evaluation
    /// points (`Share::x`) identified as off-curve. On this abort path it is the
    /// off-curve set of the MOST self-consistent (minimum-disagreement) reference
    /// fit — sound (names only true cheaters) in the first abort band of `e+1`
    /// errors, best-effort beyond it where the honest set is genuinely
    /// unidentifiable: detection is sound, blame is heuristic. Sound, exact
    /// attribution is available on the SUCCESS path via
    /// [`crate::robust::reconstruct_robust_attributed`]. NOTE: at exactly `t+1`
    /// shares (no redundancy) tampering is information-theoretically undetectable
    /// and this is NEVER returned there — see
    /// [`crate::robust::reconstruct_robust`].
    Tampered { detail: String, cheaters: Vec<u64> },
    /// **No registered backend satisfies the stated security requirement** — the
    /// fail-closed result of [`crate::backend::BackendRegistry::select`] (sq-a6p1).
    /// A federation states its [`crate::backend::SecurityRequirement`] UP FRONT
    /// and the registry matches it against each backend's
    /// [`crate::backend::BackendInfo`]; if none qualifies the request is
    /// truthfully REFUSED here rather than silently served by a weaker backend.
    /// This is the negotiation-layer twin of [`MpcError::NotYetImplemented`]: a
    /// dishonest-majority-malicious request over the SPARQL pipeline returns this,
    /// never a downgraded answer. `requirement` is the human-readable requirement
    /// that could not be met; `considered` is how many backends were inspected and
    /// rejected.
    NoBackendSatisfies {
        requirement: String,
        considered: usize,
    },
    /// Security-qualified backends all failed the declared preprocessing budget.
    ///
    /// Distinct from [`MpcError::NoBackendSatisfies`]: at least one backend met
    /// the security floor, but every such candidate exceeded a set ceiling or
    /// had an unknown/overflowing estimate for it.
    PreprocessingBudgetRefused {
        /// The security floor that candidates met.
        requirement: String,
        /// Workload and inclusive ceilings that could not be met.
        budget: crate::backend::PreprocessingBudget,
        /// Each rejected security-qualified backend's name and cost estimates.
        rejected: Vec<String>,
    },
    /// **The batched IT-MAC check FAILED** (`σ ≠ 0`) just before a value was about
    /// to be opened — the integrity step of the honest-majority
    /// IT-MAC tier (tamper-evident with abort, NOT AXIS-1 `Malicious` today) (design `research/mpc-malicious-security-design.md`
    /// §2.5, sq-km34.4 / sq-ka8m). Every authenticated value carries an IT-MAC
    /// `m_x = α·x` under a session-global, secret-shared `[α]` no party knows; the
    /// batched check opens a leakage-free `σ = Σ χ_j·[m_{y_j}] − (Σ χ_j·y_j)·[α]`,
    /// which is identically `0` for honest executions. A non-zero `σ` means some
    /// opened value `y_j` was tampered (a forged share, a wrong `degree_reduce`
    /// re-sharing, an inconsistent input) WITHOUT a consistent matching change to
    /// its MAC — impossible without knowing `α` — so the result is REFUSED
    /// fail-closed (`≈ 1 − 2^{−61}` soundness over `F_p`) rather than opened wrong
    /// or leaked. Distinct from [`MpcError::Tampered`] (the Reed–Solomon
    /// over-determination check, which needs `n > 2t+1` redundancy): this check
    /// works at the MINIMAL `n = 2t+1` because soundness comes from the *secret*
    /// `α`, not from codeword redundancy. `detail` describes the batch that failed.
    MacCheckFailed { detail: String },
}

impl MpcError {
    /// Construct a [`MpcError::NotYetImplemented`] from static descriptions.
    /// Used by every crypto-deferred trait method so the gate is uniform and
    /// greppable.
    pub fn not_yet(what: &str, gated_on: &str) -> Self {
        MpcError::NotYetImplemented {
            what: what.to_string(),
            gated_on: gated_on.to_string(),
        }
    }
}

/// Name the KIND of a result-row slot — never its value.
///
/// Refusal messages on paths that handle private inputs, join keys or opened
/// intermediate values must stay free of the values themselves, because an error
/// string travels further than the value it describes (logs, a federation peer's
/// response body, a user-visible message). The kind is enough to diagnose a
/// malformed partial; the value adds nothing a caller cannot get from its own
/// input, which it already holds.
pub(crate) fn term_slot_kind(slot: Option<&Term>) -> &'static str {
    match slot {
        None => "an unbound binding",
        Some(Term::NamedNode(_)) => "an IRI",
        Some(Term::BlankNode(_)) => "a blank node",
        Some(Term::Literal(_)) => "a literal",
        Some(_) => "a term of an unsupported kind",
    }
}

impl std::fmt::Display for MpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MpcError::LocalEval { holder, message } => {
                write!(f, "holder {holder} local sub-evaluation failed: {message}")
            }
            MpcError::Protocol(m) => write!(f, "MPC protocol precondition violated: {m}"),
            MpcError::NotYetImplemented { what, gated_on } => {
                write!(f, "not yet implemented: {what} (gated on {gated_on})")
            }
            MpcError::Tampered { detail, cheaters } => {
                write!(
                    f,
                    "tampered share(s) detected: {detail} (suspect evaluation points: {cheaters:?})"
                )
            }
            MpcError::NoBackendSatisfies {
                requirement,
                considered,
            } => {
                write!(
                    f,
                    "no registered MPC backend satisfies the security requirement \
                     [{requirement}] (refused fail-closed after considering {considered} backend(s))"
                )
            }
            MpcError::PreprocessingBudgetRefused {
                requirement,
                budget,
                rejected,
            } => {
                write!(
                    f,
                    "no security-qualified MPC backend fits preprocessing budget {budget:?} \
                     for [{requirement}] (refused fail-closed: {})",
                    rejected.join("; ")
                )
            }
            MpcError::MacCheckFailed { detail } => {
                write!(
                    f,
                    "IT-MAC check FAILED (σ != 0) before open — a tampered value/share/re-sharing \
                     was caught by the batched MAC-check; refusing fail-closed: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for MpcError {}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::Literal;

    fn partial(holder: &str) -> PartialResult {
        PartialResult {
            holder: HolderId::new(holder),
            vars: vec![Variable::new_unchecked("key")],
            rows: vec![vec![Some(Term::Literal(Literal::new_simple_literal(
                "shape-term-sentinel",
            )))]],
        }
    }

    fn shape_error(holder: &str, row: usize) -> MpcError {
        MpcError::Protocol(format!("holder {holder} expected arity 1 at row {row}"))
    }

    #[test]
    fn check_shape_refuses_short_and_long_rows_without_term_values() {
        for width in [0, 2] {
            for row in [0, 1] {
                let mut p = partial("shape-holder");
                p.rows = vec![p.rows[0].clone(); 2];
                let slot = p.rows[0][0].clone();
                p.rows[row].resize(width, slot);
                assert_eq!(p.check_shape(), Err(shape_error("shape-holder", row)));
            }
        }
    }

    #[test]
    fn check_shape_refuses_duplicate_variables_even_without_rows() {
        let mut p = partial("duplicate-holder");
        p.vars.push(p.vars[0].clone());
        for rows in [vec![], vec![vec![None, None]]] {
            p.rows = rows;
            assert_eq!(
                p.check_shape(),
                Err(MpcError::Protocol(
                    "holder duplicate-holder repeats a projected variable at column 1 (expected arity 2)"
                        .into(),
                ))
            );
        }
    }

    #[test]
    fn check_shape_accepts_empty_and_unbound_rows_without_changing_them() {
        let mut p = partial("valid-holder");
        for rows in [vec![], vec![vec![None]], p.rows.clone()] {
            p.rows = rows;
            let before = p.clone();
            assert_eq!(p.check_shape(), Ok(()));
            assert_eq!(p, before);
        }
        p.vars.clear();
        p.rows = vec![vec![]];
        assert_eq!(p.check_shape(), Ok(()));
        p.rows.clear();
        assert_eq!(p.check_shape(), Ok(()));
    }
}
