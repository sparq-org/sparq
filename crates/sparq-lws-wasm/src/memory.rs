// sq-wubkf: bounded linear-memory accounting + per-request admission control.
//! Bounded linear-memory accounting for the wasm pod.
//!
//! # The failure this removes
//!
//! On `wasm32` the whole pod lives in one linear memory. When the heap needs to grow past the
//! host's ceiling, `memory.grow` returns `-1`, the allocator hands back a null pointer, and Rust's
//! allocation-error handler aborts — which under the release profile's `panic=abort` lowers to an
//! `unreachable` trap. The host sees a `WebAssembly.RuntimeError`, the instance is poisoned, and
//! the request that tripped it gets no HTTP answer at all (the Node host recycles the instance and
//! synthesises a 503; see `packages/solid-server/src/trap-recovery.js`).
//!
//! [`BoundedAlloc`] plus [`admits_request`] turn the common, *sustained*-pressure form of that
//! failure into a clean HTTP 507 instead: the allocator keeps a running total of live heap bytes,
//! and a request whose projected peak footprint would cross the configured ceiling is refused
//! before it ever reaches the router.
//!
//! # Why the allocator accounts instead of failing allocations
//!
//! A "bounded" allocator that returns null once a ceiling is crossed does **not** help here.
//! [`std::alloc::GlobalAlloc::alloc`] returning null routes straight into `handle_alloc_error`,
//! which aborts — i.e. it produces exactly the trap this module exists to remove, only sooner.
//! Refusing allocations is therefore strictly worse than not refusing them. The bound is instead
//! enforced one layer up, at the request boundary, which is the only place a refusal can still be
//! expressed as an HTTP status.
//!
//! # What this does and does not catch
//!
//! It catches *sustained* pressure: a pod whose retained bytes have crept up to the ceiling refuses
//! further writes with 507. Because the accounting is of **live** bytes rather than of pages ever
//! grown, bytes returned to the allocator lower the total again and restore headroom — something a
//! pages-grown high-water mark could never do.
//!
//! That is a property of the *counters*, and it is all the tests below pin. Whether a given LDP
//! `DELETE` returns **enough** bytes to move a refused request back under the ceiling is **not**
//! measured: nothing here instruments the store, which keeps its map capacity after removing an
//! entry, and the wasm integration test restores the ceiling rather than deleting data. Recovery
//! from sustained pressure is therefore the accounting's intent, not a demonstrated property of
//! the delete path; establishing it needs a wasm32 end-to-end test that deletes a stored resource
//! and observes admission return under an unchanged ceiling.
//!
//! It does **not** catch a single request whose own transient peak overshoots the remaining
//! headroom by more than [`projected_request_bytes`] predicts; only fallible allocation throughout
//! the router could do that, which is not available on stable Rust. Such a request still traps and
//! is still handled by the host's trap-recovery path.
//!
//! # Host-testability
//!
//! Everything except the one `#[global_allocator]` registration is target-independent and compiled
//! on the host, so the arithmetic and the ceiling policy are covered by ordinary `cargo test`. The
//! crate's wasm-only glue merely reads these values.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Default linear-memory ceiling: 3 GiB.
///
/// `wasm32` linear memory tops out just under the 4 GiB the 32-bit address space allows, and an
/// engine typically refuses to grow well before that. Refusing at 3 GiB converts the last mile —
/// where growth is near-certain to fail — into a 507 while leaving every realistic pod (the
/// in-memory store's own blob quota is 64 MiB) completely unaffected.
pub const DEFAULT_CEILING_BYTES: usize = 3 * 1024 * 1024 * 1024;

/// Multiplier applied to a request body when projecting its peak footprint.
///
/// A written body is buffered on the wasm boundary, again inside the axum `Body`, again by the RDF
/// parser, and again by the store write, so four copies is the conservative estimate.
const BODY_FOOTPRINT_FACTOR: u64 = 4;

/// Fixed headroom for the router, header, and response allocations that do not scale with the body.
const REQUEST_HEADROOM_BYTES: u64 = 1 << 20;

/// Live-byte accounting for one allocator.
///
/// Kept as a value type rather than as bare statics so the arithmetic is testable against a fresh
/// instance instead of against whatever the test process happens to have allocated.
pub struct Accounting {
    live: AtomicUsize,
    peak: AtomicUsize,
}

