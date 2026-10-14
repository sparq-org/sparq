//! sparq-rsp-wasm: the sparq windowed **RSP-QL stream processor** compiled to
//! WebAssembly — the tier-b "W-rsp" bundle (sq-nzcb).
//!
//! This is a SEPARATE, lazy-loaded bundle from the lean `sparq-wasm` triplestore bundle.
//! It exposes `sparq-rsp`'s deterministic windowed continuous SPARQL — sliding/tumbling
//! time windows with RSTREAM / ISTREAM / DSTREAM output — for in-tab LIVE streaming on the
//! showcase site's `/surface/streaming-rsp` page.
//!
//! The crate `sparq-rsp` is "wasm-safe" by construction: it reads no wall clock and runs no
//! async runtime — **time only advances through application-supplied timestamps**. So the
//! browser tab IS the logical clock: each pushed reading carries a `ts`, a window closes
//! when the watermark (`max_ts − max_delay`) reaches its end, and the whole pipeline is a
//! pure, replayable function of the pushed `(triple, ts)` sequence. This bundle confirms
//! that wasm-safety claim — it builds for `wasm32-unknown-unknown` and the headless suite
//! drives the real exports in a genuine wasm runtime.
//!
//! Like the lean bundle it carries no serde: each closed window's SELECT result is
//! serialised through `sparq-engine`'s canonical **SPARQL 1.1 JSON** serialiser
//! (`to_sparql_json_rows`), and the per-window envelope (`{start,end,...}`) is hand-built.
//!
//! ```js
//! import init, { Rsp } from "./sparq_rsp_wasm.js";
//! await init();
//! // AVG(?v) over tumbling 60-tick windows (RANGE 60 STEP 60), full result per window.
//! const q = Rsp.select(
//!   "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/reading> ?v }",
//!   60, 60, 0, "rstream",
//! );
//! // Push timestamped readings; each push returns the windows that just CLOSED, as JSON.
//! let closed = JSON.parse(q.push("<http://ex/s1>", "<http://ex/reading>", "10", 0));
//! closed = JSON.parse(q.push("<http://ex/s1>", "<http://ex/reading>", "20", 65)); // closes [0,60)
//! // [{ "start": 0, "end": 60, "results": { "head": {...}, "results": {...} } }]
//! const tail = JSON.parse(q.flush()); // end-of-stream: close remaining windows
//! ```
#![forbid(unsafe_code)] // sq-nzcb: crate has zero `unsafe`

use oxrdf::Term;
use sparq_core::Graph;
use sparq_rsp::{ContinuousQuery, WindowResult, WindowSpec, R2S};
use wasm_bindgen::prelude::*;

// NOTE: the pure helpers below return `Result<_, String>`, NOT `Result<_, JsError>`. They
// are reached from native unit tests, and `JsError::new` is a wasm-bindgen import that
// PANICS off-wasm — so every `JsError` is constructed only inside a `#[wasm_bindgen]` method
// (which never runs natively), exactly the discipline the sibling wasm bundles follow.

