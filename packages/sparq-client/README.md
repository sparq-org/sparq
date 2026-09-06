# @sparq/client

Framework-agnostic TypeScript client for the **sparq** WASM engine. This is the **single
shared entry point** for the `WasmStore` surface (the `crates/sparq-wasm` `Store` exposed over
wasm-bindgen) plus the loaders and query helpers around it.

It exists to remove the drift liability flagged in `research/gui-design.md` (§0 / §4): the
WASM `Store` TS interface was **hand-redeclared** inside `site/src/lib/sparq-wasm.ts`, so any
future GUI would have become a **third** hand-copy kept in sync by hand. The site now
re-exports the surface from here; the (proposed) Tauri 2 GUI consumes the same package.

As of `sq-jpki` the surface is no longer hand-mirrored at all: `WasmStore`,
`WasmSolutionCursor` and `WasmStoreCtor` are **aliases over the wasm-pack-GENERATED `Store` /
`SolutionCursor` classes** (re-exported from `src/generated/sparq_wasm.d.ts`, the tracked
verbatim build output — see [`src/generated/README.md`](src/generated/README.md)). That tracked
copy is the **single source of truth**, kept byte-identical to a fresh wasm build by
`npm run check:wasm-types`.

## What's in it

- The type surface: `WasmStore`, `WasmSolutionCursor`, `WasmModule` / `WasmStoreCtor`, the
  SPARQL 1.1 JSON shapes (`SparqlResults`, `SparqlTerm`, `SparqlBinding`), and the SHACL
  report shapes (`ShaclReport`, `ShaclResult`).
- The runtime loader: `loadSparq(opts?)` / `prewarmSparq(opts?)` — single cold-start, with a
  configurable `basePath` (defaulted from `NEXT_PUBLIC_BASE_PATH` ?? `"/sparq"` so the site is
  unchanged; a desktop GUI passes its own `tauri://` / `file://` origin).
- The framework-agnostic query helpers: `matchQuads`, `countQuads`, `streamQueryRows`,
  `sparqShaclValidate`, `formatTerm`.
- **RDF-document display + serialisation helpers** (`sq-8uew` / `sq-gb4o`, all dependency-free
  and DOM-free so the site and the Tauri 2 webview share one copy): `prettyTurtle(input, opts?)`
  / `prettyTrig(input, opts?)` reshape the engine's FLAT N-Triples / N-Quads
  CONSTRUCT/DESCRIBE output (`WasmStore.queryQuads` is N-Triples only) into idiomatic, indented
  Turtle/TriG — `@prefix` abbreviation, one block per subject, `;`/`,` predicate-object lists,
  `a` for `rdf:type` — while staying ROUND-TRIP-EQUIVALENT (re-parsing the output yields the
  same triple set; literals, blank-node labels and RDF 1.2 triple terms `<<( s p o )>>` are
  lossless) and NEVER throwing (an unparseable line passes through verbatim).
  `PrettyTurtleOptions` takes `{ prefixes?, indent?, abbreviate? }`; `parseNTriples(input)`
  exposes the underlying tokeniser (`{ statements, passthrough }`) over the `RdfTerm` /
  `RdfStatement` shapes. `tokenizeTurtle(text)` is the sibling Turtle/TriG/N-Triples/N-Quads
  highlighting tokenizer (`TurtleToken` / `TurtleTokenType`) — compose as pretty-print THEN
  highlight. The `sparql-prefixes` helpers recover prefix bindings from a query so the pretty
  view abbreviates result IRIs with the USER's own declared prefixes: `declaredPrefixBindings`
  / `declaredPrefixes` / `usedPrefixes` / `missingCommonPrefixes` / `renderPrefixLines` /
  `withPrefixes`, plus the `COMMON_PREFIXES` well-known registry and the `PrefixBinding` type.
  (The PARSE direction — Turtle/SHACL-Compact text → engine — is the engine's job, not this
  package's: SHACL Compact Syntax *display* lives in `site/` and its parser is tracked under a
  separate `sparq-shacl` bead.)
- **Endpoint mode** (`src/endpoint.ts`, `sq-2mke`): the SPARQL 1.1 Protocol HTTP client —
  the companion to the in-tab `WasmStore`. Run the SAME editor against any running
  `sparq-server` (or any conformant endpoint) over `fetch`. `runEndpointQuery(config, sparql)`
  classifies the form (`classifyEndpointForm`), builds the request (`buildSparqlRequest`: a
  direct `application/sparql-query` POST for reads with per-form `Accept`, a direct
  `application/sparql-update` POST for writes, bearer-auth in the `Authorization` header only),
  and parses the response per form (SELECT/ASK JSON, CONSTRUCT/DESCRIBE N-Triples, a `204`
  update ack). `connectionSafetyWarnings(config)` is the pure, honest connection-safety
  classifier that drives the Connect-panel UX; `wsSubprotocols(token)` derives the
  `bearer.<token>` subprotocol the server's WS handshake accepts (browsers cannot set an
  `Authorization` header on a WS upgrade). This client **consumes** the existing server API and
  **never bypasses a server gate** — it claims no security the server does not provide.
