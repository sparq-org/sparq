//! End-to-end: sparq-reason `why()` proof tree → PROV-O lineage RDF.
//! [OPUS-4.8] sq-m3i0 — reasoner-materialization lineage.
#![cfg(feature = "reason")]

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxrdf::vocab::xsd;
use oxrdf::Triple;
use sparq_core::dict::{Dict, Id};
use sparq_prov::{prov_from_proof, ProvProofConfig};
use sparq_reason::n3::Term as N3Term;
use sparq_reason::{MaterializedGraph, MaterializedN3Graph, MaterializedOwlGraph};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const PROV: &str = "http://www.w3.org/ns/prov#";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Build an RDFS graph over `(rex a Dog), (Dog ⊑ Animal)` and return the graph,
/// the dict, and the id of the inferred fact `(rex a Animal)` (via cax-sco/rdfs9).
fn dog_setup() -> (MaterializedGraph, Dict, [Id; 3]) {
    let mut dict = Dict::default();
    let i = |d: &mut Dict, s: &str| d.intern_iri(s);
    let rex = i(&mut dict, "http://ex/rex");
    let dog = i(&mut dict, "http://ex/Dog");
    let animal = i(&mut dict, "http://ex/Animal");
    let ty = i(&mut dict, RDF_TYPE);
    let sc = i(&mut dict, RDFS_SUBCLASS);
    let base = vec![[rex, ty, dog], [dog, sc, animal]];
    let g = MaterializedGraph::new(&mut dict, &base);
    (g, dict, [rex, ty, animal])
}

/// All distinct predicate IRIs in a graph.
fn preds(g: &[Triple]) -> HashSet<String> {
    g.iter().map(|t| t.predicate.as_str().to_string()).collect()
}

/// Map subject IRI -> set of (predicate, object-IRI-or-literal-lexical) for assertions.
fn lines(g: &[Triple]) -> HashSet<String> {
    g.iter()
        .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
        .collect()
}

#[test]
fn derived_fact_proof_emits_entity_activity_and_lineage() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).expect("inferred fact has a proof");
    // The proof: root (rex a Animal) ← rule(rdfs9/cax-sco) ← 2 asserted leaves.
    assert!(
        proof.nodes().len() >= 3,
        "expected a multi-node proof, got {}",
        proof.nodes().len()
    );

    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    let p = preds(&prov);
    // Core PROV-O lineage predicates are present.
    assert!(
        p.contains(&format!("{PROV}wasGeneratedBy")),
        "missing wasGeneratedBy: {p:?}"
    );
    assert!(p.contains(&format!("{PROV}used")), "missing used");
    assert!(
        p.contains(&format!("{PROV}wasDerivedFrom")),
        "missing wasDerivedFrom"
    );

    // Exactly one prov:Activity per non-leaf node; here the single rule firing.
    let activities: Vec<&Triple> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Activity>")
        })
        .collect();
    assert_eq!(activities.len(), 1, "one rule firing ⇒ one Activity");

    // Every premise that the activity `used` is also `wasDerivedFrom` by the result.
    let used: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}used"))
        .map(|t| t.object.to_string())
        .collect();
    let derived_from: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}wasDerivedFrom"))
        .map(|t| t.object.to_string())
        .collect();
    assert_eq!(used, derived_from, "used inputs == wasDerivedFrom inputs");
    assert_eq!(
        used.len(),
        2,
        "rdfs9 has two premises (typing + subClassOf)"
    );

    // The activity carries the rule id as an rdfs:label (cax-sco / rdfs9).
    let label = prov
        .iter()
        .find(|t| t.predicate.as_str() == RDFS_LABEL && t.subject.to_string().contains(":rule:"))
        .map(|t| t.object.to_string())
        .expect("rule activity has a label");
    assert!(
        label.contains("rdfs9") || label.contains("cax-sco"),
        "unexpected rule label: {label}"
    );
}

