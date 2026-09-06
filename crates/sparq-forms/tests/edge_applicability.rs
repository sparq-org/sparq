//! Edge-case floors for form defaults and shape applicability.
//! [GPT-5.6] sq-bif.34

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_forms::{applicable_shapes, derive_form, FormOptions, GroupKind, Mode, ShapeVia};
use sparq_shacl::ShapesModel;

const PREFIXES: &str = r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix ex: <http://example.org/> .
"#;

fn g(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{ttl}"), "turtle").unwrap()
}

fn iri(local: &str) -> Term {
    Term::from(NamedNode::new_unchecked(format!(
        "http://example.org/{local}"
    )))
}

#[test]
fn form_options_default_contract() {
    assert_eq!(
        FormOptions::default(),
        FormOptions {
            mode: Mode::Edit,
            role: None,
            shape: None,
            max_depth: 3,
        }
    );
}

#[test]
fn target_class_requires_a_matching_focus_type() {
    let shapes = g("ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person .");
    let model = ShapesModel::parse(&shapes);
    let data = g(r#"
          ex:alice ex:name "Alice" .
          ex:bob a ex:Person .
        "#);

    let choices = applicable_shapes(&data, &shapes, &model, &iri("alice"));
    assert!(choices.is_empty());

    let matching_choices = applicable_shapes(&data, &shapes, &model, &iri("bob"));
    assert_eq!(matching_choices.len(), 1);

    let form = derive_form(&data, &shapes, &iri("alice"), &FormOptions::default());
    assert_eq!(form.focus.kind, "iri");
    assert_eq!(form.focus.value, "http://example.org/alice");
    assert_eq!(form.mode, Mode::Edit);
    assert!(form.role.is_none());
    assert!(form.shapes.is_empty());
    assert!(form.shape.is_none());

    assert_eq!(form.groups.len(), 1);
    let other = &form.groups[0];
    assert_eq!(other.kind, GroupKind::Other);
    assert!(other.group.is_none());
    assert_eq!(other.label.as_deref(), Some("Other properties"));
    assert!(other.order.is_none());
    assert_eq!(other.fields.len(), 1);
    assert_eq!(other.fields[0].path, "<http://example.org/name>");
}

// ---- [OPUS-4.8] sq-vfcxv: predicate-target applicability -------------------

/// `sh:targetSubjectsOf` applies only to nodes that actually carry the
/// predicate, and reports itself as the switcher rationale.
#[test]
fn target_subjects_of_applies_to_subjects_only() {
    let shapes = g("ex:KnowerShape a sh:NodeShape ; sh:targetSubjectsOf ex:knows .");
    let model = ShapesModel::parse(&shapes);
    let data = g(r#"
          ex:alice ex:knows ex:bob .
          ex:carol ex:name "Carol" .
        "#);

    let choices = applicable_shapes(&data, &shapes, &model, &iri("alice"));
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].via, ShapeVia::TargetSubjectsOf);
    assert_eq!(choices[0].shape.value, "http://example.org/KnowerShape");

    // ex:bob is only an OBJECT of ex:knows, ex:carol carries it not at all.
    assert!(applicable_shapes(&data, &shapes, &model, &iri("bob")).is_empty());
    assert!(applicable_shapes(&data, &shapes, &model, &iri("carol")).is_empty());
}

/// `sh:targetObjectsOf` is the mirror image — objects only.
#[test]
fn target_objects_of_applies_to_objects_only() {
    let shapes = g("ex:KnownShape a sh:NodeShape ; sh:targetObjectsOf ex:knows .");
    let model = ShapesModel::parse(&shapes);
    let data = g("ex:alice ex:knows ex:bob .");

    let choices = applicable_shapes(&data, &shapes, &model, &iri("bob"));
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].via, ShapeVia::TargetObjectsOf);

    assert!(applicable_shapes(&data, &shapes, &model, &iri("alice")).is_empty());
}

/// Switcher rank: sh:targetNode > sh:targetClass > dash:applicableToClass >
/// sh:targetSubjectsOf > sh:targetObjectsOf. The FIRST entry is the shape the
/// derivation selects, so the ordering is behaviour, not cosmetics.
#[test]
fn predicate_targets_rank_below_applicable_to_class() {
    let shapes = g(r#"
          @prefix dash: <http://datashapes.org/dash#> .
          ex:NodeShape_ a sh:NodeShape ; sh:targetNode ex:alice .
          ex:ClassShape a sh:NodeShape ; sh:targetClass ex:Person .
          ex:DashShape a sh:NodeShape ; dash:applicableToClass ex:Person .
          ex:SubjShape a sh:NodeShape ; sh:targetSubjectsOf ex:knows .
          ex:ObjShape a sh:NodeShape ; sh:targetObjectsOf ex:knows .
        "#);
    let model = ShapesModel::parse(&shapes);
    let data = g(r#"
          ex:alice a ex:Person ; ex:knows ex:bob .
          ex:carol ex:knows ex:alice .
        "#);

    let choices = applicable_shapes(&data, &shapes, &model, &iri("alice"));
    let order: Vec<ShapeVia> = choices.iter().map(|c| c.via).collect();
    assert_eq!(
        order,
        vec![
            ShapeVia::TargetNode,
            ShapeVia::TargetClass,
            ShapeVia::ApplicableToClass,
            ShapeVia::TargetSubjectsOf,
            ShapeVia::TargetObjectsOf,
        ]
    );

    // The strongest rationale is the shape the form derives against.
    let form = derive_form(&data, &shapes, &iri("alice"), &FormOptions::default());
    assert_eq!(
        form.shape.as_ref().map(|s| s.value.as_str()),
        Some("http://example.org/NodeShape_")
    );
}

/// One shape carrying SEVERAL matching targets reports the strongest, and
/// dash:applicableToClass still wins over a predicate target on the same shape.
#[test]
fn one_shape_reports_its_strongest_rationale() {
    let shapes = g(r#"
          @prefix dash: <http://datashapes.org/dash#> .
          ex:BothPredicates a sh:NodeShape ;
            sh:targetObjectsOf ex:knows ; sh:targetSubjectsOf ex:knows .
          ex:DashAndPredicate a sh:NodeShape ;
            sh:targetObjectsOf ex:knows ; dash:applicableToClass ex:Person .
        "#);
    let model = ShapesModel::parse(&shapes);
    let data = g(r#"
          ex:alice a ex:Person ; ex:knows ex:bob .
          ex:carol ex:knows ex:alice .
        "#);

    let choices = applicable_shapes(&data, &shapes, &model, &iri("alice"));
    let via_of = |local: &str| {
        choices
            .iter()
            .find(|c| c.shape.value == format!("http://example.org/{}", local))
            .map(|c| c.via)
    };
    assert_eq!(via_of("BothPredicates"), Some(ShapeVia::TargetSubjectsOf));
    assert_eq!(via_of("DashAndPredicate"), Some(ShapeVia::ApplicableToClass));
}
