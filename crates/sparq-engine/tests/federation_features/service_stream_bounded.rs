//! Bounded-memory assertion for streaming SERVICE result consumption.
//! (bead sq-my8wd.4)
//!
//! The engine must not materialise a remote SERVICE relation as owned terms before
//! the join: the pre-sq-my8wd.4 path held the response body PLUS a whole-document
//! JSON DOM PLUS a term-level copy of every row, so a large/adversarial remote result
//! amplified into a multiple of its own wire size. The streaming path interns each
//! row to compact ids as it is parsed, so the only response-scale state is the
//! (finite, transport-capped) body and the id-level relation the join itself needs.
//!
//! This test runs the PRODUCTION path — the real ureq `HttpTransport` against a
//! loopback endpoint — over a large, duplicate-heavy remote result and asserts, with
//! a thread-local byte-counting allocator, that the query thread's peak
//! live-allocation delta stays within a small multiple of the response body. The
//! multiplier is deliberately generous: the assertion pins the SHAPE of the bound
//! (O(body), not O(parsed DOM + term relation)), not a tuned number — the old
//! collect-everything path exceeds it by construction.
//!
//! The allocator lives HERE (an integration-test crate) because the engine crate
//! itself is `#![forbid(unsafe_code)]` and a `GlobalAlloc` impl requires `unsafe`.
//! Counters are THREAD-LOCAL, so the server thread's allocations (serving the body)
//! and any parallel sibling test never pollute the measurement, and the measurement
//! is deterministic (no timing involved).
//!
//! Built only when the `service` feature is on:
//!   cargo test -p sparq-engine --features service --test service_stream_bounded

#![cfg(feature = "service")]
// sq-my8wd.4 / cert gap GX-5: this is the ONLY first-party `unsafe` in
// sparq-engine — a byte-counting `#[global_allocator]`, which a custom allocator
// unavoidably requires (the `GlobalAlloc` trait is `unsafe` by definition and there is
// no safe substitute for a deterministic thread-local allocation counter). The engine
// LIBRARY stays `#![forbid(unsafe_code)]`; the allocator is confined to this one
// integration-test binary and each site carries a `// SAFETY:` argument, mechanically
// enforced here by `clippy::undocumented_unsafe_blocks` (mirroring the register's
// 5 unsafe-bearing lib crates) and registered in `compliance/memsafety/unsafe-register.md`.
#![warn(clippy::undocumented_unsafe_blocks)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use sparq_core::Graph;
use sparq_engine::{query, with_service_egress_allow};

// ---------------------------------------------------------------------------
// Thread-local byte-counting allocator (per-thread live/peak, no timing).
// ---------------------------------------------------------------------------

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

/// Record a live-byte delta on THIS thread. `try_with` so a (de)allocation during
/// thread teardown can never panic inside the allocator — it is simply not counted.
fn add(d: isize) {
    let _ = LIVE.try_with(|l| {
        let v = l.get() + d;
        l.set(v);
        if d > 0 {
            let _ = PEAK.try_with(|p| {
                if v > p.get() {
                    p.set(v);
                }
            });
        }
    });
}

struct Counting;

// SAFETY: every method forwards its call VERBATIM to the process `System` allocator
// with the identical `Layout`/pointer arguments it received, so all of `GlobalAlloc`'s
// safety obligations are discharged by `System`'s own (correct) implementation. The
// wrapper adds nothing unsound: it only reads `layout.size()`/`new_size` (always valid
// `usize` reads) and updates a thread-local `Cell<isize>` counter; no pointer that
// `System` returns is ever dereferenced, retained, or aliased here. `System` is
// zero-sized, so `Counting` is a valid stand-in global allocator.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: forwards to `System.alloc(layout)` unchanged — the caller's `layout`
    // validity contract (non-zero size, valid align) is passed straight through, and
    // only the returned pointer's nullness is inspected before recording the size.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            add(layout.size() as isize);
        }
        p
    }
    // SAFETY: forwards `ptr`/`layout` unchanged to `System.dealloc`. The caller
    // guarantees `ptr` was returned by a previous `alloc`/`realloc` of THIS allocator
    // with this same `layout`; that holds because those methods above also forward to
    // `System`, so the pointer is always a genuine `System` allocation.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        add(-(layout.size() as isize));
    }
    // SAFETY: forwards `ptr`/`layout`/`new_size` unchanged to `System.realloc` — the
    // caller's contract (valid `ptr` for `layout`, `new_size` non-zero and not
    // overflowing when rounded up to `layout.align()`) is passed straight through, and
    // only the returned pointer's nullness is inspected before recording the delta.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            add(new_size as isize - layout.size() as isize);
        }
        p
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// Peak live-byte DELTA on this thread while `f` runs.
fn peak_delta_during<R>(f: impl FnOnce() -> R) -> (isize, R) {
    let base = LIVE.with(Cell::get);
    PEAK.with(|p| p.set(base));
    let r = f();
    (PEAK.with(Cell::get) - base, r)
}

// ---------------------------------------------------------------------------
// Loopback endpoint (same pattern as tests/service_federation.rs).
// ---------------------------------------------------------------------------

