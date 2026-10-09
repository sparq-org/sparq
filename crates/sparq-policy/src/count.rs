//! Stateful `odrl:count` constraint enforcement. [OPUS-4.8] sq-zi5w.
//!
//! `odrl:count` limits the **number of times** a permission may be exercised (the
//! ODRL [count left-operand](https://www.w3.org/TR/odrl-vocab/#term-count) — e.g.
//! "may read at most 5 times"). Unlike the *stateless* constraints the base evaluator
//! checks ([`crate::evaluate`] — purpose, dateTime, recipient: a single
//! `(leftOperand, operator, rightOperand)` test against evidence the request carries),
//! a count limit is **stateful**: the truth of "have we used this up?" lives in a
//! usage counter that persists *across requests*, not in any one request's context.
//!
//! This module adds that statefulness as an **opt-in** layer (the `count-enforcement`
//! feature) ON TOP OF the unchanged evaluator. It does NOT change how
//! [`crate::evaluate`] treats `odrl:count`: a caller who supplies a count value in the
//! request context still gets the stateless numeric comparison. What this module adds
//! is a counter store the *engine itself* owns and increments per exercise, so the
//! limit is enforced even when no caller volunteers a (truthful) count.
//!
//! # The honesty boundary (what is enforced end-to-end vs deferred)
//!
//! [`evaluate_and_exercise`] is the soundly-checkable core, and it is wired end-to-end
//! through the **real** [`crate::evaluate`] decision: a request is granted ONLY if the
//! base evaluator grants it AND every applicable count limit still has budget, and the
//! grant **atomically consumes** one unit of that budget. The first *N* exercises of an
//! "at most *N*" permission grant; the *(N+1)*th denies; and a counter whose state is
//! unavailable denies (fail-closed — never silently grant beyond the limit). All of
//! this is exercised by the unit tests through this real path.
//!
//! **Deferred (documented, not claimed):** the `sparq-solid` ODRL→ACP bridge is
//! *stateless* — ACP grants are re-checked per session against static matcher
//! accept-sets, with no usage counter to decrement. Wiring this stateful path through
//! the materialized ACP enforcement (so a bridged grant *self-retracts* once the count
//! is reached) is a distinct, more invasive change and is **not** done here — the
//! bridge continues to treat `odrl:count` as one-shot/unmappable (see the
//! `usage-control-policy` skill mapping table). This module is the evaluator + store
//! seam that such a bridge would build on.
//!
//! # Concurrency / atomicity boundary
//!
//! A naive "read the count, decide, then increment" is a **TOCTOU race**: two
//! concurrent exercises could both read `count == N-1`, both decide "ok", and both
//! increment to `N+1` — granting *twice* over the limit. To close that within the
//! engine, the [`UsageCounterStore`] seam is **check-and-consume**, not read-then-write:
//! [`UsageCounterStore::try_consume`] is the single atomic operation "if budget remains,
//! consume one unit and report success; else report exhausted". The default
//! [`InMemoryCounterStore`] makes this atomic with a `Mutex` (the whole compare-and-
//! increment happens under one lock). A *distributed* store (Redis `INCR`, a SQL
//! `UPDATE ... WHERE count < limit RETURNING`, …) is the operator's concern; it MUST
//! provide the same atomicity, and the trait contract says so explicitly. We do not
//! pretend a multi-process deployment is safe with the in-memory store — that boundary
//! is documented, and the "atomic across processes" hardening is a deferred bead.

use crate::eval::{evaluate, Decision, Request, ODRL_COUNT};
use crate::model::{Operator, Policy, Rule, Value};
use crate::validate::ValidatedPolicy;
use std::collections::HashMap;
use std::sync::Mutex;

