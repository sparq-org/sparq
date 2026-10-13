//! SPARQL 1.1 Federated Query — `SERVICE` evaluation. [OPUS-4.8]
//!
//! `SERVICE <endpoint> { pattern }` evaluates `pattern` against a *remote* SPARQL
//! endpoint and joins the returned solutions back into the surrounding query, per
//! <https://www.w3.org/TR/sparql11-federated-query/>.
//!
//! ## How it works
//!
//! 1. The inner `GraphPattern` is wrapped as `SELECT * WHERE { <inner> }` using
//!    spargebra's `Display` impl (which round-trips algebra → SPARQL syntax), so the
//!    full pattern (BGPs, OPTIONAL, FILTER, sub-SELECT, …) is forwarded.
//!
//!    **Bind-join (`VALUES` pushdown).** When the SERVICE is the right side of a join
//!    (or `OPTIONAL`) whose join variables are already bound by the left side, the
//!    caller (`exec::try_bound_join_service`) pushes a *block* of those bindings into
//!    the wrapped query as a `VALUES` clause — `SELECT * WHERE { VALUES (?j…) { … }
//!    <inner> }` — so the remote returns only the rows that can survive the local join
//!    (the brTPF/FedX "bound join", bead sq-sjkj). This is ON by default and
//!    correctness-preserving; it FALLS BACK to forwarding the bare pattern verbatim
//!    when it does not apply (variable endpoint, no bound join var, a join key bound to
//!    a blank node, …). The block size is the one OPT-IN tuning knob
//!    ([`with_service_bound_join_block_size`] / `SPARQ_SERVICE_BIND_BLOCK`,
//!    default `DEFAULT_BIND_BLOCK`). The remote relation that is NOT bound-joined is
//!    still fetched in full and consumed row-by-row into the caller's id-level join
//!    input (see *Bounded result consumption* below).
//! 2. The query is sent over HTTP (form-encoded POST, `Accept:
//!    application/sparql-results+json, application/sparql-results+xml;q=0.9` — JSON is
//!    preferred but XML is accepted as a fallback).
//! 3. The response is parsed INCREMENTALLY (`parse_results_into`, bead sq-my8wd.4):
//!    each solution row (`Vec<Option<Term>>`, `None` = unbound) is handed to the
//!    caller's row sink AS IT IS PARSED — the parser never materialises the whole
//!    remote relation, and a sink error aborts the parse. The body is content-sniffed:
//!    a leading `{` is parsed as SPARQL-Results-JSON (`parse_srj_into`); a leading
//!    `<` as SPARQL-Results-XML (`parse_srx_into`). The XML path matters because
//!    some endpoints ignore `Accept` and always return XML — without it the whole
//!    SERVICE call would fail (bead sq-ycu).
//! 4. The caller (`exec::eval_service`) interns each row into the local/graph
//!    dictionaries as it arrives — exactly like `VALUES` — dropping the owned terms
//!    immediately, and joins the compact id-level relation with the rest of the query.
//!
//! ## Bounded result consumption (bead sq-my8wd.4)
//!
//! A large (or adversarial) remote result must not exhaust engine memory. The bounds:
//!
//! * the raw response BODY is capped at a finite limit (`SERVICE_MAX_BODY_BYTES`, the
//!   native transport's read limit) — a runaway endpoint cannot stream unbounded bytes;
//! * parsing is per-row streaming for BOTH result formats: the SRJ parser walks the
//!   `results.bindings` array one binding object at a time (never building a whole-
//!   document JSON DOM), and the SRX parser was already event-driven — so the parser's
//!   own working state is one solution, not the relation;
//! * the consumer interns rows to id-level `Bindings` on arrival, so the only
//!   result-sized state is the compact id relation the join itself requires.
//!
//! Result-equivalence with the previous collect-everything path (identical rows,
//! multiplicity AND order, and identical errors on malformed documents) is pinned by
//! the `streaming_equivalence` tests against a frozen DOM reference implementation.
//! Honest boundary: an SRJ document whose `results` precedes its `head` (legal JSON,
//! unseen in practice) degrades to buffering the raw binding objects until `head`
//! arrives — still bounded by the body cap; and on documents with DUPLICATE top-level
//! keys (malformed per RFC 8259 §4) the error/row precedence may reflect document
//! order rather than the old DOM's fixed check order.
//!
//! ## `SERVICE SILENT`
//!
//! Any error (DNS, connection, non-2xx status, malformed body) is swallowed when the
//! pattern is `SILENT`: evaluation yields the join identity (a single empty solution),
//! so the surrounding query keeps its existing bindings rather than failing. Without
//! `SILENT`, the error propagates and fails the query.
//!
//! ## Transport seam (testability)
//!
//! The HTTP call sits behind the [`Transport`] trait. Production uses [`HttpTransport`]
//! (ureq, a tiny blocking client — gated off wasm). Tests inject a canned-response or
//! local-loopback transport, so the SRJ parser and the algebra integration are
//! exercised without a public network dependency.
//!
//! ## `SERVICE ?var` (variable endpoint)
//!
//! `SERVICE ?ep { P }` — where the endpoint is a *variable* — is supported when `?ep` is
//! bound by the surrounding query (bead sq-d4p). The engine evaluates it per SPARQL 1.1
//! semantics (one substituted `SERVICE μ(?ep) { P }` per in-scope solution μ): it
//! partitions the already-evaluated left bindings by their `?ep` value and dispatches one
//! bind-join *per distinct endpoint IRI*, tagging each remote row with its `?ep` so the
//! surrounding join re-attaches it (see `exec::bound_join_variable_endpoint`). A left
//! solution whose `?ep` is UNBOUND or not an IRI names no valid endpoint and contributes
//! no federated answer. A TOP-LEVEL `SERVICE ?ep` with nothing to bind it (no surrounding
//! relation) has no endpoint to call and is still rejected with a clear error (or, under
//! `SILENT`, the empty result) — see `exec::eval_service`.
//!
//! ## Timeout
//!
//! The remote round-trip is bounded by the active `QueryBudget`
//! deadline (bead sq-d4p): [`HttpTransport::with_budget`] caps its socket timeout at the
//! budget's remaining time (never above the built-in `DEFAULT_SERVICE_TIMEOUT`), so a
//! query under a tight deadline does not block for the full default on an unresponsive
//! endpoint. With no deadline installed the built-in default applies.

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple, Variable};

/// Shorthand for the streaming row-sink contract (bead sq-my8wd.4): called once per
/// parsed solution row, positionally projected over the result's variables (`None` =
/// unbound). Returning `Err` ABORTS the parse and the error string is propagated to
/// the caller VERBATIM (so a sink can enforce its own resource policy). [FABLE-5]
///
/// A closure bound rather than a trait object: every call site is monomorphised, so
/// the per-row delivery adds no dynamic dispatch (the #1303 perf-neutrality stance).
pub trait RowSink: FnMut(Vec<Option<Term>>) -> Result<(), String> {}
impl<F: FnMut(Vec<Option<Term>>) -> Result<(), String>> RowSink for F {}

/// Abstracts the HTTP round-trip so tests can inject a fake endpoint. `query` is the
/// SPARQL query string; the return is the raw response body (expected to be
/// SPARQL-Results-JSON) or a transport error string.
pub trait Transport {
    fn fetch(&self, endpoint: &str, query: &str) -> Result<String, String>;
}

/// Streaming transport seam: returns the raw HTTP response body as a `Read` handle
/// so the parser can consume it incrementally WITHOUT buffering it into a `String`
/// first. (bead sq-my8wd.5) [OPUS-4.8] / [FABLE-5]
///
/// The production `HttpTransport` implements this by handing back the ureq response
/// body reader directly (with the same `SERVICE_MAX_BODY_BYTES` body cap enforced by
/// the limit-wrapped reader). Test transports that implement `Transport` get a blanket
/// impl via `TransportAsReader` so the streaming path is exercised without rewriting
/// every canned-mock.
///
/// Compared to `Transport`, the contract change is only in what the body-read lifecycle:
/// * The returned `Read` MUST be fully consumed (or EOF encountered) before the method
///   is called again; it borrows from the response, so the caller may not hold it
///   across calls.
/// * On any I/O error mid-stream the `Read::read` impl returns `Err`. The parser
///   treats any mid-stream error as a SERVICE failure (the same as a transport error).
#[cfg(feature = "service")]
pub trait ReaderTransport {
    /// Send `query` to `endpoint` and return the response body as a `Read`. On a
    /// transport/HTTP error (before any body bytes are available) returns `Err`.
    fn fetch_reader<'a>(
        &'a self,
        endpoint: &str,
        query: &str,
    ) -> Result<Box<dyn std::io::Read + 'a>, String>;
}

/// Adapter: wraps any `Transport` as a `ReaderTransport` by calling `fetch()` and
/// returning an `io::Cursor` over the owned `String` body.  This gives test mocks the
/// streaming path without any changes. [OPUS-4.8] (sq-my8wd.5)
#[cfg(feature = "service")]
pub struct TransportAsReader<'t, T: ?Sized>(pub &'t T);

#[cfg(feature = "service")]
impl<T: Transport + ?Sized> ReaderTransport for TransportAsReader<'_, T> {
    fn fetch_reader<'a>(
        &'a self,
        endpoint: &str,
        query: &str,
    ) -> Result<Box<dyn std::io::Read + 'a>, String> {
        let body = self.0.fetch(endpoint, query)?;
        Ok(Box::new(std::io::Cursor::new(body)))
    }
}

/// Evaluate one SERVICE call end-to-end, STREAMING the parsed rows into `on_row` as
/// they are decoded (bead sq-my8wd.4): send `query` to `endpoint` via `transport`,
/// content-sniff + parse the response incrementally, and return the projected
/// variable list. SILENT handling is the caller's responsibility (it owns the
/// join-identity fallback, and must discard whatever the sink accumulated when this
/// returns `Err`). [FABLE-5]
pub fn eval_remote_into<F: RowSink>(
    transport: &dyn Transport,
    endpoint: &str,
    query: &str,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    let body = transport.fetch(endpoint, query)?;
    parse_results_into(&body, on_row)
}

/// Reader-seam variant of [`eval_remote_into`]: send `query` to `endpoint` via
/// `transport`, receive the response body as a `Read` stream (NOT buffered into a
/// `String`), content-sniff + parse it incrementally, and call `on_row` for each
/// parsed solution. Peak memory stays below the response body size — the body is
/// never materialised in full. [OPUS-4.8] (bead sq-my8wd.5) [FABLE-5]
///
/// SILENT handling is the caller's responsibility (it owns the join-identity fallback,
/// and must discard whatever the sink accumulated when this returns `Err`).
#[cfg(feature = "service")]
pub fn eval_remote_into_read<F: RowSink>(
    transport: &dyn ReaderTransport,
    endpoint: &str,
    query: &str,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    let reader = transport.fetch_reader(endpoint, query)?;
    parse_results_into_read(std::io::BufReader::new(reader), on_row)
}

/// The error for a response body that starts with neither `{` nor `<`.
#[cfg(feature = "service")]
const NOT_A_RESULTS_DOCUMENT: &str =
    "SERVICE: endpoint response is neither SPARQL-Results-JSON nor -XML \
     (expected a leading '{' or '<')";

/// Parse a remote SELECT results document incrementally, content-sniffing JSON vs
/// XML, delivering each row to `on_row` and returning the projected variables.
/// [OPUS-4.8] / streaming form [FABLE-5] (sq-my8wd.4)
///
/// The SPARQL Protocol lets a client advertise an `Accept` preference, but a server MAY
/// ignore it; in practice some endpoints always emit SPARQL-Results-XML even when we ask
/// for JSON (bead sq-ycu). We therefore sniff the first non-whitespace byte rather than
/// trusting any `Content-Type` (which the `Transport` seam does not even surface): `{` ⇒
/// SPARQL-Results-JSON, `<` ⇒ SPARQL-Results-XML. Anything else is an error (or, under
/// `SILENT`, the caller's empty result).
#[cfg(feature = "service")]
pub(crate) fn parse_results_into<F: RowSink>(
    text: &str,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    match text.trim_start().as_bytes().first() {
        Some(b'<') => parse_srx_into(text, on_row),
        Some(b'{') => parse_srj_into(text, on_row),
        // An empty body or a leading byte that is neither `{` nor `<` is not a results
        // document we can parse; report it (SILENT turns this into an empty result).
        _ => Err(NOT_A_RESULTS_DOCUMENT.into()),
    }
}

/// Parse a remote SELECT results document from a `BufRead` stream WITHOUT buffering
/// the full body. Content-sniffs the first non-whitespace byte (identical dispatch
/// logic to [`parse_results_into`]) then delegates to the reader-based JSON or XML
/// parser. (bead sq-my8wd.5) [OPUS-4.8] [FABLE-5]
///
/// The `reader` is consumed incrementally; at no point is the whole body held in
/// memory. A mid-stream I/O error is reported as a SERVICE failure.
#[cfg(feature = "service")]
pub(crate) fn parse_results_into_read<R: std::io::BufRead, F: RowSink>(
    mut reader: R,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    // Content-sniff: advance past leading ASCII whitespace to find the first
    // meaningful byte. We peek into the BufReader's internal buffer without
    // consuming; if the buffer is empty we fill it first.
    loop {
        let buf = reader
            .fill_buf()
            .map_err(|e| format!("SERVICE: reading response: {}", e))?;
        if buf.is_empty() {
            return Err(NOT_A_RESULTS_DOCUMENT.into());
        }
        // Find first non-whitespace byte in this buffer chunk.
        let pos = buf.iter().position(|b| !b.is_ascii_whitespace());
        match pos {
            Some(i) => {
                let sniff = buf[i];
                // Do NOT consume the whitespace prefix here; let the downstream
                // parser see the full (trimmed-start) content. The BufReader has
                // not advanced past `buf[i]`, so the parser starts reading from
                // the correct position — but we must consume the leading whitespace
                // bytes so they are not re-read.
                reader.consume(i);
                return match sniff {
                    b'<' => parse_srx_into_read(reader, on_row),
                    b'{' => parse_srj_into_read(reader, on_row),
                    _ => Err(NOT_A_RESULTS_DOCUMENT.into()),
                };
            }
            None => {
                // The entire buffer chunk was whitespace; consume it and refill.
                let n = buf.len();
                reader.consume(n);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Bind-join (VALUES pushdown) into SERVICE — block size knob [OPUS-4.8] (sq-sjkj)
// ---------------------------------------------------------------------------
//
// When a SERVICE sub-query is the right side of a join whose join variables are
// already bound by the left side, we push a *block* of those bindings into the
// remote query as a `VALUES` clause (SPARQL 1.1 §10.2.1) — one remote request per
// block instead of materialising the whole remote relation and joining locally.
// This is the brTPF/FedX "bound join" (bead sq-sjkj, research candidate C1): for a
// selective join it slashes the data the endpoint returns and the rows we filter.
//
// The pushdown is a *correctness-preserving* optimisation of the existing SERVICE
// path: injecting `VALUES (?j) { (v1) (v2) … }` restricts the remote pattern to
// exactly the bound tuples, and the surrounding query joins the result back the
// same way it joins the unbound relation — so the answer is identical. It is ON by
// default (no new surface, zero downside on the applicable shape) and FALLS BACK to
// the verbatim path whenever it does not apply (variable endpoint, no bound join
// var, a join key bound to a blank node, an empty left side, …). The only TUNING
// KNOB — the block size — is OPT-IN via [`with_service_bound_join_block_size`] /
// the `SPARQ_SERVICE_BIND_BLOCK` env var; the default suits typical workloads.

/// Default bind-join block size: how many distinct binding tuples are pushed into
/// one remote `VALUES` request. ~50 mirrors FedX's default bound-join batch — large
/// enough to amortise the per-request round-trip, small enough to keep the injected
/// query (and the remote's VALUES-join fan-out) bounded.
pub(crate) const DEFAULT_BIND_BLOCK: usize = 50;

#[cfg(feature = "service")]
mod bind_block {
    use std::cell::Cell;

    thread_local! {
        // `None` => use the env / built-in default. A scope installs an override.
        static OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    }

    /// RAII override of the bind-join block size for the current scope.
    pub(crate) struct Guard(Option<usize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0.take()));
        }
    }

    pub(crate) fn install(n: usize) -> Guard {
        // A zero block size would mean "never batch" — clamp to 1 so a tuple still
        // gets pushed one-per-request rather than silently disabling correctness.
        let n = n.max(1);
        Guard(OVERRIDE.with(|o| o.replace(Some(n))))
    }

    /// The active block size: an installed scope override wins; otherwise the
    /// `SPARQ_SERVICE_BIND_BLOCK` env var (parsed once is unnecessary — this is off
    /// the hot path, called once per bound-join); otherwise the built-in default.
    pub(crate) fn current() -> usize {
        if let Some(n) = OVERRIDE.with(|o| o.get()) {
            return n;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(s) = std::env::var("SPARQ_SERVICE_BIND_BLOCK") {
            if let Ok(n) = s.trim().parse::<usize>() {
                if n >= 1 {
                    return n;
                }
            }
        }
        super::DEFAULT_BIND_BLOCK
    }
}

/// The bind-join block size in force for the current scope. [OPUS-4.8] (sq-sjkj)
#[cfg(feature = "service")]
pub fn bind_block_size() -> usize {
    bind_block::current()
}

/// Runs `f` with `n` as the SERVICE bind-join block size (how many distinct binding
/// tuples are pushed into one remote `VALUES` request). OPT-IN tuning knob for the
/// bound-join pushdown (bead sq-sjkj): the default (~50, or `SPARQ_SERVICE_BIND_BLOCK`)
/// suits typical workloads, but a very selective or very fan-out join can benefit
/// from a larger or smaller block. `n` is clamped to at least 1. The override is
/// thread-local and restored on return/unwind, mirroring [`with_service_egress_allow`].
///
/// This knob does NOT change results — it only trades remote-request count against
/// per-request size; the bound-join is correctness-preserving at any block size.
///
/// ```no_run
/// # #[cfg(feature = "service")] {
/// sparq_engine_service::service::with_service_bound_join_block_size(200, || {
///     // ... run a federated query with large bound-join blocks
/// });
/// # }
/// ```
#[cfg(feature = "service")]
pub fn with_service_bound_join_block_size<R>(n: usize, f: impl FnOnce() -> R) -> R {
    let _guard = bind_block::install(n);
    f()
}

// ---------------------------------------------------------------------------
// Per-query remote-request cap for high-cardinality `SERVICE ?ep` [OPUS-4.8] (sq-b93pv)
// ---------------------------------------------------------------------------
//
// A `SERVICE ?ep { P }` whose endpoint variable binds to MANY distinct endpoint IRIs
// fans out into one remote dispatch per distinct endpoint (see
// `exec::bound_join_variable_endpoint`). With nothing bounding that fan-out, an
// attacker-shaped query — or simply a large left relation whose `?ep` column is highly
// distinct — turns a single client request into an unbounded burst of outbound HTTP
// calls from the engine host (threat-model B4, the SSRF/egress family). This is the
// amplification dimension the per-request egress allowlist does NOT cover: every dialled
// host may be individually permitted yet the COUNT still runaway.
//
// The cap is an OPT-IN ceiling on the number of distinct remote endpoints a single
// `SERVICE ?ep` evaluation may dispatch to. It is enforced PRE-HTTP — at plan/eval
// time, once the distinct-endpoint set is known and BEFORE the first socket is opened —
// so it BOUNDS the runaway rather than cancelling it after the requests have already
// gone out. Exceeding it is a hard, typed refusal (the [`SERVICE_REMOTE_CAP_MARKER`]
// substring), fail-closed: it is NOT swallowed by `SILENT`, because `SILENT` means "a
// remote endpoint being unreachable/broken must not fail the query", not "my own
// resource policy refusing the query may be masked as success". This mirrors the
// `"query budget exceeded"` guard, which `SILENT` likewise does not swallow.
//
// DEFAULT is OFF (no cap): a normal `SERVICE` query — concrete endpoint, or a variable
// endpoint with a handful of distinct values — is entirely unchanged. The cap only
// exists once an embedder/server opts in via [`with_service_remote_request_cap`] or the
// `SPARQ_SERVICE_REMOTE_CAP` env var.

/// Stable marker substring embedded in the engine error string when a `SERVICE ?ep`
/// query is refused for exceeding the per-query remote-request cap. [OPUS-4.8] (sq-b93pv)
///
/// Mirrors [`SERVICE_EGRESS_REFUSED_MARKER`]: a network-exposed host (`sparq-server`)
/// can `contains()`-classify the refusal as a deliberate resource-policy decision (an
/// honest `429`/`403`-style status) rather than a server fault, and it is deliberately
/// generic (it names no endpoint) so it is safe to surface to the client.
pub const SERVICE_REMOTE_CAP_MARKER: &str = "SERVICE remote-request cap exceeded";

#[cfg(feature = "service")]
mod remote_cap {
    use std::cell::Cell;

    thread_local! {
        // `None` => no scope override; consult the env var, else the built-in (OFF).
        static OVERRIDE: Cell<Option<Option<usize>>> = const { Cell::new(None) };
    }

    /// RAII override of the remote-request cap for the current scope.
    pub(crate) struct Guard(Option<Option<usize>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0.take()));
        }
    }

    /// Installs `cap` (`Some(n)` = cap at `n`; `None` = explicitly UNCAPPED for this
    /// scope, overriding any env var) and returns a guard that restores the previous
    /// override on drop/unwind.
    pub(crate) fn install(cap: Option<usize>) -> Guard {
        Guard(OVERRIDE.with(|o| o.replace(Some(cap))))
    }

    /// The active cap: an installed scope override wins (including an explicit `None`
    /// "uncapped"); otherwise the `SPARQ_SERVICE_REMOTE_CAP` env var; otherwise OFF
    /// (`None` = no cap). Off the hot path — called once per `SERVICE ?ep` evaluation.
    pub(crate) fn current() -> Option<usize> {
        if let Some(scope) = OVERRIDE.with(|o| o.get()) {
            return scope;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(s) = std::env::var("SPARQ_SERVICE_REMOTE_CAP") {
            if let Ok(n) = s.trim().parse::<usize>() {
                return Some(n);
            }
        }
        None
    }
}

/// The per-query SERVICE remote-request cap in force for the current scope, or `None`
/// when no cap is active (the default). [OPUS-4.8] (sq-b93pv)
#[cfg(feature = "service")]
pub fn remote_request_cap() -> Option<usize> {
    remote_cap::current()
}

