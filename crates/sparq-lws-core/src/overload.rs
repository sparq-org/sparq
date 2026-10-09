// AUTHORED-BY Claude Opus 4.8
//! Overload protection — admission control (load shedding) + observability counters.
//!
//! ## Why (the availability problem)
//! Under saturation a server that keeps accepting work past its capacity does not get *slower*, it
//! *collapses*: queues grow without bound, latency tends to infinity, every request times out, and a
//! transient spike turns into a sustained outage (the "fail-closed cliff"). The fix is **admission
//! control**: when in-flight work reaches a configured ceiling, SHED excess load immediately with a
//! **503 + a jittered `Retry-After`** rather than queueing it. A shed request is cheap (it never
//! touches storage or crypto), so the server stays responsive for the requests it DID admit and
//! recovers the instant the spike passes. This is graceful degradation, not graceful death.
//!
//! ## How
//! A single [`AdmissionControl`] holds a [`tokio::sync::Semaphore`] with `max_concurrency` permits.
//! The [`admission_middleware`] tries to acquire a permit **without blocking**
//! ([`Semaphore::try_acquire_owned`]): success ⇒ the request is admitted and the permit is held for
//! its whole lifetime (dropped — released — when the response is produced); failure (no permit
//! available) ⇒ the request is **shed** with 503 + `Retry-After`, never run. `try_acquire` (not the
//! awaiting `acquire`) is what makes this LOAD-SHEDDING rather than a bounded queue: excess load is
//! rejected fast, it does not pile up waiting for a permit.
//!
//! ## Where it sits (the security-critical ordering)
//! The admission layer is the **OUTERMOST** application layer — it runs BEFORE auth, BEFORE WAC, and
//! before any storage/crypto work (it is admission control / a cheap front door). This is correct AND
//! a security property:
//! - **Shedding can never bypass auth/WAC.** A shed request returns 503 and is NEVER forwarded to the
//!   inner stack — it cannot turn an unauthorized request into a success. A 503 is strictly LESS
//!   access than the request would otherwise have gotten; it is fail-safe by construction.
//! - **Shedding happens before the expensive DPoP crypto** (admission control), so an overload spike
//!   does not spend CPU verifying tokens for requests it is about to reject.
//! - **Health/readiness endpoints are EXEMPT** — they are mounted OUTSIDE this layer (see
//!   [`crate::app::build_router`]), so a load balancer's readiness probe is never shed (which would
//!   make the LB pull a still-healthy instance, amplifying an overload into an outage).
//!
//! ## Observability
//! [`AdmissionMetrics`] tracks an **in-flight gauge** (current admitted requests) and a monotonic
//! **shed counter**; the middleware logs (at WARN) the first shed and then periodically, with the
//! current in-flight + total shed counts, so a saturation event is visible in the logs without a
//! metrics backend. The gauge/counter are plain atomics readable by a future `/metrics` exporter.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Env var: the maximum number of concurrently in-flight requests admitted before load is shed.
/// Excess requests get 503 + `Retry-After`. Unset / invalid / `0` ⇒ [`DEFAULT_MAX_CONCURRENCY`].
pub const ENV_MAX_CONCURRENCY: &str = "SOLID_SERVER_MAX_CONCURRENCY";

/// The default global concurrency ceiling. Chosen DELIBERATELY HIGH so it never trips during normal
/// use OR the conformance run (which is sequential-ish and nowhere near this) — it is a safety bound
/// against pathological overload, not a throughput throttle. An operator tunes it to their box +
/// upstream (SPARQ/S3) capacity. `tokio::sync::Semaphore` caps permits at `MAX_PERMITS`
/// (`usize::MAX >> 3`), well above this.
pub const DEFAULT_MAX_CONCURRENCY: usize = 10_000;

/// The base `Retry-After` (seconds) returned on a shed 503, before jitter. Small on purpose: an
/// overload spike usually clears in well under a second of admitted-request drain, so a short retry
/// keeps a well-behaved client responsive while the jitter (below) prevents a synchronized retry
/// stampede ("thundering herd") from every shed client retrying on the same tick.
pub const RETRY_AFTER_BASE_SECS: u64 = 1;
/// The maximum extra jitter (seconds) added on top of [`RETRY_AFTER_BASE_SECS`]. The returned value
/// is `base + rand(0..=JITTER)`, so shed clients spread their retries over `[base, base+jitter]`.
pub const RETRY_AFTER_JITTER_SECS: u64 = 4;

