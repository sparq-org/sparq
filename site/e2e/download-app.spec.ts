// [SONNET-4.6] sq-ymr2e.7 — /download + /app conversion journey E2E.
//
// WHAT IT GUARDS.
//
// 1. /download · GitHub API hermetic fixture (release / notFound / error):
//    – Windows primary row: version-agnostic alias href + version tag + sha256 from the checked-in
//      fixture.
//    – macOS primary row: arm64-darwin alias promoted by a macOS UA.
//    – All five GUI platform aliases render in the DOM under the FAIL-CLOSED ready scheme
//      (sq-vw3ax.11.2): a direct latest/download/<alias> button appears iff that alias name is in
//      the release's asset map — the checked-in `release` fixture carries the full canonical alias
//      set, so every card renders its direct button.
//    – 404 state (no stable release, prerelease-list empty): every button degrades to
//      "No release yet — watch releases" → Releases page; NO direct-download hrefs in the DOM.
//    – 500 state: graceful degradation — buttons stay live optimistically with the static
//      latest/download hrefs (the alias URL needs no API).
//
// 2. /app hard-document-navigation guard (regression):
//    – The "App" nav slot is a PLAIN <a> (hard navigation), not a next/link soft-nav. If it were
//      soft-nav the browser would request the wrong RSC payload (/app/index.txt) from the sparq
//      Next.js build, breaking the /app shell entirely. The test marks the document, clicks the
//      link, waits for the target URL, then asserts the document sentinel is GONE — proving a full
//      hard navigation replaced the document.
//
// HERMETIC-NETWORK INFRASTRUCTURE. The GitHub API is routed via the hermetic-network fixture
// (github: "release" | "notFound" | "error") — no live network hits. `serviceWorkers: "block"` is
// required for all /download tests because the coi-serviceworker re-issues fetches from inside the
// SW, bypassing `context.route`.
//
// Design of record: research/web-gui-test-program.md §1 (hermetic doctrine) + sq-ymr2e.7 bead.
import type { Page } from "@playwright/test";
import { test, expect, gotoAppReady } from "./support";

// [FABLE-5] sq-vw3ax.11.2 — repo renamed sparq-org/sparq → sparq-org/sparq; the /download client's
// stable-alias hrefs and Releases link now point at the canonical org.
const LATEST_DL = "https://github.com/sparq-org/sparq/releases/latest/download";
const RELEASES_URL = "https://github.com/sparq-org/sparq/releases";
const FIXTURE_VERSION = "v0.4.2-e2e-fixture";
const FIXTURE_WIN_SHA256 = "1".repeat(64);

// Light site-e2e CI lane builds no wasm bundle → optional-wasm fetches 404 benignly; filter that
// class of resource error while still catching every REAL client-side fault.
const BENIGN_RESOURCE_404 = /Failed to load resource:.*\b404\b/i;
// Blocking the service worker makes the coi-serviceworker registration script throw a benign
// "reading 'scope'" TypeError — a test-only artifact, never a product fault.
const BENIGN_SW_BLOCK = /reading 'scope'|ServiceWorker.*(register|controller)/i;

function trackConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (BENIGN_SW_BLOCK.test(text) || BENIGN_RESOURCE_404.test(text)) return;
    errors.push(text);
  });
  page.on("pageerror", (err) => {
    const text = String(err);
    if (BENIGN_SW_BLOCK.test(text)) return;
    errors.push(text);
  });
  return errors;
}

// ── §1 — /download · release fixture · Windows primary row ──────────────────────────────────────

