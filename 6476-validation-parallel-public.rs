// GPT-6 Astra: actual scan JSON snapshot path on a dedicated two-worker pool.
use sparq_core::Graph;
use sparq_engine::{query, query_json_with_budget, QueryBudget};
use std::sync::{Arc, atomic::AtomicBool};
fn main() {
    let n=60_000;
    let mut nt=String::new();
    for i in 0..n { nt.push_str(&format!("<urn:s{i}> <urn:p> <urn:o> .\n")); }
    let graph=Graph::load_str(&nt,"ntriples").unwrap();
    let pool=rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    let budget=QueryBudget::cancelled_by(Arc::new(AtomicBool::new(false)));
    let q="SELECT ?s WHERE { ?s <urn:p> ?o }";
    let json=pool.install(|| query_json_with_budget(&graph,q,&budget)).unwrap();
    assert_eq!(json.matches("\"s\":").count(),n);
    assert_eq!(query(&graph,q).unwrap().rows.len(),n);
    println!("public_scan_json rows={n} pool_workers=2 armed_cancel=false complete=true");
}
