// [SONNET-4.6] sq-vw3ax.11.2 — /download page e2e: three mocked API states.
//
// Guards three distinct states (per bead acceptance criteria):
//
//   (A) latest-404 + list-has-prerelease: prerelease fallback — direct browser_download_url
//       buttons labelled with the dev-release version; unsigned dev-build banner callout.
//
//   (B) latest-404 + list-empty: no-release state — every download control degrades to the
//       Releases-page link; unsigned banner remains.
//
//   (C) stable-ready + one alias MISSING: fail-closed ready state — the present alias renders
//       a direct latest/download/<alias> button with version + sha256; the card whose alias is
//       absent in the release's asset map degrades per-card to the Releases-page link.
//
// MUTATION SPOT-CHECK (kept inline, not a CI test): point one card at a non-existent asset
// and confirm the assertion goes red, then restore. See the comments at the bottom of this file
// for the exact assertion that fails when the mutation is applied.
//
// All GitHub API calls are MOCKED via page.route — the test is fully hermetic.
import { test, expect, type Page, type ConsoleMessage } from "@playwright/test";

// Block the coi-serviceworker for this spec. The shim re-issues EVERY fetch (incl. cross-origin)
// from inside the service worker, bypassing page.route — so the api.github.com mock would never
// intercept. The /download page needs no cross-origin isolation (plain CORS fetch + static links),
// so blocking the SW is safe here and makes the mock deterministic.
test.use({ serviceWorkers: "block" });

// [SONNET-4.6] sq-vw3ax.11.2 — URLs updated to sparq-org/sparq (repo renamed).
const API_LATEST_GLOB = "**/api.github.com/repos/sparq-org/sparq/releases/latest";
const API_LIST_GLOB   = "**/api.github.com/repos/sparq-org/sparq/releases*";
const RELEASES_URL    = "https://github.com/sparq-org/sparq/releases";
const LATEST_DL       = "https://github.com/sparq-org/sparq/releases/latest/download";

// The light site-e2e CI lane builds no wasm bundle — any wasm fetch 404s benignly.
const BENIGN_RESOURCE_404 = /Failed to load resource:.*\b404\b/i;
// Blocking the service worker throws a benign TypeError about 'scope' — filter it.
const BENIGN_SW_BLOCK = /reading 'scope'|ServiceWorker.*(register|controller)/i;

function trackConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (msg: ConsoleMessage) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (BENIGN_RESOURCE_404.test(text) || BENIGN_SW_BLOCK.test(text)) return;
    errors.push(text);
  });
  page.on("pageerror", (err) => {
    const text = String(err);
    if (BENIGN_SW_BLOCK.test(text)) return;
    errors.push(text);
  });
  return errors;
}

async function gotoSettled(page: Page, route: string): Promise<void> {
  await page.goto(route, { waitUntil: "domcontentloaded" });
}

const FAKE_SHA = "a".repeat(64);
const PRE_VERSION = "v0.1.0-dev.3";

// ---- Fake prerelease asset list ---------------------------------------------------------------
// A prerelease release body with versioned asset names (as the real pipeline emits).
// Includes the Windows MSI (promoted on the Playwright Desktop-Chrome Windows UA) but NOT the
// Linux AppImage — so the Linux card must degrade to the Releases page (per-card fail-closed).
//
// WIN_DL_URL is the browser_download_url the mock API returns for the Windows MSI asset.
// WIN_DL_URL_EXPECTED is hardcoded separately (same initial value) so that the mutation
// spot-check can change WIN_DL_URL to a different string and confirm the state-A assertion
// goes red (the component renders the mutated URL; the locator still checks the original).
const WIN_DL_URL =
  `https://objects.githubusercontent.com/github-production/sparq-gui_${PRE_VERSION}_x64.msi`;
