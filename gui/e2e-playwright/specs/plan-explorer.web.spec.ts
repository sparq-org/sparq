// [FABLE-5] sq-ixc3.19 — plan-explorer Playwright spec (web persona).
//
// Drives the Plan tool (gui/app/src/components/workbench/plan-explorer.tsx) with the
// in-tab WASM engine running FOR REAL (the published bundle carries the `explain-json`
// structured-plan bindings), plus a hermetic page.route() mock of a sparq-server for the
// endpoint-mode tree, the text-plan degradation, and the monitor's endpoint Kill.
//
// Stable selectors (the panel's E2E contract):
//   [data-tool="plan"]            — left-rail nav button
//   #plan-query                   — the panel's SPARQL editor textarea
//   [data-plan-explain] / [data-plan-analyze]           — in-tab source buttons
//   [data-plan-result]            — result region (attr = idle|running|tree|text|error)
//   [data-plan-tree] / [data-plan-node] / [data-plan-severity] — the operator tree
//   [data-plan-time-note]         — the wasm "wall times unmeasured" honesty note
//   [data-plan-jump-hot]          — jump-to-hot-operator (only with measured times)
//   [data-plan-endpoint-url] / [data-plan-endpoint-analyze]   — endpoint strip
//   [data-plan-monitor] / [data-plan-monitor-row] / [data-plan-kill] — the monitor
//
// Determinism rules: NO waitForTimeout; NO exact numeric assertions (row counts, q-error
// values, ms) — severity BUCKETS and element visibility only; stable selectors only.

import { webTest as test, webExpect as expect } from "../support/index.ts";

const MOCK_URL = "http://127.0.0.1:7788/sparql";