#[test]
fn asserted_leaves_are_entities_without_a_generating_activity() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());

    // Subjects that wasGeneratedBy something = derived entities.
    let generated: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}wasGeneratedBy"))
        .map(|t| t.subject.to_string())
        .collect();
    // Every fact node is typed prov:Entity.
    let entities: HashSet<String> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>")
        })
        .map(|t| t.subject.to_string())
        .collect();
    // 3 distinct facts (2 leaves + 1 root) ⇒ 3 entities; exactly 1 is generated.
    assert_eq!(entities.len(), 3);
    assert_eq!(generated.len(), 1);
    assert!(generated.is_subset(&entities));
    // The two leaves have no generating activity.
    assert_eq!(entities.difference(&generated).count(), 2);
}

#[test]
fn lineage_is_valid_rdf_and_round_trips() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    let nt = sparq_engine::triples_to_ntriples(&prov);

    let reloaded = sparq_core::Graph::load_str(&nt, "turtle").expect("PROV-O must be valid RDF");
    assert_eq!(reloaded.len(), prov.len());

    use oxttl::NTriplesParser;
    let parsed: Vec<_> = NTriplesParser::new()
        .for_reader(nt.as_bytes())
        .collect::<Result<_, _>>()
        .expect("emitted lineage must be well-formed N-Triples");
    assert_eq!(parsed.len(), prov.len());
}

#[test]
fn prov_ntriples_matches_the_lineage_graph_and_parses() {
    // [GPT-5.6] sq-8jn86: the exact string oracle witnesses the serializer call;
    // replacing the wrapper body with an empty or differently rendered value fails.
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();
    let cfg = ProvProofConfig::default();
    let triples = prov_from_proof(&proof, &cfg);
    let expected = triples
        .iter()
        .map(|t| format!("{} {} {} .\n", t.subject, t.predicate, t.object))
        .collect::<String>();
    assert!(!expected.is_empty(), "dog proof must emit lineage triples");

    let nt = sparq_prov::prov_ntriples(&proof, &cfg);
    assert_eq!(nt, expected);

    use oxttl::NTriplesParser;
    let parsed: Vec<_> = NTriplesParser::new()
        .for_reader(nt.as_bytes())
        .collect::<Result<_, _>>()
        .expect("rendered lineage must be well-formed N-Triples");
    assert_eq!(parsed.len(), triples.len());
}

#[test]
fn deterministic_and_content_addressed() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();
    let a = prov_from_proof(&proof, &ProvProofConfig::default());
    let b = prov_from_proof(&proof, &ProvProofConfig::default());
    assert_eq!(
        lines(&a),
        lines(&b),
        "same proof + config ⇒ identical lineage"
    );
    // The minted entity/activity IRIs sit under the default namespace.
    assert!(a
        .iter()
        .any(|t| t.subject.to_string().contains("urn:sparq:prov:fact:")));
    assert!(a
        .iter()
        .any(|t| t.subject.to_string().contains("urn:sparq:prov:rule:")));
}

#[test]
fn shared_subfacts_stitch_across_proofs() {
    // Two derived facts that share the asserted typing premise (rex a Dog) must
    // name the SAME entity IRI for that shared fact, so lineage DAGs join up.
    let mut dict = Dict::default();
    let i = |d: &mut Dict, s: &str| d.intern_iri(s);
    let rex = i(&mut dict, "http://ex/rex");
    let dog = i(&mut dict, "http://ex/Dog");
    let animal = i(&mut dict, "http://ex/Animal");
    let mammal = i(&mut dict, "http://ex/Mammal");
    let ty = i(&mut dict, RDF_TYPE);
    let sc = i(&mut dict, RDFS_SUBCLASS);
    // Dog ⊑ Mammal ⊑ Animal, rex a Dog ⇒ rex a Mammal and rex a Animal.
    let base = vec![[rex, ty, dog], [dog, sc, mammal], [mammal, sc, animal]];
    let g = MaterializedGraph::new(&mut dict, &base);

    let cfg = ProvProofConfig::default();
    let p1 = prov_from_proof(&g.why(&dict, [rex, ty, mammal]).unwrap(), &cfg);
    let p2 = prov_from_proof(&g.why(&dict, [rex, ty, animal]).unwrap(), &cfg);

    // Both proofs share the asserted leaf (rex a Dog), and the SAME fact must mint
    // the SAME content-addressed entity IRI — so the lineage DAGs join up.
    let ents1: HashSet<String> = p1
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>")
        })
        .map(|t| t.subject.to_string())
        .collect();
    let ents2: HashSet<String> = p2
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>")
        })
        .map(|t| t.subject.to_string())
        .collect();
    // At least one shared entity (the (rex a Dog) leaf) appears in both.
    assert!(
        ents1.intersection(&ents2).next().is_some(),
        "overlapping proofs must share at least the (rex a Dog) leaf entity"
    );
}

