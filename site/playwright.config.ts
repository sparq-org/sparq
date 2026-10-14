// [OPUS-4.8] sq-5q63 — Playwright config for the site's headless browser smoke tests.
//
// Scope: the ZK car-hire prover pre-warm smoke test (e2e/zk-prewarm.spec.ts). The test
// drives a REAL browser against a REAL Next.js server so the "first proof pays no cold
// start" claim is verified by an automated run, not just by code reasoning.
//
// [OPUS-4.8] sq-ymr2e.1 — this config is now also the base for the shared E2E foundation
// (e2e/support/**): a determinism harness (frozen clock, seeded random, animations-off,
// pinned viewport/dark/reduced-motion/UTC/en-US) and a hermetic-network fixture. The
// determinism CONTEXT options live in `use` below; the per-page bits are auto-fixtures in
// e2e/support/fixtures.ts. Design of record: research/web-gui-test-program.md §1.
//
// The server is `next dev` (NOT the static export): the ZK route is fully functional in
// dev — it dynamic-imports @noir-lang/noir_js + @aztec/bb.js (real deps) and fetches the
// committed circuit from public/zk/, none of which need the wasm-REPL/Typst prebuild
// artifacts. Driving dev directly keeps the smoke test self-contained and fast.
//
// Run locally:  npx playwright install chromium && npm run test:e2e
import { existsSync } from "node:fs";

import { defineConfig, devices } from "@playwright/test";

// [FABLE-5] sq-ymr2e.10 — the VISUAL-REGRESSION projects (e2e/visual/**) run ONLY inside the
// digest-pinned official Playwright container (research/web-gui-test-program.md §4): font
// rasterizing + antialiasing are container-stable, so the committed baselines are Linux-only by
// policy. The projects are DEFINED only when SPARQ_VR=1 (set by scripts/vr.sh, which wraps
// `docker run` on the pinned image) so a plain `npm run test:e2e` on a dev laptop never runs
// them (and can never mint an accidental macOS/Windows baseline). The guard below refuses a
// bare-host run even with SPARQ_VR=1 unless explicitly overridden — screenshots minted outside
// the container are NOT comparable to the committed baselines.
const SPARQ_VR = !!process.env.SPARQ_VR;

// [OPUS-4.8] sq-ymr2e.11 — the NIGHTLY cross-browser projects. Per-PR e2e runs headless
// chromium ONLY by design (research/web-gui-test-program.md §6.1); firefox + webkit run in the
// nightly full-sweep lane (§6.2). Playwright projects can only be DEFINED in the config, so the
// nightly lane opts them in with SPARQ_NIGHTLY_BROWSERS=1 and selects them with
// `--project firefox --project webkit`. The env gate keeps a plain `npm run test:e2e` (and every
// per-PR lane) byte-identical to before — the projects array is spread with `[]` when unset, so
// the per-PR path is untouched.
const SPARQ_NIGHTLY_BROWSERS = !!process.env.SPARQ_NIGHTLY_BROWSERS;

// [SONNET-4.6] sq-izpab — the NIGHTLY full-inventory a11y project. The per-PR `chromium`,
// `firefox`, and `webkit` projects all ignore e2e/nightly/ (testIgnore below), so the
// full-inventory AXE sweep (e2e/nightly/a11y-full-inventory.spec.ts) never runs per-PR.
// The nightly workflow sets SPARQ_NIGHTLY_A11Y=1 to activate this project for the
// nightly-full-sweep a11y job. Same chromium browser + determinism defaults as the per-PR
// project; testMatch restricted to e2e/nightly/ so it does NOT double-run the main suite.
const SPARQ_NIGHTLY_A11Y = !!process.env.SPARQ_NIGHTLY_A11Y;

