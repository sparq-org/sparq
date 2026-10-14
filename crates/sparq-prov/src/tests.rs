//! Unit tests for the CONSTRUCT → PROV-O lineage path. sq-ntcg.

use super::*;
use oxrdf::vocab::xsd;
use std::collections::HashSet;
use std::time::Duration;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 ; ex:name "Bob" .
"#;

fn g() -> Graph {
    Graph::load_str(DATA, "turtle").unwrap()
}

/// A fixed clock so activity timing (and the minted IRIs) are deterministic.
fn fixed_clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn cfg() -> ProvConfig {
    ProvConfig {
        clock: fixed_clock,
        used: vec![NamedNode::new_unchecked("http://ex/source-graph")],
        ..ProvConfig::default()
    }
}

/// Render a triple as an N-Triples line for substring assertions.
fn line(t: &Triple) -> String {
    format!("{} {} {} .", t.subject, t.predicate, t.object)
}

#[test]
fn construct_derivation_emits_core_prov_shape() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        cfg(),
    )
    .unwrap();

    // The derived data itself: one renamed triple per subject.
    assert_eq!(d.triples().len(), 2);

    let prov = d.prov_graph();
    let lines: HashSet<String> = prov.iter().map(line).collect();
    let a = d.activity().as_str();
    let e = d.entity().as_str();

    // Types.
    assert!(lines.contains(&format!(
        "<{a}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/prov#Activity> ."
    )));
    assert!(lines.contains(&format!(
        "<{e}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/prov#Entity> ."
    )));
    // Generation + derivation edges.
    assert!(lines.contains(&format!(
        "<{e}> <http://www.w3.org/ns/prov#wasGeneratedBy> <{a}> ."
    )));
    assert!(lines.contains(&format!(
        "<{a}> <http://www.w3.org/ns/prov#used> <http://ex/source-graph> ."
    )));
    assert!(lines.contains(&format!(
        "<{e}> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://ex/source-graph> ."
    )));
    // Timing: the fixed clock = 2023-11-14T22:13:20Z, typed xsd:dateTime.
    let dt = format!(
        "<{a}> <http://www.w3.org/ns/prov#startedAtTime> \"2023-11-14T22:13:20Z\"^^<{}> .",
        xsd::DATE_TIME.as_str()
    );
    assert!(lines.contains(&dt), "missing startedAtTime; got {lines:#?}");
    assert!(lines.iter().any(|l| l.contains("#endedAtTime>")));
}

/// sq-cg237: the direct accessor preserves configured order and agrees with
/// every `prov:used` edge materialised for the derivation activity.
#[test]
fn construct_used_inputs_are_exposed_in_order_and_materialised() {
    let source1 = NamedNode::new_unchecked("http://ex/source-1");
    let source2 = NamedNode::new_unchecked("http://ex/source-2");
    let inputs = [source1, source2];
    let config = ProvConfig {
        used: inputs.to_vec(),
        clock: fixed_clock,
        ..ProvConfig::default()
    };
    let derivation =
        derive_construct(&g(), "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", config).unwrap();

    assert_eq!(derivation.used_inputs(), inputs.as_slice());
    assert_eq!(derivation.used_inputs().len(), 2);

    let activity = NamedOrBlankNode::NamedNode(derivation.activity().clone());
    let provenance = derivation.prov_graph();
    for input in &inputs {
        assert!(provenance.iter().any(|triple| {
            triple.subject == activity
                && triple.predicate.as_str() == "http://www.w3.org/ns/prov#used"
                && triple.object == Term::NamedNode(input.clone())
        }));
    }
}

#[test]
fn prov_graph_is_valid_rdf_and_round_trips() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        cfg(),
    )
    .unwrap();

    // N-Triples ⊂ Turtle: the emitted lineage must load back as a graph...
    let nt = d.prov_ntriples();
    let reloaded = Graph::load_str(&nt, "turtle").expect("PROV-O output must be valid RDF");
    assert_eq!(reloaded.len(), d.prov_graph().len());

    // ...and parse cleanly with an independent N-Triples parser too.
    use oxttl::NTriplesParser;
    let parsed: Vec<_> = NTriplesParser::new()
        .for_reader(nt.as_bytes())
        .collect::<Result<_, _>>()
        .expect("emitted lineage must be well-formed N-Triples");
    assert_eq!(parsed.len(), d.prov_graph().len());
}