/// A stable identity for a single counted permission-exercise budget. Two requests
/// that should share a usage counter (the same party exercising the same permission on
/// the same target) map to the **same** key; requests that should be counted
/// independently map to **different** keys. [OPUS-4.8] sq-zi5w.
///
/// The key is `(rule_id, party, target)`:
/// - `rule_id` — *which* count-constrained rule's budget (a policy may carry several).
/// - `party` — *whose* budget (an "at most 5 reads" limit is per-assignee; one party
///   exhausting it must not lock out another). A request with no party shares the
///   anonymous bucket (`""`).
/// - `target` — *which asset's* budget (the same rule on different targets counts
///   separately). A request with no target shares the bucket (`""`).
///
/// The bound itself (the `N` of "at most N") is NOT part of the key: it lives in the
/// policy, and [`evaluate_and_exercise`] re-reads it each call, so raising/lowering the
/// limit takes effect on the next exercise without resetting the consumed count.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CountKey {
    /// The count-constrained rule's id (`Rule::id`).
    pub rule_id: String,
    /// The exercising party (WebID), or `""` for an anonymous/partyless request.
    pub party: String,
    /// The target asset/graph IRI, or `""` for a targetless request.
    pub target: String,
}

impl CountKey {
    /// The usage-counter key for `rule` exercised by `request` (see the type docs for
    /// the `(rule_id, party, target)` rationale).
    pub fn for_rule(rule: &Rule, request: &Request) -> CountKey {
        CountKey {
            rule_id: rule.id.clone(),
            party: request.party.clone().unwrap_or_default(),
            target: request.target.clone().unwrap_or_default(),
        }
    }
}

/// An injectable usage-counter store: the persistence seam for stateful `odrl:count`
/// enforcement. The engine core keeps only the in-memory default
/// ([`InMemoryCounterStore`]); the real persistence (Redis, SQL, a Solid pod resource)
/// is the operator's concern. [OPUS-4.8] sq-zi5w.
///
/// # The atomicity contract (load-bearing — read this)
///
/// The single mutating operation is [`try_consume`](UsageCounterStore::try_consume),
/// and it MUST be **atomic**: the "is there budget? if so, consume one unit"
/// check-and-decrement happens as one indivisible step, even under concurrent callers
/// for the same [`CountKey`]. This is deliberately NOT a read-then-write API — a
/// `current(key)` + `increment(key)` pair would reintroduce the TOCTOU race
/// [`evaluate_and_exercise`] exists to avoid. An implementation that cannot make
/// `try_consume` atomic (across the processes that share the limit) does NOT satisfy
/// this contract and MUST fail closed.
///
/// # Fail-closed
///
/// `try_consume` returns a [`ConsumeResult`]. [`ConsumeResult::Unavailable`] (the store
/// could not be read/written — a backend outage, a poisoned lock) is treated by
/// [`evaluate_and_exercise`] as a DENY: the engine never grants beyond a limit it
/// cannot account for.
pub trait UsageCounterStore {
    /// Atomically: if the budget for `key` has fewer than `limit` units consumed,
    /// consume one unit and return [`ConsumeResult::Consumed`] with the *new* consumed
    /// count (`1..=limit`); if the budget is already fully consumed
    /// (`consumed >= limit`), consume nothing and return [`ConsumeResult::Exhausted`];
    /// if the store cannot be accessed, return [`ConsumeResult::Unavailable`].
    ///
    /// `limit` is the `N` of the policy's "at most N" (a non-negative integer; a limit
    /// of 0 means the permission can never be exercised). The store does not persist
    /// `limit` — it is supplied each call so a policy change takes effect immediately.
    fn try_consume(&self, key: &CountKey, limit: u64) -> ConsumeResult;

    /// The number of units already consumed for `key` (for audit/inspection), or `None`
    /// if the store cannot be read. Pure read — does NOT consume. Never used to *gate*
    /// an exercise (that is `try_consume`'s atomic job); only to report state.
    fn consumed(&self, key: &CountKey) -> Option<u64>;
}

