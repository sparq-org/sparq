# Paper Selection — Novelty/SOTA-Ranked Shortlist + Venue Map

<!-- [OPUS-4.8] sq-5reoy (#1599): the in-tree `zk/ieee754` and `zk/xpath` Noir trees were externalized to the `sparq-org/noir_IEEE754` (v0.10.0) and `sparq-org/noir_XPath` (v0.2.0) face repos and REMOVED from this repo; `zk/compose` now consumes the released `sparq_ieee754` as a pinned Nargo git dependency. Any `zk/xpath/…` / `zk/ieee754/…` path below is a HISTORICAL in-tree reference — the live source is the corresponding face repo. -->

> 🤖 SPARQ agent `[FABLE-5]` — bead **sq-ytk7p** (parent epic **sq-gum8**), 2026-07-04.
> Fable-architect decomposition record: ONE design record + disjoint child beads.
> Non-sycophantic by mandate: kills are written down with reasons.

**Maintainer directive (2026-07-04):** prioritise papers by (a) genuine **NOVELTY**
of something about the engine, or (b) something about the engine being **EXTREMELY
state-of-the-art**. Select 2–3 top-tier targets.

**Honesty frame (load-bearing, inherited from the factory):**

1. **No ZK/MPC security claim past the audit gate.** The single-prover ZK verifier is
   internally re-audited only; the EXTERNAL accredited-cryptographer audit (bead
   **sq-qhy4**, P0) is open. MPC is semi-honest-only; the collaborative-proof path is
   unbuilt and fails closed. Every crypto paper below is scoped **"design +
   implementation, pre-audit"** — no soundness, zero-knowledge, privacy, or
   attestation property is claimed as achieved.
2. **No perf claim without canonical numbers.** All wall-clock figures gathered on the
   work box / M1 / Docker are NON-canonical. Only deterministic integer metrics
   (conformance floors, gate counts, byte-identity, recall floors) are canonical
   today. Perf headlines require the canonical EC2 runner
   (`research/ci-ec2-design.md`; beads sq-me8x, sq-vw3ax.12).
3. No hard-coded performance numbers in repo markdown; papers bind numbers only
   through the evidence layer (`site/papers/_lib/bench.typ` `headline()`/`ev()`).

---

## 1. Scope and relationship to prior factory records

This is the third selection-shaped record in the factory, and it does a different job
from the first two:

- `research/paper-contributions-inventory.md` (phase 2) — the *intake*: a ranked
  inventory of every candidate contribution with readiness verdicts
  (PUBLISHABLE-NOW / NEEDS-CANONICAL-BENCHMARKS / NOT-YET-SOUND).
- `research/papers-venue-audit.md` (sq-gum8.2) — the *retrospective*: per-paper
  venue-bar verdicts over the seven `.typ` papers that existed then
  (KILL × 3, REWRITE × 2, MERGE × 1, NEW_TOPIC_INSTEAD × 1), plus three missing
  topics. Its output drives the rewrite program (sq-gum8.3, in progress).
- **This record** — the *forward selection*: given the maintainer's two axes
  (novelty / extreme-SOTA) and a partially new candidate set, run a fresh
  related-work search per candidate (real literature search, 2026-07-04), kill what
  does not survive, and cut the survivors into disjoint child beads with
  contribution→evidence maps.

New evidence since the venue audit that changes the picture:

- The **SoK** (`site/papers/verifiable-fed-sparql.typ`, sq-gum8.4, Rev 2 post-review)
  now exists — the venue audit's missing-topic #3 is DONE. Any further ZK/MPC paper
  must add something the SoK does not (a construction + evaluation, not more
  systematization).
- The **zkSPARQL spec** (`site/specs/zksparql.typ`, sq-rvgr2.2, Rev 2 post-review)
  and the **MPC-SPARQL spec** (`site/specs/mpc-sparql.typ`) exist as
  architecture-overview documents with normative kernels and threat models.
- The **in-circuit numerics estate** matured: `zk/ieee754` (the maintainer's own
  vendored Noir IEEE-754 f16/f32/f64/f128 library, gate-count-optimised, with
  reproducible gate baselines under `zk/ieee754/bench/`) now has a **differential
  harness vs the hardware f32/f64 oracle** (PR #1471, PROOF M1, with a fault-injection
  non-vacuity self-test), and `zk/xpath` ships XPath/XQuery numeric operators with an
  honest per-function feasibility table (`zk/xpath/SPARQL_COVERAGE.md`, including
  explicit not-feasible rows).
- The **reasoner + streaming + geo breadth program** landed floors that did not exist
  at audit time: OWL 2 QL DL-Lite_R certain-answer oracle (sq-qo1a9), OWL 2 EL suite
  floor 50, RSP expressivity floor 317, OGC topology 197 + query-rewrite 48, SPARQL
  1.1 Protocol / Service-Description / Graph-Store-Protocol conformance lanes over an
  in-process loopback server, opt-in RIF-Core rules + RIF/XML importer
  (`crates/sparq-reason/src/rif.rs`, `rif_xml.rs`), all on the shared zero-overhead
  substrate (`crates/sparq-substrate`, phases 2–4: numeric tower, join kernels, term
  total order — behaviour-neutral, conformance-floor bit-identical).
- The **mechanized-proof program** (`sq-sqtk2`) was created 2026-07-04 — a day-old
  epic whose own architect decomposition has not run yet.

## 2. Method

For each candidate: (a) verify the in-repo evidence against the actual crates/tests
(not the epic's framing); (b) run a real related-work search (DBLP / Google Scholar /
arXiv / IACR ePrint / recent ISWC-ESWC-WWW-VLDB-SIGMOD-CCS-PETS-USENIX-ISSTA
proceedings, performed 2026-07-04 by four parallel search agents; citations below
were link-verified by those agents); (c) state the closest existing work and the
exact delta; (d) issue a verdict — **SELECT** (gets a child bead), **FOLD** (becomes
a section of a selected paper), **DEFER** (real but premature; revisit trigger
named), or **KILL** (novelty does not survive the search). SOTA claims are judged
relative to what the real engines publish (QLever, Oxigraph, RDFox, Jena, Virtuoso,
MillenniumDB, Tentris, qEndpoint, Comunica).

The candidate set was the maintainer's six, plus one **extension candidate** the
search surfaced (§3.7) — the brief explicitly allowed extending the set.

---

## 3. Candidate audits

### 3.1 zkSPARQL — ZK proofs of SPARQL result correctness over signed RDF — SELECT (re-scoped)

**Corrected premise (important).** This is NOT a paper to be written from scratch.
The paper already exists publicly at [zksparql.org](https://zksparql.org/) — *"Zero-Knowledge Proof of Correct SPARQL Evaluation over Verifiable Credentials"* (Wright,
Shadbolt, J. Zhao, R. Zhao, Braun), labeled an **ISWC 2026 submission** (research-track notification is 2026-07-16 per the ISWC 2026 CFP). The factory's job is
therefore **submission support** — the evaluation artifacts and related-work
hardening a hostile reviewer will demand — for the camera-ready if accepted, or the
fast resubmission if not.

**In-repo evidence (verified).** `zk/compose/` (scan/join/filter circuit families,
holder proof-of-possession, revocation non-membership, hidden-issuer, dual-leaf value
lanes), `crates/sparq-zk`/`sparq-zk-compose` (manifest prover + `verify_manifest`),
`zk/ieee754` (f16/f32/f64/f128 Noir library + PR #1471 differential harness),
`zk/xpath` (honest feasibility table), deterministic gate-count benches
(`bench/zk-compose/gate_counts_latest.json`, `zk/ieee754/bench/`), the vendored
`crates/sparq-trust/ontologies/zkp-sparql/` extending the maintainer's sec-prop
ontology line, and the reviewed spec `site/specs/zksparql.typ`.

**Closest work (search-verified).**

- SQL/graph analogues: **ZKSQL** (Li et al., PVLDB 16(8) 2023 — VOLE-based
  interactive ZK for SQL answers incl. ZK joins), **PoneglyphDB** (Gu et al., SIGMOD
  2025 — non-interactive PLONKish ZK for SQL), **ZKGraph** (arXiv:2507.00427, Jul
  2025 — ZK graph-query evaluation, PLONKish, no RDF/SPARQL), **VeriDKG** (PVLDB 17,
  2024 — verifiable SPARQL via an authenticated data structure; integrity, not ZK);
  ancestry IntegriDB (CCS 2015), vSQL (S&P 2017).
- In-circuit floats: **ZKLP** (Ernstberger et al., **IEEE S&P 2025**) explicitly
  claims *"the first set of ZKP circuits fully compliant with IEEE 754"* (f32/f64,
  lookup-optimised). Earlier: Garg et al. CCS 2022, Mystique USENIX Sec 2021. The
  maintainer's own public `noir_IEEE754` repo is prior art of this codebase.
- Own line (the salami-slicing exposure a reviewer will probe): Braun & Käfer ESWC
  2025 (RDF-level selective disclosure + ZKP), **Braun, Wright & Käfer** (ESWC 2026
  by DOI series) — *soundness of SPARQL results via selective disclosure*, i.e.
  dataset-soundness, not evaluation-correctness; Wright ISWC 2025 companion
  (CEUR Vol-4085).

**Delta (real, survives).** First proof-of-correct-*evaluation* for a substantive
SPARQL 1.1 algebra fragment (OPTIONAL / MINUS / EXISTS have no ZKSQL/ZKGraph
equivalent) over issuer-signed, multi-credential RDF, unifying credential-layer
proofs (possession, revocation, hidden cross-credential joins) with query-layer
proofs in one NIZK. **What kills it:** claiming "first ZK floats" (ZKLP exists),
failing to cite ZKGraph/PoneglyphDB, or an unstated delta vs the authors' own ESWC
line. The scout's judgement: novelty survives at ISWC/ESWC and plausibly PETS with a
strong evaluation; it would struggle at CCS/S&P as a crypto-novelty claim.

**Verdict: SELECT — as a submission-support bead** (related-work hardening +
deterministic constraint-count evaluation pack), NOT a new paper draft. Everything
stays scoped "design + implementation, pre-audit" (sq-qhy4 open; forge tests
toolchain-gated, sq-1gir).

### 3.2 MPC-federated SPARQL with attested sources — DEFER

**In-repo evidence (verified).** M0–M3 built semi-honest (Shamir ops, disclosed-key
join over global IRIs, bounded property paths, secure compare), the 3-axis fail-closed
capability registry, ~30 design records; the malicious-secure layer is a stub; the
attested-input join is an honest `NotYetImplemented` stub.

**Closest work (search-verified).** SQL line: SMCQL (PVLDB 2017), Conclave (EuroSys
2019), Senate (USENIX Security 2021, maliciously secure), Secrecy (NSDI 2023),
SECYAN (SIGMOD 2021), SecretFlow-SCQL (PVLDB 2024). Graph line: GOOSE (DBSec 2020 —
SPARQL but cloud-outsourcing, not multi-owner MPC), GORAM (PVLDB 18, 2025 —
secret-shared federated graphs at billion-edge scale), and — closest — the
Liverpool line (Aljuaid/Lisitsa/Schewe, ICISSP 2022–2025 + SN CS 2025): semi-honest
Shamir MPC over **federated graph databases** (Cypher/Neo4j), including a 2025
**traversal-queries** paper overlapping the bounded-path claim. No
MPC-over-federated-**SPARQL** work was found — the gap is real but thin.

**Why DEFER.** A hostile PoPETs/VLDB reviewer: (1) semi-honest-only with no
benchmarks vs Secrecy/SECYAN/GORAM is below table stakes; (2) "public IRIs as join
keys" needs a formal leakage profile (SMCQL/Conclave formalized public/private
hybrid annotations in 2017/2019); (3) the capability registry is policy engineering,
not a crypto contribution; (4) the unbounded-path negative result is oblivious-algorithm folklore (Blanton & Steele, ASIACCS 2013: pad to a public bound) unless
proven as an impossibility under stated constraints; (5) the designed-not-built
attestation layer draws a "vaporware" note. The SoK already covers the
systematization value. Today's delta is workshop-sized.

**Revisit triggers:** malicious-secure layer real (not a stub) + canonical MPC
benchmarks vs Secrecy/SECYAN/GORAM + the attested-input join built + the leakage
profile formalized — then a PoPETs construction paper becomes credible.

### 3.3 Differential + mechanized assurance methodology — KILL as framed (superseded by §3.7)

**In-repo evidence (verified).** `crates/sparq-difftest` (engine-independent
value-level comparator; deliberately depends on no sparq crate), the Oxigraph
differential fuzz gate, PR #1471 (ieee754 differential M1). The mechanized half
(`sq-sqtk2`) is a day-old epic: no Kani/Creusot/Lean proof has been completed. No
bug in a third-party engine has been found or reported upstream (the records only
*propose* carrying upstream issue links).

**Field bar (search-verified).** SQLancer (Rigger & Su): ~196 previously-unknown
bugs across production DBMSs (PQS ≥121, NoREC 51, TLP 77). Datalog: QueryFuzz
(ESEC/FSE 2021, 13 bugs), DLSmith (ISSTA 2023, 16). Graph DBs: Gremlin differential
(ISSTA 2022), GDsmith (ISSTA 2023, 27, Cypher), Gamera (PVLDB 17, 2024, 39 logic
bugs). Mechanized: HoTTSQL/Cosette/Q\*cert, Coq SQL semantics (CPP 2019), DBCert
(OOPSLA 2022) — all *completed* proofs.

**Why KILL (as framed).** Testing venues require previously-unknown, developer-confirmed bugs in third-party systems (field norm 13–196); CAV-industrial requires
completed proofs of a deployed artifact. Differentially testing your own engine
against Oxigraph is QA, not a publishable result; planned proofs count for zero.
The candidate is premature on both halves — but the search exposed a genuinely open
niche it should be retargeted at (§3.7).

### 3.4 Zero-overhead shared eval substrate across 7+ spec families — SELECT (as the engine systems paper's spine)

**In-repo evidence (verified).** `crates/sparq-substrate` (rows / numeric tower /
join kernels / term total order, monomorphic via traits, behaviour-neutral moves with
bit-identical conformance floors), consumed by `sparq-engine` + the reasoners
(`sparq-reason`, `-el`, `-ql`, `-dl`), RSP (`sparq-rsp`), with RIF-Core, GeoSPARQL
(`sparq-geo`), SHACL as opt-in family crates; the cross-family conformance
scoreboard (`crates/sparq-conformance/src/scoreboard.rs`) pins the floors.

**Closest work (search-verified).** Engine systems papers exist for QLever (CIKM
2017 + Sparqloscope ISWC 2025 from the same group), RDFox (ISWC 2015), MillenniumDB
(2023/2024), Tentris (ISWC 2020, update ISWC 2025), Virtuoso (2009), Jena (WWW
2004), OWLIM/GraphDB (SWJ 2011); **no peer-reviewed paper** for Oxigraph or Stardog.
No paper combines query + OWL profiles + RSP + geo + SHACL with a *measured shared
substrate* — but GraphDB/Stardog/Jena ship comparable breadth commercially without
papers, so breadth alone reads as product engineering. **qEndpoint** (SWJ 2024)
already published "Wikidata on commodity hardware" — the closest prior claim to the
memory-frugality headline. The prior venue audit already killed the substrate as a
standalone code-move paper; that verdict stands.

**Delta (defensible only as a combination).** "N W3C/OGC spec families on one
substrate at zero *measured* marginal overhead + QLever-class compute at a fraction
of committed memory, out-of-core, correctness differential-gated." A reviewer will
demand: head-to-head on the Bast group's **Sparqloscope** (ISWC 2025) benchmark,
native (non-Docker) QLever, and a memory-accounting-honest comparison vs qEndpoint
(mmap page cache vs committed bytes WILL be attacked). Kill risk: "breadth =
engineering; memory = accounting trick" — the paper must pre-empt both.

**Verdict: SELECT** — the maintainer's extreme-SOTA axis paper. Submission is
**hard-gated on canonical benchmarks** (sq-vw3ax.12: competitor baselines on the
canonical EC2 host; sq-me8x). The WASM story (§3.5) and the conformance breadth
(§3.6) FOLD into this paper as sections (deployment surface; correctness evidence).

### 3.5 Full-engine browser WASM — KILL standalone, FOLD into §3.4

**In-repo evidence (verified).** `crates/sparq-wasm` (full SPARQL 1.1 engine incl.
WCOJ Leapfrog Triejoin, single-threaded, npm `@sparq-org/sparq`), plus
`sparq-reason-wasm`, `-rsp-wasm`, `-shacl-wasm`, `-text-wasm`.

**Closest work (search-verified).** Oxigraph's npm package already ships SPARQL 1.1
Query+Update in-browser via Rust/WASM; Comunica (ISWC 2018) is browser-capable;
EYE-JS / HyLAR+ / Tiny-ME cover in-browser reasoning; Nemo runs Rust LFTJ Datalog
via WASM; RSP-JS exists; WasmTree (ESWC 2021) covers Rust/WASM RDF-store splits.
DuckDB-Wasm (PVLDB 2022) is the bar for "X-in-WASM as a first-tier paper" — and its
contributions were async worker execution, a paged browser filesystem for
out-of-core data, and JS UDFs, not "we compiled it."

**Why KILL standalone.** Every piece has a browser precedent; `wasm-pack` is not a
contribution. The honest delta that WOULD clear the bar — out-of-core/persistent
browser storage (OPFS paging) — is not built. Until then the WASM breadth is one
*deployment-surface section* (and the wasm-bundle-bytes CI metric one deterministic
figure) of the §3.4 systems paper.

### 3.6 Conformance breadth as a resources paper — KILL as engine-resource

**In-repo evidence (verified).** The cross-family scoreboard floors (SPARQL, RDFS/
OWL-RL 1967, SHACL 98 + SHACL-SPARQL, five JSON-LD suites, OGC 197+48, Solid
WAC/ACP 12+12 with divergence-floor 0, ODRL 67, protocol/SD/GSP lanes, RSP 317,
EL 50, QL DL-Lite 11, D-entailment, BM25 oracle).

**Track bar (search-verified).** The ISWC Resources track scores potential impact,
**reusability by others**, documentation, availability (mandatory statement,
persistent URI). Comunica's ISWC 2018 Resources paper won as a *modular research
platform with demonstrated adoption* — not as a fast engine. A hostile reviewer on
an engine-as-resource submission with little external adoption: "impressive CI; who
outside the authors uses it? Why reuse this over Oxigraph/Jena/QLever?" Survival
odds: low. A conformance scoreboard is self-evaluation, not community evidence.

**Why KILL (for now).** Adoption evidence does not exist yet and cannot be
manufactured by writing. The scoreboard's real near-term value is as the
*correctness-evidence appendix* of the §3.4 systems paper (where it is strong).
**Conditional reframe (revisit trigger):** the machine-checked multi-spec
conformance-ratchet *harness itself*, made engine-agnostic and adopted by at least
one external engine, would be a genuinely novel Resources submission — nothing
comparable surfaced in the search. Revisit if/when external adoption exists.

### 3.7 EXTENSION CANDIDATE — SPARQL-engine logic-bug testing ("SQLancer for SPARQL") — SELECT

**Where it came from.** The §3.3 search established both (a) the field bar and (b)
the gap: **no dedicated differential/metamorphic logic-bug testing paper exists for
SPARQL engines.** SQL (SQLancer line), Datalog (QueryFuzz, DLSmith), Cypher/Gremlin
(GDsmith, Gamera, ISSTA 2022) are all covered; for SPARQL the closest thing is
SparqLog (arXiv 2023) *incidentally* reporting Virtuoso wrong-results on 14/77
queries. First-mover space at ISSTA/FSE/PVLDB, and it matches the maintainer's
novelty axis exactly.

**In-repo head start (verified).** `crates/sparq-difftest` is a deliberately
engine-independent value-level comparator (multiset + ORDER BY equivalence, exact
numeric/temporal semantics, SPARQL-Results-JSON reader) — precisely the comparator
core such a campaign needs; the protocol conformance lanes (sq-jaj38/sq-1uuxz) give
HTTP client machinery for driving external endpoints; the engine itself is a second
oracle. What does NOT exist yet: SPARQL-aware metamorphic oracles (TLP must be
re-derived for SPARQL's three-valued EBV/error semantics — that re-derivation *is* a
contribution, not an obstacle), a query generator, cross-engine drivers, and — the
publishability gate — **confirmed bugs in third-party engines**.

**Honest constraint.** The paper is publishable ONLY with previously-unknown,
developer-confirmed third-party bugs (field norm 13–196). If a real campaign against
Jena/Virtuoso/Blazegraph/GraphDB/QLever/Oxigraph/MillenniumDB finds nothing
substantive, the honest fallback is a much smaller experience/negative-result note —
plan for that possibility explicitly. No fabricated or inflated bug counts; every
counted bug needs an upstream issue link and maintainer confirmation.

**Verdict: SELECT** — harness first (net-new opt-in crate), paper second, dep-edged.

---

## 4. Venue + deadline map (survivors)

Searched 2026-07-04; dates AoE; "(pattern)" = CFP not yet published, inferred from
prior years.

| Paper | First-choice venue + next deadline | Second choice | Notes |
| --- | --- | --- | --- |
| P1 zkSPARQL (§3.1) | Already submitted: ISWC 2026 (notification **2026-07-16**) | PoPETs 2027 Issue 2 (**2026-08-31**, firm) or Issue 3 (2026-11-30); IEEE S&P 2027 Cycle 2 (abs 2026-11-10 / full 2026-11-17) | Hardening pack must be ready AT notification: camera-ready if accept, fast PoPETs resubmission if reject. Scout judgement: PETS-plausible, CCS/S&P-hard |
| P2 SPARQL logic bugs (§3.7) | ISSTA 2027 (~late-Jan 2027, pattern) | PVLDB Vol 20 rolling (1st of month through 2027-03-01; Gamera precedent) or FSE 2027 | ICSE 2027 single deadline PASSED (2026-06-30). The ~6-month campaign runway fits |
| P3 Engine systems (§3.4) | PVLDB Vol 20 rolling (monthly through **2027-03-01**) | ICDE 2027 R2 (**2026-11-11**); EDBT 2027 cycle 3 (2026-10-07, stretch) | Submission hard-gated on canonical EC2 baselines (sq-vw3ax.12) + Sparqloscope + native QLever + qEndpoint memory-honesty comparison. CIDR 2027 (2026-08-04) too soon — do not rush unbenchmarked |

Killed/deferred candidates get no venue row by design.

## 5. The selection (2 new papers + 1 submission-support pack)

**P1 — zkSPARQL submission support** (novelty axis; time-critical).
Contribution→evidence: SPARQL-fragment evaluation-correctness circuits →
`zk/compose/*` + `crates/sparq-zk-compose` (manifest verifier); in-circuit IEEE-754
completeness + differential validation → `zk/ieee754` + PR #1471 harness; XPath
operator feasibility (incl. honest not-feasible rows) → `zk/xpath/SPARQL_COVERAGE.md`;
deterministic constraint counts → `bench/zk-compose/gate_counts_latest.json` +
`zk/ieee754/bench/*`; ontology lineage → `crates/sparq-trust/ontologies/zkp-sparql/`.
Work: (i) related-work hardening — cite + differentiate ZKLP (S&P 2025; kills any
"first ZK floats" phrasing), ZKGraph, PoneglyphDB, ZKSQL, VeriDKG, zk-creds/
Crescent, and an explicit self-delta vs Braun–Wright–Käfer (dataset-soundness vs
evaluation-correctness) in `site/specs/zksparql.typ` §related-work; (ii) a
reproducible constraint-count evaluation pack (scripted, deterministic, vs
ZKLP-reported figures as *cited* numbers — never re-measured claims about others'
systems). Experiments still needed: none for the pack itself (gate counts are
deterministic); prover/verifier wall-clock, if wanted, requires canonical EC2 runs.
Honesty constraints: pre-audit scoping everywhere (sq-qhy4, sq-1gir, INV-VL/CR-G8);
no security/privacy property asserted as achieved.

**P2 — the first SPARQL-engine logic-bug testing paper** (novelty axis).
Contribution→evidence: engine-independent value-level comparator →
`crates/sparq-difftest`; three-valued-logic-aware TLP/NoREC re-derivation for SPARQL
→ NEW (the harness bead's design core); cross-engine campaign machinery → protocol
lane machinery (sq-jaj38/sq-1uuxz) + NEW drivers; found-bug ledger → NEW (upstream
issue links, developer-confirmed only). Experiments still needed: the campaign
itself (Jena, Virtuoso, Blazegraph, GraphDB, QLever, Oxigraph, MillenniumDB; sparq
included as a target — finding bugs in ourselves is honest and strengthens the
paper). Honesty constraints: bug counts are confirmed-only; no wall-clock claims
needed; if the campaign yields too little, downgrade the paper honestly.

**P3 — the engine systems paper** (extreme-SOTA axis).
Contribution→evidence: out-of-core architecture (6 permutations, lazy counts, inline
tagged ValueIds, WCOJ+bind joins) → `sparq-core`/`sparq-engine` +
`research/BENCHMARKS.md` analysis (numbers non-canonical, architecture real);
zero-overhead substrate breadth → `crates/sparq-substrate` phases 2–4 +
perf-neutrality evidence + the reasoner/RSP/geo family crates; correctness →
differential gate + the cross-family conformance scoreboard; deployment breadth →
the WASM crates (§3.5 folded) with the deterministic bundle-bytes CI metric.
Experiments still needed (submission-gating): canonical EC2 baselines vs native
QLever/Oxigraph/others (sq-vw3ax.12), Sparqloscope run, qEndpoint memory-honesty
comparison, substrate zero-overhead measurement on the canonical host. Honesty
constraints: no perf headline before canonical numbers; memory accounting method
stated explicitly; component techniques attributed (LFTJ = Veldhuizen ICDT 2014;
6 permutations = RDF-3X; inline ids = QLever/Virtuoso lineage).

## 6. Decomposition — disjoint child beads (all under sq-gum8)

| Bead | Deliverable | File-area (disjointness) | Tier | Priority |
| --- | --- | --- | --- | --- |
| **sq-gum8.5** (P1 support) | zkSPARQL related-work hardening + constraint-count evaluation pack | `site/specs/zksparql.typ` + `bench/zk-compose/**` | opus | P1 |
| **sq-gum8.6** (P2 harness) | Metamorphic+differential SPARQL logic-bug harness (opt-in, net-new crate) | `crates/sparq-metamorph/**` + root `Cargo.toml` member line | opus | P1 |
| **sq-gum8.7** (P2 paper) | Logic-bug paper draft (after campaign evidence exists) | `site/papers/sparql-logic-bugs.typ` (net-new) | opus | P2 |
| **sq-gum8.8** (P3 paper) | Engine systems paper draft | `site/papers/sparq-engine-systems.typ` (net-new) | opus | P2 |
| **sq-gum8.9** (wiring) | Register new papers in the factory + evidence records | `site/src/data/papers.ts` + `site/src/data/paper-evidence.json` | haiku | P3 |

Dependency edges (real ordering only): sq-gum8.6 → sq-gum8.7 (no paper without
campaign evidence); sq-vw3ax.12 → sq-gum8.8 (no evaluation without canonical
baselines); sq-gum8.7 → sq-gum8.9 and sq-gum8.8 → sq-gum8.9 (the wiring bead is the ONLY
bead touching the shared registry/evidence files — that is what keeps the paper
beads conflict-free). The two net-new `.typ` beads may run in parallel: the site
build only compiles papers listed in the registry, so unregistered net-new files
cannot break the site surface; the single wiring bead is the serialization point
for the `site/` conflict-partition rule.

ZK/MPC-soundness-sensitive beads (P1 support) stay **maintainer-armed** — no fleet
auto-arm.

## 7. Factory infrastructure to reuse (do not rebuild)

- Single-source Typst pipeline: `site/scripts/build-papers.mjs` (registry-driven
  compile → PDF + in-site HTML fragment), evidence binding via
  `site/papers/_lib/bench.typ` (`headline()` panics on non-canonical evidence;
  `ev()` for indicative callouts), `site/src/data/paper-evidence.json` as the one
  number source.
- Honesty gates, all landed: `.typ` sources scanned by `check-privacy-claims.sh` +
  `check-no-perf-numbers.py --enforce` at the build boundary (sq-mkza), negative-test
  fixtures proving the gates are non-vacuous (sq-ddc0), the shared forbidden-phrase
  list + build-boundary assertion (sq-mraf), and the documented limits of the
  mechanical gate (sq-dxi3: semantic overclaims remain human review).
- Findability: sq-1scgk (open, sibling) puts /papers in the top nav with per-paper
  status badges — the wiring bead feeds it, does not duplicate it.

## 8. Corrected premises, judgment calls, and revisit triggers

- **Corrected premise:** zkSPARQL is not an unwritten paper — it is a live ISWC 2026
  submission (zksparql.org). The child bead is scoped to submission support.
- **Judgment call (proceed-and-document):** the "differential+mechanized assurance"
  candidate was killed as framed and REPLACED with the SPARQL logic-bug testing
  paper — a candidate outside the maintainer's original list (allowed by the brief).
  A steering issue accompanies this record's PR.
- **Judgment call:** the WASM and conformance-breadth candidates fold into P3 rather
  than standing alone; the MPC construction paper defers behind named triggers
  (§3.2) rather than duplicating the SoK.
- Revisit triggers: sq-qhy4 closing (crypto claims can strengthen), sq-sqtk2
  delivering completed mechanized proofs (a real CAV-industrial candidate then),
  canonical runner landing (P3 unblocks), external harness adoption (§3.6 reframe),
  M4+ malicious-secure MPC (§3.2 revives).

---

> Search reports: four parallel agents, 2026-07-04 (zkSPARQL/ZK-numerics; MPC +
> assurance; systems/WASM/resources; venue deadlines). All citations above were
> link-verified by those agents at search time; anything a CFP has not yet published
> is marked "(pattern)".
> <!-- privacy-claims-allow: this record REPORTS the pre-audit status of the ZK/MPC estate and scopes every paper claim as design + implementation, pre-audit (sq-qhy4 open); it asserts no soundness/privacy/attestation property. -->