/// sq-ijw35: the Derivation Turtle method preserves every provenance
/// triple, and the generation edge uses the registered prefix.
#[test]
fn prov_turtle_round_trips_the_exact_derivation_graph() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        cfg(),
    )
    .unwrap();

    let turtle = d.prov_turtle();
    assert!(
        turtle.contains("prov:wasGeneratedBy"),
        "expected the prefix-compacted generation predicate in {turtle}"
    );

    let parsed: Vec<_> = oxttl::TurtleParser::new()
        .for_slice(turtle.as_bytes())
        .collect::<Result<_, _>>()
        .expect("emitted lineage must be well-formed Turtle");
    let expected = d.prov_graph();
    assert_eq!(parsed.len(), expected.len());
    assert_eq!(
        parsed.into_iter().collect::<HashSet<_>>(),
        expected.into_iter().collect::<HashSet<_>>()
    );
}

#[test]
fn explicit_iris_and_agent_are_honoured() {
    let activity = NamedNode::new_unchecked("http://ex/act/1");
    let entity = NamedNode::new_unchecked("http://ex/result/1");
    let agent = NamedNode::new_unchecked("http://ex/service");
    let config = ProvConfig {
        activity: Some(activity.clone()),
        entity: Some(entity.clone()),
        agent: Some(agent.clone()),
        used: vec![NamedNode::new_unchecked("http://ex/g")],
        clock: fixed_clock,
    };
    let d = derive_construct(&g(), "DESCRIBE <http://ex/alice>", config).unwrap();
    assert_eq!(d.activity(), &activity);
    assert_eq!(d.entity(), &entity);

    let lines: HashSet<String> = d.prov_graph().iter().map(line).collect();
    assert!(lines.contains(
        "<http://ex/act/1> <http://www.w3.org/ns/prov#wasAssociatedWith> <http://ex/service> ."
    ));
    // DESCRIBE is a derivation too (CBD of alice = her 2 outbound triples).
    assert_eq!(d.triples().len(), 2);
}

#[test]
fn minted_iris_are_stable_for_the_same_derivation() {
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
    let a = derive_construct(&g(), q, cfg()).unwrap();
    let b = derive_construct(&g(), q, cfg()).unwrap();
    // Same query + same (fixed) clock ⇒ same minted activity/entity IRIs.
    assert_eq!(a.activity(), b.activity());
    assert_eq!(a.entity(), b.entity());
    // A different query mints different nodes.
    let c = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:n ?n } WHERE { ?s ex:name ?n }",
        cfg(),
    )
    .unwrap();
    assert_ne!(a.activity(), c.activity());
}

#[test]
fn empty_inputs_omit_used_and_derived_edges() {
    let config = ProvConfig {
        clock: fixed_clock,
        ..ProvConfig::default()
    };
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        config,
    )
    .unwrap();
    let prov = d.prov_graph();
    assert!(prov
        .iter()
        .all(|t| t.predicate.as_str() != "http://www.w3.org/ns/prov#used"));
    assert!(prov
        .iter()
        .all(|t| t.predicate.as_str() != "http://www.w3.org/ns/prov#wasDerivedFrom"));
    // The core activity/entity/generation triples are still present.
    assert!(prov
        .iter()
        .any(|t| t.predicate.as_str() == "http://www.w3.org/ns/prov#wasGeneratedBy"));
}

#[test]
fn rejects_non_graph_queries() {
    assert!(derive_construct(&g(), "SELECT * WHERE { ?s ?p ?o }", cfg()).is_err());
    assert!(derive_construct(&g(), "ASK { ?s ?p ?o }", cfg()).is_err());
}

// ── sq-bif.4: correctness-suite additions — gaps in the CONSTRUCT/datetime/config
// surface (uncovered branches + edge cases). ──────────────────────────

/// `ProvConfig::with_inputs` is the documented constructor for the common "name the
/// source dataset(s)" case; exercise it directly (not only via the README doc-test) and
/// confirm it populates `used` and leaves the rest at their defaults.
#[test]
fn with_inputs_constructor_sets_used_and_defaults() {
    let inputs = [
        NamedNode::new_unchecked("http://ex/g1"),
        NamedNode::new_unchecked("http://ex/g2"),
    ];
    let config = ProvConfig::with_inputs(inputs.clone());
    assert_eq!(config.used, inputs);
    // The other fields are the `Default` values.
    assert!(config.activity.is_none());
    assert!(config.entity.is_none());
    assert!(config.agent.is_none());

    // …and the inputs flow through to the `used`/`wasDerivedFrom` edges of a real
    // derivation, so the constructor is wired correctly end-to-end.
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        ProvConfig {
            clock: fixed_clock,
            ..ProvConfig::with_inputs(inputs.clone())
        },
    )
    .unwrap();
    let lines: HashSet<String> = d.prov_graph().iter().map(line).collect();
    let a = d.activity().as_str();
    let e = d.entity().as_str();
    for input in &inputs {
        let iri = input.as_str();
        assert!(lines.contains(&format!(
            "<{a}> <http://www.w3.org/ns/prov#used> <{iri}> ."
        )));
        assert!(lines.contains(&format!(
            "<{e}> <http://www.w3.org/ns/prov#wasDerivedFrom> <{iri}> ."
        )));
    }
}

