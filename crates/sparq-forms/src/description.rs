//! The serde-JSON [`FormDescription`] model — the headless contract between the
//! derivation engine (this crate) and any renderer (Tauri workbench, hosted web
//! via wasm, sparq-mcp agent tools). [FABLE-5] sq-lsp7k.1.1
//!
//! Everything here is plain data: renderers never need sparq types to consume a
//! form, and the model round-trips through JSON losslessly (`Deserialize` is
//! derived so the F6 agent surface can echo edits back).

use oxrdf::Term;
use serde::{Deserialize, Serialize};

/// Whether the form is being derived for display or for editing.
///
/// In [`Mode::View`] no field is editable and only viewer widgets are resolved;
/// in [`Mode::Edit`] declared (on-shape) fields carry editor widgets too.
/// Off-shape fields (the implicit "Other properties" group) are read-only in
/// BOTH modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Read-only display.
    View,
    /// Data entry / editing (the default: forms exist to edit).
    #[default]
    Edit,
}

/// A reference to an RDF term, structured for JSON consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermRef {
    /// `"iri"`, `"bnode"`, `"literal"` or `"triple"` (RDF 1.2 triple term,
    /// carried as its N-Triples text until F5 models annotations structurally).
    pub kind: String,
    /// IRI string, blank-node label, literal lexical form, or triple-term text.
    pub value: String,
    /// Literal datatype IRI (omitted for plain `xsd:string`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub datatype: Option<String>,
    /// Literal language tag (`rdf:langString`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub language: Option<String>,
}

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

impl TermRef {
    /// Structures an [`oxrdf::Term`] into a JSON-ready reference.
    pub fn from_term(t: &Term) -> TermRef {
        match t {
            Term::NamedNode(n) => TermRef {
                kind: "iri".into(),
                value: n.as_str().to_string(),
                datatype: None,
                language: None,
            },
            Term::BlankNode(b) => TermRef {
                kind: "bnode".into(),
                value: b.as_str().to_string(),
                datatype: None,
                language: None,
            },
            Term::Literal(l) => TermRef {
                kind: "literal".into(),
                value: l.value().to_string(),
                datatype: match l.datatype().as_str() {
                    XSD_STRING => None,
                    dt => Some(dt.to_string()),
                },
                language: l.language().map(str::to_string),
            },
            // RDF 1.2 triple terms (and anything else oxrdf grows): N-Triples text.
            other => TermRef {
                kind: "triple".into(),
                value: other.to_string(),
                datatype: None,
                language: None,
            },
        }
    }
}

/// How a node shape came to apply to the focus node (the switcher rationale).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShapeVia {
    /// `sh:targetNode` names the focus node.
    TargetNode,
    /// `sh:targetClass` (or SHACL implicit class target) matches an
    /// `rdf:type` of the focus node (with `rdfs:subClassOf` closure).
    TargetClass,
    /// `dash:applicableToClass` matches an `rdf:type` of the focus node.
    ApplicableToClass,
    /// `sh:targetSubjectsOf` names a predicate the focus node is a subject of.
    /// Ranked BELOW [`Self::ApplicableToClass`] in the switcher: a predicate
    /// target says the node participates in a relation, not that the shape
    /// describes what the node *is*. [OPUS-4.8] sq-vfcxv
    TargetSubjectsOf,
    /// `sh:targetObjectsOf` names a predicate the focus node is an object of
    /// (ranked below [`Self::TargetSubjectsOf`]). [OPUS-4.8] sq-vfcxv
    TargetObjectsOf,
    /// Explicitly requested via [`crate::FormOptions::shape`].
    Explicit,
}

/// One applicable node shape — an entry in the renderer's shape switcher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeChoice {
    /// The shape's node in the shapes graph.
    pub shape: TermRef,
    /// `rdfs:label` / `sh:name` of the shape, when declared.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// Why the shape applies.
    pub via: ShapeVia,
}

