// [SONNET-4.6] sq-glo5r — N3 inference journey (browser / web persona):
//   (a) Load data + an N3 rule → the SPECIFIC derived triple (ex:x a ex:B) answers an ASK true.
//   (b) BULK upload: setInputFiles with two .n3 files in one action → both listed, each with a
//       per-source enable toggle.
//   (c) Toggling a rule off removes its derivations after re-materialisation (ASK flips false).
//   (d) Rules persist per workspace: the localStorage record carries rulesDocs, and a full page
//       reload restores the rule list + the N3 regime.
//
// NON-VACUOUS by design: the gating Playwright lane (gui.yml) BUILDS the W-reason bundle
// (npm run build:reason-wasm) before exporting the app, so `reasonN3` is present and these
// tests REQUIRE the reasoner to reach "ready" and the derived triple to appear. A missing
// bundle fails this spec loudly — that is a build/sync regression, not a tolerated state.
// (The app itself still degrades honestly at runtime: a missing bundle hard-fails queries
// with a clear message while N3 is on — that error UI is product behaviour, but this spec
// never accepts it as a pass.)
//
// Runs under the chromium-web project (support/web-fixtures.ts): no Tauri globals, pure
// browser code path — the deployed /app persona.
//
// Stable selectors (sq-glo5r data-* contract):
//   [data-tool="inference"] / [data-tool="query"] — left-rail tool nav buttons
//   [data-inference-mode="n3"]       — the N3 mode button (in InferenceControl; see scoping note)
//   [data-n3-rules]                  — the N3 rules panel (visible when inference === "n3")
//   [data-n3-rule-row]               — one rule row (keyed by addedAt)
//   [data-n3-rule-toggle]            — enable/disable checkbox inside a rule row
//   [data-n3-rule-remove]            — remove button inside a rule row
//   [data-n3-author-name]            — the rule-name input
//   #n3-author-textarea              — the inline N3 editor textarea
//   [data-n3-add-rule]               — the "Add rule" button
//   [data-n3-file-input]             — stable hidden <input type="file"> for bulk upload
//   [data-inference-status]          — live status pill (loading / ready / error)
//
// SCOPING NOTE: InferenceControl is rendered in BOTH the query-toolbar (QueryWorkbench) and the
// inference tool panel (InferenceTool).  Once the Inference tab is opened, both are in the DOM
// simultaneously (the inactive tab is hidden via the HTML `hidden` attribute, not unmounted).
// Playwright strict mode counts ALL matching elements regardless of visibility, so an unscoped
// `[data-inference-mode="n3"]` resolves to 2 elements → strict-mode violation.
// Fix: scope those selectors through the inference tab panel: `[data-tab="inference"]`.
// Selectors that live ONLY in InferenceTool (data-n3-rules, data-n3-rule-row, etc.) are safe
// without scoping.
//
// Determinism notes: NO waitForTimeout; web-first assertions only. The workspace mutators
// (addRulesDocs / setRulesDocEnabled) push the rules into the engine SYNCHRONOUSLY inside the
// click handler (engineRef.current.setN3Rules), and run() re-derives the closure cache key from
// the live rules on every query — so an ASK issued after a toggle deterministically sees the
// post-toggle closure with no sleep.

import { webTest as test, webExpect as expect, waitForEngineReady } from "../support/index.ts";

type Page = import("@playwright/test").Page;
type Locator = import("@playwright/test").Locator;

const RULE_A_TO_B = "@prefix ex: <http://example.org/> . { ?s a ex:A } => { ?s a ex:B . }";
const DATA_X_A = "@prefix ex: <http://example.org/> . ex:x a ex:A .";
const ASK_DERIVED = "PREFIX ex: <http://example.org/> ASK { ex:x a ex:B }";

/** Record even a loading state that begins and ends between Playwright assertions. */
async function observeInferenceStatus(page: Page): Promise<void> {
  await page.evaluate(() => {
    const states: (string | null)[] = [];
    (window as unknown as { inferenceStates: (string | null)[] }).inferenceStates = states;
    new MutationObserver((records) => {
      for (const record of records) {
        if (record.type === "attributes") {
          states.push(record.oldValue, (record.target as Element).getAttribute("data-inference-status"));
        } else {
          for (const node of [...record.addedNodes, ...record.removedNodes]) {
            if (!(node instanceof Element)) continue;
            if (node.matches("[data-inference-status]")) states.push(node.getAttribute("data-inference-status"));
            for (const pill of node.querySelectorAll("[data-inference-status]")) {
              states.push(pill.getAttribute("data-inference-status"));
            }
          }
        }
      }
    }).observe(document.body, {
      subtree: true, childList: true, attributes: true,
      attributeFilter: ["data-inference-status"], attributeOldValue: true,
    });
  });
}

