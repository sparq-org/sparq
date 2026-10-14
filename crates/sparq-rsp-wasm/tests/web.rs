// sq-nzcb — headless wasm smoke test of the JS-facing sparq-rsp-wasm API.
//
// The crate's `#[cfg(test)] mod tests` in src/ runs on the NATIVE target only, so it
// exercises the sparq-rsp + serialiser functions the wrappers delegate to — NOT the actual
// `#[wasm_bindgen]` exports (`JsError::new` is a wasm-bindgen import that panics off-wasm).
// This module closes that gap: every test drives the REAL exported `Rsp` API compiled to and
// executed in a genuine wasm32 runtime via `wasm-pack test --node`, so the windowed
// continuous query + SPARQL-1.1-JSON serialisation across the JS boundary is asserted end to
// end — exactly the surface a `cargo build --target wasm32` never RUNS.
//
// Tiny, inline pushes. `wasm-pack test --node` runs each `#[wasm_bindgen_test]` in the Node
// executor (no browser, no DOM); Node is the default target, so no
// `wasm_bindgen_test_configure!(run_in_browser)` directive is needed.
//
// sq-734a — the numeric boundary args (`range`/`step`/`maxDelay`/`ts`) are `f64`,
// i.e. plain JS `Number`s, NOT `u64` (which would be a JS `BigInt`). The Rust-side tests pass
// `f64` literals; `js_number_args_do_not_throw_bigint_error` additionally drives the GENERATED
// JS glue with a real JS `Number` — the path the deployed page hits — the regression guard for
// the issue-#832 "Cannot convert 60 to a BigInt" page-load failure.

#![cfg(target_arch = "wasm32")]

use sparq_rsp_wasm::Rsp;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

const READING: &str = "<http://ex/reading>";
const SENSOR: &str = "<http://ex/s1>";

fn avg_query() -> String {
    format!("SELECT (AVG(?v) AS ?avg) WHERE {{ ?s {READING} ?v }}")
}

/// The headline demo: AVG(?v) over a tumbling 60-tick window fires once per window close,
/// returning a `{start,end,results}` envelope carrying the SPARQL-1.1-JSON result table.
#[wasm_bindgen_test]
fn tumbling_avg_window_fires_on_close() {
    let mut q = Rsp::select(&avg_query(), 60.0, 60.0, 0.0, "rstream").expect("register");
    // Two readings in [0,60): AVG = (10 + 20) / 2 = 15.0.
    let none = q.push(SENSOR, READING, "10", 0.0).expect("push");
    assert_eq!(
        none, "[]",
        "no window closes before the window end is reached"
    );
    let none = q.push(SENSOR, READING, "20", 30.0).expect("push");
    assert_eq!(none, "[]");
    // A push at ts=65 advances the watermark to 65 ≥ 60, closing [0,60).
    let closed = q.push(SENSOR, READING, "5", 65.0).expect("push");
    assert!(closed.contains("\"start\":0"), "closed [0,60): {closed}");
    assert!(closed.contains("\"end\":60"), "{closed}");
    assert!(closed.contains("\"avg\""), "projects ?avg: {closed}");
    assert!(closed.contains("15.0"), "AVG(10,20) = 15.0: {closed}");
}

/// `flush` closes the final, still-open window at end-of-stream (ignoring max_delay).
#[wasm_bindgen_test]
fn flush_closes_the_tail_window() {
    let mut q = Rsp::select(&avg_query(), 10.0, 10.0, 0.0, "rstream").expect("register");
    q.push(SENSOR, READING, "4", 2.0).expect("push");
    // Nothing has advanced the watermark to 10, so [0,10) is still open.
    let tail = q.flush().expect("flush");
    assert!(tail.contains("\"start\":0"), "flush closes [0,10): {tail}");
    assert!(tail.contains("4.0"), "AVG(4) = 4.0: {tail}");
}

