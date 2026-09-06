# WebAssembly build of the sparq Solid server on npm — feasibility + design (2026-07) [OPUS-4.8]

Status: **partially implemented architecture record, maintainer-requested** (epic `sq-6xasp`, P1).
[GPT-5.6] The reduced-scope wasm adapter, npm build tiers, and loopback Node host now ship; CI,
OIDC, and persistence remain separately tracked under the epic. This record gives an honest
feasibility verdict for compiling the sparq Solid server
(`sparq-lws-core`) — or its request-handling core — to `wasm32`, publishing it as
an npm package for `npx`/`npm` spin-up, and decomposes the realistic (reduced-
scope) path into disjoint child beads. It is verified against the checked-out code.

## 0. Feasibility verdict (read first)

**A wasm build of the FULL `sparq-lws-core` crate as-written is NOT feasible —
there are hard, unavoidable blockers. But the request-handling CORE (LDP verbs +
WAC access control over an in-memory store) has a clean, ALREADY-EXISTING seam,
so a REDUCED-SCOPE in-memory wasm demo published to npm is realistic and
MEDIUM-effort.** The decisive facts:

- The heavy architectural work is **already done**: the app is a pure
  `axum::Router` returned by `build_router(state) -> Router` (`app.rs:149`),
  decoupled from the listener; the LDP handlers are generic over `S: Store`
  (`ldp/handler.rs`) and call only `self.store.<method>().await` (grep of
  `handler.rs` for `tokio::spawn`/`spawn_blocking`/`tokio::net`/`tokio::time`
  returns **zero**); the `Store` trait has **in-memory doubles that are the
  default boot path** (`InMemorySparqClient` `store/sparq.rs:225`,
  `InMemoryBlobStore` `store/blob.rs:195`; `main.rs:174` default backend is
  `memory`). The engine core is **already wasm-proven** (`sparq-wasm` depends on
  `sparq-core`+`sparq-engine` and ships to wasm today).
- The blockers are concentrated in **transport + crypto + network-auth**, all in
  identifiable modules that a wasm profile excludes — not smeared through the
  handler logic.

**The one genuine hard blocker that constrains scope: `aws-lc-rs`** (the rustls
crypto backend, a C/asm library) **does not build for
`wasm32-unknown-unknown`.** It is reachable not only from TLS (`main.rs`,
`tls.rs`) but also from the **notifications** provider install (`notifications/mod.rs:248`)
and the **PoP session-key derivation** (`pop/sk/derive.rs:30`, HKDF/HMAC). So the
first wasm demo must **exclude** TLS + notifications + PoP-SK (all native-transport
concerns anyway) — which is fine, because in a Node host the HTTP listener and TLS
live in **JS**, not in the wasm module.

**Minimal working demo = in-memory store, no OIDC network, no TLS, no
notifications, no PoP.** That is a real, useful Solid pod (LDP CRUD + WAC) that a
developer can `npx` up in seconds.

## 1. The two brief premises — verified

- **"wasm32 can't run tokio's full runtime / native TLS / native fs"** —
  CORRECT. `sparq-lws-core`'s tokio features are
  `["fs","rt-multi-thread","macros","net","signal","sync","time"]`
  (`Cargo.toml:25`); `rt-multi-thread`, `net`, `signal`, `fs` are native-only on
  `wasm32-unknown-unknown`. Native TLS = `aws-lc-rs` (does not build for wasm).
  Native fs = only in `tls.rs` (PEM read, all in tests) — the in-memory store
  path touches neither `std::fs` nor `tokio::fs`.
- **"compile the REQUEST-HANDLING LOGIC over the embedded in-memory backend, JS
  host provides the listener + storage"** — CORRECT and the right shape. Refined:
  the JS host provides the **listener** (and optional persistence); the
  **storage** can stay **inside wasm** (the `InMemoryBlobStore`/`InMemorySparqClient`
  `HashMap`s live in linear memory) for the first demo, which is simpler than a
  JS-fs round-trip. A JS-fs / IndexedDB-backed `Store` is a clean follow-up
  because `Store` is already a swappable async trait.