/** Fail even if React removes and restores the why affordance within one autosave. */
async function observeWhyMarker(row: Locator): Promise<void> {
  await row.evaluate((element) => {
    const history = { lost: false };
    (window as unknown as { whyMarkerHistory: typeof history }).whyMarkerHistory = history;
    new MutationObserver((records) => {
      if (!element.hasAttribute("data-inferred-row") || !element.querySelector("[data-explain-trigger]")) {
        history.lost = true;
      }
      for (const record of records) {
        if (record.type === "attributes" && record.target === element && record.oldValue === null) {
          history.lost = true;
        }
        for (const node of record.removedNodes) {
          if (node.contains(element) ||
              (element.contains(record.target) && node instanceof Element &&
                (node.matches("[data-explain-trigger]") || node.querySelector("[data-explain-trigger]")))) {
            history.lost = true;
          }
        }
      }
    }).observe(document.body, {
      subtree: true, childList: true, attributes: true,
      attributeFilter: ["data-inferred-row"], attributeOldValue: true,
    });
  });
}

async function autosaveQuery(page: Page, query: string): Promise<void> {
  await fillReactTextarea(page, "#repl-query", query);
  await expect.poll(() => page.evaluate((text) => {
    for (const key of Object.keys(localStorage)) {
      if (!key.startsWith("sparq.workspace.v1.")) continue;
      if (key.endsWith("__last__") || key.endsWith("__index__")) continue;
      const workspace = JSON.parse(localStorage.getItem(key) ?? "{}");
      if (workspace.editor?.query === text) return true;
    }
    return false;
  }, query)).toBe(true);
  // Let post-save React effects and the mutation observer drain before checking the history.
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
  const states = await page.evaluate(() =>
    (window as unknown as { inferenceStates: (string | null)[] }).inferenceStates,
  );
  expect(states).not.toContain("loading");
  expect(states).not.toContain("error");
  await expect(page.locator('[data-tab="query"] [data-inference-status]')).toHaveAttribute(
    "data-inference-status", "ready",
  );
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/** Set a React-controlled textarea via the native setter + input event. */
async function fillReactTextarea(page: Page, selector: string, value: string): Promise<void> {
  await page.evaluate(
    ([sel, text]) => {
      const el = document.querySelector<HTMLTextAreaElement>(sel);
      if (!el) throw new Error(`Textarea not found: ${sel}`);
      const setter = Object.getOwnPropertyDescriptor(
        window.HTMLTextAreaElement.prototype,
        "value",
      )?.set;
      if (!setter) throw new Error("Could not get native value setter");
      setter.call(el, text);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    },
    [selector, value] as [string, string],
  );
}

/** Paste-import a Turtle document in REPLACE mode and wait for the ok feedback. */
async function importPasteReplace(page: Page, turtle: string): Promise<void> {
  await page.locator('[data-import-trigger="rail"]').click();
  const drawer = page.locator("[data-import-drawer]");
  await expect(drawer).toBeVisible();
  await drawer.locator('[data-import-tab="paste"]').click();
  await drawer.locator("textarea").first().fill(turtle);
  await drawer.getByRole("button", { name: "Replace store" }).click();
  await drawer.getByRole("button", { name: /Import \(replace store\)/ }).click();
  await expect(drawer.locator('[data-import-feedback="ok"]')).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden();
}

/**
 * Open the Inference tool and switch the workspace to the N3 regime.
 *
 * Returns the inference tab-panel locator (`[data-tab="inference"]`) so callers can scope
 * follow-up `[data-inference-mode]` / `[data-inference-status]` queries through it and avoid
 * the strict-mode collision with the query-toolbar's InferenceControl (see SCOPING NOTE above).
 */
async function selectN3(page: Page): Promise<Locator> {
  await page.locator('[data-tool="inference"]').click();
  // Scope to the inference tab panel — avoids the query-toolbar's InferenceControl duplicate.
  const panel = page.locator('[data-tab="inference"]');
  const n3Btn = panel.locator('[data-inference-mode="n3"]');
  await expect(n3Btn).toBeVisible();
  await n3Btn.click();
  await expect(n3Btn).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("[data-n3-rules]")).toBeVisible();
  return panel;
}