/// The outcome of an atomic [`UsageCounterStore::try_consume`]. [OPUS-4.8] sq-zi5w.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeResult {
    /// One unit was consumed; the budget had room. Carries the new consumed count
    /// (`1..=limit`).
    Consumed(u64),
    /// The budget was already fully consumed (`consumed >= limit`) — nothing consumed.
    Exhausted,
    /// The store could not be read/written — fail-closed (treated as a DENY upstream;
    /// no unit consumed).
    Unavailable,
}

/// The default in-memory [`UsageCounterStore`]: a `Mutex<HashMap<CountKey, u64>>`.
/// [OPUS-4.8] sq-zi5w.
///
/// `try_consume` performs the whole compare-and-increment **under one lock**, so it is
/// atomic with respect to other threads sharing this store — the in-process TOCTOU race
/// is closed (see [`UsageCounterStore`]). The counts live only in this process's
/// memory: this is correct for a single-process deployment and for tests, and is the
/// reference semantics a persistent store must reproduce. It is NOT shared across
/// processes — a multi-process / distributed deployment needs a shared atomic backend
/// (deferred bead).
///
/// A poisoned lock (a prior panic while holding it) is reported as
/// [`ConsumeResult::Unavailable`] rather than propagated — fail-closed.
#[derive(Debug, Default)]
pub struct InMemoryCounterStore {
    counts: Mutex<HashMap<CountKey, u64>>,
}

impl InMemoryCounterStore {
    /// A fresh store with every budget at zero consumed.
    pub fn new() -> InMemoryCounterStore {
        InMemoryCounterStore::default()
    }
}

impl UsageCounterStore for InMemoryCounterStore {
    fn try_consume(&self, key: &CountKey, limit: u64) -> ConsumeResult {
        // The entire read-decide-write is under one lock → atomic; no other thread can
        // observe or mutate this key's count between the compare and the increment.
        let Ok(mut counts) = self.counts.lock() else {
            return ConsumeResult::Unavailable; // poisoned → fail-closed
        };
        let entry = counts.entry(key.clone()).or_insert(0);
        if *entry >= limit {
            return ConsumeResult::Exhausted;
        }
        *entry += 1;
        ConsumeResult::Consumed(*entry)
    }

    fn consumed(&self, key: &CountKey) -> Option<u64> {
        let counts = self.counts.lock().ok()?;
        Some(counts.get(key).copied().unwrap_or(0))
    }
}

/// The three-valued verdict of a rule's `odrl:count` limit(s) against the usage state —
/// the count analogue of [`crate::PurposeMatch`], reporting exactly what
/// [`evaluate_and_exercise`] would act on WITHOUT consuming a unit. [OPUS-4.8] sq-zi5w.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountStatus {
    /// The rule has no `odrl:count` constraint — count places no restriction on it.
    NotConstrained,
    /// The rule has a count limit AND budget remains (a further exercise would be
    /// granted). Carries `(consumed, limit)`.
    Satisfied { consumed: u64, limit: u64 },
    /// The rule has a count limit and it is fully consumed (`consumed >= limit`) — a
    /// further exercise is a definite DENY. Carries `(consumed, limit)`.
    DefinitelyUnsatisfied { consumed: u64, limit: u64 },
    /// The rule has a count limit but the usage state is **unavailable** (store outage)
    /// or the limit is **malformed** (a count constraint whose right-operand is not a
    /// usable non-negative integer bound). Fail-closed: a further exercise is DENIED;
    /// the limit is never silently treated as "unlimited".
    Unprovable,
}

