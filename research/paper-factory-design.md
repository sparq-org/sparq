# Academic Paper-Factory — Phase-3 Design

<!-- [OPUS-4.8] phase-3 actionable design for epic sq-gum8 (academic paper factory). -->

> 🤖 **SPARQ agent** design record. This consolidates the two completed research phases —
> the novel-contribution inventory + identification process
> (`research/paper-contributions-inventory.md`, PR #317) and the auto-gen + live + PDF
> stack research (`research/paper-factory-research.md`, PR #319) — into one concrete,
> buildable phase-3 design. It is **DOC-ONLY**: no site/crate code is written here. The
> build comes after maintainer review; the phased plan in §7 enumerates each future bead.

**[OPUS-4.8] AS-BUILT CORRECTION (sq-3df4).** The factory was built in PR #336; the
as-built knowledge is in `skills/academic-paper/SKILL.md` (corrected to as-built in PR #349).
**Two design-time commitments below were superseded — read the as-built sources, not the
original text, for current behaviour. The original wording is preserved below as the
historical design record:**

1. **In-site HTML render = Typst NATIVE HTML export, NOT typst.ts.** The §1 / §2 / §3 /
   §7 / §8 text below proposes rendering the `.typ` in the browser via
   **typst.ts / `@myriaddreamin/typst.react`**. The factory instead uses **Typst's native
   HTML export** (`typst compile --format html --features html`):
   `site/scripts/build-papers.mjs` extracts the `<body>` fragment and
   `site/src/components/papers/paper-html.tsx` injects it as a static, scoped `.paper-prose`
   fragment — **no WASM compiler is shipped to the browser**. Trade-off (as-built):
   layout-only constructs (centring, page/horizontal rules) drop in the HTML view but are
   preserved in the PDF. The §7-(b) plan to add `@myriaddreamin/typst.react` + the typst.ts
   wasm to `site/` was **NOT taken**.
2. **Paper-bound numbers come from a dedicated `site/src/data/paper-evidence.json`, NOT
   from `benchmarks.generated.json`.** The Stage-2 / §2 / §5 text below binds paper numbers
   to `benchmarks.generated.json` (the per-commit work-box timing feed, all
   `environment: indicative`). The factory instead reads a **separate, canonical-only**
   `paper-evidence.json` (deterministic, machine-independent records, each `source`-traced
   to a named test); `benchmarks.generated.json` feeds the site's benchmark widgets and
   **never** a paper headline. The honesty gate is two-layer as-built:
   `build-papers.mjs::runHonestyGate()` schema-checks `paper-evidence.json` first, then
   `headline()` in `site/papers/_lib/bench.typ` panics the Typst compile on any
   non-canonical record.

**The two load-bearing constraints inherited from the research (never softened):**

1. **No ZK/MPC security property may be claimed as proven.** The single-prover ZK verifier
   is internally re-audited "sound-as-landed for its threat model" but has **no external
   audit** (bead **sq-qhy4**, open + externally gated); the collaborative/multi-prover path
   is re-open (**sq-9hrn**) and the malicious-secure MPC layer is a stub. A ZK/MPC paper
   today is an **honest design / limitations / negative-result** contribution — arXiv/WIP
   only — and **must cite sq-qhy4** as the gate.
2. **All wall-clock numbers on the dev work-box are NON-CANONICAL.** The session runs on an
   AWS EC2 box (`-aws` kernel, virtualized host). Only **deterministic integer metrics** are
   canonical today (W3C/OGC conformance floors, byte-identity invariants, recall floors,
   gate/round/byte counts, differential-fuzz pass). A speed/memory claim needs the
   **canonical runner** before publication. Indicative work-box numbers are **never
   co-tabulated with canonical numbers** and never feed an aggregate figure-of-merit.

---

## 1. Factory architecture — the end-to-end pipeline

The factory turns one sparq contribution into (a) a live in-site HTML paper and (b) a
downloadable venue-credible PDF, both bound to the same live benchmark data, with empirical
honesty enforced at the data layer rather than only in prose. Cheap glue runs in the
orchestrator; intensive stages (drafting, review, canonical benchmark runs) are delegated
to subagents per the project's delegation discipline.

```text
  CONTRIBUTION  (family A: DB-perf | B: SemWeb | C: crypto-WIP)
        │
        ▼
  ┌─ STAGE 0 — INTAKE (re-runnable identification process, from #317) ──────────────┐
  │  scan 6 sources (crates / research / beads / bench-deltas / scoreboard / skills) │
  │  → score 4 criteria (novelty / evidence / generality / honesty-soundness)        │
  │  → assign readiness verdict (PUBLISHABLE-NOW | NEEDS-CANONICAL-BENCHMARKS |       │
  │     NOT-YET-SOUND) → rank by readiness × impact → diff vs inventory               │
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 1 — CLASSIFY & VENUE-TARGET ─────────────────────────────────────────────┐
  │  map contribution → venue (§ venue map) + track (research / resource / workshop)  │
  │  pick Typst template: charged-ieee | arkheion (arXiv) | acmart-style | para-lipics│
  │  crypto-WIP ⇒ arXiv/workshop track + mandatory soundness-gap disclaimer            │
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 2 — CANONICAL BENCHMARK CAPTURE (the honesty boundary) ───────────────────┐
  │  run eval on the pinned CANONICAL runner (bare-metal, freq-pinned)                │
  │  emit result records: { key, value, unit, n_reps, dispersion(CI|stddev),          │
  │      environment: canonical|indicative, cpu, kernel, rustc, dataset_sha256,        │
  │      seed, commit }  →  benchmark-data branch  →  benchmarks.generated.json        │
  │  CI GATE: any record with environment=indicative is BLOCKED from paper-bound tables│
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 3 — DRAFT (single-source .typ, §1 writing methodology) ───────────────────┐
  │  contributions list FIRST (refutable, forward-referenced); 4-sentence abstract;   │
  │  ≤1pp intro; related-work-late & charitable; eval references #bench.<key>          │
  │  (never hard-coded numbers); run SIGPLAN-7 + benchmarking-crimes self-check         │
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 4 — BUILD (one source → two artifacts, data-bound) ───────────────────────┐
  │  PDF : typst compile paper.typ public/papers/<slug>.pdf \                          │
  │          --input data="$(cat site/src/data/benchmarks.generated.json)"             │
  │  HTML: typst.ts (@myriaddreamin/typst.react) renders the SAME .typ at /papers/<slug>│
  │  anonymized-build toggle (--input anon=true) strips sparq-org/sparq identity for        │
  │  double-blind venues                                                               │
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 5 — REVIEW GATE (claims ↔ evidence, §1.4/§1.5 rubric) ────────────────────┐
  │  section reviewers + cross-cutting honesty/repro check (subagents); resolve all    │
  │  findings; final: claims↔evidence loop closed; NO indicative number in any claim   │
  └─────────────────────────────────────┬────────────────────────────────────────────┘
        ▼
  ┌─ STAGE 6 — PUBLISH & AUTO-UPDATE ───────────────────────────────────────────────┐
  │  merge ⇒ next build serves /papers/<slug> + /papers/<slug>.pdf (static export)     │
  │  benchmarks.generated.json refresh ⇒ paper numbers AUTO-UPDATE on rebuild           │
  │  each paper carries a provenance stamp (commit + runner + dataset hash)             │
  │  venue camera-ready ⇒ optional Typst→LaTeX export step                              │
  └─────────────────────────────────────────────────────────────────────────────────┘
```

**What each guarantee buys:**

- **Single source of truth for numbers** — the eval section binds to
  `benchmarks.generated.json`; HTML and PDF cannot disagree, and a paper auto-updates as
  benchmarks improve (Stage 6).
- **Empirical honesty is enforced, not hoped-for** — the `environment` flag + CI gate
  (Stage 2) makes it *impossible* to cite a non-canonical work-box number in a paper claim.
- **Repeatable** — `skills/academic-paper/SKILL.md` encodes the Stage 0/1/3/5 methodology;
  Stages 2/4/6 are scripted; any agent can run the factory.

### Stack decision (from #319, restated as the design commitment)

- **Single source = Typst `.typ`.** Author once; inject the live JSON at build via
  `--input` / `sys.inputs`; emit a credible PDF (Typst native PDF/A+PDF/UA export) **and**
  an in-site HTML page (via **typst.ts / `@myriaddreamin/typst.react`** rendering, the
  robust choice today — Typst's own HTML export is still experimental).
- **Fallback (camera-ready only):** Pandoc Markdown → LaTeX via Tectonic against the
  official `acmart`/`IEEEtran`/`lipics` class, for the final publisher upload when a venue
  rejects Typst→LaTeX conversion. Not the day-to-day source.
- **Anti-pattern (rejected):** `@react-pdf/renderer` — forces a second document tree, so
  numbers/layout are authored twice and single-source breaks.

---

## 2. Live data flow — how a paper's numbers stay live

The repo already produces `site/src/data/benchmarks.generated.json` via
`site/scripts/sync-benchmarks.mjs` (run by `prebuild`, pulling the `benchmark-data` branch).
The factory layers paper-binding on top of that *existing* seam — it adds no second data
source.

**One JSON → both artifacts, zero duplicated numbers:**

1. **HTML page** — the React route at `/papers/<slug>` renders the `.typ` via typst.ts. The
   `.typ` is fed the JSON exactly as the PDF build is (same `sys.inputs.data`), so the
   in-site render reads the *same* numbers the site's existing benchmark UI uses. (typst.ts
   compiles `.typ` → inline SVG/canvas in the browser at full visual fidelity.)
2. **PDF** — a build step runs `typst compile` with `--input data="$(cat
   site/src/data/benchmarks.generated.json)"`, dropping `public/papers/<slug>.pdf`. The
   static export then serves it as a plain asset.
3. **In the `.typ`:** `#let bench = json(bytes(sys.inputs.data))` once at the top; the eval
   section references `#bench.<key>` (e.g. `#bench.filtered_ann.recall_at_10`) — never a
   literal number.

**Rebuild-on-change (CI trigger):** when the `benchmark-data` branch updates (or
`benchmarks.generated.json` changes on `main`), the site deploy workflow re-runs `prebuild`
+ `next build`, which re-runs the paper build step, which recompiles every registered
`.typ` against the fresh JSON. Both the HTML render and the PDF asset regenerate with
current numbers. No manual edit, no number drift.

**Concrete file/route layout under `site/`** (files to create — design only):

```text
site/
  src/
    data/
      benchmarks.generated.json      # EXISTS — the single number source
      papers.ts                      # NEW — paper registry (slug, title, authors,
                                     #       .typ path, venue, status, family, anon)
    app/
      papers/
        layout.tsx                   # NEW — wraps AppShell; "Papers" sidebar section
        page.tsx                     # NEW — /papers index (one card per registered paper)
        [slug]/
          page.tsx                   # NEW — per-paper page: typst.ts HTML render +
                                     #       "Download PDF" button + provenance stamp
    components/
      papers/
        typst-render.tsx             # NEW ("use client") — wraps @myriaddreamin/typst.react
        paper-provenance.tsx         # NEW — commit + runner + dataset-hash stamp footer
  scripts/
    build-papers.mjs                 # NEW — for each registered paper: typst compile →
                                     #       public/papers/<slug>.pdf with --input data=JSON
  public/
    papers/
      <slug>.pdf                     # GENERATED by build-papers.mjs (static asset)
  papers/                            # NEW — the .typ sources + templates live here
    <slug>/paper.typ
    _templates/{charged-ieee,arkheion,...}.typ
    _lib/bench.typ                   # shared helpers: json-load + #bench accessor + anon toggle
```

`build-papers.mjs` is wired into `package.json` `prebuild` (after `sync-benchmarks.mjs`) so
`next build` always rebuilds PDFs against fresh data, and into `dev` so local previews bind
live too. (The `papers/` `.typ` sources sit at `site/papers/` so the Typst compile and the
typst.ts import resolve from one tree; `public/papers/*.pdf` are git-ignored build outputs,
regenerable like `benchmarks.generated.json`.)

---

## 3. The `/papers` site route design

A papers index + a per-paper page, consistent with the existing AppShell + sidebar-nav and
the in-site benchmarks layout. **Design only — files listed in §2; no site code written
now.**

- **`/papers` (index)** — `app/papers/page.tsx`. One `Card` per paper from `papers.ts`
  (reusing `components/ui/card`, mirroring `app/benchmarks/page.tsx`): title, authors (or
  "anonymized" when `anon`), venue/track target, a **status `Badge`** (PUBLISHABLE-NOW /
  WIP-arXiv / DRAFT, reusing `TIER_VARIANT`-style variants), the contribution family (A/B/C),
  and links to the per-paper page + a direct "PDF" link. An honest provenance line up top
  (same pattern as the benchmarks overview's provenance framing).
- **`/papers/<slug>` (per paper)** — `app/papers/[slug]/page.tsx`, using
  `generateStaticParams()` over `papers.ts` (exact pattern of `app/surface/[slug]/page.tsx`)
  so the static export emits one HTML file per paper. Body:
  - the **typst.ts HTML render** of the paper's `.typ` (`components/papers/typst-render.tsx`,
    a `"use client"` wrapper around `@myriaddreamin/typst.react`), fed the same JSON the PDF
    gets;
  - a prominent **"Download PDF"** button linking the static
    `/papers/<slug>.pdf` asset (basePath-prefixed `/sparq/papers/<slug>.pdf`);
  - a **provenance stamp** footer (`components/papers/paper-provenance.tsx`): the commit,
    canonical-runner fingerprint, and dataset SHA-256 the numbers came from.
- **Nav** — add a **"Papers"** entry to `components/layout/sidebar-nav.tsx`, after
  "Benchmarks", as its own section (one `NavLink` per paper, like the Showcase section maps
  `FLAGSHIPS`). `isActive("/papers")` highlights it. AppShell is untouched (the route inherits
  it via `app/papers/layout.tsx`, mirroring `app/benchmarks/layout.tsx`).

**basePath note:** the site is served under `/sparq/` (GitHub Pages); every PDF link and
typst.ts asset URL must carry the `/sparq` prefix (Next config `basePath`/`assetPrefix`), as
the existing routes already do.

---

## 4. The repeatable trigger — "any novel contribution / benchmark improvement → its own paper"

The factory is event-driven off the §1 STAGE-0 re-runnable identification process
(from PR #317). The events that regenerate or create a paper:

| Event | Factory action |
| --- | --- |
| **New crate / opt-in feature lands** | re-run intake (scan 6 sources, score 4 criteria); if a new PUBLISHABLE-NOW candidate appears, register a new paper (new `papers.ts` entry + `papers/<slug>/paper.typ`) |
| **A benchmark improves / a canonical run completes** | (a) every registered paper auto-updates its numbers on rebuild (§2); (b) re-evaluate every NEEDS-CANONICAL-BENCHMARKS candidate — promote to PUBLISHABLE-NOW if now canonical-backed, then register its paper |
| **A conformance floor rises** (`scoreboard.rs`) | strengthens any conformance-backed claim; re-rank; affected paper's numbers refresh on rebuild |
| **An audit bead changes state** (esp. **sq-qhy4** external ZK audit, **sq-9hrn** coZK re-audit, **sq-1gir** forge-tests-in-CI) | a sq-qhy4 *pass* moves the single-prover ZK candidate out of NOT-YET-SOUND → it may be promoted from arXiv-WIP to a real venue; until then it cannot move |
| **A new `research/*-measured.md` / negative-result doc** | a measured correction may itself be a (small) contribution; intake screens it |

**How a new paper is registered** (the minimal mechanical step): add an entry to
`site/src/data/papers.ts` (slug, title, authors, `.typ` path, venue, track, status, family,
`anon` default) and create `site/papers/<slug>/paper.typ` from a template. The index, the
per-paper route (`generateStaticParams`), the sidebar nav, and the PDF build step are all
data-driven off `papers.ts`, so registration is the only manual touch — everything else is
generated. Wiring the intake re-run into the maintenance-automation framework
(`research/maintenance-flow-on-automation-design.md`) is the long-term goal so the diff
against the inventory opens/closes paper-candidate beads automatically.

---

## 5. Honesty / soundness gates baked in

The factory makes the two load-bearing constraints **mechanical**, not advisory:

1. **The `environment` field + CI gate (the canonical/indicative boundary).** Every result
   record in `benchmarks.generated.json` carries `environment: canonical | indicative` plus
   the full pinned fingerprint (cpu / kernel / rustc / dataset_sha256 / seed / n_reps /
   dispersion). A CI check (Stage 2) **fails the build** if any `#bench.<key>` referenced by
   a paper-bound table resolves to an `environment: indicative` record. This turns the
   project's memory rule ("EC2 measurements are NON-canonical; gate only deterministic
   metrics") into an enforced pipeline invariant. Work-box numbers are **never co-tabulated
   with canonical numbers** and never feed an aggregate figure-of-merit (cite the
   EC2-non-canonical constraint). Until the canonical runner exists
   (`research/ci-ec2-design.md`, blocked on one IAM step), a paper may publish only the
   deterministic metrics (conformance counts, recall floors, byte-identity, gate/round/byte
   counts) — which are canonical today.
2. **The ZK/MPC not-yet-sound disclaimer.** Any paper whose contribution family is C
   (crypto) carries a **mandatory soundness-gap disclaimer** citing **sq-qhy4**, is marked
   **arXiv/WIP-only** in `papers.ts` (status `WIP-arXiv`), and **may never assert a proven
   security / privacy / integrity / attestation property**. The Stage-5 review gate rejects
   any C-family draft that uses "secure" / "verifiable" / "private" as a *proven* claim
   rather than a *design goal*. A C-family paper graduates to a real venue (PoPETs / USENIX)
   only when sq-qhy4 (and, for the multi-prover path, sq-9hrn) close.
3. **[OPUS-4.8] Coarse phrase/number gate over the paper surface (beads sq-mkza / sq-4hga /
   sq-mraf).** The two repo-wide CI honesty gates were extended to also scan the factory's own
   authored surface: `check-no-perf-numbers.py` scans the paper `.typ` sources (accessor-aware,
   result-shaped numbers only) and the prose (`note`/free-text) fields of `paper-evidence.json`;
   `check-privacy-claims.sh` scans those `.typ` + the evidence JSON for the absolute forbidden
   ZK/MPC soundness/privacy phrases. Both consume **one shared forbidden-phrase list**
   (`scripts/honesty-phrases.json`) so the gate and the build cannot drift, and
   `build-papers.mjs` re-runs both at the build boundary (fail-closed) so the factory can never
   serve an un-scanned paper. See `research/paper-factory-honesty-gate-coverage.md` for the
   design. **Scope caveat (do not over-sell):** this is the **coarse** phrase/number class only
   — a hard-coded result-shaped figure, or one of a fixed set of unqualified phrases. A **subtle
   semantic overclaim** (unsupported generality, an implied superiority claim phrased without a
   forbidden phrase/unit, an unfair-baseline framing) is **not** mechanically catchable and
   stays the Stage-5 human/subagent claims↔evidence review's job. Reading the gate's green as a
   semantic-soundness guarantee would itself breach the empirical-honesty mandate.

The Stage-5 claims↔evidence review also applies the general SIGPLAN-7 + Heiser
benchmarking-crimes rubric (no overclaiming, fair baselines, error bars, full platform
spec) — codified in the skill (§ Deliverable 2).

---

## 6. Pilot selection — the first paper

**Pilot the factory with A1 + A2 together** (the inventory's own recommendation; both are
PUBLISHABLE-NOW, need no canonical runner and no external audit):

### A1 — RDF-native filtered-ANN (the central contribution)

*"Filter-as-Query: Predicate-Constrained Vector Search where the Filter is an Exact RDF
Basic Graph Pattern over the Engine's Own Dictionary Ids."* Target: **ISWC / ESWC research
track** as a systems/integration paper (could reach EDBT). Frame as integration, **not** an
algorithmic ANN novelty.

| Has today (canonical) | Needs |
| --- | --- |
| IMPLEMENTED: `crates/sparq-vectors/src/{filter.rs,rewrite.rs}`, selectivity-gated prefilter vs filtered-traversal crossover | If a **latency** headline is wanted, it becomes NEEDS-CANONICAL-BENCHMARKS (canonical runner) — so the pilot makes the **correctness/exactness** claim, not a speed claim |
| **Canonical recall vs exact-filtered ground truth** (`tests/filtered.rs`) — deterministic | A reviewer's "what's new vs ACORN/NaviX/PathFinder?" answered by: exactness + same-id-space + **transitive/connected-component pushdown** + the answer-safety ("narrow-never-widen") argument |
| BGP→`IdMask` caching is implemented (PR #292 / sq-36ol) but is **engineering, not novelty** — do NOT frame it as a contribution | a clear prior-art positioning section |

### A2 — Honest same-box benchmark methodology (the methods companion)

*"Honest Same-Box Benchmarking for RDF Engines: Differential-Correctness-Gated,
Hardware-Labelled, Negative-Results-Inclusive."* Target: a reproducibility / E&A track (VLDB
E&A or an ISWC resources/reproducibility track) or a methods note. **It strengthens A1's (and
every paper's) evaluation section** — which is exactly why it pilots alongside A1.

| Has today | Needs |
| --- | --- |
| The methodology needs no benchmark to *describe* | nothing to publish the methods description |
| IMPLEMENTED registry seam: `bench/competitors.json`, gather scripts, per-commit deterministic dashboard | the EC2-OIDC canonical lane is **designed-not-executed** (one IAM step); A2 honestly describes the methodology including the canonical-runner design even though that lane is not yet executed |

Be explicit in A2 that it is *methodology, not a performance result*, and that the canonical
lane is designed-not-executed. A2 doubles as the live demonstration of the §5 honesty gate.

---

## 7. Phased build plan (each item → a future bead)

Ordered; the orchestrator will bead each (this worktree cannot run `bd`). Items touching
`site/` are **serialized** — only one site branch in flight at a time (per the project's
one-site-branch discipline).

1. **(a) Author the factory skill — `skills/academic-paper/SKILL.md`.** [THIS PR — doc-only,
   no `site/`.] The repeatable PROCESS (intake → classify → venue → draft → build → review),
   the Typst single-source + live-`--input` recipe, the condensed venue map, the
   empirical-honesty rules, the claims↔evidence rubric.
2. **(b) Typst + typst.ts toolchain + CI build.** Add Typst to CI (digest-pinned), add
   `@myriaddreamin/typst.react` + the typst.ts wasm to `site/`, add `site/papers/_lib/bench.typ`
   + `_templates/`, add `site/scripts/build-papers.mjs`, wire it into `prebuild`/`dev`.
   *Touches `site/`.*
3. **(c) `/papers` route scaffold.** `papers.ts` registry, `app/papers/{layout,page}.tsx`,
   `app/papers/[slug]/page.tsx` (`generateStaticParams`), `components/papers/*`, the sidebar
   "Papers" nav entry. Renders an empty index until a paper is registered. *Touches `site/`.*
4. **(d) First pilot paper — A1 (+ A2 as the methods/eval companion).** Author
   `site/papers/filter-as-query/paper.typ` binding `#bench.<key>` to the canonical
   recall/correctness metrics; register it in `papers.ts`. *Touches `site/`.*
5. **(e) The honesty CI gate.** The `environment: canonical|indicative` schema check on
   `benchmarks.generated.json` + the gate that fails the build if a paper-bound table cites
   an `indicative` record (and refuses to co-tabulate the two tiers). Pairs with (b)/(d).
   *Touches CI; reads `site/`.*
6. **(f) Auto-update wiring.** The CI trigger so a `benchmark-data` / `benchmarks.generated.json`
   change re-runs the paper build (PDF + HTML regenerate); the per-paper provenance stamp;
   the long-term hook into the maintenance-automation intake re-run (§4). *Touches CI + `site/`.*

**New follow-up work surfaced here (for the orchestrator to bead):**

- **Canonical-runner execution is still blocked on one IAM admin step**
  (`research/ci-ec2-design.md`). Until it lands, the pilot publishes only deterministic
  metrics; any latency/memory headline (A1-latency, all of Tier B) stays blocked. This is a
  hard prerequisite for (e)'s value and for promoting Tier-B candidates.
- **typst.ts v0.7.0's pinned upstream compiler version** (whether it includes Typst 0.15
  features used by the templates) is unconfirmed — verify in (b) before relying on 0.15
  features; mitigation is to pin the template feature set to what v0.7.0's compiler supports.
- **Zenodo DOI snapshot + ctuning artifact-appendix** for any paper actually submitted to a
  venue with artifact evaluation (a per-submission task, not part of the live-site pipeline).

---

## 8. Open design question for the maintainer (genuinely needs a decision)

**Where do the `.typ` sources live — `site/papers/` or a top-level `papers/`?** This design
places them under `site/papers/` so the Typst CLI compile and the typst.ts browser import
resolve from one tree and the build step is a plain `site/` script. The alternative (a
repo-root `papers/`) keeps paper sources out of the web app but then needs a copy/symlink
step into `site/` at build. The `site/papers/` choice is recommended (simpler build, matches
the "numbers live in the site" goal), but it couples paper sources to the web app — worth a
maintainer call before (b).

(Lesser uncertainties — venue page limits/deadlines/blinding policies drift yearly and must
be re-verified against the current CFP before any submission; Typst HTML export is still
experimental, mitigated by using typst.ts today — are carried from #319 §7 and need no
decision now.)

---

> 🤖 SPARQ agent — epic sq-gum8 phase 3. Non-sycophantic by mandate. Consolidates #317 + #319.
