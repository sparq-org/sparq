#![cfg(feature = "schema-diff")]

use sparq_core::Graph;
use sparq_introspect::{DiffEntry, Introspection, SchemaDiff};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PERSON: &str = "http://xmlns.com/foaf/0.1/Person";
const COMPANY: &str = "http://xmlns.com/foaf/0.1/Company";
const NAME: &str = "http://xmlns.com/foaf/0.1/name";
const AGE: &str = "http://xmlns.com/foaf/0.1/age";

fn introspect(data: &str) -> Introspection {
    Introspection::build(&Graph::load_str(data, "ntriples").unwrap())
}

#[test]
fn reports_exact_schema_drift_and_json() {
    // Mutation witness for sq-yjemp: deleting either comparison path,
    // swapping base/new, or using profile-vector order breaks this exact oracle.
    let base = introspect(&format!(
        "<http://example.com/alice> <{RDF_TYPE}> <{PERSON}> .\n\
         <http://example.com/alice> <{NAME}> \"Alice\" .\n"
    ));
    let new = introspect(&format!(
        "<http://example.com/alice> <{RDF_TYPE}> <{PERSON}> .\n\
         <http://example.com/alice> <{NAME}> \"Alice\" .\n\
         <http://example.com/bob> <{RDF_TYPE}> <{PERSON}> .\n\
         <http://example.com/bob> <{NAME}> \"Bob\" .\n\
         <http://example.com/bob> <{AGE}> \"42\" .\n\
         <http://example.com/acme> <{RDF_TYPE}> <{COMPANY}> .\n"
    ));

    let diff = base.diff(&new);
    assert_eq!(
        diff,
        SchemaDiff {
            triples_base: 2,
            triples_new: 6,
            added_classes: vec![DiffEntry {
                iri: COMPANY.into(),
                base: 0,
                new: 1
            }],
            removed_classes: vec![],
            changed_classes: vec![DiffEntry {
                iri: PERSON.into(),
                base: 1,
                new: 2
            }],
            added_predicates: vec![DiffEntry {
                iri: AGE.into(),
                base: 0,
                new: 1
            }],
            removed_predicates: vec![],
            changed_predicates: vec![
                DiffEntry {
                    iri: RDF_TYPE.into(),
                    base: 1,
                    new: 3
                },
                DiffEntry {
                    iri: NAME.into(),
                    base: 1,
                    new: 2
                },
            ],
        }
    );

    let json = diff.to_json();
    assert!(json.contains("\"added_classes\""));
    assert_eq!(serde_json::from_str::<SchemaDiff>(&json).unwrap(), diff);
}

#[test]
fn reverse_direction_reports_removals() {
    let base = introspect(&format!(
        "<http://example.com/alice> <{RDF_TYPE}> <{PERSON}> .\n\
         <http://example.com/alice> <{NAME}> \"Alice\" .\n"
    ));
    let empty = introspect("");

    let diff = base.diff(&empty);
    assert_eq!(
        diff.removed_classes,
        vec![DiffEntry {
            iri: PERSON.into(),
            base: 1,
            new: 0
        }]
    );
    assert_eq!(
        diff.removed_predicates,
        vec![
            DiffEntry {
                iri: RDF_TYPE.into(),
                base: 1,
                new: 0
            },
            DiffEntry {
                iri: NAME.into(),
                base: 1,
                new: 0
            },
        ]
    );
}
