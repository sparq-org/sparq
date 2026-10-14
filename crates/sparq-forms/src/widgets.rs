//! The DASH widget-scoring registry. sq-lsp7k.1.1
//!
//! Widget auto-selection follows the DASH form-generation contract
//! (`datashapes.org/forms`): each registered widget scores its suitability for
//! a (property constraints, value) pair, and the best-scoring widget wins.
//! [`Suitability`] is the tri-state DASH defines: a numeric score `0..=100`
//! where **0 means unusable**, or **manual** (DASH's `null`) meaning "usable,
//! but only when a human/agent picks it explicitly".
//!
//! # The documented scoring table
//!
//! Scores marked *(sparq)* are documented sparq extensions: DASH leaves those
//! widgets manual-only (`null`), but a **headless** deriver must auto-select
//! deterministically, so the strong shape signals (`sh:class`, `sh:node`,
//! `dash:rootClass`) promote their natural widget. An explicit `dash:editor` /
//! `dash:viewer` statement always overrides the whole table.
//!
//! | Editor | Score |
//! |---|---|
//! | `dash:TextFieldEditor` | 10 — literal value (or permitted datatype) that is neither `rdf:langString` nor `xsd:boolean`; 2 *(sparq)* — empty untyped field with no IRI/enum/nested signal (generic fallback) |
//! | `dash:TextFieldWithLangEditor` | 11 — `rdf:langString` value, or both `rdf:langString` and `xsd:string` permitted; 5 — `rdf:langString` permitted and not `dash:singleLine false` |
//! | `dash:TextAreaEditor` | 0 — `dash:singleLine true`; 20 — `xsd:string` (value or permitted) with `dash:singleLine false`; 5 — `xsd:string` value; 2 — `xsd:string` permitted; manual — custom datatype |
//! | `dash:TextAreaWithLangEditor` | 0 — `dash:singleLine true`; 15 — `rdf:langString` value with `dash:singleLine false`; 5 — `rdf:langString` value or permitted |
//! | `dash:BooleanSelectEditor` | 10 — `xsd:boolean` value or `sh:datatype xsd:boolean`; manual — literal-shaped field without a datatype |
//! | `dash:DatePickerEditor` | 10 — `xsd:date` value; 5 — `sh:datatype xsd:date` |
//! | `dash:DateTimePickerEditor` | 10 — `xsd:dateTime` value; 5 — `sh:datatype xsd:dateTime` |
//! | `dash:EnumSelectEditor` | 10 — `sh:in` present |
//! | `dash:AutoCompleteEditor` | 1 — IRI value, or empty field shaped IRI (`sh:class` / `sh:nodeKind sh:IRI`) |
//! | `dash:InstancesSelectEditor` | 12 *(sparq)* — `sh:class` present (DASH: manual) |
//! | `dash:URIEditor` | 10 — IRI value or empty field, with `sh:nodeKind sh:IRI` and no `sh:class`; manual — other IRI values |
//! | `dash:RichTextEditor` | 10 — `rdf:HTML` value or `sh:datatype rdf:HTML` |
//! | `dash:SubClassEditor` | 15 *(sparq)* — `dash:rootClass` / `sh:rootClass` present (DASH: manual); manual — IRI-shaped otherwise |
//! | `dash:DetailsEditor` | 15 *(sparq)* — `sh:node` present and value not a literal (nested sub-form; DASH: manual); manual — non-literal values; 0 — literals |
//! | `dash:BlankNodeEditor` | 1 — blank-node value |
//!
//! | Viewer | Score |
//! |---|---|
//! | `dash:LabelViewer` | 5 — IRI value |
//! | `dash:LiteralViewer` | 1 — literal value |
//! | `dash:ImageViewer` | 50 — IRI or literal ending `.png`/`.jpg`/`.jpeg`/`.gif`/`.svg` (case-insensitive) |
//! | `dash:HyperlinkViewer` | 50 — `xsd:anyURI` literal; manual — `xsd:string` literal |
//! | `dash:HTMLViewer` | 50 — `rdf:HTML` literal |
//! | `dash:ValueTableViewer` | manual — always |
//! | `dash:DetailsViewer` | 15 *(sparq)* — `sh:node` present and value not a literal (DASH: manual); manual — IRIs and blank nodes; 0 — literals |
//! | `dash:URIViewer` | 1 — IRI value |
//! | `dash:LangStringViewer` | 10 — `rdf:langString` literal |
//! | `dash:BlankNodeViewer` | 1 — blank-node value |
//!
//! Ties break on the lexicographically smaller widget IRI so derivation is
//! deterministic; every scored runner-up (then the manual-only widgets) lands
//! in the alternatives list for the renderer's widget switcher.

use crate::description::Constraints;
use oxrdf::Term;