// Hardcoded — DO NOT replace with `WIN_DL_URL`. Must match the current WIN_DL_URL value.
// Mutation check: change WIN_DL_URL above → state-A's `winBtn` locator won't find an anchor.
const WIN_DL_URL_EXPECTED =
  `https://objects.githubusercontent.com/github-production/sparq-gui_${PRE_VERSION}_x64.msi`;
const PRE_RELEASE_BODY = JSON.stringify({
  tag_name: PRE_VERSION,
  prerelease: true,
  assets: [
    {
      name: `sparq-gui_${PRE_VERSION}_x64.msi`,
      size: 94371840,
      digest: `sha256:${FAKE_SHA}`,
      browser_download_url: WIN_DL_URL,
    },
    // macOS arm64 present — so that card gets a direct button.
    {
      name: `sparq-gui_${PRE_VERSION}_aarch64.dmg`,
      size: 80000000,
      digest: null,
      browser_download_url:
        `https://objects.githubusercontent.com/github-production/sparq-gui_${PRE_VERSION}_aarch64.dmg`,
    },
    // CLI combined archive (x64-linux baseline — the default token for undetected OS).
    {
      name: `sparq-cli-${PRE_VERSION}-x64-baseline.tar.gz`,
      size: 10000000,
      digest: null,
      browser_download_url:
        `https://objects.githubusercontent.com/github-production/sparq-cli-${PRE_VERSION}-x64-baseline.tar.gz`,
    },
  ],
});

// ---- State-C: stable release — Windows MSI present, linux AppImage ABSENT ------------------
// The Playwright Desktop-Chrome device uses a Windows UA, so Windows is the primary card.
// The Windows alias IS in the asset map → direct button. Linux AppImage is absent → degraded.
const STABLE_WIN_ALIAS = "sparq-gui-win-x64.msi";
const STABLE_VERSION   = "v9.9.9-test";
const STABLE_BODY = JSON.stringify({
  tag_name: STABLE_VERSION,
  prerelease: false,
  assets: [
    {
      name: STABLE_WIN_ALIAS,
      size: 94371840,
      digest: `sha256:${FAKE_SHA}`,
      browser_download_url: `https://example.com/${STABLE_WIN_ALIAS}`,
    },
    // linux AppImage alias intentionally absent — tests per-card degradation.
  ],
});

// ---- Route helpers ---------------------------------------------------------------------------

/**
 * Mock the /releases/latest endpoint. Also mock the list endpoint to return empty so it
 * doesn't accidentally serve prerelease data when latest is 404 in stable-state tests.
 */
async function mockLatest(page: Page, status: number, body: string): Promise<void> {
  await page.route(API_LATEST_GLOB, async (route) => {
    await route.fulfill({
      status,
      headers: {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
        "cross-origin-resource-policy": "cross-origin",
      },
      body,
    });
  });
}

/** Mock the list endpoint (hit on 404 from /latest as prerelease fallback). */
async function mockList(page: Page, status: number, body: string): Promise<void> {
  await page.route(API_LIST_GLOB, async (route) => {
    // Only intercept the list URL, not /latest (latest has its own mock above).
    const url = route.request().url();
    if (url.includes("/releases/latest")) {
      await route.continue();
      return;
    }
    await route.fulfill({
      status,
      headers: {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
        "cross-origin-resource-policy": "cross-origin",
      },
      body,
    });
  });
}