/// Report how `rule`'s `odrl:count` limit stands against `store` for `request`, WITHOUT
/// consuming a unit — the auditable, side-effect-free surface of count enforcement (the
/// count dual of [`crate::purpose_status`]). [OPUS-4.8] sq-zi5w.
///
/// This does NOT gate an exercise (that is [`evaluate_and_exercise`]'s atomic job, which
/// *consumes*). It answers "would the next exercise have count budget?" for inspection.
///
/// **All `odrl:count` constraints on one rule share ONE usage counter** — the
/// [`CountKey`] is `(rule_id, party, target)`, *not* per-constraint, so the same
/// exercise is counted against the same budget no matter how many count constraints
/// express it. ANDing them is therefore "the **tightest** (minimum) limit governs": a
/// rule with `lteq 5` AND `lteq 2` is exhausted once 2 are consumed, and a single
/// exercise consumes exactly one unit of that one counter (never one-per-constraint).
/// A malformed or store-unavailable constraint is fail-closed (`Unprovable`).
pub fn count_status(rule: &Rule, request: &Request, store: &dyn UsageCounterStore) -> CountStatus {
    // The effective limit is the minimum over all ceiling-shaped count constraints; a
    // count constraint that yields no usable bound fails the whole rule closed.
    let Some(effective) = effective_count_limit(rule) else {
        // Either the rule carries no count constraint at all, or one was malformed.
        return if rule_has_count_constraint(rule) {
            CountStatus::Unprovable // malformed bound → fail-closed
        } else {
            CountStatus::NotConstrained
        };
    };
    // One shared counter for the rule/party/target → a single read.
    let key = CountKey::for_rule(rule, request);
    let Some(consumed) = store.consumed(&key) else {
        return CountStatus::Unprovable; // store unavailable → fail-closed
    };
    if consumed >= effective {
        CountStatus::DefinitelyUnsatisfied { consumed, limit: effective }
    } else {
        CountStatus::Satisfied { consumed, limit: effective }
    }
}

/// The result of [`evaluate_and_exercise`]: the base ODRL [`Decision`] plus, when a
/// count limit applied, what the counter store did. [OPUS-4.8] sq-zi5w.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExerciseDecision {
    /// `true` ⇒ the exercise was granted AND (if count-constrained) one unit was
    /// consumed. `false` ⇒ DENY — and **no** unit was consumed (a denied request never
    /// burns budget; fail-closed both ways).
    pub allow: bool,
    /// The granting rule on an ALLOW, else the rules implicated in the DENY (mirrors
    /// [`Decision::matched_rules`]).
    pub matched_rules: Vec<String>,
    /// Why the exercise was denied (the base decision's caveats, or a count-exhausted /
    /// count-unavailable reason). Empty on a clean ALLOW.
    pub reasons: Vec<String>,
    /// On a granted count-constrained exercise: the new consumed count (`1..=limit`).
    /// `None` when the grant was not count-constrained or the request was denied.
    pub consumed: Option<u64>,
}

impl ExerciseDecision {
    fn deny(matched: Vec<String>, reasons: Vec<String>) -> ExerciseDecision {
        ExerciseDecision { allow: false, matched_rules: matched, reasons, consumed: None }
    }
}