/// ISTREAM emits only rows ADDED relative to the previous window; a sensor present in both
/// of two consecutive identical windows is NOT re-emitted the second time. Uses a plain
/// projection (not an aggregate) so the diff is over stable rows.
#[wasm_bindgen_test]
fn istream_emits_only_added_rows() {
    let select = format!("SELECT ?s WHERE {{ ?s {READING} ?v }}");
    let mut q = Rsp::select(&select, 10.0, 10.0, 0.0, "istream").expect("register");
    // Window [0,10): s1 present.
    q.push(SENSOR, READING, "1", 1.0).expect("push");
    // Window [10,20): s1 present again. ts=22 closes both [0,10) and [10,20).
    q.push(SENSOR, READING, "2", 11.0).expect("push");
    let closed = q.push(SENSOR, READING, "3", 22.0).expect("push");
    // [0,10) ISTREAMs s1 (added vs. empty); [10,20) ISTREAMs nothing (s1 already present).
    // The second window's bindings array must be empty.
    let last_window = closed.rsplit("\"start\":10").next().unwrap_or("");
    assert!(
        last_window.contains("\"bindings\":[]"),
        "the second identical window ISTREAMs no new rows: {closed}"
    );
}

/// A non-SELECT query is rejected at registration, not at the first push.
#[wasm_bindgen_test]
fn non_select_rejected_at_register() {
    let ask = format!("ASK {{ ?s {READING} ?v }}");
    assert!(Rsp::select(&ask, 10.0, 10.0, 0.0, "rstream").is_err());
}

/// An unknown R2S operator name is a clean error.
#[wasm_bindgen_test]
fn unknown_r2s_errors() {
    assert!(Rsp::select(&avg_query(), 10.0, 10.0, 0.0, "nope").is_err());
}

/// A zero-width / non-advancing window is rejected (range/step must be > 0) — a clean error,
/// NOT the panic the underlying `WindowSpec::time` would raise on a zero argument.
#[wasm_bindgen_test]
fn zero_window_rejected() {
    assert!(Rsp::select(&avg_query(), 0.0, 10.0, 0.0, "rstream").is_err());
    assert!(Rsp::select(&avg_query(), 10.0, 0.0, 0.0, "rstream").is_err());
}

/// sq-734a — a fractional / negative / non-finite numeric arg is a CLEAN error
/// (a `JsError`), not a silent truncation. A logical timestamp must be a whole number;
/// rounding one would corrupt window membership, so the boundary fails closed.
#[wasm_bindgen_test]
fn non_integer_numeric_args_rejected() {
    assert!(Rsp::select(&avg_query(), 10.5, 10.0, 0.0, "rstream").is_err());
    assert!(Rsp::select(&avg_query(), 10.0, 10.0, -1.0, "rstream").is_err());
    assert!(Rsp::select(&avg_query(), f64::NAN, 10.0, 0.0, "rstream").is_err());
    let mut q = Rsp::select(&avg_query(), 10.0, 10.0, 0.0, "rstream").expect("register");
    assert!(q.push(SENSOR, READING, "1", 2.5).is_err(), "fractional ts");
    assert!(q.push(SENSOR, READING, "1", -1.0).is_err(), "negative ts");
}

/// A malformed triple term is a clean error on push, not a panic.
#[wasm_bindgen_test]
fn malformed_triple_errors_on_push() {
    let mut q = Rsp::select(&avg_query(), 10.0, 10.0, 0.0, "rstream").expect("register");
    assert!(q.push("not a term", READING, "1", 0.0).is_err());
}

/// A late push whose every covering window already closed is dropped and COUNTED, not
/// silently swallowed — `lateDropped` reflects it. max_delay = 0, so a push behind a closed
/// window is too late.
#[wasm_bindgen_test]
fn late_push_is_counted() {
    let mut q = Rsp::select(&avg_query(), 10.0, 10.0, 0.0, "rstream").expect("register");
    q.push(SENSOR, READING, "1", 5.0).expect("push"); // [0,10)
    q.push(SENSOR, READING, "2", 25.0).expect("push"); // watermark 25 closes [0,10) and [10,20)
    // sq-734a: lateDropped() now returns a JS Number (f64), so compare to f64.
    assert_eq!(q.late_dropped(), 0.0, "no late drops yet");
    // ts=3 now lands behind the closed [0,10): too late.
    q.push(SENSOR, READING, "9", 3.0).expect("push");
    assert_eq!(q.late_dropped(), 1.0, "the ts=3 arrival is dropped as late");
}