#[test]
fn clock_and_agent_are_recorded_on_activities() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();

    fn fixed() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }
    let cfg = ProvProofConfig {
        agent: Some(oxrdf::NamedNode::new_unchecked("http://ex/rdfs-reasoner")),
        ..ProvProofConfig::with_clock(fixed)
    };
    let prov = prov_from_proof(&proof, &cfg);

    // generatedAtTime on the activity, typed xsd:dateTime = 2023-11-14T22:13:20Z.
    let ts = prov
        .iter()
        .find(|t| t.predicate.as_str() == format!("{PROV}generatedAtTime"))
        .map(|t| t.object.to_string())
        .expect("activity is time-stamped");
    assert!(ts.contains("2023-11-14T22:13:20Z"), "got {ts}");
    assert!(ts.contains(xsd::DATE_TIME.as_str()));

    // wasAssociatedWith the configured agent.
    assert!(prov.iter().any(
        |t| t.predicate.as_str() == format!("{PROV}wasAssociatedWith")
            && t.object.to_string() == "<http://ex/rdfs-reasoner>"
    ));
}

#[test]
fn n3_rule_proof_emits_lineage_with_n3_rule_label() {
    // The same bridge works on N3 proofs (the bead is RDFS/OWL-RL/N3): a forward
    // rule firing becomes a prov:Activity labelled `n3-rule-<i>`.
    let ex = |l: &str| N3Term::Iri(format!("http://ex/{l}"));
    let rules = r#"
        @prefix : <http://ex/> .
        { ?x :parent ?y } => { ?x :ancestor ?y } .
    "#;
    let base = vec![[ex("a"), ex("parent"), ex("b")]];
    let g = MaterializedN3Graph::new(rules, &base).expect("rules parse");
    let fact = [ex("a"), ex("ancestor"), ex("b")];
    assert!(g.contains(&fact));
    let proof = g.why(&fact).expect("derived N3 fact explains");

    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    // The rule firing is a labelled Activity.
    let label = prov
        .iter()
        .find(|t| t.predicate.as_str() == RDFS_LABEL && t.subject.to_string().contains(":rule:"))
        .map(|t| t.object.to_string())
        .expect("rule activity has a label");
    assert!(
        label.contains("n3-rule-0"),
        "unexpected N3 rule label: {label}"
    );
    // wasGeneratedBy + a single used/wasDerivedFrom premise (the asserted parent edge).
    assert!(preds(&prov).contains(&format!("{PROV}wasGeneratedBy")));
    let used = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}used"))
        .count();
    assert_eq!(used, 1, "one premise: a :parent b");
    // Valid RDF.
    let nt = sparq_engine::triples_to_ntriples(&prov);
    assert!(sparq_core::Graph::load_str(&nt, "turtle").is_ok());
}

