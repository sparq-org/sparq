# Federated SPARQL — feature research (epic sq-3183) [OPUS-4.8]

Deep-research design record on **federated SPARQL** features sparq should support. It is
grounded FIRST in sparq's current federation surface (so we propose GAPS, not re-propose
what exists), then surveys the literature + vendor landscape, then gives a concrete
candidate-feature table with fit/impact/effort and a prioritised recommendation.

Timings in cited sources are theirs; nothing here is a sparq-measured number.

---

## 0. What sparq does TODAY (the baseline)

Read from the codebase — `crates/sparq-engine/src/service.rs`,
`crates/sparq-engine/tests/service_federation.rs`,
`crates/sparq-server/src/{http.rs,service_config.rs}`, `crates/sparq-introspect/src/lib.rs`,
`crates/sparq-engine/src/cs.rs`, and `skills/{http-server,sparql-query}/SKILL.md`.

**sparq HAS (federation surface):**

| Capability | Where | Notes |
|---|---|---|
| SPARQL 1.1 `SERVICE <iri> { … }` | `sparq-engine/src/service.rs` (`eval_remote`, `parse_srj`) | Whole inner pattern serialised back to SPARQL via spargebra `Display` and POSTed (form-encoded, `Accept: application/sparql-results+json`); SRJ parsed (incl. SPARQL-1.2 triple terms + `its:dir`). |
| `SERVICE SILENT` | same | Any transport/HTTP/parse error swallowed → join identity (one empty solution). |
| SSRF egress allowlist (default-deny private; strict allowlist-only on the server) | `service.rs::{is_forbidden_ip,egress_policy}`, `sparq-server/src/service_config.rs` | DNS-rebinding-safe (vets the **resolved** IP via a ureq `Resolver`); cloud-metadata `169.254.169.254` blocked; `--service-allow` / `*.suffix` wildcards / env / file. This is a genuine differentiator — most engines ship SSRF-open `SERVICE`. |
| Characteristic sets (mining + planner) | `sparq-introspect` (`characteristic_set_ids`), `sparq-engine/src/cs.rs` (`CsTable`, `with_cs_table`, opt-in `cs-planner`) | Neumann & Moerkotte ICDE 2011, mined from sorted SPO scans; today **LOCAL planner only**. |
| VoID generation | `sparq-introspect::Introspection::to_void(dataset_iri)` → N-Triples | Dataset stats, class/property partitions, vocabularies. Today a **library call** — NOT served. |
| Schema/stats introspection | `sparq-introspect` | Per-predicate triples/distinct subjects-objects, class extents, observed domain/range, cross-class join hints. |
| SPARQL 1.1 Protocol endpoint | `sparq-server/src/http.rs` | `/sparql` GET+POST query/update, GSP read+write, content negotiation, `/metrics`, subscriptions (WS+SSE). |
| HDT load | `sparq-hdt` + `hdt-format` skill | A compressed RDF archive sparq can already read. |

**sparq LACKS (the gap surface — this is the research target):**

1. **No bind-join / `VALUES` pushdown into `SERVICE`.** `service.rs` says so explicitly: the
   inner pattern is forwarded verbatim, the remote relation is *materialised whole* and joined
   locally — "we do NOT push surrounding bindings down (no BindingsRestricted / VALUES
   injection)". This is the single biggest perf lever in every federation engine (§3).
2. **No `SERVICE ?var`** (variable endpoint) — rejected with a clear error.
3. **No served Service Description** — `GET /sparql` with no `query` returns no `sd:` capability
   doc; nothing at `/.well-known/void`. sparq can *generate* VoID but does not *expose* it.
