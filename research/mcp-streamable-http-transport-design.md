# `sparq-mcp`: an HTTP (Streamable HTTP + SSE) MCP transport beyond stdio

**Status:** design record, 2026-07-29. Bead `sq-2c0f0`, gh #3221. Design-for-review — this
record contains **no implementation**. It exists because
`research/mcp-rmcp-sdk-adoption-assessment.md` (sq-95zda, gh #3219) made one explicit
demand: *"If the HTTP/SSE transport work is scheduled, re-run this comparison **first** —
the decision should be made before the transport is hand-rolled, not after."* That is
[§ The decision that had to come first](#the-decision-that-had-to-come-first).

> 🤖 This record was written by a SPARQ agent.

## Verdict

1. **Build ONE transport, not two.** Implement **Streamable HTTP** (single MCP endpoint,
   `POST` + `GET` + `DELETE`, SSE bodies where useful). **Decline** the deprecated
   2024-11-05 HTTP+SSE two-endpoint transport until a named client actually requires it.
   The bead title's "SSE/streamable-HTTP" reads like two deliverables; it is one — see
   [§ Premise correction](#premise-correction-sse-is-not-a-second-transport).
2. **Hand-roll it on the axum stack already in the tree, do not adopt `rmcp`.** Measured
   today: the axum route costs **+29 crates** in `sparq-mcp`'s closure and **0 crates new
   to the workspace `Cargo.lock`**; `rmcp` cost +45–59 with 1–13 new to the lock, of which
   5 of the 8 at its server tier needed new exemptions or first-party audits. sq-95zda
   expected its trigger 1 to flip the verdict; the measurement says it **does not**, because
   the tree already owns a vetted async HTTP + SSE stack the SDK would duplicate. This is
   the one place where this record overrides a prior record's expectation, and it does so on
   a number, not an opinion.
3. **Keep `handle_message` exactly as it is.** The transport wraps the existing sync
   dispatch core behind `spawn_blocking`; the public embedder seam does not change and no
   existing test is rewritten.
4. **The security posture is the hard part, not the framing.** stdio's trust boundary *is
   the pipe*; HTTP deletes that boundary. The transport must ship Origin validation,
   loopback-default binding, a Bearer gate, and a **fail-closed refusal to serve
   `allow_update` on a non-loopback bind without a token** — see
   [§ Security](#security-the-part-that-actually-needs-review).
5. **Scope the first phase to the base `McpServer`.** `SolidMcpServer` binds ONE
   authenticated Solid session at construction, so per-HTTP-session pod identity is a
   separate, larger design — [§ Deliberately out of scope](#deliberately-out-of-scope).
   That scope has a consequence this record states rather than hides: the only queued
   server-initiated notifications in the tree belong to `SolidMcpServer`, so this
   transport builds the *channel* for them and does not itself deliver them.

## Premise correction: "SSE" is not a second transport

The MCP specification has defined exactly two standard transports since 2025-03-26: stdio
and **Streamable HTTP**. The 2024-11-05 revision's **HTTP+SSE** transport — a long-lived
`GET /sse` stream that emits an `endpoint` event, plus a separate POST endpoint for client
messages — is **deprecated and replaced**; the current spec describes it only under
*Backwards Compatibility*. In Streamable HTTP, SSE is not a transport at all: it is one of
the two permitted response *body* formats for a POST (`text/event-stream` or
`application/json`), plus an optional server-initiated `GET` stream.

So a plan that builds "SSE **and** streamable HTTP" builds one current transport and one
dead one. The right reading of the bead is: **implement Streamable HTTP; SSE falls out of
it.** Supporting the deprecated transport as well means hosting a second endpoint pair
forever for clients nobody has named — an explicit maintainer question, recorded in
[§ Open questions](#open-questions-for-the-maintainer), not an agent's default.

Sources: MCP spec, *Basic / Transports*, revisions
<https://modelcontextprotocol.io/specification/2025-06-18/basic/transports> and
<https://modelcontextprotocol.io/specification/2025-11-25/basic/transports> (both fetched
2026-07-29).

## What exists today (verified against the code, not the brief)

| surface | state | where |
| --- | --- | --- |
| `McpServer::handle_message(&mut self, &str) -> Option<String>` | sync, I/O-free, accepts a single message or a batch array | `crates/sparq-mcp/src/server.rs:187` |
| stdio transport | line-delimited loop over the above, feature `stdio` | `crates/sparq-mcp/src/transport.rs` |
| `sparq-mcp` binary | stdio only; `--allow-update` / `--format` / `--query-timeout` / `--max-rows` | `crates/sparq-mcp/src/main.rs`, `src/cli.rs` |
| any HTTP transport | **none** | — |
| server-initiated notifications | queued on `SolidMcpServer` only, and **no shipped transport drains them**; the base `McpServer` declares `subscribe: false` and enqueues nothing | `crates/sparq-mcp/src/solid.rs:873`, `crates/sparq-mcp/src/server.rs:226` |
| `PROTOCOL_VERSION` | `"2025-11-25"` | `crates/sparq-mcp/src/server.rs:29` |
| `SUPPORTED_PROTOCOL_VERSIONS` | `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` | `crates/sparq-mcp/src/server.rs:44` |
| trust model | "no built-in authentication or authorization … the stdio transport is a trust boundary you, the operator, establish" | `crates/sparq-mcp/README.md:93` |

Two of these deserve emphasis because they change what the transport is *for*.

**The notification queue has no delivery channel today, and this transport alone does not
give it one.** `SolidMcpServer` implements `resources/subscribe` and advertises
`subscribe: true`, and enqueues content-free `notifications/resources/updated` messages —
but `take_notifications` is called in exactly one place in the whole tree,
`crates/sparq-mcp/tests/solid.rs:785`. The `stdio` serve loop never drains it: it only
writes what `handle_message` returns. So an embedder must invent its own pump.

The honest scoping consequence, which the rest of this record is written to respect: that
queue is on **`SolidMcpServer`**, a distinct type, and this transport wraps the **base
`McpServer`**, which declares `subscribe: false` (`crates/sparq-mcp/src/server.rs:226`) and
enqueues nothing. A transport around `McpServer` cannot reach a `SolidMcpServer`'s queue,
and both types merely existing in one process does not bridge them. So Streamable HTTP's
server-initiated `GET` stream is the **shape** of the delivery channel that surface has
been missing, and phases 1–5 build it; actually delivering `SolidMcpServer`'s
notifications additionally needs (a) a notification-source seam the transport drains and
(b) the Solid-over-HTTP session/authorization design that owns the `SolidMcpServer`
instances — both out of scope here, see [§ Deliberately out of
scope](#deliberately-out-of-scope). The motivation for *this* bead therefore remains
remote/multiplexed clients, plus building the channel that makes the Solid work
schedulable.

**The trust model is transport-shaped.** The README's honesty about having no auth is
currently *load-bearing on stdio*: a pipe is only reachable by whoever the operator handed
it to. A TCP socket is not. Adding HTTP without adding auth would not preserve the current
posture — it would silently void it.

## The decision that had to come first

sq-95zda declined `rmcp` and named its own strongest reopen trigger: *"An HTTP/SSE MCP
transport is actually required. This is the decisive trigger. A hand-rolled
streamable-HTTP transport with session resumption is a far larger surface than 146 lines,
and at that point '+45 crates' buys something real."* The trigger has fired. Re-running it:

### The dependency delta, measured today

Measured on 2026-07-29 against the workspace `Cargo.lock` by temporarily adding an `http`
feature to `crates/sparq-mcp/Cargo.toml`, resolving `cargo tree`, diffing package-name
sets, and reverting the edit (reproduction in
[§ Reproduction](#reproduction)). Baseline: `sparq-mcp` with `--no-default-features`
resolves **61 packages** (the sq-95zda record measured 62 on 2026-07-28; the closure
drifts by a package as the lock moves).

| route | `sparq-mcp` closure | new crates in that closure | new crates in the workspace lock |
| --- | --- | --- | --- |
| today (`--no-default-features`) | 61 | — | — |
| **axum route** (`axum` 0.8 `http1,tokio,json` + `tokio` + `futures-util`, plus `stdio`) | 90 | **+29** | **0** |
| `rmcp` `["server", "transport-io"]` (sq-95zda, 2026-07-28) | 67 | +52 | +8 |
| `rmcp` default (sq-95zda, 2026-07-28) | 74 | +59 | +13 |

The +29 for the axum route are: `atomic-waker axum axum-core bytes futures-channel
futures-core futures-task futures-util http http-body http-body-util httparse httpdate
hyper hyper-util matchit mime mio percent-encoding pin-project-lite serde_path_to_error
slab socket2 sync_wrapper tokio tokio-macros tower tower-layer tower-service`.

**Every one of those 29 is already in the gated `Cargo.lock` at the resolved version**,
pulled in by `sparq-server`'s `server` feature. So the supply-chain delta for the axum
route is **zero new crates to audit, zero new exemptions** — which is precisely the
condition `rmcp` failed (5 of its 8 lock-new crates needed new exemptions or first-party
audits, `rmcp` itself included). The rustdoc/clippy all-features lanes also already
compile this stack.

The honest counter-argument, stated plainly: **+29 is not free.** `sparq-mcp`'s stated
design property, in `crates/sparq-mcp/Cargo.toml:1-6`, is that it "pulls no heavy
dependency", and an async runtime is heavy. The mitigation is the one the tree uses
everywhere else and the one sq-95zda itself recommended for adoption: an **opt-in feature,
OFF by default**. `cargo build -p sparq-mcp` stays a 61-package pure-`serde_json`
data transform; `--features http` opts into the runtime. Nothing that embeds
`handle_message` today changes.

### Why the trigger's expectation does not hold

sq-95zda expected the calculus to flip because it modelled the choice as *"hand-roll a
large HTTP+session surface from nothing"* vs *"take the SDK"*. The measurement it did not
have is that **the workspace already contains a working axum SSE server**, and this
transport is largely an instance of it:

- `crates/sparq-server/src/subscriptions.rs:700-855` — module `sse`: an
  `axum::response::sse::{Sse, Event, KeepAlive}` handler over a
  `futures_util::stream::unfold` generator, with a keep-alive interval, per-stream state
  dropped on client disconnect, `Event::id()` set for `Last-Event-ID` ordering, and
  auth checked **before** the event stream opens.
- `crates/sparq-server/tests/subscriptions_sse.rs` — 449 lines asserting the raw
  `event:` / `data:` / `id:` / `\n\n` bytes on the wire.
- `crates/sparq-server/src/http.rs:1351` — `bind_posture`: a fail-closed, auth-aware
  classification of a requested bind address (`Loopback` / `RemoteAllowed{warning}` /
  `RemoteRefused{message}`), treating `0.0.0.0` and `::` as remote.
- `crates/sparq-server/src/cors_config.rs` — a no-wildcard `Origin` allowlist that emits
  no CORS header at all for an un-listed origin.

So the residual hand-rolled surface is the MCP-specific part — the endpoint contract, the
session table, and the resumability log — not an HTTP or SSE stack. `rmcp` would supply
that MCP-specific part, which is the genuine remaining argument in its favour; it just no
longer costs 146 lines of framing to reach it, and it still costs the audit ratchet the
supply-chain gate exists to prevent.

**This does not retire the sq-95zda record.** Its other three triggers stand unchanged
(`rmcp` in an imported audit set; `rmcp` declaring a `rust-version` and stabilising; sparq
needing an MCP *client*). Trigger 1 should be marked *fired and re-assessed here*, not
deleted.

## Design

### The endpoint contract

One path, `/mcp` by default, configurable. Behaviour, mapped to the spec's MUSTs:

| method / input | response |
| --- | --- |
| `POST` body = JSON-RPC **request** | `application/json` with the single response object — the default. `text/event-stream` only when the handler has interim messages to interleave. |
| `POST` body = JSON-RPC **notification** or **response** | `202 Accepted`, empty body. |
| `POST` body = batch array | answered as a batch array in `application/json`; retained because 2025-03-26 requires receiving batches and `handle_message` already handles them. Later revisions permit only a single message per POST, so this is tolerance, never a requirement we impose. |
| `POST` `initialize` | `application/json` + `Mcp-Session-Id: <id>` on the response. |
| `GET` (Accept includes `text/event-stream`) | `text/event-stream`: the session's server-initiated notification stream. With the base `McpServer` it carries keep-alives (and, from phase 5, the priming event) **and no notifications**, because that server enqueues none; it is the transport-side channel a future notification source plugs into (see [§ Phased plan](#phased-plan-each-phase-is-a-future-bead) phase 4). |
| `GET` with `Last-Event-ID` | replay from that cursor **on that stream only**, then resume. |
| `DELETE` | terminate the session; `204`. |
| any method, unknown/expired session id | `404` — the client's cue to re-`initialize`. |
| any method except `initialize`, session id absent | `400`. |
| `Origin` header present and not allowlisted | `403`. |
| `MCP-Protocol-Version` unsupported | `400`. Absent ⇒ assume `2025-03-26`, per spec. |

Returning `application/json` by default rather than always opening an SSE stream is
deliberate: the spec permits either, every tool in `tools.rs` computes a single complete
result with no interim messages, and a plain JSON response is far cheaper to serve and to
test. SSE-on-POST earns its place only when a tool gains progress notifications.

### Concurrency: the real engineering constraint

`handle_message` takes `&mut self`, and `sparq_core::Graph` (`crates/sparq-core/src/lib.rs:64`)
does **not** implement `Clone`. Three options:

- **(a) one shared server behind a mutex** — `Arc<std::sync::Mutex<McpServer>>`, every
  dispatch inside `tokio::task::spawn_blocking`, guard acquired *inside* the blocking
  closure so no lock is ever held across an `.await`. All tool calls serialise. **Recommended
  for phase 1**: it is exactly the concurrency stdio already has, so it introduces no new
  correctness question, and "multiplexed" in the bead title is satisfied at the *session*
  level (many concurrent clients, each with its own session, sharing one dataset) even
  though execution is serial.
- **(b) `RwLock` with a read/write split** — the throughput answer, and **it is not free
  today**: `text_search` needs `&mut self` because the BM25 index is lazily built and
  incrementally reconciled into `McpServer::text_index` on the read path
  (`crates/sparq-mcp/src/lib.rs:48-50`). So a read tool mutates. Splitting reads from
  writes therefore requires making that cache interior-mutable first. That is a separate,
  independently reviewable change — a later phase, and honest about its prerequisite.
- **(c) one `McpServer` per session** — needs a `Graph` per session. `Graph` is not
  `Clone` and datasets are large. Rejected.

The sync-core-inside-async detail matters and is easy to get wrong: a SPARQL query is
CPU-bound and can run to the `query_timeout_secs` deadline, so dispatching it on an async
executor thread would stall unrelated connections. `spawn_blocking` with a `'static`
`Arc<Mutex<…>>` clone moved in is the correct shape, and `tokio::sync::Mutex` is the wrong
primitive here (it is for locks held across awaits, not for long CPU holds).

### Session state

```text
SessionId          -> 128+ random bits, hex; visible-ASCII only (spec MUST)
SessionState {
    protocol_version: &'static str,   // frozen at initialize; validates MCP-Protocol-Version
    created, last_seen: Instant,      // idle expiry -> 404
    streams: Map<StreamId, EventLog>, // bounded ring buffer for Last-Event-ID replay
    outbound: VecDeque<String>,       // fed by a notification source, drained by the GET stream
                                      // (the base McpServer supplies none — see phase 4)
}
```

Three notes:

- **The session id needs a direct dependency declaration, but adds nothing to the lock.**
  A crate cannot import a transitive dependency because it appears in its resolved
  closure, so `sparq-mcp` must *declare* whichever generator it uses — concretely
  `getrandom = { version = "0.3", optional = true }` in `[dependencies]`, added to the
  feature as `http = [..., "dep:getrandom"]` (the alternative, `uuid = { version = "1",
  features = ["v4"], optional = true }`, is heavier for 128 random bits and would pull
  `getrandom` anyway). The narrower claim that **is** true, and is the one that matters
  for the supply-chain gate: **no new package or version enters `Cargo.lock`, so there is
  no new audit target or exemption.** `getrandom` and `rand` are already in `sparq-mcp`'s
  measured native closure via `oxrdf`'s blank-node ids — the same edge
  `crates/sparq-introspect/Cargo.toml:69-73` documents and pins a wasm backend for — and
  `uuid` v4 via `sparq-engine` (`crates/sparq-engine/Cargo.toml:472-473` gates it to
  `cfg(not(target_arch = "wasm32"))`, which is where an HTTP server runs anyway).
- **The session id is a bearer credential.** It must never be logged, and 2025-11-25
  explicitly points at session-hijacking mitigations. Follow the existing
  `crates/sparq-server/src/redact.rs` posture rather than inventing one.
- **The replay log must be bounded** (per stream, by event count and by bytes). An
  unbounded resumability buffer is a memory-exhaustion surface reachable by any client
  that opens a stream and never reads it. 2025-11-25 additionally *recommends* an
  immediate priming event carrying an id, plus a `retry` field before a
  server-initiated connection close so the client polls rather than reconnecting tightly —
  both cheap, both worth doing.

### Security: the part that actually needs review

Everything above is mechanical. This is not. Ordered by how badly it goes wrong if skipped:

1. **`allow_update` + HTTP must be fail-closed.** Today `--allow-update` is safe-by-context:
   the operator handed someone a pipe. On a socket it is a remote-write surface. The
   binary must **refuse to start** with writes enabled on a non-loopback bind unless a
   Bearer token is configured — modelled on `bind_posture`'s `RemoteRefused`, which already
   refuses a non-loopback bind unless the whole surface is authenticated or the operator
   opts in explicitly.
2. **Origin validation → `403`** (a spec MUST; 2025-11-25 makes the status explicit). Default
   allowlist empty. An **absent** `Origin` is allowed — non-browser MCP clients do not send
   one — but any *present* un-allowlisted `Origin` is refused. No wildcard, ever; mirror
   `cors_config.rs`.
3. **Loopback by default** (spec SHOULD), enforced through the same fail-closed
   classification as `bind_posture`, including treating `0.0.0.0` / `::` as remote.
4. **Bearer token gate** (spec SHOULD), constant-time comparison, checked **before** an SSE
   stream opens — the ordering `subscriptions.rs:742` already documents. OAuth 2.1 /
   protected-resource-metadata is explicitly *not* in scope here.
5. **The README trust model must be rewritten, not extended.** "The stdio transport is a
   trust boundary you establish" is true and becomes misleading the moment an HTTP feature
   exists. The `http` feature needs its own honest paragraph.

There is also a documentation-honesty trap worth naming: `SUPPORTED_PROTOCOL_VERSIONS`
claims `2024-11-05`. That claim is about the *protocol revision*, not the transport, and
stays correct — but a 2024-11-05-era client that only speaks the deprecated HTTP+SSE
transport still cannot reach a Streamable-HTTP-only endpoint. The docs must say so
explicitly, or the version list reads as a compatibility promise the endpoint does not
keep.

## Deliberately out of scope

- **The deprecated 2024-11-05 HTTP+SSE transport.** No named client requires it. Revisit on
  evidence.
- **`SolidMcpServer` over HTTP.** `SolidServerConfig` fixes ONE authenticated
  Solid-OIDC-derived session at construction (`crates/sparq-mcp/src/solid.rs:159`), so
  per-HTTP-session pod identity means many `SolidMcpServer`s over a shared `PodStore` plus
  a token→session mapping. That is a genuine authorization design, not a transport detail,
  and it must not be smuggled in behind an `http` flag.
- **Delivery of `SolidMcpServer`'s notification queue.** It is *not* served by phases 1–5:
  those wrap the base `McpServer`, and being in the same process does not let a transport
  over one type drain a queue held by another. What phase 4 ships is the channel and the
  notification-source seam; wiring `take_notifications` into that seam belongs to the
  Solid-over-HTTP bead above, because it is that design which decides *which* pod session's
  notifications an HTTP session is entitled to see — an authorization question, not a
  plumbing one. Building the channel first is still the right order; it is just not
  delivery.
- **OAuth 2.1 / MCP authorization framework.** A separate bead.
- **Cross-process or persisted session state.** In-memory only; a restart invalidates
  sessions, and the spec already defines `404` → re-`initialize` for exactly that.
- **TLS termination.** Delegate to a reverse proxy, as `sparq-server` does.

## Conformance obligations, as a test list

Each row is a spec MUST or a security invariant, and each should land as an assertion, not
prose. The wire-byte-level pattern to copy is `crates/sparq-server/tests/subscriptions_sse.rs`.

| obligation | test |
| --- | --- |
| POST notification ⇒ `202`, empty body | round-trip over a bound ephemeral port |
| POST request ⇒ single JSON response, correct `id` | same |
| `initialize` ⇒ `Mcp-Session-Id` present, visible-ASCII only | header assertion |
| non-`initialize` without session id ⇒ `400` | negative |
| unknown/expired session id ⇒ `404` | negative |
| `DELETE` ⇒ session gone, next request `404` | sequence |
| present, un-allowlisted `Origin` ⇒ `403`; absent `Origin` ⇒ allowed | two cases |
| unsupported `MCP-Protocol-Version` ⇒ `400`; absent ⇒ treated as `2025-03-26` | two cases |
| `GET` stream delivers a notification queued through the phase-4 notification-source seam (a test double, since the base server emits none); `id:` present and monotonic | raw-byte read |
| `Last-Event-ID` replays only that stream's missed events | raw-byte read |
| non-loopback bind + `allow_update` + no token ⇒ **refuses to start** | unit test on the posture fn, no bind |
| a stream dropped by the client releases its state | drop-observability assertion |
| default build (`--no-default-features`) still resolves 61 packages and compiles without a runtime | feature-discipline check |

## Phased plan (each phase is a future bead)

1. **`http` feature scaffold + endpoint skeleton.** Opt-in feature; `axum`/`tokio`/`futures-util`
   as optional deps; `POST` for requests and notifications (`application/json` + `202`) over
   `Arc<Mutex<McpServer>>` + `spawn_blocking`. No sessions, no SSE. Gate: clippy `-D warnings`
   and tests green in **both** feature states; default closure unchanged.
2. **Security posture.** Origin allowlist → `403`; loopback-default bind classification ported
   from `bind_posture`; Bearer gate with constant-time compare; the fail-closed
   `allow_update`-on-remote-bind refusal. Ships with phase 1's endpoint or immediately after —
   **not later**, so no intermediate commit exposes an ungated socket.
3. **Session management.** `Mcp-Session-Id` issuance at `initialize` — which is where the
   `dep:getrandom` optional dependency lands, per [§ Session state](#session-state) — the
   session table, idle expiry, `400`/`404` semantics, `DELETE`, `MCP-Protocol-Version`
   validation.
4. **Server-initiated `GET` SSE stream + the notification-source seam.** `axum` `Sse` +
   `unfold`, keep-alive, `Event::id()`, draining `SessionState::outbound`. Because the base
   `McpServer` produces no notifications, this phase must also define *where* `outbound` is
   fed from, and that is its main design content: a small crate-internal seam — a trait with
   one "take the messages queued since last call" method, owned by the transport module and
   NOT added to `McpServer`'s public surface — with exactly one implementor in this phase, a
   test double. That keeps the stream testable end-to-end while leaving the real producer
   (`SolidMcpServer::take_notifications`) to the Solid-over-HTTP bead, which owns the
   token→pod-session mapping that decides who may see those notifications. This phase
   therefore ships **a delivery channel, not delivered notifications** — the PR body must
   say so, since the queue stays undeliverable in-tree until that separate bead lands.
5. **Resumability.** Bounded per-stream event log, `Last-Event-ID` replay with the
   "never replay another stream's events" invariant, the 2025-11-25 priming event and `retry`
   field.
6. **Binary + docs.** `--http`, `--bind`, `--allow-origin`, `--auth-token` on the `sparq-mcp`
   binary; the rewritten README trust model; `skills/agent-tools/SKILL.md` updated (its
   front-matter `description` currently says the transport surface is "the optional stdio
   feature"); a note in `jsonrpc.rs` that trigger 1 fired and where it was re-assessed.
7. **Read/write concurrency split.** *Prerequisite:* make the lazy `text_index` interior-mutable
   so read tools no longer need `&mut self`; then `RwLock` so reads run concurrently. Purely a
   throughput change — it must not alter any result, and it needs the phase-1 tests already
   green to be worth reviewing.
8. **Interop check against a real client.** One end-to-end handshake against an
   off-the-shelf MCP client over HTTP. Spec conformance and *client* conformance are not the
   same thing, and only this phase can tell them apart.

Phases 1–3 are the minimum shippable unit: an endpoint without phase 2 is a vulnerability,
and without phase 3 it is not a spec-conformant Streamable HTTP server.

## Open questions for the maintainer

1. **Deprecated HTTP+SSE transport — yes or no?** This record recommends no. A yes roughly
   doubles the endpoint surface and the test matrix for clients nobody has named.
2. **`rmcp` for the MCP-specific part only?** The measurement says hand-rolling on the
   in-tree axum stack costs no audit delta, but `rmcp` genuinely does own the session +
   resumability logic. If the answer is "adopt `rmcp` for the transport under a separate
   opt-in feature", that is sq-95zda's own recommended shape and it should be decided here,
   before phase 1 — not discovered during phase 5.
3. **Where does the endpoint live — `sparq-mcp` or `sparq-server`?** This record assumes
   `sparq-mcp` behind an opt-in feature, keeping the crate self-contained. Mounting `/mcp`
   as a route on the existing `sparq-server` router instead would reuse its auth, CORS,
   bind-posture, tracing and slow-loris hardening *directly* rather than by imitation, at
   the cost of making the MCP server depend on the full HTTP server. Genuinely arguable;
   the answer changes phases 1–2 substantially.
4. **Is `allow_update` over HTTP permitted at all**, even loopback + token? A defensible
   stricter position is that network writes require the Solid/WAC authorization path, never
   a single shared secret.

## Uncertainties

- Package counts are **name-set** deltas from `cargo tree`, not compile-time or
  binary-size measurements. No timing or size figure is claimed anywhere in this record.
- The claim that the +29 crates need no new audit rests on their already being in the
  gated `Cargo.lock` at the resolved versions. That is sound by construction *if* the vet
  gate is green on `main`; the implementing PR must still run the real gate rather than
  trust this paragraph.
- Feature resolution was measured for `axum` 0.8 with `default-features = false, features =
  ["http1", "tokio", "json"]`. That axum's SSE types are reachable under that feature set is
  inferred from `sparq-server`, which uses `axum::response::sse` while enabling
  `["http1","tokio","query","json","ws"]` and no separate SSE feature — verify at
  implementation time rather than taking it from here.
- The `rmcp` rows are copied from sq-95zda (2026-07-28) and were **not** re-measured today.
  They are one day old; `rmcp`'s release cadence, recorded there as three majors in five
  months, means they should be re-derived before anyone acts on option 2 above.
- Spec text was read at the two URLs cited on 2026-07-29. The 2025-11-25 revision is the
  newest at that date; a later revision could move these MUSTs.

## Reproduction

```sh
# baseline closure (measured: 61 packages)
cargo tree -p sparq-mcp --no-default-features -e normal --prefix none \
  | sed 's/ (\/.*//; s/ (\*)//' | awk '{print $1}' | sort -u

# the axum-route closure: temporarily add to crates/sparq-mcp/Cargo.toml
#   [features] http = ["dep:axum", "dep:tokio", "dep:futures-util"]
#   [dependencies]
#   axum        = { version = "0.8", optional = true, default-features = false, \
#                   features = ["http1", "tokio", "json"] }
#   tokio       = { version = "1", optional = true, default-features = false, \
#                   features = ["rt-multi-thread", "macros", "net", "time", "sync", "io-util"] }
#   futures-util = { version = "0.3", optional = true, default-features = false }
# then re-run the command above with `--features http,stdio` (measured: 90), diff the
# two name sets (+29), diff that against `grep '^name = ' Cargo.lock` (0 new), and REVERT.
```

## Follow-ups (beads/issues, not TODOs)

- `research/mcp-rmcp-sdk-adoption-assessment.md` should be annotated: trigger 1 has fired
  and was re-assessed in this record; the verdict held, for a reason that record did not
  have (the in-tree axum SSE stack). Triggers 2–4 are untouched. `crates/sparq-mcp/src/jsonrpc.rs:15`
  says "an HTTP/SSE transport is the strongest one" and should point here.
- `SolidMcpServer` over a multi-session transport is its own bead — a `PodStore`-sharing,
  token→Solid-session authorization design, explicitly not a transport change. It also owns
  the *last* step of notification delivery: implementing phase 4's notification-source seam
  over `take_notifications`, scoped to whatever pod session the HTTP session is entitled to.
- The lazy `text_index` on the read path (`text_search` needs `&mut self`) blocks any
  read/write concurrency split. Worth capturing independently of this transport, since it
  constrains every future concurrent embedder.
- `take_notifications` currently has no in-tree consumer outside a test, and **still will
  not have one when phases 1–8 are done** — see the phase-4 seam and the out-of-scope entry.
  Whatever happens to this bead, that is a shipped-but-undeliverable surface and should be
  recorded as such, tracked against the Solid bead above rather than this transport.
