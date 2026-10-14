<!-- [FABLE-5] sq-hmd7l.22 — cross-engine CONFORMANCE pass-rate record: sparq's
scoreboard floors (origin/main) vs the peers' PUBLISHED EARL / implementation-report
results. Peer cells only from pinned published sources with provenance; a peer
without a published number is an explicit NOT-PUBLISHED cell, never a sparq win. -->

# Gap record — cross-engine conformance pass-rates (2026-07)

**Axis:** #22 in `research/comparative-benchmarking-everything.md` §6 (epic `sq-hmd7l`, bead `sq-hmd7l.22`).
**Status:** peer cells retrieved 2026-07-10 with provenance; verdicts below.

**Headline (honest):** the dominant finding is that **peers rarely publish per-suite
pass COUNTS the way sparq's scoreboard does.** The one numeric multi-engine SPARQL
dataset (the W3C 2013 implementation report) predates every modern engine of interest
and a decade of suite churn; today's engines (Oxigraph, QLever, Comunica) publish a
BINARY "compliant"/known-failure-list posture, not a count. So most SPARQL/RDF-syntax
rows are **NOT-COMPARABLE** — an honest verdict, not a sparq win. The families where a
genuine, same-suite comparison exists are **SolidLab ODRL** (sparq matches the
reference evaluator's own 67/68 EXACTLY) and, with the requirements-weighted and
re-implemented-comparator caveats, **OGC GeoSPARQL** (a published third-party
benchmark). JSON-LD and SHACL have a rich published EARL, but at a slightly different
suite revision than sparq's pin.

## 1. Method and honesty rules

- **sparq's numbers are the ACTUAL CI-enforced scoreboard floors** on `origin/main`
  (commit `6df375096`): the central registry `crates/sparq-conformance/src/scoreboard.rs`
  (`SUITES`), the lib-side JSON-LD floor consts
  (`crates/sparq-conformance/src/floors/*.rs`), and the crate-local floor consts the
  guard test `tests/scoreboard_floors.rs` pins to the registry. Reproduce with
  `cargo run --release -p sparq-conformance --bin sparq-conformance-scoreboard`.
- **Peer numbers come ONLY from their published EARL / implementation reports** (or,
  where flagged, a published third-party benchmark) — cited with URL + retrieval
  date. Nothing is estimated, and **a peer without a published number is an explicit
  NOT-PUBLISHED cell, never a sparq win by omission.**
- **Suite-version mismatches are flagged per row.** A pass-rate is only strictly
  comparable at the same suite revision; most published EARL predates the current
  consolidated `w3c/rdf-tests` tree.
- **Verdicts use the fixed vocabulary** from
  `research/comparative-benchmarking-everything.md` §5.3: CLEARLY-AHEAD /
  AHEAD-BUT-NOT-OOM / PARITY / BEHIND / NOT-MEASURED / NOT-COMPARABLE.
- Conformance pass-rates are **correctness counts, not performance figures**; no
  latency/throughput numbers appear in this record.

## 2. sparq's scoreboard (verified on origin/main)

Sources: `scoreboard::SUITES` registry rows + the runner-local floor consts named
below; suite pins from `scripts/fetch-conformance.sh` (`w3c/rdf-tests`
`f25dbc092c65`), `scripts/fetch-jsonld-tests.sh` (`w3c/json-ld-api` `8654ac22b6cf`),
`scripts/fetch-jsonld-framing-tests.sh` (`w3c/json-ld-framing`
`3bf782ba9a40dd1b143435abe386d38df64f2b47`),
`crates/sparq-shacl/fetch-shacl-tests.sh` (`w3c/data-shapes` `b6e73695d619`),
`scripts/fetch-inference-suites.sh` (`w3c/N3` `23ccf3d56b25`).

| Suite | sparq floor (CI-enforced) | Denominator at the pin | Source const |
|---|---|---|---|
| W3C SPARQL 1.0/1.1/1.2 query+update+syntax | 1229 (1225 pass + 4 documented divergences, 0 fail, 0 skip) | 1229/1229 of the full harness scope | `ci.yml` `RATCHET=1229`; `FINDINGS.md` round 5 |
| Inference (rdf-mt / OWL 2 RL / N3 / sparql11-entailment / rdf-turtle) | 1967 (1949 pass + 18 documented divergences, 0 fail) | 100% of run scope: rdf-mt 48/48, OWL 2 RL 78+13 div, N3 1463+5 div, sparql11/entailment 47, rdf-turtle 313 (out-of-scope buckets excluded + reported) | `ci.yml` `RATCHET=1967` (2026-06 snapshot in the job header) |
| W3C SHACL core | 98 pass | 98/98 `sht:Validate` cases at the pin | `w3c_core.rs` `BASELINE_PASS = 98` |
| W3C SHACL-SPARQL (node+property) | 5 pass | 5/5 in the node+property sub-suites | `w3c_sparql.rs` `SHACL_SPARQL_FLOOR = 5` |
| W3C JSON-LD 1.1 toRdf | 413 pass | of 467 manifest entries | `floors/to_rdf.rs` `FLOOR = 413` |
| W3C JSON-LD 1.1 fromRdf | 51 pass | of 53 | `floors/from_rdf.rs` `FLOOR = 51` |
| W3C JSON-LD 1.1 expand | 276 pass | of 385 | `floors/expand.rs` `FLOOR = 276` |
| W3C JSON-LD 1.1 compact | 186 pass | of 246 | `floors/compact.rs` `FLOOR = 186` |
| W3C JSON-LD 1.1 flatten | 53 pass | of 58 | `floors/flatten.rs` `FLOOR = 53` |
| W3C JSON-LD 1.1 frame | 61 pass | of 92 (json-ld-framing) | `floors/frame.rs` `FLOOR = 61` |
| OGC GeoSPARQL topology (hand-curated assertions) | 197 | sparq-authored battery, NOT the OGC suite | `ogc_compliance_ratchet.rs` `OGC_RATCHET_FLOOR = 197` |
| Solid WAC / ACP decision parity | 12 + 12 scenarios, differential oracles at 0 divergences | library-level, NOT wire-level CTH | `sparq-solid` floor consts |
| SolidLab ODRL Test Suite | 67 pass | of 68 (1 documented not-implemented) | `odrl_test_suite.rs` `ODRL_SUITE_FLOOR = 67` |
| W3C SPARQL 1.1 sparql11/service evaluation | 6 pass | of 7 `mf:QueryEvaluationTest` entries (1 documented skip: variable `SERVICE ?ep`) | `service_eval_suite.rs` `SERVICE_EVAL_FLOOR = 6` |
| W3C SPARQL 1.1 Protocol (HTTP) | 21 pass | sparq-authored raw-HTTP battery over the Protocol contract | `http_protocol_suite.rs` `HTTP_PROTOCOL_FLOOR = 21` |
| SPARQL 1.1 Service Description + Graph Store Protocol | 39 pass | sparq-authored battery | `sd_gsp_suite.rs` `SD_GSP_FLOOR = 39` |
| W3C SPARQL 1.1 D-entailment | 1 pass | the single D-only sparql11/entailment case graduated from the binary's OutOfScope bucket (the full sparql11/entailment manifest carries 70 `mf:QueryEvaluationTest` entries; the inference binary asserts 47, broader `pr:*` intensional arms stay experimental/OutOfScope) | `d_entail_suite.rs` `D_ENTAIL_FLOOR = 1` |
| W3C RIF WG Core test suite | 3 pass | pinned `Core_v1.22` archive; most cases in the importer skip taxonomy | `rif_wg_core_suite.rs` `RIF_WG_CORE_FLOOR = 3` |

The registry's `family = "sparq extension"` rows (BM25 oracle, RSP/SRBench,
RIF-Core expressivity, OWL 2 QL/EL/DL arms, D value-space matrix) are **not**
standards conformance and are excluded here by construction — comparing them
against peers' W3C EARL would be a category error.