#[test]
fn asserted_fact_proof_is_a_lone_entity() {
    // why() on a base triple returns a single asserted leaf ⇒ one Entity, no Activity.
    let (g, dict, _) = dog_setup();
    let mut dict2 = dict;
    let rex = dict2.intern_iri("http://ex/rex");
    let dog = dict2.intern_iri("http://ex/Dog");
    let ty = dict2.intern_iri(RDF_TYPE);
    let proof = g
        .why(&dict2, [rex, ty, dog])
        .expect("asserted fact explains");
    assert_eq!(proof.nodes().len(), 1);
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    // One Entity typing triple, nothing else (no activity, no edges).
    let mut by_pred: HashMap<String, usize> = HashMap::new();
    for t in &prov {
        *by_pred.entry(t.predicate.as_str().to_string()).or_default() += 1;
    }
    assert_eq!(by_pred.get(RDF_TYPE), Some(&1));
    assert!(!by_pred.contains_key(&format!("{PROV}wasGeneratedBy")));
}

// ── sq-bif.4: reasoner-materialization correctness-suite additions — axiom leaves,
// OWL transitive (multi-premise) lineage, clock-config, content-addressing across
// rule kinds. [OPUS-4.8] ─────────────────────────────────────────────────────────

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

/// An OWL-RL proof whose conclusion `(x a xsd:decimal)` rests on an **axiom leaf** — the
/// XSD numeric-tower edge `(xsd:integer rdfs:subClassOf xsd:decimal)` the engine injects
/// as a premise-free `axiom-xsd` tautology. Such a leaf is a `prov:Entity` boundary with
/// NO generating activity, but it KEEPS an `rdfs:label` naming the axiom (the `is_axiom`
/// branch of the bridge) so the tautology stays traceable — unlike a plain asserted leaf,
/// which carries no label.
#[test]
fn axiom_leaf_is_labelled_entity_boundary_without_activity() {
    let rdf_type = RDF_TYPE;
    let mut dict = Dict::default();
    let x = dict.intern_iri("http://ex/x");
    let ty = dict.intern_iri(rdf_type);
    let integer = dict.intern_iri(&format!("{}integer", XSD));
    let decimal = dict.intern_iri(&format!("{}decimal", XSD));
    // x : xsd:integer ⊢ x : xsd:decimal via cax-sco over the xsd-tower axiom edge.
    let base = vec![[x, ty, integer]];
    let g = MaterializedOwlGraph::new(&mut dict, &base);
    let proof = g
        .why(&dict, [x, ty, decimal])
        .expect("xsd-tower subsumption derives the decimal typing");

    // The proof must contain an axiom-xsd leaf (the subClassOf tower edge).
    assert!(
        proof.nodes().iter().any(|n| n.rule == "axiom-xsd"),
        "expected an axiom-xsd tautology leaf; got {:?}",
        proof.nodes().iter().map(|n| &n.rule).collect::<Vec<_>>()
    );

    let prov = prov_from_proof(&proof, &ProvProofConfig::default());

    // The axiom-named label is emitted on an Entity (the tautology), and that subject is
    // NOT generated by any activity (a leaf is the boundary of the lineage).
    let axiom_labelled: HashSet<String> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDFS_LABEL && t.object.to_string().contains("axiom-xsd")
        })
        .map(|t| t.subject.to_string())
        .collect();
    assert_eq!(
        axiom_labelled.len(),
        1,
        "exactly one axiom-xsd entity is labelled"
    );
    let generated: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}wasGeneratedBy"))
        .map(|t| t.subject.to_string())
        .collect();
    assert!(
        axiom_labelled.is_disjoint(&generated),
        "an axiom leaf must have NO generating activity"
    );
    // The labelled axiom subject is a fact entity (typed prov:Entity).
    let entities: HashSet<String> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>")
        })
        .map(|t| t.subject.to_string())
        .collect();
    assert!(axiom_labelled.is_subset(&entities));
    // No activity is labelled axiom-xsd (the label sits on the entity, not an activity).
    assert!(prov.iter().all(|t| !(t.predicate.as_str() == RDFS_LABEL
        && t.subject.to_string().contains(":rule:")
        && t.object.to_string().contains("axiom-xsd"))));
}