/// Runs `f` with a ceiling of `n` distinct remote endpoints per `SERVICE ?ep`
/// evaluation. [OPUS-4.8] (sq-b93pv)
///
/// A `SERVICE ?ep { P }` whose endpoint variable binds to MANY distinct endpoint IRIs
/// dispatches one remote bind-join per distinct endpoint. This OPT-IN cap bounds that
/// fan-out: if the number of distinct endpoints the query would dial exceeds `n`, the
/// query is REJECTED with a typed error (carrying [`SERVICE_REMOTE_CAP_MARKER`]) BEFORE
/// any HTTP request is sent — a true pre-dispatch bound, not a post-hoc cancellation.
///
/// The cap is **not** swallowed by `SERVICE SILENT`: `SILENT` masks an endpoint being
/// unreachable, not a deliberate resource-policy refusal (the same stance the query
/// budget takes). It is also a no-op for a concrete-IRI `SERVICE <…>` (a single
/// endpoint) and for a variable endpoint that binds to at most `n` distinct IRIs, so a
/// normal federated query is unaffected.
///
/// The default (no enclosing scope and no `SPARQ_SERVICE_REMOTE_CAP` env var) is
/// UNCAPPED — the behaviour every existing `SERVICE` query already had. Passing `n = 0`
/// caps at zero, which refuses any `SERVICE ?ep` that resolves to one or more
/// endpoints. The override is thread-local and restored on return/unwind, mirroring
/// [`with_service_egress_allow`].
///
/// ```no_run
/// # #[cfg(feature = "service")] {
/// // Allow a high-cardinality SERVICE ?ep to dial at most 8 distinct endpoints.
/// sparq_engine_service::service::with_service_remote_request_cap(8, || {
///     // ... run a federated query containing `SERVICE ?ep { ... }`
/// });
/// # }
/// ```
#[cfg(feature = "service")]
pub fn with_service_remote_request_cap<R>(n: usize, f: impl FnOnce() -> R) -> R {
    let _guard = remote_cap::install(Some(n));
    f()
}

/// Renders a `VALUES` block that binds `vars` to each tuple in `tuples`, in the
/// SPARQL 1.1 syntax accepted inside a group graph pattern. [OPUS-4.8] (sq-sjkj)
///
/// Each term is emitted via its canonical N-Triples/Turtle form (oxrdf `Display`),
/// which is valid SPARQL term syntax for IRIs and literals (the only kinds the
/// caller pushes — see [`pushable_term`]). Single-variable blocks use the short
/// `VALUES ?v { v1 v2 }` form; multi-variable blocks use the parenthesised
/// `VALUES (?a ?b) { (a1 b1) (a2 b2) }` form. `UNDEF` is never emitted: the caller
/// only pushes fully-bound tuples (a tuple with an unbound join var falls back to
/// the verbatim path), so every cell is a concrete term.
#[cfg(feature = "service")]
pub fn render_values_block(vars: &[Variable], tuples: &[Vec<Term>]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    if vars.len() == 1 {
        let _ = write!(s, "VALUES {} {{", vars[0]);
        for t in tuples {
            let _ = write!(s, " {}", t[0]);
        }
        s.push_str(" }");
    } else {
        s.push_str("VALUES (");
        for (i, v) in vars.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{v}");
        }
        s.push_str(") {");
        for tuple in tuples {
            s.push_str(" (");
            for (i, t) in tuple.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                let _ = write!(s, "{t}");
            }
            s.push(')');
        }
        s.push_str(" }");
    }
    s
}

/// Whether a `Term` may be pushed as a `VALUES` data value into a remote query.
/// [OPUS-4.8] (sq-sjkj)
///
/// IRIs and literals are pushable (their `Display` is valid SPARQL term syntax and
/// their identity is global). A **blank node** is NOT: blank-node labels are scoped
/// to a single result document, so a local bnode label is meaningless to the remote
/// endpoint — pushing it would silently change semantics. A **triple term** (RDF 1.2
/// `<<( … )>>`) is conservatively excluded too: not every endpoint accepts it in
/// VALUES, and a join key is rarely a triple term. When a join-key tuple contains
/// any non-pushable term the caller abandons the bound-join for the verbatim path,
/// preserving exact semantics.
#[cfg(feature = "service")]
pub fn pushable_term(t: &Term) -> bool {
    matches!(t, Term::NamedNode(_) | Term::Literal(_))
}

// ---------------------------------------------------------------------------
// SPARQL Results JSON parser
// (https://www.w3.org/TR/sparql11-results-json/)
// ---------------------------------------------------------------------------

/// Parse a SELECT result document INCREMENTALLY (bead sq-my8wd.4): the
/// `results.bindings` array is walked one binding object at a time through a custom
/// serde seed (never building a whole-document JSON DOM), each row is projected
/// positionally over `head.vars` and handed to `on_row` immediately, and the
/// projected variables are returned. ASK results (`{"boolean": …}`) are reported as
/// an error — `SERVICE { … }` always wraps a SELECT in our forwarding, so a boolean
/// body indicates a misbehaving endpoint. [FABLE-5]
///
/// Result-equivalent to the previous whole-DOM parse (same rows, order, multiplicity
/// and errors — pinned by the `streaming_equivalence` tests), with two documented
/// pathological-input caveats (see the module doc): a `results`-before-`head`
/// document buffers raw binding objects until `head` arrives, and duplicate top-level
/// keys (malformed JSON) resolve in document order rather than DOM check order.
#[cfg(feature = "service")]
pub fn parse_srj_into<F: RowSink>(
    text: &str,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    parse_srj_with(serde_json::Deserializer::from_str(text), on_row)
}

/// Reader-seam variant of [`parse_srj_into`]: parse a SPARQL-Results-JSON document
/// from a `Read` stream WITHOUT buffering the body, calling `on_row` per solution.
/// Uses `serde_json::Deserializer::from_reader`, so the only in-memory state is one
/// binding object at a time — identical streaming guarantees to [`parse_srj_into`],
/// extended to the HTTP body read. (bead sq-my8wd.5) [OPUS-4.8] [FABLE-5]
///
/// Result-semantics are IDENTICAL to [`parse_srj_into`] on the same byte sequence:
/// same rows, order, multiplicity, and errors (the `srj_stream` seed logic is shared).
#[cfg(feature = "service")]
pub(crate) fn parse_srj_into_read<R: std::io::Read, F: RowSink>(
    reader: R,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    parse_srj_with(serde_json::Deserializer::from_reader(reader), on_row)
}

/// The SPARQL-Results-JSON driver behind [`parse_srj_into`] and [`parse_srj_into_read`].
#[cfg(feature = "service")]
fn parse_srj_with<'de, R: serde_json::de::Read<'de>, F: RowSink>(
    mut de: serde_json::Deserializer<R>,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    use serde::de::DeserializeSeed;

    let mut st = srj_stream::State::new(on_row);
    if let Err(e) = srj_stream::TopSeed(&mut st).deserialize(&mut de) {
        // A semantic error (bad term / bad variable / sink refusal) was smuggled out
        // through the side channel VERBATIM; anything else is a JSON syntax error and
        // gets the same wrapping the DOM path used.
        return Err(st
            .take_fail()
            .unwrap_or_else(|| format!("SERVICE: invalid results JSON: {}", e)));
    }
    de.end()
        .map_err(|e| format!("SERVICE: invalid results JSON: {}", e))?;
    // Post-parse checks in the SAME precedence order as the old DOM path: ASK boolean
    // first, then a missing/invalid `head.vars`, then a missing `results.bindings`.
    st.finish()
}

/// Streaming serde seeds for SPARQL-Results-JSON (bead sq-my8wd.4). [FABLE-5]
///
/// The document shape is `{"head": {"vars": […]}, "results": {"bindings": […]}}`.
/// Only the `bindings` ARRAY is result-sized, so that is the one place a streaming
/// seed walks elements one at a time; `head` (and each individual binding object) is
/// small and is still read through a `serde_json::Value` so the term reconstruction
/// (`srj_term`) and the `head.vars` extraction are byte-identical to the old DOM
/// path. Every other value shape is accepted-and-ignored exactly where the DOM path
/// tolerated it, so the "missing head.vars" / "missing results.bindings" errors fire
/// under the same conditions.
#[cfg(feature = "service")]
mod srj_stream {
    use super::{srj_term, RowSink};
    use oxrdf::{Term, Variable};
    use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use serde_json::Value;
    use std::fmt;

    /// Shared parse state threaded through the seeds.
    pub(super) struct State<'f, F> {
        on_row: &'f mut F,
        /// `head.vars`, once seen (rows are projected positionally over these).
        vars: Option<Vec<Variable>>,
        /// Raw binding objects seen BEFORE `head` (the reversed-order fallback);
        /// flushed the moment `head.vars` arrives. Bounded by the response-body cap.
        pending: Vec<Value>,
        /// Whether a `results.bindings` ARRAY was seen (empty is fine).
        bindings_seen: bool,
        /// Whether a top-level `boolean` key was seen (an ASK body — rejected).
        boolean_seen: bool,
        /// Side channel for semantic errors: serde's `Error::custom` would append
        /// position info, so the exact message is smuggled out here instead.
        fail: Option<String>,
    }

    impl<'f, F: RowSink> State<'f, F> {
        pub(super) fn new(on_row: &'f mut F) -> Self {
            State {
                on_row,
                vars: None,
                pending: Vec::new(),
                bindings_seen: false,
                boolean_seen: false,
                fail: None,
            }
        }

        pub(super) fn take_fail(&mut self) -> Option<String> {
            self.fail.take()
        }

        /// The post-parse checks, in the old DOM path's precedence order.
        pub(super) fn finish(self) -> Result<Vec<Variable>, String> {
            if self.boolean_seen {
                return Err(
                    "SERVICE: endpoint returned an ASK boolean, expected SELECT bindings".into(),
                );
            }
            let vars = self
                .vars
                .ok_or_else(|| "SERVICE: results JSON missing head.vars".to_string())?;
            if !self.bindings_seen {
                return Err("SERVICE: results JSON missing results.bindings".to_string());
            }
            Ok(vars)
        }

        /// Record `msg` in the side channel and produce the serde error that aborts
        /// the deserialisation (the caller surfaces the side channel verbatim).
        fn bail<E: serde::de::Error>(&mut self, msg: String) -> E {
            let e = E::custom(&msg);
            self.fail = Some(msg);
            e
        }

        /// Project one binding object over `vars` and hand the row to the sink.
        /// Identical cell handling to the old DOM path (absent variable ⇒ `None`).
        fn emit(&mut self, sol: &Value) -> Result<(), String> {
            let row = {
                let vars = self.vars.as_ref().expect("emit is only called once vars are known");
                let obj = sol.as_object().ok_or_else(|| {
                    "SERVICE: a solution binding is not a JSON object".to_string()
                })?;
                let mut row: Vec<Option<Term>> = Vec::with_capacity(vars.len());
                for var in vars {
                    match obj.get(var.as_str()) {
                        Some(cell) => row.push(Some(srj_term(cell)?)),
                        None => row.push(None),
                    }
                }
                row
            };
            (self.on_row)(row)
        }
    }

    /// Implements the non-target `Visitor` shapes as accept-and-ignore, so a seed
    /// tolerates any JSON type exactly where the DOM path did (leaving the relevant
    /// `*_seen` flag unset ⇒ the same "missing …" error at the end).
    macro_rules! ignore_scalars {
        () => {
            fn visit_bool<E: serde::de::Error>(self, _v: bool) -> Result<(), E> {
                Ok(())
            }
            fn visit_i64<E: serde::de::Error>(self, _v: i64) -> Result<(), E> {
                Ok(())
            }
            fn visit_u64<E: serde::de::Error>(self, _v: u64) -> Result<(), E> {
                Ok(())
            }
            fn visit_f64<E: serde::de::Error>(self, _v: f64) -> Result<(), E> {
                Ok(())
            }
            fn visit_str<E: serde::de::Error>(self, _v: &str) -> Result<(), E> {
                Ok(())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                Ok(())
            }
        };
    }

    /// Drain-and-ignore a sequence / map (the value is structurally skipped).
    fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }
    fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }

    /// Top-level document seed: `{"head": …, "results": …, "boolean": …, …}`.
    pub(super) struct TopSeed<'a, 'f, F>(pub &'a mut State<'f, F>);

    impl<'de, F: RowSink> DeserializeSeed<'de> for TopSeed<'_, '_, F> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            d.deserialize_any(self)
        }
    }

    impl<'de, F: RowSink> Visitor<'de> for TopSeed<'_, '_, F> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a SPARQL results document")
        }
        // A non-object top level parses, binds nothing, and yields the DOM path's
        // "missing head.vars" from the post-checks.
        ignore_scalars!();
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<(), A::Error> {
            drain_seq(seq)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "head" => {
                        // `head` is small; a Value keeps the vars extraction
                        // byte-identical to the DOM path (non-string entries are
                        // SKIPPED, a bad variable NAME is an error).
                        let hv: Value = map.next_value()?;
                        let extracted = hv
                            .get("vars")
                            .and_then(|a| a.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|s| s.as_str())
                                    .map(|s| {
                                        Variable::new(s).map_err(|e| {
                                            format!(
                                                "SERVICE: bad result variable {:?}: {}",
                                                s, e
                                            )
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .transpose()
                            .map_err(|m| self.0.bail(m))?;
                        if let Some(vars) = extracted {
                            self.0.vars = Some(vars);
                            // Flush any rows that arrived before `head` (reversed-order
                            // documents), in their original order.
                            for sol in std::mem::take(&mut self.0.pending) {
                                self.0.emit(&sol).map_err(|m| self.0.bail(m))?;
                            }
                        }
                    }
                    "results" => map.next_value_seed(ResultsSeed(&mut *self.0))?,
                    "boolean" => {
                        self.0.boolean_seen = true;
                        let _: IgnoredAny = map.next_value()?;
                    }
                    // Unknown members (e.g. `link`) are ignored, as in the DOM path.
                    _ => {
                        let _: IgnoredAny = map.next_value()?;
                    }
                }
            }
            Ok(())
        }
    }

    /// Seed for the `results` member: only its `bindings` key matters.
    struct ResultsSeed<'a, 'f, F>(&'a mut State<'f, F>);

    impl<'de, F: RowSink> DeserializeSeed<'de> for ResultsSeed<'_, '_, F> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            d.deserialize_any(self)
        }
    }

    impl<'de, F: RowSink> Visitor<'de> for ResultsSeed<'_, '_, F> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a SPARQL results member")
        }
        // A non-object `results` leaves `bindings_seen` unset ⇒ the DOM path's
        // "missing results.bindings".
        ignore_scalars!();
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<(), A::Error> {
            drain_seq(seq)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key.as_str() == "bindings" {
                    map.next_value_seed(BindingsSeed(&mut *self.0))?;
                } else {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
            Ok(())
        }
    }

    /// Seed for the `bindings` array — THE streaming site: one binding object is
    /// deserialised (as a small `Value`), projected and delivered at a time, so the
    /// parser's working state is a single solution, never the relation.
    struct BindingsSeed<'a, 'f, F>(&'a mut State<'f, F>);

    impl<'de, F: RowSink> DeserializeSeed<'de> for BindingsSeed<'_, '_, F> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            d.deserialize_any(self)
        }
    }

    impl<'de, F: RowSink> Visitor<'de> for BindingsSeed<'_, '_, F> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a SPARQL bindings array")
        }
        // A non-array `bindings` leaves `bindings_seen` unset ⇒ "missing
        // results.bindings", matching the DOM path's `as_array` guard.
        ignore_scalars!();
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<(), A::Error> {
            drain_map(map)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            self.0.bindings_seen = true;
            while let Some(sol) = seq.next_element::<Value>()? {
                if self.0.vars.is_some() {
                    self.0.emit(&sol).map_err(|m| self.0.bail(m))?;
                } else {
                    // `head` has not arrived yet (reversed-order document): buffer the
                    // raw binding for the flush at `head` — bounded by the body cap.
                    self.0.pending.push(sol);
                }
            }
            Ok(())
        }
    }
}

/// Reconstruct one term from an SRJ binding value object. Mirrors the conformance
/// suite's `srj_term` (uri / bnode / literal / SPARQL-1.2 triple terms). Public so
/// `sparq-fedclient` decodes remote results with the same function as SERVICE.
#[cfg(feature = "service")]
pub fn srj_term(val: &serde_json::Value) -> Result<Term, String> {
    let get = |k: &str| val.get(k).and_then(|s| s.as_str());
    match get("type") {
        Some("uri") => {
            let iri = get("value").unwrap_or_default();
            Ok(Term::NamedNode(
                NamedNode::new(iri).map_err(|e| format!("SERVICE: bad IRI {iri:?}: {e}"))?,
            ))
        }
        Some("bnode") => {
            let id = get("value").unwrap_or_default();
            Ok(Term::BlankNode(
                BlankNode::new(id).map_err(|e| format!("SERVICE: bad bnode {id:?}: {e}"))?,
            ))
        }
        // Both "literal" and the legacy "typed-literal" (pre-2013 endpoints) map here.
        Some("literal") | Some("typed-literal") | None => {
            let value =
                get("value").ok_or_else(|| "SERVICE: literal binding without value".to_string())?;
            literal(
                value.to_string(),
                get("xml:lang"),
                get("its:dir"),
                get("datatype"),
            )
        }
        Some("triple") => {
            let v = val
                .get("value")
                .ok_or_else(|| "SERVICE: triple term without value".to_string())?;
            let part = |k: &str| -> Result<Term, String> {
                srj_term(
                    v.get(k)
                        .ok_or_else(|| format!("SERVICE: triple term without {k}"))?,
                )
            };
            triple_term(part("subject")?, part("predicate")?, part("object")?)
        }
        Some(other) => Err(format!("SERVICE: unknown binding type {other:?}")),
    }
}

/// A literal from a results binding's lexical value and its optional `xml:lang`, `its:dir`
/// and `datatype`, shared by the SRJ and SRX readers.
///
/// [OPUS-4.8] sq-s955: a SPARQL 1.2 result carries the RDF 1.2 base direction as a separate
/// `its:dir` next to the bare language tag, so a language-tagged value with a valid direction
/// becomes a `dirLangString`. The direction is validated by `dict::parse_base_direction`, the
/// same rule the stored-slot, materialised and outbound paths use; an absent or invalid
/// `its:dir` degrades to a plain language-tagged literal, as `split_lang_dir` /
/// `reconstruct_ref` do for a malformed stored slot.
#[cfg(feature = "service")]
fn literal(
    value: String,
    lang: Option<&str>,
    dir: Option<&str>,
    datatype: Option<&str>,
) -> Result<Term, String> {
    if let Some(lang) = lang {
        let bad_lang = |e| format!("SERVICE: bad language tag {lang:?}: {e}");
        Ok(Term::Literal(
            match dir.and_then(sparq_core::dict::parse_base_direction) {
                Some(dir) => Literal::new_directional_language_tagged_literal(value, lang, dir)
                    .map_err(bad_lang)?,
                None => Literal::new_language_tagged_literal(value, lang).map_err(bad_lang)?,
            },
        ))
    } else if let Some(dt) = datatype {
        let dt = NamedNode::new(dt).map_err(|e| format!("SERVICE: bad datatype {dt:?}: {e}"))?;
        Ok(Term::Literal(Literal::new_typed_literal(value, dt)))
    } else {
        Ok(Term::Literal(Literal::new_simple_literal(value)))
    }
}

/// A SPARQL 1.2 triple term from its parsed parts, shared by the SRJ and SRX readers: the
/// subject must be an IRI or blank node and the predicate an IRI.
#[cfg(feature = "service")]
fn triple_term(subject: Term, predicate: Term, object: Term) -> Result<Term, String> {
    let subject = match subject {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
        other => return Err(format!("SERVICE: invalid triple-term subject: {other}")),
    };
    let Term::NamedNode(predicate) = predicate else {
        return Err(format!(
            "SERVICE: invalid triple-term predicate: {predicate}"
        ));
    };
    Ok(Term::Triple(Box::new(Triple {
        subject,
        predicate,
        object,
    })))
}

// ---------------------------------------------------------------------------
// SPARQL-Results-XML (SRX) parsing [OPUS-4.8] (bead sq-ycu)
// ---------------------------------------------------------------------------
//
// The JSON path (`parse_srj`) is the preferred format, but some endpoints ignore the
// `Accept` header and only ever emit SPARQL-Results-XML
// (<https://www.w3.org/TR/rdf-sparql-XMLres/>, with the SPARQL 1.2 `<triple>` extension).
// This parser is the streaming-event analogue of the conformance suite's `parse_srx`
// (`sparq-conformance/src/results.rs`) — same quick-xml event handling, same predefined-
// entity decode discipline — but it (a) takes a `&str` body rather than a file path, and
// (b) projects each `<result>` *positionally* over the declared `<variable>` list (an
// absent binding ⇒ `None`), so it yields a `ServiceRelation` byte-for-byte compatible with
// `parse_srj`. An ASK `<boolean>` body is rejected, exactly like `parse_srj`, because
// SERVICE always wraps a SELECT.

/// Resolve one XML entity reference name (as quick-xml 0.40 hands it out in a
/// `Event::GeneralRef`, i.e. WITHOUT the surrounding `&`/`;`) to its replacement text:
/// the five predefined named entities (`amp`/`lt`/`gt`/`quot`/`apos`) and numeric
/// character references (`#38` decimal, `#x26` hex). [OPUS-4.8]
#[cfg(feature = "service")]
fn resolve_xml_entity(name: &str) -> Result<String, String> {
    if let Some(rep) = quick_xml::escape::resolve_predefined_entity(name) {
        return Ok(rep.to_string());
    }
    if let Some(rest) = name.strip_prefix('#') {
        let cp = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16)
        } else {
            rest.parse::<u32>()
        }
        .map_err(|_| format!("SERVICE: bad numeric character reference &{name};"))?;
        return char::from_u32(cp)
            .map(|c| c.to_string())
            .ok_or_else(|| format!("SERVICE: numeric character reference &{name}; out of range"));
    }
    Err(format!("SERVICE: unknown XML entity &{name};"))
}

/// The open SRX value element (`<uri>`, `<bnode>` or `<literal>`): its kind, its literal
/// attributes, and the character data collected so far.
#[cfg(feature = "service")]
struct SrxValue {
    kind: &'static str,
    lang: Option<String>,
    dir: Option<String>,
    datatype: Option<String>,
    text: String,
}

#[cfg(feature = "service")]
impl SrxValue {
    /// The term this element denotes: the SRX counterpart of `srj_term`'s leaf arms.
    fn into_term(self) -> Result<Term, String> {
        let text = self.text;
        match self.kind {
            "uri" => Ok(Term::NamedNode(
                NamedNode::new(&text).map_err(|e| format!("SERVICE: bad IRI {text:?}: {e}"))?,
            )),
            "bnode" => {
                Ok(Term::BlankNode(BlankNode::new(&text).map_err(|e| {
                    format!("SERVICE: bad bnode {text:?}: {e}")
                })?))
            }
            _ => literal(
                text,
                self.lang.as_deref(),
                self.dir.as_deref(),
                self.datatype.as_deref(),
            ),
        }
    }
}

