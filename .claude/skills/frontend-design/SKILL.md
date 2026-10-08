---
name: frontend-design
description: 'How sparq designs its TWO distinct frontends — the explanatory marketing-docs WEBSITE (site/, Next.js static export) and the operational desktop GUI (gui/, Tauri 2 + native engine). Covers the methodology that keeps them distinct, scannable, and honest: information architecture + content-reduction (fight "too much text"), explanatory-site patterns (show-don''t-tell, progressive disclosure, one killer artifact), operational-GUI patterns (workbench shell, command palette, keyboard-first), the shared visual-design system, and the a11y/perf budget. Use when restructuring site/ navigation or pages, building/extending gui/, deciding whether content belongs on the site vs the GUI vs a SKILL.md, adding a surface, cutting a dense page, or reviewing a frontend PR. Grounded in research/website-redesign.md + research/gui-design.md + the real site/src tree.'
---

# Frontend design (sparq)

sparq ships **two** frontends that are deliberately **different products**, not two
skins of one app. Conflating them is the single biggest design failure to avoid — and the
one the maintainer flagged: *"the website has too much text; it is overwhelming."* The fix
is not "trim words"; it is to keep each frontend doing only its own job.

| | **Website** (`site/`) | **GUI** (`gui/`) |
|---|---|---|
| Audience | the RDF/SPARQL-**curious** dev evaluating *"is this real, should I care?"* | the dev who already decided and wants to **do work** |
| Job | persuade + route + prove with a few live demos; link out for depth | operate the engine as a real tool over persistent local data |
| Stack | Next.js `output: "export"`, React 19, Tailwind v4, radix/shadcn; GitHub Pages | Tauri 2 webview + **direct native-Rust engine link**; desktop bundle |
| Shape | a small set of **pages** you read top-to-bottom | a **workbench shell**: rails + tabs + command palette, no marketing pages |
| Content | marketing-docs: one-liner → demo → link to repo/SKILL.md/papers | operational: import → query → inspect → operate surfaces as tools |
| Depth lives | `crates/*/README.md`, `skills/*/SKILL.md`, `research/`, `/papers` | the engine itself + a Help → opens the website in the system browser |

> **Litmus test for any new frontend work:** *"Does this help someone DECIDE
> (website) or DO (GUI)?"* If a change makes the website more of a workbench, or the
> GUI more of a brochure, it is going the wrong way. The GUI must **never just wrap the
> marketing site** (today `gui/` is a `src-tauri` scaffold whose `frontendDist` points at
> `site/out` — that is a placeholder, not the design).

Design records (read these for the full spec — this skill is the reusable *method*):
`research/website-redesign.md` and `research/gui-design.md`.

---

## 1. Information architecture + content reduction (fight "too much text")

The overwhelm is structural, not verbal. Audited, the *same* ~16 surfaces appear in **up
to three places at once** (`site/src/components/layout/app-shell.tsx`): a persistent
`w-64` sidebar rendering the full 6-group/16-surface tree (`sidebar-nav.tsx`) **and** a
top-tab bar (Showcase/Benchmarks/About) **and** the landing page re-renders all surfaces as
cards. Cutting prose does not fix a tripled nav. **Cut structure first.**

Principles (apply in this order):

1. **ONE nav, not two.** A persistent full sidebar tree *and* a top-tab bar split attention
   and imply two mental models. Pick one primary structure (the website chose a slim top
   bar of **5 content destinations**); the other becomes utility-only (search / GitHub /
   theme) or is deleted.
2. **Cut surface COUNT in the nav, not just words per page.** 16 flat entries overwhelm;
   group them under a handful of capability **themes** and let the user navigate *concepts*
   then drill to the feature. Re-group ONCE at the single source (`site/src/data/surfaces.ts`
   `GROUPS`) so the nav and every landing grid update together.
3. **Progressive disclosure in three layers.** (1) **SCAN** — one sentence + one tier
   badge + one runnable thing, above the fold; (2) **UNDERSTAND** — a tight capability list
   + the demo; (3) **VERIFY/DEEP** — caveats, reproduction commands, threat-model hedging,
   links to crate/SKILL — behind a closed `<details>`. Never make a hurried reader read the
   deep layer to use the surface.
4. **Earn each section; don't repeat a template.** A rigid template that *forces*
   intro + N capability cards + runsNote + caveat on every page guarantees uniform density:
   a simple surface looks as heavy as a research one. Make every block **optional** and let
   weight follow importance.
5. **Centralize cross-cutting explanation once; reference it.** The "what runs where / how
   this runs" honesty story was told four times (hero, /about, every per-page runsNote,
   every badge tooltip). State the **tier model once**, let the badge be the per-page
   pointer, and shrink each per-page "How this runs" to one sentence + a link.