/** Author an inline rule in the N3 rules panel; waits for its row to appear. */
async function authorRule(page: Page, name: string, text: string): Promise<void> {
  const before = await page.locator("[data-n3-rule-row]").count();
  await page.getByRole("button", { name: /Author a rule/i }).click();
  await page.locator("[data-n3-author-name]").fill(name);
  await fillReactTextarea(page, "#n3-author-textarea", text);
  await page.locator("[data-n3-add-rule]").click();
  await expect(page.locator("[data-n3-rule-row]")).toHaveCount(before + 1);
}

/** Run an ASK in the Query tool and assert its boolean rendering. */
async function runAsk(page: Page, query: string, expected: "true" | "false"): Promise<void> {
  await page.locator('[data-tool="query"]').click();
  await fillReactTextarea(page, "#repl-query", query);
  await page.getByRole("button", { name: "Run query" }).click();
  const result = page.locator('[data-result-kind="ask"]');
  await expect(result).toBeVisible({ timeout: 60_000 });
  await expect(result).toContainText(expected);
}

test.describe("n3-inference", () => {
  // The webPersona auto-fixture navigates to "/" and waits for engine ready before each test.

  test("fresh workspace N3 stays ready through editor autosaves without rule documents", async ({ page }) => {
    await page.locator('[data-tab="query"] [data-inference-mode="n3"]').click();
    await expect(page.locator('[data-tab="query"] [data-inference-status="ready"]')).toBeVisible();
    await observeInferenceStatus(page);
    await autosaveQuery(page, "SELECT ?s ?p ?o WHERE { ?s ?p ?o } # first autosave");
    await autosaveQuery(page, "SELECT ?s ?p ?o WHERE { ?s ?p ?o } # second autosave");
  });

  test("N3 entailed keys keep why markers available through an editor autosave", async ({ page }) => {
    await importPasteReplace(page, DATA_X_A);
    const panel = await selectN3(page);
    await authorRule(page, "autosave-rule", RULE_A_TO_B);
    await expect(panel.locator('[data-inference-status="ready"]')).toBeVisible();
    await page.locator('[data-tool="query"]').click();
    const query = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    await fillReactTextarea(page, "#repl-query", query);
    await page.getByRole("button", { name: "Run query" }).click();
    const inferredRow = page.locator("[data-inferred-row]").filter({
      has: page.getByRole("cell", { name: "<http://example.org/B>", exact: true }),
    });
    await expect(inferredRow).toHaveCount(1);
    await expect(inferredRow.locator("[data-explain-trigger]")).toBeVisible();
    await observeInferenceStatus(page);
    await observeWhyMarker(inferredRow);
    await autosaveQuery(page, `${query} # keep the entailment cache`);
    expect(await page.evaluate(() =>
      (window as unknown as { whyMarkerHistory: { lost: boolean } }).whyMarkerHistory.lost,
    )).toBe(false);
    await expect(inferredRow).toHaveCount(1);
    await expect(inferredRow.locator("[data-explain-trigger]")).toBeVisible();
  });

  test("(a) data + an N3 rule derives a specific triple visible to queries", async ({ page }) => {
    // 1. Replace the store with exactly one asserted triple: ex:x a ex:A.
    await importPasteReplace(page, DATA_X_A);

    // 2. Switch to the N3 regime and attach the {A}=>{B} rule.
    const panel = await selectN3(page);
    await authorRule(page, "test-rule", RULE_A_TO_B);

    // 3. The reasoner must reach READY (the Playwright lane always ships the W-reason bundle;
    //    "error" here is a real regression, never a tolerated alternative).
    //    Scope through `panel` to avoid the query-toolbar's duplicate InferenceStatusPill.
    await expect(panel.locator('[data-inference-status="ready"]')).toBeVisible({
      timeout: 60_000,
    });

    // 4. The SPECIFIC derived triple: ex:x a ex:B was never asserted — only the rule can put it
    //    in query results. This is the non-vacuous core assertion of sq-glo5r.
    await runAsk(page, ASK_DERIVED, "true");

    // 5. Fail-closed sanity: the derived triple is query-time only. Switching inference Off
    //    returns exactly the asserted data (the derivation disappears).
    //    Navigate back to inference, then scope the mode button through the panel.
    await page.locator('[data-tool="inference"]').click();
    await panel.locator('[data-inference-mode="off"]').click();
    await runAsk(page, ASK_DERIVED, "false");
  });

  test("(b) BULK upload: two .n3 files in one action, each listed with a toggle", async ({
    page,
  }) => {
    await selectN3(page);

    // One setInputFiles call = one user action carrying BOTH files (the bulk contract).
    const input = page.locator("[data-n3-file-input]");
    await expect(input).toBeAttached();
    await input.setInputFiles([
      {
        name: "rule-a.n3",
        mimeType: "text/plain",
        buffer: Buffer.from(RULE_A_TO_B),
      },
      {
        name: "rule-b.n3",
        mimeType: "text/plain",
        buffer: Buffer.from(
          "@prefix ex: <http://example.org/> . { ?s a ex:C } => { ?s a ex:D . }",
        ),
      },
    ]);

    // Both rules are listed, each row carrying its own per-source enable toggle.
    const rows = page.locator("[data-n3-rule-row]");
    await expect(rows).toHaveCount(2);
    await expect(page.locator("[data-n3-rules]")).toContainText("rule-a.n3");
    await expect(page.locator("[data-n3-rules]")).toContainText("rule-b.n3");
    await expect(page.locator("[data-n3-rule-toggle]")).toHaveCount(2);
    for (const i of [0, 1]) {
      await expect(page.locator("[data-n3-rule-toggle]").nth(i)).toBeChecked();
    }
  });

  test("(c) toggling a rule off removes its derivations after re-materialisation", async ({
    page,
  }) => {
    // Full chain first: data + rule + N3 → the derived triple answers true.
    await importPasteReplace(page, DATA_X_A);
    const panel = await selectN3(page);
    await authorRule(page, "toggle-test", RULE_A_TO_B);
    // Scope status check through the panel (avoids the toolbar's InferenceStatusPill duplicate).
    await expect(panel.locator('[data-inference-status="ready"]')).toBeVisible({
      timeout: 60_000,
    });
    await runAsk(page, ASK_DERIVED, "true");

    // Toggle the rule OFF. setRulesDocEnabled pushes the new rules list into the engine
    // synchronously, so the next query re-materialises without the rule.
    await page.locator('[data-tool="inference"]').click();
    const toggle = page.locator("[data-n3-rule-toggle]").first();
    await expect(toggle).toBeChecked();
    await toggle.uncheck();
    await expect(toggle).not.toBeChecked();

    // The derivation is GONE from query results after re-materialisation — while N3 stays ON
    // and the base data is untouched.
    await runAsk(page, ASK_DERIVED, "false");
    await runAsk(page, "PREFIX ex: <http://example.org/> ASK { ex:x a ex:A }", "true");

    // And the re-materialised closure reports zero entailed triples in the status pill.
    await page.locator('[data-tool="inference"]').click();
    // Scope through the panel to avoid the toolbar's InferenceStatusPill duplicate.
    await expect(panel.locator('[data-inference-status="ready"]')).toContainText("+0 entailed", {
      timeout: 60_000,
    });
  });

  test("(d) rules persist per workspace and survive a reload", async ({ page }) => {
    await selectN3(page);
    await authorRule(page, "persist-test", RULE_A_TO_B);

    // The workspace record in localStorage carries the rulesDocs array (deterministic flush:
    // poll until the persisted record contains the rule — same pattern as workspace-restore).
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const prefix = "sparq.workspace.v1.";
            for (let i = 0; i < localStorage.length; i++) {
              const k = localStorage.key(i);
              if (!k?.startsWith(prefix)) continue;
              if (k.endsWith("__index__") || k.endsWith("__last__")) continue;
              try {
                const ws = JSON.parse(localStorage.getItem(k) ?? "{}") as {
                  inference?: string;
                  rulesDocs?: Array<{ name?: string }>;
                };
                if (
                  ws.inference === "n3" &&
                  Array.isArray(ws.rulesDocs) &&
                  ws.rulesDocs.some((d) => d?.name === "persist-test")
                ) {
                  return true;
                }
              } catch {
                /* skip corrupt entries */
              }
            }
            return false;
          }),
        { timeout: 5_000 },
      )
      .toBe(true);

    // RELOAD — the restored workspace must come back in the N3 regime with the rule listed.
    await page.reload();
    await waitForEngineReady(page, { timeout: 90_000 });

    // After reload only the Query tab is open; open Inference and get a fresh panel locator.
    await page.locator('[data-tool="inference"]').click();
    const panel = page.locator('[data-tab="inference"]');
    // Scope through panel to avoid the toolbar's InferenceControl duplicate (SCOPING NOTE).
    await expect(panel.locator('[data-inference-mode="n3"]')).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.locator("[data-n3-rules]")).toBeVisible();
    await expect(page.locator("[data-n3-rule-row]")).toHaveCount(1);
    await expect(page.locator("[data-n3-rules]")).toContainText("persist-test");
  });
});
