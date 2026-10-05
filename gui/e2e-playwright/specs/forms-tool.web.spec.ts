// [OPUS-5] sq-q4apb (#2644) — hosted-web Forms journeys over the REAL in-tab wasm bridge.
//
// Until #2644 the js `build:wasm` bundle this lane serves was built WITHOUT the opt-in `forms`
// cargo feature, so `Store.deriveForm` was genuinely absent and these journeys asserted the
// explicit-absence state plus a structurally mocked method installed on `Object.prototype`.
// `build:wasm` now enables `forms`, so the served bundle carries a real `Store.deriveForm`: the
// wasm-bindgen `Store.prototype` method SHADOWS any `Object.prototype` stand-in, and the absence
// branch is unreachable in this bundle. Both journeys therefore run against the real derivation —
// a strictly stronger assertion than the mock they replace.
//
// The absent-bridge routing contract (`FormsBridgeUnavailableError`, never demo data) and the
// exact positional `deriveForm(data, shapes, focus, format, optionsJson)` argument contract keep
// their dedicated coverage in gui/app/src/lib/forms-bridge.test.ts, which drives the adapter
// directly and does not depend on which cargo features the bundle was built with.
//
// The seeded workspace (gui/app/src/data/sample-graph.ts) carries no SHACL shapes, so the real
// derivation returns the read-only implicit "Other properties" group over the focus node's own
// triples — no declared group and no shape switcher. That is the honest hosted-web behaviour for
// this workspace.
import { webTest as test, webExpect as expect } from "../support/index.ts";

const EX = "http://example.org/";

test("the served wasm bundle derives the active workspace form in-tab, never demo data", async ({
  page,
}) => {
  await page.locator('[data-tool="forms"]').click();
  const panel = page.locator('[data-tool-panel="forms"]');

  // The real bundle exposes Store.deriveForm, so the web host derives instead of reporting the
  // unavailable state.
  await expect(panel.locator("[data-form-derive-source]")).toContainText("WASM");
  await expect(panel.locator("[data-form-unavailable]")).toHaveCount(0);
  await expect(panel.locator("[data-form-derive-error]")).toHaveCount(0);
  await expect(panel.locator("[data-form-focus]")).toContainText(`${EX}alice`);

  // Off-shape workspace triples land in the implicit read-only "Other properties" group (rendered
  // collapsed), holding the values the live store carries — content the browser never rebuilds.
  const other = panel.locator('[data-form-group="other"]');
  await other.evaluate((details: HTMLDetailsElement) => {
    details.open = true;
  });
  await expect(other.locator('[data-form-field="name"]')).toContainText("Alice");
  await expect(other.locator('[data-form-field="city"]')).toContainText("London");

  // Manual JSON stays available alongside the derived form, and demo data is never rendered.
  await panel.locator("[data-form-json-source]").evaluate((details: HTMLDetailsElement) => {
    details.open = true;
  });
  await expect(panel.getByRole("textbox", { name: "FormDescription JSON" })).toHaveValue("");
  await expect(panel.getByText("order42", { exact: false })).toHaveCount(0);
});

test("focus and mode changes re-derive the whole description from the wasm engine", async ({
  page,
}) => {
  await page.locator('[data-tool="forms"]').click();
  const panel = page.locator('[data-tool-panel="forms"]');
  await expect(panel.locator("[data-form-derive-source]")).toContainText("WASM");

  await panel
    .locator("[data-form-focus-picker]")
    .selectOption(JSON.stringify(["iri", `${EX}carol`, null, null]));
  await expect(panel.locator("[data-form-focus]")).toContainText(`${EX}carol`);

  // A new focus replaces the whole description: the values are Carol's, from the live store.
  const other = panel.locator('[data-form-group="other"]');
  await other.evaluate((details: HTMLDetailsElement) => {
    details.open = true;
  });
  await expect(other.locator('[data-form-field="name"]')).toContainText("Carol");
  await expect(other.locator('[data-form-field="name"]')).not.toContainText("Alice");

  await panel.locator('[data-form-mode-choice="view"]').click();
  await expect(panel.locator("[data-form-renderer]")).toHaveAttribute("data-form-mode", "view");
  await expect(panel.locator("[data-form-derive-error]")).toHaveCount(0);
});
