# Academic Paper-Factory — Phase-1 Research

<!-- [OPUS-4.8] research seed for epic sq-gum8 (academic paper factory), phase 1. -->

> 🤖 **SPARQ agent** deep-research record. This document seeds the design of an
> auto-generation pipeline that turns a sparq contribution into (a) a live HTML paper
> hosted in the Next.js site and (b) a downloadable, venue-credible PDF, both bound to
> live benchmark data. It covers: the writing methodology to codify, a venue map for
> sparq's contribution types, the recommended auto-gen + live + PDF stack, the
> reproducibility / empirical-honesty handling, a skill recommendation, and a concrete
> proposed phase-3 pipeline architecture.
>
> **Citation discipline.** Every URL below is real and was fetched or web-searched during
> this research run. Items that could not be fetched live (e.g. ACM's site 403s automated
> fetchers, dblp timed out) are corroborated from mirrors and **flagged "verify"**. No
> numbers or citations are fabricated. Re-verify all venue page limits / deadlines /
> blinding policies against the current-year CFP before any submission — those drift yearly.

---

## 0. Scope and the contribution types this factory must serve

sparq is a from-scratch Rust RDF triplestore + SPARQL 1.1 engine (dictionary-encoded, six
sorted permutation indexes, parallel/streaming/mmap out-of-core, RDFS/OWL-RL/N3 inference,
a wasm build, a W3C-conformant HTTP server) plus opt-in capability crates. Its publishable
contributions fall into three families, each of which lands at a different venue tier:

- **A — DB/engine performance**: dictionary encoding, sorted permutation indexes,
  parallel + streaming execution, mmap out-of-core + compressed on-disk format, fast
  ingest (fused decompress+parse), compression. Empirical/systems papers.
- **B — Semantic Web / RDF / SPARQL features**: federation, SHACL, GeoSPARQL, RSP-QL
  streaming, inference, GenAI retrieval over RDF, RDFC-1.0 canonicalization. Resource /
  in-use / research papers.
- **C — Security/privacy (ZK / MPC)**: zero-knowledge proofs of SPARQL-query correctness
  over Verifiable Credentials (Noir/Barretenberg), MPC over federated SPARQL. **Honestly
  NOT-yet-sound / work-in-progress** (a prior soundness audit found the v1 verifier is not
  sound). This shapes the venue choice strongly: WIP → workshop/preprint now, top venue later.

The factory must therefore be **multi-template** (DB systems vs SemWeb resource vs crypto)
and must enforce sparq's **empirical-honesty mandate** (no presenting non-canonical work-box
numbers as canonical) at the data layer, not just in prose.

---

## 1. Academic writing craft — the methodology to codify

Synthesized from the canonical primary sources (Peyton Jones, Widom, Ernst, Freeman, Heiser,
the SIGPLAN Empirical Evaluation Committee, and VLDB/USENIX policy). This section is the
basis for the reusable skill in §5.

### 1.1 Structure (the canonical skeleton)

