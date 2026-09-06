//! The derivation engine: (data graph, shapes graph, focus node, options) →
//! [`FormDescription`]. [FABLE-5] sq-lsp7k.1.1

use crate::description::{
    Constraints, FormDescription, FormField, FormGroup, FormValue, GroupKind, Mode, ShapeChoice,
    ShapeVia, TermRef, WidgetChoice,
};
use crate::widgets::{WidgetContext, WidgetRegistry};
use crate::FormOptions;
use oxrdf::Term;
use sparq_shacl::model::{Component, Shape, ShapesModel, Target};
use sparq_shacl::view::GraphView;
use sparq_shacl::Path;
use std::collections::HashSet;

const SH: &str = "http://www.w3.org/ns/shacl#";
const DASH: &str = "http://datashapes.org/dash#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const RDFS_COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";

fn sh(local: &str) -> String {
    format!("{SH}{local}")
}

fn dash(local: &str) -> String {
    format!("{DASH}{local}")
}

/// Nested-recursion guard: (property-shape index, focus term) pairs currently
/// being derived — re-entry means a shapes-graph cycle over the same value.
type Visiting = HashSet<(usize, Term)>;

pub(crate) fn applicable(
    data: &GraphView,
    shapes: &GraphView,
    model: &ShapesModel,
    focus: &Term,
) -> Vec<ShapeChoice> {
    let mut out: Vec<(u8, String, ShapeChoice)> = Vec::new();
    for shape in &model.shapes {
        // Only node shapes enter the switcher; property shapes are fields.
        if shape.path.is_some() || shape.deactivated {
            continue;
        }
        let mut via: Option<ShapeVia> = None;
        for t in &shape.targets {
            let matched = match t {
                Target::Node(n) if n == focus => Some(ShapeVia::TargetNode),
                Target::Class(c) | Target::ImplicitClass(c) if data.is_instance_of(focus, c) => {
                    Some(ShapeVia::TargetClass)
                }
                // [OPUS-4.8] sq-vfcxv: predicate targets DO drive applicability —
                // the focus node is a subject (resp. object) of the predicate in
                // the data graph — but rank below the class-based rationales.
                Target::SubjectsOf(p) if !data.objects(focus, p).is_empty() => {
                    Some(ShapeVia::TargetSubjectsOf)
                }
                Target::ObjectsOf(p) if !data.subjects(p, focus).is_empty() => {
                    Some(ShapeVia::TargetObjectsOf)
                }
                // sh:targetWhere / SPARQL-valued targets need a conformance
                // check (resp. a node-expression evaluation) that sparq-shacl
                // does not expose publicly — still skipped, see the crate docs.
                _ => None,
            };
            via = stronger(via, matched);
            if via == Some(ShapeVia::TargetNode) {
                break; // strongest rationale
            }
        }
        // dash:applicableToClass outranks the predicate targets, so it is still
        // consulted when only one of those matched. [OPUS-4.8] sq-vfcxv
        if via.is_none_or(|v| via_rank(v) > via_rank(ShapeVia::ApplicableToClass))
            && shapes
                .objects(&shape.node, &dash("applicableToClass"))
                .iter()
                .any(|c| data.is_instance_of(focus, c))
        {
            via = Some(ShapeVia::ApplicableToClass);
        }
        if let Some(via) = via {
            // Strongest rationale first; the FIRST choice becomes the default
            // selected shape.
            let rank = via_rank(via);
            let choice = ShapeChoice {
                shape: TermRef::from_term(&shape.node),
                label: shape_label(shapes, &shape.node),
                via,
            };
            out.push((rank, shape.node.to_string(), choice));
        }
    }
    // Deterministic switcher order: rationale rank, then the node's text.
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, _, c)| c).collect()
}

/// Switcher strength of a rationale — LOWER sorts first. An explicit node
/// target beats a class target, which beats `dash:applicableToClass`, which
/// beats the predicate targets (`sh:targetSubjectsOf` then
/// `sh:targetObjectsOf`). [OPUS-4.8] sq-vfcxv
fn via_rank(via: ShapeVia) -> u8 {
    match via {
        ShapeVia::TargetNode => 0,
        ShapeVia::TargetClass => 1,
        ShapeVia::ApplicableToClass => 2,
        ShapeVia::TargetSubjectsOf => 3,
        ShapeVia::TargetObjectsOf => 4,
        ShapeVia::Explicit => 0, // unreachable in `applicable` (inserted by derive)
    }
}

