---
name: shacl-forms
description: "Derive a DASH-compatible, renderer-agnostic form description from SHACL shapes with the opt-in sparq-forms crate: (data graph, shapes graph, focus node, view|edit mode) -> serde-JSON FormDescription — applicable-shape switcher, property-group/field layout (sh:name/sh:description/sh:order/sh:group, sh:inversePath incoming references, sh:deactivated, implicit read-only Other group), DASH widget auto-selection via the documented 0-100 scoring registry (TextField/TextArea/BooleanSelect/Date+DateTimePicker/EnumSelect/InstancesSelect/URIEditor/RichText/SubClass/DetailsEditor and the viewers) with dash:editor/dash:viewer overrides, required/multi cardinality typing, dash:hidden/dash:readOnly/sh:defaultValue presentation flags, per-field constraints and opt-in live SHACL validation hints, and nested sh:node sub-forms. Use when an agent or GUI needs a shape-directed data-entry/edit/view form for a focus node (headless: no GUI deps, builds for wasm32)."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq-forms — SHACL/DASH form derivation

`sparq-forms` is an **opt-in** crate (depending on it is the opt-in; nothing in
the default workspace pulls it) that turns SHACL shapes into a **headless form
model**: a pure function from (data graph, shapes graph, focus node, options)
to a serde-JSON `FormDescription` a renderer (Tauri workbench, web via wasm, an
MCP agent tool) can draw without knowing any SHACL.

```rust
use sparq_core::Graph;
use sparq_forms::{derive_form, FormOptions, Mode};
use oxrdf::{NamedNode, Term};

let shapes = Graph::load_str(SHAPES_TTL, "turtle")?;   // sh:NodeShape + sh:property …
let data = Graph::load_str(DATA_TTL, "turtle")?;
let focus = Term::from(NamedNode::new("http://example.org/alice")?);

let form = derive_form(&data, &shapes, &focus, &FormOptions::default()); // edit mode
let json = serde_json::to_string_pretty(&form)?;        // ship to any renderer
```

After a renderer edits the `values` of editable fields, build the corresponding
SPARQL 1.1 Update without depending on the query engine:

```rust
use sparq_forms::{to_sparql_update, FormDiff};

let diff = FormDiff::between(&before, &after);
let update = to_sparql_update(&before, &after);
if !update.is_empty() {
    // Send `update` to a SPARQL endpoint or apply it with sparq-engine.
}
```

The descriptions must name the same focus node. Only values on editable bare
forward-predicate paths (`<p>`) participate; read-only/off-shape, inverse,
computed, and complex property-path fields are excluded. A no-change or
mismatched-focus input returns an empty update string.

What the description carries (all serde `Serialize + Deserialize`):

- **`shapes`** — the applicable-shape switcher: `sh:targetNode`,
  `sh:targetClass`/implicit class targets (focus `rdf:type` with
  `rdfs:subClassOf` closure in the data graph), `dash:applicableToClass`,
  then the `sh:targetSubjectsOf`/`sh:targetObjectsOf` predicate targets,
  ranked strongest-first (the first entry is the shape the form derives
  against); `FormOptions::shape` forces an explicit choice.
- **`groups[].fields[]`** — one field per property shape: SPARQL-path text
  (`^<p>` for `sh:inversePath` incoming references), label/description
  (`sh:name`/`sh:description`, `rdfs:label`/`rdfs:comment` fallbacks),
  fractional `sh:order`, `sh:group` → `sh:PropertyGroup` sections,
  `sh:deactivated` suppressed, plus an implicit trailing **"Other properties"**
  group holding off-shape data triples **read-only**.
- **`widget`** — DASH widget auto-selection from the documented scoring table
  (`sparq_forms::widgets` rustdoc; deviations from stock DASH are marked
  *(sparq)* there): explicit `dash:editor`/`dash:viewer` win, ties break
  deterministically, runner-ups land in `*_alternatives`. `Mode::View`
  resolves viewers only.
- **`required` / `multi`** — `sh:minCount >= 1` / `sh:maxCount != 1` (the
  add/remove affordance signal).
- **`constraints`** — per-field counts/datatypes/classes/nodeKind/`sh:in`/
  pattern/length/range/`sh:or` for renderer-side guidance.
- **`hidden` / `editable` / `default_value`** — `dash:hidden true` flags a
  field the renderer should omit (it still derives, with values and
  constraints), `dash:readOnly true` forces `editable: false` even in edit
  mode, and `sh:defaultValue` is carried verbatim as the seed term a renderer
  pre-fills when a field has no values. All additive: omitted from the JSON
  when the property shape does not declare them.
- **`validation`** — with `derive_form_validated(data, shapes, model, focus,
  opts, registry)`, SHACL results for the focus node are attached to the
  matching declared, editable field by property path. Each `ValidationHint`
  carries the source-component IRI, selected shape message (or generated
  fallback), optional offending value, and severity. Plain `derive_form` and
  `derive_form_with_model` leave this vector empty.
- **`values[].nested`** — `sh:node` values recurse into nested sub-forms
  (`dash:DetailsEditor`), `max_depth`-limited and cycle-safe.

Amortise shape parsing across focus nodes with `derive_form_with_model`
(`ShapesModel::parse` once + a `WidgetRegistry` — extend it with
`register_editor`/`register_viewer` for custom widgets), and get just the
switcher with `applicable_shapes`. Blank-node labels in the output are
renamed deterministically (`b0`, `b1`, …) — do not treat them as graph handles.

Scope: derivation, opt-in live validation hints, plus pure edit-to-UPDATE
building. An MCP agent can call the derivation as the `describe_form` tool
(sparq-mcp feature `shacl`, `FormDescription` JSON verbatim — see
[`agent-tools`](../agent-tools/SKILL.md)). [FABLE-5] sq-lsp7k.1.6. Applying the
request, validate-before-commit guards, draft graphs, DASH suggestions,
`dash:propertyRole`, and the GUI renderer are follow-on beads
(sq-lsp7k.1.2/.1.4/.1.5); `sparq-shacl` (see
[`shacl-validation`](../shacl-validation/SKILL.md)) already validates the same
graphs.

_(status: Verified against sparq-forms 0.1.0 [OPUS-4.8] (sq-vfcxv, 2026-07-27):
56 unit/integration tests incl. per-score widget tests + 4 golden-file fixtures
(groups/order, enum + nested sh:node, inverse + multi-shape, predicate
targets). Caveats: (1) widget scores follow datashapes.org/forms
with documented (sparq) auto-selection extensions where DASH is manual-only —
InstancesSelect on sh:class, SubClass on dash:rootClass, Details on sh:node.
(2) SHACL 1.2 sh:targetWhere and SPARQL-valued targets do not drive form
applicability (sh:targetSubjectsOf/ObjectsOf now do, ranked below
dash:applicableToClass — sq-vfcxv). (3) `values` for SPARQL-computed sh:values
shapes are not evaluated (F5). (4) rdf:langString base direction untouched.)_
