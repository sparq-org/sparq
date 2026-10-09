# sparq-kb

> 🤖 **SPARQ agent** — Phase-1 artifacts for dogfooding sparq as a **Project Knowledge
> Graph (PKG)**: store the project's own research findings, sources, techniques, and `bd` task model
> as RDF behind sparq's SPARQL + SHACL + reasoning surface. Research prototype; adoption is gated on a
> token-A/B verdict (epic `sq-2m6zm`, design record `research/dogfooding-sparq-knowledge-graph.md`,
> PR #1063). NOT published.

## 🚀 Quickstart

The crate ships Turtle artifacts as `&str` constants (default build is a pure data +
Rust-vocab crate — no engine code):

```text
sparq_kb::PKG_ONTOLOGY   // ontology/pkg/pkg.ttl      — the reuse-first PKG vocabulary
sparq_kb::PKG_SHAPES     // shapes/pkg.shapes.ttl     — the SHACL write-time guardrails
sparq_kb::PKG_EXAMPLE    // examples/pkg-example.ttl  — a tiny instance file
sparq_kb::PKG_FINDINGS   // ingest/agents-findings.ttl — the write-path-compiled tier
sparq_kb::PKG_INSTANCES  // ingest/pkg-instances.ttl  — the Phase-1 ingested graph
sparq_kb::PKG_TRUST_GRAPH_FINDINGS // ingest/trust-graph-findings.ttl — sq-7utko tier
sparq_kb::vocab::*       // the pkg: IRIs as constants, pinned against the Turtle
```

Dogfood the guardrails with sparq's own SHACL engine (opt-in `validate` feature): `cargo test -p
sparq-kb --features validate` loads the ontology + example instances, runs `pkg.shapes.ttl` via
`sparq-shacl`, and asserts the valid Findings/Tasks PASS while the deliberately invalid ones
(missing source/confidence, stale edge) are REPORTED.

## 🧪 Phase-1 ingestion PoC (`sq-2m6zm.2`)

`ingest/ingest_pkg.py` is the **structured-parse** ingestion pipeline over the head docs
(the highest read-frequency × formalisability slice — **not** a full corpus capture):

- `.beads/issues.jsonl` → `pkg:Task` — a **mechanical** projection of the `bd` model (status / type /
  priority / labels / typed edges; `bd blocks` → §2.2 `pkg:dependsOn`, `parent-child` →
  `dcterms:isPartOf`, `research/<doc>.md` → `dcterms:relation` a `pkg:Source`). `bd` stays
  source-of-record; the PKG mirrors it.
- the heaviest `skills/*/SKILL.md` **front-matter** → `pkg:Source` + `pkg:Technique`, plus the
  **write-path** Findings tier (`sq-mztg8.2`, below).

**Write-path (FO-bridge Phase 4, `sq-mztg8.2`):** the `AGENTS.md` Findings tier is **authored**
in a compact, IRI-free `ingest/agents-findings.yaml.ld` (generalising the shipped
`sec-prop.yaml.ld` pattern) and **deterministically compiled** to schema.org-typed PKG Turtle by
`ingest/yamlld_compile.py` — a compiler, not an LLM. Concept TOKENS (e.g. `about: merge-discipline`)
resolve via the **guarded `V()`** against the `concepts:` catalog; an **ambiguous token is a HARD
compile error** (fail-closed — never a silent wrong IRI). Regenerate from the repo root:

```text
python3 crates/sparq-kb/ingest/ingest_pkg.py \
  --beads .beads/issues.jsonl --skills-dir skills \
  --findings-yamlld crates/sparq-kb/ingest/agents-findings.yaml.ld \
  --out crates/sparq-kb/ingest/pkg-instances.ttl
```

**SHACL conformance is the gate**: the ingest **conforms with 0 violations** (`--features
validate`). Stale closed→open `bd` edges are **guardrail-excluded** (to `*.stale-edges.tsv`, not
dropped); `stale_edge_is_caught_by_the_guardrail` proves the §4.4 constraint fires. The write-path's
compiled Findings tier is itself gated for **SHACL conformance + triple-for-triple round-trip** vs
the hand-authored tier (`--test yamlld_write_path`; self-test `scripts/tests/test_yamlld_compile.py`).

## 🔬 bd-bridge eval — can sparq REPLACE bd? (`sq-2m6zm.5`)

The honest answer (`tests/bd_bridge_eval.rs`, over the real backlog) is **BRIDGE, do not
replace** — the read-model meets **0/4** of the §4.5 gate. The **bd→RDF read-model**
(`ingest_pkg.py`) projects every issue to a SHACL-conformant `pkg:Task` and runs three
structural queries `bd`'s flat CLI cannot: transitive blocked-by chains (`pkg:dependsOn+`),
the §4.1 ready-frontier, and the **knowledge↔task JOIN** (which `bd` cannot express at all).

| # | §4.5 replacement criterion | Met? | What `bd` does that the read-model does not |
|---|---|---|---|
| a | conflict-safe mutation ≥ Dolt 3-way merge | ❌ | read-model has no write-path / row merge; `bd`+Dolt give 3-way merge of issue rows (biggest reason to bridge) |
| b | frontier includes live git/gh/nproc state | ❌ | SPARQL is the dependency half only; `push-frontier.sh`'s in-flight / conflict-partition / CPU-cap are live state in no store |
| c | ready-query latency ≈ `bd ready` (offline) | ❌ | `bd ready` is offline, sub-second, no spin-up; sparq pays graph-load per process; nlq/frontier unexposed over CLI/HTTP |
| d | SessionStart / CLI ergonomics ≈ `bd` | ❌ | `bd` is a mature CLI + hooks + autoclose CI; sparq exposes raw SPARQL + VoID only |

**Verdict: BRIDGE.** Keep `bd` as source-of-record and mirror it into the KG — replacing the
write/merge/latency/CLI estate buys nothing (`part2_four_criterion_replacement_gate`).

## ✨ Features

- **Reuse-first ontology** — generalises the vendored `zkp-sparql` `sig-impl:Assertion`
  reified-claim pattern into `pkg:Finding`; reuses PROV-O, SKOS, DCAT, **DQV**
  (`pkg:confidence`/`pkg:assurance` as named `dqv:QualityMeasurement`/`dqv:Metric`; DQV is a W3C
  **Note**, caveat recorded), FaBiO/FRBR/DC, **CiTO**, schema.org, and nanopublications. ~4 bespoke
  terms net-new plus the `pkg:dependsOn`/`pkg:blockedBy` `owl:inverseOf` pair. Full record in
  [`ontology/pkg/PROVENANCE.md`](ontology/pkg/PROVENANCE.md).
- **SHACL guardrails** — `pkg.shapes.ttl` makes a source + a confidence value + an assurance basis +
  non-filler content **mandatory** on every `pkg:Finding`, and enforces a valid status / bounded
  priority / no-stale-edge on every `pkg:Task`.
- **Literature ingestion + tiered dump** (`literature`/`literature-live`, `sq-2489d.5`/`sq-tzars.*`) —
  `[connector]→[extract]→[ground]→[SHACL gate]→[sidecar]` on committed fixtures, ZERO network in CI;
  live adds CORE v3 connector + fail-closed licence routing; `run_tiered` splits per-tier ARTIFACTS.
- **Boundary-reject accounting** (`sq-tx8v9`) — a live extractor's defensive boundary drops likely
  hallucinations BEFORE the pipeline sees them; each drop is a `BoundaryReject { source_doi, reason }`
  carrying only the claimed DOI + a stable reason category (`justification-not-anchored` |
  `unknown-source`), no abstract-derived text (licence-safe for the committed sidecar). These are
  **counted, never silently dropped**: `RunMetrics::set_boundary_rejects` surfaces `boundary_rejects`
  (count), `boundary_reject_reasons` (category→count), and `boundary_rejects_detail` (`(DOI, reason)`)
  so the fetched→extracted honesty gap shows even though `grounding_rate` sees only survivors.
- **Opt-in by construction** — default build is data + constants only; the `validate` feature is the only thing that pulls in `sparq-core` + `sparq-shacl`.
- **No-drift guard** — `vocab.rs` is byte-pinned against `pkg.ttl` by a sync test.
- **Write-path authoring** (`sq-mztg8.2`) — author Findings in `*.yaml.ld`; `yamlld_compile.py`
  expands them to PKG Turtle via the **guarded `V()`** (ambiguous TOKEN = hard error).
- **NL-tool envelope** (`query` feature, `sq-ve5dy`) — `query::nl_tool` returns the **executed
  SPARQL + resolved IRIs + grounding confidence** (`pkg-query --json`) so the caller can verify the
  answer was not fabricated.
- **Citation renderer** (`query` feature, `sq-2489d.11`) — `query::citations::render_citations` renders
  `prov:wasDerivedFrom` sources as `[source]` citations; reports citation-resolution-rate (target 1.0)
  + fabricated-count (target 0, MEASURED by the harness, not assumed).

## 📚 Learn more

- Design records: `research/dogfooding-sparq-knowledge-graph.md` (PR #1063), `research/provenance-driven-genai-kb.md` (§4/§5 literature, `sq-2489d.5`), `research/research-kb-program.md` (`sq-tzars`).
- `ontology/pkg/PROVENANCE.md` — per-term reuse + `skos:closeMatch` alignment; precedent: `crates/sparq-trust/`.
- Epic `sq-2m6zm`: ontology/shapes `.1`; ingestion `.2`; **query-the-PKG** helper + skill `.3`; bd-bridge `.5`; write-path `sq-mztg8.2`.
- `pkg-query --extra-graph <path>` loads extra Turtle alongside the PKG; its triples join `--close owl-rl` — the FO-KM benchmark seam (`sq-mztg8` Metric 1; `bench/fo-km/`).

## License

MIT — see the repository-root [`LICENSE`](../../LICENSE). © 2026 Jesse Wright.
