//! A **distributed** [`UsageCounterStore`] over any backend that can do one atomic
//! conditional-increment — the Redis-`INCR` / SQL-`UPDATE…WHERE consumed<limit RETURNING`
//! seam. sq-u3yo.
//!
//! [`crate::count::InMemoryCounterStore`] shares a budget within one process; [`crate::
//! count_file::FileCounterStore`] (sq-5z1q) shares it across processes on one host (or a
//! sane NFS mount). The bead's headline backends — **Redis** and **SQL** — share a budget
//! across *hosts*, and the [`UsageCounterStore`] trait was written for exactly that drop-in
//! (its [`try_consume`](UsageCounterStore::try_consume) contract says a distributed store
//! "MUST provide the same atomicity … across the processes that share the limit"). This
//! module supplies the piece that makes such a backend **trivially correct to wire**.
//!
//! # Why a trait + adapter, not a bundled Redis/SQL client
//!
//! `sparq-policy` is a `publish = false`, **std-only, dependency-of-nothing** crate; the
//! `count-enforcement` feature "adds ONLY the `count` module … no new deps". Bundling a
//! `redis` / `tokio-postgres` / `rusqlite` client (and the running service each needs to
//! test) would break that invariant and could not be gated in CI, which has no such
//! service. So this crate ships the **seam**, not the connection:
//!
//! - [`AtomicCounterBackend`] — the ONE primitive an operator implements: an atomic
//!   "if `consumed < limit` then `consumed += 1` and return the new value, else report
//!   exhausted". That single operation **is** a Redis `INCR` behind a Lua cap (or
//!   `WATCH`/`MULTI`/`EXEC`), and **is** a SQL `UPDATE … SET consumed = consumed + 1 WHERE
//!   key = ? AND consumed < ? RETURNING consumed`. See the [trait docs](AtomicCounterBackend)
//!   for the verbatim statements.
//! - [`BackendCounterStore`] — the adapter that turns any `AtomicCounterBackend` into a
//!   full [`UsageCounterStore`]: it maps a [`CountKey`] to a single collision-free backend
//!   key, calls the backend's atomic op once, and translates the outcome into the
//!   trait's [`ConsumeResult`] (including the fail-closed mapping of a backend error to
//!   [`ConsumeResult::Unavailable`]). This is the only logic that lives in *this* crate;
//!   it is fully tested here against an in-crate reference backend that reproduces the
//!   exact atomic semantics of Redis/SQL, so the adapter is proven without a live service.
//!
//! An operator therefore writes ~one method (`checked_increment` against their DB) and
//! gets a correct, fail-closed [`UsageCounterStore`] for free — no re-deriving the
//! check-and-consume contract, no TOCTOU footgun (the adapter never reads-then-writes;
//! the whole decision is the backend's single atomic call).
//!
//! # Atomicity contract (load-bearing — read this)
//!
//! The adapter pushes the **entire** compare-and-increment into ONE backend round-trip
//! ([`AtomicCounterBackend::checked_increment`]). It does NOT read the count, decide, then
//! increment — that would reintroduce the cross-host TOCTOU race [`evaluate_and_exercise`]
//! exists to avoid. The backend MUST make that single op atomic across all hosts sharing
//! the budget (Redis is single-threaded per key; the SQL `UPDATE…WHERE` is one atomic
//! statement). A backend that cannot guarantee that does NOT satisfy the contract.
//!
//! # Fail-closed
//!
//! Every backend error ([`BackendError`]) maps to [`ConsumeResult::Unavailable`] — a DENY
//! upstream, never a silent grant. A read error makes `BackendCounterStore::consumed`
//! return `None`. We never grant beyond a limit we cannot account for.
//!
//! [`evaluate_and_exercise`]: crate::count::evaluate_and_exercise

use crate::count::{ConsumeResult, CountKey, UsageCounterStore};

