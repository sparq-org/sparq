//! Direct tests for the public derivation API: `derive_form`,
//! `derive_form_with_model`, `applicable_shapes`, `FormOptions`.
//! sq-lsp7k.1.1

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_forms::{
    applicable_shapes, derive_form, derive_form_with_model, FormOptions, GroupKind, Mode,
    ShapeVia, Suitability, WidgetContext, WidgetRegistry, DASH,
};
use sparq_shacl::ShapesModel;

const PREFIXES: &str = r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
  @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
  @prefix dash: <http://datashapes.org/dash#> .
  @prefix ex: <http://example.org/> .
"#;

fn g(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{ttl}"), "turtle").unwrap()
}

fn iri(s: &str) -> Term {
    Term::from(NamedNode::new_unchecked(format!("http://example.org/{s}")))
}

const PERSON_SHAPES: &str = r#"
  ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:name "Name" ; sh:order 1 ;
                  sh:datatype xsd:string ; sh:minCount 1 ; sh:maxCount 1 ] ;
    sh:property [ sh:path ex:nick ; sh:name "Nickname" ; sh:order 2 ;
                  sh:datatype xsd:string ] .
"#;

const ALICE: &str = r#"
  ex:alice a ex:Person ; ex:name "Alice" ; ex:nick "Al" , "Ali" .
"#;

#[test]
fn derive_form_returns_fields_widgets_and_cardinality() {
    let form = derive_form(&g(ALICE), &g(PERSON_SHAPES), &iri("alice"), &FormOptions::default());
    assert_eq!(form.focus.value, "http://example.org/alice");
    assert_eq!(form.mode, Mode::Edit);
    assert_eq!(form.shape.as_ref().unwrap().value, "http://example.org/PersonShape");
    assert_eq!(form.shapes.len(), 1);
    assert_eq!(form.shapes[0].via, ShapeVia::TargetClass);

    // Default group first (both fields ungrouped), Other group holds rdf:type.
    assert_eq!(form.groups[0].kind, GroupKind::Default);
    let fields = &form.groups[0].fields;
    assert_eq!(fields.len(), 2);
    let name = &fields[0];
    assert_eq!(name.label, "Name");
    assert_eq!(name.order, Some(1.0));
    assert!(name.required, "sh:minCount 1 marks required");
    assert!(!name.multi, "sh:maxCount 1 removes the add affordance");
    assert!(name.editable);
    assert_eq!(name.widget.editor.as_deref(), Some(&*format!("{DASH}TextFieldEditor")));
    assert_eq!(name.widget.score, Some(10));
    assert_eq!(name.values.len(), 1);
    assert_eq!(name.values[0].term.value, "Alice");
    assert_eq!(name.constraints.min_count, Some(1));
    assert_eq!(name.constraints.datatype, vec!["http://www.w3.org/2001/XMLSchema#string"]);

    let nick = &fields[1];
    assert_eq!(nick.label, "Nickname");
    assert!(nick.multi && !nick.required);
    assert_eq!(nick.values.len(), 2);

    // rdf:type is off-shape → read-only field in the implicit Other group.
    let other = form.groups.last().unwrap();
    assert_eq!(other.kind, GroupKind::Other);
    assert_eq!(other.label.as_deref(), Some("Other properties"));
    let ty = &other.fields[0];
    assert_eq!(ty.label, "Type");
    assert!(!ty.editable);
    assert!(ty.widget.editor.is_none());
    assert_eq!(ty.widget.viewer.as_deref(), Some(&*format!("{DASH}LabelViewer")));
}

#[test]
fn derive_form_view_mode_has_no_editors() {
    let opts = FormOptions {
        mode: Mode::View,
        ..FormOptions::default()
    };
    let form = derive_form(&g(ALICE), &g(PERSON_SHAPES), &iri("alice"), &opts);
    let field = &form.groups[0].fields[0];
    assert!(field.widget.editor.is_none());
    assert!(field.widget.editor_alternatives.is_empty());
    assert!(!field.editable);
    assert_eq!(field.widget.viewer.as_deref(), Some(&*format!("{DASH}LiteralViewer")));
}

