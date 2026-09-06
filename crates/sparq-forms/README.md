# sparq-forms

Headless **SHACL-to-form derivation** for sparq: a pure function from
(data graph, shapes graph, focus node, mode) to a serde-JSON **`FormDescription`**
— DASH-compatible auto-generated data-entry/edit forms, with **no GUI
dependencies** (wasm-able by construction).

> Model: Claude Fable 5 [FABLE-5] (sq-lsp7k.1.1). Design record
> `research/competitive-feature-analysis-2026-07.md` §3.

## 🚀 Quickstart

```rust
use sparq_core::Graph;
use sparq_forms::{derive_form, FormOptions};
use oxrdf::{NamedNode, Term};

let shapes = Graph::load_str(r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
  @prefix ex: <http://example.org/> .
  ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:name "Name" ; sh:order 1 ;
                  sh:datatype xsd:string ; sh:minCount 1 ; sh:maxCount 1 ] .
"#, "turtle").unwrap();
let data = Graph::load_str(r#"
  @prefix ex: <http://example.org/> .
  ex:alice a ex:Person ; ex:name "Alice" .
"#, "turtle").unwrap();

let focus = Term::from(NamedNode::new("http://example.org/alice").unwrap());
let form = derive_form(&data, &shapes, &focus, &FormOptions::default());
let field = &form.groups[0].fields[0];
assert_eq!(field.label, "Name");
assert!(field.required && !field.multi);
assert_eq!(field.widget.editor.as_deref(),
           Some("http://datashapes.org/dash#TextFieldEditor"));
```

## ✨ Features

- **Shape selection** — `sh:targetNode`, `sh:targetClass` / implicit class
  targets (matched against the focus node's `rdf:type` with `rdfs:subClassOf`
  closure), `dash:applicableToClass`, the `sh:targetSubjectsOf` /
  `sh:targetObjectsOf` predicate targets (ranked last), plus an explicit
  override; every applicable shape lands in the description's shape-switcher
  list, strongest rationale first.
- **Field enumeration & layout** — one field per property shape: `sh:path`
  (including `sh:inversePath` incoming-reference fields), `sh:name` /
  `sh:description` with `rdfs:label`/`rdfs:comment` fallbacks, fractional
  `sh:order`, `sh:group` → `sh:PropertyGroup` sections, `sh:deactivated`
  suppression, and an implicit read-only **"Other properties"** group for
  off-shape triples.
- **DASH widget scoring registry** — the documented 0–100 suitability table
  (`sparq_forms::widgets` rustdoc) over the DASH editors/viewers (TextField /
  TextFieldWithLang / TextArea / BooleanSelect / Date+DateTimePicker /
  EnumSelect on `sh:in` / AutoComplete + InstancesSelect on `sh:class` /
  URIEditor / RichText on `rdf:HTML` / SubClassEditor / nested DetailsEditor /
  BlankNode fallback; Label/Image/Hyperlink/HTML/ValueTable viewers), with
  explicit `dash:editor` / `dash:viewer` overrides and runner-up alternatives.
- **Nested sub-forms** — `sh:node` values recurse into a nested
  `FormDescription` (`dash:DetailsEditor`), depth-limited and cycle-safe.
- **Constraints carried per field** — counts, datatypes, classes, node kinds,
  `sh:in` enumerations, pattern/length/range bounds, `sh:or` unions — the
  widget-scoring inputs.
- **Presentation flags** — `dash:hidden` (field derives but a renderer omits
  it), `dash:readOnly` (forces `editable: false` even in edit mode), and
  `sh:defaultValue` (verbatim seed term to pre-fill an empty field); all three
  are additive JSON keys, omitted when the property shape does not declare them.
- **Opt-in live validation** — `derive_form_validated(data, shapes, model,
  focus, opts, registry)` runs the base SHACL validator and adds each
  focus-node property violation to the matching editable field's `validation`
  vector without changing the data graph or the plain derivation APIs.
- **Pure edit diff** — `FormDiff::between(&before, &after)` reports added and
  removed RDF terms, while `to_sparql_update` renders them as one SPARQL 1.1
  `DELETE`/`INSERT` request. It intentionally excludes read-only, inverse,
  computed, and non-bare-property-path fields.
- **Headless & opt-in** — consumes `sparq-shacl`'s shapes model; no GUI deps;
  builds for `wasm32-unknown-unknown`; nothing in the default workspace
  depends on it, so the engine core stays lean.

## 📚 Learn more

- [DASH form generation](https://datashapes.org/forms.html) — the widget
  vocabulary and scoring contract this crate implements (documented
  deviations are marked *(sparq)* in the `widgets` module docs).
- `sparq-shacl` — the shapes model + validation engine this crate consumes.
- Roadmap: F2 GUI renderer (sq-lsp7k.1.2), DASH suggestions (sq-lsp7k.1.4),
  F5 RDF 1.2 / computed fields (sq-lsp7k.1.5),
  F6 sparq-mcp agent tools (sq-lsp7k.1.6).

## License

MIT — see the workspace root `LICENSE`.