/// `Derivation::timing()` exposes the captured start/end instants; under a fixed clock
/// they equal the configured instant (start == end for a same-clock derivation), and the
/// `startedAtTime` literal must agree with the reported start.
#[test]
fn timing_reports_the_captured_instants() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        cfg(),
    )
    .unwrap();
    let (started, ended) = d.timing();
    // The fixed clock returns the same instant for both reads.
    assert_eq!(started, fixed_clock());
    assert_eq!(ended, fixed_clock());
    assert!(ended >= started, "end must not precede start");
    // The reported start matches the emitted startedAtTime literal.
    let lines: HashSet<String> = d.prov_graph().iter().map(line).collect();
    assert!(lines
        .iter()
        .any(|l| l.contains("#startedAtTime>") && l.contains("2023-11-14T22:13:20Z")));
}

/// A CONSTRUCT whose WHERE matches nothing derives the empty graph — a valid PROV
/// derivation that generated zero triples. The activity/entity + lineage edges still
/// stand (an activity that produced an empty result is still an activity).
#[test]
fn empty_construct_is_still_a_derivation() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:nonexistent ?a }",
        cfg(),
    )
    .unwrap();
    assert!(d.triples().is_empty(), "no match ⇒ empty derived graph");
    // The lineage record is unchanged: the activity ran, the entity (empty result) was
    // generated and derived from the named input.
    let lines: HashSet<String> = d.prov_graph().iter().map(line).collect();
    assert!(lines.iter().any(|l| l.contains("#wasGeneratedBy>")));
    assert!(lines.iter().any(|l| l.contains("#wasDerivedFrom>")));
    assert!(lines.iter().any(|l| l.contains("#Activity>")));
    assert!(lines.iter().any(|l| l.contains("#Entity>")));
}

/// The activity carries the verbatim query text as a `prov:value` recipe and the kind as
/// an `rdfs:label` — the human-facing annotations that make the lineage self-describing.
#[test]
fn activity_records_kind_label_and_query_recipe() {
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
    let d = derive_construct(&g(), q, cfg()).unwrap();
    let lines: HashSet<String> = d.prov_graph().iter().map(line).collect();
    let a = d.activity().as_str();
    // CONSTRUCT kind as rdfs:label.
    assert!(lines.contains(&format!(
        "<{a}> <http://www.w3.org/2000/01/rdf-schema#label> \"CONSTRUCT\" ."
    )));
    // The exact query text as prov:value.
    assert!(lines.contains(&format!(
        "<{a}> <http://www.w3.org/ns/prov#value> \"{q}\" ."
    )));
}

/// Every emitted `startedAtTime`/`endedAtTime` literal is typed `xsd:dateTime` — never an
/// untyped/`xsd:string` literal (a PROV-O conformance point).
#[test]
fn timing_literals_are_typed_xsd_datetime() {
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        cfg(),
    )
    .unwrap();
    for t in d.prov_graph() {
        if t.predicate.as_str().ends_with("AtTime") {
            match &t.object {
                Term::Literal(lit) => assert_eq!(
                    lit.datatype(),
                    xsd::DATE_TIME,
                    "timing literal must be xsd:dateTime"
                ),
                other => panic!("timing object must be a literal, got {:?}", other),
            }
        }
    }
}

/// `datetime_literal` is the bridge from a captured `SystemTime` to the emitted
/// `xsd:dateTime` literal. A pre-epoch instant (before 1970) yields the documented
/// clamp-to-epoch behaviour rather than a panic — a defensive boundary on the clock.
#[test]
fn datetime_literal_clamps_pre_epoch_to_unix_epoch() {
    // SystemTime before UNIX_EPOCH ⇒ duration_since errors ⇒ clamped to epoch (0).
    let pre_epoch = UNIX_EPOCH - Duration::from_secs(10_000);
    let lit = datetime_literal(pre_epoch);
    assert_eq!(lit.value(), "1970-01-01T00:00:00Z");
    assert_eq!(lit.datatype(), xsd::DATE_TIME);
}