#[test]
fn derive_form_with_model_uses_custom_registry() {
    let data = g(ALICE);
    let shapes = g(PERSON_SHAPES);
    let model = ShapesModel::parse(&shapes);
    let mut registry = WidgetRegistry::dash();
    fn always_90(_: &WidgetContext) -> Suitability {
        Suitability::Score(90)
    }
    registry.register_editor("http://example.org/MegaEditor", always_90);
    let form = derive_form_with_model(
        &data,
        &shapes,
        &model,
        &iri("alice"),
        &FormOptions::default(),
        &registry,
    );
    let field = &form.groups[0].fields[0];
    assert_eq!(field.widget.editor.as_deref(), Some("http://example.org/MegaEditor"));
    assert_eq!(field.widget.score, Some(90));
    // The DASH pick demoted to first alternative.
    assert_eq!(field.widget.editor_alternatives[0], format!("{DASH}TextFieldEditor"));
}

#[test]
fn applicable_shapes_covers_target_kinds_and_subclass_closure() {
    // ex:Student ⊑ ex:Person in the DATA graph (SHACL instance semantics);
    // one shape by class target, one by dash:applicableToClass, one by node.
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person .
      ex:AuditShape a sh:NodeShape ; dash:applicableToClass ex:Student .
      ex:BobShape a sh:NodeShape ; sh:targetNode ex:bob .
      ex:OffShape a sh:NodeShape ; sh:targetClass ex:Unrelated .
      ex:SleepyShape a sh:NodeShape ; sh:targetClass ex:Person ; sh:deactivated true .
    "#;
    let data_ttl = r#"
      ex:Student rdfs:subClassOf ex:Person .
      ex:bob a ex:Student ; ex:name "Bob" .
    "#;
    let data = g(data_ttl);
    let shapes = g(shapes_ttl);
    let model = ShapesModel::parse(&shapes);
    let choices = applicable_shapes(&data, &shapes, &model, &iri("bob"));
    let summary: Vec<(&str, ShapeVia)> = choices
        .iter()
        .map(|c| (c.shape.value.as_str(), c.via))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("http://example.org/BobShape", ShapeVia::TargetNode),
            ("http://example.org/PersonShape", ShapeVia::TargetClass),
            ("http://example.org/AuditShape", ShapeVia::ApplicableToClass),
        ],
        "strongest rationale first; OffShape must not apply; deactivated SleepyShape \
         suppressed; subclass closure honoured"
    );
}

#[test]
fn explicit_shape_option_wins_and_is_marked() {
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:name ] .
      ex:UntargetedShape a sh:NodeShape ;
        sh:property [ sh:path ex:nick ; sh:name "Nick only" ] .
    "#;
    let opts = FormOptions {
        shape: Some(iri("UntargetedShape")),
        ..FormOptions::default()
    };
    let form = derive_form(&g(ALICE), &g(shapes_ttl), &iri("alice"), &opts);
    assert_eq!(form.shape.as_ref().unwrap().value, "http://example.org/UntargetedShape");
    assert_eq!(form.shapes[0].via, ShapeVia::Explicit);
    assert_eq!(form.groups[0].fields[0].label, "Nick only");
}

#[test]
fn inverse_path_yields_incoming_reference_field() {
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path [ sh:inversePath ex:knows ] ; sh:name "Known by" ] .
    "#;
    let data_ttl = r#"
      ex:alice a ex:Person .
      ex:bob ex:knows ex:alice .
      ex:carol ex:knows ex:alice .
    "#;
    let form = derive_form(&g(data_ttl), &g(shapes_ttl), &iri("alice"), &FormOptions::default());
    let field = &form.groups[0].fields[0];
    assert!(field.inverse);
    assert_eq!(field.path, "^(<http://example.org/knows>)");
    let mut incoming: Vec<&str> = field.values.iter().map(|v| v.term.value.as_str()).collect();
    incoming.sort_unstable();
    assert_eq!(incoming, vec!["http://example.org/bob", "http://example.org/carol"]);
}

