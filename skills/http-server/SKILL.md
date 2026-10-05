---
name: http-server
description: Run or point an agent at a sparq SPARQL 1.1 Protocol HTTP endpoint (sparq-server) — /sparql query+update over GET/POST (plus the query-only HTTP QUERY method, w3c/sparql-protocol#40, for Oxigraph interop), content negotiation (SELECT/ASK JSON/XML/CSV/TSV; CONSTRUCT/DESCRIBE + Graph Store N-Triples/prefix-Turtle/RDF-XML/JSON-LD — JSON-LD via the default-on jsonld feature), Graph Store read AND write (PUT/POST/DELETE/PATCH on graph resources, RDF/XML + default-on JSON-LD bodies accepted, atomic SPARQL-Update + opt-in Solid N3-Patch on PATCH), EXPLAIN, Prometheus /metrics, WebSocket + SSE subscriptions, opt-in grouped facet counts and prefix completion, and generation-pinned snapshot reads (a Sparq-Generation header + ?generation=N pin in the DEFAULT build, bounded to the ring's concurrency-retention window; opt-in time-travel widens it). Use when starting the server, querying/updating a running endpoint, choosing Accept/Content-Type, or embedding the axum router.
---

# sparq-http-server

`sparq-server` is a W3C-conformant HTTP server (axum/tokio) that exposes the sparq
query engine over a `sparq_core::Graph` — in-memory by default, or **durable on disk**
with `--persist DIR` (updates WAL-fsync'd, survive a restart with no rebuild; see the
"Durability" gotcha). It implements the **SPARQL 1.1 Protocol** (`query` + `update` at
`/sparql`) and the **Graph Store HTTP Protocol** (read + write), with `Accept`-driven
content negotiation, hardening guards, Prometheus `/metrics`, WebSocket + SSE subscriptions,
and opt-in grouped facet counts, prefix completion, time-travel, and GeoSPARQL.

Build with the default-off `response-compression` feature to negotiate gzip or zstd for outbound
result bodies from `Accept-Encoding`. Compression is transport-transparent and streaming; SSE
subscription responses and bodies that already have a `Content-Encoding` are left uncompressed:

```sh
cargo run -p sparq-server --features response-compression
curl --compressed -G http://127.0.0.1:3030/sparql \
  --data-urlencode 'query=SELECT * WHERE { ?s ?p ?o }'
```

## Quickstart

Run the binary (server stack is the default-on `server` feature):

```sh
# serve a Turtle file on the default address 127.0.0.1:3030 (loopback — safe default)
cargo run -p sparq-server -- --format turtle data.ttl
# no data file => empty default graph (still answers queries, just no rows)
cargo run -p sparq-server
# custom bind addr / format (turtle | ntriples | nquads | trig). A NON-loopback bind
# (e.g. 0.0.0.0) is REFUSED unless --allow-remote / SPARQ_ALLOW_REMOTE=1 (no auth — see below).
cargo run -p sparq-server -- --addr 0.0.0.0:8080 --allow-remote --format ntriples data.nt
```

Opt-in HTTP/2 switches the TCP listener from its default HTTP/1.1-only builder to the
hyper-util h1+h2 auto builder. With no certificate flags it accepts HTTP/1.1 and cleartext h2c;
supplying both PEM paths enables TLS 1.3 with ALPN `h2,http/1.1`:

```sh
cargo run -p sparq-server --features http2 -- data.ttl \
  --tls-cert ./cert.pem --tls-key ./key.pem
```

The PEM flags are a pair: supplying only one fails startup. Omitting both preserves plain HTTP.
The HTTP/1 slow-loris header deadline, body idle deadline, peer `ConnectInfo`, graceful drain, and
WebSocket upgrade remain on the same bespoke serve path; HTTP/2 uses the same router and request
middleware. [GPT-5.6] sq-oprna.6

Opt-in HTTP/3 adds an encrypted QUIC/UDP listener beside the unchanged plain-HTTP TCP
listener. The two listeners dispatch through the same router; the UDP address defaults to
`--addr` and can be overridden independently:

```sh
cargo run -p sparq-server --features http3 -- data.ttl \
  --http3 --http3-addr 127.0.0.1:3443 \
  --tls-cert ./cert.pem --tls-key ./key.pem
```

Both PEM paths are required when `--http3` is set, and malformed or mismatched material
fails startup. QUIC is mandatorily encrypted. With `http3` alone the TCP listener remains plain
HTTP for proxy/backward compatibility; with `--features http2,http3` the same PEM pair also
secures TCP and negotiates h2/h1 through ALPN. The pre-1.0 h3 stack is contained behind the
default-off feature. `/subscriptions` WebSockets remain HTTP/1.1-only; clients must fall back because
HTTP/3 extended CONNECT is not implemented. Once the UDP endpoint has bound, every plain-HTTP TCP
response advertises its live port as `Alt-Svc: h3=":<port>"; ma=86400`; no header is emitted when
`--http3` is absent or startup cannot bind the QUIC listener. [GPT-5.6] sq-oprna.4

> **Security: optional Bearer-token write gate; loopback by default.** With no token
> configured, every endpoint is unauthenticated (the back-compat default). Set
> `--auth-token <TOKEN>` (env `SPARQ_AUTH_TOKEN`) to require `Authorization: Bearer <TOKEN>`
> on every **write** (a SPARQL Update on `/sparql` — `application/sparql-update`, or a
> `query`/`update` body that parses as an update — and the GSP `PUT`/`POST`/`DELETE`
> methods); otherwise `401` with `WWW-Authenticate: Bearer` (constant-time compared; mirrors
> QLever's `-a <token>`). Add `--auth-token-read` (env `SPARQ_AUTH_TOKEN_READ=1`) to ALSO
> gate reads. The subscription transports — the `/subscriptions` WebSocket and the
> `/subscriptions/sse` SSE stream (both a *read* surface) — are gated by `--auth-token-read`
> too (bead `sq-cxk5`, closing the prior read-auth bypass): the SSE GET takes the
> `Authorization: Bearer` header; the WS upgrade accepts that header OR (for browsers, which
> cannot set headers on a WS handshake) a `Sec-WebSocket-Protocol: bearer.<token>` subprotocol.
> The Prometheus `GET /metrics` endpoint is gated by `--auth-token-read` as well (bead
> `sq-9jrx`): its gauges leak the live triple count and active-subscription count, so it is a
> read; `/health` stays ungated for liveness probes.
> With no read gate, both are open (back-compatible). The server binds **loopback by default** and **refuses a non-loopback
> bind** (e.g. `0.0.0.0`) unless you set `--allow-remote` (env `SPARQ_ALLOW_REMOTE=1`) OR the
> whole surface is authenticated (`--auth-token` AND `--auth-token-read`) — a write-token
> alone still leaves reads open. Deliver the token over TLS (terminate it at a proxy). For
> real per-user authz, front it with a reverse proxy / API gateway (or `sparq-solid`). SPARQL
> `SERVICE` federation is OFF
> in the default build (the `service` cargo feature is off); a `SERVICE` clause then
> errors at execution. Build with `--features service` to enable it — and even then the
> server is **default-DENY-all SERVICE**: a `SERVICE <iri>` reaches **nothing** unless its
> host is on the egress allowlist (`--service-allow` / `--service-allow-file` /
> `SPARQ_SERVICE_ALLOW`; bead `sq-4w18`). This is an SSRF guard: a `SERVICE` clause turns
> attacker-controlled query text into an outbound request from the server host (worst case
> the `169.254.169.254` cloud-metadata IP). The allowlist is enforced before any socket is
> opened, on the *resolved* IP (DNS-rebinding-safe). See "SERVICE federation (egress
> allowlist)" below.
>
> **Security response headers (always on, ASVS V14.4 / ASVS-G1; beads `sq-cmvh`, `sq-2bhm`).**
> Every response — success, streamed, error and auth-gated (`401`) alike — carries a hardening
> header set, stamped by a `map_response` layer in `harden()`:
> `X-Content-Type-Options: nosniff`,
> `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`,
> `X-Frame-Options: DENY`, and `Referrer-Policy: no-referrer`. These suit a SPARQL *data* API
> (no HTML is rendered): the tightest CSP says the body loads/runs nothing, and `frame-ancestors`
> + `X-Frame-Options: DENY` say it is never meant to be framed. Each header is only added when
> absent, so a custom handler can override one. **Deliberately omitted:** `Strict-Transport-
> Security` (the origin serves plain HTTP — HSTS belongs on the fronting TLS proxy);
> `X-XSS-Protection` (deprecated, superseded by CSP); `Cross-Origin-*` / `Permissions-Policy`
> (browser-app document policies, meaningless for a data API). **CORS is OFF by default** (no
> `Access-Control-*` header — a cross-origin browser read is blocked, the safe posture) but is an
> opt-in **first-party origin allowlist** (`sq-o7o0`; see "CORS" below). No *blanket*
> `Cache-Control: no-store` is forced: results are uncached by
> default (no `ETag`/`Cache-Control: public` is ever set), so there is nothing to tighten, and a
> blanket value would wrongly override `/health` / `/metrics` — but the sensitive auth-refusal
> (`401` from `unauthorized()`) **does** carry `Cache-Control: no-store` so a shared cache never
> retains it (`sq-2bhm`).
>
> **Error bodies carry a generic class, never internals (ASVS V7 / ASVS-G3; beads `sq-cz89`,
> `sq-j9zs`, [OPUS-4.8] `sq-kfel`).** Every error is the structured `{"error":"<msg>"}` envelope
> where `<msg>` is a STABLE generic category (malformed-query / auth / not-found / server-error) —
> never the caller's input, a loaded-RDF fragment, a server filesystem path, a secret, or a
> `Debug` of an internal type. The full detail goes to the server log under
> `target: "sparq_server"` (gated behind `--verbose` / `RUST_LOG`), not the response body. All
> sensitive error paths funnel through one `sanitized_error` helper; regression-guarded by
> `tests/hardening.rs` (`no_echo_*` + `FORBIDDEN_INTERNALS`) and `tests/tpf.rs`.

Point a client at it (the endpoint is `/sparql`):

```sh
# GET, query as URL param (URL-encoded); default result media is SPARQL-JSON
curl -G http://127.0.0.1:3030/sparql --data-urlencode 'query=SELECT * WHERE { ?s ?p ?o } LIMIT 5'

# POST direct: body IS the query
curl http://127.0.0.1:3030/sparql -H 'Content-Type: application/sparql-query' \
     --data 'ASK { ?s ?p ?o }'

# HTTP QUERY method (sq-b3df9, w3c/sparql-protocol#40) — query-only, interoperates with
# Oxigraph: body IS the query under Content-Type application/sparql-query; graph params
# (default-graph-uri / named-graph-uri) ride the URL query string. Same downstream query
# execution + Accept negotiation as POST; an update= form or application/sparql-update body
# is rejected (400 / 415).
curl -X QUERY http://127.0.0.1:3030/sparql -H 'Content-Type: application/sparql-query' \
     -H 'Accept: application/sparql-results+json' --data 'SELECT * WHERE { ?s ?p ?o } LIMIT 5'

# POST url-encoded form, negotiate CSV
curl http://127.0.0.1:3030/sparql -H 'Accept: text/csv' \
     --data-urlencode 'query=SELECT * WHERE { ?s ?p ?o }'

# SPARQL Update -> 204 No Content (atomic; failure -> 400, no partial effect)
curl -i http://127.0.0.1:3030/sparql -H 'Content-Type: application/sparql-update' \
     --data 'INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }'
```

Embed the router in your own tokio app (library use):

```rust
use sparq_core::Graph;
use sparq_server::{router, AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = Graph::load_str(include_str!("data.ttl"), "turtle")?;
    let app = router(AppState::new(graph));          // axum::Router
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3030").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

## Key APIs

Library surface re-exported from `sparq_server` (behind the default `server` feature):

- `fn router(state: AppState) -> axum::Router` — builds the hardened endpoint router
  (`/sparql`, `/sparql/graph`, `/graphs/*path`, `/subscriptions`, `/subscriptions/sse`,
  `/health`, `/metrics`, plus feature-gated routes such as `/facets` and `/complete`). [OPUS-4.8] sq-bxog:
  `/subscriptions/sse` is the SSE transport. [GPT-5.6] sq-lsp7k.5.2: `/facets` needs the
  `facets` build feature and the runtime `ServerConfig::facets` flag. [GPT-5.6] sq-lsp7k.9.3:
  `/complete` similarly needs the `complete` feature and `ServerConfig::complete`.
- `AppState::new(graph: Graph) -> AppState` — default `ServerConfig`.
- `AppState::with_config(graph: Graph, config: ServerConfig) -> AppState`.
- `AppState::current(&self) -> PinnedGen` — lock-free pin of the current immutable
  generation snapshot (`PinnedGen = Arc<sparq_serve::Generation<Graph>>`; call
  `.snapshot() -> &Graph`, `.number() -> u64`, `.published_at()`, `.epochs()`).
- `AppState::apply_update(&self, sparql: &str) -> Result<u64, String>` — submit a SPARQL
  Update through the sequenced writer; **blocks** until the containing generation is
  published; returns that generation number (read-your-writes token). Call off the async
  workers (`spawn_blocking`). With **adaptive group-commit** (default; `adaptive_commit`),
  a serial interactive client commits in engine-time (µs) — the writer drains only the
  already-queued backlog and commits, instead of paying a fixed group-commit window;
  concurrent load still batches. FIFO order, per-update atomicity and durability are
  unchanged (`sq-p7kk5`).
- `AppState::at(&self, number: u64) -> Option<PinnedGen>` — pin a retained generation
  (**`time-travel` feature only**; the HTTP `?generation=N` pin does NOT need this — it
  resolves against the ring's concurrency-retention window directly, so it works in the
  default build via `sq-ci2d6`).
- `struct ServerConfig { query_timeout: Option<Duration>, update_where_timeout: Option<Duration>,
  adaptive_commit: bool,
  max_body_bytes: usize,
  max_concurrent: usize, header_read_timeout: Option<Duration>, body_read_timeout: Option<Duration>, max_results: Option<usize>, max_query_rows: Option<usize>,
  max_query_bytes: Option<usize>,
  max_decompress_ratio: usize, max_subscriptions: usize, max_subscriptions_per_conn: usize,
  verbose: bool, redact_logs: bool, allow_remote: bool, auth_token: Option<String>, auth_token_read: bool,
  service_allow: ServiceAllowlist, /* + time_travel_* / facets / complete under their features, + audit_log under audit-log feature */ }` with
  `ServerConfig::default()` and `ServerConfig::from_env()`.
  (`update_where_timeout` = separate, typically-SHORTER writer-side WHERE deadline for SPARQL
  UPDATE that bounds writer-queue **head-of-line blocking** from a slow update — `None` =
  use `query_timeout`, `sq-nulp`;
  `adaptive_commit` = **adaptive group-commit** (default `true`): a serial interactive client
  commits in engine-time (µs) rather than paying a fixed group-commit window; concurrent load
  still batches. Pure latency change — FIFO/atomicity/durability unchanged. `false` = always
  windowed. `--no-adaptive-commit` / `SPARQ_ADAPTIVE_COMMIT=0` — `sq-p7kk5`;
  `max_query_rows` = coarse memory cap; `max_query_bytes` =
  byte-accounted memory cap that also prices row WIDTH + computed-literal size — `sq-s5is`;
  `max_decompress_ratio` = zip-bomb guard — `sq-ebii`;
  `header_read_timeout` = **slow-loris guard**: max time a connection may take to send its
  complete request-header block, enforced at hyper's HTTP/1 connection layer by
  `sparq_server::serve` — `None` disables; default 15s — `sq-2gqr`;
  `body_read_timeout` = **slow-body guard** (the complement to `header_read_timeout`): an idle
  deadline between consecutive request-**body** reads, applied by `sparq_server::serve` via a
  `tower_http` `RequestBodyTimeoutLayer`/`TimeoutBody` (the timer resets after each chunk, so an
  honest large upload is not penalised — only a stall is) — closes the slow-**body** dribble a
  complete-header client could otherwise use; `None` disables; default 30s — `sq-lodb`.)
  `auth_token` (set: gates the write surface with a Bearer token, constant-time compared;
  `None`: no write auth) and `auth_token_read` (gate reads too) are honoured by the library
  `router` itself — embedders get the gate for free (`sq-zcby`).
- `fn bind_posture(addr: &SocketAddr, allow_remote: bool, auth: AuthPosture) -> BindPosture`
  — the bind gate the binary applies: `Loopback` (proceed), `RemoteAllowed { warning }`
  (proceed + log), or `RemoteRefused { message }` (refuse). `AuthPosture::{None, WriteOnly,
  ReadAndWrite}` (via `AuthPosture::from_config(&config)`) folds the token + read-gate into
  the decision: a non-loopback bind is allowed when `--allow-remote` is set OR the surface is
  fully authenticated (`ReadAndWrite`); a write-token alone (`WriteOnly`) still requires
  `--allow-remote` because reads stay open. This is a *bind-time* posture gate; per-request
  auth is the `auth_token` fields above (enforced by `router`/`harden`-wrapped handlers).
- `fn harden(routes: axum::Router, config: &ServerConfig) -> axum::Router` — wrap any
  router in the production middleware (panic→500, concurrency-limit→429, body-limit→413,
  JSON error bodies, optional trace).
- `async fn serve(listener: tokio::net::TcpListener, app: axum::Router, header_read_timeout:
  Option<Duration>, body_read_timeout: Option<Duration>, shutdown: impl Future<Output=()>) ->
  std::io::Result<()>` — the accept + graceful-drain loop the binary runs **instead of
  `axum::serve`** (`sq-2gqr` / `sq-lodb`). It is a faithful port of axum's own loop (per-connection
  task, watch-channel drain, `with_upgrades()` so the `/subscriptions` WebSocket still works) with
  two additions: (1) it installs a `hyper_util` TokioTimer and hyper's HTTP/1
  `header_read_timeout` (the slow-loris HEADER guard); (2) it optionally wraps the request body in a
  `tower_http` `RequestBodyTimeoutLayer` keyed on `body_read_timeout` (the slow-BODY guard — applied
  via `tower::util::option_layer`, so `None` is a true no-op `Identity` layer). **Why:** `axum::serve`
  never installs a timer, so hyper's header-read deadline is inert there and a slow-loris client
  holds a connection (and a `concurrency_limit` slot) open indefinitely; and even with that fixed, a
  complete-header client can still dribble the BODY to hold the slot — `body_read_timeout` closes
  that complementary hole. Pass `None` on either to opt back out of that guard. With the
  default-off `http2` feature this same function uses hyper-util's h1+h2 auto builder (h2c on a
  plain listener) and retains the timer on the builder's HTTP/1 configuration.
- `async fn serve_tls(listener: tokio::net::TcpListener, app: axum::Router, tls_config:
  Arc<rustls::ServerConfig>, header_read_timeout: Option<Duration>, body_read_timeout:
  Option<Duration>, shutdown: impl Future<Output=()>) -> std::io::Result<()>` (**`http2` feature
  only**) — TLS counterpart to `serve`; the binary supplies a TLS-1.3 config advertising
  `h2,http/1.1` via ALPN. It shares the auto-builder connection body, middleware, timeout hooks,
  peer `ConnectInfo`, and graceful drain with the cleartext feature-on path. [GPT-5.6] sq-oprna.6
- Re-exports for cache layers/tests: `PinnedGen`, `GLOBAL_POD: &str`
  (`"urn:sparq:pod:global"`), and `sparq_serve::{Epoch, PodEpochs, PodId}`.
- **Response-bytes result cache** (opt-in, `sparq-serve`'s `result-cache` feature,
  OFF by default — [OPUS-4.8] sq-jluc). A serving-layer cache from a request
  *identity* to the pre-serialized response body, distinct from `sparq-engine`'s
  in-engine algebra-keyed `result-cache`. Public surface (feature-gated):
  `sparq_serve::{ResultCache, CacheConfig, ScopeKey, ReadFootprint, LeaseOutcome,
  CacheStats}`. **Key = canonical-query × visibility-scope × per-pod epoch-vector**
  (`ResultCache::lease`/`get`): the **scope** is the identity of the accessible
  graph set (`ScopeKey::of_graphs(AuthIndex::accessible(session, mode))`), **never**
  the WebID — bytes cached for one access scope can never be served to another (an
  access-control correctness invariant, *not* a privacy guarantee). Invalidation is
  per-pod epoch bumps (`ReadFootprint::Pods`) or the global generation
  (`ReadFootprint::Unbounded`); single-flight leases dedup a stampede; a byte-budget
  LRU + admission bound keeps it from caching oversize/streaming bodies. Wiring it
  into the axum endpoint and the canonical perf-validation are follow-ups (the perf
  targets need a canonical host).
- Serializer/negotiation helpers (always compiled, no `server` feature): module
  `sparq_server::negotiate` — `fn negotiate(accept: Option<&str>) -> Format` (lenient, always
  yields a format), `fn negotiate_or_406(accept: Option<&str>) -> Result<Format, NotAcceptable>`
  (Oxigraph-parity strict — the query path uses this; a present-but-unsatisfiable `Accept` is
  `Err(NotAcceptable)` → 406), `fn negotiate_graph(accept: Option<&str>) -> GraphFormat` and its
  strict sibling `fn negotiate_graph_or_406(...) -> Result<GraphFormat, NotAcceptable>`; module
  `sparq_server::exec` — `fn prepare(&str) -> Result<Prepared, PrepareError>` and
  `fn prepare_with_dataset(&str, &DatasetOverride) -> Result<Prepared, PrepareError>` (applies the
  SPARQL-Protocol `default-graph-uri`/`named-graph-uri` override, sq-z33x),
  `fn apply_update_dataset(&str, &UsingOverride) -> Result<String, UpdateDatasetError>` (the
  update-side `using-*` override), `enum QueryForm { Select, Ask, Construct, Describe }`; module
  `sparq_server::results`.

### In-process embedding seam — `sparq_serve::embed` ([OPUS-4.8] sq-xa15c, #1248)

When the consumer is **another Rust process** (e.g. `solid-server-rs`) and wants to
drop the HTTP hop entirely, embed the engine in-process via the `sparq_serve::embed`
facade instead of running `sparq-server` and talking to it over HTTP. It is the
documented **embedding seam**: thin wrappers over the engine entry points plus a
re-export of the runtime-agnostic concurrency wrapper. No axum/tokio, no HTTP.

- Data path (over one `&Graph` / `&mut Graph`): `embed::query_json(&Graph, sparql)
  -> Result<String, _>` (SPARQL-JSON), `embed::query(&Graph, sparql) ->
  Result<QueryResult, _>`, `embed::ask(&Graph, sparql) -> Result<bool, _>`,
  `embed::update_in_place(&mut Graph, sparql)` (+ `..._atomic` for all-or-nothing on
  one graph), `embed::apply_delta_nquads(&mut Graph, inserts, deletes)` (quad-level,
  per-graph; blank nodes by label), and the probes `embed::exists(&Graph) -> bool`,
  `embed::named_graph_exists(&Graph, &Term) -> bool`, `embed::metadata(&Graph) ->
  Metadata { triples, named_graphs }`. `query_json_with_budget` takes a `QueryBudget`.
- Concurrency wrapper (re-exported from the crate root): `GenerationRing` /
  `Generation` / `GraphApplier` / `Writer` (+ `RingConfig`, `WriterConfig`, `PodId`,
  `TimeTravelConfig`) — the SAME `fork → update → publish` + generation-pinning model
  `sparq-server` wraps behind its endpoint. A reader `ring.current()` pins an
  immutable snapshot; the writer publishes new generations without blocking readers.
- **Stability — API tier-1 (proposed-stable):** the proposed semver-stable embedding
  surface in the [API stability & deprecation policy](../../docs/api-stability.md), but
  **NOT yet frozen** — the formal freeze is the maintainer's to ratify on #1248 / #1346
  (pre-`1.0` minor releases MAY still change it). Pin to `sparq_serve::embed` rather
  than reaching into `sparq-core` / `sparq-engine` directly so the freeze, when
  ratified, has one well-defined shape.

## Common recipes

**1. Query forms and result negotiation.** Default result media is SPARQL-JSON. Set
`Accept` to choose (q-value aware, defaults to JSON for SELECT/ASK, N-Triples for
CONSTRUCT/DESCRIBE). <!-- [OPUS-4.8] sq-406acc --> An **absent / empty / `*/*` `Accept` gets
that default**; a present `Accept` that names **only unsupported media types and no wildcard is
`406 Not Acceptable`** (Oxigraph parity, w3c/sparql-protocol#40) — sparq no longer silently
falls back to JSON in that case (the EXPLAIN `Accept: text/x-sparq-explain` short-circuits this
and the Graph-Store-Protocol read path keeps its lenient default):

| Query form | Accept | Content-Type returned |
| --- | --- | --- |
| SELECT | `application/sparql-results+json` (default) / `+xml` / `text/csv` / `text/tab-separated-values` | matching results media |
| ASK | json (default) / xml | `application/sparql-results+json` / `+xml` |
| CONSTRUCT / DESCRIBE | `application/n-triples` (default) / `text/turtle` / `application/rdf+xml` / `application/ld+json` (the `jsonld` feature — **default-on**) | matching RDF media; N-Triples, prefix-compacting Turtle, RDF/XML, <!-- [OPUS-4.8] sq-rt6v --> or flattened JSON-LD <!-- [OPUS-4.8] sq-oy1f.1/.4 --> |

<!-- [SONNET-4.6] sq-7d3dj.12: CSV/TSV SELECT responses are now streamed as row-oriented
chunks (mirrors the JSON T16 path): Content-Length is known up-front (chunks are fully
evaluated first), the body streams chunk-by-chunk via hyper, and peak memory never holds a
second full-result copy. XML stays buffered (prefix compaction). -->

<!-- [OPUS-4.8] sq-u79ee (survey §C1 / FINDINGS F21) -->
Per the W3C SPARQL Results TSV format, the **TSV** serialiser abbreviates an
`xsd:integer` / `xsd:decimal` / `xsd:double` / `xsd:boolean` literal whose lexical form is
a valid Turtle token to its **bare** token (no quotes, no `^^datatype`) — e.g. `30`, `2.2`,
`1.0E6`, `true`; everything else (incl. integer/decimal *subtypes* like `xsd:negativeInteger`
and custom datatypes) stays quoted + typed, and `xsd:string` keeps the implicit-datatype short
form. The bare token is the literal's **own** lexical form, so a data-sourced literal
round-trips its original spelling (`"1.0E6"^^xsd:double` → `1.0E6`, not canonicalised);
**computed** numerics arrive already in the engine's canonical form, so they serialise
canonically. **CSV** writes each value's bare lexical string (datatype/lang dropped — lossy by
spec); **JSON/XML** carry the full term (value + datatype) unchanged.

<!-- [OPUS-4.8] sq-7d3dj.34.2 -->
**SELECT-JSON streams (TTFB).** The `application/sparql-results+json` SELECT path streams its
body: the engine serialises on a blocking worker feeding a bounded channel, so the results
header + early solutions are written to the socket **before the whole result is serialised**
(first byte before full materialisation, rather than after it). The streamed body is
**byte-identical** to the single materialised JSON string (same solution order, same escaping) —
`query_json_stream_with_budget` shares one emit core with the buffered serialiser. Two response
shapes: a **single-chunk** (small, ≤ one 64 KiB chunk) result is returned buffered with a
`Content-Length`; a **multi-chunk** result streams under **chunked transfer-encoding** with **no
`Content-Length`** (the total is unknown until the last row). *Note the streaming scope:* a
DISTINCT / join / ORDER-BY SELECT must fully evaluate before any solution exists, so its first
byte still lands after the join materialises (the header is flushed the instant that finishes,
before serialisation) — the earliest-first-byte win is largest on scan-shaped and
below-parallel-threshold results; XML/CSV/TSV stay buffered. **Error mid-stream (honest
contract):** the status is chosen from the *first* chunk, so a failure detected before any byte
(parse error, or a row/byte cap / deadline the engine confirms before the header) still returns
the correct `400` / `413` / `503`. But once the header has been flushed for a genuinely
multi-chunk result, the HTTP status is committed: a later cap/deadline trip can only **truncate**
the body — a streamed `200` cannot retroactively become a `413`/`503`.

<!-- [SONNET-4.6] sq-7d3dj.26 -->
**Truncation safety (the invariant).** *A client MUST NOT be able to mistake a truncated stream
for a complete result.* Two mechanisms enforce it, the second strictly on top of the first:

1. **The document is never closed on a truncation.** A complete `sparql-results+json` body ends
   with `]}}`. The server WITHHOLDS the document-closing chunk until the engine confirms it
   produced the whole result, and DROPS it on any mid-stream abort — so a truncated body is not
   valid JSON and any conformant parser errors instead of silently accepting a short result.
   This is the floor guarantee and it needs no client opt-in.
2. **The reason is reported out of band.** A client that sends `TE: trailers` gets a `Trailer`
   response header plus, after the last data chunk, either `X-Sparq-Complete: true` or
   `X-Sparq-Truncated: <reason>` where the reason is `deadline` | `max-rows` | `max-bytes` |
   `cancelled` | `panic` | `error` — the same classification the pre-first-byte path maps to
   `503`/`413`. A client that did NOT negotiate trailers instead sees the chunked stream abort
   **without its terminating zero-length chunk** (a transport error), since hyper would drop a
   trailers frame for it anyway.

A worker that dies mid-result (an engine panic) is treated as a truncation, not a completion:
the completeness claim is only ever made when the engine itself returned success. **A
well-formed, correctly-terminated SHORT `200` is a forbidden outcome** — `--max-results` remains
an honest refusal, never a silent truncation, and hitting it mid-stream drops the closing `]}}`
rather than emitting a clean short document.

<!-- [OPUS-4.8] sq-7d3dj.34.1 -->
**Single parse per request (HTTP floor).** The read path parses each request query with
`spargebra` exactly ONCE — the server parses to classify the form + apply any protocol dataset
override, then hands the resulting algebra straight to the engine's `*_prepared` entry points
(`query_json_stream_prepared_with_budget` for JSON SELECT, `ask_prepared_with_budget` for ASK),
so the engine does not re-parse the query string. This shaves one full parse off the per-request
floor for ARBITRARY novel queries (no query cache — parsing is still paid, just not twice);
negligible on a trivial `ASK{}` (sub-µs), and larger on realistic multi-pattern queries.

```sh
curl -G http://127.0.0.1:3030/sparql -H 'Accept: application/sparql-results+xml' \
     --data-urlencode 'query=SELECT ?s WHERE { ?s ?p ?o }'
# prefix-compacting Turtle:
curl -G http://127.0.0.1:3030/sparql -H 'Accept: text/turtle' \
     --data-urlencode 'query=CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'
# RDF/XML:
curl -G http://127.0.0.1:3030/sparql -H 'Accept: application/rdf+xml' \
     --data-urlencode 'query=CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'
# JSON-LD (flattened) — default-on, works on the standard build: [OPUS-4.8] sq-oy1f.1/.4
curl -G http://127.0.0.1:3030/sparql -H 'Accept: application/ld+json' \
     --data-urlencode 'query=CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'
```

> **HTTP `QUERY` method (sq-b3df9, w3c/sparql-protocol#40, epic sq-my8wd) — query-only,
> Oxigraph-interop.** `/sparql` accepts the registered HTTP `QUERY` verb in addition to
> GET/POST. It is a new INPUT path that feeds the SAME query execution + Accept negotiation as
> POST, so all the result-media rows above apply unchanged. Mirroring Oxigraph
> (`("/sparql", "POST" | "QUERY")`, `let is_query = method == "QUERY"`):
> - **Send the query** as the raw `application/sparql-query` body (preferred), OR as the
>   `query=` value of an `application/x-www-form-urlencoded` body. No other Content-Type is
>   accepted (else **415**); missing Content-Type is **415**.
> - **Dataset graphs** (`default-graph-uri` / `named-graph-uri` / `union-default-graph`) ride
>   the **URL query string**, never the body — even when the query is the raw body.
> - **Query-only:** an `update=` form field is a **400** and an `application/sparql-update`
>   body is a **415** (it is not a valid operation under QUERY). Use POST for an update.
> - It enforces the SAME auth / egress gates as GET/POST (a QUERY is a read — gated by
>   `--auth-token-read`). It does **not** emit `Cache-Control` / `Content-Location` headers
>   (Oxigraph does not either). [OPUS-4.8]
> ```sh
> curl -X QUERY 'http://127.0.0.1:3030/sparql?default-graph-uri=http://ex/g1' \
>      -H 'Content-Type: application/sparql-query' \
>      -H 'Accept: text/csv' \
>      --data 'SELECT * WHERE { ?s ?p ?o }'
> ```

<!-- [OPUS-4.8] sq-jaj38: comment separates the two adjacent blockquotes (markdownlint MD028). -->

> **SPARQL 1.1 Protocol conformance lane (sq-jaj38, epic sq-my8wd) — what is ratcheted.** The
> protocol surface above is now covered by a dedicated W3C **SPARQL 1.1 Protocol (HTTP)**
> conformance suite in `sparq-conformance` (opt-in `http-protocol` feature, OFF by default; it
> reuses the in-process loopback server — `sparq_server::serve` on an ephemeral `127.0.0.1:0`
> port — and drives RAW HTTP at the bound port, NOT the federated SERVICE transport, so it does
> not touch the engine's egress allowlist). It ratchets a MEASURED PASS floor over: query via
> GET / POST-urlencoded / POST-direct, the `QUERY` method, update via POST, the
> `default-graph-uri` / `named-graph-uri` overrides, SELECT/ASK negotiation (SRJ / SRX / CSV /
> TSV), a **present-but-unsatisfiable `Accept` → 406 Not Acceptable** (Oxigraph parity,
> w3c/sparql-protocol#40 — <!-- [OPUS-4.8] sq-406acc --> formerly a divergence, now a genuine
> PASS that raised the floor 20→21), and the **200 / 400 / 405 (with `Allow`) / 406 / 415**
> status codes. **Honest boundary:** two behaviours remain DOCUMENTED DIVERGENCES (reported
> separately, NOT summed into the floor, so they never inflate the conformance number): an
> **absent / `*/*` `Accept` defaults to SPARQL-results JSON** (a W3C-permitted default
> representation — only a *present-but-unsatisfiable* `Accept` is a 406), and an **ASK with
> `Accept: text/csv` falls back to a JSON boolean** (CSV/TSV have no boolean serialisation). Run
> it with
> `cargo test -p sparq-conformance --features http-protocol --test http_protocol_suite`; the row
> is in the central scoreboard (`W3C SPARQL 1.1 Protocol (HTTP)`). [OPUS-4.8]

<!-- [OPUS-4.8] sq-1uuxz: comment separates the two adjacent blockquotes (markdownlint MD028). -->

> **Service Description + Graph Store Protocol conformance lane (sq-1uuxz, epic sq-my8wd) — what
> is ratcheted.** The federation-descriptor + GSP write surfaces (see "Federation discovery" and
> the Graph-Store-Protocol section below) are covered by a dedicated **SD + GSP** conformance suite
> in `sparq-conformance` (opt-in `federation-descriptors` feature, OFF by default; it reuses the
> in-process loopback server — `sparq_server::serve` on an ephemeral `127.0.0.1:0` port, stood up
> with the server's `federation-descriptors` runtime flag ON — and drives RAW HTTP at the bound
> port, so it does not touch the engine's egress allowlist). It ratchets a MEASURED PASS floor
> over **(A) the Service Description** (`GET /sparql` with no query): the `sd:Service` advertises
> exactly the result/input formats (SRJ/SRX/CSV/TSV + Turtle/N-Triples/RDF-XML), the query/update
> languages, the SPARQL versions (`sd:supportedVersion` 1.0/1.1/1.2) and `sd:BasicFederatedQuery`
> that the server GENUINELY implements — **no over-advertising** (each advertised result format is
> cross-checked against a real SELECT request, and JSON-LD — not served in this build — must NOT
> appear) — and **(B) the Graph Store Protocol**: a GET/PUT/POST/DELETE round-trip on a named graph
> (indirect `?graph=<iri>` + direct `/graphs/<path>`) and the default graph (`?default`), VERIFYING
> store state after every op (PUT→GET-back-equal; PUT replaces; POST merges; DELETE removes;
> **200/201/204/400/404/405 (with `Allow`)/415**). **Honest boundary:** a GSP read of an ABSENT
> named graph is **200 + empty graph, NOT 404** (GSP-permitted — sparq treats an empty graph as
> existing); this is a DOCUMENTED DIVERGENCE, reported separately and NOT summed into the floor. Run
> it with `cargo test -p sparq-conformance --features federation-descriptors --test sd_gsp_suite`;
> the row is in the central scoreboard (`SPARQL 1.1 Service Description + Graph Store Protocol`).
> [OPUS-4.8]

<!-- [OPUS-4.8] sq-b3df9: comment separates the two adjacent blockquotes (markdownlint MD028). -->

> **JSON-LD content negotiation (`jsonld` feature — default-on, [OPUS-4.8] sq-oy1f.4).** The
> server speaks `application/ld+json` out of the box (the `jsonld` feature is in the default set —
> a maintainer-directed exception to opt-in-by-default). `application/ld+json` joins the
> q-value-aware RDF negotiation in BOTH directions: a CONSTRUCT/DESCRIBE or a Graph-Store-Protocol
> read with `Accept: application/ld+json` is served as the engine's *flattened* JSON-LD (a
> `{"@graph": […]}` node-object document that `toRdf`-round-trips to the same graph), and a GSP
> `PUT`/`POST` with `Content-Type: application/ld+json` is parsed into the store via the engine's
> JSON-LD parser. Toggleable off via `--no-default-features --features server`: then
> `application/ld+json` is unrecognised — an `Accept` for it falls back to a supported graph format
> (never a 406 — the endpoint always has a representation), and a write body in it is a plain `415`
> — byte-identical to a JSON-LD-disabled build. What is default-on now: JSON-LD parse + serialise
> (flattened) + content-negotiation; full conneg-conformance ratcheting is on the sq-oy1f roadmap.

<!-- [FABLE-5] sq-7d3dj.30.13: comment separates the two adjacent blockquotes (markdownlint MD028). -->

> **Default-on algebra rewrite (`algebra-rewrite` feature — [FABLE-5] sq-7d3dj.30.13).** The
> server's default set also lights sparq-engine's pre-execution algebra rewrite pass (#1735): a
> result-equivalent `FILTER(?v = <iri>)` IRI-constant folding + `FILTER(!bound)` anti-join applied
> at parse time, so the shipped server executes the same plans the CLI and the canonical benchmarks
> measure. IRI constants only (a literal equality is never rewritten — the `sq-lr2ii` avoidance
> contract); zero new dependencies. Drop it with `--no-default-features --features server,jsonld`
> for an explicitly rewrite-dark build; the sparq-engine LIBRARY default remains OFF for lean
> library consumers.

<!-- [OPUS-5] sq-7d3dj.30.15: comment separates the two adjacent blockquotes (markdownlint MD028). -->

> **Default-on DPccp join-order planner (`dp-planner` feature — [OPUS-5] sq-7d3dj.30.15).** The
> server's default set also lights sparq-engine's DPccp planner (sq-7d3dj.30.5): a connected BGP
> with 3 or more patterns that fits the connected-subgraph budget is planned as a cost-optimal
> BUSHY join tree instead of by greedy GOO. It is DEFAULT-ON once compiled, so every request gets
> it with no explicit install, and it is result-equivalent — only join ORDER changes, never the
> answer. This closes the gap where sq-7d3dj.30.5 lit the planner in `sparq-cli` alone, which made
> an HTTP-measured query plan differently from the CLI-measured one the canonical benchmarks use.
> Zero new dependencies. Drop it with `--no-default-features --features server,jsonld` for an
> explicitly greedy-GOO build; the sparq-engine LIBRARY default remains OFF for lean library
> consumers.

**2. EXPLAIN a query plan (no execution) or analyze (execute + per-operator trace).**
`text/plain` response. Use `explain` / `explain=plan` (or `Accept: text/x-sparq-explain`)
for the dry run, `explain=analyze` to run + trace (SELECT/ASK only):

```sh
curl -G 'http://127.0.0.1:3030/sparql?explain=true' \
     --data-urlencode 'query=SELECT * WHERE { ?a <http://ex/knows> ?b . ?b <http://ex/age> ?age }'