6. **Move reference-grade content OUT of the frontend.** Reproduction commands, the full
   capability enumeration, edge-case boundaries, and threat-model nuance are **reference**:
   they belong in `crates/<x>/README.md`, `skills/<x>/SKILL.md`, and `research/` — which
   already exist. The website persuades; the docs exhaust. Link, do not inline.
7. **A search / command palette is the real answer to "N surfaces is a lot."** A fuzzy
   index lets you SHRINK visible nav without burying anything — power users jump by name in
   0 clicks. On the website this is Cmd-K; in the GUI it is the spine of the keyboard-first
   model. **It is load-bearing: ship it BEFORE deleting the sidebar**, or the collapsed
   surfaces lose their only fast path.

Content-reduction is a *commitment*, not a vibe. A redesign must name **what to cut, what to
collapse, and what to move** (see `research/website-redesign.md` §"content-reduction plan").

---

## 2. Explanatory-site patterns (the website)

The website's strongest asset is the **live REPL** (`site/src/components/repl.tsx`) — a
real in-tab SPARQL engine, no mocks. Build every page around that energy.

- **ONE killer artifact beats ten paragraphs.** A developer trusts a runnable result far
  more than prose claiming it works. Lead with the demo/result; prose is the **caption**,
  not the lede. The hero already nails this — the failure was that *inner* pages reverted to
  prose-first (e.g. a GeoSPARQL intro of ~3 dense paragraphs + ~30 inline `<code>` spans
  *before* the walkthrough). Invert it everywhere.
- **Show, don't tell — lead with a runnable/copyable example.** The first interaction is
  *doing*, not reading. Copyable code (with a copy button) is itself a navigation aid: it
  tells a dev "you belong here."
- **One-line what-is-it above the fold, then proof.** A stranger should understand
  *what this is and why care* in one sentence, immediately followed by ONE compelling proof
  (a live query, a number). Do **not** front-load a multi-sentence honesty preamble; reduce
  it to one badge + a tier-legend strip.
- **≤3 canonical next steps, always visible.** Converge on a small fixed CTA set
  (Try it / Docs / GitHub). The hero gets ~2 buttons, not 20 equally-weighted links.
- **Scannability is typographic before it's editorial.** Walls of justified prose studded
  with inline `<code>` read as noise. Capabilities → a scannable list with a **bolded lead
  term**; "what's NOT covered" → a secondary/collapsed block, not a co-equal warning card
  competing with the demo.
- **Honesty can be terse — and is MORE honest terse.** A 5-clause hedging paragraph gets
  skipped; a one-line bold flag ("Research-grade — not externally audited") + a "Read the
  full caveat" `<details>` gets *read*. Keep the honesty-tier taxonomy
  (`Tier` in `surfaces.ts`) verbatim; the badge carries the per-page truth.
- **Triage by tier for prominence.** "Live in your tab" surfaces are the proof and should
  lead; "walkthrough" surfaces are credible-but-static and sit in a secondary tier or behind
  a theme. Equal visual weight for unequal content reads as "too much."

Exemplars worth imitating (why each): **tRPC / Turso / DuckDB** — one runnable example
carries the pitch, deep API lives in a separate `/docs`. **Tailwind / Stripe docs** — a
large feature set grouped into ~8 themed, collapsible sections + a prominent Cmd-K; short
conceptual intro then collapsible deep sections. **Linear** — each capability is one bold
line + one clause + one visual, never a paragraph (a sophisticated product reads as MORE
capable when each claim is one scannable line).

**Static-export gotcha (load-bearing).** `site/` is `output: "export"` (no server), so you
**cannot issue a real 301**. Removing a route (e.g. collapsing `/surface/*`) requires
generating a **client-side redirect stub** page at the old path, or external/inbound links
and the existing `[slug]` catch-all (`dynamicParams=false`) hard-404. Always ship stubs when
you delete a route.

**Lazy-mount heavy demos (load-bearing).** A consolidated gallery that mounts many
interactive components (ZK prover via `bb.js`/`noir_js` ~MB, MPC sim, geo/vector
walkthroughs) on one route MUST `next/dynamic`/`React.lazy` each demo so its chunk loads
**only on expand** — preserving the route-scoping `sidebar-nav.tsx` already enforces for the
ZK prover. Otherwise the "lighter" gallery is heavier than the pages it replaced.

**Optional code loads on invocation, never on initial load (standing policy).**
When a site/GUI capability is all three of (1) not already part of the product's core path,
(2) uncertain-value or rarely used, and (3) bundle-size-increasing, its implementation MUST
sit behind a literal ESM `import()` split point. Fetch it only from the user action that needs
it: selecting a compressed format, expanding a demo, opening a specialist tool, or starting a
prover. A feature flag, conditional render, or top-level static import is insufficient because
the code still enters the initial bundle. Keep import specifiers literal so the bundler can
produce deterministic chunks; use `next/dynamic(() => import("literal"), { ssr: false })` for
React components and `await import("literal")` inside the invocation path for libraries and
codecs. Native browser APIs and small code already used across the primary shell are not
optional split candidates.

