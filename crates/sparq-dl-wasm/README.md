# sparq-dl-wasm

**The OWL 2 DL reasoner WebAssembly bundle** behind `checkOwlDl` in the
[`@sparq-org/sparq`](../../js/README.md) npm package: the `owl2dl` module of
[`sparq-reason-dl`](../sparq-reason-dl/README.md) (fail-closed reverse RDF mapping,
OWL 2 DL profile check, hypertableau consistency and classification) compiled to wasm.

It is a SEPARATE bundle from the lean [`sparq-wasm`](../sparq-wasm/README.md) one. The
npm package ships it as `wasm-dl/` and imports it dynamically on the first `checkOwlDl`
call, so pages that never call it never fetch it.

> Distributed via npm, not crates.io (`publish = false`). It is a wasm packaging layer
> over `sparq-reason-dl`, built with `wasm-pack`, not a Rust library dependency.

## 🚀 Quickstart

```sh
cd js && npm run build:wasm:dl   # wasm-pack build ../crates/sparq-dl-wasm --target web + copy to js/wasm-dl/
```

```js
import init, { checkOwlDl } from "./sparq_dl_wasm.js";
await init();
// tbox, abox (or undefined), format ("turtle" | "ntriples" | "nquads" | "trig"),
// options JSON (or undefined): {"budget":{"maxSteps":N,"maxNodes":N}}
const result = JSON.parse(checkOwlDl(tbox, abox, "turtle", undefined));
```

Most callers want the typed wrapper instead: `import { checkOwlDl } from '@sparq-org/sparq'`.

## ✨ Features

- **One stateless export.** `checkOwlDl` parses the TBox and the optional ABox into ONE
  graph (blank-node labels never join across the two documents), runs
  `owl2dl::extract` then `owl2dl::check` (which runs the profile check itself), and returns
  the result as a JSON string:

  ```text
  { "consistent": true | false | "unknown",
    "reason": null | { "kind": "OutOfFragment", "construct", "triples" }
                   | { "kind": "NotInDl", "violations" } | { "kind": "ResourceBudget" }
                   | { "kind": "DatatypeUnknown", "triples" } | { "kind": "Unconfirmed" }
                   | { "kind": "ImportsNotLoaded", "imports" }
                   | { "kind": "Unparsed" | "Malformed" | "Ambiguous" | "Unsupported", "detail", "triples" },
    "profile": { "dl": true | false | "unknown", "violations": [{ "kind", "detail", "triples" }],
                 "unchecked": [{ "rule", "triples" }] },
    "unsatisfiable": [iri], "inferredSubClassOf": [[sub, super]],
    "classification": "complete" | { "unknown": kind } | null }
  ```

  `triples` are the input triples behind each answer as N-Triples lines (blank-node labels may be renamed or generated). Field-by-field semantics:
  [`skills/javascript-wasm/references/owl-dl.md`](../../skills/javascript-wasm/references/owl-dl.md).
- **Fail-closed.** An ontology the reverse mapping refuses is a result
  (`"consistent": "unknown"` with the refusal as `reason`), not an error; only a parse error
  or malformed options throw. The top and bottom properties,
  datatypes outside the OWL 2 map and object/data property punning are outside the decided
  fragment (SROIQ(D) without them) and answer `"unknown"` with reason `OutOfFragment`; an undecided
  datatype question answers `DatatypeUnknown`, and a model with nominals the reasoner
  cannot confirm `Unconfirmed`.
- **No serde.** The JSON is written (and the options object parsed) by a small hand-written,
  unit-tested JSON module; all logic is the plain-Rust `check_owl_dl_json`, tested natively
  with `cargo test -p sparq-dl-wasm`. The `#[wasm_bindgen]` export only maps the error;
  `tests/web.rs` drives it in a real wasm runtime (`wasm-pack test --node`).
- **Single-threaded + pure-Rust.** `sparq-core` without default features (no rayon); the
  wasm32 graph is guarded by `scripts/wasm-deps-guard.sh`.

## 📚 Learn more

- **How-to (JS)** — [`skills/javascript-wasm/references/owl-dl.md`](../../skills/javascript-wasm/references/owl-dl.md).
- **The reasoner** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (the
  `owl2dl` part of the OWL 2 Direct Semantics section).
- **Size** — not quoted here (it drifts with the toolchain); `js.yml` prints the built
  `wasm-dl/sparq_dl_wasm_bg.wasm` size in its log.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