// sq-734a — the wasm-bindgen NUMERIC-MARSHALLING reproduction for issue #832.
//
// The crate's own high-level `#[wasm_bindgen] Rsp` class is module-scoped in the generated JS
// glue, so a `wasm_bindgen_test` cannot reach it from JS to call it through the glue (calling
// it Rust→Rust, as every test above does, bypasses the marshalling entirely — which is exactly
// why the bug shipped). So this guard reproduces the *marshalling discipline itself*: it hands
// a plain JS `Number` (`60`) across the wasm-bindgen boundary into a callback whose parameter
// has the boundary's numeric type, and asserts which type accepts a Number and which throws.
//
//   * A parameter typed `f64` (what `Rsp.select`/`push` now use) is a JS `Number` — `cb(60)`
//     marshals cleanly.
//   * A parameter typed `u64` (what they used BEFORE the fix) is a JS `BigInt`; the generated
//     glue marshals it with `BigInt.asUintN(64, n)`, which throws `TypeError: Cannot convert
//     60 to a BigInt` for a `Number` — the exact page-load error.
//
// `callWith60` is the JS caller (the role the deployed page plays); the two closures are the
// `f64` and `u64` boundary shapes. This fails-before (the `f64` path did not exist) / passes-
// after, and pins the root cause so a future regression to a `BigInt` boundary type re-trips it.
#[wasm_bindgen(inline_js = r#"
export function callWith60(cb) {
    // The page calls Rsp.select(query, 60, ...) with a plain JS Number; mirror that here.
    try { cb(60); return null; }
    catch (e) { return String(e && e.message ? e.message : e); }
}
"#)]
extern "C" {
    fn callWith60(cb: &JsValue) -> JsValue;
}

/// THE regression guard for issue #832 ("Cannot convert 60 to a BigInt").
#[wasm_bindgen_test]
fn js_number_arg_marshals_as_f64_not_bigint() {
    use wasm_bindgen::closure::Closure;

    // The fixed boundary shape: an `f64` parameter is a JS `Number` — `cb(60)` must NOT throw.
    let seen = std::rc::Rc::new(std::cell::Cell::new(f64::NAN));
    let seen2 = seen.clone();
    let f64_cb = Closure::<dyn FnMut(f64)>::new(move |n: f64| seen2.set(n));
    let err = callWith60(f64_cb.as_ref());
    assert!(
        err.is_null(),
        "an f64 boundary param must accept the JS Number 60 (got error: {err:?})"
    );
    assert_eq!(seen.get(), 60.0, "the f64 callback received 60");

    // The buggy boundary shape: a `u64` parameter is a JS `BigInt`, so the Number 60 is
    // REJECTED at the boundary. The exact message depends on the wasm-bindgen build mode —
    // the RELEASE glue the deployed page runs throws `TypeError: Cannot convert 60 to a
    // BigInt` (`BigInt.asUintN(64, 60)`), while the DEV glue `wasm-pack test` builds throws
    // `expected a bigint argument, found number` (`_assertBigInt`). Either way a JS Number
    // does NOT cross a `u64`/BigInt boundary — which is the whole defect. (`callWith60`
    // returns the message on throw, `null` on success.)
    let u64_cb = Closure::<dyn FnMut(u64)>::new(move |_n: u64| {});
    let err = callWith60(u64_cb.as_ref());
    let msg = err.as_string().unwrap_or_default();
    let rejected_as_bigint = msg.to_ascii_lowercase().contains("bigint");
    assert!(
        rejected_as_bigint,
        "a u64 boundary param (a JS BigInt) must reject the Number 60 — the bug #832 we fixed \
         by using f64; expected a BigInt-coercion error, got: {msg:?}"
    );
}