/// `datetime_literal` truncates sub-second precision to whole seconds (the lexical is
/// `…SSZ`, no fraction) — so two instants in the same second format identically.
#[test]
fn datetime_literal_truncates_subsecond_precision() {
    let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let with_nanos = base + Duration::from_nanos(999_999_999);
    assert_eq!(
        datetime_literal(base).value(),
        datetime_literal(with_nanos).value(),
        "sub-second precision is truncated to whole seconds"
    );
    assert_eq!(datetime_literal(base).value(), "2023-11-14T22:13:20Z");
}

// ── sq-qcnn.20: mutation-kill tests — assert EXACT provenance CONTENTS, not counts.

/// The FNV-1a hash driving `mint()` must use XOR-then-multiply (`h ^= b`), not bit-OR
/// (`h |= b`). The two operations produce different hash values because XOR can CLEAR bits
/// that were previously set, whereas OR can only set them. A mutation replacing `^=` with
/// `|=` produces a completely different 64-bit hash, yielding a different minted IRI.
/// This test pins the EXACT minted activity/entity IRI for a known (fixed-clock) derivation
/// so any change to the hash constant or operation is immediately caught.
#[test]
fn minted_iri_has_exact_known_value() {
    // Fixed clock: UNIX_EPOCH + 1_700_000_000 s = 2023-11-14T22:13:20Z.
    // Nanos = 1_700_000_000 * 1_000_000_000 = 0x17979cfe362a0000.
    // FNV-1a (offset 0xcbf29ce484222325, prime 0x00000100000001b3) of the query below:
    // → 0xc95c64ae92bd3485.
    // Minted IRI = urn:sparq:prov:{role}:17979cfe362a0000-17979cfe362a0000-c95c64ae92bd3485.
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
    let d = derive_construct(&g(), q, cfg()).unwrap();

    // Exact activity IRI (role = "activity").
    assert_eq!(
        d.activity().as_str(),
        "urn:sparq:prov:activity:17979cfe362a0000-17979cfe362a0000-c95c64ae92bd3485",
        "activity IRI must match the FNV-1a XOR hash of the query + fixed-clock nanos"
    );
    // Exact entity IRI (role = "entity").
    assert_eq!(
        d.entity().as_str(),
        "urn:sparq:prov:entity:17979cfe362a0000-17979cfe362a0000-c95c64ae92bd3485",
        "entity IRI must match the FNV-1a XOR hash of the query + fixed-clock nanos"
    );
    // Confirm the derived triples are the expected two age-renamed triples.
    assert_eq!(d.triples().len(), 2, "CONSTRUCT should produce two triples");
}

// ── sq-qcnn.39: mutation-kill tests — exact prov_graph triple counts for CONSTRUCT.

/// `Derivation::prov_graph()` for a CONSTRUCT with 1 used input and no agent must emit
/// exactly 9 triples: 2 type, 2 timing, rdfs:label, prov:value, wasGeneratedBy, used,
/// wasDerivedFrom. Pinning the count kills mutations that drop any `push()` call.
/// sq-qcnn.39
#[test]
fn construct_prov_graph_exact_triple_count() {
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
    let d = derive_construct(&g(), q, cfg()).unwrap();
    assert_eq!(
        d.prov_graph().len(),
        9,
        "CONSTRUCT prov_graph with 1 input must emit exactly 9 triples; got {:?}",
        d.prov_graph().len()
    );
}

/// With an agent configured, one extra `wasAssociatedWith` triple is added, so the
/// count is 10 (9 + 1). Confirms the agent branch emits exactly one triple.
/// sq-qcnn.39
#[test]
fn construct_prov_graph_with_agent_exact_triple_count() {
    let config = ProvConfig {
        activity: Some(NamedNode::new_unchecked("http://ex/a")),
        entity: Some(NamedNode::new_unchecked("http://ex/e")),
        agent: Some(NamedNode::new_unchecked("http://ex/agent")),
        used: vec![NamedNode::new_unchecked("http://ex/src")],
        clock: fixed_clock,
    };
    let d = derive_construct(
        &g(),
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        config,
    )
    .unwrap();
    // 9 base + 1 wasAssociatedWith = 10
    assert_eq!(
        d.prov_graph().len(),
        10,
        "CONSTRUCT prov_graph with agent must emit 10 triples"
    );
}
