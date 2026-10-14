//! `embed_labels` integration on a small in-memory graph: predicate priority, literal
//! filtering, and term-keyed nearest-neighbour lookup end to end.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{
    embed_labels, embed_labels_with, nearest_term_exact, Embedder, HashEmbedder, LabelConfig,
    VectorStore,
};
// (sq-ip3a) The HNSW index is the approximate backend — `approx-ann` only.
#[cfg(feature = "approx-ann")]
use sparq_vectors::VectorIndex;

const TTL: &str = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix ex:   <http://example.org/> .

ex:bolt     rdfs:label "Usain Bolt" ; a ex:Athlete .
ex:bolt2    rdfs:label "Usain Bolt Junior" ; a ex:Athlete .
ex:coubertin skos:prefLabel "Pierre de Coubertin" ; a ex:Founder .
# rdfs:label wins over skos:prefLabel (priority order):
ex:both     rdfs:label "From rdfs label" ; skos:prefLabel "From skos" .
# Non-literal label object and unlabeled entity: skipped.
ex:weird    rdfs:label ex:bolt .
ex:silent   a ex:Athlete .
# Empty label: skipped.
ex:blank    rdfs:label "   " .
"#;

fn term(iri: &str) -> Term {
    Term::NamedNode(NamedNode::new(iri).unwrap())
}

#[test]
fn embed_labels_covers_labeled_entities_only() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let path = std::env::temp_dir()
        .join(format!("sparq-vectors-labels-{}.spqv", std::process::id()));
    let embedder = HashEmbedder::new(32);
    let mut store = VectorStore::create(&path, 32).unwrap();
    let n = embed_labels(&g, &mut store, &embedder).unwrap();
    assert_eq!(n, 4, "bolt, bolt2, coubertin, both");
    store.finalize().unwrap();

    let id = |iri: &str| g.id_of(&term(iri)).unwrap();
    assert!(store.get(id("http://example.org/bolt")).is_some());
    assert!(store.get(id("http://example.org/coubertin")).is_some());
    assert!(store.get(id("http://example.org/both")).is_some());
    assert!(store.get(id("http://example.org/weird")).is_none(), "IRI-object label skipped");
    assert!(store.get(id("http://example.org/silent")).is_none(), "unlabeled entity skipped");
    assert!(store.get(id("http://example.org/blank")).is_none(), "whitespace label skipped");

    // Priority: ex:both must be embedded from its rdfs:label, not its skos:prefLabel.
    let both = store.get(id("http://example.org/both")).unwrap();
    let expect = &embedder.embed(&["From rdfs label"]).unwrap()[0];
    assert_eq!(both, &expect[..]);

    // Term-keyed lookup, exact and HNSW: bolt's nearest labeled neighbor is bolt2
    // (shared n-grams), itself excluded.
    let exact = nearest_term_exact(&store, &g, &term("http://example.org/bolt"), 1);
    assert_eq!(exact[0].0, term("http://example.org/bolt2"));
    #[cfg(feature = "approx-ann")]
    {
        let index = VectorIndex::build(&store);
        let approx = index.nearest_term(&term("http://example.org/bolt"), &g, &store, 1);
        assert_eq!(approx[0].0, term("http://example.org/bolt2"));
        assert!(index
            .nearest_term(&term("http://example.org/nowhere"), &g, &store, 3)
            .is_empty());
    }

    // Unembedded / unknown terms return empty, not errors.
    assert!(nearest_term_exact(&store, &g, &term("http://example.org/silent"), 3).is_empty());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn custom_predicates_and_dim_mismatch() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let path = std::env::temp_dir()
        .join(format!("sparq-vectors-labels2-{}.spqv", std::process::id()));

    // skos-only config: only coubertin and both have skos:prefLabel.
    let cfg = LabelConfig {
        predicates: vec![NamedNode::new("http://www.w3.org/2004/02/skos/core#prefLabel").unwrap()],
        batch: 1,
    };
    let mut store = VectorStore::create(&path, 16).unwrap();
    let n = embed_labels_with(&g, &mut store, &HashEmbedder::new(16), &cfg).unwrap();
    assert_eq!(n, 2);

    // Embedder/store dimension mismatch is an error.
    let mut bad = VectorStore::create(&path, 8).unwrap();
    assert!(embed_labels(&g, &mut bad, &HashEmbedder::new(16)).is_err());

    let _ = std::fs::remove_file(&path);
}
