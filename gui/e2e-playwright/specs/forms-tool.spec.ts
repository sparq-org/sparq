// [GPT-5.6] sq-3eukz — desktop Forms journey over the active workspace + mocked Tauri adapter.
import { test, expect, readIpcLog } from "../support/index.ts";
import { readFile } from "node:fs/promises";

const EX = "http://example.org/";

test.describe("forms-tool (desktop persona)", () => {
  test.beforeEach(async ({ page }) => {
    await page.locator('[data-tool="forms"]').click();
    await expect(page.locator('[data-tool-panel="forms"]')).toBeVisible();
    await expect(page.locator('[data-form-field="Workspace name"]')).toBeVisible();
  });

  test("focus, mode, and shape changes re-invoke Tauri with the complete workspace request", async ({
    page,
  }) => {
    const panel = page.locator('[data-tool-panel="forms"]');
    const picker = panel.locator("[data-form-focus-picker]");
    await expect(picker.locator("option")).toHaveCount(5);

    await picker.selectOption(JSON.stringify(["iri", `${EX}bob`, null, null]));
    await expect(panel.locator("[data-form-focus]")).toContainText(`${EX}bob`);
    await expect(panel.getByRole("textbox", { name: "Workspace name" })).toHaveValue("Bob");

    let calls = (await readIpcLog(page)).filter((entry) => entry.cmd === "derive_form");
    let args = calls.at(-1)?.args as Record<string, unknown>;
    expect(args.focus).toBe(`${EX}bob`);
    expect(args.dataset).toBe(args.shapes);
    expect(args.format).toBe("nquads");
    expect(args.mode).toBe("edit");
    expect(args.shape).toBeUndefined();

    await panel.locator('[data-form-mode-choice="view"]').click();
    await expect(panel.locator("[data-form-renderer]")).toHaveAttribute("data-form-mode", "view");
    calls = (await readIpcLog(page)).filter((entry) => entry.cmd === "derive_form");
    args = calls.at(-1)?.args as Record<string, unknown>;
    expect(args.focus).toBe(`${EX}bob`);
    expect(args.mode).toBe("view");
    expect(args.shape).toBe(`${EX}PersonShape`);

    await panel.locator("[data-form-shape-switcher]").selectOption({ label: "Auditable" });
    await expect
      .poll(async () => {
        const log = (await readIpcLog(page)).filter((entry) => entry.cmd === "derive_form");
        return (log.at(-1)?.args as Record<string, unknown>)?.shape;
      })
      .toBe(`${EX}AuditableShape`);
  });

  test("desktop host fixture renders the shared FormDescription content", async ({ page }) => {
    const panel = page.locator('[data-tool-panel="forms"]');
    await expect(panel.locator('[data-form-derive-source]')).toContainText("Tauri");
    await expect(panel.locator('[data-form-group="declared"]')).toContainText("Workspace profile");
    await expect(panel.getByRole("textbox", { name: "Workspace name" })).toHaveValue("Alice");
  });

  test("manual JSON remains available and malformed input does not replace the host form", async ({
    page,
  }) => {
    const panel = page.locator('[data-tool-panel="forms"]');
    await panel.locator("[data-form-json-source]").evaluate((details: HTMLDetailsElement) => {
      details.open = true;
    });
    await panel
      .getByRole("textbox", { name: "FormDescription JSON" })
      .fill('{"mode":"edit","groups":[],"shapes":[]}');
    await panel.locator("[data-load-form-json]").click();
    await expect(panel.getByRole("alert")).toContainText("focus");
    await expect(panel.locator("[data-form-focus]")).toContainText(`${EX}alice`);
  });

  test("manual form edits retain focus and widget choice until the mode changes", async ({ page }) => {
    const panel = page.locator('[data-tool-panel="forms"]');
    const json = await readFile(new URL(
      "../../../crates/sparq-forms/tests/fixtures/person_groups.golden.json",
      import.meta.url,
    ), "utf8");
    await panel.locator("[data-form-json-source]").evaluate((details: HTMLDetailsElement) => {
      details.open = true;
    });
    await panel.getByRole("textbox", { name: "FormDescription JSON" }).fill(json);
    await panel.locator("[data-load-form-json]").click();

    const field = panel.locator('[data-form-field="Full name"]');
    await expect(field).toBeVisible();
    await expect(panel.getByRole("status").filter({ hasText: "Deriving the complete form" })).toBeHidden();
    const widget = field.locator("[data-widget-switcher]");
    await widget.selectOption({ label: "TextAreaEditor" });
    const input = field.getByRole("textbox", { name: "Full name", exact: true });
    await input.fill("Edited name");
    await expect(input).toBeFocused();
    await expect(field.locator('[data-field-editor="textarea"]')).toBeVisible();

    await panel.locator('[data-form-mode-choice="view"]').click();
    await expect(field).toContainText("Edited name");
    await panel.locator('[data-form-mode-choice="edit"]').click();
    await expect(field.locator('[data-field-editor="text"]')).toBeVisible();
    await expect(input).toHaveValue("Edited name");
  });
});
