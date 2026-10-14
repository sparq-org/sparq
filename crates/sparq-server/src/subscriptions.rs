//! SEPA-style SPARQL subscriptions over WebSocket (T23).
//!
//! Spec lineage: the **SEPA** W3C member submission ("SPARQL 1.1 Subscribe Language",
//! <https://www.w3.org/submissions/2018/SUBM-sparql11-subscribe-20181016/>) — a client
//! registers a SELECT query and is notified with **added/removed bindings diffs**
//! whenever a committed update changes the query's result. We keep SEPA's notification
//! *shape* (both diffs are SPARQL JSON results objects) but speak plain JSON over a
//! plain WebSocket — no `sparql-se` URI scheme, no separate subscribe HTTP endpoint.
//! See `crates/sparq-server/SUBSCRIPTIONS.md` for the full protocol and divergences.
//!
//! # Architecture: re-evaluate + diff per committed update
//!
//! * **Commit hook** — [`crate::http::AppState::apply_update`] advances a
//!   `tokio::sync::watch` channel to the published generation number *after* the
//!   sequenced writer's group-commit ack. Every `/subscriptions` connection task holds
//!   a `watch::Receiver`.
//! * **Re-evaluation** — when the generation changes, the connection task re-runs each of
//!   its active SELECTs against a freshly pinned generation
//!   ([`crate::http::AppState::current`]) on the blocking pool (`spawn_blocking`), under
//!   the server's [`sparq_engine::QueryBudget`] (query timeout + `--max-results` row
//!   cap), then diffs against the stored previous result.
//! * **Diff** — each solution row is canonicalised to its SPARQL-JSON binding object
//!   (variables sorted by `serde_json`'s map ordering) and the serialised string is the
//!   row's identity key in a `HashMap<key, binding>`. Added = keys in the new result only;
//!   removed = keys in the old result only. Set semantics: duplicate rows collapse to one
//!   key (documented divergence — SELECT is a bag, the diff is over distinct bindings).
//! * **Coalescing (dirty-flag pattern)** — the `watch` channel stores only the latest
//!   generation, and `Receiver::changed()` resolves once when the seen value is stale, no
//!   matter how many commits produced it. If commits land *while* a re-evaluation is
//!   running, the generation advances again, so the loop's next `changed()` fires
//!   immediately and one more re-evaluation — against the latest snapshot — covers the
//!   whole burst. Notifications are therefore per *re-evaluation*, not per commit, and a
//!   client may observe several commits as a single combined diff. Because the generation
//!   is bumped after the swap, a snapshot taken after `changed()` resolves is always at
//!   least as new as the commit that woke us; an "early" snapshot only leads to a later
//!   empty diff, which is suppressed.
//! * **Limits** — per-connection cap (`--max-subscriptions-per-conn`, default 16) checked
//!   against the connection's own table; global cap (`--max-subscriptions`, default 256)
//!   via an atomic acquire/release counter with a panic-safe release on connection drop.
//!   An initial evaluation that overflows `--max-results` (or times out) *refuses* the
//!   subscription, mirroring the HTTP 413/503 semantics; a later re-evaluation failure
//!   terminates the subscription with an `error` message.
//!
//! Known limitation: the engine relabels blank nodes between evaluations, so a result row
//! containing a blank node can show up as a remove+add pair across commits even when it is
//! semantically unchanged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use sparq_core::Graph;
use sparq_engine::QueryBudget;

use crate::exec::{prepare, PrepareError, QueryForm};
use crate::http::{make_budget, AppState, ServerConfig, TIMEOUT_GRACE};

// ---------------------------------------------------------------------------
// Global bookkeeping
// ---------------------------------------------------------------------------

/// Server-global subscription bookkeeping, shared by every `/subscriptions` connection
/// through [`AppState`].
#[derive(Default)]
pub(crate) struct SubscriptionCounters {
    /// Active subscriptions across the whole server (bounded by `--max-subscriptions`).
    active: AtomicUsize,
    /// Monotonic subscription-id allocator (ids are unique across the server's lifetime).
    next_id: AtomicU64,
}

impl SubscriptionCounters {
    /// Currently active subscriptions server-wide (the `/metrics` gauge, T22).
    pub(crate) fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

/// The slots a single connection holds against the global cap. Slots are acquired on
/// `subscribe` and released on `unsubscribe`/termination; `Drop` releases whatever is
/// still held, so a socket that disconnects (or a handler that panics) can never leak
/// global capacity.
///
/// sq-bxog: `pub(crate)` so the SSE transport ([`crate::subscriptions::sse`])
/// reuses the SAME global-cap accounting as the WebSocket path — one stream holds one slot,
/// released on disconnect by `Drop` exactly like a WS connection's slots.
pub(crate) struct ConnSlots {
    counters: Arc<SubscriptionCounters>,
    held: usize,
}

impl ConnSlots {
    pub(crate) fn new(counters: Arc<SubscriptionCounters>) -> Self {
        Self { counters, held: 0 }
    }

