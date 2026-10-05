// [SONNET-4.6] sq-ymr2e.5 — Playwright config for the GUI mocked-IPC deterministic lane.
//
// Drives gui/app/out (the Next.js static export) in headless Chromium.  window.__TAURI__ +
// window.__TAURI_INTERNALS__ are stubbed by the auto-fixture in support/fixtures.ts via
// page.addInitScript (the withGlobalTauri injection pattern from research/web-gui-test-program.md
// §5). The in-tab WASM engine (queries / inference) is NOT mocked — it runs for real.
//
// Design rules (§1 determinism doctrine):
//   - NO waitForTimeout — web-first assertions only
//   - 0 retries — flakes are fixed not hidden
//   - 1 worker — serial execution for determinism
//   - External network blocked per test (hermetic)
//   - Stable selectors: role/data-* only, never CSS classes
//   - Timing values (ms, row counts) never asserted exactly — only presence checks

import { defineConfig, devices } from "@playwright/test";

// (sq-eydh9) [SONNET-4.6] Allow overriding the dev-server port so concurrent worktree test runs
// don't collide.  Default is 3007 (the canonical CI port); set PLAYWRIGHT_PORT=<n> locally when
// another worktree already holds 3007.
const SERVE_PORT = process.env.PLAYWRIGHT_PORT ?? "3007";

export default defineConfig({
  testDir: "./specs",

  // WASM instantiation can be slow; give each assertion generous room.
  timeout: 120_000,
  expect: { timeout: 90_000 },

  // Determinism doctrine §1.1 — 0 retries: a flaky test is a broken test.
  retries: 0,
  // Serial execution keeps results deterministic across the shared WASM tab.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["list"]] : [["list"]],

  use: {
    baseURL: `http://127.0.0.1:${SERVE_PORT}`,
    // [SONNET-4.6] retain-on-failure: with retries=0, "on-first-retry" never fires → no trace
    // ever captured. retain-on-failure emits a trace for every failed test regardless of retries.
    trace: "retain-on-failure",
    viewport: { width: 1280, height: 720 },
    colorScheme: "dark",
    timezoneId: "UTC",
    locale: "en-US",
    contextOptions: { reducedMotion: "reduce" },
  },

  // [FABLE-5] sq-u0my5 — TWO personas over the SAME served export:
  //   chromium-mock-ipc — desktop persona: support/fixtures.ts injects window.__TAURI__ via
  //                       addInitScript, so isTauriRuntime() === true (the original lane).
  //   chromium-web      — browser persona: support/web-fixtures.ts installs NO Tauri globals,
  //                       so isTauriRuntime() === false and the pure-browser code paths (the
  //                       deployed /app) are exercised in CI for the first time.
  // The split is by filename: *.web.spec.ts → chromium-web; everything else → chromium-mock-ipc.
  // Existing specs are untouched and keep their exact prior behavior.
  projects: [
    {
      name: "chromium-mock-ipc",
      testIgnore: /.*\.web\.spec\.ts$/,
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1280, height: 720 },
      },
    },
    {
      name: "chromium-web",
      testMatch: /.*\.web\.spec\.ts$/,
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1280, height: 720 },
      },
    },
  ],

  // Serves gui/app/out (the Tauri-target static export).  -s = SPA mode so every route
  // resolves to index.html (required for Next.js static export with client-side routing).
  // [SONNET-4.6] serve is a pinned devDependency (14.2.6) in this workspace member.
  // gui/e2e-playwright is an npm workspace member so `serve` is HOISTED to the repo-root
  // node_modules/.bin — it does NOT exist at ./node_modules/.bin/serve. `npx serve` resolves
  // the hoisted binary via npm's node_modules hierarchy walk with NO network fetch (the dep is
  // already installed). Using `npm run serve` via a package.json script would also work; npx
  // is simpler (single config change, no extra script entry). [SONNET-4.6] sq-ymr2e.5 fix.
  webServer: {
    // [SONNET-4.6] --no-install: fails CLOSED if serve is not already installed rather than
    // fetching from the registry. serve is a pinned devDependency so it WILL be present after
    // `npm install`; --no-install makes the dependency on that being true explicit + hermetic.
    command: `npx --no-install serve -l ${SERVE_PORT} -s ../app/out`,
    url: `http://127.0.0.1:${SERVE_PORT}`,
    // [SONNET-4.6] sq-vgq0o — never reuse a listener on this fixed port. In local multi-worktree
    // development it may belong to another checkout and serve stale gui/app/out content. Let
    // Playwright fail loudly on a port collision; PLAYWRIGHT_PORT remains available for parallel
    // runs that intentionally need distinct servers.
    reuseExistingServer: false,
    timeout: 15_000,
  },
});
