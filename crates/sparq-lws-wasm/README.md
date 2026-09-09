<!-- [GPT-5.6] sq-6xasp.3: full-template README for the opt-in wasm Solid-server adapter. -->
# sparq-lws-wasm

An opt-in WebAssembly adapter for SPARQ's experimental Solid/LDP server core. It
owns a single-threaded in-memory pod and drives the real axum router as a Tower
service. The JavaScript host supplies the HTTP listener and authenticated WebID.

> This local-development adapter is not a production Solid server. Storage is
> in linear memory and ephemeral unless the host opts into snapshots; TLS and
> OIDC verification stay in the host; PoP, notifications, networking, and remote
> storage backends are excluded from wasm.

## 🚀 Quickstart

```sh
wasm-pack build crates/sparq-lws-wasm --target web
```

The `@sparq-org/solid-server` workspace package stages the generated JavaScript,
TypeScript declarations, and wasm binary in its own `wasm/` directory:

```sh
npm --workspace @sparq-org/solid-server run build:lws-wasm
npm --workspace @sparq-org/solid-server run build:lws-wasm-core
```

The first command enables the [GPT-5.6] `sparql-endpoint` passthrough and stages
the full LDP/WAC + query artifact. `build:lws-wasm-core` leaves that feature off,
so `/sparql` and the query-engine dependencies are absent from the core artifact.

```js
import init, { SolidServer } from "./pkg/sparq_lws_wasm.js";
await init();

const owner = "https://id.example/alice#me";
const pod = new SolidServer("https://pod.example", owner);
const response = await pod.handleRequest(
  "PUT",
  "/card",
  ["content-type", "text/turtle"],
  new TextEncoder().encode("<card> <name> \"Ada\" .\n"),
  owner,
);
console.log(response.status);
```

## ✨ Features

- `CompositeStore<InMemorySparqClient, InMemoryBlobStore>` keeps metadata and
  bytes inside wasm linear memory.
- `handleRequest(method, path, headers, body, authenticatedWebid)` constructs an
  HTTP request and invokes the existing LDP + WAC router. Headers are flat
  name/value arrays so repeated fields survive the wasm boundary.
- The constructor provisions one owner ACL. Each request is anonymous when
  `authenticatedWebid` is absent; otherwise the host must have verified that
  WebID before passing it to wasm.
- The default-off `sparql-endpoint` feature adds query-only GET/POST `/sparql`
  over the resources that the supplied WebID may read. SELECT/ASK return
  SPARQL-results JSON; CONSTRUCT returns N-Triples.
- [SONNET-4.6] A `console_error_panic_hook` is installed at module init so any
  Rust panic emits a diagnostic to `console.error` before the wasm `unreachable`
  trap propagates to the host as `WebAssembly.RuntimeError`.
- [SONNET-4.6] A bounded `#[global_allocator]` tracks live linear-memory bytes, and
  `handleRequest` refuses a request whose projected peak would cross the ceiling with
  a clean HTTP 507 instead of trapping at the linear-memory wall. `lwsMemoryLiveBytes`,
  `lwsMemoryPeakBytes`, `lwsMemoryCeilingBytes`, and `lwsSetMemoryCeilingBytes` expose
  the counters and the knob. The total is of live bytes rather than pages ever grown, so
  bytes returned to the allocator restore headroom — but that an LDP `DELETE` frees enough
  to re-admit a refused request is not yet demonstrated end to end.
- [GPT-5.6] Persistence is opt-in and lives behind the same `Store` trait.
  `SolidServer.withSnapshot(baseUrl, ownerWebid, bytes)` builds the pod behind a
  journaling store decorator; `snapshot()` returns the bytes the host writes to
  `node:fs` or IndexedDB, and handing them back to `withSnapshot` rebuilds the
  pod's contents after a listener restart. `new SolidServer(...)` is unchanged
  and journals nothing, so `snapshot()` is `undefined` for it. Restart preserves
  the content-derived `ETag` but re-stamps `Last-Modified`; the host owns the
  durable medium and the flush policy.
- No Tokio reactor, native listener, filesystem, TLS, OIDC verifier, PoP,
  notifications, or network backend is linked into the wasm artifact.

## 📚 Learn more

- Usage and host-authentication contract:
  [`skills/javascript-wasm/SKILL.md`](../../skills/javascript-wasm/SKILL.md).
- Portable request core: [`sparq-lws-core`](../sparq-lws-core/README.md), built
  with its default-off `wasm` feature.
- [GPT-5.6] The [`@sparq-org/solid-server`](../../packages/solid-server/) package owns
  the loopback Node listener and `npx` entry. Its fixed-owner mode is for local
  development only; OIDC verification remains outside this wasm crate.
- [SONNET-4.6] The Node host catches `WebAssembly.RuntimeError` and recycles the
  `SolidServer` instance before the next request (sq-250si). A single wasm trap no
  longer bricks the process; the triggering request receives HTTP 503. The bounded
  allocator (sq-wubkf) removes the *sustained*-pressure form of that trap ahead of
  time; a single request whose own transient peak overshoots the remaining headroom
  still traps and still relies on trap recovery.

## License

[MIT](../../LICENSE).