    /// Tries to take one global slot (CAS loop so concurrent connections never overshoot).
    pub(crate) fn try_acquire(&mut self, max_global: usize) -> bool {
        let ok = self
            .counters
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < max_global).then(|| n + 1))
            .is_ok();
        if ok {
            self.held += 1;
        }
        ok
    }

    pub(crate) fn release_one(&mut self) {
        debug_assert!(self.held > 0);
        self.held -= 1;
        self.counters.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for ConnSlots {
    fn drop(&mut self) {
        if self.held > 0 {
            self.counters.active.fetch_sub(self.held, Ordering::AcqRel);
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket endpoint
// ---------------------------------------------------------------------------

/// `GET /subscriptions` — upgrades to the subscription WebSocket. Incoming text frames
/// are capped at the server's `--max-body-bytes` (same guard as HTTP request bodies).
///
/// sq-cxk5: this is a READ surface (live SELECT diffs), so when `--auth-token-read`
/// is configured the UPGRADE is gated behind the read token EXACTLY like a `/sparql` GET —
/// rejected with the same 401 BEFORE the socket is upgraded. Because a browser cannot set an
/// `Authorization` header on a WS handshake, the token is accepted from EITHER the
/// `Authorization: Bearer <token>` header (non-browser clients) OR a
/// `Sec-WebSocket-Protocol: bearer.<token>` subprotocol (browsers) — see
/// `crate::http::ws_auth_gate`. When a `bearer.<token>` subprotocol is present and accepted, it
/// is echoed back as the selected subprotocol (RFC 6455 requires the server to confirm one of the
/// client's offered subprotocols, or a browser rejects the handshake). When no read token is
/// configured the upgrade is unchanged (open access) — back-compatible.
pub async fn subscriptions_endpoint(State(state): State<AppState>, headers: HeaderMap, mut ws: WebSocketUpgrade) -> Response {
    // Gate the upgrade behind the read token (fail-closed when required + absent/wrong).
    if let Some(resp) = crate::http::ws_auth_gate(state.config(), &headers, crate::http::Operation::Read) {
        return resp;
    }
    // RFC 6455: if the client offered a `bearer.<token>` subprotocol (the browser auth channel),
    // the server MUST confirm exactly one offered subprotocol or the browser rejects the
    // handshake. We confirm the matched `bearer.<token>` value (its token was already validated
    // above — confirming it is NOT a second auth check). Non-`bearer.` subprotocols are not part
    // of this protocol, so none is selected for them.
    if let Some((proto, _tok)) = crate::http::subprotocol_bearer_token(&headers) {
        if let Ok(value) = axum::http::HeaderValue::from_str(proto) {
            ws.set_selected_protocol(value);
        }
    }
    let max_msg = state.config().max_body_bytes;
    ws.max_message_size(max_msg).on_upgrade(move |socket| handle_socket(socket, state))
}

/// One active subscription: the query, its identity, and the last seen result keyed by
/// canonical row encoding (the diff baseline).
///
/// sq-bxog: `pub(crate)` (with [`Subscription::reevaluate_step`]) so the SSE
/// transport drives the SAME re-evaluate + diff state machine as the WebSocket path — only
/// the wire framing differs.
pub(crate) struct Subscription {
    alias: Option<String>,
    query: String,
    /// Per-subscription notification sequence; 0 is the initial full result.
    sequence: u64,
    vars: Vec<String>,
    /// canonical row key (serialised binding object) → binding object.
    rows: HashMap<String, Value>,
}

/// sq-bxog: the outcome of one [`Subscription::reevaluate_step`] — the
/// transport-agnostic core that both `/subscriptions` (WS) and `/subscriptions/sse` reuse.
pub(crate) enum ReevalStep {
    /// The result is unchanged (or only coalesced no-ops): emit nothing.
    Unchanged,
    /// The result changed: emit this `notification` JSON (added/removed diff).
    Notify(Value),
    /// Re-evaluation failed (timeout / max-rows overflow): the caller must drop the
    /// subscription and emit this terminating `error` JSON.
    Terminate(Value),
}

impl Subscription {
    /// Re-evaluates this subscription against the latest snapshot and advances its diff
    /// baseline, returning the JSON to emit (or [`ReevalStep::Unchanged`]). This is the
    /// single source of truth for the re-evaluate + diff semantics shared by both
    /// transports — see the module docs. On a [`ReevalStep::Terminate`] the baseline is NOT
    /// advanced (the caller drops the subscription).
    pub(crate) async fn reevaluate_step(&mut self, id: u64, state: &AppState) -> ReevalStep {
        match evaluate(state, self.query.clone()).await {
            Ok((vars, new_rows)) => {
                let added: Vec<&Value> =
                    sorted_pairs(new_rows.iter().filter(|(k, _)| !self.rows.contains_key(*k)));
                let removed: Vec<&Value> =
                    sorted_pairs(self.rows.iter().filter(|(k, _)| !new_rows.contains_key(*k)));
                if added.is_empty() && removed.is_empty() {
                    return ReevalStep::Unchanged; // result unchanged — coalesced no-ops included
                }
                self.sequence += 1;
                let msg = notification(id, self.alias.as_deref(), self.sequence, &vars, &added, &removed);
                self.vars = vars;
                self.rows = new_rows;
                ReevalStep::Notify(msg)
            }
            Err(e) => ReevalStep::Terminate(error_msg(
                &format!("re-evaluation failed, subscription terminated: {}", e.message),
                Some(id),
                self.alias.as_deref(),
            )),
        }
    }
}

/// The per-connection driver: a single task owns the socket and its subscription table,
/// `select!`-ing between client messages and the commit-generation watch channel. While a
/// re-evaluation batch runs, incoming frames simply queue on the socket — `unsubscribe`
/// is processed right after the batch.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut commits = state.subscribe_commits();
    // Drop any generations committed before this connection existed: the initial
    // notification (sent on subscribe) already reflects the current graph.
    commits.mark_unchanged();
    let mut subs: HashMap<u64, Subscription> = HashMap::new();
    let mut slots = ConnSlots::new(state.subs.clone());

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // axum 0.8: ws Message::Text now wraps Utf8Bytes, not String.
                        if !handle_client_message(&mut socket, &state, &mut subs, &mut slots, text.as_str()).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        if send(&mut socket, &error_msg("binary frames are not part of the protocol; send JSON text frames", None, None)).await.is_err() {
                            break;
                        }
                    }
                    // axum answers Ping automatically; Pong needs no action.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                }
            }
            changed = commits.changed() => {
                if changed.is_err() {
                    break; // server shutting down
                }
                if !reevaluate_all(&mut socket, &state, &mut subs, &mut slots).await {
                    break;
                }
            }
        }
    }
    // `slots` drops here, releasing the connection's global subscription slots.
}

/// Handles one client text frame. Returns `false` when the socket is gone.
async fn handle_client_message(
    socket: &mut WebSocket,
    state: &AppState,
    subs: &mut HashMap<u64, Subscription>,
    slots: &mut ConnSlots,
    text: &str,
) -> bool {
    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return send(socket, &error_msg(&format!("message is not valid JSON: {e}"), None, None))
                .await
                .is_ok();
        }
    };
    if let Some(req) = parsed.get("subscribe") {
        return handle_subscribe(socket, state, subs, slots, req).await;
    }
    if let Some(req) = parsed.get("unsubscribe") {
        return handle_unsubscribe(socket, subs, slots, req).await;
    }
    send(
        socket,
        &error_msg("unknown message; expected {\"subscribe\": {...}} or {\"unsubscribe\": {...}}", None, None),
    )
    .await
    .is_ok()
}

/// sq-bxog: a successfully-registered subscription, ready to stream. The
/// transport-agnostic product of [`subscribe_init`]: the live [`Subscription`] (its diff
/// baseline holds the initial full result), its server-wide `id`, and the two opening
/// frames a client expects — the `subscribed` ack and the sequence-0 notification (the full
/// result as `addedResults`). A global slot is held on `slots` for it.
pub(crate) struct NewSubscription {
    pub(crate) id: u64,
    pub(crate) subscription: Subscription,
    pub(crate) subscribed: Value,
    pub(crate) initial: Value,
}