## 3. Cross-engine tables (peer cells: published sources only)

Every peer cell is quoted from a page retrieved 2026-07-10. `NOT-PUBLISHED` means a
genuine search found no published per-suite result — it is NOT a sparq win. Where a
peer publishes only a BINARY "compliant" posture (no count), the cell says so and the
verdict is NOT-COMPARABLE (no like-for-like number exists to rank against).

### 3.1 W3C SPARQL — query / update / syntax

sparq: **1229** (1225 pass + 4 documented divergences, 0 fail) over the full harness
scope at `w3c/rdf-tests` `f25dbc0` (current consolidated repo, 2026-06 pin).

| Engine | Published SPARQL-suite result | Source (retrieved 2026-07-10) | Suite version |
|---|---|---|---|
| **sparq** | 1229 pass+div / 0 fail (query+update+syntax, full scope) | `scoreboard::SUITES` + `ci.yml RATCHET=1229`; `FINDINGS.md` round 5 | rdf-tests `f25dbc0`, 2026-06 |
| Apache Jena ARQ | Query 100.0%, Update 100.0% (percentages, no absolute count) | W3C "SPARQL 1.1 Evaluation Test Results" `www.w3.org/2009/sparql/implementations/` (Last-Modified 2013-03-20) | REC-era `data-sparql11`, 2013 |
| RDFLib | Query 99.6%, Update 98.7% (as `rdflib_sparql`) | same W3C 2013 report | REC-era, 2013 |
| OpenRDF Sesame (RDF4J predecessor) | Query 75.0%, Update 78.8% | same W3C 2013 report | REC-era, 2013 |
| Oxigraph | BINARY: "nearly fully conformant … 1.1"; runs live w3c/rdf-tests in CI minus an in-code exclusion list (sparql11-query: 5 excluded incl. 4 property-path "// Our property path handling is wrong"; sparql11-fed: `service5`; CSV/TSV: 3) — **no published count** | README + `testsuite/tests/sparql.rs`, `github.com/oxigraph/oxigraph` | current w3c/rdf-tests (live) |
| QLever | BINARY: "compliant with the SPARQL 1.1 standard since the end of June 2025"; CI dashboard is a JS app with no server-rendered numbers — **no published count** | wiki `Current-deviations…`; `docs.qlever.dev/compliance/` | current w3c/rdf-tests (live) |
| Comunica | BINARY: "all tests from the SPARQL 1.1 test suite now pass" (2020); no EARL artifact found — **no published count** | `comunica.dev/blog/2020-08-24-release_1_16/` | W3C SPARQL 1.1 query suite, 2020 |
| RDF4J | TCK test classes exist (`SPARQL11{Query,Update}ComplianceTest`); **no published pass count** | `rdf4j.org/javadoc/latest/…manifest/` | live W3C Approved SPARQL 1.1 |
| Virtuoso / Blazegraph / GraphDB / MillenniumDB | NOT-PUBLISHED (feature/deviation docs only, no suite counts); GraphDB documents exactly 2 self-described test-issue deviations | OpenLink/BlazeGraph/Ontotext/MillenniumDB docs | — |

