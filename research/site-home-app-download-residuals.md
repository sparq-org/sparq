# Home / /app / /download residuals — grounding + FRONT decomposition (sq-vw3ax.11)

> 🤖 SPARQ agent — FRONT decomposition record (Fable tier, [FABLE-5], 2026-07-10).
> This pass grounded the epic against the **live deploy** (`https://sparq.jeswr.org`) and the
> **actual code** (site/src, .github/workflows, the GitHub Releases API) — unlike the
> original prose-only design pass ([site-redesign-home-try-app-download.md](site-redesign-home-try-app-download.md)),
> whose interface design this record does NOT redo. Verdict up front: **three of the epic's
> four items are shipped and live-verified; the residual is four narrow gaps**, decomposed
> below into four disjoint child beads.

## 1. Epic premise, corrected against reality

`sq-vw3ax.11` (maintainer, 2026-07-02) asked for: (1) an in-browser WASM runner + Run button
on Home and a `/try` that needs no live endpoint, (2) a fix for `/app` landing on
`…/app/index.txt`, (3) `/download` buttons that directly download the right file. Since the
epic was written, the ground moved four times:

- **`/try` was removed entirely** (maintainer directive 2026-07-05, sq-4hiqe): it is now a
  permanent hard client-redirect stub to `/app` (`site/src/app/try/page.tsx`). Every
  "/try defaults to in-browser" sub-item is **moot**.
- **The site cut over to a custom domain** (sq-uj38w): it serves at the `sparq.jeswr.org`
  root. The epic's `jeswr.github.io/sparq/...` URLs are dead (live-verified 404), which also
  means any *remaining in-site links* to that origin are dead links (§4, R4).
