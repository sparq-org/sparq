//! [GPT-6 Astra] Exact-source recorder comparison; diagnostic only.
use sparq_core::Graph;
fn main() {
    rayon::ThreadPoolBuilder::new().num_threads(1).build_global().unwrap();
    let mut ttl=String::new();
    for i in 0..2000 { ttl.push_str(&format!("<urn:s:{i}> <urn:p> {i} ; <urn:q> {i} .\n")); }
    let graph=Graph::load_str(&ttl,"turtle").unwrap();
    let q="ASK { ?s <urn:p> ?o . ?s <urn:q> ?o . FILTER(?o + 0 < 0) }";
    assert!(!sparq_engine::ask(&graph,q).unwrap());
    let _guard=sparq_engine::zk::install();
    assert!(!sparq_engine::ask(&graph,q).unwrap());
    let witness=sparq_engine::zk::take();
    assert_eq!(witness.patterns.len(),2);
    assert!(witness.patterns.iter().all(|p|p.triples.len()==2000));
    assert!(witness.first_uncaptured().is_none());
    assert!(!witness.filters.is_empty());
    assert!(witness.filters.iter().flat_map(|f|&f.rows).all(|(_,pass)|!*pass));
    println!("{witness:?}");
}