/// Evaluate `policy` against `request` and, on a grant, **atomically consume** one unit
/// of any applicable `odrl:count` budget from `store` — the stateful enforcement entry
/// point. [OPUS-4.8] sq-zi5w.
///
/// Semantics, wired through the **real** [`evaluate`] decision (no parallel evaluator):
///
/// 1. Run [`evaluate`]. A base DENY (no permission, a prohibition carve-out, an unmet
///    stateless constraint, an undischarged duty) is returned verbatim — **nothing is
///    consumed** (a denied request never burns count budget).
/// 2. On a base ALLOW, find the granting rule and its `odrl:count` limit(s). If it has
///    none, the grant stands as-is (no counter touched).
/// 3. If it has a count limit `N`, call [`UsageCounterStore::try_consume`] (the single
///    atomic check-and-consume):
///    - [`ConsumeResult::Consumed(n)`](ConsumeResult::Consumed) (`n <= N`) ⇒ ALLOW, one
///      unit consumed. The first `N` exercises take this branch.
///    - [`ConsumeResult::Exhausted`] (`consumed >= N`) ⇒ DENY. The `(N+1)`th exercise
///      takes this branch — the limit is reached.
///    - [`ConsumeResult::Unavailable`] ⇒ DENY (fail-closed: never grant beyond a limit
///      we cannot account for).
///
/// A *malformed* count constraint (a right-operand that is not a usable non-negative
/// integer) denies (fail-closed) — the base evaluator already rejects the request via
/// its stateless count comparison in most shapes, but this path denies regardless.
///
/// **Multiple count limits on one rule** all bind the *same* usage counter (the
/// [`CountKey`] is `(rule_id, party, target)`, never per-constraint), so several count
/// constraints are not several budgets — they are several *bounds on one budget*, and the
/// **tightest (minimum) bound governs** (`lteq 5` AND `lteq 2` ⇒ at most 2). The exercise
/// is therefore a **single** atomic [`try_consume`](UsageCounterStore::try_consume)
/// against that one effective limit: exactly one unit is consumed per exercise (never one
/// per constraint), and there is **no** read-then-consume gap — the multi-limit case is as
/// atomic as the single-limit case, closing the prior TOCTOU window (sq-ea27).
pub fn evaluate_and_exercise(
    policy: &ValidatedPolicy,
    request: &Request,
    store: &dyn UsageCounterStore,
) -> ExerciseDecision {
    // 1. The real base decision — the single source of allow/deny for everything that
    //    is NOT the stateful count. The `odrl:count` constraints are STRIPPED from the
    //    rules for this base evaluation (the stateless evaluator would otherwise deny on
    //    a missing/`Unprovable` count value); count is enforced below against the store
    //    instead. Everything else (action/target/assignee, prohibitions, purpose,
    //    dateTime, recipient, duties) is checked by the unchanged evaluator on the real
    //    policy shape.
    let stripped = match strip_count_constraints(policy).validate() {
        Ok(p) => p,
        Err(why) => return ExerciseDecision::deny(Vec::new(), vec![why]),
    };
    let decision = evaluate(&stripped, request);
    if !decision.allow {
        return ExerciseDecision::deny(decision.matched_rules, decision.unmet_constraints);
    }

    // 2. Which rule granted? `evaluate` returns its id in `matched_rules` on an ALLOW.
    //    Look it up in the ORIGINAL policy (which still carries the count constraints).
    let Some(rule) = decision
        .matched_rules
        .first()
        .and_then(|id| policy.permissions.iter().find(|r| &r.id == id))
    else {
        // ALLOW with no identifiable rule (should not happen for a permission grant) —
        // pass through unchanged; nothing to count.
        return granted(decision, None);
    };

    // 3. Reduce this rule's count constraint(s) to ONE effective limit. All count
    //    constraints on the rule bind the SAME counter (the `CountKey` is per
    //    rule/party/target, never per-constraint), so several constraints are several
    //    bounds on one budget and the tightest (minimum) governs. `None` here means
    //    either no count constraint (grant unchanged) or a malformed one (fail-closed).
    let effective_limit = match effective_count_limit(rule) {
        Some(limit) => limit,
        None if rule_has_count_constraint(rule) => {
            // A count constraint was present but malformed → fail-closed.
            return ExerciseDecision::deny(
                vec![rule.id.clone()],
                vec![format!("permission {} has a malformed odrl:count limit", rule.id)],
            );
        }
        None => return granted(decision, None),
    };

    // 4. A SINGLE atomic check-and-consume against the one effective limit — exactly one
    //    unit per exercise, no read-then-consume gap (no multi-limit TOCTOU window).
    let key = CountKey::for_rule(rule, request);
    match store.try_consume(&key, effective_limit) {
        ConsumeResult::Consumed(n) => granted(decision, Some(n)),
        ConsumeResult::Exhausted => ExerciseDecision::deny(
            vec![rule.id.clone()],
            vec![format!(
                "permission {} count limit reached (>= {effective_limit}); denied",
                rule.id
            )],
        ),
        ConsumeResult::Unavailable => ExerciseDecision::deny(
            vec![rule.id.clone()],
            vec![format!(
                "permission {} count state unavailable; denied (fail-closed)",
                rule.id
            )],
        ),
    }
}

