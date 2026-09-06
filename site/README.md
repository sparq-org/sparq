# sparq feature-showcase site

The live-interactive feature-demonstration website for sparq, served at
**https://sparq.jeswr.org/**. A Next.js static export (`output: "export"`)
styled as a sibling of [`jeswr/solid-pod-manager`](https://github.com/jeswr/solid-pod-manager):
Tailwind v4 theme-in-CSS, the privacy-first teal OKLCH palette, shadcn (`radix-nova`),
Inter, `--radius 0.7rem`. Design record: `research/feature-showcase-site-design.md`.

## What's in this foundation

- **App shell** — `w-64` sidebar (feature surfaces grouped by the IA), sticky `h-16`
  backdrop-blur header, `max-w-6xl` main, mobile `Sheet` drawer, light/dark theme.
- **Landing / overview** — what sparq is, the in-fold SPARQL runner, the three flagship cards,
  and the full surface grid (each card links to its page; unbuilt pages render an honest
  "coming soon" placeholder that states the planned execution tier).
- **In-fold SPARQL runner** (the home hero's `hero-runner.tsx`) — runs **real SPARQL** against a
  sample graph via the actual Rust engine compiled to WebAssembly. Not a fixture; nothing is
  sent to a server. The runner is code-split and the wasm engine loads lazily on browser idle,
  so it never blocks the initial page load (see *Lazy wasm loading* below — `sq-4296`).
  Its "Open in workbench →" button hard-navigates to `/app`, the single operational workbench.
- **The full workbench lives at `/app`** (`sq-4hiqe`, maintainer directive 2026-07-05) — the
  operational GUI (`gui/app`, overlaid at `/app/` by the Pages deploy) is the one place to load
  data, run SELECT/CONSTRUCT/UPDATE, save cross-session workspaces, connect a server, and inspect
  plans. The old in-tab `/try` REPL playground was removed ("very broken; the app has everything
  you need") and `/try` now serves a permanent client redirect stub to `/app` (same static-export
  pattern as `/gui`). Cross-session workspace persistence still ships in the `@sparq/client`
  `workspace.ts` library (Tauri disk / `localStorage` / in-memory backends) consumed by `/app`.
- **/about** — the honest "what runs where" matrix.

The per-surface interactive demos and the three flagship demos (ZK car-hire, MPC £100k,
Solid pairs) are tracked as child beads of the showcase epic (`sq-4r4b`).

## Develop

```bash
# The REPL needs the wasm bundle. Build it once from the workspace root:
(cd ../js && npm ci && npm run build:wasm)   # → js/wasm/

cd site
npm ci
npm run dev        # sync-wasm → public/wasm, then next dev
npm run build      # → out/  (static export)
npm run lint
npm run test:unit  # node:test helpers + native sparq-policy parity fixture (needs cargo)
npm run test:e2e   # Playwright — headless browser smoke tests (see below)
```

`public/wasm/` is generated (git-ignored): `scripts/sync-wasm.mjs` copies the
wasm-pack `--target web` output from `js/wasm/`, then `scripts/bundle-wasm-esm.mjs`
([`esbuild`](https://esbuild.github.io/)) bundles the `@sparq-org/sparq` RDF/JS surface
(`js/src`) into a single self-hosted `public/wasm/sparq.js` (see below). The `prebuild`
script runs both automatically before `next build`.

### Lazy wasm loading — keeping the engine off the critical path — `sq-4296` (#935 / #981)

The sparq engine wasm is heavy, so it is kept **out of the initial page load** in three
layers — the page paints and becomes interactive before any of it has finished loading:

1. **The wasm binary is never bundled into JS.** `@sparq/client`'s `loadSparq()`
   dynamic-imports the wasm-pack glue with a `/* webpackIgnore: true */` hint, so the glue
   (`public/wasm/sparq_wasm.js`) is fetched as a plain ESM module at runtime and the
   `sparq_wasm_bg.wasm` binary is a static asset — neither enters a webpack chunk. (Verify:
   no `_next` JS chunk embeds the `.wasm`; the loader chunk only holds the *URL string*.)
2. **The in-fold runner is code-split.** `site/src/components/home/hero-runner-lazy.tsx` wraps
   the wasm-backed `HeroQueryRunner` in `next/dynamic(..., { ssr: false })` with a skeleton
   fallback, so the home page (`/`) renders its hero shell first and streams the runner chunk in
   afterwards. The server page renders the lazy wrapper rather than the runner directly.
3. **The engine warm-up is deferred to browser idle.** Components call
   `prewarmSparqWhenIdle()` (not the eager `prewarmSparq()`), which schedules the
   fetch+instantiate via `requestIdleCallback` (with a `setTimeout` fallback) so the wasm
   loads *after* first paint. The first interaction (`Run query` / `Validate`) still
   `await`s the **memoised** `loadSparq()`, so it joins the in-flight load rather than
   re-paying a cold start — and never calls into an uninitialised wasm.

**Self-hosted ESM `<script type="module">` import — named `Dataset` (#981, `sq-55w5a`).**
`scripts/bundle-wasm-esm.mjs` bundles the `@sparq-org/sparq` RDF/JS surface into a single
self-contained `public/wasm/sparq.js`, published into the static export — so the named
`Dataset` entry can be imported directly from this project's **own** GitHub Pages origin, with
**no third-party CDN**. The bundle keeps the ~MB engine `.wasm` **external** (it re-exports the
sibling wasm-pack glue), so it is fetched lazily by the first `await Dataset.…`, never by the
import line — the #981 lazy-load posture:

```html
<script type="module">
  import { Dataset, DataFactory as DF } from "https://sparq.jeswr.org/wasm/sparq.js";
  const ds = await Dataset.fromString('<a> <b> "x" .', "ntriples"); // wasm lazy-fetched HERE
  ds.add(DF.quad(DF.namedNode("a"), DF.namedNode("b"), DF.literal("y")));
  console.log(ds.size, ds.store.queryBoolean("ASK { ?s ?p ?o }"));
</script>
```

The same named entry is also available from an ESM CDN (the published `@sparq-org/sparq` npm
package): `import { Dataset } from "https://esm.sh/@sparq-org/sparq"`.

**Low-level glue (`Store`).** The wasm-pack `--target web` glue is itself a real ESM module, so
the engine `Store` class can be imported directly — instantiate it (its default `init`) before
use; the ~MB `.wasm` is fetched lazily by that init, not by the script tag:

```html
<script type="module">
  // basePath-aware: '/sparq/...' on GitHub Pages, '/...' under the Tauri webview root.
  import init, { Store } from "https://sparq.jeswr.org/wasm/sparq_wasm.js";
  await init(); // lazily fetches + instantiates sparq_wasm_bg.wasm (off the critical path)
  const store = Store.load("<a> <b> <c> .", "ntriples");
  console.log(store.query("SELECT * WHERE { ?s ?p ?o }"));
</script>
```

In an app, prefer `@sparq/client`'s `loadSparq()` (memoised, basePath-defaulted) over a
hand-rolled `init()` so the cold start happens at most once across the whole page.

### Build modes — `basePath` (Pages vs Tauri) — `sq-9vw5`

`next.config.ts` env-switches `basePath`/`assetPrefix` off `NEXT_PUBLIC_BASE_PATH` so the
**same** `out/` export serves two hosts. The `@sparq/client` wasm loader keys its runtime
asset URLs off the *same* env var, so the build-time route prefix and the runtime wasm-fetch
prefix stay in lockstep.

| Host | Command | `basePath` | When |
|---|---|---|---|
| **GitHub Pages** @ custom-domain root (production) | `cross-env NEXT_PUBLIC_BASE_PATH= npm run build` (the Pages workflow sets `NEXT_PUBLIC_BASE_PATH=''`) | `''` (root-relative) | served at `https://sparq.jeswr.org/` (org-migration cutover, `sq-uj38w`) — every asset/route is root-relative (`/_next/…`) |
| **Tauri 2 webview** | `npm run build:tauri` (= `cross-env NEXT_PUBLIC_BASE_PATH= npm run build`) | `''` (root-relative) | the desktop GUI serves the export from the `tauri://` root, where a `/sparq` prefix would 404 |
| **Legacy sub-path** (fallback) | `npm run build` (no env) | `/sparq` | the historical `jeswr.github.io/sparq/` sub-path; kept only as the unset-env fallback, not used in production |

The env var is read once in `next.config.ts`: **unset** keeps the historical `/sparq` sub-path as a
LEGACY fallback (no test/caller change); an explicit **empty string** selects the root-relative
export used by BOTH production Pages (custom domain) and Tauri; a malformed value falls back to the
legacy default. The `build:tauri` script uses [`cross-env`](https://www.npmjs.com/package/cross-env)
(`cross-env NEXT_PUBLIC_BASE_PATH= npm run build`) so the env var is set to an **empty string**
identically on Linux, macOS, and Windows `cmd.exe` — the bare `NEXT_PUBLIC_BASE_PATH='' npm run build`
inline-prefix form is a Unix-shell idiom that `cmd.exe` cannot parse. The GUI's
`gui/src-tauri/tauri.conf.json` `beforeBuildCommand` runs this script, so a Tauri build gets the
right export with no extra step.

### Browser smoke tests (Playwright)

`e2e/` holds headless-browser smoke tests driven by Playwright against a real `next dev`
server (config: `playwright.config.ts`). The first time, install the browser:

```bash
npx playwright install chromium
npm run test:e2e
```

Coverage spans the critical site flows. **Critical-flow smoke** (bead sq-jp7ry, issue #835):
`home-smoke.spec.ts` asserts the home hero + primary nav boot with zero console errors;
`home-runner.spec.ts` runs a trivial `SELECT * WHERE { ?s ?p ?o }` on the bundled sample in the
home in-fold runner and asserts a non-empty results table; `capabilities-smoke.spec.ts`
asserts the `/capabilities` showcase renders its hero, flagship band and all five theme
sections. **Regression guards:** `shacl-rerun-regression.spec.ts` (sq-jp7ry) drives the
`/surface/shacl` validator three times in one session and asserts no wasm object-lifecycle
fault (`__wbg_ptr` / "null pointer passed to rust" / "recursive use of an object") and a
fresh report each run — guarding the issue-#835 SHACL `__wbg_ptr` class; `home-runner.spec.ts`
and `shacl-validator.spec.ts` cover the home runner's typed result table and the single-validate
report. The
**ZK car-hire prover pre-warm** (`zk-prewarm.spec.ts`, sq-5q63) loads `/showcase/zk-car-hire`,
waits for the *Prover ready* pill, and asserts the first **Generate ZK proof** click pays no
cold start (observed via a test-only `window.__zkProverColdStarts` counter — pure observability).
The wasm-engine specs (`try-query`, `shacl-*`, `repl-results`) `test.skip` when the wasm bundle
is absent, so the **light** CI lane (no Rust toolchain) stays green; they run in full once
`npm run sync-wasm` has synced a `build:wasm` bundle. CI runs this lane on site-touching PRs
(`.github/workflows/site-e2e.yml`); Playwright outputs (`test-results/`, `playwright-report/`,
the browser cache) are git-ignored.

## Papers (the academic paper factory)

The `/papers` route is generated by the paper factory (epic **sq-gum8**; design record
`research/paper-factory-design.md`, process `skills/academic-paper/SKILL.md`).

- **Sources** — `papers/*.typ` (single-source [Typst](https://typst.app/)); shared helpers
  in `papers/_lib/bench.typ` (deterministic evidence) and `papers/_lib/timing.typ` (measured
  wall-clock). Numbers are **never hard-coded** — each paper reads them from the paper-bound
  evidence file (`src/data/paper-evidence.json`) via `--input data=...`, so the PDF and the
  in-site HTML cannot disagree and a paper auto-updates as evidence refreshes.
- **Build** — `scripts/build-papers.mjs` (run by `prebuild` + `dev`, also `npm run
  build-papers`) compiles each registered paper to a **PDF** (`public/papers/<slug>.pdf`, the
  download) and a **semantic HTML fragment** (`src/generated/papers/<slug>.html`, the in-site
  render). Both `public/papers/` and `src/generated/` are git-ignored build outputs.
- **In-site render** — Typst's native HTML export (`typst compile --format html --features
  html`), rendered as a static asset (no WASM compiler shipped to the browser). This was
  chosen over `typst.ts` for static-export compatibility + a far smaller client payload; the
  trade-off is that page-layout-only constructs (centring, rules) are dropped in the HTML
  view but preserved in the PDF.
- **Honesty gate** — `papers/_lib/bench.typ`'s `headline(key)` accessor **fails the build** if
  a paper cites an `environment: "indicative"` (non-canonical work-box) number as a headline
  result; only `environment: "canonical"` (deterministic, machine-independent) numbers may
  back a claim. `build-papers.mjs` also schema-checks the evidence file first.
- **Measured timings** — a separate, stricter door (bead **sq-gum8.16**).
  `scripts/sync-canonical-timing.mjs` (run by `prebuild` + `dev`, also `npm run
  sync-canonical-timing`) **derives** `src/data/paper-timing.generated.json` from the committed
  canonical measurement envelopes under `bench/canonical-competitor-results/`; only an envelope
  that self-declares `"canonical": true` is admitted, and within one, only a row whose result
  count matches the suite's committed `bench/<suite>/expected-rows.tsv` — loaded independently
  of the envelope, so a run cannot bless its own counts (count is checked before time, so a
  fast-but-wrong comparator cannot be published). `build-papers.mjs` re-derives and
  byte-compares, so a hand-edited or stale timing **fails the build**. Papers render these only
  via `papers/_lib/timing.typ`'s `headline_timing(key)` / `timing_provenance(key)`, so the
  provenance (aggregate, host class, commit) always renders with the number; because Typst has
  no private module bindings, that boundary is enforced by the `runTimingSourceGate()` build
  step (`scripts/timing-source-gate.mjs`), which fails the build on any other route to a value:
  the rule is on Typst's data-loading builtins, non-literal `#import` paths, and the reflective
  constructs (`eval`, `std`) that could manufacture a loader without naming one — rather than on
  the file name, so neither a constructed path nor a constructed loader slips past it. It is a
  source-text gate over `papers/**/*.typ`, not a Typst capability sandbox.
- **Toolchain** — needs the **Typst CLI 0.15+**. Install the release binary
  (`https://github.com/typst/typst/releases`) on `PATH` (or `~/.local/bin` / `~/.cargo/bin`,
  or set `TYPST_BIN`). Without it, `build-papers.mjs` degrades to placeholders + a warning so
  local dev still runs; **CI installs Typst** (pinned + SHA-256-verified in `pages.yml`) so
  the real artifacts are produced. To register a new paper: add an entry to
  `src/data/papers.ts` + a `papers/<slug>.typ`.

## Deploy

`.github/workflows/pages.yml` (on push to `main`) builds the wasm bundle + the static
export, **overlays the existing benchmark dashboard** (`dev/` from the `benchmark-data`
branch) into `out/dev/`, and publishes one Pages artifact via `actions/deploy-pages`.
The dashboard at `/sparq/dev/bench/` is preserved verbatim; the workflow never writes
the `benchmark-data` branch (bench.yml owns it). Publishing requires the repo's Pages
source to be **"GitHub Actions"** (Settings → Pages) — a one-time switch from the legacy
branch source.
