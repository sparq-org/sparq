# sparq GUI — design record (design-for-review)

<!-- [OPUS-4.8] sq-uau8 — re-lead per maintainer #757: the PRIMARY purpose is a
     self-contained downloadable app embedding the native engine per platform (usable with
     NO deployed server); server-connect mode is an explicitly-KEPT secondary use-case.
     Adds the persistent-workspace model + a credential/Solid workspace-types proposal. -->

Status: **part-built, part design-for-review.** This record began as a fully design-only
proposal; since then the framework decision and the first phases have **shipped to `main`**,
so it is now a mixed record. Each claim is tagged: **shipped** (landed on `main`, cited to
the file/bead), **feasible-now** (infrastructure exists, not yet wired for the GUI), or
**proposed** (design-for-review, not built). It is the deliverable of the open epic
`sq-ixc3` ("EPIC: cross-platform GUI to interact with the sparq engine — developed +
maintained"). Its sibling epic is `sq-v286` (the release-CI epic; deliverable
`research/release-ci-design.md`). The GUI scaffold (`gui/README.md`) cites this record back
as its single source of truth.

## The headline framing (maintainer #757)

The **primary** purpose of the GUI is a **self-contained, downloadable desktop application
that embeds the built `sparq` engine for the user's platform as a direct native Rust
link** — so a user can download the app and use the full engine **without having to connect
it to a `sparq-server` that is already deployed**. The app *is* a Rust binary with the
engine compiled in, not a thin client. Everything below is organised around that goal.

The **server-connect mode** (point the same editor at an already-running SPARQL 1.1
endpoint) is an explicitly **kept** secondary use-case, **not** a thing to remove. It has
already shipped (endpoint mode `sq-2mke`, the live-subscriptions view `sq-9ij6`, the server
health panel `sq-he72`) and stays a first-class connection mode alongside the embedded
engine. The two modes coexist: embedded-native for "just download and run", and endpoint
mode for "drive my deployed server / a third-party endpoint".

Within the app, work is organised into persistent, cross-session **workspaces** (§2a) —
each with its own imported data, SPARQL editor state, optional natural-language querying
(gated on a user-supplied model API key), and live inference-mode toggles. Credentials and
Solid are handled via **feature-customised workspace types** (§2b), a proposal for
maintainer review.

This record (1) records the framework decision + why (shipped), (2) defines the workspace
model and the per-workspace feature surface, (3) defines the MVP/phase feature set (early
phases shipped), (4) gives a maintenance / architecture plan (component sharing with the
site, CI, tests, releases, versioning), and (5) proposes credential/Solid handling for
review.

## Honesty preamble (load-bearing, non-negotiable)

These constraints are inherited from the repo's standing honesty discipline and must
survive into any GUI that ships:

- **No performance numbers.** This work box is non-canonical (see `AGENTS.md` and the
  MEMORY hygiene rule). The only quantitative figure this document gestures at is the
  on-disk size of the site's WASM bundle (`js/wasm/sparq_wasm_bg.wasm`, on the order of
  ~2.6 MB for the `shacl,jsonld` build) — a build-output size that drifts with the build,
  **not** a benchmark, and not load-bearing to any design choice here. GUI copy may show a
  live in-tab `performance.now()` latency for the query the user just ran (measured,
  labelled), but must never bake in a benchmark claim.
- **ZK and MPC are NOT externally audited.** The in-browser ZK proving path is real
  (`site/src/lib/zk-prover.ts` drives `@aztec/bb.js` UltraHonk), but the file itself
  records that the broader ZK estate is "research-grade, internally reviewed only, and NOT
  externally audited" (`site/src/lib/zk-prover.ts` header). The MPC surface on the site is
  a **faithful in-tab JS simulation**, explicitly "an ILLUSTRATION of the protocol SHAPE,
  not the hardened crate and NOT live MPC" (`site/src/lib/mpc-sim.ts` header), using
  `Math.random` rather than a CSPRNG. The native `sparq-mpc` crate is honest-majority
  semi-honest with a stubbed correctness-proof layer. The GUI must keep these caveats
  exactly as the site does today. The CI gate `scripts/check-privacy-claims.sh` enforces
  this on all user-facing copy.
- **The honesty-tier system is preserved verbatim.** Every surface in the site already
  declares which tier it runs in (`Tier` in `site/src/data/surfaces.ts:27-34`). The GUI
  inherits this taxonomy unchanged; a `walkthrough` surface must never be silently
  dressed up as `live`.

## A. The distinct operational design — a workbench, NOT the marketing site

> Read this alongside the website redesign ([`research/website-redesign.md`](website-redesign.md))
> and the shared method ([`.claude/skills/frontend-design/SKILL.md`](../.claude/skills/frontend-design/SKILL.md)).
> The contrast between the two frontends is the **point**: the website *persuades and routes*
> a curious developer; the GUI *operates the engine* for a developer who already decided. The
> single biggest failure to avoid — and the one the current scaffold risks — is the GUI
> becoming **a thin wrapper around the marketing website**.

### A.1 The anti-goal: stop wrapping the marketing site

Ground truth today: `gui/src-tauri/tauri.conf.json` sets `frontendDist` to `site/out` — the
**whole static marketing site**. That is a deliberate scaffold placeholder (the GUI README
says "a scaffold, not a shipped app"), **not** the design. If the GUI ships as a webview onto
the marketing site, it inherits everything that makes the site the *wrong* tool for working:
a hero, a 4-sentence honesty preamble, a `/capabilities` gallery of *previews*, `/about`,
`/papers`, `/benchmarks`, prose that *describes* features instead of letting you *run* them.

**The GUI must NOT render `Showcase` / `Benchmarks` / `About` / `Papers`.** Reference and
marketing content does **not** ship in the app. A single **Help → "sparq on the web"** menu
item opens the website / GitHub in the **system browser**. *The app is the tool; the website
is the explainer.* When the GUI frontend is built (it is net-new — see §0), it imports the
reusable **component logic** from `site/` (the editor, results, validation playgrounds,
connect/health/subscriptions panels) into **operational hosts**, and ignores the site's
hero/prose/nav shells entirely.

### A.2 Primary model — a workbench shell, not a route tree

The site's mental model is *pages you read*. The GUI's is *an IDE you work in*. The route
tree is **dropped** in favour of a persistent shell with three fixed regions and a
per-workspace tabbed work area:

