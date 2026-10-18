#![doc = include_str!("../README.md")] // README is the docs.rs front page
#![forbid(unsafe_code)] // sq-r06h: crate has zero `unsafe`

// sq-zi5w: stateful `odrl:count` enforcement, gated on `count-enforcement`
// (OFF by default — the default build carries zero counter-store code or runtime cost).
#[cfg(feature = "count-enforcement")]
pub mod count;

// sq-5z1q: a CROSS-PROCESS `UsageCounterStore` (a file + OS lockfile) — the
// distributed-atomicity follow-up to sq-zi5w's in-process `InMemoryCounterStore`. Same
// feature gate, same `try_consume` contract, std-only (no new deps, no `unsafe`).
#[cfg(feature = "count-enforcement")]
pub mod count_file;

// sq-u3yo: the CROSS-HOST `UsageCounterStore` seam — an `AtomicCounterBackend`
// trait (one Redis-`INCR` / SQL-`UPDATE…WHERE consumed<limit` atomic op) + a
// `BackendCounterStore` adapter, the drop-in for a distributed backend against the
// unchanged trait. Still std-only (no Redis/SQL client bundled — the operator brings it).
#[cfg(feature = "count-enforcement")]
pub mod count_backend;

// sq-uor3g (epic sq-0dksu, Phase 4): the sparq security-property ODRL
// PROFILE — the `secx:requires…` leftOperands made first-class (Rust constants + the
// leftOperand→dimension map + a recogniser + the published `odrl:Profile`), gated on
// `secprop-leftoperands` (OFF by default — the default build carries zero secprop
// code, no new dep). It does NOT change the stateless `evaluate` path.
#[cfg(feature = "secprop-leftoperands")]
pub mod secprop;

// sq-mu4au: opt-in, read-only summaries of already-computed decisions.
#[cfg(feature = "decision-report")]
pub mod report;

// sq-zabv: static (request-free) policy analysis — permission/prohibition
// conflict + policy containment/refinement, on the query-containment comparison
// semantics. Always compiled (no feature gate); it adds no deps and no core cost.
mod compare;
mod eval;
pub mod model;
mod parse;
mod validate;

pub use compare::{
    conflict_admissibility, contains, detect_conflicts, Conflict, Containment, Overlap,
};
pub use eval::{
    cmp_datetime, datetime_status, decide, duty_discharged, evaluate, matched_prohibition,
    prohibition_status, purpose_status, recipient_status, spatial_status, DateTimeMatch, Decision,
    Permit, ProhibitionStatus, PurposeMatch, RecipientMatch, Request, SpatialMatch, ODRL_COUNT, ODRL_DATETIME, ODRL_PURPOSE,
    ODRL_RECIPIENT, ODRL_SPATIAL,
};
pub use model::{
    Action, ConflictStrategy, Constraint, ConstraintNode, Duty, LogicalConstraint, LogicalOperator,
    Operator, Policy, Rule, Value, ODRL_NS,
};
pub use parse::{
    constraint_budget_refusal, parse_policy, parse_policy_str, parse_policy_str_with_memberships,
    target_memberships, ConstraintBudget, CONSTRAINT_BUDGET_REFUSAL,
};
pub use validate::ValidatedPolicy;

// sq-zi5w: re-export the count-enforcement surface at the crate root when the
// feature is on (matches how the stateless evaluator surface is flat-exported).
#[cfg(feature = "count-enforcement")]
pub use count::{
    base_decision, count_status, evaluate_and_exercise, ConsumeResult, CountKey, CountStatus, ExerciseDecision,
    InMemoryCounterStore, UsageCounterStore,
};

// sq-5z1q: the cross-process file-backed store, flat-exported alongside the
// in-memory one (a drop-in for the same `UsageCounterStore` seam).
#[cfg(feature = "count-enforcement")]
pub use count_file::FileCounterStore;

// sq-u3yo: the cross-host backend seam — the `AtomicCounterBackend` trait an
// operator implements over Redis/SQL + the `BackendCounterStore` adapter (and its
// outcome/error types), flat-exported alongside the in-memory/file stores.
#[cfg(feature = "count-enforcement")]
pub use count_backend::{AtomicCounterBackend, BackendCounterStore, BackendError, BackendOutcome};