/// Parse a SPARQL-Results-XML SELECT document incrementally, delivering each row to
/// `on_row` at its closing `</result>` and returning the declared variables. ASK
/// `<boolean>` bodies are rejected (SERVICE always wraps a SELECT in our forwarding).
/// The quick-xml event loop was already streaming; the only sq-my8wd.4 change is that
/// a finished row goes to the sink instead of a collected `Vec` — same projection,
/// same order, same errors. [FABLE-5]
///
/// In memory this borrows events straight from `text`; [`parse_srx_into_read`] runs the
/// same loop over a `BufRead` stream, so a response body is never buffered into a
/// `String` first (bead sq-my8wd.5).
#[cfg(feature = "service")]
pub(crate) fn parse_srx_into<F: RowSink>(
    text: &str,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(false);
    parse_srx_events(&mut reader, on_row)
}

/// Streaming form of [`parse_srx_into`].
#[cfg(feature = "service")]
pub(crate) fn parse_srx_into_read<R: std::io::BufRead, F: RowSink>(
    reader: R,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    let mut reader = quick_xml::Reader::from_reader(reader);
    reader.config_mut().trim_text(false);
    parse_srx_events(&mut (reader, Vec::new()), on_row)
}

/// The XML event source behind [`parse_srx_events`]: zero-copy over a `&[u8]`, or a
/// reader plus the scratch buffer its events are decoded into.
#[cfg(feature = "service")]
trait SrxEvents {
    fn next_event(&mut self) -> Result<quick_xml::events::Event<'_>, quick_xml::Error>;
}

#[cfg(feature = "service")]
impl SrxEvents for quick_xml::Reader<&[u8]> {
    fn next_event(&mut self) -> Result<quick_xml::events::Event<'_>, quick_xml::Error> {
        self.read_event()
    }
}

#[cfg(feature = "service")]
impl<R: std::io::BufRead> SrxEvents for (quick_xml::Reader<R>, Vec<u8>) {
    fn next_event(&mut self) -> Result<quick_xml::events::Event<'_>, quick_xml::Error> {
        self.1.clear();
        self.0.read_event_into(&mut self.1)
    }
}

/// The namespace of the SPARQL Query Results XML Format.
#[cfg(feature = "service")]
const SRX_NS: &[u8] = b"http://www.w3.org/2005/sparql-results#";

/// The W3C Internationalization Tag Set namespace, which SPARQL 1.2's `its:dir` is in.
#[cfg(feature = "service")]
const ITS_NS: &[u8] = b"http://www.w3.org/2005/11/its";

/// A declared namespace, as far as results XML cares.
#[cfg(feature = "service")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SrxNs {
    Results,
    Its,
    /// `xmlns=""`: unprefixed elements are back in no namespace.
    Unbound,
    Other,
}

/// The namespace declarations in scope while reading results XML. Declarations are taken
/// from the one pass the reader makes over each tag's attributes; a full
/// namespace-resolving reader costs more on this hot path.
#[cfg(feature = "service")]
struct SrxScopes {
    depth: usize,
    /// `(depth of the declaring element, prefix (empty for the default), namespace)`.
    bindings: Vec<(usize, Vec<u8>, SrxNs)>,
    /// Whether an unprefixed element is a results element, kept current as `bindings`
    /// changes, so the common unprefixed element needs no lookup.
    unprefixed_ours: bool,
}

#[cfg(feature = "service")]
impl Default for SrxScopes {
    fn default() -> Self {
        SrxScopes { depth: 0, bindings: Vec::new(), unprefixed_ours: true }
    }
}

#[cfg(feature = "service")]
impl SrxScopes {
    /// Enters an element; its declarations follow through [`Self::declare`].
    fn enter(&mut self) {
        self.depth += 1;
    }

    /// Takes in the entered element's `xmlns` (empty `prefix`) or `xmlns:prefix`
    /// declaration of `ns`, its attribute value with references already replaced.
    fn declare(&mut self, prefix: &[u8], ns: &[u8]) {
        let ns = match ns {
            SRX_NS => SrxNs::Results,
            ITS_NS => SrxNs::Its,
            b"" if prefix.is_empty() => SrxNs::Unbound,
            _ => SrxNs::Other,
        };
        self.bindings.push((self.depth, prefix.to_vec(), ns));
        if prefix.is_empty() {
            self.unprefixed_ours = matches!(ns, SrxNs::Results | SrxNs::Unbound);
        }
    }

    /// Whether `name`, inside the innermost open element, is a results element: its
    /// prefix binds the results namespace, or it is unprefixed and no default namespace
    /// is declared (an endpoint that omits `xmlns`). An undeclared prefix is foreign.
    fn ours(&self, name: quick_xml::name::QName<'_>) -> bool {
        // Element names are short: a plain scan beats `QName::prefix`'s vectorised search.
        let name = name.into_inner();
        match name.iter().position(|&c| c == b':') {
            Some(colon) => self.lookup(&name[..colon]) == Some(SrxNs::Results),
            None => self.unprefixed_ours,
        }
    }

    /// The namespace the innermost declaration of `prefix` binds.
    fn lookup(&self, prefix: &[u8]) -> Option<SrxNs> {
        self.bindings.iter().rev().find(|(_, p, _)| p == prefix).map(|b| b.2)
    }

    /// Leaves the innermost open element, dropping its declarations.
    fn close(&mut self) {
        if self.bindings.last().is_some_and(|b| b.0 == self.depth) {
            while self.bindings.last().is_some_and(|b| b.0 == self.depth) {
                self.bindings.pop();
            }
            self.unprefixed_ours =
                matches!(self.lookup(&[]), None | Some(SrxNs::Results | SrxNs::Unbound));
        }
        self.depth = self.depth.saturating_sub(1);
    }
}

/// The deepest triple-term nesting an SRX document may carry. A nested term is built and later
/// dropped recursively, so its depth must be bounded; this is the depth the SRJ reader admits,
/// whose bound comes from `serde_json`'s nesting limit (pinned by a test on both readers).
#[cfg(feature = "service")]
const MAX_SRX_TRIPLE_DEPTH: usize = 61;