/// The stronger of two rationales for the SAME shape — one shape may carry
/// several matching targets, and the switcher reports the strongest.
fn stronger(a: Option<ShapeVia>, b: Option<ShapeVia>) -> Option<ShapeVia> {
    match (a, b) {
        (Some(a), Some(b)) if via_rank(b) < via_rank(a) => Some(b),
        (Some(a), _) => Some(a),
        (None, b) => b,
    }
}

pub(crate) fn derive(
    data: &GraphView,
    shapes: &GraphView,
    model: &ShapesModel,
    focus: &Term,
    opts: &FormOptions,
    registry: &WidgetRegistry,
) -> FormDescription {
    let mut visiting = Visiting::new();
    let mut form = derive_inner(data, shapes, model, focus, opts, registry, 0, &mut visiting);
    // Parser-generated blank-node labels are random (anonymous `[ … ]` shapes
    // get a fresh id per parse), so rename every blank node in first-encounter
    // order: the description becomes DETERMINISTIC for identical inputs (the
    // golden-file contract), and no meaningless parser ids leak to renderers.
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    normalize_form(&mut form, &mut names);
    form
}

// ---- deterministic blank-node renaming over the emitted description ---------

fn normalize_form(form: &mut FormDescription, names: &mut std::collections::HashMap<String, String>) {
    normalize_ref(&mut form.focus, names);
    for c in &mut form.shapes {
        normalize_ref(&mut c.shape, names);
    }
    if let Some(s) = &mut form.shape {
        normalize_ref(s, names);
    }
    for g in &mut form.groups {
        if let Some(n) = &mut g.group {
            normalize_ref(n, names);
        }
        for f in &mut g.fields {
            if let Some(n) = &mut f.property_shape {
                normalize_ref(n, names);
            }
            normalize_constraints(&mut f.constraints, names);
            // sh:defaultValue may (degenerately) be a blank node — keep the
            // deterministic-label contract. [FABLE] sq-lsp7k.1.5
            if let Some(dv) = &mut f.default_value {
                normalize_ref(dv, names);
            }
            for v in &mut f.values {
                normalize_ref(&mut v.term, names);
                if let Some(nested) = &mut v.nested {
                    normalize_form(nested, names);
                }
            }
        }
    }
}

fn normalize_constraints(
    c: &mut crate::description::Constraints,
    names: &mut std::collections::HashMap<String, String>,
) {
    for r in &mut c.class {
        normalize_ref(r, names);
    }
    for r in &mut c.in_values {
        normalize_ref(r, names);
    }
    for r in [
        &mut c.min_inclusive,
        &mut c.max_inclusive,
        &mut c.min_exclusive,
        &mut c.max_exclusive,
        &mut c.root_class,
        &mut c.node_shape,
    ]
    .into_iter()
    .flatten()
    {
        normalize_ref(r, names);
    }
    for branch in &mut c.or {
        normalize_constraints(branch, names);
    }
}

fn normalize_ref(r: &mut TermRef, names: &mut std::collections::HashMap<String, String>) {
    if r.kind == "bnode" {
        let next = format!("b{}", names.len());
        r.value = names.entry(r.value.clone()).or_insert(next).clone();
    }
}