test.describe("/download · release fixture · Windows primary row", () => {
  // Desktop Chrome UA is Windows — the fixture is the checked-in release payload.
  test.use({ serviceWorkers: "block", github: "release" });

  test("Windows primary row: primary button href is the exact version-agnostic alias, version tag + sha256 render", async ({
    page,
  }) => {
    const consoleErrors = trackConsoleErrors(page);

    await page.goto("download/", { waitUntil: "domcontentloaded" });

    // Detected-platform heading.
    await expect(page.getByText("For your Windows PC")).toBeVisible();

    // Primary button has the static version-agnostic alias href.
    const primary = page
      .locator(`a[href="${LATEST_DL}/sparq-gui-win-x64.msi"]`)
      .first();
    await expect(primary).toBeVisible();

    // Version tag rendered from fixture.
    await expect(page.getByText(FIXTURE_VERSION).first()).toBeVisible();

    // Open the first-launch details to expose the sha256.
    await page.getByText("First launch & verification").first().click();
    await expect(page.getByText(FIXTURE_WIN_SHA256).first()).toBeVisible();

    expect(consoleErrors, `console errors:\n${consoleErrors.join("\n")}`).toEqual([]);
  });
});

// ── §2 — /download · release fixture · macOS primary row ────────────────────────────────────────

test.describe("/download · release fixture · macOS primary row", () => {
  test.use({
    serviceWorkers: "block",
    github: "release",
    // macOS UA: "mac" substring triggers the macos-arm detection bucket (arm64 is the default for
    // macOS because Apple Silicon vs Intel is not reliably distinguishable from the browser).
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
  });

  test("macOS primary row: primary button href is the exact arm64-darwin alias", async ({
    page,
  }) => {
    const consoleErrors = trackConsoleErrors(page);

    await page.goto("download/", { waitUntil: "domcontentloaded" });

    // macOS UA promotes the Apple-Silicon bucket.
    await expect(page.getByText("For your Mac (Apple Silicon)")).toBeVisible();

    // Static arm64-darwin alias href is present.
    const primary = page
      .locator(`a[href="${LATEST_DL}/sparq-gui-arm64-darwin.dmg"]`)
      .first();
    await expect(primary).toBeVisible();

    expect(consoleErrors, `console errors:\n${consoleErrors.join("\n")}`).toEqual([]);
  });
});

// ── §3 — /download · release fixture · all platform aliases present ──────────────────────────────

test.describe("/download · release fixture · all platform aliases present", () => {
  // Windows UA is fine — we just need the release state to verify all static hrefs are in the DOM.
  test.use({ serviceWorkers: "block", github: "release" });

  test("every platform alias href is present in DOM (static alias-asset scheme)", async ({
    page,
  }) => {
    await page.goto("download/", { waitUntil: "domcontentloaded" });

    // All five static alias hrefs must be attached to the DOM, regardless of which platform is
    // promoted. Verified with toBeAttached (not toBeVisible) because some are in the grid below
    // the fold or in a non-promoted card.
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-win-x64.msi"]`).first(),
    ).toBeAttached();
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-arm64-darwin.dmg"]`).first(),
    ).toBeAttached();
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-x64-darwin.dmg"]`).first(),
    ).toBeAttached();
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-x64-linux.AppImage"]`).first(),
    ).toBeAttached();
    // Secondary Linux .deb alias.
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-x64-linux.deb"]`).first(),
    ).toBeAttached();
  });
});

// ── §4 — /download · 404 fixture (no release yet) ───────────────────────────────────────────────