/// Resolve the configured max-concurrency from the env value.
/// - absent / empty / non-numeric / `0` ⇒ [`DEFAULT_MAX_CONCURRENCY`] (a `0` would shed EVERYTHING —
///   never silently brick the server on a typo; an explicit positive number is required to change it).
/// - `>0` ⇒ that literal ceiling.
pub fn max_concurrency_from_env() -> usize {
    parse_max_concurrency(std::env::var(ENV_MAX_CONCURRENCY).ok())
}

/// Testable core of [`max_concurrency_from_env`]. See that fn for the rules.
pub fn parse_max_concurrency(raw: Option<String>) -> usize {
    match raw.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_MAX_CONCURRENCY,
        Some(s) => match s.parse::<usize>() {
            Ok(0) | Err(_) => DEFAULT_MAX_CONCURRENCY,
            Ok(n) => n,
        },
    }
}

/// Observability counters for admission control: an in-flight gauge + a monotonic shed counter.
/// Plain atomics so a future `/metrics` exporter can read them lock-free.
#[derive(Debug, Default)]
pub struct AdmissionMetrics {
    /// Current number of admitted, in-flight requests (the gauge).
    in_flight: AtomicUsize,
    /// Monotonic total number of requests shed (503'd) since boot (the counter).
    shed_total: AtomicU64,
}

impl AdmissionMetrics {
    /// Current in-flight (admitted) request count.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }
    /// Total requests shed (503'd) since boot.
    pub fn shed_total(&self) -> u64 {
        self.shed_total.load(Ordering::Relaxed)
    }
}

/// The shared admission-control state: a permit pool sized to the concurrency ceiling + the metrics.
#[derive(Clone)]
pub struct AdmissionControl {
    semaphore: Arc<Semaphore>,
    metrics: Arc<AdmissionMetrics>,
    max_concurrency: usize,
}

impl AdmissionControl {
    /// Build admission control with `max_concurrency` permits. `max_concurrency` is clamped to at
    /// least 1 (a 0-permit pool would shed every request — the env parser already rejects 0, but this
    /// makes the type itself safe to construct directly, e.g. in tests).
    pub fn new(max_concurrency: usize) -> Self {
        let permits = max_concurrency.max(1);
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            metrics: Arc::new(AdmissionMetrics::default()),
            max_concurrency: permits,
        }
    }

    /// The configured concurrency ceiling (after the >=1 clamp).
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// The metrics handle (in-flight gauge + shed counter), e.g. for a `/metrics` exporter.
    pub fn metrics(&self) -> Arc<AdmissionMetrics> {
        self.metrics.clone()
    }

    /// Try to admit a request: acquire a permit WITHOUT blocking. `Some(permit)` ⇒ admitted (hold the
    /// permit for the request lifetime — dropping it releases the slot); `None` ⇒ at capacity, shed.
    fn try_admit(&self) -> Option<OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }

    /// Acquire and HOLD a permit, modelling an in-flight request — for tests that need to drive the
    /// admission layer to capacity deterministically (hold N permits, then assert the next request is
    /// shed). Holding the returned permit occupies a slot until it is dropped. This shares the same
    /// `Arc<Semaphore>` as the live middleware (a `clone()` of `AdmissionControl` does too), so a
    /// permit held here makes the middleware observe "at capacity" and shed.
    #[doc(hidden)]
    pub fn try_admit_for_test(&self) -> Option<OwnedSemaphorePermit> {
        self.try_admit()
    }
}

/// Compute the jittered `Retry-After` (seconds) for a shed response: `base + rand(0..=jitter)`. The
/// jitter spreads shed clients' retries so they do not all retry on the same tick (a thundering-herd
/// guard). Uses the OS RNG (`getrandom`) — the same minimal entropy source the blob-key suffix uses;
/// a jitter value needs no cryptographic strength, but reusing the one OS source keeps the surface
/// small. On the (vanishingly unlikely) `getrandom` failure, fall back to no jitter (just `base`) —
/// the 503 is still correct, only the spread is lost.
fn jittered_retry_after_secs() -> u64 {
    let mut buf = [0u8; 8];
    let jitter = if getrandom::getrandom(&mut buf).is_ok() {
        u64::from_le_bytes(buf) % (RETRY_AFTER_JITTER_SECS + 1)
    } else {
        0
    };
    RETRY_AFTER_BASE_SECS + jitter
}

