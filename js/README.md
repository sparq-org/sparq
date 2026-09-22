# @sparq-org/sparq


[RDF/JS](https://rdf.js.org/)-style bindings for **sparq**, a Rust RDF
triplestore + SPARQL engine compiled to WebAssembly. One compact wasm artifact,
one tiny runtime npm dependency ([`fzstd`](https://www.npmjs.com/package/fzstd),
dynamically imported only when ingesting zstd); works in Node ≥ 18 and the
browser. (The wasm bundle bytes are tracked per-commit on the perf dashboard,
<https://sparq.jeswr.org/dev/bench>.)

- Dictionary-encoded in-memory store (optionally block-compressed: substantially
  less index memory for a bounded scan cost).
- SPARQL 1.1 SELECT (BGPs with worst-case-optimal joins, FILTER, OPTIONAL,
  UNION, MINUS, BIND, VALUES, aggregates, ORDER BY, DISTINCT/LIMIT/OFFSET,
  sub-SELECT) and ASK — both evaluated natively (ASK early-exits at the first
  solution).
- SPARQL 1.1 CONSTRUCT / DESCRIBE: `queryQuads()` returns the constructed
  graph as RDF/JS `Quad`s, `queryQuadsString()` as raw N-Triples, and
  `queryQuadsStream()` streams a large graph one quad at a time.
- Named graphs (`options.dataset`): `GRAPH <iri>` / `GRAPH ?g` patterns,
  `FROM` / `FROM NAMED`, graph-aware `match()`.
- SPARQL 1.1 Update over the full dataset (`INSERT/DELETE DATA` with `GRAPH`
  blocks, `DELETE/INSERT … WHERE` with graph templates, `CLEAR`/`DROP`/
  `CREATE`/`ADD`/`COPY`/`MOVE`), applied in place through the engine's delta
  overlay — O(batch), no index rebuild — plus quad-level `applyDelta` /
  `addQuads` / `removeQuads`.
- RDF/JS Query streaming: `await queryBindings()` returns a Query-spec
  `ResultStream<Bindings>` with `data` / `end` / `error` events and pull
  backpressure (`read()`, plus `pause()` / `resume()`). The lower-level
  `queryBindingsStream()` generator and `queryJsonChunks()` expose the same
  ~64 KiB cursor path directly — no giant JSON blob or result array.
- Compressed ingest: `fromCompressed()` decodes `.nt.zst` (including
  multi-frame zstd from sparq's `CompressedSink`) via pure-JS `fzstd`, and
  `.gz` via the platform.
- Dictionary-fetch protocol client (`SparqDictionaryClient`): zstd
  vocabulary-dictionary negotiation with a sparq server — content-addressed
  dictionary caching, background warm-up, pluggable dict-capable decoder.
- Results as RDF/JS Query-spec `Bindings` (Map-like, `.get(variable)`), terms as
  spec-compliant RDF/JS `Term`s (typed against `@rdfjs/types`); an
  **Oxigraph-compatible** accessor (`querySolutions()` → `Map<string, Term>[]`,
  `Bindings.toMap()`) ports Oxigraph result-processing code unchanged.
- The named **`Dataset`** export implements the **full RDF/JS `Dataset`
  interface** (the set algebra `union`/`intersection`/`difference`/… on top of
  `DatasetCore`), with the binary set ops **interoperating with foreign RDF/JS
  datasets** (N3.js, `@rdfjs/dataset`), not just our own.

## Install / build

The package ships the wasm artifact; from a source checkout build it first:

```sh
npm run build   # wasm-pack build ../crates/sparq-wasm (--features shacl) + tsc
npm test        # node --test against the built dist/
```

### Reproducing the `js` CI gate locally

Install from the **repo root**, not from `js/` — this is the one step that differs
from a normal single-package checkout, and skipping it is what makes the gate look
irreproducible from a fresh worktree:

```sh
cd .. && npm ci   # repo-root npm workspaces install
cd js && npm run build && npm test
```

`js/test/rdfjs-conformance.test.mjs` imports `@rdfjs-test/conformance` (the
`packages/rdfjs-conformance` workspace member) and `js/test/solid-differential.test.mjs`
imports `@solid/acl-check` / `@solidlab/policy-engine` / `rdflib` (resolved out of the
repo-root install). Those are test-only and stay **out** of `js/package.json` on
purpose, because the `js-sbom` lane derives the published `@sparq-org/sparq` SBOM from
this manifest and test deps there would pollute the runtime component list. An
`npm ci` run inside `js/` installs only this member's own closure, so those imports
fail with `ERR_MODULE_NOT_FOUND`; a root install hoists them and links the workspace
member. `.github/workflows/js.yml` installs from the root for exactly this reason.

`npm test`'s `pretest` guardrail (`guardrails/check-test-deps.mjs`) checks this up
front and names the missing packages, rather than letting the suite fail two thirds
of the way through. To run one file without a root install, invoke the runner
directly — that path skips `pretest`:

```sh
node --test test/store.test.mjs
```

### Pinning a git build (before the npm release)

Until `@sparq-org/sparq` is published to npm under its settled name, depend on it by
**pinning a git build**. The package's `prepare` script compiles the wasm engine
+ TypeScript on install, so a git pin yields a working binding (the registry
tarball ships those prebuilt). Add to the consumer's `package.json`:

```jsonc
"dependencies": {
  // pin an immutable commit; `directory: "js"` is read from this package.json
  "@sparq-org/sparq": "github:sparq-org/sparq#<commit-sha>"
}
```

A git-pinned install needs the Rust → wasm toolchain on the build machine
(`rustup target add wasm32-unknown-unknown` + `cargo install wasm-pack --locked
--version =0.15.0`); without it `prepare` fails loudly with the install command
rather than silently shipping an engine-less binding. Name that version rather
than letting it float: it is the pin CI's gating `js` lane installs, and each
wasm-pack release bundles its own wasm-bindgen CLI, so an unversioned local
install builds this bundle with a toolchain no lane exercises. (`=0.15.0` is
cargo's exact requirement — a bare `0.15.0` means the `^0.15.0` range.) After
install, verify the engine actually landed:

```sh
node -e "import('@sparq-org/sparq').then(m=>m.SparqStore.fromString('<a> <b> <c> .','ntriples')).then(s=>{s.free?.();console.log('ok')})"
```

Maintainers run the publish guardrail this repo gates on — `npm run
check:package` — which proves `prepare` is wired, the `files` allowlist is
intact, and the packed tarball actually ships `dist/` + `wasm/*_bg.wasm`.

The default build (and the published bundle) ships with `--features shacl` so
`SparqStore.validate` works out of the box. SHACL is not free in the wasm
binary — it pulls in the SHACL engine + `regex` + the SPARQL query path for
`sh:sparql`, which roughly **doubles** the `.wasm` (measured ~1.21 MiB → ~2.19
MiB, +~1.0 MiB / +85%, before gzip). If you do not need validation and bundle
size matters, build the lean variant — `npm run build:wasm:lean` — which omits
SHACL entirely (`SparqStore.validate` then throws a clear error if called).

### CommonJS (`require`) consumers

The main entry (`@sparq-org/sparq`) is ESM-only: it is built from the `--target web`
wasm-pack output, whose glue is a real ESM module. From CommonJS, reach it with a
dynamic import — supported in CJS since Node 12.17, and the way to get the full
RDF/JS wrapper (`SparqStore`, `Dataset`, `DataFactory`, …):

```js
const { SparqStore } = await import('@sparq-org/sparq');   // inside an async function
```

`npm run build` **also** produces a `--target nodejs` build of the same engine,
with the same feature set, into `wasm-node/`. That one is plain CommonJS and
instantiates the module eagerly at `require()` time (it `readFileSync`s the
`.wasm` next to the glue), so it is `require()`-able and needs **no `init()`**:

```js
const { Store } = require('@sparq-org/sparq/wasm-node');   // synchronous, no await

const store = Store.load('<http://e/a> <http://e/b> <http://e/c> .', 'ntriples');
console.log(store.size, store.ask('ASK { ?s ?p ?o }'));   // `size` is a getter
store.free();
```

That subpath exposes the **raw generated `Store`** (the same surface documented
under *Raw wasm `Store`* in `skills/javascript-wasm/SKILL.md`) — strings in,
SPARQL-JSON strings out, explicit `.free()` — not the RDF/JS wrapper. Use it when
you want a synchronous engine in a CommonJS file; use `await import(...)` when you
want `SparqStore`/`Dataset`. The two builds are separate artifacts, so a package
that only ever uses one still downloads both in the tarball.

## Usage

```js
import { SparqStore, DataFactory as DF } from '@sparq-org/sparq';

const store = await SparqStore.fromString(`
  @prefix ex: <http://ex/> .
  ex:alice ex:name "Alice" ; ex:knows ex:bob .
  ex:bob   ex:name "Bob"@en .
`, 'turtle'); // or 'ntriples' | 'nquads' | 'trig'

// SELECT → RDF/JS ResultStream<Bindings>
const bindings = await store.queryBindings(
  'PREFIX ex: <http://ex/> SELECT ?s ?name WHERE { ?s ex:name ?name }',
);
bindings.on('data', (row) => {
  console.log(row.get('s').value, '→', row.get('name').value);
});
bindings.on('end', () => console.log('done'));
bindings.on('error', console.error);

// Synchronous materialisation remains available through query():
const rows = store.query('SELECT ?s WHERE { ?s ?p ?o }'); // Bindings[]

// ASK → boolean
store.queryBoolean('PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }'); // true

// CONSTRUCT / DESCRIBE → RDF/JS quads (default graph)
for (const quad of store.queryQuads(
  'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }',
)) {
  console.log(quad.subject.value, quad.predicate.value, quad.object.value);
}
store.queryQuadsString('DESCRIBE <http://ex/bob>'); // raw N-Triples string

// RDF/JS-style triple lookup (generated SELECT under the hood)
store.match(null, DF.namedNode('http://ex/name'), null); // → Quad[]
store.countQuads(null, DF.namedNode('http://ex/name'));  // → 2 (no materialisation)

// Build a store from RDF/JS quads (serialised to N-Quads internally)
const fromQuads = await SparqStore.fromQuads([
  DF.quad(DF.namedNode('http://ex/s'), DF.namedNode('http://ex/p'), DF.literal('o')),
]);

// SPARQL Update (the engine rebuilds its immutable index; the handle swaps in place)
store.update('PREFIX ex: <http://ex/> INSERT DATA { ex:carol ex:name "Carol" }');

// Oxigraph-compatible SELECT results: an array of plain Map<string, Term>
// (drop-in for Oxigraph's `Store.query` — `binding.get("s").value`)
for (const binding of store.querySolutions('SELECT ?s WHERE { ?s ?p ?o } LIMIT 10')) {
  console.log(binding.get('s').value);
}

// Raw SPARQL 1.1 JSON results, skipping JS-side term materialisation
const json = JSON.parse(store.queryJson('SELECT * WHERE { ?s ?p ?o } LIMIT 10'));

// Lower-level cursor generator (also no whole-result JSON/array):
for await (const row of store.queryBindingsStream('SELECT ?s ?o WHERE { ?s ?p ?o }')) {
  console.log(row.get('s').value);
}

// Incremental, O(batch) quad-level updates (no index rebuild):
store.addQuads([DF.quad(DF.namedNode('http://ex/d'), DF.namedNode('http://ex/p'), DF.literal('o'))]);
store.removeQuads(store.match(null, DF.namedNode('http://ex/p')));
store.applyDelta(insertQuads, deleteQuads); // deletes applied first, then inserts

// Named graphs: load as a DATASET (N-Quads / TriG / fromQuads keep their graphs)
const ds = await SparqStore.fromString(nquads, 'nquads', { dataset: true });
const namedRows = ds.query('SELECT ?g ?s WHERE { GRAPH ?g { ?s ?p ?o } }');
ds.update('INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> "o" } }');
ds.match(null, null, null, DF.namedNode('http://ex/g')); // graph-aware lookup

// Compressed ingest: .nt.zst (multi-frame OK) / .ttl.gz, codec sniffed by magic
const fromZst = await SparqStore.fromCompressed(zstBytes, 'ntriples');

// Memory-constrained devices: block-compressed index
const compact = await SparqStore.fromString(bigTurtle, 'turtle', { compressed: true });
compact.heapBytes(); // rough wasm-side footprint

// SHACL validation (data graph vs shapes graph) → a typed ValidationReport.
// Stateless one-shot: does NOT consult the store's own triples (a drop-in for
// rdf-validate-shacl). conforms counts every result; filter by severity for a gate.
const report = store.validate(dataTurtle, shapesTurtle, 'turtle'); // format defaults to 'turtle'
report.conforms;                                                    // boolean
for (const r of report.results) {
  console.log(r.focusNode, r.path, r.severity, r.message);          // per-violation fields
}

store.free(); // release wasm memory (also `using store = …` via Symbol.dispose)
```

### `Dataset` — the full RDF/JS `Dataset` algebra, importable from a `<script type="module">`

For an RDF/JS-`Dataset`-shaped surface (rather than the SPARQL-first
`SparqStore`), import the named **`Dataset`** export. It implements the **full
RDF/JS [`Dataset`](https://rdf.js.org/dataset-spec/) interface** — the
`DatasetCore` members (`add` / `delete` / `has` / `match` / `size` /
`[Symbol.iterator]`) **plus** the set algebra and iteration helpers
(`union` / `intersection` / `difference` / `addAll` / `deleteMatches` /
`contains` / `equals` / `filter` / `map` / `forEach` / `some` / `every` /
`reduce` / `import` / `toStream` / `toArray` / `toString` / `toCanonical`) — with
the **full SPARQL surface one accessor away** (`dataset.store`).

The binary set ops are **library-agnostic**: `union(other)`,
`intersection(other)`, `difference(other)`, `addAll`, `contains` and `equals`
accept either another sparq `Dataset` **or any foreign RDF/JS dataset/store**
(e.g. an [`N3.Store`](https://github.com/rdfjs/N3.js) or
[`@rdfjs/dataset`](https://github.com/rdfjs/dataset)) — detected through the
`[Symbol.iterator]` every RDF/JS dataset exposes, with a fast native path when
the operand is our own:

```js
import { Dataset } from '@sparq-org/sparq';
import { Store as N3Store } from 'n3';

const a = await Dataset.fromString('<http://ex/a> <http://ex/p> <http://ex/b> .', 'ntriples');
const n3 = new N3Store(/* … RDF/JS quads from N3 … */);
a.union(n3);         // works across libraries (foreign operand)
a.intersection(n3);  // ditto
a.difference(n3);
a.contains(n3);
a.equals(n3);
```

> `toCanonical` returns the **RDFC-1.0** (RDF Dataset Canonicalization) canonical
> N-Quads — blank nodes relabelled to `_:c14nN`, lines canonically sorted — so two
> datasets that are RDF-isomorphic (differ only in blank-node labels and/or quad
> order) canonicalise byte-identically. `equals` is isomorphism-aware (it compares
> canonical forms) and `contains` recognises a relabelled subgraph (a blank-node
> homomorphism), so "differences in blank node labels are ignored" per the RDF/JS
> spec. Backed by the engine's RDFC-1.0 implementation surfaced over wasm. (RDF-1.2
> triple terms are outside the W3C RDFC-1.0 data model and `toCanonical` throws on
> them.)

Because the instance methods are
synchronous, you obtain an instance through an **async factory** —
`Dataset.create()` / `Dataset.fromString()` / `Dataset.fromQuads()` — each of
which `await`s the wasm engine first. That is the lazy-load point: the
~MB `.wasm` is fetched on the first `await Dataset.…`, never on import, and the
cold start is paid at most once per page.

This is what makes the GitHub-issue ESM snippet (#981) work — you can import the
name directly in a browser `<script type="module">` from any ESM CDN, and the
engine streams in only when you first build a dataset:

```html
<script type="module">
  // From an ESM CDN (or a bundler import) — the ~MB wasm is lazily fetched by
  // the first `Dataset.fromString(...)`, not by the import below.
  import { Dataset, DataFactory as DF } from "https://esm.sh/@sparq-org/sparq";

  const ds = await Dataset.fromString(
    "<http://ex/a> <http://ex/name> \"Alice\" .",
    "ntriples",
  );
  console.log(ds.size); // 1

  ds.add(DF.quad(DF.namedNode("http://ex/b"), DF.namedNode("http://ex/name"), DF.literal("Bob")));
  for (const q of ds.match(null, DF.namedNode("http://ex/name"), null)) {
    console.log(q.subject.value, q.object.value);
  }

  // Drop to the SPARQL engine when DatasetCore is not enough:
  console.log(ds.store.queryBoolean("ASK { ?s ?p ?o }")); // true
  ds.free();
</script>
```

If you only need the low-level engine handle, the wasm-pack `--target web` glue
is itself a real ESM module — `import init, { Store } from ".../wasm/sparq_wasm.js";
await init();` — but `Dataset` (and `SparqStore`) is the ergonomic, memoised-init
entry to prefer in an app.

### Talking to a sparq server: dictionary-fetch protocol

Sparq servers compress small SPARQL responses with shared zstd *vocabulary
dictionaries* (markedly smaller on small bodies) — but only when the client proves
it already holds the dictionary, so no request ever waits on one.
`SparqDictionaryClient` wraps `fetch` with the whole negotiation:

```js
import { SparqDictionaryClient } from '@sparq-org/sparq';

const client = new SparqDictionaryClient({
  // fzstd cannot decode dictionary frames — supply a dict-capable decoder
  // (zstd-wasm in the browser; node:zlib in Node). Without it the client
  // simply never advertises dictionaries and responses stay plain zstd.
  decodeWithDictionary: (body, dict) => zstdDecompressSync(body, { dictionary: Buffer.from(dict) }),
});
const { body, dictionary } = await client.fetch('https://host/sparql?query=…');
```

Held dictionaries are advertised via `Sparq-Dictionary`; the response echoes
the one used and `Sparq-Dictionary-Current` triggers a background, content-
verified warm-up from `GET /dictionary/{dict-id}` for the *next* request.

### Notes & limits

- Named graphs are folded into the default graph **unless** the store is
  loaded with `options.dataset`; `size`/`heapBytes` always report the default
  graph (use `countQuads()` for dataset totals). `dataset` is not combinable
  with `compressed` yet.
- Federated (`SERVICE`) queries are not exposed at the JS wrapper layer (tracked in beads — `bd list -l area:js`).
- `validate()` (SHACL) runs in-process and is best for small documents
  (~10–100 triples); for large data graphs validate server-side via the
  `sparq-server` HTTP `validate` path. It needs a `--features shacl` bundle
  (shipped by default; `build:wasm:lean` omits it — see *Install / build*).
- `REGEX`/`REPLACE` are compiled out of the wasm build (the engine's
  non-default `regex` cargo feature) to keep the bundle small — use
  `CONTAINS`/`STRSTARTS`/… or a custom build.
- A specific blank node in `match()` is matched by label via a post-filter
  (SPARQL itself cannot reference a particular bnode); `applyDelta` deletes
  also address bnodes by label (unlike SPARQL `DELETE DATA`).
- `update()` and `applyDelta()` mutate in place through the engine's delta
  overlay (append-only dictionary growth; deletes are overlay tombstones until
  the wasm store is reloaded).
- Browsers silently truncate **multi-member gzip** to the first member —
  `fromCompressed` uses `node:zlib` in Node (which loops members) and
  `DecompressionStream` in the browser (single-member only); multi-frame
  **zstd** decodes fully everywhere via the bundled fzstd.
- **SPARQL-injection guard.** `match()`/`countQuads()` build a query string and
  `addQuads`/`applyDelta`/`fromQuads` build N-Quads, both embedding RDF/JS terms
  via `termToNT`. Hostile term values cannot break out of their token: IRIs are
  percent-encoded over the full `IRIREF`-illegal set (`< > " { } | ^` `` ` ``
  `\` and `#x00–#x20`, so a `>` in an ACL-pointer IRI becomes `%3E`) and literal
  values escape `"`, `\` and all control chars — the same rules QLever's lexer
  enforces. This is proved end-to-end against the engine's real parser in
  `test/injection.test.mjs`. Note percent-encoding is canonicalising: an IRI
  value that *contains* illegal chars stores under its encoded form.

## Benchmarks

`npm run bench` (see `bench/vs-oxigraph.mjs`) compares load + SELECT workloads
against [oxigraph](https://www.npmjs.com/package/oxigraph)'s npm package when it
is installed (`npm i --no-save oxigraph`); it skips gracefully otherwise.