#[allow(clippy::too_many_arguments)] // internal recursion seam, not public API
fn derive_inner(
    data: &GraphView,
    shapes: &GraphView,
    model: &ShapesModel,
    focus: &Term,
    opts: &FormOptions,
    registry: &WidgetRegistry,
    depth: usize,
    visiting: &mut Visiting,
) -> FormDescription {
    let mut choices = applicable(data, shapes, model, focus);
    // An explicit shape request joins the switcher (marked Explicit) and wins.
    let selected_idx: Option<usize> = match &opts.shape {
        Some(node) => {
            let idx = model.by_node(node);
            if idx.is_some() && !choices.iter().any(|c| term_ref_is(node, &c.shape)) {
                choices.insert(
                    0,
                    ShapeChoice {
                        shape: TermRef::from_term(node),
                        label: shape_label(shapes, node),
                        via: ShapeVia::Explicit,
                    },
                );
            }
            idx
        }
        None => choices
            .first()
            .and_then(|c| model.by_node(&term_of_ref(&c.shape))),
    };

    let mut declared_fields: Vec<(Option<Term>, FormField)> = Vec::new(); // (group node, field)
    let mut covered_predicates: HashSet<String> = HashSet::new();
    if let Some(idx) = selected_idx {
        let shape = &model.shapes[idx];
        for &child in &shape.property_children {
            let prop = &model.shapes[child];
            if prop.deactivated {
                continue; // sh:deactivated true suppresses the field
            }
            let Some(path) = &prop.path else { continue };
            if let Path::Predicate(p) = path {
                covered_predicates.insert(p.clone());
            }
            let group = shapes.object(&prop.node, &sh("group"));
            let field = build_field(
                data, shapes, model, focus, opts, registry, child, prop, path, depth, visiting,
            );
            declared_fields.push((group, field));
        }
    }

    // ---- group assembly: default group, declared sh:PropertyGroups, Other ----
    let mut groups: Vec<FormGroup> = Vec::new();
    let mut default_fields: Vec<FormField> = Vec::new();
    let mut declared_groups: Vec<(Term, FormGroup)> = Vec::new();
    for (group_node, field) in declared_fields {
        match group_node {
            None => default_fields.push(field),
            Some(node) => {
                if let Some((_, g)) = declared_groups.iter_mut().find(|(n, _)| *n == node) {
                    g.fields.push(field);
                } else {
                    declared_groups.push((
                        node.clone(),
                        FormGroup {
                            kind: GroupKind::Declared,
                            group: Some(TermRef::from_term(&node)),
                            label: shape_label(shapes, &node),
                            order: order_of(shapes, &node),
                            fields: vec![field],
                        },
                    ));
                }
            }
        }
    }
    sort_fields(&mut default_fields);
    for (_, g) in &mut declared_groups {
        sort_fields(&mut g.fields);
    }
    // Groups sort by sh:order (fractional decimals; unordered last), then label.
    declared_groups.sort_by(|(_, a), (_, b)| {
        order_key(a.order)
            .total_cmp(&order_key(b.order))
            .then_with(|| a.label.cmp(&b.label))
    });
    if !default_fields.is_empty() {
        groups.push(FormGroup {
            kind: GroupKind::Default,
            group: None,
            label: None,
            order: None,
            fields: default_fields,
        });
    }
    groups.extend(declared_groups.into_iter().map(|(_, g)| g));

    // ---- the implicit read-only "Other properties" group (off-shape triples) ----
    let other_fields = other_fields(data, shapes, focus, registry, &covered_predicates);
    if !other_fields.is_empty() {
        groups.push(FormGroup {
            kind: GroupKind::Other,
            group: None,
            label: Some("Other properties".to_string()),
            order: None,
            fields: other_fields,
        });
    }

    FormDescription {
        focus: TermRef::from_term(focus),
        mode: opts.mode,
        role: opts.role.clone(),
        shape: selected_idx.map(|i| TermRef::from_term(&model.shapes[i].node)),
        shapes: choices,
        groups,
    }
}