## 2. The existing wasm pattern in the repo (reuse it)

Five wasm crates share one pattern — `sparq-wasm`, `sparq-reason-wasm`,
`sparq-text-wasm`, `sparq-rsp-wasm`, `sparq-shacl-wasm`:

- crate-type `["cdylib","rlib"]`, `publish = false`, `wasm-bindgen = "0.2"`,
  `wasm-bindgen-test = "0.3"` (dev). All deps `default-features = false` (no
  rayon — no wasm threads).
- Entry shape: a `#[wasm_bindgen]` struct with `#[wasm_bindgen(constructor)]` +
  camelCase methods (`js_name = queryQuads`, …), plus free functions. Synchronous,
  in-memory, single-threaded (`sparq-wasm/src/lib.rs:113`).
- `getrandom`: the workspace `.cargo/config.toml` sets
  `--cfg getrandom_backend="wasm_js"` + `+simd128` for all wasm32 builds; each
  crate adds `getrandom` under `[target.'cfg(target_arch="wasm32")'.dependencies]`.
  **Caveat:** `sparq-lws-core` uses **getrandom 0.2** (needs the `js` feature),
  not 0.3 (`wasm_js` cfg) — the `sparq-wasm` `getrandom02` shim
  (`sparq-wasm/Cargo.toml:70`) is the template.
- Build/publish: `js/package.json` scripts run
  `wasm-pack build ../crates/<crate> --target web --profile release-wasm`; npm
  scope is **`@sparq-org/sparq`** (`js/package.json:2`) (a second `@sparq-org` scope
  also exists). CI: `.github/workflows/js.yml` (gating wasm-pack build+test) and
  `.github/workflows/publish.yml` (OIDC trusted-publishing to npm + SLSA
  provenance, `--access public`). A new bundle adds a `build:lws-wasm` script +
  a `package.json`, mirroring `sparq-reason-wasm`.

## 3. Wasm-incompatibility audit of `sparq-lws-core` (verified)

| Dependency / concern | Verdict | Notes |
| --- | --- | --- |
| **tokio** `fs,rt-multi-thread,net,signal,time` | HARD (replace/exclude) | native-only. The listener/serve is in `main.rs` (excluded). `spawn_blocking` in `store/embedded.rs:166` and `spawn` in `store/reconcile.rs:563` must be cfg'd out. **Handlers themselves have no tokio primitives** — only `Store` `.await`, trivial on in-memory doubles. Needs `wasm-bindgen-futures` (or a sync entry) to drive the async trait. |
| **aws-lc-rs** (via rustls) | **HARD BLOCKER** | C/asm; does not build for wasm32. Reachable from `main.rs:253` (provider install), `notifications/mod.rs:248`, `pop/sk/derive.rs:30`. Exclude TLS + notifications + PoP-SK. |
| **axum-server** (`tls-rustls`) | HARD (out of core seam) | native listener; confined to `main.rs`/`tls.rs`, not in `build_router`. |
| **axum 0.8** (`ws`) | partial / cfg-gate | `Router`/extractors/handlers largely portable; `axum::serve`, `ws` upgrade, hyper server are native → excluded. The wasm entry calls handlers via the `Router` directly. |
| **solid-oidc-verifier** (`network`) | HARD (network feature) | `network` pulls hickory-resolver/hickory-proto + reqwest — native. The verifier **core** (`Verifier::verify`) is seam-based over in-memory doubles (`StaticJwksProvider`, `InMemoryReplayStore`, `auth.rs:39`). Reduced demo depends on the verifier **without `network`** (inject static JWKS) OR stubs auth. **Unverified risk:** whether the verifier's no-network core compiles to wasm32 (pulls sha2/base64/p256-ish crypto — likely OK, needs a spike). |
| **object_store 0.13** | drop (unused) | grep of `src/` for `object_store::` = **zero** real uses (doc-comments only). Real bytes → `InMemoryBlobStore`. |
| **embedded-sparq** (`sparq-core`+`sparq-engine`) | core wasm-OK, wiring not | engine builds for wasm (proven). BUT lws-core enables `sparq-core` with **`mmap`** (`store/embedded.rs:100` `Graph::open`) and runs engine calls through `spawn_blocking` — both native. For wasm: in-memory `Graph` (no mmap), synchronous engine call (as `sparq-wasm` does), or just use `InMemorySparqClient`. |
| **mimalloc** | exclude | vendored C global allocator, installed in `main.rs` only. |
| **redis / r2d2** (`redis-replay`) | exclude | opt-in, off by default. |
| **hyper / hyper-util** | exclude | HTTP client + transport hardening; embedded path doesn't need them. |
| **getrandom 0.2** | needs `js` feature | used in `overload.rs:167`, `rate_limit.rs:629`, `pop/sk/derive.rs:91`, blob-key mint. |
| **sha2, subtle, base64, url, itoa, bytes, http, thiserror, serde/serde_json, async-trait, pin-project-lite** | wasm-OK | pure-Rust auth_cache + response-build primitives. |

