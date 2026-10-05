# Upstream Next.js: add the `radix-ui` monolith to the DEFAULT `optimizePackageImports` list

**Bead:** sq-w728o · **Status:** evidence prepared, NOT yet posted upstream — awaiting @jeswr review per the
upstream-contribution protocol (AGENTS.md § *Upstream contributions — how to open the PR*) ·
**Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-08-01

## Verdict up front — do NOT open a PR

The change this bead proposes **is already open upstream**: vercel/next.js **[#76065]** — *"Add `radix-ui`,
`radix-ui/internal` to the default `optimizePackageImports` list"* by @jeremy-code, opened 2025-02-14,
`+4/−0` over 2 files, base `canary`. It has never been reviewed: the only activity is the codeowner-notify
bot, `ijjk`'s unticked *"Allow CI Workflow Run"* checkbox, and the author's own unanswered maintainer ping
of 2025-04-06.

AGENTS.md's standing rule is to keep the *existing* upstream thread alive rather than open a competing one.
So the sparq deliverable is **not a duplicate PR** — it is a supporting-evidence comment on #76065 (drafted
in § *Draft comment* below) plus the rebase note in § *Why #76065 is stale*. Posting it is an outbound
upstream filing and therefore **owner-only (`needs:user`)**: @jeswr posts, not the agent.