#[cfg(feature = "service")]
fn parse_srx_events<E: SrxEvents, F: RowSink>(
    events: &mut E,
    on_row: &mut F,
) -> Result<Vec<Variable>, String> {
    use quick_xml::events::Event;

    let mut vars: Vec<Variable> = Vec::new();
    // The current row, positional over `vars`: a binding for an undeclared variable is
    // ignored, a repeated binding keeps the last term, and a missing one is UNBOUND.
    let mut cur_row: Vec<Option<Term>> = Vec::new();
    // The column of the open `<binding name=…>` (the first declared variable of that name).
    let mut cur_var: Option<usize> = None;
    let mut cur_val: Option<SrxValue> = None;
    // SPARQL 1.2 `<triple>` nesting: each frame is (active slot, [s, p, o]).
    let mut triple_stack: Vec<(usize, [Option<Term>; 3])> = Vec::new();
    let mut in_boolean = false;
    let mut boolean: Option<bool> = None;
    let mut scopes = SrxScopes::default();

    // Route a finished term into the enclosing `<triple>` frame's active slot, or, at top
    // level, into the current row keyed by the open `<binding name=…>`.
    fn commit(
        term: Term,
        triple_stack: &mut [(usize, [Option<Term>; 3])],
        cur_row: &mut Vec<Option<Term>>,
        cur_var: Option<usize>,
    ) {
        if let Some((slot, parts)) = triple_stack.last_mut() {
            parts[*slot] = Some(term);
        } else if let Some(col) = cur_var {
            if cur_row.len() <= col {
                cur_row.resize(col + 1, None);
            }
            cur_row[col] = Some(term);
        }
    }
    fn set_slot(triple_stack: &mut [(usize, [Option<Term>; 3])], slot: usize) {
        if let Some((s, _)) = triple_stack.last_mut() {
            *s = slot;
        }
    }

    loop {
        match events
            .next_event()
            .map_err(|e| format!("SERVICE: invalid results XML: {e}"))?
        {
            Event::Eof => break,
            ev @ (Event::Start(_) | Event::Empty(_)) => {
                let is_empty = matches!(ev, Event::Empty(_));
                let e = match &ev {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                // One pass over the attributes: namespace declarations, and the first
                // well-formed value of each attribute the results format reads. `name`
                // and `datatype` are unprefixed (a default namespace never applies to an
                // attribute), `lang` is `xml:lang`, and `dir` is in the ITS namespace,
                // resolved once the tag's own declarations are in.
                scopes.enter();
                let [mut name_attr, mut lang, mut datatype] = [None, None, None];
                let mut dirs = Vec::new();
                let text = |a: &quick_xml::events::attributes::Attribute<'_>| {
                    std::str::from_utf8(&a.value)
                        .ok()
                        .and_then(|v| quick_xml::escape::unescape(v).ok())
                        .map(std::borrow::Cow::into_owned)
                };
                for a in e.attributes().filter_map(|a| a.ok()) {
                    let key = a.key.into_inner();
                    let prefix = match key.strip_prefix(b"xmlns") {
                        Some([]) => Some(&[][..]),
                        Some([b':', prefix @ ..]) => Some(prefix),
                        _ => None,
                    };
                    if let Some(prefix) = prefix {
                        let ns = text(&a).ok_or_else(|| {
                            "SERVICE: invalid results XML: bad namespace declaration".to_string()
                        })?;
                        scopes.declare(prefix, ns.as_bytes());
                        continue;
                    }
                    let slot = match key {
                        b"name" => &mut name_attr,
                        b"xml:lang" => &mut lang,
                        b"datatype" => &mut datatype,
                        _ => {
                            if let Some(prefix) = key.strip_suffix(b":dir") {
                                dirs.push((prefix, a));
                            }
                            continue;
                        }
                    };
                    if slot.is_none() {
                        *slot = text(&a);
                    }
                }
                // An `its` prefix that is never declared is still read as ITS, as
                // endpoints that omit the declaration are common.
                let dir = dirs.iter().find_map(|(prefix, a)| {
                    let its = match scopes.lookup(prefix) {
                        Some(ns) => ns == SrxNs::Its,
                        None => *prefix == b"its",
                    };
                    its.then(|| text(a)).flatten()
                });
                let ours = scopes.ours(e.name());
                let value = |kind| SrxValue {
                    kind,
                    lang: None,
                    dir: None,
                    datatype: None,
                    text: String::new(),
                };
                // An element in another namespace is not a results element, whatever its
                // local name: it is ignored like any unknown element.
                let name = e.local_name();
                let name: &[u8] = if ours { name.as_ref() } else { &[] };
                match name {
                    b"variable" => {
                        if let Some(v) = name_attr {
                            vars.push(Variable::new(&v).map_err(|e| {
                                format!("SERVICE: bad result variable {v:?}: {e}")
                            })?);
                        }
                    }
                    b"result" => cur_row.clear(),
                    b"binding" => {
                        cur_var = name_attr
                            .and_then(|name| vars.iter().position(|v| v.as_str() == name))
                    }
                    b"uri" => cur_val = Some(value("uri")),
                    b"bnode" => cur_val = Some(value("bnode")),
                    b"literal" => {
                        cur_val = Some(SrxValue {
                            lang,
                            dir,
                            datatype,
                            ..value("literal")
                        })
                    }
                    b"triple" => {
                        if triple_stack.len() == MAX_SRX_TRIPLE_DEPTH {
                            return Err(format!(
                                "SERVICE: results XML nests triple terms more than \
                                 {MAX_SRX_TRIPLE_DEPTH} deep"
                            ));
                        }
                        triple_stack.push((0, [None, None, None]))
                    }
                    b"subject" => set_slot(&mut triple_stack, 0),
                    b"predicate" => set_slot(&mut triple_stack, 1),
                    b"object" => set_slot(&mut triple_stack, 2),
                    b"boolean" => in_boolean = true,
                    _ => {}
                }
                // Self-closing value elements (`<literal/>`, `<bnode/>`) get no End event:
                // commit the (empty-text) term right away. Any other empty element leaves
                // an open value alone.
                if is_empty && matches!(name, b"uri" | b"bnode" | b"literal") {
                    if let Some(v) = cur_val.take() {
                        commit(v.into_term()?, &mut triple_stack, &mut cur_row, cur_var);
                    }
                }
                if is_empty {
                    scopes.close();
                }
            }
            Event::Text(t) => {
                // quick-xml 0.40 splits entity references out into `Event::GeneralRef`
                // (handled below), so the decoded text carries no `&…;` to unescape — it
                // is the verbatim character data.
                let s = t.decode().map_err(|e| e.to_string())?;
                if in_boolean {
                    boolean = Some(s.trim() == "true");
                } else if let Some(v) = cur_val.as_mut() {
                    v.text.push_str(&s);
                }
            }
            // A `&amp;` / `&lt;` / numeric `&#38;` / `&#x26;` reference inside the open
            // value element's text. (boolean bodies never carry references.) [OPUS-4.8]
            Event::GeneralRef(r) => {
                if let Some(v) = cur_val.as_mut() {
                    let name = r.decode().map_err(|e| e.to_string())?;
                    v.text.push_str(&resolve_xml_entity(&name)?);
                }
            }
            Event::CData(t) => {
                if let Some(v) = cur_val.as_mut() {
                    v.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(e) => {
                let ours = scopes.ours(e.name());
                scopes.close();
                let name = e.local_name();
                let name: &[u8] = if ours { name.as_ref() } else { &[] };
                match name {
                    b"uri" | b"bnode" | b"literal" => {
                        if let Some(v) = cur_val.take() {
                            commit(v.into_term()?, &mut triple_stack, &mut cur_row, cur_var);
                        }
                    }
                    b"triple" => {
                        let Some((_, [s, p, o])) = triple_stack.pop() else {
                            return Err("SERVICE: stray </triple> in results XML".into());
                        };
                        let part = |t: Option<Term>, k: &str| {
                            t.ok_or_else(|| format!("SERVICE: triple term without {k}"))
                        };
                        let term = triple_term(
                            part(s, "subject")?,
                            part(p, "predicate")?,
                            part(o, "object")?,
                        )?;
                        commit(term, &mut triple_stack, &mut cur_row, cur_var);
                    }
                    b"binding" => cur_var = None,
                    b"result" => {
                        // Project the row positionally over the declared variables; an
                        // absent binding is UNBOUND (`None`) — the same VALUES-UNDEF
                        // semantics the SRJ path uses. Delivered to the sink NOW
                        // (streaming, sq-my8wd.4): a sink error aborts the parse and
                        // propagates verbatim.
                        let mut row = std::mem::take(&mut cur_row);
                        row.resize(vars.len(), None);
                        on_row(row)?;
                    }
                    b"boolean" => in_boolean = false,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if boolean.is_some() {
        return Err(
            "SERVICE: endpoint returned an ASK boolean, expected SELECT bindings".into(),
        );
    }
    Ok(vars)
}

// ---------------------------------------------------------------------------
// SSRF egress policy (default-deny private / internal ranges) [OPUS-4.8]
// ---------------------------------------------------------------------------

/// [OPUS-4.8] (sq-iu0c) Stable marker substring embedded in EVERY engine error string
/// for a SERVICE egress refusal (a host blocked by the allowlist / default-deny SSRF
/// policy). It survives the transport-error wrapping in `HttpTransport::fetch`, so a
/// network-exposed host (e.g. `sparq-server`) can `contains()`-classify the refusal as a
/// **policy** decision — an honest `403`-style status — rather than a server-fault `500`.
/// This mirrors the existing `"query budget exceeded (timeout)"` → `503` marker pattern.
///
/// The marker is deliberately generic (it names no host) so it is safe to surface; the
/// host detail still travels in the surrounding (server-log-only) error text.
pub const SERVICE_EGRESS_REFUSED_MARKER: &str = "SERVICE egress refused";

//
// The `SERVICE` clause turns an attacker-controlled SPARQL string into an
// outbound HTTP request from the engine host (threat-model B4 / T-SERVICE-SSRF,
// bead sq-2v6f). With no egress filter that is a textbook SSRF primitive into
// the internal network — the worst case being the cloud-metadata endpoint
// 169.254.169.254, which on most clouds hands out credentials. The DEFAULT here
// is therefore DENY: a SERVICE endpoint that resolves to any non-global address
// is refused, and a deployer must explicitly opt a host/range back in via the
// allowlist (mirroring how `update.rs` gates `LOAD file://` behind
// `with_load_base`).
//
// DNS-rebinding safety: the check runs on the *resolved* IP, not the literal IRI
// host, and the production transport installs the policy as ureq's `Resolver`.
// ureq then connects only to the addresses the resolver returns, so the IP that
// is vetted is exactly the IP that is dialled — a hostile DNS answer that points
// at 127.0.0.1 / 169.254.169.254 is dropped before any socket is opened, and
// there is no resolve-then-reresolve TOCTOU window.

/// Classifies a resolved [`IpAddr`](std::net::IpAddr) as a forbidden destination for
/// SERVICE federation: `true` unless the address is globally reachable unicast. The shared
/// classifier ([`sparq_core::net::is_forbidden_ip`]), re-exported here so the SERVICE surface
/// keeps its name for it.
#[cfg(feature = "service")]
pub use sparq_core::net::is_forbidden_ip;

/// SERVICE egress allowlist + policy mode. A host (DNS name or IP literal, exactly
/// as written in the SERVICE IRI authority) on this list is exempt from
/// [`is_forbidden_ip`] — its resolved addresses are permitted even when private.
///
/// ## Port scoping ([OPUS-4.8] sq-a7jw4)
///
/// An allowlist entry may be **host-level** (`sparql.internal`, `127.0.0.1`,
/// `.example.org`) or **port-scoped** (`127.0.0.1:8053`, `sparql.internal:8443`,
/// `.example.org:443`). A host-level entry keeps its original meaning — it permits the
/// host on *every* port. A port-scoped entry is strictly NARROWER: it permits the host
/// ONLY on that exact port and rejects every other port on the same host. This lets a
/// deployer (or the in-process loopback SERVICE federation harness) re-open exactly
/// `127.0.0.1:<ephemeral>` without re-opening the whole loopback host. There is no
/// wildcard port — a port-scoped entry tightens, never loosens; default-deny stays
/// default-deny.
///
/// The port checked is the SERVICE IRI authority's port (its explicit `:port`, or the
/// scheme default 443/80), which is exactly the port `to_socket_addrs(host:port)` dials
/// for *every* resolved address — so port-scoping applies to the post-resolution connect
/// target and cannot be bypassed by DNS rebinding (the resolved IP is still re-vetted by
/// [`is_forbidden_ip`] at connect time; the allowlist exemption only fires when BOTH the
/// host pattern AND the port constraint match).
///
/// Two modes (the [`Mode`] flag), both default-deny but at different strictnesses:
///
/// * **`Mode::DenyPrivate`** (the engine's standalone default, installed by
///   [`with_service_egress_allow`]): public IPs are reachable, private/internal IPs
///   are refused unless the host is on the allowlist. Allowlist entries only *add*
///   permission (re-open a private host).
/// * **`Mode::AllowlistOnly`** (the strict mode the *server* uses, installed by
///   [`with_service_egress_policy`]): ONLY hosts on the allowlist may be reached at
///   all — every other host is refused even if it resolves to a public address. An
///   empty allowlist in this mode is therefore "deny ALL SERVICE", which is the
///   server's safe default for the network-exposed surface.
///
/// Empty + `DenyPrivate` (the thread-local default before any scope installs a
/// policy) preserves the original behaviour: public allowed, private denied.
/// Installed for a scope via [`with_service_egress_allow`] /
/// [`with_service_egress_policy`], mirroring `update.rs`'s `with_load_base`
/// thread-local allowlist pattern. [OPUS-4.8] (sq-4w18)
#[cfg(feature = "service")]
pub(crate) mod egress_policy {
    use std::cell::RefCell;
    use std::collections::HashSet;

    /// How the allowlist is interpreted for hosts NOT on it. [OPUS-4.8] (sq-4w18)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Mode {
        /// Hosts off the allowlist are reachable iff they resolve to a *public*
        /// address (private/internal IPs are refused). The engine's standalone
        /// default and the semantics of [`super::with_service_egress_allow`].
        DenyPrivate,
        /// Hosts off the allowlist are refused unconditionally (even public IPs).
        /// The server installs this so federation is restricted to exactly the
        /// operator-listed hosts; an empty allowlist = deny ALL SERVICE.
        AllowlistOnly,
    }

    struct Policy {
        allow: HashSet<String>,
        mode: Mode,
    }

    thread_local! {
        static POLICY: RefCell<Policy> =
            RefCell::new(Policy { allow: HashSet::new(), mode: Mode::DenyPrivate });
    }

    /// Restores the previous policy when the installing scope returns (also on
    /// unwind, so a panicking SERVICE call never leaks a relaxed policy).
    pub(crate) struct Guard(Option<Policy>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                POLICY.with(|p| *p.borrow_mut() = prev);
            }
        }
    }

    /// Installs `hosts` (lower-cased) + `mode` as the active SERVICE egress policy
    /// for the duration of the returned guard; the previous policy is restored on
    /// drop.
    pub(crate) fn install(hosts: impl IntoIterator<Item = String>, mode: Mode) -> Guard {
        let allow: HashSet<String> = hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect();
        let next = Policy { allow, mode };
        // Swap in the new policy and hand the previous one to the Guard for restore.
        POLICY.with(|p| Guard(Some(std::mem::replace(&mut *p.borrow_mut(), next))))
    }

    /// Split one allowlist entry into its `(host_pattern, port_constraint)`. [OPUS-4.8] (sq-a7jw4)
    ///
    /// An entry is **port-scoped** (`Some(port)`) only when it carries an *unambiguous*
    /// trailing `:port`; otherwise it is **host-level** (`None` = every port). The split is
    /// conservative so a bare IPv6 literal is never amputated:
    ///   * `[::1]:8053` / `[2001:db8::1]:443` — bracketed IPv6 + port → `("::1", Some(8053))`
    ///     (brackets are stripped from the host pattern, mirroring `is_allowed`'s bare-host key).
    ///   * `127.0.0.1:8053` / `sparql.internal:8443` / `.example.org:443` — a single-colon
    ///     `host:digits` (the suffix dot is part of the host pattern) → `Some(port)`.
    ///   * `::1` / `2001:db8::1` — a bare (unbracketed) multi-colon IPv6 literal → `None`
    ///     (the trailing hextet is NOT read as a port).
    ///   * `sparql.internal` / `.example.org` / `127.0.0.1` — no colon → `None`.
    ///
    /// A `:port` whose digits do not parse as a `u16` (or is empty) is treated as part of
    /// the host pattern (it will simply never match a real authority) rather than silently
    /// dropping the port constraint — fail-closed, never fail-open.
    pub(crate) fn split_entry(entry: &str) -> (&str, Option<u16>) {
        // Bracketed IPv6 authority: `[addr]` or `[addr]:port`.
        if let Some(rest) = entry.strip_prefix('[') {
            if let Some((inner, after)) = rest.split_once(']') {
                let port = after
                    .strip_prefix(':')
                    .and_then(|p| p.parse::<u16>().ok());
                return (inner, port);
            }
            return (entry, None); // malformed (no closing bracket) — host pattern as-is
        }
        // Unbracketed. Only a single-colon `host:port` is a port-bearing authority; a
        // multi-colon string is a bare IPv6 literal whose last hextet must NOT be read as
        // a port (e.g. `2001:db8::1`).
        if let Some((host, port)) = entry.split_once(':') {
            if !host.contains(':') && !port.contains(':') {
                if let Ok(p) = port.parse::<u16>() {
                    return (host, Some(p));
                }
            }
        }
        (entry, None)
    }

    /// True iff `host_pattern` (an entry's host part, already lower-cased) matches the
    /// lower-cased connect host `h`: exact, or a leading-dot suffix wildcard. [OPUS-4.8]
    ///   * **exact** — the pattern equals the host (`"sparql.example.org"`).
    ///   * **suffix wildcard** — a pattern beginning with a dot (`".example.org"`)
    ///     matches any host ending in that suffix INCLUDING the bare apex
    ///     (`example.org`, `a.example.org`, `a.b.example.org`). The leading-dot
    ///     boundary means `.example.org` does NOT match `notexample.org`.
    fn host_matches(host_pattern: &str, h: &str) -> bool {
        if let Some(suffix) = host_pattern.strip_prefix('.') {
            h == suffix || h.ends_with(host_pattern)
        } else {
            host_pattern == h
        }
    }

    /// True if `(host, port)` is permitted by the active allowlist. [OPUS-4.8] (sq-a7jw4)
    ///
    /// `host` is the SERVICE IRI authority host (case-insensitive); `port` is the
    /// authority's port (its explicit `:port` or the scheme default — the SAME port that is
    /// actually dialled for every resolved address). An entry matches when its host pattern
    /// matches (exact or `.suffix` wildcard, per [`host_matches`]) AND its port constraint
    /// is satisfied:
    ///   * a **host-level** entry (no `:port`) permits the host on EVERY port — the original
    ///     `sq-4w18` semantics, preserved exactly for backward compatibility;
    ///   * a **port-scoped** entry (`host:port`) permits the host ONLY on that exact port
    ///     and rejects every other port on the same host — strictly narrower (`sq-a7jw4`).
    ///
    /// There is NO wildcard port: a port-scoped entry can only tighten, never widen, so
    /// default-deny stays default-deny.
    pub(crate) fn is_allowed(host: &str, port: u16) -> bool {
        let h = host.to_ascii_lowercase();
        POLICY.with(|p| {
            p.borrow()
                .allow
                .iter()
                .any(|e| entry_permits(e, &h, port))
        })
    }

    /// True iff the single allowlist `entry` permits the lower-cased connect host `h` on
    /// `port` — the pure per-entry predicate `is_allowed`'s `.any(…)` closure applies to
    /// every stored entry. [OPUS-4.8] (sq-a7jw4)
    ///
    /// Factored out so the host:port parsing + matching semantics (port-0/overflow/
    /// IPv6-bracket/trailing-colon all handled by [`split_entry`]) live in ONE place and can
    /// be shared verbatim by the `sparq-fedclient` egress guard via
    /// [`super::allowlist_entry_permits`] — there is no second, divergent copy of the
    /// host:port rules (bead sq-vbnyc). `h` is expected pre-lower-cased; the public wrapper
    /// lower-cases for callers that hold a raw host.
    pub(crate) fn entry_permits(entry: &str, h: &str, port: u16) -> bool {
        let (host_pattern, entry_port) = split_entry(entry);
        host_matches(host_pattern, h) && entry_port.is_none_or(|ep| ep == port)
    }

    /// True iff the allowlist `entry`'s HOST part matches the lower-cased connect host `h`,
    /// IGNORING any port constraint — i.e. "is `h` named by this entry on *some* port". [OPUS-4.8]
    /// (sq-vbnyc). The fedclient guard's backward-compatible "is this host allowlisted at all"
    /// query reuses this so the host-pattern parsing (bracket-stripping, suffix wildcard) lives in
    /// ONE place rather than being re-derived client-side.
    pub(crate) fn entry_host_matches(entry: &str, h: &str) -> bool {
        let (host_pattern, _entry_port) = split_entry(entry);
        host_matches(host_pattern, h)
    }

    /// The active policy mode.
    pub(crate) fn mode() -> Mode {
        POLICY.with(|p| p.borrow().mode)
    }
}

/// True iff a single SSRF-allowlist `entry` permits connecting to `host` on `port`. [OPUS-4.8] (sq-vbnyc)
///
/// This is the engine's SERVICE-egress per-entry matching rule, exposed so the
/// **`sparq-fedclient`** crate's independent host-level egress guard can adopt the SAME
/// port-scoping semantics rather than re-implementing the host:port parsing/matching (one
/// source of truth — bead sq-vbnyc, follow-up to the engine's port-scoped allowlist sq-a7jw4).
/// The fedclient guard owns its own per-instance allowlist storage but delegates the
/// per-entry decision here, so the two guards agree byte-for-byte on every edge case:
///
/// * a **host-level** entry (no `:port`, e.g. `"sparql.internal"` or `".example.org"`)
///   permits the host on EVERY port — the original, backward-compatible meaning;
/// * a **port-scoped** entry (`host:port`, e.g. `"127.0.0.1:8053"` / `"[::1]:8080"` /
///   `".example.org:443"`) permits the host ONLY on that exact port and rejects every other
///   port on the same host — strictly narrower;
/// * a malformed `:port` (out-of-range `:99999`, empty `:` trailing colon, non-numeric) is
///   treated as part of a never-matching host pattern — fail-CLOSED, never widened;
/// * a bare (unbracketed) IPv6 literal (`"2001:db8::1"`) is NOT amputated — host-level.
///
/// `host` is matched case-insensitively; matching is exact or a leading-dot suffix wildcard.
/// There is NO wildcard port and no global bypass — a port-scoped entry can only tighten.
#[cfg(feature = "service")]
pub fn allowlist_entry_permits(entry: &str, host: &str, port: u16) -> bool {
    egress_policy::entry_permits(entry, &host.to_ascii_lowercase(), port)
}

/// True iff a single SSRF-allowlist `entry`'s HOST part names `host`, IGNORING any port
/// constraint — "is `host` reachable through this entry on *some* port". [OPUS-4.8] (sq-vbnyc)
///
/// Companion to [`allowlist_entry_permits`] for the `sparq-fedclient` guard's backward-compatible
/// "is this host allowlisted at all" query (where the dialled port is not yet known), so the
/// host-pattern parsing (IPv6-bracket stripping + `.suffix` wildcard) is NOT re-implemented
/// client-side. Matching is case-insensitive, exact or leading-dot suffix wildcard.
#[cfg(feature = "service")]
pub fn allowlist_entry_host_matches(entry: &str, host: &str) -> bool {
    egress_policy::entry_host_matches(entry, &host.to_ascii_lowercase())
}

/// Runs `f` with `hosts` allowlisted for SERVICE federation: each host's resolved
/// addresses are permitted even if they fall in a private/internal range that the
/// default-deny SSRF policy would otherwise refuse. A host is matched
/// case-insensitively against the *authority* of the SERVICE IRI (DNS name or IP
/// literal, e.g. `"localhost"`, `"10.0.0.5"`, `"sparql.internal"`).
///
/// Without an installed allowlist, every SERVICE endpoint that resolves to a
/// loopback / RFC1918 / link-local / unique-local / unspecified address is
/// REJECTED — the secure default. This mirrors `with_load_base`, which
/// gates `LOAD file://` the same way. Only effective with the `service` feature.
///
/// ```no_run
/// # #[cfg(feature = "service")] {
/// // Permit federation to a trusted internal endpoint that resolves privately.
/// sparq_engine_service::service::with_service_egress_allow(["sparql.internal".to_string()], || {
///     // ... run a query containing `SERVICE <http://sparql.internal/> { ... }`
/// });
/// # }
/// ```
#[cfg(feature = "service")]
pub fn with_service_egress_allow<R>(
    hosts: impl IntoIterator<Item = String>,
    f: impl FnOnce() -> R,
) -> R {
    let _guard = egress_policy::install(hosts, egress_policy::Mode::DenyPrivate);
    f()
}

/// Runs `f` under a STRICT SERVICE egress policy: only the listed `hosts` may be
/// reached, and EVERY other host is refused — even one resolving to a public
/// address. This is the policy the network-exposed **server** installs (bead
/// sq-4w18): the SERVICE clause turns attacker-controlled SPARQL into outbound HTTP,
/// so federation is restricted to exactly the operator-configured endpoints.
///
/// `strict = false` is identical to [`with_service_egress_allow`] (default-deny
/// *private*: public hosts reachable, private/internal only if allowlisted). The
/// server wires this directly to its `--service-allow` config so the same call site
/// expresses both "no federation at all" (strict + empty list) and "an explicit
/// allowlist" (strict + hosts). Host matching is case-insensitive against the
/// SERVICE IRI *authority* (DNS name or IP literal), exactly like
/// [`with_service_egress_allow`].
///
/// ```no_run
/// # #[cfg(feature = "service")] {
/// // Restrict SERVICE to a single trusted endpoint; anything else is refused.
/// sparq_engine_service::service::with_service_egress_policy(true, ["sparql.example.org".to_string()], || {
///     // ... run a query that may contain `SERVICE <…> { ... }`
/// });
/// // Strict + empty list = federation fully disabled (deny ALL SERVICE).
/// sparq_engine_service::service::with_service_egress_policy(true, std::iter::empty(), || { /* ... */ });
/// # }
/// ```
#[cfg(feature = "service")]
pub fn with_service_egress_policy<R>(
    strict: bool,
    hosts: impl IntoIterator<Item = String>,
    f: impl FnOnce() -> R,
) -> R {
    let mode = if strict {
        egress_policy::Mode::AllowlistOnly
    } else {
        egress_policy::Mode::DenyPrivate
    };
    let _guard = egress_policy::install(hosts, mode);
    f()
}

// ---------------------------------------------------------------------------
// Production HTTP transport (ureq, blocking, native-only)
// ---------------------------------------------------------------------------

/// The real network transport: a blocking ureq POST with the SPARQL query
/// form-encoded in the body and an `Accept` header that prefers SPARQL-Results-JSON but
/// also accepts SPARQL-Results-XML (the response is content-sniffed by `parse_results`).
///
/// Gated to `cfg(not(wasm32))` AND the `service` feature so neither ureq nor any of
/// its TLS stack ever enters the wasm bundle.
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
pub struct HttpTransport {
    timeout: std::time::Duration,
}

/// The transport's own finite default round-trip timeout, used when no per-query
/// budget deadline constrains it further. A slow/unreachable endpoint cannot hang the
/// engine past this; SILENT then turns the timeout into an empty result. [OPUS-4.8]
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
pub(crate) const DEFAULT_SERVICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Floor on a budget-derived SERVICE timeout. A deadline that is already expired (or
/// within a few ms) would otherwise yield a zero/near-zero socket timeout that fails
/// every dial instantly; we still give the round-trip a brief window, and the engine's
/// own cooperative budget check (`exec::budget::check`) reports the over-deadline query
/// as `"query budget exceeded (timeout)"` either way. [OPUS-4.8] (sq-d4p)
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
pub(crate) const MIN_SERVICE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
impl HttpTransport {
    /// Construct a transport whose per-request timeout is the active query budget's
    /// remaining time, capped by the built-in `DEFAULT_SERVICE_TIMEOUT`. [OPUS-4.8] (sq-d4p)
    ///
    /// `remaining` is the time left until the `QueryBudget`
    /// deadline (from `exec::budget::remaining_timeout`):
    /// * `None` — no deadline installed → use the built-in default in full.
    /// * `Some(d)` — bound the remote round-trip by `min(d, default)`, so a query under
    ///   a tight deadline does not block for the full default on an unresponsive
    ///   endpoint. (The budget's *local* cooperative check only fires AFTER the blocking
    ///   HTTP call returns, so without this cap the deadline would not bite the remote
    ///   call.) A non-zero floor (`MIN_SERVICE_TIMEOUT`) is applied so a nearly- or
    ///   just-expired deadline still attempts a quick round-trip rather than a
    ///   guaranteed-instant failure; the local budget check then converts an
    ///   over-deadline query into the timeout error.
    pub fn with_budget(remaining: Option<std::time::Duration>) -> Self {
        let timeout = match remaining {
            None => DEFAULT_SERVICE_TIMEOUT,
            Some(d) => d.min(DEFAULT_SERVICE_TIMEOUT).max(MIN_SERVICE_TIMEOUT),
        };
        HttpTransport { timeout }
    }

    /// The configured per-request timeout (test accessor for the budget-cap logic).
    #[cfg(test)]
    pub(crate) fn timeout_for_test(&self) -> std::time::Duration {
        self.timeout
    }
}

/// ureq [`Resolver`](ureq::unversioned::resolver::Resolver) wrapper that enforces the SSRF egress policy
/// on the *resolved* addresses (DNS-rebinding-safe). [OPUS-4.8]
///
/// It resolves `netloc` with the standard system resolver, drops every address
/// the [`is_forbidden_ip`] policy refuses (unless the host is on the active
/// allowlist), and returns only the survivors — so ureq dials only vetted IPs.
/// If resolution yields nothing but forbidden addresses, it returns a
/// `PermissionDenied` error rather than an empty set, which surfaces to the
/// caller as a SERVICE failure (and an empty result under SILENT).
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
#[derive(Debug)]
struct EgressFilterResolver;

// [OPUS-4.8] sq-g2xs: the ureq-3 `Resolver` trait takes a parsed `&http::Uri` (+ `Config` +
// timeout) and returns an `ArrayVec<SocketAddr, 16>` rather than ureq-2's `&str` netloc → `Vec`.
// The SSRF logic is otherwise byte-for-byte the ureq-2 policy: derive `host:port` from the URI
// authority + scheme default port, key the allowlist by the bare host, resolve, drop every
// `is_forbidden_ip` address (unless allowlisted), and refuse with a `PermissionDenied` io error
// (carried as `ureq::Error::Io`, so the `SERVICE_EGRESS_REFUSED_MARKER` survives the wrapping
// and the kind stays `PermissionDenied` for the egress tests). A refusal is a HARD error (never
// an empty address set), so ureq cannot fall through to an unguarded dial.

/// `host:port` (for resolution) + bare host (for the allowlist key, IPv6 brackets stripped,
/// lowercased) + the numeric `port` (for the port-scoped allowlist check, sq-a7jw4) from a
/// ureq-3 request [`Uri`](ureq::http::Uri). `port` falls back to the scheme default (443 for
/// https, 80 otherwise). [OPUS-4.8] sq-g2xs / sq-a7jw4.
///
/// The returned `port` is exactly the port `to_socket_addrs(host:port)` dials for EVERY
/// resolved address, so a port-scoped allowlist entry is checked against the port actually
/// connected to — no resolve-then-reconnect port TOCTOU.
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
fn uri_host_port(uri: &ureq::http::Uri) -> Option<(String, String, u16)> {
    let authority = uri.authority()?;
    let host = authority.host();
    if host.is_empty() {
        return None;
    }
    let port = authority.port_u16().unwrap_or_else(|| {
        match uri.scheme_str() {
            Some("https") => 443,
            _ => 80,
        }
    });
    // The authority host keeps IPv6 brackets (`[::1]`); strip them for the allowlist key and
    // for `to_socket_addrs` (which wants the bare host + a separate port).
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    Some((format!("{bare}:{port}"), bare.to_ascii_lowercase(), port))
}

/// Wrap a refusal reason as a `PermissionDenied` [`ureq::Error::Io`], preserving both the kind
/// (the egress tests assert on it) and the [`SERVICE_EGRESS_REFUSED_MARKER`] text. [OPUS-4.8].
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
fn egress_refused(reason: String) -> ureq::Error {
    ureq::Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        reason,
    ))
}

#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
impl ureq::unversioned::resolver::Resolver for EgressFilterResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        use std::net::ToSocketAddrs;
        let (host_port, host, port) = uri_host_port(uri).ok_or_else(|| {
            egress_refused(format!(
                "{SERVICE_EGRESS_REFUSED_MARKER}: request URI {uri} has no host authority to vet"
            ))
        })?;
        // [OPUS-4.8] (sq-a7jw4) The allowlist check is port-scoped: a `host:port` entry permits
        // only that host on that exact port; a bare-host entry still permits every port. `port`
        // is the authority port that is actually dialled for every resolved address.
        let allowed = egress_policy::is_allowed(&host, port);
        // [OPUS-4.8] (sq-4w18) STRICT (AllowlistOnly) mode — the server's policy — refuses any
        // host not on the allowlist BEFORE resolving DNS, so a host that is not explicitly
        // permitted never triggers even a lookup. An empty allowlist here = deny ALL SERVICE.
        if !allowed && egress_policy::mode() == egress_policy::Mode::AllowlistOnly {
            return Err(egress_refused(format!(
                "{SERVICE_EGRESS_REFUSED_MARKER}: host {host:?} is not on the SERVICE allowlist \
                 (strict allowlist-only policy; add it via --service-allow / SPARQ_SERVICE_ALLOW \
                 on the server, or with_service_egress_policy in an embedder)"
            )));
        }
        let resolved = host_port.to_socket_addrs().map_err(ureq::Error::Io)?;
        let mut permitted: ureq::unversioned::resolver::ResolvedSocketAddrs = arrayvec_default();
        // `ResolvedSocketAddrs` is a fixed-capacity `ArrayVec<_, 16>`; cap to its capacity (the
        // same `MAX_ADDRS` ureq's own resolver uses) so `push` never overruns the backing array.
        for sa in resolved
            .filter(|sa| allowed || !is_forbidden_ip(sa.ip()))
            .take(RESOLVED_ADDRS_CAP)
        {
            permitted.push(sa);
        }
        if permitted.is_empty() {
            return Err(egress_refused(format!(
                "{SERVICE_EGRESS_REFUSED_MARKER}: {host_port} resolves only to private/internal addresses \
                 (default-deny SSRF policy; allowlist the host via with_service_egress_allow)"
            )));
        }
        Ok(permitted)
    }
}

/// Capacity of ureq-3's [`ResolvedSocketAddrs`](ureq::unversioned::resolver::ResolvedSocketAddrs)
/// (`ArrayVec<SocketAddr, 16>`). Matches ureq's own `MAX_ADDRS`. [OPUS-4.8] sq-g2xs.
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
const RESOLVED_ADDRS_CAP: usize = 16;

/// An empty [`ResolvedSocketAddrs`](ureq::unversioned::resolver::ResolvedSocketAddrs) backing
/// store (a fixed-capacity `ArrayVec`). [OPUS-4.8] sq-g2xs.
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
fn arrayvec_default() -> ureq::unversioned::resolver::ResolvedSocketAddrs {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    // `ArrayVec::from_fn` fills the backing array, but the logical length starts at 0
    // (the same idiom ureq's `DefaultResolver::empty` uses), so it is genuinely empty.
    ureq::unversioned::resolver::ArrayVec::from_fn(|_| {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    })
}

#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
impl HttpTransport {
    /// POST `query` to `endpoint` through the SSRF-guarded agent and return the response, or
    /// the SERVICE error text for a transport failure or a non-2xx status.
    fn post(
        &self,
        endpoint: &str,
        query: &str,
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        // [OPUS-4.8] sq-g2xs: ureq-3 builds an `Agent` from a `Config` + a custom resolver via
        // `Agent::with_parts`; the resolver carries the default-deny SSRF policy exactly as in
        // ureq 2 (the resolved-and-vetted IP is the dialled IP — no DNS-rebinding re-resolve).
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .user_agent(concat!("sparq-engine/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            EgressFilterResolver,
        );
        // POST with the query in an `application/x-www-form-urlencoded` `query=` field
        // (SPARQL Protocol §2.1.2 "query via POST with URL-encoded parameters") — the
        // most broadly supported method and not subject to URL-length limits.
        agent
            .post(endpoint)
            // Prefer JSON, but accept XML — some endpoints ignore `Accept` and only emit
            // SPARQL-Results-XML, which `parse_results` now handles (bead sq-ycu). [OPUS-4.8]
            .header(
                "Accept",
                "application/sparql-results+json, application/sparql-results+xml;q=0.9",
            )
            .send_form([("query", query)])
            .map_err(|e| match e {
                // ureq-3 surfaces non-2xx as `Error::StatusCode`; treat both transport and HTTP
                // errors uniformly (the caller decides SILENT vs propagate).
                ureq::Error::StatusCode(code) => {
                    format!("SERVICE: endpoint {endpoint} returned HTTP {code}")
                }
                e => format!("SERVICE: request to {endpoint} failed: {e}"),
            })
    }
}

#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
impl Transport for HttpTransport {
    fn fetch(&self, endpoint: &str, query: &str) -> Result<String, String> {
        // ureq-3 caps `read_to_string` at 10 MiB by default; a federated SELECT result can
        // exceed that, so raise the limit generously (a finite cap still bounds memory).
        self.post(endpoint, query)?
            .body_mut()
            .with_config()
            .limit(SERVICE_MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|e| format!("SERVICE: reading response from {endpoint}: {e}"))
    }
}

/// `ReaderTransport` for `HttpTransport`: returns the ureq response body as a byte
/// stream WITHOUT calling `read_to_string`, so the body is NEVER fully buffered as a
/// `String`. The same `SERVICE_MAX_BODY_BYTES` cap is enforced by the limit-wrapped
/// reader. (bead sq-my8wd.5) [OPUS-4.8] [FABLE-5]
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
impl ReaderTransport for HttpTransport {
    fn fetch_reader<'a>(
        &'a self,
        endpoint: &str,
        query: &str,
    ) -> Result<Box<dyn std::io::Read + 'a>, String> {
        let resp = self.post(endpoint, query)?;
        Ok(Box::new(
            resp.into_body()
                .into_with_config()
                .limit(SERVICE_MAX_BODY_BYTES)
                .reader(),
        ))
    }
}

