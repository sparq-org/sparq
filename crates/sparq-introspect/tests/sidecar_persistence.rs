// sq-bif.41: integration coverage for the public sidecar naming and
// persistence APIs under the crate's default feature set.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sparq_core::Graph;
use sparq_introspect::{sidecar_path_for, Introspection};

const TINY_TURTLE: &str = r#"
@prefix ex: <http://example.com/> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:knows ex:bob .
ex:bob a ex:Person .
"#;

fn introspection() -> Introspection {
    let graph = Graph::load_str(TINY_TURTLE, "turtle").expect("parse tiny Turtle fixture");
    Introspection::build(&graph)
}

fn unique_sidecar_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sparq-introspect-persistence-{}-{stamp}.introspect",
        std::process::id()
    ))
}

#[test]
fn sidecar_path_appends_extension_to_the_full_dataset_name() {
    assert_eq!(
        sidecar_path_for("data/olympics.nt"),
        PathBuf::from("data/olympics.nt.introspect")
    );
    // Mutation witness: replacing this with `g.introspect` fails because the
    // dataset extension is preserved rather than replaced.
    assert_eq!(sidecar_path_for("g.ttl"), PathBuf::from("g.ttl.introspect"));
}

#[test]
fn json_round_trip_is_byte_exact_and_malformed_json_is_rejected() {
    assert!(
        Introspection::from_json("{ not json").is_err(),
        "malformed JSON must not produce an introspection"
    );

    let original = introspection().to_json();
    let parsed = Introspection::from_json(&original).expect("parse generated introspection JSON");

    assert_eq!(
        parsed.to_json(),
        original,
        "JSON persistence must preserve every serialized field and its canonical ordering"
    );
}

#[test]
fn file_persistence_round_trips_and_missing_sidecars_error() {
    let original = introspection();
    let path = unique_sidecar_path();

    original.save(&path).expect("save introspection sidecar");
    let loaded = Introspection::load(&path).expect("load saved introspection sidecar");
    std::fs::remove_file(&path).expect("remove temporary introspection sidecar");

    assert_eq!(
        loaded.to_json(),
        original.to_json(),
        "file persistence must preserve the complete introspection"
    );

    let error = Introspection::load(&path).expect_err("a nonexistent sidecar must fail to load");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}