[#76065]: https://github.com/vercel/next.js/pull/76065

## Problem

`radix-ui` (the monolith published 2025-01-22, superseding the per-primitive `@radix-ui/react-*` packages as
the recommended entry point) is a **pure namespace-re-export barrel**. Verified against
`radix-ui@1.6.0` — the version this repo's lockfile resolves for both `site/` and `gui/app`:

```js
// radix-ui@1.6.0 dist/index.mjs — 35 of these, then one `export { … }` of all 35
import * as Dialog from "@radix-ui/react-dialog";
import * as Select from "@radix-ui/react-select";
…
```

Importing *one* primitive therefore reaches the whole primitive set. All 60 `@radix-ui/*` packages in the
closure declare `"sideEffects": false` at their locked versions (checked, every one), so this is not a
missing-metadata bug — it is the `import * as ns` **namespace-object** shape that webpack's `usedExports`
analysis cannot see through, which is exactly the class of package `optimizePackageImports` exists to
rewrite. `lucide-react` is on the default list; `radix-ui` is not, in either the code list
(`packages/next/src/server/config.ts`) or the docs page — checked on `canary` `16.3.0-canary.105`,
2026-08-01.

## What sparq had to do about it

`site/next.config.ts:62` carries a hand-rolled `experimental: { optimizePackageImports: ["radix-ui"] }` —
added by **sq-qgkwy.1 / PR #1985** precisely because the default list does not cover the monolith. The site
imports 3 distinct primitives (`Slot`, `Dialog`, `Tooltip`) across 5 import sites.

`gui/app` has the **same** consumption pattern (`Slot`, `Dialog`, `Popover`, `DropdownMenu` across 7 files)
and **no** such config — so the Tauri desktop bundle still ships the full barrel. Every downstream app has
to rediscover this workaround independently; that is the argument for fixing it in the default list.

## Evidence A — bundle level (in-repo, cited, NOT re-verified here)

The `site/next.config.ts` comment written alongside the measurement records that before the flag the entire
primitive set landed in a **~170 KB raw commons chunk shipped on almost every route's first load**,
attributed via source maps. Attribution caveats, stated honestly:

- The figure comes from sq-qgkwy.1's own measurement, not from this record. PR #1985 **bundled two
  changes** (the barrel flag *and* making the benchmark snapshot server-only), so the commit subject's
  "halve most routes' first-load JS" is the *combined* effect and must not be attributed to the barrel alone.
- It is a work-box measurement and therefore **non-canonical** per AGENTS.md.
- It could not be re-run in this checkout: no package manager is installed on the box, so no
  `next build` is possible here.

## Evidence B — artifact level (measured here, reproducible, no build required)

Independent of any bundler, the *published npm artifacts* show how much of the barrel an app can reach.
Computed 2026-08-01 against this repo's `package-lock.json` (`radix-ui@1.6.0`) by summing every `.mjs`
(ESM, unminified) file each package publishes, sizes from the jsDelivr package API:

| Set | Packages | ESM bytes |
|---|---:|---:|
| `radix-ui`'s own `dist/*.mjs` (the re-export shims) | 1 | 4,378 |
| Its transitive `@radix-ui/*` closure — what the barrel links | 60 | **529,292** |
| Closure of the 3 primitives `site/` actually imports (`Slot`, `Dialog`, `Tooltip`) | 24 | 98,419 |
| **Reachable only through the barrel, never used** | 36 | **430,873** (81.4 %) |

Largest never-used contributors: `react-select` (51,434 B), `react-menu` (35,113 B),
`react-navigation-menu` (32,440 B), `react-scroll-area` (29,987 B), `react-toast` (26,477 B).

The 60 lockfile `@radix-ui/*` entries are *exactly* `radix-ui@1.6.0`'s transitive closure — nothing else in
the tree pulls a Radix primitive — so installing the monolith installs the whole primitive set by
construction. That is not asserted by hand: the script below walks the closure from `radix-ui` and **fails**
unless it equals the lockfile's `@radix-ui/*` set, and likewise fails unless the used-primitive closure is a
subset of it. The three seeds are the primitives `site/` actually imports — `Slot`, `Dialog`, `Tooltip`,
`import { … } from "radix-ui"` across `ui/badge.tsx`, `ui/button.tsx`, `ui/sheet.tsx`, `ui/tooltip.tsx`,
`command-palette.tsx` (5 sites). Note this is an **unminified source-artifact** figure: it is an upper bound
on emitted bytes and is deliberately not comparable to the minified bundle figure in Evidence A. The two
agree in direction and in which primitives dominate.

Reproduce (network + this repo's lockfile only) — prints every row of the table, the percentage, and the
largest-never-used list, at the versions the lockfile pins; throws if any package, version or jsDelivr
response is missing:

```js
// node repro.js — from the repo root. Recomputes every value in the table above.
const P = require('./package-lock.json').packages;
const ROOT = 'radix-ui', USED = ['@radix-ui/react-slot', '@radix-ui/react-dialog', '@radix-ui/react-tooltip'];
const die = m => { throw new Error(m); };
const entry = n => P['node_modules/' + n] || die(`not in lockfile: ${n}`);
// Transitive closure over @radix-ui/* dependency edges. The lockfile tree is flat here
// (no nested node_modules/@radix-ui copies), so a name resolves to exactly one version.
const closure = (seeds, keepSeeds) => {
  const seen = new Set(), q = [...seeds];
  while (q.length) {
    const n = q.pop();
    if (seen.has(n)) continue;
    seen.add(n);
    for (const d of Object.keys(entry(n).dependencies || {})) if (d.startsWith('@radix-ui/')) q.push(d);
  }
  if (!keepSeeds) for (const s of seeds) seen.delete(s);
  return seen;
};
const mjs = async n => {                       // sum of published .mjs bytes AT THE LOCKED VERSION
  const v = entry(n).version, r = await fetch(`https://data.jsdelivr.com/v1/packages/npm/${n}@${v}`);
  if (!r.ok) die(`jsDelivr ${r.status} for ${n}@${v}`);
  const j = await r.json();
  if (!Array.isArray(j.files) || j.version !== v) die(`bad jsDelivr payload for ${n}@${v}`);
  let t = 0;
  (function walk(fs) { for (const f of fs) f.type === 'directory' ? walk(f.files) : f.name.endsWith('.mjs') && (t += f.size); })(j.files);
  return t;
};
const sum = async ns => (await Promise.all([...ns].map(mjs))).reduce((a, b) => a + b, 0);
(async () => {
  const barrel = closure([ROOT], false), used = closure(USED, true);
  const locked = new Set(Object.keys(P).filter(k => k.startsWith('node_modules/@radix-ui/')).map(k => k.slice('node_modules/'.length)));
  // (1) the barrel's closure IS exactly the set of @radix-ui/* packages the lockfile installs
  if (barrel.size !== locked.size || [...locked].some(n => !barrel.has(n))) die('barrel closure != lockfile @radix-ui/* set');
  // (2) what the app imports is a subset of what the barrel links
  for (const n of used) if (!barrel.has(n)) die(`used package outside barrel closure: ${n}`);
  const unused = [...barrel].filter(n => !used.has(n));
  const [own, cB, uB, nB] = [await mjs(ROOT), await sum(barrel), await sum(used), await sum(unused)];
  if (uB + nB !== cB) die('used + unused != closure bytes');
  console.log(`own dist/*.mjs   1 pkg  ${own} B`);
  console.log(`barrel closure  ${barrel.size} pkgs ${cB} B`);
  console.log(`used closure    ${used.size} pkgs ${uB} B`);
  console.log(`never used      ${unused.length} pkgs ${nB} B (${(100 * nB / cB).toFixed(1)} %)`);
  console.log('largest never used:', (await Promise.all(unused.map(async n => [n, await mjs(n)])))
    .sort((a, b) => b[1] - a[1]).slice(0, 5).map(([n, b]) => `${n} ${b}`).join(', '));
})();
```

Run in this checkout on 2026-08-01 (`node v20`), it prints exactly the table's values and passes all three
assertions:

```text
own dist/*.mjs   1 pkg  4378 B
barrel closure  60 pkgs 529292 B
used closure    24 pkgs 98419 B
never used      36 pkgs 430873 B (81.4 %)
largest never used: @radix-ui/react-select 51434, @radix-ui/react-menu 35113, @radix-ui/react-navigation-menu 32440, @radix-ui/react-scroll-area 29987, @radix-ui/react-toast 26477
```

## Why #76065 is stale (the rebase note)

The PR's two hunks no longer apply cleanly to `canary`:

| PR touched | Current location on `canary` (`16.3.0-canary.105`) |
|---|---|
| `packages/next/src/server/config.ts` @ line 972 | same file, the list moved to **line 1506** (`result.experimental.optimizePackageImports = [...new Set([...])]` inside `assignDefaults`); the list has since grown the `@effect/*` entries |
| `docs/01-app/**04**-api-reference/05-config/01-next-config-js/optimizePackageImports.mdx` | **path deleted** — the docs tree renumbered to `docs/01-app/**03**-api-reference/05-config/01-next-config-js/optimizePackageImports.mdx` (and a parallel `docs/02-pages/04-api-reference/04-config/…` page now exists) |

So the ask on the thread is: rebase onto current `canary`, re-target the docs page (and consider the pages
-router copy), and a maintainer ticks *Allow CI Workflow Run*. The substance of the change — two string
literals — is unchanged and still correct: `radix-ui/internal` is likewise still a namespace-re-export
barrel in `1.6.0` (`dist/internal.mjs`, `import * as Menu from "@radix-ui/react-menu"`, …).

## Prior art (searched 2026-08-01)

- vercel/next.js **#76065** — the proposal itself (open, unreviewed). Links a StackBlitz repro; carries **no
  measured numbers**, which is the specific gap this record fills.
- No other open or closed issue/PR in `vercel/next.js` proposes `radix-ui` for the default list (repo issue
  search for `radix-ui optimizePackageImports` returns #76065 plus three unrelated hits).
- Adjacent sparq record: `research/nextjs-nomodule-polyfills-upstream.md` (sq-zv37m) — same protocol, same
  "prepared, awaiting @jeswr" status.

## Draft comment for #76065 (for @jeswr to post)

> Some measured support for this, from a Next 15.5 app that had to set
> `experimental.optimizePackageImports: ['radix-ui']` by hand for exactly this reason.
>
> `radix-ui@1.6.0`'s `dist/index.mjs` is 35 `import * as X from '@radix-ui/react-*'` bindings re-exported as
> one object — the namespace-object shape webpack's `usedExports` can't see through, even though every
> `@radix-ui/*` package already declares `"sideEffects": false`. Measuring the published artifacts (sum of
> each package's `.mjs`, unminified, sizes from the jsDelivr package API): the barrel's transitive closure is
> **60 packages / 529,292 B**, while the closure of the three primitives our app actually imports (`Slot`,
> `Dialog`, `Tooltip`) is **24 packages / 98,419 B**. So **81 %** of what the barrel links is unreachable —
> `react-select` (51 KB), `react-menu` (35 KB), `react-navigation-menu` (32 KB) and `react-scroll-area`
> (30 KB) lead the list. At the bundle level that showed up for us as a large commons chunk on nearly every
> route's first load until the flag was set; the flag fixes it, so the transform handles this barrel shape
> correctly today — it's only the default list that's missing an entry.
>
> Since `radix-ui` is now the recommended entry point for Radix primitives, every app hits this and has to
> rediscover the workaround. Worth noting the PR needs a rebase: the default list has moved to
> `packages/next/src/server/config.ts:1506` on current canary, and the docs page it edits was renumbered to
> `docs/01-app/03-api-reference/05-config/01-next-config-js/optimizePackageImports.mdx`.

## Next steps

1. **@jeswr (owner-only):** post the comment above on vercel/next.js#76065, self-identifying per the
   upstream-contribution protocol. Do not open a competing PR. If @jeremy-code goes unresponsive and a
   maintainer asks for a rebase, offer the rebase *on that PR's branch*, not a new one.
2. **When #76065 lands and sparq's Next floor includes it:** delete
   `experimental.optimizePackageImports: ["radix-ui"]` from `site/next.config.ts` — the config comment
   points here so the workaround is removed rather than left to rot.
3. **Independent of upstream:** `gui/app` needs the same flag today (tracked separately — it is a bundle
   change with its own measurement obligation, not part of this upstream record).