### 3.2 W3C SPARQL 1.1 Protocol, Graph Store Protocol, Service Description

sparq: Protocol (HTTP) **21**, Service Description + GSP **39**, sparql11/service
evaluation **6/7** — all sparq-authored raw-HTTP / loopback batteries (see §2). No
peer publishes a modern count for these; the only published numbers are 2013.

| Engine | Protocol | GSP | Service Description | Source | Suite version |
|---|---|---|---|---|---|
| **sparq** | 21 (raw-HTTP battery) | 39 (SD+GSP battery) | (folded into the 39) | `http_protocol_suite.rs`, `sd_gsp_suite.rs` | sparq-authored |
| Apache Jena Fuseki | 100.0% | 100.0% | (Service Description 100.0% for Akamu/Leviathan/RDF::Query/SWObjects; Fuseki not in that column) | W3C 2013 report | REC-era, 2013 |
| everyone else (Oxigraph/QLever/Comunica/RDF4J/…) | NOT-PUBLISHED (the dedicated W3C `protocol_report/` + `gsp_report/` pages are 404) | NOT-PUBLISHED | NOT-PUBLISHED | — | — |

### 3.3 SPARQL 1.1 entailment regimes

sparq: the inference binary asserts **47** of the 70 `mf:QueryEvaluationTest`
sparql11/entailment cases; the opt-in `d-entail` lane graduates the **1** D-only case;
broader `pr:*` intensional arms (QL/EL/DL/RIF regimes) stay experimental/OutOfScope and
are NEVER summed into a conformance number.

| Engine | Entailment-suite result | Source | Suite version |
|---|---|---|---|
| **sparq** | 47 asserted + 1 D-only graduated; RDFS/RDF/D via `sparq-reason` | inference binary; `d_entail_suite.rs` | rdf-tests, 2026-06 |
| Pellet | 84.3% (highest in the report) | W3C 2013 report | REC-era, 2013 |
| SPARQLing HermiT | 67.1% | W3C 2013 report | REC-era, 2013 |
| Corese | 60.0% | W3C 2013 report | REC-era, 2013 |
| ARQ+Inference | 55.7% | W3C 2013 report | REC-era, 2013 |
| Stardog | 45.7% | W3C 2013 report | REC-era, 2013 |
| RDFLib | 31.4% | W3C 2013 report | REC-era, 2013 |
| modern Jena/Stardog/GraphDB/RDF4J | NOT-PUBLISHED (no current entailment-suite counts) | — | — |

### 3.4 W3C RDF-syntax suites (Turtle / N-Triples / N-Quads / TriG)

sparq runs the W3C rdf-turtle suite through its parser inside the inference ratchet
(**313** pass, folded into the 1967 inference total; see §2); it does not publish a
per-syntax-suite EARL row of its own beyond that. Peers DO publish rich EARL here.

| Parser | Turtle (of 291) | N-Triples (68) | N-Quads (85) | TriG (335) | Source | Suite version |
|---|---|---|---|---|---|---|
| **sparq** | 313 pass in the inference lane (rdf-turtle at the 2026-06 pin; different suite revision + denominator from the 291-test 2017 report) | (via oxttl for expected N-Triples) | — | — | inference binary | rdf-tests, 2026-06 |
| Apache Jena RIOT | 291/291 (100%) | 68/68 | 85/85 | 335/335 | `w3c.github.io/rdf-tests/…/reports/` (publishDate 2015–2017) | RDF 1.1-era |
| Raptor / Serd / N3.js / dotNetRDF / EYE | 291/291 (100%) | 68/68 | 85/85 | 335/335 | same reports | RDF 1.1-era |
| RDFLib | 287/291 (98.6%) | 48/68 (70.6%) | 85/85 | 335/335 | same reports | RDF 1.1-era |
| RDF4J/Rio, Oxigraph/oxttl | NOT-PUBLISHED (not in these rdf-tests reports) | — | — | — | — | — |