Simon Peyton Jones's *How to write a great research paper* gives the page budget keyed to how
many readers reach each part: **Title** (1000 readers) → **Abstract** 4 sentences (100) →
**Introduction** 1 page (100) → **The problem** 1 page (10) → **My idea** 2 pages (10) →
**The details** 5 pages (3) → **Related work** 1–2 pages (10) → **Conclusions** 0.5 pages.
([slides](https://www.cis.upenn.edu/~sweirich/icfp-plmw15/slides/peyton-jones.pdf),
[landing](https://simon.peytonjones.org/great-research-paper/))

- **Abstract = 4 sentences**, written last: (1) the problem; (2) why it is interesting;
  (3) what your solution achieves; (4) what follows. (PJ; echoed by Widom.)
- **Introduction ≤ 1 page**, answering in order: what is the problem / why important / why
  hard (why naive approaches fail) / why unsolved before (what distinguishes you) / key
  components + results — ending in a **bulleted contributions list**. The new technical
  contribution must be clear **by the end of page 3**. (Widom, *Tips for Writing Technical
  Papers*, <https://cs.stanford.edu/people/widom/paper-writing.html>; Heiser, *Writing Good
  Systems Papers*, <https://trustworthy.systems/publications/papers/Heiser_12:div.slides.pdf>.)
- **Forward-reference every important part from the intro; delete "the rest of this paper is
  organized as follows"** — the contributions list does that job. (PJ.)
- **Give away the punchline early; top-down; a mini-intro per section; maintain reader
  state.** (Ernst, *How to write a technical paper*,
  <https://homes.cs.washington.edu/~mernst/advice/write-technical-paper.html>; Heiser; Widom.)
- **Real examples, never `foo`/`bar`; active voice; simple direct language; strong visual
  structure (sections, bullets, laid-out code, figures); self-contained figure captions that
  tell the reader what to notice** (reviewers skim — Freeman, *How to write a good paper*,
  <https://deviparikh.com/citizenofcvpr/static/slides/freeman_how_to_write_papers.pdf>).
- **Conclusion** summarizes with concrete numbers; **avoid a wishlist "future work"**
  section ("you get no partial credit for neat things you wanted to do but didn't" — Freeman).

### 1.2 Framing a novel contribution — "what's new + why it matters + evidence"

- **Write the contributions list first; it drives the whole paper** ("the paper substantiates
  the claims you have made"). (PJ.)
- Each contribution must be **refutable**: it names *what* is achieved (and ideally *how*),
  is specific enough to be disproved, and **forward-references the section that delivers the
  evidence**. PJ's contrast: NO = "We describe the WizWoz system. It is really cool."; YES =
  "We give the syntax and semantics of a language that supports concurrent processes
  (Section 3) … We prove the type system sound and type-checking decidable (Section 4) … the
  result is half the length of the Java version (Section 5)."
- The **triad** maps cleanly: the bullet states *what's new* (refutably) + gestures at *why it
  matters* (the intro's "why interesting/important") + carries a forward reference to the
  *evidence*. Ernst's version: convince the reader that (1) the problem is important, (2) it is
  hard, (3) you solved it — and explain novelty *contextually* ("why others rejected/didn't try
  this"), not just "this is new."
- **Claims ↔ evidence loop**: check each intro claim, identify its evidence (analysis /
  theorem / measurement / case study), forward-reference it. (PJ.)

### 1.3 Positioning against related work without strawmanning

- **"Credit is not like money"** — giving credit does not diminish yours. Be generous; refute
  the fallacy that you must make others look bad. Failing to credit can *kill* the paper (a
  referee who knows the idea isn't yours concludes you either didn't know or are pretending).
  (PJ.)
- **Compare and contrast, don't list**; label any inferior/strawman approach **explicitly and
  up front** ("a reader assumes you believe whatever you write unless clearly marked"). (Ernst.)
- Useful reframing from the empirical-SE primer (<https://arxiv.org/pdf/2506.11002>): write
  related work "as if telling the cited authors why they should care about your work" — "there
  is no need to show prior work is all wrong"; doing so puts reviewers (often those authors)
  in a combative frame.
- **Placement**: defer related work to near the end (early comparison is incomprehensible
  before the reader understands the problem); place it early *only* if it is concise and you
  need an immediate defensive stance. (PJ; Widom nuance.)

### 1.4 Reviewer-rejection pitfalls (what to pre-empt)

From Heiser's reviewer's-eye list and the **SIGPLAN Empirical Evaluation Checklist** (Berger,
Blackburn, Hauswirth, Hicks — <https://www.sigplan.org/Resources/EmpiricalEvaluation/>,
[checklist PDF](https://github.com/SIGPLAN/empirical-evaluation/raw/master/checklist/checklist.pdf),
[manifesto](https://blog.sigplan.org/2019/08/28/a-checklist-manifesto-for-empirical-evaluation-a-preemptive-strike-against-a-replication-crisis-in-computer-science/)):

- **Overclaiming / implied generality** — "works for all Java" when only a subset; "on real
  hardware" when only simulated; "automatic" when supervised. *Misleading summary*: never
  summarize speedups of 4/6/7/49% as "up to 49%."
- **Weak/unfair baselines** — straw-man baseline misrepresented as SOTA; unfair config
  (baseline at `-O0`, you at `-O3`); comparing only to your own prior version or an outdated
  SOTA. Heiser escalates: using a competitor's sub-optimal config "probably constitutes
  scientific misconduct."
- **Missing ablations / sensitivity** — key design parameters must be explored over a range.
- **No error bars / no statistical rigor** — report variability (variance / std-dev /
  quantiles) and/or confidence intervals; enough trials for non-deterministic systems.
- **Irreproducibility** — insufficient info to repeat (all parameters incl. defaults, all
  software versions, full hardware). Heiser: "relative numbers only" (ratios with no
  absolutes) prevents sanity-checking.
- **Related-work gaps / "delta too small"** — the antidote is the refutable, evidence-backed
  contribution list, not louder claims.
- Operational reality (Freeman): chairs → area chairs → 3–5 reviewers who skim; "~1/3 are
  obvious rejects"; "the most dangerous mistake is assuming the reviewer will understand your
  point"; "you get no partial credit."

### 1.5 Honest-evaluation norms (the empirical-honesty mandate, restated as a rubric)

The SIGPLAN seven categories are the most directly codifiable artifact (a red/yellow/green
rubric, *not* a binary gate — "the checklist supports judgment, not supplants it"):

1. **Clearly stated claims** — explicit + appropriately scoped (no implied generality).
2. **Suitable comparison** — appropriate, fairly-configured baseline.
3. **Principled benchmark choice** — established suites in-context; justify subsets; no
   cherry-picking; don't test on the training set.
4. **Adequate data analysis** — enough trials; correct summary statistic (geometric mean for
   differing ranges, harmonic for rates, median under outliers); **report variability / CIs**.
5. **Relevant metrics** — measure *all* important effects (e.g. compile/index time alongside
   runtime), justify any proxy.
6. **Appropriate & clear experimental design** — enough detail to repeat; representative
   platform; explore key parameters.
7. **Appropriate presentation** — zero-based axes; log/normalized ratios; right precision;
   a summary reflecting the *full distribution*.

On the honesty norm itself: Freeman — "Be honest, scrupulously honest… don't succumb to the
perceived pressure to over-sell, hide drawbacks, and disparage others' work"; he ties this to
credibility (a best paper awarded partly because the results could be *trusted*). Ernst —
"admit errors forthrightly; submit only if you are proud to attach your name in its current
form." Heiser — "go out of your way to be fair; anticipate any scepticism; think of ways your
approach could fail."

**Mapping to sparq's non-canonical numbers** (synthesis of SIGPLAN cats 1+6 + Heiser + the
project mandate): report the platform fully; report variability; scope the claim ("on this EC2
instance" is part of the claim, not a footnote); report absolutes not ratios-only; don't
cherry-pick the favourable run. See §4 for the operationalization.

---

## 2. Venue map for sparq's contribution types

> All page limits / deadlines / blinding policies **drift yearly — verify against the current
> CFP before submitting.** ACM/IEEE/USENIX CFP pages 403 automated fetchers, so format
> specifics below are from search summaries + established norms unless a URL was fetched.

### 2.1 Jesse Wright (jeswr) — maintainer precedent

DPhil researcher, **University of Oxford** (CS Human-Centred Computing / EWADA project,
supervisors **Nigel Shadbolt** & **Jun Zhao**); **Solid Lead at the Open Data Institute**;
active **W3C** contributor; prior ANU University-Medal honours thesis on decentralised-web
reasoning; research on decentralised agents, Solid, and logic/reasoning on the Web (Notation3,
RDF Surfaces). Sites: <https://www.jeswr.org/>, <https://blog.jeswr.org/>,
<https://www.cs.ox.ac.uk/people/jesse.wright/>, <https://github.com/jeswr>,
<https://scholar.google.com/citations?user=J_HhOU8AAAAJ>.

**Venue precedent — tightly clustered in the Semantic Web community:**
- **ISWC is the home venue**: ISWC 2020 full LNCS paper (*Schímatos*,
  <https://link.springer.com/chapter/10.1007/978-3-030-62466-8_5>); **three** ISWC 2024
  Posters/Demos/Industry papers (N3.js Reasoner, "Here's Charlie!"
  <https://arxiv.org/abs/2409.04465>, EYE JS — <https://ewada.ox.ac.uk/news/2024/08/26/iswc.html>).
- **ESWC / Springer "The Semantic Web" LNCS**: a **ZK-SPARQL soundness chapter** (Braun, Käfer,
  Wright — DOI <https://link.springer.com/chapter/10.1007/978-3-032-25156-5_16>; exact venue/year
  verify). **This is directly the sparq ZK lineage.**
- **SEMANTiCS** workshop (NXDG 2024, <https://ceur-ws.org/Vol-3891/paper4.pdf>); **ISWC 2025
  Doctoral Consortium** (<https://ceur-ws.org/Vol-4085/paper19.pdf> — the RQ1/RQ2 framing the
  repo's own zkp/mpc research docs cite); arXiv preprints (e.g. RDF Surfaces
  <https://arxiv.org/pdf/2406.10659>); FOSDEM for the VC/wallets standards audience.
- **zkSPARQL** (<https://zksparql.org/>) is **submitted to ISWC 2026** — directly the sparq lineage.
- **Style:** exploratory, "thinking in public" / "ideas, not papers", implementation-first,
  hybrid engineer-researcher — open-source-tool-centric papers where *the tool is the
  contribution*. Maps **exactly** to sparq (an engine as the artifact) and the ISWC/ESWC
  **Resources** track.
- dblp <https://dblp.org/pid/189/1514.html> (timed out on fetch this run — verify the full list).

**Implication:** sparq's natural centre of gravity is **ISWC/ESWC** (Resources track for the
engine; Research track for specific algorithms) — consistent with the maintainer's entire
precedent. The DB-systems contributions are strong enough for **PVLDB** if a database-community
footprint is wanted (PVLDB's rolling single-blind model fits his ship-when-ready style). The
ZK/MPC work belongs at **non-archival WIP venues now** (HotPETs / ZKProof Workshop / TPMPC) or
an **SoK** (note: **CCS does not accept SoK**), graduating to USENIX/PoPETs once sound.
Co-authorship/supervision likely routes through Oxford (Shadbolt/Zhao).

### 2.2 Contribution-type → venue table

**A — DB/engine performance**

| Venue | Fit | Format / bar (verify) | Notes |
|---|---|---|---|
| **PVLDB / VLDB** | STRONG | ~12 pp + refs; **rolling monthly** submission; double-blind; reproducibility track (EA&B papers *must* submit for repro) | Premier storage/index/query-execution venue. Rolling deadlines suit an iterative engine. <https://www.vldb.org/2027/submission-guidelines.html> |
| **SIGMOD** | STRONG | ~12 pp ACM 2-col; double-blind; multiple rounds/yr; ARI badging | Top-tier; systems-y storage/exec fits; R1/R2 revision cycle. <https://reproducibility.sigmod.org/> |
| **ICDE** | GOOD | IEEE ~12 pp; abstract+full mid-year; artifact track | More applied/systems-friendly; good fallback. |
| **EDBT** | GOOD (best first-paper target) | ~12 pp; annual; lower bar; reproducibility track | Most accessible serious DB venue; good for a focused indexing/ingest paper. <https://www.edbt.org/> |

**B — Semantic Web / RDF / SPARQL features**

| Venue | Fit | Format / bar (verify) | Notes |
|---|---|---|---|
| **ISWC** | STRONG (home) | LNCS ~12–15 pp; Research / **Resource** / In-Use + Poster/Demo + workshops | THE semantic-web venue. **Resource track is purpose-built for a reusable engine/tool** like sparq. Annual (autumn). <https://iswc.semanticweb.org/> |
| **ESWC** | STRONG | LNCS ~15 pp; Research/Resource/In-Use + posters/demos/workshops | European sibling; same Resource-track fit; good first-paper venue. <https://www.eswc-conferences.org/> |
| **TheWebConf (WWW)** | GOOD (selective) | ACM ~10–12 pp double-blind; "Semantics & Knowledge" track | Higher bar, broader audience; for a high-impact SPARQL/federation result. |
| **SEMANTICS** | GOOD (applied/industry) | mixed academic + industry | Lower research bar; strong EU industry audience; good for in-use. |
| **Journal of Web Semantics (JWS)** | GOOD (journal) | Elsevier, no page limit, long-form | For a mature, consolidated/extended engine description. Archival but slow. |

**C — Security/privacy (ZK / MPC), currently NOT-yet-sound**

| Venue | Fit | Format / bar (verify) | Notes |
|---|---|---|---|
| **PETS / PoPETs** | **BEST "real" target (once sound)** | PoPETs is a **journal with 4 rolling deadlines/yr**; privacy focus; artifact badging | Natural privacy venue for ZK/MPC-over-SPARQL; rolling deadlines + privacy scope. <https://petsymposium.org/> |
| **IEEE S&P (Oakland)** / **USENIX Security** / **ACM CCS** | ASPIRATIONAL (not yet) | top-tier, need soundness proofs + evaluated system | Only once the ZK system is sound. USENIX has strong AE culture (good later). |
| **arXiv + workshops** | **BEST FIT NOW** | preprint / WIP tracks | The honest venue for not-yet-sound crypto **today** (see §2.3). |

### 2.3 Honest WIP track for the crypto work (now-appropriate)

Given the v1 verifier is **not sound**, the honest move today is preprint + workshop, not a
top-venue full paper:

- **arXiv preprint** — timestamp the design, invite scrutiny, **state the soundness gap
  explicitly** and mark WIP. (Maintainer already uses arXiv.)
- **ISWC/ESWC workshops** — privacy / decentralization / policy-and-trust workshops co-located
  with the semantic-web venues fit a ZK-SPARQL design paper.
- **Security workshops** — e.g. WPES (co-located with CCS), DPM/CBT (co-located with ESORICS),
  ZKProof community standards events for the ZK-specific design. (Verify each runs this year.)
- **Poster/demo tracks** at ISWC/ESWC — show the working (if unsound) prototype honestly.

### 2.4 Submission cadence, blinding & arXiv interaction (verify current-year)

- **PVLDB** — complete paper on the **1st of each month** (no abstract), **single-blind**, one
  revision (~2.5 mo); the **EA&B (Experiment/Analysis/Benchmark) track legitimizes
  measurement-heavy papers** (12 pp). Best "submit when ready."
- **SIGMOD** — fixed rounds, paper deadlines **~17 Jan / 17 Apr / 17 Jul / 17 Oct**,
  **double-blind** (anonymize the repo too).
- **ISWC 2026** (Bari, 25–29 Oct): abstract ~2 May / paper ~7 May 2026, double-blind + rebuttal,
  mandatory Supplemental Material Statement. **ESWC** is double-blind with a **strict
  no-preprint window (~1 month pre-deadline → notification)** — this **conflicts with
  arXiv-first** for ESWC; ESWC 2026 deadline has passed, so **ESWC 2027 is the live target**.
- **Security cycles** (next live): USENIX Sec '26 Cycle-2 paper ~5 Feb 2026; CCS 2026 Cycle A
  ~14 Jan / Cycle B ~29 Apr; **PoPETs 2027** quarterly (~31 May / 31 Aug / 30 Nov 2026 /
  28 Feb 2027); IEEE S&P rolling cycles. **IEEE S&P and USENIX explicitly allow preprints
  during review; SIGMOD double-blind discourages non-anonymized preprints** in the review
  window — so the crypto papers can arXiv-first freely, the SIGMOD engine paper cannot.
- **Gap / opportunity:** there is **no recurring crypto-on-RDF workshop** at ISWC/ESWC — a
  genuine gap and a possible workshop-proposal opportunity for the ZK-SPARQL line.

### 2.5 Cross-cutting submission facts

- **Artifact evaluation is near-universal** at the strong venues (§3). sparq's open-source,
  multi-binding, conformance-suite-backed nature is a major asset — lead with it.
- **Double-blind** is standard at SIGMOD/VLDB/ICDE/WWW/CCS/S&P → the factory **must support an
  anonymized build** (strip sparq-org/sparq identity, anonymize the artifact link). ISWC/ESWC
  blinding varies by track/year.
- **Resource track (ISWC/ESWC)** is the single best structural fit for "sparq the engine."
- **Rolling cadence** (PVLDB monthly, PoPETs quarterly, SIGMOD multi-round) → the factory
  should target a rolling submission cadence, not one annual crunch.

---

## 3. Reproducibility & artifact evaluation

### 3.1 ACM Artifact Review and Badging v1.1

Canonical policy: <https://www.acm.org/publications/policies/artifact-review-and-badging-current>
(v1.1, effective 24 Aug 2020; the ACM site 403s automated fetchers — wording corroborated from
venue mirrors such as <https://sysartifacts.github.io/sosp2021/badges> and
<https://sigir.org/general-information/acm-sigir-artifact-badging/>). Three independent badge
families:

- **Artifacts Available** — author artifacts on a *publicly accessible archival* repo with a
  **DOI**. A bare GitHub URL is **not** sufficient (GitHub is mutable, not archival) — use a
  Zenodo/figshare DOI snapshot (Zenodo integrates with GitHub for this).
- **Artifacts Evaluated — Functional** — documented, consistent, complete, exercisable (it
  runs from the documented steps).
- **Artifacts Evaluated — Reusable** — a strict superset of Functional: very carefully
  documented and well-structured "to the extent that reuse and repurposing are facilitated."
- **Results Reproduced** = different team, **using** author artifacts. **Results Replicated** =
  different team, **without** author artifacts. (v1.1 re-aligned this with NISO/National
  Academies usage and **inverted** the older ACM-2016 names — always cite the v1.1 date.)

### 3.2 How venues run AE (and what reviewers check)

- **USENIX (OSDI/Security/FAST/…)** — three ACM-mirroring badges; optional, post-acceptance;
  a "kick-the-tires" phase then full eval; criteria independent ("an artifact need not be
  available or functional to earn Results Reproduced"). USENIX Security '26 makes acceptance
  *conditional on artifact-availability verification*. (<https://www.usenix.org/conference/osdi25/call-for-artifacts>,
  <https://www.usenix.org/conference/usenixsecurity26/call-for-artifacts>,
  <https://www.usenix.org/conference/usenixsecurity22/artifact-appendix-guidelines>,
  guide <https://secartifacts.github.io/usenixsec2023/guide>.)
- **SIGMOD ARI** — two-phase (single-anonymous availability → zero-anonymous reproducibility by
  an independent group). The reviewer Quick Guide
  (<https://reproducibility.sigmod.org/documents/Guidelines-Reviewers.pdf>) is directly relevant:
  "take a look at the **hardware and software required**"; "eliminate any interference … machines
  used are **dedicated**"; results reproduced when behaviour "matches the **conclusions**" (not
  bit-identical numbers).
- **PVLDB** — Availability via `\vldbavailabilityurl{}`; separate Reproducibility track; **EA&B
  papers required** to submit. Expected: prototype + data(-gen) + instrumentation +
  raw→figure scripts. (<https://www.vldb.org/2027/submission-guidelines.html>.)
- **EuroSys** — three ACM badges; authors pick a combo, including **Functional+Reproduced
  (private hardware)** for artifacts that *cannot* be made public — the escape hatch when an
  artifact needs hardware reviewers can't access. (<https://sysartifacts.github.io/eurosys2026/badges>;
  5-yr lessons <https://www.sigops.org/2025/lessons-from-five-years-of-artifact-evaluation-at-eurosys/>.)

### 3.3 The Artifact Appendix

De-facto cross-community template: **ctuning/artifact-evaluation** "Artifact Appendix &
Reproducibility Checklist" (<https://github.com/ctuning/artifact-evaluation/blob/master/docs/checklist.md>).
Sections: Abstract; **Artifact check-list (meta-information)** — Algorithm / Program /
Compilation / Data set / Run-time environment / **Hardware** / Run-time state / Execution /
Metrics / Output / Experiments / disk / time-to-prepare / time-to-run / publicly-available? /
licenses / **Archived (DOI)**; Description (how-to-access, **HW deps**, **SW deps**, datasets,
models); Installation; Experiment workflow; **Evaluation and expected results** (map each
experiment to a claim/figure + how to compare to the paper's numbers); Customization;
Reusability; Notes. USENIX's appendix guidelines require the same buckets and explicit
approximate runtime. Filled example:
<https://www.usenix.org/system/files/usenixsecurity25-appendix-hao.pdf>.

### 3.4 Pinning HW/SW/data — the norm, and its hard limit

Grounded in Gernot Heiser's **Systems Benchmarking Crimes**
(<https://gernot-heiser.org/benchmarking-crimes.html>; academic version van der Kouwe et al.,
<https://arxiv.org/abs/1801.02381>), Kalibera & Jones's **Rigorous Benchmarking in Reasonable
Time** (<https://kar.kent.ac.uk/33611/45/p63-kaliber.pdf>), and the PVLDB reproducibility
tutorial (<https://www.vldb.org/pvldb/vol17/p4221-hirn.pdf>, template
<https://github.com/db-reproducibility/template>):

- **Hardware** — CPU model + microarchitecture, core/thread count, base/boost clock (and
  whether turbo/SMT/frequency-scaling were pinned/disabled), RAM size/speed, **all cache levels
  + associativity** (for memory-system benchmarks), NUMA topology, storage (NVMe model, fs),
  any accelerator. Heiser names "missing specification of evaluation platform" as a crime.
- **Software** — OS distro + **kernel version (release number)**, compiler + exact version +
  flags (`rustc` + `Cargo.lock`, `-C target-cpu`, opt-level), library + baseline-system
  versions. Capture via **Docker (pinned by `@sha256:` digest, not a floating tag) or a Nix
  flake** for bit-reproducible builds.
- **Data** — dataset name + version + exact size + **content hash (SHA-256)**; for synthetic
  data, the generator + **fixed seed**; provenance/URL.
- **Statistics** — fixed seeds; enough repetitions; report **central tendency + dispersion
  (std-dev / CIs / min–max)**, never a bare single number; geometric mean for normalized
  ratios; discard warm-up; state warm vs cold cache.
- **Hard limit (load-bearing for sparq):** the PVLDB tutorial states verbatim that *"Docker
  does not simplify hardware requirements. If an experiment requires a specific hardware setup,
  it must still be provided by the user."* Containers fix *software* drift; they do **not** make
  a non-canonical CPU/kernel produce canonical numbers.

### 3.5 sparq's non-canonical (EC2 work-box) handling — the recommendation

sparq's benchmarks run on an AWS EC2 work-box (`-aws` kernel, shared/virtualized host) and are
explicitly **non-canonical**. Presenting those numbers as results would commit two named
crimes ("missing platform spec", and without variance, "no indication of significance"), and
SIGMOD reviewers are told to use **dedicated, interference-free** machines. Recommended
operationalization (this is the bridge from the project's existing memory rule to an *enforced*
artifact invariant):

1. **Define ONE canonical runner environment, pinned to crime-free precision** (CPU model +
   cores + clock/turbo policy + cache levels, RAM, OS **release + kernel**, `rustc` +
   `Cargo.lock`, dataset version + SHA-256, seeds). Prefer a **bare-metal / dedicated** box (or
   a fixed bare-metal cloud instance type) so the kernel is a real distro kernel, not `-aws`.
   **Every published number and every comparative claim comes only from this runner**, repeated
   per Kalibera–Jones, reported with std-dev / CIs.
2. **Capture it as a DOI-archived artifact** — digest-pinned Docker image (or Nix flake) +
   `Cargo.lock` + dataset hash + seeds + a one-command runner emitting CSV→figures, on Zenodo
   (earns Available; enables Functional/Reproduced). State explicitly that the container
   reproduces *software*, but canonical numbers need the specified hardware (use EuroSys's
   private-HW badge combo if the canonical box can't be shared).
3. **Two-tier prose convention.** *Canonical results* → headline tables/figures, with variance.
   *Indicative (non-canonical) measurements* → EC2/dev-box numbers live only in clearly-labelled
   "indicative development measurement" callouts that (i) name the actual instance type +
   `-aws` kernel, (ii) state they are not the basis of any claim, (iii) are **never co-tabulated
   with canonical numbers and never used for an aggregate figure-of-merit** (avoiding selective
   benchmarking). Never quote a speedup that blends the two.
4. **Make the split machine-checkable.** A result record must carry an
   `environment: canonical | indicative` field plus the full pinned fingerprint
   (CPU/kernel/compiler/dataset-hash/seed/n-reps/dispersion); a CI gate **refuses to let an
   `indicative` number be cited in any paper-bound table or claims JSON**. This turns sparq's
   memory rule ("EC2 measurements are NON-canonical, gate only deterministic metrics") into an
   enforced pipeline invariant. (See §6 for where this gate sits.)

---

## 4. Auto-generation + live-document + PDF stack (the key technical decision)

**Need.** Each paper must (a) live as an HTML page **inside the existing Next.js 15 + React 19
static-export site**, (b) **auto-update from live benchmark data** (`site/src/data/
benchmarks.generated.json`, produced by `site/scripts/sync-benchmarks.mjs` from the
`benchmark-data` branch — confirmed present in-repo, and `next build` already runs the sync via
`prebuild`), and (c) offer a **downloadable, credible academic PDF** — ideally **single-source**.

### 4.1 Recommendation: Typst as the single source

**Author each paper in `.typ`; inject the live benchmark JSON at build time via `--input` /
`sys.inputs`; emit two artifacts from one source** — a credible PDF (mature Typst PDF export,
PDF/A + PDF/UA as of 0.15) and an in-site HTML page (via **typst.ts** rendering, the robust
choice today; migrate to Typst's native HTML export once it leaves "experimental").

Why Typst wins (verified June 2026):

- **Typst 0.15 (released 2026-06-15, <https://typst.app/blog/2026/typst-0.15>)**: HTML export
  renders equations via MathML (screen-reader-friendly), adds multi-file bundle export, and
  **PDF export now supports PDF/A + PDF/UA simultaneously**.
- **Data-driven generation is best-in-class.** Native `json()`/`csv()`/`yaml()`/`toml()`/
  `read()` (<https://typst.app/docs/reference/data-loading/json/>), and **CLI injection via
  `sys.inputs`**: the official "Automated PDF Generation" blog
  (<https://typst.app/blog/2025/automated-generation/>) documents
  `typst compile --input customer="$(cat mike.json)" main.typ` + in-template
  `#let customer = json(bytes(sys.inputs.customer))`, notes "*compilations commonly complete in
  milliseconds*", and shows Docker + Rust-library embedding for CI/batch. This is the cleanest
  live-data binding of any tool reviewed.
- **typst.ts / reflexo-typst** is the web-native bridge — "Run Typst in JavaScriptWorld",
  **actively maintained (v0.7.0, 2026-06-01)**, with `@myriaddreamin/typst.react`; proven in a
  Next.js app (<https://github.com/Myriad-Dreamin/typst.ts>,
  <https://github.com/Mapaor/typst-online-editor>). Renders the same `.typ` to inline SVG/canvas
  in a React route at full visual fidelity.
- **Templates** for credible look: `charged-ieee` (Typst GmbH, IEEE 2-col,
  <https://typst.app/universe/package/charged-ieee/>), `arkheion` (arXiv-style,
  <https://typst.app/universe/package/arkheion/>), `para-lipics`
  (<https://typst.app/universe/package/para-lipics/>), ACM-style + ML templates
  (<https://github.com/daskol/typst-templates>).

**Two known risks (flagged):**
- *Typst HTML export is still experimental* — the official reference says verbatim "Do not use
  this feature for production use cases" and "Typst currently does not output CSS style sheets"
  (<https://typst.app/docs/reference/html/>); positioned content must be wrapped in
  `html.frame(...)` (inline SVG, with known rough edges — issues
  <https://github.com/typst/typst/issues/6406>, <https://github.com/typst/typst/issues/7114>).
  **Mitigation: use typst.ts for the live page today**; switch to native HTML export when it
  matures.
- *Publisher camera-ready:* `para-lipics` notes accepted LIPIcs papers must be **converted
  Typst→LaTeX** for the publisher; some ACM/IEEE production pipelines may also require LaTeX.
  For arXiv / preprints / our own site this is a non-issue — only the final venue upload may
  need a LaTeX export step.
- *Uncertain:* whether typst.ts v0.7.0's pinned upstream compiler includes 0.15-specific
  features — verify before relying on them.

### 4.2 Fallback: Pandoc single-source Markdown → LaTeX/PDF + HTML

If **publisher camera-ready compliance dominates**, author in Markdown and route the PDF
through the *official* LaTeX venue templates: **Pandoc** + `pandoc-crossref` (figure/table/eq
numbering, <https://github.com/lierdakil/pandoc-crossref>) + **citeproc/CSL** (IEEE/ACM styles
from a `.bib`), PDF via **Tectonic** (<https://tectonic-typesetting.github.io/en-US/>, CI action
<https://github.com/WtfJoke/setup-tectonic>) using `acmart`/`IEEEtran`/`lipics`. Data injection:
a small codegen step emits a `bench.md`/`bench.tex` fragment from the same JSON. True
single-source HTML+PDF with real citations, reusing the gold-standard LaTeX templates — at the
cost of more toolchain glue and weaker typographic control than Typst.

### 4.3 The other options (why not primary)

- **LaTeX (acmart/IEEEtran/lipics) direct** — unbeatable for *official camera-ready* and the
  venue gold standard, but **no native data import** (must codegen `.tex` macros) and **no good
  single-source HTML path** (LaTeXML/make4ht are lossy). Keep it as an **export target** for
  venue submission, not the day-to-day source.
- **HTML-native (React + print CSS) → PDF** — best web-native authoring and React data-binding,
  but to reach credible academic PDF you either pay for **PrinceXML** (best paged-media —
  footnotes/running-heads/cross-refs — but **commercial; free build watermarks output**;
  <https://www.princexml.com/purchase/license_faq/>) or accept **Paged.js (MIT,
  <https://pagedjs.org/>) + headless Chrome/Playwright** (good-not-great paged media; Chromium
  has the *weakest* footnote/running-header engine despite best CSS/JS).
  `@react-pdf/renderer` is an **anti-pattern** here — it forces a *second* document tree, so
  numbers/layout are authored twice (breaks single-source). HTML→PDF engine comparison drawn
  from <https://ironsoftware.com/suite/blog/comparison/html-to-pdf-2026-guide/>,
  <https://print-css.rocks/tools>.

### 4.4 Data-binding — one source of truth → numbers in both outputs

The repo already produces `site/src/data/benchmarks.generated.json` via
`site/scripts/sync-benchmarks.mjs`. Pattern (one JSON feeds both artifacts, zero duplicated
numbers):

- **HTML page**: the React route imports the JSON directly (numbers/tables/figures bind as
  React props) — same data the site's existing benchmark UI uses.
- **PDF**: pass the **same JSON** into the Typst compile —
  `typst compile paper.typ public/papers/<slug>.pdf --input data="$(cat site/src/data/benchmarks.generated.json)"`,
  then in `paper.typ`: `#let bench = json(bytes(sys.inputs.data))` and reference
  `#bench....` — run as a build step within/before `next build`; drop the PDF in
  `public/papers/` so the static export serves it (<https://nextjs.org/docs/app/guides/static-exports>).
- **Anti-pattern:** authoring the PDF with `@react-pdf/renderer` (duplicates the numbers).

---

## 5. Skills discovery — install vs author

**The maintainer asked.** There is a clear precedent in-repo: a `logo-designer` skill is
already installed (`skills/`-style usage skills follow the [agentskills.io] open format —
`name`/`description` frontmatter + Markdown — confirmed by `skills/SKILL.md` + the
`MAINTENANCE RULE` in `AGENTS.md`).

**Existing academic-writing skills surveyed** (real, via web search):

- **Imbad0202/academic-research-skills** (<https://github.com/Imbad0202/academic-research-skills>)
  — a 10-stage research→write→review→revise→finalize pipeline (13/12/7/10-agent skills),
  `.claude` plugin + symlink install, anti-sycophancy/anti-AI-failure-mode checklists,
  `top_journals_by_field.md`, optional `repro_lock`, PDF via **tectonic + LaTeX (APA class)** or
  Pandoc. **Blocker: licensed CC-BY-NC 4.0 (non-commercial)** — incompatible with a permissively
  licensed engine repo.
- **K-Dense-AI/claude-scientific-writer** (<https://github.com/K-Dense-AI/claude-scientific-writer>)
  — IMRaD, ScholarEval 8-dimension review, citation verification, PDF via LaTeX; **MIT**, but
  **life-sciences-leaning** (Nature/Science/clinical examples); no DB/SemWeb/crypto venue map,
  no non-canonical-benchmark handling, no live-data/PDF-in-a-Next.js-site pipeline.
- Others (andrehuang/academic-writing-agents, vishalsachdev/paper-writing, Orchestra-Research,
  delibae/claude-prism) — general-science / ML-paper-leaning, plugin-packaged.

**Verdict: AUTHOR a reusable in-repo skill** (do not install an existing one). Rationale:
1. **License**: the strongest pipeline (Imbad0202) is **CC-BY-NC** — unusable in this repo.
2. **Field fit**: none capture sparq's specific **venue map** (DB + SemWeb-Resource + crypto-WIP),
   the **ISWC/Resource-track + Jesse-Wright precedent**, or the security-WIP "preprint/workshop
   now, top venue later" honesty.
3. **The two load-bearing sparq specifics are absent everywhere**: (a) the **empirical-honesty /
   non-canonical-benchmark handling** (canonical-runner pinning + `environment` flag + CI gate,
   §3.5), and (b) the **live-data-bound Typst→{HTML-in-Next.js, PDF}** pipeline (§4). These ARE
   the contribution of the factory.
4. **Repo convention**: usage methodology lives in `skills/<surface>/SKILL.md` (per `AGENTS.md`),
   so an in-repo skill is the idiomatic home and stays version-controlled with the pipeline.

**Recommended skill:** `skills/academic-paper/SKILL.md` (dir name == `name` frontmatter),
capturing the §1 writing methodology checklist, the §2 venue map + Wright precedent, the §3
reproducibility/honesty handling, and the §4 stack + data-binding recipe — plus
`references/` (the SIGPLAN-7 rubric, the artifact-appendix template, venue table) and
`scripts/` (the Typst compile + JSON-inject invocation, the anonymized-build toggle, the
`environment`-flag CI gate). It should be **dual-purpose**: a `skills/` usage skill *and*
installable as a Claude Code skill, so the factory is repeatable by any agent. (Optionally
*borrow ideas* — anti-sycophancy checklist, stage orchestration — from the MIT-licensed
K-Dense skill, with attribution, but write our own.)

---

## 6. Proposed phase-3 pipeline architecture (contribution → live paper + PDF, data-bound, repeatable)

A contribution flows through these stages. Cheap glue runs in the orchestrator; intensive
stages (drafting, review, benchmark runs) are delegated to subagents per the project's
delegation discipline.

```text
                       sparq contribution (A: DB perf | B: SemWeb | C: crypto-WIP)
                                          │
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 1 — CLASSIFY & TARGET                                                      │
   │  • map contribution → venue (§2 table) + track (research/resource/workshop)     │
   │  • pick template: charged-ieee / arkheion (arXiv) / acmart-style / lipics       │
   │  • crypto-WIP ⇒ arXiv/workshop + soundness-gap disclaimer (honesty gate)         │
   └──────────────────────────────────────┬───────────────────────────────────────┘
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 2 — CANONICAL BENCHMARK CAPTURE (the honesty boundary, §3.5)              │
   │  • run the eval on the pinned CANONICAL runner (bare-metal, frequency-pinned)   │
   │  • emit result records: {value, unit, n_reps, dispersion(CI/stddev),            │
   │       environment: canonical|indicative, cpu, kernel, rustc, dataset_sha256,    │
   │       seed, commit}  →  benchmark-data branch  →  benchmarks.generated.json     │
   │  • CI GATE: any record with environment=indicative is BLOCKED from paper tables │
   └──────────────────────────────────────┬───────────────────────────────────────┘
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 3 — DRAFT (single-source .typ, methodology of §1)                         │
   │  • contributions list FIRST (refutable, forward-referenced)                     │
   │  • 4-sentence abstract; ≤1pp intro; related-work-late & charitable              │
   │  • eval section references #bench.<key> (NOT hard-coded numbers)                 │
   │  • run SIGPLAN-7 + benchmarking-crimes self-check; state limitations            │
   └──────────────────────────────────────┬───────────────────────────────────────┘
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 4 — BUILD (one source → two artifacts, data-bound)                        │
   │  PDF : typst compile paper.typ public/papers/<slug>.pdf \                       │
   │           --input data="$(cat site/src/data/benchmarks.generated.json)"         │
   │  HTML: typst.ts (@myriaddreamin/typst.react) renders the SAME .typ into a       │
   │        Next.js route /papers/<slug>, JSON bound as React props                  │
   │  • anonymized-build toggle (strip sparq-org/sparq identity) for double-blind venues │
   │  • artifact: Zenodo DOI snapshot + digest-pinned Docker/Nix + ctuning appendix  │
   └──────────────────────────────────────┬───────────────────────────────────────┘
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 5 — REVIEW & GATE (subagent reviewers, §1.4/§1.5 rubric)                  │
   │  • section reviewers + cross-cutting honesty/repro check; resolve all findings  │
   │  • final: claims↔evidence loop closed; no indicative number in a claim          │
   └──────────────────────────────────────┬───────────────────────────────────────┘
   ┌──────────────────────────────────────┴───────────────────────────────────────┐
   │ STAGE 6 — PUBLISH & AUTO-UPDATE                                                 │
   │  • merge → next build serves /papers/<slug> + /papers/<slug>.pdf (static)       │
   │  • benchmarks.generated.json refresh ⇒ paper numbers AUTO-UPDATE on rebuild     │
   │    (every paper carries a provenance stamp: commit + runner + dataset hash)     │
   │  • venue camera-ready ⇒ optional Typst→LaTeX export step                        │
   └─────────────────────────────────────────────────────────────────────────────┘
```

**Key properties this architecture guarantees:**
- **Single source of truth for numbers** — the eval section binds to
  `benchmarks.generated.json`; HTML and PDF cannot disagree, and the paper auto-updates as
  benchmarks improve (Stage 6).
- **Empirical honesty is enforced, not hoped-for** — the `environment` flag + CI gate (Stage 2)
  makes it *impossible* to cite a non-canonical work-box number in a paper claim.
- **Repeatable** — the `skills/academic-paper` skill (§5) encodes Stages 1/3/5 methodology;
  Stages 2/4/6 are scripted; any agent can run the factory.
- **Multi-venue** — Stage 1's template + track selection + the anonymized-build toggle cover
  DB systems (PVLDB/SIGMOD/EDBT), SemWeb resource (ISWC/ESWC), and crypto-WIP (arXiv/workshop).

---

## 7. Key uncertainties (re-verify before acting)

- **Venue format specifics** — page limits, deadlines, double-blind policies per venue/year
  (ACM/IEEE/USENIX CFP pages 403 automated fetchers; figures here are from search summaries +
  norms). Re-pull the current-year CFP before any submission.
- **Jesse Wright's full bibliography** — confirmed ISWC 2024 ×3 + the RDF Surfaces arXiv;
  dblp (pid 189/1514) timed out on fetch — re-pull for a complete list.
- **ACM badging v1.1 wording** — corroborated from venue mirrors (ACM site 403'd); accurate but
  cite the ACM URL as record.
- **Typst HTML export** is experimental (mitigated by typst.ts); **typst.ts v0.7.0's pinned
  upstream compiler version** (whether it includes 0.15 features) is unconfirmed — verify.
- **PrinceXML licensing price** — sources ranged widely across tiers/eras; confirm at
  princexml.com if the HTML→PDF fallback is ever chosen (it is not the recommendation).
- **LIPIcs / some ACM-IEEE camera-ready** may require a Typst→LaTeX conversion step for the
  final publisher upload (non-issue for arXiv / the live site).

---

## Appendix — consolidated source list (all real; "verify" = not fetched live this run)

**Writing craft.** Peyton Jones *How to write a great research paper*
<https://www.cis.upenn.edu/~sweirich/icfp-plmw15/slides/peyton-jones.pdf> /
<https://simon.peytonjones.org/great-research-paper/> · Widom *Tips for Writing Technical Papers*
<https://cs.stanford.edu/people/widom/paper-writing.html> · Ernst *How to write a technical paper*
<https://homes.cs.washington.edu/~mernst/advice/write-technical-paper.html> · Freeman *How to
write a good paper* <https://deviparikh.com/citizenofcvpr/static/slides/freeman_how_to_write_papers.pdf>
· Heiser *Writing Good Systems Papers* <https://trustworthy.systems/publications/papers/Heiser_12:div.slides.pdf>
· SIGPLAN Empirical Evaluation Checklist <https://www.sigplan.org/Resources/EmpiricalEvaluation/>,
PDF <https://github.com/SIGPLAN/empirical-evaluation/raw/master/checklist/checklist.pdf>,
manifesto <https://blog.sigplan.org/2019/08/28/a-checklist-manifesto-for-empirical-evaluation-a-preemptive-strike-against-a-replication-crisis-in-computer-science/>
· empirical-SE primer <https://arxiv.org/pdf/2506.11002>.

**Venues.** PVLDB <https://www.vldb.org/2027/submission-guidelines.html> · SIGMOD repro
<https://reproducibility.sigmod.org/> · EDBT <https://www.edbt.org/> · ISWC
<https://iswc.semanticweb.org/> · ESWC <https://www.eswc-conferences.org/> · PETS/PoPETs
<https://petsymposium.org/> · USENIX Security CFA
<https://www.usenix.org/conference/usenixsecurity26/call-for-artifacts>. Jesse Wright: dblp
<https://dblp.org/pid/189/1514.html> (verify), ISWC 2024 <https://ewada.ox.ac.uk/news/2024/08/26/iswc.html>,
RDF Surfaces <https://arxiv.org/pdf/2406.10659>, GitHub <https://github.com/jeswr>.

**Reproducibility / AE.** ACM badging v1.1
<https://www.acm.org/publications/policies/artifact-review-and-badging-current> (verify; mirror
<https://sysartifacts.github.io/sosp2021/badges>) · USENIX OSDI'25 CFA
<https://www.usenix.org/conference/osdi25/call-for-artifacts>, Security'22 appendix
<https://www.usenix.org/conference/usenixsecurity22/artifact-appendix-guidelines>, guide
<https://secartifacts.github.io/usenixsec2023/guide>, filled example
<https://www.usenix.org/system/files/usenixsecurity25-appendix-hao.pdf> · SIGMOD reviewer guide
<https://reproducibility.sigmod.org/documents/Guidelines-Reviewers.pdf> · PVLDB tutorial
<https://www.vldb.org/pvldb/vol17/p4221-hirn.pdf>, template <https://github.com/db-reproducibility/template>
· EuroSys badges <https://sysartifacts.github.io/eurosys2026/badges>, 5-yr lessons
<https://www.sigops.org/2025/lessons-from-five-years-of-artifact-evaluation-at-eurosys/> · ctuning
checklist <https://github.com/ctuning/artifact-evaluation/blob/master/docs/checklist.md> · Heiser
Benchmarking Crimes <https://gernot-heiser.org/benchmarking-crimes.html>, van der Kouwe et al.
<https://arxiv.org/abs/1801.02381> · Kalibera & Jones <https://kar.kent.ac.uk/33611/45/p63-kaliber.pdf>
· Docker-for-repro <https://arxiv.org/pdf/2308.14122>.

**Auto-gen stack.** Typst 0.15 <https://typst.app/blog/2026/typst-0.15> · automated generation
(sys.inputs) <https://typst.app/blog/2025/automated-generation/> · data loading
<https://typst.app/docs/reference/data-loading/json/> · HTML reference
<https://typst.app/docs/reference/html/> · typst.ts <https://github.com/Myriad-Dreamin/typst.ts>,
Next.js editor <https://github.com/Mapaor/typst-online-editor> · templates: charged-ieee
<https://typst.app/universe/package/charged-ieee/>, arkheion <https://typst.app/universe/package/arkheion/>,
para-lipics <https://typst.app/universe/package/para-lipics/>, daskol <https://github.com/daskol/typst-templates>
· Tectonic <https://tectonic-typesetting.github.io/en-US/>, setup action
<https://github.com/WtfJoke/setup-tectonic> · pandoc-crossref <https://github.com/lierdakil/pandoc-crossref>
· Paged.js <https://pagedjs.org/> · PrinceXML <https://www.princexml.com/purchase/license_faq/> ·
HTML→PDF survey <https://ironsoftware.com/suite/blog/comparison/html-to-pdf-2026-guide/>,
<https://print-css.rocks/tools> · Next.js static export <https://nextjs.org/docs/app/guides/static-exports>.

**Skills.** academic-research-skills <https://github.com/Imbad0202/academic-research-skills>
(CC-BY-NC) · claude-scientific-writer <https://github.com/K-Dense-AI/claude-scientific-writer>
(MIT) · academic-writing-agents <https://github.com/andrehuang/academic-writing-agents>.
