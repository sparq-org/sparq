// [OPUS-4.8] sq-vw3ax.7 / sq-rclb8 — headless smoke test for the COLLAPSED navigation (Option-B).
//
// WHAT IT GUARDS. The redesign removed the persistent w-64 sidebar tree + the duplicate top-tab
// bar, leaving ONE slim top bar of content destinations (research/website-redesign.md §2, §7).
// [OPUS-4.8] sq-4hiqe — the /try SPARQL playground was removed entirely: the top-nav "Try" item is
// GONE and /try now hard-redirects to /app (a redirect stub, like the legacy /gui path).
// [OPUS-4.8] sq-1scgk — "Papers" was promoted INTO the slim bar (maintainer 2026-07-04 item 9b:
// make the paper-factory output prominently findable). The slim bar's destinations are now
// Home · Capabilities · App · Benchmarks · Papers · Download, where "App" → /app is the live
// operational GUI. This test asserts the slim bar's six destinations route correctly, that the old
// full sidebar tree is GONE, that there is NO "Try" nav item, and that both the legacy /gui path
// and the removed /try path redirect to /app.
// [OPUS-4.8] sq-ymr2e.1 — migrated onto the shared E2E foundation: the hermetic + deterministic
// `test` (e2e/support) and the shared `gotoAppReady` barrier, which absorbs the coi-serviceworker
// one-time reload via the SW-controller signal and waits for the app-shell hydration — replacing
// the fixed 500ms sleep with deterministic signals (research/web-gui-test-program.md §1).
import { test, expect, gotoAppReady } from "./support";
import { type Page } from "@playwright/test";

const MOD = "Control";

// The site ships the coi-serviceworker shim which reloads the tab ONCE on a fresh visit to take
// effect. That reload tears down any pending client-side navigation, so a click-then-navigate
// test can race it. `gotoAppReady` waits until the SW is CONTROLLING the page — the deterministic
// signal that the one-time reload has already happened and won't fire again — plus the app-shell
// hydration barrier, before returning.
async function gotoSettled(page: Page, route: string): Promise<void> {
  await gotoAppReady(page, route);
}

test("the slim top bar shows the destinations (no Try item) and no full sidebar tree", async ({
  page,
}) => {
  await gotoSettled(page, "");

  const primary = page.getByRole("navigation", { name: "Primary" }).first();
  // [OPUS-4.8] sq-1scgk — "Papers" is now a first-class bar destination alongside the rest.
  for (const label of [
    "Home",
    "Docs",
    "Capabilities",
    "App",
    "Benchmarks",
    "Papers",
    "Download",
  ]) {
    await expect(primary.getByRole("link", { name: label, exact: true })).toBeVisible();
  }
  // #2549 — mutation witness: Docs must point at the mdBook guide overlaid at /guide/ by
  // pages.yml (basePath-prefixed, trailing slash), not at the agent-skills router.
  await expect(primary.getByRole("link", { name: "Docs", exact: true })).toHaveAttribute(
    "href",
    /\/guide\/$/,
  );
  // [OPUS-4.8] sq-4hiqe — the "Try" nav item is REMOVED (the /try playground is gone). The bar
  // must expose NO "Try" destination now.
  await expect(primary.getByRole("link", { name: "Try", exact: true })).toHaveCount(0);
  // The OLD single "Try the GUI" entry is likewise gone.
  await expect(page.getByRole("link", { name: /Try the GUI/i })).toHaveCount(0);

  // The OLD full sidebar tree is gone: there is no persistent "Feature surfaces" nav landmark
  // (the sidebar's aria-label) and no per-surface tree link like "GeoSPARQL" on a content page.
  await expect(page.getByRole("navigation", { name: "Feature surfaces" })).toHaveCount(0);
});

test("Capabilities is reachable from the top bar and renders the 5 themes", async ({ page }) => {
  await gotoSettled(page, "");
  await page
    .getByRole("navigation", { name: "Primary" })
    .first()
    .getByRole("link", { name: "Capabilities", exact: true })
    .click();
  await page.waitForURL("**/capabilities/**", { timeout: 15_000 });

  for (const theme of [
    "Query & data",
    "Reason & validate",
    "Search & GenAI",
    "Privacy (ZK / MPC)",
    "Serve & embed",
  ]) {
    await expect(page.getByRole("heading", { name: theme })).toBeVisible();
  }
});

test("the top bar points App at the GUI (/app)", async ({ page }) => {
  await gotoSettled(page, "");
  const primary = page.getByRole("navigation", { name: "Primary" }).first();
  // The App destination points at the live operational GUI (assert the href directly —
  // deterministic, and not subject to the dev-server's one-time coi-serviceworker reload racing a
  // click-then-navigate).
  await expect(primary.getByRole("link", { name: "App", exact: true })).toHaveAttribute(
    "href",
    /\/app\/?$/,
  );
});

// [OPUS-4.8] sq-4hiqe — the /try SPARQL playground was removed; its page is now a hard-redirect
// stub (window.location → /app, the same mechanism as the legacy /gui path). Navigating to /try
// therefore lands on /app — there is no REPL/workbench there anymore. (The redirect is also
// covered in redirect-stubs.spec.ts; kept here for the nav-facing regression.)
test("the removed /try path redirects to the live /app GUI", async ({ page }) => {
  await gotoSettled(page, "try/");
  await page.waitForURL("**/app/**", { timeout: 15_000 });
  expect(new URL(page.url()).pathname).toContain("/app");
  await expect(page.getByRole("heading", { name: "App", level: 1 })).toBeVisible();
});

test("App navigates to the live operational GUI destination", async ({ page }) => {
  await gotoSettled(page, "");
  await page
    .getByRole("navigation", { name: "Primary" })
    .first()
    .getByRole("link", { name: "App", exact: true })
    .click();
  await page.waitForURL("**/app/**", { timeout: 30_000 });
  expect(new URL(page.url()).pathname).toContain("/app");
  await expect(page.getByRole("heading", { name: "App", level: 1 })).toBeVisible();
});

test("Download is a reachable destination", async ({ page }) => {
  await gotoSettled(page, "");
  await page
    .getByRole("navigation", { name: "Primary" })
    .first()
    .getByRole("link", { name: "Download", exact: true })
    .click();
  await page.waitForURL("**/download/**", { timeout: 30_000 });
  expect(new URL(page.url()).pathname).toContain("/download");
});

// [OPUS-4.8] sq-1scgk — "Papers" is promoted into the slim bar (maintainer 2026-07-04 item 9b);
// assert the new bar link routes to the /papers index (the paper-factory output surface).
test("Papers is a reachable destination", async ({ page }) => {
  await gotoSettled(page, "");
  await page
    .getByRole("navigation", { name: "Primary" })
    .first()
    .getByRole("link", { name: "Papers", exact: true })
    .click();
  await page.waitForURL("**/papers/**", { timeout: 30_000 });
  expect(new URL(page.url()).pathname).toContain("/papers");
});

test("the legacy /gui path client-redirects to /app", async ({ page }) => {
  await gotoSettled(page, "gui/");
  await page.waitForURL("**/app/**", { timeout: 15_000 });
  expect(new URL(page.url()).pathname).toContain("/app");
  await expect(page.getByRole("heading", { name: "App", level: 1 })).toBeVisible();
});

test("Cmd-K still opens after the sidebar removal (the fast path to every surface)", async ({
  page,
}) => {
  await gotoSettled(page, "");
  await page.keyboard.press(`${MOD}+KeyK`);
  await expect(page.getByRole("dialog", { name: "Command palette" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Command palette" })).toHaveCount(0);
});
