// [OPUS-4.8] sq-ymr2e.1 — the deterministic "app is ready to drive" barrier.
//
// Design of record: research/web-gui-test-program.md §1.1 — "No time-based waits." This replaces
// the fixed 500ms `page.waitForTimeout` beats the smoke specs used to sleep after navigation with
// two DETERMINISTIC signals, so a run is reproducible and the no-timeout grep gate under site/e2e
// holds:
//
//   1. The coi-serviceworker one-time reload is absorbed by waiting for the SW to CONTROL the page
//      (`navigator.serviceWorker.controller != null`) — that flips only AFTER the single reload,
//      so the subsequent interaction lands on the live document, never a torn-down one. Wrapped so
//      a spec that blocks the SW, or a browser without SW support, proceeds immediately.
//   2. A hydration barrier: the app shell exposes `[data-app-ready]` from its client snapshot (see
//      src/components/layout/app-shell.tsx), so its appearance means React has hydrated the shell
//      AND the wrapping <CommandPalette>'s global ⌘K keydown effect is armed (the attribute is
//      applied on the hydration update AFTER the passive-effect batch). This is the deterministic
//      replacement for the old fixed 500ms hydration beat — a mere server-rendered element's
//      attachment would NOT do, since Next SSRs the whole shell (it is present pre-hydration).
//      It re-resolves after a client redirect (the redirect-stub case).
//
// Both are bounded and best-effort (`.catch`): if a signal never arrives the barrier yields to
// Playwright's own web-first auto-waiting on the assertions that follow — it never hard-fails, it
// only removes the fixed sleep.
import type { Page } from "@playwright/test";

const SW_CONTROLLER_TIMEOUT = 10_000;
const HYDRATION_TIMEOUT = 15_000;

/** Wait until the app shell is hydrated and the coi-serviceworker reload (if any) has settled. */
export async function waitForAppReady(page: Page): Promise<void> {
  await page.waitForLoadState("domcontentloaded");
  await page
    .waitForFunction(() => navigator.serviceWorker?.controller != null, undefined, {
      timeout: SW_CONTROLLER_TIMEOUT,
    })
    .catch(() => {});
  await page
    .locator("[data-app-ready]")
    .first()
    .waitFor({ state: "attached", timeout: HYDRATION_TIMEOUT })
    .catch(() => {});
}

/** Navigate to a route (relative to the /sparq/ baseURL) and wait for app-ready. */
export async function gotoAppReady(page: Page, route: string): Promise<void> {
  await page.goto(route, { waitUntil: "domcontentloaded" });
  await waitForAppReady(page);
}