/// sq-bxog (Copilot #120): a subscription-registration refusal, carrying BOTH the
/// `error` JSON to emit AND the HTTP status the SSE transport must surface — classified at the
/// point of refusal so the SSE path is consistent with the `/sparql` endpoint's status
/// semantics ([`crate::http::engine_error_response`]):
///
///   * malformed / non-SELECT query → **400** (the client's request is wrong);
///   * the initial evaluation overflowing `--max-results` / `--max-query-rows` (the row cap)
///     → **413** PAYLOAD_TOO_LARGE — EXACTLY as `/sparql` maps `query budget exceeded
///     (max-rows)`, instead of the previous blanket 503;
///   * the initial evaluation timing out, OR subscription-slot exhaustion (per-connection /
///     server-wide cap) → **503** SERVICE_UNAVAILABLE — genuine capacity / overload, mirroring
///     `/sparql`'s timeout 503;
///   * any other engine error (e.g. a denied SERVICE) → **500**, mirroring `/sparql`'s
///     `execution_error`.
///
/// The WebSocket transport ignores `status` (it has no HTTP response, only the `error` frame);
/// only the SSE GET, which must set a real status BEFORE the stream opens, consults it.
pub(crate) struct Refusal {
    pub(crate) error: Value,
    pub(crate) status: StatusCode,
}

/// sq-bxog: the transport-agnostic "register a subscription" core, shared by the
/// WebSocket `subscribe` frame and the SSE `GET /subscriptions/sse` handler. Validates the
/// query (SELECT-only), enforces the per-connection then the global cap (acquiring one slot
/// on `slots`), runs the initial evaluation (refusing on timeout / max-rows overflow,
/// mirroring the HTTP 503/413 semantics), allocates the server-wide id, and builds the two
/// opening frames. On refusal it returns the `error` JSON to emit and holds NO slot.
pub(crate) async fn subscribe_init(
    state: &AppState,
    slots: &mut ConnSlots,
    current_conn_subs: usize,
    query: &str,
    alias: Option<String>,
) -> Result<NewSubscription, Refusal> {
    // sq-bxog (Copilot #120): carry the HTTP status alongside the `error` JSON so the
    // SSE transport surfaces the SAME status `/sparql` would (400 client / 413 row-cap / 503
    // capacity-or-timeout / 500 other) — see [`Refusal`].
    let refuse = |msg: String, status: StatusCode| Refusal { error: error_msg(&msg, None, alias.as_deref()), status };

    // Only SELECT is subscribable: the diff is defined over solution bindings. A wrong-form or
    // malformed query is the client's error → 400, exactly like a `/sparql` parse error.
    match prepare(query) {
        Ok(p) if p.form == QueryForm::Select => {}
        Ok(_) => return Err(refuse("only SELECT queries can be subscribed".into(), StatusCode::BAD_REQUEST)),
        Err(PrepareError::Malformed(e)) => return Err(refuse(format!("malformed query: {e}"), StatusCode::BAD_REQUEST)),
        // sq-z33x: subscriptions carry no SPARQL-Protocol dataset override, so this
        // arm is unreachable in practice — but a bad graph IRI is a client error regardless.
        Err(PrepareError::BadGraphUri(e)) => return Err(refuse(e, StatusCode::BAD_REQUEST)),
    }

    // Limits: per-connection first (cheap), then the global slot. Both are genuine
    // capacity/overload (subscription-slot exhaustion) → 503, NOT a payload refusal.
    let config = state.config();
    if current_conn_subs >= config.max_subscriptions_per_conn {
        let max = config.max_subscriptions_per_conn;
        return Err(refuse(format!("subscription limit reached for this connection ({max}); unsubscribe first or raise --max-subscriptions-per-conn"), StatusCode::SERVICE_UNAVAILABLE));
    }
    if !slots.try_acquire(config.max_subscriptions) {
        let max = config.max_subscriptions;
        return Err(refuse(format!("server-wide subscription limit reached ({max}); retry later or raise --max-subscriptions"), StatusCode::SERVICE_UNAVAILABLE));
    }

    // Initial evaluation. Failure refuses the subscription with the SAME status `/sparql` would
    // give the equivalent engine error: a row-cap overflow (`--max-results` / `--max-query-rows`)
    // is a 413 (mirroring `crate::http::engine_error_response`), a timeout is a 503, else a 500.
    let (vars, rows) = match evaluate(state, query.to_string()).await {
        Ok(r) => r,
        Err(e) => {
            slots.release_one();
            return Err(refuse(format!("initial evaluation failed: {}", e.message), e.status()));
        }
    };

    let id = state.subs.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let subscription = Subscription { alias, query: query.to_string(), sequence: 0, vars, rows };

    // {"subscribed": ...} then the sequence-0 notification: full result as added, empty removed.
    let subscribed = with_alias(json!({ "subscribed": { "id": id } }), "subscribed", subscription.alias.as_deref());
    let added: Vec<&Value> = sorted_bindings(&subscription.rows);
    let initial = notification(id, subscription.alias.as_deref(), 0, &subscription.vars, &added, &[]);
    Ok(NewSubscription { id, subscription, subscribed, initial })
}

async fn handle_subscribe(
    socket: &mut WebSocket,
    state: &AppState,
    subs: &mut HashMap<u64, Subscription>,
    slots: &mut ConnSlots,
    req: &Value,
) -> bool {
    let alias = req.get("alias").and_then(Value::as_str).map(str::to_string);

    let Some(query) = req.get("query").and_then(Value::as_str) else {
        return send(socket, &error_msg("subscribe requires a string 'query' field", None, alias.as_deref())).await.is_ok();
    };

    let new = match subscribe_init(state, slots, subs.len(), query, alias).await {
        Ok(n) => n,
        // sq-bxog (Copilot #120): the WS transport has no HTTP status — it just emits
        // the `error` frame (the refusal's `status` is for the SSE GET only).
        Err(refusal) => return send(socket, &refusal.error).await.is_ok(),
    };

    if send(socket, &new.subscribed).await.is_err() {
        return false;
    }
    if send(socket, &new.initial).await.is_err() {
        return false;
    }
    subs.insert(new.id, new.subscription);
    true
}

async fn handle_unsubscribe(
    socket: &mut WebSocket,
    subs: &mut HashMap<u64, Subscription>,
    slots: &mut ConnSlots,
    req: &Value,
) -> bool {
    let Some(id) = req.get("id").and_then(Value::as_u64) else {
        return send(socket, &error_msg("unsubscribe requires a numeric 'id' field", None, None)).await.is_ok();
    };
    if subs.remove(&id).is_some() {
        slots.release_one();
        send(socket, &json!({ "unsubscribed": { "id": id } })).await.is_ok()
    } else {
        send(socket, &error_msg("no active subscription with that id on this connection", Some(id), None))
            .await
            .is_ok()
    }
}