#[allow(clippy::too_many_arguments)] // internal seam: one call site in derive_inner
fn build_field(
    data: &GraphView,
    shapes: &GraphView,
    model: &ShapesModel,
    focus: &Term,
    opts: &FormOptions,
    registry: &WidgetRegistry,
    prop_idx: usize,
    prop: &Shape,
    path: &Path,
    depth: usize,
    visiting: &mut Visiting,
) -> FormField {
    let constraints = constraints_of(shapes, model, prop, 2);
    let raw_values = path.values(data, focus);

    // ---- widget resolution: explicit dash:editor / dash:viewer beat scoring ----
    let sample = raw_values.first();
    let ctx = WidgetContext {
        value: sample,
        constraints: &constraints,
    };
    let editor_res = registry.resolve_editor(&ctx);
    let viewer_res = registry.resolve_viewer(&ctx);
    let explicit_editor = iri_object(shapes, &prop.node, &dash("editor"));
    let explicit_viewer = iri_object(shapes, &prop.node, &dash("viewer"));
    let explicit = explicit_editor.is_some() || explicit_viewer.is_some();
    let editor = match (&opts.mode, &explicit_editor) {
        (Mode::View, _) => None,
        (Mode::Edit, Some(e)) => Some(e.clone()),
        (Mode::Edit, None) => editor_res.selected.clone(),
    };
    let viewer = explicit_viewer.clone().or(viewer_res.selected.clone());
    let widget = WidgetChoice {
        editor,
        viewer,
        explicit,
        score: if explicit { None } else { editor_res.score.or(viewer_res.score) },
        editor_alternatives: match &opts.mode {
            Mode::View => Vec::new(),
            Mode::Edit => alternatives(&editor_res, &explicit_editor),
        },
        viewer_alternatives: alternatives(&viewer_res, &explicit_viewer),
    };

    // ---- values (+ nested sub-forms for sh:node / DetailsEditor recursion) ----
    let node_shape_idx = prop.components.iter().find_map(|c| match c {
        Component::Node(i) => Some(*i),
        _ => None,
    });
    let values: Vec<FormValue> = raw_values
        .iter()
        .map(|v| {
            let nested = match node_shape_idx {
                Some(nidx)
                    if !matches!(v, Term::Literal(_))
                        && depth < opts.max_depth
                        && visiting.insert((prop_idx, v.clone())) =>
                {
                    let nested_opts = FormOptions {
                        mode: opts.mode,
                        role: opts.role.clone(),
                        shape: Some(model.shapes[nidx].node.clone()),
                        max_depth: opts.max_depth,
                    };
                    let sub = derive_inner(
                        data,
                        shapes,
                        model,
                        v,
                        &nested_opts,
                        registry,
                        depth + 1,
                        visiting,
                    );
                    visiting.remove(&(prop_idx, v.clone()));
                    Some(Box::new(sub))
                }
                _ => None,
            };
            FormValue {
                term: TermRef::from_term(v),
                nested,
            }
        })
        .collect();

    // ---- dash presentation flags (additive; absent statements leave the ----
    // ---- description exactly as before). [FABLE] sq-lsp7k.1.5           ----
    let hidden = bool_object(shapes, &prop.node, &dash("hidden"));
    let read_only = bool_object(shapes, &prop.node, &dash("readOnly"));
    let default_value = shapes
        .object(&prop.node, &sh("defaultValue"))
        .map(|t| TermRef::from_term(&t));

    let label = field_label(shapes, prop, path);
    FormField {
        property_shape: Some(TermRef::from_term(&prop.node)),
        path: render_path(path),
        inverse: matches!(path, Path::Inverse(_)),
        label,
        description: literal_object(shapes, &prop.node, &sh("description")).or_else(|| {
            predicate_term(path).and_then(|p| literal_object(shapes, &p, RDFS_COMMENT))
        }),
        order: order_of(shapes, &prop.node),
        required: constraints.min_count.is_some_and(|c| c >= 1),
        multi: constraints.max_count != Some(1),
        // dash:readOnly true forces read-only even in edit mode. [FABLE]
        editable: opts.mode == Mode::Edit && !read_only,
        hidden,
        default_value,
        widget,
        values,
        constraints,
        validation: Vec::new(),
    }
}