/// Build the 503 shed response: `503 Service Unavailable` + `Retry-After: <jittered seconds>` + a
/// short plain-text body. This is a FAIL-SAFE response — it grants strictly less than the request
/// would otherwise have gotten, so it can never be an authorization bypass.
fn shed_response() -> Response {
    let retry_after = jittered_retry_after_secs();
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [
            (header::RETRY_AFTER, retry_after.to_string()),
            // Make it explicit + cache-safe that this is a transient overload, not a resource state.
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        "503 Service Unavailable: server is shedding load (overloaded). Retry after the indicated \
         delay.\n",
    )
        .into_response()
}

/// The admission-control middleware. The OUTERMOST application layer (runs before auth/WAC). At
/// capacity it sheds (503 + jittered `Retry-After`) WITHOUT running the inner stack; otherwise it
/// admits, holds a permit for the request lifetime, and updates the in-flight gauge.
///
/// `State` is the shared [`AdmissionControl`]. The held [`OwnedSemaphorePermit`] is moved into the
/// async body and dropped when it returns — so the slot is released on EVERY exit path (success,
/// error, panic-unwind of the future), never leaked.
pub async fn admission_middleware(
    State(admission): State<AdmissionControl>,
    req: Request,
    next: Next,
) -> Response {
    let permit = match admission.try_admit() {
        Some(p) => p,
        None => {
            // Shed: count it, log it (rate-limited), and 503 — the inner stack is NOT run.
            let shed = admission.metrics.shed_total.fetch_add(1, Ordering::Relaxed) + 1;
            let in_flight = admission.metrics.in_flight();
            // Log the first shed and then every 100th, so a saturation event is visible without
            // flooding the log on a sustained overload.
            if shed == 1 || shed % 100 == 0 {
                eprintln!(
                    "  OVERLOAD: shedding load — returned 503 (shed_total={shed}, in_flight={in_flight}, \
                     max_concurrency={}). Excess requests get 503 + Retry-After.",
                    admission.max_concurrency
                );
            }
            return shed_response();
        }
    };

    // Admitted: bump the gauge for the request lifetime. An RAII guard decrements on EVERY exit path
    // (including an early-return / panic-unwind inside the inner stack), so the gauge can never drift.
    admission.metrics.in_flight.fetch_add(1, Ordering::Relaxed);
    let _gauge = InFlightGuard(admission.metrics.clone());

    // The permit is shared with the request: work a handler detaches from the request (a write that
    // must finish once started) clones the [`AdmissionSlot`] and holds it until it ends, so the slot
    // stays taken while that work runs even after the request itself has timed out or gone away.
    let mut req = req;
    let slot = AdmissionSlot {
        _permit: Arc::new(permit),
    };
    req.extensions_mut().insert(slot.clone());
    let response = next.run(req).await;
    // This hold and the gauge guard drop here (after the response is produced); the slot is released
    // once every detached holder has finished too.
    drop(slot);
    response
}

/// A share of an admitted request's admission permit. The slot is released when the last share is
/// dropped, so work detached from a request keeps counting against the concurrency ceiling until
/// it finishes.
#[derive(Clone)]
pub struct AdmissionSlot {
    _permit: Arc<OwnedSemaphorePermit>,
}