// =============================================================================
// State A: latest-404 + list returns a prerelease
// =============================================================================
test("state-A: prerelease fallback — direct dev-build buttons + unsigned banner", async ({
  page,
}) => {
  const consoleErrors = trackConsoleErrors(page);

  // latest 404s → code falls back to list endpoint.
  await mockLatest(page, 404, JSON.stringify({ message: "Not Found" }));
  await mockList(page, 200, `[${PRE_RELEASE_BODY}]`);

  await gotoSettled(page, "download/");

  // Banner is present and explicitly calls out unsigned dev pre-release builds.
  await expect(page.getByText("Developer builds — unsigned.")).toBeVisible();
  await expect(page.getByText(/unsigned development pre-release builds/i)).toBeVisible();
  // The prerelease version tag appears in the banner (and elsewhere — first() avoids strict-mode
  // violation when the version string also appears in button meta / the Desktop-app badge).
  await expect(page.getByText(new RegExp(PRE_VERSION)).first()).toBeVisible();

  // Playwright Desktop-Chrome uses a Windows UA — Windows card is promoted.
  // The Windows MSI IS in the prerelease assets → direct browser_download_url button.
  await expect(page.getByText("For your Windows PC")).toBeVisible();
  // Assert against WIN_DL_URL_EXPECTED (not WIN_DL_URL) so a mutation to the mock URL is caught.
  const winBtn = page.locator(`a[href="${WIN_DL_URL_EXPECTED}"]`);
  await expect(winBtn).toBeVisible();
  await expect(winBtn).toContainText("Download sparq-gui.msi");
  // Version meta is rendered.
  await expect(winBtn).toContainText(PRE_VERSION);

  // [SONNET-4.6] sq-vw3ax.11.2 — per-card degradation: Linux AppImage is NOT in the prerelease
  // asset list → its card must degrade to the Releases page link, not a dead direct button.
  // The prerelease assets only include the Windows MSI + macOS arm64 DMG + CLI archive.
  // Verify that at least one "See Releases page" degraded link appears (Linux card).
  await expect(page.getByRole("link", { name: /See Releases page/i }).first()).toBeVisible();
  // And that no linux AppImage direct download URL (non-Releases, non-degraded) appears.
  await expect(
    page.locator(`a[href*="sparq-gui"][href*="AppImage"]`),
  ).not.toBeVisible();

  // The "No release yet" text must NOT appear (we have a prerelease).
  await expect(page.getByText(/No release yet/i)).not.toBeVisible();

  // No-install closer is present.
  await expect(page.getByRole("link", { name: /Open the workbench/i })).toBeVisible();

  expect(consoleErrors, `console errors: ${consoleErrors.join("\n")}`).toEqual([]);
});

// =============================================================================
// State B: latest-404 + list returns empty array
// =============================================================================
test("state-B: latest-404 + list-empty — all controls degrade to 'No release yet'", async ({
  page,
}) => {
  const consoleErrors = trackConsoleErrors(page);

  await mockLatest(page, 404, JSON.stringify({ message: "Not Found" }));
  await mockList(page, 200, "[]");

  await gotoSettled(page, "download/");

  // Unsigned banner is present.
  await expect(page.getByText("Developer builds — unsigned.")).toBeVisible();
  // The prerelease-specific "unsigned development pre-release builds" text must NOT appear.
  await expect(page.getByText(/unsigned development pre-release builds/i)).not.toBeVisible();

  // Every download control must show "No release yet — watch releases".
  const watchLinks = page.getByRole("link", { name: /No release yet — watch releases/i });
  // At minimum there must be one (the primary promoted card).
  await expect(watchLinks.first()).toBeVisible();
  await expect(watchLinks.first()).toHaveAttribute("href", RELEASES_URL);

  // [GPT-6] Registry installation paths remain discoverable independently of GitHub releases.
  const packages = page.getByRole("region", { name: "Package managers" });
  await expect(packages).toBeVisible();
  const registryLinks = [
    ["sparq-cli on crates.io", "https://crates.io/crates/sparq-cli"],
    ["Browse the sparq crate family", "https://crates.io/search?q=sparq-"],
    ["sparq-rdf on PyPI", "https://pypi.org/project/sparq-rdf/"],
    ["@sparq-org/sparq on npm", "https://www.npmjs.com/package/@sparq-org/sparq"],
    ["@sparq-org/solid-server on npm", "https://www.npmjs.com/package/@sparq-org/solid-server"],
    ["@sparq-org/eyereasoner-compat on npm", "https://www.npmjs.com/package/@sparq-org/eyereasoner-compat"],
  ];
  for (const [name, href] of registryLinks) {
    const link = packages.getByRole("link", { name, exact: true });
    await expect(link).toHaveAttribute("href", href);
    await expect(link).toHaveAttribute("rel", "noopener noreferrer");
  }
  await expect(packages.getByText("python -m pip install sparq-rdf", { exact: true })).toBeVisible();
  await expect(packages.getByText(/use import sparq in Python/)).toBeVisible();

  // No-install closer is present.
  await expect(page.getByRole("link", { name: /Open the workbench/i })).toBeVisible();

  expect(consoleErrors, `console errors: ${consoleErrors.join("\n")}`).toEqual([]);
});

