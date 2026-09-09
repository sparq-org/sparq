# @rdfjs-test/conformance

Reusable **RDF/JS conformance test suites** that run against **any** RDF/JS
implementation you pass in. They mirror the design of the official
`@rdfjs/data-model/test` (which exports `runTests({ factory, mocha })`) **but
have ZERO external test-framework dependency**: they default to Node's built-in
[`node:test`](https://nodejs.org/api/test.html) (`describe`/`test`) plus
`node:assert/strict`, so a consumer drives them with `node --test`.

> **Candidate contribution.** This package is a **proposal** intended to be
> offered under <https://github.com/rdfjs/> after maintainer review. It is not
> yet published to npm (`"private": true`). The name `@rdfjs-test/conformance`
> is provisional and subject to the rdfjs maintainers' decision. The drafted
> proposal — venue, name and publish-to-npm options — lives in
> [`research/rdfjs-conformance-upstream.md`](../../research/rdfjs-conformance-upstream.md);
> it has **not** been filed upstream yet.

Every assertion derives its expected value **from the RDF/JS spec**
([data-model](https://rdf.js.org/data-model-spec/),
[dataset](https://rdf.js.org/dataset-spec/),
[stream](https://rdf.js.org/stream-spec/)) — never from any single
implementation. Quads are compared with an implementation-agnostic
N-Quads-ish key built from `.value` / `.termType` / `.language` /
`.datatype.value`, so the suites pass `n3` and `@rdfjs/dataset` alike, and fail
a deliberately-broken impl.

## 🚀 Quickstart

Author a test file and run it with `node --test`:

```js
// test/conformance.test.mjs
import { DataFactory, Store } from 'n3';
import { runDataFactoryTests, runDatasetTests } from '@rdfjs-test/conformance';

await runDataFactoryTests({ factory: DataFactory, label: 'n3' });
await runDatasetTests({
  factory: DataFactory,
  datasetFactory: (quads) => new Store(quads ? [...quads] : undefined),
  label: 'n3 (Store)',
});
```

```bash
node --test "test/*.test.mjs"
```

Run the same suites against the sparq WASM engine (`@sparq-org/sparq`) — anything
that exposes an RDF/JS `DataFactory` / `DatasetCore` works the same way:

```js
import { DataFactory, Store } from '@sparq-org/sparq';
import { runAll } from '@rdfjs-test/conformance';

await runAll({
  factory: DataFactory,
  datasetFactory: (quads) => new Store(quads ? [...quads] : undefined),
  label: '@sparq-org/sparq',
});
```

## ✨ Features

Three reusable runners, each accepting an optional `{ describe, test }` override
so they are framework-agnostic in spirit (default: `node:test`):

- **`runDataFactoryTests({ factory, label })`** — `namedNode` / `blankNode` /
  `literal` / `variable` / `defaultGraph` / `quad` / `fromTerm` / `fromQuad`
  plus the `Term#equals` hierarchy. Covers termType/value constants, `equals`
  reflexivity/symmetry and negatives across all term types, literal language
  **lowercasing** + datatype defaults (`xsd:string` plain, `rdf:langString`
  lang-tagged), the quad default graph, fresh blank-node uniqueness, and
  `fromTerm`/`fromQuad` deep-copy — **including copying a foreign plain-object
  term**. `variable` is skipped gracefully if absent.
- **`runDatasetTests({ factory, datasetFactory, label })`** — the **DatasetCore**
  surface (`size`, `add` idempotent + returns this, `delete`, `has`, all `match`
  wildcard combos + empty result + a new independent result, `[Symbol.iterator]`)
  and the **Dataset algebra** (`addAll`, `deleteMatches`, `union`,
  `intersection`, `difference`, `contains`, `equals`, `filter`, `map`,
  `forEach`, `some`, `every`, `reduce`, `toArray`, `toCanonical`, `toString`,
  `import`, `toStream`). Each algebra method is **feature-detected and probed**,
  so a DatasetCore-only impl (e.g. `@rdfjs/dataset`) still passes the core part.
- **`runStreamTests({ source, sink, factory, label })`** — the **optional**
  Stream/Source/Sink surface; a no-op-with-skip when no `source`/`sink` is given.
- **`runAll(impl)`** — runs whichever of the above are wired in `impl`
  (`{ factory, datasetFactory, source, sink, label }`).

Hand-written TypeScript declarations (`src/index.d.ts`, typed against
`@rdfjs/types`) ship alongside the plain-ESM runtime, so `tsc` consumers get
full types with no transpile step.

## 📚 Learn more

- RDF/JS specs: [data-model](https://rdf.js.org/data-model-spec/) ·
  [dataset](https://rdf.js.org/dataset-spec/) ·
  [stream](https://rdf.js.org/stream-spec/)
- Prior art: [`@rdfjs/data-model`](https://github.com/rdfjs-base/data-model) and
  its mocha-driven `runTests` harness, which this package re-imagines without a
  mocha dependency.
- `test/n3-parity.test.mjs` in this package proves the harness is
  implementation-agnostic: it runs the suites against **N3.js** and
  **`@rdfjs/dataset`** and is green.

No performance number is asserted anywhere in this package.

## License

MIT.