/// RAII guard that decrements the in-flight gauge on drop — so an admitted request always
/// decrements, on every exit path, even if the inner future is cancelled or panics.
struct InFlightGuard(Arc<AdmissionMetrics>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

// --- Request timeout ------------------------------------------------------------------------------

/// Env var: the per-request timeout in seconds. A request that runs longer is aborted with a **504**
/// (so a stuck request — e.g. a wedged backend round-trip — cannot pin a worker/permit forever).
/// Unset / invalid ⇒ [`DEFAULT_REQUEST_TIMEOUT_SECS`]; `0` ⇒ NO timeout (disabled).
pub const ENV_REQUEST_TIMEOUT_SECS: &str = "SOLID_SERVER_REQUEST_TIMEOUT_SECS";

/// The default per-request timeout (seconds). Generous enough for a normal LDP request including a
/// backend SPARQ round-trip + RDF render, short enough to reclaim a genuinely stuck request. An
/// operator lowers it for a tighter SLO or raises it for large uploads.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Resolve the per-request timeout from the env value.
/// - absent / empty / non-numeric ⇒ `Some(DEFAULT_REQUEST_TIMEOUT_SECS)` (timeout ENABLED, default);
/// - `0` ⇒ `None` (timeout explicitly DISABLED);
/// - `>0` ⇒ `Some(n)` (that timeout).
pub fn request_timeout_from_env() -> Option<Duration> {
    parse_request_timeout(std::env::var(ENV_REQUEST_TIMEOUT_SECS).ok())
}

/// Testable core of [`request_timeout_from_env`]. See that fn for the rules.
pub fn parse_request_timeout(raw: Option<String>) -> Option<Duration> {
    let secs = match raw.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_REQUEST_TIMEOUT_SECS,
        Some(s) => match s.parse::<u64>() {
            Ok(0) => return None, // explicit disable
            Ok(n) => n,
            Err(_) => DEFAULT_REQUEST_TIMEOUT_SECS,
        },
    };
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_concurrency_default_on_absent_empty_invalid_or_zero() {
        // Absent / empty / non-numeric / 0 ⇒ the default (a 0 would shed EVERYTHING — never silently
        // brick the server on a typo).
        assert_eq!(parse_max_concurrency(None), DEFAULT_MAX_CONCURRENCY);
        assert_eq!(
            parse_max_concurrency(Some("".into())),
            DEFAULT_MAX_CONCURRENCY
        );
        assert_eq!(
            parse_max_concurrency(Some("  ".into())),
            DEFAULT_MAX_CONCURRENCY
        );
        assert_eq!(
            parse_max_concurrency(Some("abc".into())),
            DEFAULT_MAX_CONCURRENCY
        );
        assert_eq!(
            parse_max_concurrency(Some("0".into())),
            DEFAULT_MAX_CONCURRENCY
        );
        assert_eq!(
            parse_max_concurrency(Some("-5".into())),
            DEFAULT_MAX_CONCURRENCY
        );
    }

    #[test]
    fn max_concurrency_explicit_positive_is_honoured() {
        assert_eq!(parse_max_concurrency(Some("1".into())), 1);
        assert_eq!(parse_max_concurrency(Some("256".into())), 256);
        assert_eq!(parse_max_concurrency(Some("  4096  ".into())), 4096);
    }

    #[test]
    fn request_timeout_rules() {
        // default when absent/empty/invalid; disabled on 0; honoured on >0.
        assert_eq!(
            parse_request_timeout(None),
            Some(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        );
        assert_eq!(
            parse_request_timeout(Some("".into())),
            Some(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        );
        assert_eq!(
            parse_request_timeout(Some("garbage".into())),
            Some(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        );
        assert_eq!(parse_request_timeout(Some("0".into())), None);
        assert_eq!(
            parse_request_timeout(Some("5".into())),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn admission_clamps_to_at_least_one_permit() {
        let ac = AdmissionControl::new(0);
        assert_eq!(ac.max_concurrency(), 1, "0 permits would shed everything");
        // The single permit is acquirable.
        let p = ac.try_admit().expect("first admit succeeds");
        assert!(ac.try_admit().is_none(), "second admit sheds at capacity 1");
        drop(p);
        assert!(ac.try_admit().is_some(), "slot released after drop");
    }

    #[test]
    fn admission_sheds_at_capacity_then_recovers() {
        let ac = AdmissionControl::new(2);
        let p1 = ac.try_admit().expect("admit 1");
        let p2 = ac.try_admit().expect("admit 2");
        assert!(ac.try_admit().is_none(), "at capacity ⇒ shed");
        drop(p1);
        let p3 = ac.try_admit().expect("a freed slot re-admits");
        drop(p2);
        drop(p3);
        // Both freed ⇒ two slots available again.
        let _a = ac.try_admit().expect("recovered slot 1");
        let _b = ac.try_admit().expect("recovered slot 2");
    }

    #[test]
    fn jittered_retry_after_within_bounds() {
        // Always within [base, base+jitter] — the spread guard never produces an out-of-band value.
        for _ in 0..1000 {
            let v = jittered_retry_after_secs();
            assert!(
                (RETRY_AFTER_BASE_SECS..=RETRY_AFTER_BASE_SECS + RETRY_AFTER_JITTER_SECS)
                    .contains(&v),
                "retry-after {v} out of [{RETRY_AFTER_BASE_SECS}, {}]",
                RETRY_AFTER_BASE_SECS + RETRY_AFTER_JITTER_SECS
            );
        }
    }

    #[test]
    fn metrics_track_in_flight_via_guard() {
        let metrics = Arc::new(AdmissionMetrics::default());
        assert_eq!(metrics.in_flight(), 0);
        metrics.in_flight.fetch_add(1, Ordering::Relaxed);
        let g = InFlightGuard(metrics.clone());
        assert_eq!(metrics.in_flight(), 1);
        drop(g);
        assert_eq!(metrics.in_flight(), 0, "guard decrements on drop");
        assert_eq!(metrics.shed_total(), 0);
    }
}
