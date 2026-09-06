//! [OPUS-4.8] sq-0wo9e.5 (epic sq-0wo9e) — integration tests for the P4 **flexible grounding**
//! selector + verbaliser over a REAL graph + a REAL `.spqv` store (the modalities that need a
//! built store / schema header cannot be exercised by the in-crate unit tests alone).
//!
//! What is proven here:
//! - the `OutputType → Modality` dispatcher routes a request to the right modality and `ground`
//!   produces the matching object (subgraph / typed sub-vector / NL string / typed value);
//! - the **typed-sub-vector** modality projects ONLY the requested blocks out of a real stored
//!   vector, keyed by the `.spqv` `SchemaHeader` — minimal by construction;
//! - **profile-relative completeness**: a fact entailed only via the closure
//!   (`close_for_vectorise`) appears in the subgraph grounding over the closed graph, and is ABSENT
//!   over the un-closed graph (the honest "complete relative to whatever you closed under" contract);
//! - the subgraph grounding contains only facts present in the graph (never an approximate signal).
//!
//! [OPUS-5] sq-w2af4 adds the **provenance-weighting** half of the design's §USE-1 integration
//! points, both end-to-end through the REAL path (never a mock):
//! - point 3: graph provenance → `SchemaHeader` → `ground`/`ground_weighted` → `fuse_rrf_weighted`.
//!   `weight_header` supplies the graph-global per-block *default*; a `NodeWeighting` handed to
//!   `ground_weighted` instead scales each node's modality by **that node's own incident edges**,
//!   and the test pins that two differently-provenanced nodes then get *different* weights;
//! - point 2: `sketch_predicate` pools a node's multi-valued predicate over the real `.spqv` store,
//!   weighted by **the asserting statement's** `w(t)` — reified statement provenance where the
//!   graph carries it, and an exact arithmetic mean where it does not (the honest no-op), as under
//!   the ablation-off `WeightMode::Uniform`.
//!
//! Gated on `structure` (the grounding module).
#![cfg(feature = "structure")]

use sparq_core::Graph;
use sparq_reason::Profile;
use sparq_vectors::encode::{Block, Encoder, Metric, SchemaHeader};
use sparq_vectors::provenance::{ProvenanceWeights, WeightMode};
use sparq_vectors::structure::close_for_vectorise;
use sparq_vectors::{
    fuse_rrf_weighted, ground, ground_weighted, sketch_predicate, Grounding, GroundingConfig,
    Modality, NodeWeighting, OutputType, VectorStore,
};

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sparq_vec_grounding_{}_{name}.spqv",
        std::process::id()
    ));
    p
}

fn iri(s: &str) -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
}

const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:   <http://ex/> .

ex:SportsCar rdfs:subClassOf ex:Car .
ex:Car       rdfs:subClassOf ex:Vehicle .

ex:bolt a ex:SportsCar ;
        rdfs:label "Bolt" ;
        ex:seats 2 ;
        ex:electric true .
"#;

#[test]
fn typed_sub_vector_projects_only_requested_blocks() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let bolt = g.id_of(&iri("http://ex/bolt")).unwrap();

    // A 6-dim structured row: a 4-dim numeric block + a 2-dim text block.
    let header = SchemaHeader::new(vec![
        Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4),
        Block::new(Encoder::Other, Metric::Euclidean, 4, 2),
    ])
    .unwrap();

    let mut store = VectorStore::create(tmp("subvec"), 6).unwrap();
    store.put(bolt, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
    store.finalize().unwrap();

    // Keep only the numeric block → the first 4 dims.
    let cfg = GroundingConfig {
        keep_blocks: vec![Encoder::Numeric],
        ..Default::default()
    };
    let Some(Grounding::TypedSubVector {
        values,
        blocks,
        weights,
    }) = ground(
        &g,
        &iri("http://ex/bolt"),
        Modality::TypedSubVector,
        &cfg,
        None,
        Some((&store, &header)),
    ) else {
        panic!("expected a typed-sub-vector grounding");
    };
    assert_eq!(values, vec![1.0, 2.0, 3.0, 4.0], "numeric block only");
    assert_eq!(blocks, vec![Encoder::Numeric]);
    // [OPUS-5] sq-w2af4: a header recording no fusion weight grounds fail-open at 1.0.
    assert_eq!(weights, vec![1.0], "one fail-open weight per kept block");

    // Keep all (empty keep list) → the full row.
    let all = GroundingConfig::default();
    let Some(Grounding::TypedSubVector {
        values,
        blocks,
        weights,
    }) = ground(
        &g,
        &iri("http://ex/bolt"),
        Modality::TypedSubVector,
        &all,
        None,
        Some((&store, &header)),
    ) else {
        panic!()
    };
    assert_eq!(values, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], "all blocks");
    assert_eq!(blocks, vec![Encoder::Numeric, Encoder::Other]);
    assert_eq!(weights, vec![1.0, 1.0]);
}