## 4. The clean seam (why this is medium, not a rewrite)

- **`build_router(state) -> axum::Router`** (`app.rs:149`) and
  `build_router_with_overload` (`app.rs:172`) return a pure `Router`; all
  listener/serve/TLS/signal handling lives in `main.rs`.
- **Handlers generic over `S: Store`** (`get_handler<S: Store>`, etc.,
  `ldp/handler.rs:660,973,1112,1277,1403`), calling only `self.store.<method>().await`
  and pure functions — runtime-agnostic.
- **WAC** (`authz/` — acl/wac/mode/wac_allow) is pure logic reading `.acl`
  through the same `Store`; works unchanged in wasm.
- **`Store` is a swappable `async_trait`** (`store/mod.rs:125`) with in-memory
  doubles that are **already the default boot path** — no new storage abstraction
  is needed for the demo.
- Module map of `src/` (~38k lines): `app.rs` (router), `ldp/` (verb surface,
  portable), `authz/` (WAC, portable), `auth.rs`/`auth_cache.rs` (verifier seam),
  `store/` (seam + in-memory doubles + embedded/http/blob backends),
  **native-only (exclude for the demo):** `notifications/` (tokio broadcast +
  aws-lc-rs), `pop/` (mTLS/DPoP-SK + aws-lc-rs), `overload.rs`, `rate_limit.rs`,
  `body_limit.rs`, `transport.rs`, `tls.rs`, `main.rs`, `nodelay.rs`,
  `redis_replay.rs`.

## 5. Design — the realistic path

### 5.1 Architecture

A **new crate `sparq-lws-wasm`** (crate-type `["cdylib","rlib"]`, `publish =
false`) that depends on `sparq-lws-core` via a **new `wasm` feature** on
`sparq-lws-core` that cfg-gates OUT the native runtime/transport/crypto/network
surface (tokio net/rt/signal/fs coupling, `axum-server`, `tls`, `pop`,
`notifications`, `mimalloc`, `object_store`, `redis`, and the verifier `network`
feature). The wasm crate exposes a `#[wasm_bindgen]` entry over a
`CompositeStore<InMemorySparqClient, InMemoryBlobStore>`.

Two viable entry shapes; the design **recommends (a)** to preserve the exact
routing/middleware semantics:

- **(a) Drive the existing `Router`** through a hand-built `http::Request` via
  `tower::Service::call` under `wasm-bindgen-futures` (async wasm methods). Keeps
  axum routing, content negotiation, conditional requests, WAC middleware — the
  server behaves identically to native, minus the excluded transport.
