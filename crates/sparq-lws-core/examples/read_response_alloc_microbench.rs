// AUTHORED-BY Claude Opus 4.8
//! DETERMINISTIC allocation-COUNT micro-benchmark of the GET/HEAD read-response header construction
//! (perf round-C). It builds the SAME response `HeaderMap` the public-GET hot path builds — the
//! Content-Type / ETag / Accept-Ranges / Allow / Accept-Patch / 2 discovery Links / 1 type Link /
//! 1 acl Link / WAC-Allow / Content-Length set — in TWO forms: the pre-round-C "BEFORE" path
//! (`HeaderMap::new()` + `format!`/`HeaderValue::from_str` per request for every line, `to_string`
//! for the Content-Length numeral) and the round-C "AFTER" path (`with_capacity` + interned/static
//! `HeaderValue`s for the request-invariant lines + PRECOMPUTED discovery values + `itoa` for the
//! numeral). A COUNTING global allocator tallies the number of heap allocations each path makes.
//!
//! Unlike a wall-clock timing (advisory on a loaded box per the perf-gate rule), the allocation
//! COUNT is fully DETERMINISTIC — it is the number of `GlobalAlloc::alloc` calls, identical run to
//! run, machine to machine. That op-count reduction is the trustworthy, kept figure for this round.
//!
//! The "BEFORE"/"AFTER" header-build code below REPRODUCES the two `serve_read` formulations inline
//! (the handler's `add_*`/`set_*` helpers are module-private, exactly as `iri_guard_microbench.rs`
//! reproduces both IRI-guard predicates inline). The discovery `Link` values come from the SAME
//! public `link_headers` the server uses, so the BEFORE/AFTER bytes match the real path. A built-in
//! assertion proves BEFORE and AFTER emit BYTE-IDENTICAL header sets before any count is reported.
//!
//! Run: `cargo run --release --example read_response_alloc_microbench`.

#![warn(clippy::undocumented_unsafe_blocks)] // every unsafe site needs a // SAFETY:
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use sparq_lws_core::notifications::ws::link_headers;

/// A pass-through allocator that counts `alloc` calls while ARMED. Only the measured region arms it,
/// so the bench's own setup/printing allocations are excluded — the count is the build's alloc ops.
struct CountingAlloc;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

// SAFETY: pure pass-through wrapper over `System` (a sound allocator) — every method forwards
// verbatim with the caller's own arguments, so `System` discharges all of `GlobalAlloc`'s
// obligations; the wrapper only reads the ARMED flag and bumps a Relaxed atomic counter, and never
// dereferences, retains, or aliases any pointer `System` returns. Registered in the unsafe-count
// ratchet: compliance/memsafety/unsafe-register.md (sq-gg0qq.2).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            // A realloc is a (re)allocation op — count it (e.g. a HeaderMap grow-and-rehash).
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Count the heap alloc/realloc ops a closure makes.
fn count_allocs<F: FnOnce() -> R, R>(f: F) -> (u64, R) {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    let r = f();
    ARMED.store(false, Ordering::Relaxed);
    (ALLOCS.load(Ordering::Relaxed), r)
}

fn set_str(headers: &mut HeaderMap, name: HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(name, v);
    }
}

/// The per-request inputs a read response varies on (grouped so the build fns take few args).
struct ReadInputs<'a> {
    base_url: &'a str,
    target_iri: &'a str,
    content_type: &'a str,
    etag: &'a str,
    content_len: u64,
}

/// The AFTER path's request-INVARIANT precomputed values (built ONCE, as the server does at
/// construction): the precomputed discovery Link values + the interned `from_static` statics.
struct AfterInvariants {
    discovery_values: Vec<HeaderValue>,
    hv_resource: HeaderValue,
    hv_allow: HeaderValue,
    hv_accept_patch: HeaderValue,
    hv_accept_ranges: HeaderValue,
}

// --- BEFORE: the pre-round-C per-request formulation (one `format!`/`from_str` per line) ---------
fn build_before(input: &ReadInputs) -> HeaderMap {
    let &ReadInputs {
        base_url,
        target_iri,
        content_type,
        etag,
        content_len,
    } = input;
    let mut out = HeaderMap::new();
    set_str(&mut out, header::CONTENT_TYPE, content_type);
    set_str(&mut out, header::ETAG, etag);
    set_str(&mut out, header::ACCEPT_RANGES, "bytes");
    // method advertisement (plain resource: Allow + Accept-Patch)
    set_str(
        &mut out,
        header::ALLOW,
        "OPTIONS, HEAD, GET, PUT, POST, DELETE, PATCH",
    );
    set_str(
        &mut out,
        HeaderName::from_static("accept-patch"),
        "text/n3, application/sparql-update",
    );
    // discovery links — re-derived + re-formatted per request
    for (rel, t) in link_headers(base_url) {
        let value = format!("<{t}>; rel=\"{rel}\"");
        if let Ok(v) = HeaderValue::from_str(&value) {
            out.append(header::LINK, v);
        }
    }
    // type link (plain resource: only ldp:Resource) — re-formatted per request
    let value = format!("<{}>; rel=\"type\"", "http://www.w3.org/ns/ldp#Resource");
    if let Ok(v) = HeaderValue::from_str(&value) {
        out.append(header::LINK, v);
    }
    // acl link (PER-TARGET — same in both paths, intentionally not interned)
    let acl_url = format!("{target_iri}.acl");
    let value = format!("<{acl_url}>; rel=\"acl\"");
    if let Ok(v) = HeaderValue::from_str(&value) {
        out.append(header::LINK, v);
    }
    // WAC-Allow (per-request value — same in both paths)
    set_str(
        &mut out,
        HeaderName::from_static("wac-allow"),
        "user=\"read\",public=\"read\"",
    );
    // Content-Length via to_string (heap String)
    set_str(&mut out, header::CONTENT_LENGTH, &content_len.to_string());
    out
}