/// sq-734a — converts a JS-supplied numeric argument (`range` / `step` /
/// `max_delay` / `ts`) to the `u64` the underlying `sparq-rsp` window API uses.
///
/// The wasm-boundary parameters are typed `f64` — a plain JS `Number` — NOT `u64`. A `u64`
/// wasm-bindgen parameter is a JS `BigInt`, and the release glue marshals it with
/// `BigInt.asUintN(64, n)`, which **throws** `TypeError: Cannot convert <n> to a BigInt`
/// when handed an ordinary JS `Number` (it does not coerce one). The site (and every other
/// caller) naturally passes Numbers — `Rsp.select(q, 60, 60, 0, "rstream")` — so the page
/// threw `Cannot convert 60 to a BigInt` on load. Taking `f64` here accepts the Number and
/// this helper does the checked narrowing in Rust instead, where a bad value is a clean
/// `JsError` rather than a thrown wasm-glue `TypeError`.
///
/// `name` labels the argument in the error. The value must be a non-negative, finite,
/// integral `f64` no larger than 2⁵³−1 (`Number.MAX_SAFE_INTEGER`) — the range over which a
/// JS `Number` represents every integer EXACTLY, so the `u64` round-trips losslessly. A
/// fractional, negative, NaN/∞, or too-large value is rejected (a silent truncation of a
/// logical timestamp would corrupt window membership, so this fails closed).
fn u64_arg(name: &str, v: f64) -> Result<u64, String> {
    // 2^53 - 1 = Number.MAX_SAFE_INTEGER — the largest f64 with exact integer round-trip.
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    if !v.is_finite() {
        return Err(format!("{} must be a finite number, got {}", name, v));
    }
    if v.fract() != 0.0 {
        return Err(format!("{} must be a whole number, got {}", name, v));
    }
    if v < 0.0 {
        return Err(format!("{} must be >= 0, got {}", name, v));
    }
    if v > MAX_SAFE {
        return Err(format!(
            "{} must be <= 2^53-1 (Number.MAX_SAFE_INTEGER), got {}",
            name, v
        ));
    }
    Ok(v as u64)
}

/// Parses the JS-supplied relation-to-stream operator name (`"rstream"` | `"istream"` |
/// `"dstream"`, case-insensitive, plus the one-letter aliases) or returns an error message
/// naming the accepted values — the single place the R2S string is validated.
fn parse_r2s(r2s: &str) -> Result<R2S, String> {
    match r2s.to_ascii_lowercase().as_str() {
        "rstream" | "r" => Ok(R2S::RStream),
        "istream" | "i" => Ok(R2S::IStream),
        "dstream" | "d" => Ok(R2S::DStream),
        other => Err(format!(
            "unknown R2S operator {other:?}; expected \"rstream\", \"istream\" or \"dstream\""
        )),
    }
}

/// Reconstructs a `[Term; 3]` from three **Turtle**-syntax term strings by reusing
/// `sparq-engine`'s exhaustively-tested Turtle parser: the three terms are assembled into one
/// statement, parsed, and decoded back to terms. This bundle therefore owns NO term-parsing
/// code of its own — the JS side hands the same lexical syntax the rest of sparq round-trips.
///
/// Turtle (a superset of N-Triples) is chosen over N-Triples deliberately: it accepts the
/// **numeric-literal shorthand** the streaming demo leans on — a bare `10` is `xsd:integer`,
/// `10.5` is `xsd:decimal` — alongside `<iri>`, `"str"`, `"str"@en`, `"v"^^<dt>` and `_:b`.
/// N-Triples would force the verbose `"10"^^<…#integer>`, which is hostile for a UI that
/// streams sensor readings. (Turtle `@prefix`/`a` shorthands also work, but the JS side
/// supplies absolute terms.)
fn parse_triple(s: &str, p: &str, o: &str) -> Result<[Term; 3], String> {
    // Assemble exactly one Turtle statement. A blank node / IRI / literal subject & object
    // and an IRI predicate are all valid here; the parser rejects anything else.
    let line = format!("{s} {p} {o} .\n");
    let (dict, triples) = Graph::parse_to_triples(&line, "turtle")
        .map_err(|e| format!("could not parse triple as Turtle ({s} {p} {o}): {e}"))?;
    match triples.as_slice() {
        [[sid, pid, oid]] => Ok([dict.term(*sid), dict.term(*pid), dict.term(*oid)]),
        // parse_to_triples succeeds only on a well-formed document; a single statement
        // yields exactly one triple. Anything else (multiple statements smuggled in, or a
        // predicate-object list) is a misuse of the per-element push API.
        other => Err(format!(
            "expected exactly one triple per push, got {} (do not pass multiple statements or predicate-object lists)",
            other.len()
        )),
    }
}