/// The DASH vocabulary namespace.
pub const DASH: &str = "http://datashapes.org/dash#";

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_ANYURI: &str = "http://www.w3.org/2001/XMLSchema#anyURI";
const RDF_LANGSTRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const RDF_HTML: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#HTML";
const SH_IRI: &str = "http://www.w3.org/ns/shacl#IRI";
const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// A widget's suitability for a (constraints, value) pair — DASH's score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suitability {
    /// Auto-selectable, higher wins (DASH scores are 0–100; 0 is
    /// [`Suitability::Unsuitable`], so use 1–100 here).
    Score(u8),
    /// DASH `null`: usable, but only via an explicit `dash:editor` /
    /// `dash:viewer` statement or the renderer's widget switcher.
    Manual,
    /// DASH `0`: must not be offered.
    Unsuitable,
}

/// What a widget scorer sees: the field's constraints and a sample value
/// (`None` for an empty field).
#[derive(Debug, Clone, Copy)]
pub struct WidgetContext<'a> {
    /// The field's first value, when it has one.
    pub value: Option<&'a Term>,
    /// The field's derived constraints (datatype/class/nodeKind/in/…).
    pub constraints: &'a Constraints,
}

/// A scoring function: (context) → suitability.
pub type Scorer = fn(&WidgetContext) -> Suitability;

/// The outcome of resolving one widget slot (editor or viewer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The best-scoring widget IRI (`None` when nothing scored above 0).
    pub selected: Option<String>,
    /// The winning score.
    pub score: Option<u8>,
    /// Runner-up widgets: scored ones first (best first), then manual-only
    /// ones; ties break on the IRI.
    pub alternatives: Vec<String>,
}

/// The widget registry: DASH's editors/viewers plus any registered custom
/// widgets. Construct with [`WidgetRegistry::dash`]; extend with
/// [`WidgetRegistry::register_editor`] / [`WidgetRegistry::register_viewer`].
pub struct WidgetRegistry {
    editors: Vec<(String, Scorer)>,
    viewers: Vec<(String, Scorer)>,
}

impl WidgetRegistry {
    /// The default registry: the documented DASH scoring table (module docs).
    pub fn dash() -> WidgetRegistry {
        let e = |n: &str| format!("{DASH}{n}");
        WidgetRegistry {
            editors: vec![
                (e("TextFieldEditor"), text_field as Scorer),
                (e("TextFieldWithLangEditor"), text_field_with_lang),
                (e("TextAreaEditor"), text_area),
                (e("TextAreaWithLangEditor"), text_area_with_lang),
                (e("BooleanSelectEditor"), boolean_select),
                (e("DatePickerEditor"), date_picker),
                (e("DateTimePickerEditor"), date_time_picker),
                (e("EnumSelectEditor"), enum_select),
                (e("AutoCompleteEditor"), auto_complete),
                (e("InstancesSelectEditor"), instances_select),
                (e("URIEditor"), uri_editor),
                (e("RichTextEditor"), rich_text),
                (e("SubClassEditor"), sub_class),
                (e("DetailsEditor"), details_editor),
                (e("BlankNodeEditor"), blank_node_editor),
            ],
            viewers: vec![
                (e("LabelViewer"), label_viewer as Scorer),
                (e("LiteralViewer"), literal_viewer),
                (e("ImageViewer"), image_viewer),
                (e("HyperlinkViewer"), hyperlink_viewer),
                (e("HTMLViewer"), html_viewer),
                (e("ValueTableViewer"), value_table_viewer),
                (e("DetailsViewer"), details_viewer),
                (e("URIViewer"), uri_viewer),
                (e("LangStringViewer"), lang_string_viewer),
                (e("BlankNodeViewer"), blank_node_viewer),
            ],
        }
    }

    /// Registers (or replaces, by IRI) a custom editor widget.
    pub fn register_editor(&mut self, iri: &str, scorer: Scorer) {
        register(&mut self.editors, iri, scorer);
    }

    /// Registers (or replaces, by IRI) a custom viewer widget.
    pub fn register_viewer(&mut self, iri: &str, scorer: Scorer) {
        register(&mut self.viewers, iri, scorer);
    }

    /// Scores one editor widget by IRI (`None` when not registered).
    pub fn score_editor(&self, iri: &str, ctx: &WidgetContext) -> Option<Suitability> {
        self.editors.iter().find(|(i, _)| i == iri).map(|(_, s)| s(ctx))
    }

    /// Scores one viewer widget by IRI (`None` when not registered).
    pub fn score_viewer(&self, iri: &str, ctx: &WidgetContext) -> Option<Suitability> {
        self.viewers.iter().find(|(i, _)| i == iri).map(|(_, s)| s(ctx))
    }

    /// Resolves the editor slot: best score wins, ties break on the IRI.
    pub fn resolve_editor(&self, ctx: &WidgetContext) -> Resolution {
        resolve(&self.editors, ctx)
    }

    /// Resolves the viewer slot: best score wins, ties break on the IRI.
    pub fn resolve_viewer(&self, ctx: &WidgetContext) -> Resolution {
        resolve(&self.viewers, ctx)
    }
}