```

**Structured plan tree (`explain-json` feature, default-on for the server binary —
sq-ixc3.19).** `Accept: application/x-sparq-explain+json` answers the SAME `explain` /
`explain=analyze` surface with the typed plan tree (`explain_json::PlanNode`) as camelCase
JSON — per operator `{"operator", "estimated", "actual", "nanos", "qError", "children"}`
(the sq-jbqh4 schema contract the GUI plan explorer renders; `qError` =
max(est/actual, actual/est)). Like the text CT, the JSON `Accept` alone requests a
plan-only explain. A `--no-default-features --features server,…` build answers it **406**
(the text explain still works) so clients degrade explicitly, never mis-parse text as JSON:

```sh
curl -G -H 'Accept: application/x-sparq-explain+json' \
     'http://127.0.0.1:3030/sparql?explain=analyze' \
     --data-urlencode 'query=SELECT * WHERE { ?a <http://ex/knows> ?b . ?b <http://ex/age> ?age }'
```

**3. Generation-pinned snapshot reads (DEFAULT build — `sq-ci2d6`).** Every `/sparql` response
carries the `Sparq-Generation` header (the generation the response was produced against; an
update's `204` carries the generation *containing* the write — the read-your-writes token).
Capture it and pin a later read with `?generation=N` (URL param or url-encoded body — body wins)
to get an **immutable snapshot** of the store as of that generation. This is available with **no
feature enabled**: the pin is bounded to the generation ring's **concurrency-retention window** —
the last **K = 4** generations older than current are always kept (`sparq_serve::ring::DEFAULT_RETAIN`;
`RingConfig::retain`). That is exactly the window that makes **multi-request `LIMIT`/`OFFSET`
pagination snapshot-consistent**: pin one generation, page it, and the pages union to the
single-shot result at that generation even if writes interleave (with a deterministic order —
add `ORDER BY`, or rely on the stable order over the immutable snapshot):

```sh
# read-your-writes: capture the generation an update lands in (default build — no feature)
G=$(curl -si http://127.0.0.1:3030/sparql -H 'content-type: application/sparql-update' \
      --data 'INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }' \
      | grep -i sparq-generation | tr -d '\r' | awk '{print $2}')
