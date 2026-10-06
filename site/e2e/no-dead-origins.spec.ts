// [OPUS-4.8] sq-vw3ax.11.4 — STATIC guard: no user-facing links to either STALE origin
// may reappear in site/src.
//
// WHY. The site cut over to the custom domain sparq.jeswr.org (sq-uj38w) and the repo
// moved to sparq-org/sparq. Two prior-location origins must never re-land in the source:
//   • https://jeswr.github.io/sparq — the old GitHub-Pages sub-path origin, DEAD
//     (live-verified 404); any href/URL string pointing at it is a broken link.
//   • https://github.com/jeswr/sparq — the old repository location; GitHub's rename
//     redirect keeps it working, but R4 migrates every such reference to
//     github.com/sparq-org/sparq so the site never advertises the stale name.
// This spec greps the source tree and FAILS the moment either string is reintroduced, so
// the regression can never silently re-land. NOTE: the `@sparq-org/sparq` npm package name is
// unrelated and stays — the github.com/ anchor on the repo pattern excludes it.
// Design of record: research/site-home-app-download-residuals.md §4 R4.
//
// It is a pure filesystem scan (no browser/page needed) — Playwright is just the runner
// that the site's `npx playwright test e2e/` acceptance already invokes.
//
// SCOPE. Two files are owned by sibling in-flight beads and are excluded here until they
// land (they carry their own sweep):
//   • src/app/download/download-client.tsx  (sq-vw3ax.11.2)
//   • src/components/command-palette.tsx     (sq-vw3ax.11.1)
// When those beads land, delete their entry from EXCLUDED below so the guard covers them.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import { test, expect } from "@playwright/test";

// site/src, resolved from this spec's location (site/e2e/…) so the test is CWD-independent.
const SRC_DIR = fileURLToPath(new URL("../src", import.meta.url));

// The stale origins. Each is kept deliberately broad (host + repo segment) so ANY link
// shape — https://jeswr.github.io/sparq, github.com/jeswr/sparq/actions, a bare reference
// — is caught. The `github.com/` anchor on the repo pattern means the live `@sparq-org/sparq`
// npm package name (and other bare `sparq-org/sparq` mentions) is NOT flagged.
const STALE_ORIGINS = ["jeswr.github.io/sparq", "github.com/jeswr/sparq"];

// Paths (relative to site/src, POSIX-style) exempted until their owning bead lands.
const EXCLUDED = new Set<string>([
  "app/download/download-client.tsx", // sq-vw3ax.11.2
  "components/command-palette.tsx", // sq-vw3ax.11.1
]);

// Text source we scan. Binary assets (images/fonts) can never carry a clickable href and
// are skipped so a stray null byte never trips a false "binary file matches".
const TEXT_EXT = /\.(tsx?|jsx?|mjs|cjs|json|md|mdx|css)$/;

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const abs = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...walk(abs));
    } else if (entry.isFile() && TEXT_EXT.test(entry.name)) {
      out.push(abs);
    }
  }
  return out;
}

test("no stale jeswr.github.io/sparq or github.com/jeswr/sparq origins remain in site/src", () => {
  const offenders: string[] = [];
  for (const abs of walk(SRC_DIR)) {
    const rel = relative(SRC_DIR, abs).split("\\").join("/");
    if (EXCLUDED.has(rel)) continue;
    const lines = readFileSync(abs, "utf8").split("\n");
    lines.forEach((line, i) => {
      const hit = STALE_ORIGINS.find((origin) => line.includes(origin));
      if (hit) {
        offenders.push(`src/${rel}:${i + 1}: [${hit}] ${line.trim()}`);
      }
    });
  }
  expect(
    offenders,
    `Stale origin(s) ${STALE_ORIGINS.map((o) => `"${o}"`).join(", ")} found in site/src — ` +
      `replace with the live custom domain https://sparq.jeswr.org (page routes) or ` +
      `github.com/sparq-org/sparq (repo links):\n` +
      offenders.join("\n"),
  ).toEqual([]);
});
