# Competitive feature-gap analysis — 2026-07

> 🤖 **SPARQ agent** (product-architect synthesis). Synthesizes a five-way competitor
> research fan-out (RDFox, Stardog, GraphDB, TopBraid EDG, and the enterprise cluster
> Neptune / AllegroGraph / Virtuoso / Neo4j-n10s / MarkLogic) against the sparq
> self-inventory. Output: the feature matrix below, a prioritized gap list, a concrete
> design sketch for **SHACL-driven forms** (the TopBraid-parity workbench feature the
> maintainer explicitly asked for), and the bead map under the competitive-parity epic.
>
> Companion records: `research/rdfox-claims-inventory.md` (verbatim RDFox perf claims),
> `research/comparative-benchmarking-everything.md` (bench program, epic sq-hmd7l),
> `research/gui-design.md` (workbench shell), `research/website-redesign.md`.
>
> Honesty rules applied throughout: sparq's own perf numbers are NOT hard-coded here
> (bench artifacts are the source of truth); competitor numbers are their published
> claims, cited as such; where sparq is BEHIND or unmeasured the matrix says so.

## 0. How to read the matrix

Status values for the **sparq** column:

- **LEAD** — sparq has this and is ahead of the named competitors (feature breadth,
  architecture, or measured perf — measured claims live in bench, not here).
- **PARITY** — sparq has a comparable capability.
- **PARTIAL** — sparq has real machinery here but a specific competitor capability is
  missing or unmeasured; the delta is named.
- **GAP** — sparq does not have it; worth-building verdict given.
- **SKIP** — deliberate non-goal (with reason); not a gap we intend to fill.

Competitor columns: ✓ = has it, ~ = partial/dated, — = lacks it. "Ent." = the
enterprise cluster (Neptune/AllegroGraph/Virtuoso/Neo4j/MarkLogic; noted individually
when only one of them has the feature).

## 1. Competitive feature matrix

