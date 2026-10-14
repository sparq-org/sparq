//! sq-lsp7k.1.1 — sparq-forms derivation micro-bench (registered in
//! bench/benchmarks.toml as `forms-derive-overhead`, featured = false).
//!
//! Measures the cost of `derive_form_with_model` over a SYNTHETIC in-process
//! shapes+data graph (no external dataset, no network): a person-record shape
//! with groups/orders/enums/nested sh:node, derived for N focus nodes. The
//! committed facts are the DETERMINISTIC structural counts (shapes, fields,
//! nested forms); the per-derivation µs are advisory + box-sensitive +
//! NON-CANONICAL and must never be baked into docs or gates.
//!
//! Usage: cargo run -p sparq-forms --example derive_form_bench --release [-- --json <path>]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_forms::{derive_form_with_model, FormOptions, WidgetRegistry};
use sparq_shacl::ShapesModel;
use std::fmt::Write as _;

const FOCI: usize = 200;

fn main() {
    let shapes_ttl = r#"
      @prefix sh: <http://www.w3.org/ns/shacl#> .
      @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
      @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
      @prefix ex: <http://example.org/> .
      ex:MainGroup a sh:PropertyGroup ; rdfs:label "Main" ; sh:order 0 .
      ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
        sh:property [ sh:path ex:name ; sh:name "Name" ; sh:order 1 ; sh:group ex:MainGroup ;
                      sh:datatype xsd:string ; sh:minCount 1 ; sh:maxCount 1 ] ;
        sh:property [ sh:path ex:status ; sh:name "Status" ; sh:order 2 ; sh:group ex:MainGroup ;
                      sh:in ( "new" "active" "retired" ) ; sh:maxCount 1 ] ;
        sh:property [ sh:path ex:employer ; sh:name "Employer" ; sh:order 3 ;
                      sh:class ex:Org ; sh:nodeKind sh:IRI ] ;
        sh:property [ sh:path ex:address ; sh:name "Address" ; sh:order 4 ;
                      sh:node ex:AddressShape ; sh:maxCount 1 ] .
      ex:AddressShape a sh:NodeShape ;
        sh:property [ sh:path ex:city ; sh:name "City" ; sh:datatype xsd:string ] .
    "#;
    let mut data_ttl = String::from("@prefix ex: <http://example.org/> .\n");
    for i in 0..FOCI {
        let _ = writeln!(
            data_ttl,
            "ex:p{i} a ex:Person ; ex:name \"Person {i}\" ; ex:status \"active\" ; \
             ex:employer ex:org{} ; ex:address ex:addr{i} ; ex:extra \"off-shape {i}\" .\n\
             ex:addr{i} ex:city \"City {i}\" .",
            i % 7
        );
    }
    let shapes = Graph::load_str(shapes_ttl, "turtle").expect("shapes parse");
    let data = Graph::load_str(&data_ttl, "turtle").expect("data parse");
    let model = ShapesModel::parse(&shapes);
    let registry = WidgetRegistry::dash();
    let opts = FormOptions::default();

    let start = std::time::Instant::now();
    let mut fields = 0usize;
    let mut nested = 0usize;
    let mut switcher_entries = 0usize;
    for i in 0..FOCI {
        let focus = Term::from(NamedNode::new_unchecked(format!("http://example.org/p{i}")));
        let form = derive_form_with_model(&data, &shapes, &model, &focus, &opts, &registry);
        switcher_entries += form.shapes.len();
        for g in &form.groups {
            fields += g.fields.len();
            nested += g
                .fields
                .iter()
                .flat_map(|f| &f.values)
                .filter(|v| v.nested.is_some())
                .count();
        }
    }
    let elapsed = start.elapsed();
    let per_form_us = elapsed.as_micros() as f64 / FOCI as f64;

    // Deterministic structural facts (the assertable part).
    println!("forms-derive-overhead: foci={FOCI} switcher_entries={switcher_entries} fields={fields} nested_forms={nested}");
    assert_eq!(switcher_entries, FOCI, "one applicable shape per focus");
    assert_eq!(nested, FOCI, "one nested address sub-form per focus");
    // Advisory, box-sensitive, NON-CANONICAL.
    println!("advisory: total={elapsed:?} per_form_us={per_form_us:.1}");

    if let Some(pos) = std::env::args().position(|a| a == "--json") {
        if let Some(path) = std::env::args().nth(pos + 1) {
            let json = format!(
                "{{\"foci\":{FOCI},\"switcher_entries\":{switcher_entries},\"fields\":{fields},\"nested_forms\":{nested}}}\n"
            );
            std::fs::write(path, json).expect("write --json");
        }
    }
}