- **Live subscriptions** (`src/subscriptions.ts`, `sq-9ij6`): the SSE transport of the server's
  `/subscriptions` surface — the companion to endpoint mode for the LIVE case.
  `openSubscription(config, query, handlers)` subscribes a SELECT to `/subscriptions/sse` over
  `fetch` + a streaming `ReadableStream` reader (NOT native `EventSource`, which cannot set an
  `Authorization` header — so the SAME bearer token a query uses reaches an authenticated
  server) and drives `onOpen` / `onEvent` / `onClose(error?)`. `buildSubscriptionUrl` derives the
  SSE URL from the same `EndpointConfig`; `parseSubscriptionData` parses the SEPA envelopes
  (`subscribed` / `notification` / `error`); `applyNotification` reduces the streamed
  added/removed diffs onto a `LiveResultSet` keyed by the server's canonical row key (`rowKey`),
  and `liveResults` shapes it back into a `SparqlResults` document so the SAME `extractTable`
  renders a live row exactly like a queried one. It reuses endpoint mode's `EndpointConfig` +
  bearer posture verbatim, runs the SAME `connectionSafetyWarnings` classifier, and **bypasses no
  server gate** (the SSE read surface is gated by `--auth-token-read` exactly as `/sparql` GET).
- **Multiplexed WebSocket subscriptions** (`src/ws-subscriptions.ts`, `sq-140b`): the OTHER
  transport of the same server surface — ONE `/subscriptions` WebSocket carrying **many**
  subscriptions via the server's `{"subscribe":{…}}` / `{"unsubscribe":{"id"}}` frames, so a GUI
  holding several live views pays one connection. `openSubscriptionSocket(config, handlers?)`
  opens the socket (`buildSubscriptionSocketUrl`: `http`→`ws`, `https`→`wss`, `/sparql` →
  `/subscriptions`); its `subscribe(query, handlers, {alias?})` returns a per-subscription handle
  whose `close()` unsubscribes just that subscription. The bearer token is offered ONLY as the
  `bearer.<token>` subprotocol (`wsSubprotocols`) the server's `ws_auth_gate` validates for a
  browser upgrade — never in the URL. Frames are the SAME SEPA envelopes as SSE
  (`parseSubscriptionData` / `applyNotification` reused verbatim; `unsubscribed` is the one
  WS-only kind); refusals and terminating errors surface per subscription as
  `onEvent({kind:"error"})` then `onClose()`, exactly the SSE order, so one handler drives both
  transports. Runs the SAME `connectionSafetyWarnings` classifier and **bypasses no server
  gate**; a failed handshake is reported with an honest LIST of possible causes (the browser
  hides the real one — no fabricated single diagnosis).
- **Server health / capabilities** (`src/server-health.ts`, `sq-he72`): reads the connected
  server's OPERATIONAL surface, reusing the SAME `EndpointConfig` + bearer posture.
  `fetchServerHealth(config)` reads `/health`, the Prometheus `/metrics`, the opt-in VoID
  (`/.well-known/void`) and the opt-in SPARQL Service Description (a `GET /sparql` with no
  `query`) concurrently off the configured endpoint's ORIGIN (the operational endpoints live at
  the server root, NOT under `/sparql` — `deriveServerUrl` does the path swap), and returns each
  as a discriminated `FetchOutcome` (`ok` / `not-exposed` / `unauthorized` / `error`).
  `parsePrometheusMetrics` is a focused, dependency-free parser for the server's hand-rolled
  Prometheus [text exposition format] (`crates/sparq-server/src/metrics.rs`) → `MetricFamily[]`
  (HELP / TYPE / labelled samples, with histogram buckets grouped under their base family).
  `extractVoidSummary` / `extractServiceDescription` reshape the RDF descriptors (requested as
  `application/n-triples`, parsed via `parseNTriples`) into readable `VoidSummary` /
  `ServiceDescriptionSummary` facts (dataset counts; endpoint, supported languages, features,
  result/input formats, registered extension functions, named graphs). Crucially, a disabled
  opt-in feature answers `404`, surfaced as `not-exposed` — an **honest "the operator turned this
  off"**, never a fabricated metric or capability. Consumes the existing `sparq-server` API and
  **bypasses no server gate**.

  [text exposition format]: https://prometheus.io/docs/instrumenting/exposition_formats/

## Endpoint-mode safety posture (honest, never an overclaim)

`connectionSafetyWarnings` mirrors the real `sparq-server` posture (`crates/sparq-server/README.md`)
and the browser's transport rules, as classified `SafetyWarning`s (each with a stable `code`):

- `invalid-url` / `mixed-content` (**error**, hard block): not a valid http(s) URL; or an HTTPS
  page cannot `fetch` a plaintext `http:` endpoint (the browser blocks it before any request).
