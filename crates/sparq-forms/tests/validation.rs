//! Live-validation coverage for `derive_form_validated` (sq-lsp7k.1.3).

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_forms::{derive_form, derive_form_validated, FormOptions, Mode, WidgetRegistry};
use sparq_shacl::ShapesModel;

const PREFIXES: &str = r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix ex: <http://example.org/> .
"#;

fn graph(turtle: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{turtle}"), "turtle").unwrap()
}

fn focus() -> Term {
    Term::from(NamedNode::new_unchecked("http://example.org/alice"))
}

fn field<'a>(form: &'a sparq_forms::FormDescription, label: &str) -> &'a sparq_forms::FormField {
    form.groups
        .iter()
        .flat_map(|group| &group.fields)
        .find(|field| field.label == label)
        .unwrap_or_else(|| panic!("missing field {label}"))
}

#[test]
fn validation_attaches_one_hint_to_the_violating_declared_field() {
    let shapes = graph(
        r#"
        ex:PersonShape a sh:NodeShape ;
          sh:targetNode ex:alice ;
          sh:class ex:Employee ;
          sh:property [ sh:path ex:code ; sh:name "Code" ;
                        sh:pattern "^AB" ;
                        sh:message "Le code doit commencer par AB"@fr,
                                   "Code must start with AB" ] ;
          sh:property [ sh:path ex:name ; sh:name "Name" ; sh:minLength 2 ] .
        "#,
    );
    let data = graph(
        r#"
        ex:alice a ex:Person ;
          ex:code "wrong" ;
          ex:name "Alice" ;
          ex:extra "off shape" .
        "#,
    );
    let model = ShapesModel::parse(&shapes);
    let opts = FormOptions::default();
    let registry = WidgetRegistry::dash();

    let plain = derive_form(&data, &shapes, &focus(), &opts);
    let validated = derive_form_validated(&data, &shapes, &model, &focus(), &opts, &registry);

    let code = field(&validated, "Code");
    assert_eq!(code.validation.len(), 1);
    let hint = &code.validation[0];
    assert_eq!(
        hint.source_component,
        "http://www.w3.org/ns/shacl#PatternConstraintComponent"
    );
    assert_eq!(hint.message, "Code must start with AB");
    assert_eq!(hint.severity, "http://www.w3.org/ns/shacl#Violation");
    assert_eq!(
        hint.value.as_ref().map(|value| value.value.as_str()),
        Some("wrong")
    );

    assert!(field(&validated, "Name").validation.is_empty());
    assert!(field(&validated, "extra").validation.is_empty());

    // Removing the additive hints must recover the exact plain description:
    // validation cannot perturb widgets, constraints, values, or layout.
    let mut without_hints = validated;
    for form_field in without_hints
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.fields)
    {
        form_field.validation.clear();
    }
    assert_eq!(without_hints, plain);
}

#[test]
fn validation_does_not_attach_to_read_only_fields() {
    let shapes = graph(
        r#"
        ex:PersonShape a sh:NodeShape ; sh:targetNode ex:alice ;
          sh:property [ sh:path ex:code ; sh:name "Code" ; sh:pattern "^AB" ] .
        "#,
    );
    let data = graph(r#"ex:alice ex:code "wrong" ."#);
    let model = ShapesModel::parse(&shapes);
    let opts = FormOptions {
        mode: Mode::View,
        ..FormOptions::default()
    };

    let validated = derive_form_validated(
        &data,
        &shapes,
        &model,
        &focus(),
        &opts,
        &WidgetRegistry::dash(),
    );

    let code = field(&validated, "Code");
    assert!(!code.editable);
    assert!(code.validation.is_empty());
}

#[test]
fn validation_uses_the_generated_message_when_the_shape_has_none() {
    let shapes = graph(
        r#"
        ex:PersonShape a sh:NodeShape ; sh:targetNode ex:alice ;
          sh:property [ sh:path ex:code ; sh:name "Code" ; sh:pattern "^AB" ] .
        "#,
    );
    let data = graph(r#"ex:alice ex:code "wrong" ."#);
    let model = ShapesModel::parse(&shapes);

    let validated = derive_form_validated(
        &data,
        &shapes,
        &model,
        &focus(),
        &FormOptions::default(),
        &WidgetRegistry::dash(),
    );

    assert_eq!(
        field(&validated, "Code").validation[0].message,
        "Value does not match pattern \"^AB\""
    );
}