- **(b) A thinner `handle_request(method, path, headers, body) -> Response`** that
  calls the `*_handler` functions directly. Less faithful (bypasses the router's
  layer stack), only if (a) hits a wasm-bindgen-futures/async-trait snag.

The npm package is a **Node wrapper** exposing `startSolidServer(opts)` (and an
`npx @sparq-org/solid-server` bin) that boots a Node `http` listener and routes each
request into the wasm handler; storage is in-wasm-memory for v1. TLS, if wanted,
is terminated in Node/a proxy — never in wasm.

### 5.2 Auth for the demo

For the **first** demo, **disable/stub OIDC** (or inject a `StaticJwksProvider` +
`InMemoryReplayStore` with a pre-baked token) — **no network**. This is honest:
the demo is an **unauthenticated or static-token** Solid pod for local dev, not a
production IdP-verifying server. Real OIDC in wasm (JWKS fetch via JS `fetch`,
WebID resolution) is a **follow-up bead** contingent on the verifier's no-network
core compiling to wasm.

### 5.3 The hard blocker + the minimal working demo

**Hard blocker:** `aws-lc-rs` (and thus in-wasm TLS, notifications-provider,
PoP-SK) cannot compile to wasm32. **Minimal working demo that dodges it entirely:**
in-memory store, no TLS (Node/proxy terminates it), no notifications, no PoP, no
OIDC network → a Solid pod that round-trips LDP CRUD + WAC. This is a genuinely
useful artifact and the correct v1 scope.

### 5.4 Feature tiers — full vs core-Solid (maintainer-requested 2026-07-12)

Both the **native** Solid server and the **wasm/npm** distribution must be
buildable in **two tiers**, so the pure Solid protocol can be exercised in
isolation from the SPARQL query surface (cleaner, faster, smaller-surface
testing):

- **`full`** — LDP + WAC **plus the SPARQL query/update endpoint** (the `/sparql`
  route over the embedded engine). The default for the shipped server and the
  default npm bundle.
- **`core`** — **pure Solid protocol only**: LDP verbs + WAC + notifications where
  applicable, with the SPARQL endpoint route **compiled out**. No `sparq-engine`
  query endpoint surface; smaller wasm object; a protocol-conformance test target
  with no query-engine confound.

**Reality check (verified 2026-07-12 against `origin/main`):** `sparq-lws-core`
today exposes **only LDP + WAC** — `build_router` has **no `/sparql` route**, and
`Store` exposes LDP operations, not arbitrary SPARQL query/update. So the current
server **already IS the `core` tier**; the `full` tier is **net-new work** — a
SPARQL query endpoint must be *built* behind the flag, not merely gated off
existing code. Design for that endpoint (its own bead `sq-r1ei8`): a WAC-scoped,
**query-only-v1** `GET/POST /sparql` (SPARQL 1.1 Protocol) that evaluates SELECT/
ASK/CONSTRUCT via the embedded `sparq-engine` over **only** the pod resources the
authenticated agent has `acl:Read` on (WAC-scoped dataset assembly is the hard,
authz-soundness-sensitive part → Opus review). SPARQL UPDATE = a later follow-up.

**Mechanism — one shared cargo feature on `sparq-lws-core`:**
`sparql-endpoint` (**default-on** once the endpoint exists), gating the (net-new)
SPARQL endpoint route registration in `build_router` + its handler module.
It is **orthogonal** to the `wasm` feature (§5.1): the two compose into four
build points, of which we ship the useful three —

| Build | features | ships as |
| --- | --- | --- |
| native full | `sparql-endpoint` (default) | the `solid-server` binary (default) |
| native core | `--no-default-features` (or without `sparql-endpoint`) | `solid-server` core / a `core` CI test target |
| wasm full | `wasm,sparql-endpoint` | `@sparq-org/solid-server` (default bundle) |
| wasm core | `wasm` (no `sparql-endpoint`) | `@sparq-org/solid-server` **core** variant / a `build:lws-wasm-core` script |