test.describe("/download · 404 fixture (no release yet)", () => {
  test.use({ serviceWorkers: "block", github: "notFound" });

  test("buttons degrade to watch-releases link in the no-release state", async ({ page }) => {
    const consoleErrors = trackConsoleErrors(page);

    // [FABLE-5] sq-vw3ax.11.2 — the hermetic layer fixture-routes only `/releases/latest`. On a
    // definitive 404 the client falls back to `/releases?per_page=1` (newest incl. prereleases) to
    // detect a dev pre-release. That list endpoint is NOT covered by the hermetic route, so mock it
    // here to return an EMPTY list → the client resolves to the "none" (no-release) state cleanly
    // (a blocked/aborted fetch would otherwise log a net::ERR_FAILED console error). The RegExp
    // matches the list endpoint (query string present) but NOT `/releases/latest`, so the hermetic
    // 404 fixture still drives the first fetch.
    await page.route(/api\.github\.com\/repos\/[^/]+\/sparq\/releases\?/, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        headers: {
          "access-control-allow-origin": "*",
          "cross-origin-resource-policy": "cross-origin",
        },
        body: "[]",
      });
    });

    await page.goto("download/", { waitUntil: "domcontentloaded" });

    // Unsigned-build honesty banner stays present.
    await expect(page.getByText("Developer builds — unsigned.")).toBeVisible();

    // Every download control is the "watch releases" link.
    const watchLink = page
      .getByRole("link", { name: /No release yet — watch releases/i })
      .first();
    await expect(watchLink).toBeVisible();
    await expect(watchLink).toHaveAttribute("href", RELEASES_URL);

    // NO direct-download hrefs in the DOM when there is no release.
    await expect(
      page.locator(`a[href^="${LATEST_DL}/"]`),
    ).toHaveCount(0);

    expect(consoleErrors, `console errors:\n${consoleErrors.join("\n")}`).toEqual([]);
  });
});

// ── §5 — /download · error fixture (graceful degradation) ───────────────────────────────────────

test.describe("/download · error fixture (graceful degradation)", () => {
  test.use({ serviceWorkers: "block", github: "error" });

  test("buttons stay live with static latest/download hrefs when API fetch fails", async ({
    page,
  }) => {
    await page.goto("download/", { waitUntil: "domcontentloaded" });

    // In the error state the buttons degrade to live static hrefs (not to the "watch" link), so
    // both primary GUI aliases are attached.
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-win-x64.msi"]`).first(),
    ).toBeAttached();
    await expect(
      page.locator(`a[href="${LATEST_DL}/sparq-gui-arm64-darwin.dmg"]`).first(),
    ).toBeAttached();
  });
});

// ── §6 — /app · hard-document-navigation guard ──────────────────────────────────────────────────

test.describe("/app · hard-document-navigation guard", () => {
  // Do NOT block serviceWorkers here — gotoAppReady needs the SW to fire the hydration signal.

  test("/app nav slot is a plain hard <a> with base-path-prefixed href (soft-nav regression guard)", async ({
    page,
  }) => {
    // Navigate to home and wait for SW + hydration (no sleep — uses event-based waiters).
    await gotoAppReady(page, "");

    // Locate the "App" link in the Primary navigation.
    const appLink = page
      .getByRole("navigation", { name: "Primary" })
      .first()
      .getByRole("link", { name: "App", exact: true });

    // The href must be the hard-navigation base-path-prefixed URL, not a next/link soft-nav route.
    await expect(appLink).toHaveAttribute("href", "/sparq/app/");

    // Mark the current document so we can detect whether a hard navigation replaced it.
    await page.evaluate(() => {
      (window as unknown as Record<string, unknown>).__sparq_e2e_doc_mark = "hard-nav-guard";
    });

    // Set up the navigation waiter BEFORE clicking so we don't miss the event.
    const navPromise = page.waitForURL("**/app/**", { waitUntil: "domcontentloaded" });

    await appLink.click();

    await navPromise;

    // If the navigation was SOFT (next/link client-side push), the same document remains and the
    // sentinel persists. If it was HARD, the document was replaced and the sentinel is gone.
    const markPresent = await page.evaluate(
      () =>
        (window as unknown as Record<string, unknown>).__sparq_e2e_doc_mark === "hard-nav-guard",
    );

    expect(
      markPresent,
      "Expected hard navigation (document replaced) but got soft-nav: sentinel survived. " +
        "The /app NavLink must render as a plain <a>, not a next/link, to avoid fetching the " +
        "wrong RSC payload (/app/index.txt) from the sparq Next.js build.",
    ).toBe(false);
  });
});