/// The single atomic primitive a distributed counter backend must provide — the seam an
/// operator drops Redis / SQL into. sq-u3yo.
///
/// # The one method ([`checked_increment`](AtomicCounterBackend::checked_increment))
///
/// "Atomically: if the budget at `key` has consumed fewer than `limit` units, consume one
/// and return the new consumed count; else consume nothing and report exhausted." This is
/// deliberately a *single* op, not a read + a write — the whole decision is indivisible, so
/// no cross-host TOCTOU window can open between the check and the increment.
///
/// ## It IS a Redis `INCR`-with-cap
///
/// ```text
/// -- KEYS[1] = the count key, ARGV[1] = limit. Atomic (Redis runs one Lua script
/// -- to completion, single-threaded per key):
/// local c = tonumber(redis.call('GET', KEYS[1]) or '0')
/// if c >= tonumber(ARGV[1]) then return -1 end        -- Exhausted
/// return redis.call('INCR', KEYS[1])                  -- Incremented(new)
/// ```
///
/// ## It IS a SQL `UPDATE … WHERE consumed < limit RETURNING`
///
/// ```text
/// UPDATE odrl_counts
///    SET consumed = consumed + 1
///  WHERE key = $1 AND consumed < $2
/// RETURNING consumed;                                 -- one row => Incremented; zero rows => Exhausted
/// -- (the row is INSERTed with consumed = 0 ON CONFLICT DO NOTHING first, or an
/// --  INSERT … ON CONFLICT (key) DO UPDATE … WHERE consumed < $2 done as one statement.)
/// ```
///
/// Both are atomic at the storage layer, so the adapter inherits that atomicity unchanged.
///
/// # Errors
///
/// A connection drop / server error / lock-acquire failure is a [`BackendError`]; the
/// adapter maps it to [`ConsumeResult::Unavailable`] (fail-closed). An implementation MUST
/// return `Err` (never a fabricated `Incremented`) when it cannot prove the increment
/// durably happened.
pub trait AtomicCounterBackend {
    /// Atomically increment the budget at `key` iff it is below `limit`; see the trait
    /// docs for the verbatim Redis / SQL statements this corresponds to.
    ///
    /// - `Ok(BackendOutcome::Incremented(n))` — one unit consumed; `n` is the **new**
    ///   consumed count (`1..=limit`).
    /// - `Ok(BackendOutcome::Exhausted)` — already at/over `limit`; nothing consumed.
    /// - `Err(BackendError)` — could not be performed (connection / server / lock error);
    ///   the adapter fails closed.
    fn checked_increment(&self, key: &str, limit: u64) -> Result<BackendOutcome, BackendError>;

    /// Read the current consumed count at `key` (`0` if absent). Pure read — never gates an
    /// exercise (that is [`checked_increment`](AtomicCounterBackend::checked_increment)'s
    /// atomic job); only for audit/inspection. A backend error returns `Err` and the
    /// adapter surfaces it as `None` from [`BackendCounterStore::consumed`].
    fn read(&self, key: &str) -> Result<u64, BackendError>;
}

/// The outcome of an [`AtomicCounterBackend::checked_increment`]. sq-u3yo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendOutcome {
    /// One unit was consumed; carries the new consumed count (`1..=limit`).
    Incremented(u64),
    /// The budget was already at/over the limit — nothing consumed.
    Exhausted,
}

/// An opaque backend error (connection drop, server error, lock timeout) — carries a human
/// message for logging; its only semantic effect is fail-closed ([`ConsumeResult::
/// Unavailable`] / `None`). sq-u3yo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    message: String,
}

impl BackendError {
    /// A backend error carrying `message` (for logs/inspection).
    pub fn new(message: impl Into<String>) -> BackendError {
        BackendError {
            message: message.into(),
        }
    }

    /// The error message (for logging).
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "counter backend error: {}", self.message)
    }
}

impl std::error::Error for BackendError {}

