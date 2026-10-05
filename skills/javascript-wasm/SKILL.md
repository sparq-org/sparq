---
name: javascript-wasm
description: Use the sparq RDF+SPARQL engine from JavaScript/TypeScript (Node >=18 or the browser) via its WebAssembly build and the @sparq-org/sparq RDF-JS wrapper — load Turtle/N-Triples/N-Quads/TriG, run SPARQL 1.1 SELECT/ASK, stream large results, count without materialising, apply SPARQL Update / quad deltas, do RDF-JS match()/countQuads(), and ingest gzip/zstd-compressed RDF. Reach for this when wiring sparq into a Node service, browser tab, or RDF-JS pipeline.
---

# sparq from JavaScript / WebAssembly

sparq is a Rust RDF triplestore + SPARQL engine compiled to WebAssembly. The npm package **`@sparq-org/sparq`** wraps it in an idiomatic [RDF/JS](https://rdf.js.org/) surface (`SparqStore`, Map-like `Bindings`, spec terms via `@rdfjs/types`); it runs unchanged in Node >= 18 and the browser. There is also a thin raw wasm class (`Store`) if you want SPARQL-JSON strings with no JS-side term materialisation. The bundle size is tracked live on the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench) under the Core metrics (`wasm_bundle_bytes`).

Use `SparqStore` (the high-level wrapper) by default — it covers SELECT/ASK (`query`/`queryBindings`/`queryBoolean`) and CONSTRUCT/DESCRIBE (`queryQuads`, returning RDF/JS `Quad`s). Drop to the raw `Store` only to skip term materialisation entirely (SPARQL-JSON / N-Triples strings) or for query-plan introspection.

## Quickstart

```js
import { SparqStore, DataFactory as DF } from '@sparq-org/sparq';

// init() runs lazily on first construction.
const store = await SparqStore.fromString(`
  @prefix ex: <http://ex/> .
  ex:alice ex:name "Alice" ; ex:knows ex:bob .
  ex:bob   ex:name "Bob"@en .
`, 'turtle'); // 'turtle' (default) | 'ntriples' | 'nquads' | 'trig' | 'jsonld'

// SELECT -> asynchronous RDF/JS ResultStream<Bindings>
const bindings = await store.queryBindings(
  'PREFIX ex: <http://ex/> SELECT ?s ?name WHERE { ?s ex:name ?name }',
);
await new Promise((resolve, reject) => {
  bindings.on('error', reject);
  bindings.on('end', resolve);
  bindings.on('data', (row) => {
    console.log(row.get('s').value, '->', row.get('name').value);
  });
});

// ASK -> boolean (native early-exit at first solution)
store.queryBoolean('PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }'); // true

// count without materialising rows
store.count('PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:name ?o }'); // 2

store.free(); // release wasm memory (or: `using store = await SparqStore.fromString(...)`)
```

Building from a source checkout of the sparq repo (the package ships the wasm prebuilt, but a checkout must build it):

```sh
cd js
npm run build   # = build:wasm (wasm-pack --profile release-wasm + copy) then build:ts (tsc)
npm test        # node --test against the built dist/
```

## Key APIs

`SparqStore` (from `@sparq-org/sparq`) — the high-level store:

```ts
// Construction (all async; each runs wasm init() once, memoised)
static empty(): Promise<SparqStore>                                        // empty + mutable; grow via update()/addQuads() (named graphs work out of the box)
static fromString(data: string, format?: RdfFormat, opts?: SparqStoreOptions): Promise<SparqStore>
static fromBytes(bytes: Uint8Array, format?: RdfFormat, opts?: Pick<SparqStoreOptions, 'baseIri'>): Promise<SparqStore>
static fromQuads(quads: Iterable<RDF.Quad>, opts?: SparqStoreOptions): Promise<SparqStore>     // serialised to N-Quads internally
static fromCompressed(bytes: Uint8Array, format?: RdfFormat,
                      opts?: SparqStoreOptions & { codec?: 'zstd' | 'gzip' }): Promise<SparqStore>
// Sync variants (engine must already be initialised — see fromStringSync): emptySync(), fromStringSync(...), fromQuadsSync(...)

type RdfFormat = 'turtle' | 'ntriples' | 'nquads' | 'trig' | 'jsonld'; // jsonld: a JSON-LD @graph is preserved with { dataset: true }
interface SparqStoreOptions { compressed?: boolean; dataset?: boolean; baseIri?: string; } // compressed/dataset/baseIri NOT mutually combinable

// Reading
get size(): number                                  // deduped triples in the DEFAULT graph
heapBytes(): number                                 // rough wasm-side footprint
query(sparql): Bindings[] | boolean                 // dispatches: SELECT->Bindings[], ASK->boolean
queryBindings(sparql, context?): Promise<ResultStream<Bindings>> // SELECT -> RDF/JS event stream
querySolutions(sparql): Map<string, Term>[]          // SELECT -> OXIGRAPH-shaped: array of plain Map (drop-in for oxigraph Store.query)
querySolutionsStream(sparql): Generator<Map<string,Term>> // streaming querySolutions
queryBoolean(sparql): boolean                        // ASK (native path; rejects non-ASK)
queryJson(sparql): string                            // raw SPARQL 1.1 JSON string (SELECT or ASK)
queryQuads(sparql): Quad[]                            // CONSTRUCT/DESCRIBE -> RDF/JS quads (default graph); rejects SELECT/ASK
queryQuadsString(sparql): string                     // CONSTRUCT/DESCRIBE -> raw N-Triples (valid Turtle subset)
count(sparql): number                                // solution count, no materialisation
serialize(format?, opts?: SerializeOptions): string  // store -> Turtle/TriG/JSON-LD doc string (serialize-rdf bundle); dump() is an alias
queryBindingsStream(sparql): Generator<Bindings>     // stream solutions, for...of / for await...of
queryQuadsStream(sparql, batchSize?): Generator<Quad> // stream CONSTRUCT/DESCRIBE quads (default batchSize 1024)
queryJsonChunks(sparql): Generator<string>           // raw ~64 KiB JSON chunks (concat == queryJson)

// SHACL validation (data graph vs shapes graph) — typed report; needs a shacl bundle (shipped by default)
validate(data: string, shapes: string, format?: RdfFormat): ValidationReport  // { conforms, results[] }; stateless
// (store-backed validation — the store's OWN triples — is on the raw wasm Store: validateStore(shapes, format))

// RDF/JS quad lookup (null/undefined/Variable = wildcard; generated SELECT under the hood)
match(s?, p?, o?, g?): Quad[]
matchStream(s?, p?, o?, g?): Generator<Quad>          // lazy match(): pulls from the engine in chunks, never materialised whole (backs Source.match)
countQuads(s?, p?, o?, g?): number

// Mutation (all IN PLACE through the engine's O(batch) delta overlay)
update(sparql): void                                 // SPARQL 1.1 Update (INSERT/DELETE DATA, DELETE/INSERT WHERE, CLEAR/DROP/CREATE/ADD/COPY/MOVE)
applyDelta(inserts: Iterable<RDF.Quad>, deletes?: Iterable<RDF.Quad>): void  // deletes first, then inserts
addQuads(quads): void                                // applyDelta(quads, [])
removeQuads(quads): void                             // applyDelta([], quads) — bnodes matched BY LABEL
free(): void                                         // also Symbol.dispose
```

`Bindings` (RDF/JS Query-spec, Map-like): `.get('var') -> RDF.Term | undefined`, `.has`, `.keys()`, `.values()`, `.entries()`, `.size`, `.equals`, immutable `.set/.delete/.filter/.map/.merge`, iterable, plus `.toMap() -> Map<string, Term>` (the Oxigraph-shaped bare-string-keyed view — #1123). Terms: `.termType` (`'NamedNode' | 'Literal' | 'BlankNode' | ...`), `.value`, plus `.language` / `.datatype` / `.direction` (`'ltr' | 'rtl' | ''`, RDF 1.2 base direction) on literals.

`Dataset` (named export — the full RDF/JS [`Dataset`](https://rdf.js.org/dataset-spec/) over the engine; async factories `Dataset.create/fromString/fromQuads`): `DatasetCore` (`add/delete/has/match/size/[Symbol.iterator]`) PLUS the algebra `union/intersection/difference/addAll/deleteMatches/contains/equals/filter/map/forEach/some/every/reduce/import/toStream/toArray/toString/toCanonical`. The binary set ops (`union/intersection/difference/addAll/contains/equals`) are INTEROP-aware — the operand may be another sparq `Dataset` OR any foreign RDF/JS dataset/store (N3.Store, @rdfjs/dataset), detected via `[Symbol.iterator]`. Full SPARQL surface one accessor away via `dataset.store`. (`toCanonical/equals/contains` are RDFC-1.0 blank-node-ISOMORPHISM-aware — `toCanonical` emits canonical `_:c14nN` N-Quads, `equals` compares canonical forms, `contains` matches a relabelled subgraph; backed by the engine's RDFC-1.0 surfaced over the opt-in `canon` wasm feature.)

`SparqSource` (named export, also `store.asSource()` — the RDF/JS [Stream spec](https://rdf.js.org/stream-spec/)'s `Source`/`Sink`/`Store` over a `SparqStore`): `match(s?, p?, o?, g?)` returns an event-based quad `Stream` that PULLS from `matchStream` (never materialised whole); `import(stream, opts?)` / `remove(stream, opts?)` consume a quad `Stream` and apply it as O(batch) deltas of `opts.chunkSize` quads (default 1024, so the JS heap holds at most one chunk; `0` buffers the whole stream and applies ONE all-or-nothing delta on `end` — with chunking, quads applied before a later `error` stay applied, as in N3.js); `removeMatches(s?, p?, o?, g?)` and `deleteGraph(graph)` mutate by pattern. Every mutation returns a stream that emits `end` only once the delta is actually applied. `deleteGraph` takes a `NamedNode`/`BlankNode`/`DefaultGraph` term or a string — `''` is the default graph, any other string a named-graph IRI (N3.js's convention, except that sparq reads a leading `_:` as an IRI rather than a blank-node graph name). An argument that cannot NAME a graph (a `Variable`, a `Literal`, nothing at all) emits `error` and deletes nothing — in particular a `Variable` no longer falls through to `match`'s "every graph" wildcard and deletes the entire dataset. `new SparqSource(store, { chunkSize })` sets the default for that adapter.

Other named exports from `@sparq-org/sparq`: `DataFactory` (RDF/JS factory: `namedNode`, `blankNode`, `literal`, `variable`, `quad`, ...) and term classes `NamedNode/BlankNode/Literal/Variable/DefaultGraph/Quad`; `init` (idempotent wasm bootstrap); compression helpers `decompress / decompressToString / sniffCodec`; SPARQL helpers `termFromSparqlJson / termToNT / quadsToNQuads / detectQueryForm / SparqlJsonRowsParser`; and the `SparqDictionaryClient` (server dictionary-fetch protocol). `termFromSparqlJson` accepts the SPARQL 1.2 JSON directional-literal shape — `{ type: 'literal', value: 'x', 'xml:lang': 'en', 'its:dir': 'ltr' }` yields a `Literal` with `.language === 'en'`, `.direction === 'ltr'`, and datatype `rdf:dirLangString` (round-tripped by `termToNT` as `"x"@en--ltr`); without `its:dir` the literal stays plain `rdf:langString`.

Raw wasm `Store` (from `../wasm/sparq_wasm.js`, re-exported as `WasmStore` internally) — use only when you need CONSTRUCT/DESCRIBE, batch cursors, or **query-plan introspection**. Methods return SPARQL-JSON / N-Triples / plan-text **strings**, not RDF/JS terms: `Store.load/loadDataset/loadCompressed(text, format)`, `Store.loadBytes(bytes, format)` / `Store.loadBytesWithBase(bytes, format, base)` (ingest a `Uint8Array` directly, `bytes-ingest` bundle only — see below), `Store.loadJsonLdWithContexts(text, [[url, documentText], …])` (JSON-LD whose `@context` is named by URL, resolved from host-fetched documents, `jsonld-contexts` bundle only — see below), `.query(sparql)`, `.queryChunks(sparql)`, `.queryCursor(sparql, batchSize)`, `.queryQuads(sparql)` (CONSTRUCT/DESCRIBE -> N-Triples), `.queryQuadsChunks(sparql, batchSize)`, `.count`, `.ask`, `.askWithMaxRows(sparql, maxRows)`, `.explain(sparql)`, `.explainAnalyze(sparql)`, `.validate(data, shapes, format)` (SHACL report as a JSON **string**, shacl bundle only), `.validateStore(shapes, format)` (the same report over the store's OWN triples, shacl bundle only — see below), `.serialize(format, pretty, indent, abbreviate, prefixes?)` (the store's contents as a Turtle / TriG / JSON-LD **string**, serialize-rdf bundle only — see below), `.parseShaclCompact(text, base?)` (SHACL Compact Syntax → the shapes graph as a Turtle **string**, scs bundle only — see below), `.update`, `.updateInPlace`, `.applyDelta(inserts, deletes)`, `.size`, `.heapBytes()`.

### Solid server wasm adapter (host integration)

The dedicated `sparq-lws-wasm` crate is the opt-in request adapter for a Solid-server wasm host; it
is separate from the `@sparq-org/sparq` RDF/JS store package. It owns the real axum LDP routes and WAC
evaluator over `CompositeStore<InMemorySparqClient, InMemoryBlobStore>`, while excluding the native
listener, Tokio runtime, TLS/PoP/notifications, live OIDC verifier, and non-memory backends.

Build and stage it with
`npm --workspace @sparq-org/solid-server run build:lws-wasm`; use `build:lws-wasm-core` for the named
core tier. [GPT-5.6] The full build enables `sparq-lws-wasm/sparql-endpoint`; the core command leaves
that feature off, so the core wasm omits the route and query-engine dependency graph. Construct
`SolidServer` with the pod base URL and the WebID that owns its provisioned root ACL, then call the
Promise-returning
`handleRequest(method, path, headers, body, authenticatedWebid)`. Header arguments and response
headers are flat name/value arrays. Omit `authenticatedWebid` for a public request.

```js
const owner = "https://id.example/alice#me";
const pod = new SolidServer("https://pod.example", owner);
const response = await pod.handleRequest(
  "PUT", "/card", ["content-type", "text/turtle"], turtleBytes, owner,
);
```

The host MUST complete OIDC validation before supplying an authenticated WebID; the constructor's
owner parameter only provisions WAC and is not authentication. Do not enable or stub
`solid-oidc-verifier` inside wasm: its pinned crypto backend is native-only.

[GPT-5.6] Pod contents are ephemeral by default — `new SolidServer(...)` keeps everything in linear
memory and loses it when the host drops the instance. `SolidServer.withSnapshot(baseUrl, ownerWebid,
bytes)` is the opt-in persistent constructor: the same in-memory pod behind a journaling `Store`
decorator. `snapshot()` returns the bytes to persist (`undefined` for a pod built with `new`), and
`snapshotRevision()` is a monotonic mutation counter so a host can skip a flush when nothing moved.

```js
const previous = await readFile(statePath).catch(() => new Uint8Array());
const pod = SolidServer.withSnapshot("https://pod.example", owner, previous);
// ... serve requests ...
await writeFile(statePath, pod.snapshot());   // survives the next listener restart
```

The wasm module owns only the byte format and the replay; the durable medium (`node:fs`,
IndexedDB) and the flush policy stay in the host. A `Store` that awaits a JS promise per operation
is not available: the trait is `Send + Sync` and every `JsValue` is `!Send`. The encoding folds
superseded writes and deleted resources away, so it tracks current contents rather than full
history. Restart replays the writes, so `Last-Modified` becomes the replay instant, while the
body-derived `ETag` is unchanged — a pre-restart `If-None-Match` still gets its 304. An owner ACL is provisioned only when the
restored pod has none, so a restart never reverts an ACL the owner edited; a snapshot that cannot be
decoded or replayed is refused rather than booting a partly-populated pod.

[GPT-5.6] The full tier's `/sparql` is query-only SPARQL 1.1 Protocol: GET uses the `query`
parameter; POST accepts `application/sparql-query` or form encoding. SELECT/ASK return
`application/sparql-results+json`; CONSTRUCT returns `application/n-triples`; DESCRIBE and UPDATE
are refused in v1. Each request walks authoritative `ldp:contains`, applies the same per-resource
WAC read decision as LDP GET, and gives the engine only admitted RDF. Each resource is a named graph
at its canonical IRI, the standing default graph is empty, and
`FROM <http://www.w3.org/ns/solid/sparql#union-default-graph>` opts into the admitted union.
Unreadable, malformed, or indeterminate resources are excluded. Assembly is currently O(pod) per
query; the endpoint does not yet retain a cross-request dataset cache. Within one server instance,
assembly and evaluation hold a shared read barrier while LDP mutations take its write side, so a
query cannot combine resources from an interleaved LDP write.

[GPT-5.6] The in-memory Solid store is bounded by default: its physical blob map admits at most
64 MiB in aggregate and 4,096 stored entries, and its metadata map independently admits at most
4,096 indexed resources. Physical usage includes unreferenced blob versions awaiting reconciliation,
so repeated overwrites cannot bypass the ceiling. A PUT/POST that would exceed either limit fails
closed with HTTP 507; deleting a current blob releases its bytes and entry slot. Rust embedders can
configure both ceilings with `InMemoryStoreLimits::new(max_total_bytes, max_resource_count)` and
`CompositeStore::in_memory_with_limits(limits)`, then inspect the concrete store's `usage()` and
`quota()` views. The current JS `SolidServer` constructor uses the bounded Rust defaults and does not
yet expose per-instance limit options.

[GPT-5.6] `@sparq-org/solid-server` supplies the local Node host. Install and run it with
`npx @sparq-org/solid-server --port 3000 --base-url http://127.0.0.1:3000 --owner-webid
https://id.example/alice#me`, or import `startSolidServer({ port, baseUrl, ownerWebid })`; it resolves
to a listening Node `http.Server` with an async `closeAsync()` helper. The listener binds
`127.0.0.1`, owns one wasm pod for its lifetime, and preserves repeated request/response headers.
It defaults to deliberately unauthenticated fixed-owner mode: every caller acts as `ownerWebid`.
For the opt-in Node authentication path, call
`startSolidServer({ port, baseUrl, ownerWebid, oidc: true })`. The host then requires and verifies a
Solid-OIDC access token plus request-bound DPoP proof with `@solid/access-token-verifier` before
passing the resolved WebID into wasm; missing, invalid, expired, replayed, bearer-only, or ambiguous
credentials pass no WebID and WAC treats them as anonymous. `ownerWebid` provisions the root ACL but
is not proof of identity in this mode. `baseUrl` must be the public request origin used in each DPoP
proof because the host binds the proof to the reconstructed request URL. The verifier dereferences
WebID and issuer/JWKS documents.
Use the package only for local development, do not expose it as a production server, and expect all
data to disappear on shutdown. TLS termination, persistent storage, and notifications remain
absent; OIDC verification runs in Node, not wasm.

[FABLE-5] **Transport-agnostic host mode (#2323).** For consumers that already run a web
framework (or need a per-request handler with no Node socket), `@sparq-org/solid-server` also exports
`createSolidPod({ baseUrl, ownerWebid, oidc })` — the same pod behind a dispatcher instead of a
listener. `pod.dispatch({ method, url, rawHeaders, body })` resolves to `{ status, headers, body }`
(`url` origin-form including any query, headers flat name/value pairs both ways, `body` in as
`string | Buffer | Uint8Array`, out as `Buffer`) and owns the whole host contract: the 2 MiB body
ceiling → the plain-text 413 shape, trap recycle → 503 (a later request is never served by a
poisoned instance), wasm-response copy + free, and repeated-header preservation. `baseUrl` is
required (no listener port to derive it from; OIDC DPoP proofs bind to it); `pod.free()` releases
the wasm instance. The first-party Fastify plugin sits on the `./fastify` subpath with `fastify`
as an OPTIONAL peer dependency (the core package keeps its single runtime dep):
`import { solidPod } from '@sparq-org/solid-server/fastify'; await fastify.register(solidPod,
{ baseUrl, ownerWebid, oidc })`. The plugin is mechanical over `dispatch`: it replaces all
content-type parsers with a `'*'` buffer parser (JSON-LD/N3 bytes reach wasm unparsed, capped at
2 MiB with `FST_ERR_CTP_BODY_TOO_LARGE` mapped to the host 413 shape), registers one catch-all
`all('/*')` route (`exposeHeadRoutes: false`) forwarding `request.raw.url` + `request.raw.rawHeaders`
(repeated headers and `/sparql?query=` intact), and sends grouped array-valued response headers
(repeated `Link`/`Vary` preserved). Do not put `@fastify/cors` in front of it — wasm owns CORS.
The building blocks are exported for assembling other hosts (a service-worker `fetch` handler)
downstream: `SolidServer`, `createPodDispatcher`, and the `http.js` helpers (`MAX_BODY_BYTES`,
`flattenRequestHeaders`, `readRequestBody`, `copyWasmResponse`, `writeNodeResponse`) from the
package root, and the raw wasm glue via the `./wasm` subpath export.

[SONNET-4.6] **Bounded linear memory → HTTP 507 (sq-wubkf).** The whole pod lives in one wasm32
linear memory, so an allocation that cannot grow it aborts into an `unreachable` trap and the
triggering request gets no HTTP answer. `sparq-lws-wasm` installs a `System`-delegating
`#[global_allocator]` that tracks **live** heap bytes, and `handleRequest` refuses a request whose
projected peak (`live + 4 × body + 1 MiB` headroom) would cross a ceiling — default 3 GiB — with a
507 whose status and `insufficient storage` body are byte-identical to the store-quota 507 above,
before the router runs. Because the accounting is of live bytes rather than of pages ever grown,
bytes returned to the allocator lower the total again and restore headroom, which a pages-grown
high-water mark could not — but only the counter arithmetic is tested. That an LDP `DELETE` frees
**enough** for a refused request to be admitted again has no end-to-end wasm32 test (the store
keeps its map capacity after removing an entry), so treat recovery from sustained pressure as the
accounting's intent rather than a demonstrated property. Read and tune it
from the host with `lwsMemoryLiveBytes()`, `lwsMemoryPeakBytes()`, `lwsMemoryCeilingBytes()`, and
`lwsSetMemoryCeilingBytes(bytes)` (`0` disables the bound; a negative or non-finite argument throws
rather than silently unbounding) — lower the ceiling when one module hosts several pods. This
covers *sustained* pressure only: a single request whose own transient peak overshoots the
remaining headroom still traps, and is still handled by trap recovery below. The allocator never
refuses an allocation — a null return from `GlobalAlloc::alloc` aborts, which is the very trap
being removed, so the bound is enforced at the request boundary instead.

[SONNET-4.6] **Trap recovery (sq-250si).** The wasm artifact uses `panic=abort` (release profile):
a Rust panic or allocation failure at the wasm32 linear-memory ceiling lowers to an `unreachable`
trap that the host sees as `WebAssembly.RuntimeError`. Before the abort fires, a `console_error_panic_hook`
installed at module init emits the panic message to `console.error`. The Node host catches the
`WebAssembly.RuntimeError`, frees the poisoned `SolidServer`, constructs a fresh instance (state
loss is acceptable for the ephemeral dev server), and returns HTTP 503 for the triggering request.
The next request is served by the new instance — a single trap no longer bricks the server process
indefinitely. The `attachTrapRecoveryHandler` and `isWasmTrap` helpers are exported from
`@sparq-org/solid-server/src/trap-recovery.js` for testing without a real wasm binary.

## Common recipes

**Decompress a browser dataset before loading it (`@sparq/client`).** The shared site/GUI client
selects gzip, ZIP, zstd, or bzip2 by payload magic first and filename extension second. gzip and
ZIP use native browser streams; zstd and bzip2 are separate lazy chunks fetched only when invoked.

```ts
import { decompressDatasetBytes } from '@sparq/client';

const compressed = new Uint8Array(await file.arrayBuffer());
const { bytes, innerName, codec } = await decompressDatasetBytes(compressed, file.name);
const rdfText = new TextDecoder().decode(bytes);
// Route rdfText using innerName (for example, "dataset.nt") and record codec if useful.
```

ZIP selects the first RDF-looking STORED or DEFLATE member and reports its member name. Encrypted
ZIP, ZIP64, unsupported ZIP methods, zstd dictionary frames, and malformed streams reject rather
than returning undecoded bytes. Browser gzip follows `DecompressionStream` semantics; do not rely
on it to concatenate multiple gzip members.

**Stream a large SELECT without holding the whole result.** One solution at a time, from ~64 KiB wasm-boundary chunks; `break` frees the cursor.

```js
for await (const row of store.queryBindingsStream('SELECT ?s ?o WHERE { ?s ?p ?o }')) {
  console.log(row.get('s').value);
}
```

**RDF/JS match() / countQuads().** Wildcards are `null`/`undefined`/Variable.

```js
store.match(null, DF.namedNode('http://ex/name'), null);     // -> Quad[]
store.countQuads(null, DF.namedNode('http://ex/name'));      // -> 2 (no materialisation)
```

**Build a store from nothing ([OPUS-4.8] sq-ty78o / #1114).** `SparqStore.empty()` (or the raw `new Store()`) yields an empty, mutable store; grow it with `update()` / `addQuads()`. Named graphs work out of the box — the overlay creates a named graph on the first targeted insert, so an empty store needs no `dataset` flag to accept `INSERT DATA { GRAPH … }` (dataset mode only matters when *loading* a document whose named graphs would fold).

```js
const store = await SparqStore.empty();
store.update('INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }');
const graphBindings = await store.queryBindings('SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }');
await new Promise((resolve, reject) => {
  graphBindings.on('error', reject);
  graphBindings.on('end', resolve);
  graphBindings.on('data', (row) => {
    console.log(row.get('g').value); // http://ex/g
  });
});
```

**Resolve relative IRIs against a base ([OPUS-4.8] sq-f66jz / #1115).** Pass `{ baseIri }` to `fromString` (or raw `Store.loadWithBase(text, format, base)`) for a document fetched from a URL (or a shapes graph / manifest) that carries relative IRIs and no `@base`. A document-level `@base` still overrides it; line-based formats ignore it; an invalid base throws. Not combinable with `dataset` / `compressed`.

```js
const s = await SparqStore.fromString('<a> <p> <../up/o> .', 'turtle', { baseIri: 'http://ex/dir/' });
// <a> -> http://ex/dir/a ; <../up/o> -> http://ex/up/o
```

**Mutate in place (O(batch), no index rebuild).**

```js
store.update('PREFIX ex: <http://ex/> INSERT DATA { ex:carol ex:name "Carol" }');
store.addQuads([DF.quad(DF.namedNode('http://ex/d'), DF.namedNode('http://ex/p'), DF.literal('o'))]);
store.removeQuads(store.match(null, DF.namedNode('http://ex/p')));
store.applyDelta(insertQuads, deleteQuads); // deletes applied first, then inserts
```

**Named graphs (load as a dataset).** Without `dataset: true`, all quads fold into the default graph and `GRAPH`/named-graph lookups see nothing. Applies to N-Quads, TriG, and a JSON-LD `@graph` (with an outer `@id`). `'jsonld'` parsing is compiled in only when the bundle is built with the opt-in `jsonld` feature (the published `@sparq-org/sparq` bundle enables it — see the bundle-features note below); the lean default bundle omits it.

```js
const ds = await SparqStore.fromString(nquads, 'nquads', { dataset: true });
const namedBindings = await ds.queryBindings('SELECT ?g ?s WHERE { GRAPH ?g { ?s ?p ?o } }');
await new Promise((resolve, reject) => {
  namedBindings.on('error', reject);
  namedBindings.on('end', resolve);
  namedBindings.on('data', (row) => {
    console.log(row.get('g').value, row.get('s').value);
  });
});
ds.update('INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> "o" } }');
ds.match(null, null, null, DF.namedNode('http://ex/g')); // graph-aware
```

**Compressed ingest + memory-tight index.** `fromCompressed` sniffs the codec by magic number (`.nt.zst`, `.ttl.gz`); `{ compressed: true }` halves index memory at a small per-scan decode cost.

```js
const fromZst = await SparqStore.fromCompressed(zstBytes, 'ntriples');           // codec auto-sniffed
const compact = await SparqStore.fromString(bigTurtle, 'turtle', { compressed: true });
```

**CONSTRUCT / DESCRIBE.** `SparqStore.queryQuads(sparql)` returns the constructed/described graph as RDF/JS `Quad`s in the default graph (template bnodes freshened per solution; illegal slots dropped per §16.2). `queryQuadsString(sparql)` gives the raw N-Triples (a valid Turtle subset) and `queryQuadsStream(sparql, batchSize?)` streams the quads for a large graph. A SELECT/ASK routed here throws.

```js
const quads = store.queryQuads('PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }');
quads[0].subject.value; // 'http://ex/alice'
for (const q of store.queryQuadsStream('DESCRIBE <http://ex/bob>')) { /* one Quad at a time */ }
const nt = store.queryQuadsString('DESCRIBE <http://ex/bob>'); // raw N-Triples string
```

The raw `Store.queryQuads` / `queryQuadsChunks` (returning N-Triples strings) remain available for zero term-materialisation.

**Serialise / dump the store ([OPUS-4.8] sq-u78ol / #1117 / #1129, serialize-rdf bundle only).** The high-level **`SparqStore.serialize(format?, opts?)`** (with a `dump(format?, opts?)` alias) is the primary entry point — an options-object wrapper over the raw `Store.serialize(...)` below (`opts: { pretty?, indent?, abbreviate?, prefixes? }`, all defaulted: pretty Turtle, two-space indent, abbreviated, engine default prefixes). It throws a clear "requires a serialize-rdf-enabled wasm bundle" error on a lean bundle (rather than a cryptic "not a function").

```js
const ttl = store.serialize('turtle');                                   // pretty Turtle, default opts
const trig = store.dump('trig');                                         // dump() == serialize()
const mine = store.serialize('turtle', { prefixes: [['ex', 'http://ex/']] }); // caller prefix policy (#1129)
const full = store.serialize('turtle', { abbreviate: false });           // full <…> IRIs, no @prefix header
```

The raw wasm `Store.serialize(format, pretty, indent, abbreviate, prefixes?)` (positional) is the same engine writer. Where `queryQuads` writes a CONSTRUCT/DESCRIBE *result graph* as flat N-Triples, `serialize(...)` writes the **store itself** as a readable Turtle (default graph), TriG (whole dataset, named graphs as `GRAPH <g> { … }` blocks), or JSON-LD (whole dataset) **string**. `pretty=true` indents the output (Turtle/TriG: the sorted, blank-line-separated `prettyTurtle` shape; JSON-LD: a structurally re-indented document); `indent` is the indent unit (`null`/`undefined` ⇒ two spaces, ignored when `pretty=false`). For Turtle/TriG, `abbreviate=true` emits a sorted `@prefix` header and compacts IRIs to `prefix:local`, `false` keeps full `<…>` IRIs. `prefixes` is an OPTIONAL `[[prefix, iri], …]` array selecting the compaction prefix map: omit it (`undefined`/`null`) for the engine's well-known defaults (`rdf`/`rdfs`/`xsd`/`owl`/`schema` → `http://schema.org/`/`foaf`/`dc`/`skos`/`sh`) — **byte-for-byte the prior behaviour** — or pass your OWN policy (the [`COMMON_PREFIXES`](../../packages/sparq-client/src/sparql-prefixes.ts) registry with `https://schema.org/` + `dcterms`/`prov`/`geo`/`void`/`ex` → `http://example.org/`, or a query's `declaredPrefixBindings(...)`) to drive `@prefix`/`@context` compaction under that policy and get byte-parity output. It applies to Turtle/TriG (with `abbreviate=true`) and the JSON-LD `jsonld-compacted` `@context`; a malformed entry throws. `format` (case-insensitive) is `'turtle'` (`'ttl'`/`'text/turtle'`), `'trig'` (`'application/trig'`), or **JSON-LD**: `'jsonld'` (`'json-ld'`/`'application/ld+json'`) emits the **expanded** form by default, and `'jsonld-expanded'` / `'jsonld-flattened'` / `'jsonld-compacted'` pick the [JSON-LD 1.1 document form](../data-formats/SKILL.md) explicitly. For JSON-LD `abbreviate` is **ignored** — IRI abbreviation is chosen by the `jsonld-compacted` form (which carries a prefix `@context`). An unrecognised `format` (or a malformed `prefixes`) throws. All three formats route through the SAME engine writer the native CLI/HTTP surface uses (so JS consumes the engine writer rather than hand-formatting JSON-LD). Compiled in only with the opt-in `serialize-rdf` bundle feature (see below); the lean bundle omits it. (JSON-LD serialise-OUT needs no extra feature — only `serialize-rdf`; the `jsonld` feature is INGEST-only.) **`jsonld-compacted` vs `serializeCompact` (full Compaction).** The wasm `serialize` `'jsonld-compacted'` form is the *prefix-only* `@context` (it abbreviates IRIs to `prefix:local` CURIEs from the `prefixes` map). For the **full W3C JSON-LD 1.1 Compaction Algorithm** against a caller-supplied `@context` (term definitions / `@vocab` / type-language-`@container` coercion / `@reverse` / `@id`/`@type` aliasing / value+node+IRI compaction), use the sibling **`serializeCompact(context, pretty, indent?)`** binding (sq-oy1f.5) — `context` is the `@context` **JSON text** (e.g. `'{"@vocab":"http://schema.org/"}'`), `pretty`/`indent` mirror `serialize`. It routes through `sparq_engine::serialize::graph_to_jsonld_compact` (still dependency-free, see the [data-formats SKILL](../data-formats/SKILL.md)) and is **lossless** (a JSON-LD→RDF round-trip reconstructs the triples). A non-object / malformed `context` throws. Same `serialize-rdf` bundle feature; absent on the lean bundle.

```js
// Full Compaction against a caller @context (NOT the prefix-only 'jsonld-compacted' form):
const doc = store.serializeCompact('{"@vocab":"http://schema.org/"}', true, '  ');
// → bare @vocab-relative predicate keys ("name", "knows"), the @context echoed.
```

```js
const ttl = store.serialize('turtle', true, '  ', true);          // pretty Turtle, 2-space indent, default prefixes
const trig = store.serialize('trig', true, null, true);            // pretty TriG over the whole dataset
const jsonld = store.serialize('jsonld-compacted', true, '  ', true); // pretty JSON-LD, compacted @context
// Caller-supplied prefix policy (e.g. the site's COMMON_PREFIXES → https://schema.org/, ex: → example.org):
const mine = store.serialize('turtle', true, '  ', true, [['ex', 'http://example.org/'], ['schema', 'https://schema.org/']]);
```

**Parse SHACL Compact Syntax (raw wasm `Store.parseShaclCompact`, scs bundle only).** `parseShaclCompact(text, base?)` parses a [SHACL Compact Syntax](https://www.w3.org/TR/shacl12-compact-syntax/) (SCS) document into the equivalent SHACL **shapes graph** and returns it as a pretty **Turtle string** — the SCS *input* direction (the `'Compact → shapes'` mode for the workbench). `base` (optional) is the base IRI relative IRIs and the `owl:Ontology` subject resolve against; `undefined`/`null` uses the SCS no-`BASE` convention (`urn:x-base:default`), and a document-level `BASE` directive overrides it. It reuses the `Store.serialize('turtle', true, '  ', true)` engine writer above (no second serialiser), so the bytes match that call; the shapes it yields validate data **identically** to the equivalent hand-written Turtle shapes (it is the same triples `validate` consumes), covering the grammar the W3C `shacl12-cs` corpus exercises. It is **stateless** (ignores the receiver's triples). A malformed SCS document throws a `JsError` carrying the 1-based source line.

```js
import init, { Store } from '@sparq-org/sparq/wasm/sparq_wasm.js'; // the raw wasm Store
await init();
const store = Store.load('', 'turtle');                       // stateless; receiver ignored
const shapesTurtle = store.parseShaclCompact(
  'PREFIX ex: <http://example.org/>\nshapeClass ex:Person {\n  ex:name xsd:string [1..1] .\n}\n');
// shapesTurtle is a pretty-Turtle shapes graph; feed it back to store.validate(...) or display it.
```

**EXPLAIN / EXPLAIN ANALYZE (query-plan introspection, raw wasm only).** Same plan text the Rust API (`sparq_engine::explain`) and the HTTP endpoint (`?explain` / `?explain=analyze`, or `Accept: text/x-sparq-explain`) return — returned to JS as a plain string. `explain` is a planning-only dry run (no execution; every query form); `explainAnalyze` runs the query (SELECT/ASK only) and appends a per-operator row-count trace.

```js
const plan = raw.explain('PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a }');
// "EXPLAIN (SELECT) — planning-only dry run; nothing is executed.\n...Plan:\n  ..."
const trace = raw.explainAnalyze('PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }');
// plan + per-operator output rows (wall times read 0 on wasm32 — no monotonic clock)
```

**Structured plan tree (`explainPlanJson` / `explainPlanAnalyzeJson`, explain-json bundle
only — the published bundle enables it; sq-ixc3.19).** The TYPED plan the GUI plan explorer
renders: `sparq-engine`'s `explain_json::PlanNode` as camelCase JSON — per operator
`{"operator", "estimated", "actual", "nanos", "qError", "children"}` (the sq-jbqh4 schema
contract, identical to the server's `Accept: application/x-sparq-explain+json` response).
The dry-run form populates only `estimated`; the analyze form (SELECT/ASK only) fills
`actual` rows and `qError` = max(est/actual, actual/est) exactly — `nanos` still reads 0 on
wasm32 (the same no-monotonic-clock caveat as `explainAnalyze`; real per-operator wall
times come from the desktop-native or server paths). `@sparq/client` exports the matching
`PlanNode` type + the `parsePlanJson` defensive parse.

```js
const tree = JSON.parse(raw.explainPlanAnalyzeJson('SELECT ?n WHERE { ?s <http://ex/name> ?n }'));
// { operator: "...", estimated: ..., actual: ..., nanos: 0, qError: ..., children: [...] }
```

**SHACL validation (`SparqStore.validate`, shipped by default).** `validate(data, shapes, format?)` validates an RDF **data graph** against a SHACL **shapes graph** and returns a typed `ValidationReport` (the JSON the wasm binding emits, parsed for you). It runs `sparq-shacl`'s SHACL Core + SHACL-SPARQL (`sh:sparql`) engine inside wasm — a drop-in for `rdf-validate-shacl`. It is **stateless** (does not consult the store's own triples). `format` defaults to `'turtle'` and accepts the same set as `fromString`.

```ts
import { SparqStore, type ValidationReport } from '@sparq-org/sparq';

const store = await SparqStore.fromString('', 'turtle'); // validate ignores the receiver
const report: ValidationReport = store.validate(dataTurtle, shapesTurtle); // format defaults to 'turtle'
// ValidationReport = { conforms: boolean;
//   results: Array<{ focusNode; path; value; sourceShape;
//                    sourceConstraintComponent; severity; message }> }
report.conforms;                                     // false if ANY result (incl. Warning/Info)
const violations = report.results.filter(
  r => r.severity === 'http://www.w3.org/ns/shacl#Violation');   // violations-only gate
```

The raw `Store.validate(data, shapes, format)` returns the same report as a JSON **string** (`JSON.parse` it yourself). `focusNode`/`value`/`sourceShape` are N-Triples term strings; `path` is a SHACL Turtle path expression; `severity`/`sourceConstraintComponent` are full IRIs; `message` is the first `sh:message` text (or a generated default). `path`/`value`/`message` are `null` when absent. Only a graph parse failure throws; malformed shapes are skipped, not surfaced. For large data graphs validate server-side via the `sparq-server` HTTP `validate` endpoint instead (the other half of the #162 decision). See the `shacl-validation` skill for the engine's SHACL coverage.

**Store-backed SHACL (raw wasm `Store.validateStore`, shacl bundle only, gh-2520).** `validateStore(shapes, format)` is the **stateful** counterpart: the data graph is the store's own contents — whatever `load`/`loadDataset`/`update`/`applyDelta` left in it — so only the *shapes* document is parsed per call. Report shape, `sh:conforms` semantics and error behaviour are identical to `validate`'s (only a shapes parse failure throws). Use it when one loaded store is re-validated as shapes are edited, instead of re-passing (and re-parsing) the data document every time.

```js
const store = Store.load(dataTurtle, 'turtle');            // parse the data ONCE
const report = JSON.parse(store.validateStore(shapesTurtle, 'turtle'));
```

Two caveats. It observes the store's **default graph** only — triples put in named graphs by `loadDataset` are not focus nodes or value nodes (`load` folds named graphs into the default graph, so a store built that way validates in full). And blank-node labels in `sourceShape` are minted fresh by each shapes parse, so they differ between any two calls (of either method) — treat them as per-call identifiers, not stable keys. The typed `SparqStore.validate` wrapper has no store-backed twin yet; reach for the raw `Store` for this one.

**Raw wasm `Store`: init (`initSync` vs async default), errors, and `.free()` (#1127).** The wasm-pack `--target web` glue (`../wasm/sparq_wasm.js`) is a real ESM module that exports a **default async `init`** plus a synchronous **`initSync`** — one of the two MUST run before any `Store.*` static (`SparqStore`/`Dataset` do this for you via the package's memoised `init()`; you only call these when using the raw `Store` directly).

```js
import init, { initSync, Store } from '@sparq-org/sparq/wasm/sparq_wasm.js';

// (a) async default — the normal path. In the browser/Deno it fetches the .wasm
// relative to the module; pass explicit bytes/URL to override.
await init();                                   // or: await init({ module_or_path: bytes })
// (b) initSync — when you ALREADY hold the compiled module/bytes (no top-level await,
// e.g. a bundler that inlines the wasm, or a Cloudflare-Worker WebAssembly.Module import).
initSync({ module_or_path: wasmModuleOrBytes });

const store = Store.load('<a> <b> <c> .', 'ntriples');
```

- **When to use which.** Prefer the high-level `@sparq-org/sparq` entry (`SparqStore`/`Dataset`), whose `init()` picks the right path per environment (Node reads the bytes off disk; browser/Deno `fetch`es) and memoises it — so the ~MB `.wasm` is paid at most once. Drop to `init`/`initSync` only when you import the raw `Store` glue directly: async `init()` for the common fetch-relative case, `initSync(...)` only when you have the module/bytes in hand and want no `await`.
- **`init()`/`initSync()` not called first → `Store.*` throws** a wasm-bindgen "must call init" error. Calling `init()` again is idempotent.
- **CommonJS: `require('@sparq-org/sparq/wasm-node')` — no init at all (sq-2hk).** Alongside the `--target web` build in `wasm/`, `npm run build` produces a `--target nodejs` build of the same crate with the same feature set into `wasm-node/`, shipped in the tarball and exported as the `./wasm-node` subpath (with a nested `{"type": "commonjs"}` marker, since the package root is `"type": "module"`). That glue is CommonJS and instantiates the module eagerly inside `require()` — it reads the `.wasm` off disk next to itself — so it exposes no `init`/`initSync` and needs none: `const { Store } = require('@sparq-org/sparq/wasm-node'); Store.load(…)` works synchronously. Everything else on this list (format strings, thrown errors, `.free()`) applies unchanged. The `@sparq-org/sparq` main entry stays ESM-only — from CommonJS reach the RDF/JS wrapper with `const { SparqStore } = await import('@sparq-org/sparq')`.
- **Format strings** for `Store.load`/`loadDataset`/`loadCompressed` are the same case-sensitive set the engine accepts: `turtle` (`ttl`/`text/turtle`), `ntriples` (`n-triples`/`nt`), `nquads` (`n-quads`/`nq`), `trig`, and `jsonld` (`json-ld`/`application/ld+json`, opt-in `jsonld` bundle). An **unrecognised format is an `Err`/throw** — it is NOT silently parsed as Turtle (so a `'jsonld'` load on the lean bundle throws rather than mis-parsing).
- **Errors are JS exceptions.** Every fallible `Store` method (`load*`, `query`, `ask`, `update`, `applyDelta`, `validate`, …) maps a Rust `Err(String)` to a thrown `Error` (wasm-bindgen `JsError`) carrying the engine message (parse error with position, malformed SPARQL, an unrecognised format, a query form routed to the wrong method — e.g. CONSTRUCT through `query`). Wrap calls in `try/catch`; there are no silent failures.
- **`.free()` — when and on what.** wasm linear memory is NOT GC'd: call `.free()` on every `Store` you create, AND on each cursor object (`QueryChunks` / the `queryCursor` / `queryQuadsChunks` handles) once drained or abandoned. After `.free()` the handle (and any cursor it spawned) must not be touched. In the high-level wrapper use `store.free()` or `using store = await SparqStore.from…()` (`Symbol.dispose`); for the raw `Store`, `break`ing out of a streaming `for…of` over a cursor still requires freeing that cursor. Leaking handles grows the 4 GB wasm heap until the tab/process OOMs.

## Gotchas / feature flags / prerequisites

- **ESM only, Node >= 18.** The package is `"type": "module"`; `init()` is idempotent and runs automatically on the first `SparqStore.from*` call (in Node it reads the wasm bytes from disk; in the browser/Deno it `fetch`es relative to the module). One runtime dep: `fzstd` (~8 KB, dynamically imported only when decoding zstd).
- **`SparqStore.query()` is SELECT/ASK only.** It returns `Bindings[]` (SELECT) or `boolean` (ASK); a CONSTRUCT/DESCRIBE routed through it throws. Use `queryQuads()` (RDF/JS `Quad`s) / `queryQuadsString()` (N-Triples) / `queryQuadsStream()` for the graph-valued forms. Federated (`SERVICE`) queries are still **not** exposed at the JS wrapper layer.
- **`REGEX` / `REPLACE` are compiled out** of the wasm build (the engine's non-default `regex` cargo feature is off to keep the bundle small). Use `CONTAINS`/`STRSTARTS`/`STRENDS`/... or a custom wasm build with `--features regex`.
- **SHACL is on `SparqStore.validate` (typed) AND raw `Store.validate` (JSON string), shipped by default.** The published bundle is built with `--features shacl`, so `validate` works out of the box. SHACL is **not free in the binary** — it pulls in the SHACL engine + `regex` + the `sh:sparql` query path, which roughly **doubles** the `.wasm` (measured ~1.21 MiB → ~2.19 MiB, +~1.0 MiB / +85%, pre-gzip). Size-sensitive consumers can build the lean variant (`npm run build:wasm:lean`, equivalently `wasm-pack build … --profile release-wasm` with no `--features`), which omits SHACL — `SparqStore.validate` then throws a clear "requires a SHACL-enabled wasm bundle" error. Use `--features shacl-af` to also compile `sh:rule`. Validation is in-process and best for small documents (~10–100 triples); large graphs should use the server-side HTTP `validate` path.
- **JSON-LD ingest (`'jsonld'`) is shipped in the published bundle but is an OPT-IN cargo feature.** The published `@sparq-org/sparq` bundle is built with `--features shacl,jsonld` (the js `build:wasm` script), so `fromString(_, 'jsonld')` works out of the box. JSON-LD adds non-trivial binary size (linked via the `oxjsonld` parser), so the **lean default bundle** (`build:wasm:lean` / `cargo build -p sparq-wasm` with no `--features`) omits it to minimize overhead. On the lean bundle a `'jsonld'` load is not recognised as JSON-LD. Turtle / N-Triples / N-Quads / TriG are always present (no feature needed).
- **Remote-`@context` JSON-LD (`Store.loadJsonLdWithContexts`) needs the `jsonld-contexts` bundle feature ([SONNET-4.6] sq-yz27r, #3251).** Plain `'jsonld'` ingest drives `oxjsonld` with **no** `LoadDocumentCallback`, so a document that names its `@context` by URL rather than inline — which is how essentially every real Verifiable Credential is written (`"@context": "https://www.w3.org/2018/credentials/v1"`) — throws `No LoadDocumentCallback has been set to load remote contexts`. The opt-in `jsonld-contexts` feature (implies `jsonld`) adds `Store.loadJsonLdWithContexts(text, contexts)`, where `contexts` is an ordered `[[url, documentText], …]` array of context documents **the host has already retrieved**: `const ctx = await (await fetch(url)).text(); Store.loadJsonLdWithContexts(vcJson, [[url, ctx]])`. Nested/second-level remote contexts resolve through the same callback (it is `oxjsonld`'s own recursion, not a pre-parse rewrite of the document's `@context`). **The fetch stays in JS by necessity, not by preference** — `oxjsonld`'s `LoadDocumentCallback` is *synchronous* and cannot await `fetch`, so the split is host-fetches / binding-parses; the binding itself performs **no I/O** and the bundle links no HTTP code. **Fail-closed:** a `@context` URL the document references but `contexts` does not carry throws a `JsError` naming that URL (never a silent partial parse that would drop every term the context defines); the supplied map *is* the allowlist, so any same-origin / CSP policy is the host's to enforce at fetch time. Named graphs fold into the default graph, as `load` does. OFF by default so the lean `wasm_bundle_bytes` baseline is byte-identical; `build:wasm` does not yet enable it, and on a bundle without the feature the method is absent.
- **`serialize` / `dump` need the `serialize-rdf` bundle feature.** Exposed on BOTH the high-level `SparqStore.serialize(format?, opts?)` (with a `dump` alias) and the raw `Store.serialize(...)` ([OPUS-4.8] sq-u78ol / #1117 / #1129). The published `@sparq-org/sparq` bundle is built with `--features shacl,jsonld,serialize-rdf` (the js `build:wasm` script), so `serialize(...)` works out of the box. It is **not free in the binary** — it links `sparq-engine`'s serializer matrix (the pretty Turtle / TriG writers) — so the **lean default bundle** (`build:wasm:lean` / `cargo build -p sparq-wasm` with no `--features`) omits it. On the lean bundle `SparqStore.serialize`/`dump` throw a clear "requires a serialize-rdf-enabled wasm bundle" error (and the raw `Store.serialize` method is absent).
- **`Store.parseShaclCompact` needs the `scs` bundle feature.** The published `@sparq-org/sparq` bundle is built with `--features shacl,jsonld,serialize-rdf,scs` (the js `build:wasm` script), so `parseShaclCompact(...)` works out of the box. The `scs` feature implies `shacl` + `serialize-rdf` (it REUSES the `serialize` engine writer to emit the shapes Turtle — no second serialiser) and forwards to `sparq-shacl/scs` (a hand-rolled parser, **no new dependency**). The **lean default bundle** (`build:wasm:lean` / `cargo build -p sparq-wasm` with no `--features`) omits it, so the lean `.wasm` is unchanged; on the lean bundle the `parseShaclCompact` method is absent. (Raw `Store` only for now; the playground "Compact → shapes" input mode and any typed `SparqStore` wrapper are tracked separately.)
- **ODRL policy evaluation is an EXPERIMENTAL probe, NOT in the published bundle ([FABLE-5] sq-586sh, #890 ask A).** The opt-in `policy` cargo feature compiles `sparq-policy`'s stateless ODRL evaluator to wasm32 and exposes two probe free functions on the raw wasm surface — `policyEvaluate(policyRdf, format, action, target?, party?)` → decision JSON (`{allow, matchedRules, unmetConstraints}`, fail-closed: malformed policy throws, unmatched/empty policy denies) and `policyConflicts(policyRdf, format)` → static conflict/admissibility JSON. `build:wasm` does **not** enable it: the full JS API (per-dimension audit statuses, per-duty status, purpose/party taxonomies) awaits a maintainer public-contract decision (the sq-586sh report issue). The stateful `count-enforcement` feature is deliberately not forwarded (no cross-tab atomicity in a browser). See `usage-control-policy` for the evaluator semantics.
- **Byte-ingest (`Store.loadBytes` / `loadBytesWithBase`) needs the `bytes-ingest` bundle feature ([FABLE-5] sq-3ul2n.3).** The opt-in `bytes-ingest` cargo feature exposes `Store.loadBytes(bytes, format)` and `Store.loadBytesWithBase(bytes, format, base)`, which take the `Uint8Array` you already hold (e.g. `new Uint8Array(await response.arrayBuffer())`) and feed it to the SAME parse path as `load`/`loadWithBase`, skipping the UTF-16 JS-string round-trip. The result is byte-for-byte the store `Store.load(new TextDecoder().decode(bytes), format)` builds (equal size + probe-query JSON). Invalid UTF-8 is rejected **fail-closed** with a catchable `JsError` (RDF text formats are all UTF-8) — never a panic or lossy decode, the same error surface as a malformed document to `load`. ZERO new dependencies. OFF by default so the lean `wasm_bundle_bytes` baseline is byte-identical; the published bundle turns it on (the js `build:wasm` wiring is tracked separately, sq-3ul2n.5). On a bundle without the feature the `loadBytes` methods are absent.
- **SHACL-to-form derivation (`Store.deriveForm`) needs the `forms` bundle feature ([SONNET-4.6] sq-q4apb, #2396).** The opt-in `forms` cargo feature compiles `sparq-forms`' derivation to wasm32 and exposes `Store.deriveForm(data, shapes, focus, format, optionsJson)` on the raw wasm surface — the hosted-web half of the GUI forms bridge (`gui/app`'s `forms-bridge.ts` feature-detects exactly this method; desktop uses the Tauri `derive_form` command instead, and the adapter never falls through between hosts). Stateless one-shot: `data`/`shapes` are serialized workspace snapshots in `format` (named graphs preserved dataset-style; the workbench sends N-Quads), `focus` is an absolute IRI or `_:label`, `optionsJson` is the snake_case `{"mode":"edit"|"view","shape"?:iri-or-_:label}` object. Returns the `FormDescription` serde JSON **string** verbatim — byte-identical to the desktop bridge for the same inputs; `JSON.parse` it, never reconstruct keys/groups/widgets. Malformed graphs/focus/options throw a `JsError`; there is no demo-data fallback. OFF by default (the lean `wasm_bundle_bytes` baseline is byte-identical); `build:wasm` — the bundle the hosted /app, the site and the published package all consume — enables it ([SONNET-4.6] sq-q4apb, #2644), so `deriveForm` is present there and absent from `build:wasm:lean`. On a bundle without the feature the method is absent. See the `shacl-forms` skill for the derivation semantics.
- **`options.dataset` is not combinable with `options.compressed`** — there is no compressed dataset loader yet (the constructor throws). `compact-index` (3 permutations, ~half the memory) is auto-selected for wasm32 regardless; `compressed` adds block compression on top.
- **`size` / `heapBytes` report the DEFAULT graph only.** For dataset totals use `countQuads()` (its graph wildcard spans named graphs).
- **Mutation is overlay-based.** `update()` / `applyDelta()` write through an append-only delta overlay: the dictionary only grows, and deletes are tombstones until the wasm store is reloaded. Blank nodes in `applyDelta`/`removeQuads` are matched **by label** (so bnode triples can be retracted — impossible via SPARQL `DELETE DATA`).
- **Browser gzip truncation.** Browsers silently truncate **multi-member gzip** to the first member; `fromCompressed` uses `node:zlib` in Node (loops members) and `DecompressionStream` in the browser (single-member only). Multi-frame **zstd** decodes fully everywhere via fzstd. fzstd cannot decode zstd **dictionary** frames — for those supply a dict-capable decoder via `SparqDictionaryClient`'s `decodeWithDictionary` hook.
- **Lifetime.** Call `.free()` (or use `using`) to release wasm linear memory; the store and any held cursors must not be used afterward. wasm32 caps linear memory at 4 GB (a real tab is happier under ~2 GB): ~30 M triples raw, ~75 M with `compressed`.
- **Raw `Store` budget knobs are wasm-portable only.** `askWithMaxRows` bounds the working set by row count; the engine's wall-clock deadline budget is native-only (`std::time::Instant` is unusable on wasm32).
- **`explainAnalyze` wall times read 0 on wasm32.** There is no monotonic clock in the wasm bundle, so the per-operator trace reports 0 for every wall time; the per-operator **row counts are exact**. `explainAnalyze` executes the query (SELECT/ASK only) — CONSTRUCT/DESCRIBE/UPDATE are rejected; use `explain` (a non-executing dry run that accepts every query form) for those.
- **The bundle ships under the size-optimised `release-wasm` cargo profile ([OPUS-4.8] sq-7d3dj.1).** `build:wasm{,:lean}` build `wasm-pack --profile release-wasm` (root `Cargo.toml`): the COLD run-once parse crates (`spargebra`/`oxiri`/`oxttl`) compile at `opt-level = "z"` and local symbols are stripped (`strip = "symbols"`), while the HOT engine/core query-eval crates stay `opt-level 3` — so the bundle is materially smaller with no runtime-perf change (pure parser code, off every hot query path). The `wasm_bundle_bytes` perf-gate (`scripts/ci-bench.sh`) builds under the SAME cargo profile as the shipped bundle — it ratchets a raw `cargo build` `.wasm` (not the post-`wasm-bindgen`/`wasm-opt` npm artifact), so it tracks the profile the shipped bundle is built under rather than the shipped bytes themselves.
- **Benchmarking the bundle vs the JS/WASM ecosystem ([FABLE-5] sq-hmd7l.17).** `bench/wasm-compare/run.sh --bundle-only` is the DETERMINISTIC shipped-bundle-bytes comparison vs the pinned `oxigraph` npm artifact (the one wasm metric that can be canonical without a quiet box); `bench/wasm-compare/browser/` holds the per-phase browser harness (sq-3ul2n.1: Chromium/Firefox/WebKit + Node, shipped bundle only) and `compare.mjs`, the cross-library latency comparison (sparq vs oxigraph-npm vs N3.js+quadstore) on the same oracle-checked workload — no latency row without row-count agreement. Competitor packages are gather-only installs; first-read gap record `research/gap-wasm-2026-07.md`.

## See also

- `fused-decompress-parse`, `rust-parallel-parsing` — server/native ingest internals behind the codecs you feed `fromCompressed`.
- `hdt-format` — loading `.hdt` archives (native crate, not in the wasm bundle).
- `noir-circuit-patterns`, `noir-optimisation`, `mpc-protocols`, `verifiable-credentials-zk`, `sparql-formal-semantics` — the ZK/MPC estate (separate `zk` feature; not part of the JS/wasm surface).