INVARIANT: `sparql-endpoint` gates **route + handler only** — the `Store`'s own
SPARQL-backed methods (used internally by LDP/WAC where relevant) are unaffected;
LDP/WAC behaviour is byte-identical between tiers. Feature-**off** build must be
byte-stable vs the always-compiled surface (cf the feature-off-wasm-drift trap).
The npm package selects the tier at **build time** (two wasm artifacts), not at
runtime, to keep the core bundle genuinely smaller. `startSolidServer(opts)` in
the Node host loads whichever artifact the consumer installed; the bin advertises
the active tier in `--version`/startup log. This split is the acceptance surface
for cleaner testing: a `core`-tier smoke test asserts the `/sparql` route returns
404/is-absent while LDP CRUD + WAC still round-trip.

## 6. Recommendation

1. **Feasible only in reduced scope; ship that.** New `sparq-lws-wasm` crate +
   a `wasm` feature on `sparq-lws-core` gating out native transport/crypto/network;
   in-memory store; drive the real `Router` via `wasm-bindgen-futures`.
2. **De-risk with a spike FIRST** (bead 1): confirm (i) `solid-oidc-verifier`
   without `network` compiles to wasm32 (or decide to stub auth), and (ii) the
   async-trait `Store` path drives cleanly under `wasm-bindgen-futures` with no
   tokio reactor. This is the highest-uncertainty item; do it before committing
   the crate structure.
3. **Sequence:** spike → `wasm` feature/cfg-gating on `sparq-lws-core` → wasm
   entry crate → Node host + npm package → smoke test. Auth (real OIDC) and
   persistent storage (JS-fs/IndexedDB `Store`) are explicit follow-ups.
4. **Be honest in the README/package:** v1 is a local-dev, in-memory,
   (un)authenticated Solid pod — **not** a production server. No perf claims.

## 7. Phased plan (each phase → a future child bead of `sq-6xasp`)

Ordered; feasibility spike first. Each bead states {crate, model_tier, INVARIANT,
ACCEPTANCE TEST, depends_on}. cfg-gating soundness → `fable`; JS host + packaging
→ `gpt`. Bead IDs assigned at mint; sequence = build order.

1. **Feasibility spike: prove the wasm-critical unknowns compile/run.** (crate:
   throwaway/experimental; tier: `fable`.) INVARIANT: no change to shipped crates;
   findings captured as a bead note. ACCEPTANCE: a documented result answering —
   does `solid-oidc-verifier` (no `network`) build for `wasm32-unknown-unknown`?
   does the async-trait `Store` + a minimal handler drive under
   `wasm-bindgen-futures` without a tokio reactor? does `sparq-engine`'s
   in-memory `Graph` query path work in wasm (expected yes)? Pass = a written
   go/no-go with the exact deps that must be stubbed. depends_on: none.
2. **`wasm` feature on `sparq-lws-core` cfg-gating out native
   transport/crypto/network.** (crate: `sparq-lws-core`; tier: `fable`.)
   INVARIANT: default (native) build byte-identical — every gate is
   `#[cfg(not(target_arch = "wasm32"))]` / `#[cfg(feature = "wasm")]` around
   `main`/`tls`/`pop`/`notifications`/`transport`/`redis`/mimalloc/object_store +
   the `spawn_blocking`/`spawn` sites; the verifier `network` feature and
   `sparq-core/mmap` are off under `wasm`; getrandom `js`. ACCEPTANCE: native
   `cargo build`/`clippy -D warnings` unchanged (feature-off byte-identical);
   `cargo build -p sparq-lws-core --features wasm --target wasm32-unknown-unknown`
   compiles the router + LDP handlers + WAC + in-memory store (no aws-lc-rs, no
   tokio-net in the wasm object). depends_on: 1.
