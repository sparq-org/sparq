// [SONNET-4.6] sq-ymr2e.5 — workbench-query journey: SPARQL editor run/error/pagination.
//
// Exercises the Query Workbench (gui/app/src/components/workbench/query-workbench.tsx) end-to-end
// with the in-tab WASM engine running for real (NOT mocked).  The Tauri IPC layer IS mocked so
// the disk_usage / load_path / load_text commands resolve deterministically.
//
// Stable selectors used (declared in the E2E contract comment in query-workbench.tsx):
//   #repl-query                 — the SPARQL editor textarea
//   button "Run query"          — the run trigger (text match)
//   [data-result-kind="select"] — outer container when a SELECT result is rendered
//   [data-result-view="table"]  — the table sub-view (default for SELECT)
//   [data-result-kind="error"]  — rendered when the engine rejects the query
//   [data-result-pager]         — the pagination bar (present whenever totalRows > 0)
//
// Determinism rules: NO waitForTimeout; NO exact numeric assertions (row counts, ms values);
// web-first assertions on visible UI state only.

import { test, expect } from "../support/index.ts";

// ---------------------------------------------------------------------------
// Helper: set the editor value via the native setter + dispatch an input event.
// Required because WorkbenchSparqlEditor is a React-controlled <textarea>: directly
// assigning `el.value` would bypass React's synthetic event system.
// ---------------------------------------------------------------------------
async function setEditorValue(
  page: import("@playwright/test").Page,
  value: string,
): Promise<void> {
  await page.evaluate((text) => {
    const el = document.querySelector<HTMLTextAreaElement>("#repl-query");
    if (!el) throw new Error("Editor textarea #repl-query not found");
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      "value",
    )?.set;
    if (!setter) throw new Error("Could not access native value setter");
    setter.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
}

test.describe("workbench-query", () => {
  // The tauriMock auto-fixture navigates to "/" and waits for engine ready before each test.

  test("default query runs and shows Alice in the result table", async ({ page }) => {
    // The default query is a SELECT over the seeded sample graph (foaf:name + foaf:age,
    // ORDER BY name → Alice · Bob · Carol · Dan).
    const runBtn = page.getByRole("button", { name: "Run query" });
    await expect(runBtn).toBeEnabled();
    await runBtn.click();

    // Wait for the SELECT result container.
    const selectResult = page.locator('[data-result-kind="select"]');
    await expect(selectResult).toBeVisible();

    // The table sub-view is the default for SELECT.
    const tableView = page.locator('[data-result-view="table"]');
    await expect(tableView).toBeVisible();

    // Alice is the first row (ORDER BY ?name, "Alice" < "Bob" < "Carol" < "Dan").
    // Presence-check only — never assert an exact row count.
    await expect(page.getByRole("cell", { name: "Alice" })).toBeVisible();
  });

  test("custom query shows a result table", async ({ page }) => {
    // Clear the editor and type a simple SELECT.
    await setEditorValue(page, "SELECT * WHERE { ?s ?p ?o } LIMIT 5");

    const runBtn = page.getByRole("button", { name: "Run query" });
    await runBtn.click();

    // Wait for any SELECT result (not error).
    const selectResult = page.locator('[data-result-kind="select"]');
    await expect(selectResult).toBeVisible();
  });

  test("entity-shaped SELECT renders as a node-link Graph view", async ({ page }) => {
    await setEditorValue(page, "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5");
    await page.getByRole("button", { name: "Run query" }).click();
    await expect(page.locator('[data-result-kind="select"]')).toBeVisible();

    await page.getByRole("button", { name: "Graph", exact: true }).click();
    await expect(page.locator("[data-select-result-graph]")).toBeVisible();
    await expect(
      page.getByRole("img", { name: /Node-link graph of .* from the SELECT result/ }),
    ).toBeVisible();
  });

  test("error result on invalid SPARQL", async ({ page }) => {
    await setEditorValue(page, "THIS IS NOT SPARQL AT ALL");

    const runBtn = page.getByRole("button", { name: "Run query" });
    await runBtn.click();

    // The engine rejects the query and renders an error container.
    const errorResult = page.locator('[data-result-kind="error"]');
    await expect(errorResult).toBeVisible();
  });

  test("pagination bar is present after a query with results", async ({ page }) => {
    // Run the default query (4 rows — the sample graph has Alice, Bob, Carol, Dan).
    // The pager renders whenever totalRows > 0 (even for a single page).
    const runBtn = page.getByRole("button", { name: "Run query" });
    await runBtn.click();

    await expect(page.locator('[data-result-kind="select"]')).toBeVisible();

    // The pager shows the row-range + page-nav controls whenever there are results.
    const pager = page.locator("[data-result-pager]");
    await expect(pager).toBeVisible();
  });

  test("new results and page-size changes reset pagination", async ({ page }) => {
    const values = Array.from({ length: 20 }, (_, index) => index).join(" ");
    await setEditorValue(page,
      `SELECT (?a * 20 + ?b AS ?n) WHERE { VALUES ?a { ${values} } VALUES ?b { ${values} } } ORDER BY ?n`,
    );
    const run = page.getByRole("button", { name: "Run query" });
    await run.click();
    const pager = page.locator("[data-result-pager]");
    const pageNumber = pager.locator("[data-pager-page]");
    await expect(pageNumber).toHaveText("Page 1 / 4");
    await pager.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(pageNumber).toHaveText("Page 2 / 4");

    await pager.getByRole("combobox", { name: "Rows per page" }).selectOption("50");
    await expect(pageNumber).toHaveText("Page 1 / 8");
    await pager.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(pageNumber).toHaveText("Page 2 / 8");
    await run.click();
    await expect(pageNumber).toHaveText("Page 1 / 8");
    await expect(page.locator('[data-result-view="table"] tbody tr').first()).toHaveText('"0"^^xsd:integer');
  });
});