impl Default for WidgetRegistry {
    fn default() -> Self {
        WidgetRegistry::dash()
    }
}

fn register(slot: &mut Vec<(String, Scorer)>, iri: &str, scorer: Scorer) {
    if let Some(entry) = slot.iter_mut().find(|(i, _)| i == iri) {
        entry.1 = scorer;
    } else {
        slot.push((iri.to_string(), scorer));
    }
}

fn resolve(widgets: &[(String, Scorer)], ctx: &WidgetContext) -> Resolution {
    let mut scored: Vec<(u8, &str)> = Vec::new();
    let mut manual: Vec<&str> = Vec::new();
    for (iri, scorer) in widgets {
        match scorer(ctx) {
            Suitability::Score(s) if s > 0 => scored.push((s, iri)),
            Suitability::Manual => manual.push(iri),
            _ => {}
        }
    }
    // Best score first; ties on the lexicographically smaller IRI (determinism).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    manual.sort_unstable();
    let (selected, score) = match scored.first() {
        Some((s, iri)) => (Some((*iri).to_string()), Some(*s)),
        None => (None, None),
    };
    let alternatives = scored
        .iter()
        .skip(1)
        .map(|(_, iri)| (*iri).to_string())
        .chain(manual.iter().map(|iri| (*iri).to_string()))
        .collect();
    Resolution {
        selected,
        score,
        alternatives,
    }
}

// ---- shared predicates ------------------------------------------------------

/// The literal datatype of the sample value, when the value is a literal.
fn value_datatype(ctx: &WidgetContext) -> Option<String> {
    match ctx.value {
        Some(Term::Literal(l)) => Some(l.datatype().as_str().to_string()),
        _ => None,
    }
}

fn value_is_iri(ctx: &WidgetContext) -> bool {
    matches!(ctx.value, Some(Term::NamedNode(_)))
}

fn value_is_bnode(ctx: &WidgetContext) -> bool {
    matches!(ctx.value, Some(Term::BlankNode(_)))
}

fn value_is_literal(ctx: &WidgetContext) -> bool {
    matches!(ctx.value, Some(Term::Literal(_)))
}

/// `sh:datatype` permits `dt` (empty set permits nothing specific).
fn permits(ctx: &WidgetContext, dt: &str) -> bool {
    ctx.constraints.datatype.iter().any(|d| d == dt)
}

/// The field is shaped as an IRI field: `sh:class`, or `sh:nodeKind sh:IRI`.
fn iri_shaped(ctx: &WidgetContext) -> bool {
    !ctx.constraints.class.is_empty() || ctx.constraints.node_kind.iter().any(|k| k == SH_IRI)
}

/// `dash:singleLine` (also set by the SHACL 1.2 `sh:singleLine` component).
fn single_line(ctx: &WidgetContext) -> Option<bool> {
    ctx.constraints.single_line
}

// ---- editor scorers (the documented table) -----------------------------------

fn text_field(ctx: &WidgetContext) -> Suitability {
    if let Some(v) = ctx.value {
        return match v {
            Term::Literal(l)
                if l.datatype().as_str() != RDF_LANGSTRING
                    && l.datatype().as_str() != XSD_BOOLEAN =>
            {
                Suitability::Score(10)
            }
            _ => Suitability::Unsuitable,
        };
    }
    if ctx
        .constraints
        .datatype
        .iter()
        .any(|d| d != RDF_LANGSTRING && d != XSD_BOOLEAN && d != RDF_HTML)
    {
        return Suitability::Score(10);
    }
    // (sparq) generic fallback for an empty, wholly unconstrained literal-ish
    // field: nothing signals IRI/enum/nested, so a plain text field applies.
    if ctx.constraints.datatype.is_empty()
        && !iri_shaped(ctx)
        && ctx.constraints.in_values.is_empty()
        && ctx.constraints.node_shape.is_none()
        && !ctx.constraints.node_kind.iter().any(|k| k.ends_with("BlankNode"))
    {
        return Suitability::Score(2);
    }
    Suitability::Unsuitable
}

fn text_field_with_lang(ctx: &WidgetContext) -> Suitability {
    if value_datatype(ctx).as_deref() == Some(RDF_LANGSTRING)
        || (ctx.value.is_none() && permits(ctx, RDF_LANGSTRING) && permits(ctx, XSD_STRING))
    {
        return Suitability::Score(11);
    }
    if permits(ctx, RDF_LANGSTRING) && single_line(ctx) != Some(false) {
        return Suitability::Score(5);
    }
    Suitability::Unsuitable
}