fn granted(decision: Decision, consumed: Option<u64>) -> ExerciseDecision {
    ExerciseDecision {
        allow: true,
        matched_rules: decision.matched_rules,
        reasons: Vec::new(),
        consumed,
    }
}

/// The integer count bound `N` of an `odrl:count` constraint under a "≤"-shaped
/// operator, or `None` if the constraint does not express a usable upper bound.
///
/// A count *limit* is "at most N times": `lteq N` (≤ N) and `lt N` (< N, i.e. at most
/// `N-1`) express upper bounds and are honoured. `eq N` is treated as "exactly/at most
/// N" (the common shorthand "count = 5" for a 5-use allowance). Order operators that do
/// NOT bound from above (`gt`/`gteq`) and equality-set operators (`neq`/`isPartOf`/
/// `isA`/`isAnyOf`/`isNoneOf`) do not express a usage *ceiling* and return `None` (the
/// stateful path then treats them as having no limit — they are left to the stateless
/// evaluator). A negative or non-integer right-operand is rejected (`None`).
fn count_limit(op: Operator, right: &Value) -> Option<u64> {
    let n = match right {
        Value::Num(x) => *x,
        // a numeric-looking string bound
        Value::Str(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
        return None;
    }
    let n = n as u64;
    match op {
        // "at most N" allowances.
        Operator::Lteq | Operator::Eq => Some(n),
        // "< N" ⇒ at most N-1 (a 0 bound stays 0 — never exercisable).
        Operator::Lt => Some(n.saturating_sub(1)),
        // gt/gteq/neq/isPartOf/isA/isAnyOf/isNoneOf do not express a usage ceiling.
        _ => None,
    }
}

/// A copy of `policy` with every `odrl:count` constraint removed from its
/// **permission** rules, so the stateless [`evaluate`] does not deny a count-limited
/// permission for lack of a request-supplied count value — the count is enforced
/// separately against the [`UsageCounterStore`]. Prohibitions are left untouched: a
/// prohibition is a carve-out, never a permission grant, so a count constraint there
/// keeps its (stateless) meaning and is not part of usage-counter enforcement.
fn strip_count_constraints(policy: &Policy) -> Policy {
    let mut out = policy.clone();
    for rule in &mut out.permissions {
        rule.constraints.retain(|c| c.left != ODRL_COUNT);
    }
    out
}

fn rule_has_count_constraint(rule: &Rule) -> bool {
    rule.constraints.iter().any(|c| c.left == ODRL_COUNT)
}

/// The ONE effective usage-counter limit for `rule`: the **minimum** over every
/// ceiling-shaped `odrl:count` constraint, because all of them bind the same counter
/// (the [`CountKey`] is per rule/party/target, not per-constraint), so several count
/// constraints are several bounds on one budget and the tightest governs. [OPUS-4.8]
/// sq-ea27.
///
/// Returns `None` when the rule yields no enforceable count ceiling. Callers distinguish
/// the two `None` cases via [`rule_has_count_constraint`]:
/// - no `odrl:count` constraint at all (the stateful path leaves the rule alone), and
/// - a count constraint that does NOT express an upper bound (`gt`/`gteq`/`neq`/… —
///   left to the stateless evaluator) **or is malformed** (a non-integer/negative
///   right-operand) — fail-closed by the callers when a count constraint is present but
///   yields no usable bound.
fn effective_count_limit(rule: &Rule) -> Option<u64> {
    rule.constraints
        .iter()
        .filter(|c| c.left == ODRL_COUNT)
        .filter_map(|c| count_limit(c.operator, &c.right))
        .min()
}