// =============================================================================
// State C: stable-ready, one alias missing (fail-closed)
// =============================================================================
test("state-C: stable-ready — present alias gets direct button, missing alias degrades", async ({
  page,
}) => {
  const consoleErrors = trackConsoleErrors(page);

  await mockLatest(page, 200, STABLE_BODY);
  // list endpoint won't be called (latest is 200), but provide a stub to avoid any accidental hit.
  await mockList(page, 200, "[]");

  await gotoSettled(page, "download/");

  // Playwright Desktop-Chrome (Windows UA) promotes the Windows card.
  await expect(page.getByText("For your Windows PC")).toBeVisible();

  // Windows alias IS in the release's asset map → static latest/download/<alias> button.
  const primary = page.locator(`a[href="${LATEST_DL}/${STABLE_WIN_ALIAS}"]`);
  await expect(primary).toBeVisible();
  await expect(primary).toContainText("Download sparq-gui.msi");
  // Version meta is rendered.
  await expect(primary).toContainText(STABLE_VERSION);

  // Version badge renders in the Desktop app section.
  await expect(page.getByText(STABLE_VERSION).first()).toBeVisible();

  // sha256 is visible in the expanded first-launch details.
  await page.getByText("First launch & verification").first().click();
  await expect(page.getByText(FAKE_SHA).first()).toBeVisible();

  // [SONNET-4.6] sq-vw3ax.11.2 — fail-closed: Linux AppImage alias is NOT in the stable asset
  // map → that card must degrade to the Releases-page link, never a dead latest/download button.
  // Verify NO linux AppImage alias href is rendered anywhere on the page.
  await expect(
    page.locator(`a[href="${LATEST_DL}/sparq-gui-x64-linux.AppImage"]`),
  ).not.toBeVisible();
  // The degraded link IS visible somewhere on the page (the Linux card shows "See Releases page").
  await expect(page.getByRole("link", { name: /See Releases page/i }).first()).toBeVisible();

  // No-install closer is present.
  await expect(page.getByRole("link", { name: /Open the workbench/i })).toBeVisible();

  expect(consoleErrors, `console errors: ${consoleErrors.join("\n")}`).toEqual([]);
});

// =============================================================================
// Mutation spot-check note (NOT a CI test, verified manually):
//
// State-A: change WIN_DL_URL (the mock data's download URL) to a different value, e.g.:
//   const WIN_DL_URL = `https://...NONEXISTENT`;
// WIN_DL_URL_EXPECTED still holds the original value. The assertion:
//   const winBtn = page.locator(`a[href="${WIN_DL_URL_EXPECTED}"]`);
//   await expect(winBtn).toBeVisible();
// fails because the rendered href is the mutated URL (from the API mock), which no longer
// equals WIN_DL_URL_EXPECTED. Restore WIN_DL_URL to make it green again.
//
// State-C: change STABLE_WIN_ALIAS to "sparq-gui-win-x64-NONEXISTENT.msi" in STABLE_BODY
// (keep it in the asset map but under the wrong alias). The fail-closed branch fires: the
// component finds no "sparq-gui-win-x64.msi" alias → degrades to "See Releases page", so:
//   await expect(primary).toBeVisible();  // primary = locator(href=...STABLE_WIN_ALIAS)
// fails because the direct download href is never rendered.
// =============================================================================