### 3.5 W3C JSON-LD 1.1

sparq floors (of the 2026-06 pinned manifests, denominators in parens): toRdf
**413**/467, fromRdf **51**/53, expand **276**/385, compact **186**/246, flatten
**53**/58, frame **61**/92. The official JSON-LD 1.1 EARL report (10-Apr-2025) reports
PERCENTAGES over slightly SMALLER denominators (toRdf 456, expand 376, compact 244,
flatten 55, fromRdf 52, frame 91) — a real **suite-version mismatch** (sparq pins a
later w3c/json-ld-api HEAD with more tests), so the pass-RATES are only loosely
comparable. sparq is NOT a subject in that EARL report; the peers below are the report's
own subjects (jsonld.js ≈ Comunica ecosystem; PyLD is Digital Bazaar's, not RDFLib).

| Category | sparq pass-rate | Best-published peer (rate, of the report denominator) | Notable peers | Source | Suite-version note |
|---|---|---|---|---|---|
| toRdf | 413/467 = 88.4% | jsonld-cpp 99.8%, JSON-LD.ex 99.6%, Titanium 97.6% (of 456) | jsonld.js 95.2%, PyLD 96.1% | `w3c.github.io/json-ld-api/reports/` | sparq denom 467 vs report 456 |
| fromRdf | 51/53 = 96.2% | Titanium 98.1%, Sophia 98.1% (of 52) | jsonld.js 94.2% | same | 53 vs 52 |
| expand | 276/385 = 71.7% | jsonld-cpp 100.0%, JSON-LD.ex 100.0%, Titanium 98.1% (of 376) | jsonld.js 97.3% | same | 385 vs 376 |
| compact | 186/246 = 75.6% | JSON-LD.ex 99.6%, Titanium 98.0% (of 244) | jsonld.js 97.5% | same | 246 vs 244 |
| flatten | 53/58 = 91.4% | all listed peers 100.0% (of 55) | Titanium/PyLD/jsonld.js 100.0% | same | 58 vs 55 |
| frame | 61/92 = 66.3% | jsonld.js 97.8%, Titanium 96.7% (of 91) | JSON-goLD 38.5% | same + Titanium README (frame 90/91) | 92 vs 91 |

Apache Jena delegates JSON-LD 1.1 to Titanium and publishes no run of its own; Oxigraph
publishes only a binary "1.1 conformant"; RDFLib has only a JSON-LD **1.0**-era result
(97%, 2013); QLever has no JSON-LD support. (All NOT-PUBLISHED as engine-level 1.1
counts.)

### 3.6 W3C SHACL

sparq: SHACL core **98/98** (`sht:Validate` at the 2026-06 pin) and SHACL-SPARQL
node+property **5/5** (plus a separate `component` runner at 3). The W3C SHACL report
partitions its 121-row matrix as 98 core + 23 SPARQL, so sparq's **98** core passes ==
the full core partition at this pin.

| Implementation | SHACL result | Source (retrieved 2026-07-10) | Suite version |
|---|---|---|---|
| **sparq** | core 98/98; SHACL-SPARQL node+property 5/5 (+3 component) | `w3c_core.rs BASELINE_PASS=98`, `w3c_sparql.rs`, `w3c_sparql_component.rs` | data-shapes `b6e7369`, 2026-06 |
| TopBraid SHACL API | 121/121 (100%), whole matrix | W3C `data-shapes-test-suite/` report (updated 2017-10-24) | 2017-era matrix (121) |
| dotNetRDF | 121/121 (100%) | same report (2019-07-01) | 2017-era matrix |
| pySHACL | 119/121 (99%) | same report (2018-09-24); FEATURES.md names its 2 failures | 2017-era matrix |
| Netage | 100/121 (83%) | same report (2017-05-23) | 2017-era matrix |
| Corese / shaclex | 98/121 (81%) | same report | 2017-era matrix |
| RDFUnit | 82/121 (68%) | same report (2017-10-18) | 2017-era matrix |
| Apache Jena SHACL / RDF4J | NOT-PUBLISHED (not in the W3C report; RDF4J states "SPARQL is not supported") | jena/rdf4j docs | — |

Version note: the W3C SHACL matrix (121 rows: 98 core + 23 SPARQL) and sparq's pin
share the same core denominator (98), so sparq's core row is directly comparable to the
peers' core sub-scores where those can be split out; the peer TOTALS above are over all
121 (core+SPARQL) and are thus a superset of sparq's asserted scope.

### 3.7 OGC GeoSPARQL (published third-party benchmark, requirements-weighted)

The published cross-engine data is the Jovanovik/Homburg/Spasić "GeoSPARQL Compliance
Benchmark" (ISPRS IJGI 10(7):487, 2021; arXiv:2102.06139), whose "compliance %" is
**requirements-weighted** (30 requirements × 3.33% each over 206 queries) — a DIFFERENT
unit from a test-count pass-rate, and it tests GeoSPARQL **1.0**. sparq's own 197
hand-curated DE-9IM/WKT/GML assertions are on a different axis and stay reported
separately; **sparq has now also been run through the benchmark itself** (sq-ql2iy), so
the row below is a same-artifact comparison on the same requirements weighting rather
than an estimate. It is NOT a claim of a bit-identical comparator: the runner
re-implements the benchmark's answer comparison, and the bound on that is quantified
under *Comparator equivalence* below.

| System | GeoSPARQL compliance (requirements-weighted, of 30 reqs / 206 queries) | Source |
|---|---|---|
| **sparq** | **72.22%** (143/206 correct) | `bench/geo/gsb.sh` + `crates/sparq-geo/examples/gsb_compliance.rs` |
| GeoSPARQL Fuseki 3.17 | 82.75% (177/206 correct) | arXiv:2102.06139 Table 2 |
| Ontotext GraphDB 9.3.3 | 69.75% | same |
| OpenLink Virtuoso 7.3 | 63.46% | same |
| Eclipse RDF4J 3.4.0 | 58.33% | same |
| Stardog 7.4.0 / Blazegraph 2.1.5 / plain Jena Fuseki 3.14 | 56.67% | same |
| Apache Marmotta 3.4.0 | 46.67% | same |

**How the sparq row was produced** (`bench/geo/gsb.sh` → `gsb_compliance`). The upstream
artifact is GPL-2.0, so it is fetched gather-only under a pinned sha256 and never
vendored; the runner reproduces the benchmark's own scoring table and RE-IMPLEMENTS its
answer comparator (ordered rows, `geo:wktLiteral` whitespace/case-folded, `geo:gmlLiteral`
put through a bounded XML normaliser — documented in `canonical_xml`, and deliberately
NOT claimed to be XML C14N — any `-alternative-N.srx` accepted). All 206 queries
evaluated in every configuration; none errored.

**Provenance — re-derived with the corrected comparator (2026-07-26).** An earlier
recording of this section (PR #3990, before review round 1) was measured with an answer
comparator that dropped XML entity references while parsing `.srx`, so every *escaped*
expected GML answer was mangled before comparison and could not match; the GML normaliser
was also lossy (empty elements, escaping, malformed input). Both were fixed in round 1,
and **all four configurations have since been re-run end-to-end** against the pinned
corpus (`bench/geo/gsb.sh`; the fetched tarball's sha256 matched the pin in its header).
The sparq row, the configuration matrix, and the per-requirement breakdown below are the
output of that corrected run — and are unchanged from the pre-fix recording, because the
requirements the fixes touch (R18/R19, the only ones whose expected answers carry GML)
did not change verdict. The numbers are no longer provisional.

**Comparator equivalence is BOUNDED, not established.** The runner re-implements the
benchmark's answer comparison instead of invoking the upstream harness: the corpus is a
gather-only GPL-2.0 download, so no differential run against the upstream canonicaliser
can be made from this tree. The `geo:wktLiteral` and plain-term path follows the upstream
normalisations; the `geo:gmlLiteral` path uses `canonical_xml`, which is explicitly not
XML C14N (notably it does not fold the namespace axis). That approximation can only move
decisions on the **23 of 206 queries whose expected answers carry a `geo:gmlLiteral`** —
1 in R18 (which passes) and 22 in R19 (which scores zero in every configuration) — so its
influence is confined to **2 of the 30 requirements, i.e. at most ±3.33 points** of the
72.22%; the other 28 requirements never reach that code path. That bound is small against
the 10.5-point gap to the leader, so the BEHIND verdict is safe — but it is LARGER than
the 2.47-point margin over GraphDB (69.75%), so **the 2nd-of-9 placing specifically is not
robust**: it rests on R18's single GML answer comparing equal. Until a differential check
against the upstream harness exists, read "same scoring" as *same artifact + same
requirements weighting*, NOT as a verified-identical comparator, and treat the placing as
2nd-or-3rd.

The row above is sparq's **best** configuration, and it is also the SHIPPED-DEFAULT query
entry point (`geosparql_rewrite` is opt-in and off by default). Both configuration axes
were measured, because both change the score materially:

| RDFS closure | query entry point | correct | compliance |
|---|---|---|---|
| on | standard (`sparq_engine::query`) | **143/206** | **72.22%** |
| off | standard | 144/206 | 70.00% |
| on | `geosparql_rewrite` | 134/206 | 68.47% |
| off | `geosparql_rewrite` | 140/206 | 68.33% |

Two things the matrix says. First, RDFS materialisation is worth ~2 points *despite*
lowering the raw correct-answer count: it wins R25–R27 outright while costing partial
credit on R3/R8/R9, where the benchmark's expected answers resolve `geo:hasGeometry` to
the asserted triple only (R3's `my:M` is a `my:PlaceOfInterest`, `rdfs:subClassOf
geo:Feature`, with no asserted `rdf:type geo:Feature` — the benchmark is internally
inconsistent with its own R25–R27 requirements there). Second, **`geosparql_rewrite`
currently *costs* ~4 points**: see the R28–R30 gap row below.

Per-requirement result in the best configuration (29 of 30 requirements carry queries;
R17 has none and is credited by the benchmark's own rule):

| Result | Requirements |
|---|---|
| full marks | R1, R2, R4, R5, R6, R7, R10, R11, R12, R14, R15, R18, R20, R21, R22, R23, R24, R25, R26, R27 |
| partial | R8 (1/2), R9 (1/6) |
| zero | R3, R13, R16, R19, R28, R29, R30 |

R3/R8/R9 are the RDFS trade-off just described, not capability gaps. The rest are genuine,
reproducible gaps, not harness artifacts (each triaged with `GSB_DEBUG=<query-id>`):

- **R28–R30 (query-rewrite extension, 24 queries).** Zero in EVERY configuration, and the
  one place the opt-in rewrite should have earned points. These queries ask the topology
  properties of `my:G`, whose `geo:sf*`/`eh*`/`rcc8*` triples the benchmark deliberately
  leaves unasserted. `sparq-geo`'s `geosparql_rewrite` expands the pattern to
  `(hasDefaultGeometry|hasGeometry)/asWKT` on both sides plus a `geof:` FILTER, which
  diverges from the expected answers three ways — it *replaces* rather than unions with
  the asserted-triple arm, it never matches a `geo:Geometry` subject directly (the
  expected answers include the geometry IRIs), and the property alternation duplicates
  rows. Net effect: it gains nothing on R28–R30 and *loses* R4 (8/8 → 3/8) and R5 (8/8 →
  4/8), which is the whole ~4-point cost in the matrix. See the follow-up issue.
- **R13/R16 (empty geometry, 4 queries).** An empty `geo:wktLiteral` / `geo:gmlLiteral` is
  a parse error in `sparq-geo` rather than an empty geometry, so `geof:sfEquals` on two
  empty literals leaves the variable unbound where the benchmark expects `true`.
- **R19 (non-topological `geof:` functions, 28 queries).** Every group evaluates, but none
  matches lexically: `geof:distance` differs from the reference metre values (~2%),
  `geof:buffer` is scored against a reference that buffered in coordinate units while
  sparq honours `uom:metre`, and `convexHull`/`envelope`/`boundary` return the same
  geometry with a different start vertex, winding, or `MULTI*` wrapper. The benchmark
  compares serialisations, not geometries — every published system is scored the same way.

### 3.8 W3C RIF Core

sparq: the W3C RIF WG Core lane asserts **3** of the pinned `Core_v1.22` archive
(honest-denominator lane — most cases sit in the importer's skip taxonomy; the sparq
EXTENSION RIF-Core expressivity ratchet, 73 assertions, is tallied separately and is
NOT a W3C-suite count). The only published W3C RIF results are one 2010 page, 3 systems,
mostly "no data".

| System | RIF Core result | Source | Suite version |
|---|---|---|---|
| **sparq** | 3 pass, W3C RIF WG Core cases end-to-end (honest skip taxonomy is the denominator) | `rif_wg_core_suite.rs RIF_WG_CORE_FLOOR=3` | `Core_v1.22` archive |
| Oracle Business Rules | Approved Core 24% (of 42) | `www.w3.org/2005/rules/test/results/report` (2010-08-16, suite v1.21) | RIF v1.21, 2010 |
| RIFLE | Approved Core 0% | same | 2010 |
| SILK | (no Core results; BLD 13%) | same | 2010 |
| Jena / Stardog / GraphDB / modern reasoners | NOT-PUBLISHED (no RIF-suite results) | — | — |

### 3.9 Solid WAC / ACP and SolidLab ODRL

sparq: WAC/ACP are **library-level decision-parity** (12+12 scenarios, differential
oracles at 0 divergences), explicitly NOT a wire-level Conformance-Test-Harness (CTH)
result (see `research/solid-cth-wire-conformance-feasibility.md`). ODRL is the one
DIRECT same-suite comparison in this whole record.

| Item | sparq | Peer / reference | Source | Comparability |
|---|---|---|---|---|
| SolidLab ODRL Test Suite | **67/68** (`ODRL_SUITE_FLOOR=67`, 1 documented not-implemented) | the SolidLab ODRL-Evaluator (the suite's own reference) scores **"67 … correct … out of 68"** | `odrl_test_suite.rs`; `github.com/SolidLabResearch/ODRL-Test-Suite` README | SAME 68-case suite, SAME oracle → directly comparable, EXACT match |
| Solid WAC / ACP | library-level 12+12 parity, 0-divergence oracle | Solid CTH per-server results are DELIBERATELY WITHHELD by the project; solidservers.org publishes checkmark-level Jest-suite results per SERVER (NSS/CSS/ESS), not pass counts | `solidservers.org` (via Wayback 2026-04-13); `solid-contrib/specification-tests` | NOT-COMPARABLE — different subject (library API vs whole-server wire) + no peer count |

## 4. Verdicts (fixed vocabulary)

Verdicts rank sparq only where a genuine, same-suite peer NUMBER exists. Where the peer
posture is binary or the suite/unit differs, the honest verdict is NOT-COMPARABLE — that
is a first-class outcome, not a hidden sparq win.

| Suite family | Verdict | Rationale |
|---|---|---|
| W3C SPARQL query/update/syntax | **NOT-COMPARABLE** | sparq has a hard 1229-count on the current suite; modern peers publish only a binary "compliant" posture (Oxigraph/QLever/Comunica) and the sole numeric multi-engine dataset is 2013 (REC-era, different suite). No like-for-like count to rank. sparq's fully-enumerated count is a transparency lead, not a measured pass-rate win. |
| SPARQL Protocol / GSP / Service Description | **NOT-COMPARABLE** | only 2013 Fuseki 100%/100% is published; no modern peer counts; sparq's are self-authored batteries. |
| SPARQL 1.1 entailment regimes | **NOT-COMPARABLE** (leaning BEHIND the 2013 OWL reasoners on breadth) | sparq asserts 47+1 of 70 with RDFS/RDF/D; the 2013 report's dedicated OWL reasoners (Pellet 84.3%, HermiT 67.1%) covered more of the regime surface, but at a different suite revision and sparq scopes OWL regimes to audited fragments by design. Honest: sparq does NOT claim full entailment-regime conformance. |
| RDF-syntax (Turtle/NT/NQ/TriG) | **NOT-COMPARABLE** | peers publish 100% at a 291/68/85/335-test RDF-1.1-era suite; sparq runs rdf-turtle (313) at a later pin inside the inference lane — different denominators + sparq publishes no per-syntax EARL row. No regression implied; just not the same measurement. |
| JSON-LD 1.1 (all 6 lanes) | **BEHIND** (behind most published processors) | at loosely-comparable denominators, mature JSON-LD processors (Titanium, JSON-LD.ex, jsonld-cpp, jsonld.js) sit at 97–100% on expand/compact/toRdf/flatten/frame; sparq is 66–96%. sparq's JSON-LD is a native round-trip implementation with honest recorded gaps (remote-context/writer-shape), not a full JSON-LD processor — genuine gap rows below. |
| SHACL core | **PARITY** (with the 100% reference impls) | sparq 98/98 core == the full core partition of the W3C matrix; TopBraid/dotNetRDF are 121/121 over core+SPARQL, pySHACL 119/121. On the CORE partition sparq is at the ceiling; the gap is SHACL-SPARQL breadth (sparq asserts node+property 5 + component 3), not core. |
| OGC GeoSPARQL | **BEHIND** (2nd–3rd of 9 on the published table) | sq-ql2iy ran sparq through the SAME 206-query benchmark under the SAME requirements weighting: **72.22%** (re-derived 2026-07-26 with the corrected comparator, §3.7), ahead of GraphDB 9.3.3 (69.75%) and every other peer, but 10.5 points behind the GeoSPARQL-Fuseki 3.17 leader (82.75%). The answer comparator is a re-implementation, not the upstream one, and its GML path is bounded (§3.7). The BEHIND-the-leader verdict is robust to that bound (the worst case stays 7 points behind Fuseki); the **2nd-of-9 placing is NOT** — the margin over GraphDB is 2.47 points, less than the 3.33 that R18 alone is worth, so 2nd place rests on R18's GML answer comparing equal under a comparator not differentially checked against upstream. Rank sparq 2nd-or-3rd until it is. Gap rows in §3.7 and fix beads in §5. |
| RIF Core | **NOT-COMPARABLE** (leaning AHEAD of the 2010 record) | sparq's 3 W3C-Core passes + 73-assertion expressivity extension exceed the 2010 record's best (Oracle 24% of 42 Core), but the 2010 suite is v1.21 and only 3 systems ever submitted — too thin + too stale to rank meaningfully. |
| SolidLab ODRL | **PARITY** (exact match with the reference evaluator) | 67/68 on the identical suite with the identical oracle — the one strictly comparable, strictly matched row. |
| Solid WAC/ACP | **NOT-COMPARABLE** | sparq is library-level; peers are whole-server wire-level and their CTH counts are deliberately withheld. Different subject entirely. |

## 5. Follow-ups (candidate beads for the orchestrator)

- **JSON-LD 1.1 gap-closure (P2).** sparq trails mature processors on expand
  (71.7%), frame (66.3%), compact (75.6%). File a conformance-gap bead to raise the
  `floors::{expand,frame,compact}::FLOOR` consts toward the 97–100% peer band, closing
  the honest recorded divergences (remote-context, writer-shape). This is the one
  clearly-BEHIND family with a same-family comparison. **BEHIND → immediate fix bead
  per the §5.3 rule.**
- **GeoSPARQL benchmark run — DONE (sq-ql2iy).** sparq-geo now runs the 206-query
  Jovanovik benchmark (`bench/geo/gsb.sh` + `crates/sparq-geo/examples/gsb_compliance.rs`),
  scoring **72.22%** against Fuseki 82.75% et al., re-derived 2026-07-26 with the corrected
  answer comparator. The row in §3.7 is a same-artifact comparison and the verdict moved
  NOT-COMPARABLE → BEHIND.
- **GeoSPARQL comparator differential check (P3).** The runner re-implements the
  benchmark's answer comparison; its bounded GML normaliser is not differentially checked
  against the upstream harness (§3.7). The exposure is 2 of 30 requirements, but that
  exceeds the margin over GraphDB, so it is what keeps the 2nd-of-9 placing at
  2nd-or-3rd. A check would need the upstream GPL-2.0 harness run out-of-tree.
- **GeoSPARQL query-rewrite semantics (P1).** The largest scoring gap and the only one
  that is a REGRESSION rather than a missing feature: R28–R30 (24 of the 63 wrong answers,
  10% of the total score) score zero with the rewrite on AND off, and turning the opt-in
  rewrite on additionally *loses* R4/R5 — a net −4 points versus not using it. The
  expansion replaces the asserted-triple arm instead of unioning with it, never matches a
  `geo:Geometry` subject directly, and duplicates rows via the property alternation.
  Fixing it is worth ~10 points and would put sparq within 5 of the leader. **BEHIND →
  immediate fix bead per the §5.3 rule.**
- **Empty geometry literals (P3).** R13/R16: an empty `geo:wktLiteral` / `geo:gmlLiteral`
  should parse as an empty geometry, not error.
- **SPARQL 1.1 entailment-regime breadth (tracked, not new).** sparq asserts 47+1/70 by
  design (audited fragments only); broadening is already tracked under the `sq-pbz04`
  reasoner epic — no new bead, cross-referenced here for the honesty record.
- No follow-up for SPARQL/Protocol/syntax/ODRL/SHACL-core: NOT-COMPARABLE (no peer
  number) or PARITY (at the ceiling).

## 6. Sources (all retrieved 2026-07-10)

- W3C "SPARQL 1.1 Evaluation Test Results" — <https://www.w3.org/2009/sparql/implementations/> (Last-Modified 2013-03-20).
- W3C RDF-tests reports — <https://w3c.github.io/rdf-tests/> (rdf-turtle publishDate 2017/01/10; n-triples 2015/06/21; n-quads 2015/01/03; trig 2015/06/21).
- W3C JSON-LD 1.1 Processor Conformance report (10 Apr 2025) — <https://w3c.github.io/json-ld-api/reports/>.
- Titanium JSON-LD README — <https://github.com/filip26/titanium-json-ld>.
- W3C SHACL Test Suite and Implementation Report — <https://w3c.github.io/data-shapes/data-shapes-test-suite/>.
- pySHACL FEATURES.md — <https://github.com/RDFLib/pySHACL/blob/master/FEATURES.md>.
- Jovanovik, Homburg, Spasić, "A GeoSPARQL Compliance Benchmark", ISPRS IJGI 10(7):487, 2021 — arXiv:2102.06139; benchmark repo <https://github.com/OpenLinkSoftware/GeoSPARQLBenchmark>.
- W3C RIF Test Results (2010-08-16, suite v1.21) — <https://www.w3.org/2005/rules/test/results/report>.
- SolidLab ODRL Test Suite — <https://github.com/SolidLabResearch/ODRL-Test-Suite>.
- solidservers.org Solid Test Suite panel (via Wayback 2026-04-13) — <https://web.archive.org/web/20260413015257/https://solidservers.org/>.
- Oxigraph README + `testsuite/tests/sparql.rs` — <https://github.com/oxigraph/oxigraph>.
- QLever compliance docs — <https://docs.qlever.dev/compliance/>.
- Comunica 1.16 release (SPARQL 1.1) — <https://comunica.dev/blog/2020-08-24-release_1_16/>.
- Ruby RDF::N3 implementation report — <https://ruby-rdf.github.io/rdf-n3/etc/earl.html>.
