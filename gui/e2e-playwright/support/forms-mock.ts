// [GPT-5.6] sq-3eukz — one deterministic FormDescription factory for the mocked Tauri
// `derive_form` command, echoing focus/mode/shape so re-derivation assertions are non-vacuous.
//
// [OPUS-5] sq-q4apb (#2644): the companion `wasmFormsMockScript` — a structural `deriveForm`
// installed on `Object.prototype` — is DELETED. js `build:wasm` now enables the opt-in `forms`
// cargo feature, so the served bundle defines a real `Store.prototype.deriveForm` that shadows
// any `Object.prototype` stand-in; the mock could no longer be reached, and the hosted-web
// journeys (specs/forms-tool.web.spec.ts) now drive the real derivation instead.

export const formsMockPrelude = `
  function formsMockDescription(focus, mode, selectedShape) {
    var EX = "http://example.org/";
    var DASH = "http://datashapes.org/dash#";
    var shape = selectedShape || EX + "PersonShape";
    var local = String(focus || "").split(/[\\/#]/).filter(Boolean).pop() || "resource";
    var label = local.charAt(0).toUpperCase() + local.slice(1);
    return JSON.stringify({
      focus: { kind: "iri", value: focus },
      mode: mode,
      shapes: [
        { shape: { kind: "iri", value: EX + "PersonShape" }, label: "Person", via: "target-class" },
        { shape: { kind: "iri", value: EX + "AuditableShape" }, label: "Auditable", via: "applicable-to-class" }
      ],
      shape: { kind: "iri", value: shape },
      groups: [{
        kind: "declared",
        label: "Workspace profile",
        fields: [{
          path: "<" + EX + "name>",
          label: "Workspace name",
          multi: false,
          editable: mode === "edit",
          widget: {
            editor: mode === "edit" ? DASH + "TextFieldEditor" : undefined,
            viewer: DASH + "LiteralViewer"
          },
          values: [{ term: { kind: "literal", value: label } }],
          constraints: {}
        }]
      }]
    });
  }
`;