3. **`sparq-lws-wasm` crate: `#[wasm_bindgen]` entry over
   `CompositeStore<InMemorySparqClient, InMemoryBlobStore>`, driving the real
   `Router` via `wasm-bindgen-futures`.** (crate: new `sparq-lws-wasm`; tier:
   `fable` for the async/store wiring soundness.) INVARIANT: single-threaded,
   in-memory, no network; auth stubbed/static for v1 (documented). ACCEPTANCE:
   `wasm-pack build crates/sparq-lws-wasm --target web` succeeds; a
   `wasm-bindgen-test` issues a PUT then GET of an LDP resource against the wasm
   handler and the bytes round-trip; a WAC-denied request returns 401/403.
   depends_on: 2.
4. **Node host + npm package: `startSolidServer(opts)` + `npx @sparq-org/solid-server`
   bin routing a Node HTTP listener into the wasm handler; in-memory storage;
   `package.json` + `build:lws-wasm` script; wire into `js.yml`/`publish.yml`.**
   (crate: `js/` package + `sparq-lws-wasm` glue; tier: `gpt`.) INVARIANT: no TLS
   in wasm (Node/proxy terminates); package README states the honest v1 scope
   (local-dev, in-memory, (un)authenticated). ACCEPTANCE: `npm install` +
   `npx @sparq-org/solid-server` boots a listener; a `curl`/`fetch` PUT then GET of a
   Turtle resource round-trips; the CI `js` leg builds the bundle. depends_on: 3.
5. **Smoke test: install → spin up → a Solid LDP request round-trips (CI leg).**
   (crate: `js/` tests; tier: `gpt`.) INVARIANT: the smoke test runs in the
   existing `js.yml` matrix and does not gate the native workspace. ACCEPTANCE: a
   scripted `npx`-boot + LDP CREATE/READ/UPDATE/DELETE + a WAC check passes in CI;
   failure is visible on the PR. depends_on: 4.
6. **(Follow-up, non-blocking) Real OIDC in wasm via JS `fetch` JWKS/WebID
   adapters (contingent on bead-1 verifier-wasm result).** (crate: `sparq-lws-wasm`
   + `js/`; tier: `fable` for the auth-path soundness.) INVARIANT: default demo
   stays stub-auth; real-auth is opt-in and clearly scoped. ACCEPTANCE: a wasm
   build with a JS-`fetch`-backed `JwksProvider` verifies a real DPoP-bound token
   against a live IdP in a test. depends_on: 5 + a positive bead-1 verdict.
7. **(Follow-up, non-blocking) Persistent `Store` for wasm: a JS-fs / IndexedDB
   backend behind the existing `Store` trait.** (crate: `sparq-lws-wasm` + `js/`;
   tier: `gpt`.) INVARIANT: the in-memory store stays the default; persistence is
   opt-in. ACCEPTANCE: data survives a listener restart via the JS-backed store in
   a test. depends_on: 5.

## 8. Open questions for the maintainer

- **v1 auth stance:** OK to ship the first npm demo as **unauthenticated /
  static-token** (no OIDC network), with real OIDC as bead 6? This is the honest
  minimal-scope path around the aws-lc-rs blocker.
- **npm scope + name:** `@sparq-org/sparq` is the existing scope. Publish as
  `@sparq-org/solid-server` (proposed `npx` target) or under `@sparq-org`? I've
  written the plan against `@sparq-org/solid-server`.
- **Storage location for v1:** in-wasm-memory (simplest, proposed) vs a JS-fs
  `Store` from the start? I've proposed in-memory for v1, JS-fs as bead 7.
- **Verifier-wasm dependency:** bead 6 (real OIDC) is contingent on
  `solid-oidc-verifier` compiling to wasm without `network` — if it doesn't, real
  wasm OIDC needs upstream work on that crate (a separate track). The bead-1 spike
  settles this.