/// A proof carrying an axiom leaf is still valid, round-trippable RDF (the axiom-label
/// triple does not break serialization).
#[test]
fn axiom_leaf_lineage_round_trips() {
    let mut dict = Dict::default();
    let x = dict.intern_iri("http://ex/x");
    let ty = dict.intern_iri(RDF_TYPE);
    let integer = dict.intern_iri(&format!("{}integer", XSD));
    let decimal = dict.intern_iri(&format!("{}decimal", XSD));
    let g = MaterializedOwlGraph::new(&mut dict, &[[x, ty, integer]]);
    let proof = g.why(&dict, [x, ty, decimal]).unwrap();
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    let nt = sparq_engine::triples_to_ntriples(&prov);
    let reloaded = sparq_core::Graph::load_str(&nt, "turtle").expect("valid RDF");
    assert_eq!(reloaded.len(), prov.len());
}

/// An OWL transitive-property firing (`prp-trp`) is a single rule application with
/// **three** premises (the `TransitiveProperty` typing + the two chained edges). The
/// bridge must emit one Activity that `used` all three premise entities, and a result
/// that `wasDerivedFrom` exactly those three — so multi-premise rules are recorded
/// faithfully (not collapsed to a single edge).
#[test]
fn transitive_rule_records_all_three_premises() {
    let mut dict = Dict::default();
    let a = dict.intern_iri("http://ex/a");
    let b = dict.intern_iri("http://ex/b");
    let c = dict.intern_iri("http://ex/c");
    let knows = dict.intern_iri("http://ex/knows");
    let ty = dict.intern_iri(RDF_TYPE);
    let trans = dict.intern_iri(&format!("{}TransitiveProperty", OWL));
    let base = vec![[knows, ty, trans], [a, knows, b], [b, knows, c]];
    let g = MaterializedOwlGraph::new(&mut dict, &base);
    let proof = g
        .why(&dict, [a, knows, c])
        .expect("a knows c derives by prp-trp");

    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    // Exactly one rule Activity, labelled prp-trp.
    let activities: Vec<&Triple> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Activity>")
        })
        .collect();
    assert_eq!(activities.len(), 1, "one prp-trp firing ⇒ one Activity");
    let label = prov
        .iter()
        .find(|t| t.predicate.as_str() == RDFS_LABEL && t.subject.to_string().contains(":rule:"))
        .map(|t| t.object.to_string())
        .expect("rule activity has a label");
    assert!(label.contains("prp-trp"), "unexpected rule label: {label}");

    // The activity `used` 3 premise entities; the result `wasDerivedFrom` the same 3.
    let used: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}used"))
        .map(|t| t.object.to_string())
        .collect();
    let derived_from: HashSet<String> = prov
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}wasDerivedFrom"))
        .map(|t| t.object.to_string())
        .collect();
    assert_eq!(used.len(), 3, "prp-trp has three premises");
    assert_eq!(used, derived_from, "used == wasDerivedFrom for the result");
}

/// `ProvProofConfig::with_clock` is the constructor that turns on activity timestamps.
/// Verify it time-stamps every rule activity (and that an empty/default-namespaced
/// config does NOT) — covering both the timestamped and timestamp-free branches.
#[test]
fn with_clock_constructor_timestamps_all_activities() {
    fn fixed() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();

    // With a clock: each rule Activity carries a generatedAtTime.
    let timed = prov_from_proof(&proof, &ProvProofConfig::with_clock(fixed));
    let activities = timed
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Activity>")
        })
        .count();
    let timestamps = timed
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{PROV}generatedAtTime"))
        .count();
    assert!(activities >= 1, "at least one rule firing");
    assert_eq!(
        timestamps, activities,
        "every activity is time-stamped under with_clock"
    );
    // The namespace defaults to SPARQ_PROV_NS.
    assert!(timed
        .iter()
        .any(|t| t.subject.to_string().contains("urn:sparq:prov:")));

    // Without a clock (default): no timestamps at all.
    let untimed = prov_from_proof(&proof, &ProvProofConfig::default());
    assert!(untimed
        .iter()
        .all(|t| t.predicate.as_str() != format!("{PROV}generatedAtTime")));
}