/// The kind of a form group (renderers may style them differently).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupKind {
    /// Fields whose property shape names no `sh:group` (rendered first).
    Default,
    /// An explicit `sh:PropertyGroup` (`sh:group`), ordered by `sh:order`.
    Declared,
    /// The implicit trailing "Other properties" group: focus-node triples whose
    /// predicate no declared field covers, shown READ-ONLY.
    Other,
}

/// A collapsible section of the form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormGroup {
    pub kind: GroupKind,
    /// The `sh:PropertyGroup` node ([`GroupKind::Declared`] only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub group: Option<TermRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// `sh:order` of the group (fractional decimals supported).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub order: Option<f64>,
    pub fields: Vec<FormField>,
}

/// The widget resolution for a field: the auto-selected (or explicitly
/// declared) editor/viewer plus the scored alternatives for a switcher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetChoice {
    /// Selected editor widget IRI (`dash:*Editor`); absent in view mode.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub editor: Option<String>,
    /// Selected viewer widget IRI (`dash:*Viewer`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub viewer: Option<String>,
    /// `true` when the editor/viewer came from an explicit `dash:editor` /
    /// `dash:viewer` statement rather than the scoring registry.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub explicit: bool,
    /// The winning suitability score (absent for explicit declarations and
    /// for manual-only selections).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub score: Option<u8>,
    /// Other usable editor widgets, best first (scored > 0, then manual-only).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub editor_alternatives: Vec<String>,
    /// Other usable viewer widgets, best first (scored > 0, then manual-only).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub viewer_alternatives: Vec<String>,
}

/// The constraints carried per field — both the widget-scoring inputs and the
/// renderer's live-validation hints (F3 wires full validation).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Constraints {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_count: Option<u64>,
    /// `sh:datatype` allowed-datatype set (SHACL 1.2 lists supported).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub datatype: Vec<String>,
    /// `sh:class` value-class IRIs.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub class: Vec<TermRef>,
    /// `sh:nodeKind` allowed-kind IRIs.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub node_kind: Vec<String>,
    /// `sh:in` enumeration, in list order.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub in_values: Vec<TermRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pattern_flags: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_inclusive: Option<TermRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_inclusive: Option<TermRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_exclusive: Option<TermRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_exclusive: Option<TermRef>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub language_in: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub unique_lang: bool,
    /// `dash:singleLine` (drives TextField vs TextArea scoring).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub single_line: Option<bool>,
    /// `dash:rootClass` / `sh:rootClass` (drives `dash:SubClassEditor`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root_class: Option<TermRef>,
    /// The `sh:node` shape values must conform to (drives the nested
    /// `dash:DetailsEditor` sub-form).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub node_shape: Option<TermRef>,
    /// `sh:or` union branches, each flattened to its own constraint set
    /// (one level deep; renderers surface these as "any of").
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub or: Vec<Constraints>,
}

/// One value of a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormValue {
    pub term: TermRef,
    /// Nested sub-form (`sh:node` + `dash:DetailsEditor` recursion), derived
    /// with this value as the focus node, depth-limited by
    /// [`crate::FormOptions::max_depth`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nested: Option<Box<FormDescription>>,
}

/// [GPT-5.6] One SHACL validation result attached to its declared form field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationHint {
    /// Constraint-component IRI (for example, `sh:PatternConstraintComponent`).
    pub source_component: String,
    /// Shape-provided `sh:message`, or the validator's generated fallback.
    pub message: String,
    /// Offending value node, when the constraint reports one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<TermRef>,
    /// Result severity IRI (`sh:Violation`, `sh:Warning`, `sh:Info`, or custom).
    pub severity: String,
}