fn text_area(ctx: &WidgetContext) -> Suitability {
    if single_line(ctx) == Some(true) {
        return Suitability::Unsuitable;
    }
    let string_value = value_datatype(ctx).as_deref() == Some(XSD_STRING);
    let string_permitted = ctx.value.is_none() && permits(ctx, XSD_STRING);
    if (string_value || string_permitted) && single_line(ctx) == Some(false) {
        return Suitability::Score(20);
    }
    if string_value {
        return Suitability::Score(5);
    }
    if string_permitted {
        return Suitability::Score(2);
    }
    // Custom (non-XSD, non-RDF-namespace) datatype constraint: manual only.
    if ctx
        .constraints
        .datatype
        .iter()
        .any(|d| !d.starts_with(XSD_NS) && !d.starts_with(RDF_NS))
    {
        return Suitability::Manual;
    }
    Suitability::Unsuitable
}

fn text_area_with_lang(ctx: &WidgetContext) -> Suitability {
    if single_line(ctx) == Some(true) {
        return Suitability::Unsuitable;
    }
    let lang_value = value_datatype(ctx).as_deref() == Some(RDF_LANGSTRING);
    if lang_value && single_line(ctx) == Some(false) {
        return Suitability::Score(15);
    }
    if lang_value || permits(ctx, RDF_LANGSTRING) {
        return Suitability::Score(5);
    }
    Suitability::Unsuitable
}

fn boolean_select(ctx: &WidgetContext) -> Suitability {
    match value_datatype(ctx).as_deref() {
        Some(XSD_BOOLEAN) => return Suitability::Score(10),
        Some(_) => return Suitability::Unsuitable,
        None => {}
    }
    if ctx.value.is_some() {
        return Suitability::Unsuitable; // non-literal value
    }
    if permits(ctx, XSD_BOOLEAN) {
        return Suitability::Score(10);
    }
    if ctx.constraints.datatype.is_empty() && !iri_shaped(ctx) {
        return Suitability::Manual; // literal-shaped without a datatype
    }
    Suitability::Unsuitable
}

fn date_picker(ctx: &WidgetContext) -> Suitability {
    date_like(ctx, XSD_DATE)
}

fn date_time_picker(ctx: &WidgetContext) -> Suitability {
    date_like(ctx, XSD_DATETIME)
}

fn date_like(ctx: &WidgetContext, dt: &str) -> Suitability {
    if value_datatype(ctx).as_deref() == Some(dt) {
        return Suitability::Score(10);
    }
    if permits(ctx, dt) {
        return Suitability::Score(5);
    }
    Suitability::Unsuitable
}

fn enum_select(ctx: &WidgetContext) -> Suitability {
    if !ctx.constraints.in_values.is_empty() {
        return Suitability::Score(10);
    }
    Suitability::Unsuitable
}

fn auto_complete(ctx: &WidgetContext) -> Suitability {
    if value_is_iri(ctx) || (ctx.value.is_none() && iri_shaped(ctx)) {
        return Suitability::Score(1);
    }
    Suitability::Unsuitable
}

fn instances_select(ctx: &WidgetContext) -> Suitability {
    // (sparq) DASH leaves this manual-only; sh:class is the strongest signal
    // that the field edits references to instances, so it auto-selects.
    if !ctx.constraints.class.is_empty() {
        return Suitability::Score(12);
    }
    Suitability::Unsuitable
}

fn uri_editor(ctx: &WidgetContext) -> Suitability {
    let node_kind_iri = ctx.constraints.node_kind.iter().any(|k| k == SH_IRI);
    let iri_ok = value_is_iri(ctx) || ctx.value.is_none();
    if iri_ok && node_kind_iri && ctx.constraints.class.is_empty() {
        return Suitability::Score(10);
    }
    if value_is_iri(ctx) {
        return Suitability::Manual;
    }
    Suitability::Unsuitable
}

fn rich_text(ctx: &WidgetContext) -> Suitability {
    if value_datatype(ctx).as_deref() == Some(RDF_HTML)
        || (ctx.value.is_none() && permits(ctx, RDF_HTML))
    {
        return Suitability::Score(10);
    }
    Suitability::Unsuitable
}

fn sub_class(ctx: &WidgetContext) -> Suitability {
    // (sparq) DASH leaves this manual-only; dash:rootClass / sh:rootClass is
    // its defining signal, so it auto-selects.
    if ctx.constraints.root_class.is_some() {
        return Suitability::Score(15);
    }
    if value_is_iri(ctx) || iri_shaped(ctx) {
        return Suitability::Manual;
    }
    Suitability::Unsuitable
}

fn details_editor(ctx: &WidgetContext) -> Suitability {
    if value_is_literal(ctx) {
        return Suitability::Unsuitable;
    }
    // (sparq) sh:node names the sub-form's shape, so nesting auto-selects.
    if ctx.constraints.node_shape.is_some() {
        return Suitability::Score(15);
    }
    if ctx.value.is_some() {
        return Suitability::Manual;
    }
    Suitability::Unsuitable
}

fn blank_node_editor(ctx: &WidgetContext) -> Suitability {
    if value_is_bnode(ctx) {
        return Suitability::Score(1);
    }
    Suitability::Unsuitable
}

// ---- viewer scorers (the documented table) ------------------------------------

fn label_viewer(ctx: &WidgetContext) -> Suitability {
    if value_is_iri(ctx) {
        return Suitability::Score(5);
    }
    Suitability::Unsuitable
}

