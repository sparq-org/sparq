// [FABLE-5] sq-lsp7k.1.1 (forms F1): single-source the crate overview from README.md
// so crates.io and the docs.rs front page render identical content; the README's
// `## Usage` rust fence compiles as a doctest.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod derive;
pub mod description;
pub mod diff;
pub mod widgets;

pub use description::{
    Constraints, FormDescription, FormField, FormGroup, FormValue, GroupKind, Mode, ShapeChoice,
    ShapeVia, TermRef, ValidationHint, WidgetChoice,
};
pub use diff::{to_sparql_update, FieldValueDiff, FormDiff};
pub use widgets::{Resolution, Scorer, Suitability, WidgetContext, WidgetRegistry, DASH};

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use sparq_shacl::ShapesModel;

/// Options for a form derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormOptions {
    /// View (read-only) or edit (the default) — see [`Mode`].
    pub mode: Mode,
    /// Opaque role echoed into the description (`dash:propertyRole` filtering
    /// lands with F5, sq-lsp7k.1.5).
    pub role: Option<String>,
    /// Explicit node-shape choice: derive against THIS shape (it need not
    /// target the focus node) instead of the first applicable one.
    pub shape: Option<Term>,
    /// Maximum `sh:node` sub-form nesting depth (default 3); deeper values
    /// render without a nested form (the renderer can derive on demand).
    pub max_depth: usize,
}

impl Default for FormOptions {
    fn default() -> Self {
        FormOptions {
            mode: Mode::Edit,
            role: None,
            shape: None,
            max_depth: 3,
        }
    }
}

/// Derives the [`FormDescription`] for `focus` from a data graph and a SHACL
/// shapes graph, using the default DASH widget registry.
///
/// Parses the shapes graph on every call — use [`derive_form_with_model`] to
/// amortise shape parsing (and/or supply a custom [`WidgetRegistry`]) across
/// many focus nodes.
pub fn derive_form(
    data: &Graph,
    shapes: &Graph,
    focus: &Term,
    opts: &FormOptions,
) -> FormDescription {
    let model = ShapesModel::parse(shapes);
    derive_form_with_model(data, shapes, &model, focus, opts, &WidgetRegistry::dash())
}

/// [`derive_form`] against an already-parsed [`ShapesModel`] and an explicit
/// [`WidgetRegistry`]. `shapes` must be the graph `model` was parsed from: the
/// model carries the constraint components while the graph supplies the
/// presentation vocabulary (`sh:name`/`sh:description`/`sh:order`/`sh:group`,
/// `dash:editor`/`dash:viewer`/`dash:singleLine`/`dash:applicableToClass`).
pub fn derive_form_with_model(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    focus: &Term,
    opts: &FormOptions,
    registry: &WidgetRegistry,
) -> FormDescription {
    derive::derive(
        &GraphView::new(data),
        &GraphView::new(shapes),
        model,
        focus,
        opts,
        registry,
    )
}

/// [GPT-5.6] Derives a form and attaches live SHACL results for `focus` to
/// matching declared, editable fields without mutating `data`.
///
/// Validation is additive and opt-in: [`derive_form`] and
/// [`derive_form_with_model`] continue to return fields with empty validation
/// vectors. Property-level results are matched using the same rendered SHACL
/// path stored in [`FormField::path`]; node-level and unmatched results are not
/// attached.
pub fn derive_form_validated(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    focus: &Term,
    opts: &FormOptions,
    registry: &WidgetRegistry,
) -> FormDescription {
    let mut form = derive_form_with_model(data, shapes, model, focus, opts, registry);
    let report = sparq_shacl::validate_with_model(data, model);

    for result in report.results {
        if result.focus_node != *focus {
            continue;
        }
        let Some(path) = result.path.as_ref() else {
            continue;
        };
        let rendered_path = derive::render_path(path);
        let Some(field) = form
            .groups
            .iter_mut()
            .flat_map(|group| &mut group.fields)
            .find(|field| {
                field.editable
                    && field.property_shape.is_some()
                    && field.path == rendered_path
            })
        else {
            continue;
        };
        let message =
            derive::pick_literal(result.messages).unwrap_or(result.default_message);
        field.validation.push(ValidationHint {
            source_component: result.source_component,
            message,
            value: result.value.as_ref().map(TermRef::from_term),
            severity: result.severity,
        });
    }

    form
}

/// The node shapes applicable to `focus` — the renderer's shape switcher,
/// strongest rationale first: `sh:targetNode`, `sh:targetClass` / implicit
/// class targets (matched against the focus node's `rdf:type`s with
/// `rdfs:subClassOf` closure in the data graph), `dash:applicableToClass`,
/// then the predicate targets `sh:targetSubjectsOf` / `sh:targetObjectsOf`
/// (the focus node is a subject, resp. object, of the named predicate).
///
/// SHACL 1.2 `sh:targetWhere` and SPARQL-valued targets still do NOT drive
/// applicability: both need evaluation machinery (single-node conformance
/// against an inline shape; running a node expression) that `sparq-shacl` keeps
/// crate-private. [OPUS-4.8] sq-vfcxv
pub fn applicable_shapes(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    focus: &Term,
) -> Vec<ShapeChoice> {
    derive::applicable(&GraphView::new(data), &GraphView::new(shapes), model, focus)
}