/// Max bytes read from a SERVICE response body. ureq-3's default `read_to_string` cap is 10 MiB;
/// a federated SELECT can legitimately exceed that, so we raise it to a generous-but-finite bound
/// (memory is still bounded — a runaway endpoint cannot OOM the engine). [OPUS-4.8] sq-g2xs.
#[cfg(all(feature = "service", not(target_arch = "wasm32")))]
const SERVICE_MAX_BODY_BYTES: u64 = 1024 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Tests (parser + transport seam; no public network)
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "service"))]
mod tests {
    use super::*;

    /// A fully-collected remote SELECT result (`None` = unbound), for comparing parses.
    #[derive(Debug, PartialEq)]
    struct ServiceRelation {
        vars: Vec<Variable>,
        rows: Vec<Vec<Option<Term>>>,
    }

    type Sink<'a> = &'a mut dyn FnMut(Vec<Option<Term>>) -> Result<(), String>;

    /// Run a streaming parse to completion, collecting its rows.
    fn collect(
        parse: impl FnOnce(Sink<'_>) -> Result<Vec<Variable>, String>,
    ) -> Result<ServiceRelation, String> {
        let mut rows = Vec::new();
        let vars = parse(&mut |row| {
            rows.push(row);
            Ok(())
        })?;
        Ok(ServiceRelation { vars, rows })
    }

    fn eval_remote(
        t: &dyn Transport,
        endpoint: &str,
        query: &str,
    ) -> Result<ServiceRelation, String> {
        collect(|mut sink| eval_remote_into(t, endpoint, query, &mut sink))
    }

    fn parse_results(text: &str) -> Result<ServiceRelation, String> {
        collect(|mut sink| parse_results_into(text, &mut sink))
    }

    fn parse_srj(text: &str) -> Result<ServiceRelation, String> {
        collect(|mut sink| parse_srj_into(text, &mut sink))
    }

    fn parse_srx(text: &str) -> Result<ServiceRelation, String> {
        collect(|mut sink| parse_srx_into(text, &mut sink))
    }

    // ---------------------------------------------------------------------
    // [OPUS-4.8] (sq-6vshe.4 / sq-d4p) Per-request timeout wired to the QueryBudget
    // deadline. MOVED here from sparq-engine's exec.rs WITH the `HttpTransport` it
    // exercises (seam A2): `timeout_for_test` is `#[cfg(test)]`, so this test only
    // compiles in THIS crate's own test build. The caller side (`budget::remaining_timeout`
    // feeding `with_budget`) stays tested in exec.rs.
    // ---------------------------------------------------------------------
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn http_transport_timeout_tracks_budget() {
        use std::time::Duration;
        // No deadline -> the built-in default in full.
        assert_eq!(HttpTransport::with_budget(None).timeout_for_test(), DEFAULT_SERVICE_TIMEOUT);
        // A deadline tighter than the default caps the round-trip to the remaining time.
        let tight = Duration::from_secs(5);
        assert_eq!(HttpTransport::with_budget(Some(tight)).timeout_for_test(), tight);
        // A deadline looser than the default never RAISES the timeout above the default.
        let loose = Duration::from_secs(120);
        assert_eq!(
            HttpTransport::with_budget(Some(loose)).timeout_for_test(),
            DEFAULT_SERVICE_TIMEOUT
        );
        // An already-expired (zero) deadline still gets the small non-zero floor.
        assert_eq!(
            HttpTransport::with_budget(Some(Duration::ZERO)).timeout_for_test(),
            MIN_SERVICE_TIMEOUT
        );
    }

    #[test]
    fn parses_uri_and_literal_bindings() {
        let body = r#"{
            "head": { "vars": ["s", "name"] },
            "results": { "bindings": [
                { "s": {"type":"uri","value":"http://ex/a"},
                  "name": {"type":"literal","value":"Alice"} },
                { "s": {"type":"uri","value":"http://ex/b"},
                  "name": {"type":"literal","value":"Bob","xml:lang":"en"} }
            ] }
        }"#;
        let rel = parse_srj(body).unwrap();
        assert_eq!(rel.vars.len(), 2);
        assert_eq!(rel.rows.len(), 2);
        assert_eq!(
            rel.rows[0][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/a").unwrap()))
        );
        assert_eq!(
            rel.rows[1][1],
            Some(Term::Literal(
                Literal::new_language_tagged_literal("Bob", "en").unwrap()
            ))
        );
    }

    #[test]
    fn unbound_variable_becomes_none() {
        let body = r#"{
            "head": { "vars": ["a", "b"] },
            "results": { "bindings": [ { "a": {"type":"uri","value":"http://ex/x"} } ] }
        }"#;
        let rel = parse_srj(body).unwrap();
        assert_eq!(rel.rows[0][0], Some(Term::NamedNode(NamedNode::new("http://ex/x").unwrap())));
        assert_eq!(rel.rows[0][1], None);
    }

