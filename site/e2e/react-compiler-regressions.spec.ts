import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test, expect, gotoAppReady } from "./support";

const wasmAvailable = existsSync(fileURLToPath(
  new URL("../public/wasm/sparq_wasm_bg.wasm", import.meta.url),
));

test.describe("compiler-compatible demo state", () => {
  test.skip(!wasmAvailable, "requires the lean wasm bundle in public/wasm");

  test("WASM panels refresh on deltas and reset, while input edits wait for a run", async ({ page }) => {
    await gotoAppReady(page, "surface/javascript-wasm/");
    await expect(page.getByText("Engine ready", { exact: true })).toBeVisible();
    await expect(page.getByText("6 foaf:Person", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Stream rows", exact: true }).click();
    await expect(page.getByText("6 rows in 3 batches of ≤2", { exact: true })).toBeVisible();
    for (const name of ["Alice", "Bob", "Carol", "Dave", "Erin", "Frank"]) {
      await expect(page.getByText(new RegExp(`^"${name}" · `))).toBeVisible();
    }
    await page.getByRole("button", { name: "match()", exact: true }).click();
    await page.getByRole("button", { name: "count()", exact: true }).click();
    await expect(page.getByText("countQuads() = 6", { exact: true })).toBeVisible();
    await expect(page.getByText("6 solutions", { exact: true })).toBeVisible();

    await page.getByLabel("object", { exact: true }).fill("<http://example.org/alice>");
    await expect(page.getByText("countQuads() = 6", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "+ insert :grace", exact: true }).click();
    await expect(page.getByText("7 foaf:Person", { exact: true })).toBeVisible();
    await expect(page.getByText("7 solutions", { exact: true })).toBeVisible();
    await expect(page.getByText("countQuads() = 3", { exact: true })).toBeVisible();
    const table = page.getByRole("table");
    await expect(table.getByRole("cell", { name: "<http://example.org/grace>", exact: true })).toBeVisible();
    await expect.poll(async () => (
      await table.locator("tbody tr td:first-child").allTextContents()
    ).sort()).toEqual([
      "<http://example.org/bob>",
      "<http://example.org/carol>",
      "<http://example.org/grace>",
    ]);

    await page.getByRole("button", { name: "− delete :frank", exact: true }).click();
    await expect(page.getByText("6 foaf:Person", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Reset to seed", exact: true }).click();
    await expect(page.getByText("6 solutions", { exact: true })).toBeVisible();
    await expect(page.getByText("countQuads() = 2", { exact: true })).toBeVisible();
    await expect(table.getByRole("cell", { name: "<http://example.org/grace>", exact: true })).toHaveCount(0);
  });

  test("Solid selectors re-run the edited query only after the first run", async ({ page }) => {
    await gotoAppReady(page, "showcase/solid-pairs/");
    const pairA = page.getByRole("group", { name: "Pair A", exact: true });
    await pairA.locator('input[value="owner-health-app"]').check();
    await expect(page.getByRole("table")).toHaveCount(0);
    await page.getByRole("button", { name: "Run the demo query for both pairs", exact: true }).click();
    await expect(page.getByRole("table").first()).toBeVisible();
    const editor = page.getByRole("textbox").first();
    await editor.fill("ASK WHERE { GRAPH <https://alice.pod.example/health/records> { ?s ?p ?o } }");
    // Editing alone keeps the previous SELECT table until a selector or Run changes it.
    await expect(page.getByRole("table").first()).toBeVisible();
    await pairA.locator('input[value="owner-own-app"]').check();
    await expect(page.getByRole("table")).toHaveCount(0);
    await expect(page.getByText("ASK → false", { exact: true })).toHaveCount(2);
    await pairA.locator('input[value="owner-health-app"]').check();
    await expect(page.getByText("ASK → true", { exact: true })).toHaveCount(1);
    await expect(page.getByText("ASK → false", { exact: true })).toHaveCount(1);
    const pairB = page.getByRole("group", { name: "Pair B", exact: true });
    await pairB.locator('input[value="owner-health-app"]').check();
    await expect(page.getByText("ASK → true", { exact: true })).toHaveCount(2);
    await pairB.locator('input[value="owner-own-app"]').check();
    await expect(page.getByText("ASK → true", { exact: true })).toHaveCount(1);
    await expect(page.getByText("ASK → false", { exact: true })).toHaveCount(1);
  });

  test("credentials start empty and still accept an explicit import", async ({ page }) => {
    await gotoAppReady(page, "showcase/verifiable-credentials/");
    await expect(page.locator("[data-vc-count]")).toHaveText("0 · 0 triples");
    await page.getByRole("tab", { name: "Paste", exact: true }).click();
    await page.getByLabel("Format", { exact: true }).selectOption("turtle");
    await page.getByPlaceholder('{ "@context": "https://www.w3.org/ns/credentials/v2", "type": ["VerifiableCredential"], … }').fill(
      '<urn:test:credential> <https://www.w3.org/2018/credentials#issuer> <urn:test:issuer> .',
    );
    await page.getByRole("button", { name: "Add pasted credential", exact: true }).click();
    await expect(page.locator("[data-vc-count]")).toHaveText("1 · 1 triples");
    await expect(page.locator("[data-vc-cred]")).toContainText("issuer: urn:test:issuer");
  });
});

test("subscription transcript finishes, replays and resets", async ({ page }) => {
  await gotoAppReady(page, "capabilities/");
  await page.getByRole("button", { name: /HTTP server/i }).click();
  const demo = page.locator('[data-demo-body="http-server"]');
  await demo.getByRole("button", { name: "Play", exact: true }).click();
  await expect(demo.getByRole("button", { name: "Replay", exact: true })).toBeEnabled();
  await demo.getByRole("button", { name: "Replay", exact: true }).click();
  await expect(demo.getByRole("button", { name: "Playing…", exact: true })).toBeDisabled();
  await demo.getByRole("button", { name: "Reset", exact: true }).click();
  await expect(demo.getByRole("button", { name: "Play", exact: true })).toBeEnabled();
  await expect(demo.getByText("Press Play to step through the captured stream.", { exact: true })).toBeVisible();
});

test.describe("external stream registration", () => {
  // Service-worker fetches bypass Playwright's module interception.
  test.use({ serviceWorkers: "block" });

  test.beforeEach(async ({ page }) => {
    // Each handle returns its configuration and its own push counter through the UI.
    // This exercises the React lifecycle without requiring the optional RSP wasm build.
    await page.route("**/wasm/rsp/sparq_rsp_wasm.js", (route) => route.fulfill({
      contentType: "text/javascript",
      body: `
        export default async function init() {
          if (window.__rspDeferInit) {
            await new Promise(resolve => { window.__rspFinishInit = resolve; });
          }
        }
        export const Rsp = { select(query, range, step) {
          if (window.__rspFailRegistration) throw new Error("fixture registration failed");
          window.__rspConfigurations ??= [];
          window.__rspConfigurations.push({ query, range, step });
          let pushes = 0;
          return {
            push: () => JSON.stringify([{
              start: 0, end: range,
              results: {
                head: { vars: ["reading", "query"] },
                results: { bindings: [{
                  reading: { type: "literal", value: range + "/" + step + ":" + (++pushes) },
                  query: { type: "literal", value: query }
                }] }
              }
            }]),
            flush: () => "[]", lateDropped: () => 0
          };
        } };
      `,
    }));
  });

  test("stream configuration changes and Reset use a fresh external handle", async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __rspDeferInit: boolean }).__rspDeferInit = true;
    });
    await gotoAppReady(page, "capabilities/");
    await page.getByRole("button", { name: /Streaming RSP/i }).click();
    const demo = page.locator('[data-demo-body="streaming-rsp"]');
    const push = demo.getByRole("button", { name: /Push next reading/ });
    await expect(demo.getByText("Registering on the wasm engine…", { exact: true })).toBeVisible();
    await expect(push).toBeDisabled();
    // Change configuration while the first registration is still in flight.
    await demo.locator("#rsp-range").press("ArrowRight");
    await expect.poll(() => page.evaluate(() => typeof (
      window as unknown as { __rspFinishInit?: () => void }
    ).__rspFinishInit)).toBe("function");
    await page.evaluate(() => (
      window as unknown as { __rspFinishInit: () => void }
    ).__rspFinishInit());
    await expect(push).toBeEnabled();
    await push.click();
    await expect(demo.getByRole("cell", { name: '"65/60:1"', exact: true })).toBeVisible();
    await demo.locator("#rsp-range").press("ArrowRight");
    await expect(demo.getByText(/0 \/ \d+ readings pushed/)).toBeVisible();
    await expect(demo.getByRole("table")).toHaveCount(0);
    await push.click();
    await expect(demo.getByRole("cell", { name: '"70/60:1"', exact: true })).toBeVisible();
    await demo.locator("#rsp-step").press("ArrowLeft");
    await expect(demo.getByText(/0 \/ \d+ readings pushed/)).toBeVisible();
    await push.click();
    await expect(demo.getByRole("cell", { name: '"70/55:1"', exact: true })).toBeVisible();
    const configurations = () => page.evaluate(() => (
      window as unknown as { __rspConfigurations: { range: number }[] }
    ).__rspConfigurations);
    const registrations = (await configurations()).length;
    await demo.getByRole("button", { name: "Reset", exact: true }).click();
    await expect(demo.getByText(/0 \/ \d+ readings pushed/)).toBeVisible();
    await expect.poll(async () => (await configurations()).length).toBe(registrations + 1);
    await push.click();
    await expect(demo.getByRole("cell", { name: '"70/55:1"', exact: true })).toBeVisible();
    await demo.getByRole("button", { name: "COUNT per window", exact: true }).click();
    await expect(demo.getByText(/0 \/ \d+ readings pushed/)).toBeVisible();
    await push.click();
    await expect(demo.getByRole("cell", { name: '"30/30:1"', exact: true })).toBeVisible();
    await expect(demo.getByRole("cell", {
      name: '"SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/reading> ?v }"', exact: true,
    })).toBeVisible();
  });

  test("a failed stream registration leaves Reset available for recovery", async ({ page }) => {
    await gotoAppReady(page, "capabilities/");
    await page.getByRole("button", { name: /Streaming RSP/i }).click();
    const demo = page.locator('[data-demo-body="streaming-rsp"]');
    const push = demo.getByRole("button", { name: /Push next reading/ });
    await expect(push).toBeEnabled();
    await page.evaluate(() => {
      (window as unknown as { __rspFailRegistration: boolean }).__rspFailRegistration = true;
    });
    await demo.locator("#rsp-range").press("ArrowRight");
    await expect(demo.getByText("fixture registration failed", { exact: true })).toBeVisible();
    await expect(push).toBeDisabled();
    const reset = demo.getByRole("button", { name: "Reset", exact: true });
    await expect(reset).toBeEnabled();
    await page.evaluate(() => {
      (window as unknown as { __rspFailRegistration: boolean }).__rspFailRegistration = false;
    });
    await reset.click();
    await expect(push).toBeEnabled();
    await expect(demo.getByText("fixture registration failed", { exact: true })).toHaveCount(0);
    await push.click();
    await expect(demo.getByRole("cell", { name: '"65/60:1"', exact: true })).toBeVisible();
  });
});