fn literal_viewer(ctx: &WidgetContext) -> Suitability {
    if value_is_literal(ctx) {
        return Suitability::Score(1);
    }
    Suitability::Unsuitable
}

fn image_viewer(ctx: &WidgetContext) -> Suitability {
    let text = match ctx.value {
        Some(Term::NamedNode(n)) => n.as_str().to_string(),
        Some(Term::Literal(l)) => l.value().to_string(),
        _ => return Suitability::Unsuitable,
    };
    let lower = text.to_ascii_lowercase();
    if [".png", ".jpg", ".jpeg", ".gif", ".svg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return Suitability::Score(50);
    }
    Suitability::Unsuitable
}

fn hyperlink_viewer(ctx: &WidgetContext) -> Suitability {
    match value_datatype(ctx).as_deref() {
        Some(XSD_ANYURI) => Suitability::Score(50),
        Some(XSD_STRING) => Suitability::Manual,
        _ => Suitability::Unsuitable,
    }
}

fn html_viewer(ctx: &WidgetContext) -> Suitability {
    if value_datatype(ctx).as_deref() == Some(RDF_HTML) {
        return Suitability::Score(50);
    }
    Suitability::Unsuitable
}

fn value_table_viewer(_ctx: &WidgetContext) -> Suitability {
    Suitability::Manual
}

fn details_viewer(ctx: &WidgetContext) -> Suitability {
    if value_is_literal(ctx) {
        return Suitability::Unsuitable;
    }
    // (sparq) mirror of dash:DetailsEditor: sh:node auto-selects the nested view.
    if ctx.constraints.node_shape.is_some() {
        return Suitability::Score(15);
    }
    if ctx.value.is_some() {
        return Suitability::Manual;
    }
    Suitability::Unsuitable
}

fn uri_viewer(ctx: &WidgetContext) -> Suitability {
    if value_is_iri(ctx) {
        return Suitability::Score(1);
    }
    Suitability::Unsuitable
}

fn lang_string_viewer(ctx: &WidgetContext) -> Suitability {
    if value_datatype(ctx).as_deref() == Some(RDF_LANGSTRING) {
        return Suitability::Score(10);
    }
    Suitability::Unsuitable
}

fn blank_node_viewer(ctx: &WidgetContext) -> Suitability {
    if value_is_bnode(ctx) {
        return Suitability::Score(1);
    }
    Suitability::Unsuitable
}