// --- AFTER: the round-C formulation (interned/precomputed invariants + itoa numeral) -------------
fn build_after(inv: &AfterInvariants, input: &ReadInputs) -> HeaderMap {
    let &ReadInputs {
        base_url: _,
        target_iri,
        content_type,
        etag,
        content_len,
    } = input;
    let mut out = HeaderMap::with_capacity(16);
    set_str(&mut out, header::CONTENT_TYPE, content_type);
    set_str(&mut out, header::ETAG, etag);
    out.insert(header::ACCEPT_RANGES, inv.hv_accept_ranges.clone());
    // method advertisement — interned statics (clone = refcount bump, no alloc)
    out.insert(header::ALLOW, inv.hv_allow.clone());
    out.insert(
        HeaderName::from_static("accept-patch"),
        inv.hv_accept_patch.clone(),
    );
    // discovery links — PRECOMPUTED (clone = refcount bump)
    for v in &inv.discovery_values {
        out.append(header::LINK, v.clone());
    }
    // type link — interned static (clone = refcount bump)
    out.append(header::LINK, inv.hv_resource.clone());
    // acl link (PER-TARGET — identical to BEFORE, intentionally per-request)
    let acl_url = format!("{target_iri}.acl");
    let value = format!("<{acl_url}>; rel=\"acl\"");
    if let Ok(v) = HeaderValue::from_str(&value) {
        out.append(header::LINK, v);
    }
    // WAC-Allow (per-request value — identical to BEFORE)
    set_str(
        &mut out,
        HeaderName::from_static("wac-allow"),
        "user=\"read\",public=\"read\"",
    );
    // Content-Length via itoa (stack buffer, no heap String)
    let mut buf = itoa::Buffer::new();
    if let Ok(v) = HeaderValue::from_str(buf.format(content_len)) {
        out.insert(header::CONTENT_LENGTH, v);
    }
    out
}

fn sorted_lines(h: &HeaderMap) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = h
        .iter()
        .map(|(n, val)| (n.as_str().to_string(), val.to_str().unwrap().to_string()))
        .collect();
    v.sort();
    v
}

fn main() {
    let base = "https://localhost:3000";
    let input = ReadInputs {
        base_url: base,
        target_iri: "https://localhost:3000/alice/profile/card",
        content_type: "text/turtle",
        etag: "\"abc123\"",
        content_len: 4096,
    };

    // Precompute the AFTER path's request-invariant values ONCE (as the server does at construction).
    let inv = AfterInvariants {
        discovery_values: link_headers(base)
            .into_iter()
            .filter_map(|(rel, t)| HeaderValue::from_str(&format!("<{t}>; rel=\"{rel}\"")).ok())
            .collect(),
        hv_resource: HeaderValue::from_static("<http://www.w3.org/ns/ldp#Resource>; rel=\"type\""),
        hv_allow: HeaderValue::from_static("OPTIONS, HEAD, GET, PUT, POST, DELETE, PATCH"),
        hv_accept_patch: HeaderValue::from_static("text/n3, application/sparql-update"),
        hv_accept_ranges: HeaderValue::from_static("bytes"),
    };

    // CORRECTNESS GATE: BEFORE and AFTER must emit a BYTE-IDENTICAL header set (same names+values).
    let before = build_before(&input);
    let after = build_after(&inv, &input);
    assert_eq!(
        sorted_lines(&before),
        sorted_lines(&after),
        "BEFORE and AFTER must emit byte-identical response headers"
    );

    // Measure: count the heap alloc/realloc ops each header-build makes (the measured region is the
    // single build call; the precomputed AFTER invariants are built ONCE above, outside the count —
    // exactly as the server amortises them across the process lifetime).
    let (before_allocs, _) = count_allocs(|| build_before(&input));
    let (after_allocs, _) = count_allocs(|| build_after(&inv, &input));

    println!(
        "# read-response header-build allocation count (per public-GET response, plain resource)"
    );
    println!("# DETERMINISTIC heap alloc/realloc op count (counting global allocator)");
    println!("# header set is byte-identical between BEFORE and AFTER (asserted above)");
    println!();
    println!("BEFORE (format!/from_str per line, to_string numeral)   {before_allocs:3} alloc ops");
    println!("AFTER  (interned + precomputed invariants + itoa)        {after_allocs:3} alloc ops");
    let saved = before_allocs.saturating_sub(after_allocs);
    let pct = if before_allocs > 0 {
        100.0 * saved as f64 / before_allocs as f64
    } else {
        0.0
    };
    println!("DELTA                                                    -{saved:2} alloc ops ({pct:.0}% fewer)");
}
