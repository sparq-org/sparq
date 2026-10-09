<!-- sq-inzv: README brought to template. -->
# sparq-nlq

<p>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Natural-language questions → SPARQL** over a sparq graph — an **opt-in** crate (GenAI
phase 3) built as a deliberately **lean loop** (retrieve + repair beats sprawling agents).
Only opt-in `sparq-mcp` / `sparq-terse` features depend on it, the default build omits it, and the engine carries zero GenAI code.

The loop: **GROUND** (`sparq_introspect` schema summary + few-shot examples + question)
→ **GENERATE** (`Llm::complete`) → **VALIDATE** (extract the ` ```sparql ` block,
`spargebra` parse) → on error **REPAIR** (≤ N rounds) → **EXECUTE**
(`sparq_engine::query_with_budget`) → `Answer { sparql, result, repairs, transcript }`.

## 🚀 Quickstart

```rust,no_run
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# use sparq_core::Graph;
use sparq_nlq::{Nlq, ReplayLlm};

# let graph = Graph::load_str("", "turtle")?;
// Offline / CI path: serve recorded prompt→completion pairs from a fixture.
let replay = ReplayLlm::from_file("tests/fixtures/olympics_replay.json")?;
let nlq = Nlq::new(&graph, Box::new(replay));
let answer = nlq.ask("How many athletes are on each team?")?;
println!("{}\n{} rows, {} repair(s)", answer.sparql, answer.result.len(), answer.repairs);
# Ok(()) }
```

Two opt-in network backends (both off by default): `--features live` wraps
`live::AnthropicLlm::from_env()?` (`ANTHROPIC_API_KEY`) in a `RecordingLlm` to capture
fixtures; `--features nlq-endpoint` points `EndpointLlm` at **your own** base URL.

## ✨ Features

- **Grounded generation** — the introspect crate's token-budgeted schema summary (exact
  counts from the store's permutation indexes, not guesses) + two schema-agnostic few-shots.
- **Index-grounded linking — no model, no network** ([`link`](src/link.rs), opt-in, both default **off**:
  each changes the prompt string, so re-record fixtures first). `NlqConfig::link_entities` resolves proper
  nouns to IRIs present in *this* store (label index over `rdfs:label`/`skos:prefLabel`/… plus predicate
  local-names) and expands each with structurally-similar siblings via [`sparq-sim`](../sparq-sim).
  `NlqConfig::link_values` adds the **exact dictionary** tier: a question span that IS a literal the store
  holds binds with its **datatype and language tag** (`"1994"^^xsd:gYear`, `"France"@en`) plus the predicates
  it objects, and an IRI written verbatim is probed via `Graph::id_of` — so `FILTER`/value-bound queries stop
  guessing lexical forms the store does not have. <!-- sq-uw40 · sq-na0q -->
- **Validate before execute** — parses with `spargebra` *before* the engine sees the
  query; execution failures (unsupported forms, budget trips) are repair signals too.
- **Dictionary-grounded repair** ([`constrain`](src/constrain.rs), opt-in via
  `NlqConfig::check_dictionary`) — once a query parses, every predicate / `rdf:type` class
  IRI is checked against the **live dictionary** (`Graph::id_of`); an ungrounded IRI becomes
  a *targeted* did-you-mean repair signal. Strict logit-level grammar-constrained
  **decoding** needs logit access the Messages API does not expose: **not implemented, not
  claimed** — this is the design doc's fallback (`genai-nl-to-sparql.md` §11). <!-- sq-9yjp -->
- **Injection + budget hardening** ([`guard`](src/guard.rs), **on by default**) — the
  question, the graph-derived prompt text (labels, schema samples) and the model's echoed
  output are untrusted: each is fence-neutralized and pinned to its prompt line, the
  question is length-capped *before* any LLM call, and a query that federates (`SERVICE` —
  the exfiltration/SSRF payload) is **refused before execution** as a repair signal. No-op
  on benign text, so fixtures still replay. This bounds an injection's **consequences**;
  no text transform *prevents* one, and none is claimed —
  [`research/nlq-threat-model.md`](../../research/nlq-threat-model.md). <!-- sq-j1wv -->
- **Budgeted execution** — LLM output is untrusted, so every query runs under a
  [`QueryBudget`] (default **bounded**: 10 s, 1M rows; opting out is explicit). Mutation
  is unreachable, not filtered: the loop parses with `parse_query`.
- **Full provenance** — `Answer` carries the final query, result, repair count and the
  **full transcript**; `prompt_for`/`repair_prompt_for` are public + deterministic.
- **Citations from provenance** ([`cite`](src/cite.rs) / [`provenance`](src/provenance.rs),
  opt-in via `--features citations`, default **off**) — `Answer::citations(&graph)` resolves
  each answer-row binding to its `prov:wasDerivedFrom` source + `dcterms:source` anchor +
  `pkg:confidence`/`pkg:assurance` as numbered footnotes — **emitted from provenance, never
  generated** (rate 1.0, zero fabricated refs); unsourced ⇒ **"no source recorded"**.
- **Answer-qualification — hedge + abstention** ([`qualify`](src/qualify.rs), opt-in via
  `--features citations` + `NlqConfig::qualify`, default **off**) — `Nlq::ask_qualified`
  folds the answer's supporting `pkg:assurance`/`pkg:confidence` (the same Phase-1 join)
  **weakest-link** into a verb hedge + verbal band, and below a `min_confidence` floor
  **abstains**. The band *reflects asserted assurance*, **not** calibrated confidence.
- **`Llm` trait — record/replay, offline CI** — `ReplayLlm` serves recorded pairs (the CI
  path); `RecordingLlm<L>` wraps any backend and `save()`s a fixture.
- **Configurable endpoint client** ([`endpoint`](src/endpoint.rs), opt-in via `--features
  nlq-endpoint`, default **off**) — `EndpointLlm`, a **provider-agnostic** OpenAI-compatible
  chat-completions client whose base URL, model, and (optional) key are **entirely
  user-supplied** (args or `SPARQ_NLQ_ENDPOINT_*` env), so an external user runs it against
  their **own** endpoint (Ollama, vLLM, OpenAI…). Nothing is baked in, it never phones
  home; **quality depends on the user-chosen model**. <!-- sq-2m6zm.6 -->
- **Exec-accuracy harness** ([`eval`](src/eval.rs)) — grades executed SPARQL the QALD way (**answer-set F1**, not query-string equality) against a live-recomputed gold query.

## Honest status — what is and is not measured

- **Validated offline (the CI gate)**: the engine-side loop and the exec-accuracy harness,
  end-to-end on real data with scripted/recorded backends; the
  record→`save`→`from_file`→identical-scores round-trip is CI-gated.
- **NOT yet measured against a real model**: live-model exec-accuracy *numbers* — the
  offline scores demonstrate the *harness* + *mechanism*, **not** a real model's accuracy;
  the live/endpoint paths need a key + a **canonical** host (`sq-g0lw`), and both clients
  are stub-tested, never networked. Grammar-constrained decoding stays unimplemented and
  unclaimed (`sq-9yjp`). <!-- sq-05rv sq-g0lw sq-9yjp -->
- **NOT solved — prompt injection itself.** The hardening bounds an injection's
  *consequences*; it does not stop a model being talked into a different query.
  [`research/nlq-threat-model.md`](../../research/nlq-threat-model.md) states exactly what
  is and is not contained. <!-- sq-j1wv -->

## 📚 Learn more

- **How-to** — [`skills/genai-retrieval/SKILL.md`](../../skills/genai-retrieval/SKILL.md);
  **API reference** — `cargo doc -p sparq-nlq --open` (not on crates.io / docs.rs: `publish = false`).
- **Design** — [`research/genai-nl-to-sparql.md`](../../research/genai-nl-to-sparql.md)
  (§4.3), [`research/genai-design.md`](../../research/genai-design.md) (§4); grounding in
  [`sparq-introspect`](../sparq-introspect); **threat model** in
  [`research/nlq-threat-model.md`](../../research/nlq-threat-model.md).
- **Performance** — the eval harnesses run network-free in CI; tracked figures live on the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench), not in docs.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE). <!-- sq-lsxd -->