/// One field of the form (per property shape, or per off-shape predicate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    /// The property shape node in the shapes graph (absent for off-shape
    /// "Other properties" fields).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub property_shape: Option<TermRef>,
    /// The field's path as a SPARQL property-path expression (e.g. `<p>` or
    /// `^<p>` for incoming references).
    pub path: String,
    /// `true` for `sh:inversePath` fields (incoming references).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub inverse: bool,
    /// `sh:name`, falling back to the predicate's `rdfs:label` in the shapes
    /// graph, falling back to the IRI local name.
    pub label: String,
    /// `sh:description`, falling back to the predicate's `rdfs:comment`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// `sh:order` (fractional decimals supported); unordered fields sort last.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub order: Option<f64>,
    /// `sh:minCount >= 1`.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub required: bool,
    /// `sh:maxCount != 1`: the renderer shows add/remove affordances.
    pub multi: bool,
    /// `false` for view mode, for off-shape (read-only) fields, and for
    /// property shapes declaring `dash:readOnly true` (read-only even in
    /// edit mode). [FABLE-5]
    pub editable: bool,
    /// `dash:hidden true` on the property shape: the field still participates
    /// in the data model (values, constraints, diffing) but a renderer should
    /// not display it. [FABLE-5] sq-lsp7k.1.5
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub hidden: bool,
    /// `sh:defaultValue` on the property shape, carried verbatim: the seed
    /// value a renderer pre-fills when the field currently has no values.
    /// [FABLE-5] sq-lsp7k.1.5
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default_value: Option<TermRef>,
    pub widget: WidgetChoice,
    pub values: Vec<FormValue>,
    pub constraints: Constraints,
    /// [GPT-5.6] Live SHACL results for this declared, editable field.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub validation: Vec<ValidationHint>,
}

/// The whole derived form: what a renderer needs to draw (and an agent needs
/// to understand) a shape-directed view/editor for one focus node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormDescription {
    pub focus: TermRef,
    pub mode: Mode,
    /// Opaque role passed through from [`crate::FormOptions`] (dash:propertyRole
    /// filtering is F5, sq-lsp7k.1.5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,
    /// All applicable node shapes — the renderer's shape switcher.
    pub shapes: Vec<ShapeChoice>,
    /// The selected shape ([`crate::FormOptions::shape`] or the first
    /// applicable one); absent when no shape applies (all-off-shape form).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub shape: Option<TermRef>,
    pub groups: Vec<FormGroup>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};

    #[test]
    fn term_ref_from_term_structures_all_kinds() {
        let iri = Term::from(NamedNode::new_unchecked("http://example.org/a"));
        assert_eq!(
            TermRef::from_term(&iri),
            TermRef {
                kind: "iri".into(),
                value: "http://example.org/a".into(),
                datatype: None,
                language: None
            }
        );
        let b = Term::from(BlankNode::new_unchecked("b7"));
        let r = TermRef::from_term(&b);
        assert_eq!((r.kind.as_str(), r.value.as_str()), ("bnode", "b7"));
        // Plain xsd:string literals omit the datatype (JSON noise reduction).
        let plain = Term::from(Literal::new_simple_literal("hi"));
        let r = TermRef::from_term(&plain);
        assert_eq!((r.kind.as_str(), r.value.as_str()), ("literal", "hi"));
        assert!(r.datatype.is_none() && r.language.is_none());
        let typed = Term::from(Literal::new_typed_literal(
            "4",
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        ));
        assert_eq!(
            TermRef::from_term(&typed).datatype.as_deref(),
            Some("http://www.w3.org/2001/XMLSchema#integer")
        );
        let lang = Term::from(Literal::new_language_tagged_literal_unchecked("hei", "no"));
        let r = TermRef::from_term(&lang);
        assert_eq!(r.language.as_deref(), Some("no"));
        assert_eq!(
            r.datatype.as_deref(),
            Some("http://www.w3.org/1999/02/22-rdf-syntax-ns#langString")
        );
    }

    #[test]
    fn mode_default_is_edit_and_serializes_lowercase() {
        assert_eq!(Mode::default(), Mode::Edit);
        assert_eq!(serde_json::to_string(&Mode::View).unwrap(), "\"view\"");
    }
}