/// Serialises one closed window to a JSON object
/// `{"start":S,"end":E,"results":<SPARQL-1.1-JSON>}` — the window bounds plus the R2S-filtered
/// SELECT result table in the standard, self-contained SPARQL 1.1 JSON form. The `results`
/// value is produced by `sparq-engine`'s canonical serialiser (so the page can hand it to
/// any SPARQL-JSON renderer); the envelope is hand-built (the bundle carries no serde).
fn window_result_json(out: &mut String, w: &WindowResult) {
    out.push_str("{\"start\":");
    out.push_str(&w.start.to_string());
    out.push_str(",\"end\":");
    out.push_str(&w.end.to_string());
    out.push_str(",\"results\":");
    out.push_str(&sparq_engine::json::to_sparql_json_rows(&w.vars, &w.rows));
    out.push('}');
}

/// Serialises a batch of closed windows to a JSON array `[{...},{...}]` (empty `[]` when no
/// window closed on this push). Each `push` / `flush` returns one such array, so the JS side
/// renders zero or more newly-closed windows per pushed reading.
fn windows_json(windows: &[WindowResult]) -> String {
    let mut out = String::from("[");
    for (i, w) in windows.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        window_result_json(&mut out, w);
    }
    out.push(']');
    out
}

/// A live, in-tab RSP-QL continuous SELECT: a windowed SPARQL query fed by timestamped
/// pushes, with sliding/tumbling time windows and RSTREAM / ISTREAM / DSTREAM output.
///
/// The handle is STATEFUL across pushes (unlike the one-shot reasoner bundle): it owns the
/// window state and — for ISTREAM/DSTREAM — the previous window's result (the diff base), so
/// the UI can stream readings in and watch windows fire. Construct it with
/// [`select`](Self::select), feed [`push`](Self::push)es, and [`flush`](Self::flush) at
/// end-of-stream.
#[wasm_bindgen]
pub struct Rsp {
    query: ContinuousQuery,
}

#[wasm_bindgen]
impl Rsp {
    /// Registers a continuous SELECT over a TIME window. `sparql` is a standard SPARQL
    /// SELECT (validated NOW — a malformed or non-SELECT query is rejected here, not at the
    /// first push). `range` / `step` are the RSP-QL window parameters in the same logical
    /// time unit the pushes use: `step == range` is tumbling, `step < range` sliding.
    /// `max_delay` is the out-of-order tolerance (the watermark lags the newest timestamp by
    /// this much; 0 = no tolerance). `r2s` is `"rstream"` (full result per window — the
    /// default), `"istream"` (rows added vs. the previous window) or `"dstream"` (rows
    /// removed).
    ///
    /// `range` / `step` / `max_delay` are plain JS `Number`s (sq-734a). They are
    /// the integer logical-time unit the pushes use; each must be a whole number in
    /// `[0, 2^53-1]` (negative / fractional / NaN / too-large is a clean error). See
    /// `u64_arg` for why the boundary is `f64` and not `u64`.
    ///
    /// Errors if the SPARQL is malformed / not a SELECT, the R2S name is unknown,
    /// `range`/`step` is zero (a zero-width or non-advancing window is meaningless), or a
    /// numeric argument is out of range.
    pub fn select(
        sparql: &str,
        range: f64,
        step: f64,
        max_delay: f64,
        r2s: &str,
    ) -> Result<Rsp, JsError> {
        let range = u64_arg("range", range).map_err(|e| JsError::new(&e))?;
        let step = u64_arg("step", step).map_err(|e| JsError::new(&e))?;
        let max_delay = u64_arg("maxDelay", max_delay).map_err(|e| JsError::new(&e))?;
        if range == 0 {
            return Err(JsError::new("window RANGE must be > 0"));
        }
        if step == 0 {
            return Err(JsError::new("window STEP must be > 0"));
        }
        let r2s = parse_r2s(r2s).map_err(|e| JsError::new(&e))?;
        let spec = WindowSpec::time(range, step).with_max_delay(max_delay);
        let query = ContinuousQuery::register(sparql, spec).map_err(|e| JsError::new(&e))?;
        Ok(Rsp {
            query: query.with_r2s(r2s),
        })
    }

