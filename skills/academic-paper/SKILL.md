---
name: academic-paper
description: "Use when turning a sparq contribution into an academic paper — identifying a genuinely-novel contribution (the re-runnable intake), classifying it and picking a venue (ISWC/ESWC Resource for the engine, PVLDB/SIGMOD/EDBT for DB-systems, arXiv/workshop for not-yet-sound ZK/MPC), drafting a single-source Typst (.typ) paper whose eval section binds to live benchmark data via --input/sys.inputs, building both a PDF and an in-site HTML page from that one source, and reviewing claims↔evidence under sparq's empirical-honesty mandate (canonical vs indicative numbers; the ZK/MPC not-yet-sound disclaimer). The paper-factory for epic sq-gum8."
---

# academic-paper — the sparq paper-factory

This is the repeatable PROCESS for producing one paper from a sparq contribution. It is
**factory machinery**, not prose advice: it enforces sparq's empirical-honesty mandate at
the *data* layer, binds every number to live benchmark data, and emits a PDF + an in-site
HTML page from one Typst source. Design record: `research/paper-factory-design.md`
(consolidates the contribution inventory + the auto-gen/PDF research). Epic **sq-gum8**.

**Two non-negotiable constraints govern every step.** They are the point of the factory; do
not soften them.

1. **No ZK/MPC security property may be claimed as proven.** The single-prover ZK verifier
   has no external audit (bead **sq-qhy4**, open); the multi-prover path is re-open
   (**sq-9hrn**); the malicious-secure MPC layer is a stub. A ZK/MPC paper today is an <!-- privacy-claims-allow: "malicious-secure" here is a negation ("is a stub"), not an achieved-property claim -->
   **honest design / limitations / negative-result** contribution, arXiv/WIP only, and
   **must cite sq-qhy4**. Never write "secure" / "verifiable" / "private" as a *proven*
   property — only as a *design goal*.
2. **Work-box (EC2 `-aws`) wall-clock numbers are NON-CANONICAL.** Only deterministic
   integer metrics are canonical today (W3C/OGC conformance floors, byte-identity, recall
   floors, gate/round/byte counts, differential-fuzz pass). A speed/memory claim needs the
   **canonical runner** (`research/ci-ec2-design.md`). Indicative numbers are **never
   co-tabulated with canonical ones** and never feed an aggregate figure-of-merit.

The factory has six stages. Stages 0/1/3/5 are the methodology this skill encodes; 2/4/6 are
scripted (see `research/paper-factory-design.md` §1–§2 for the script/route layout).

---

## Stage 0 — Intake: identify a genuinely-novel contribution (re-runnable)

Run before claiming anything is publishable. This is the re-runnable identification process;
its current ranked output is `research/paper-contributions-inventory.md` — read it first and
diff against it rather than starting from scratch.