/** Set the plan editor value via the native setter (React-controlled textarea). */
async function setPlanQuery(
  page: import("@playwright/test").Page,
  value: string,
): Promise<void> {
  // The Plan panel is a lazy chunk — page.evaluate does not auto-wait like a locator
  // action does, so wait for the editor to mount first (a web-first wait, no sleep).
  await page.locator("#plan-query").waitFor();
  await page.evaluate((text) => {
    const el = document.querySelector<HTMLTextAreaElement>("#plan-query");
    if (!el) throw new Error("Editor textarea #plan-query not found");
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      "value",
    )?.set;
    if (!setter) throw new Error("Could not access native value setter");
    setter.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
}

/** A canned structured plan (the sq-jbqh4 camelCase schema) WITH real wall nanos. */
const CANNED_TREE = JSON.stringify({
  operator: "Project ?name",
  estimated: null,
  actual: 1,
  nanos: 2500000,
  qError: null,
  children: [
    {
      operator: "BGP (1 patterns, 0 filters)",
      estimated: 40,
      actual: 1,
      nanos: 2000000,
      qError: 40,
      children: [],
    },
  ],
});

test.describe("plan-explorer (web persona)", () => {
  test.beforeEach(async ({ page }) => {
    await page.locator('[data-tool="plan"]').click();
  });

  test("in-tab EXPLAIN renders the structured operator tree (dry run)", async ({ page }) => {
    await setPlanQuery(page, "SELECT ?name WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?name }");
    await page.locator("[data-plan-explain]").click();

    const tree = page.locator("[data-plan-tree]");
    await expect(tree).toBeVisible();
    await expect(tree).toHaveAttribute("data-plan-source", "wasm");
    // At least the root operator row renders.
    await expect(page.locator("[data-plan-node]").first()).toBeVisible();
    // A dry run measures nothing — no wall-time honesty note (it is ANALYZE-only)…
    await expect(page.locator("[data-plan-time-note]")).toBeHidden();
    // …and no hot operator to jump to.
    await expect(page.locator("[data-plan-jump-hot]")).toBeHidden();
  });

  test("in-tab ANALYZE shows q-error heat on a known misestimate + the wasm time note", async ({
    page,
  }) => {
    // The planner estimates the BGP leaf from the pattern (all foaf:name rows in the
    // seeded sample graph) while the FILTER keeps a single binding — a genuine, fixture-
    // known misestimate, so SOME operator lands in a non-"none"/"ok" heat bucket.
    await setPlanQuery(
      page,
      'SELECT ?name WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?name FILTER(?name = "Alice") }',
    );
    await page.locator("[data-plan-analyze]").click();

    await expect(page.locator("[data-plan-tree]")).toBeVisible();
    // The summary strip renders (derived values only — no exact numbers asserted).
    await expect(page.locator("[data-plan-summary]")).toBeVisible();
    // q-error heat: at least one operator row sits in the warn/hot bucket.
    await expect(
      page.locator('[data-plan-severity="warn"], [data-plan-severity="hot"]').first(),
    ).toBeVisible();
    // HONESTY: the in-tab engine cannot measure wall time — the note says so.
    await expect(page.locator("[data-plan-time-note]")).toBeVisible();
  });

  test("operator rows collapse and expand (navigable tree)", async ({ page }) => {
    await setPlanQuery(
      page,
      "SELECT ?n ?a WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?n . ?s <http://xmlns.com/foaf/0.1/age> ?a }",
    );
    await page.locator("[data-plan-explain]").click();
    await expect(page.locator("[data-plan-tree]")).toBeVisible();

    const rows = page.locator("[data-plan-node]");
    const collapseBtn = page.getByRole("button", { name: "Collapse operator" }).first();
    await expect(collapseBtn).toBeVisible();
    // Collapse the root: only the root row remains; expand restores the children.
    await collapseBtn.click();
    await expect(rows).toHaveCount(1);
    await page.getByRole("button", { name: "Expand operator" }).first().click();
    await expect(rows.nth(1)).toBeVisible();
  });

  test("endpoint mode renders the structured tree with jump-to-hot (mocked server)", async ({
    page,
  }) => {
    await page.route(`${MOCK_URL}**`, (route) => {
      void route.fulfill({
        status: 200,
        body: CANNED_TREE,
        contentType: "application/x-sparq-explain+json; charset=utf-8",
      });
    });

    await page.locator("[data-plan-endpoint-url]").fill(MOCK_URL);
    await page.locator("[data-plan-endpoint-analyze]").click();

    const tree = page.locator("[data-plan-tree]");
    await expect(tree).toBeVisible();
    await expect(tree).toHaveAttribute("data-plan-source", "endpoint");
    // The canned plan carries REAL wall nanos → the hot-operator jump is offered…
    const jump = page.locator("[data-plan-jump-hot]");
    await expect(jump).toBeVisible();
    await jump.click();
    // …and the mocked misestimate (est 40 vs actual 1) lands in the hot bucket.
    await expect(page.locator('[data-plan-severity="hot"]').first()).toBeVisible();
    // A real-times source shows no wasm honesty note.
    await expect(page.locator("[data-plan-time-note]")).toBeHidden();
  });

  test("endpoint text-plan degradation renders verbatim text, never a faked tree", async ({
    page,
  }) => {
    // A lean server: 406 to the structured Accept, then the TEXT plan on the retry.
    let calls = 0;
    await page.route(`${MOCK_URL}**`, (route) => {
      calls += 1;
      if (calls === 1) {
        void route.fulfill({ status: 406, body: "explain-json unavailable", contentType: "text/plain" });
      } else {
        void route.fulfill({
          status: 200,
          body: "EXPLAIN (SELECT)\nPlan:\n  BGP …",
          contentType: "text/plain; charset=utf-8",
        });
      }
    });

    await page.locator("[data-plan-endpoint-url]").fill(MOCK_URL);
    await page.locator("[data-plan-endpoint-explain]").click();

    await expect(page.locator("[data-plan-text]")).toBeVisible();
    await expect(page.locator("[data-plan-tree]")).toBeHidden();
  });

  test("monitor lists an in-flight endpoint explain and Kill aborts it", async ({ page }) => {
    // Idle state first.
    await expect(page.locator("[data-plan-monitor-empty]")).toBeVisible();

    // A hanging endpoint: never fulfil — the request stays in flight until aborted.
    await page.route(`${MOCK_URL}**`, () => {
      /* hold the request open; the Kill aborts it client-side */
    });

    await page.locator("[data-plan-endpoint-url]").fill(MOCK_URL);
    await page.locator("[data-plan-endpoint-analyze]").click();

    const row = page.locator("[data-plan-monitor-row]");
    await expect(row).toBeVisible();
    const kill = row.locator("[data-plan-kill]");
    await expect(kill).toBeEnabled();
    await kill.click();

    // The abort surfaces: the run finishes (row delisted) and the panel returns to idle.
    await expect(page.locator("[data-plan-monitor-empty]")).toBeVisible();
    await expect(page.locator('[data-plan-result="idle"]')).toBeVisible();
  });

  test("endpoint registry monitoring is explicit, renders rows, and Kill refreshes", async ({
    page,
  }) => {
    let deleted = false;
    let listCalls = 0;
    await page.route("http://127.0.0.1:7788/queries**", (route) => {
      if (route.request().method() === "DELETE") {
        deleted = true;
        void route.fulfill({ status: 204 });
        return;
      }
      listCalls += 1;
      void route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          queries: deleted
            ? []
            : [{
                id: "0000000000000001",
                kind: "query",
                start: 1_700_000_000_000,
                fingerprint: "0123456789abcdef",
                elapsed_ms: 250,
              }],
        }),
      });
    });

    await page.locator("[data-plan-endpoint-url]").fill(MOCK_URL);
    await page.locator("[data-plan-endpoint-token]").fill("sentinel-read-token");
    expect(listCalls).toBe(0);
    await expect(page.locator("[data-plan-endpoint-monitor-row]")).toBeHidden();
    await expect(page.locator("[data-plan-endpoint-monitor-toggle]")).toBeEnabled();
    await page.locator("[data-plan-endpoint-monitor-toggle]").click();
    await expect(page.locator("[data-plan-endpoint-monitor-row]")).toBeVisible();
    await page.locator("[data-plan-endpoint-kill]").click();
    await expect(page.locator("[data-plan-endpoint-monitor-empty]")).toBeVisible();
    expect(deleted).toBe(true);
    expect(listCalls).toBeGreaterThan(1);
  });

  test("endpoint registry disabled is informational and polling stops", async ({ page }) => {
    let calls = 0;
    await page.clock.install();
    await page.route("http://127.0.0.1:7788/queries**", (route) => {
      calls += 1;
      void route.fulfill({ status: 404 });
    });

    await page.locator("[data-plan-endpoint-url]").fill(MOCK_URL);
    await page.locator("[data-plan-endpoint-monitor-toggle]").click();
    const message = page.locator("[data-plan-endpoint-monitor-error]");
    await expect(message).toContainText("registry not enabled");
    await expect(message).toHaveClass(/text-muted-foreground/);
    await page.clock.fastForward(5_000);
    expect(calls).toBe(1);
  });

  test("endpoint monitoring blocks a token only for non-loopback plaintext", async ({ page }) => {
    await page.locator("[data-plan-endpoint-url]").fill("http://example.test/sparql");
    await page.locator("[data-plan-endpoint-token]").fill("sentinel-read-token");
    await expect(page.locator("[data-plan-endpoint-monitor-toggle]")).toBeDisabled();
    await expect(page.getByText("bearer token would be sent over plaintext")).toBeVisible();
  });

  test("Query tool EXPLAIN now renders the shared operator tree", async ({ page }) => {
    // Both tool panels stay mounted (hidden tabs) — scope to the Query tab's subtree.
    await page.locator('[data-tool="query"]').click();
    const queryTab = page.locator('[data-tab="query"]');
    await queryTab.getByRole("button", { name: "EXPLAIN", exact: true }).click();
    const result = queryTab.locator('[data-result-kind="explain"]');
    await expect(result).toBeVisible();
    await expect(result.locator("[data-plan-tree]")).toBeVisible();
  });
});
