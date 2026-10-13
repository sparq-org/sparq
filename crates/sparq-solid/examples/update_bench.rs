//! `PodStore::update_as` cost per update shape, on the WAC fixture with 20 filler triples
//! per document (issue #6796). Run:
//!     cargo run -p sparq-solid --example update_bench --release
//!
//! Each line is the median and spread of single-update wall times for one shape:
//!   1. the LDP PATCH shape, `DELETE { … } INSERT { … } WHERE { … }` on a document;
//!   2. a plain `INSERT DATA` into a document;
//!   3. the PATCH shape on a container's `.acl` (the rules change and are rebuilt).

use sparq_core::Graph;
use sparq_solid::fixture::{wac_fixture_sized, ALICE};
use sparq_solid::{PodStore, Session};
use std::time::Instant;

const TITLE: &str = "https://ex.dev/ns#title";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";

fn report(label: &str, mut ms: Vec<f64>) {
    ms.sort_by(f64::total_cmp);
    let q = |p: f64| ms[((ms.len() - 1) as f64 * p).round() as usize];
    println!(
        "{label:44} n={:5}  median {:9.4} ms  p10 {:9.4}  p90 {:9.4}",
        ms.len(),
        q(0.5),
        q(0.1),
        q(0.9)
    );
}

fn time(mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1000);
    let graph = Graph::load_dataset(&wac_fixture_sized(20), "nquads").expect("fixture loads");
    let mut store = PodStore::new(graph);
    store.materialize_wac().expect("materializes");
    let alice = Session {
        agent: Some(ALICE),
        client: None,
        issuer: None,
        now: None,
    };
    // Alice holds Write over all of priv0/: 6 × 6 containers × 4 documents.
    let docs: Vec<String> = (0..6)
        .flat_map(|j| (0..6).flat_map(move |k| (0..4).map(move |d| (j, k, d))))
        .map(|(j, k, d)| format!("https://pod.ex/priv0/c{j}/g{k}/d{d}.ttl"))
        .collect();

    let mut patch = Vec::with_capacity(n);
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        let doc = &docs[i % docs.len()];
        let upd = format!(
            "DELETE {{ GRAPH <{doc}> {{ <{doc}#it> <{TITLE}> ?t }} }} \
             INSERT {{ GRAPH <{doc}> {{ <{doc}#it> <{TITLE}> \"title {i}\" }} }} \
             WHERE {{ GRAPH <{doc}> {{ <{doc}#it> <{TITLE}> ?t }} }}"
        );
        patch.push(time(|| {
            store.update_as(&alice, &upd).expect("PATCH permitted")
        }));
        let ins = format!(
            "INSERT DATA {{ GRAPH <{doc}> {{ <{doc}#it> <https://ex.dev/ns#tag> \"t{i}\" }} }}"
        );
        data.push(time(|| {
            store
                .update_as(&alice, &ins)
                .expect("INSERT DATA permitted")
        }));
    }
    report("PATCH shape (DELETE/INSERT WHERE), document", patch);
    report("INSERT DATA, document", data);

    let acl = "https://pod.ex/priv0/c0/.acl";
    let comment = "http://www.w3.org/2000/01/rdf-schema#comment";
    let mut rules = Vec::new();
    for i in 0..(n / 25).max(4) {
        // Restate a comment on the container's owner rule: a write to an `.acl`, so the
        // rules are rebuilt every time, while the grants stay the same.
        let upd = format!(
            "DELETE {{ GRAPH <{acl}> {{ ?r <{comment}> ?c }} }} \
             INSERT {{ GRAPH <{acl}> {{ ?r <{comment}> \"v{i}\" }} }} \
             WHERE {{ GRAPH <{acl}> {{ ?r <{ACL}agent> <{ALICE}> OPTIONAL {{ ?r <{comment}> ?c }} }} }}"
        );
        rules.push(time(|| {
            store.update_as(&alice, &upd).expect("rule PATCH permitted")
        }));
    }
    report("PATCH shape on a container .acl (rules)", rules);
}