    #[test]
    fn typed_literal_roundtrips() {
        let body = r#"{
            "head": { "vars": ["n"] },
            "results": { "bindings": [
                { "n": {"type":"literal","value":"42",
                        "datatype":"http://www.w3.org/2001/XMLSchema#integer"} }
            ] }
        }"#;
        let rel = parse_srj(body).unwrap();
        let want = Literal::new_typed_literal(
            "42",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        );
        assert_eq!(rel.rows[0][0], Some(Term::Literal(want)));
    }

    /// [OPUS-4.8] sq-s955: inbound parity for the outbound `its:dir` emission. A SERVICE
    /// endpoint's results carry the RDF 1.2 base direction as a SEPARATE `its:dir` field
    /// next to the bare `xml:lang` tag; the parser must reconstruct the `dirLangString` so
    /// the direction survives INTO the local join (it was previously dropped). This mirrors
    /// the outbound `json.rs` parity test — direction round-trips both ways.
    #[test]
    fn dir_lang_string_direction_roundtrips_inbound() {
        for (dir, want) in [
            ("ltr", oxrdf::BaseDirection::Ltr),
            ("rtl", oxrdf::BaseDirection::Rtl),
        ] {
            let body = format!(
                r#"{{
                "head": {{ "vars": ["g"] }},
                "results": {{ "bindings": [
                    {{ "g": {{"type":"literal","value":"مرحبا","xml:lang":"ar","its:dir":"{dir}"}} }}
                ] }}
            }}"#
            );
            let rel = parse_srj(&body).unwrap();
            match &rel.rows[0][0] {
                Some(Term::Literal(l)) => {
                    assert_eq!(l.value(), "مرحبا");
                    assert_eq!(l.language(), Some("ar"));
                    // The direction round-trips inbound (the bug: it was None).
                    assert_eq!(l.direction(), Some(want), "its:dir={dir} must survive inbound");
                }
                other => panic!("expected a directional literal, got {other:?}"),
            }
        }
    }

    /// [OPUS-4.8] sq-s955: an ABSENT or INVALID `its:dir` degrades to a PLAIN language-tagged
    /// literal — the same decision `dict::split_lang_dir` / `reconstruct_ref` make for a
    /// malformed stored slot — so the inbound, stored-slot, materialised and outbound paths
    /// all AGREE on `(lang, dir)`. (An uppercase `LTR` or any other value is not a direction.)
    #[test]
    fn invalid_or_absent_its_dir_is_plain_language_literal() {
        let plain = Literal::new_language_tagged_literal("hi", "en").unwrap();
        for cell in [
            r#"{"type":"literal","value":"hi","xml:lang":"en"}"#, // absent its:dir
            r#"{"type":"literal","value":"hi","xml:lang":"en","its:dir":"LTR"}"#, // wrong case
            r#"{"type":"literal","value":"hi","xml:lang":"en","its:dir":"sideways"}"#, // bogus
            r#"{"type":"literal","value":"hi","xml:lang":"en","its:dir":""}"#, // empty
        ] {
            let body =
                format!(r#"{{"head":{{"vars":["g"]}},"results":{{"bindings":[{{"g":{cell}}}]}}}}"#);
            let rel = parse_srj(&body).unwrap();
            assert_eq!(
                rel.rows[0][0],
                Some(Term::Literal(plain.clone())),
                "invalid/absent its:dir in {cell} must degrade to a plain language-tagged literal"
            );
        }
    }

    #[test]
    fn empty_results_is_ok() {
        let body = r#"{"head":{"vars":["x"]},"results":{"bindings":[]}}"#;
        let rel = parse_srj(body).unwrap();
        assert!(rel.rows.is_empty());
        assert_eq!(rel.vars.len(), 1);
    }

    #[test]
    fn malformed_json_errors() {
        assert!(parse_srj("not json at all").is_err());
        assert!(parse_srj(r#"{"head":{}}"#).is_err()); // no vars
        assert!(parse_srj(r#"{"boolean":true}"#).is_err()); // ASK, not SELECT
    }

    // ---------------------------------------------------------------------
    // SPARQL-Results-XML (SRX) parsing [OPUS-4.8] (bead sq-ycu)
    // ---------------------------------------------------------------------

    const SRX_NS: &str = "http://www.w3.org/2005/sparql-results#";

    #[test]
    fn srx_parses_uri_and_literals() {
        let body = format!(
            r#"<?xml version="1.0"?>
            <sparql xmlns="{SRX_NS}">
              <head><variable name="s"/><variable name="name"/></head>
              <results>
                <result>
                  <binding name="s"><uri>http://ex/a</uri></binding>
                  <binding name="name"><literal>Alice</literal></binding>
                </result>
                <result>
                  <binding name="s"><uri>http://ex/b</uri></binding>
                  <binding name="name"><literal xml:lang="en">Bob</literal></binding>
                </result>
              </results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert_eq!(rel.vars.len(), 2);
        assert_eq!(rel.rows.len(), 2);
        assert_eq!(
            rel.rows[0][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/a").unwrap()))
        );
        assert_eq!(
            rel.rows[0][1],
            Some(Term::Literal(Literal::new_simple_literal("Alice")))
        );
        assert_eq!(
            rel.rows[1][1],
            Some(Term::Literal(
                Literal::new_language_tagged_literal("Bob", "en").unwrap()
            ))
        );
    }

    #[test]
    fn srx_unbound_variable_becomes_none() {
        // `b` is absent in the single solution and the bindings appear out of
        // declaration order — projection must be positional over <variable> order.
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="a"/><variable name="b"/></head>
              <results>
                <result><binding name="a"><uri>http://ex/x</uri></binding></result>
              </results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert_eq!(
            rel.rows[0][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/x").unwrap()))
        );
        assert_eq!(rel.rows[0][1], None);
    }

    #[test]
    fn srx_projection_keeps_first_column_last_binding_and_ignores_undeclared() {
        // A duplicated <variable> fills only its first column, a repeated binding keeps the
        // last term, a binding for an undeclared variable is dropped, and each row starts
        // empty.
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="a"/><variable name="b"/><variable name="a"/></head>
              <results>
                <result>
                  <binding name="a"><uri>http://ex/1</uri></binding>
                  <binding name="a"><uri>http://ex/2</uri></binding>
                  <binding name="zz"><uri>http://ex/3</uri></binding>
                </result>
                <result><binding name="b"><literal>x</literal></binding></result>
              </results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        let iri = |s: &str| Some(Term::NamedNode(NamedNode::new(s).unwrap()));
        assert_eq!(rel.rows[0], vec![iri("http://ex/2"), None, None]);
        let x = Some(Term::Literal(Literal::new_simple_literal("x")));
        assert_eq!(rel.rows[1], vec![None, x, None]);
    }

    #[test]
    fn srj_and_srx_admit_the_same_triple_term_depth() {
        let iri = |s: &str| format!(r#"{{"type":"uri","value":"http://ex/{s}"}}"#);
        let srj = |depth: usize| {
            let mut t = iri("o");
            for _ in 0..depth {
                t = format!(
                    r#"{{"type":"triple","value":{{"subject":{},"predicate":{},"object":{t}}}}}"#,
                    iri("s"),
                    iri("p")
                );
            }
            format!(r#"{{"head":{{"vars":["x"]}},"results":{{"bindings":[{{"x":{t}}}]}}}}"#)
        };
        let srx = |depth: usize| {
            let mut t = "<uri>http://ex/o</uri>".to_string();
            for _ in 0..depth {
                t = format!(
                    "<triple><subject><uri>http://ex/s</uri></subject>\
                     <predicate><uri>http://ex/p</uri></predicate><object>{t}</object></triple>"
                );
            }
            format!(
                r#"<sparql xmlns="{SRX_NS}"><head><variable name="x"/></head><results>
                   <result><binding name="x">{t}</binding></result></results></sparql>"#
            )
        };
        let max = MAX_SRX_TRIPLE_DEPTH;
        assert!(parse_srj(&srj(max)).is_ok());
        assert!(parse_srj(&srj(max + 1)).is_err());
        assert!(parse_srx(&srx(max)).is_ok());
        let err = parse_srx(&srx(max + 1)).unwrap_err();
        assert!(err.contains("nests triple terms"), "{err}");
    }

    #[test]
    fn srx_typed_literal_and_bnode_and_entities() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="n"/><variable name="b"/><variable name="t"/></head>
              <results>
                <result>
                  <binding name="n"><literal datatype="http://www.w3.org/2001/XMLSchema#integer">42</literal></binding>
                  <binding name="b"><bnode>b0</bnode></binding>
                  <binding name="t"><literal>a &amp; b &#38; c &#x3C; d</literal></binding>
                </result>
              </results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert_eq!(
            rel.rows[0][0],
            Some(Term::Literal(Literal::new_typed_literal(
                "42",
                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap()
            )))
        );
        assert_eq!(
            rel.rows[0][1],
            Some(Term::BlankNode(oxrdf::BlankNode::new("b0").unwrap()))
        );
        // Predefined (&amp; -> &), decimal (&#38; -> &) and hex (&#x3C; -> <) refs all decode.
        assert_eq!(
            rel.rows[0][2],
            Some(Term::Literal(Literal::new_simple_literal("a & b & c < d")))
        );
    }

    #[test]
    fn srx_dir_lang_string_direction_roundtrips() {
        for (dir, want) in [
            ("ltr", oxrdf::BaseDirection::Ltr),
            ("rtl", oxrdf::BaseDirection::Rtl),
        ] {
            let body = format!(
                r#"<sparql xmlns="{SRX_NS}">
                  <head><variable name="g"/></head>
                  <results><result>
                    <binding name="g"><literal xml:lang="ar" its:dir="{dir}">مرحبا</literal></binding>
                  </result></results>
                </sparql>"#
            );
            let rel = parse_srx(&body).unwrap();
            match &rel.rows[0][0] {
                Some(Term::Literal(l)) => {
                    assert_eq!(l.value(), "مرحبا");
                    assert_eq!(l.language(), Some("ar"));
                    assert_eq!(l.direction(), Some(want), "its:dir={dir} must survive");
                }
                other => panic!("expected a directional literal, got {other:?}"),
            }
        }
    }

    #[test]
    fn srx_triple_term_parses() {
        // SPARQL 1.2 triple-term value: <<( <s> <p> "o" )>>.
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="t"/></head>
              <results><result>
                <binding name="t"><triple>
                  <subject><uri>http://ex/s</uri></subject>
                  <predicate><uri>http://ex/p</uri></predicate>
                  <object><literal>o</literal></object>
                </triple></binding>
              </result></results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        let want = Term::Triple(Box::new(Triple {
            subject: NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            predicate: NamedNode::new("http://ex/p").unwrap(),
            object: Term::Literal(Literal::new_simple_literal("o")),
        }));
        assert_eq!(rel.rows[0][0], Some(want));
    }

    #[test]
    fn srx_empty_results_is_ok() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}"><head><variable name="x"/></head><results></results></sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert!(rel.rows.is_empty());
        assert_eq!(rel.vars.len(), 1);
    }

    #[test]
    fn srx_ask_boolean_is_rejected() {
        // SERVICE always wraps a SELECT, so an ASK boolean body is an error (mirrors SRJ).
        let body =
            format!(r#"<sparql xmlns="{SRX_NS}"><head/><boolean>true</boolean></sparql>"#);
        assert!(parse_srx(&body).is_err());
    }

    /// Elements are matched by namespace as well as local name: one in another namespace,
    /// or with an undeclared prefix, is ignored even when its local name is a results
    /// element's, and a declaration stops applying when its element ends. Both readers.
    #[test]
    fn srx_ignores_elements_in_other_namespaces() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}" xmlns:x="http://ex/other#">
              <head><variable name="a"/><variable name="b"/><x:variable name="c"/></head>
              <results>
                <ext xmlns="http://ex/other#"><result/><boolean>true</boolean></ext>
                <ext xmlns="http://ex/other#"/>
                <its xmlns="http://www.w3.org/2005/11/its"><result>
                  <binding name="a"><uri>http://ex/its</uri></binding>
                </result></its>
                <result>
                  <binding name="a"><literal>ke<x:literal/>pt</literal></binding>
                  <binding name="b"><x:uri>http://ex/foreign</x:uri></binding>
                  <x:binding name="a"><uri>http://ex/foreign</uri></x:binding>
                  <y:binding name="b"><uri>http://ex/undeclared</uri></y:binding>
                </result>
                <x:result><binding name="a"><uri>http://ex/foreign</uri></binding></x:result>
              </results>
            </sparql>"#
        );
        let check = |vars: &[Variable], rows: &[Vec<Option<Term>>]| {
            let names: Vec<&str> = vars.iter().map(Variable::as_str).collect();
            assert_eq!(names, ["a", "b"]);
            assert_eq!(rows.len(), 1);
            let kept = Term::Literal(Literal::new_simple_literal("kept"));
            assert_eq!(rows[0], [Some(kept), None]);
        };
        let rel = parse_srx(&body).unwrap();
        check(&rel.vars, &rel.rows);
        let mut rows = Vec::new();
        let vars = parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        check(&vars, &rows);
    }

    /// Parses `body` with both the in-memory and the streaming reader, checking they agree.
    fn parse_srx_both(body: &str) -> ServiceRelation {
        let rel = parse_srx(body).unwrap();
        let mut rows = Vec::new();
        let vars = parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!((&vars, &rows), (&rel.vars, &rel.rows));
        rel
    }

    /// The results namespace may be bound to a prefix, or left out altogether, also by
    /// undeclaring the default namespace with `xmlns=""`; a declaration is compared once
    /// its character references are replaced.
    #[test]
    fn srx_reads_a_prefixed_or_absent_results_namespace() {
        let prefixed = format!(
            r#"<s:sparql xmlns:s="{SRX_NS}">
              <s:head><s:variable name="a"/></s:head>
              <s:results><s:result>
                <s:binding name="a"><s:uri>http://ex/a</s:uri></s:binding>
              </s:result></s:results>
            </s:sparql>"#
        );
        let bare = r#"<sparql><head><variable name="a"/></head><results><result>
            <binding name="a"><uri>http://ex/a</uri></binding></result></results></sparql>"#;
        let undeclared = format!(
            r#"<sparql xmlns="{SRX_NS}"><head><variable name="a"/></head><results>
              <result xmlns=""><binding name="a"><uri>http://ex/a</uri></binding></result>
            </results></sparql>"#
        );
        let escaped = SRX_NS.replace('#', "&#35;");
        let referenced = [
            format!(
                r#"<sparql xmlns="{escaped}"><head><variable name="a"/></head><results>
                  <result><binding name="a"><uri>http://ex/a</uri></binding></result>
                </results></sparql>"#
            ),
            prefixed.replace(&format!(r#"xmlns:s="{SRX_NS}""#), &format!(r#"xmlns:s="{escaped}""#)),
        ];
        assert!(referenced.iter().all(|body| body.contains("&#35;")));
        for body in [prefixed.as_str(), bare, undeclared.as_str()]
            .into_iter()
            .chain(referenced.iter().map(String::as_str))
        {
            let rel = parse_srx_both(body);
            assert_eq!(
                rel.rows,
                [[Some(Term::NamedNode(NamedNode::new("http://ex/a").unwrap()))]]
            );
        }
    }

    /// Attributes are matched by namespace too: `name` and `datatype` only unprefixed,
    /// `lang` only as `xml:lang`, and `dir` only in the ITS namespace (or an undeclared
    /// `its` prefix). A foreign attribute with one of those local names changes nothing.
    #[test]
    fn srx_ignores_attributes_in_other_namespaces() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}" xmlns:x="http://ex/other#"
                xmlns:i="http://www.w3.org/2005/11/its">
              <head><variable name="a"/><variable name="b"/><variable x:name="c"/></head>
              <results><result>
                <binding x:name="b" name="a"><literal
                  x:lang="fr" xml:lang="en" x:dir="ltr" i:dir="rtl">t</literal></binding>
                <binding name="b"><literal x:datatype="http://ex/dt" lang="fr">u</literal></binding>
              </result></results>
            </sparql>"#
        );
        let rel = parse_srx_both(&body);
        let names: Vec<&str> = rel.vars.iter().map(Variable::as_str).collect();
        assert_eq!(names, ["a", "b"]);
        let a = Literal::new_directional_language_tagged_literal(
            "t",
            "en",
            oxrdf::BaseDirection::Rtl,
        )
        .unwrap();
        let b = Literal::new_simple_literal("u");
        assert_eq!(rel.rows, [[Some(Term::Literal(a)), Some(Term::Literal(b))]]);
    }

    /// The core bead requirement: an endpoint that ignores `Accept` and returns XML must
    /// still be parsed (content-sniffed) by the end-to-end path, not just `parse_srj`.
    #[test]
    fn eval_remote_handles_xml_endpoint() {
        let body = r#"<?xml version="1.0"?>
            <sparql xmlns="http://www.w3.org/2005/sparql-results#">
              <head><variable name="x"/></head>
              <results><result><binding name="x"><uri>http://ex/1</uri></binding></result></results>
            </sparql>"#;
        let rel = eval_remote(&Canned(body), "http://unused/", "SELECT * WHERE {}").unwrap();
        assert_eq!(rel.rows.len(), 1);
        assert_eq!(
            rel.rows[0][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/1").unwrap()))
        );
    }

    #[test]
    fn parse_results_rejects_non_json_non_xml() {
        // A leading byte that is neither `{` nor `<` (e.g. an HTML error page without a
        // doctype, or plain text) is reported rather than silently mis-parsed.
        assert!(parse_results("connection reset by peer").is_err());
        assert!(parse_results("").is_err());
        // Leading whitespace before the sniff byte is tolerated.
        assert!(parse_results("   {\"head\":{\"vars\":[\"x\"]},\"results\":{\"bindings\":[]}}").is_ok());
    }

    // ------------------------------------------------------------------
    // Bind-join block-size knob direct unit tests [OPUS-4.8] (sq-sjkj)
    // ------------------------------------------------------------------

    /// `bind_block_size()` returns the default when no scope is installed.
    #[test]
    fn bind_block_size_returns_default_outside_scope() {
        // [OPUS-4.8] sq-sjkj: direct coverage for the public accessor.
        // The default is DEFAULT_BIND_BLOCK (50) when no override is active.
        let s = bind_block_size();
        assert_eq!(s, DEFAULT_BIND_BLOCK, "default bind-block size must be DEFAULT_BIND_BLOCK");
    }

    /// `with_service_bound_join_block_size` scopes the override and restores it.
    #[test]
    fn with_service_bound_join_block_size_scopes_and_restores() {
        // [OPUS-4.8] sq-sjkj: direct coverage for the scoped override entry point.
        let before = bind_block_size();
        with_service_bound_join_block_size(999, || {
            assert_eq!(bind_block_size(), 999, "override must be active inside scope");
        });
        // The previous value is restored after the scope.
        assert_eq!(bind_block_size(), before, "override must be gone after scope");
    }

    /// `with_service_bound_join_block_size(0, …)` is clamped to 1.
    #[test]
    fn with_service_bound_join_block_size_zero_is_clamped_to_one() {
        // [OPUS-4.8] sq-sjkj: a zero block size is clamped to 1 so a tuple still
        // gets pushed one-per-request rather than silently disabling the knob.
        with_service_bound_join_block_size(0, || {
            assert_eq!(bind_block_size(), 1, "zero must be clamped to 1");
        });
    }

    // ------------------------------------------------------------------
    // Per-query remote-request cap direct unit tests [OPUS-4.8] (sq-b93pv)
    // ------------------------------------------------------------------

    /// `remote_request_cap()` returns `None` when no scope is installed.
    #[test]
    fn remote_request_cap_returns_none_outside_scope() {
        // [OPUS-4.8] sq-b93pv: direct coverage for the public accessor.
        // The default is uncapped (None) when no override is active.
        let cap = remote_request_cap();
        assert_eq!(cap, None, "default remote-request cap must be None (uncapped)");
    }

    /// `with_service_remote_request_cap` scopes the cap and restores it.
    #[test]
    fn with_service_remote_request_cap_scopes_and_restores() {
        // [OPUS-4.8] sq-b93pv: direct coverage for the scoped cap entry point.
        assert_eq!(remote_request_cap(), None); // no prior override
        with_service_remote_request_cap(8, || {
            assert_eq!(remote_request_cap(), Some(8), "cap must be active inside scope");
        });
        assert_eq!(remote_request_cap(), None, "cap must be gone after scope");
    }

    /// Canned-response transport: proves `eval_remote` wires the transport into the
    /// parser without touching the network.
    struct Canned(&'static str);
    impl Transport for Canned {
        fn fetch(&self, _endpoint: &str, _query: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn eval_remote_uses_injected_transport() {
        let body = r#"{"head":{"vars":["x"]},
            "results":{"bindings":[{"x":{"type":"uri","value":"http://ex/1"}}]}}"#;
        let rel = eval_remote(&Canned(body), "http://unused/", "SELECT * WHERE {}").unwrap();
        assert_eq!(rel.rows.len(), 1);
    }

    struct Failing;
    impl Transport for Failing {
        fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
            Err("connection refused".into())
        }
    }

    #[test]
    fn eval_remote_propagates_transport_error() {
        let err = eval_remote(&Failing, "http://unused/", "SELECT * WHERE {}").unwrap_err();
        assert!(err.contains("connection refused"));
    }

    // ---------------------------------------------------------------------
    // SSRF egress policy [OPUS-4.8] (bead sq-2v6f)
    // ---------------------------------------------------------------------

    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    /// [OPUS-4.8] (sq-a7jw4) Host-level `is_allowed` check at a representative port. A
    /// host-level (no-`:port`) entry matches EVERY port, so any port reads the same — these
    /// host-level tests use 80. Port-scoping tests call `is_allowed(host, port)` directly.
    fn allowed(host: &str) -> bool {
        egress_policy::is_allowed(host, 80)
    }

    #[test]
    fn loopback_is_forbidden() {
        assert!(is_forbidden_ip(v4(127, 0, 0, 1)));
        assert!(is_forbidden_ip(v4(127, 255, 255, 254))); // anywhere in 127/8
        assert!(is_forbidden_ip(IpAddr::V6(Ipv6Addr::LOCALHOST))); // ::1
    }

    #[test]
    fn rfc1918_private_is_forbidden() {
        assert!(is_forbidden_ip(v4(10, 0, 0, 1))); // 10/8
        assert!(is_forbidden_ip(v4(10, 255, 255, 255)));
        assert!(is_forbidden_ip(v4(172, 16, 0, 1))); // 172.16/12 (low edge)
        assert!(is_forbidden_ip(v4(172, 31, 255, 255))); // 172.16/12 (high edge)
        assert!(is_forbidden_ip(v4(192, 168, 0, 1))); // 192.168/16
    }

    #[test]
    fn link_local_and_cloud_metadata_are_forbidden() {
        assert!(is_forbidden_ip(v4(169, 254, 0, 1))); // 169.254/16
        // The cloud-metadata endpoint — the highest-value SSRF target.
        assert!(is_forbidden_ip(v4(169, 254, 169, 254)));
        // IPv6 link-local fe80::/10.
        assert!(is_forbidden_ip(IpAddr::V6("fe80::1".parse().unwrap())));
        assert!(is_forbidden_ip(IpAddr::V6("febf::1".parse().unwrap()))); // top of /10
    }

    #[test]
    fn unique_local_v6_is_forbidden() {
        assert!(is_forbidden_ip(IpAddr::V6("fc00::1".parse().unwrap()))); // fc00::/7
        assert!(is_forbidden_ip(IpAddr::V6("fd12:3456::1".parse().unwrap())));
    }

    #[test]
    fn unspecified_is_forbidden() {
        assert!(is_forbidden_ip(v4(0, 0, 0, 0)));
        assert!(is_forbidden_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED))); // ::
    }

    #[test]
    fn cgnat_and_broadcast_are_forbidden() {
        assert!(is_forbidden_ip(v4(100, 64, 0, 1))); // 100.64/10 CGNAT (low)
        assert!(is_forbidden_ip(v4(100, 127, 255, 255))); // CGNAT (high)
        assert!(!is_forbidden_ip(v4(100, 63, 0, 1))); // just below CGNAT — public
        assert!(!is_forbidden_ip(v4(100, 128, 0, 1))); // just above CGNAT — public
        assert!(is_forbidden_ip(v4(255, 255, 255, 255))); // broadcast
    }

    #[test]
    fn ipv4_mapped_v6_is_unwrapped_and_classified() {
        // ::ffff:127.0.0.1 must be refused as the embedded private v4.
        assert!(is_forbidden_ip(IpAddr::V6("::ffff:127.0.0.1".parse().unwrap())));
        assert!(is_forbidden_ip(IpAddr::V6("::ffff:10.0.0.1".parse().unwrap())));
        assert!(is_forbidden_ip(IpAddr::V6("::ffff:169.254.169.254".parse().unwrap())));
        // A public v4 mapped into v6 is still allowed.
        assert!(!is_forbidden_ip(IpAddr::V6("::ffff:8.8.8.8".parse().unwrap())));
    }

    #[test]
    fn public_addresses_are_allowed() {
        assert!(!is_forbidden_ip(v4(8, 8, 8, 8))); // Google DNS
        assert!(!is_forbidden_ip(v4(1, 1, 1, 1))); // Cloudflare DNS
        assert!(!is_forbidden_ip(v4(93, 184, 216, 34))); // example.com (historical)
        assert!(!is_forbidden_ip(v4(172, 15, 0, 1))); // just below 172.16/12 — public
        assert!(!is_forbidden_ip(v4(172, 32, 0, 1))); // just above 172.16/12 — public
        assert!(!is_forbidden_ip(IpAddr::V6("2001:4860:4860::8888".parse().unwrap()))); // public v6
    }

    #[test]
    fn allowlist_plumbing_install_and_restore() {
        // Default: nothing is allowlisted.
        assert!(!allowed("localhost"));
        {
            let _g = egress_policy::install(
                ["localhost".to_string(), "10.0.0.5".to_string()],
                egress_policy::Mode::DenyPrivate,
            );
            assert!(allowed("localhost"));
            assert!(allowed("LOCALHOST")); // case-insensitive
            assert!(allowed("10.0.0.5"));
            assert!(!allowed("other.host"));
        }
        // Restored to empty on guard drop.
        assert!(!allowed("localhost"));
    }

    #[test]
    fn with_service_egress_allow_scopes_the_allowlist() {
        assert!(!allowed("sparql.internal"));
        let seen = with_service_egress_allow(["sparql.internal".to_string()], || {
            allowed("sparql.internal")
        });
        assert!(seen);
        // Allowlist is gone after the scope returns.
        assert!(!allowed("sparql.internal"));
    }

    #[test]
    fn strict_allowlist_only_mode_scopes_and_restores() {
        // [OPUS-4.8] (sq-4w18) Strict mode: only listed hosts are allowed; the mode
        // and allowlist both restore on scope exit.
        assert_eq!(egress_policy::mode(), egress_policy::Mode::DenyPrivate);
        assert!(!allowed("a.example"));
        with_service_egress_policy(true, ["a.example".to_string()], || {
            assert_eq!(egress_policy::mode(), egress_policy::Mode::AllowlistOnly);
            assert!(allowed("a.example"));
            assert!(allowed("A.EXAMPLE")); // case-insensitive
            assert!(!allowed("b.example"));
        });
        assert_eq!(egress_policy::mode(), egress_policy::Mode::DenyPrivate);
        assert!(!allowed("a.example"));
    }

    #[test]
    fn suffix_wildcard_allowlist_matches_apex_and_subdomains() {
        // [OPUS-4.8] (sq-4w18) A ".example.org" entry matches the apex and any
        // subdomain, but not a host that merely ends in the same letters.
        with_service_egress_policy(true, [".example.org".to_string()], || {
            assert!(allowed("example.org")); // apex
            assert!(allowed("sparql.example.org")); // subdomain
            assert!(allowed("a.b.example.org")); // deep subdomain
            assert!(allowed("SPARQL.EXAMPLE.ORG")); // case-insensitive
            assert!(!allowed("notexample.org")); // boundary respected
            assert!(!allowed("example.org.evil.com")); // suffix only
        });
    }

    #[test]
    fn non_strict_policy_matches_allow_helper() {
        // strict=false behaves exactly like with_service_egress_allow (DenyPrivate).
        with_service_egress_policy(false, ["c.example".to_string()], || {
            assert_eq!(egress_policy::mode(), egress_policy::Mode::DenyPrivate);
            assert!(allowed("c.example"));
        });
    }

    #[test]
    fn allowlist_restores_on_unwind() {
        // A panic inside the scope must still restore the previous (empty) policy —
        // a relaxed allowlist must never leak past the scope on unwind.
        let _ = std::panic::catch_unwind(|| {
            with_service_egress_allow(["leaky.host".to_string()], || {
                assert!(allowed("leaky.host"));
                panic!("boom");
            });
        });
        assert!(!allowed("leaky.host"));
    }

    // ---------------------------------------------------------------------
    // Port-scoped allowlist entries [OPUS-4.8] (bead sq-a7jw4)
    // ---------------------------------------------------------------------

    #[test]
    fn split_entry_parses_host_and_optional_port() {
        // Direct unit test for the entry parser (coverage ratchet).
        use egress_policy::split_entry;
        // Host-level (no port constraint).
        assert_eq!(split_entry("sparql.internal"), ("sparql.internal", None));
        assert_eq!(split_entry("127.0.0.1"), ("127.0.0.1", None));
        assert_eq!(split_entry(".example.org"), (".example.org", None));
        // Port-scoped.
        assert_eq!(split_entry("127.0.0.1:8053"), ("127.0.0.1", Some(8053)));
        assert_eq!(split_entry("sparql.internal:8443"), ("sparql.internal", Some(8443)));
        assert_eq!(split_entry(".example.org:443"), (".example.org", Some(443)));
        // Bracketed IPv6 — brackets stripped from the host pattern.
        assert_eq!(split_entry("[::1]:8080"), ("::1", Some(8080)));
        assert_eq!(split_entry("[2001:db8::1]:443"), ("2001:db8::1", Some(443)));
        assert_eq!(split_entry("[::1]"), ("::1", None));
        // Bare (unbracketed) IPv6 — NOT amputated; host-level.
        assert_eq!(split_entry("::1"), ("::1", None));
        assert_eq!(split_entry("2001:db8::1"), ("2001:db8::1", None));
        // Out-of-range / empty / non-numeric port: kept as host pattern (fail-closed).
        assert_eq!(split_entry("127.0.0.1:99999"), ("127.0.0.1:99999", None));
        assert_eq!(split_entry("127.0.0.1:"), ("127.0.0.1:", None));
        assert_eq!(split_entry("127.0.0.1:http"), ("127.0.0.1:http", None));
    }

    #[test]
    fn allowlist_entry_permits_is_the_shared_per_entry_rule() {
        // Direct unit test for the PUBLIC per-entry predicate that `sparq-fedclient` reuses
        // (coverage ratchet + the one-source-of-truth contract for bead sq-vbnyc). It is the
        // pure, stateless form of `egress_policy::is_allowed`'s `.any(…)` closure — no policy
        // install required.
        use super::allowlist_entry_permits as permits;
        // Host-level entry: all ports, case-insensitive host, suffix wildcard.
        assert!(permits("sparql.internal", "sparql.internal", 80));
        assert!(permits("sparql.internal", "SPARQL.INTERNAL", 8443)); // host case-insensitive
        assert!(permits(".example.org", "a.example.org", 443));
        assert!(permits(".example.org", "example.org", 80)); // apex included
        assert!(!permits(".example.org", "notexample.org", 80)); // boundary respected
        // Port-scoped entry: exact port only.
        assert!(permits("127.0.0.1:8053", "127.0.0.1", 8053));
        assert!(!permits("127.0.0.1:8053", "127.0.0.1", 8054)); // other port rejected
        assert!(!permits("127.0.0.1:8053", "127.0.0.2", 8053)); // other host rejected
        // Bracketed IPv6 + port; bare IPv6 host-level.
        assert!(permits("[::1]:8080", "::1", 8080));
        assert!(!permits("[::1]:8080", "::1", 80));
        assert!(permits("2001:db8::1", "2001:db8::1", 443)); // bare IPv6 = all ports
        // Malformed port → never-matching host pattern (fail-closed, never widened).
        assert!(!permits("127.0.0.1:99999", "127.0.0.1", 80));
        assert!(!permits("127.0.0.1:", "127.0.0.1", 80));
    }

    #[test]
    fn port_scoped_entry_permits_exact_port_only() {
        // A `host:port` entry permits ONLY that host on THAT port, and rejects the
        // same host on any OTHER port — strictly narrower than a host-level entry.
        with_service_egress_policy(true, ["127.0.0.1:8053".to_string()], || {
            assert!(egress_policy::is_allowed("127.0.0.1", 8053)); // (a) exact host:port
            assert!(!egress_policy::is_allowed("127.0.0.1", 8054)); // (b) other port rejected
            assert!(!egress_policy::is_allowed("127.0.0.1", 80)); //   (b) default port rejected
            assert!(!egress_policy::is_allowed("127.0.0.2", 8053)); // (c) different host rejected
        });
    }

    #[test]
    fn host_level_entry_permits_all_ports_backward_compat() {
        // (d) An existing host-level entry (no port) keeps its meaning: every port on
        // that host. This is the unchanged sq-4w18 semantics.
        with_service_egress_policy(true, ["127.0.0.1".to_string()], || {
            assert!(egress_policy::is_allowed("127.0.0.1", 80));
            assert!(egress_policy::is_allowed("127.0.0.1", 8053));
            assert!(egress_policy::is_allowed("127.0.0.1", 65535));
            assert!(!egress_policy::is_allowed("127.0.0.2", 80)); // different host still rejected
        });
    }

    #[test]
    fn port_scoped_suffix_wildcard_is_port_constrained() {
        // A port on a suffix-wildcard entry constrains the port too: `.example.org:443`
        // permits any subdomain on 443 only.
        with_service_egress_policy(true, [".example.org:443".to_string()], || {
            assert!(egress_policy::is_allowed("sparql.example.org", 443)); // subdomain on 443
            assert!(egress_policy::is_allowed("example.org", 443)); // apex on 443
            assert!(!egress_policy::is_allowed("sparql.example.org", 80)); // wrong port
            assert!(!egress_policy::is_allowed("notexample.org", 443)); // boundary respected
        });
    }

    #[test]
    fn bracketed_ipv6_port_scoped_entry() {
        // A bracketed IPv6 `[::1]:8080` entry is port-scoped on the bare `::1` host.
        with_service_egress_policy(true, ["[::1]:8080".to_string()], || {
            assert!(egress_policy::is_allowed("::1", 8080));
            assert!(!egress_policy::is_allowed("::1", 80));
        });
    }

    #[test]
    fn bare_ipv6_entry_is_host_level_not_port_amputated() {
        // A bare (unbracketed) IPv6 literal must NOT have its last hextet read as a
        // port — `2001:db8::1` is a host-level entry matching every port.
        with_service_egress_policy(true, ["2001:db8::1".to_string()], || {
            assert!(egress_policy::is_allowed("2001:db8::1", 443));
            assert!(egress_policy::is_allowed("2001:db8::1", 80));
        });
    }

    #[test]
    fn mixed_host_and_port_scoped_entries_for_same_host() {
        // Two entries for the same loopback host: a host-level `127.0.0.1` (all ports)
        // and a redundant port-scoped one. The host-level entry wins for any port —
        // additive allowlists only ever widen, never remove a granted port.
        with_service_egress_policy(
            true,
            ["127.0.0.1".to_string(), "10.0.0.5:9000".to_string()],
            || {
                assert!(egress_policy::is_allowed("127.0.0.1", 1234)); // host-level: any port
                assert!(egress_policy::is_allowed("10.0.0.5", 9000)); // port-scoped: exact
                assert!(!egress_policy::is_allowed("10.0.0.5", 9001)); // port-scoped: other port
            },
        );
    }

    #[test]
    fn malformed_port_in_entry_fails_closed() {
        // A `:port` that does not parse as a u16 is treated as part of the (never-matching)
        // host pattern, NOT as "drop the port constraint and match every port" — fail-closed.
        with_service_egress_policy(true, ["127.0.0.1:99999".to_string()], || {
            // 99999 > u16::MAX: the entry matches no real authority at all.
            assert!(!egress_policy::is_allowed("127.0.0.1", 80));
            assert!(!egress_policy::is_allowed("127.0.0.1", 65535));
        });
    }

    // The resolver path is native-only (it wraps ureq's Resolver).
    #[cfg(not(target_arch = "wasm32"))]
    mod resolver {
        use super::*;

        /// [OPUS-4.8] sq-g2xs: invoke the ureq-3 `Resolver` for a `host:port` netloc by building
        /// the `http://<netloc>/` URI the resolver parses (default `Config` + no-deadline timeout).
        fn resolve_netloc(
            netloc: &str,
        ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
            use ureq::unversioned::resolver::Resolver;
            let uri: ureq::http::Uri = format!("http://{netloc}/").parse().unwrap();
            let config = ureq::Agent::config_builder().build();
            let timeout = ureq::unversioned::transport::NextTimeout {
                after: ureq::unversioned::transport::time::Duration::NotHappening,
                reason: ureq::Timeout::Global,
            };
            EgressFilterResolver.resolve(&uri, &config, timeout)
        }

        /// `true` iff `e` is the egress-refusal `PermissionDenied` io error.
        fn is_permission_denied(e: &ureq::Error) -> bool {
            matches!(e, ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied)
        }

        #[test]
        fn resolver_refuses_loopback_endpoint() {
            // 127.0.0.1 resolves to itself; with no allowlist the policy must
            // refuse it with PermissionDenied — never returning a dial-able addr.
            let err = resolve_netloc("127.0.0.1:8080").unwrap_err();
            assert!(is_permission_denied(&err), "got {err:?}");
        }

        #[test]
        fn resolver_refuses_cloud_metadata_endpoint() {
            let err = resolve_netloc("169.254.169.254:80").unwrap_err();
            assert!(is_permission_denied(&err), "got {err:?}");
        }

        #[test]
        fn resolver_refuses_ipv6_loopback_endpoint() {
            // ureq passes IPv6 netlocs bracketed.
            let err = resolve_netloc("[::1]:80").unwrap_err();
            assert!(is_permission_denied(&err), "got {err:?}");
        }

        #[test]
        fn resolver_allows_public_endpoint() {
            // 8.8.8.8 is a literal so no DNS lookup happens; it is global, so it
            // passes the filter and comes back as a dial-able address.
            let addrs = resolve_netloc("8.8.8.8:443").unwrap();
            assert_eq!(addrs.len(), 1);
            assert_eq!(addrs[0].ip(), v4(8, 8, 8, 8));
        }

        #[test]
        fn resolver_permits_allowlisted_private_endpoint() {
            // With 127.0.0.1 on the allowlist, the loopback endpoint is dial-able.
            let addrs = with_service_egress_allow(["127.0.0.1".to_string()], || {
                resolve_netloc("127.0.0.1:8080")
            })
            .unwrap();
            assert_eq!(addrs.len(), 1);
            assert!(addrs[0].ip().is_loopback());
        }

        // [OPUS-4.8] (sq-4w18) Strict allowlist-only mode — the server's policy.

        #[test]
        fn strict_refuses_public_host_off_the_allowlist() {
            let err = with_service_egress_policy(true, std::iter::empty(), || {
                resolve_netloc("8.8.8.8:443")
            })
            .unwrap_err();
            assert!(is_permission_denied(&err), "got {err:?}");
        }

        #[test]
        fn strict_empty_allowlist_denies_all() {
            for netloc in ["8.8.8.8:443", "1.1.1.1:80", "127.0.0.1:8080"] {
                let err = with_service_egress_policy(true, std::iter::empty(), || {
                    resolve_netloc(netloc)
                })
                .unwrap_err();
                assert!(is_permission_denied(&err), "{netloc} must be refused, got {err:?}");
            }
        }

        #[test]
        fn strict_permits_allowlisted_host() {
            let addrs = with_service_egress_policy(true, ["8.8.8.8".to_string()], || {
                resolve_netloc("8.8.8.8:443")
            })
            .unwrap();
            assert_eq!(addrs.len(), 1);
            assert_eq!(addrs[0].ip(), v4(8, 8, 8, 8));

            let addrs = with_service_egress_policy(true, ["127.0.0.1".to_string()], || {
                resolve_netloc("127.0.0.1:8080")
            })
            .unwrap();
            assert_eq!(addrs.len(), 1);
            assert!(addrs[0].ip().is_loopback());
        }

        #[test]
        fn non_strict_resolver_allows_public_off_list() {
            let addrs = with_service_egress_policy(false, std::iter::empty(), || {
                resolve_netloc("8.8.8.8:443")
            })
            .unwrap();
            assert_eq!(addrs.len(), 1);
        }

        // [OPUS-4.8] (sq-a7jw4) Port-scoped allowlist entries at the resolver — the real
        // egress path, not just the `is_allowed` predicate.

        #[test]
        fn resolver_port_scoped_permits_exact_port_rejects_others() {
            // A `127.0.0.1:8053` entry lets the loopback endpoint be dialled on 8053 only;
            // the SAME loopback host on any other port is refused (the private-IP default-deny
            // re-applies because the allowlist exemption does not match the port).
            let addrs = with_service_egress_policy(true, ["127.0.0.1:8053".to_string()], || {
                resolve_netloc("127.0.0.1:8053")
            })
            .unwrap();
            assert_eq!(addrs.len(), 1);
            assert!(addrs[0].ip().is_loopback());
            assert_eq!(addrs[0].port(), 8053);

            let err = with_service_egress_policy(true, ["127.0.0.1:8053".to_string()], || {
                resolve_netloc("127.0.0.1:9999")
            })
            .unwrap_err();
            assert!(is_permission_denied(&err), "other port must be refused, got {err:?}");
        }

        #[test]
        fn resolver_in_process_loopback_service_use_case() {
            // The bead's load-bearing use case: an in-process mock SERVICE endpoint bound to
            // an EPHEMERAL loopback port. Permit EXACTLY 127.0.0.1:<ephemeral>; the same host
            // on a different port (a co-resident service, or an attacker probing) is refused.
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let ephemeral = listener.local_addr().unwrap().port();
            let other = ephemeral.wrapping_add(1).max(1);
            let entry = format!("127.0.0.1:{ephemeral}");

            let permitted = with_service_egress_policy(true, [entry.clone()], || {
                resolve_netloc(&format!("127.0.0.1:{ephemeral}"))
            });
            let addrs = permitted.expect("the exact ephemeral loopback endpoint must be dial-able");
            assert_eq!(addrs.len(), 1);
            assert!(addrs[0].ip().is_loopback());
            assert_eq!(addrs[0].port(), ephemeral);

            if other != ephemeral {
                let err = with_service_egress_policy(true, [entry], || {
                    resolve_netloc(&format!("127.0.0.1:{other}"))
                })
                .unwrap_err();
                assert!(
                    is_permission_denied(&err),
                    "127.0.0.1:{other} (a different port) must be refused, got {err:?}"
                );
            }
        }

        #[test]
        fn resolver_port_scoped_does_not_weaken_dns_rebind_revet() {
            // The port-scoping must NOT bypass the resolved-IP re-vet. A `127.0.0.1:8053`
            // allowlist entry permits the loopback host on 8053; but if the SAME port were
            // dialled toward a DIFFERENT private host that is NOT the allowlisted one
            // (here 169.254.169.254, the cloud-metadata IP), the per-IP `is_forbidden_ip`
            // check still rejects it — the allowlist exemption is keyed to the host pattern,
            // so a rebind to an off-allowlist IP on the very same port is still refused.
            let err = with_service_egress_policy(true, ["127.0.0.1:8053".to_string()], || {
                resolve_netloc("169.254.169.254:8053")
            })
            .unwrap_err();
            assert!(
                is_permission_denied(&err),
                "an off-allowlist private IP on the scoped port must still be re-vetted, got {err:?}"
            );
        }

        #[test]
        fn resolver_host_level_entry_permits_all_ports() {
            // Backward compat at the resolver: a bare `127.0.0.1` entry dials on any port.
            for port in [80u16, 8053, 65535] {
                let addrs = with_service_egress_policy(true, ["127.0.0.1".to_string()], || {
                    resolve_netloc(&format!("127.0.0.1:{port}"))
                })
                .unwrap();
                assert_eq!(addrs.len(), 1);
                assert_eq!(addrs[0].port(), port);
            }
        }
    }

    // ---------------------------------------------------------------------
    // Streaming / bounded result consumption [FABLE-5] (bead sq-my8wd.4)
    // ---------------------------------------------------------------------

    mod streaming_equivalence {
        use super::*;

        /// FROZEN pre-sq-my8wd.4 whole-DOM implementation of `parse_srj`, kept
        /// verbatim as the result-equivalence oracle: the streaming parser must
        /// produce the SAME relation (rows, multiplicity AND order) and the SAME
        /// errors. `srj_term` is shared deliberately — it is unchanged by the
        /// streaming rework; the delta under test is the document walk.
        fn parse_srj_reference(text: &str) -> Result<ServiceRelation, String> {
            let v: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| format!("SERVICE: invalid results JSON: {e}"))?;
            if v.get("boolean").is_some() {
                return Err(
                    "SERVICE: endpoint returned an ASK boolean, expected SELECT bindings".into(),
                );
            }
            let vars: Vec<Variable> = v
                .pointer("/head/vars")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .map(|s| {
                            Variable::new(s)
                                .map_err(|e| format!("SERVICE: bad result variable {s:?}: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .ok_or_else(|| "SERVICE: results JSON missing head.vars".to_string())?;

            let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
            for sol in v
                .pointer("/results/bindings")
                .and_then(|a| a.as_array())
                .ok_or_else(|| "SERVICE: results JSON missing results.bindings".to_string())?
            {
                let obj = sol.as_object().ok_or_else(|| {
                    "SERVICE: a solution binding is not a JSON object".to_string()
                })?;
                let mut row: Vec<Option<Term>> = Vec::with_capacity(vars.len());
                for var in &vars {
                    match obj.get(var.as_str()) {
                        Some(cell) => row.push(Some(srj_term(cell)?)),
                        None => row.push(None),
                    }
                }
                rows.push(row);
            }
            Ok(ServiceRelation { vars, rows })
        }

        /// Assert the streaming parser and the frozen DOM reference agree on one
        /// document — `Ok`: identical vars + rows (order and multiplicity); `Err`:
        /// identical message.
        fn assert_equiv(body: &str) {
            let got = parse_srj(body);
            let want = parse_srj_reference(body);
            assert_eq!(got, want, "streaming vs DOM reference diverged on: {}", body);
        }

        #[test]
        fn equivalence_on_representative_and_boundary_documents() {
            for body in [
                // Boundary: no vars / no rows / a single row.
                r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#,
                r#"{"head":{"vars":["x"]},"results":{"bindings":[]}}"#,
                r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"uri","value":"http://ex/1"}}]}}"#,
                // Unbound cells + solution keys out of head order.
                r#"{"head":{"vars":["a","b"]},"results":{"bindings":[
                    {"b":{"type":"literal","value":"1"}},
                    {"b":{"type":"bnode","value":"b0"},"a":{"type":"uri","value":"http://ex/a"}}]}}"#,
                // Every term kind: uri, bnode, plain / lang / dir-lang / typed
                // literal, legacy typed-literal, SPARQL 1.2 triple term.
                r#"{"head":{"vars":["u","b","l","g","d","t","o","r"]},"results":{"bindings":[{
                    "u":{"type":"uri","value":"http://ex/u"},
                    "b":{"type":"bnode","value":"n1"},
                    "l":{"type":"literal","value":"plain"},
                    "g":{"type":"literal","value":"hi","xml:lang":"en"},
                    "d":{"type":"literal","value":"مرحبا","xml:lang":"ar","its:dir":"rtl"},
                    "t":{"type":"literal","value":"7","datatype":"http://www.w3.org/2001/XMLSchema#integer"},
                    "o":{"type":"typed-literal","value":"1.5","datatype":"http://www.w3.org/2001/XMLSchema#decimal"},
                    "r":{"type":"triple","value":{
                        "subject":{"type":"uri","value":"http://ex/s"},
                        "predicate":{"type":"uri","value":"http://ex/p"},
                        "object":{"type":"literal","value":"o"}}}}]}}"#,
                // Unknown members are ignored (SRJ `link`, arbitrary extras).
                r#"{"head":{"vars":["x"],"link":["http://ex/meta"]},"results":{"bindings":[]},"extra":42}"#,
                // Reversed member order: `results` BEFORE `head` (legal JSON — the
                // documented buffered fallback must be result-identical).
                r#"{"results":{"bindings":[{"x":{"type":"uri","value":"http://ex/r"}}]},"head":{"vars":["x"]}}"#,
                // Leading whitespace before the sniffed `{`.
                " \n\t {\"head\":{\"vars\":[\"x\"]},\"results\":{\"bindings\":[]}}",
                // Non-string entries in head.vars are SKIPPED (filter_map parity).
                r#"{"head":{"vars":["x",5,"y"]},"results":{"bindings":[{"y":{"type":"literal","value":"v"}}]}}"#,
            ] {
                assert_equiv(body);
            }
        }

        #[test]
        fn equivalence_on_malformed_documents() {
            for body in [
                "not json at all",
                "{",                                                   // truncated
                r#"{"head":{}}"#,                                      // missing head.vars
                r#"{"head":{"vars":5}}"#,                              // vars not an array
                r#"{"head":{"vars":["x"]}}"#,                          // missing results
                r#"{"head":{"vars":["x"]},"results":{}}"#,             // results without bindings
                r#"{"head":{"vars":["x"]},"results":5}"#,              // results not an object
                r#"{"head":{"vars":["x"]},"results":[1,2]}"#,          // results an array
                r#"{"head":{"vars":["x"]},"results":{"bindings":5}}"#, // bindings not an array
                r#"{"head":{"vars":["x"]},"results":{"bindings":{}}}"#, // bindings an object
                r#"{"head":{"vars":["x"]},"results":{"bindings":[5]}}"#, // binding not an object
                r#"{"boolean":true}"#,                                 // ASK body
                // ASK key trailing a full SELECT shape — still rejected.
                r#"{"head":{"vars":["x"]},"results":{"bindings":[]},"boolean":false}"#,
                r#"{"head":{"vars":["not a var"]},"results":{"bindings":[]}}"#, // bad var name
                // Bad IRI / unknown binding type inside a solution.
                r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"uri","value":"no spaces"}}]}}"#,
                r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"wat","value":"?"}}]}}"#,
                // Non-object top levels (all "missing head.vars" in the DOM path).
                "[]",
                "5",
                "\"x\"",
                "null",
                "true",
                // Trailing garbage after a valid document.
                r#"{"head":{"vars":["x"]},"results":{"bindings":[]}} trailing"#,
            ] {
                assert_equiv(body);
            }
        }

        #[test]
        fn equivalence_on_a_large_duplicate_heavy_document() {
            // 10_000 rows over 7 distinct solutions: multiplicity AND ordering must
            // survive the streaming rework exactly (row-for-row equality, not just
            // multiset equality).
            let mut bindings = Vec::with_capacity(10_000);
            for i in 0..10_000usize {
                let k = i % 7;
                bindings.push(format!(
                    r#"{{"s":{{"type":"uri","value":"http://ex/dup/{}"}},"n":{{"type":"literal","value":"{}","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}}}"#,
                    k, k
                ));
            }
            let body = format!(
                r#"{{"head":{{"vars":["s","n"]}},"results":{{"bindings":[{}]}}}}"#,
                bindings.join(",")
            );
            let got = parse_srj(&body).unwrap();
            let want = parse_srj_reference(&body).unwrap();
            assert_eq!(got.rows.len(), 10_000);
            assert_eq!(got.vars, want.vars);
            assert_eq!(got.rows, want.rows, "row-for-row identical incl. duplicates");
        }

        #[test]
        fn srj_streams_rows_before_later_input_is_parsed() {
            // Two good rows then a JSON syntax error. A materialise-first parser (the
            // frozen DOM reference) cannot deliver ANY row from such a document; the
            // streaming parser must have delivered both BEFORE reaching the error —
            // the load-bearing "no full-relation buffering" property.
            let body = r#"{"head":{"vars":["x"]},"results":{"bindings":[
                {"x":{"type":"uri","value":"http://ex/1"}},
                {"x":{"type":"uri","value":"http://ex/2"}},
                {{{"#;
            let mut seen = 0usize;
            let err = parse_srj_into(body, &mut |_row| {
                seen += 1;
                Ok(())
            })
            .unwrap_err();
            assert!(err.contains("invalid results JSON"), "got: {}", err);
            assert_eq!(seen, 2, "rows are delivered as parsed, not after the document");
            // The DOM reference, by contrast, delivers nothing from this document.
            assert!(parse_srj_reference(body).is_err());
        }

        #[test]
        fn srx_streams_rows_before_later_input_is_parsed() {
            // Two good results, then a stray `</triple>` the parser rejects: both
            // rows must already have been delivered when the error is reported.
            let body = r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
              <head><variable name="x"/></head>
              <results>
                <result><binding name="x"><uri>http://ex/1</uri></binding></result>
                <result><binding name="x"><uri>http://ex/2</uri></binding></result>
                </triple>
              </results></sparql>"#;
            let mut seen = 0usize;
            let err = parse_srx_into(body, &mut |_row| {
                seen += 1;
                Ok(())
            })
            .unwrap_err();
            assert!(err.contains("SERVICE:"), "got: {}", err);
            assert_eq!(seen, 2, "rows are delivered as parsed, not after the document");
        }

        #[test]
        fn srj_sink_error_aborts_the_parse_and_propagates_verbatim() {
            // A 50_000-row document refused by the sink after 3 rows: the consumer's
            // bound is ENFORCED (the parse stops, no further rows are delivered) and
            // the sink's own error string surfaces unchanged — the seam an embedder's
            // resource policy hangs off.
            let row = r#"{"x":{"type":"uri","value":"http://ex/big"}}"#;
            let body = format!(
                r#"{{"head":{{"vars":["x"]}},"results":{{"bindings":[{}]}}}}"#,
                vec![row; 50_000].join(",")
            );
            let mut seen = 0usize;
            let err = parse_srj_into(&body, &mut |_row| {
                seen += 1;
                if seen >= 3 {
                    Err("row cap exceeded (test sink)".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(err, "row cap exceeded (test sink)", "sink error is verbatim");
            assert_eq!(seen, 3, "no rows are delivered after the sink refuses");
        }

        #[test]
        fn srx_sink_error_aborts_the_parse_and_propagates_verbatim() {
            let mut body = String::from(
                r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
                   <head><variable name="x"/></head><results>"#,
            );
            for _ in 0..10_000 {
                body.push_str(
                    r#"<result><binding name="x"><uri>http://ex/big</uri></binding></result>"#,
                );
            }
            body.push_str("</results></sparql>");
            let mut seen = 0usize;
            let err = parse_srx_into(&body, &mut |_row| {
                seen += 1;
                if seen >= 3 {
                    Err("row cap exceeded (test sink)".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(err, "row cap exceeded (test sink)", "sink error is verbatim");
            assert_eq!(seen, 3, "no rows are delivered after the sink refuses");
        }

        #[test]
        fn eval_remote_into_streams_rows_from_the_transport() {
            // Direct coverage for the streaming end-to-end entry point (the
            // production path exec.rs drives): transport → content sniff → rows.
            let body = r#"{"head":{"vars":["x"]},"results":{"bindings":[
                {"x":{"type":"uri","value":"http://ex/1"}},
                {"x":{"type":"uri","value":"http://ex/2"}}]}}"#;
            let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
            let vars = eval_remote_into(
                &super::Canned(body),
                "http://unused/",
                "SELECT * WHERE {}",
                &mut |r| {
                    rows.push(r);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(vars.len(), 1);
            assert_eq!(rows.len(), 2);
        }

        #[test]
        fn parse_results_into_sniffs_both_formats_and_rejects_neither() {
            let srj = r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"uri","value":"http://ex/j"}}]}}"#;
            let srx = r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
                <head><variable name="x"/></head>
                <results><result><binding name="x"><uri>http://ex/x</uri></binding></result></results>
              </sparql>"#;
            for body in [srj, srx] {
                let mut n = 0usize;
                let vars = parse_results_into(body, &mut |_r| {
                    n += 1;
                    Ok(())
                })
                .unwrap();
                assert_eq!(vars.len(), 1);
                assert_eq!(n, 1);
            }
            let mut n = 0usize;
            assert!(parse_results_into("plain text", &mut |_r| {
                n += 1;
                Ok(())
            })
            .is_err());
            assert_eq!(n, 0);
        }

        // ------------------------------------------------------------------
        // Direct coverage for the reader-seam path (sq-my8wd.5) [OPUS-4.8]
        // ------------------------------------------------------------------

        /// `eval_remote_into_read` round-trips through the `TransportAsReader`
        /// adapter (the test-seam path) and produces the same relation as the
        /// non-reader path — direct coverage for the production entry point.
        #[test]
        fn eval_remote_into_read_via_transport_as_reader() {
            let body = r#"{"head":{"vars":["x","y"]},"results":{"bindings":[
                {"x":{"type":"uri","value":"http://ex/1"},"y":{"type":"literal","value":"a"}},
                {"x":{"type":"uri","value":"http://ex/2"}}]}}"#;
            let transport = super::Canned(body);
            let tr = super::TransportAsReader(&transport);
            let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
            let vars = eval_remote_into_read(&tr, "http://unused/", "SELECT * WHERE {}",
                &mut |r| { rows.push(r); Ok(()) }).unwrap();
            assert_eq!(vars.len(), 2);
            assert_eq!(rows.len(), 2);
        }

        /// `parse_results_into_read` correctly sniffs and routes SRJ via a reader.
        #[test]
        fn parse_results_into_read_sniffs_srj() {
            let body = r#"{"head":{"vars":["z"]},"results":{"bindings":[
                {"z":{"type":"bnode","value":"b0"}}]}}"#;
            let mut n = 0usize;
            let vars = parse_results_into_read(std::io::BufReader::new(body.as_bytes()),
                &mut |_r| { n += 1; Ok(()) }).unwrap();
            assert_eq!(vars.len(), 1);
            assert_eq!(n, 1);
        }

        /// `parse_results_into_read` correctly sniffs and routes SRX via a reader.
        #[test]
        fn parse_results_into_read_sniffs_srx() {
            let body = r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
                <head><variable name="z"/></head>
                <results><result><binding name="z">
                    <uri>http://ex/srx</uri>
                </binding></result></results></sparql>"#;
            let mut n = 0usize;
            let vars = parse_results_into_read(std::io::BufReader::new(body.as_bytes()),
                &mut |_r| { n += 1; Ok(()) }).unwrap();
            assert_eq!(vars.len(), 1);
            assert_eq!(n, 1);
        }

        /// `parse_srj_into_read` produces the same result as `parse_srj_into` on a
        /// representative document — direct coverage for the streaming SRJ reader.
        #[test]
        fn parse_srj_into_read_agrees_with_into_on_representative_doc() {
            let body = r#"{"head":{"vars":["s","o"]},"results":{"bindings":[
                {"s":{"type":"uri","value":"http://ex/s1"},"o":{"type":"literal","value":"v1"}},
                {"s":{"type":"uri","value":"http://ex/s2"}}]}}"#;
            let mut from_str: Vec<Vec<Option<Term>>> = Vec::new();
            let v1 = parse_srj_into(body, &mut |r| { from_str.push(r); Ok(()) }).unwrap();
            let mut from_read: Vec<Vec<Option<Term>>> = Vec::new();
            let v2 = parse_srj_into_read(body.as_bytes(), &mut |r| { from_read.push(r); Ok(()) }).unwrap();
            assert_eq!(v1, v2);
            assert_eq!(from_str, from_read);
        }

        /// `parse_srx_into_read` produces the same result as `parse_srx_into` on a
        /// representative document — direct coverage for the streaming SRX reader.
        #[test]
        fn parse_srx_into_read_agrees_with_into_on_representative_doc() {
            let body = r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
                <head><variable name="s"/><variable name="o"/></head>
                <results>
                    <result>
                        <binding name="s"><uri>http://ex/s1</uri></binding>
                        <binding name="o"><literal>v1</literal></binding>
                    </result>
                    <result><binding name="s"><uri>http://ex/s2</uri></binding></result>
                </results></sparql>"#;
            let mut from_str: Vec<Vec<Option<Term>>> = Vec::new();
            let v1 = parse_srx_into(body, &mut |r| { from_str.push(r); Ok(()) }).unwrap();
            let mut from_read: Vec<Vec<Option<Term>>> = Vec::new();
            let v2 = parse_srx_into_read(std::io::BufReader::new(body.as_bytes()),
                &mut |r| { from_read.push(r); Ok(()) }).unwrap();
            assert_eq!(v1, v2);
            assert_eq!(from_str, from_read);
        }

        // ------------------------------------------------------------------
        // Reader-seam error paths (sq-my8wd.5) [OPUS-4.8]
        // ------------------------------------------------------------------

        /// `parse_results_into_read` on an EMPTY body returns an error matching
        /// the non-reader path, not a panic or silent empty result.
        #[test]
        fn parse_results_into_read_empty_body_is_error() {
            // [OPUS-4.8] sq-my8wd.5 coverage: the `buf.is_empty()` early-return
            // branch in `parse_results_into_read` (the empty-response path).
            let err = parse_results_into_read(std::io::BufReader::new(b"".as_ref()),
                &mut |_r| Ok(())).unwrap_err();
            assert!(
                err.contains("neither SPARQL-Results-JSON nor -XML"),
                "empty body must report the sniff error: {err}"
            );
        }

        /// `parse_results_into_read` on a body that is neither `{` nor `<`
        /// returns the sniff error — mirrors `parse_results_into`'s rejection.
        #[test]
        fn parse_results_into_read_unknown_format_is_error() {
            // [OPUS-4.8] sq-my8wd.5 coverage: the `_ => Err(…)` sniff branch in
            // `parse_results_into_read` (the unrecognised-format path).
            let err = parse_results_into_read(std::io::BufReader::new(b"plain text".as_ref()),
                &mut |_r| Ok(())).unwrap_err();
            assert!(
                err.contains("neither SPARQL-Results-JSON nor -XML"),
                "unrecognised body must report the sniff error: {err}"
            );
            // Whitespace-only body hits the empty-buffer path after draining whitespace.
            let err2 = parse_results_into_read(std::io::BufReader::new(b"   ".as_ref()),
                &mut |_r| Ok(())).unwrap_err();
            assert!(
                err2.contains("neither SPARQL-Results-JSON nor -XML"),
                "whitespace-only body must report the sniff error: {err2}"
            );
        }

        /// `parse_results_into_read` with a body prefixed by leading whitespace still
        /// routes correctly to the SRJ parser — exercises the whitespace-consume loop.
        #[test]
        fn parse_results_into_read_leading_whitespace_routes_srj() {
            // [OPUS-4.8] sq-my8wd.5 coverage: the `None` (all-whitespace-chunk)
            // branch in `parse_results_into_read`, then routing to SRJ.
            let body = b"   \n\t {\"head\":{\"vars\":[\"x\"]},\"results\":{\"bindings\":[]}}";
            let mut n = 0usize;
            let vars = parse_results_into_read(std::io::BufReader::new(body.as_ref()),
                &mut |_r| { n += 1; Ok(()) }).unwrap();
            assert_eq!(vars.len(), 1);
            assert_eq!(n, 0); // no rows
        }
    }

    // -------------------------------------------------------------------------
    // Coverage-raise tests (sq-qcnn.34) [SONNET-4.6]
    //
    // Named regions: SPARQL-Results JSON/XML parse paths, bound-join VALUES
    // rendering, HttpTransport timeout math, SSRF egress-policy scoping.
    // Each test asserts EXACT values (not just "doesn't panic") so a branch-flip
    // or comparison mutant goes red (#1250 direct-unit-test discipline).
    // -------------------------------------------------------------------------

    // --- render_values_block -------------------------------------------------

    /// Single-variable block, empty tuples list — `VALUES ?x { }`.
    #[test]
    fn render_values_block_empty_tuples() {
        let vars = vec![Variable::new("x").unwrap()];
        let got = render_values_block(&vars, &[]);
        assert_eq!(got, "VALUES ?x { }");
    }

    /// Single-variable block, one IRI tuple.
    #[test]
    fn render_values_block_single_var_one_tuple() {
        let vars = vec![Variable::new("x").unwrap()];
        let tuples = vec![vec![Term::NamedNode(NamedNode::new("http://ex/a").unwrap())]];
        let got = render_values_block(&vars, &tuples);
        assert_eq!(got, "VALUES ?x { <http://ex/a> }");
    }

    /// Single-variable block, multiple tuples — terms are space-separated.
    #[test]
    fn render_values_block_single_var_multi_tuple() {
        let vars = vec![Variable::new("x").unwrap()];
        let tuples = vec![
            vec![Term::NamedNode(NamedNode::new("http://ex/a").unwrap())],
            vec![Term::Literal(Literal::new_simple_literal("hello"))],
        ];
        let got = render_values_block(&vars, &tuples);
        assert_eq!(got, "VALUES ?x { <http://ex/a> \"hello\" }");
    }

    /// Multi-variable block: header uses `VALUES (?a ?b)` form; values are
    /// parenthesised row-by-row with a space separator between cells.
    #[test]
    fn render_values_block_multi_var_single_tuple() {
        let vars = vec![Variable::new("a").unwrap(), Variable::new("b").unwrap()];
        let iri = Term::NamedNode(NamedNode::new("http://ex/i").unwrap());
        let lit = Term::Literal(Literal::new_typed_literal(
            "42",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        ));
        let got = render_values_block(&vars, &[vec![iri, lit]]);
        assert!(got.starts_with("VALUES (?a ?b) { ("), "header: {got}");
        assert!(got.contains("<http://ex/i>"), "IRI present: {got}");
        assert!(
            got.contains("\"42\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
            "typed literal present: {got}"
        );
        assert!(got.ends_with(") }"), "trailing ) }}: {got}");
    }

    /// Multi-variable block with two tuples.
    #[test]
    fn render_values_block_multi_var_multi_tuple() {
        let vars = vec![Variable::new("s").unwrap(), Variable::new("o").unwrap()];
        let r1 = vec![
            Term::NamedNode(NamedNode::new("http://ex/s1").unwrap()),
            Term::NamedNode(NamedNode::new("http://ex/o1").unwrap()),
        ];
        let r2 = vec![
            Term::NamedNode(NamedNode::new("http://ex/s2").unwrap()),
            Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap()),
        ];
        let got = render_values_block(&vars, &[r1, r2]);
        assert_eq!(got.matches("http://ex/s").count(), 2, "two subject IRIs: {got}");
        assert!(got.contains("\"hi\"@en"), "lang-tagged literal: {got}");
    }

    // --- pushable_term -------------------------------------------------------

    /// BlankNode is NOT pushable — blank-node labels are local to one document.
    #[test]
    fn pushable_term_bnode_returns_false() {
        let b = Term::BlankNode(BlankNode::new("b0").unwrap());
        assert!(!pushable_term(&b), "blank node must not be pushable");
    }

    /// Triple term (RDF 1.2 `<<( s p o )>>`) is NOT pushable.
    #[test]
    fn pushable_term_triple_returns_false() {
        let t = Term::Triple(Box::new(Triple {
            subject: NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            predicate: NamedNode::new("http://ex/p").unwrap(),
            object: Term::Literal(Literal::new_simple_literal("o")),
        }));
        assert!(!pushable_term(&t), "triple term must not be pushable");
    }

    // --- parse_results collecting wrapper with rows --------------------------

    /// `parse_results` is the collecting wrapper: the closure body (row accumulator)
    /// is only executed when the result set is non-empty.  This test exercises that
    /// closure so the line-coverage gate registers it.
    #[test]
    fn parse_results_collecting_wrapper_delivers_rows() {
        let body = r#"{"head":{"vars":["x"]},"results":{"bindings":[
            {"x":{"type":"uri","value":"http://ex/r1"}},
            {"x":{"type":"uri","value":"http://ex/r2"}}
        ]}}"#;
        let rel = parse_results(body).unwrap();
        assert_eq!(rel.vars.len(), 1);
        assert_eq!(rel.rows.len(), 2);
        assert_eq!(
            rel.rows[0][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/r1").unwrap()))
        );
        assert_eq!(
            rel.rows[1][0],
            Some(Term::NamedNode(NamedNode::new("http://ex/r2").unwrap()))
        );
    }

    // --- parse_srj_into_read error path --------------------------------------

    /// When `parse_srj_into_read` encounters malformed JSON it returns the JSON
    /// error — covers the `Err` branch in the reader-based loop (L674-676).
    #[test]
    fn parse_srj_into_read_error_propagates_from_reader() {
        let malformed = b"{ bad json !!! }";
        let err = parse_srj_into_read(std::io::Cursor::new(malformed), &mut |_r| Ok(()))
            .unwrap_err();
        assert!(
            err.contains("invalid results JSON"),
            "expected JSON error, got: {err}"
        );
    }

    // --- SRJ term-level error paths ------------------------------------------

    /// A SRJ binding with `type:"literal"` but NO `"value"` field must be
    /// rejected — fail-closed parse discipline.
    #[test]
    fn srj_term_literal_missing_value_is_error() {
        let body = r#"{
            "head":{"vars":["x"]},
            "results":{"bindings":[{"x":{"type":"literal"}}]}
        }"#;
        let err = parse_srj(body).unwrap_err();
        assert!(
            err.contains("literal binding without value"),
            "expected literal-without-value error, got: {err}"
        );
    }

    /// A SRJ binding with `type:"triple"` whose `subject` is a literal must be
    /// rejected — the invalid-subject guard.
    #[test]
    fn srj_term_triple_invalid_subject_is_error() {
        let body = r#"{
            "head":{"vars":["t"]},
            "results":{"bindings":[{"t":{
                "type":"triple",
                "value":{
                    "subject":{"type":"literal","value":"bad"},
                    "predicate":{"type":"uri","value":"http://ex/p"},
                    "object":{"type":"literal","value":"o"}
                }
            }}]}
        }"#;
        let err = parse_srj(body).unwrap_err();
        assert!(
            err.contains("invalid triple-term subject"),
            "expected invalid-subject error, got: {err}"
        );
    }

    /// A SRJ binding with `type:"triple"` whose `predicate` is a literal must
    /// be rejected — the invalid-predicate guard.
    #[test]
    fn srj_term_triple_invalid_predicate_is_error() {
        let body = r#"{
            "head":{"vars":["t"]},
            "results":{"bindings":[{"t":{
                "type":"triple",
                "value":{
                    "subject":{"type":"uri","value":"http://ex/s"},
                    "predicate":{"type":"literal","value":"bad"},
                    "object":{"type":"literal","value":"o"}
                }
            }}]}
        }"#;
        let err = parse_srj(body).unwrap_err();
        assert!(
            err.contains("invalid triple-term predicate"),
            "expected invalid-predicate error, got: {err}"
        );
    }

    // --- resolve_xml_entity: named and numeric character references ----------

    /// The five predefined XML entities resolve to their exact characters.
    #[test]
    fn resolve_xml_entity_predefined_named_entities() {
        // [GPT-5.6] (sq-5kh4d) Directly cover the named-entity branch rather than
        // relying only on the SRX parser integration test.
        for (name, expected) in [
            ("amp", "&"),
            ("lt", "<"),
            ("gt", ">"),
            ("quot", "\""),
            ("apos", "'"),
        ] {
            assert_eq!(super::resolve_xml_entity(name).unwrap(), expected);
        }
    }

    /// Decimal numeric character reference `#38` resolves to `&` (U+0026).
    #[test]
    fn resolve_xml_entity_decimal_numeric_ref() {
        let result = super::resolve_xml_entity("#38").unwrap();
        assert_eq!(result, "&");
    }

    /// Hex numeric character reference `#x3C` resolves to `<` (U+003C).
    #[test]
    fn resolve_xml_entity_hex_numeric_ref() {
        let result = super::resolve_xml_entity("#x3C").unwrap();
        assert_eq!(result, "<");
    }

    /// Upper-case `#X` prefix is also accepted as hex.
    #[test]
    fn resolve_xml_entity_hex_upper_x() {
        let result = super::resolve_xml_entity("#X26").unwrap();
        assert_eq!(result, "&");
    }

    /// An out-of-range code point (> 0x10FFFF) is rejected with an error.
    #[test]
    fn resolve_xml_entity_out_of_range_is_error() {
        let err = super::resolve_xml_entity("#x200000").unwrap_err();
        assert!(err.contains("out of range"), "got: {err}");
    }

    /// A completely unknown named entity is rejected.
    #[test]
    fn resolve_xml_entity_unknown_named_entity_is_error() {
        let err = super::resolve_xml_entity("unknownentity").unwrap_err();
        assert!(err.contains("unknown XML entity"), "got: {err}");
    }

    // --- SRX parser: triple terms, self-closing elements, error paths --------

    /// SRX triple-term parsing (SPARQL 1.2 `<triple>…</triple>`) in the
    /// `parse_srx_into` path — exercises the triple_stack, set_slot, commit
    /// helpers, and the triple-subject/predicate/object arms.
    #[test]
    fn parse_srx_triple_term_complete_roundtrip() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="t"/></head>
              <results><result>
                <binding name="t"><triple>
                  <subject><uri>http://ex/s</uri></subject>
                  <predicate><uri>http://ex/p</uri></predicate>
                  <object><literal>o</literal></object>
                </triple></binding>
              </result></results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        let want = Term::Triple(Box::new(Triple {
            subject: NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            predicate: NamedNode::new("http://ex/p").unwrap(),
            object: Term::Literal(Literal::new_simple_literal("o")),
        }));
        assert_eq!(rel.rows[0][0], Some(want));
    }

    /// An unbalanced `</triple>` tag produces an `invalid results XML` error.
    /// (quick-xml's well-formedness check fires before the stray-triple guard,
    /// so the error is the ill-formed-document variant of the XML error wrapper.)
    #[test]
    fn parse_srx_unbalanced_triple_tag_is_xml_error() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="x"/></head>
              <results></triple></results>
            </sparql>"#
        );
        let err = parse_srx(&body).unwrap_err();
        assert!(
            err.contains("invalid results XML"),
            "expected XML parse error, got: {err}"
        );
    }

    /// An invalid variable name in `<variable name="…"/>` must be rejected.
    #[test]
    fn parse_srx_bad_variable_name_is_error() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="bad name with spaces"/></head>
              <results></results>
            </sparql>"#
        );
        let err = parse_srx(&body).unwrap_err();
        assert!(
            err.contains("bad result variable"),
            "expected bad-variable error, got: {err}"
        );
    }

    /// A self-closing value element `<literal/>` commits an empty simple literal
    /// — the `is_empty` branch in the event loop.  (`<bnode/>` is not used here
    /// because an empty bnode label is invalid in oxrdf.)
    #[test]
    fn parse_srx_self_closing_literal_is_empty_string() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="v"/></head>
              <results><result>
                <binding name="v"><literal/></binding>
              </result></results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert_eq!(
            rel.rows[0][0],
            Some(Term::Literal(Literal::new_simple_literal(""))),
            "self-closing literal must yield empty simple literal"
        );
    }

    /// CData sections (`<![CDATA[…]]>`) inside a value element are decoded as
    /// literal text by the `Event::CData` handler.
    #[test]
    fn parse_srx_cdata_section_is_decoded_as_text() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="x"/></head>
              <results><result>
                <binding name="x"><literal><![CDATA[a & b < c]]></literal></binding>
              </result></results>
            </sparql>"#
        );
        let rel = parse_srx(&body).unwrap();
        assert_eq!(
            rel.rows[0][0],
            Some(Term::Literal(Literal::new_simple_literal("a & b < c")))
        );
    }

    // --- parse_srx_into_read: extended coverage for the reader variant -------

    /// Triple terms round-trip through `parse_srx_into_read` — covers the
    /// reader-variant commit_r / set_slot_r helpers and the triple build path.
    #[test]
    fn parse_srx_into_read_triple_term_roundtrip() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="t"/></head>
              <results><result>
                <binding name="t"><triple>
                  <subject><uri>http://ex/s</uri></subject>
                  <predicate><uri>http://ex/p</uri></predicate>
                  <object><literal>o</literal></object>
                </triple></binding>
              </result></results>
            </sparql>"#
        );
        let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
        let vars = parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!(vars.len(), 1);
        let want = Term::Triple(Box::new(Triple {
            subject: NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            predicate: NamedNode::new("http://ex/p").unwrap(),
            object: Term::Literal(Literal::new_simple_literal("o")),
        }));
        assert_eq!(rows[0][0], Some(want));
    }

    /// An unbalanced `</triple>` tag in the reader variant produces an
    /// `invalid results XML` error (quick-xml's well-formedness check fires first).
    #[test]
    fn parse_srx_into_read_unbalanced_triple_tag_is_xml_error() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="x"/></head>
              <results></triple></results>
            </sparql>"#
        );
        let err =
            parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |_r| Ok(()))
                .unwrap_err();
        assert!(
            err.contains("invalid results XML"),
            "expected XML parse error, got: {err}"
        );
    }

    /// An ASK `<boolean>true</boolean>` body is rejected in the reader variant —
    /// SERVICE always wraps a SELECT.
    #[test]
    fn parse_srx_into_read_boolean_body_is_error() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}"><head/><boolean>true</boolean></sparql>"#
        );
        let err =
            parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |_r| Ok(()))
                .unwrap_err();
        assert!(
            err.contains("ASK boolean"),
            "expected ASK-boolean rejection, got: {err}"
        );
    }

    /// CData sections in the reader variant are decoded to literal text.
    #[test]
    fn parse_srx_into_read_cdata_section() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="x"/></head>
              <results><result>
                <binding name="x"><literal><![CDATA[a & b]]></literal></binding>
              </result></results>
            </sparql>"#
        );
        let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
        parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            rows[0][0],
            Some(Term::Literal(Literal::new_simple_literal("a & b")))
        );
    }

    /// Named entity references (`&amp;` → `&`) in the reader variant are resolved
    /// by the `Event::GeneralRef` handler.
    #[test]
    fn parse_srx_into_read_general_ref_entity() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="x"/></head>
              <results><result>
                <binding name="x"><literal>a &amp; b</literal></binding>
              </result></results>
            </sparql>"#
        );
        let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
        parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            rows[0][0],
            Some(Term::Literal(Literal::new_simple_literal("a & b")))
        );
    }

    /// A self-closing `<literal/>` in the reader variant commits an empty simple
    /// literal — mirrors the from-str `is_empty` path.
    #[test]
    fn parse_srx_into_read_self_closing_element() {
        let body = format!(
            r#"<sparql xmlns="{SRX_NS}">
              <head><variable name="v"/></head>
              <results><result>
                <binding name="v"><literal/></binding>
              </result></results>
            </sparql>"#
        );
        let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
        parse_srx_into_read(std::io::BufReader::new(body.as_bytes()), &mut |r| {
            rows.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            rows[0][0],
            Some(Term::Literal(Literal::new_simple_literal(""))),
            "self-closing literal must yield empty simple literal"
        );
    }

    // --- egress policy: allowlist_entry_host_matches and split_entry edge case

    /// `allowlist_entry_host_matches` — the port-ignoring host-pattern match.
    #[test]
    fn allowlist_entry_host_matches_various_patterns() {
        // Exact host-level match.
        assert!(
            allowlist_entry_host_matches("sparql.example.org", "sparql.example.org"),
            "exact host-level match"
        );
        // Suffix wildcard: `.example.org` matches `sparql.example.org`.
        assert!(
            allowlist_entry_host_matches(".example.org", "sparql.example.org"),
            "suffix wildcard host match"
        );
        // Non-matching host.
        assert!(
            !allowlist_entry_host_matches("sparql.example.org", "other.example.org"),
            "non-matching host"
        );
        // Port-scoped entry: host part matches, port constraint is ignored.
        assert!(
            allowlist_entry_host_matches("127.0.0.1:8053", "127.0.0.1"),
            "port-scoped entry, host part matches (port ignored)"
        );
        assert!(
            !allowlist_entry_host_matches("127.0.0.1:8053", "127.0.0.2"),
            "port-scoped entry, host part does not match"
        );
    }

    /// `split_entry` on a malformed bracketed entry (opening `[` but no closing
    /// `]`) falls back to treating the whole string as the host pattern.
    #[test]
    fn split_entry_malformed_bracket_no_close_is_host_level() {
        // `[::1` has an opening bracket but no `]` — returned as-is.
        let (host, port) = egress_policy::split_entry("[::1");
        assert_eq!(host, "[::1");
        assert_eq!(port, None, "malformed bracket must have no port constraint");
    }

    // --- uri_host_port (native-only) -----------------------------------------

    #[cfg(not(target_arch = "wasm32"))]
    mod uri_host_port_tests {
        /// A URI with an empty host (authority `:port` with no host part) yields
        /// `None` from the `host.is_empty()` guard in `uri_host_port`.
        #[test]
        fn empty_host_yields_none() {
            // `http://:8080/path` — authority has empty host, explicit port 8080.
            if let Ok(uri) = "http://:8080/path".parse::<ureq::http::Uri>() {
                // Only assert if the http crate accepts this form.
                assert!(
                    super::super::uri_host_port(&uri).is_none(),
                    "empty-host URI must return None"
                );
            }
            // If the URI doesn't parse, the test passes vacuously — the branch
            // is not reachable from any well-formed URI the ureq client would produce.
        }

        /// An `http://` URI with no explicit port defaults to 80.
        #[test]
        fn http_no_port_defaults_to_80() {
            let uri: ureq::http::Uri = "http://example.com/path".parse().unwrap();
            let (netloc, host, port) = super::super::uri_host_port(&uri).unwrap();
            assert_eq!(host, "example.com");
            assert_eq!(port, 80);
            assert_eq!(netloc, "example.com:80");
        }

        /// An `https://` URI with no explicit port defaults to 443.
        #[test]
        fn https_no_port_defaults_to_443() {
            let uri: ureq::http::Uri = "https://example.com/path".parse().unwrap();
            let (_netloc, _host, port) = super::super::uri_host_port(&uri).unwrap();
            assert_eq!(port, 443);
        }

        /// An `http://` URI with an explicit port uses that port.
        #[test]
        fn explicit_port_is_used() {
            let uri: ureq::http::Uri = "http://example.com:9090/sparql".parse().unwrap();
            let (_netloc, host, port) = super::super::uri_host_port(&uri).unwrap();
            assert_eq!(host, "example.com");
            assert_eq!(port, 9090);
        }
    }

    // --- HttpTransport (native-only, minimal loopback HTTP server) -----------
    //
    // These tests spin up a minimal loopback HTTP/1.1 server on an ephemeral
    // port, allowlist that host:port pair, and call the production transport
    // methods — covering the ureq-3 code paths that are unreachable from the
    // canned-transport tests (`fetch`, `fetch_reader`, `Ok` arms).
    // The server is single-shot (one connection), no persistent resources.
    // [SONNET-4.6] sq-qcnn.34

    #[cfg(not(target_arch = "wasm32"))]
    mod http_transport_tests {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        /// Bind a loopback server that accepts one HTTP request and sends back
        /// `body` in a minimal HTTP/1.1 200 response.  Returns the ephemeral
        /// port; the server runs on a detached background thread.
        fn serve_once(body: String) -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                // Drain the HTTP request headers (stop at blank line).
                let mut line = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).ok();
                    if line.trim().is_empty() {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/sparql-results+json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                reader.get_mut().write_all(response.as_bytes()).ok();
            });
            port
        }

        /// `HttpTransport::fetch` returns the SPARQL-Results body from a real
        /// (loopback) HTTP endpoint — covers the `Ok` arm of the Transport impl.
        #[test]
        fn http_transport_fetch_success() {
            let srj = r#"{"head":{"vars":["x"]},"results":{"bindings":[
                {"x":{"type":"uri","value":"http://ex/ht1"}}
            ]}}"#
            .to_string();
            let port = serve_once(srj);
            let endpoint = format!("http://127.0.0.1:{port}/sparql");
            let result = with_service_egress_allow(
                [format!("127.0.0.1:{port}")],
                || HttpTransport::with_budget(None).fetch(&endpoint, "SELECT * WHERE {}"),
            );
            assert!(result.is_ok(), "fetch must succeed: {:?}", result);
            assert!(
                result.unwrap().contains("http://ex/ht1"),
                "body must contain expected IRI"
            );
        }

        /// `HttpTransport::fetch_reader` streams the response body without
        /// buffering — covers the `ReaderTransport` impl for `HttpTransport`.
        #[test]
        fn http_transport_fetch_reader_success() {
            let srj = r#"{"head":{"vars":["y"]},"results":{"bindings":[
                {"y":{"type":"literal","value":"streamed"}}
            ]}}"#
            .to_string();
            let port = serve_once(srj);
            let endpoint = format!("http://127.0.0.1:{port}/sparql");
            // `fetch_reader` returns a reader whose lifetime is tied to `transport`,
            // so we must read the body while the transport is still alive — inside
            // the closure, before any move.
            let body = with_service_egress_allow(
                [format!("127.0.0.1:{port}")],
                || -> Result<String, String> {
                    let transport = HttpTransport::with_budget(None);
                    let mut reader =
                        transport.fetch_reader(&endpoint, "SELECT * WHERE {}")?;
                    let mut body = String::new();
                    std::io::Read::read_to_string(&mut reader, &mut body)
                        .map_err(|e| format!("read error: {e}"))?;
                    Ok(body)
                },
            )
            .unwrap_or_else(|e| panic!("fetch_reader must succeed: {e}"));
            assert!(body.contains("streamed"), "body must contain the literal: {body}");
        }
    }
}
