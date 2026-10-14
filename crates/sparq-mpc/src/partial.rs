// [OPUS-4.8] Shared value types crossing the MPC module boundaries (M0).
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
    /// The projected variables, in column order (matches `rows`).
    pub vars: Vec<Variable>,
    /// Disclosed solution rows; one entry per `vars` column, `None` = unbound.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl PartialResult {
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
    /// reconstruction (guarantee (D), malicious-honest-majority abort — sq-m34i).
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
    /// rejected. `[OPUS-4.8]`
    NoBackendSatisfies {
        requirement: String,
        considered: usize,
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
    /// `[OPUS-4.8]`
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