Before adding a frontend dependency, classify it in the PR: **core/shared** (justify why it
belongs in the initial bundle) or **optional** (name the invocation boundary and lazy chunk).
Extend `site/scripts/check-bundle.mjs` and its synthetic-manifest unit test for every new
optional dependency whose accidental static import would materially regress first load. The
guard must prove both **presence** as a split point and **isolation** from the guarded route's
first-load files; an isolation-only check can miss a dependency folded into an entry chunk.

**Current audit (sq-mrrn4).** No retrofit candidate remains in the checked-in website:

- capability demos and the home runner are split with `next/dynamic`;
- `@aztec/bb.js` and `@noir-lang/noir_js` load only when proving starts;
- `fzstd` loads only when a user invokes zstd archive decoding;
- the reason, RSP, and text wasm helpers load route-specific glue at runtime; and
- `cmdk`, Radix, and the small styling utilities are shared shell UI, while Playwright/Axe is
  test-only.

Re-run this audit when dependencies or entry points change; do not treat the list as permission
to make future imports static. `site/scripts/check-bundle.mjs` is the executable guard for the
known heavy/optional modules.

---

## 3. Operational-GUI patterns (the desktop app)

The GUI is a **workbench shell**, not a route tree. Its model is an IDE, not a brochure.

- **Persistent shell, per-workspace tabs.** Three fixed regions — a stateful **left rail**
  (workspace switcher · datasets tree of the live store · TOOLS list as *verbs you open*),
  a thin **top bar** (connection target LOCAL⇄ENDPOINT · store size · status LED · theme), a
  **work area** = an IDE-style **tab strip** of open tools (default = Query editor), and a
  **bottom status bar** (measured last-run latency · row count · target · persistence
  backend). No Showcase/Benchmarks/About tabs.
- **The 16 marketing surfaces become TOOLS, not pages.** Query · Graph view · SHACL ·
  Inference · Full-text · Vector · GeoSPARQL · Federation · ZK · MPC · Server — each is a
  verb the user *opens as a tab against the current store*, carrying a small honesty-tier
  dot — **not** a page that *describes* the feature.
- **Command palette (Cmd-K) is the spine.** Fuzzy-index every tool, named graph, recent
  query, import/connect/export action, "run as EXPLAIN", "switch workspace". This *replaces*
  the website's sidebar-as-discovery and is what makes "many surfaces" navigable.
- **Keyboard-first, offline-first.** Run = Cmd-Enter; works with no network against a
  persistent local native store. The native engine link (no wasm ceiling: threads, mmap,
  persistence, native-only formats like HDT) is the GUI's reason to exist over the website.
- **Operate over a LIVE persistent store, not a fixture.** Where the site's playgrounds run
  against a sample graph, every GUI tool runs against the active workspace's real
  imported-from-disk/URL store, persisted across sessions (the `@sparq/client`
  `createWorkspaceStore` → Tauri fs backend model).
- **Reuse component LOGIC, rebuild the HOST.** The GUI should import the editor/results/
  validation *logic* from `site/` (`sparql-editor.tsx`, `@sparq/client` `results.ts`,
  `shacl-playground.tsx`, …) but mount it in operational hosts (full-height panes, multiple
  co-resident result views, a drawer-based importer) — **not** the site's hero+prose wrappers.
- **Reference/marketing content does NOT ship in the app.** No `/about`, `/papers`,
  `/benchmarks`, `/showcase` routes. A single **Help** item opens the website / GitHub in the
  system browser. *The app is the tool; the website is the explainer.*

