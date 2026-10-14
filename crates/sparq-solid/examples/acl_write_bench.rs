//! Cost of an access-control write and where it goes (the write-path profile the scoped
//! materializer design asks for first, `research/solid-pod-scoped-materializer-design.md`
//! §5.1). Run:
//!     cargo run -p sparq-solid --example acl_write_bench --release -- [pods] [docs] [runs]
//!
//! Builds `pods` WAC pods of `docs` documents each, every pod governed by its own root
//! `.acl`, then reports the median and spread of `put_acl` of one pod's root `.acl` (the
//! whole view is rebuilt for one write) and of the four phases of that same rebuild, from
//! `PodStore::last_rebuild`: assembling the reasoning input, the reasoning fixpoint,
//! installing the view, and rebuilding the index (with the per-origin cache diff).

use sparq_core::Graph;
use sparq_solid::PodStore;
use std::time::Instant;

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const TITLE: &str = "https://ex.dev/ns#title";
const COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";

fn report(label: &str, mut ms: Vec<f64>) {
    ms.sort_by(f64::total_cmp);
    let q = |p: f64| ms[((ms.len() - 1) as f64 * p).round() as usize];
    println!(
        "{label:28} n={:4}  median {:9.4} ms  p10 {:9.4}  p90 {:9.4}",
        ms.len(),
        q(0.5),
        q(0.1),
        q(0.9)
    );
}

/// The root `.acl` of pod `p`, as N-Triples, with a comment that varies per write.
fn root_acl(p: usize, note: usize) -> String {
    let r = format!("https://p{p}.ex/.acl#owner");
    format!(
        "<{r}> <{RDF_TYPE}> <{ACL}Authorization> .\n\
         <{r}> <{ACL}accessTo> <https://p{p}.ex/> .\n\
         <{r}> <{ACL}default> <https://p{p}.ex/> .\n\
         <{r}> <{ACL}agent> <https://owner{p}.ex/card#me> .\n\
         <{r}> <{ACL}mode> <{ACL}Read> .\n\
         <{r}> <{ACL}mode> <{ACL}Write> .\n\
         <{r}> <{ACL}mode> <{ACL}Control> .\n\
         <{r}> <{COMMENT}> \"v{note}\" .\n"
    )
}

fn dataset(pods: usize, docs: usize) -> String {
    let mut nq = String::new();
    for p in 0..pods {
        for line in root_acl(p, 0).lines() {
            let triple = line.trim_end_matches(" .");
            nq.push_str(&format!("{triple} <https://p{p}.ex/.acl> .\n"));
        }
        for d in 0..docs {
            let doc = format!("https://p{p}.ex/notes/d{d}");
            nq.push_str(&format!("<{doc}#it> <{TITLE}> \"doc {d}\" <{doc}> .\n"));
        }
    }
    nq
}

fn main() {
    let arg = |i: usize, default: usize| {
        std::env::args()
            .nth(i)
            .and_then(|a| a.parse().ok())
            .unwrap_or(default)
    };
    let (pods, docs, runs) = (arg(1, 200), arg(2, 20), arg(3, 30));
    let graph = Graph::load_dataset(&dataset(pods, docs), "nquads").expect("dataset loads");
    let mut store = PodStore::new(graph);
    store.materialize_wac().expect("materializes");
    println!("{pods} pods x {docs} documents, {runs} runs");

    let (mut put, mut assemble, mut reason, mut install, mut index) =
        (vec![], vec![], vec![], vec![], vec![]);
    for i in 1..=runs {
        let acl = root_acl(0, i);
        let t = Instant::now();
        store
            .put_acl("https://p0.ex/.acl", &acl, "ntriples")
            .expect("put_acl");
        put.push(t.elapsed().as_secs_f64() * 1e3);
        // The rebuild this `put_acl` ran, origin-scoped index diff included.
        let s = store.last_rebuild().expect("put_acl rebuilt the view");
        assemble.push(s.assemble_millis);
        reason.push(s.reason_millis);
        install.push(s.install_millis);
        index.push(s.index_millis);
    }
    report("put_acl, one pod's root .acl", put);
    report("  assemble", assemble);
    report("  reason", reason);
    report("  install view", install);
    report("  index", index);
}
