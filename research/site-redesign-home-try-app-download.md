# Site redesign — Home, /try, /app, /download

> 🤖 SPARQ agent — design record (Fable tier). Designed from the orchestrator's prose brief
> (page inventories + problem list + design-principles digest), deliberately **without**
> opening `site/` sources, to keep the design pass on-tier. File paths named below are the
> brief's paths; implementers should verify exact names against the tree. Assumptions are
> flagged in §6.

North star: a scannable, polished, **self-proving** marketing-docs site. The one killer
artifact is the in-browser WASM query runner — everything else is caption. Dark-first
teal OKLCH tokens (`.bg-atmos`, `.bg-grid`, `.text-gradient`, `.display-1/2`, `.kicker`,
elevation scale) are the shared visual ground.

---

## 0. Cross-page ground fix (problem 7)

Mount `.bg-atmos` + `.bg-grid` **once**, in the root layout / AppShell wrapper, behind all
routes — not opt-in per page. Pages that mounted them individually drop their local mounts.
Result: the atmospheric teal ground is consistent on every route (including /app and
/download, which currently feel like a different site). Pages may opt *out* with a
`plainGround` prop if ever needed; none do today.

---

## 1. Home — the runner IS the hero

### 1.1 First-fold layout

Replace the current "hero → scroll to full REPL" pattern with a **split hero** that puts a
*lightweight* query runner in the first fold. The full `Repl` workbench no longer mounts on
home at all (problem 2) — home gets a new, small `HeroQueryRunner` component
(`site/src/components/home/hero-runner.tsx`), code-split + `ssr:false` like `ReplLazy`, but
containing only: a mini editor, a Run button, and a results table. No left rail, no connect
panel, no plan views, no subscriptions.

Grid at `lg`+: **left column ~5/12** — `.kicker` label (`IN-BROWSER SPARQL`), `.display-1`
`.text-gradient` headline (one line of promise, e.g. "A full SPARQL engine. In this tab."),
one-sentence sub-lede, then exactly **two CTAs**: primary "Open the full workbench →"
(`/try`), secondary ghost "GitHub". The old "Run a query now" scroll-CTA is deleted — the
Run button itself is now the third and primary interaction in the fold (stays within the
≤3-CTA budget). **Right column ~7/12** — the runner panel: an `--elevation-2` card with
`--teal-glow` edge, sitting on the shared `.bg-atmos` ground. On mobile the columns stack:
headline block, then runner.

The four-cell structural stat strip moves to a slim full-width band **directly under** the
hero (unchanged content: SPARQL forms / RDF formats / capability surfaces / "0 servers").
Sections 3 (flagship showcase) and 4 (capability-theme grid + tier legend) keep their
current content and order below it. Net section order: hero+runner → stat band →
"See it work" flagships → capability grid.

### 1.2 Runner panel anatomy

Top edge: two small tabs — **Query** (default) | **Data** — both editable, so the "sample
data pre-loaded" is inspectable and hackable without leaving the fold. Under the tabs, the
shared `SparqlEditor` (Query tab) or Turtle editor (Data tab), sized to ~8 visible lines,
mono, no line-number gutter (marketing density, not IDE density). Bottom bar: left — a
privacy chip in muted teal: "runs in your browser · nothing is sent to a server"; right —
the **Run** button: large, teal gradient (same treatment as the /try Run button),
keyboard-bound to Ctrl/Cmd+Enter. Below the editor card: the results area (§1.4).