    /// Pushes one timestamped triple `(s, p, o)` at logical timestamp `ts`. The terms are
    /// Turtle-syntax strings: `<iri>`, the numeric shorthand `10` / `10.5`, `"str"`,
    /// `"hi"@en`, `"v"^^<…#decimal>`, `_:b`. Returns the windows this push CLOSED — a JSON
    /// ARRAY of `{"start","end","results"}` objects, oldest first, possibly empty (`[]`).
    ///
    /// `ts` is a plain JS `Number` (sq-734a) — a whole logical timestamp in
    /// `[0, 2^53-1]` (see `u64_arg`); a fractional / negative / too-large `ts` is a clean
    /// error.
    ///
    /// Errors if a term fails to parse as Turtle, `ts` is out of range, or the engine errors
    /// evaluating a closed window (the query itself was validated at registration).
    pub fn push(&mut self, s: &str, p: &str, o: &str, ts: f64) -> Result<String, JsError> {
        let ts = u64_arg("ts", ts).map_err(|e| JsError::new(&e))?;
        let triple = parse_triple(s, p, o).map_err(|e| JsError::new(&e))?;
        let mut closed = Vec::new();
        self.query
            .push(triple, ts, |w| closed.push(w))
            .map_err(|e| JsError::new(&e))?;
        Ok(windows_json(&closed))
    }

    /// End-of-stream: closes every window up to the last timestamp seen (ignoring
    /// `max_delay`) and returns them as the same JSON array [`push`](Self::push) returns.
    /// Idempotent once drained (a second `flush` with no new pushes closes nothing and
    /// returns `[]`).
    pub fn flush(&mut self) -> Result<String, JsError> {
        let mut closed = Vec::new();
        self.query
            .flush(|w| closed.push(w))
            .map_err(|e| JsError::new(&e))?;
        Ok(windows_json(&closed))
    }

    /// The number of arrivals dropped as TOO LATE (every covering window had already
    /// closed) — the observability counter the page surfaces so a late push that lands in no
    /// window is visible rather than silently swallowed.
    ///
    /// Returned as a JS `Number` (sq-734a) — symmetric with the `f64` inputs, so
    /// the page reads it without a `BigInt`. The count is bounded by the number of pushes, so
    /// it stays within `Number`'s exact-integer range in every realistic stream.
    #[wasm_bindgen(js_name = lateDropped)]
    pub fn late_dropped(&self) -> f64 {
        self.query.late_dropped() as f64
    }