/// Re-evaluates every subscription on this connection against the latest snapshot and
/// pushes non-empty diffs. A failed re-evaluation (timeout / max-rows overflow after the
/// data grew) terminates that subscription with an `error` message. Returns `false` when
/// the socket is gone.
async fn reevaluate_all(
    socket: &mut WebSocket,
    state: &AppState,
    subs: &mut HashMap<u64, Subscription>,
    slots: &mut ConnSlots,
) -> bool {
    let mut ids: Vec<u64> = subs.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        // sq-bxog: the re-evaluate + diff core is `Subscription::reevaluate_step`,
        // shared verbatim with the SSE transport; the WS path only handles the framing.
        let sub = subs.get_mut(&id).expect("subscription vanished mid-batch");
        match sub.reevaluate_step(id, state).await {
            ReevalStep::Unchanged => {}
            ReevalStep::Notify(msg) => {
                if send(socket, &msg).await.is_err() {
                    return false;
                }
            }
            ReevalStep::Terminate(msg) => {
                subs.remove(&id);
                slots.release_one();
                if send(socket, &msg).await.is_err() {
                    return false;
                }
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Evaluation + diff primitives
// ---------------------------------------------------------------------------

/// sq-bxog (Copilot #120): a categorised initial-evaluation failure — the
/// human-readable `message` (named after the same limits a `/sparql` 413/503 would name) plus a
/// [`RefuseKind`] so the refusal status mirrors the HTTP surface
/// ([`crate::http::engine_error_response`]) instead of collapsing everything into 503.
struct EvalError {
    message: String,
    kind: RefuseKind,
}

/// sq-bxog (Copilot #120): the cause of an initial-evaluation failure, classified from
/// the engine's budget-violation string EXACTLY as [`crate::http::engine_error_response`] does, so
/// the SSE status is consistent with `/sparql`.
enum RefuseKind {
    /// `query budget exceeded (timeout)` → 503 (genuine overload), matching `/sparql`'s timeout.
    Timeout,
    /// `query budget exceeded (max-rows)` → 413 (the result is too large, NOT a capacity problem).
    RowCap,
    /// Any other engine error (a denied SERVICE, a worker panic) → 500, like `execution_error`.
    Other,
}

impl EvalError {
    /// The HTTP status the SSE transport surfaces for this failure — the SAME mapping
    /// [`crate::http::engine_error_response`] applies on `/sparql`.
    fn status(&self) -> StatusCode {
        match self.kind {
            RefuseKind::Timeout => StatusCode::SERVICE_UNAVAILABLE,
            RefuseKind::RowCap => StatusCode::PAYLOAD_TOO_LARGE,
            RefuseKind::Other => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Runs the SELECT on the blocking pool under the server budget (deadline + row cap),
/// with the same hard await-cap as the HTTP path. Returns the head vars and the result
/// keyed by canonical row encoding.
async fn evaluate(state: &AppState, query: String) -> Result<(Vec<String>, HashMap<String, Value>), EvalError> {
    let config = state.config().clone();
    let budget = make_budget(&config, true);
    // Pin the current generation for this evaluation (lock-free; never blocked by the
    // writer). Each re-evaluation pins afresh, so it always sees the latest commit.
    let gen = state.current();
    // sq-4w18: the SERVICE egress allowlist applies to subscription SELECTs
    // exactly as to /sparql ones — a federated subscription is gated like a read.
    let cfg = config.clone();
    let task = tokio::task::spawn_blocking(move || eval_rows(gen.snapshot(), &query, &budget, &cfg));
    let joined = match config.query_timeout {
        Some(t) => tokio::time::timeout(t + TIMEOUT_GRACE, task)
            .await
            .map_err(|_| budget_error("query budget exceeded (timeout)", &config))?,
        None => task.await,
    };
    match joined {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(budget_error(&e, &config)),
        Err(_) => Err(EvalError { message: "query worker panicked".into(), kind: RefuseKind::Other }),
    }
}

/// Maps the engine's budget-violation strings onto the messages the HTTP layer uses (so a refused
/// subscription names the same limits as a 503/413 would) AND classifies the cause ([`RefuseKind`])
/// so the SSE status matches `/sparql` ([`crate::http::engine_error_response`]): timeout → 503,
/// row cap → 413, else → 500.
fn budget_error(e: &str, config: &ServerConfig) -> EvalError {
    if e.contains("query budget exceeded (timeout)") {
        let secs = config.query_timeout.map(|t| t.as_secs()).unwrap_or(0);
        return EvalError { message: format!("query timed out (server limit: {secs}s)"), kind: RefuseKind::Timeout };
    }
    if e.contains("query budget exceeded (max-rows)") {
        let max = config.max_results.unwrap_or(0);
        return EvalError { message: format!("result exceeds the server's max-results limit ({max} rows)"), kind: RefuseKind::RowCap };
    }
    EvalError { message: e.to_string(), kind: RefuseKind::Other }
}

/// Evaluates the SELECT and canonicalises each solution row to its SPARQL-JSON binding
/// object; the serialised object is the row's identity key. `serde_json`'s map keeps a
/// deterministic key order, so equal rows always serialise identically. Duplicate rows
/// collapse (set semantics — see module docs).
fn eval_rows(graph: &Graph, query: &str, budget: &QueryBudget, config: &ServerConfig) -> Result<(Vec<String>, HashMap<String, Value>), String> {
    // Extension functions (the `geo` feature's geof: registry) AND the SERVICE egress
    // allowlist (sq-4w18) apply to subscription SELECTs exactly as to /sparql ones.
    let r = crate::http::with_engine_scope(config, || sparq_engine::query_with_budget(graph, query, budget))?;
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut rows = HashMap::with_capacity(r.rows.len());
    for row in &r.rows {
        let mut obj = serde_json::Map::new();
        for (vi, term) in row.iter().enumerate() {
            if let Some(t) = term {
                obj.insert(vars[vi].clone(), term_json(t));
            }
        }
        let binding = Value::Object(obj);
        rows.insert(binding.to_string(), binding);
    }
    Ok((vars, rows))
}

/// SPARQL 1.1 JSON results encoding of an RDF term — same conventions as the engine's
/// `query_json` (no `datatype` member for plain `xsd:string`; the SPARQL 1.2
/// `{"type":"triple"}` encoding for RDF 1.2 triple terms).
fn term_json(t: &oxrdf::Term) -> Value {
    match t {
        oxrdf::Term::NamedNode(n) => json!({ "type": "uri", "value": n.as_str() }),
        oxrdf::Term::BlankNode(b) => json!({ "type": "bnode", "value": b.as_str() }),
        oxrdf::Term::Literal(l) => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), "literal".into());
            obj.insert("value".into(), l.value().into());
            if let Some(lang) = l.language() {
                obj.insert("xml:lang".into(), lang.into());
            } else {
                let dt = l.datatype();
                if dt.as_str() != "http://www.w3.org/2001/XMLSchema#string" {
                    obj.insert("datatype".into(), dt.as_str().into());
                }
            }
            Value::Object(obj)
        }
        oxrdf::Term::Triple(t) => {
            let subject = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => term_json(&oxrdf::Term::NamedNode(n.clone())),
                oxrdf::NamedOrBlankNode::BlankNode(b) => term_json(&oxrdf::Term::BlankNode(b.clone())),
            };
            json!({
                "type": "triple",
                "value": {
                    "subject": subject,
                    "predicate": term_json(&oxrdf::Term::NamedNode(t.predicate.clone())),
                    "object": term_json(&t.object),
                }
            })
        }
    }
}

/// All bindings of a stored result, in deterministic (key) order.
fn sorted_bindings(rows: &HashMap<String, Value>) -> Vec<&Value> {
    sorted_pairs(rows.iter())
}

/// Sorts `(key, binding)` pairs by canonical key so notification output is deterministic
/// (`HashMap` iteration order is randomised).
fn sorted_pairs<'a>(iter: impl Iterator<Item = (&'a String, &'a Value)>) -> Vec<&'a Value> {
    let mut pairs: Vec<(&String, &Value)> = iter.collect();
    pairs.sort_unstable_by_key(|(k, _)| *k);
    pairs.into_iter().map(|(_, v)| v).collect()
}

// ---------------------------------------------------------------------------
// Message builders
// ---------------------------------------------------------------------------

/// A SPARQL JSON results object (`{"head":{"vars":[…]},"results":{"bindings":[…]}}`).
fn results_json(vars: &[String], bindings: &[&Value]) -> Value {
    json!({ "head": { "vars": vars }, "results": { "bindings": bindings } })
}

/// The SEPA-shaped notification: added/removed are both full SPARQL JSON results objects.
fn notification(id: u64, alias: Option<&str>, sequence: u64, vars: &[String], added: &[&Value], removed: &[&Value]) -> Value {
    let body = json!({
        "notification": {
            "id": id,
            "sequence": sequence,
            "addedResults": results_json(vars, added),
            "removedResults": results_json(vars, removed),
        }
    });
    with_alias(body, "notification", alias)
}

/// Echoes the client's alias inside the named envelope, when one was given (SEPA echoes
/// the alias so clients can correlate without tracking ids).
fn with_alias(mut msg: Value, envelope: &str, alias: Option<&str>) -> Value {
    if let Some(alias) = alias {
        msg[envelope]["alias"] = alias.into();
    }
    msg
}

fn error_msg(message: &str, id: Option<u64>, alias: Option<&str>) -> Value {
    let mut inner = serde_json::Map::new();
    inner.insert("message".into(), message.into());
    if let Some(id) = id {
        inner.insert("id".into(), id.into());
    }
    if let Some(alias) = alias {
        inner.insert("alias".into(), alias.into());
    }
    json!({ "error": Value::Object(inner) })
}

async fn send(socket: &mut WebSocket, msg: &Value) -> Result<(), axum::Error> {
    // axum 0.8: ws Message::Text wraps Utf8Bytes; String converts via Into.
    socket.send(Message::Text(msg.to_string().into())).await
}

// ---------------------------------------------------------------------------
// sq-bxog: SSE (Server-Sent Events) transport
// ---------------------------------------------------------------------------

/// SSE (`text/event-stream`) transport for SPARQL update subscriptions, ALONGSIDE the
/// WebSocket `/subscriptions` endpoint. Both transports share ONE notification source —
/// `subscribe_init` (register + initial result) and `Subscription::reevaluate_step`
/// (re-evaluate + diff), driven by the same commit-generation `watch` channel
/// (`AppState::subscribe_commits`) and the same global/per-connection slot accounting
/// (`ConnSlots`). Only the wire framing differs: SSE emits `event:`/`data:`/`id:` frames
/// instead of WebSocket JSON text frames.
///
/// Because SSE is a one-way GET stream (no client→server channel after the request),
/// each connection carries exactly ONE subscription, identified by the `query` (and
/// optional `alias`) query-string parameters — the natural REST shape, versus the WS
/// path's multiplexed many-subscriptions-per-socket protocol. There is no `unsubscribe`
/// frame: a client unsubscribes by closing the stream, which drops the per-stream state
/// (releasing its global slot via `ConnSlots`'s `Drop`) with no leak.
pub mod sse {
    use std::collections::HashMap;

    use axum::extract::{Query, State};
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::{IntoResponse, Response};
    use futures_util::Stream;
    use serde_json::Value;

    use super::{subscribe_init, ConnSlots, ReevalStep, Subscription};
    use crate::http::AppState;

    /// SSE keep-alive comment interval (`: ping\n\n` every 15 s) — holds idle connections
    /// open across proxies/load balancers that would otherwise reap a silent stream.
    const KEEP_ALIVE_SECS: u64 = 15;

    /// The per-stream state threaded through the [`futures_util::stream::unfold`] generator.
    /// Dropping it (on client disconnect, when axum stops polling the body) drops `_slots`,
    /// releasing the global subscription slot — the leak-free disconnect path, mirroring the
    /// WebSocket connection task's `ConnSlots` drop.
    struct StreamState {
        id: u64,
        subscription: Subscription,
        commits: tokio::sync::watch::Receiver<u64>,
        state: AppState,
        /// Opening frames (subscribed ack, then the sequence-0 notification) and any
        /// terminating error, queued FIFO so each `unfold` step yields exactly one event.
        pending: std::collections::VecDeque<Event>,
        /// Once a `Terminate` (re-evaluation failure) has been queued, the stream ends after
        /// draining `pending`.
        finished: bool,
        /// Held for the stream's lifetime; `Drop` releases the global slot on disconnect.
        _slots: ConnSlots,
    }

    /// `GET /subscriptions/sse?query=<SELECT>[&alias=<x>]` — registers ONE SPARQL update
    /// subscription and streams notifications as Server-Sent Events.
    ///
    /// Auth (sq-cxk5): this is a READ surface (live SELECT diffs), so it mirrors the
    /// WebSocket `/subscriptions` path AND the other `/sparql` GET routes — when
    /// `--auth-token-read` is configured the GET is gated behind the read token via
    /// `crate::http::auth_gate` (`crate::http::Operation::Read`) and refused with the SAME
    /// 401 BEFORE the event-stream opens. As a plain GET, the `Authorization: Bearer <token>`
    /// header is the only auth channel here (no WS subprotocol). When no read token is configured
    /// the GET is unchanged (open access) — back-compatible.
    ///
    /// A registration refusal is returned as a normal JSON HTTP error BEFORE the event-stream
    /// opens — SSE cannot set a status once the stream is flowing — classified to match the
    /// `/sparql` endpoint (`crate::http::engine_error_response`): missing/non-SELECT/malformed
    /// query → **400**; the initial evaluation over the row cap (`--max-results` /
    /// `--max-query-rows`) → **413**; the initial evaluation timing out, or subscription-slot
    /// exhaustion → **503**; any other engine error → **500**. The status is carried on the
    /// `super::Refusal`. On success the response is `text/event-stream` and the first two
    /// frames are the `subscribed` ack and the full initial result.
    pub async fn sse_endpoint(State(state): State<AppState>, headers: axum::http::HeaderMap, Query(params): Query<HashMap<String, String>>) -> Response {
        // sq-cxk5: gate the read surface behind the read token (fail-closed when
        // required + absent/wrong), BEFORE opening the stream — exactly like a `/sparql` GET.
        if let Some(resp) = crate::http::auth_gate(state.config(), &headers, crate::http::Operation::Read) {
            return resp;
        }
        let Some(query) = params.get("query") else {
            return crate::http::json_error(
                axum::http::StatusCode::BAD_REQUEST,
                "the 'query' query-string parameter is required (the SELECT to subscribe)",
            );
        };
        let alias = params.get("alias").cloned();

        let mut slots = ConnSlots::new(state.subs.clone());
        // One subscription per SSE stream, so the per-connection count starts at 0.
        let new = match subscribe_init(&state, &mut slots, 0, query, alias).await {
            Ok(n) => n,
            Err(refusal) => {
                // sq-bxog (Copilot #120): the refusal already carries the HTTP status,
                // classified at the point of refusal to match the `/sparql` endpoint (400 client /
                // 413 row-cap / 503 capacity-or-timeout / 500 other) — see [`super::Refusal`].
                let msg = refusal.error["error"]["message"].as_str().unwrap_or("subscription refused").to_string();
                return crate::http::json_error(refusal.status, &msg);
            }
        };

        // Drop generations committed before this stream existed: the initial frame already
        // reflects the current graph (same `mark_unchanged` discipline as the WS task).
        let mut commits = state.subscribe_commits();
        commits.mark_unchanged();

        let mut pending = std::collections::VecDeque::with_capacity(2);
        pending.push_back(json_event("subscribed", &new.subscribed));
        pending.push_back(notification_event(&new.initial));

        let init = StreamState {
            id: new.id,
            subscription: new.subscription,
            commits,
            state,
            pending,
            finished: false,
            _slots: slots,
        };

        Sse::new(into_event_stream(init))
            .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(KEEP_ALIVE_SECS)).text("ping"))
            .into_response()
    }

    /// Builds the infallible `Stream<Item = Result<Event, Infallible>>` axum's [`Sse`]
    /// wants. Each `unfold` step first drains any queued frame (the opening pair, or a
    /// terminating error); when empty, it awaits the next commit and re-evaluates,
    /// emitting one diff `notification` (or terminating on failure). Returning `None`
    /// ends the stream; the watch channel closing (server shutdown) ends it too.
    fn into_event_stream(init: StreamState) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
        futures_util::stream::unfold(init, |mut st| async move {
            loop {
                if let Some(ev) = st.pending.pop_front() {
                    return Some((Ok(ev), st));
                }
                if st.finished {
                    return None; // terminating error already drained — close the stream
                }
                // Block until a commit advances the published generation (the watch
                // channel coalesces bursts — see the module docs), then re-evaluate.
                if st.commits.changed().await.is_err() {
                    return None; // server shutting down
                }
                let id = st.id;
                match st.subscription.reevaluate_step(id, &st.state).await {
                    ReevalStep::Unchanged => continue, // no diff — keep waiting (no frame)
                    ReevalStep::Notify(msg) => return Some((Ok(notification_event(&msg)), st)),
                    ReevalStep::Terminate(msg) => {
                        st.finished = true;
                        return Some((Ok(json_event("error", &msg)), st));
                    }
                }
            }
        })
    }

    /// A `notification` SSE frame: `event: notification`, the JSON as `data:`, and the
    /// per-subscription `sequence` as the SSE `id:` (so a client can track ordering /
    /// `Last-Event-ID` resumption semantics at the application layer).
    fn notification_event(msg: &Value) -> Event {
        let ev = json_event("notification", msg);
        match msg["notification"]["sequence"].as_u64() {
            Some(seq) => ev.id(seq.to_string()),
            None => ev,
        }
    }

    /// A typed SSE frame: `event: <name>` with the JSON value serialised as the `data:`
    /// payload (single line — `serde_json` never emits a bare newline, so SSE framing is
    /// preserved without escaping). The on-the-wire framing (`event:`/`data:`/`id:`/`\n\n`)
    /// is asserted end-to-end in `tests/subscriptions_sse.rs`, which reads the raw bytes
    /// (axum's [`Event`] finalises to bytes internally and exposes no inspection API).
    fn json_event(name: &str, value: &Value) -> Event {
        Event::default().event(name).data(value.to_string())
    }
}

