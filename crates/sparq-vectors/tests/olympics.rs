//! Integration test on the real olympics dataset (1.78M triples, 137k rdfs:labels).
//! SKIPS (passes, with a note on stderr) when `bench/qlever-olympics/olympics.nt` is
//! absent — the file is a downloaded benchmark fixture, not checked in. Override the
//! path with `SPARQ_OLYMPICS_NT`. Pipeline under test: `embed_labels` with the
//! deterministic `HashEmbedder` (lexical, offline — see its docs) → `.spqv` finalize →
//! HNSW build → `nearest_term`, with a same-type sanity check on the neighbours.

// (sq-ip3a) HNSW (VectorIndex) is the approximate backend — gated behind `approx-ann`.
#![cfg(feature = "approx-ann")]

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_vectors::{embed_labels, HashEmbedder, VectorIndex, VectorStore};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn data_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SPARQ_OLYMPICS_NT") {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/qlever-olympics/olympics.nt");
    p.exists().then_some(p)
}

/// The rdf:type objects of `id`, via one SPO range scan.
fn types_of(g: &Graph, type_pid: Id, id: Id) -> Vec<Id> {
    let scan = g.store().scan(&[Some(id), Some(type_pid), None]);
    scan.rows.iter().map(|row| scan.to_spo(row)[2]).collect()
}

#[test]
fn olympics_label_embeddings_nearest_term_sanity() {
    let Some(path) = data_path() else {
        eprintln!("skipping: olympics.nt not present (downloaded benchmark fixture)");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let g = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    assert!(g.len() > 1_700_000, "expected the 1.78M-triple olympics dataset, got {}", g.len());

    let spqv = std::env::temp_dir()
        .join(format!("sparq-vectors-olympics-{}.spqv", std::process::id()));
    let embedder = HashEmbedder::new(64);
    let mut store = VectorStore::create(&spqv, 64).unwrap();
    let n = embed_labels(&g, &mut store, &embedder).expect("embed labels");
    assert!(n > 100_000, "olympics has 137k rdfs:labels, embedded only {n}");
    store.finalize().unwrap();

    // Sparse coverage at scale: labeled entities resolve, unlabeled terms do not.
    let reopened = VectorStore::open(&spqv).unwrap();
    assert_eq!(reopened.len(), n);

    let index = VectorIndex::build(&store);
    let athlete = Term::NamedNode(
        NamedNode::new("http://wallscope.co.uk/resource/olympics/athlete/AAananthaSambuMayavo")
            .unwrap(),
    );
    let top = index.nearest_term(&athlete, &g, &store, 10);
    assert_eq!(top.len(), 10, "a 137k-vector store must yield 10 neighbours");

    let type_pid = g
        .id_of(&Term::NamedNode(NamedNode::new(RDF_TYPE).unwrap()))
        .expect("olympics has rdf:type");
    let athlete_id = g.id_of(&athlete).unwrap();
    let athlete_types = types_of(&g, type_pid, athlete_id);
    assert!(!athlete_types.is_empty());

    let mut prev = f32::INFINITY;
    let mut same_type = 0;
    for (t, s) in &top {
        assert!(*s > -1.0001 && *s <= 1.0001, "cosine out of range: {s}");
        assert!(*s <= prev, "scores must be non-increasing");
        prev = *s;
        assert_ne!(t, &athlete, "self must be excluded");
        assert!(!matches!(t, Term::Literal(_)), "only labeled entities are embedded");
        let nid = g.id_of(t).expect("neighbour must be a dictionary term");
        if types_of(&g, type_pid, nid).iter().any(|ty| athlete_types.contains(ty)) {
            same_type += 1;
        }
    }
    // HashEmbedder similarity is lexical (names look like names), so an athlete's
    // nearest labeled neighbours should overwhelmingly share its type. Loose floor;
    // the recall gate lives in tests/recall.rs and the latency numbers in the README.
    assert!(same_type >= 5, "expected mostly same-type neighbours, got {same_type}/10: {top:?}");

    let _ = std::fs::remove_file(&spqv);
}