# page a large read pinned to that generation — page 1, then page 2, still consistent
# even if another client writes between the two requests:
curl -G http://127.0.0.1:3030/sparql --data-urlencode "generation=$G" \
     --data-urlencode 'query=SELECT ?child WHERE { <c> <member> ?child } ORDER BY ?child LIMIT 1000 OFFSET 0'
curl -G http://127.0.0.1:3030/sparql --data-urlencode "generation=$G" \
     --data-urlencode 'query=SELECT ?child WHERE { <c> <member> ?child } ORDER BY ?child LIMIT 1000 OFFSET 1000'
```
**Honest boundary (the retention contract).** The default window is bounded by the ring's
*concurrency* retention (K = 4), **not** a durable history: a pin more than K generations behind
current has **aged out** → `410 Gone` (the snapshot is gone; re-read the current generation and
restart pagination — the server **never** silently substitutes a different generation). If a
pagination sweep must survive more than K interleaved writes, build with the opt-in **`time-travel`**
feature, which EXTENDS retention (`--time-travel-generations N`, `--time-travel-max-age SECS`) so
`?generation=N` reaches far older snapshots — same header/pin mechanics, wider window:

```sh
cargo run -p sparq-server --features time-travel -- data.ttl --time-travel-generations 32
```
Status: aged-out generation → `410 Gone`; never-published / unparsable / pinning an
*update* (an update always applies to current) → `400`. All identical in both feature states —
`time-travel` only changes how far back a pin can reach.

**4. WebSocket subscriptions (SEPA-style live SELECT).** Connect to
`ws://127.0.0.1:3030/subscriptions`, send `subscribe`, get `subscribed` + an initial
`notification` (sequence 0 = full result as `addedResults`), then added/removed bindings
diffs after each committed update that changes the result:

```text
client:  {"subscribe": {"query": "SELECT ?s WHERE { ?s <http://ex/age> ?o }", "alias": "ages"}}
server:  {"subscribed": {"id": 1, "alias": "ages"}}
server:  {"notification": {"id": 1, "sequence": 0, "addedResults": {…full result…},
                           "removedResults": {"head":{"vars":["s"]},"results":{"bindings":[]}}}}
         …POST /sparql update commits…
server:  {"notification": {"id": 1, "sequence": 1, "addedResults": {…}, "removedResults": {…}}}
client:  {"unsubscribe": {"id": 1}}
server:  {"unsubscribed": {"id": 1}}
```
`addedResults`/`removedResults` are each full SPARQL-JSON results objects. Refusals and
failed re-evaluations come back as `{"error": {"message": …, "id"?: n}}`.

[OPUS-4.8] sq-cxk5: when `--auth-token-read` is set, the upgrade is gated behind the read
token (`401` before upgrade otherwise). A non-browser client sends `Authorization: Bearer
<TOKEN>` on the handshake; a **browser** (which cannot set WS handshake headers) passes it as
a subprotocol: `new WebSocket("ws://host/subscriptions", ["bearer." + token])`. The server
takes the substring after the `bearer.` prefix as the token, validates it (constant-time),
and echoes the subprotocol back per RFC 6455. With no read token configured, the upgrade is
open (back-compatible).

**4b. SSE subscriptions (`text/event-stream`).** [OPUS-4.8] sq-bxog: the SAME subscription
engine over Server-Sent Events, for clients that prefer a plain HTTP GET stream to a
WebSocket. `GET /subscriptions/sse?query=<SELECT>[&alias=<x>]` opens one subscription per
stream (the query is in the query string — SSE is one-way, so there is no `subscribe`/
`unsubscribe` frame; close the stream to unsubscribe). The events carry the SAME JSON as
the WS path — only the framing differs (`event:` / `data:` / `id:` lines, blank-line
terminated, `: ping` keep-alive comments hold idle connections open). The SSE `id:` mirrors
the per-subscription `sequence`.

```sh
curl -N 'http://127.0.0.1:3030/subscriptions/sse?query=SELECT%20?s%20WHERE%20{%20?s%20%3Chttp://ex/age%3E%20?o%20}&alias=ages'
```
```text
event: subscribed
data: {"subscribed":{"id":1,"alias":"ages"}}

event: notification
id: 0
data: {"notification":{"id":1,"sequence":0,"alias":"ages","addedResults":{…full result…},"removedResults":{…empty…}}}

  …POST /sparql update commits…
event: notification
id: 1
data: {"notification":{"id":1,"sequence":1,"alias":"ages","addedResults":{…},"removedResults":{…}}}
```
[OPUS-4.8] sq-cxk5: like the WS path, when `--auth-token-read` is set this GET is gated
behind the read token via the `Authorization: Bearer <TOKEN>` header (it is a plain GET, so
the header is the only auth channel — no WS subprotocol) — `401` before the stream opens
otherwise. A registration refusal (missing/non-SELECT/malformed `query` → `400`; capacity/budget →
`503`) is returned as a normal `{"error":"…"}` JSON HTTP response BEFORE the stream opens —
SSE cannot set a status once the stream is flowing. A later re-evaluation failure ends the
stream with a final `event: error` frame. Both transports share one registry + change
source, so the per-conn / server-wide subscription caps and the `sparq_active_subscriptions`
gauge count SSE streams and WS subscriptions together.

**5. Graph Store read + write + operational endpoints.**

```sh
# READ (GET/HEAD): serialises the addressed graph in the Accept-negotiated RDF syntax
# (default N-Triples; also text/turtle = prefix-compacting Turtle, application/rdf+xml = RDF/XML,
#  and application/ld+json = flattened JSON-LD with `--features jsonld` [OPUS-4.8] sq-oy1f.1)
curl http://127.0.0.1:3030/sparql/graph?default                 # GSP indirect (default graph)
curl 'http://127.0.0.1:3030/sparql/graph?graph=http://ex/g'     # GSP indirect (named graph)
curl http://127.0.0.1:3030/graphs/whatever                      # GSP direct (request URI is the graph IRI)
curl -H 'Accept: application/rdf+xml' http://127.0.0.1:3030/sparql/graph?default   # RDF/XML read [OPUS-4.8] sq-rt6v
curl -H 'Accept: application/ld+json' http://127.0.0.1:3030/sparql/graph?default   # JSON-LD read (--features jsonld) [OPUS-4.8] sq-oy1f.1

# WRITE (sq-gxsj): body is RDF, format by Content-Type
#   (turtle | n-triples | n-quads | trig | application/rdf+xml [OPUS-4.8] sq-rt6v
#    | application/ld+json with `--features jsonld` [OPUS-4.8] sq-oy1f.1)
# PUT = REPLACE graph contents (201 if created, 204 if replaced):
curl -X PUT 'http://127.0.0.1:3030/sparql/graph?graph=http://ex/g' \
     -H 'content-type: text/turtle' --data '<http://ex/s> <http://ex/p> <http://ex/o> .'
# POST = MERGE (additive); selector-less POST to /sparql/graph creates a fresh server-named graph:
curl -X POST 'http://127.0.0.1:3030/sparql/graph?graph=http://ex/g' \
     -H 'content-type: application/n-triples' --data '<http://ex/s2> <http://ex/p> <http://ex/o2> .'
# DELETE = DROP the graph (204; 404 if a named graph is absent; ?default → CLEAR DEFAULT, always 204):
curl -X DELETE 'http://127.0.0.1:3030/sparql/graph?graph=http://ex/g'

# PATCH = atomic, graph-scoped in-place MODIFY (sq-hj4n; 204 on success). Two body dialects:
#   1) application/sparql-update (ALWAYS-ON) — the body IS a SPARQL Update, applied atomically
#      through the same writer, with its WHERE dataset scoped to the addressed graph:
curl -X PATCH 'http://127.0.0.1:3030/sparql/graph?graph=http://ex/g' \
     -H 'content-type: application/sparql-update' \
     --data 'DELETE DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } } ;
             INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o2> } }'
#   2) text/n3 (Solid N3-Patch, OPT-IN: needs the `n3-patch` build feature AND --n3-patch / SPARQ_N3_PATCH=1;
#      else 415). A solid:InsertDeletePatch translated into ONE atomic graph-scoped DELETE/INSERT … WHERE:
curl -X PATCH 'http://127.0.0.1:3030/sparql/graph?default' \
     -H 'content-type: text/n3' --data '@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:where   { ?s ex:name "Alice" . };
  solid:deletes { ?s ex:age 30 . };
  solid:inserts { ?s ex:age 31 . }.'

curl http://127.0.0.1:3030/health                               # -> "ok"
curl http://127.0.0.1:3030/metrics                              # Prometheus text exposition (gated by --auth-token-read; sq-9jrx)

# Admin WAL compaction / vacuum for ERASURE-COMPLETENESS (sq-x32t). POST-only; gated by the
# WRITE token. Physically purges data removed by DELETE / DROP (incl. orphaned literal VALUES) from the
# on-disk store so a logical erasure is followed by real erasure. 200 ok; 409 if in-memory
# (no --persist, nothing to purge); 503 on a transient durable-write error (retryable):
curl -X POST -H 'Authorization: Bearer <TOKEN>' http://127.0.0.1:3030/admin/compact

# Online consistent-snapshot backup + restore (feature `backup`, default OFF). POST-only, WRITE-gated.
# Backup streams a single self-describing artifact of the LIVE store WHILE SERVING (no stop-the-world):
curl -X POST -H 'Authorization: Bearer <TOKEN>' http://127.0.0.1:3030/admin/backup -o snapshot.spqb
# Restore atomically installs a store rehydrated from that artifact (in-memory; fail-closed):
curl -X POST -H 'Authorization: Bearer <TOKEN>' --data-binary @snapshot.spqb http://127.0.0.1:3030/admin/restore
# On a --persist server, opt into write-through so the restore survives a restart (else 409):
curl -X POST -H 'Authorization: Bearer <TOKEN>' --data-binary @snapshot.spqb 'http://127.0.0.1:3030/admin/restore?persist=true'
# Restore on start (bootstrap a fresh node / PITR base):  sparq-server --restore snapshot.spqb
#   …into a durable store (survives restart):             sparq-server --persist DIR --restore snapshot.spqb --restore-persist
# Incremental change-stream / point-in-time recovery (PITR): a DELTA between a retained
# generation N and the current generation (replayed forward onto the matching base):
curl -X POST -H 'Authorization: Bearer <TOKEN>' 'http://127.0.0.1:3030/admin/backup/delta?from=0' -o d0.spqd
# Recover to the chosen point: restore the base, then replay the delta chain (oldest first):
sparq-server --restore snapshot.spqb --restore-delta d0.spqd --restore-delta d1.spqd
```

`/metrics` is hand-rolled Prometheus text exposition (no metrics dependency); the
middleware wraps the whole hardening stack, so shed (`429`), body-limit (`413`) and panic
(`500`) responses are counted with the status the client saw:

| Metric | Type | What |
| --- | --- | --- |
| `sparq_http_requests_total{endpoint,status}` | counter | requests by endpoint + response status |
| `sparq_query_duration_seconds` | histogram | wall time of `/sparql` (query + update); buckets 1 ms … 10 s |
| `sparq_active_subscriptions` | gauge | active WebSocket subscriptions (scrape time) |
| `sparq_graph_triples` | gauge | triples in the published graph (scrape time) |
| `sparq_updates_total` | counter | successfully applied SPARQL updates |

**`POST /admin/compact` — WAL compaction / vacuum (erasure-completeness, `sq-x32t`).** A logical
`DELETE` / `DROP GRAPH` retracts data from the live view but leaves the superseded bytes in earlier
`--persist` WAL segments (and the dictionary) until a compaction folds the live state into a fresh
base. This admin op does that on demand: it **physically rewrites** the on-disk store to only the
current live triples, with a **re-interned (purged) dictionary**, then **atomically swaps** the
directory (rollback-safe two-rename + WAL truncate; an interrupted swap is healed on the next
open). So a deleted triple's value — including an orphaned **literal value** (e.g. personal data) —
is gone from disk, not just hidden. It runs on the single writer thread strictly **between
batches** (no race with a concurrent write), preserves the live triple set **exactly** (no
generation published; reads keep flowing throughout). **POST-only**, gated by the **write** token
(`--auth-token`), like an UPDATE. Responses: `200` ok; `409` if the server is in-memory (no
`--persist` — there is no on-disk history to purge, so a no-op success would mislead); `503` on a
transient durable-write error (retryable, writer stays alive). **Offline equivalent:** stop the
server and run `sparq-cli compact <persist-dir>` (see the `cli` skill). **Honest scope:** this
scrubs the engine's own on-disk segments + dictionary; it cannot reach bytes already copied off-box
(filesystem snapshots, COW history, external backups) — those are the operator's responsibility
(see `compliance/privacy/retention-erasure-runbook.md` §7a/§7b).