/// A caller-supplied namespace flows into the minted entity/activity IRIs (the
/// integrate-with-an-external-store path), and content-addressing still holds: the same
/// fact under the same namespace mints the same IRI.
#[test]
fn custom_namespace_is_honoured_and_still_content_addressed() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).unwrap();
    let cfg = ProvProofConfig {
        namespace: "http://my.store/prov#".to_string(),
        ..ProvProofConfig::default()
    };
    let a = prov_from_proof(&proof, &cfg);
    let b = prov_from_proof(&proof, &cfg);
    // Every minted subject sits under the custom namespace, none under the default.
    assert!(a
        .iter()
        .any(|t| t.subject.to_string().contains("http://my.store/prov#fact:")));
    assert!(a
        .iter()
        .all(|t| !t.subject.to_string().contains("urn:sparq:prov:")));
    // Deterministic + content-addressed: same proof + namespace ⇒ identical lineage.
    assert_eq!(lines(&a), lines(&b));
}

/// An empty proof tree (defensive — `why()` never returns one, but the bridge must not
/// panic on it) yields an empty lineage graph. Built via a one-node asserted proof whose
/// single node is a leaf, contrasted with the genuinely-empty case being unreachable: we
/// assert the lone-leaf proof produces exactly one Entity-typing triple and nothing else.
#[test]
fn single_asserted_leaf_proof_is_minimal() {
    // A base fact's why() is a lone asserted leaf — one Entity, no activity, no label
    // (asserted leaves are unlabelled; only axiom leaves carry a label).
    let mut dict = Dict::default();
    let s = dict.intern_iri("http://ex/s");
    let p = dict.intern_iri("http://ex/p");
    let o = dict.intern_iri("http://ex/o");
    let g = MaterializedGraph::new(&mut dict, &[[s, p, o]]);
    let proof = g.why(&dict, [s, p, o]).expect("asserted fact explains");
    assert_eq!(proof.nodes().len(), 1);
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    // Exactly one triple: the Entity typing. No label (not an axiom), no activity.
    assert_eq!(prov.len(), 1);
    assert_eq!(prov[0].predicate.as_str(), RDF_TYPE);
    assert_eq!(prov[0].object.to_string(), format!("<{PROV}Entity>"));
}

