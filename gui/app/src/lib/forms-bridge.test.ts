// [GPT-5.6] sq-3eukz — focused host-routing tests for workspace-derived Forms descriptions.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  deriveFormWith,
  FormsBridgeUnavailableError,
  formOptionsJson,
  formWorkspaceSnapshot,
  type DeriveFormRequest,
  type FormHostAdapters,
} from "./forms-bridge.js";

const DATASET =
  '<http://example.org/alice> <http://example.org/name> "Alice" .';
const FOCUS = { kind: "iri", value: "http://example.org/alice" } as const;
const SHAPE = { kind: "iri", value: "http://example.org/PersonShape" } as const;
const REQUEST: DeriveFormRequest = {
  data: DATASET,
  shapes: DATASET,
  focus: FOCUS,
  mode: "edit",
  format: "nquads",
};

test("Forms skips the workspace snapshot without a selection or before readiness", () => {
  let calls = 0;
  const snapshotStore = () => { calls += 1; return DATASET; };
  assert.deepEqual(formWorkspaceSnapshot(true, null, snapshotStore), {
    snapshot: null, error: null,
  });
  assert.equal(calls, 0);
  assert.deepEqual(formWorkspaceSnapshot(false, FOCUS, snapshotStore), {
    snapshot: null, error: null,
  });
  assert.equal(calls, 0);
  assert.deepEqual(formWorkspaceSnapshot(true, FOCUS, snapshotStore), {
    snapshot: DATASET, error: null,
  });
  assert.equal(calls, 1);
});

test("Forms captures snapshot errors for a selected focus", () => {
  assert.deepEqual(formWorkspaceSnapshot(true, FOCUS, () => { throw new Error("store failed"); }), {
    snapshot: null, error: "store failed",
  });
  assert.deepEqual(formWorkspaceSnapshot(true, FOCUS, () => { throw "store failed"; }), {
    snapshot: null, error: "store failed",
  });
  assert.deepEqual(formWorkspaceSnapshot(true, FOCUS, () => null), {
    snapshot: null, error: null,
  });
});

function response(mode = "edit", shape = SHAPE.value): string {
  return JSON.stringify({
    focus: FOCUS,
    mode,
    shapes: [{ shape: SHAPE, label: "Person", via: "target-class" }],
    shape: { kind: "iri", value: shape },
    groups: [
      {
        kind: "declared",
        label: "Profile",
        fields: [
          {
            path: "<http://example.org/name>",
            label: "Workspace name",
            multi: false,
            editable: mode === "edit",
            widget: { editor: "http://datashapes.org/dash#TextFieldEditor" },
            values: [{ term: { kind: "literal", value: "Alice" } }],
            constraints: {},
          },
        ],
      },
    ],
  });
}

test("formOptionsJson emits the host's snake_case mode/shape options", () => {
  assert.equal(formOptionsJson(REQUEST), '{"mode":"edit"}');
  assert.equal(
    formOptionsJson({ ...REQUEST, mode: "view", shape: SHAPE }),
    '{"mode":"view","shape":"http://example.org/PersonShape"}',
  );
});

test("desktop focus/mode/shape request invokes only Tauri with the expected workspace args", async () => {
  let desktopArgs: Parameters<FormHostAdapters["desktopDerive"]>[0] | undefined;
  let webCalls = 0;
  const result = await deriveFormWith(
    { ...REQUEST, mode: "view", shape: SHAPE },
    {
      desktopAvailable: true,
      desktopDerive: async (args) => {
        desktopArgs = args;
        return response("view");
      },
      webDerive: () => {
        webCalls += 1;
        return response();
      },
    },
  );

  assert.deepEqual(desktopArgs, {
    dataset: DATASET,
    shapes: DATASET,
    format: "nquads",
    focus: FOCUS.value,
    mode: "view",
    shape: SHAPE.value,
  });
  assert.equal(webCalls, 0);
  assert.equal(result.source, "desktop");
  assert.equal(result.description.groups[0]?.fields[0]?.label, "Workspace name");
});

test("desktop and web adapter fixtures parse to equivalent FormDescription content", async () => {
  const json = response();
  const common = {
    desktopDerive: async () => json,
    webDerive: () => json,
  };
  const desktop = await deriveFormWith(REQUEST, { ...common, desktopAvailable: true });
  const web = await deriveFormWith(REQUEST, { ...common, desktopAvailable: false });

  assert.deepEqual(desktop.description, web.description);
  assert.equal(web.description.focus.value, FOCUS.value);
  assert.equal(web.description.groups[0]?.fields[0]?.values[0]?.term.value, "Alice");
});

test("web mode/shape request invokes deriveForm with data, shapes, focus, format, options", async () => {
  let webArgs: unknown[] | undefined;
  await deriveFormWith(
    { ...REQUEST, mode: "view", shape: SHAPE },
    {
      desktopAvailable: false,
      desktopDerive: async () => null,
      webDerive: (...args) => {
        webArgs = args;
        return response("view");
      },
    },
  );

  assert.deepEqual(webArgs, [
    DATASET,
    DATASET,
    FOCUS.value,
    "nquads",
    '{"mode":"view","shape":"http://example.org/PersonShape"}',
  ]);
});

test("an absent web bridge is explicit and never supplies demo content", async () => {
  await assert.rejects(
    () =>
      deriveFormWith(REQUEST, {
        desktopAvailable: false,
        desktopDerive: async () => null,
        webDerive: () => null,
      }),
    FormsBridgeUnavailableError,
  );
});