### 1.1 Reasoning

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| OWL profile coverage (RL/EL/QL/…) | ~ (RL only) | ✓ (all, query-time) | ~ (rulesets) | ~ (SHACL rules) | — | **LEAD on breadth**: RL forward-chaining + complete EL classifier + QL CQ-gate + Direct-Semantics checker in progress (sq-6tykl/sq-pbz04). RDFox has no EL/QL classification. |
| Materialization w/ incremental insert | ✓ (patented, parallel) | — (rewriting only) | ✓ | — | — | PARITY — sparq-reason incremental maintenance exists; per-dimension perf vs RDFox claims = sq-hmd7l axis. |
| **Incremental DELETION at FBF grade** | ✓ (FBF/DRed/B-F, their crown jewel) | — | ✓ ("smooth delete") | — | — | **PARTIAL** — incremental.rs handles deletes, but deletion-heavy efficiency is unmeasured vs FBF/smooth-delete. Bead: deletion-workload bench + optimization. |
| **Stratified negation + aggregation in recursive rules** | ✓ (their most-bought feature) | ~ (Stride beta) | ~ (.pie: inequality/Cut, no aggregates) | — | — | **GAP** — N3/RIF support doesn't cover stratified `AGGREGATE()`/NAF in recursive rules with incremental maintenance. The single biggest engine gap vs RDFox. Bead (XL). |
| owl:sameAs canonical-representative rewriting | ✓ (disabled under NAF/agg) | — | ✓ (representative nodes) | — | — | PARTIAL — UnionFind sameAs inside the OWL-RL fixpoint exists (sq-6w7x6); store-level representative rewriting absent. **Defer** (even RDFox can't compose it with NAF/agg). |
| Explanation / proof trees (engine) | ✓ | ✓ | ~ | ~ (inferences panel) | — | PARITY — `explain` feature, `why(triple)` proof trees. GUI wiring is the gap (see 1.6). |
| Query-time rewriting mode + multiple named schemas | — | ✓ (Blackout/Stride) | — | — | — | PARTIAL — QL is rewriting-shaped; a first-class "reason under schema X per query" toggle is absent. Defer; note for the reasoner program. |
| SWRL ingestion | ✓ | ✓ (rules) | — | — | — | GAP (small) — migration-story parser only. Defer, noted not beaded. |
| **Reasoning profiler (per-rule cost)** | ✓ | ~ | — | — | — | **GAP (small)** — what makes a rule engine debuggable; sparq-introspect + slow-query ring are seeds. Bead (S/M). |
| SHACL-AF rules + sh:values node expressions | — | — (admit no SHACL-AF) | — | ✓ (incl. backward-chained sh:values) | — | **LEAD among engines** — full node-expression algebra + sh:values + gated W3C SHACL-AF harness landed (sq-1m0n, sq-d1dw). TopBraid-only elsewhere. Surface it in forms (computed fields). |

### 1.2 Engine / query capabilities

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| SPARQL 1.1 conformance | ~ (federation only 2025-12) | ✓ | ✓ | ✓ | ~ | **LEAD** — 1229/0 W3C ratchet + 1.2 triple terms; RDFox's SPARQL surface trails its reasoner (publicize honestly). |
| RDF-star / RDF 1.2 | ~ | ~ (docs: "known perf problems, not recommended") | ✓ (mature) | ~ | ~ (AG render) | **LEAD-candidate** — native triple terms in engine + parsers; Stardog is publicly weak here → bench wedge (sq-hmd7l). Reification→star migration tool = nice-to-have, not beaded. |
| **PATHS full-path queries** (endpoints + intermediate nodes, VIA patterns) | — | ✓ (`PATHS SHORTEST/ALL`) | ~ (variant) | — | ~ (Gruff UI) | **GAP** — W3C property paths return endpoints only. De-facto-standard extension, pure engine work on home turf. Bead (L). |
| **Standing queries w/ incremental answer deltas** ("delta queries") | ✓ (non-experimental 7.3+) | — | — | — | — | **PARTIAL** — /subscriptions WS+SSE stream result deltas today; the RDFox differentiator is true incremental view maintenance (no re-eval) + retrievable change history. Bead (L/XL). |
| **Anytime queries** (partial results under budget + resumption) | — | — | — | — | ✓ (Virtuoso; keeps DBpedia alive) | **GAP** — budgets/timeouts exist but return errors, not partial results + continuation token. Bead (M); matters the day anyone hosts a public sparq endpoint. |
| **In-query graph analytics** (PageRank etc. composable in queries) | — | — (Spark offload) | — | — | ✓ (Neptune Analytics CALL) | **PARTIAL** — sparq-algos has the algorithms; the SPARQL-callable surface is designed and blocked on sign-off (**sq-mg5hk, P3 → bumped**). Unblocking an existing bead beats new work. |
| **Facet-count fast path** (grouped class/predicate/value distributions) | — | — | — | — | ✓ (Virtuoso, at B-triple scale) | **GAP** — engine-side aggregate fast path + API; columnar layout is well-suited. Feeds the faceted-browse UI. Bead (M). |
| **Point-in-time query (arbitrary timestamp)** | — | ~ (retired their versioning) | ✓ (DSPOCI `FROM <at/ts>`) | ~ (audit) | ~ (MarkLogic bitemporal) | **PARTIAL** — `time-travel` feature pins queries to *recent generations* (ring); CDC stream + PITR backup deltas exist. Gap = persistent temporal index for arbitrary timestamps. Bead (L). |
| ACID + MVCC multi-user serving | ✓ | ✓ | ✓ | ✓ | ✓ | PARTIAL — update-in-place + COW snapshots + CDC exist; the held serving PRs (#941 ArcSwap, #904 cache, #903 replanner, sq-0g6g) are the path. Already maintainer-flagged; no new bead. |
| HA / replication | ~ (shared-NFS+UDP, modest) | ✓ (cluster) | ✓ (Raft) | — | ✓ | SKIP for now (single-node dominance first; CDC stream is the future seed). RDFox's own HA is architecturally modest — no moat lost. |
| Persistence / snapshot-restore / encryption at rest | ✓ | ✓ | ✓ | — | ✓ | PARITY-PARTIAL — SPQCPRM2 on-disk format, mmap, online backup + delta PITR (sq-o5bi/sq-bu1a); at-rest encryption researched (research/crypto-erase-at-rest.md), not shipped. Checkbox, defer. |

### 1.3 Integration / interop

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| SPARQL SERVICE federation | ~ (only since 7.5) | ✓ | ✓ | — | ~ | **LEAD** — full streaming federation client: SD discovery, TPF/brTPF, pushdown, bind-vs-hash planning, adaptive re-planning, fail-closed egress (sq-dnko closed). |
| **MCP server** | — | — | ✓ (11.x) | ✓ (2026) | — | PARITY-PLUS — sparq-mcp shipped (sq-0z43i). Extension bead: shape-aware editing tools (FormDescription), FTS/vector/facet tools, stored templates. |
| **Tabular→RDF ingest (CSV/R2RML materializing import)** | ✓ (CSV data source) | ✓ (Designer mapping) | ✓ (Ontotext Refine) | ~ | ✓ (Neo4j Importer) | **GAP** — the #1 enterprise adoption funnel ("my data is in CSV/Postgres"). CLI-first slice: CSV direct mapping + R2RML materializing import (sparq-arrow seed). Bead (M). |
| Virtual graphs / SQL pushdown (query external DBs in place) | ✓ (tuple tables incl. rules) | ✓✓ (their #1 differentiator) | ✓ (Ontop) | — | ✓ (Virtuoso) | **GAP — deliberately scoped**: full per-dialect virtualization is a multi-year platform play. sparq's counter-story: materializing import + reload speed. Only if demanded: one-dialect SERVICE-to-SQL bridge later. Not beaded. |
| **BI/SQL wire-protocol facade** (Tableau/Power BI connect directly) | — | ✓ (MySQL protocol) | — | — | — | **GAP (big rock, XL)** — almost no OSS RDF store does this; Rust has mature pgwire crates. Converts every BI tool into a client. Beaded P3 pending maintainer appetite. |
| GraphQL over RDF/shapes | — | ✓ | ✓ (from OWL/SHACL) | ✓ | — | **GAP (defer)** — bounded opt-in crate (shapes→schema, read-only) once forms land; low evidence of demand vs SPARQL/BI. Bead P3. |
| Kafka/CDC change feed out | — | — | ✓ (Kafka) | — | — | PARITY on the substance — durable replayable CDC stream landed (#906); Kafka packaging = niche, skip. |
| **Notebook UX** (%%magic + hosted notebooks) | — | — | — | — | ✓ (Neptune graph-notebook) | **GAP (small)** — `%%sparq` magic in sparq-py + JupyterLite/WASM zero-install demo; note graph-notebook already speaks generic SPARQL so server compat may be nearly free. Bead (S/M). |
| Third-party explorer compat (aws/graph-explorer etc.) | — | — | — | — | ✓ | **GAP (trivial)** — verify + document sparq-server as a graph-explorer SPARQL target = free viz story. Folded into the notebook bead. |
| JSON-LD 1.1 full pipeline | — | ~ | ~ | ~ | ~ | **LEAD** — native dependency-free expansion/flattening/compaction/framing default-on (sq-oy1f). |
| HDT read+write | — | — | — | — | — | **LEAD** — write support is rare anywhere. |
| Arrow columnar results | — | — | — | — | — | **LEAD** — sparq-arrow (#910). |
| Python/JS bindings | ✓ | ✓ | ✓ (11.2) | ✓ (2026 SDK) | ✓ | PARITY — sparq-py + @sparq-org/sparq WASM; RDF/JS conformance program open (sq-iwhl8/sq-xqchl). |

### 1.4 ML / vector / NLQ

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| Vector index architecture | ~ (SEMSIM experimental) | ~ (dated spa: layer) | ~ (external ES/OpenSearch sync) | — | ✓ (Neptune/AG native) | **LEAD architecturally** — native in-engine HNSW + DiskANN + quantization, and the only in-BROWSER graph+vector story (wasm). HONEST caveat: raw ANN perf vs hnswlib/FAISS is being closed under sq-lhcot (matched-recall Pareto is the gate). No new bead — sq-lhcot owns it. |
| NL→SPARQL grounded loop | — | ✓ (Voicebox, 10-agent SaaS) | ✓ (TTYG, explain-response) | ✓ (Copilot) | ✓ (AG ChatStream) | PARITY-PLUS on the core loop — sparq-nlq ground→generate→validate→execute→repair + measured cheap-model NL tool (PKG verdict). GUI wiring = existing bead sq-96o1. Skip the agent-platform zoo. |
| LLM-in-SPARQL magic predicates | — | — | ✓ (gpt:ask) | — | ✓ (AG llm:) | SKIP — nondeterminism inside eval muddies semantics/caching; MCP (LLM calls the DB) is the right direction and sparq has it. |
| GNN pipelines | — | — | — | — | ✓ (Neptune ML) | SKIP — interop instead: .spqv embedding import/export already exists. |
| FTS in SPARQL | ~ (external Lucene mount) | ✓ | ✓ (connectors) | — | ✓ | PARITY-PLUS — native BM25 `text:` magic predicates, in-process, wasm build; bench axis vs jena-text in flight (sq-hmd7l). |

### 1.5 SHACL & validation

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| SHACL Core validation | ✓ (tuple table) | ✓ (ICV) | ✓ (commit-time) | ✓ | ~ (n10s procs) | PARITY-PLUS — full Core + SHACL-SPARQL + custom components + W3C 1.2 suite (sq-waf9o closed) + SHACL-AF (nobody else). SHACL-CS parser program = sq-tonhr. |
| **Validation callable in-query** (violations as rows) + asserted-vs-inferred domain | ✓ (rdfox:SHACL) | ✓ (icv:validate SERVICE) | — | — | — | **GAP (small deltas on existing crates)** — expose validate as function/SERVICE returning rows; fact-domain switch once reasoner closure is queryable. Bead (M). |
| **Guard mode** (transactions rejected on violation) | — | ✓ (icv.enabled) | ✓ (ShaclSail) | ✓ (workflow) | — | **GAP** — commit-time enforcement in sparq-server; folded into the same bead. Also feeds forms commit. |
| **SHACL-driven FORMS (authoring UX)** | — | — | — | ✓✓ (their entire product) | — | **GAP = THE white space.** No engine vendor contests it — RDFox/Stardog/GraphDB/Neptune all stop at validation; editing is raw triples everywhere. Full design §3. Sub-epic. |
| DASH suggestions (one-click constraint repair) | — | — | — | ✓ | — | **GAP** — parameterized SPARQL-UPDATE repair actions; small once forms exist; agent-friendly. Bead (M). |

### 1.6 Workbench UI (GUI = Tauri workbench; site = static export)

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| SPARQL editor + results + saved state | ✓ | ✓ (Studio) | ✓ (YASGUI) | ✓ | ✓ | PARITY — editor w/ highlighting/completion, multi-view results, workspaces, palette (sq-ixc3 shipped core). |
| **Visual query-plan explorer** | ~ (plan access) | ✓ (Studio's standout) | — | — | — | **GAP, cheap** — engine already emits typed plan tree + per-operator q-error + EXPLAIN ANALYZE (#902); GUI panel = rendering work. Turns the perf mandate into a demo. Bead (M) + existing sq-jbqh4 (schema alignment). |
| **Click-to-explain inferred facts (proof-tree UI)** | ✓✓ (their killer UI) | ✓ (explanations) | — | ~ | — | **GAP, cheap** — engine why() exists; wire into the Inference tool + results/graph views. No OSS engine offers this. Bead (M). |
| Dataset summary views (class bubble/chord, domain-range) | — | — | ✓ | — | — | **GAP, cheap** — 2-3 aggregate queries + sparq-introspect schema card/VoID already computed. Bead (S/M). |
| Graph viz of results + expansion | ✓ (CONSTRUCT Explorer) | ✓ | ✓ | ~ | ✓ (Gruff best-in-class) | PARITY on basics (sq-lyp8 shipped). Gap slices: **SPARQL-configurable expansion lenses** (GraphDB's genuinely good idea) + **RDF-star annotation rendering on edges** (sparq leads on engine RDF-star; almost no viz shows it). Bead (M). |
| **Faceted browse** (search→expand→filter w/ counts) | — | ~ (Explorer filters) | — (connector-only) | ~ | ✓ (Virtuoso at scale; graph-explorer) | **GAP** — UI + the engine facet API (1.2). Includes **auto map view** on geo literals (Stardog Explorer's loved detail; GeoSPARQL already in). Bead (M/L, dep on facet API). |
| **Visual query builder** (diagram→SPARQL) | — | ✓ (model-driven) | — | — | ✓ (Gruff) | **GAP** — emit honest SPARQL into the editor; shape-aware suggestions once forms land. Bead (L). |
| **Autocomplete index + rank-ordered defaults** | — | ✓ | ✓ (dedicated index + RDF Rank) | ✓ | ~ | **GAP** — editor completion exists, but a dedicated IRI/label autocomplete index that stays snappy at 100M+ triples, used workbench-wide + PageRank-ordered defaults (sparq-algos has PageRank). Bead (M, engine+GUI). |
| Instance editing / authoring | — (raw SPARQL) | — (read-only Explorer) | ~ (raw triple table) | ✓✓ (forms) | ~ (Bloom inline) | **GAP = forms program** (§3). |
| Ontology/shape editor (meta-circular) | — | ✓ (Designer, models only) | — | ✓✓ | — | GAP — follow-on AFTER forms: the forms engine pointed at shape resources IS the shape editor (TopBraid's move). Not beaded yet; noted as forms phase 2. |
| Server-side parameterized query/update templates | — | ~ (stored queries) | ✓ (smart updates) | — | — | **GAP (small)** — prepared/parameterized queries exist in-engine (#901); add named server-side templates via REST + MCP + GUI. Safe update surface for apps and agents. Bead (S/M). |
| NL chat in workbench | — | ✓ | ✓ | ✓ | — | Existing bead sq-96o1 (no new bead). |
| Admin/monitoring/kill-query | ✓ | ✓ | ✓ | ✓ | ✓ | PARITY-PARTIAL — health panel + Prometheus + subscriptions shipped; kill-query UI not — small, folded into plan-explorer bead acceptance. |

### 1.7 Governance / trust / deployment

| Feature | RDFox | Stardog | GraphDB | TopBraid | Ent. | sparq |
|---|---|---|---|---|---|---|
| Access control expressiveness | ✓ (element-level) | ✓ | ✓ | ✓ (roles) | ✓✓ (AG triple ABAC) | **LEAD on language** — WAC + ACP + ODRL fail-closed evaluation (nobody else has ODRL); AG-style per-quad attribute enforcement parked under trust program. **GAP on plumbing**: OIDC/JWT/API-keys/RBAC = already-flagged open #907 (no new bead). |
| ZK / MPC / verifiable queries / canonicalization / VC | — | — | — | — | — | **UNIQUE LEAD** (research-grade, honestly labelled unaudited — sq-qhy4 gate). No competitor plays here at all. |
| Provenance / lineage | ✓ (proof trees) | ~ (catalog) | ✓ (plugin) | ✓ (lineage models) | ~ | PARITY-PLUS — PROV-O lineage for derived graphs + attestation estate. |
| Versioning / audit | — | ~ (retired) | ✓ (history plugin) | ✓ (full audit) | ~ | PARTIAL — see point-in-time row (1.2). |
| Edge / embedded / browser | ✓ (S25 on-device) | — | — | — | — | **LEAD** — WASM in-page with reasoning/SHACL/FTS/RSP included runs where RDFox cannot; Rust FFI covers on-device. Keep wasm builds first-class; mobile = sq-v286.9. |

## 2. Where sparq already LEADS (do not re-build)

1. **Performance program + honest bench machinery** — the mandate and sq-hmd7l own this.
2. **Browser/WASM deployment with capabilities included** — unique; answers the Samsung-PDE narrative.
3. **Federation client** — capability-aware pushdown/adaptive replanning beats RDFox (federation only Dec-2025) and matches Stardog's SERVICE story.
4. **ZK/MPC/trust/canon/VC estate** — uncontested; unaudited status honestly labelled.
5. **SHACL breadth incl. SHACL-AF + sh:values** — ahead of every engine vendor; TopBraid is the only peer, and it's not an engine.
6. **Governance languages (WAC/ACP/ODRL)** — uncontested among engines.
7. **Native vectors + FTS + MCP + Arrow + JSON-LD + HDT-write** — architectural leads; vector raw-perf closure is owned by sq-lhcot.
8. **Opt-in architecture** — every feature proposed below is an opt-in crate/cargo feature or a GUI panel; sparq-core/engine stay lean.

## 3. SHACL-driven forms — design sketch (the TopBraid-parity feature)

**Thesis.** Every engine competitor stops at SHACL *validation*; data editing is raw
triples everywhere (GraphDB "View resource" table, Stardog read-only Explorer, RDFox
raw SPARQL). TopBraid EDG owns "model once → forms + validation + API for free" but is
not an engine. A sparq workbench that renders **DASH-compatible auto-generated forms**
out-flanks every engine simultaneously and neutralizes TopBraid's moat by implementing
the *open* spec (datashapes.org/forms + DASH vocabulary) rather than inventing one —
existing EDG/shaperone/shacl-form shape libraries render unchanged.

**Architecture: headless core + two renderers + an agent surface.**

```text
crates/sparq-forms  (opt-in crate, no GUI deps, wasm-able)
  (data graph, shapes graph, focus node, mode: view|edit, role) ->
  FormDescription (serde JSON):
    groups[sh:group→sh:PropertyGroup, ordered by sh:order]
      fields[per sh:PropertyShape]
        { path | inversePath, label(sh:name→rdfs:label), description,
          widget IRI (DASH), values[term refs incl. RDF 1.2 triple terms],
          constraints {minCount,maxCount,datatype,class,nodeKind,in,pattern,
                       min/maxLength,min/maxInclusive,uniqueLang,or-unions},
          editability (view|edit|computed-readonly), suggestions[] }
```

1. **Shape selection.** Applicable node shapes for a focus node: `sh:targetClass` vs
   `rdf:type` (+ subclass closure when reasoning is on), `dash:applicableToClass`,
   explicit shape choice; multiple applicable shapes → view-switcher in the renderer.
2. **Field enumeration + layout.** Per property shape: `sh:path` (IRI, and
   `sh:inversePath` = incoming-references fields), `sh:name`/`sh:description`,
   `sh:order` (decimal, fractional insertion), `sh:group` → `sh:PropertyGroup`
   (collapsible sections), implicit "Other properties" group, `sh:deactivated`
   suppression, off-shape triples shown read-only ("also display undeclared").
3. **Widget resolution — the DASH scoring registry.** Explicit `dash:editor` /
   `dash:viewer` wins; otherwise registered widgets score themselves 0–100 against
   (property shape, value) per the documented DASH scores (TextField 10 for literals,
   TextFieldWithLang 11 for `rdf:langString`, TextArea 20 when `dash:singleLine false`,
   BooleanSelect 10, Date/DateTimePicker 10, EnumSelect 10 on `sh:in`, AutoComplete for
   IRIs with `sh:class` (candidate query = instances of class, backed by the
   autocomplete index bead), URIEditor 10 on `sh:nodeKind sh:IRI` sans class,
   RichText on `rdf:HTML`, SubClassEditor w/ `dash:rootClass`, **DetailsEditor =
   recursive nested sub-form via `sh:node`**, BlankNode fallback; viewers mirror:
   Label/Image/Hyperlink/HTML/ValueTable). Ties → user-visible widget switcher.
   Single vs Multi widget types own add/remove affordances (`sh:maxCount 1` removes
   "add"; `sh:minCount ≥1` marks required).
4. **Validation-in-form.** The same constraints that chose the widget validate live —
   field-level sparq-shacl validation on edit (native via Tauri command, wasm on web),
   violations render inline at the field. **DASH suggestions**: violations carrying
   `dash:SPARQLUpdateSuggestionGenerator` render as one-click repairs (parameterized
   SPARQL UPDATE, previewed before apply).
5. **Edit/commit.** Form edits accumulate as a term-level diff → one SPARQL UPDATE
   (DELETE/INSERT WHERE); optional **guard mode** = validate-before-commit
   (transaction rejected on violation — pairs with the engine guard-mode bead);
   optional **working-copy** mode = edits into a draft named graph + diff + apply
   (the useful primitive under TopBraid's workflow apparatus, cheap on a quad-store).
6. **RDF 1.2 awareness.** Triple-term values render as annotation sub-fields (the
   viz/editing gap every competitor has); `rdf:langString` + base-direction handled in
   the lang widgets; computed fields = SHACL-AF `sh:values` node expressions (already
   in sparq-shacl) evaluated on demand and rendered read-only next to asserted data.
7. **Agent surface.** The same FormDescription JSON is exposed as sparq-mcp tools
   (`describe_form`, `apply_form_edit`) — a shape-aware, validated editing API for
   agents. TopBraid bolted MCP on in 2026; sparq ships forms agent-native.
8. **Phase 2 (not beaded yet): meta-circularity.** Point the forms engine at shape
   resources themselves (property-shape shapes) → the shape/ontology editor falls out,
   TopBraid-style, without building a second editor.

**Sequencing:** F1 headless crate → F2 GUI renderer (needs F1) → F3 validation +
suggestions, F4 edit/commit (need F2), F5 RDF-1.2/computed fields (needs F1) →
F6 MCP tools (needs F1+F4). F2 benefits from (not blocked by) the autocomplete-index
bead. All opt-in; zero sparq-core impact.

## 4. Prioritized gap list (value / effort)

**Quick wins (S/M, high value):**
1. GUI visual query-plan explorer (engine EXPLAIN ANALYZE already structured) — perf-mandate demo.
2. GUI click-to-explain proof trees (engine why() exists) — matches RDFox's most demo-able feature; no OSS engine has it.
3. Dataset summary views (bubble/chord/domain-range) — introspect data already computed.
4. Reasoning profiler (per-rule cost) — debuggability table-stakes for the reasoner program.
5. Server-side parameterized templates + MCP editing/FTS/facet tools — the agent-era safe surface.
6. Notebook %%sparq magic + graph-explorer/graph-notebook compat docs + JupyterLite demo.
7. Bump sq-mg5hk (SPARQL-callable analytics — designed, was invisible at P3).

**The strategic play (M/L each, high value):** the SHACL-forms sub-epic (§3) — six
beads; uncontested white space vs all engine competitors.

**Bigger rocks (L/XL, sequenced):**
- Stratified NAF + aggregation in recursive rules w/ incremental maintenance (XL) — the RDFox capability enterprises buy; extends sq-6tykl.
- Deletion-heavy incremental maintenance bench + FBF-grade retraction (M/L) — extends sq-6tykl + sq-hmd7l.
- PATHS full-path queries (L); anytime partial results (M); facet-count fast path (M) + faceted browse UI (M/L); standing-query IVM delta queries (L/XL); point-in-time temporal index (L); CSV/R2RML materializing import (M); autocomplete index + rank ordering (M); visual query builder (L).
- Maintainer-appetite gates: BI/SQL wire facade (XL, P3), GraphQL-from-shapes (L, P3 defer).

**Deliberate skips (recorded so nobody re-litigates):** Spark-offload analytics, GNN
pipelines, LLM-in-SPARQL magic predicates, full virtualization platform, HA clustering
(for now), sameAs store-rewriting (for now), SWRL (parser-level, when migration demand
appears), Sponger-style middleware, catalog/lineage governance products, Kafka
packaging, connector zoos.

## 5. Epic + bead map (cross-referenced to EXISTING epics)

New epic (this program): **sq-lsp7k — competitive feature parity + workbench
extension** — child beads carry `{track, competitor_parity, crate_or_surface, effort,
value, acceptance}`. Beads that belong to existing programs are parented THERE, not
duplicated:

| Destination | Beads (created 2026-07-11) |
|---|---|
| **sq-ixc3** (GUI epic) | plan explorer sq-ixc3.19 (P1), click-to-explain sq-ixc3.20 (P1), summary views sq-ixc3.21, viz lenses + RDF-star edges sq-ixc3.22, faceted browse UI sq-ixc3.23 (dep sq-lsp7k.5), visual query builder sq-ixc3.24 |
| **sq-6tykl** (reasoner program) | stratified NAF+aggregation sq-6tykl.3 (P1, XL, design-record-first), deletion-grade incremental sq-6tykl.4 (P1), reasoning profiler sq-6tykl.5 |
| **sq-hmd7l** (bench-everything) | deletion-workload axis rides sq-6tykl.4 acceptance; RDF-star-vs-Stardog-edge-properties wedge + facet/PATHS axes register after landing |
| **sq-vw3ax** (site) | honest competitive-matrix page sq-vw3ax.16 (P3; no fabricated numbers; links to bench artifacts) |
| **sq-lhcot / sq-89fyr** (vectors/GenAI) | own vector-perf closure + NLQ growth — nothing new added here |
| **sq-tonhr** (SHACL-CS) | unchanged; forms consume sparq-shacl, not the parser program |
| **sq-96o1 / sq-jbqh4 / sq-mg5hk** (existing beads) | NL-in-GUI, EXPLAIN-schema alignment (folded into sq-ixc3.19), analytics surface (**sq-mg5hk bumped P3→P2**) — referenced/bumped, not duplicated |
| **sq-lsp7k** (new epic) | SHACL-forms sub-epic **sq-lsp7k.1** (F1 .1.1 P1 / F2 .1.2 P1 / F3 .1.3 / F4 .1.4 / F5 .1.5 / F6 .1.6), validate-in-query + guard mode sq-lsp7k.2, PATHS sq-lsp7k.3, anytime sq-lsp7k.4, facet API sq-lsp7k.5, standing-query IVM sq-lsp7k.6 (XL), temporal index sq-lsp7k.7 (P3), CSV/R2RML import sq-lsp7k.8, autocomplete index sq-lsp7k.9, templates+MCP tools sq-lsp7k.10, notebook compat sq-lsp7k.11, BI facade sq-lsp7k.12 (P3, XL, needs:maintainer), GraphQL sq-lsp7k.13 (P3, defer) |

Same-crate collision notes for wave scheduling: the two sq-6tykl reasoner beads touch
sparq-reason (don't run in parallel); GUI beads are panel-scoped but share gui/app;
anytime + PATHS both touch sparq-engine; temporal index + standing-query IVM both
touch sparq-serve/server.

## 6. Open questions for the maintainer

1. **Forms renderer platform:** the GUI is Tauri (native validation available); the
   hosted-web build must run sparq-shacl via wasm. Proposal: FormDescription is
   computed in sparq-forms (wasm-able) so BOTH targets share it — confirm the hosted
   /app surface should get forms at parity, or desktop-first?
2. **Site vs GUI boundary:** does a public competitive-matrix page belong on the
   static site now (sq-vw3ax child, honest, no numbers until canonical benches land),
   or hold until sq-vw3ax.12 baselines exist?
3. **BI/SQL wire facade (XL):** real appetite? Postgres-wire vs Stardog's MySQL choice;
   parked at P3 pending a verdict.
4. **Datalog dialect surface:** stratified NAF+aggregation — expose via N3 builtins,
   a RIF extension, or a small native rule syntax? (XL bead starts with a design
   record; steer there.)
5. **Which competitor to chase FIRST in marketing terms:** RDFox (perf+reasoning) is
   the standing mandate; this analysis says the *workbench* fight (TopBraid white
   space) is the cheapest uncontested territory — confirm the forms sub-epic's P1.
6. **Standing-query IVM:** acceptable to evolve /subscriptions semantics (explicit
   additions/deletions delta records + retrievable change history) or must the
   existing wire contract stay frozen?