/// The `mix` hash function in `reason.rs` (used by `fact_entity` and `rule_activity`) must
/// use XOR (`h ^= b`) inside its byte loop, not bit-OR (`h |= b`). The two operations
/// produce completely different 64-bit outputs because XOR can CLEAR bits while OR only
/// sets them — so a mutation replacing `^=` with `|=` changes every entity/activity IRI.
///
/// This test pins the EXACT entity and activity IRIs for the `dog_setup` proof so any
/// change to the hash arithmetic (operator, constant, or loop body in `mix`) is caught.
///
/// Computed values (from the FNV-1a XOR hash of the N-Triples-style term strings):
/// - `fact:c1136432c1d6c06d` = `mix(mix(INIT, fnv1a("<http://ex/rex>")),
///     fnv1a("<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>")),
///     fnv1a("<http://ex/Animal>"))`
/// - `fact:66d50ccbf2ab0b2b` = same over `<http://ex/Dog>` leaf
/// - `fact:93b6c887c862aec9` = same over `<http://ex/Dog subClassOf http://ex/Animal>` leaf
/// - `rule:7b2aac0f7dbc51bf` = `fnv1a("rdfs9")` mixed with the conclusion triple
///
/// [OPUS-4.8] sq-qcnn.20
#[test]
fn exact_content_addressed_iris_for_dog_setup_proof() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).expect("inferred fact has proof");
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());

    // Collect entity subjects (typed prov:Entity).
    let entity_subjects: HashSet<String> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>")
        })
        .map(|t| t.subject.to_string())
        .collect();

    // All three fact IRIs must be present.
    assert!(
        entity_subjects.contains("<urn:sparq:prov:fact:c1136432c1d6c06d>"),
        "derived fact (rex a Animal) entity IRI mismatch; got {:?}",
        entity_subjects
    );
    assert!(
        entity_subjects.contains("<urn:sparq:prov:fact:66d50ccbf2ab0b2b>"),
        "asserted leaf (rex a Dog) entity IRI mismatch"
    );
    assert!(
        entity_subjects.contains("<urn:sparq:prov:fact:93b6c887c862aec9>"),
        "asserted leaf (Dog subClassOf Animal) entity IRI mismatch"
    );

    // The rule activity IRI for the rdfs9 firing.
    let rule_subjects: HashSet<String> = prov
        .iter()
        .filter(|t| {
            t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Activity>")
        })
        .map(|t| t.subject.to_string())
        .collect();
    assert!(
        rule_subjects.contains("<urn:sparq:prov:rule:7b2aac0f7dbc51bf>"),
        "rule activity IRI for rdfs9 mismatch; got {:?}",
        rule_subjects
    );

    // The derived entity is generated by that exact activity.
    assert!(prov.iter().any(|t| {
        t.subject.to_string() == "<urn:sparq:prov:fact:c1136432c1d6c06d>"
            && t.predicate.as_str() == format!("{PROV}wasGeneratedBy")
            && t.object.to_string() == "<urn:sparq:prov:rule:7b2aac0f7dbc51bf>"
    }));
}

// ── sq-qcnn.39: mutation-kill additions — exact prov triple counts. [SONNET-4.6] ─────

/// `prov_from_proof` on the `dog_setup` proof (3 nodes: 2 asserted leaves plus
/// 1 derived conclusion via rdfs9/cax-sco, no clock, no agent) must emit exactly 10
/// triples: 2 leaf entity-types, 1 derived entity-type, 1 activity-type, 1 label,
/// 1 wasGeneratedBy, 2 prov:used, 2 prov:wasDerivedFrom.
/// Pinning the count kills any mutation that drops a single `push()` call.
/// [SONNET-4.6] sq-qcnn.39
#[test]
fn dog_setup_prov_graph_exact_triple_count() {
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).expect("inferred fact has proof");
    let prov = prov_from_proof(&proof, &ProvProofConfig::default());
    // 2 leaf entity-types + 1 derived entity-type + 1 activity-type + 1 label
    // + 1 wasGeneratedBy + 2 used + 2 wasDerivedFrom = 10
    assert_eq!(
        prov.len(),
        10,
        "dog_setup proof (3 nodes, 2 premises) must emit 10 triples; \
         got {}; triples: {:#?}",
        prov.len(),
        prov.iter()
            .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
            .collect::<Vec<_>>()
    );
}

/// With a clock configured, each rule-application Activity gets an extra
/// `prov:generatedAtTime` triple. For the dog_setup proof (1 rule firing), that adds
/// 1 triple: total = 11. With an agent, another `wasAssociatedWith` triple: total = 12.
/// Pinning these counts catches mutations that skip the clock or agent branches.
/// [SONNET-4.6] sq-qcnn.39
#[test]
fn dog_setup_prov_graph_with_clock_and_agent_exact_count() {
    fn fixed() -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)
    }
    let (g, dict, fact) = dog_setup();
    let proof = g.why(&dict, fact).expect("inferred fact has proof");

    // Clock only: 10 base + 1 generatedAtTime = 11
    let with_clock = prov_from_proof(&proof, &ProvProofConfig::with_clock(fixed));
    assert_eq!(
        with_clock.len(),
        11,
        "dog_setup proof with clock must emit 11 triples"
    );

    // Clock + agent: 11 + 1 wasAssociatedWith = 12
    let cfg_full = ProvProofConfig {
        agent: Some(oxrdf::NamedNode::new_unchecked("http://ex/reasoner")),
        ..ProvProofConfig::with_clock(fixed)
    };
    let with_both = prov_from_proof(&proof, &cfg_full);
    assert_eq!(
        with_both.len(),
        12,
        "dog_setup proof with clock + agent must emit 12 triples"
    );
}