- **LEFT RAIL** (collapsible, `w-56` — denser than the site's `w-64`): a **stateful
  navigator**, not a marketing nav. Three stacked sections:
  1. **WORKSPACE SWITCHER** — current workspace + dropdown to switch/create/delete (drives
     `@sparq/client` `createWorkspaceStore` → Tauri fs backend; the `sq-atb0` model), with a
     `● saved to disk` / `○ unsaved` indicator reflecting the `WorkspaceBackend`.
  2. **DATASETS tree** — the live store of the active workspace: default graph + each named
     graph with per-graph triple counts (reuse `repl-dataset-panel` logic), an **Imports**
     subgroup listing `WorkspaceSourceMeta` (file/URL, bytes, re-fetch for URL sources), and
     a `+ Import` affordance.
  3. **TOOLS list** — operational, **replacing the 16-surface marketing grid**: Query ·
     Graph view · SHACL · Inference · Full-text · Vector · GeoSPARQL · Federation · ZK · MPC
     · Server. Each is a **verb the user OPENS as a tab against the current store**, carrying
     a small honesty-tier dot (live / native / research) — **NOT a page describing the
     feature.**
- **TOP BAR** (`h-10`, thin): the active connection target switch (**LOCAL ENGINE ⇄
  ENDPOINT**, reusing `connect-panel`'s `ModeSwitch` idea) + store size + a Cmd-K hint +
  theme + a status LED (engine ready / server reachable / proving). **No Showcase /
  Benchmarks / About tabs.**
- **WORK AREA** (center, fills remaining space, **no `max-w` cap — full bleed**): an IDE-style
  **TAB STRIP** of open tools. Default tab is the Query editor. Tabs are dockable panes.
- **BOTTOM STATUS BAR** (`h-6`): **measured** last-run latency (live `performance.now()`,
  labelled), row count, active engine target, workspace persistence backend, errors.

The density and full-bleed work area are deliberately the *opposite* of the website's roomy
`max-w-6xl` marketing layout — the visual contrast reinforces "different product."

### A.3 The command palette is the spine (Cmd-K / Ctrl-K)

This is the keyboard-first model's backbone and the real answer to surface count. It indexes
**every tool, every named graph, recent queries, import actions, connect actions, "run
query", "run as EXPLAIN", "export CSV", "switch workspace"** — fuzzy. It **replaces the
site's left-tree-as-discovery entirely**: there is no need to *see* every surface in the nav
when any of them is one fuzzy keystroke away. (Note the parallel with the website redesign,
which adds the same Cmd-K so it too can shrink its nav — the two frontends share the *pattern*
but mount it in different shells.)

### A.4 Tools, not pages — the surface-to-tool translation

Every one of the website's 16 *surfaces* becomes a GUI *tool* that operates over the **live
persistent native store**, not a fixture, and is mounted in an operational host stripped of
the site's hero/prose wrappers:

| Website surface (a *preview* that links to depth) | GUI tool (a *verb* run against the live store) |
|---|---|
| `/capabilities#sparql` page + sample-graph REPL | **Query tab** — full-height editor + multi-view results over the persistent store |
| `/showcase/*` flagship detail page | the ZK / MPC / Solid **tools**, run on the user's own imported credentials/data |
| `/surface/shacl` page + fixture | **SHACL tab** — validate the *active store* (or a pasted doc) operationally |
| `/surface/inference` walkthrough | **Inference tab** — materialise closure over the active store; optionally COMMIT it back |
| (no equivalent — the site cannot do this) | **Graph view tab**, **Import drawer** (real disk/URL ingest via the native loader), **Server health** over a *connected* endpoint |

What the GUI adds that the site fundamentally cannot: a persistent local store across
sessions, real disk/URL ingest through the **native** loader (threads, mmap, native-only HDT
— no wasm ceiling), a live subscriptions stream off a connected `sparq-server`, and a graph
visualisation of results. These are *operational* capabilities, not explanations of them.

### A.5 What the GUI deliberately cuts from each migrated surface

When a site component's *logic* is reused, its *marketing chrome* is cut: the intro
paragraphs, the tier card, the 6-card capability grid, and the always-open "How this runs" /
"Honest caveats" cards (the exact blocks the website redesign also collapses) are **all
removed** in the GUI — the honesty-tier dot on the tool + a one-line status in the bottom bar
carry the same truth in an operational register. The Query tab specifically drops the site
REPL's built-in-dataset picker and explanatory captions (datasets live in the left rail now)
and runs full-height with multiple co-resident result views (Table | Graph | Raw JSON |
N-Triples/Turtle) against the persistent store rather than a sample graph.

#### A.5.1 Paginated results + the lazy-evaluation gate (`sq-9w4t`, #817)

[OPUS-4.8] Maintainer #817 observed that **serialising / rendering the whole kept result is
the dominant per-render cost**, while the user only ever looks at one screenful. The SELECT
**Table** view is therefore **paginated**: only the visible page of rows is shaped into DOM
cells, and a **bounded read-ahead page cache** (`@sparq/client`'s `createPageCache`, default
~2 pages ahead) warms the next page so a ⏭ is instant while never holding more than
`1 + 2·readAhead` shaped slices live. The pure page-math (`paginateTable` / `pageCount` /
`clampPage` / `createPageCache`) lives in `@sparq/client/results.ts` (host-agnostic, unit-
tested in `site/test/results-pagination.test.mjs`); the React pager is in `query-workbench.tsx`.

This is the **rendering** half. The bead's other half — **demand-driven query *evaluation***
that computes the engine work only up to the current page and advances on a page change — is
**deliberately NOT shipped** and is gated on a design pre-condition that does not hold today:

- The engine result path is **materialised**, not pull-based. The wasm `SolutionCursor`
  (`crates/sparq-wasm/src/lib.rs`) slices an already-fully-evaluated `result.rows`; `next()`
  hands out `result.rows[pos..end]`. There is no pull/iterator that can stop after page *k*'s
  worth of solutions, so "page-wise evaluation" would today still evaluate the whole query and
  only **chunk the already-computed rows** — exactly what `streamQueryRows` already does.
- Real lazy page-wise evaluation needs **one** of: (a) a **pull/Volcano iterator execution
  model** in `sparq-engine` whose top operator can be driven `n` rows at a time and paused; or
  (b) a **query-rewrite** strategy (`… ORDER BY … LIMIT pageSize OFFSET k·pageSize`) re-run per
  page — which is only correct under a **stable total order** (an `ORDER BY` the user did not
  necessarily write) and re-pays the scan each page, so it is a trade, not a clear win.

Until that exec-model decision is made and measured, pagination over the streamed-in-hand rows
is the honest, shippable surface; the evaluation half is tracked as a separate follow-up bead
(no performance number is claimed for either half).

## 0. Ground truth — what exists today

<!-- [OPUS-4.8] sq-uau8 — CORRECTED. The earlier draft said "Nothing is built yet /
     greenfield, grep for tauri returns nothing". That is now FALSE: the Tauri scaffold,
     the shared TS package, the editor uplift, endpoint mode, subscriptions, the health
     panel, the dataset panel, and CSV/TSV export have all landed on main. -->

**Correction to the original draft.** This section once read "the GUI is greenfield; a grep
for `tauri` returns nothing." That is no longer true and the doc must describe reality. The
GUI framework decision and several phases have **shipped to `main`**. What exists today:

- **`gui/` — a Tauri 2 scaffold (shipped, `sq-2e93`).** `gui/src-tauri/` is a real Rust
  crate (`gui/src-tauri/Cargo.toml`, `tauri.conf.json`, `build.rs`, a least-privilege
  `capabilities/default.json`) whose `src/engine.rs` is the **direct native engine link**: a
  `sparq_core::Graph` behind a `Mutex` in Tauri managed state, exposing nine `#[tauri::command]`
  handlers that mirror the wasm `Store` surface (`load`, `query`, `query_quads`,
  `update_in_place`, `explain`, `explain_analyze`, `count`, `ask`, `store_size`) but backed
  by the full native store. `gui/README.md` is explicit that this is a **CI-validated
  scaffold, not a shipped app**: a full `cargo tauri build` needs the webview system
  libraries (webkit2gtk / WebView2 / WKWebView) not present on every dev box, so the
  end-to-end build is validated only in the path-scoped lane `.github/workflows/gui.yml`
  (`sq-bu69`). The engine command layer itself is unit-tested natively.
- **`packages/sparq-client` — the shared TS package (shipped, `sq-jpki`).** The repo now
  has a **root `package.json` with npm workspaces** (`"workspaces": ["packages/*", "js",
  "site", "gui/e2e"]`). `@sparq/client` is the framework-agnostic single source for the
  `WasmStore` type, loaders, SPARQL/Turtle/JSON-LD highlighters, the SPARQL-JSON result
  shapes, and the endpoint client. The site and the GUI both consume it, so the GUI is a
  **zero-new-copy** consumer rather than the third hand-redeclaration the original draft
  warned about — the §0 "biggest long-term liability" below is **resolved**.
- **The editor uplift (shipped, `sq-n5aw` + `sq-ixc3.1`).** The plain `<textarea>` is
  replaced by a real code editor (`site/src/components/sparql-editor.tsx`,
  `rdf-editor.tsx`) with SPARQL syntax highlighting, prefix awareness, keyword/example
  completion, and JSON-LD highlighting in editor + results.
- **Server-connect mode (shipped, `sq-2mke` / `sq-9ij6` / `sq-he72`).** `connect-panel.tsx`
  routes the same editor at any SPARQL 1.1 endpoint with optional bearer auth and live
  connection-safety warnings (all wire logic in `@sparq/client`'s endpoint module);
  `subscriptions-view.tsx` streams `/subscriptions/sse` (or WS) result deltas; a health
  panel renders Prometheus `/metrics` + the VoID / Service Description.
- **Dataset panel + results (shipped, `sq-daru` / `sq-x0kp`).** `repl-dataset-panel.tsx`
  gives a named-graph list with per-graph triple counts; the results panel adds a raw
  SPARQL-JSON view and CSV/TSV export alongside the table + N-Triples toggle.
- **Release matrix + CI (shipped, `sq-8n1c` / `sq-bu69` / `sq-9zjy`).** `release.yml` carries
  the desktop GUI bundles (`.dmg`/`.msi`/AppImage/`.deb`) on the existing per-platform rows
  with SLSA/SBOM/VEX riding for free; `gui.yml` is the path-scoped per-platform GUI lane;
  the COOP/COEP service-worker registration is basePath-aware for the Tauri webview.

What is **still greenfield**: there is no `uniffi` / `cargo-ndk` / `xcframework` in the tree
(the mobile leg is unstarted), and the persistent **workspace model**, the per-workspace
**inference toggles**, **in-workspace NLQ**, and **credential/Solid workspace types** below
are **proposed** — they are the remaining design surface this revision adds.

### The reuse surface that already exists

The "site" (`site/`) is a real, maintained Next.js application — this is the largest asset
the GUI shares from:

- **Stack** (`site/package.json`, `site/next.config.ts`): Next.js with
  `output: "export"` (a fully static, backend-free `out/` tree for GitHub Pages), React 19,
  Tailwind v4, radix/shadcn UI primitives. A hardcoded `basePath: "/sparq"` +
  `assetPrefix: "/sparq"` + `images: { unoptimized: true }` (`site/next.config.ts:8-13`).
- **Components**: ~40 React components under `site/src/components/`, including the flagship
  `repl.tsx` (564 lines — a full SPARQL playground), the per-feature playgrounds
  (`shacl-playground.tsx`, `inference-playground.tsx`, `text-playground.tsx`,
  `rsp-playground.tsx`), the flagship demos (`zk-car-hire.tsx`, `mpc-demo.tsx`,
  `solid-pairs-demo.tsx`), and the captured walkthroughs (`http-server-walkthrough.tsx`
  and the vector/genai/geosparql walkthroughs), plus shared `components/ui/` primitives.
- **Engine-binding TypeScript** (`site/src/lib/`): `sparq-wasm.ts` is a production-grade,
  documented client wrapper around the WASM `Store` — `loadSparq()` / `prewarmSparq()`
  single-init (`site/src/lib/sparq-wasm.ts:243-271`), `loadIntoStore`, `storeToNQuads` /
  `datasetSize`, RDF/JS-style `matchQuads` / `countQuads`, `streamQueryRows` batched
  cursor consumer (`:201`), `sparqShaclValidate`, and SPARQL-JSON render types +
  `formatTerm` (`:343`). Siblings: `reason-wasm.ts`, `sparq-rsp-wasm.ts`,
  `sparq-text-wasm.ts`, `zk-prover.ts`, `mpc-sim.ts`, `http-server.ts`.

### The engine — two real embeddings that already ship

- **Native Rust**: `sparq-cli` (binary) and `sparq-server` (binary + rlib; axum/hyper/tokio
  HTTP) link the engine library crates (`sparq-core`, `sparq-engine`, …) directly, with
  `rayon` threads, `mmap`, and `save`/`open` persistence on natively.
- **WASM**: `crates/sparq-wasm` (`crate-type = ["cdylib","rlib"]`, `wasm-bindgen 0.2`),
  built by `js/`'s `wasm-pack build --target web` into the npm package `@sparq-org/sparq`.
  The site default bundle is built `--features shacl,jsonld` (`js/package.json:37`). Four
  satellite WASM bundles exist (`crates/sparq-{reason,rsp,text,shacl}-wasm`) and prove the
  "one feature, one bundle" pattern.

### The constraints the WASM path imposes

Documented in `crates/sparq-wasm/src/lib.rs`: the wasm `Store` is **single-threaded** (no
rayon), has **no filesystem** (`save`/`open`/`mmap` unavailable), grows **append-only**
until rebuild, and lives under the browser tab's **~4 GiB / practically <2 GiB**
linear-memory ceiling. The doc-comment frames the wasm store as "read-replica shaped," not
a full-power primary store. The built artifact is on the order of **~2.6 MB** for the
`shacl,jsonld` site build (a build-output size that drifts; the "~1.2 MB" note in
`site/src/lib/sparq-wasm.ts` is the lean, feature-less baseline, not what the site ships).

### The two structural gaps a static site cannot close

- **No live backend.** `/surface/http-server` is, by its own header in
  `site/src/lib/http-server.ts:3-8`, a captured curl/SSE transcript fallback — "the static
  GitHub-Pages site has no backend to talk to." The vector / genai / geosparql surfaces are
  `walkthrough` for the same reason, and MPC is `live-sim`. A desktop app with a real local
  engine is exactly what closes these.
- **No shared type source (RESOLVED — `sq-jpki`).** The WASM `Store` surface was once
  declared once in `js/` (the `wasm-pack`-generated `sparq_wasm.d.ts`) and **re-declared by
  hand** as the `WasmStore` interface in `site/src/lib/sparq-wasm.ts`, giving two
  hand-synced copies — flagged here originally as the single biggest long-term-maintenance
  liability and the reason a GUI risked becoming a third copy. That is now **fixed**:
  `packages/sparq-client` (`@sparq/client`) is the one shared TS surface, adopted via
  repo-root npm workspaces, and both the site and the GUI consume it (§3). The GUI is
  therefore a zero-new-copy consumer; this gap is closed.

## 1. Framework recommendation

**Decision (SHIPPED): Tauri 2** — the framework choice below is no longer a proposal; the
scaffold landed on `main` (`sq-2e93`, `gui/src-tauri/`). On desktop, the engine is embedded
as a **direct native Rust link** (the app *is* a Rust binary, so it depends on
`sparq-engine` / `sparq-core` and the `sparq-server` rlib directly — not WASM, not HTTP).
This direct link is **the headline of the whole design** (maintainer #757): it is exactly
what lets the downloaded app run the full engine with **no deployed server** in the loop.
The reasoning is retained below for the record. The existing Next.js /
React / Tailwind / radix frontend for the webview. Treat mobile (Android / iOS) and a thin
WASM-only web fallback as **separate, spike-gated, later tracks**.

This matches the lead candidate named in `sq-ixc3`'s own description.

### The three options, weighed

| Criterion | Tauri 2 (web UI + Rust backend) | egui / iced (pure-Rust native) | Packaged-web (Electron / PWA) |
|---|---|---|---|
| Reuse of the existing site | **High** — renders the same React 19 / Next.js / Tailwind / radix components in a system webview; the ~40 components + surface pages port largely as-is. | **None** — every screen (REPL editor, result tables, the surfaces, ZK/MPC demos) reimplemented in an immediate-mode/retained Rust UI. Largest rewrite. | High (Electron) to total (PWA = literally the static site). |
| Engine embedding | **Direct native Rust link** — full `rayon`, `mmap`, `save`/`open`, all opt-in crates; no WASM ceiling, no HTTP hop. | Direct native Rust link (same engine strength); embedding is trivial because the app is Rust — the cost is the UI. | Electron bundles the WASM engine (inherits the <2 GiB / no-mmap / single-thread limits) or shells out to a sidecar `sparq-server` over HTTP. PWA = WASM-only. |
| Bundle size | Small — no bundled Chromium (uses OS WebView2 / WKWebView / WebKitGTK). | Smallest (single Rust binary). | **Largest** (Electron ships Chromium). PWA ships nothing extra but is not a "real" app. |
| Native feel | Good — native window/menus/tray/FS dialogs; UI is HTML (matches the site). | **Best** raw native, but our UI vocabulary (code-editor, rich result grids, the Noir JS ZK tooling) is web-shaped; rebuilding it native is high-effort, lower-fidelity. | Electron good; PWA constrained by browser chrome. |
| Mobile (Android / iOS) | **Yes** — Tauri 2 added mobile targets; same webview UI; the engine on mobile is the existing native Rust library cross-compiled (NDK / xcframework). Proposed, not started. | iced/egui mobile is immature/experimental — weakest mobile story. | Electron: **none**. PWA: installable but WASM-bound, no native integration. |
| Maintenance burden | **Lowest marginal** — one React codebase serves web + desktop + mobile. Adds a Tauri shell + IPC command layer. | **Highest** — a second, parallel UI codebase forever diverging from the site (against the repo's anti-duplication discipline). | Electron: heavy Chromium-CVE update treadmill. PWA: low, but not the deliverable. |
| Ties into `sq-v286` | **Directly** — `release.yml` already builds per-tier native binaries on `macos-14` / `macos-15-intel` / `ubuntu-latest` / `ubuntu-24.04-arm` / `windows-latest` with SLSA provenance + SBOM/VEX. Tauri's bundler output is "native binary + signed assets," which is what that pipeline is shaped for. | Same native binaries would work, but you ship a worse UI. | Electron's Chromium bundle fights the "Rust native binary + provenance" model `release.yml` is built around. |

### Why Tauri 2, concretely

1. **It maximises the largest existing asset.** The site is a substantial, current React 19
   codebase (~40 components, the surface pages, the ZK tooling). Tauri renders that same
   frontend in a webview; egui/iced throws all of `site/src/components/**` away. The
   alternative is maintaining two UIs forever — which the repo's hygiene discipline is
   explicitly against.
2. **It unlocks the full engine, not the WASM-limited one.** Because a Tauri app's backend
   *is* a Rust binary, you link `sparq-engine` / `sparq-core` (and the `sparq-server` rlib)
   directly. That escapes every WASM constraint documented in
   `crates/sparq-wasm/src/lib.rs`: `rayon` parallel scan/parse, `mmap`-backed dictionaries,
   `save`/`open` persistence, and no ~2 GiB tab ceiling. The desktop app becomes the
   *primary* store the WASM tier admits it is not. Embedding preference, in order:
   **direct Rust link > WASM > HTTP-to-`sparq-server`**. (HTTP-to-sidecar remains a viable
   fallback if process isolation is wanted, but it adds a localhost server + serialization
   hop for no functional gain over a direct link.)
3. **It closes the real gap on the static site.** `/surface/http-server` is a recorded
   transcript today because Pages has no backend. A desktop app ships a real local engine,
   so those surfaces can become live.
4. **It is the only credible mobile path here.** `sq-v286` wants Android / iOS. Tauri 2
   supports mobile with the same UI; iced/egui mobile is experimental; Electron has none.
   The mobile engine artifact is the existing native Rust library cross-compiled
   (NDK / xcframework) — exactly the "library, not app" binding work `sq-v286` already
   scopes ("uniffi / cargo-ndk / xcframework"). **This is proposed/greenfield**: no
   `uniffi` / `cargo-ndk` exists in the repo yet, and `release.yml`'s matrix has no
   Android/iOS rows today.
5. **It rides the CI already built.** `release.yml` builds the desktop targets the GUI
   needs and attests them (SLSA provenance, CycloneDX SBOM + VEX, `cargo-auditable`,
   `SHA256SUMS`). A GUI bundle added to that matrix inherits all of it.

### Honest caveats on the framework choice

- **Static-export friction.** The site hardcodes `basePath: "/sparq"` for Pages. A Tauri
  build loads assets from `tauri://` / `file://`, so the `basePath` / `assetPrefix` must be
  made conditional (env-switched). Minor but real config work, not free.
- **Next.js in a webview.** `output: "export"` already produces a backend-free static
  bundle (good — Tauri needs no Node server). The current site is fully static, so any
  future feature relying on a Next server runtime must stay client-side.
- **Mobile is the riskiest leg.** Tauri 2 mobile is newer than its desktop story;
  cross-compiling the engine (with its `rayon` / native deps and the opt-in crates) to
  Android/iOS is unproven *in this repo* and may force a reduced feature set per target.
  Validate with a spike before committing.

## 2a. Workspaces — the persistent, cross-session organising model (proposed)

<!-- [OPUS-4.8] sq-uau8 — NEW section per maintainer #757: the first-class "workspace"
     concept. Realised by impl beads sq-atb0 (model + persistence), sq-tp1m (inference
     toggles), sq-96o1 (in-workspace NLQ), and §2b's sq-tlo2/sq-3p0z. -->

Maintainer #757 floated the central UX concept: the app is organised into **workspaces**
that **persist between sessions**. A workspace is the unit a user opens, fills with data,
queries, and returns to later. This is the spine the embedded-native app hangs everything
off — without it, "download and run with no server" has nowhere to keep state.

### What a workspace is

A workspace is a named, persisted bundle of:

1. **Imported data** — one or more data sources, each either a **local file** (loaded from
   disk via the Tauri file-open dialog, already permitted by the scaffold's
   `dialog:allow-open` capability) **or a URL** the user points at (fetched and parsed). The
   embedded native store ingests them into the workspace's `sparq_core::Graph`,
   named-graph-preserving, via the existing `load` command. (On desktop this is the full
   native ingest — `rayon`, no ~2 GiB ceiling — not the wasm read-replica.)
2. **SPARQL editor state** — the current query text, prefix declarations, and recent-query
   history, restored on reopen.
3. **An optional natural-language-query (NLQ) configuration** — see "Honesty tier" below;
   active only when the user supplies a model API key.
4. **Inference-mode toggles** — per-workspace on/off switches for RDFS / OWL-RL / N3 closure
   (see below).
5. **A workspace *type*** — see §2b: the type selects which feature surfaces and credential
   handling the workspace exposes.

### How workspaces persist, switch, and isolate

- **Persistence (proposed, Tauri local storage).** Each workspace's metadata (name, type,
  source list, editor state, inference toggles, NLQ-config *without* the secret key) is
  serialised to the app's per-user data directory under Tauri's app-local path
  (`tauri::path` app-data dir). The Tauri `store` plugin (or a small JSON-file store behind a
  new IPC command) is the persistence layer; the scaffold currently registers no plugins
  (`tauri.conf.json` `plugins: []`) and a least-privilege `capabilities/default.json`, so
  adding persistence means **adding one plugin + one capability**, called out here for
  review. **Honest open question:** whether the *imported triples* themselves are persisted
  (re-ingested cheaply from the recorded sources on reopen) or **snapshotted** to disk via
  the engine's native `save`/`open` (`sparq-core`) so a large workspace reopens without
  re-fetching. The native `save`/`open` path is real but **not yet wired into the GUI** (the
  scaffold's `engine.rs` notes it as a follow-up); the recommendation is to start with
  re-ingest-from-sources (simple, no new format on disk) and add a `save`/`open` snapshot
  cache once large workspaces justify it (a future bead).
- **Switching.** A workspace switcher (sidebar / dropdown) swaps the active
  `sparq_core::Graph`. Two designs: (a) one managed `EngineState` whose `Graph` is replaced
  on switch (simplest; only the active workspace is resident — matches the current
  single-`Mutex<Graph>` scaffold), or (b) a map of `workspace-id → Graph` kept resident for
  instant switching at a higher memory cost. **Recommendation:** start with (a) — load on
  activate, drop on switch — and revisit (b) only if switch latency is felt.
- **Isolation.** Each workspace is its own store, so queries, updates, and inference in one
  workspace never see another's data. Credentials/keys are scoped to the workspace that owns
  them (§2b). This isolation is what makes a "Solid workspace" and a throwaway
  "scratch workspace" safe to keep side by side.

### Per-workspace inference-mode toggles (proposed, `sq-tp1m`)

Each workspace exposes live on/off toggles for the engine's reasoning regimes (RDFS /
OWL-RL / N3), wired to the embedded engine the same way the site's
`inference-playground.tsx` / `reason-wasm.ts` drive materialisation today. **Honesty tier:
this is the native / in-browser-wasm tier (`live-new-wasm`)** — real closure computed by the
engine (with the proof-tree `why()` view available), not a mock. Toggling a regime on
materialises (or, on toggle-off, drops back to) the base graph; the design must make clear
to the user whether a query runs over the base or the materialised graph at that moment. No
performance claim is made about materialisation cost (this work box is non-canonical).

### In-workspace natural-language query (proposed, `sq-96o1`)

When — and only when — the user supplies their **own model API key**, a workspace can offer
NL→SPARQL: the user types a question, the engine's introspection grounds a schema summary,
a model proposes SPARQL, and the engine validates + executes it (with a repair round on a
parse/exec failure). This is the **native `sparq-nlq` loop** (ground → generate → validate
→ execute → repair); the GUI is a front door to it with the user's key as the model backend.

**Honesty tier — do NOT overclaim (load-bearing).** On the static site this surface is a
**captured-output `walkthrough`** with `IS_LIVE_LLM = false`: the *plumbing and executed
results* are real, but the NL→SPARQL *generation step* is a committed `ReplayLlm` fixture,
NOT a live model (`site/src/lib/genai.ts` header). In the embedded app the generation step
becomes **live only because the user brought a model + key** — it is the **model-backed /
native tier**, contingent on that key and a reachable model host. The GUI must (a) state
that the key is the user's and never bundle one, (b) keep the standing caveat that
**NLQ exec-accuracy is not a correctness guarantee** — a query can parse, execute and return
rows while answering the wrong question (the site's genai page already says this), and
(c) never silently dress the captured `walkthrough` up as live when no key is present. The
key is a credential and is handled per §2b (workspace-scoped, OS-keychain, never persisted in
plaintext workspace metadata).

## 2b. Credential & Solid handling — feature-customised workspace types (proposal for review)

<!-- [OPUS-4.8] sq-uau8 — proposal per maintainer #757 ("Perhaps we have different types of
     workspaces customised based on the features enabled? Could you propose the right
     approach"). Realised by impl beads sq-tlo2 (cred/Solid) + sq-3p0z (VC import, #822). -->

Maintainer #757 asked specifically: *"I haven't thought as much about how credentials, or
Solid would be handled. Perhaps we have different types of workspaces that are set up in a
way that is customised based on the features that are enabled? Could you propose the right
approach here."* This section is that proposal — **design-for-review, options + a
recommendation, not over-committed.**

### The core idea: a workspace *type* selects features + a credential profile

Rather than one monolithic workspace with every toggle, a workspace has a **type** chosen at
creation that customises (a) which feature panels are shown, and (b) what credentials the
workspace is allowed to hold and how they are stored. Proposed types:

| Workspace type | Data sources | Credentials it holds | Feature panels it adds | Tier honesty |
|---|---|---|---|---|
| **Local / scratch** (default) | local files + plain URLs | *none* (optional NLQ key, OS-keychain) | the core workbench (editor, results, inference toggles) | all `live` / `live-new-wasm` |
| **Endpoint** | a remote SPARQL 1.1 endpoint | optional bearer token (read/write) | the shipped connect panel + subscriptions + health (`sq-2mke`/`sq-9ij6`/`sq-he72`) | `live` against the user's own server |
| **Solid** (proposed) | Pod resources behind a WebID | a Solid OIDC session (DPoP token) | login, Pod data-source browser, WAC/ACP-aware querying | see below — engine restriction is `live`, the auth flow is new |
| **Verifiable-Credentials** (proposed) | imported / dragged-in VCs | the VCs themselves (signed docs) + optional issuer keys | a VC import + query surface (`sq-3p0z`, #822) | see below — query is `live`; ZK over them is **not externally audited** |

A workspace's type is **mostly additive** to the base workbench: a Solid or VC workspace is
a Local workspace plus a credential profile and one or two extra panels. This keeps the core
lean (the repo's opt-in-feature discipline) and means a user who never touches Solid never
sees its UI or its credential prompts.

### Credential storage — the cross-cutting rule

All secrets (NLQ API keys, bearer tokens, Solid sessions, issuer keys) follow one rule:
**store in the OS secret store, never in the plaintext workspace metadata file.** Tauri's
ecosystem has a keychain/`stronghold`-style plugin path for this; the workspace metadata
persists only a *reference* (which credential, which service) and the secret lives in the OS
keychain (macOS Keychain / Windows Credential Manager / libsecret). This is a new capability
+ plugin to add (called out for review, like the persistence plugin in §2a). Bearer tokens
are already handled correctly by the shipped connect panel (sent only in the `Authorization`
header, never logged) — extend that discipline to all credential types.

### Solid workspace (proposed)

- **What is feasible-now.** The engine half already exists: `sparq-solid` does WAC/ACP-aware
  query rewriting — `rewrite_for(sparql, allowed)` injects a `FROM NAMED <g>` clause per
  authorised graph and maps the empty set to a guaranteed-absent sentinel
  (`urn:sparq:nothing`) so an ungranted query is **fail-closed** (zero rows, never an
  accidental union-of-everything) (`crates/sparq-solid/src/rewrite.rs`, mirrored in
  `site/src/lib/solid-acl.ts`). The crate also has loader / materialize / ACP-conformance
  modules. So "query a Solid Pod under its access-control decision" is **already an engine
  capability**, and the embedded app can link it directly.
- **What is new (the auth flow).** What does *not* exist is the interactive **WebID / Solid
  OIDC login + DPoP-bound token acquisition** and the **Pod resource discovery** that feeds
  the Pod's named graphs into the workspace. That is net-new GUI + a thin client. Two
  options: (a) run the OIDC flow in the Tauri webview against the user's identity provider
  and hold the DPoP key in the OS keychain; (b) shell to an external browser for login and
  catch the redirect via a custom URL scheme. **Recommendation:** (a) — keeps the flow
  in-app and the key in the keychain — but flag this as the **highest-uncertainty piece**:
  Solid-OIDC + DPoP in a Tauri webview is unproven *in this repo* and warrants a spike before
  committing (a future bead).
- **Honesty.** The access-control *restriction* is the real SPARQL engine (`live`); on the
  static site the access *decision* is materialised at build time (`solid-pairs-demo.tsx`),
  but in the embedded app with a live Solid session the decision comes from the Pod's actual
  WAC/ACP — an honest upgrade, but only once the live auth flow exists. Until then, the Solid
  workspace type is **proposed**, not shipped.

### Verifiable-Credentials workspace (proposed; relates to #822 + `sq-3p0z` + `sq-1s2.5`)

- **The ask (#822 / `sq-3p0z`).** Let a user **import VCs (drag-drop or URL) and run SPARQL
  queries over them.** A VC is an RDF document (or a JSON-LD doc that maps to RDF), so the
  base capability — load it into a workspace graph and query it — is **just the existing
  ingest path** and is `live`. The new GUI work is the **import surface** (drag-drop zone,
  URL fetch, a "credentials" view listing imported VCs with issuer/subject/validity), and
  parsing the common VC envelopes (JSON-LD VC, and recognising — not necessarily verifying —
  SD-JWT-VC / JWT-VC forms).
- **Verification vs. querying — keep them distinct (honesty).** *Querying* over VC triples
  is `live`. *Cryptographically verifying* a VC's signature, or producing a **zero-knowledge
  proof** about it, is the separate VC/ZK estate (`sq-1s2.5` configurable
  commitment/circuit/signature framework; `sparq-zk` / `sparq-zk-compose`). That estate is
  **research-grade, internally re-audited, and NOT externally audited** — external
  accredited-cryptographer sign-off is **pending** (`sq-qhy4`). So a VC workspace may show
  "imported, parsed, queryable" as a `live` fact, but must **not** present "verified" or any
  ZK property as a production guarantee; any verify/ZK affordance carries the existing
  not-externally-audited caveat (the `scripts/check-privacy-claims.sh` gate enforces this on
  user-facing copy). **Recommendation:** ship VC *import + query* first (`live`, low risk),
  and treat in-app VC *verification* / ZK-over-VCs as a later, explicitly-caveated phase
  riding the `sq-1s2.5` estate — do not couple the simple import surface to the unaudited
  crypto.

### Recommendation (credential/Solid)

Adopt the **feature-customised workspace-type** model the maintainer floated. Concretely:
ship **Local** and **Endpoint** types first (both are essentially the already-shipped
surfaces, re-housed under a workspace); add a **VC** type as **import + query only** (`live`,
no crypto coupling); and treat the **Solid** type as a spike-gated proposal whose engine half
(`sparq-solid` rewrite) is feasible-now but whose live OIDC/DPoP auth flow is the real new
work. Store every secret in the OS keychain, never in workspace metadata. Keep types
additive so the core stays lean and a user only ever sees the credential UI for the features
they opted into.

## 2c. Feature set — MVP and later phases (MVP + Phase 2 shipped)

The GUI is **not greenfield in features**: the site already has a genuinely-live in-tab
SPARQL engine and ~20 showcase surfaces, and the MVP workbench + server-connect phases have
**shipped** (§0). The remaining job is the workspace model (§2a), the credential/Solid types
(§2b), and the later-phase showcases — not building a playground from zero.

### Engine capability → GUI feature → current tier

| Engine capability (real surface) | GUI feature | Current tier / state |
|---|---|---|
| SPARQL 1.1/1.2 SELECT/ASK/CONSTRUCT/DESCRIBE/UPDATE, paths, RDF 1.2 triple terms, EXPLAIN/ANALYZE (lean wasm) | Query editor + results + plan view | **`live`** — `repl.tsx` dispatches all forms + EXPLAIN/ANALYZE via `sparq-wasm.ts` (`query`, `queryQuads`, `updateInPlace`, `explain`, `explainAnalyze`, `queryCursor`, `count`, `ask`, `applyDelta`) |
| Ingest: Turtle/N-Triples/N-Quads/TriG/JSON-LD (+ compressed) | Upload / paste / URL-load, named-graph-preserving | **`live`** — `repl-datasets.tsx` + `loadIntoStore`. HDT is native-only (`skills/hdt-format`), not in the wasm bundle |
| SHACL Core + SHACL-SPARQL → W3C report | Shapes editor + report with `sh:detail` | **`live`** — `validate` (gated `--features shacl`), `shacl-playground.tsx` |
| RDFS / OWL-RL / N3 closure + proof trees | Materialize + proof-tree view | **`live-new-wasm`** — `inference-playground.tsx`, `reason-wasm.ts` |
| BM25 full-text via magic predicates | Search box + ranked results | **`live-new-wasm`** — `text-playground.tsx`, `sparq-text-wasm.ts` |
| RSP-QL streaming windows | Window config + R/I/DSTREAM tick view | **`live-new-wasm`** — `rsp-playground.tsx`, `sparq-rsp-wasm.ts` |
| Vector kNN (`vec:nearest`) | Embedding viz + kNN results | **`walkthrough`** — `sparq-vectors` opt-in native, not in the lean wasm; captured output today |
| GeoSPARQL (`geof:`, R-tree, topology) | Map overlay + spatial-join results | **`walkthrough`** — `sparq-geo` native-only, server `geo` feature off by default |
| GenAI / NLQ (schema-card / VoID → NL→SPARQL) | Schema card + NL prompt → generated SPARQL | **`walkthrough`** — `sparq-nlq` / `sparq-introspect` native, needs a model backend |
| ZK query proofs (BGP+FILTER, attestation, revocation) | Commit → prove → verify, in-tab | **`live-bbjs`** — `zk-car-hire.tsx`, `zk-prover.ts` via `@noir-lang/noir_js` + `@aztec/bb.js` UltraHonk. **Not externally audited** |
| MPC federation (additive sharing) | Multi-party threshold demo | **`live-sim`** — `mpc-demo.tsx`, `mpc-sim.ts` — a faithful JS illustration, **not** the native `sparq-mpc` protocol, no CSPRNG, **not** live MPC |
| Access control (Solid WAC/ACP) FROM NAMED restriction | (user, app)-pair → different result sets | **`live`** (engine restriction) — `solid-pairs-demo.tsx`; ACP decision materialized at build time |
| HTTP server: SPARQL Protocol, GSP, `/metrics`, WS+SSE subscriptions, VoID/Service Description | Connect-to-endpoint mode + live subscription stream + health panel | **`live` (against the user's own server)** — SHIPPED in the GUI/site as endpoint mode (`sq-2mke`, `connect-panel.tsx`), subscriptions (`sq-9ij6`, `subscriptions-view.tsx`), health (`sq-he72`). The static Pages demo of these stays a `walkthrough`; the app drives a real endpoint |
| Federation client (TPF/brTPF/SPARQL, pushdown) | Multi-source federation plan + per-leaf fan-out | **`soon`** — `sparq-fedclient` native, feature-gated; no GUI surface yet |
| Usage-control policy (ODRL) conflict/containment | Policy editor + conflict/refinement view | **`soon`** — `sparq-policy` (`contains` / `detect_conflicts`); no GUI |
| PROV-O lineage of derived data | Lineage graph of a CONSTRUCT derivation | **`soon`** — `sparq-prov` (`derive_construct`); no GUI |

The honest pattern: everything in the **lean wasm bundle** (core SPARQL, data formats,
SHACL) is `live`. Everything that is an opt-in **native crate not compiled to wasm**
(vector, geo, nlq, fedclient, policy, prov, native MPC) is `walkthrough` / `live-sim` /
`soon`, because the GitHub Pages host has no backend and those crates aren't in `js/wasm`.

### MVP — a credible workbench around the live engine (SHIPPED)

The MVP was **consolidation + the editor uplift + the Tauri shell**, almost entirely `live`
tier. All of it has **shipped**:

1. **Query editor uplift (SHIPPED, `sq-n5aw` + `sq-ixc3.1`).** The plain `<textarea>` is now
   a real code editor (`site/src/components/sparql-editor.tsx`) with SPARQL syntax
   highlighting, prefix awareness, keyword/example completion, and JSON-LD highlighting in
   editor + results. The highlighters live in `@sparq/client`.
2. **Results: table + raw SPARQL-JSON + N-Triples + CSV/TSV export (SHIPPED, `sq-x0kp`).**
3. **Dataset load / manage panel (SHIPPED, `sq-daru`).** `repl-dataset-panel.tsx`: built-in
   picker, upload, URL, merge, dataset viewer, **and a named-graph list with per-graph
   triple counts**. HDT upload stays out of scope (native-only).
4. **Keep the showcase surfaces exactly as tiered (ongoing honesty discipline).** SHACL /
   inference / full-text / RSP stay `live` / `live-new-wasm`; ZK stays `live-bbjs`; MPC /
   Solid stay `live-sim` / `live`. **Never silently upgrade a `walkthrough` to look live.**
5. **The Tauri desktop shell itself (SHIPPED scaffold, `sq-2e93`).** The frontend is wrapped
   in a Tauri 2 webview with the env-switched `basePath` (`sq-9zjy` made the COOP/COEP
   service-worker registration basePath-aware), and the native engine is linked with a
   command path that loads a file from disk and runs a query — proving the direct-Rust-link
   embedding end to end. Note the standing caveat in `gui/README.md`: this is a
   **CI-validated scaffold**, and the native `save`/`open` persistence path is a follow-up,
   not yet wired (it is the backbone of workspace snapshotting, §2a).

### Phase 2 — connect to a running `sparq-server` (SHIPPED)

The server API already existed (`crates/sparq-server/src/http.rs`): `/sparql` (GET + both
POST forms), `/sparql/graph` + `/graphs/{*path}` (Graph Store read **and** write),
`/subscriptions` (WS) + `/subscriptions/sse` (SSE), `/health`, `/metrics`, `/admin/compact`,
and the opt-in `/.well-known/void`, `/tpf`, `/shacl/validate`. The GUI work was a client +
auth/CORS UX, not new engine work — and it has **shipped**. This whole phase is the
**server-connect mode** the headline framing (#757) marks as an explicitly-KEPT secondary
use-case, NOT removable:

6. **Endpoint mode (SHIPPED, `sq-2mke`).** `connect-panel.tsx` runs the *same* editor against
   any SPARQL 1.1 Protocol endpoint with an optional bearer token. The wire logic lives in
   `@sparq/client`'s endpoint module; the panel renders the form, the classified
   connection-safety warnings, and a "Test connection" `ASK {}` result. The token is sent
   only in the `Authorization` header and never logged. The server's constant-time Bearer
   write gate (`--auth-token`) / optional read gate (`--auth-token-read`) and the WebSocket
   `Sec-WebSocket-Protocol: bearer.<token>` handshake are honoured.
7. **Live subscriptions view (SHIPPED, `sq-9ij6`).** `subscriptions-view.tsx` consumes
   `/subscriptions/sse` (or WS) and streams result deltas as the dataset mutates — the
   standout "live" demo a static site fundamentally cannot do.
8. **`/metrics` + Service Description panel (SHIPPED, `sq-he72`).** Renders the Prometheus
   `/metrics` and the VoID / SPARQL Service Description as a "server health / capabilities"
   view.

The GUI must respect the server's security posture (`crates/sparq-server/README.md`): **no
auth by default** (anyone reaching the port reads and writes), loopback bind by default
(non-loopback refused unless the auth×bind matrix is satisfied, `--allow-remote`), and a
**strict SERVICE-egress allowlist** that refuses all federation before any network call
when empty (the default). The GUI surfaces these as connection-safety UX, never bypasses
them.

### Phase 2.5 — the workspace model + per-workspace features (proposed; the next track)

This is the **new core track** this revision adds (§2a/§2b), the spine of the
embedded-native app. The impl beads already exist:

a. **Persistent cross-session workspace model + Tauri local persistence (`sq-atb0`)** — the
   foundational concept that **blocks** the per-workspace feature beads (per-workspace data
   import local + URL, saved SPARQL editor state, switch/isolate). §2a.
b. **Per-workspace inference-mode toggles (`sq-tp1m`)** — RDFS/OWL-RL/N3 on/off wired to the
   embedded engine; `live-new-wasm` tier. Depends on (a).
c. **In-workspace NLQ gated on a user-supplied API key (`sq-96o1`)** — the native
   `sparq-nlq` loop fronted by the user's key; model-backed/native tier, NOT a silent
   live-upgrade of the captured `walkthrough`. Depends on (a).
d. **Credential/Solid handling via feature-customised workspace types (`sq-tlo2`)** — the
   §2b proposal; itself design-first, then impl beads. Relates to `sq-1s2.5`.
e. **Drag-drop / import Verifiable Credentials and query over them (`sq-3p0z`, #822)** — the
   VC workspace's import + query surface; `live` query, ZK-over-VCs explicitly later +
   caveated.

### Phase 3 — visualisation + heavier showcases going live (proposed)

9. **Graph/triple visualisation of results (`sq-lyp8`, `live`).** CONSTRUCT/DESCRIBE already
   returns N-Triples in-tab; render it as a node-link graph. Pure client-side over existing
   output.
10. **Vector + Geo go `live-new-wasm` (`sq-zeai`).** Portability spikes to add
    `sparq-vectors` / `sparq-geo` to a wasm bundle (the surfaces' own tier comments name this
    as the blocker), upgrading kNN viz and the GeoSPARQL map overlay from `walkthrough` to
    live. Follows the proven `sparq-{shacl,reason,text,rsp}-wasm` pattern.
11. **Policy / PROV / Federation views (`sq-6v53`, `soon` today).** An ODRL policy editor +
    conflict / containment view over `sparq-policy`; a PROV-O lineage graph over
    `sparq-prov::derive_construct`; a federation-plan visualiser over `sparq-fedclient`.
    Each needs either a wasm port or endpoint mode first.
12. **Paginated / lazy results (`sq-9w4t`).** Page-wise query evaluation with a bounded
    read-ahead cache, for large result sets in the embedded store.

### Explicitly out of scope / stays caveated

- **ZK and MPC are not externally audited** — the GUI keeps the existing caveats. ZK
  proving is real (`live-bbjs`, in-tab UltraHonk) but circuits/soundness are not
  third-party audited; MPC on the site is a **JS simulation** (`live-sim`), not the native
  protocol. Never present either as production-grade.
- **No hard-coded performance numbers** in any GUI copy. A live in-tab `performance.now()`
  latency for the query just run is fine (measured, labelled); benchmark *claims* may only
  come from the repo's existing generated benchmark data.

## 3. Architecture & maintenance plan

### Monorepo structure (SHIPPED) — extend, don't fork

```text
packages/sparq-client/   # SHIPPED shared TS pkg: the ONE WasmStore type + loaders + endpoint client
js/                      # @sparq-org/sparq — npm wrapper; consumes @sparq/client
site/                    # Next.js site — consumes @sparq/client
gui/                     # SHIPPED Tauri 2 scaffold: frontend consumes @sparq/client;
                         #   gui/src-tauri/ is the Rust shell linking sparq-engine; gui/e2e/ is the e2e pkg
```

This is **shipped** (`sq-jpki`): the repo now has a root `package.json` with
`"workspaces": ["packages/*", "js", "site", "gui/e2e"]`, and `@sparq/client` is the one
shared TS surface (the `WasmStore` type, loaders, the SPARQL/Turtle/JSON-LD highlighters,
result shapes, and the endpoint client). Both `site/` and `gui/` import from it, so the GUI
is a **zero-new-copy** consumer — the hand-redeclared `WasmStore` drift the original draft
flagged as the single biggest liability is **eliminated**. The earlier "adopting npm
workspaces is a reviewable change" caveat is **resolved**: the adoption already happened.

### Engine embedding (desktop SHIPPED; mobile + persistence proposed)

- **Desktop (SHIPPED scaffold)**: `gui/src-tauri/` is a Rust crate that depends on the engine
  library crates directly. The Tauri IPC command layer (`src/engine.rs`) bridges the webview
  UI to the engine: nine commands (`load`, `query`, `query_quads`, `update_in_place`,
  `explain`, `explain_analyze`, `count`, `ask`, `store_size`) map onto the same operations
  the WASM `Store` exposes, but backed by a native `sparq_core::Graph` behind a `Mutex` in
  Tauri managed state. The command layer is unit-tested natively; the full `cargo tauri
  build` is CI-only (needs the webview system libraries). **Workspace persistence (proposed,
  §2a)** extends this: a persistence plugin + capability for workspace metadata, and
  optionally the engine's native `save`/`open` for a snapshot cache (not yet wired). The GUI
  Rust crate inherits the workspace's `forbid(unsafe_code)` posture (see the
  `unsafe-rust-attestation` skill) and the clippy-`-D warnings` discipline.
- **Mobile** (later track): the same native library cross-compiled via NDK (Android) and an
  xcframework (iOS) — net-new `uniffi` / `cargo-ndk` work that does not exist in the repo
  yet; spike-gated, possibly with a reduced per-target feature set.
- **Web fallback** (optional): the existing WASM bundle, inheriting the documented
  single-thread / no-mmap / <2 GiB ceilings — i.e. exactly the site as it is today.

### CI for the GUI (SHIPPED, `sq-bu69`)

The path-scoped per-platform `gui.yml` lane (build + lint + typecheck + clippy + tauri-driver
e2e) has **shipped**, modelled on `site-e2e.yml`. The design notes below are retained as the
record of how it was shaped. The GUI lane (`gui.yml`) is
**path-scoped, per-platform, and NOT a merge-queue lane**, mirroring `site-e2e.yml` and
`js.yml`:

- **Path scoping + triggers**: copy `site-e2e.yml`'s shape —
  `on: pull_request: paths: ["gui/**", ".github/workflows/gui.yml"]` plus
  `push: branches:[main]` with the same paths. As the `site-e2e.yml:8-14` comment
  documents, a Rust-/docs-only PR then skips the lane and the required `ci-summary / gate`
  "simply never waits on it" (cross-workflow `needs:` is impossible). The GUI lane stays
  off `merge_group` exactly as `site-e2e` / `js` / `pages` already are.
- **Per-platform build matrix**: build / lint / typecheck / e2e on `macos`, `windows`,
  `ubuntu`. Reuse the exact runner labels already proven in `release.yml`
  (`macos-14`, `macos-15-intel`, `ubuntu-latest`, `ubuntu-24.04-arm`, `windows-latest`) so
  the GUI CI matrix and the release matrix can't drift.
- **Build + lint + typecheck**: `next lint` + `tsc` are already the site's gate
  (`site/package.json:15`); the GUI adds `cargo build` / `cargo clippy` for
  `gui/src-tauri/`, inheriting the repo's `-D warnings` bar.
- **E2E**: Playwright is already wired for the site (`@playwright/test`,
  `site/playwright.config.ts`, the real `site/e2e/zk-prewarm.spec.ts`). Tauri's WebView e2e
  uses `tauri-driver` (WebDriver), not Playwright's Chromium — so the GUI e2e lane is a
  **new** harness, not a copy. Per-platform e2e is expensive; scope it path-scoped with
  `cancel-in-progress`, like `site-e2e.yml`.
- **Least-privilege**: every existing JS/site lane sets `permissions: contents: read` for
  the Scorecard TokenPermissions check; the GUI lane must do the same.

**Required-gate wiring.** `ci-summary.yml` is "the SINGLE required status check for branch
protection on `main`" and explicitly does **not** require the site lanes by name (it can't
— cross-workflow `needs:` is impossible, per its header). So the GUI lane, like
`site-e2e` / `js` / `pages` today, will be **advisory** unless the maintainer adds it to the
branch-protection required-contexts list — a `needs:user` repo-settings action.

### Releases (proposed, riding `sq-v286`)

The release plumbing the GUI rides already exists and is sophisticated; the GUI extends the
matrix rather than inventing a pipeline:

- **Trigger**: `release.yml` fires on `push: tags: ["v*"]` (releasing is a deliberate act).
  `sq-v286` proposes automating the tag via conventional-commits / semantic-release; the
  manual `v*` tag → full release already works today.
- **Matrix to extend**: `release.yml`'s matrix already builds the **exact desktop targets**
  the GUI needs (arm64/x64 darwin, x64 baseline/v2/v3/v4 + arm64 linux, win-x64/arm64). The
  GUI's desktop bundles (`.dmg` / `.msi` / AppImage/`.deb` / etc.) are *additional*
  artifacts on those same rows. **Honest gap**: that matrix has **no Android/iOS rows**
  today — mobile GUI artifacts are net-new, consistent with `sq-v286` saying a mobile app
  artifact "requires the GUI epic."
- **Supply-chain evidence rides for free**: every released artifact already gets a SLSA
  build-provenance attestation (`actions/attest-build-provenance`), a per-release CycloneDX
  SBOM + VEX (`scripts/gen-sbom-vex.sh`), `cargo-auditable` embedding, and `SHA256SUMS`
  coverage. A GUI bundle added to the `subject-path` inherits all of it (the
  `rust-supply-chain-attestation` skill documents this estate).
- **Code-signing is the real blocker, and it is `needs:user`.** `release.yml` attests
  provenance but does **not** sign binaries. Apple notarization, Windows Authenticode, and
  the Android keystore all require user-provided credentials (per `sq-v286`). Unsigned CI
  builds are fine for testing; a shippable, un-quarantined GUI is gated on the maintainer
  supplying signing creds. This cannot be designed away.

### Versioning in lockstep with the engine (feasible-now foundation + proposed policy)

- **Foundation exists**: the workspace is single-version —
  `[workspace.package] version = "0.1.0"` (`Cargo.toml:18`), and `crates/sparq-wasm`
  uses `version.workspace = true`. `@sparq-org/sparq` (`js/package.json`) and the site are also
  at `0.1.0`. Lockstep is the current de-facto model.
- **Proposed policy**: the `gui/src-tauri` crate inherits the workspace version via
  `version.workspace = true` (exactly like `sparq-wasm`), and the GUI's `package.json`
  version is stamped from the same source the release tag drives. `sq-v286`'s scope item —
  Rust-workspace versioning via `release-plz` / `cargo-release` — is the tool that would
  bump `workspace.package.version` and the JS `package.json`s together from
  conventional-commit history.
- **Honest tension**: with one workspace version, a breaking change in any crate bumps
  everything, including the GUI. That's the intended lockstep, but it means GUI releases are
  not independent — a deliberate trade for the maintainer to sign off in
  `research/release-ci-design.md`.

## 4. The Pages-root blocker that predates the GUI

`sq-svtt` is an **open, `needs:user`** product decision the GUI work must not trample:
GitHub Pages has one deploy slot, currently held by the showcase (`pages.yml`, live at
`/sparq/` + the bench dashboard at `/sparq/dev/`), and the mdBook-guide design (`sq-w9sr`)
also wants root. A GUI that wants its own web-hosted demo or download page is a *fourth*
claimant on that single slot. **Resolve `sq-svtt` first**; this design references it and
does not re-open it.

## 5. Bottom line (verdict)

- **The headline (maintainer #757):** the **primary** product is a downloadable, per-platform
  desktop app that embeds the native engine as a **direct Rust link** — full engine, **no
  deployed server required**. That decision is **shipped** (Tauri 2 scaffold, `sq-2e93`).
  **Server-connect mode is explicitly KEPT** as a secondary use-case (endpoint / subscriptions
  / health, `sq-2mke` / `sq-9ij6` / `sq-he72`), not removed.
- **Shipped (landed on `main`):** the Tauri 2 scaffold with the direct native-engine link
  (`gui/src-tauri/`, `sq-2e93`); the `@sparq/client` shared-TS package on repo-root npm
  workspaces, which **eliminated** the hand-redeclared `WasmStore` drift (`sq-jpki`); the
  editor uplift (`sq-n5aw`/`sq-ixc3.1`); the results SPARQL-JSON + CSV/TSV (`sq-x0kp`); the
  dataset/named-graph panel (`sq-daru`); endpoint mode + subscriptions + health (`sq-2mke` /
  `sq-9ij6` / `sq-he72`); the `gui.yml` CI lane (`sq-bu69`); GUI bundles on `release.yml`
  (`sq-8n1c`); the basePath-aware service-worker fallback (`sq-9zjy`).
- **Proposed (design-for-review — this revision's new surface):** the persistent
  cross-session **workspace model** + Tauri local persistence (`sq-atb0`); per-workspace
  **inference toggles** (`sq-tp1m`); in-workspace **NLQ** gated on a user-supplied key
  (`sq-96o1`); **credential/Solid feature-customised workspace types** (`sq-tlo2`) and the
  **VC import + query** surface (`sq-3p0z`, #822); plus the Phase-3 viz/heavy-showcase beads
  (`sq-lyp8` / `sq-zeai` / `sq-6v53` / `sq-9w4t`) and the still-greenfield **mobile** leg
  (no `uniffi`/`cargo-ndk` yet).
- **Hard `needs:user` blockers (cannot be designed away):** Apple notarization / Windows
  Authenticode / Android keystore credentials for a *distributable* GUI; the `sq-svtt`
  Pages-root decision; branch-protection required-contexts if the GUI lane is to be a *hard*
  merge gate; and the highest-uncertainty new piece, the **Solid OIDC/DPoP auth flow** in a
  Tauri webview (spike-gated). Mobile GUI artifacts are net-new — no rows in `release.yml`.
- **Caveats (honesty tiers preserved):** **in-workspace NLQ is model-backed** — live only
  when the user supplies their own key and a reachable model; otherwise the captured
  `walkthrough` (`IS_LIVE_LLM = false`, `genai.ts`) — and exec-accuracy is **not** a
  correctness guarantee. **Inference toggles are the native / in-browser-wasm tier**
  (`live-new-wasm`) — real closure, never a mock. The ZK/MPC surfaces (`zk-car-hire.tsx`,
  `mpc-demo.tsx`) and any VC verification / ZK-over-VCs are **research-grade, not externally
  audited** (external accredited-cryptographer sign-off pending, `sq-qhy4`; `sparq-mpc` is
  honest-majority semi-honest only) — never presented as production-trust; the
  `scripts/check-privacy-claims.sh` gate enforces this on user-facing copy. No performance
  numbers are asserted (this work box is non-canonical).

## Key file citations

- Epics: `sq-ixc3`, `sq-v286`, `sq-svtt`, `sq-w9sr` (`.beads/issues.jsonl`).
- GUI (shipped): `gui/README.md`, `gui/src-tauri/{Cargo.toml,tauri.conf.json,src/lib.rs,
  src/engine.rs,src/main.rs,capabilities/default.json}`, `gui/e2e/`, the root
  `package.json` (`workspaces`), `packages/sparq-client/`,
  `site/src/components/{connect-panel.tsx,subscriptions-view.tsx,repl-dataset-panel.tsx,
  sparql-editor.tsx,rdf-editor.tsx,solid-pairs-demo.tsx}`,
  `.github/workflows/gui.yml`.
- GUI impl beads (proposed track): `sq-atb0` (workspace model), `sq-tp1m` (inference
  toggles), `sq-96o1` (NLQ), `sq-tlo2` (cred/Solid), `sq-3p0z` (VC import, #822);
  shipped: `sq-2e93` / `sq-jpki` / `sq-n5aw` / `sq-x0kp` / `sq-daru` / `sq-2mke` / `sq-9ij6` /
  `sq-he72` / `sq-bu69` / `sq-8n1c` / `sq-9zjy`.
- Honesty tiers: `site/src/lib/genai.ts` (`IS_LIVE_LLM = false`), `site/src/lib/inference.ts`
  + `reason-wasm.ts`, `site/src/lib/solid-acl.ts`, `crates/sparq-solid/src/rewrite.rs`,
  `crates/sparq-nlq/`, the VC/ZK estate `crates/sparq-zk*/` (`sq-1s2.5`, audit pending
  `sq-qhy4`), `scripts/check-privacy-claims.sh`.
- Site: `site/package.json`, `site/next.config.ts`, `site/src/components/repl.tsx`,
  `site/src/lib/{sparq-wasm.ts,zk-prover.ts,mpc-sim.ts,http-server.ts}`,
  `site/src/data/surfaces.ts`, `site/README.md`, `research/feature-showcase-site-design.md`.
- Engine / WASM: `crates/sparq-wasm/{src/lib.rs,Cargo.toml}`,
  `crates/sparq-{reason,rsp,text,shacl}-wasm/src/lib.rs`, `js/package.json`,
  `js/wasm/sparq_wasm_bg.wasm`.
- Server: `crates/sparq-server/src/{http.rs,negotiate.rs,service_config.rs}`,
  `crates/sparq-server/README.md`.
- CI / release: `.github/workflows/{gui.yml,site-e2e.yml,js.yml,pages.yml,release.yml,ci-summary.yml}`,
  `scripts/gen-sbom-vex.sh`.
- Workspace: `Cargo.toml`, root `package.json`.
- Opt-in crates: `crates/sparq-{vectors,geo,nlq,fedclient,policy,prov,mpc,introspect,solid,zk,zk-compose}/`.