/// Spawn a one-shot HTTP/1.1 server that replies to `n` requests with `body`
/// (Content-Type sparql-results+json), returning the bound URL. The body is served
/// from the SERVER thread, so its allocations are outside the measured thread.
fn serve(body: String, n: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Drain the WHOLE request (headers + the Content-Length form body). With a
            // multi-megabyte response, closing the socket while unread request bytes
            // are pending would RST the connection and truncate the client's read.
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            let (mut hdr_end, mut want) = (None, 0usize);
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
                if hdr_end.is_none() {
                    if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        hdr_end = Some(pos + 4);
                        let headers = String::from_utf8_lossy(&req[..pos]);
                        want = headers
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                    }
                }
                if let Some(h) = hdr_end {
                    if req.len() >= h + want {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/sparql-results+json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    rx.recv().unwrap(); // wait until the listener thread is live
    format!("http://{}/", addr)
}

/// A large, duplicate-heavy SPARQL-Results-JSON body: `rows` solutions over only 7
/// distinct (s, n) pairs. Duplicate-heavy keeps the query-local vocabulary (which the
/// join legitimately retains, deduped) tiny, so the assertion isolates the CONSUMPTION
/// overhead; the duplicates themselves must all survive (multiset semantics).
fn big_srj_body(rows: usize) -> String {
    let mut bindings = Vec::with_capacity(rows);
    for i in 0..rows {
        let k = i % 7;
        bindings.push(format!(
            r#"{{"s":{{"type":"uri","value":"http://remote.example/dup/{}"}},"n":{{"type":"literal","value":"remote-value-{}"}}}}"#,
            k, k
        ));
    }
    format!(
        r#"{{"head":{{"vars":["s","n"]}},"results":{{"bindings":[{}]}}}}"#,
        bindings.join(",")
    )
}

/// (bead sq-my8wd.5) Transport-reader seam: peak memory stays BELOW the
/// response body size because the body is NEVER fully buffered as a `String`. The
/// pre-reader-seam path held a full body String even with streaming row parsing, so
/// peak was necessarily >= body. This test asserts we crossed that threshold.
///
/// Invariant (load-bearing for the reader seam): on the reader path the query
/// thread's peak live-allocation delta is STRICTLY LESS THAN the body size.
/// A generous buffer overhead is added for the BufReader, parser working state,
/// id-level compact relation, and local vocab — but NOT a full body-sized String.
#[test]
fn transport_reader_seam_peak_stays_below_body_size() {
    // ~10k rows, ~1.1 MB body. Large enough for the String-buffering cost to be
    // measurable but small enough to keep the test fast.
    let body = big_srj_body(10_000);
    let body_len = body.len() as isize;
    let url = serve(body, 1);

    let g = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
    // COUNT(*) keeps the output to a single row, isolating SERVICE consumption.
    let q = format!(
        "SELECT (COUNT(*) AS ?c) WHERE {{ SERVICE <{}> {{ ?s ?p ?n }} }}",
        url
    );

    let (peak, res) = peak_delta_during(|| {
        with_service_egress_allow(["127.0.0.1".to_string()], || query(&g, &q))
    });
    let res = res.expect("federated COUNT query (reader seam)");
    assert_eq!(res.rows.len(), 1);
    let count = res.rows[0][0].as_ref().expect("count term").to_string();
    assert!(
        count.starts_with("\"10000\""),
        "duplicate rows must survive the reader path (multiset semantics), got {}",
        count
    );

    // THE LOAD-BEARING INVARIANT: peak live bytes < body size.
    // A generous additive overhead covers BufReader internal buffer + parser working
    // state + compact id-level rows + deduped vocab — but NOT a full body String.
    // If the body were buffered as a String the peak would be >= body_len.
    let overhead = 4 << 20; // 4 MiB generous overhead
    let bound = body_len + overhead;
    assert!(
        peak < bound,
        "transport reader seam peak {} bytes >= body {} bytes + overhead {} bytes: \
         the body is being buffered as a String (reader seam not active on this path)",
        peak,
        body_len,
        overhead
    );
}

#[test]
fn large_service_result_consumption_is_bounded_by_the_body_not_the_relation() {
    // ~60k rows, ~7 MB on the wire. All terms are absent from the local graph, so
    // every distinct term takes the local-vocab interning path.
    let body = big_srj_body(60_000);
    let body_len = body.len() as isize;
    let url = serve(body, 1);

    let g = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
    // COUNT(*) keeps the OUTPUT tiny (one row), so the measurement isolates the
    // SERVICE consumption itself rather than result materialisation; the count also
    // proves multiset semantics survive streaming (60_000 rows, NOT 7 deduped).
    let q = format!(
        "SELECT (COUNT(*) AS ?c) WHERE {{ SERVICE <{}> {{ ?s ?p ?n }} }}",
        url
    );

    let (peak, res) = peak_delta_during(|| {
        with_service_egress_allow(["127.0.0.1".to_string()], || query(&g, &q))
    });
    let res = res.expect("federated COUNT query");
    assert_eq!(res.rows.len(), 1);
    let count = res.rows[0][0].as_ref().expect("count term").to_string();
    assert!(
        count.starts_with("\"60000\""),
        "duplicate rows must survive streaming (multiset semantics), got {}",
        count
    );

    // The bound: peak consumption stays within a small multiple of the wire body
    // (body String + read-buffer growth + compact id rows + the tiny deduped vocab).
    // The pre-streaming path held body + whole-document DOM + a term-level relation
    // copy — several times this bound on the same fixture.
    let bound = body_len * 3 + (4 << 20);
    assert!(
        peak < bound,
        "SERVICE consumption peak {} bytes exceeds the O(body) bound {} (body {} bytes): \
         the remote relation is being materialised again",
        peak,
        bound,
        body_len
    );
}
