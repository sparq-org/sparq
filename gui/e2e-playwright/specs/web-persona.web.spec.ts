// [FABLE-5] sq-u0my5 — web-persona smoke: the workbench boots and a tool works with NO Tauri.
//
// Runs under the chromium-web project (support/web-fixtures.ts): no window.__TAURI__ /
// __TAURI_INTERNALS__ is ever installed, so isTauriRuntime() === false and the page takes the
// pure-browser code paths of the deployed /app — the persona the mock-IPC lane structurally
// cannot see. The webPersona auto-fixture already guarantees, before the test body runs:
//   * hermetic network (only 127.0.0.1 / localhost / blob: / data:),
//   * the Tauri globals are genuinely absent,
//   * the in-tab WASM engine reached "Engine ready" (browser-only boot).
//
// This spec then proves:
//   1. the Query tool renders AND executes the default query against the in-tab engine
//      (a tool demonstrably *works*, not merely mounts, without a native backend);
//   2. the Import drawer's File tab shows a browser multi-file uploader (sq-eydh9) in the
//      browser persona, NOT the old "Desktop app only" notice — and the uploader renders
//      the Dropzone + the stable data-web-file-input the sq-eydh9 Playwright spec targets.
//
// Stable selectors (all declared E2E hooks — role / data-* only):
//   #repl-query                 — the SPARQL editor textarea (query-workbench.tsx contract)
//   button "Run query"          — the run trigger
//   [data-result-kind="select"] — SELECT result container
//   [data-import-trigger="rail"]— the left rail's "+ Import data…" entry point
//   [data-import-drawer]        — the Import drawer dialog content
//   [data-import-tab="file"] / ["paste"] — the drawer's tab buttons (role="tab", aria-selected)
//   [data-web-file-input]       — the stable hidden <input type="file"> for browser upload
//
// Determinism rules: NO waitForTimeout; NO exact numeric assertions; web-first assertions only.

import { webTest as test, webExpect as expect } from "../support/index.ts";

test.describe("web-persona", () => {
  // The webPersona auto-fixture navigates to "/" and waits for engine ready before each test.

  test("theme toggle hydrates with its icon and no hydration warnings", async ({ page }) => {
    const hydrationErrors: string[] = [];
    page.on("console", (message) => {
      if (/hydrat|Minified React error #(418|423|425)/i.test(message.text())) {
        hydrationErrors.push(message.text());
      }
    });
    page.on("pageerror", (error) => {
      if (/hydrat|Minified React error #(418|423|425)/i.test(error.message)) {
        hydrationErrors.push(error.message);
      }
    });
    // Observe a full load, including the initial server markup and client hydration.
    await page.reload();
    const toggle = page.getByRole("button", { name: "Toggle theme", exact: true });
    await expect(toggle.locator("svg")).toBeVisible();
    await expect(page.getByText("Engine ready", { exact: true })).toBeVisible();
    await toggle.click();
    await expect(page.locator("html")).toHaveClass(/light/);
    await expect(toggle.locator("svg")).toBeVisible();
    expect(hydrationErrors).toEqual([]);
  });

  test("workbench boots and the Query tool runs against the in-tab engine", async ({ page }) => {
    // The workbench shell is up: the SPARQL editor and its run trigger render.
    const editor = page.locator("#repl-query");
    const runBtn = page.getByRole("button", { name: "Run query" });
    await expect(editor).toBeVisible();
    await expect(runBtn).toBeEnabled();

    // The default query executes against the in-tab WASM engine — no native backend exists in
    // this persona, so a SELECT result rendering proves the browser code path works end-to-end.
    await runBtn.click();
    const selectResult = page.locator('[data-result-kind="select"]');
    await expect(selectResult).toBeVisible();

    // The seeded sample graph is loaded (Alice is its first ORDER BY ?name row); presence-only,
    // no row-count / timing assertion.
    await expect(selectResult.getByText("Alice").first()).toBeVisible();
  });

  test("import drawer File tab shows web uploader (not desktop-only notice) in the browser persona", async ({
    page,
  }) => {
    // Open the Import drawer from the left rail.
    await page.locator('[data-import-trigger="rail"]').click();
    const drawer = page.locator("[data-import-drawer]");
    await expect(drawer).toBeVisible();

    // Browser persona default: the drawer opens on Paste (the File tab is the desktop default —
    // import-drawer.tsx picks the initial tab off isTauriRuntime()).
    await expect(drawer.locator('[data-import-tab="paste"]')).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // (sq-eydh9) The File tab now shows the web multi-file uploader, NOT the old "Desktop app
    // only" notice.  Confirm: the old warning is gone, and the web upload affordances are present.
    await drawer.locator('[data-import-tab="file"]').click();
    await expect(drawer.getByText("Desktop app only")).not.toBeAttached();

    // The Dropzone "Browse files…" button is present (keyboard-accessible picker path).
    await expect(drawer.getByRole("button", { name: /Browse files/i })).toBeVisible();

    // The stable hidden input for Playwright setInputFiles() is in the DOM.
    await expect(drawer.locator("[data-web-file-input]")).toBeAttached();

    // An honest note about what the web path CANNOT do (compressed / HDT) is present so the
    // capability boundary is never silently misrepresented.
    await expect(drawer.getByText(/HDT/).first()).toBeVisible();

    // Close the drawer to leave the page in its initial state.
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();
  });
});