**Scan six sources:** crates (`crates/sparq-*/src`, READMEs — what is IMPLEMENTED vs
designed-only; `NotYetImplemented`/`#[ignore]` markers); `research/*.md` (claimed technique +
prior-art + the author's own honesty hedges + negative results); beads
(`.beads/issues.jsonl` — open audit/gating beads, esp. sq-qhy4/sq-9hrn/sq-1gir);
benchmark deltas (`research/BENCHMARKS.md`, `bench/` — which numbers exist, canonical vs
work-box); the conformance scoreboard (`crates/sparq-conformance/src/scoreboard.rs` — the
only fully-canonical evidence); skills (`skills/*/SKILL.md` — distilled durable findings).

**Score four criteria — a candidate must pass all four:**

1. **Novelty vs prior art** — a new algorithm/construction/finding, or a faithful
   re-implementation of cited prior art? Name the prior systems (ACORN, NaviX, QLever,
   RDFox, CostFed, SPDZ, coZK, …). Integration *can* be a contribution — but framed as a
   *systems/integration* paper, never dressed as a new algorithm. A measured negative result
   is also a contribution.
2. **Evidence available** — canonical evidence *today*, or only work-box numbers / a design?
3. **Generality** — does the claim generalise beyond sparq's exact code, or is it a local
   tuning artefact?
4. **Honesty / soundness** — for crypto: proven, or merely implemented + internally
   reviewed? For perf: canonical or not? **This is the veto against overclaiming.**

**Assign a readiness verdict:** PUBLISHABLE-NOW (sound claim + canonical evidence today, or a
design/methodology contribution needing no benchmark) | NEEDS-CANONICAL-BENCHMARKS (honest +
real, but the headline rests on work-box numbers — publishable once re-gathered on the
canonical runner) | NOT-YET-SOUND / NEEDS-EXTERNAL-AUDIT (ZK/MPC — honest design / negative
result only; cite sq-qhy4). **Rank by readiness × impact** (a PUBLISHABLE-NOW candidate with
a real audience outranks a high-impact unaudited crypto claim).

**Re-run triggers:** a new crate/feature lands; a benchmark improves or a canonical run
completes (re-evaluate every NEEDS-CANONICAL-BENCHMARKS candidate); a conformance floor rises;
an audit bead changes state (a sq-qhy4 *pass* moves the ZK candidate out of NOT-YET-SOUND —
until then it cannot move); a new `*-measured.md`/negative-result doc appears. The re-run is
cheap: re-scan, re-score, re-verdict, re-rank, diff the inventory, open/close paper beads.

---

## Stage 1 — Classify & venue-target

Map the contribution to its family, then to a venue + track + Typst template. **Verify every
page limit / deadline / blinding policy against the current-year CFP before submitting — they
drift yearly.** Maintainer precedent (jeswr, Oxford / ODI / W3C) centres on **ISWC/ESWC**;
the engine-as-artifact maps exactly to their **Resource track**.

| Family | Best venues | Track / notes |
| --- | --- | --- |
| **A — DB/engine performance** (indexes, parallel/streaming/mmap exec, ingest, compression) | **PVLDB** (rolling monthly, single-blind, EA&B track for measurement-heavy papers); **SIGMOD** (fixed rounds, double-blind); **ICDE**; **EDBT** (most accessible first DB paper) | systems/empirical; reproducibility/artifact tracks near-universal |
| **B — Semantic Web / RDF / SPARQL** (federation, SHACL, GeoSPARQL, RSP-QL, inference, GenAI-over-RDF, RDFC-1.0) | **ISWC** (home venue; **Resource track purpose-built for a reusable engine**); **ESWC** (European sibling; note its strict no-preprint window conflicts with arXiv-first); TheWebConf; SEMANTiCS; JWS (journal) | Research / **Resource** / In-Use + posters/demos/workshops |
| **C — Security/privacy (ZK/MPC), NOT-yet-sound** | **arXiv preprint + workshop NOW** (HotPETs / ZKProof / WPES / ISWC-ESWC privacy workshops); **PoPETs** = best real target *once sound* (journal, 4 rolling deadlines/yr); USENIX/CCS/S&P aspirational | preprint/WIP today; graduate to a real venue only when sq-qhy4 (and sq-9hrn) close |

**Template:** `charged-ieee` (IEEE 2-col) | `arkheion` (arXiv-style) | `para-lipics` (LIPIcs;
accepted papers need a Typst→LaTeX conversion for the publisher) | an acmart-style template.
**Double-blind venues** (SIGMOD/VLDB/ICDE/WWW/CCS/S&P) require the anonymized build (Stage 4);
ISWC/ESWC blinding varies by track. **Rolling cadence** (PVLDB monthly, PoPETs quarterly,
SIGMOD multi-round) → target submit-when-ready, not one annual crunch.

---

## Stage 2 — Evidence capture (the honesty boundary)

Paper-bound numbers live in a **dedicated evidence file, `site/src/data/paper-evidence.json`**
— distinct from `site/src/data/benchmarks.generated.json`. The two MUST NOT be conflated:

- **`paper-evidence.json`** — the *canonical-only* paper evidence. Every record is a
  deterministic, machine-INDEPENDENT metric (a recall floor, an answer-safety invariant, a
  cost-model crossover) lifted from a named test, so each is `environment: "canonical"` and
  carries a `source` field tracing it to that test/dataset. [OPUS-4.8: as-built, PR #336.]
- **`benchmarks.generated.json`** — per-commit *timing* on the dev work-box, **all
  `environment: "indicative"`** (folded in by `site/scripts/sync-benchmarks.mjs` from the
  `benchmark-data` branch). It feeds the site's benchmark widgets, **never** a paper headline.
- **`paper-timing.generated.json`** — MEASURED wall-clock, **all
  `environment: "canonical-timing"`**, entirely **DERIVED** (never hand-written) by
  `site/scripts/sync-canonical-timing.mjs` from the committed canonical envelopes under
  `bench/canonical-competitor-results/**`. An envelope is admitted only if it self-declares
  `"canonical": true`, so a work-box timing cannot enter the class by construction. Rendered
  only via `headline_timing()` — see gate 5. [OPUS-5: as-built, bead sq-gum8.16 / F4.]

A paper-evidence record (a `binding` upgrades its `source` from asserted to machine-verified —
see gate 4):

```json
{ "value": 98, "unit": "passing-assertions", "environment": "canonical",
  "kind": "conformance-ratchet-floor",
  "source": "crates/sparq-shacl/tests/w3c_core.rs::BASELINE_PASS (mirrored in scoreboard.rs)",
  "binding": { "kind": "rust-anchor", "file": "crates/sparq-shacl/tests/w3c_core.rs", "anchor": "BASELINE_PASS" },
  "note": "machine-independent: a const a CI test asserts, not a timing." }
```

**The honesty gate (as-built; PR #336 landed layers 1–3, sq-gum8.13 added the binding layer 4):**

1. **Data layer — `site/scripts/build-papers.mjs::runHonestyGate()` runs FIRST**, before any
   compile: it schema-checks `paper-evidence.json` (every record needs a valid `environment`
   ∈ {canonical, indicative}, a `source`, and a `value`) so a malformed/untraceable record is
   a clear early build failure, not a Typst stack trace.
2. **Compile layer — `headline(key)` in `site/papers/_lib/bench.typ` panics the Typst
   compile** if a record's `environment != "canonical"`. `headline()` is the ONLY accessor
   allowed inside a headline result table/figure; the ungated `ev(key)` is for
   clearly-labelled *indicative* callouts only.

So any non-canonical (work-box / indicative) number can never appear as a paper's headline
evidence. Indicative and canonical numbers are never in the same table and never feed an
aggregate. `paper-evidence.json` holds **only the deterministic metrics** (conformance counts,
recall floors, byte-identity / answer-safety invariants, gate/round/byte counts) — machine-
independent by construction. A **measured** wall-clock number is a different class with a
different door (gate 5): it may be cited only from a committed canonical envelope and only
through `headline_timing()`. There is still no *continuous* canonical latency runner
(`research/ci-ec2-design.md`, blocked on one IAM step — sq-me8x), so measured numbers come in
one-off gathered envelopes rather than per-commit; a claim must be scoped to exactly the
corpus, queries and commit the cited envelope measured.

3. **Coarse phrase/number gate over the paper surface — `check-no-perf-numbers.py` +
   `check-privacy-claims.sh`** (beads sq-mkza / sq-4hga / sq-mraf). Both CI gates also scan
   the paper `.typ` sources **and** the prose (`note`/free-text) fields of
   `paper-evidence.json`: a result-shaped number typed inline (not routed through the
   accessor) trips the perf gate (accessor-call spans + Typst comments are skipped, so the
   canonical data path is unflaggable); an unqualified ZK/MPC soundness/privacy phrase trips
   the privacy gate (with the same `privacy-claims-allow: <why>` inline marker + same-line
   negator/hedge exemption). They share **one** forbidden-phrase list
   (`scripts/honesty-phrases.json`), and `build-papers.mjs` re-runs both at the build
   boundary. This is a **COARSE** class only — see Stage 5 for what it does and does **not**
   catch; do not read its green as a semantic-correctness guarantee.
4. **Value↔source binding gate — `scripts/verify-paper-evidence.py`** (bead sq-gum8.13, F1).
   Layer 1 checks a `source` is *present*; this layer checks the value still *matches* it.
   Each canonical record carries a machine-checkable `binding`, one of:
   - `json-pointer` `{file, pointer}` — the value must EQUAL the RFC-6901-pointed scalar in a
     committed JSON (derivation-strength);
   - `rust-anchor` `{file, anchor}` — the anchor (const/test-fn name) must EXIST in the file AND
     the value must appear within its window (**honest limitation: literal-adjacency only** —
     it can in principle false-pass if the same literal sits nearby for another reason; strictly
     weaker than json-pointer, strictly stronger than unchecked free text);
   - `doc-anchor` `{file, quote}` — the exact quote must be present (existence-strength only).

   **Fail-closed:** a canonical record without a passing binding aborts the paper build unless
   it is on the shrink-only allowlist (`scripts/paper-evidence-binding-allowlist.json`); the
   verifier fails if that allowlist ever GROWS (F3/sq-gum8.15 drives it to empty; F2/sq-gum8.14
   exports the scoreboard consts so the conformance floors become `json-pointer`s). Wired into
   `build-papers.mjs`; `--self-test` is the CI gate (a value-DRIFT fixture and a missing-anchor
   fixture each exit non-zero). Like gate 3 it is **MECHANICAL** — value↔source match only; a
   *semantic* overclaim (a true number framed misleadingly) stays the Stage-5 human review.
5. **Measured-timing derivation gate — `site/scripts/sync-canonical-timing.mjs`** (bead
   sq-gum8.16, F4). Papers **may** headline a measured wall-clock number, but only under all of:
   - **Derived, never typed.** The value comes from a committed envelope under
     `bench/canonical-competitor-results/**`; there is no author-writable timing file.
     `build-papers.mjs::runCanonicalTimingGate()` re-derives and **byte-compares** against the
     committed `paper-timing.generated.json`, so a hand-edited or stale timing aborts the build.
   - **`"canonical": true` or nothing.** The envelope must self-declare it (the dedicated
     quiet-EC2 protocol). Work-box timings have no path in.
   - **COUNT BEFORE TIME.** A row is admitted only if that engine's own result count matches the
     suite's committed `bench/<suite>/expected-rows.tsv`, which the derivation loads
     **independently of the envelope** — an envelope's own `count_crosscheck.expected` is a
     self-assertion and cannot bless its own counts. Absent a committed row, agreement is
     **re-derived** from the per-engine counts (≥2 engines, all equal to the published row); the
     envelope's `all_agree` flag is not trusted on its own. So a fast-but-WRONG comparator can
     never have its timing published. Refusals are recorded with their reason in the generated
     file rather than silently dropped.
   - **Provenance is unavoidable.** `headline_timing(key)` always renders aggregate + host class
     + git commit beside the number, and `timing_provenance(key)` gives the full block form
     (corpus, rows, gather date, envelope path). Typst cannot enforce this on its own — it has
     no private module bindings, so the lib's parsed dataset and internal lookup would be
     importable — so the boundary is enforced mechanically by
     `build-papers.mjs::runTimingSourceGate()` (`site/scripts/timing-source-gate.mjs`): a paper
     source may import only `headline_timing` / `timing_provenance` / `timing_keys` from the
     lib, never the whole module, and may not load data from disk *at all* — the rule is on
     Typst's loader builtins (`json`/`read`/`csv`/…) and on non-literal `#import` paths, not on
     the file name, because `json("/src/data/paper-" + "timing.generated.json")` spells neither.
     A loader can also be *manufactured* without naming it (`eval("j" + "son", mode: "code")`, or
     reached as `std.json(…)`), which no loader-name rule can see, so those reflective constructs
     are themselves refused in code position — prose/raw `eval(P)` notation stays writable.
     The two evidence libraries that must load data get one pinned, audited expression each.
     Unit tests run negative fixtures (raw-data import, internal accessor, wildcard, direct
     `json()`, concatenated path, aliased loader, reflective `eval`, `std`-qualified loader, raw
     `read()`, computed `#import`) through that gate and require each to fail. Honest scope: a
     source-text gate over the compiled paper sources, not a Typst capability sandbox.

   Deliberately **not** provided: any ratio / speed-up helper. A cross-engine ratio is a *claim*
   needing prose framing (which corpus, which queries, what was excluded), so it belongs in the
   paper's text under Stage 5, not in a helper that would make it look derived. Only the same-box
   per-query envelope family is derived today; the HTTP, materialization and HDT families are
   skipped **with a recorded reason** (see `skipped[]`) rather than guessed at.

---

## Stage 3 — Draft: single-source Typst, bound to live data

**Writing methodology (the order matters):**

- **Contributions list FIRST** — it drives the whole paper ("the paper substantiates the
  claims"). Each bullet is **refutable** (names *what* is achieved, specific enough to be
  disproved) and **forward-references the section that delivers the evidence**. PJ's
  contrast: NO = "We describe the WizWoz system. It is really cool."; YES = "We give the
  syntax and semantics … (§3); we prove the type system sound … (§4); … half the length of
  the Java version (§5)."
- **Abstract = 4 sentences, written last:** (1) the problem; (2) why it is interesting;
  (3) what your solution achieves; (4) what follows.
- **Introduction ≤ 1 page**, in order: what is the problem / why important / why hard (why
  naive approaches fail) / why unsolved before (what distinguishes you) / key components +
  results → ending in the bulleted contributions list. The new contribution must be clear by
  **end of page 3**. Delete "the rest of this paper is organized as follows" — the
  contributions list does that job.
- **Real examples, never `foo`/`bar`; active voice; self-contained figure captions that tell
  the reader what to notice** (reviewers skim).
- **Related work late & charitable** — "credit is not like money": be generous; compare and
  contrast, don't list; label any inferior approach explicitly and up front; write it "as if
  telling the cited authors why they should care." Defer to near the end unless a concise
  early defensive stance is needed.
- **Conclusion** with concrete numbers; **no wishlist "future work"** ("no partial credit for
  neat things you wanted to do but didn't").

**The live-data recipe (the load-bearing factory mechanism).** Author one `.typ`; the build
injects `paper-evidence.json` via `--input data=...`. Numbers are read through the shared
`site/papers/_lib/bench.typ` helpers, **never** hard-coded:

```typ
#import "_lib/bench.typ": headline, ev, provenance, authors, anon
```

`bench.typ` binds `#let evidence = json(bytes(sys.inputs.data))` once and exposes the
accessors. The eval section uses `#headline(key)` for any headline table/figure (it panics
the compile on a non-canonical record — the honesty gate), and the ungated `#ev(key)` only
inside an explicitly-labelled *indicative* callout:

```typ
The filtered top-k matches the exact-filtered ground truth at recall@10
>= #headline("filtered_ann.recall_at_10_floor").
#provenance("filtered_ann.recall_at_10_floor")
```

For a **measured** wall-clock number, import the separate, stricter lib instead. It reads the
DERIVED `paper-timing.generated.json` from the Typst project root (no `--input` needed, so it is
not bounded by the OS argument-size limit) and offers no raw-value accessor — the number cannot
be shown stripped of its provenance:

```typ
#import "_lib/timing.typ": headline_timing, timing_provenance

sparq answers SP2Bench q01 in #headline_timing("timing.sp2b.sparq.q01").
#timing_provenance("timing.sp2b.sparq.q01")
```

Keys are `timing.<suite>.<engine>.<query>`; `npm run sync-canonical-timing` lists what exists
and, in `skipped[]`, what was refused and why. A key that is not in the derived file panics the
compile — check `skipped[]` before assuming a number should be there.

Run the self-check before handing off to review:

- **SIGPLAN-7 rubric** (red/yellow/green, supports judgment not a binary gate): (1) claims
  clearly stated + scoped, no implied generality; (2) suitable, fairly-configured baseline;
  (3) principled benchmark choice (established suites, justify subsets, no cherry-picking,
  don't test on the training set); (4) adequate analysis (enough trials; geometric mean for
  ratios, harmonic for rates, median under outliers; report variability/CIs); (5) relevant
  metrics (measure *all* important effects — index/compile time alongside runtime); (6) clear
  reproducible design (full platform spec, key parameters explored); (7) appropriate
  presentation (zero-based axes, right precision, distribution-reflecting summary).
- **Benchmarking-crimes self-check** (Heiser): full HW spec (CPU+µarch, cores, clock/turbo
  policy, cache levels, RAM, kernel release, storage); full SW spec (`rustc` + `Cargo.lock` +
  flags, baseline-system versions); dataset name+version+SHA-256+seed; report absolutes not
  ratios-only; never summarize 4/6/7/49% as "up to 49%"; never use a competitor's sub-optimal
  config (Heiser: that "probably constitutes scientific misconduct").

---

## Stage 4 — Build: one source → PDF + in-site HTML

`site/scripts/build-papers.mjs` (wired into `prebuild` + `dev`, after
`sync-canonical-timing.mjs`) drives both artifacts for every paper in `papers.ts`, injecting the
SAME `paper-evidence.json` so they cannot diverge. Measured timings need no injection — both
compiles read the same committed `src/data/paper-timing.generated.json` from `--root`, so they
cannot diverge either. The two compile invocations it runs (paths relative to `site/`,
`--root site`):

```bash
# PDF (the download — static asset under public/papers/, served by the Next.js export):
typst compile papers/<source>.typ public/papers/<slug>.pdf \
  --root . --input data="$(cat src/data/paper-evidence.json)"

# In-site HTML — Typst NATIVE HTML export (NOT typst.ts):
typst compile papers/<source>.typ src/generated/papers/<slug>.html \
  --root . --format html --features html --input data="$(cat src/data/paper-evidence.json)"

# anonymized build for a double-blind venue (the .typ's authors()/anon honour this):
typst compile papers/<source>.typ /tmp/<slug>-anon.pdf \
  --root . --input data="$(cat src/data/paper-evidence.json)" --input anon=true
```

[OPUS-4.8: as-built, PR #336.] The **in-site HTML** page at `/papers/<slug>` uses **Typst's
native HTML export** — *not* typst.ts/`@myriaddreamin/typst.react`. The build extracts the
`<body>` inner HTML and the React route (`components/papers/paper-html.tsx`) injects it as a
static fragment into a scoped `.paper-prose` block (no WASM compiler shipped to the browser).
The PDF and the HTML are built from the *same* `.typ` + the *same* injected evidence, so the
numbers can't diverge. **Trade-off:** native HTML export is experimental, so **layout-only
constructs (centring, alignment, page rules / horizontal rules) drop in the HTML view** — the
expected `--features html` "experimental feature" + page-rule warnings on stderr are harmless;
those constructs are **preserved in the PDF**. Author for both: rely on semantic structure
(headings, tables, paragraphs) for meaning, treat alignment as PDF-only polish. If Typst is
not on `PATH`, the script emits an honest placeholder fragment and warns (CI installs Typst so
real artifacts always build). When Typst *is* present the build first compiles
`papers/_lib/timing-selfcheck.typ` (not a registered paper — a fixture) to a temp dir in both
export modes, so the measured-timing accessor is verified against the real derived data on every
build. A `benchmark-data`/evidence change — or a **new canonical envelope** landing under
`bench/canonical-competitor-results/` — re-triggers `next build` and both artifacts
regenerate, which is what makes a published measured number auto-update. For a venue that
rejects Typst→LaTeX, the camera-ready fallback is
Pandoc/LaTeX via Tectonic against `acmart`/`IEEEtran`/`lipics`. **Do not** author the PDF with
`@react-pdf/renderer` — it duplicates the numbers (breaks single-source).

---

## Stage 5 — Review gate: claims ↔ evidence

For every claim in the intro/contributions list, identify its evidence (analysis / theorem /
measurement / case study) and confirm the forward-reference resolves. The gate (run as
subagent section-reviewers + one cross-cutting honesty/repro reviewer) **blocks** on:

- **Any indicative number in a claim** (the Stage-2 build-time honesty gate —
  `build-papers.mjs` schema-check + the `headline()` compile panic — enforces this
  mechanically; the reviewer also checks no indicative/canonical co-tabulation).
- **A measured timing whose claim outruns its envelope.** Gate 5 proves the number came from a
  canonical envelope with a correct row count; it says nothing about *scope*. The reviewer
  checks the claim names the corpus, the query set and the commit the cited envelope actually
  measured, does not generalise across suites, and does not quietly omit the queries where the
  project's own engine lost (the derived data contains those too — cherry-picking them out is a
  Stage-5 violation no gate can see).
- **A C-family (ZK/MPC) draft using "secure"/"verifiable"/"private" as a proven property** —
  must be a design goal, with the sq-qhy4 soundness-gap disclaimer present.
- **Overclaiming / implied generality, weak/unfair baselines, missing ablations, no error
  bars, irreproducibility** (the SIGPLAN-7 + benchmarking-crimes findings from Stage 3).

**What is — and is NOT — mechanically gated (do not over-sell the gate).** [OPUS-4.8] As of
beads sq-mkza / sq-4hga / sq-mraf, the two CI honesty gates also scan the paper-factory's own
surface: `check-no-perf-numbers.py` scans the paper `.typ` sources (accessor-aware, result-
shaped numbers only) **and** the prose (`note`/free-text) fields of `paper-evidence.json`;
`check-privacy-claims.sh` scans those `.typ` + the evidence JSON for the absolute forbidden
ZK/MPC phrases. Both consume **one shared forbidden-phrase list**
(`scripts/honesty-phrases.json`), and `build-papers.mjs` re-runs both at the build boundary
so the factory cannot serve an un-scanned paper (`scripts/tests/test_typ_honesty_gates.sh` +
`scripts/tests/test_build_papers_honesty_boundary.sh` pin this). **But this is a COARSE,
mechanical class only** — a hard-coded result-shaped number, or one of a fixed set of
unqualified soundness/privacy phrases. A **subtle semantic overclaim** — an unsupported
"generalises to all RDF stores", an implied causal/superiority claim phrased without a
forbidden phrase or a result-unit, an unfair-baseline framing — is **NOT** caught by any
gate and **remains this Stage-5 human/subagent review's responsibility**. Treating the gate's
green as a soundness guarantee would itself be an empirical-honesty violation.

Resolve **all** findings before publish. Final check: the claims↔evidence loop is closed and
every headline number is canonical.

---

## Stage 6 — Publish & auto-update

Merge → `next build` serves `/papers/<slug>` + `/papers/<slug>.pdf` (static export, under the
`/sparq` basePath). A `paper-evidence.json` refresh re-runs the build → both artifacts
regenerate with current numbers. Each headline number carries its **provenance** inline (the
`provenance(key)` helper prints the record's `source` test + `environment`). A venue
camera-ready triggers the optional Typst→LaTeX export. To **register a new paper**, add an
entry to `site/src/data/papers.ts` (`slug` + `source`) and a `site/papers/<source>.typ` (e.g.
`filtered-ann.typ`) — the index, the per-paper route, the nav, and the PDF build
are all data-driven off `papers.ts`.

---

## Empirical-honesty rules (the rubric, condensed)

- **Canonical vs indicative** — headline tables/figures use only `environment: canonical`
  records, reported with variability. Indicative (EC2 `-aws`) numbers live only in clearly
  labelled "indicative development measurement" callouts that name the instance type + `-aws`
  kernel, state they are not the basis of any claim, and are never co-tabulated with canonical
  numbers. Never quote a speedup blending the two.
- **ZK/MPC not-yet-sound disclaimer** — every C-family paper cites **sq-qhy4**, is marked
  arXiv/WIP-only, and frames any security/privacy/integrity/attestation property as a *design
  goal*, never proven. It graduates to a real venue only when sq-qhy4 (and, for the
  multi-prover path, sq-9hrn) close.
- **Honesty norm** — be scrupulously honest; don't over-sell, hide drawbacks, or disparage
  others' work; submit only if proud to attach your name as-is; anticipate scepticism and
  state how the approach could fail.

## Pilot

The factory's first pilot is **A1 + A2** together (both PUBLISHABLE-NOW, no canonical runner
or external audit needed): **A1** = RDF-native filtered-ANN ("Filter-as-Query" — exact
transitive-BGP filter over the engine's own dict-ids + answer-safety; canonical recall
evidence today; frame as integration, not an ANN-algorithm novelty); **A2** = the honest
same-box benchmark methodology (a methods/reproducibility contribution that strengthens A1's
eval). See `research/paper-contributions-inventory.md` (Part 2) and
`research/paper-factory-design.md` (§6).

## Paper P2 instrument — the SPARQL logic-bug harness (`crates/sparq-metamorph`)

[FABLE-5] sq-gum8.6. Paper **P2** (draft bead `sq-gum8.7`) is the first dedicated
logic-bug testing paper for SPARQL engines (SQL/Datalog/Cypher are covered by
SQLancer/queryFuzz/GDsmith; SPARQL is not). Its instrument is the opt-in
`crates/sparq-metamorph` crate (`publish = false`; nothing in the shipping graph links
it): **TLP re-derived for SPARQL** — partitioning on `FILTER(c)` / `FILTER(!c)` /
`FILTER(COALESCE(IF(c, false, false), true))`, where the third branch reifies SPARQL's
evaluation-*error* outcome (type errors + unbound variables), which unlike SQL's `NULL`
is not a testable value — the spec-cited case analysis in the `tlp` module docs IS the
paper's core claim; **NoREC for SPARQL** (predicate moved to projection position);
a **differential oracle** reusing `sparq-difftest`'s engine-independent comparators;
a **seeded deterministic generator** (in-crate SplitMix64 — a ledger seed reproduces
its case exactly); SPARQL-protocol **drivers** (`protocol-drivers` cargo feature,
off by default) for Fuseki/Virtuoso/Oxigraph-class endpoints; and the **found-bug
ledger** (JSONL; an entry REQUIRES an upstream issue URL + developer-confirmation
status — the paper counts developer-confirmed bugs, never raw oracle alarms; the
wrong-result class is strictly separated from engine errors). The bug-hunting
campaign itself runs **off-CI** (work box / EC2; docker endpoints fine); CI runs the
network-free mutation self-tests (a seeded wrong-result mutant must be flagged by all
three oracles — non-vacuity). sparq is itself a campaign target: finding our own bugs
is honest and reportable.

## Notes

- This skill was **authored in-repo** (not installed). The strongest external pipeline
  (Imbad0202/academic-research-skills) is CC-BY-NC — incompatible with this permissively
  licensed repo; the MIT K-Dense scientific-writer is life-sciences-leaning and lacks
  sparq's venue map + non-canonical-benchmark handling + the live-data Typst pipeline. Those
  two sparq specifics ARE the contribution of the factory.
- This is a **site/process** surface, not a public crate API, so it is not in the
  `skills/SKILL.md` public-surface router (that router lists code-integration surfaces).
  Keep this skill in sync with `research/paper-factory-design.md` if the pipeline changes.
- Verify before relying on version-specific features: Typst's native HTML export is still
  experimental (the layout-only-drop caveat in Stage 4 — re-check each Typst release whether
  more layout survives the HTML view); the current-year CFP for any target venue.