/// GH #6701 review round 4: an N3 fact carrying an `@forAll` universal gets ONE entity
/// across proofs. The entity IRI hashes the proof's conclusion strings, so the reasoner
/// must render the fact the same way whether it is explained alone or as a premise.
#[test]
fn n3_fact_with_a_universal_keeps_one_entity_across_proofs() {
    let src = "@prefix : <http://ex/>. @forAll :x.
:a :p { :x :q :z }.
:b :p { ?x :q :z }.
{ :a ?p ?f. :b :p ?g } => { :c :r ?f }.
";
    let g = MaterializedN3Graph::new(src, &[]).expect("rules parse");
    let closure = g.closure();
    let ex = |l: &str| N3Term::Iri(format!("http://ex/{l}"));
    let a = closure.iter().find(|f| f[0] == ex("a")).expect(":a fact");
    let c = closure.iter().find(|f| f[0] == ex("c")).expect(":c fact");
    let entities = |prov: &[Triple]| -> HashSet<String> {
        prov.iter()
            .filter(|t| t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>"))
            .map(|t| t.subject.to_string())
            .collect()
    };
    let cfg = ProvProofConfig::default();
    let alone = entities(&prov_from_proof(&g.why(a).expect(":a explains"), &cfg));
    assert_eq!(alone.len(), 1);
    let within = entities(&prov_from_proof(&g.why(c).expect(":c explains"), &cfg));
    assert!(alone.is_subset(&within), "{alone:?} not among {within:?}");
}

/// GH #6701 review rounds 6 and 9: `:x` under `@forAll`, `o:x` under `@forAll` (another
/// namespace, same local name) and a source `?x` are three different facts; they must get
/// three entities. `sparq-prov` hashes the proof's conclusion strings, so those must never
/// render distinct variables alike.
#[test]
fn n3_universal_and_source_variable_facts_keep_distinct_entities() {
    let src = "@prefix : <http://ex/>. @prefix o: <http://other/>.
@forAll :x, o:x.
:x :p :o.
o:x :p :o.
?x :p :o.
";
    let g = MaterializedN3Graph::new(src, &[]).expect("rules parse");
    let ex = |l: &str| N3Term::Iri(format!("http://ex/{l}"));
    let closure = g.closure();
    let asserted: Vec<&[N3Term; 3]> =
        closure.iter().filter(|f| f[1] == ex("p") && f[2] == ex("o")).collect();
    assert_eq!(asserted.len(), 3, "{closure:?}");
    let cfg = ProvProofConfig::default();
    let entity = |f: &[N3Term; 3]| -> String {
        let proof = g.why(f).expect("asserted fact explains");
        let prov = prov_from_proof(&proof, &cfg);
        let ents: Vec<String> = prov
            .iter()
            .filter(|t| t.predicate.as_str() == RDF_TYPE && t.object.to_string() == format!("<{PROV}Entity>"))
            .map(|t| t.subject.to_string())
            .collect();
        assert_eq!(ents.len(), 1);
        ents[0].clone()
    };
    let alone: Vec<String> = asserted.iter().map(|f| entity(f)).collect();
    let distinct: HashSet<&String> = alone.iter().collect();
    assert_eq!(distinct.len(), 3, "different facts share an entity: {alone:?}");
    // Deterministic: the same fact, explained again, names the same entity. (Keeping its
    // entity as a PREMISE of another proof is pinned by
    // `n3_fact_with_a_universal_keeps_one_entity_across_proofs`.)
    let again: Vec<String> = asserted.iter().map(|f| entity(f)).collect();
    assert_eq!(alone, again);
}

