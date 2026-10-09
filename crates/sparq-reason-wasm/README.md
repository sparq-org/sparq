<!-- sq-inzv: full-template README — tier-b W-reason WASM showcase bundle. -->
# sparq-reason-wasm

**The tier-b "W-reason" WebAssembly bundle** (sq-6qw3) for
[`sparq-reason`](../sparq-reason/README.md) — RDFS / OWL 2 RL / Notation3
forward-chaining **inference, live in the browser tab**.

This is a SEPARATE, lazy-loaded bundle from the lean [`sparq-wasm`](../sparq-wasm/README.md)
triplestore bundle. The lean bundle deliberately carries no reasoning code; the
showcase site's `/surface/inference` page loads this bundle on demand
(`next/dynamic`, client-only) so the landing page stays light. It mirrors the
per-bundle-crate pattern of `sparq-wasm`.

> Distributed via npm, not crates.io (`publish = false`). It is a wasm packaging
> layer over `sparq-reason`, built via `wasm-pack`, not a Rust library dependency.

## 🚀 Quickstart

```sh
wasm-pack build crates/sparq-reason-wasm --target web --release
```

```js
import init, { Reasoner } from "./sparq_reason_wasm.js";
await init();

// Full RDFS closure (asserted base + every entailed triple) as N-Triples:
const closure = Reasoner.materialize(turtle, "turtle", "rdfs");
// Only the NEWLY-entailed triples (the "what reasoning added" delta):
const added = Reasoner.entailed(turtle, "turtle", "owl-rl");
// Base vs entailed counts, without serialising the closure:
const { baseTriples, closureTriples, entailed } =
  JSON.parse(Reasoner.materializeStats(turtle, "turtle", "rdfs"));
// Notation3 rule reasoning ({ … } => { … }) — the full ground closure:
const derived = Reasoner.reasonN3(n3);
// eye-js-compat: the newly-derived triples only (EYE --pass-only-new), and the
// query filter (EYE --query, a CONSTRUCT over the closure). See @sparq-org/eyereasoner-compat.
const onlyNew = Reasoner.reasonN3New(n3);
const selected = Reasoner.reasonN3Query(dataN3, queryN3);
```

## ✨ Features

- **Stateless one-shot entry points.** `profile` is `"rdfs"` or `"owl-rl"` (OWL 2 RL
  includes RDFS). Document syntaxes are those `sparq_core::Graph::load_str` accepts:
  `"turtle"`, `"ntriples"`, `"nquads"`, `"trig"` (named graphs folded into the
  default graph). Output is canonical N-Triples (a valid Turtle subset), serialised
  through the engine's tested CONSTRUCT path — this bundle owns no
  term-serialisation code of its own.
- **eye-js migration entry points.** `Reasoner.reasonN3New(n3)` returns only the
  newly-derived ground triples (the EYE `--pass-only-new` delta); `Reasoner.reasonN3Query(data,
  query)` materialises the closure of `data` then evaluates the N3 query rule(s) as a SPARQL
  `CONSTRUCT` over it (the EYE `--query` filter), failing closed on query builtins/formulae/
  lists/quoted-triple terms. These back the `@sparq-org/eyereasoner-compat` npm package (the drop-in eye-js API).
- **`why()` proof trees — opt-in `explain` feature.** Built with `--features explain`,
  the bundle also exposes the derivation **proof tree** for a single entailed triple
  via `Reasoner.why(...)` (RDFS / OWL 2 RL) and `Reasoner.whyN3(...)` (N3 rules,
  over the same combined rules+facts document `reasonN3` consumes), returning
  `sparq-reason`'s flat, premises-before-conclusion `ProofTree::to_json` shape
  (leaves are `"asserted"` base facts; internal nodes name the rule that fired —
  `n3-rule-<i>` for N3) — **one** derivation (the first in deterministic search
  order), or `null` if not entailed. `explain` is OFF
  by default, so the standard bundle carries zero proof-tree code; CI builds and
  tests the bundle in BOTH feature states.
- **Single-threaded + pure-Rust.** No `rayon` in the bundle. The reasoner's `regex`
  dependency (the N3 `string:matches` builtin) compiles to
  `wasm32-unknown-unknown` — the regex automata are the bundle-size consideration the
  design flags, which is why this bundle is lazy-loaded on its page only and built
  `-Oz`.

## 📚 Learn more

- **How-to** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md).
- **Performance** — we deliberately quote **no** hard-coded byte/MB figure here
  (bundle size drifts with the `rustc` / `wasm-bindgen` / `wasm-opt` toolchain and
  dependency versions). The gzip transfer size — what end users actually download,
  since the site serves the `.wasm` gzip-compressed — is reproducible per toolchain:

  ```sh
  wasm-pack build crates/sparq-reason-wasm --target web --release  # add `-- --features explain` for the why() variant
  f=crates/sparq-reason-wasm/pkg/sparq_reason_wasm_bg.wasm
  echo "pre-gzip: $(stat -c%s "$f") bytes   gzip -9: $(gzip -9 -c "$f" | wc -c) bytes"
  ```

- **Status** — this crate delivers the wasm-compatibility changes, the `Reasoner`
  entry points, the `why()` proof-tree binding (opt-in), and a headless
  `wasm-pack test --node` smoke suite. The npm wrapper packaging and the GitHub Pages
  deploy wiring are tracked separately (the inference page bead sq-0po6 and the Pages
  workflow).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