// sq-lsp7k.1.1: one test per documented scoring-table row — an
// implementation drift from the table flips the exact asserted number red.
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};

    fn iri(s: &str) -> Term {
        Term::from(NamedNode::new_unchecked(s.to_string()))
    }
    fn bnode() -> Term {
        Term::from(BlankNode::new_unchecked("b0"))
    }
    fn lit(v: &str) -> Term {
        Term::from(Literal::new_simple_literal(v))
    }
    fn typed(v: &str, dt: &str) -> Term {
        Term::from(Literal::new_typed_literal(
            v,
            NamedNode::new_unchecked(dt.to_string()),
        ))
    }
    fn lang(v: &str) -> Term {
        Term::from(Literal::new_language_tagged_literal_unchecked(v, "en"))
    }
    fn score(
        reg: &WidgetRegistry,
        editor: bool,
        name: &str,
        value: Option<&Term>,
        c: &Constraints,
    ) -> Suitability {
        let ctx = WidgetContext {
            value,
            constraints: c,
        };
        let iri = format!("{DASH}{name}");
        if editor {
            reg.score_editor(&iri, &ctx).expect("registered editor")
        } else {
            reg.score_viewer(&iri, &ctx).expect("registered viewer")
        }
    }

    fn c() -> Constraints {
        Constraints::default()
    }

    #[test]
    fn text_field_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "TextFieldEditor", Some(&lit("x")), &c()), Suitability::Score(10));
        assert_eq!(score(&r, true, "TextFieldEditor", Some(&lang("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, true, "TextFieldEditor", Some(&typed("true", XSD_BOOLEAN)), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, true, "TextFieldEditor", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
        let mut dt = c();
        dt.datatype = vec!["http://www.w3.org/2001/XMLSchema#integer".into()];
        assert_eq!(score(&r, true, "TextFieldEditor", None, &dt), Suitability::Score(10));
        // (sparq) generic fallback: empty, wholly unconstrained field.
        assert_eq!(score(&r, true, "TextFieldEditor", None, &c()), Suitability::Score(2));
        let mut cls = c();
        cls.class = vec![crate::description::TermRef::from_term(&iri("http://x/C"))];
        assert_eq!(score(&r, true, "TextFieldEditor", None, &cls), Suitability::Unsuitable);
    }

    #[test]
    fn text_field_with_lang_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "TextFieldWithLangEditor", Some(&lang("x")), &c()), Suitability::Score(11));
        let mut both = c();
        both.datatype = vec![RDF_LANGSTRING.into(), XSD_STRING.into()];
        assert_eq!(score(&r, true, "TextFieldWithLangEditor", None, &both), Suitability::Score(11));
        let mut lang_only = c();
        lang_only.datatype = vec![RDF_LANGSTRING.into()];
        assert_eq!(score(&r, true, "TextFieldWithLangEditor", None, &lang_only), Suitability::Score(5));
        lang_only.single_line = Some(false);
        assert_eq!(score(&r, true, "TextFieldWithLangEditor", None, &lang_only), Suitability::Unsuitable);
        assert_eq!(score(&r, true, "TextFieldWithLangEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn text_area_scores() {
        let r = WidgetRegistry::dash();
        let mut sl_true = c();
        sl_true.single_line = Some(true);
        assert_eq!(score(&r, true, "TextAreaEditor", Some(&lit("x")), &sl_true), Suitability::Unsuitable);
        let mut sl_false = c();
        sl_false.single_line = Some(false);
        assert_eq!(score(&r, true, "TextAreaEditor", Some(&lit("x")), &sl_false), Suitability::Score(20));
        assert_eq!(score(&r, true, "TextAreaEditor", Some(&lit("x")), &c()), Suitability::Score(5));
        let mut s = c();
        s.datatype = vec![XSD_STRING.into()];
        assert_eq!(score(&r, true, "TextAreaEditor", None, &s), Suitability::Score(2));
        s.single_line = Some(false);
        assert_eq!(score(&r, true, "TextAreaEditor", None, &s), Suitability::Score(20));
        let mut custom = c();
        custom.datatype = vec!["http://example.org/customType".into()];
        assert_eq!(score(&r, true, "TextAreaEditor", None, &custom), Suitability::Manual);
        assert_eq!(score(&r, true, "TextAreaEditor", Some(&typed("1", "http://www.w3.org/2001/XMLSchema#integer")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn text_area_with_lang_scores() {
        let r = WidgetRegistry::dash();
        let mut sl_false = c();
        sl_false.single_line = Some(false);
        assert_eq!(score(&r, true, "TextAreaWithLangEditor", Some(&lang("x")), &sl_false), Suitability::Score(15));
        assert_eq!(score(&r, true, "TextAreaWithLangEditor", Some(&lang("x")), &c()), Suitability::Score(5));
        let mut permits_lang = c();
        permits_lang.datatype = vec![RDF_LANGSTRING.into()];
        assert_eq!(score(&r, true, "TextAreaWithLangEditor", None, &permits_lang), Suitability::Score(5));
        let mut sl_true = c();
        sl_true.single_line = Some(true);
        assert_eq!(score(&r, true, "TextAreaWithLangEditor", Some(&lang("x")), &sl_true), Suitability::Unsuitable);
        assert_eq!(score(&r, true, "TextAreaWithLangEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn boolean_select_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "BooleanSelectEditor", Some(&typed("true", XSD_BOOLEAN)), &c()), Suitability::Score(10));
        assert_eq!(score(&r, true, "BooleanSelectEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, true, "BooleanSelectEditor", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
        let mut b = c();
        b.datatype = vec![XSD_BOOLEAN.into()];
        assert_eq!(score(&r, true, "BooleanSelectEditor", None, &b), Suitability::Score(10));
        assert_eq!(score(&r, true, "BooleanSelectEditor", None, &c()), Suitability::Manual);
        let mut cls = c();
        cls.class = vec![crate::description::TermRef::from_term(&iri("http://x/C"))];
        assert_eq!(score(&r, true, "BooleanSelectEditor", None, &cls), Suitability::Unsuitable);
    }

    #[test]
    fn date_picker_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "DatePickerEditor", Some(&typed("2026-07-11", XSD_DATE)), &c()), Suitability::Score(10));
        let mut d = c();
        d.datatype = vec![XSD_DATE.into()];
        assert_eq!(score(&r, true, "DatePickerEditor", None, &d), Suitability::Score(5));
        assert_eq!(score(&r, true, "DatePickerEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn date_time_picker_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "DateTimePickerEditor", Some(&typed("2026-07-11T00:00:00", XSD_DATETIME)), &c()), Suitability::Score(10));
        let mut d = c();
        d.datatype = vec![XSD_DATETIME.into()];
        assert_eq!(score(&r, true, "DateTimePickerEditor", None, &d), Suitability::Score(5));
        assert_eq!(score(&r, true, "DateTimePickerEditor", Some(&typed("2026-07-11", XSD_DATE)), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn enum_select_scores() {
        let r = WidgetRegistry::dash();
        let mut e = c();
        e.in_values = vec![crate::description::TermRef::from_term(&lit("a"))];
        assert_eq!(score(&r, true, "EnumSelectEditor", None, &e), Suitability::Score(10));
        assert_eq!(score(&r, true, "EnumSelectEditor", Some(&lit("a")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn auto_complete_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "AutoCompleteEditor", Some(&iri("http://x/a")), &c()), Suitability::Score(1));
        let mut cls = c();
        cls.class = vec![crate::description::TermRef::from_term(&iri("http://x/C"))];
        assert_eq!(score(&r, true, "AutoCompleteEditor", None, &cls), Suitability::Score(1));
        assert_eq!(score(&r, true, "AutoCompleteEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn instances_select_scores() {
        let r = WidgetRegistry::dash();
        let mut cls = c();
        cls.class = vec![crate::description::TermRef::from_term(&iri("http://x/C"))];
        // (sparq) 12 — DASH leaves this manual-only.
        assert_eq!(score(&r, true, "InstancesSelectEditor", None, &cls), Suitability::Score(12));
        assert_eq!(score(&r, true, "InstancesSelectEditor", None, &c()), Suitability::Unsuitable);
    }

    #[test]
    fn uri_editor_scores() {
        let r = WidgetRegistry::dash();
        let mut nk = c();
        nk.node_kind = vec![SH_IRI.into()];
        assert_eq!(score(&r, true, "URIEditor", Some(&iri("http://x/a")), &nk), Suitability::Score(10));
        assert_eq!(score(&r, true, "URIEditor", None, &nk), Suitability::Score(10));
        nk.class = vec![crate::description::TermRef::from_term(&iri("http://x/C"))];
        assert_eq!(score(&r, true, "URIEditor", Some(&iri("http://x/a")), &nk), Suitability::Manual);
        assert_eq!(score(&r, true, "URIEditor", Some(&iri("http://x/a")), &c()), Suitability::Manual);
        assert_eq!(score(&r, true, "URIEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn rich_text_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "RichTextEditor", Some(&typed("<b>x</b>", RDF_HTML)), &c()), Suitability::Score(10));
        let mut h = c();
        h.datatype = vec![RDF_HTML.into()];
        assert_eq!(score(&r, true, "RichTextEditor", None, &h), Suitability::Score(10));
        assert_eq!(score(&r, true, "RichTextEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn sub_class_scores() {
        let r = WidgetRegistry::dash();
        let mut rc = c();
        rc.root_class = Some(crate::description::TermRef::from_term(&iri("http://x/C")));
        // (sparq) 15 — DASH leaves this manual-only.
        assert_eq!(score(&r, true, "SubClassEditor", None, &rc), Suitability::Score(15));
        assert_eq!(score(&r, true, "SubClassEditor", Some(&iri("http://x/a")), &c()), Suitability::Manual);
        assert_eq!(score(&r, true, "SubClassEditor", Some(&lit("x")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn details_editor_scores() {
        let r = WidgetRegistry::dash();
        let mut ns = c();
        ns.node_shape = Some(crate::description::TermRef::from_term(&iri("http://x/S")));
        // (sparq) 15 — nested sub-form via sh:node.
        assert_eq!(score(&r, true, "DetailsEditor", Some(&bnode()), &ns), Suitability::Score(15));
        assert_eq!(score(&r, true, "DetailsEditor", None, &ns), Suitability::Score(15));
        assert_eq!(score(&r, true, "DetailsEditor", Some(&iri("http://x/a")), &c()), Suitability::Manual);
        assert_eq!(score(&r, true, "DetailsEditor", Some(&lit("x")), &ns), Suitability::Unsuitable);
    }

    #[test]
    fn blank_node_editor_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, true, "BlankNodeEditor", Some(&bnode()), &c()), Suitability::Score(1));
        assert_eq!(score(&r, true, "BlankNodeEditor", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn viewer_scores() {
        let r = WidgetRegistry::dash();
        assert_eq!(score(&r, false, "LabelViewer", Some(&iri("http://x/a")), &c()), Suitability::Score(5));
        assert_eq!(score(&r, false, "LabelViewer", Some(&lit("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "LiteralViewer", Some(&lit("x")), &c()), Suitability::Score(1));
        assert_eq!(score(&r, false, "LiteralViewer", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "ImageViewer", Some(&iri("http://x/a.PNG")), &c()), Suitability::Score(50));
        assert_eq!(score(&r, false, "ImageViewer", Some(&lit("pic.jpeg")), &c()), Suitability::Score(50));
        assert_eq!(score(&r, false, "ImageViewer", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "HyperlinkViewer", Some(&typed("http://x/", XSD_ANYURI)), &c()), Suitability::Score(50));
        assert_eq!(score(&r, false, "HyperlinkViewer", Some(&lit("x")), &c()), Suitability::Manual);
        assert_eq!(score(&r, false, "HyperlinkViewer", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "HTMLViewer", Some(&typed("<b>x</b>", RDF_HTML)), &c()), Suitability::Score(50));
        assert_eq!(score(&r, false, "HTMLViewer", Some(&lit("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "ValueTableViewer", None, &c()), Suitability::Manual);
        assert_eq!(score(&r, false, "ValueTableViewer", Some(&lit("x")), &c()), Suitability::Manual);
        assert_eq!(score(&r, false, "DetailsViewer", Some(&lit("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "DetailsViewer", Some(&iri("http://x/a")), &c()), Suitability::Manual);
        let mut ns = c();
        ns.node_shape = Some(crate::description::TermRef::from_term(&iri("http://x/S")));
        assert_eq!(score(&r, false, "DetailsViewer", Some(&bnode()), &ns), Suitability::Score(15));
        assert_eq!(score(&r, false, "URIViewer", Some(&iri("http://x/a")), &c()), Suitability::Score(1));
        assert_eq!(score(&r, false, "LangStringViewer", Some(&lang("x")), &c()), Suitability::Score(10));
        assert_eq!(score(&r, false, "LangStringViewer", Some(&lit("x")), &c()), Suitability::Unsuitable);
        assert_eq!(score(&r, false, "BlankNodeViewer", Some(&bnode()), &c()), Suitability::Score(1));
        assert_eq!(score(&r, false, "BlankNodeViewer", Some(&iri("http://x/a")), &c()), Suitability::Unsuitable);
    }

    #[test]
    fn resolve_editor_picks_best_and_lists_alternatives() {
        let r = WidgetRegistry::dash();
        // sh:in on a literal: EnumSelect(10) beats TextField(10)? both 10 —
        // ties break lexicographically: EnumSelectEditor < TextFieldEditor.
        let e = Constraints {
            in_values: vec![crate::description::TermRef::from_term(&lit("a"))],
            ..Constraints::default()
        };
        let value = lit("a");
        let ctx = WidgetContext {
            value: Some(&value),
            constraints: &e,
        };
        let res = r.resolve_editor(&ctx);
        assert_eq!(res.selected.as_deref(), Some(&*format!("{DASH}EnumSelectEditor")));
        assert_eq!(res.score, Some(10));
        // TextField(10) is the first runner-up; TextArea(5) follows.
        assert_eq!(res.alternatives[0], format!("{DASH}TextFieldEditor"));
        assert!(res.alternatives.contains(&format!("{DASH}TextAreaEditor")));
    }

    #[test]
    fn resolve_viewer_prefers_strong_signal() {
        let r = WidgetRegistry::dash();
        let v = typed("<b>hi</b>", RDF_HTML);
        let ctx = WidgetContext {
            value: Some(&v),
            constraints: &Constraints::default(),
        };
        let res = r.resolve_viewer(&ctx);
        assert_eq!(res.selected.as_deref(), Some(&*format!("{DASH}HTMLViewer")));
        assert_eq!(res.score, Some(50));
        // Manual-only widgets trail the scored runner-ups.
        assert!(res.alternatives.contains(&format!("{DASH}ValueTableViewer")));
    }

    #[test]
    fn register_editor_adds_and_replaces() {
        let mut r = WidgetRegistry::dash();
        fn always_99(_: &WidgetContext) -> Suitability {
            Suitability::Score(99)
        }
        r.register_editor("http://example.org/MyEditor", always_99);
        let ctx = WidgetContext {
            value: None,
            constraints: &Constraints::default(),
        };
        assert_eq!(r.score_editor("http://example.org/MyEditor", &ctx), Some(Suitability::Score(99)));
        assert_eq!(r.resolve_editor(&ctx).selected.as_deref(), Some("http://example.org/MyEditor"));
        // Replacement by IRI (not duplication).
        fn always_manual(_: &WidgetContext) -> Suitability {
            Suitability::Manual
        }
        r.register_editor("http://example.org/MyEditor", always_manual);
        assert_eq!(r.score_editor("http://example.org/MyEditor", &ctx), Some(Suitability::Manual));
    }

    #[test]
    fn register_viewer_adds() {
        let mut r = WidgetRegistry::dash();
        fn always_60(_: &WidgetContext) -> Suitability {
            Suitability::Score(60)
        }
        r.register_viewer("http://example.org/MyViewer", always_60);
        let v = lit("x");
        let ctx = WidgetContext {
            value: Some(&v),
            constraints: &Constraints::default(),
        };
        assert_eq!(r.score_viewer("http://example.org/MyViewer", &ctx), Some(Suitability::Score(60)));
        assert_eq!(r.resolve_viewer(&ctx).selected.as_deref(), Some("http://example.org/MyViewer"));
    }

    #[test]
    fn score_unknown_widget_is_none() {
        let r = WidgetRegistry::dash();
        let ctx = WidgetContext {
            value: None,
            constraints: &Constraints::default(),
        };
        assert_eq!(r.score_editor("http://example.org/Nope", &ctx), None);
        assert_eq!(r.score_viewer("http://example.org/Nope", &ctx), None);
    }

    #[test]
    fn default_registry_is_dash() {
        let r = WidgetRegistry::default();
        let ctx = WidgetContext {
            value: None,
            constraints: &Constraints::default(),
        };
        assert!(r.score_editor(&format!("{DASH}TextFieldEditor"), &ctx).is_some());
    }
}