- **The repo moved to `sparq-org/sparq`**. GitHub's rename redirect keeps
  `github.com/sparq-org/sparq` links working, but ~50 references across `site/src` (including
  the download page's `REPO_URL`/`API_LATEST`) point at the old name.
- **Only a pre-release exists** (`v0.1.0-dev.3`, 2026-06-21, prerelease=true — predating the
  current release pipeline), so `releases/latest` 404s. Release PR **#1084 (v0.1.0)** is
  open; the moment it merges and tags, the alias gap in §3/R2 becomes user-visible.

### Live-verified status per epic item

| Epic item | Status | Evidence |
| --- | --- | --- |
| (1a) Home in-browser runner + Run button | **Shipped, live** | `sparq.jeswr.org/` hero renders the two-tab Query/Data editor + teal Run button; Run computes in-tab via the wasm engine (`site/src/components/home/hero-runner.tsx`, lazy chunk + idle prewarm per sq-vw3ax.9) |
| (1b) `/try` in-browser by default | **Moot** | `/try` removed (sq-4hiqe); hard redirect stub → `/app` |
| (2) `/app` txt-redirect | **Fixed for every path except one** | Live `/app/` serves the real gui/app workbench (pages.yml overlay, sq-vnd0i); nav bar / hero / capability CTAs / download / stubs are all hard anchors. **Residual:** the Cmd-K palette still `router.push`es `/app` (§4, R1) |
| (3) `/download` direct buttons | **Page shipped; pipeline half missing** | `download-client.tsx` implements `releases/latest/download/<alias>` buttons + API enrichment + no-release state. **Residual:** release.yml never publishes the aliases (§3, R2), and prerelease-only reality means every button currently degrades to "No release yet" despite real assets existing (§4, R3) |

## 2. The four design questions, settled

**(a) How does the Home Run button share the WASM engine with /try?** Moot as posed — /try
is gone. The real architecture: the Home runner owns a code-split chunk
(`hero-runner-lazy.tsx`, `ssr:false`) over the site's memoised `loadSparq()` +
`prewarmSparqWhenIdle()` (`site/src/lib/sparq-wasm.ts`), so the engine wasm never rides the
first-load bundle. The `/app` workbench is a **separate Next.js build** overlaid at `/app/`
with its own wasm copy; navigation between them is a hard full-page load, so *instance*
sharing is impossible by construction and *byte* sharing (a common wasm URL across the two
builds) is **rejected**: it would couple the two deploys' asset layout and cache
invalidation to save one cold fetch on a cross-app hop. No bead.

**(b) What is /try's default dataset/query with no live endpoint?** Moot for /try. The Home
runner's default is `site/src/data/hero-sample.ts`: a small inline Turtle sample + a
join/`SUM`/`ORDER BY` query chosen so the answer is visibly computed (not fakeable by a
static page), with the idle state honestly labelled a preview. Keep as-is. No bead.

**(c) /app txt-redirect — root cause + fix shape.** Root cause (established during
implementation, reconciled in the prior record §3.1): `/app` is served by a *different*
Next.js build, so a `next/link`/`router.push` soft navigation from the site fetches the
foreign RSC Flight payload `/app/index.txt` instead of loading the GUI's HTML. Fix shape:
**every** navigation to `/app` must be a hard, full-page navigation (plain `<a>` with
`withBasePath("/app/")`, or `window.location.assign`). That rule landed at every entry
point except one residual: `command-palette.tsx` registers `/app` in `TOP_PAGES` and its
`go()` handler does `router.push(href)` for all entries — selecting "App" via Cmd-K is the
last live instance of the bug class (with a stale "hosted web app coming soon" blurb, and a
dead `slug === "try"` special case). → **R1**, plus an e2e regression guard so the class
stays dead.

**(d) Where do direct-download buttons source artifact URLs?** Decision (confirming the
prior record §4.1, now with the pipeline reality): **version-stable alias assets** at
`releases/latest/download/<alias>` remain the primary mechanism — static hrefs, no per-release
site rebuild, no API dependency for the click itself — with the client-side
`releases/latest` fetch as *enrichment only* (version, size, sha256 digest). Two hard
corrections from grounding:

1. `release.yml`'s `Stage GUI bundles` step emits **only versioned names**
   (`sparq-gui_<version>_<arch>.<ext>`; CLI archives are `sparq-cli-<version>-<tier>`).
   The alias-publication half designed in the prior record **never landed** — so every
   direct button 404s even once a stable release exists. → **R2** (fail-closed: the release
   job must fail if a site-referenced alias is missing).
2. The CLI archive **already contains both binaries** (`sparq-cli` + `sparq-server`,
   build-matrix.yml archive mode), so the site's separate `sparq-server-<token>.<ext>`
   alias has **no source asset** and never will under the current pipeline. The site must
   sell one combined "CLI + server" archive honestly instead. → folded into **R3**.

## 3. The alias contract (normative for R2 + R3)

Aliases are extra uploads of the same bytes (GitHub release assets cannot be symlinked);
versioned names remain the provenance/archive names and stay in `SHA256SUMS` — alias
entries are appended to `SHA256SUMS` too, and each alias inherits a per-asset `digest` via
the Releases API. Where a platform has multiple micro-architecture tiers, the alias maps to
the **baseline** tier (portability-first: it runs on every machine of that OS/arch; power
users can still pick a tier from the Releases page).

| Alias (site-facing, version-stable) | Source asset (versioned) |
| --- | --- |
| `sparq-gui-arm64-darwin.dmg` | `sparq-gui_<v>_aarch64.dmg` |
| `sparq-gui-x64-darwin.dmg` | `sparq-gui_<v>_x86_64.dmg` |
| `sparq-gui-win-x64.msi` | `sparq-gui_<v>_x64.msi` |
| `sparq-gui-x64-linux.AppImage` | `sparq-gui_<v>_amd64.AppImage` |
| `sparq-gui-x64-linux.deb` | `sparq-gui_<v>_amd64.deb` |
| `sparq-gui-arm64-linux.AppImage` | `sparq-gui_<v>_arm64.AppImage` |
| `sparq-gui-arm64-linux.deb` | `sparq-gui_<v>_arm64.deb` |
| `sparq-cli-arm64-darwin.tar.gz` | `sparq-cli-<v>-arm64-darwin.tar.gz` |
| `sparq-cli-x64-darwin.tar.gz` | `sparq-cli-<v>-x64-darwin.tar.gz` |
| `sparq-cli-x64-linux.tar.gz` | `sparq-cli-<v>-x64-baseline.tar.gz` |
| `sparq-cli-arm64-linux.tar.gz` | `sparq-cli-<v>-arm64-linux.tar.gz` |
| `sparq-cli-win-x64.zip` | `sparq-cli-<v>-win-x64-baseline.zip` |

Dropped from the site: `sparq-server-<token>.<ext>` (no source asset; the `sparq-cli-*`
archive ships both binaries and the download card says so). The contract check in R2 treats
the **site's usage as the source of truth**: every alias referenced by
`site/src/app/download/download-client.tsx` must exist in the release's asset list, fail-closed
(extra aliases are allowed; missing ones fail the release job). That is why R3 (which
finalises the site's alias usage) **blocks** R2.

### Prerelease fallback (R3)

Today — and whenever only prereleases exist — `releases/latest` 404s and the page degrades
every control to "No release yet", even though real assets exist. Decision: on a
definitive `latest` 404, fall back to the **newest release including prereleases**
(`GET /releases?per_page=1`), render per-asset **direct** `browser_download_url` buttons,
and label the state plainly as an **unsigned development pre-release** (the existing
unsigned-builds banner stays; no capability inflation — a card whose platform has no asset
in that release degrades to the Releases-page link, per-card). Because prerelease asset
names are versioned and use pipeline arch tokens, matching is by pinned per-card pattern
(e.g. GUI macOS arm64 ⇒ `^sparq-gui_.*_aarch64\.dmg$`; CLI Linux x64 ⇒
`^sparq-cli-.*-x64-baseline\.tar\.gz$` — baseline-tier preference as above). In the `ready`
(stable-release) state the same fail-closed rule applies: a button renders as a direct
download only if its alias (or matching versioned asset) is present in the release's asset
map; on API *error* (rate-limit etc.) the static alias buttons stay optimistically live, as
already documented in the component.

## 4. Residual gaps → child beads (disjoint file ownership)

All four beads are children of `sq-vw3ax.11`. File ownership is **exclusive** — no file
appears in two beads. The three site beads are additionally **chained** (R1 → R3 → R4) to
honour the one-in-flight-per-surface partition, and R3 → R2 orders the alias contract.
None of the owned files overlap the recently merged #1803/#1807 site work
(`site/src/components/benchmarks/*`, `site/src/app/surface/inference/page.tsx` is owned by
R4 only for a link-string sweep and both PRs are already on `origin/main`).

| Bead | Surface | Tier | Owned files | Gap |
| --- | --- | --- | --- | --- |
| R1 | site | sonnet | `site/src/components/command-palette.tsx`, `site/e2e/command-palette.spec.ts` | Cmd-K "App" soft-navs (`router.push`) across the two Next builds — the last live txt-redirect path; stale "coming soon" blurb; dead `"try"` special case; no e2e guard for the hard-nav rule in the palette |
| R2 | ci-infra | sonnet | `.github/workflows/release.yml`, `scripts/check-release-aliases.sh` (new) | Version-stable alias assets never published; must be fail-closed against the site's alias usage (§3) and land **before** the v0.1.0 tag (#1084) |
| R3 | site | sonnet | `site/src/app/download/download-client.tsx`, `site/e2e/download-page.spec.ts` | Prerelease fallback + per-card fail-closed asset matching (§3); drop the sourceless `sparq-server-*` aliases for one honest combined CLI+server card; `REPO_URL`/`API_LATEST` → `sparq-org/sparq` |
| R4 | site | haiku | The ~23 `site/src` files with stale origins **excluding** the two owned above, + a new static guard test | Dead `jeswr.github.io/sparq` links (live 404) and stale `github.com/sparq-org/sparq` references → `sparq.jeswr.org` / `sparq-org/sparq`; guard test keeps the dead origin from returning |

Invariants and acceptance tests are carried on each bead (bd); the shared gate for the site
beads is the epic's: green static export + lint + typecheck + e2e.

## 5. Rejected alternatives

- **Sharing one wasm URL between the site and gui/app builds** — rejected (§2a): deploy
  coupling for one cold fetch on a hard cross-app navigation.
- **API-only download buttons (no aliases)** — rejected: makes every click depend on
  `api.github.com` availability and rate limits; the alias mechanism keeps the primary
  click path static. The API stays enrichment + fallback.
- **Renaming release assets to version-agnostic names only** — rejected: loses versioned
  provenance names in `SHA256SUMS`/attestations; aliases are additive copies instead.
- **Publishing `sparq-server-*` alias copies of the combined archive** — rejected: doubles
  upload bytes to paper over naming; honest fix is the site copy (one combined card).
- **Waiting for v0.1.0 instead of a prerelease fallback** — rejected: leaves every download
  control dead-ended today, and the fallback stays correct after v0.1.0 (the `latest` path
  simply takes precedence again).

## 6. Sequencing and follow-ups

Order: **R1** (live bug, no deps) → **R3** (site download) → { **R2** (release aliases,
needs R3's final alias usage), **R4** (link sweep) }. R2 should merge **before** release
PR #1084 is tagged, else v0.1.0 ships without aliases and the stable-state direct buttons
404 (R3's fail-closed ready-state rendering limits the blast radius to honest degradation).
Cutting v0.1.0 itself — and whether to first dispatch a developer test release to exercise
the alias path end-to-end (release.yml has a `workflow_dispatch` mode for exactly this) —
is the maintainer's call; flagged via a proceed-and-document issue rather than a bead.

Adjacent artefact noted while grounding: parent-epic child `sq-vw3ax.10` ("/try Graph
view", P3) has been **resolved by implementation** — the node-link graph view is now
live in the home hero runner (the surviving primary surface for result visualization;
`site/src/components/home/hero-runner.tsx` lines 440–465, Table | Graph toggle active
when results are entity-relationship shaped).