// [SONNET-4.6] sq-ledny — the NIGHTLY full-surface visual flag. The per-PR visual-desktop
// and visual-mobile projects match ONLY the committed key-layouts spec by default; the
// full-surface sweep (e2e/visual/full-surface.spec.ts) has no committed baselines on first
// land and is nightly-only by design. The nightly workflow sets SPARQ_NIGHTLY_VR=1 so the
// visual-sweep job (via vr.sh) matches ALL e2e/visual/**/*.spec.ts. Without the flag the
// per-PR site-visual.yml lane keeps running exactly the pre-existing key-layouts tests.
const SPARQ_NIGHTLY_VR = !!process.env.SPARQ_NIGHTLY_VR;
if (SPARQ_VR && !existsSync("/ms-playwright") && !process.env.SPARQ_VR_ALLOW_HOST) {
  throw new Error(
    "SPARQ_VR=1 outside the pinned Playwright container (/ms-playwright not found). " +
      "Run the visual suite via `npm run vr` / `npm run vr:update` (scripts/vr.sh), or set " +
      "SPARQ_VR_ALLOW_HOST=1 to bypass (the screenshots will NOT match the committed baselines).",
  );
}

const PORT = Number(process.env.PORT ?? 3210);
// basePath "/sparq" (next.config.ts) prefixes every route, so the demo lives under it.
// Keep the TRAILING SLASH: Playwright resolves a relative `page.goto("showcase/…")`
// against this baseURL, and only a trailing slash makes the basePath a path *prefix*
// (without it, the relative segment would replace "/sparq" instead of nesting under it).
const BASE_URL = `http://127.0.0.1:${PORT}/sparq/`;