/// Read-only fields for focus-node predicates no declared field covers.
fn other_fields(
    data: &GraphView,
    shapes: &GraphView,
    focus: &Term,
    registry: &WidgetRegistry,
    covered: &HashSet<String>,
) -> Vec<FormField> {
    let mut predicates: Vec<String> = Vec::new();
    let mut values_of: std::collections::HashMap<String, Vec<Term>> =
        std::collections::HashMap::new();
    for (p, o) in data.predicate_objects(focus) {
        let Term::NamedNode(p) = &p else { continue };
        let p = p.as_str().to_string();
        if covered.contains(&p) {
            continue;
        }
        if !values_of.contains_key(&p) {
            predicates.push(p.clone());
        }
        values_of.entry(p).or_default().push(o);
    }
    predicates.sort_unstable();
    predicates
        .into_iter()
        .map(|p| {
            let mut vals = values_of.remove(&p).unwrap_or_default();
            vals.sort_by_key(|t| t.to_string());
            let constraints = Constraints::default();
            let ctx = WidgetContext {
                value: vals.first(),
                constraints: &constraints,
            };
            let viewer_res = registry.resolve_viewer(&ctx);
            let pred_term = Term::from(oxrdf::NamedNode::new_unchecked(p.clone()));
            let label = if p == RDF_TYPE {
                "Type".to_string()
            } else {
                literal_object(shapes, &pred_term, RDFS_LABEL)
                    .unwrap_or_else(|| local_name(&p).to_string())
            };
            FormField {
                property_shape: None,
                path: format!("<{p}>"),
                inverse: false,
                label,
                description: None,
                order: None,
                required: false,
                multi: true,
                editable: false, // off-shape triples are ALWAYS read-only
                hidden: false,      // presentation flags live on property shapes
                default_value: None, // [FABLE] sq-lsp7k.1.5
                widget: WidgetChoice {
                    editor: None,
                    viewer: viewer_res.selected.clone(),
                    explicit: false,
                    score: viewer_res.score,
                    editor_alternatives: Vec::new(),
                    viewer_alternatives: alternatives(&viewer_res, &None),
                },
                values: vals
                    .iter()
                    .map(|v| FormValue {
                        term: TermRef::from_term(v),
                        nested: None,
                    })
                    .collect(),
                constraints,
                validation: Vec::new(),
            }
        })
        .collect()
}

/// Flattens a property shape's components into renderer-facing [`Constraints`].
/// `or_depth` bounds `sh:or` recursion (branches referencing further unions).
fn constraints_of(
    shapes: &GraphView,
    model: &ShapesModel,
    prop: &Shape,
    or_depth: usize,
) -> Constraints {
    let mut c = Constraints::default();
    for comp in &prop.components {
        match comp {
            Component::Class(t) => c.class.push(TermRef::from_term(t)),
            Component::ClassIn(ts) => c.class.extend(ts.iter().map(TermRef::from_term)),
            Component::Datatype(dts) => c.datatype.extend(dts.iter().cloned()),
            Component::NodeKind(ks) => c.node_kind.extend(ks.iter().cloned()),
            Component::MinCount(n) => c.min_count = Some(*n),
            Component::MaxCount(n) => c.max_count = Some(*n),
            Component::MinInclusive(t) => c.min_inclusive = Some(TermRef::from_term(t)),
            Component::MaxInclusive(t) => c.max_inclusive = Some(TermRef::from_term(t)),
            Component::MinExclusive(t) => c.min_exclusive = Some(TermRef::from_term(t)),
            Component::MaxExclusive(t) => c.max_exclusive = Some(TermRef::from_term(t)),
            Component::MinLength(n) => c.min_length = Some(*n),
            Component::MaxLength(n) => c.max_length = Some(*n),
            Component::Pattern { source, flags } => {
                c.pattern = Some(source.clone());
                c.pattern_flags = flags.clone();
            }
            Component::LanguageIn(ls) => c.language_in.extend(ls.iter().cloned()),
            Component::UniqueLang => c.unique_lang = true,
            Component::SingleLine => c.single_line = Some(true),
            Component::RootClass(t) => c.root_class = Some(TermRef::from_term(t)),
            Component::In(ts) => c.in_values.extend(ts.iter().map(TermRef::from_term)),
            Component::Node(i) => {
                c.node_shape = Some(TermRef::from_term(&model.shapes[*i].node));
            }
            Component::Or(branches) if or_depth > 0 => {
                for &b in branches {
                    c.or
                        .push(constraints_of(shapes, model, &model.shapes[b], or_depth - 1));
                }
            }
            _ => {} // validation-only components carry no form semantics (F3)
        }
    }
    // dash:singleLine on the property shape (sh:singleLine already mapped above).
    if let Some(v) = literal_object(shapes, &prop.node, &dash("singleLine")) {
        c.single_line = Some(v == "true");
    }
    // dash:rootClass (the SHACL 1.2 sh:rootClass component already mapped above).
    if c.root_class.is_none() {
        if let Some(t) = shapes.object(&prop.node, &dash("rootClass")) {
            c.root_class = Some(TermRef::from_term(&t));
        }
    }
    c
}