// ---------------------------------------------------------------------------
// Unit tests (pure pieces; the protocol end-to-end lives in tests/subscriptions.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(ttl: &str) -> Graph {
        Graph::load_str(ttl, "turtle").unwrap()
    }

    #[test]
    fn eval_rows_canonical_keys_are_stable_across_evaluations() {
        let g = graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:c ex:p \"x\"@en .");
        let q = "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }";
        let (vars1, rows1) = eval_rows(&g, q, &QueryBudget::unlimited(), &ServerConfig::default()).unwrap();
        let (vars2, rows2) = eval_rows(&g, q, &QueryBudget::unlimited(), &ServerConfig::default()).unwrap();
        assert_eq!(vars1, vec!["s", "o"]);
        assert_eq!(vars1, vars2);
        assert_eq!(rows1.keys().collect::<std::collections::BTreeSet<_>>(), rows2.keys().collect());
        assert_eq!(rows1.len(), 2);
    }

    #[test]
    fn diff_detects_added_and_removed() {
        let before = graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:c ex:p ex:d .");
        let after = graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:e ex:p ex:f .");
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";
        let (_, old) = eval_rows(&before, q, &QueryBudget::unlimited(), &ServerConfig::default()).unwrap();
        let (_, new) = eval_rows(&after, q, &QueryBudget::unlimited(), &ServerConfig::default()).unwrap();
        let added = sorted_pairs(new.iter().filter(|(k, _)| !old.contains_key(*k)));
        let removed = sorted_pairs(old.iter().filter(|(k, _)| !new.contains_key(*k)));
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["s"]["value"], "http://ex/e");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0]["s"]["value"], "http://ex/c");
    }

    #[test]
    fn term_json_matches_sparql_json_conventions() {
        use oxrdf::{Literal, NamedNode, Term};
        let uri = term_json(&Term::NamedNode(NamedNode::new("http://ex/a").unwrap()));
        assert_eq!(uri, json!({"type": "uri", "value": "http://ex/a"}));
        // Plain xsd:string carries no datatype member.
        let plain = term_json(&Term::Literal(Literal::new_simple_literal("hi")));
        assert_eq!(plain, json!({"type": "literal", "value": "hi"}));
        let lang = term_json(&Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap()));
        assert_eq!(lang, json!({"type": "literal", "value": "hi", "xml:lang": "en"}));
        let typed = term_json(&Term::Literal(Literal::new_typed_literal(
            "1",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )));
        assert_eq!(typed["datatype"], "http://www.w3.org/2001/XMLSchema#integer");
    }

    #[test]
    fn unbound_variables_are_omitted_from_the_binding() {
        let g = graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b .");
        let q = "SELECT ?s ?missing WHERE { ?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?missing } }";
        let (vars, rows) = eval_rows(&g, q, &QueryBudget::unlimited(), &ServerConfig::default()).unwrap();
        assert_eq!(vars, vec!["s", "missing"]);
        let binding = rows.values().next().unwrap();
        assert!(binding.get("missing").is_none());
        assert_eq!(binding["s"]["type"], "uri");
    }

    #[test]
    fn max_rows_budget_refuses_oversized_results() {
        let g = graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:c ex:p ex:d . ex:e ex:p ex:f .");
        let budget = QueryBudget { max_rows: Some(2), ..QueryBudget::unlimited() };
        let err = eval_rows(&g, "SELECT ?s WHERE { ?s ?p ?o }", &budget, &ServerConfig::default()).unwrap_err();
        assert!(err.contains("max-rows"), "unexpected error: {err}");
    }

    #[test]
    fn notification_shape_is_sepa_like() {
        let vars = vec!["s".to_string()];
        let b = json!({"s": {"type": "uri", "value": "http://ex/a"}});
        let msg = notification(7, Some("watch"), 3, &vars, &[&b], &[]);
        assert_eq!(msg["notification"]["id"], 7);
        assert_eq!(msg["notification"]["alias"], "watch");
        assert_eq!(msg["notification"]["sequence"], 3);
        assert_eq!(msg["notification"]["addedResults"]["head"]["vars"], json!(["s"]));
        assert_eq!(msg["notification"]["addedResults"]["results"]["bindings"][0], b);
        assert_eq!(msg["notification"]["removedResults"]["results"]["bindings"], json!([]));
    }

    // sq-4vao: behavioural tests for the lowest-covered subscription pieces — the
    // budget→status classification, the RDF-1.2 triple-term encoding, the error-frame builder
    // edges, and the transport-agnostic reevaluate_step / subscribe_init state machine (the
    // added/removed diff, mid-stream termination, and the refusal classes).

    #[test]
    fn term_json_encodes_an_rdf12_triple_term() {
        // The `oxrdf::Term::Triple` arm (the SPARQL 1.2 triple-term encoding) is the only
        // term shape the existing tests miss. A solution row whose object is a triple term must
        // serialise to `{"type":"triple","value":{subject,predicate,object}}`.
        use oxrdf::{Literal, NamedNode, Term, Triple};
        let inner = Triple::new(
            NamedNode::new("http://ex/s").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Literal::new_simple_literal("o"),
        );
        let v = term_json(&Term::Triple(Box::new(inner)));
        assert_eq!(v["type"], "triple");
        assert_eq!(v["value"]["subject"], json!({"type": "uri", "value": "http://ex/s"}));
        assert_eq!(v["value"]["predicate"], json!({"type": "uri", "value": "http://ex/p"}));
        assert_eq!(v["value"]["object"], json!({"type": "literal", "value": "o"}));
    }

    // sq-qcnn.37: cover the two term_json branches left uncovered by the existing
    // tests — the top-level BlankNode arm and the Triple-subject BlankNode arm.

    #[test]
    fn term_json_encodes_a_blank_node() {
        // The `Term::BlankNode` arm is the only top-level shape the existing tests miss.
        // A blank node must serialise to `{"type":"bnode","value":"<identifier>"}`.
        use oxrdf::{BlankNode, Term};
        let b = BlankNode::new("b0").unwrap();
        let v = term_json(&Term::BlankNode(b));
        assert_eq!(v["type"], "bnode");
        assert_eq!(v["value"], "b0");
    }

    #[test]
    fn term_json_triple_term_with_blank_node_subject() {
        // The `NamedOrBlankNode::BlankNode` arm inside the Triple path is missed by the
        // existing `term_json_encodes_an_rdf12_triple_term` test (which uses a NamedNode
        // subject). A Triple with a BlankNode subject must embed a `bnode` sub-object.
        use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
        let inner = Triple::new(
            BlankNode::new("b1").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Literal::new_simple_literal("val"),
        );
        let v = term_json(&Term::Triple(Box::new(inner)));
        assert_eq!(v["type"], "triple");
        assert_eq!(v["value"]["subject"], json!({"type": "bnode", "value": "b1"}));
    }

    #[test]
    fn budget_error_classifies_engine_strings_into_sse_statuses() {
        let cfg = ServerConfig {
            query_timeout: Some(std::time::Duration::from_secs(7)),
            max_results: Some(99),
            ..ServerConfig::default()
        };
        // Timeout → 503, and the message names the configured limit in seconds.
        let timeout = budget_error("query budget exceeded (timeout)", &cfg);
        assert_eq!(timeout.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(timeout.message.contains("7s"), "msg: {}", timeout.message);
        // Row cap → 413, and the message names the configured max-results.
        let rowcap = budget_error("query budget exceeded (max-rows)", &cfg);
        assert_eq!(rowcap.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(rowcap.message.contains("99"), "msg: {}", rowcap.message);
        // Anything else → 500, message passed through verbatim.
        let other = budget_error("denied SERVICE host", &cfg);
        assert_eq!(other.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(other.message, "denied SERVICE host");
    }

    #[test]
    fn error_msg_carries_id_and_alias_when_present() {
        // The id+alias arms (the `Some` branches) are exercised only by a terminating /
        // unsubscribe-failure frame; assert both members land under the `error` envelope.
        let with = error_msg("re-evaluation failed", Some(42), Some("watch"));
        assert_eq!(with["error"]["message"], "re-evaluation failed");
        assert_eq!(with["error"]["id"], 42);
        assert_eq!(with["error"]["alias"], "watch");
        // And that neither is emitted when absent (the `None` arms).
        let bare = error_msg("bad json", None, None);
        assert_eq!(bare["error"]["message"], "bad json");
        assert!(bare["error"].get("id").is_none());
        assert!(bare["error"].get("alias").is_none());
    }

    /// A `Subscription` whose diff baseline is the result of `query` over `state` right now,
    /// leaking its global slot (the test owns the state lifetime and asserts nothing about leaks).
    async fn seeded_sub(state: &AppState, alias: Option<&str>, query: &str) -> Subscription {
        let mut slots = ConnSlots::new(state.subs.clone());
        let new = match subscribe_init(state, &mut slots, 0, query, alias.map(str::to_string)).await {
            Ok(n) => n,
            Err(r) => panic!("initial subscribe should succeed, refused with {}", r.status),
        };
        std::mem::forget(slots); // hold the acquired slot for the test's lifetime
        new.subscription
    }

    #[tokio::test]
    async fn reevaluate_step_is_unchanged_then_notifies_on_a_real_diff() {
        let state = AppState::new(graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b ."));
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";
        let mut sub = seeded_sub(&state, Some("w"), q).await;

        // No change to the graph → the diff is empty → Unchanged (emit nothing).
        assert!(matches!(sub.reevaluate_step(1, &state).await, ReevalStep::Unchanged));

        // Insert a matching triple → re-evaluation yields one added row, no removed rows.
        state
            .apply_update("INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }")
            .unwrap();
        match sub.reevaluate_step(1, &state).await {
            ReevalStep::Notify(msg) => {
                assert_eq!(msg["notification"]["id"], 1);
                assert_eq!(msg["notification"]["alias"], "w");
                // sequence advanced from the seeded 0 to 1.
                assert_eq!(msg["notification"]["sequence"], 1);
                let added = &msg["notification"]["addedResults"]["results"]["bindings"];
                assert_eq!(added.as_array().unwrap().len(), 1);
                assert_eq!(added[0]["s"]["value"], "http://ex/c");
                assert_eq!(msg["notification"]["removedResults"]["results"]["bindings"], json!([]));
            }
            ReevalStep::Unchanged => panic!("expected Notify after an insert, got Unchanged"),
            ReevalStep::Terminate(msg) => panic!("expected Notify, got Terminate: {msg}"),
        }
        // The baseline advanced, so an immediate re-evaluation with no further change is Unchanged.
        assert!(matches!(sub.reevaluate_step(1, &state).await, ReevalStep::Unchanged));
    }

    #[tokio::test]
    async fn reevaluate_step_terminates_when_a_later_result_overflows_the_row_cap() {
        // Seed under a row cap of 1 with exactly one matching row (initial eval fits), then grow
        // the graph past the cap so the NEXT re-evaluation overflows → Terminate, carrying the
        // row-cap message and the subscription id.
        let cfg = ServerConfig { max_results: Some(1), ..ServerConfig::default() };
        let state = AppState::with_config(graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b ."), cfg);
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";
        let mut sub = seeded_sub(&state, None, q).await;

        state
            .apply_update("INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }")
            .unwrap();
        match sub.reevaluate_step(5, &state).await {
            ReevalStep::Terminate(msg) => {
                assert_eq!(msg["error"]["id"], 5);
                let m = msg["error"]["message"].as_str().unwrap();
                assert!(m.contains("re-evaluation failed"), "msg: {m}");
                assert!(m.contains("max-results"), "msg: {m}");
            }
            ReevalStep::Unchanged => panic!("expected Terminate on row-cap overflow, got Unchanged"),
            ReevalStep::Notify(msg) => panic!("expected Terminate on row-cap overflow, got Notify: {msg}"),
        }
    }

    #[tokio::test]
    async fn subscribe_init_refuses_non_select_and_malformed_queries() {
        let state = AppState::new(graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b ."));
        let mut slots = ConnSlots::new(state.subs.clone());

        // A well-formed but non-SELECT query (ASK) is a client error → 400.
        let ask = subscribe_init(&state, &mut slots, 0, "ASK { ?s ?p ?o }", None)
            .await
            .err()
            .expect("ASK is not subscribable");
        assert_eq!(ask.status, StatusCode::BAD_REQUEST);
        assert!(ask.error["error"]["message"].as_str().unwrap().contains("only SELECT"));

        // A malformed query is also a 400, with the parse error surfaced.
        let bad = subscribe_init(&state, &mut slots, 0, "SELECT ?s WHERE {", None)
            .await
            .err()
            .expect("a malformed query is refused");
        assert_eq!(bad.status, StatusCode::BAD_REQUEST);
        assert!(bad.error["error"]["message"].as_str().unwrap().contains("malformed query"));

        // A refusal holds NO global slot.
        assert_eq!(state.subs.active_count(), 0);
    }

    #[tokio::test]
    async fn subscribe_init_enforces_per_connection_and_global_slot_caps() {
        let cfg = ServerConfig { max_subscriptions_per_conn: 2, max_subscriptions: 1, ..ServerConfig::default() };
        let state = AppState::with_config(graph("@prefix ex: <http://ex/> . ex:a ex:p ex:b ."), cfg);
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";

        let mut slots = ConnSlots::new(state.subs.clone());
        // current_conn_subs already at the per-conn cap → refuse with 503, no slot taken.
        let per_conn = subscribe_init(&state, &mut slots, 2, q, None)
            .await
            .err()
            .expect("per-connection cap refuses");
        assert_eq!(per_conn.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(per_conn.error["error"]["message"].as_str().unwrap().contains("this connection"));
        assert_eq!(state.subs.active_count(), 0);

        // Take the single global slot, then a second attempt hits the global cap → 503.
        assert!(subscribe_init(&state, &mut slots, 0, q, None).await.is_ok(), "first global slot should be granted");
        assert_eq!(state.subs.active_count(), 1);
        let global = subscribe_init(&state, &mut slots, 0, q, None)
            .await
            .err()
            .expect("global cap refuses");
        assert_eq!(global.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(global.error["error"]["message"].as_str().unwrap().contains("server-wide"));
        // The failed global acquire released its provisional slot — still exactly one held.
        assert_eq!(state.subs.active_count(), 1);
    }
}