export default defineConfig({
  testDir: "./e2e",
  // Real ZK proving in a single-thread browser is the slow step; give it room. The
  // assertion is correctness (cold-start count), never a timing threshold — wall-clock
  // here is non-canonical (work-box / CI-runner), so we never gate on a duration.
  timeout: 120_000,
  // A flaky network/instantiate cold start is the prover's documented self-reset path,
  // not a product bug; one retry absorbs that without masking a real regression (a true
  // second cold start fails deterministically on every attempt).
  retries: process.env.CI ? 1 : 0,
  // Proving is CPU-bound and single-threaded; serial keeps the box quiet + the run honest.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["list"]] : [["list"]],
  use: {
    baseURL: BASE_URL,
    // trace on the first retry is CI telemetry: a pass-on-retry is a defect to fix, not a
    // success (determinism doctrine §1.5).
    trace: "on-first-retry",
    // [OPUS-4.8] sq-ymr2e.1 — DETERMINISM defaults that are CONTEXT options, so they apply to
    // EVERY spec (migrated or not) with no per-spec change (research/web-gui-test-program.md §1):
    // a pinned viewport, the site's dark-first colour scheme, reduced motion, and a fixed
    // timezone/locale. Frozen clock + seeded random + animations-off are applied per-page by the
    // support/ auto-fixtures (they need page.clock / addInitScript) — see e2e/support/determinism.ts.
    viewport: { width: 1280, height: 720 },
    colorScheme: "dark",
    timezoneId: "UTC",
    locale: "en-US",
    // reducedMotion is a browser-context option in the test runner (not a top-level `use` key).
    contextOptions: { reducedMotion: "reduce" },
  },
  // [FABLE-5] sq-ymr2e.10 — snapshot layout for the visual projects: baselines live under
  // e2e/visual/baselines/<project>/<name>.png with NO platform suffix — the platform is pinned
  // by the container policy above (Linux-only baselines), so a suffix would only invite a
  // per-OS baseline zoo. Only the visual specs call toHaveScreenshot, so this template is inert
  // for every functional spec.
  snapshotPathTemplate: "{testDir}/visual/baselines/{projectName}/{arg}{ext}",
  expect: {
    timeout: 90_000,
    // Belt-and-suspenders stability for screenshot capture (the harness already injects an
    // animations-off stylesheet + reducedMotion): freeze CSS animations at a deterministic
    // state, hide the text caret, and snapshot at CSS pixels regardless of device scale.
    toHaveScreenshot: { animations: "disabled", caret: "hide", scale: "css" },
  },
  projects: [
    {
      name: "chromium",
      // Keep the pinned viewport at the project level too (device presets carry their own).
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 720 } },
      // Visual specs run only in the container-scoped visual-* projects below.
      // Nightly a11y specs run only in the nightly-a11y project (activated by SPARQ_NIGHTLY_A11Y=1).
      // Both are excluded here so the per-PR path is byte-unchanged.
      testIgnore: [/e2e[\\/]visual[\\/]/, /e2e[\\/]nightly[\\/]/],
    },
    // [SONNET-4.6] sq-izpab — nightly-only full-inventory a11y project (see SPARQ_NIGHTLY_A11Y above).
    ...(SPARQ_NIGHTLY_A11Y
      ? [
          {
            name: "nightly-a11y",
            use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 720 } },
            // Only pick up the nightly specs; the main e2e suite is already run by the
            // chromium project in the same nightly job (npm run test:e2e runs both).
            testMatch: /e2e[\\/]nightly[\\/].*\.spec\.ts/,
          },
        ]
      : []),
    // [FABLE-5] sq-ymr2e.10 — container-only visual projects (see the SPARQ_VR guard above).
    // Two pinned viewports per §4: 1280×720 desktop + 390×844 mobile. Plain viewports (no
    // isMobile/deviceScaleFactor emulation): the site is responsive via CSS breakpoints, and
    // DSF=1 keeps baselines small + byte-stable.
    //
    // [SONNET-4.6] sq-ledny — testMatch is split on SPARQ_NIGHTLY_VR:
    //   • Without the flag (per-PR site-visual.yml): only key-layouts.spec.ts — the specs
    //     whose baselines ARE committed. full-surface.spec.ts is excluded so missing baselines
    //     never advisory-fail the per-PR lane.
    //   • With SPARQ_NIGHTLY_VR=1 (nightly visual-sweep via vr.sh): all e2e/visual/**/*.spec.ts
    //     — includes full-surface and any future specs added to e2e/visual/.
    ...(SPARQ_VR
      ? [
          {
            name: "visual-desktop",
            testMatch: SPARQ_NIGHTLY_VR
              ? /e2e[\\/]visual[\\/].*\.spec\.ts/
              : /e2e[\\/]visual[\\/]key-layouts\.spec\.ts/,
            use: { viewport: { width: 1280, height: 720 } },
          },
          {
            name: "visual-mobile",
            testMatch: SPARQ_NIGHTLY_VR
              ? /e2e[\\/]visual[\\/].*\.spec\.ts/
              : /e2e[\\/]visual[\\/]key-layouts\.spec\.ts/,
            use: { viewport: { width: 390, height: 844 } },
          },
        ]
      : []),
    // [OPUS-4.8] sq-ymr2e.11 — nightly-only firefox + webkit (see SPARQ_NIGHTLY_BROWSERS above).
    // Same pinned 1280×720 viewport + the shared determinism `use` defaults; the container-only
    // visual specs are excluded (visual baselines are chromium/Linux-container-pinned by policy).
    ...(SPARQ_NIGHTLY_BROWSERS
      ? [
          {
            name: "firefox",
            use: { ...devices["Desktop Firefox"], viewport: { width: 1280, height: 720 } },
            // Exclude both container-only visual and nightly-only a11y specs.
            testIgnore: [/e2e[\\/]visual[\\/]/, /e2e[\\/]nightly[\\/]/],
          },
          {
            name: "webkit",
            use: { ...devices["Desktop Safari"], viewport: { width: 1280, height: 720 } },
            // Exclude both container-only visual and nightly-only a11y specs.
            testIgnore: [/e2e[\\/]visual[\\/]/, /e2e[\\/]nightly[\\/]/],
          },
        ]
      : []),
  ],
  webServer: {
    // The local Next binary serves the dev route under /sparq; bypass the `dev` npm
    // script (which runs sync-wasm/Typst prebuild steps the ZK route does not need).
    // `--webpack`: Next 16 defaults to Turbopack, which does not honour next.config.ts's
    // webpack hook (the `.js`→`.ts` extensionAlias + `@sparq/client` alias).
    command: `npx next dev --webpack -p ${PORT}`,
    url: BASE_URL + "showcase/zk-car-hire/",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