// ---- small lookups ----------------------------------------------------------

fn alternatives(res: &crate::widgets::Resolution, explicit: &Option<String>) -> Vec<String> {
    match explicit {
        // Under an explicit declaration the auto-selected widget itself becomes
        // an alternative (unless it IS the declared one).
        Some(e) => res
            .selected
            .iter()
            .chain(res.alternatives.iter())
            .filter(|iri| *iri != e)
            .cloned()
            .collect(),
        None => res.alternatives.clone(),
    }
}

/// `sh:name` (smallest by language-then-value for determinism), else
/// `rdfs:label` of the shape/group node.
fn shape_label(shapes: &GraphView, node: &Term) -> Option<String> {
    pick_literal(shapes.objects(node, &sh("name")))
        .or_else(|| pick_literal(shapes.objects(node, RDFS_LABEL)))
}

fn field_label(shapes: &GraphView, prop: &Shape, path: &Path) -> String {
    if let Some(l) = shape_label(shapes, &prop.node) {
        return l;
    }
    if let Some(p) = predicate_term(path) {
        if let Some(l) = pick_literal(shapes.objects(&p, RDFS_LABEL)) {
            return l;
        }
        if let Term::NamedNode(n) = &p {
            return local_name(n.as_str()).to_string();
        }
    }
    render_path(path)
}

/// [GPT-5.6] Canonical field/result key for a parsed SHACL property path.
pub(crate) fn render_path(path: &Path) -> String {
    path.to_sparql_property_path()
        .unwrap_or_else(|| path.to_turtle())
}

/// The predicate the label/comment lookups key on: `<p>` or `^<p>`.
fn predicate_term(path: &Path) -> Option<Term> {
    match path {
        Path::Predicate(p) => Some(Term::from(oxrdf::NamedNode::new_unchecked(p.clone()))),
        Path::Inverse(inner) => predicate_term(inner),
        _ => None,
    }
}

/// Deterministic literal pick: no-language first, then by tag, then by value.
pub(crate) fn pick_literal(terms: Vec<Term>) -> Option<String> {
    let mut lits: Vec<(bool, String, String)> = terms
        .into_iter()
        .filter_map(|t| match t {
            Term::Literal(l) => Some((
                l.language().is_some(),
                l.language().unwrap_or_default().to_string(),
                l.value().to_string(),
            )),
            _ => None,
        })
        .collect();
    lits.sort();
    lits.into_iter().next().map(|(_, _, v)| v)
}

fn literal_object(g: &GraphView, node: &Term, pred: &str) -> Option<String> {
    pick_literal(g.objects(node, pred))
}

/// `true` iff the node declares `<pred> true` (lexical `true`, matching the
/// `dash:singleLine` handling in `constraints_of`). [FABLE-5] sq-lsp7k.1.5
fn bool_object(g: &GraphView, node: &Term, pred: &str) -> bool {
    literal_object(g, node, pred).is_some_and(|v| v == "true")
}

fn iri_object(g: &GraphView, node: &Term, pred: &str) -> Option<String> {
    g.objects(node, pred).into_iter().find_map(|t| match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        _ => None,
    })
}

/// `sh:order` as an f64 (xsd:decimal / xsd:integer lexical forms).
fn order_of(g: &GraphView, node: &Term) -> Option<f64> {
    literal_object(g, node, &sh("order")).and_then(|v| v.parse::<f64>().ok())
}

fn order_key(o: Option<f64>) -> f64 {
    o.unwrap_or(f64::INFINITY)
}

fn sort_fields(fields: &mut [FormField]) {
    fields.sort_by(|a, b| {
        order_key(a.order)
            .total_cmp(&order_key(b.order))
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/']).next().unwrap_or(iri)
}

fn term_of_ref(r: &TermRef) -> Term {
    match r.kind.as_str() {
        "iri" => Term::from(oxrdf::NamedNode::new_unchecked(r.value.clone())),
        _ => Term::from(oxrdf::BlankNode::new_unchecked(r.value.clone())),
    }
}

fn term_ref_is(t: &Term, r: &TermRef) -> bool {
    TermRef::from_term(t) == *r
}