impl Accounting {
    /// A zeroed counter pair.
    pub const fn new() -> Self {
        Self {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// Record `size` bytes handed out, updating the high-water mark.
    pub fn record_alloc(&self, size: usize) {
        let live = self
            .live
            .fetch_add(size, Ordering::Relaxed)
            .saturating_add(size);
        self.peak.fetch_max(live, Ordering::Relaxed);
    }

    /// Record `size` bytes returned to the allocator.
    pub fn record_dealloc(&self, size: usize) {
        self.live.fetch_sub(size, Ordering::Relaxed);
    }

    /// Bytes currently held by live allocations.
    pub fn live(&self) -> u64 {
        self.live.load(Ordering::Relaxed) as u64
    }

    /// The largest [`Accounting::live`] value observed so far.
    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed) as u64
    }
}

impl Default for Accounting {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide counters [`BoundedAlloc`] maintains.
static ACCOUNTING: Accounting = Accounting::new();

/// The configured ceiling; `0` means unbounded.
static CEILING_BYTES: AtomicUsize = AtomicUsize::new(DEFAULT_CEILING_BYTES);

/// A `System`-delegating global allocator that tracks live heap bytes.
///
/// Every method forwards to [`System`] verbatim and only adds `Relaxed` counter updates; it never
/// refuses an allocation (see the module documentation for why refusing would be counterproductive).
/// `alloc_zeroed` is deliberately left as the trait default, which routes through
/// [`BoundedAlloc::alloc`] and is therefore accounted too.
///
/// This is installed as the `#[global_allocator]` only on `wasm32`, where this crate is the leaf
/// bundle; on other targets it is an ordinary type that exists so the accounting can be tested.
pub struct BoundedAlloc;

// SAFETY: pure pass-through wrapper over `System` (a sound allocator). Every method forwards the
// caller's `ptr`/`layout`/`new_size` unchanged, so `System` discharges all of `GlobalAlloc`'s
// obligations; this wrapper only reads `Layout::size()`/`new_size` and bumps `Relaxed` atomics. No
// pointer `System` returns is ever dereferenced, retained, or aliased. Registered in the
// unsafe-count ratchet: compliance/memsafety/unsafe-register.md (sq-wubkf).
unsafe impl GlobalAlloc for BoundedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ACCOUNTING.record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        ACCOUNTING.record_dealloc(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            ACCOUNTING.record_dealloc(layout.size());
            ACCOUNTING.record_alloc(new_size);
        }
        new_ptr
    }
}

// sq-wubkf: the accounting is only meaningful when this crate owns the process
// allocator, which it does exactly on `wasm32`, where it is the leaf bundle. On any other target
// the crate is a compile-only shim and the host binary owns its own allocator.
#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOCATOR: BoundedAlloc = BoundedAlloc;

/// Projected peak heap footprint while a request carrying `body_bytes` is in flight.
///
/// Saturating throughout: on `wasm32` `usize` is 32-bit, so a hostile length must clamp rather than
/// wrap into a small number that would wrongly be admitted.
pub fn projected_request_bytes(live_bytes: u64, body_bytes: u64) -> u64 {
    live_bytes
        .saturating_add(body_bytes.saturating_mul(BODY_FOOTPRINT_FACTOR))
        .saturating_add(REQUEST_HEADROOM_BYTES)
}

/// Whether a request carrying `body_bytes` may be admitted under `ceiling_bytes`.
///
/// A `ceiling_bytes` of `0` means unbounded and always admits.
pub fn admits(ceiling_bytes: u64, live_bytes: u64, body_bytes: u64) -> bool {
    ceiling_bytes == 0 || projected_request_bytes(live_bytes, body_bytes) <= ceiling_bytes
}

/// Whether a request carrying `body_bytes` may be admitted under the live counters.
pub fn admits_request(body_bytes: u64) -> bool {
    admits(ceiling_bytes(), live_bytes(), body_bytes)
}

/// Bytes currently held by live allocations.
pub fn live_bytes() -> u64 {
    ACCOUNTING.live()
}

/// The largest [`live_bytes`] value observed so far.
pub fn peak_bytes() -> u64 {
    ACCOUNTING.peak()
}

/// The configured ceiling in bytes; `0` means unbounded.
pub fn ceiling_bytes() -> u64 {
    CEILING_BYTES.load(Ordering::Relaxed) as u64
}

/// Replace the ceiling. `0` disables the bound.
pub fn set_ceiling_bytes(bytes: usize) {
    CEILING_BYTES.store(bytes, Ordering::Relaxed);
}