#[test]
fn explicit_dash_editor_overrides_scoring() {
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:name ; sh:datatype xsd:string ;
                      dash:editor dash:TextAreaEditor ] .
    "#;
    let form = derive_form(&g(ALICE), &g(shapes_ttl), &iri("alice"), &FormOptions::default());
    let field = &form.groups[0].fields[0];
    assert!(field.widget.explicit);
    assert_eq!(field.widget.editor.as_deref(), Some(&*format!("{DASH}TextAreaEditor")));
    assert!(field.widget.score.is_none(), "explicit declarations carry no score");
    // The auto pick (TextFieldEditor) is offered as an alternative instead.
    assert!(field
        .widget
        .editor_alternatives
        .contains(&format!("{DASH}TextFieldEditor")));
}

#[test]
fn fractional_order_and_groups_sort_fields_and_groups() {
    let shapes_ttl = r#"
      ex:MainGroup a sh:PropertyGroup ; rdfs:label "Main" ; sh:order 0 .
      ex:ExtraGroup a sh:PropertyGroup ; rdfs:label "Extra" ; sh:order 1 .
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:b ; sh:name "B" ; sh:order 2 ; sh:group ex:MainGroup ] ;
        sh:property [ sh:path ex:mid ; sh:name "Mid" ; sh:order 1.5 ; sh:group ex:MainGroup ] ;
        sh:property [ sh:path ex:a ; sh:name "A" ; sh:order 1 ; sh:group ex:MainGroup ] ;
        sh:property [ sh:path ex:x ; sh:name "X" ; sh:group ex:ExtraGroup ] ;
        sh:property [ sh:path ex:loose ; sh:name "Loose" ] .
    "#;
    let form = derive_form(&g(ALICE), &g(shapes_ttl), &iri("alice"), &FormOptions::default());
    // Ungrouped fields render first, then groups by sh:order.
    assert_eq!(form.groups[0].kind, GroupKind::Default);
    assert_eq!(form.groups[0].fields[0].label, "Loose");
    assert_eq!(form.groups[1].label.as_deref(), Some("Main"));
    assert_eq!(form.groups[1].order, Some(0.0));
    let labels: Vec<&str> = form.groups[1].fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, vec!["A", "Mid", "B"], "fractional sh:order 1.5 sorts between 1 and 2");
    assert_eq!(form.groups[2].label.as_deref(), Some("Extra"));
}

#[test]
fn deactivated_property_shape_is_suppressed() {
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:name ; sh:name "Name" ] ;
        sh:property [ sh:path ex:secret ; sh:name "Secret" ; sh:deactivated true ] .
    "#;
    let form = derive_form(&g(ALICE), &g(shapes_ttl), &iri("alice"), &FormOptions::default());
    let labels: Vec<&str> = form
        .groups
        .iter()
        .flat_map(|g| g.fields.iter().map(|f| f.label.as_str()))
        .collect();
    assert!(labels.contains(&"Name"));
    assert!(!labels.contains(&"Secret"));
}

#[test]
fn nested_sh_node_recurses_and_cycles_terminate() {
    let shapes_ttl = r#"
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:friend ; sh:name "Friend" ; sh:node ex:PersonShape ] .
    "#;
    // alice → bob → alice: a data cycle over the same recursive shape.
    let data_ttl = r#"
      ex:alice a ex:Person ; ex:friend ex:bob .
      ex:bob a ex:Person ; ex:friend ex:alice .
    "#;
    let form = derive_form(&g(data_ttl), &g(shapes_ttl), &iri("alice"), &FormOptions::default());
    let friend = &form.groups[0].fields[0];
    assert_eq!(friend.widget.editor.as_deref(), Some(&*format!("{DASH}DetailsEditor")));
    let nested = friend.values[0].nested.as_ref().expect("nested sub-form for bob");
    assert_eq!(nested.focus.value, "http://example.org/bob");
    // bob's nested form points back at alice — recursion terminated (either by
    // the visiting guard or max_depth), so SOME ancestor stops nesting.
    let mut depth = 0;
    let mut cur = Some(nested.as_ref());
    while let Some(f) = cur {
        depth += 1;
        assert!(depth <= 4, "unbounded sh:node recursion");
        cur = f
            .groups
            .first()
            .and_then(|g| g.fields.first())
            .and_then(|fl| fl.values.first())
            .and_then(|v| v.nested.as_deref());
    }
}