**`POST /admin/backup` + `POST /admin/restore` — online consistent snapshot backup/restore
(feature `backup`, default OFF; `sq-o5bi`).** An ONLINE point-in-time backup of the live serving
store, distinct from the offline `sparq-cli save` (stop-the-world, index rebuild) and from the
`--persist` per-graph WAL. **`/admin/backup`** pins the CURRENT generation lock-free and serialises
it off the immutable `Arc` — so it runs **while serving**: readers never block the writer and the
writer never blocks readers throughout. The response is **one self-describing artifact** (Stardog
backup-ID model, "Option A"): a small textual header recording the generation number (= the single
writer's seq), the per-pod epoch vectors, the triple count and a body integrity digest, then the
full dataset (default + named graphs) as N-Quads. **`/admin/restore`** imports such an artifact and
**atomically installs** a freshly-built ring+writer rehydrated from it — readers in flight keep
serving from the old store until they release their pin; every read/update after the swap sees the
restored store. **Fail-closed:** a corrupt / truncated / version-mismatched / non-artifact body is
rejected (`400`) and the live store is left **untouched** (the new core is built fully before the
swap). Both routes are **POST-only** and gated by the **write** token (the admin gate). Responses:
backup `200` (`application/octet-stream`); restore `200` on success, `400` on a rejected artifact.
**Single-flight (`sq-fy8ci`):** a restore posted while another is still in flight is rejected
**`409`** (`a restore is already in progress`) instead of being silently serialized behind it by
the writer thread (last-writer-wins) — retry after the first completes. Embedders driving
`AppState::restore_from` directly can claim the same permit via `AppState::try_begin_restore()`
(an RAII `RestoreGuard`; dropping it — even on a panic — releases the permit).

**Restore into a live durable store (`sq-ft7u`).** On a `--persist` server, a plain
`/admin/restore` (no opt-in) is **`409`** — an in-memory-only swap on a durable server would be
silently lost on the next restart (a footgun). Add **`?persist=true`** to write the restore
**THROUGH to the durable directory**, so it **survives a restart** (the on-disk base becomes the
restored image). The durable swap runs **on the single writer thread** (sequenced with updates — no
lock, no race with a concurrent durable commit) and is **crash-safe**: the imported image is written
to a sibling and swapped in with a **two-rename directory swap** (parent dir fsync'd between renames)
that an interrupted crash heals deterministically (`Graph::restore_into_durable` reuses the exact WAL
compaction swap + `recover_compaction` healer). **Fail-closed throughout:** the artifact is imported
+ validated **before** any on-disk change, so a corrupt artifact is `400` with the durable store
**untouched** and no swap-sibling leftovers; a swap I/O error leaves the OLD store intact and the
writer alive. `?persist=true` on an **in-memory** server is `409` (no durable dir). **Restore-on-start:**
`--restore <FILE>` / `SPARQ_RESTORE` seeds the in-memory store before binding (bootstrap for
horizontal-scaling stage-2 + PITR base); with `--persist DIR` it is **refused unless** you add
**`--restore-persist`** / `SPARQ_RESTORE_PERSIST=1`, which writes the artifact through to `DIR`
crash-safely on start. **Honest scope:** **at-rest encryption of the artifact is out of scope** — the
integrity digest detects accidental corruption, not tampering, and is not a confidentiality or
authenticity guarantee; protect the artifact at the storage tier.

**`POST /admin/backup/delta?from=N` + `--restore-delta` — incremental change-stream / PITR
(same `backup` feature; `sq-bu1a`).** The Option-A backup above is the BASE half of a
point-in-time-recovery story; this is the incremental companion. **`/admin/backup/delta`** streams
a single self-describing **delta artifact** (distinct magic `SPARQ-BACKUP-DELTA`) keyed off the
generation/writer-seq: a header recording the `from-generation` N → the current `to-generation`
plus the per-pod epoch vector at `to`, then the **quad-set change** (inserted + deleted quads, each
as N-Quads) between those two generations. `from` must still be **retained** by the ring — without
the `time-travel` feature only the last few generations (the concurrency window) are retained, so a
`from` older than that is `410 Gone` (widen retention with `time-travel`); a missing/invalid `from`
is `400`. To recover to a chosen point, restore the base then **replay the delta chain forward**:
`sparq-server --restore base.spqb --restore-delta d0.spqd --restore-delta d1.spqd …` (oldest
first; or `SPARQ_RESTORE_DELTA`). **Fail-closed:** a corrupt / version-mismatched / out-of-order /
gapped delta aborts the whole recovery and the live store is untouched (import + full replay happen
before any swap). **Honest scope:** deltas are **same-lineage** only — the chain must be the writer
history of that base (blank-node labels are stable within a lineage, which is what the quad-set diff
relies on); cross-lineage diffing is unsupported. At-rest encryption is out of scope, same as the
base.

**Durable, replayable change-data-capture stream (`sparq-serve` feature `change-stream`,
default OFF; `sq-b4fns`, gh-906).** Where the delta above is the PITR *backup* companion, this is
a **continuous CDC stream** in the Amazon-Neptune-Streams shape: `sparq_serve::change_stream::ChangeLog`
records **every** commit as one ordered, monotonically-sequenced **change record** —
`(seq, generation, timestamp, +inserts / −deletes as N-Quads)` — to a **segmented, fsync'd
append-only log on disk**. A consumer `poll(from_seq)`s from any offset and **replays after a
process restart** (`ChangeLog::open` re-reads the segments, recovers a torn tail, and resumes at the
next seq) — a raw durable cross-service feed (replication / live downstream index / triggers),
distinct from the *ephemeral* in-process WebSocket/SSE subscriptions (a disconnect misses every
change). Same N-Quads same-lineage quad-set diff + fail-closed FNV-1a digest as the backup family
(no new dependency; no HTTP/async in the library). At-rest encryption + authenticity are out of
scope, same as the backup family. **Retention/truncation (`sq-n9s4d`)**: `ChangeLog::apply_retention`
(and the deterministic `apply_retention_at`) drops whole OLD segments — oldest-first, never the
active segment, never a partial segment — under a `RetentionPolicy` composing a consumer-ack
watermark (a HARD safety bound: any unacked record keeps its segment), `max_age`, and
`max_total_bytes` pressure; the default policy is a no-op and nothing is ever dropped implicitly.
A `poll` from a trimmed-away offset **fails closed** (never a silent skip) — consumers resume from
`ChangeLog::first_seq`.
**External-broker sink (`sq-l6zks`, gh-3216) — the separate, heavier `sparq-serve` opt-in
`change-sink` (default OFF, implies `change-stream`).** `change_sink::ChangeSink` is the pluggable
broker seam; `BrokerRelay::open(dir, consumer, sink, config)` + `pump()` is the resumable pump from
that durable log to a sink, carrying a **durable delivered-through watermark** (persisted as
`changesink-<consumer>.offset` beside the segments) so a restarted process resumes rather than
replays. Delivery is **at-least-once** — the watermark is persisted after the sink's `flush`, so
consumers dedupe on `sequenceNumber`; the partition key is CONSTANT per stream so a partitioned
broker preserves commit order; a re-base gap record is delivered as an explicit `"op": "REBASE"`
entry, never as an empty commit. The relay runs **off the writer thread** (a broker outage stalls
the relay, never commits), and feeding `delivered_through_seq()` into
`RetentionPolicy::acked_through_seq` keeps retention from trimming past it (a trimmed watermark
fails the pump closed). One in-tree sink ships — `NatsSink` (core NATS over plain TCP, std-only,
**no TLS**). There is deliberately **no in-tree Kafka client**: Kafka (and TLS/SASL/retries) is
reached by implementing `ChangeSink` over the host's own client, so no broker client and no async
runtime enter the library-first crate. Payloads are plaintext, unsigned JSON — same boundary as the
backup family — so pointing a relay at a broker is a data-egress decision.
**Recording seam (`sq-bdaw5`):** install `ChangeLog::into_commit_hook(on_error)` via
`Writer::spawn_with_commit_hook` and every commit-publish is recorded **on the writer thread** —
one append+fsync per PUBLISHED GENERATION, gapless by construction (the writer is the sole
publisher), with **no caller-side lock and no serialisation of submitters**; acks happen-after the
record is durable. Restore publishes never fire the hook (not same-lineage — re-baseline
explicitly). A recording failure is dropped + reported to `on_error`, never failing the
already-published write; the log's discontinuity check then fail-closes later records rather than
silently gapping. **Resync (`sq-r2cu1`):** `ChangeLog::rebase_to(generation)` is that explicit
operator re-base — it appends an honest **gap record** (`ChangeRecord::rebase = true`, no changes)
marking the uncaptured span and re-arms recording from `generation` (strictly forward-only,
fail-closed otherwise) **without wiping the log**; a consumer replaying past the gap re-bootstraps
(e.g. from a backup at/after that generation) instead of trusting a silently incomplete diff stream.
On a RUNNING recorder, `ChangeLog::into_commit_hook_with_control` yields a `ChangeStreamControl`
that runs the resync on the LIVE log (serialized with the hook — no stop/re-open), and
`rebase_to_new_lineage` is the post-restore variant for a REPLACED lineage whose generation
numbering restarted (gh-2436).

**`POST /admin/change-stream/rebase` — operator resync of the running server's tracked log
(`sparq-server` feature `change-stream`; gh-2436).** WRITE/admin-gated, POST-only, same
double-opt-in `404` as `/streams`. Empty body = re-baseline to the writer's CURRENT generation
(the dropped-record resync); JSON fields `generation` (explicit target) and `newLineage: true`
(AFTER `/admin/restore` — the restored ring restarts at generation 0, so the same-lineage
forward-only check would reject it forever). `200` returns the gap record (`seq`/`generation`/
`rebase` + `nextSequenceNumber`); a non-forward same-lineage target is a `409`; an unknown body
field is a fail-closed `400`. `GET /streams` renders the gap as ONE explicit `"op": "REBASE"`
marker entry (no `data`) — never silently flattened away.

**`GET /streams` — CDC poll endpoint (`sparq-server` feature `change-stream`, default OFF;
`sq-2999l`, gh-906).** The HTTP poll surface over that durable log, in the Amazon-Neptune-Streams
`GetRecords` shape. Configure a log directory (`--change-stream DIR` / `SPARQ_CHANGE_STREAM`,
`ServerConfig::change_stream_dir`) and the server (1) RECORDS every published group-commit
generation as one ordered change record on the writer thread (possibly containing several
concurrent SPARQL Updates; [GPT-5.6] `sq-kqofk`), and (2) serves `GET /streams` over it (the route
is `404` unless the directory is set — the same double-opt-in as `/tpf`). Parameters: `iteratorType`
(`TRIM_HORIZON` = replay everything RETAINED, the default; `AT_SEQUENCE_NUMBER` + `at=N`;
`AFTER_SEQUENCE_NUMBER` + `after=N`, the resume case; `LATEST` = tail only) and `limit` (max commits per page, default 100,
clamped to 10000). The JSON response flattens each commit's quad-level changes to one stream record
per `(op, quad)` — `{ eventId: { commitNum: <seq>, opNum: <1-based> }, op: ADD|REMOVE, generation,
commitTimestampNanos, data: { stmt: "<n-quads line>" } }` — plus a `nextSequenceNumber` continuation
token (pass as `at=`/`after=`), `lastSequenceNumber`, `totalRecords` (commit count) and
`hasMoreRecords`. A poll is a READ (gated by the read auth). A sequence-anchored `iteratorType` with
no anchor is a fail-closed `400` (never a silent replay-all). **Retention-aware (`sq-iz7ag`)**:
`TRIM_HORIZON` resolves to the log's TRIM HORIZON (`ChangeLog::first_seq`), not seq 0 — polling
below the horizon fails closed in the durable log, so pinning the whole-stream replay at 0 would
`500` on any trimmed log. An `at`/`after` anchor whose RESOLVED start (`after=N` ⇒ `N+1`) is below
the horizon is a `410 Gone` naming the seq to resume from — the records are permanently gone and
that consumer must re-bootstrap — never a silent restart at the horizon (which would skip records
it has not seen). The horizon is the read handle's view, taken when the server opened the log
directory: sparq-server never applies retention itself, so a host that trims through its own handle
while the server runs still gets the fail-closed `500` until a restart. With the feature off the route + the
recording hook are `#[cfg]`-stripped (byte-identical); with it on, recording rides the sequenced
writer's group commit and does not serialise submitters. Relaying that log onward to an external
broker is the separate `sparq-serve` `change-sink` opt-in (`sq-l6zks`, above), not part of this
endpoint.

**`GET /queries` + `DELETE /queries/{id}` — running-query registry (`sparq-server` feature
`query-registry`, default OFF; sq-qsm5z, [SONNET-4.6]).** An opt-in in-memory registry of
currently executing SPARQL queries, providing GraphDB query-monitoring and kill parity.

- **`GET /queries`** — READ-gated (fail-closed). Returns `{"queries":[…]}` where each entry
  carries `id` (opaque hex), `kind` (`"query"` or `"update"`), `start` (Unix epoch ms),
  `fingerprint` (FNV-1a hex of the trimmed query text — never raw text, per the #241 /
  sq-m9prn audit-log posture), and `elapsed_ms`. Sorted by `id` (registration order).
- **`DELETE /queries/{id}`** — WRITE-gated (fail-closed). Cooperatively cancels the named
  query by flipping the `sq-kq9ia` `Arc<AtomicBool>` cancel flag wired into its `QueryBudget`.
  The engine observes it at the next poll site and aborts. Returns `204 No Content` on success;
  `404` if the id is not found (already finished or bad id).
- **RAII lifetime**: each executing SELECT/ASK/CONSTRUCT/DESCRIBE/**EXPLAIN (plan + analyze)**
  registers on start and deregisters on completion, error, or panic — the entry is always cleaned
  up. (EXPLAIN ANALYZE wiring added in sq-t1isr, [SONNET-4.6].)
- **SPARQL UPDATEs are registered too** (`kind: "update"`; sq-m9prn, [SONNET-4.6]). An UPDATE
  registers when the sequenced writer thread STARTS applying it — not when it is queued — so a
  row names exactly what is consuming the writer, which is the operation whose cancellation
  also unblocks every write queued behind it. The flag reaches the `DELETE/INSERT … WHERE`
  evaluation (the engine installs the UPDATE's `QueryBudget` thread-locally for the whole
  update). A cancelled UPDATE is **rejected, not partially applied**: the writer forks per
  batch and seals only on success, so the store is left at its pre-update state and the writer
  keeps serving. Non-WHERE operations (`INSERT DATA`, `CLEAR`/`DROP`, …) do not consult the
  budget, so they are listed but complete rather than cancel.
- **Fingerprint-only — and what that does NOT buy you (honest boundary).** The `GET /queries`
  listing never exposes raw query or update text; only the fingerprint is visible, satisfying
  the audit-log redaction discipline. But the construction is **FNV-1a: an unkeyed, 64-bit,
  non-cryptographic checksum**. It is a *stable correlation tag*, **not** a confidentiality
  boundary and **not** a one-way function in any cryptographic sense: anyone who can read the
  listing can hash candidate texts offline and confirm a match, so a guessable — or
  low-entropy, or template-generated — body is recoverable by search, and 64 bits resists
  neither exhaustive guessing of a small candidate set nor collision search. This matters most
  for `kind: "update"`, whose `INSERT DATA` body can embed secrets or personal data: the
  fingerprint stops the payload being *transmitted*, it does not stop it being *guessed*. Treat
  the fingerprint as sensitive metadata, keep `/queries` behind its READ gate (it is
  fail-closed), and do not rely on it to protect update content from a party who already has
  read access. (Keying the fingerprint — e.g. an HMAC under a per-process secret — would close
  the offline-guessing gap and is tracked as follow-up work, not shipped here.)
- **Zero cost when feature is OFF**: no `AppState` fields, no routes; byte-identical to before.

```sh
# (--features query-registry required at build time)
# List running queries:
curl -H 'Authorization: Bearer <TOKEN>' http://127.0.0.1:3030/queries

# Cancel a query by id:
curl -X DELETE -H 'Authorization: Bearer <TOKEN>' http://127.0.0.1:3030/queries/0000000000000001
```

GSP **writes** translate into a server-minted SPARQL Update (`DROP`/`CLEAR` + `INSERT
DATA`) and submit through the SAME sequenced group-commit writer the
`application/sparql-update` operation uses — so they share its atomicity, snapshot
consistency, blocking-on-commit semantics, the `Sparq-Generation` header (default build,
`sq-ci2d6`), AND its **auth gate** (a GSP write is as powerful as an UPDATE, so `PUT`/`POST`/
`DELETE`/`PATCH` are gated by `--auth-token` exactly like an UPDATE; `GET`/`HEAD` are reads). A
malformed body → `400`; an unsupported body Content-Type → `415`.

**`PATCH` — atomic, graph-scoped in-place modify (`sq-hj4n`, gh-916).** A `PATCH` on a graph
resource applies an atomic modify to the addressed graph, with two body dialects:

- **`application/sparql-update` (ALWAYS-ON, no feature):** the body IS a SPARQL Update, applied
  atomically through the shared writer. It is **scoped** to the addressed graph by defaulting the
  update's WHERE dataset to that graph (the SPARQL 1.1 Protocol §2.2 `using-graph-uri` mechanism).
  **Honest scope:** this scopes what the WHERE *reads*; an operation that explicitly names a
  *different* graph (`INSERT DATA { GRAPH <other> { … } }`, or an in-body `WITH`/`USING`) still
  targets where it says — and supplying the override alongside an in-body `USING`/`WITH` on a
  *named*-graph PATCH is the §2.2 conflict `400`. For `?default` the scoping is a no-op (the default
  graph is already the WHERE default). Success → `204`.
- **`text/n3` (OPT-IN — the `n3-patch` build feature AND `--n3-patch` / `SPARQ_N3_PATCH=1`):** a
  Solid-style **N3-Patch** (`solid:InsertDeletePatch` with `solid:where` / `solid:deletes` /
  `solid:inserts` formulas), translated into ONE atomic graph-scoped SPARQL Update (every block
  wrapped in `GRAPH <g> { … }` for a named graph). With a `solid:where` it is a pattern-based
  `DELETE/INSERT … WHERE`; without one it is ground `DELETE DATA`/`INSERT DATA` — either way one
  writer submission. Validation: exactly one patch resource, each formula property at most once, at
  least one of `deletes`/`inserts`, and no blank node in `deletes`/`where` (only `inserts` may mint
  one) → otherwise `400`. **Double opt-in** (build feature + runtime flag) mirrors `tpf`/`shacl`:
  with the feature **off** the N3-Patch parser + dispatch are `#[cfg]`-stripped (a `text/n3` body is
  then a plain `415`, byte-identical to before, and no new dependency / no `sparq-core` /
  `sparq-engine` / wasm impact); with the feature **on** but the flag **off** a `text/n3` body is
  also `415`. The N3 parsing rides `oxttl`'s `N3Parser` — already a server dependency — so even
  feature-on adds NO new crate. Success → `204`. `PATCH` is a WRITE, gated by `--auth-token` like an
  UPDATE.

**5b. Container (ghcr.io).** Published on every `vX.Y.Z` release tag as a distroless OCI image
index at `ghcr.io/sparq-org/sparq-server`, with `linux/amd64` and `linux/arm64` runtime images.
[GPT-5.6] `sq-fvzi6`: each release publishes `:X.Y.Z` (pin this for reproducible deployments),
`:X.Y` (tracks the newest patch in that minor line), and `:latest` (tracks the newest release);
an omitted tag selects `:latest`. The image sets `SPARQ_ALLOW_REMOTE=1` so the `0.0.0.0` bind it
needs (loopback is unreachable through Docker's port map) boots out of the box — running the
container is the operator's explicit choice to publish a surface. **This default has NO auth.**
Because every `SPARQ_*` var is read from the environment, secure it with `-e` (no flag wiring):

```sh
docker run --rm -p 3030:3030 ghcr.io/sparq-org/sparq-server                       # empty graph, no auth
docker run --rm -p 3030:3030 -v "$PWD/data:/data:ro" \
  ghcr.io/sparq-org/sparq-server --format turtle /data/dataset.ttl                # serve a dataset
docker run --rm -p 3030:3030 \
  -e SPARQ_AUTH_TOKEN="$TOK" -e SPARQ_AUTH_TOKEN_READ=1 \
  ghcr.io/sparq-org/sparq-server                                                  # fully Bearer-gated
```

Deliver the token over TLS (terminate at a proxy). See `crates/sparq-server/README.md` →
"Running the container image".

The image self-declares a `HEALTHCHECK` (CIS Docker §4.6, `sq-toze.36`). Since distroless has
no shell/`curl`/`wget`, the check is the server binary probing **itself**: `sparq-server
--health-probe` opens a TCP connection to the loopback `/health` and exits 0 (healthy)/non-zero
(unhealthy) — exec-form in the Dockerfile. Override the probed address with `--health-probe-addr
HOST:PORT` or `SPARQ_HEALTH_PROBE_ADDR`. `docker ps` then shows `(healthy)`/`(unhealthy)`; k8s
usually runs its own `/health` probe and ignores the image healthcheck.

**5c. Federation discovery — VoID + Service Description (OPT-IN, `sq-d3d8`).** A server can
advertise itself as a discoverable federation node by serving two read-only RDF descriptors:

- `GET /.well-known/void` — a [W3C VoID](https://www.w3.org/TR/void/) dataset description
  (`void:triples` / `void:entities` / `void:classes` / `void:properties` + one
  `void:classPartition` per class and `void:propertyPartition` per predicate), **plus the
  characteristic-set source statistics** (sq-mr32, federation A3/Z2): sparq already mines
  per-entity-type predicate co-occurrence + multiplicity (Neumann & Moerkotte characteristic
  sets), so the served VoID also emits them under a documented sparq extension vocab
  `scs:` (`<http://sparq.dev/ns/cs#>`) — `scs:characteristicSet` linking the dataset to one
  `scs:CharacteristicSet` node per retained set (`scs:subjects` = `count(C)`, plus one
  per-predicate `scs:predicateStat` reusing `void:property`/`void:triples` and adding
  `scs:avgMultiplicity`), with the EXACT distinct-set count on `scs:distinctCharacteristicSets`.
  The CS stats are a strict superset of the standard VoID (a VoID-only client ignores the
  `scs:` triples; a CostFed/Odyssey-class source-selector gets sharp star/multi-join
  cardinalities). Generated by `sparq-introspect`'s `Introspection::to_void_with_cs`
  (`Introspection::to_void` remains the CS-free base).
- `GET /sparql` **with no `query` parameter** — a [SPARQL 1.1 Service
  Description](https://www.w3.org/TR/sparql11-service-description/) generated from the server's
  **actual** capabilities (sq-qfcb), never a hard-coded fiction:
    - `sd:Service` + `sd:endpoint` (the request `Host`'s `/sparql`);
    - `sd:supportedLanguage` — `SPARQL11Query` always, **plus** the SPARQL-1.2-SD
      version-agnostic `SPARQLQuery` (sq-2msb); `SPARQL11Update` + `SPARQLUpdate` **only when an
      anonymous client can run one** (suppressed when a `--auth-token` write gate is configured,
      because then an unauthenticated SD reader cannot use Update);
    - `sd:supportedVersion` (sq-2msb, gh-917) — the SPARQL language **versions** this build
      conformance-verifies, as `sparql:version-*` IRIs (`http://www.w3.org/ns/sparql#`). SPARQL
      1.2 SD moves version negotiation off `sd:Language` onto `sd:supportedVersion`, so a
      1.2-aware federation client can discover triple-term / `dir`-lang support without probing.
      sparq advertises `version-1.0`, `version-1.1` and the **full** `version-1.2` (not the
      `version-1.2-basic` profile) because the engine passes the complete W3C SPARQL 1.0/1.1/1.2
      suites at 100% (`conformance-report.md`). **HONESTY GATE**: there is no `sparql12`/`rdf12`
      cargo feature — SPARQL 1.2 is compiled into the base engine — so this is keyed off the
      DOCUMENTED conformance state (`descriptors::CONFORMANCE_VERIFIED_VERSIONS`), never a `cfg!`
      or an aspiration; were any 1.2 group to regress to a partial pass, the honest edit is to
      drop to `version-1.2-basic` (or omit 1.2) in that one constant;
    - `sd:resultFormat` — the four SPARQL-results serialisations (JSON/XML/CSV/TSV) plus the RDF
      graph serialisations the CONSTRUCT/DESCRIBE/GSP-read path emits (Turtle/N-Triples/RDF-XML),
      and `sd:inputFormat` — the RDF serialisations the GSP write path parses (Turtle/N-Triples/
      RDF-XML). These mirror exactly what `crate::negotiate` produces/accepts; an integration test
      drives a real SELECT per advertised result format and asserts the returned `Content-Type`
      matches, so the advertisement cannot over-promise;
    - `sd:feature sd:BasicFederatedQuery` — **only** when the `service` cargo feature is compiled
      in (the `SERVICE` clause is then evaluable); omitted otherwise;
    - `sd:feature <http://sparq.dev/ns/prov#lineage>` (sq-yyy3) — the sparq W3C-PROV-O
      data-lineage extension, advertised **only** when this node genuinely serves PROV-O lineage
      for the data it derives (`Capabilities::provenance`). `sparq-server` exposes no lineage
      endpoint today, so it keeps the flag `false` and the feature is **omitted** — advertising it
      would over-promise. The descriptor + the `sparq-fedclient` discovery parser
      (`Capability::provenance_lineage`) support the round-trip so a lineage-serving node can flip
      it honestly without a vocabulary change;
    - `sd:extensionFunction` — one per function the engine has **actually registered**: the
      `geof:` GeoSPARQL set with the `geo` feature (read back through
      `FunctionRegistry::iris()`, so it can never drift from what runs), none without it;
    - the default dataset — its default graph linked to the VoID document via `dcterms:source`,
      **plus** an `sd:namedGraph` enumeration of every IRI-named graph in the served dataset
      (sq-optl): each is an `sd:NamedGraph` with its `sd:name` (the `FROM NAMED`-referenceable IRI)
      and an `sd:graph` `sd:Graph` carrying that graph's `void:triples` count. The names come off
      the same pinned snapshot the VoID descriptor reads, sorted for determinism, and **IRI-only**
      — a blank-node-named graph is skipped because it is not `FROM NAMED`-referenceable, so
      advertising it would be a fiction. A default-only dataset emits no `sd:namedGraph`.

  Note `sd:UnionDefaultGraph` is deliberately **not** advertised — the engine's default graph
  holds only default-graph triples (named graphs are not folded in), so claiming it would be
  dishonest.

Both are **double opt-in** and OFF by default: compiled only with the
`federation-descriptors` cargo feature **and** served only when
`--federation-descriptors` / `SPARQ_FEDERATION_DESCRIPTORS=1` is also set. Without the
feature there is zero cost (no `sparq-introspect` dependency, no routes); with the feature
but not the flag, `/.well-known/void` is `404` and a no-query `GET /sparql` is the historical
`400 missing 'query'`. Both content-negotiate the RDF syntax (`Accept: text/turtle` default,
`application/n-triples`, `application/rdf+xml`); reads are gated by `--auth-token-read` like
any GET. The dataset/endpoint IRIs self-describe the `Host` the client used.

```sh
cargo run -p sparq-server --features federation-descriptors -- data.ttl --federation-descriptors
curl http://127.0.0.1:3030/.well-known/void                         # VoID + scs: char-set stats (Turtle by default)
curl -H 'Accept: application/n-triples' http://127.0.0.1:3030/sparql # Service Description (no query)
```

**5c-bis. Grouped facet counts (OPT-IN, [GPT-5.6] `sq-lsp7k.5.2`; feature `facets`).**
`POST /facets` evaluates one `sparq_introspect::FacetRequest` against a pinned store snapshot and
returns its `FacetResponse` JSON: candidate-subject count plus ranked type, predicate, and requested
object-value distributions. `class` is an optional `rdf:type` class IRI; every `constraints` pair is
`[predicate IRI, object N-Triples term]` and the pairs are AND-combined; `facet_predicates: null`
requests values for every candidate predicate; `top_k` bounds every retained distribution.

```sh
cargo run -p sparq-server --features facets -- data.ttl --facets
curl -X POST -H 'Content-Type: application/json' http://127.0.0.1:3030/facets -d '{
  "class":"http://ex/Person",
  "constraints":[["http://ex/status","<http://ex/active>"]],
  "facet_predicates":["http://ex/tag"],
  "top_k":10
}'
```

The response is `200 application/json`; malformed JSON is a sanitized `400`. The scan runs on the
blocking pool and holds one immutable generation pin for the whole evaluation. The endpoint is a
READ, so `--auth-token-read` gates it. It is double opt-in and OFF by default: the `facets` cargo
feature compiles the route + `sparq-introspect` dependency, and `--facets` / `SPARQ_FACETS=1`
serves it. Feature on but flag off gives `404`; feature off compiles no route or dependency.

**5c-ter. IRI/label prefix completion (OPT-IN, [GPT-5.6] `sq-lsp7k.9.3`; feature
`complete`).** `GET /complete?q=<prefix>&limit=<k>` returns case-insensitive prefix matches from
complete IRI strings, IRI local names, and literal `rdfs:label` / `skos:prefLabel` values. `q` is
required (missing → `400`); `limit` defaults to 20 and is capped at 100. The response is a JSON
array in deterministic rank/key/id/source order:

```json
[
  {"iri":"http://ex/alice","key":"alice","kind":"localName","score":0.0},
  {"iri":"http://ex/alice","key":"alice","kind":"label","score":0.0}
]
```

`kind` is `iri`, `localName`, or `label`. One entity may appear more than once when multiple keys
or sources match. v1 passes no external rank scores, so `score` is `0.0`; PageRank-backed ranking
is a separate composition step. The server pins one immutable generation, lazily builds its
`CompletionIndex` on the blocking pool, caches it in `AppState`, and rebuilds when a later update
publishes a different generation. Thus a successful update is visible to the next completion
request without serving stale candidates.

```sh
cargo run -p sparq-server --features complete -- data.ttl --complete
curl -G http://127.0.0.1:3030/complete --data-urlencode 'q=ali' --data-urlencode 'limit=10'
```

The endpoint is a READ, so `--auth-token-read` gates it. It is double opt-in and OFF by default:
the `complete` cargo feature compiles the route and the pure-index `sparq-text` dependency
(`default-features = false`), while `--complete` / `SPARQ_COMPLETE=1` serves it. Feature on but
flag off gives `404`; feature off compiles no route, cache, or text dependency.

**5d. Triple Pattern Fragments / LDF source endpoint (OPT-IN, `sq-bzh1`).** A server can expose
itself as a low-cost [Linked Data Fragments](http://linkeddatafragments.org/) /
[Triple Pattern Fragments](https://www.hydra-cg.com/spec/latest/triple-pattern-fragments/)
**source** that a TPF client (Comunica / the LDF client) drives a join against — far cheaper per
request than a full SPARQL endpoint:

- `GET /tpf?subject=&predicate=&object=` — a **paged** RDF fragment of the triples matching one
  triple pattern. Each of `subject` / `predicate` / `object` is an **N-Triples term**
  (`<iri>`, `"lit"`, `"lit"@en`, `"lit"^^<dt>`); an absent / empty parameter is a variable
  (unbound). `page=N` (0-based) selects the page; the page size is bounded (default 100).
- The fragment carries **Hydra controls**: `hydra:totalItems` / `void:triples` (the matched-triple
  count, reusing the engine's cheap cardinality **estimate** — NOT a full scan),
  `hydra:itemsPerPage`, the full `PartialCollectionView` paging vocabulary —
  `hydra:next` / `hydra:previous` (present only when a next / previous page exists) plus
  `hydra:first` / `hydra:last` (emitted on EVERY page so a client can jump to either end of the
  view from anywhere; `first` is always page 0, `last` is the page holding the final match —
  derived from the same estimate as `totalItems`/`next`) — and a `hydra:search` / `hydra:template`
  / `hydra:mapping` control describing the `{subject,predicate,object}` URI template so a generic
  client can request any other pattern.

**5d-bis. brTPF — bind-restricted Triple Pattern Fragments (OPT-IN, `sq-dxhb`; feature `brtpf`,
implies `tpf`).** brTPF ([Hartig & Buil-Aranda, ODBASE 2016](http://olafhartig.de/files/HartigBuilAranda_ODBASE2016_Preprint.pdf))
extends the SAME `/tpf` endpoint so a client can attach a **set of solution mappings** and the
server returns only the page of pattern matches COMPATIBLE WITH AT LEAST ONE supplied binding —
pushing a bind-join's semi-join down to the source, so far less data crosses the wire than
re-fetching the whole pattern once per binding.

- The binding set rides the `values` query parameter (`GET`) or — preferred for a large set —
  the request **body** of a `POST /tpf`. The wire format is one mapping per line; within a line,
  whitespace-separated `position=term` pairs where `position` is `subject`/`predicate`/`object`
  (or short `s`/`p`/`o`) and `term` is the SAME N-Triples-term grammar as the pattern parameters
  (e.g. `s=<http://ex/alice>`). A blank line / empty payload is the no-restriction (plain-TPF)
  case; a malformed payload is a sanitized `400` (the offending input is NOT echoed).
- The fragment is the **deduplicated union** of each mapping's specialised-pattern matches.
  `hydra:totalItems` and the paging window reflect the bindings-RESTRICTED result, not the
  unrestricted pattern, and the `hydra:search` control advertises an extra `hydra:mapping` for the
  `values` variable so a client discovers the dataset accepts a restriction.
- A `tpf`-only build is **byte-identical** to before: the `values` parsing + the `POST` route are
  `#[cfg]`-stripped, so a stray `values` parameter is just an ignored unknown parameter (plain
  TPF). Still governed by the same `--tpf` runtime flag and read-auth (`POST /tpf` is a READ — it
  returns a fragment, it never writes).
- **DoS caps on the binding set (`sq-r74h`).** The brTPF fragment runs ONE index scan per
  attached mapping (`tpf::evaluate_brtpf`), so the per-request cost is super-linear in the mapping
  **count**, not the payload **bytes** — and `--max-body-bytes` bounds the count only transitively
  (a 1 MiB body of `s=<a>`-sized mappings is ~150k scans) and does NOT cover the `values`
  **query-string** carrier of a `GET /tpf` at all (it is a body limit). Two dedicated, ON-by-default
  caps close that: `--brtpf-max-bindings` (default `1024`) bounds the mapping count, and
  `--brtpf-max-values-bytes` (default `1 MiB`) bounds the raw `values` payload bytes — enforced
  BEFORE any parse/index work. A breach is a `413` (the same refusal class as `--max-body-bytes`,
  distinct from the malformed-payload `400`); the message names the cap, never the caller's input
  (no echo). `0` disables either cap. The pure parser is `tpf::parse_bindings_capped(payload,
  tpf::BindingLimits { max_mappings, max_payload_bytes })`, returning `tpf::BindingError::{Malformed
  → 400, TooLarge → 413}`.

```sh
cargo run -p sparq-server --features brtpf -- data.ttl --tpf
# restrict `?s ex:knows ?o` to a single subject via the `values` parameter
curl 'http://127.0.0.1:3030/tpf?predicate=%3Chttp%3A%2F%2Fex%2Fknows%3E&values=s%3D%3Chttp%3A%2F%2Fex%2Fcarol%3E'
# a larger binding set in a POST body (one `position=term` mapping per line)
curl -X POST -H 'Accept: application/n-triples' \
  --data $'s=<http://ex/alice>\ns=<http://ex/carol>' \
  'http://127.0.0.1:3030/tpf?predicate=%3Chttp%3A%2F%2Fex%2Fknows%3E'
```

**Double opt-in**, OFF by default and **READ-only** (no write path): compiled only with the `tpf`
cargo feature **and** served only when `--tpf` / `SPARQ_TPF=1` is also set (mirrors
`federation-descriptors`). Without the feature, zero cost (no route); with the feature but not
the flag, `/tpf` is `404`. Content-negotiates `Accept: text/turtle` (default) /
`application/n-triples` / `application/rdf+xml`; reads are gated by `--auth-token-read` like any
GET. The fragment/dataset/template IRIs self-describe the `Host` the client used.

```sh
cargo run -p sparq-server --features tpf -- data.ttl --tpf
# all triples with predicate ex:knows, page 0 (Turtle by default)
curl 'http://127.0.0.1:3030/tpf?predicate=%3Chttp%3A%2F%2Fex%2Fknows%3E'
# a fully-bound pattern, as N-Triples
curl -H 'Accept: application/n-triples' \
  'http://127.0.0.1:3030/tpf?subject=%3Chttp%3A%2F%2Fex%2Falice%3E&predicate=%3Chttp%3A%2F%2Fex%2Fknows%3E'
```

**5e. SHACL validation endpoint (OPT-IN, `sq-r868`, from-pss gh-162 follow-up; feature
`shacl`).** Validate the server's **currently-loaded data graph** against a SHACL **shapes**
graph the client POSTs — the server-side / large-graph path from gh-162 (the store is already in
memory, so there is no per-request data parse, and the 100k-node case where the JS
`rdf-validate-shacl` OOMs is handled natively by `sparq-shacl`).

- `POST /shacl/validate` — the request **body** is the SHACL shapes graph (RDF: `text/turtle` /
  `application/n-triples` / `application/n-quads` / `application/trig` / `application/rdf+xml`,
  classified by `Content-Type` like a GSP write body, and gzip-decoded under the same zip-bomb
  cap). The **data** graph is the server's pinned store snapshot.
- **Response, content-negotiated from `Accept`:** the default is the JSON projection PSS / the
  wasm `shacl` binding consume — `{ "conforms": bool, "results": [{ "focusNode", "path", "value",
  "sourceShape", "sourceConstraintComponent", "severity", "message" }] }`; `Accept: text/turtle`
  yields the W3C SHACL report-vocabulary graph (`sparq_shacl::ValidationReport::to_turtle`).
- Always `200` regardless of conformance — the verdict is in the body (`conforms`), not the HTTP
  status. A malformed shapes body is a `400`, an unsupported `Content-Type` a `415`, a non-`POST`
  method a `405`.
- Covers SHACL Core + SHACL-SPARQL (`sh:sparql`, §5.2) + custom SPARQL constraint components (§6)
  — whatever `sparq-shacl::validate` supports (it is the SAME engine the wasm binding and the
  `sparq-shacl` crate expose; see the `shacl-validation` skill). SHACL-AF `sh:rule` inference is
  not part of validation and is not run by this endpoint.

```sh
cargo run -p sparq-server --features shacl -- data.ttl --shacl
# validate the loaded store against POSTed shapes → JSON report (default)
curl -X POST -H 'Content-Type: text/turtle' --data-binary @shapes.ttl \
  http://127.0.0.1:3030/shacl/validate
# the W3C report vocabulary as Turtle
curl -X POST -H 'Content-Type: text/turtle' -H 'Accept: text/turtle' --data-binary @shapes.ttl \
  http://127.0.0.1:3030/shacl/validate
```

**Double opt-in**, OFF by default and **READ-only** (validation never mutates the store):
compiled only with the `shacl` cargo feature **and** served only when `--shacl` / `SPARQ_SHACL=1`
is also set (mirrors `tpf` / `federation-descriptors`). Without the feature, zero cost (no route,
no SHACL code — `sparq-core`/the wasm bundle are untouched); with the feature but not the flag,
`/shacl/validate` is `404`. Reads are gated by `--auth-token-read` like any GET.

**SHACL transaction guard (`sq-lsp7k.2.4`, same `shacl` feature; [GPT-5.6]).** Build with
`--features shacl`, load a shapes graph with `--shacl-shapes FILE` (env
`SPARQ_SHACL_SHAPES`), and enable `--shacl-guard` (env `SPARQ_SHACL_GUARD=1`) to validate the
post-state of every SPARQL Update and Graph Store write. A conforming candidate commits normally;
a non-conforming candidate is rejected with **`422 Unprocessable Entity`** and the same JSON
validation-report projection as `/shacl/validate`, and the published/durable store is unchanged.
Validation runs on the writer's private candidate before persistence and publish, so GSP
`PUT`/`POST`/`DELETE`/`PATCH`, template UPDATEs, and `/sparql` updates share the guard. The guard is
runtime-OFF by default and independent of the read-only `--shacl` endpoint flag. Enabling it
without a loaded shapes graph is a startup error. Library embedders set
`ServerConfig { shacl_guard: true, shacl_shapes: Some(ShaclShapes::new(shapes)),
..Default::default() }`.

```sh
cargo run -p sparq-server --features shacl -- data.ttl \
  --shacl-guard --shacl-shapes shapes.ttl
```

**5f. Terse transpiler endpoint (OPT-IN, `sq-vczh2`, epic `sq-2m6zm`; feature `terse`).** Transpile
a **terse** query (the `K:<name>` keyword layer over canonical SPARQL) into the **canonical,
conformant SPARQL** it expands to, returning the **verifiable expansion** — NOT an answer. This is
the LLM-ergonomic surface from `research/llm-ergonomic-sparql-surface.md` §4: the network contract
is the auditable expansion the agent can inspect, never an opaque oracle.

- `POST /terse/transpile` — the request **body** is the terse query text (read verbatim as UTF-8,
  gzip-decoded under the same zip-bomb cap). The server runs the LEAN `sparq_terse::terse_to_sparql`
  (the `K:<name>` keyword layer + the silent-rewrite canary) and returns JSON:
  `{ "canonical_sparql", "keywords": [{ "keyword", "iri", "legendVersion" }], "resolutions": [],
  "warnings": [], "legendVersion" }`.
- The whole **contract** is `canonical_sparql` — standard SPARQL the agent then runs through the
  normal `/sparql` path. The endpoint **never executes** the query and never reads the store.
- **Loud-fail**, never a silent guess: an unknown `K:<name>` keyword, a `PREFIX K:` collision, or
  non-conformant input (the canary) is a `400` carrying the transpiler's own message; a non-`POST`
  method is a `405`.
- `resolutions` is always `[]` in this server build: `V("phrase")` concept resolution needs a
  graph-bound resolver + an embedder (the crate's `vectors` feature), a **future** extension, so a
  `V(...)` construct is a `400` here rather than guessed. **Caveat (`sq-26fdp`):** the lexical
  linker can loud-fail a phrase that IS a verbatim `skos:prefLabel` when it shares a token with a
  sibling — a known soundness-conservative behaviour, fix tracked separately.

```sh
cargo run -p sparq-server --features terse -- data.ttl --terse
# expand the K: keyword layer to canonical SPARQL (the agent then runs canonical_sparql at /sparql)
curl -X POST --data-binary 'SELECT ?s WHERE { ?s K:derivedFrom ?o }' \
  http://127.0.0.1:3030/terse/transpile
# → {"canonical_sparql":"SELECT ?s WHERE { ?s <http://www.w3.org/ns/prov#wasDerivedFrom> ?o }",
#    "keywords":[{"keyword":"derivedFrom","iri":"http://www.w3.org/ns/prov#wasDerivedFrom",
#    "legendVersion":"pkg-keywords/v1"}],"resolutions":[],"warnings":[],"legendVersion":"pkg-keywords/v1"}
```

**Double opt-in**, OFF by default: compiled only with the `terse` cargo feature **and** served only
when `--terse` / `SPARQ_TERSE=1` is also set (mirrors `shacl` / `tpf`). Without the feature, zero
cost (no route, no terse code — `sparq-core`/`sparq-engine`/the wasm bundle untouched); with the
feature but not the flag, `/terse/transpile` is `404`. Transpiling is query-shaped, so it is gated
by `--auth-token-read` like a GET. The CLI exposes the same transpiler as `sparq-cli terse` (see the
`cli` skill).

**5g. Named parameterized SPARQL templates (OPT-IN, `sq-lsp7k.10`; feature `templates`).
[FABLE-5]** Server-stored, IRI-identified query/UPDATE templates with **typed, fail-closed
parameter binding** — the GraphDB "SPARQL templates" (smart updates) / Stardog "stored
queries" parity surface, and what an app backend or LLM agent should call instead of
composing free-form UPDATE strings.

- `GET /templates` — list every stored definition (read-gated). `GET /templates/{name}` —
  one definition; the `{name}` segment is a wildcard capture, so IRI names work verbatim.
- `PUT /templates/{name}` — store/replace (**write-gated**); the JSON body is
  `{ "text" | "sparql", "parameters": { name → auto|iri|string|boolean|integer|decimal|double|<datatype-IRI> },
  "description"? }`. **Fail-closed registration**: unparseable text, a declared parameter
  that is not a free bindable placeholder (e.g. a literal type in a predicate slot), or a
  result/aggregate/BIND output variable is a `400` — a stored template is always invocable.
  `DELETE /templates/{name}` — remove (**write-gated**).
- `POST /templates/{name}` — **invoke** with a JSON argument object. Binding is the #901
  `params` **algebra rewrite** (`sparq_engine::templates`): values become typed terms inside
  the parsed AST, never string concatenation, so a hostile value cannot inject syntax.
  **Fail-closed invocation**: an unknown/missing/mistyped argument is a `400`. SELECT/ASK →
  SPARQL-JSON; CONSTRUCT/DESCRIBE → N-Triples; an **UPDATE template is write-gated and runs
  through the SAME sequenced-writer path (`run_update`) as a `/sparql` update** — budgets,
  atomicity, durability and the gated-update posture are identical.
- `--templates-file PATH` (env `SPARQ_TEMPLATES_FILE`) makes the store durable: loaded
  fail-closed at startup (a corrupt file is a startup ERROR, never a silently-empty store),
  rewritten atomically (write-temp + rename) on every successful `PUT`/`DELETE`.

```sh
cargo run -p sparq-server --features templates -- data.ttl --templates
curl -X PUT -H 'Content-Type: application/json' http://127.0.0.1:3030/templates/friends \
  -d '{"text":"SELECT ?f WHERE { ?who <http://ex/knows> ?f }","parameters":{"who":"iri"}}'
curl -X POST -H 'Content-Type: application/json' http://127.0.0.1:3030/templates/friends \
  -d '{"who":"http://ex/alice"}'          # → SPARQL-JSON, typed + injection-safe
```

**Double opt-in**, OFF by default: compiled only with the `templates` cargo feature **and**
served only when `--templates` / `SPARQ_TEMPLATES=1` is also set (mirrors `shacl`/`tpf`/
`terse`); without the flag every `/templates` path is `404`. The MCP surface exposes the same
template layer as `template_invoke` (see the `agent-tools` skill).

### Solid WAC/ACP authorization endpoints (`solid-authz` feature, default OFF; `sq-snopa.6`, issue #992 FR-4)

A **thin, fail-closed HTTP shell** over the [`sparq-solid`](../../crates/sparq-solid) library
authoriser — the deliberately-opt-in `sparq-server` → `sparq-solid` workspace dependency. All three
endpoints are **POST** and take the pod dataset (N-Quads, incl. the `.acl`/`.acr` control graphs)
plus an **already-resolved** session in a JSON body — or, with `"source":"server"` (sq-snopa.8, see
below), the **server's own loaded store** instead of a body dataset.
The server does **NOT** authenticate — mapping a
WebID + a request path is the caller's job (`sparq-solid` is a library authoriser with no HTTP
surface, `research/sparq-solid-scope.md` §4); this is exactly that missing shell. `view` (`"wac"` /
`"acp"`) selects the model, else it is inferred (`.acr` present → ACP, else WAC). See also the
`access-control` skill for the library `decide` / `wac_allow` / `query_as` surface underneath.

- `POST /authz/decide` — body `{ "dataset", "session": { "agent"?, "client"?, "issuer"?, "now"? },
  "resource", "mode": "read"|"write"|"append"|"control", "view"? }`. Returns
  `{ "allow", "grantedModes", "governingAcl", "scope", "status", "aclLink" }`. An **allow** is `200`;
  a **deny** maps the FR-6 status — a definitive one (`resolved` without the mode / `noAcl`) is `403`,
  a retryable one (`unloaded` / `transient`) is `503`. When a governing ACL was discovered the
  response also carries `Link: <acl-iri>; rel="acl"` (RFC 8288, FR-5, sq-snopa.7) — the `aclLink`
  body field holds the same value. Fail-closed: no Link header when no governing ACL exists.
- `POST /authz/wac-allow` — body `{ "dataset", "session", "resource", "view"? }` → `{ "wacAllow":
  "user=\"…\",public=\"…\"" }`, the RFC permission advertisement. Also emits `Link: <acl-iri>;
  rel="acl"` when a governing ACL was discovered (FR-5, sq-snopa.7); omitted when none.
- `POST /authz/query` — body `{ "dataset", "session", "mode"?, "query", "view"? }` runs an
  **access-controlled** SPARQL query as the session and returns SPARQL-results JSON; a grant-less
  session sees ZERO rows (empty view), never the whole store.
  - **ODRL lane** (`odrl-authz` cargo feature, default OFF; sq-lrtc3.1 + sq-3mu76): when the
    request dataset carries ODRL policy rules (`odrl:permission`/`odrl:prohibition`), the handler
    parses them from the UNION of the dataset's graphs and runs the `sparq-solid` bridge
    (`PodStore::materialize_odrl_policy`, BOTH sides, deny-overrides) for
    (party = session agent, action = `odrl:read`, target = each rule's target graph) BEFORE the
    query — an ODRL prohibition beats a static WAC grant through the unchanged
    `∪ allow ∖ ∪ deny` enforcement. The lane runs on **all three endpoints** (sq-3mu76): the
    advisory `/authz/decide` and `/authz/wac-allow` never report an allow `/authz/query` would
    refuse to honour, and their advertisements are **read-scoped** while the lane evidences only
    `odrl:read` — `grantedModes` is masked to the requested mode, `wacAllow` carries at most
    `user="read"` with `public=""` (anonymous is refused wherever the lane fires). Fail-closed 4xx
    refusals (never a silent allow): malformed
    policy, unimplementable `odrl:conflict` strategy, a rule without a concrete target graph IRI
    (pattern targets = sq-lrtc3.3), a non-read `mode` (query-action contract = sq-lrtc3.2), an
    anonymous session, or a `trust` block combined with an ODRL-carrying dataset on `/decide`
    (the trust dispatch would bypass the lane). Constraints the stateless lane cannot evidence
    (`odrl:purpose`, `odrl:count` — stateful budgets remain unimplemented) never grant. The lane
    reads the request BODY dataset, so it does not apply to `"source":"server"`; an ODRL-carrying
    server store is REFUSED there rather than un-enforced (see `"source"` below). `/decide` stays
    governing-ACL-scoped: with no discoverable `.acl`/`.acr` it returns its `noAcl` deny even
    where a bridged grant lets `/authz/query` see rows (a deny-side-only divergence). Feature OFF
    ⇒ byte-identical `solid-authz` behaviour (the standard build never compiles `sparq-policy`).

**FAIL-CLOSED (the soundness invariant):** every error path DENIES — an unparseable dataset is a
`400` (never an empty-dataset allow), a materialisation failure a `503`, an unknown mode a `403`
deny, never a grant. **Double opt-in**, OFF by default: compiled only with the `solid-authz` cargo
feature **and** served only when `--solid-authz` / `SPARQ_SOLID_AUTHZ=1` is set (mirrors `shacl` /
`terse`); with the feature but not the flag, `/authz/*` is `404`. Read-gated by `--auth-token-read`.

**Which pod: `"source"` (`sq-snopa.8`).** Every endpoint takes an optional `"source"` field:

- `"body"` (**default**, and the only mode before sq-snopa.8) — **stateless**: the pod dataset
  arrives in `"dataset"` and dies with the response. Unchanged behaviour.
- `"server"` — **stateful**: authorise over the **server's own loaded store**. The body must NOT
  carry a `"dataset"` (a body naming both pods is ambiguous → `400`). A pod resource IS a named
  graph of the loaded store, so `"resource"` must be that graph's absolute IRI; relative paths are
  not resolved. The pinned current generation is forked, materialised once, and cached in
  `AppState` keyed by the **generation number** — so the N3 materialisation is paid once per
  generation, not once per request, and **an `.acl`/`.acr` write re-materialises automatically**
  (every commit publishes a new generation, which makes the cached view stale by construction;
  there is no window in which a revoked grant is still served).

The stateful lane is fail-closed on the same terms, and additionally **refuses** (never silently
un-enforces) two combinations it cannot evaluate faithfully: under `odrl-authz`, a server store
carrying ODRL policy rules (`400` — dropping a prohibition would be fail-OPEN); under
`solid-authz-trust`, a request carrying a `"trust"` block (`400` — that extension decides over its
own body-derived store).

```sh
curl -s -X POST http://127.0.0.1:3030/authz/decide -H 'Content-Type: application/json' -d '{
  "source": "server", "session": { "agent": "https://alice.ex/card#me" },
  "resource": "https://pod.ex/notes/n1", "mode": "read", "view": "wac" }'
```

```sh
cargo run -p sparq-server --features solid-authz -- data.ttl --solid-authz
curl -si -X POST http://127.0.0.1:3030/authz/decide -H 'Content-Type: application/json' -d '{
  "dataset": "<https://pod.ex/n1#it> <https://ex.dev/ns#t> \"hi\" <https://pod.ex/n1> .\n<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .",
  "session": { "agent": "https://alice.ex/card#me" }, "resource": "https://pod.ex/n1", "mode": "read", "view": "wac" }'
# HTTP/1.1 200 OK
# link: <https://pod.ex/.acl>; rel="acl"
# → {"allow":true,"grantedModes":["read"],"governingAcl":"https://pod.ex/.acl","scope":"http://www.w3.org/ns/auth/acl#default","status":"resolved","aclLink":"<https://pod.ex/.acl>; rel=\"acl\""}
```

**6. Hardening — flags / env / library.** Each flag overrides its `SPARQ_*` env var; the
env overrides the default.

| Flag | Env | Default | Effect |
| --- | --- | --- | --- |
| `--query-timeout SECS` | `SPARQ_QUERY_TIMEOUT` | `30` (`0`=off) | per-request timeout → `503` |
| `--update-where-timeout SECS` | `SPARQ_UPDATE_WHERE_TIMEOUT` | unset (`0`/unset = use `--query-timeout`) | **separate, typically-SHORTER writer-side WHERE deadline for SPARQL UPDATE** — bounds writer-queue **head-of-line blocking** from a slow update (the single sequenced writer is released within this window instead of holding it for the full read timeout); the update WHERE budget is `min(query_timeout, update_where_timeout)` → slow update `503` (`sq-nulp`) |
| `--no-adaptive-commit` | `SPARQ_ADAPTIVE_COMMIT` (`0` disables) | adaptive **on** | **adaptive group-commit**: a serial interactive client commits in engine-time (µs) instead of paying a fixed group-commit window; concurrent load still batches. Pure latency change — FIFO/atomicity/durability unchanged. `--no-adaptive-commit` restores the always-windowed writer (`sq-p7kk5`) |
| `--max-body-bytes N` | `SPARQ_MAX_BODY_BYTES` | `1048576` | body cap → `413` |
| `--max-concurrent N` | `SPARQ_MAX_CONCURRENT` | `32` | in-flight cap, load-shed → `429` |
| `--header-read-timeout SECS` | `SPARQ_HEADER_READ_TIMEOUT` | `15` (`0`=off) | **slow-loris guard**: max time a connection may take to send its complete request-header block — enforced at hyper's HTTP/1 connection layer by `sparq_server::serve` (NOT `axum::serve`, which never installs a timer so its header deadline is inert), so it fires BEFORE a handler and frees the concurrency slot a dribbling client would otherwise hold forever; connection closed when exceeded (`sq-2gqr`) |
| `--body-read-timeout SECS` | `SPARQ_BODY_READ_TIMEOUT` | `30` (`0`=off) | **slow-body guard** (complement to `--header-read-timeout`): idle deadline between consecutive request-**body** reads — applied by `sparq_server::serve` via a `tower_http` `RequestBodyTimeoutLayer`. Closes the slow-**body** dribble a complete-header client could otherwise use (declare a large `Content-Length`, send a few bytes, stall) to hold the slot under `--max-body-bytes` and before `--query-timeout` starts. The timer RESETS after each received chunk, so an honest large upload is not penalised — only an idle stall is; body read fails → request aborted (`sq-lodb`) |
| `--max-results N` | `SPARQ_MAX_RESULTS` | unlimited (`0`=off) | result/solution cap (SELECT + CONSTRUCT/DESCRIBE WHERE-solutions + EXPLAIN ANALYZE; not ASK/GSP-read/UPDATE) → honest `413` (not truncation) |
| `--max-query-rows N` | `SPARQ_MAX_QUERY_ROWS` | unlimited (`0`=off) | **memory cap** (coarse): working-set ROW ceiling on **every** form → honest `413` (`sq-ebii`) |
| `--max-query-bytes N` | `SPARQ_MAX_QUERY_BYTES` | unlimited (`0`=off) | **byte-accounted memory cap**: prices working-set row WIDTH (`rows × vars × id-size`) + computed-literal bytes on **every** form → honest `413` (`sq-s5is`) |
| `--max-decompress-ratio N` | `SPARQ_MAX_DECOMPRESS_RATIO` | `20` (`0`=refuse gzip) | **zip-bomb guard**: cap on decompressed:compressed for a `Content-Encoding: gzip` body → `413` (`sq-ebii`) |
| `--max-subscriptions N` | `SPARQ_MAX_SUBSCRIPTIONS` | `256` | server-wide subs |
| `--max-subscriptions-per-conn N` | `SPARQ_MAX_SUBSCRIPTIONS_PER_CONN` | `16` | per-socket subs |
| `--verbose` | — | off | TraceLayer request logging (respects `RUST_LOG`); request URIs **redacted by default** — see "Request-log redaction" |
| `--log-full-requests` | `SPARQ_LOG_FULL_REQUESTS` | off (redaction ON) | (`sq-toze.34`) OPT OUT of request-log redaction: log the raw request URI (incl. the full `?query=` SPARQL text) verbatim. Inert without `--verbose` — see "Request-log redaction" |
| `--auth-token TOKEN` | `SPARQ_AUTH_TOKEN` | off (no auth) | require `Authorization: Bearer TOKEN` on every WRITE (SPARQL Update + GSP PUT/POST/DELETE) → `401` + `WWW-Authenticate: Bearer` otherwise; constant-time compared (QLever's `-a`) |
| `--auth-token-read` | `SPARQ_AUTH_TOKEN_READ` | off | ALSO gate reads with the same token (only meaningful with a token set) |
| `--allow-remote` | `SPARQ_ALLOW_REMOTE` | off | opt in to a non-loopback bind; without it a non-loopback `--addr` is **refused** unless the surface is fully authenticated (`--auth-token` AND `--auth-token-read`), with it it warns and proceeds |
| `--service-allow HOST[:PORT]\|*.SUFFIX[:PORT]` (repeatable) | `SPARQ_SERVICE_ALLOW` (comma/ws-sep) | empty = **deny ALL SERVICE** | (feature `service`) allowlist a SERVICE egress host (exact or `*.suffix` wildcard); a `:PORT` makes it **port-scoped** — that host on THAT port only (else every port) (`sq-a7jw4`); CLI + file + env are all merged (combined additively) |
| `--service-allow-file PATH` | — | — | (feature `service`) load allowlist entries, one per line (`#` comments + blanks ignored) |
| `--cors-allow-origin ORIGIN` (repeatable) | `SPARQ_CORS_ALLOW_ORIGIN` (comma/ws-sep) | empty = **no CORS headers** | allowlist a first-party browser origin (`scheme://host[:port]`); a listed `Origin` is reflected into `Access-Control-Allow-Origin` (never `*`, never credentials) + `Vary: Origin`, preflight `OPTIONS` answered; CLI + file + env merged additively — see "CORS" |
| `--cors-allow-origin-file PATH` | — | — | load CORS origins, one per line (`#` comments + blanks ignored) |
| `--time-travel-generations N` | `SPARQ_TIME_TRAVEL_GENERATIONS` | `16` | (feature) retained generations |
| `--time-travel-max-age SECS` | `SPARQ_TIME_TRAVEL_MAX_AGE` | off | (feature) age-out window |
| `--federation-descriptors` | `SPARQ_FEDERATION_DESCRIPTORS` | off | (feature `federation-descriptors`) serve a VoID at `/.well-known/void` + a SPARQL Service Description on `GET /sparql` with no query — see "Federation discovery" |
| `--facets` | `SPARQ_FACETS` | off | (feature `facets`) serve `POST /facets` grouped type/predicate/value counts over a pinned snapshot; read-gated — see "Grouped facet counts" ([GPT-5.6] sq-lsp7k.5.2) |
| `--complete` | `SPARQ_COMPLETE` | off | (feature `complete`) serve `GET /complete?q=<prefix>&limit=<k>` IRI/local-name/label prefix completion from a generation-cached index; read-gated — see "IRI/label prefix completion" ([GPT-5.6] sq-lsp7k.9.3) |
| `--tpf` | `SPARQ_TPF` | off | (feature `tpf`) serve a Triple Pattern Fragments / LDF source endpoint at `GET /tpf?subject=&predicate=&object=` (paged, full Hydra paging incl. `first`/`last`, read-only); same flag also serves brTPF bind-restricted fragments (`values` param / `POST` body) when built with the `brtpf` feature — see "Triple Pattern Fragments" |
| `--shacl` | `SPARQ_SHACL` | off | (feature `shacl`) serve the SHACL validate endpoint `POST /shacl/validate` — POST a shapes graph, the server validates its loaded data graph against it; JSON report (default) or W3C report Turtle (`Accept: text/turtle`); read-only — see "SHACL validation endpoint" |
| `--shacl-guard` | `SPARQ_SHACL_GUARD` | off | (feature `shacl`) reject non-conforming UPDATE/GSP post-states with `422` + JSON validation report; store unchanged |
| `--shacl-shapes FILE` | `SPARQ_SHACL_SHAPES` | unset | (feature `shacl`) load the guard shapes graph once at startup; required when the guard is on |
| `--solid-authz` | `SPARQ_SOLID_AUTHZ` | off | (feature `solid-authz`) serve the Solid WAC/ACP authorization endpoints `POST /authz/decide`+`/wac-allow`+`/query` — a fail-closed HTTP shell over `sparq-solid`; POST the pod dataset (or `"source":"server"` for the server's own loaded store, sq-snopa.8) + an already-resolved session, get the decision / `WAC-Allow` value / access-controlled query result; read-only — see "Solid WAC/ACP authorization endpoints" |
| `--solid-authz-trust` | `SPARQ_SOLID_AUTHZ_TRUST` | off | (feature `solid-authz-trust`, implies `solid-authz`) opt-in stateless trust-graph extension to `POST /authz/decide` — a request may carry an additional `"trust"` JSON block containing credentials, a trust policy, and signed certification edges; the server runs the cert-graph closure (`derive_effective_rules`) and the `sparq_trust::admit` gate over them, injects any admitted facts into the pod dataset, then runs the unchanged WAC/ACP decision; double-opt-in: the feature must be compiled AND this flag set AND the request must carry a `"trust"` block — see "Stateless trust-graph decision extension (sq-pfae.17)"; honest scope: anchored-not-proven clear-path only (no ZK/unlinkability claim; sq-qhy4 external audit PENDING) |
| `--brtpf-max-bindings N` | `SPARQ_BRTPF_MAX_BINDINGS` | `1024` (`0`=off) | (feature `brtpf`) **DoS cap on the brTPF binding-set mapping COUNT** — one index scan per mapping, so cost is super-linear in the count, not the bytes → `413` (`sq-r74h`) |
| `--brtpf-max-values-bytes N` | `SPARQ_BRTPF_MAX_VALUES_BYTES` | `1048576` (`0`=off) | (feature `brtpf`) **DoS cap on the raw brTPF `values` payload BYTES** — bounds the GET query-string carrier that `--max-body-bytes` never sees → `413` (`sq-r74h`) |
| `--audit-log` | `SPARQ_AUDIT_LOG` | off | (feature `audit-log`) per-query **access audit log** — see "Access audit log" |
| `--access-audit <file\|stderr>` | `SPARQ_ACCESS_AUDIT` | off | (feature `access-audit`) richer **structured access-audit sink** (typed JSON-Lines: actor / action / resource / decision+basis / ts / fingerprint) — see "Structured access-audit sink" |
| `--audit-webid-header <name>` | `SPARQ_AUDIT_WEBID_HEADER` | off | (feature `access-audit`) **trust a fronting auth layer's forwarded WebID header** so the audit `actor` is the real authenticated subject (`webid:<iri>`) — set ONLY behind a trusted front (spoofable otherwise); see "Structured access-audit sink" |

In a library: `AppState::with_config(graph, ServerConfig { max_concurrent: 64, ..Default::default() })`
then `router(state)`, or `harden(my_router, &config)`.

### Server hardening — the DoS/SSRF limits (`sq-ebii` + `sq-4w18` + `sq-s5is`)

The threat model (a public, unauthenticated endpoint behind a gateway) calls for these
distinct limits. **Be precise about what each bounds** — only the body-size and ratio caps
are byte-hard; the timeout and both memory caps are *cooperative* (approximate in time), and
the memory caps are coarse working-set ceilings (row count / estimated bytes), not an RSS
quota:

1. **Query timeout** (`--query-timeout`, `SPARQ_QUERY_TIMEOUT`, default `30s`, `0`=off).
   The engine's cooperative `QueryBudget.deadline` stops the worker at its next coarse check
   (operator entry / per outer loop iteration); a wall-clock hard cap of `timeout + 2s` grace
   guarantees the HTTP `503` even if the engine is mid-stretch. Applies to **all** forms now
   — SELECT / ASK / CONSTRUCT / DESCRIBE / GSP-read **and** SPARQL Update
   (`application/sparql-update` + GSP `PUT`/`POST`/`DELETE`): the update path runs under the
   same cooperative budget on the writer thread *and* the same wall-clock await cap on the
   HTTP side. *Bounds:* wall-clock per request, approximately (next-check granularity).
   *Caveat:* updates are sequenced on a single writer, so a long update blocks the queue
   behind it until it finishes — this cap bounds the **client's own wait**, while the writer
   runs the WHERE to its cooperative stop. To bound that **head-of-line blocking** of the
   queue, set a separate, shorter WHERE deadline — see (1b).
   - **1b. UPDATE writer-side WHERE deadline / head-of-line bound**
     (`--update-where-timeout`, `SPARQ_UPDATE_WHERE_TIMEOUT`, default **unset** = use
     `--query-timeout`; `sq-nulp`). A SEPARATE, typically-shorter cooperative deadline applied
     ONLY to the WHERE phase of a SPARQL UPDATE on the writer thread: the update's budget
     deadline becomes `min(query_timeout, update_where_timeout)`, so a slow update releases the
     single sequenced writer within this window instead of holding it for the full (usually
     longer) read timeout — bounding how long one slow update can head-of-line block every
     queued update behind it. Cooperative, like the query timeout (next-check granularity); a
     tunable backstop, not hard preemption. Unset ⇒ the update WHERE budget is exactly
     `--query-timeout` (unchanged). The offending update gets a `503`.
2. **Memory cap** (`--max-query-rows`, `SPARQ_MAX_QUERY_ROWS`, default **off**, `0`=off).
   A coarse OOM circuit-breaker: an upper bound on the **row count** of any *materialised
   intermediate or final* result the engine builds, on **every** form (including
   CONSTRUCT/DESCRIBE and an UPDATE's `DELETE/INSERT … WHERE`), via the engine's
   `QueryBudget.max_rows` working-set bound. A join blow-up aborts with an honest `413`
   instead of OOMing; the speculative cross-product allocation is also capped up-front.
   *Bounds:* **cardinality (rows), not bytes.* Peak heap ≈ `rows × per-row term cost`, so a
   query with few but very wide rows (many vars / huge literals) can still exceed the implied
   memory; dictionary growth, sort/group scratch and a CONSTRUCT template are outside it. It
   is also approximate in time (coarse checks). Treat it as a blunt anti-OOM breaker, **not**
   an RSS quota. Distinct from `--max-results` (the result/solution cap, folded into the
   budget on SELECT / CONSTRUCT/DESCRIBE / EXPLAIN ANALYZE — but not ASK / GSP-read / UPDATE);
   on a path where both apply, the effective cap is the tighter of the two. For the row cap's
   width/literal blind spots, see the byte-accounted companion (2b); writer-queue
   head-of-line blocking from a slow UPDATE is deferred (`sq-nulp`).
2b. **Byte-accounted memory cap** (`--max-query-bytes`, `SPARQ_MAX_QUERY_BYTES`, default
   **off**, `0`=off; `sq-s5is`). The byte-accounted twin of (2): instead of counting ROWS it
   costs the estimated working-set BYTES — `rows × width × size_of::<Id>()` for each
   materialised intermediate (so it prices the WIDTH the row cap is blind to) PLUS the bytes
   of query-COMPUTED terms (BIND / aggregate / CONSTRUCT scratch interned into the per-query
   local vocab — the non-row allocations the row cap misses). Enforced via
   `QueryBudget.max_bytes` on **every** form (incl. an UPDATE's WHERE), at the same coarse
   cooperative sites, honest `413` on overflow. *Bounds:* the QUERY working set, estimated as
   a portable **lower** bound on real heap (ignores allocator overhead, `SmallVec`
   inline-vs-spill, and the pre-existing dictionary/index memory) — so it is strictly tighter
   and more width/literal-aware than the row cap, but still a coarse circuit-breaker, **not**
   an exact RSS quota. Composes with (2) and `--max-results`: whichever ceiling trips first
   aborts.
3. **Decompression-ratio cap** (`--max-decompress-ratio`, `SPARQ_MAX_DECOMPRESS_RATIO`,
   default `20`×, `0`=refuse gzip). When a GSP write body arrives `Content-Encoding: gzip`
   the server inflates it with a hard ceiling of
   `min(ratio × compressed_len, max_body_bytes)`, checked **during** inflate (bounded
   `Read::take`), and refuses with `413` the moment the decompressed output would cross it —
   so a tiny but pathologically compressible body cannot inflate into an OOM. `0` refuses
   gzip bodies outright (fail-closed). *Bounds:* the bodies **the server itself** inflates
   (GSP `PUT`/`POST`). An unknown `Content-Encoding` is a `415`. *Caveat:* it does **not**
   cover a compressed payload the *engine* fetches behind a SPARQL `LOAD <url>` / `SERVICE`
   — those use their own ingest; `SERVICE` egress is bounded separately by limit 4. The
   compressed bytes pass the `--max-body-bytes` gate first, and the decompressed ceiling is
   `min(ratio × compressed_len, max_body_bytes)` — so the decompressed output is itself capped
   at `max_body_bytes`, never `max_body_bytes × ratio`.
4. **SERVICE-SSRF egress allowlist** (`--service-allow` / `--service-allow-file` /
   `SPARQ_SERVICE_ALLOW`, default **deny ALL**, feature `service`) — shipped in `sq-4w18`,
   see "SERVICE federation (egress allowlist)" below. A `SERVICE <iri>` turns
   attacker-controlled query text into an outbound request from the server host (textbook
   SSRF; worst case the `169.254.169.254` cloud-metadata IP), so it reaches **nothing**
   unless its host is allowlisted, enforced on the *resolved* IP before any socket opens
   (DNS-rebinding-safe), uniformly across queries / ASK / CONSTRUCT/DESCRIBE / subscriptions
   / federated `INSERT … WHERE`. An entry may be **port-scoped** (`host:port`) to permit a
   host on exactly one port — strictly narrower than the bare host (`sq-a7jw4`).

Library callers set these on `ServerConfig` (`query_timeout`, `max_query_rows`,
`max_query_bytes`, `max_decompress_ratio`, `service_allow`). Embedders driving the engine
directly thread a `sparq_engine::QueryBudget { deadline, max_rows, max_bytes, ..QueryBudget::unlimited() }` into
`*_with_budget` query entry points and `update_in_place_with_budget`, and wrap calls in
`sparq_engine::with_service_egress_policy(strict, [host], || …)`.

### CORS — off by default; opt-in first-party origin allowlist (`sq-o7o0`, ASVS V14.5.3)

`sparq-server` is a SPARQL **data API**, so by default it emits **no CORS headers** — a
cross-origin browser `fetch` cannot read its responses (the safe posture). Do nothing and you
keep that. For a **first-party** browser app on another origin, allowlist its exact origin(s):

```sh
cargo run -p sparq-server -- --cors-allow-origin https://app.example.org data.ttl
SPARQ_CORS_ALLOW_ORIGIN='https://app.example.org, http://localhost:5173' \
  cargo run -p sparq-server -- data.ttl
# or one origin per line: --cors-allow-origin-file ./cors-origins.txt
```

Each entry is an RFC 6454 origin `scheme://host[:port]`. CLI + `--cors-allow-origin-file` + env
merge additively (same precedence as the SERVICE allowlist). When non-empty, a middleware in
`harden()` reflects an **allowlisted** request `Origin` into `Access-Control-Allow-Origin` (+
`Vary: Origin`) and answers the `OPTIONS` preflight for it; an **un-listed** origin gets **no**
CORS header (browser blocks it). Deliberately conservative: **never** `Access-Control-Allow-
Origin: *`, **never** `Access-Control-Allow-Credentials`, a `*` entry is rejected at startup. It
is a **browser-read gate only** — it does *not* relax the Bearer-token auth gate, the bind
posture, the body limit, the SERVICE allowlist, or the row caps (an allowlisted browser still
needs the token to write). Library callers set `ServerConfig.cors_allow: CorsAllowlist`
(re-exported at the crate root).

### Access audit log — opt-in per-query audit trail (`sq-0bxp`, CDMC CD-2)

A **per-query access audit log** for compliance regimes that need a per-subject / per-query
trail (CDMC CD-2, ISO 27001 A.8.15, EU CRA logging) — distinct from the aggregate-only
`/metrics`. **Doubly opt-in:** compile with the `audit-log` cargo feature, then turn it on at
runtime with `--audit-log` (env `SPARQ_AUDIT_LOG=1`). Off (either), the module and every call
site are `#[cfg]`-stripped or short-circuited before any record is built — a request pays
essentially zero.

For each query / update / Graph-Store request the server emits **one** structured `tracing`
event under the dedicated `target: "sparq_server::audit"` (`tracing::info!`). Route it to your
sink with the standard `RUST_LOG` machinery, independently of the `--verbose` request log:
`RUST_LOG=sparq_server::audit=info`. (`--audit-log` installs a subscriber on its own if
`--verbose` did not.) Fields:

| field | meaning |
| --- | --- |
| `requester` | `anonymous`, or `token:<fnv1a-hash>` of the presented Bearer token — **never the raw token** |
| `op` | `query` / `update` / `graph_read` / `graph_write` (operation class, keyed on whether it mutates) |
| `fingerprint` | FNV-1a hash (hex) of the trimmed query/update text — **not the query text** |
| `decision` | `allowed` or `denied` |
| `reason` | denial reason (`auth: missing or invalid Bearer token`); empty when allowed |
| `status` | the HTTP status the client saw (`200` / `401` / `413` / …) |
| `rows` | result-row count when known (else absent — `status` is the authoritative outcome) |
| `duration_us` | handler wall-clock in microseconds |

**No-PII / no-info-leak posture (reuses the #241 lesson):** this is a **server-side** log under
the operator's control — it is NEVER written to the HTTP response. It deliberately does **not**
log the full query text (raw SPARQL can disclose loaded-data fragments or caller PII — the #241
contract) nor the Bearer secret; only a stable, non-reversible fingerprint of each, enough to
correlate repeated identical queries / a recurring caller across requests and restarts. Library
callers set `ServerConfig { audit_log: true, .. }` (field present only with the feature). See
`crates/sparq-server/src/audit.rs`.

### Request-log redaction — keep query text out of the `--verbose` log (`sq-toze.34`, ON by default)

`--verbose` installs `tower_http::trace`, whose default span records the request **URI** — and
for `GET /sparql?query=…` the *full SPARQL query text is in that URI*, where it can carry PII (a
patient IRI, an email in a `FILTER`, a literal in `INSERT DATA`). Logging it verbatim leaks
sensitive content into operator logs.

**Redaction is ON by default** (always compiled — no feature gate): with `--verbose` the request
log keeps the URI *path* verbatim but replaces the *query string* with `?<redacted len=N fp=…>`
— a length signal + a stable, non-reversible FNV-1a fingerprint (the same construction the audit
log uses). Logs stay correlation-useful (same query → same `fp`) without exposing content.
**`--log-full-requests`** (env `SPARQ_LOG_FULL_REQUESTS=1`) opts OUT and logs the URI verbatim,
as the bare TraceLayer did — the deliberate debug escape hatch. Library: `ServerConfig {
verbose: true, redact_logs: true /* default */, .. }`.

**Default rationale:** enabling `--verbose` for debugging should not silently write
potentially-sensitive query text to disk / a SIEM; content-logging is the *deliberate* choice.
**Honest boundary — log-CONTENT redaction, not anonymity:** the log still records method,
path/endpoint, status, a size signal and timing (it would not be a request log otherwise), so
an adversary still learns *that* a request of roughly-this-size hit *this* endpoint at *this*
time and (via `fp`) that the same query recurred. That metadata is not erased. It is also NOT
the ZK/MPC privacy story — purely operator-log hygiene, complementary to error-body sanitisation
(`sq-kfel`/#241) and the audit fingerprint. See `crates/sparq-server/src/redact.rs`.

### Structured access-audit sink — opt-in pluggable JSON-Lines trail (`sq-gos8`, epic sq-toze, ASVS V7 / ISO 27001 A.8.15 / CDMC CD-2)

A **richer, structured** sibling of the `audit-log` trail above, for compliance audit trails that
need a TYPED, self-describing access record per ENFORCED decision rather than a flat `tracing`
line. **Opt-in:** compile with the `access-audit` cargo feature, then configure a sink with
`--access-audit <file|stderr>` (env `SPARQ_ACCESS_AUDIT`; the literal `stderr` writes to stderr,
any other value is a file path). Off (no feature, or no sink configured), the module + every call
site are `#[cfg]`-stripped / short-circuited (`Option` check) — a request pays essentially zero.

It hooks the **real enforcement seam** (the same `auth_gate` that actually allows/denies the
request), so the recorded decision is the one the server enforced — never a claimed-but-
disconnected one. Each event is emitted through a pluggable **`AuditSink` trait** (the default
`WriterSink` writes one JSON object per line; heavy/external sinks — a SIEM client, an OTel
exporter — stay OUT of core, an embedder implements the trait and installs an
`Arc<dyn AuditSink>`). Record fields:

| field | meaning |
| --- | --- |
| `ts` | RFC-3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SS.mmmZ`) |
| `actor` | `anonymous`, `token:<fnv1a>` (Bearer fingerprint, **never the raw token**), or `webid:<iri>` (an authenticated WebID/agent IRI — recorded verbatim) |
| `action` | `query` / `update` / `graph_read` / `graph_write` |
| `resource` + `resource_kind` | the dataset (`/sparql`) or the named-**graph IRI** the request addressed (`named_graph`) |
| `decision` | `allow` / `deny` (the ACTUALLY-enforced outcome) |
| `policy_basis` | the enforcement reason (`bearer-auth: allowed` / `bearer-auth: missing or invalid token`) |
| `fingerprint` | FNV-1a hash (hex) of the trimmed query/update — **not the query text** (`-` for a GSP body) |
| `status` | the HTTP status the client saw |
| `duration_us` | handler wall-clock, microseconds |

**Privacy boundary — stated honestly:** an audit trail exists to record WHO accessed WHAT, so —
**by design, and unlike the request log** — this sink **records identities and resource IRIs**
(the actor + the named-graph IRI are first-class fields; that is the operator's deliberate opt-in
choice). What it does **NOT** record is query **CONTENT**: the query/update text is logged only as
its non-reversible `fingerprint`, never raw, because a query body can carry PII (a patient IRI in a
`FILTER`, an email literal) — the #241 / sq-toze.34 redaction posture. It does **not** double-log
the content the redaction work just protected. One line: **identities + resources are logged;
content stays fingerprinted.** Library callers set `ServerConfig { access_audit:
Some(SinkTarget::File(path)), .. }` (field present only with the feature). See
`crates/sparq-server/src/access_audit.rs`.

**Actor enrichment from a trusted front (`sq-ljfz`).** By default the `actor` is derived from the
local Bearer gate (`token:<fnv1a>` / `anonymous`) — `sparq-server`'s own auth is a single shared
secret, not per-user. To record the **real authenticated WebID** instead, run the server behind a
fronting authorization layer — `sparq-solid`, or any Solid/WAC reverse-proxy / identity gateway
(the "reverse proxy / gateway or sparq-solid" the bind warnings name) — that authenticates the
user (resolves their WebID from a WAC/ACP session) and forwards it in a request header, then name
that header with `--audit-webid-header <name>` (env `SPARQ_AUDIT_WEBID_HEADER`, library field
`ServerConfig { audit_webid_header: Some(name), .. }`). When set, a present non-empty value of that
header becomes `actor = webid:<iri>` (the audit attributes access to *who*, not to *which shared
secret*); empty/absent falls back to the Bearer gate, byte-identical to before. **Security:** a
forwarded header is client-spoofable, so this is honoured ONLY when an operator explicitly names a
trusted header — set it **only** when the server runs behind that trusted front (never exposed
directly to untrusted clients), and ensure the front sets/overwrites the header so a direct client
cannot inject an arbitrary WebID. With no trusted header configured (the default) any such header
is ignored.

**Composing the two audit sinks (`sq-bif.14`).** `audit-log` (a process-global `tracing` sink) and
`access-audit` (a per-`AppState` JSON-Lines file sink) can be compiled in and turned on at runtime
TOGETHER. They hook the SAME request handler, so a single request emits BOTH records. The
load-bearing invariant — verified by `tests/audit_composition.rs` (gated on
`--features audit-log,access-audit`) — is that the two independently-derived records AGREE on the
request's operation, the ACTUALLY-enforced decision, the HTTP status, and the NON-reversible query
`fingerprint` (both sinks share the same FNV-1a-over-trimmed-text fingerprint), and that the
redaction boundary holds in BOTH (neither writes the raw query text or the Bearer token, and the
fingerprint never leaks to the HTTP response body) for an allowed AND a denied request. This is a
redaction-posture invariant, not an unqualified security guarantee — the audit trail records
identities and resource IRIs by design (see the privacy-boundary note above).

## Gotchas / feature flags / prerequisites

- **Auth — optional Bearer write gate; loopback-by-default; non-loopback bind refused
  unless safe.** With no `--auth-token` the server is unauthenticated (read+write open to
  anyone who can reach the port) — the back-compat default. Set `--auth-token <TOKEN>` (env
  `SPARQ_AUTH_TOKEN`) to require `Authorization: Bearer <TOKEN>` on every WRITE (SPARQL Update
  + GSP `PUT`/`POST`/`DELETE`); `401` + `WWW-Authenticate: Bearer` otherwise (constant-time
  compared; QLever's `-a <token>`; missing-vs-wrong are indistinguishable). The classification
  keys on whether the request **mutates**, not the route — an Update smuggled through the
  query path is gated too. Add `--auth-token-read` (env `SPARQ_AUTH_TOKEN_READ=1`) to ALSO
  gate reads — INCLUDING the subscription transports (`/subscriptions` WS + `/subscriptions/sse`
  SSE, both a read surface), closing the prior read-auth bypass (bead `sq-cxk5`): the SSE GET
  takes the `Authorization: Bearer` header; the WS upgrade accepts that header OR (for browsers)
  a `Sec-WebSocket-Protocol: bearer.<token>` subprotocol. The binary binds
  `127.0.0.1:3030` by default and **refuses** a non-loopback
  `--addr` (incl. `0.0.0.0`/`::`) unless `--allow-remote` / `SPARQ_ALLOW_REMOTE=1` OR the
  whole surface is authenticated (`--auth-token` AND `--auth-token-read`); even then it warns.
  A write-token alone still leaves reads open, so it does NOT by itself make a remote bind
  safe. Deliver the token over TLS (terminate at a proxy). For per-user authz front it with a
  reverse proxy / gateway (or `sparq-solid`). The token is authentication, NOT a resource
  cap: it is orthogonal to the DoS caps. No rate limit; `--max-results` / `--max-query-rows`
  are unlimited by default — set the four hardening caps (timeout, memory, decompression-ratio,
  SERVICE allowlist; see "Server hardening — the four DoS/SSRF limits") **plus** a gateway
  rate limiter before exposing it (beads `sq-zcby`, `sq-o4qf`, `sq-ebii`, `sq-4w18`).
- **SERVICE federation (egress allowlist).** `SERVICE` is OFF in the default build (build
  with `--features service` to enable it). Even enabled, the server is **default-DENY-all
  SERVICE**: a `SERVICE <iri>` clause reaches **nothing** unless its host is allowlisted —
  via `--service-allow HOST` / `*.SUFFIX` (repeatable), `--service-allow-file PATH` (one
  entry per line), or `SPARQ_SERVICE_ALLOW` (comma/whitespace-separated). All three are
  combined additively (the CLI only ever widens the env baseline). Rationale: a `SERVICE`
  clause turns attacker-controlled query text into an outbound request from the server
  host (textbook SSRF; worst case the `169.254.169.254` cloud-metadata IP), so the
  network-exposed surface must opt in to every reachable host. Matching is
  case-insensitive against the SERVICE IRI authority; a `*.example.org` entry matches the
  apex `example.org` and any subdomain. An entry may be **host-level** (`192.0.2.10` — every
  port) or **port-scoped** (`192.0.2.10:8080`, `[::1]:8053`, `sparql.example.org:8443`,
  `*.example.org:443` — that host on THAT port ONLY, rejecting every other port) (`sq-a7jw4`):
  a port-scoped entry is strictly NARROWER — the way to permit exactly one ephemeral loopback
  endpoint for in-process SERVICE federation without re-opening the whole host. (This TIGHTENS
  earlier behaviour: a `host:port` entry used to have its port stripped and widen to every
  port; it now means EXACTLY that port. A port-LESS entry is unchanged — still every port.)
  Unlike the engine's standalone default (which lets
  public IPs through and only blocks private ones), the server is **strict**: even a public
  host must be on the allowlist. The allowlist applies uniformly to queries, ASK,
  CONSTRUCT/DESCRIBE, subscriptions and federated `INSERT … WHERE` updates, and is enforced
  before any socket is opened (DNS-rebinding-safe, on the *resolved* IP). The startup log
  prints the effective allowlist. A non-`SILENT` `SERVICE` that is refused is an honest
  **`403 Forbidden`** (`sq-iu0c`) — a *policy* refusal, distinct from the `500` an
  unclassified execution error gives, so clients and alerting can tell a blocked egress
  target apart from a real server fault (the refused host is sanitized out of the body and
  goes only to the server log). `SERVICE SILENT` still swallows the refusal to the empty
  relation (no `403`). Beads `sq-4w18` (this wiring), `sq-2v6f` (the engine SSRF
  filter), `sq-iu0c` (the `403` classification). Embedders that drive the engine directly use
  `sparq_engine::with_service_egress_policy(strict, [host], || …)` /
  `with_service_egress_allow([host], || …)`.
  - **Per-request / per-query egress override (`sq-9xoh`, feature `service`).** A multi-tenant
    or gateway deployment can make the reachable SERVICE host set depend on the *request* instead
    of the single static allowlist: set `ServerConfig::service_allow_override` to a
    `ServiceAllowOverride::new(|headers| …)` hook (a `Fn(&HeaderMap) -> Option<ServiceAllowlist>`,
    `Send + Sync`). It is invoked once per **read** request (SELECT / ASK / CONSTRUCT / DESCRIBE /
    EXPLAIN ANALYZE); returning `Some(allowlist)` substitutes that allowlist for that one request
    (installed in the same STRICT/allowlist-only mode — an empty returned allowlist therefore
    **denies all** SERVICE for the request), and `None` falls back to the static `service_allow`.
    `None` (the default) keeps the historical single-allowlist behaviour exactly. The hook can
    only narrow or substitute the host set — it can never relax the fail-closed posture. It is
    **not** applied to the SPARQL-Update writer path: updates are sequenced and group-committed on
    one shared writer thread with no per-request header context (batch-mates may carry different
    tokens), so federated `INSERT … WHERE` updates continue to use the operator's static
    `service_allow`.
- **Feature flags.** `server` (default-on) pulls axum/tokio/tower — the binary needs it
  (`required-features = ["server"]`). `?generation=N` pinning + the `Sparq-Generation`
  header are in the **default build** (bounded to the ring's concurrency-retention window —
  `sq-ci2d6`); `time-travel` (default **off**) only EXTENDS the retention window (the
  `--time-travel-generations` / `--time-travel-max-age` flags, `AppState::at`, and the wider
  `?generation=N` reach). `geo` (default **off**) installs sparq-geo's `geof:` GeoSPARQL
  functions on query/update/subscription paths; without it an unknown `geof:` IRI is a
  `500`. `federation-descriptors` (default **off**, `sq-d3d8`) pulls the light
  `sparq-introspect` crate and serves the OPT-IN VoID + Service-Description discovery
  endpoints (still gated at runtime by `--federation-descriptors`; see "Federation
  discovery"). `facets` (default **off**, [GPT-5.6] `sq-lsp7k.5.2`) uses the same light
  `sparq-introspect` dependency for `POST /facets`, still gated at runtime by `--facets`.
  `complete` (default **off**, [GPT-5.6] `sq-lsp7k.9.3`) pulls `sparq-text` with its default
  features disabled and serves `GET /complete`, still gated at runtime by `--complete`.
  Run feature tests: `cargo test -p sparq-server --features time-travel` / `--features geo` /
  `--features federation-descriptors` / `--features facets` / `--features complete`.
- **Named graphs are real (since conformance round 3).** The engine stores the FULL dataset
  — default graph + named graphs — so `GRAPH <g> { … }` / `GRAPH ?g { … }` evaluate, and a
  GSP graph resource (`?graph=<iri>` or the direct request URI) addresses a genuine named
  graph (no longer a default-graph alias). `FROM`/`FROM NAMED` re-scope the active dataset, and
  the protocol's dataset-override params are **applied** (sq-z33x), not just accepted:
  `default-graph-uri`/`named-graph-uri` synthesize the query's active dataset (replacing any
  in-query `FROM`/`FROM NAMED` per §2.1.4), and `using-graph-uri`/`using-named-graph-uri` re-scope
  an update's `WHERE` (per §2.2; combining with an in-update `USING`/`USING NAMED`/`WITH` is a
  `400`, as is a non-IRI graph value). Time-travel pinning is `/sparql` queries only (GSP
  read/write and subscriptions always operate on current).
- **Update operations.** Engine handles `INSERT DATA`, `DELETE DATA`, `CLEAR`/`DROP`/`CREATE`
  (DEFAULT / named / ALL), `LOAD`, and `DELETE/INSERT … WHERE` — over the default graph AND
  named graphs. A failing operation is refused with `400`, atomically (no partial effect
  published). `apply_update` **blocks** (group-commit + O(graph) fork) — never call it on the
  async runtime directly; the HTTP handler (and the GSP write verbs) already use
  `spawn_blocking`.
- **Multi-operation update bodies are accepted and applied ATOMICALLY (one request, one
  commit).** A SPARQL 1.1 Update request is a sequence of `;`-separated operations, and the
  endpoint takes the WHOLE body as ONE update (it is never split on `;`): e.g. a Solid PSS
  `putDocument` body — `DROP SILENT GRAPH <r> ; INSERT DATA { GRAPH <r> … ; GRAPH <parent>
  ldp:contains <r> }` — is one request → a single `204`, with the resource graph AND the parent
  containment triple either BOTH applied or (on any operation failing) NEITHER. The sequenced
  writer applies the body to a private fork and publishes only on full success, so a partial
  body is never visible (in-memory all-or-nothing). On `--persist` the whole body's resolved
  delta is committed as ONE fsync'd transaction-journal frame BEFORE the `204`, so a crash
  mid-body can never leave the parent `ldp:contains` desynced from the child graph it points at
  (the journal redoes the whole frame, or none of it, on `Graph::open`). (sq-ycle / gh-48; see
  `crates/sparq-server/tests/persist.rs::pss_combined_multiop_body_accepted_and_atomic` and
  `::invalid_second_op_leaves_no_partial_write`.)
- **GSP write created-vs-replaced status is advisory.** PUT/POST sample graph existence from
  the current generation to choose `201` vs `204`/`200`; the write itself is atomic on the
  sequenced writer regardless. An existing-but-empty named graph reads as absent (the engine
  has no separate "empty graph exists" bit outside an in-flight update), so it may report
  `201` on a write — this never affects correctness of the data, only the status code.
- **Durability — opt-in via `--persist DIR` (default in-memory).** With NO `--persist` the
  server is in-memory and updates are **lost on restart** (the back-compat default). Pass
  `--persist <DIR>` (env `SPARQ_PERSIST_DIR`) to make the on-disk index at `DIR` the durable,
  rebuildable source of truth (QLever's `--persist-updates`): every committed UPDATE — default
  graph AND named graphs — is write-ahead-logged + **fsync'd before the group-commit ack** (the
  `204`), so a process restart on the same `DIR` restores **all** prior updates with **no
  rebuild** (`Graph::open` replays the WAL). On startup an existing store at `DIR` is opened (its
  WAL replayed; any `DATA_FILE` seed ignored — the persisted store wins); an empty `DIR` is
  seeded from `DATA_FILE`. Library callers set `ServerConfig::persist_dir` and build with
  `AppState::try_with_config` (returns the durable-open error). Deferred hardening (beaded):
  byte-accounted durability metrics, graceful degradation on a *transient* disk error (today a
  durability failure refuses the write rather than losing it), and WAL-durable `CLEAR`/`DROP
  GRAPH <g>` of an existing named graph. (sq-7cxr / gh-44.) The fail-closed contract under a
  durable-write failure is regression-guarded over HTTP by `tests/persist.rs` via the
  `test-seams`-only seal seam (`AppState::with_config_inject_durable_failure`): a refused write
  returns `503` and is NEVER published, the writer thread survives, reads keep serving the last
  published snapshot, and a recovered write is durable across a restart — covered for a transient
  burst, a permanent jam, AND (sq-bif.14) an INTERLEAVED recover→fail→recover sequence whose final
  durable set is exactly the successful writes (no refused write resurrects).
- **Online backup/restore — opt-in `backup` feature (default OFF).** Build with
  `--features backup` to mount the WRITE-gated, POST-only `/admin/backup` (stream a
  self-describing snapshot artifact of the live store WITHOUT stopping the world) and
  `/admin/restore` (atomically install a store rehydrated from one, **fail-closed** on a
  corrupt/mismatched artifact). On an in-memory server the restore is RAM-only; on a `--persist`
  server it is `409` UNLESS you opt in with `?persist=true`, which writes the restore THROUGH to the
  durable dir (crash-safe, fail-closed) so it survives a restart (`sq-ft7u`).
  `--restore <FILE>` / `SPARQ_RESTORE` runs the same restore on start (bootstrap a fresh
  replica / PITR base); with `--persist` it is refused unless `--restore-persist` /
  `SPARQ_RESTORE_PERSIST=1` is set (then it writes through to the durable dir on start). DISTINCT from the offline `sparq-cli
  save` and from the `--persist` WAL. **At-rest encryption of the artifact is out of scope** —
  the body digest detects accidental corruption, not tampering. Without the feature the routes +
  the `--restore` setting are compiled out AND the serving core stays the plain `ring`/`writer`
  pair — the `ArcSwap<ServingCore>` that backs the atomic online restore is `backup`-gated, so the
  DEFAULT read path is byte-identical to before #941 (no extra atomic load when `backup` is OFF;
  sq-0g6g resolved in the lean direction). (sq-o5bi.)
- **Incremental change-stream / PITR — same `backup` feature (sq-bu1a).** The base backup above
  is the BASE half of point-in-time recovery; `/admin/backup/delta?from=N` streams an incremental
  **delta artifact** (distinct `SPARQ-BACKUP-DELTA` kind) — the quad-set change between a *retained*
  generation N and the current one, keyed off the generation/writer-seq range + the epoch vector at
  `to`. Recover to a chosen point by restoring the base then replaying the chain forward:
  `--restore base --restore-delta d0 --restore-delta d1 …` (oldest first; or `SPARQ_RESTORE_DELTA`).
  **Fail-closed** on a corrupt / out-of-order / gapped chain (import + full replay before any swap);
  `from` not retained is `410` (widen retention with `time-travel`). **Same-lineage only** — the
  chain must be the writer history of that base (the diff relies on lineage-stable blank-node
  labels). At-rest encryption out of scope, same as the base.
- **Time-travel memory cost is real.** Each retained generation is a full *logical* `Graph`
  snapshot, but not an independent copy of the store: today's `Graph::fork` shares the base's
  immutable storage structurally (the permutation indexes, planner stats, the dictionary's frozen
  base and the numeric/temporal caches are `Arc`-shared) and copies only the pending delta. So the
  extra memory a generation costs grows with how far it has diverged from the base it shares — and
  a compaction folds the overlay into a *fresh* base, after which generations retained from before
  it hold the older base resident on their own. Size `--time-travel-generations` conservatively —
  retain only as many generations as your deployment's memory budget allows. See
  [`research/concurrent-serving.md`](../../research/concurrent-serving.md) for architecture details.
- **Error bodies.** Every error is structured JSON `{"error": "..."}` with
  `Content-Type: application/json` (the `405` keeps its `Allow` header). POST query
  requires `Content-Type: application/sparql-query` or `application/x-www-form-urlencoded`
  (else `415`); a GET without `query=` is `400`. An **unmatched route** is a `404` with the
  categorised body `{"error":"not found"}` ([OPUS-4.8] sq-pj6u — previously the bare
  `{"error":""}`); the message is server-constructed and never echoes the requested path.
- **Transient vs permanent status contract (for retry classifiers — sq-r5bv / gh-50).** A retry
  classifier should treat **only `429` and `503` as transient** (a retry of the identical request
  may succeed): `429` is a concurrency shed (the request never ran), `503` is a query/UPDATE
  **timeout**, a durable-write refusal (write NOT applied), or a subscription-capacity refusal.
  Everything else is **permanent** for the identical request — `400`/`401`/`404`/`405`/`410`/
  `413`/`415` — and `500` is a defect (caught panic / unclassified internal error), not
  back-pressure. The trap a `5xx`-only classifier hits against sparq: a **`413` result/row cap is a
  PERMANENT honest refusal** (narrow the query / add `LIMIT`), not a transient signal and not a
  truncation. **Classify on the status code, not the body text** (bodies are sanitised generic
  classes — see the next bullet). There is **no `Retry-After`** header today. Full contract +
  rationale: the `sparq_server::status_contract` crate doc, asserted by `tests/status_contract.rs`.
- **The versioned HTTP wire contract ([FABLE-5] sq-fdurb / gh-1416, PSS ask).** The endpoints,
  params, media types, negotiation rules, status codes and error-body shape an **HTTP-only
  consumer** may rely on are enumerated as the **v1 wire contract** in
  [`docs/http-wire-contract.md`](../../docs/http-wire-contract.md) — frozen-vs-unstable
  partition, plus the wire-semver policy (breaking vs additive). Pinned end-to-end by the
  served-surface snapshot suite `tests/wire_contract.rs` (one direct test per documented
  endpoint behaviour / error class), so an accidental wire break fails CI. Status: **PROPOSED**
  — the freeze ratification is the maintainer's call, like the embedding freeze in
  [`docs/api-stability.md`](../../docs/api-stability.md).
- **Error bodies are sanitized — no information leak (sq-cz89 / sq-j9zs).** On the
  no-auth-by-default path an error body carries only a **stable, generic CLASS message**
  (e.g. `malformed query`, `malformed RDF body`, `malformed gzip body`,
  `query execution error`, `update failed: invalid SPARQL update`). It deliberately does
  **NOT** echo the caller's submitted query/UPDATE/RDF text, a fragment of the loaded RDF
  (parsers like `oxttl`/`spargebra` quote the offending token — that would confirm loaded
  triples), or any server-side filesystem path (e.g. a `--persist` mirror's path inside a
  transient durable-write `503`). The full detailed cause is preserved for the **operator**
  via the server-side `tracing` log (target `sparq_server`), surfaced only through the
  opt-in `--verbose` / `RUST_LOG` subscriber — never the HTTP response. Status semantics
  (the `400`/`413`/`415`/`503`/`500` classification) are unchanged; only the prose detail
  is withheld. Regression tests assert a sentinel token never appears in any error body.
- **mimalloc** is the binary's global allocator (matters under concurrent load).

## See also

- `serve` — the underlying `sparq-serve` generation-ring + sequenced group-commit writer
  (the concurrency primitives `router`/`AppState` wire up).
- `engine` — `sparq-engine` query/ask/construct/describe + `QueryBudget` the server drives.
- `cli` — the `sparq` command-line surface over the same engine.
- `geo` — the `geof:` GeoSPARQL extension functions enabled by `--features geo`.
- `core` — `sparq_core::Graph` (`Graph::load_str`) the dataset is loaded into.
