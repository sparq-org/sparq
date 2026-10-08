# sparq-terse

> 🤖 **SPARQ agent** — an opt-in, **verifiable** LLM-ergonomic SPARQL
> surface: a *pre-parse* transpiler that always emits **canonical, conformant
> SPARQL** — it never touches the vendored `spargebra` grammar. The engine only ever
> runs standard SPARQL the agent can inspect. Research prototype (epic `sq-2m6zm`,
> design `research/llm-ergonomic-sparql-surface.md`, PR #1074); NOT published.

The motivating finding (design §1.1): the dominant first-shot failure in
text-to-SPARQL is **semantic grounding** (picking the wrong IRI), *not* verbosity.
So the high-value lever is `V("phrase")` concept resolution, behind a soundness
envelope — *a convenience that shows its work, never an oracle that hides it*.

## 🚀 Quickstart

Phase 1 + 3 — the verifiable transpiler + the `K:<name>` keyword layer (default build,
lean: only `spargebra`):

```rust
use sparq_terse::terse_to_sparql;

// Canonical SPARQL passes through BYTE-IDENTICAL, after the silent-rewrite canary
// (re-parse under spargebra) guarantees the emission is conformant.
let exp = terse_to_sparql("SELECT ?s WHERE { ?s <http://ex/p> ?o }")?;
assert_eq!(exp.canonical_sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?o }");

// Lever 1: a K:<name> keyword expands to its frozen IRI (no PREFIX line needed); the
// expansion is echoed in exp.keywords and lands in canonical_sparql.
let exp = terse_to_sparql("SELECT ?f WHERE { ?f K:type K:Finding ; K:derivedFrom ?s }")?;
assert!(exp.canonical_sparql.contains("<http://www.w3.org/ns/prov#wasDerivedFrom>"));
// An unknown keyword or a clash with a real `PREFIX K:` is a HARD error, never a guess.
assert!(terse_to_sparql("ASK { ?s K:notAKeyword ?o }").is_err());

// A V("...") construct in the default build fails LOUDLY (needs the `vectors` feature).
assert!(terse_to_sparql("SELECT ?f WHERE { ?f <http://ex/about> V(\"cats\") }").is_err());
# Ok::<(), sparq_terse::TerseError>(())
```

Phase 2 — `V("phrase")` lexical-first concept resolution (`vectors` feature):

```rust,ignore
use sparq_terse::{terse_to_sparql_with, ResolveCtx};
use sparq_core::Graph;

let graph = Graph::load_str(turtle, "turtle")?;
let ctx = ResolveCtx::lexical(&graph);            // no model, no network (the default)
// embedder is consulted ONLY for the vector fallback (a phrase lexical linking misses):
let exp = terse_to_sparql_with(
    "SELECT ?f WHERE { ?f <http://ex/about> V(\"cardinality estimation\") }",
    &ctx,
    |_phrase| None,                                // lexical-only here
)?;
// canonical_sparql now has <iri> spliced in; every bind is echoed for the agent:
for r in &exp.resolutions {
    println!("V(\"{}\") -> <{}>  score {:.3} confidence {:.3} via {}",
             r.phrase, r.iri, r.score, r.confidence, r.method.as_str());
}
```

## ✨ Features

- **Always canonical, verifiable.** `terse_to_sparql` returns `Expansion {
  canonical_sparql, resolutions, warnings }`. `canonical_sparql` is standard SPARQL,
  is what runs, and is what the agent sees — there is no hidden rewrite path.
- **Silent-rewrite canary** (design §6.7). Every emission is re-parsed under the
  unmodified `spargebra` parser; a non-parsing output is `TerseError::CanaryFailed`,
  never handed back. Canonical input is emitted byte-identical (no silent rewrite).
- **The `K:<name>` keyword layer** (lever 1, default build, design §3.1). A small,
  fixed, versioned legend (`legend()`, `LEGEND_VERSION`) of the PKG hot
  predicates/classes — `K:derivedFrom` → `<…prov#wasDerivedFrom>` — expanded pre-parse
  so an agent skips the `PREFIX` line. Every expansion is echoed in `Expansion.keywords`;
  an unknown keyword (`TerseError::UnknownKeyword`, with did-you-mean) and a clash with a
  real `PREFIX K:` (`TerseError::KeywordPrefixCollision`) are HARD errors, never a guess.
  Publish the legend once behind the prompt-cache breakpoint with `legend_card()` (the
  token win is a *caching* property — design §1.6).
- **`V("phrase")` lexical-first concept resolution** (`vectors` feature, design §3.3).
  The deterministic, no-model `sparq-nlq` lexical linker is the PRIMARY path; the
  staleness-guarded `sparq-vectors` search is a FALLBACK for genuinely fuzzy phrases.
- **The §6 soundness envelope, enforced.** Every bind echoes IRI + score + runner-up +
  confidence + method; below the confidence floor or inside the ambiguity margin `V()`
  refuses to bind (`TerseError::Unresolved` with candidates) — loud-fail beats
  silent-wrong; a stale vector store is a hard `TerseError::StaleStore` (§6.5).
- **Opt-in / lean-core by construction.** The default build depends only on
  `spargebra` (to re-parse its own output). The `vectors` feature is the only thing
  that pulls `sparq-core`/`sparq-nlq`/`sparq-vectors`. **Zero edits** to `sparq-core`,
  `sparq-engine`, or the vendored parser. The embedding model is the *caller's*
  explicit dependency (this crate never embeds free text itself; design §6/§9 Q5).

## 📚 Learn more

- Design record: `research/llm-ergonomic-sparql-surface.md` (PR #1074) — the lever
  ranking (§7), the soundness envelope (§6), and the falsifiable token/quality A/B (§5).
- Adoption verdict (`bench/terse/RESULTS.md`, bead `sq-bzign`, PR #1174 — MEASURED,
  work-box / NON-CANONICAL): **lever 1 (`K:`) is a conditional adopt** — clears the
  cache-discounted token bar *and* ties plain SPARQL on quality, pending a real-transcript
  fan-out (`sq-bmpzd`). **Lever 3 (`V()`) is do-NOT-adopt on quality** — it is **not** a
  drop-in for an explicit IRI; in the A/B it (correctly) loud-failed on a punctuation-heavy
  verbatim `prefLabel`, dropping resolution-correctness below 1.0 (the envelope working as
  specified). Resolver-coverage fix tracked in `sq-26fdp`.
- The reused machinery: `crates/sparq-nlq/src/link.rs` (lexical entity linking),
  `crates/sparq-vectors/src/ann.rs` (`nearest_exact`, the staleness-guarded
  `nearest_term_exact_checked`), `crates/sparq-vectors/src/store.rs`
  (`VectorStore::check_graph`).
- Epic `sq-2m6zm` (dogfood sparq as a Project Knowledge Graph): this surface is
  `sq-leg8n` (Phase 1 skeleton + Phase 2 `V()`) and `sq-vfeme` (Phase 3 keyword layer);
  Phase 5 (the A/B verdict) landed in `sq-bzign` (above). Remaining follow-ups: the
  full-session transcript fan-out (`sq-bmpzd`), the `V()` resolver-coverage fix (`sq-26fdp`),
  and server/CLI exposure of the transpiler (`sq-vczh2`). The keyword set is **frozen at v1**.

## License

MIT — see the repository-root [`LICENSE`](../../LICENSE). © 2026 Jesse Wright.