/// Convert a JavaScript number to a byte ceiling.
///
/// Returns `None` for a negative or non-finite value so a mistyped host argument is reported as an
/// error rather than silently disabling the bound. A value beyond the address space is clamped to
/// `usize::MAX`, which is unbounded in practice.
pub fn ceiling_from_js(bytes: f64) -> Option<usize> {
    if !bytes.is_finite() || bytes < 0.0 {
        return None;
    }
    if bytes >= usize::MAX as f64 {
        return Some(usize::MAX);
    }
    Some(bytes as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounting_tracks_live_and_peak_independently() {
        let counters = Accounting::new();
        assert_eq!(counters.live(), 0);
        assert_eq!(counters.peak(), 0);

        counters.record_alloc(1_000);
        counters.record_alloc(500);
        assert_eq!(counters.live(), 1_500);
        assert_eq!(counters.peak(), 1_500);

        counters.record_dealloc(1_200);
        assert_eq!(counters.live(), 300, "freeing lowers the live total");
        assert_eq!(counters.peak(), 1_500, "the high-water mark is sticky");

        counters.record_alloc(100);
        assert_eq!(counters.peak(), 1_500, "a smaller total does not raise the peak");
    }

    #[test]
    fn projection_charges_four_body_copies_plus_fixed_headroom() {
        assert_eq!(
            projected_request_bytes(0, 0),
            REQUEST_HEADROOM_BYTES,
            "an empty request still costs the fixed headroom"
        );
        assert_eq!(
            projected_request_bytes(1_000, 256),
            1_000 + 256 * 4 + REQUEST_HEADROOM_BYTES
        );
    }

    #[test]
    fn projection_saturates_instead_of_wrapping() {
        assert_eq!(projected_request_bytes(0, u64::MAX), u64::MAX);
        assert_eq!(projected_request_bytes(u64::MAX, 1), u64::MAX);
    }

    #[test]
    fn a_request_is_refused_once_its_projection_crosses_the_ceiling() {
        let ceiling = 8 * REQUEST_HEADROOM_BYTES;
        assert!(admits(ceiling, 0, 0), "an idle pod admits");
        assert!(
            admits(ceiling, ceiling - REQUEST_HEADROOM_BYTES, 0),
            "the projection may land exactly on the ceiling"
        );
        assert!(
            !admits(ceiling, ceiling - REQUEST_HEADROOM_BYTES + 1, 0),
            "one byte past the ceiling is refused"
        );
        assert!(
            !admits(ceiling, 0, ceiling),
            "a body whose four copies exceed the ceiling is refused"
        );
    }

    #[test]
    fn a_zero_ceiling_is_unbounded() {
        assert!(admits(0, u64::MAX, u64::MAX));
    }

    // Counter-level only: this drives `Accounting` directly, so it pins that a *deallocation*
    // restores headroom, NOT that any request path (an LDP DELETE, say) frees enough to get there.
    // See the module docs — the end-to-end claim needs a wasm32 test that does not exist yet.
    #[test]
    fn recorded_deallocations_restore_headroom() {
        let counters = Accounting::new();
        let ceiling = 4 * REQUEST_HEADROOM_BYTES;
        counters.record_alloc(4 * REQUEST_HEADROOM_BYTES as usize);
        assert!(
            !admits(ceiling, counters.live(), 0),
            "a full pod refuses further writes"
        );
        counters.record_dealloc(3 * REQUEST_HEADROOM_BYTES as usize);
        assert!(
            admits(ceiling, counters.live(), 0),
            "bytes returned to the allocator lower the live total — unlike a pages-grown high-water mark"
        );
    }

    #[test]
    fn a_js_ceiling_must_be_a_finite_non_negative_number() {
        assert_eq!(ceiling_from_js(0.0), Some(0));
        assert_eq!(ceiling_from_js(1_048_576.0), Some(1_048_576));
        assert_eq!(ceiling_from_js(-1.0), None);
        assert_eq!(ceiling_from_js(f64::NAN), None);
        assert_eq!(ceiling_from_js(f64::INFINITY), None);
        assert_eq!(ceiling_from_js(f64::MAX), Some(usize::MAX));
    }

    #[test]
    fn the_default_ceiling_leaves_the_stores_own_quota_untouched() {
        let store_quota = 64 * 1024 * 1024;
        assert!(
            admits(DEFAULT_CEILING_BYTES as u64, store_quota, 2 * 1024 * 1024),
            "a pod at its 64 MiB blob quota still admits a 2 MiB body"
        );
    }

    #[test]
    fn the_global_ceiling_round_trips() {
        let restore = ceiling_bytes();
        set_ceiling_bytes(12_345);
        assert_eq!(ceiling_bytes(), 12_345);
        set_ceiling_bytes(restore as usize);
        assert_eq!(ceiling_bytes(), restore);
    }
}