No example-chip strip in the hero (that is /try's job); one sample is enough to prove the
point. A single text link under the results — "More examples in the workbench →" — carries
overflow demand to /try.

### 1.3 Default sample data + query

Chosen to be legible in five seconds and to produce an *obviously computed* answer (a join
+ aggregate + sort — not something a static site could fake credibly). 23 triples (11 on
the three people, 12 on the four components — count the `a` / `linesOfCode` / `lang`
components once expanded):

```turtle
@prefix : <https://sparq.dev/demo#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

:ada    a foaf:Person ; foaf:name "Ada"   ; :wrote :parser , :planner .
:grace  a foaf:Person ; foaf:name "Grace" ; :wrote :optimizer .
:lin    a foaf:Person ; foaf:name "Lin"   ; :wrote :storage , :optimizer .

:parser    a :Component ; :linesOfCode 4200 ; :lang "Rust" .
:planner   a :Component ; :linesOfCode 6100 ; :lang "Rust" .
:optimizer a :Component ; :linesOfCode 3800 ; :lang "Rust" .
:storage   a :Component ; :linesOfCode 5200 ; :lang "Rust" .
```

Default query:

```sparql
PREFIX :     <https://sparq.dev/demo#>
PREFIX foaf: <http://xmlns.com/foaf/0.1/>

SELECT ?name (SUM(?loc) AS ?linesOfRust) WHERE {
  ?person foaf:name ?name ;
          :wrote     ?component .
  ?component :linesOfCode ?loc .
}
GROUP BY ?name
ORDER BY DESC(?linesOfRust)
```

Three result rows, aggregated and ordered — visibly real SPARQL 1.1/1.2 (BGP join +
GROUP BY + SUM + ORDER BY; aggregation entered the language at SPARQL 1.1), answerable in
one glance.

### 1.4 Runner states

- **idle (pre-run)** — the results area shows the *expected* answer as a dimmed,
  clearly-labelled preview: a ghosted table with a centered overlay pill — "Preview —
  press **Run** to compute it live in your tab". This converts honesty into the hook:
  the site openly says "don't trust the screenshot; verify it". Never presented as a live
  result (no timing footer in this state).
- **running** — Run button disabled, spinner + "Running…". *First* run may include the
  engine-boot substate: if `loadSparq()` has not resolved yet, the button reads "Starting
  engine… (first run only)" with an indeterminate bar; subsequent runs skip straight to
  "Running…".
- **results** — typed table: header row from `head.vars`; IRI cells in teal mono
  (abbreviated by the demo prefixes), literals neutral, numeric literals right-aligned with
  a subtle datatype affordance on hover. Footer strip (mono, muted):
  `3 results · <t> ms · in-browser · 0 network requests` — timing measured around the
  `store.query()` call. "0 network requests" is the load-bearing proof line, **scoped to the
  query execution**: pressing Run makes no server round-trip — the engine evaluates entirely
  in-tab (the same posture as the site's existing "nothing is sent to a server" copy). It is
  not a claim that the page issued zero fetches overall — the wasm/JS bundle is a one-time
  page-load cost (warm-up, §1.5), fetched *before* Run, not per query. A small
  "Open in workbench →" action (§1.6) sits at the footer's right.
- **error** — a compact destructive-token strip *between editor and results*: first line of
  the engine's parse/eval error, plus line:col when derivable; the previous results (or the
  preview) stay visible but dimmed. Editing the query clears the strip. Errors never blank
  the panel.

### 1.5 WASM warm-up policy (problem 5)

Keep the wasm out of the critical path (perf budget) but stop gambling on
`requestIdleCallback` for the hero artifact:

1. `HeroQueryRunner` fires `prewarmSparqWhenIdle()` **immediately on hydration** of its
   lazy chunk (which is itself post-paint), not on browser-idle.
2. Additionally, any `pointerdown`/`focusin` inside the runner panel triggers an eager
   `loadSparq()` — intent-based warm-up, so by the time a visitor has read the query and
   reached for Run, the engine is typically instantiated.
3. Run never blocks on warm-up state: clicking always works; cold clicks get the
   "Starting engine…" substate (§1.4) instead of a skeleton.
4. The preview-table idle state (§1.4) means the fold is *never* a skeleton loader, even on
   slow connections — there is always a rendered, labelled artifact.

### 1.6 Handoff to /try

"Open in workbench →" serializes `{query, data, format:'turtle'}` to
`sessionStorage['sparq:handoff']` and navigates to `/try`; /try consumes and clears the key
on mount. Fallback for shared links: `/try#q=<base64url(query)>` (query only). This makes
the hero a genuine on-ramp instead of a dead end.

### 1.7 What home loses

- The full `Repl`/`ReplLazy` mount and its `#repl` section — deleted from home.
- The "Run a query now" scroll CTA — replaced by the in-fold Run.
- Any copy duplicated verbatim on /try (see §2.1); home keeps the *claim*, /try keeps the
  *tooling* voice.

---

## 2. /try — the workbench, in-browser by default, decluttered

/try already executes in-browser by default with no endpoint required; the redesign makes
that *legible* and demotes everything that suggests otherwise.

### 2.1 Slim header, distinct angle (problem 6)

Replace the hero strip (which currently repeats home's "real Rust engine compiled to
WebAssembly" pitch) with a one-line workbench header: `.kicker` `WORKBENCH`, `.display-2`
(not display-1) heading — "Your SPARQL workbench. No install." — and a single sub-line in
tooling voice: "Load your own Turtle/TriG/JSON-LD, run SELECT/CONSTRUCT/UPDATE, inspect
plans." Someone arriving from home has already been sold; this page's job is *do*, not
*decide*. The header band is short enough that the editor + Run sit above the fold on a
laptop viewport.

### 2.2 Run front-and-centre; EXPLAIN demoted to a split-button

Keep the teal-gradient Run as the sole primary action. Collapse the Run / EXPLAIN / ANALYZE
toolbar row into **one split-button**: `Run ▾` — default action "Run query"
(Ctrl/Cmd+Enter); the dropdown carries "Run EXPLAIN" and "EXPLAIN ANALYZE" (with their
existing plan-tree result views). One primary control instead of three peers; plan tooling
remains one click away for the users who want it.

### 2.3 Remote endpoint → an "Advanced" drawer (never required)

The `ConnectPanel` leaves the center column entirely. New placement: a collapsed disclosure
at the bottom of the right rail (or a toolbar overflow item) labelled
**"Advanced · Connect to a sparq-server"**. Collapsed by default; the endpoint URL field,
protocol notes, and `ServerHealthPanel` render only inside the expanded drawer. States:

- **disconnected (default)** — no URL field visible anywhere; the status bar shows a quiet
  chip: `Engine: in-browser · nothing leaves this tab`.
- **connected** — the status-bar chip flips to `Remote: <host> ●` with a disconnect action;
  the drawer stays available for health/details. Execution target follows the connection;
  disconnecting reverts to in-browser without losing editor state.

The in-browser path must never render an empty endpoint input or any affordance implying a
server is expected — that is the single biggest declutter *and* the honesty win.

### 2.4 Declutter rules

- **Left rail**: Workspace / Dataset / Graphs become an accordion with **Dataset open by
  default**; the rail collapses to icons (persistent-workbench-shell pattern) and its state
  persists.
- **Example chips**: cap the strip at one row (~5 chips) + a "More examples…" item that
  opens the Cmd-K palette pre-filtered to examples; no wrapping chip walls.
- **Right pane**: Results is the only always-on panel. `SubscriptionsView` renders only
  when at least one subscription exists; `ServerHealthPanel` only inside the Advanced
  drawer when connected. Empty panels never mount.
- **Handoff**: consume `sessionStorage['sparq:handoff']` / `#q=` per §1.6.

### 2.5 Keyboard spine

Ctrl/Cmd+Enter = Run; Cmd-K opens the palette (examples, panel toggles, "connect to
server…", format switch). The palette is the load-bearing prerequisite the design skill
names for any nav/panel collapse — ship it with, not after, the declutter.

---

## 3. /app — fix the deploy, then earn the nav slot

### 3.1 The txt-redirect is a cross-app soft-nav, not a stale export

> **Implementation note (reconciled with PR #1352, sq-vw3ax.11):** this record's original
> premise — a *checked-in, stale* `site/out/` that a fresh build heals — was **falsified by
> the repo during implementation**. `site/out/` is **gitignored** (`site/.gitignore: /out`)
> and rebuilt fresh on every deploy by `pages.yml`, so there is no committed export to drift
> and an export-parity gate would be dead machinery over a non-committed tree. The subsection
> below is corrected to the actual root cause.

The real mechanism: `/app` in production is served by a **separate Next.js app** (`gui/app`
— the hosted workbench, sq-vnd0i / Option B) that the Pages deploy builds with `build:web`
and **overlays at `/sparq/app/`**, deliberately replacing this site's own `/app`. The site
linked to `/app` with `next/link` (a **soft** SPA navigation). A soft nav across two
*distinct* Next builds fetches the foreign app's RSC Flight payload `/sparq/app/index.txt`
→ the browser lands on a raw `.txt`. (It reproduces only in the deployed overlay; `next dev`
and the e2e suite render the site's own `/app`, so it never showed locally.) Fix:

1. **Make the "App" nav slot a HARD, full-page navigation** — a plain
   `<a href="/sparq/app/">` (base-path-prefixed, trailing slash for `trailingSlash: true`),
   not `next/link`. The browser then loads the overlaid GUI's own HTML instead of chasing the
   site build's RSC payload. The legacy `/gui` redirect stub likewise becomes a hard
   `window.location` redirect.
2. **No export-parity gate is warranted** (superseded): `out/` is generated fresh per deploy,
   never committed, so there is no source-vs-deploy drift to guard. The `/app` *source* page
   is kept as the honest local/preview/lychee-target fallback (it never renders in production
   because of the overlay).

### 3.2 What /app should be

> **Implementation note (reconciled with PR #1352, sq-vw3ax.11):** this record's original
> recommendation — *remove* the "App" nav slot and rewrite `/app` to a "being built" bridge —
> **also rested on a premise the repo falsifies: the hosted web GUI has already shipped** and
> is deployed at `/sparq/app` (sq-vnd0i / the maintainer's Option B). So during implementation
> the slot was **kept** (removing it would regress a live feature and break
> `e2e/site-nav.spec.ts`, which hard-asserts the "App" slot), made a hard-nav link (§3.1), and
> the `/app` source copy was corrected from "coming soon" to the honest "the hosted GUI is
> **live** at `/sparq/app`" — a "being built" bridge would have re-introduced a now-false
> claim. The original spec below is retained for the record but is superseded on these points;
> de-listing the GUI for a maturity/trust reason remains the maintainer's separate call.

~~Keep the route as the honest bridge the source already is — but **remove "App" from the
top-bar nav until the hosted web GUI (sq-rclb8 / epic sq-ixc3) actually ships**.~~ A nav slot
that lands on "coming soon" erodes trust in the whole bar (problem 4) and violates the
"fewer top-level entries" principle; a live route without a nav slot costs nothing and
keeps deep links working.

Page spec (low-key bridge, one screen): shared atmos ground; `.kicker` `HOSTED APP`;
`.display-2` heading "The hosted sparq app is being built."; warning-tier honesty badge
"Hosted web app — in development" with a collapsed details block linking the tracking
epic. Below, the existing two cards, re-weighted: **primary** (teal, elevation-2) — "Use it
now, in this tab → /try" (the workbench is the real answer to what /app visitors want);
secondary — "Install the desktop GUI → /download". No other content. When the hosted app
lands, this route becomes its entry shell and the nav slot returns.

---

## 4. /download — direct per-asset downloads

### 4.1 Mechanism: `releases/latest/download/<alias>` direct links

GitHub serves a **direct file download** (302 to the asset CDN) at
`https://github.com/sparq-org/sparq/releases/latest/download/<asset-name>` — no release page
interstitial — *provided the asset name is stable across versions*. The current
`sparq-gui-<version>-<label>-…` naming breaks that. Fix in two halves:

1. **Release pipeline**: publish **version-agnostic alias assets** alongside the versioned
   ones on every release: `sparq-gui-arm64-darwin.dmg`, `sparq-gui-x64-darwin.dmg`,
   `sparq-gui-win-x64.msi`, `sparq-gui-x64-linux.AppImage`, `sparq-gui-x64-linux.deb`,
   plus `sparq-cli-<platform>.tar.gz` / `sparq-server-<platform>.tar.gz` aliases (same
   bytes, second upload or duplicate name — GitHub allows distinct names only, so upload
   the aliased copy). Versioned names remain for provenance/archive.
2. **Site**: every platform button's `href` becomes the corresponding
   `releases/latest/download/<alias>` URL — a one-click direct download that never needs a
   site rebuild when a new version ships. The CLI and sparq-server cards use the same
   mechanism (tarball aliases) instead of `LATEST_RELEASE_URL`.

### 4.2 Version / size / checksum display

The static export can't know the current version, so fetch
`https://api.github.com/repos/sparq-org/sparq/releases/latest` **client-side** on page load
(the page is already a client component with OS detection): render `v<x.y.z>`, asset size,
and — if the release publishes a `SHA256SUMS` asset (add to the pipeline) — a copyable mono
checksum line inside each card's existing collapsed "First-launch instructions" details
block. Failures degrade gracefully: buttons keep working (the latest/download URL needs no
API), only the metadata line is omitted.

### 4.3 No-release-yet state (current reality)

Until the first tagged release exists, the same API call returns 404: render every download
button in a disabled-styled state — "No release yet — watch releases" — where that link
(to the releases page) is the **only** GitHub-page link retained, and keep the current
honest copy. The moment a release ships, the page flips live with zero code change. The
unsigned-builds banner stays front-and-centre in both states.

### 4.4 Card layout

Keep progressive OS detection, but *promote* the detected platform: a full-width primary
row at the top — "For your Mac (Apple Silicon)" with the big teal direct-download button
(`Download sparq-gui.dmg · v0.x.y · <size>`), first-launch details collapsed beneath. The
remaining platforms render as a compact 3-up grid of smaller cards below. Then the
CLI/server pair, then the existing "No install needed → /try" closer (which now also
mirrors home's privacy chip).

### 4.5 Rejected alternatives

- **Committing binaries into `site/public/`** — rejected: hundreds of MB per release in
  the repo/Pages artifact, LFS cost, stale-copy risk; `public/` stays wasm-only.
- **Keeping `releases/latest` page links** — rejected as primary affordance: an
  interstitial that asks the visitor to find the right asset among many defeats the
  per-platform card. Retained only as the disabled-state fallback (§4.3).
- **A site-baked version constant** — rejected: guarantees staleness in a static export.

---

## 5. Implementation cut-lines

Independently shippable, in value order (orchestrator to file/schedule beads):

1. Rebuild + redeploy the static export (unbreaks /app today) + export-parity gate (§3.1).
2. Root-layout atmos ground (§0) — one-file change, whole-site consistency.
3. `HeroQueryRunner` + home first-fold swap + remove full REPL from home (§1).
4. /try declutter: header, split-button Run, Advanced connect drawer, conditional panels
   (§2) + handoff plumbing shared with (3).
5. /download direct-download rework (§4 site half) + release-pipeline alias/checksum
   assets (§4 pipeline half — separate bead, different owner surface).
6. Nav: drop the App slot until sq-rclb8 (§3.2) — trivial, ship with (2).

## 6. Assumptions (prose-only design pass)

- Component/file names for *new* artifacts (`hero-runner.tsx`, storage key, prop names)
  are suggestions; existing paths are quoted from the brief, not verified against the tree.
- `SparqlEditor` and the results-table cell renderers are reusable outside the full `Repl`
  without dragging in the workbench shell; if not, extract shared primitives first.
- The GitHub Pages basePath is `/sparq` and the wasm assets stay in `site/public/wasm/`.
- The release workflow can upload additional alias assets + a `SHA256SUMS` file.
- `sessionStorage` is acceptable for the home→/try handoff on a static export (same
  origin); the `#q=` fragment covers cross-tab/shared-link cases.