/// [OPUS-5] sq-w2af4 — the point-3 loop, END-TO-END through the REAL path: graph provenance →
/// `weight_header` → the `.spqv` `SchemaHeader` → `ground` → `fuse_rrf_weighted`. A block fed by a
/// LOW-provenance PREDICATE hands the fusion path a smaller modality multiplier than one fed by a
/// high-provenance predicate, so the high-provenance modality wins the fused ranking.
///
/// These are the header's graph-global per-block **defaults**: mined from every subject asserting
/// the feeding predicate and stored on the one shared header, so every node grounded through
/// [`ground`] sees the same ones. The test pins that explicitly; the per-node path is
/// `ground_weighted_scales_each_node_by_its_own_incident_edges` below.
#[test]
fn typed_sub_vector_carries_predicate_global_fusion_weights() {
    // `ex:good` is asserted by a Proven, fully-confident head; `ex:weak` by a Conjectured,
    // low-confidence head derived from an unreliable source.
    const PROV: &str = r#"
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .

ex:src-weak pkg:confidence "0.4" .

ex:solid  pkg:assurance secx:Proven ; ex:good ex:x .
ex:shaky  pkg:assurance secx:Conjectured ; pkg:confidence "0.3" ;
          prov:wasDerivedFrom ex:src-weak ; ex:weak ex:y .
"#;
    let g = Graph::load_str(PROV, "turtle").unwrap();
    let solid = g.id_of(&iri("http://ex/solid")).unwrap();
    let shaky = g.id_of(&iri("http://ex/shaky")).unwrap();
    let pw = ProvenanceWeights::mine(&g);

    // Two modality blocks in one row: block 0 fed by `ex:good`, block 1 fed by `ex:weak`.
    let plain = SchemaHeader::new(vec![
        Block::new(Encoder::Numeric, Metric::Euclidean, 0, 2),
        Block::new(Encoder::Other, Metric::Euclidean, 2, 2),
    ])
    .unwrap();
    let header = pw
        .weight_header(
            &g,
            &plain,
            &[Some("http://ex/good"), Some("http://ex/weak")],
            WeightMode::Provenance,
        )
        .unwrap();

    let mut store = VectorStore::create(tmp("provweights"), 4).unwrap();
    store.put(solid, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    store.put(shaky, &[5.0, 6.0, 7.0, 8.0]).unwrap();
    store.finalize().unwrap();

    let Some(Grounding::TypedSubVector { weights, .. }) = ground(
        &g,
        &iri("http://ex/solid"),
        Modality::TypedSubVector,
        &GroundingConfig::default(),
        None,
        Some((&store, &header)),
    ) else {
        panic!("expected a typed-sub-vector grounding");
    };
    assert_eq!(weights.len(), 2);
    assert!(
        weights[1] < weights[0],
        "the low-provenance modality must carry the smaller fusion weight ({} < {})",
        weights[1],
        weights[0]
    );

    // The HEADER weights are not per-node: `ex:solid` has no incident `ex:weak` edge at all —
    // block 1's multiplier above came entirely from `ex:shaky ex:weak ex:y`. Grounding `ex:shaky`
    // (a node with the opposite provenance, and no incident `ex:good` edge) against the same
    // header returns the SAME weights. That is what "graph-global default" means, asserted rather
    // than implied — `ground_weighted` is the per-node path.
    let Some(Grounding::TypedSubVector { weights: other, .. }) = ground(
        &g,
        &iri("http://ex/shaky"),
        Modality::TypedSubVector,
        &GroundingConfig::default(),
        None,
        Some((&store, &header)),
    ) else {
        panic!("expected a typed-sub-vector grounding");
    };
    assert_eq!(
        other, weights,
        "block weights come from the shared header, so every node sees the same ones"
    );

    // ...and that weight is a REAL fuse weight: the high-provenance modality wins the fusion.
    let good: Vec<(&str, f64)> = vec![("from_good", 1.0)];
    let weak: Vec<(&str, f64)> = vec![("from_weak", 1.0)];
    let fused = fuse_rrf_weighted(
        &[
            (&good, weights[0] as f64),
            (&weak, weights[1] as f64),
        ],
        sparq_vectors::RRF_K,
        10,
    );
    assert_eq!(fused[0].0, "from_good", "higher-provenance modality ranks first");
    assert!(fused[0].1 > fused[1].1, "and strictly outranks the lower-provenance one");

    // Under the ablation-OFF arm every block is the fail-open 1.0 — grounding is unchanged.
    let off = pw
        .weight_header(
            &g,
            &plain,
            &[Some("http://ex/good"), Some("http://ex/weak")],
            WeightMode::Uniform,
        )
        .unwrap();
    let Some(Grounding::TypedSubVector { weights, .. }) = ground(
        &g,
        &iri("http://ex/solid"),
        Modality::TypedSubVector,
        &GroundingConfig::default(),
        None,
        Some((&store, &off)),
    ) else {
        panic!()
    };
    assert_eq!(weights, vec![1.0, 1.0], "ablation-off leaves every block unweighted");
}

/// [OPUS-5] sq-w2af4 — the PER-NODE half of point 3, END-TO-END: `ground_weighted` scales a node's
/// modality by **that node's own incident edges**, not by any graph-wide aggregate. Two nodes with
/// the same feeding predicate but opposite incident-edge provenance must get *different* weights,
/// each derived only from its own contributions — and a node with no incident edge for the
/// predicate must fail open at `1.0` rather than inherit the graph's average.
#[test]
fn ground_weighted_scales_each_node_by_its_own_incident_edges() {
    const PROV: &str = r#"
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .

ex:src-weak pkg:confidence "0.4" .

ex:strong pkg:assurance secx:Proven      ; pkg:confidence "0.95" ; ex:p ex:o1 .
ex:frail  pkg:assurance secx:Conjectured ; pkg:confidence "0.3"  ;
          prov:wasDerivedFrom ex:src-weak ; ex:p ex:o2 .
ex:silent ex:other ex:o3 .
"#;
    let g = Graph::load_str(PROV, "turtle").unwrap();
    let pw = ProvenanceWeights::mine(&g);
    let ids: Vec<_> = ["strong", "frail", "silent"]
        .iter()
        .map(|n| g.id_of(&iri(&format!("http://ex/{}", n))).unwrap())
        .collect();

    let mut store = VectorStore::create(tmp("nodeweight"), 2).unwrap();
    for (k, id) in ids.iter().enumerate() {
        store.put(*id, &[k as f32, 1.0]).unwrap();
    }
    store.finalize().unwrap();

    // One block, fed by `ex:p`. The header carries the GRAPH-GLOBAL default for that predicate.
    let plain = SchemaHeader::new(vec![Block::new(Encoder::Numeric, Metric::Euclidean, 0, 2)])
        .unwrap();
    let preds = [Some("http://ex/p")];
    let header = pw.weight_header(&g, &plain, &preds, WeightMode::Provenance).unwrap();
    let global = header.blocks()[0].fusion_weight();

    let weight_of_node = |name: &str, mode: WeightMode| -> f32 {
        let weighting = NodeWeighting { weights: &pw, block_predicates: &preds, mode };
        let Some(Grounding::TypedSubVector { weights, .. }) = ground_weighted(
            &g,
            &iri(&format!("http://ex/{}", name)),
            Modality::TypedSubVector,
            &GroundingConfig::default(),
            None,
            Some((&store, &header)),
            Some(&weighting),
        ) else {
            panic!("expected a typed-sub-vector grounding for {}", name);
        };
        assert_eq!(weights.len(), 1);
        weights[0]
    };

    let strong = weight_of_node("strong", WeightMode::Provenance);
    let frail = weight_of_node("frail", WeightMode::Provenance);
    let silent = weight_of_node("silent", WeightMode::Provenance);

    // The load-bearing property: DISTINCT nodes, DISTINCT weights, each from its own edges only.
    assert!(frail < strong, "the frail node's own edge weighs less ({} < {})", frail, strong);
    assert!(
        (strong - pw.weight_for_subject(ids[0])).abs() < 1e-6,
        "derived only from ex:strong's own incident edge: {}",
        strong
    );
    assert!(
        (frail - pw.weight_for_subject(ids[1])).abs() < 1e-6,
        "derived only from ex:frail's own incident edge: {}",
        frail
    );
    // Neither is the graph-global default the header persisted.
    assert!(
        (strong - global).abs() > 1e-6 && (frail - global).abs() > 1e-6,
        "per-node weights must differ from the graph-global default {}",
        global
    );
    // A node with no incident `ex:p` edge fails OPEN, rather than inheriting the graph average.
    assert_eq!(silent, 1.0, "no incident edge → 1.0, not the graph mean");

    // Ablation-off leaves every node unweighted...
    for name in ["strong", "frail", "silent"] {
        assert_eq!(weight_of_node(name, WeightMode::Uniform), 1.0);
    }
    // ...and plain `ground` still returns the header's shared graph-global default for every node.
    for name in ["strong", "frail"] {
        let Some(Grounding::TypedSubVector { weights, .. }) = ground(
            &g,
            &iri(&format!("http://ex/{}", name)),
            Modality::TypedSubVector,
            &GroundingConfig::default(),
            None,
            Some((&store, &header)),
        ) else {
            panic!()
        };
        assert_eq!(weights, vec![global], "ground() keeps the persisted default");
    }
}

/// [OPUS-5] sq-w2af4 — the point-2 loop, END-TO-END: `sketch_predicate` pools a node's
/// multi-valued predicate over the REAL `.spqv` store, weighting each contribution by **the
/// asserting statement's** `w(t)`. The fixture reifies one of `ex:hub`'s two `ex:cites` statements
/// as doubtful (RDF 1.2 `rdf:reifies` + a low `pkg:confidence`) while `ex:hub` itself stays a
/// high-assurance entity — the case only per-statement provenance can express.
///
/// Scope of the claim: the weight says *this assertion* is doubtful, and nothing about the object
/// entity's own quality. On a graph with no reified statements every one of `ex:hub`'s edges shares
/// its head weight and the pool is exactly the arithmetic mean — asserted below as the honest
/// no-op, not glossed as weighting.
#[test]
fn sketch_predicate_pools_statements_confidence_weighted() {
    const PROV: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .

ex:hub pkg:assurance secx:Proven ; ex:cites ex:trusted , ex:dubious .

# The DOUBT is on the statement, not on either entity: `ex:dubious` is unannotated.
ex:st rdf:reifies <<( ex:hub ex:cites ex:dubious )>> ; pkg:confidence "0.2" .
"#;
    let g = Graph::load_str(PROV, "turtle").unwrap();
    let hub_id = g.id_of(&iri("http://ex/hub")).unwrap();
    let cites = g.id_of(&iri("http://ex/cites")).unwrap();
    let trusted = g.id_of(&iri("http://ex/trusted")).unwrap();
    let dubious = g.id_of(&iri("http://ex/dubious")).unwrap();
    let pw = ProvenanceWeights::mine(&g);
    assert_eq!(pw.annotated_statements(), 1, "one reified statement carries provenance");

    let mut store = VectorStore::create(tmp("sketch"), 2).unwrap();
    store.put(trusted, &[1.0, 0.0]).unwrap();
    store.put(dubious, &[0.0, 1.0]).unwrap();
    store.finalize().unwrap();

    let hub = iri("http://ex/hub");
    // OFF arm: the plain arithmetic mean of (1,0) and (0,1).
    let mean = sketch_predicate(&g, &store, &hub, "http://ex/cites", &pw, WeightMode::Uniform)
        .unwrap()
        .expect("two neighbours have stored vectors");
    assert!((mean[0] - 0.5).abs() < 1e-6 && (mean[1] - 0.5).abs() < 1e-6, "{:?}", mean);

    // ON arm: the undoubted statement's contribution dominates, in the exact w(t) proportion.
    let pooled =
        sketch_predicate(&g, &store, &hub, "http://ex/cites", &pw, WeightMode::Provenance)
            .unwrap()
            .unwrap();
    let wt = pw.weight_of([hub_id, cites, trusted], WeightMode::Provenance);
    let wd = pw.weight_of([hub_id, cites, dubious], WeightMode::Provenance);
    assert!(wd < wt, "the reified statement is the doubted one ({} < {})", wd, wt);
    assert!(
        pooled[0] > pooled[1],
        "the higher-provenance statement must dominate the sketch ({:?})",
        pooled
    );
    assert!((pooled[0] - wt / (wt + wd)).abs() < 1e-6, "{:?}", pooled);

    // WITHOUT statement provenance the axis is an exact no-op, not an entity-quality proxy: strip
    // the reifier and the same call returns the plain mean even under `Provenance`.
    let bare = Graph::load_str(
        "@prefix ex: <http://ex/> .\nex:hub ex:cites ex:trusted , ex:dubious .",
        "turtle",
    )
    .unwrap();
    let bare_pw = ProvenanceWeights::mine(&bare);
    let mut bare_store = VectorStore::create(tmp("sketch_bare"), 2).unwrap();
    bare_store.put(bare.id_of(&iri("http://ex/trusted")).unwrap(), &[1.0, 0.0]).unwrap();
    bare_store.put(bare.id_of(&iri("http://ex/dubious")).unwrap(), &[0.0, 1.0]).unwrap();
    bare_store.finalize().unwrap();
    let flat = sketch_predicate(
        &bare,
        &bare_store,
        &hub,
        "http://ex/cites",
        &bare_pw,
        WeightMode::Provenance,
    )
    .unwrap()
    .unwrap();
    assert!((flat[0] - 0.5).abs() < 1e-6 && (flat[1] - 0.5).abs() < 1e-6, "{:?}", flat);

    // An unknown predicate / a node with no vectorised neighbours pools to None, never an error.
    assert!(sketch_predicate(&g, &store, &hub, "http://ex/absent", &pw, WeightMode::Provenance)
        .unwrap()
        .is_none());
}

#[test]
fn typed_sub_vector_none_when_node_has_no_vector() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let header =
        SchemaHeader::new(vec![Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4)]).unwrap();
    let mut store = VectorStore::create(tmp("novec"), 4).unwrap();
    // Put a DIFFERENT node's vector; bolt has none.
    let other = g.id_of(&iri("http://ex/Car")).unwrap();
    store.put(other, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    store.finalize().unwrap();
    assert!(
        ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::TypedSubVector,
            &GroundingConfig::default(),
            None,
            Some((&store, &header)),
        )
        .is_none(),
        "a node with no stored vector grounds to None for TypedSubVector"
    );
}