/// A [`UsageCounterStore`] over any [`AtomicCounterBackend`] — the drop-in adapter for a
/// Redis / SQL distributed budget. sq-u3yo.
///
/// See the module docs for the whole rationale. The adapter is intentionally tiny: it
/// encodes the [`CountKey`] into one collision-free backend key, runs the backend's single
/// atomic op, and translates the result into the [`UsageCounterStore`] contract. All the
/// distributed atomicity lives in the backend; all the *correctness of the wiring* is here
/// and is tested against an in-crate reference backend.
///
/// ```rust,ignore
/// // cargo: sparq-policy with --features count-enforcement
/// use sparq_policy::{evaluate_and_exercise, BackendCounterStore, Request};
/// // `MyRedisBackend` implements AtomicCounterBackend with one Lua INCR-with-cap.
/// let store = BackendCounterStore::new(MyRedisBackend::connect("redis://...")?);
/// let req = Request::new("http://www.w3.org/ns/odrl/2/read")
///     .on("urn:asset/x").by("https://alice.ex/me"); // policy: count lteq 3
/// let _ = evaluate_and_exercise(&policy, &req, &store);
/// ```
#[derive(Debug, Clone)]
pub struct BackendCounterStore<B> {
    backend: B,
}

impl<B: AtomicCounterBackend> BackendCounterStore<B> {
    /// Wrap `backend` in the [`UsageCounterStore`] adapter.
    pub fn new(backend: B) -> BackendCounterStore<B> {
        BackendCounterStore { backend }
    }

    /// Borrow the wrapped backend (e.g. for backend-specific admin/teardown).
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: AtomicCounterBackend> UsageCounterStore for BackendCounterStore<B> {
    fn try_consume(&self, key: &CountKey, limit: u64) -> ConsumeResult {
        match self.backend.checked_increment(&encode_key(key), limit) {
            Ok(BackendOutcome::Incremented(n)) => ConsumeResult::Consumed(n),
            Ok(BackendOutcome::Exhausted) => ConsumeResult::Exhausted,
            Err(_) => ConsumeResult::Unavailable, // fail-closed
        }
    }

    fn consumed(&self, key: &CountKey) -> Option<u64> {
        self.backend.read(&encode_key(key)).ok()
    }
}

/// Encode a [`CountKey`] into a single, **injective** backend key (a usable Redis key / SQL
/// primary-key string). The three fields are percent-escaped on the framing-significant
/// bytes (`%` and the `\u{1f}` field separator) and joined with `\u{1f}` (ASCII Unit
/// Separator), so two distinct `CountKey`s can never collapse to the same string — the
/// property a backend primary key relies on to keep distinct budgets in distinct rows.
///
/// The escaping mirrors [`crate::count_file`]'s TSV escaping in spirit (escape `%` first so
/// the encoding is unambiguous), but uses the Unit Separator rather than TAB so the encoded
/// key is a single token free of whitespace — friendlier as a Redis key / SQL string.
fn encode_key(key: &CountKey) -> String {
    let mut out = String::with_capacity(key.rule_id.len() + key.party.len() + key.target.len() + 2);
    escape_into(&key.rule_id, &mut out);
    out.push('\u{1f}');
    escape_into(&key.party, &mut out);
    out.push('\u{1f}');
    escape_into(&key.target, &mut out);
    out
}

/// Percent-escape `%` and the `\u{1f}` separator into `out` so a field is a single token
/// that cannot forge a field boundary. `%` first keeps the escape unambiguous.
fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\u{1f}' => out.push_str("%1F"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(rule: &str, party: &str, target: &str) -> CountKey {
        CountKey {
            rule_id: rule.into(),
            party: party.into(),
            target: target.into(),
        }
    }

    #[test]
    fn encode_key_is_injective_across_field_boundaries() {
        // The separator byte appearing literally in a field is escaped, so it cannot forge
        // a boundary: ("a\u{1f}b","","") and ("a","b","") must encode differently.
        let a = encode_key(&key("a\u{1f}b", "", ""));
        let b = encode_key(&key("a", "b", ""));
        assert_ne!(a, b);
        // A `%` in a field is escaped so it cannot impersonate an escape sequence.
        let c = encode_key(&key("a%1Fb", "", ""));
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn backend_error_displays() {
        let e = BackendError::new("boom");
        assert_eq!(e.message(), "boom");
        assert!(e.to_string().contains("boom"));
    }
}