#[test]
fn form_options_default_values() {
    let opts = FormOptions::default();
    assert_eq!(opts.mode, Mode::Edit);
    assert!(opts.role.is_none() && opts.shape.is_none());
    assert_eq!(opts.max_depth, 3);
}

// ---- dash presentation flags: hidden / readOnly / defaultValue -------------
// sq-lsp7k.1.5

const FLAGGABLE_SHAPES: &str = r#"
  ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
    sh:property ex:nameProp ; sh:property ex:secretProp ; sh:property ex:codeProp .
  ex:nameProp sh:path ex:name ; sh:name "Name" ; sh:order 1 .
  ex:secretProp sh:path ex:secret ; sh:name "Secret" ; sh:order 2 .
  ex:codeProp sh:path ex:code ; sh:name "Code" ; sh:order 3 .
"#;

const PRESENTATION_FLAGS: &str = r#"
  ex:secretProp dash:hidden true .
  ex:codeProp dash:readOnly true ; sh:defaultValue ex:draft .
  ex:nameProp sh:defaultValue "Anon" .
"#;

#[test]
fn dash_hidden_readonly_and_default_value_surface_on_fields() {
    let shapes = g(&format!("{FLAGGABLE_SHAPES}{PRESENTATION_FLAGS}"));
    let form = derive_form(&g(ALICE), &shapes, &iri("alice"), &FormOptions::default());
    assert_eq!(form.mode, Mode::Edit);
    let fields = &form.groups[0].fields;
    let by = |label: &str| fields.iter().find(|f| f.label == label).unwrap();

    // dash:hidden true → flagged; the field still derives (the RENDERER omits it).
    let secret = by("Secret");
    assert!(secret.hidden);
    assert!(secret.editable, "dash:hidden does not imply read-only");

    // dash:readOnly true → editable=false even though the form is in Edit mode.
    let code = by("Code");
    assert!(!code.editable, "dash:readOnly true forces read-only in Mode::Edit");
    assert!(!code.hidden);

    // sh:defaultValue terms carried verbatim (literal and IRI kinds).
    let name = by("Name");
    assert!(!name.hidden && name.editable);
    let dv = name.default_value.as_ref().expect("literal default");
    assert_eq!((dv.kind.as_str(), dv.value.as_str()), ("literal", "Anon"));
    let dv = code.default_value.as_ref().expect("IRI default");
    assert_eq!(
        (dv.kind.as_str(), dv.value.as_str()),
        ("iri", "http://example.org/draft")
    );
    assert!(secret.default_value.is_none());
}

#[test]
fn presentation_flags_are_additive_stripping_them_recovers_plain() {
    let data = g(ALICE);
    let opts = FormOptions::default();
    let plain = derive_form(&data, &g(FLAGGABLE_SHAPES), &iri("alice"), &opts);

    // Shapes WITHOUT the statements serialize with none of the new keys
    // (goldens separately pin the flag-free JSON byte-for-byte).
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("\"hidden\"") && !json.contains("\"default_value\""));

    // Resetting the additive flags must recover the exact plain description:
    // presentation flags cannot perturb widgets, constraints, values, or layout.
    let flagged_shapes = g(&format!("{FLAGGABLE_SHAPES}{PRESENTATION_FLAGS}"));
    let mut stripped = derive_form(&data, &flagged_shapes, &iri("alice"), &opts);
    for field in stripped.groups.iter_mut().flat_map(|group| &mut group.fields) {
        field.hidden = false;
        field.default_value = None;
        if field.label == "Code" {
            field.editable = true; // undo dash:readOnly
        }
    }
    assert_eq!(stripped, plain);
}

#[test]
fn role_is_echoed_and_focus_without_shapes_is_all_other() {
    let opts = FormOptions {
        role: Some("expert".into()),
        ..FormOptions::default()
    };
    // No shape applies: everything lands read-only in the Other group.
    let form = derive_form(&g(ALICE), &g(""), &iri("alice"), &opts);
    assert_eq!(form.role.as_deref(), Some("expert"));
    assert!(form.shape.is_none() && form.shapes.is_empty());
    assert_eq!(form.groups.len(), 1);
    assert_eq!(form.groups[0].kind, GroupKind::Other);
    assert!(form.groups[0].fields.iter().all(|f| !f.editable));
}