#[test]
fn completeness_is_profile_relative_closure_vs_no_closure() {
    // The entailed fact `ex:bolt a ex:Vehicle` exists only after the RDFS closure.
    let asserted = Graph::load_str(TTL, "turtle").unwrap();
    let closed = close_for_vectorise(TTL, "turtle", Profile::Rdfs)
        .unwrap()
        .graph;

    let cfg = GroundingConfig {
        minimal_type_pattern: false,
        ..Default::default()
    };

    let has_vehicle = |g: &Graph| -> bool {
        match ground(
            g,
            &iri("http://ex/bolt"),
            Modality::Subgraph,
            &cfg,
            None,
            None,
        ) {
            Some(Grounding::Subgraph(facts)) => facts.iter().any(|f| {
                f.predicate == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                    && f.object == "http://ex/Vehicle"
            }),
            _ => false,
        }
    };

    // Over the CLOSED graph the entailed type is present (complete relative to RDFS).
    assert!(
        has_vehicle(&closed),
        "closure-before-grounding must surface the entailed Vehicle type"
    );
    // Over the UN-closed graph it is silently absent — the honest profile-relative boundary.
    assert!(
        !has_vehicle(&asserted),
        "an un-closed graph is incomplete outside the asserted facts (no answer-completeness claim)"
    );
}

#[test]
fn dispatch_via_output_type_grounds_each_modality() {
    let g = close_for_vectorise(TTL, "turtle", Profile::Rdfs)
        .unwrap()
        .graph;
    let node = iri("http://ex/bolt");

    // Facts → Subgraph.
    let m = Modality::for_output(OutputType::Facts);
    assert!(matches!(
        ground(&g, &node, m, &GroundingConfig::default(), None, None),
        Some(Grounding::Subgraph(_))
    ));
    // Text → NlString.
    let m = Modality::for_output(OutputType::Text);
    assert!(matches!(
        ground(&g, &node, m, &GroundingConfig::default(), None, None),
        Some(Grounding::NlString(_))
    ));
    // Value → TypedValue (bolt has seats/electric).
    let m = Modality::for_output(OutputType::Value);
    assert!(matches!(
        ground(&g, &node, m, &GroundingConfig::default(), None, None),
        Some(Grounding::TypedValue(_))
    ));
    // Ambiguous → the EXACT subgraph default.
    let m = Modality::for_output(OutputType::Ambiguous);
    assert!(matches!(
        ground(&g, &node, m, &GroundingConfig::default(), None, None),
        Some(Grounding::Subgraph(_))
    ));
}