4. **No federation CLIENT** — sparq evaluates a single `SERVICE` clause against a single named
   endpoint; there is no multi-source source-selection, no cost-based federation planner, no
   "pass N sources, treat as one virtual dataset" (Comunica's default).
5. **No low-cost publish interface** — no TPF / brTPF / Star-Pattern-Fragments / smart-KG server,
   no link-traversal client.
6. **No federation registry / membership / advertisement** beyond a bare endpoint.

sparq's UNIQUE assets to relate against: the **ZK/MPC privacy layer** (`sparq-zk`, `sparq-mpc`,
`sparq-zk-compose`); **dict-id colocated permutation indexes** (the substrate the characteristic-set
miner already exploits with pure sorted scans); the **GenAI/vector** surface (`sparq-vectors`,
`sparq-introspect`'s schema cards); and **`sparq-solid`** (a Solid client — the natural decentralised
federation member).

---

## 1. Literature + vendor landscape

### 1.1 The three roles

- **Queried node (data source).** Advertise capabilities + statistics so a *remote* optimiser can
  plan against you without expensive probing: **SPARQL 1.1 Service Description**
  [SD‑Rec](https://www.w3.org/TR/sparql11-service-description/) and **VoID**
  [VoID‑Note](https://www.w3.org/TR/void/) (totals, per-predicate/per-class partitions, distinct
  subject/object counts, `void:uriSpace`, discovered at
  [`/.well-known/void`](https://www.w3.org/TR/void/#well-known)). Richer estimators —
  **characteristic sets** ([Neumann & Moerkotte ICDE 2011](https://dblp.org/rec/conf/icde/NeumannM11.html))
  and **skew-aware per-predicate stats** ([CostFed, Semantics 2018](https://www.sciencedirect.com/science/article/pii/S1877050918316211)).
  Execution contract: **SaGe / web-preemption** for complete results under quotas
  ([SaGe, WWW 2019](https://arxiv.org/abs/1902.04790)); result streaming. The empirical case:
  only ~⅓ of public endpoints expose any SD/VoID and availability is poor/bimodal
  ([Buil-Aranda et al., ISWC 2013 "Ready for Action?"](https://link.springer.com/chapter/10.1007/978-3-642-41338-4_18);
  [SPARQLES monitoring](https://journals.sagepub.com/doi/10.3233/SW-170254)).

- **Network member.** Beyond `SERVICE`
  ([SPARQL 1.1 Federated Query](https://www.w3.org/TR/sparql11-federated-query/) — note `SERVICE`,
  `SILENT`, `SERVICE ?var` non-normative, and that **`VALUES` pushdown is an engine optimisation
  the spec does not mandate**; strategy analysis in
  [Buil-Aranda et al., ISWC 2014](https://aic.ai.wu.ac.at/~polleres/publications/buil-etal-2014iswc.pdf)),
  the **Linked Data Fragments cost spectrum**
  ([linkeddatafragments.org](https://linkeddatafragments.org/)): data-dump → **TPF**
  ([Verborgh et al., JWS 2016](https://linkeddatafragments.org/publications/jws2016.pdf)) → **brTPF**
  ([Hartig & Buil-Aranda, ODBASE 2016](http://olafhartig.de/brTPF-ODBASE2016/)) →
  **Star Pattern Fragments** ([Aljbreen/Hose, arXiv 2002.09172](https://arxiv.org/abs/2002.09172)) /
  **smart-KG** (HDT partition shipping,
  [Azzam et al., SWJ 2024](https://www.semantic-web-journal.net/system/files/swj3571.pdf)) →
  full endpoint. **SPARQL-LD** (`SERVICE <any-RDF-URI>`,
  [Fafalios & Tzitzikas](https://ceur-ws.org/Vol-1486/paper_71.pdf)) and
  **link-traversal / LTQP** ([Comunica LTQP](https://comunica.dev/research/link_traversal/);
  [Solid type-index-guided, ISWC 2023](https://comunica.github.io/Article-ISWC2023-SolidQuery/))
  bridge to the open Web. Membership/discovery: SD + VoID + **DCAT 3 `DataService`**
  ([DCAT‑3](https://www.w3.org/TR/vocab-dcat-3/)). Trust: **WebID/Solid-OIDC** + **WAC/ACP**
  ([Solid Protocol](https://solidproject.org/TR/protocol)) and
  **access-control-aware source selection** ([SAFE](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5288952/)).

- **Federation client.** **Comunica**
  ([Taelman et al., ISWC 2018](https://link.springer.com/chapter/10.1007/978-3-030-00668-6_15);
  [resource page](https://comunica.github.io/Article-ISWC2018-Resource/)) — modular **actor /
  bus / mediator** engine; federation is the *default* (pass N sources → one virtual dataset, no
  `SERVICE` needed); per-source-type quad-pattern actors (endpoint / TPF / dump / HDT / link
  traversal); a **Join-Coefficients mediator** scoring `iterations / persistedItems /
  blockingItems / requestTime` to pick a physical join, ordering smallest-cardinality-first
  ([joins docs](https://comunica.dev/docs/modify/advanced/joins/)).

### 1.2 Federation-engine comparison (source selection · join · cost model)

| Engine | Source selection | Join | Cost model | Headline | Source |
|---|---|---|---|---|---|
| **FedX** | index-free per-pattern **SPARQL ASK** (cached) + **exclusive groups** | **bind-join** (block-nested-loop, `VALUES`) | heuristic | minimise remote requests; the baseline | [arXiv 1210.5403](https://arxiv.org/pdf/1210.5403) |
| **ANAPSID** | predicate→endpoint catalog, star decomposition | **adaptive non-blocking** (agjoin/adjoin on XJoin/symmetric hash) | runtime-adaptive | robust to slow/down endpoints | [ISWC 2011](http://iswc2011.semanticweb.org/fileadmin/iswc/Papers/Research_Paper/03/70310017.pdf) |
| **CostFed** | **trie of URI prefixes** + char-sets, join-aware | bind/hash, exclusive groups | **cost-based, non-linear skew** | 3–121× vs prior on FedBench | [Semantics 2018](http://olafhartig.de/files/SaleemEtAl_CostFed_Semantics2018_Preprint.pdf) |
| **SemaGrow** | VoID index + ASK fallback | bind/merge/hash via cost fn | VoID stats | beats SPLENDID | [ACM 2814886](https://dl.acm.org/doi/abs/10.1145/2814864.2814886) |
| **Odyssey** | char-sets / char-pairs | **DP planning** | detailed stats | better plans, fewer intermediates | [arXiv 1705.06135](https://arxiv.org/abs/1705.06135) |
| **HiBISCuS** | **hypergraph + URI-authority** pruning (a layer atop FedX/SPLENDID/DARQ) | host engine's | capability pruning | fewer tp-wise sources | [ESWC 2014](https://2014.eswc-conferences.org/sites/default/files/papers/paper_50.pdf) |
| **SPLENDID** | **VoID** + ASK fallback | bind + parallel hash | VoID stats | reference VoID system | [COLD 2011](https://ceur-ws.org/Vol-782/GoerlitzAndStaab_COLD2011.pdf) |

Cross-cutting (the fine-grained eval,
[Saleem et al., SWJ 2016](https://www.semantic-web-journal.net/system/files/swj625.pdf)): the
dominant metrics are **total triple-pattern-wise sources selected** and **number of remote/ASK
requests** — "the total number of tp-wise selected sources has a direct impact on query execution
time". **ASK-only over-estimates** (selects sources that contribute nothing to a join); join-aware
prefix/char-set selection is the cure. In federation, **network round-trips dominate local CPU**,
so bind-joins + exclusive groups (one request for a whole single-source sub-pattern) are the core
win; non-blocking operators (symmetric/XJoin, ANAPSID) prevent one slow endpoint stalling the query.

### 1.3 Vendor table

| Product | `SERVICE` | Mechanism | FedX inside? | Source |
|---|---|---|---|---|
| **Stardog** | yes + service vars | cost-based federated optimiser + **virtual graphs** (SQL/Mongo/ES pushdown) | no (own) | [docs](https://docs.stardog.com/query-stardog/) |
| **Ontotext GraphDB** | yes | **FedX** (RDF4J SAIL); bound-joins via `VALUES` default-on | **yes — FedX** | [docs](https://graphdb.ontotext.com/documentation/11.2/fedx-federation.html) |
| **Virtuoso** | yes (SPARQL-FED) | per-SERVICE sub-query; capability discovery | no | [docs](https://docs.openlinksw.com/virtuoso/) |
| **Amazon Neptune** | yes (2019) | optimiser places SERVICE; bound-var forwarding; **read-only, must be VPC-reachable, same acct/region** | no | [docs](https://docs.aws.amazon.com/neptune/latest/userguide/sparql-service.html) |
| **QLever** | yes | client-side SERVICE; cross-dataset (OSM+Wikidata) | no (C++) | [repo](https://github.com/ad-freiburg/qlever) |
| **Jena/Fuseki (ARQ)** | yes | plain HTTP SPARQL, **no bind-join** (naive) — the explicit unoptimised baseline | no | [docs](https://jena.apache.org/documentation/query/service.html) |

**Takeaway for sparq:** GraphDB (FedX bound-joins) is the product to beat for client federation;
Jena ARQ is the "naive `SERVICE`" baseline sparq currently *matches* (no pushdown) and should beat.
Stardog is the reference for capability-rich federation; Neptune supplies the operational-constraint
catalogue (reachability/IAM/read-only) — which validates sparq's SSRF-default-deny stance.

---

## 2. CANDIDATE FEATURE TABLE

FIT key: `clear-fit:<component>` = lands in an existing crate with a known seam ·
`new-component-but-fits` = new module/crate but architecturally clear · `ambiguous-ask-user` =
strategic/large, needs a product call. Impact 1–5 (federation value). Effort S/M/L.

### Role 1 — sparq as a QUERIED node (expose metadata)

| # | Feature | Fit | Imp | Eff | Rationale + key source |
|---|---|---|---|---|---|
| A1 | **Serve a SPARQL 1.1 Service Description** at `GET /sparql` (no `query`) → `sd:` graph: `supportedLanguage`, `feature` (UnionDefaultGraph / BasicFederatedQuery), `resultFormat`, named-graph inventory | `clear-fit:sparq-server` (new route/negotiate) | 4 | S | Makes sparq planner-discoverable; only ~⅓ of endpoints do this → remote optimisers fall back to ASK-probing. [SD‑Rec](https://www.w3.org/TR/sparql11-service-description/) |
| A2 | **Serve VoID** at `/.well-known/void` (+ link from SD) — reuse `Introspection::to_void` (already exists, just unwired) | `clear-fit:sparq-server` + `sparq-introspect` | 5 | S | The quantitative numbers cost-based federators consume (`void:triples`, per-pred/per-class partitions, distinct S/O, `void:uriSpace`). The generator already exists — this is pure plumbing. [VoID](https://www.w3.org/TR/void/) |
| A3 | **Expose characteristic-set statistics** in the VoID/stats doc (a VoID extension partition) | `new-component-but-fits` (`sparq-introspect` serialiser + server) | 4 | M | sparq *already mines* char-sets (`cs.rs`/introspect); exposing them lets remote optimisers (CostFed/Odyssey-class) estimate star/multi-join cardinality accurately — a near-unique source-side capability. [Neumann & Moerkotte](https://dblp.org/rec/conf/icde/NeumannM11.html), [CostFed](http://olafhartig.de/files/SaleemEtAl_CostFed_Semantics2018_Preprint.pdf) |
| A4 | **Advertise result-row cap / max query time** in SD (and honour it explicitly) | `clear-fit:sparq-server` | 3 | S | "Ready for Action?" had to *empirically discover* endpoint limits — advertising avoids silent truncation in federation. [Buil-Aranda ISWC 2013](https://link.springer.com/chapter/10.1007/978-3-642-41338-4_18) |
| A5 | **Chunked / streaming SPARQL results** (TSV/JSON, `Transfer-Encoding: chunked`) | `clear-fit:sparq-server` (`results.rs`/`negotiate.rs`) | 3 | M | Lowers time-to-first-byte so a federating client pipelines the join. [SPARQL Results JSON](https://www.w3.org/TR/sparql11-results-json/) |
| A6 | **SaGe-style preemptible execution** + complete-results contract | `ambiguous-ask-user` (executor must save/restore iterator state in bounded space — large) | 4 | L | Strongest source contract (no silent truncation under quota); but a deep executor change. Strategic. [SaGe WWW 2019](https://arxiv.org/abs/1902.04790) |

### Role 2 — sparq as a NETWORK MEMBER (publish interfaces / membership)

| # | Feature | Fit | Imp | Eff | Rationale + key source |
|---|---|---|---|---|---|
| B1 | **TPF server endpoint** (single-triple-pattern fragments + count metadata + hypermedia controls + paging) | `new-component-but-fits` (`sparq-server` route over `sparq-core` index scans) | 4 | M | High-availability, CDN-cacheable, cheap publish interface; the pattern scans map directly onto sparq's permutation indexes. [TPF JWS 2016](https://linkeddatafragments.org/publications/jws2016.pdf) |
| B2 | **brTPF** (binding-restricted requests on the TPF server) | `clear-fit:` (delta on B1) | 4 | S→M | Bind-join at the LDF level — big network-load cut for a small delta over B1; shares the B6/C1 pushdown machinery. [brTPF ODBASE 2016](http://olafhartig.de/brTPF-ODBASE2016/) |
| B3 | **smart-KG-style HDT partition shipping** (star "families" as HDT partitions evaluated client-side) | `new-component-but-fits` (leans on `sparq-hdt` + `hdt-format`/`fused-decompress-parse` skills) | 3 | L | Strongest point on the cost spectrum, and *uniquely cheap for sparq* — it already reads HDT. But the partition-builder is real work. [smart-KG SWJ 2024](https://www.semantic-web-journal.net/system/files/swj3571.pdf) |
| B4 | **SPARQL-LD** (`SERVICE <any dereferenceable RDF URI>`) | `clear-fit:sparq-engine` (`service.rs` transport: dereference + parse instead of POST) | 3 | M | Turns the whole RDF Web into federation members via standard syntax; stepping-stone to LTQP. [SPARQL-LD](https://ceur-ws.org/Vol-1486/paper_71.pdf) |
| B5 | **Link-traversal client (LTQP)** with reachability semantics; type-index-guided for Solid (`sparq-solid`) | `ambiguous-ask-user` (completeness/termination hard; new engine mode) | 4 | L | The decentralised-federation bet — required to query Solid pods (not SPARQL endpoints). Directly leverages `sparq-solid`. Strategic. [Comunica LTQP](https://comunica.dev/research/link_traversal/), [Solid ISWC 2023](https://comunica.github.io/Article-ISWC2023-SolidQuery/) |
| B6 | **DCAT/VoID self-advertisement into a registry** (catalog membership) | `clear-fit:sparq-introspect` + `sparq-server` (extends A1/A2) | 2 | S | How a sparq node is *discovered* in a network. Low cost once SD/VoID exist. [DCAT‑3](https://www.w3.org/TR/vocab-dcat-3/) |
| B7 | **WebID/Solid-OIDC auth on federated calls + access-control-aware source skipping (SAFE-style)** | **SKIPPING HALF LANDED** in `sparq-solid` (opt-in `source-auth`, `sq-lzvl`); the **auth half is still `ambiguous-ask-user`** | 4 | L | The trust boundary for federation, and the natural seam for sparq's ZK/MPC privacy track (attested, authorised per-source access). Strategic. The source-skipping half is now `PodStore::authorize_source` — a **plan-time** WAC/ACP decision that narrows a source to its authorised graphs (`research/mpc-untrusted-planner-routing-design.md` §8 Phase 7 / §9 Q4). It does **not** authenticate: WebID/Solid-OIDC on the federated call is untouched and the session stays caller-asserted, so the trust boundary is not yet closed. [Solid Protocol](https://solidproject.org/TR/protocol), [SAFE](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5288952/) |

### Role 3 — sparq as a federation CLIENT (Comunica-style)

| # | Feature | Fit | Imp | Eff | Rationale + key source |
|---|---|---|---|---|---|
| C1 | **`VALUES` bind-join pushdown into `SERVICE`** | `clear-fit:sparq-engine` (`service.rs`/`exec.rs::eval_service` — the code *names* this as the missing piece) | 5 | M | THE federation perf lever: ship batched bindings as one `VALUES`-injected remote request instead of materialising the whole remote relation. sparq currently does the naive Jena-ARQ thing. [Buil-Aranda ISWC 2014](https://aic.ai.wu.ac.at/~polleres/publications/buil-etal-2014iswc.pdf), [FedX](https://arxiv.org/pdf/1210.5403) |
| C2 | **`SERVICE ?var`** (variable endpoint, per-binding/grouped invocation) | `clear-fit:sparq-engine` (`eval_service` already rejects it with a clear seam) | 3 | M | Enables runtime-chosen endpoints / dynamic membership; spec-described (non-normative). [Fed Query](https://www.w3.org/TR/sparql11-federated-query/) |
| C3 | **Multi-source federation client** — pass N sources, treat as one virtual dataset; source selection (ASK + URI-prefix/char-set pruning, exclusive groups) | `new-component-but-fits` (a federation planner module over `sparq-engine`) | 5 | L | The Comunica/FedX core: this is what makes sparq a *federation engine* not just a `SERVICE` evaluator. Reuse sparq's char-sets for join-aware selection (over-estimation is the #1 cost driver). [Comunica](https://comunica.github.io/Article-ISWC2018-Resource/), [SWJ eval](https://www.semantic-web-journal.net/system/files/swj625.pdf) |
| C4 | **Network-aware join cost model** (a `requestTime`-dominant coefficient; smallest-cardinality-first; bind vs hash choice) | `clear-fit:sparq-engine` (extends the existing planner / `cs.rs`) | 4 | M | sparq already has a cardinality planner + char-set table; add a network coefficient so remote round-trips dominate the choice. [Comunica joins](https://comunica.dev/docs/modify/advanced/joins/) |
| C5 | **Non-blocking adaptive operators** (symmetric/XJoin-style; ANAPSID-style adaptivity to slow/down endpoints) | `new-component-but-fits` (executor) | 3 | L | A slow/unreachable endpoint must not stall the whole query; results stream incrementally. [ANAPSID](http://iswc2011.semanticweb.org/fileadmin/iswc/Papers/Research_Paper/03/70310017.pdf) |
| C6 | **Federation caching** (per-pattern ASK source-selection cache; cached SD/VoID/char-sets per endpoint) | `clear-fit:` (alongside C3) | 3 | S | FedX caches ASK results so recurring patterns don't re-probe — cheap, high-value once C3 exists. [FedX](https://arxiv.org/pdf/1210.5403) |
| C7 | **Heterogeneous source abstraction** (one quad-pattern actor per source type: endpoint / TPF / brTPF / dump / HDT / link-traversal) | `ambiguous-ask-user` (an architectural bet — Comunica's whole design premise) | 4 | L | Makes every source type a swappable handler so federation "just works" and B1–B5 plug in as *clients* too. Big but foundational. [Comunica](https://comunica.github.io/Article-ISWC2018-Resource/) |

### Unique-asset crossovers (sparq-specific, beyond stock federation)

| # | Feature | Fit | Imp | Eff | Rationale |
|---|---|---|---|---|---|
| Z1 | **Privacy-preserving federation** — combine the federation client (C3) with `sparq-mpc`/`sparq-zk` so a multi-source join runs under MPC and/or returns a ZK proof of correct federated evaluation | `ambiguous-ask-user` (the MPC+ZKP epic's federated dimension) | 5 | L | This is sparq's *differentiator* no other engine has: federated SPARQL where sources never reveal raw bindings (MPC) and the client gets an attested-correct answer (ZK). Aligns with the MPC+ZKP project. [SAFE](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5288952/), `mpc-protocols` skill |
| Z2 | **Char-set-driven source selection** reusing the existing dict-id char-set miner for join-aware pruning across endpoints | `clear-fit:` (feeds C3) | 4 | S→M | sparq already computes char-sets for the *local* planner over colocated dict-id indexes — the same table is exactly what CostFed/Odyssey use for *federated* source selection. Asset reuse, not new research. [CostFed](http://olafhartig.de/files/SaleemEtAl_CostFed_Semantics2018_Preprint.pdf) |

---

## 3. Top recommendations (prioritisation)

The fastest path to "sparq is a real federation engine" is the **clear-fit, high-impact** items
first, then the strategic bets:

**Tier 1 — high impact, low/medium effort, clear seam (do first):**
1. **C1 — `VALUES` bind-join pushdown into `SERVICE`** (impact 5/M). The code already *names* this
   as the missing piece; it is the single biggest federation perf win and moves sparq off the naive
   Jena-ARQ baseline onto the FedX/GraphDB par. No new component.
2. **A2 — serve VoID at `/.well-known/void`** (impact 5/S). The generator (`to_void`) already
   exists and is unwired — almost pure plumbing — and makes sparq plannable by *every* external
   federator.
3. **A1 — serve a Service Description** (impact 4/S). Pairs with A2; cheap discoverability/pushdown
   gating.
4. **Z2 + A3 — expose & reuse characteristic sets** (impact 4, S→M). sparq already mines char-sets;
   exposing them (A3) makes it a best-in-class *source*, and reusing them (Z2) primes the future
   client's source selection. Asset leverage, not new science.

**Tier 2 — the federation client (medium/large, the strategic core):**
5. **C3 + C4 + C6 — multi-source federation client** with a network-aware cost model and ASK/
   char-set caching. This is what turns sparq from a `SERVICE` evaluator into a *federation engine*.
   Build C1's pushdown first so the client inherits it.
6. **B1 + B2 — TPF / brTPF server** (impact 4, M then S). A cheap, cacheable publish interface that
   maps directly onto sparq's permutation indexes; brTPF shares C1's binding machinery.

**Tier 3 — strategic / ask-the-user bets:**
7. **Z1 — privacy-preserving federation (MPC/ZK over a federated join).** sparq's genuine
   differentiator and the federated face of the MPC+ZKP epic — but large and product-defining;
   flagged `ambiguous-ask-user`.
8. **B5 / B7 — link-traversal client + heterogeneous-source architecture** (Comunica-class). The
   decentralised-Web / Solid bet via `sparq-solid`; high value but an architectural commitment.

**Honest framing:** sparq today is a *correct but naive* `SERVICE` evaluator (matches Jena ARQ:
no pushdown, single endpoint) with one real differentiator (SSRF-safe egress) and two *latent*
assets it does not yet expose (char-sets, VoID generation). The cheapest, highest-leverage moves
are to (a) push bindings down (C1) and (b) expose what it already computes (A1/A2/A3/Z2) — those
four land sparq at FedX/GraphDB par on the client side and best-in-class on the source side for
low effort, before committing to the large client (Tier 2) or the strategic privacy/LTQP bets.