- `token-over-plaintext` / `non-loopback-no-tls` (**warning**): a bearer token — or the query and
  results — sent over plaintext `http:` to a **non-loopback** host travel in cleartext. (A token
  over **loopback** `http:` stays on the machine, so it is only an `info` note.) The server's
  Bearer gate is one shared secret with no per-user identity.
- `service-allowlist` / `cors-required` (**info**): a `SERVICE` clause is refused before any
  socket unless the operator set the egress allowlist (off / default-DENY); and the server emits
  **no CORS headers** by default, so a cross-origin browser fetch is blocked until an origin is
  opted in (`--cors-allow-origin`).

## What's NOT in it (deliberately)

- **Dataset-format coupling.** The site's `loadIntoStore` / `storeToNQuads` / `datasetSize`
  depend on `site/src/lib/repl-dataset.ts` (which named-graph formats route through
  `loadDataset`, the all-quads serialisation query). Those stay in the site; this package
  takes no opinion on dataset formats.
- **Any framework.** No React, no Next.js. The only browser globals touched are the two the
  wasm-pack glue needs at load time (`window.location`, dynamic `import()`), both guarded so
  the module is importable under `tsc` / Node type-checking.

## Consumption (repo-root npm workspaces) — `sq-jpki`

The repo now has **repo-root npm workspaces** (a root `package.json` with a `workspaces` field
spanning `packages/*`, `site`, `js`, `gui/e2e`), so a root `npm install` symlinks
`node_modules/@sparq/client` → this package. The site still resolves `@sparq/client` to this
package's **TypeScript source** (`src/index.ts`) via a `tsconfig` path + a matching
`next.config.ts` webpack alias — the site bundles from source rather than from a built `dist/`,
and the alias keeps that resolution identical in both the workspace and a bare `site/`-only
install. (Per-package `package-lock.json` files are retained so the existing `npm ci`-in-subdir
CI lanes — `pages.yml`, `site-e2e.yml` — keep working unchanged; migrating those to install
from the workspace root with the single root lock is a follow-up for the CI lane.)

### Single-source-of-truth guard — the tracked generated d.ts

The design's §4 end-state — the `Store` surface is **generated, not hand-mirrored** — is now
realised. The hand-written `WasmStore` / `WasmSolutionCursor` / `WasmStoreCtor` interface block
is **deleted**; those names alias the wasm-pack-generated `Store` / `SolutionCursor` classes,
re-exported from the tracked `src/generated/sparq_wasm.d.ts`.

The generated d.ts lives in the **git-ignored** build tree `js/wasm/` (`js/.gitignore`), so it
does not exist until `cd js && npm run build:wasm` runs — which is why a **tracked verbatim
copy** is checked in: it is the only type source the artifact-free bare `tsc` (the `gui.yml`
`shared-client` job, no wasm build) can resolve. To stop that tracked copy from silently
drifting from the actual binding, `npm run check:wasm-types` asserts it is **byte-identical** to
a freshly built `js/wasm/sparq_wasm.d.ts`:

```bash
(cd ../../js && npm run build:wasm)   # produces js/wasm/sparq_wasm.d.ts
npm run check:wasm-types              # FAILS if src/generated/sparq_wasm.d.ts has drifted
```

`check:wasm-types` is wired into the `gui.yml` `site-with-shared-client` job (which builds the
bundle); where no fresh bundle is present it SKIPs cleanly (exit 0), so the artifact-free
`shared-client` job is unaffected. After a binding change, re-sync the tracked copy and commit
it:

```bash
(cd ../../js && npm run build:wasm)
npm run sync:wasm-types               # copies the fresh js/wasm d.ts into src/generated/
```

This eliminates the last hand-copy: the export **is** the generated type, and a hand mirror
cannot silently reappear because the byte-identity check ties the tracked surface to the engine.

## Running the tests — and which lane gates them

```bash
npm install    # from here or from the repo root; this is a workspace member
npm test       # node --test over test/*.test.mjs, through the ts-loader
```

The gating lane for this package is the **`gui.yml` `shared TS client typecheck` job**
(`npm install` → `npm run typecheck` → `npm test`), which carries no
`.github/advisory-registry.json` declaration and therefore gates. **`js.yml` never runs this
suite** — it covers `js/`, `packages/rdfjs-conformance`, `packages/eyereasoner-compat` and
`packages/solid-server` only — so a green `js` check is not evidence about anything here.

`npm test` runs a `pretest` preflight (`scripts/check-test-deps.mjs`) that resolves the lazy
codec dependencies first. `src/decompress.ts` loads `fzstd` / `seek-bzip` through `import()`
inside the invocation path, so without that preflight an install which skipped this member
surfaces as failing bzip2 decode tests rather than as the missing dependency it is (#3006).

## Honesty

No performance number is asserted anywhere in this package (this repo's work box is
non-canonical). A caller may time a single query with `performance.now()` and label it as a
measured per-query latency. The ZK and MPC surfaces this client can drive elsewhere in the
repo are **research-grade and not externally audited** — that framing lives with those
surfaces and is unchanged by this package.
