# @sparq-org/eyereasoner-compat

A **drop-in migration package for [eye-js](https://github.com/eyereasoner/eye-js)**: the same
`n3reasoner(data, query?, options?)` API, backed by [sparq](https://github.com/sparq-org/sparq)'s
native Notation3 forward-chaining reasoner compiled to WebAssembly — **no SWI-Prolog**, and a
lighter browser payload. Migrating is a one-line `package.json` change.

## 🚀 Quickstart

```sh
npm install @sparq-org/eyereasoner-compat
```

```js
import { n3reasoner } from '@sparq-org/eyereasoner-compat';

const data = `
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#>.
@prefix : <http://example.org/socrates#>.
:Socrates a :Human.
:Human rdfs:subClassOf :Mortal.
{?A rdfs:subClassOf ?B. ?S a ?A} => {?S a ?B}.`;

const query = `@prefix : <http://example.org/socrates#>. {:Socrates a ?WHAT} => {:Socrates a ?WHAT}.`;

await n3reasoner(data, query);   // => ":Socrates a :Human, :Mortal" (as N-Triples)
await n3reasoner(data);          // => all inferred (derivations)
```

The wasm engine ships **inside the tarball** and loads with no configuration (Node reads it
from disk; browsers/CDNs resolve it relative to the module). No network fetch of a hosted asset.

### CDN usage (no bundler)

```html
<script type="module">
  import { n3reasoner } from 'https://esm.sh/@sparq-org/eyereasoner-compat';
  console.log(await n3reasoner('@prefix : <http://ex/>. {:a a :A} => {:a a :B}. :a a :A.'));
</script>
```

Also available via `https://cdn.jsdelivr.net/npm/@sparq-org/eyereasoner-compat/+esm` and
`https://unpkg.com/@sparq-org/eyereasoner-compat`. If a bundler cannot auto-resolve the wasm,
point at it explicitly: `import { configureWasm } from '...'; configureWasm(new URL(...));`
before the first call.

### Classic `<script>` tag (no modules, no bundler)

For a page that cannot use `<script type="module">`, the tarball also ships a **non-module IIFE
bundle** at `dist/eyereasoner-compat.iife.js`. It defines one global, `eyereasoner`:

```html
<script src="https://cdn.jsdelivr.net/npm/@sparq-org/eyereasoner-compat/dist/eyereasoner-compat.iife.js"></script>
<script>
  eyereasoner
    .n3reasoner('@prefix : <http://ex/>. {:a a :A} => {:a a :B}. :a a :A.')
    .then(console.log);
</script>
```

The global carries the same named surface as the ESM entry (`n3reasoner`, `configureWasm`,
`dataFactory`, `parseNTriples`, `writeQuads`, and the migration stubs).

The engine wasm is **not** inlined into the script — it is fetched lazily on the first
`n3reasoner(...)` call from `../wasm/` **relative to the `<script src>` itself** (a classic
script has no `import.meta.url`, so the loader reads `document.currentScript` instead). That
works as-is from any npm CDN and from a self-hosted copy of the `dist/` + `wasm/` pair. If you
relocate or inline the script, name the engine explicitly first:

```html
<script>eyereasoner.configureWasm('/assets/sparq_reason_wasm_bg.wasm');</script>
```

This build is browser-only; in Node, use the ESM entry (`import { n3reasoner } from '@sparq-org/eyereasoner-compat'`).

## ✨ Features

- **`n3reasoner` — the eye-js drop-in.** Same overloads: `string`/`[string,…]` → `string`,
  `Quad[]` → `Quad[]` (RDF/JS), overridable with `options.outputType: 'string' | 'quads'`.
  String output is canonical **N-Triples** (a valid Turtle/N3 subset).
- **Output modes** (`options.output`, query-less calls):

  | mode | eye-js flag | sparq backing | status |
  | --- | --- | --- | --- |
  | `derivations` *(default)* | `--pass-only-new` | newly-derived triples only | ✅ |
  | `deductive_closure` | `--pass` | full ground closure | ✅ |
  | `none` | — | empty result | ✅ |
  | `deductive_closure_plus_rules` | `--pass-all` | — | ❌ throws (deferred) |
  | `grounded_deductive_closure_plus_rules` | `--pass-all-ground` | — | ❌ throws (deferred) |

  The `…_plus_rules` modes echo the *rules* into the output; sparq's chainer consumes rules and
  emits only ground triples, so they fail loudly rather than return a different result set.
- **Query filter.** A query `{ premise } => { conclusion }` is evaluated over the deductive
  closure of the data (EYE `--query` semantics): every instantiated conclusion is an answer,
  including one already present in the closure. The premise runs through the reasoner's own
  matcher, so **builtins, quoted `{ … }` formulae and first-class `( … )` lists all work** in a
  query rule, exactly as in a document rule. Combining an explicit `output` with a query throws,
  as in eye-js. The query document's own *facts* are not loaded as data; a query document with
  no forward rule is an error rather than an empty answer.
- **Builtins coverage (honest).** sparq's N3 engine supports a subset of EYE's library:
  `math:` (`sum`, `difference`, `product`, `quotient`, `greaterThan`, `lessThan`, `equalTo`, …),
  `string:` (`concatenation`, `contains`, `startsWith`, `endsWith`, `matches`, `replace`, …),
  `list:` (`member`, `append`, `memberCount`, `first`, `last`, …), `time:` and `log:` core.
  EYE's **full** builtin library is larger; a rule using an unimplemented builtin simply does not
  fire — in a *query* rule too, so such a query answers nothing rather than answering wrongly.
  See `skills/inference/SKILL.md`.
- **SWIPL/EYE-image surface → migration stubs.** `SwiplEye`, `loadEyeImage`, `loadImage`,
  `runQuery`, `buildQuery`, `qaQuery`, `query`, `queryOnce`, `executeBasicEyeQuery`,
  `linguareasoner`, `EYE_PVM` are re-exported so imports compile, but **throw** a clear
  migration error — they are inherently SWI-Prolog-bound and cannot be backed by sparq. The
  `SWIPL` / `cb` options are accepted and **warn-and-ignore**.
- **Lighter payload.** The single wasm asset is markedly smaller than eye-js's SWI-Prolog build
  (`swipl-web.wasm` + `swipl-web.data`, ~3.83 MB combined). Reproduce the exact bytes:

  ```sh
  wasm-pack build ../../crates/sparq-reason-wasm --target web --release --out-dir pkg-eyereasoner-compat
  f=../../crates/sparq-reason-wasm/pkg-eyereasoner-compat/sparq_reason_wasm_bg.wasm
  echo "raw: $(stat -c%s "$f")  gzip -9: $(gzip -9 -c "$f" | wc -c)"
  ```

## 📚 Learn more

- **Reasoning semantics / builtins** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md).
- **The wasm engine** — [`crates/sparq-reason-wasm`](../../crates/sparq-reason-wasm/README.md).
- **eye-js** — the upstream API this mirrors: <https://github.com/eyereasoner/eye-js>.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
