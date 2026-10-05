# Standalone `bench/dashboard/` retirement assessment (sq-p744)

<!-- [OPUS-4.8] Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
     Assessment-only design record for sq-p744. No code retired — see "Verdict". -->

> 🤖 SPARQ agent assessment. Decides whether the standalone `bench/dashboard/` can be
> retired now that the Next.js site has an in-site Benchmarks section. **Verdict: DO NOT
> retire now — retire-after-gap-closure.** The in-site view is a strict subset of the
> standalone dashboard, and the standalone dashboard is the *live published* GitHub Pages
> artifact. Deleting it today would both lose capability and break the served dashboard.
> **Post-record status:** The separate sq-iigf Pages source-mode cutover is complete
> on the repo side: the producer workflow exists. The Pages service setting is reported
> as flipped but is not verified here; see the linked runbook's Provenance section.
> The in-site trend and scaling chart gaps below have since closed (sq-hsyg), but the
> retirement cutover remains open: `bench.yml` still publishes the standalone dashboard
> to `benchmark-data`, and `pages.yml` overlays that tree into the site export.

## What each surface is

- **Standalone `bench/dashboard/`** (`index.html` + `dashboard.js` + `dashboard.css` +
  `vendor/Chart.min.js` + `root-redirect.html`, with data from `metric-labels.json` and
  `competitors.json`). This is **the live GitHub Pages dashboard**: `.github/workflows/bench.yml`
  (the "Seed Pages dashboard onto benchmark-data" step, main-only) copies the UI files +
  data files into `dev/bench/` on the `benchmark-data` branch every CI run, where they render
  the per-commit history written by `github-action-benchmark` (`window.BENCHMARK_DATA`,
  capped at a 20-commit trend window via `max-items-in-chart: 20`).
- **In-site Benchmarks** (`site/src/app/benchmarks/**`, `site/src/components/benchmarks/**`,
  data `site/src/data/benchmarks.generated.json` produced by `site/scripts/sync-benchmarks.mjs`).
  This is a **latest-commit-only, table-based** view. The generated JSON carries only
  `{ generatedAt, source, latest, labels, competitors }` — there is **no history/series** in it.

## Coverage comparison (standalone vs in-site)

| Feature | Standalone dashboard | In-site | Gap |
|---|---|---|---|
| Latest-value metric tables, human labels | yes | yes | none |
| Capability families | ~13 (Core, SPARQL, SHACL, GeoSPARQL, Full-Text, Vector/ANN, Reasoning, ZK, Solid, HDT, RSP, GenAI, GPU) | 7 (Core, SPARQL, SHACL, GeoSPARQL, Full-Text, Vector/ANN, Reasoning) | **ZK / Solid / HDT / RSP / GenAI / GPU families not shown in-site** |
| Same-box SPARQL comparison | yes | yes (data mostly null pending gather) | none (data-side gap is sq-sxso/sq-axky) |
| External reference baselines | yes | yes (`references-note`) | none |
| Speedup-vs-best | per-suite + per-metric | per-suite-group pill only | minor |
| **Trend / history charts** (per-commit time series) | **yes** — the dashboard's core reason to live on `benchmark-data`; renders the 20-commit window | **NO** — no history data, no charting library, no chart component | **MAJOR — total absence in-site** |
| **Scaling charts** (metric vs dataset size/depth: Deep-Taxonomy depths, WatDiv SF) | yes | **NO** (depth values render as flat rows, not plotted) | **MAJOR** |
| Featured cross-suite competitor matrix (QLever/Oxigraph/EYE/RDFox columns) | yes | **partial/no** — base metric table has only Benchmark/sparq/Unit columns; competitor data appears only inside the same-box table + references note | gap |

(In-site feature inventory verified against `site/src/data/benchmarks.ts`,
`site/src/app/benchmarks/{page,layout,[type]/page}.tsx`, and
`site/src/components/benchmarks/{type-sidebar,suite-group,metric-table,same-box-table,competitive-summary,references-note}.tsx`.)

## Load-bearing data files (must NOT move regardless)

`bench/dashboard/metric-labels.json` and `bench/dashboard/competitors.json` are read **directly**
by `site/scripts/sync-benchmarks.mjs` at site build time. They are also referenced by
`scripts/gen-metric-labels.py`, `scripts/drift-scan.py`, `scripts/dashboard-smoke.js`,
`research/capability-benchmark-program.md`, and the per-suite READMEs. They are shared
infrastructure, **not** part of any dashboard UI that could be retired.

## Verdict — retire-AFTER-gap-closure (do NOT delete now)

The bead's premise — that the in-site benchmarks may have made the standalone dashboard
redundant — does **not** survive inspection. The in-site view is a *strict subset*: it covers
the latest-value tables well, but it has **no trend charts, no scaling charts**, fewer
capability families, and a narrower competitor presentation. On top of that, the standalone
dashboard is the **live served Pages artifact** wired into every main-branch CI run; deleting
its files would break the seed step in `bench.yml` and take down the published dashboard.

Retirement is therefore **premature**. It becomes warranted once the in-site Benchmarks
section reaches parity on the two MAJOR gaps (trend charts and scaling charts) — at which
point the `bench.yml` seed step + `benchmark-data` Pages publishing must be cut over to the
site (this is the broader Pages-cutover work in `docs/pages-cutover-runbook.md`), and only
then can the standalone UI files be removed (the data files stay).

### Gaps that gate retirement (beads)

- **Trend/history charts in-site** (`sq-hsyg`, per the bead brief) — port the per-commit
  trend charts. This needs `sync-benchmarks.mjs` to emit *history* (not just `latest`) and a
  charting component in the site. **Blocker for retirement.**
- **Scaling charts in-site** — port the size/depth scaling view. **Blocker for retirement.**
- **Family coverage in-site** — register the ZK / Solid / HDT / RSP / GenAI / GPU families
  (shown as "not yet reported" until they emit), matching the dashboard's honest full-coverage list.
- **Featured cross-suite competitor matrix in-site** — render the competitor columns the
  dashboard shows (gated on the stalled competitor gather, `sq-sxso` / `sq-axky`).
- **Pages cutover** (`docs/pages-cutover-runbook.md`) — once the above land, repoint the
  `bench.yml` seed step / `benchmark-data` publishing at the site export. **This is the actual
  retirement step**; the standalone UI files come out only here.

No files were retired by sq-p744; this record captures the decision and the gate conditions.