    /// The registered SPARQL text (echo, for the UI).
    pub fn sparql(&self) -> String {
        self.query.sparql().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reading vocabulary the streaming page demonstrates.
    const READING: &str = "<http://ex/reading>";
    const SENSOR: &str = "<http://ex/s1>";

    /// R2S name parsing accepts the documented names (case-insensitive, plus the one-letter
    /// aliases) and rejects others.
    #[test]
    fn r2s_parsing() {
        assert_eq!(parse_r2s("rstream").unwrap(), R2S::RStream);
        assert_eq!(parse_r2s("ISTREAM").unwrap(), R2S::IStream);
        assert_eq!(parse_r2s("d").unwrap(), R2S::DStream);
        assert!(parse_r2s("nope").is_err());
    }

    /// A single Turtle statement round-trips to a `[Term; 3]` whose object is the reading
    /// literal — the shape the AVG demo pushes (a bare `10`, via Turtle's numeric shorthand).
    #[test]
    fn parse_triple_reading() {
        let [s, p, o] = parse_triple(SENSOR, READING, "10").unwrap();
        assert_eq!(s.to_string(), SENSOR);
        assert_eq!(p.to_string(), READING);
        // Turtle's bare-integer shorthand parses `10` as an xsd:integer with value "10".
        match o {
            Term::Literal(l) => {
                assert_eq!(l.value(), "10");
                assert_eq!(
                    l.datatype().as_str(),
                    "http://www.w3.org/2001/XMLSchema#integer"
                );
            }
            other => panic!("expected a literal object, got {other:?}"),
        }
    }

    /// A malformed term is a clean error, not a panic; and smuggling a second statement in is
    /// rejected (the push API is one-triple-per-call).
    #[test]
    fn parse_triple_rejects_garbage_and_multistatement() {
        assert!(parse_triple("not a term", READING, "1").is_err());
        // Two statements on one "object" arg: the trailing ` . <a> <b> <c>` is a second
        // triple, which the per-element API forbids.
        assert!(parse_triple(
            SENSOR,
            READING,
            "<http://ex/o> . <http://ex/a> <http://ex/b>"
        )
        .is_err());
    }

    // The #[wasm_bindgen] wrappers (Rsp::select/push/flush) return JsError, which cannot run
    // natively (`JsError::new` is a wasm-bindgen import that panics off-wasm). So — exactly as
    // the other bundles' native tests do — these exercise the underlying sparq-rsp calls and
    // the pure serialisers; the headless `wasm-pack test --node` suite (tests/web.rs) drives
    // the real exported API across the JS boundary.

    /// The window-result serialiser emits the `{start,end,results}` envelope with the
    /// SPARQL-1.1-JSON result table inside.
    #[test]
    fn window_json_envelope() {
        let mut q = ContinuousQuery::register(
            &format!("SELECT (AVG(?v) AS ?avg) WHERE {{ ?s {READING} ?v }}"),
            WindowSpec::time(10, 10),
        )
        .unwrap();
        let t = |v: i32| parse_triple(SENSOR, READING, &v.to_string()).unwrap();
        let mut closed = Vec::new();
        q.push(t(2), 1, |w| closed.push(w)).unwrap();
        q.push(t(4), 5, |w| closed.push(w)).unwrap();
        q.push(t(9), 12, |w| closed.push(w)).unwrap(); // ts 12 closes [0,10)
        assert_eq!(closed.len(), 1);
        let json = windows_json(&closed);
        // Envelope bounds.
        assert!(json.contains("\"start\":0"), "{json}");
        assert!(json.contains("\"end\":10"), "{json}");
        // SPARQL-1.1-JSON results table, AVG(2,4) = 3.0 (xsd:decimal per SPARQL typing).
        assert!(json.contains("\"head\""), "{json}");
        assert!(json.contains("\"bindings\""), "{json}");
        assert!(json.contains("\"avg\""), "{json}");
        assert!(json.contains("3.0"), "AVG(2,4) must be 3.0: {json}");
    }

    /// An empty batch (no window closed on this push) serialises to `[]`.
    #[test]
    fn empty_batch_is_empty_array() {
        assert_eq!(windows_json(&[]), "[]");
    }

    /// sq-734a — the `f64 → u64` boundary narrowing accepts the JS-`Number`
    /// values the streaming page sends (a whole `60.0`) and the boundary cases (`0`,
    /// `Number.MAX_SAFE_INTEGER`), and rejects fractional / negative / non-finite / too-large
    /// values cleanly. This is the regression guard for the `Cannot convert 60 to a BigInt`
    /// page-load failure: the page passes Numbers, so the boundary must take Numbers.
    #[test]
    fn u64_arg_narrows_js_numbers() {
        // The exact value from the issue: `Rsp.select(q, 60, 60, 0, "rstream")`.
        assert_eq!(u64_arg("range", 60.0).unwrap(), 60);
        assert_eq!(u64_arg("max_delay", 0.0).unwrap(), 0);
        // Boundaries: 2^53-1 round-trips exactly through an f64; one past it does not.
        assert_eq!(
            u64_arg("ts", 9_007_199_254_740_991.0).unwrap(),
            9_007_199_254_740_991
        );
        assert!(u64_arg("ts", 9_007_199_254_740_992.0).is_err());
        // Rejected: fractional, negative, NaN, +inf.
        assert!(u64_arg("ts", 10.5).is_err());
        assert!(u64_arg("ts", -1.0).is_err());
        assert!(u64_arg("ts", f64::NAN).is_err());
        assert!(u64_arg("ts", f64::INFINITY).is_err());
    }
}