**Ground-truth caveat (do not repeat the false premise).** `gui/` today is a `src-tauri`
scaffold with **no React frontend** and zero imports of `site/` components ("a scaffold, not
a shipped app"). Building the GUI frontend that imports site components is **net-new work**,
gated on its CI lane (`.github/workflows/gui.yml`) being green — it is *not* a move within an
existing reuse path.

---

## 4. Visual design system (shared between both)

Both frontends share ONE design language (so the project reads as one product) while the
*shells* differ.

- **Tokens, not hex.** Tailwind v4 + the shadcn/radix token set already in `site/src` —
  `bg-background`, `text-foreground`, `bg-sidebar`, `border`, `muted`, semantic variants.
  Never hardcode colours; theme (light/dark via `theme-provider.tsx`) must keep working.
- **Honesty-tier badge is a first-class component.** `TIER_LABEL` / `TIER_VARIANT`
  (`surfaces.ts`) map each tier to a colour + label; `success`=live, `warning`=hosted,
  `muted`=walkthrough, `default`=sim. Reuse it on the website (per-surface badge) and in the
  GUI (per-tool dot). A `walkthrough` must never be dressed up as `live`.
- **Density follows the product.** Website = roomy, marketing density (`max-w-6xl`,
  generous padding). GUI = dense, IDE chrome (denser rail `w-56`, thin `h-10` top bar,
  `h-6` status bar, **no `max-w` cap** — full-bleed work area).
- **Iconography:** `lucide-react`, one icon per surface, declared once in `surfaces.ts`.
- **Code rendering:** the SPARQL/Turtle/JSON-LD highlighters (`sparql-editor.tsx`,
  `rdf-highlight.tsx`, `pretty-turtle`) are shared assets — reuse, don't re-implement.
- **Logo / brand:** `site/src/components/logo.tsx`; for new branding/SVG work use the
  vendored `logo-designer` skill. The wordmark + tier-badge language are the brand.

---

## 5. Accessibility + performance budget (non-negotiable)

- **A11y:** semantic landmarks (`<nav aria-label>`, `<main>`), `aria-current` on the active
  nav item, focus-visible rings, ESC-closes-dialog, a command palette reachable by keyboard,
  `sr-only` titles on icon-only controls and drawers (the existing shell already does several
  of these — keep them). Colour is never the *only* signal a tier carries — pair it with the
  text label.
- **Perf (website):** it is a static bundle on Pages — keep the **main bundle wasm-free**
  (load the ~2.5 MB engine wasm *async after* page load; lazy-load every heavy demo chunk
  on-expand, never on route entry). Image `unoptimized` + a hardcoded `basePath:"/sparq"` are
  load-bearing for Pages/Tauri — don't break them. Track regressions; do not bundle the wasm
  into the initial payload.
- **Perf (GUI):** stream large SELECTs (`streamQueryRows`) so big results don't blow memory;
  show **measured** `performance.now()` latency per run (labelled), never a baked benchmark
  number.
- **Honesty gates apply to all frontend copy.** `scripts/check-privacy-claims.sh` (ZK/MPC
  must stay qualified — research-grade, not externally audited; the MPC site surface is a
  faithful JS *simulation*), the terminology gate, and the no-hard-coded-perf-numbers rule
  gate user-facing copy. Write copy that passes them by construction.

---

## 6. Where content belongs (the routing rule)

When you have a piece of content, route it by **intent**, not by convenience:

| Content | Home |
|---|---|
| "What is sparq, why care" (one line) + one live proof | Website home + `/try` |
| A curated runnable demo ("show me it working") | Website `/examples` + `/showcase/*` detail |
| Compact per-surface blurb + tier + expand-in-place demo | Website `/capabilities` (one gallery) |
| The full capability matrix, every flag, edge cases, repro commands | `crates/<x>/README.md` + `skills/<x>/SKILL.md` |
| Threat-model / soundness nuance, prior art, the design itself | `research/*.md` |
| Benchmark provenance | Website `/benchmarks` (honest, per-commit, same-box framing) |
| Papers | Website `/papers` |
| **Doing** RDF/SPARQL work over real data | the **GUI**, never a website page |

If you are about to inline reference material on a marketing page, stop — link to the asset
that already holds it. If you are about to add a marketing explainer to the GUI, stop — link
to the website. Keeping content in its right home is *how* the website stops being
overwhelming and *how* the GUI stays a tool.

---

## Key file citations (ground truth)

- Design records: `research/website-redesign.md`, `research/gui-design.md`,
  `research/feature-showcase-site-design.md`.
- Single IA source: `site/src/data/surfaces.ts` (`Tier`, `GROUPS`, `FLAGSHIPS`,
  `TIER_LABEL`, `TIER_VARIANT`).
- Website shell + nav (the audited overwhelm): `site/src/components/layout/app-shell.tsx`,
  `site/src/components/layout/sidebar-nav.tsx`.
- The rigid template to make optional: `site/src/components/surface-content.tsx`.
- The killer artifact: `site/src/components/repl.tsx`; reusable logic:
  `site/src/components/sparql-editor.tsx`, `site/src/components/rdf-highlight.tsx`,
  the per-surface playgrounds/walkthroughs, `@sparq/client` (`packages/sparq-client`).
- GUI panels to migrate (net-new host): `site/src/components/connect-panel.tsx`,
  `server-health-panel.tsx`, `subscriptions-view.tsx`.
- Static-export config (don't break): `site/next.config.ts`.
- Honesty gates: `scripts/check-privacy-claims.sh`, the terminology + no-perf gates in
  `.github/workflows/docs-quality.yml`.
- Epics: website redesign epic + GUI epic `sq-ixc3` (`.beads/issues.jsonl`).
