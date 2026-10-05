//! (#4898) Same-box Turtle writer comparison: `graph_to_turtle_with` vs
//! oxttl's `TurtleSerializer` (the Turtle writer behind Oxigraph 0.5's `RdfSerializer`)
//! over a deterministic, DOCUMENT-SHAPED synthetic graph — the shape a Markdown-derived
//! document graph has (headings / paragraphs / list items, each with a string literal of
//! widely varying length, an order integer, parent / next links, and heavy prefix
//! compaction opportunities).
//!
//! ```sh
//! cargo run --release -p sparq-engine-serialize --features serialize-rdf \
//!     --example turtle_vs_oxttl -- [n_elements] [iters]
//! ```
//!
//! Also times the TriG writers (buffered, pretty, and — with `streaming-serialization` —
//! streaming) over a two-graph dataset: the same corpus in the default graph and in a named
//! graph `doc:graph`.
//!
//! Prints min-of-`iters` wall per writer. Timings are whatever this machine measured —
//! NON-canonical work-box numbers, never to be transcribed into docs.

use oxrdf::Triple;
use oxttl::TurtleSerializer;
use sparq_core::Graph;
use sparq_engine_serialize::serialize::{
    default_prefixes, graph_to_trig_pretty_with, graph_to_trig_with, graph_to_turtle,
    graph_to_turtle_with, PrettyOptions,
};
use std::time::{Duration, Instant};

const DOC: &str = "https://example.org/doc/readme#";
const ONT: &str = "https://ruddydoc.example/ontology#";

/// Deterministic LCG so the corpus is identical on every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

const WORDS: &[&str] = &[
    "the",
    "graph",
    "store",
    "query",
    "turtle",
    "serializer",
    "document",
    "heading",
    "list",
    "paragraph",
    "markdown",
    "\"quoted\"",
    "back\\slash",
    "naïve",
    "café",
    "résumé",
    "data",
    "export",
    "prefix",
    "literal",
    "with",
    "and",
    "of",
    "to",
    "a",
    "in",
    "is",
    "for",
];

fn gen_doc(n: usize) -> String {
    let mut rng = Lcg(0x5eed_4898);
    let mut nt = String::new();
    let kinds = ["Heading", "Paragraph", "ListItem", "CodeBlock", "Link"];
    for i in 0..n {
        let s = format!("<{DOC}el-{i}>");
        let kind = kinds[(rng.next() % kinds.len() as u64) as usize];
        nt.push_str(&format!(
            "{s} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ONT}{kind}> .\n"
        ));
        nt.push_str(&format!("{s} <{ONT}partOf> <{DOC}document> .\n"));
        nt.push_str(&format!(
            "{s} <{ONT}order> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"
        ));
        if i > 0 {
            nt.push_str(&format!("{s} <{ONT}previous> <{DOC}el-{}> .\n", i - 1));
        }
        if kind == "Heading" {
            nt.push_str(&format!(
                "{s} <{ONT}level> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                1 + rng.next() % 4
            ));
        }
        // Text of widely varying length: 1..~120 words, occasional newline escape.
        let words = 1 + (rng.next() % 6) * (rng.next() % 20);
        let mut text = String::new();
        for w in 0..words {
            if w > 0 {
                text.push(if rng.next().is_multiple_of(40) {
                    '\n'
                } else {
                    ' '
                });
            }
            text.push_str(WORDS[(rng.next() % WORDS.len() as u64) as usize]);
        }
        let esc = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        nt.push_str(&format!("{s} <{ONT}textContent> \"{esc}\" .\n"));
        if rng.next().is_multiple_of(5) {
            nt.push_str(&format!(
                "{s} <http://www.w3.org/2000/01/rdf-schema#label> \"{esc}\"@en .\n"
            ));
        }
        if kind == "Link" {
            nt.push_str(&format!(
                "{s} <{ONT}href> <https://example.com/some/path/{}?q=1> .\n",
                rng.next() % 1000
            ));
        }
    }
    nt
}

fn time_min(iters: usize, mut f: impl FnMut() -> usize) -> (Duration, usize) {
    let mut best = Duration::MAX;
    let mut bytes = 0;
    for _ in 0..iters {
        let t = Instant::now();
        bytes = std::hint::black_box(f());
        best = best.min(t.elapsed());
    }
    (best, bytes)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(1500);
    let iters: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(200);

    let graph = Graph::load_str(&gen_doc(n), "nt").expect("corpus parses");
    let mut prefixes = default_prefixes();
    prefixes.insert("doc".into(), DOC.into());
    prefixes.insert("ont".into(), ONT.into());

    let triples: Vec<Triple> = graph
        .iter_ids()
        .map(|[s, p, o]| {
            let subject = match graph.dict.term(s) {
                oxrdf::Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
                oxrdf::Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
                _ => unreachable!(),
            };
            let oxrdf::Term::NamedNode(predicate) = graph.dict.term(p) else {
                unreachable!()
            };
            Triple {
                subject,
                predicate,
                object: graph.dict.term(o),
            }
        })
        .collect();
    println!("document graph: {n} elements, {} triples", triples.len());
    // Optional: dump the sparq output (byte-identity checks across writer changes).
    if let Ok(path) = std::env::var("TURTLE_DUMP") {
        std::fs::write(&path, graph_to_turtle_with(&graph, &prefixes)).expect("dump");
        std::fs::write(format!("{path}.default"), graph_to_turtle(&graph)).expect("dump");
    }

    let oxttl_run = |ts: &[Triple]| {
        let mut ser = TurtleSerializer::new();
        for (p, ns) in &prefixes {
            ser = ser.with_prefix(p, ns).expect("valid prefix");
        }
        let mut w = ser.for_writer(Vec::with_capacity(1 << 16));
        for t in ts {
            w.serialize_triple(t).expect("in-memory write");
        }
        w.finish().expect("in-memory write").len()
    };

    let (sparq, sb) = time_min(iters, || graph_to_turtle_with(&graph, &prefixes).len());
    let (ox_pre, ob) = time_min(iters, || oxttl_run(&triples));
    let (ox_full, _) = time_min(iters, || {
        let ts: Vec<Triple> = graph
            .iter_ids()
            .map(|[s, p, o]| {
                let subject = match graph.dict.term(s) {
                    oxrdf::Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
                    oxrdf::Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
                    _ => unreachable!(),
                };
                let oxrdf::Term::NamedNode(predicate) = graph.dict.term(p) else {
                    unreachable!()
                };
                Triple {
                    subject,
                    predicate,
                    object: graph.dict.term(o),
                }
            })
            .collect();
        oxttl_run(&ts)
    });
    println!(
        "sparq graph_to_turtle_with          : {:>10.3?}  ({sb} bytes)",
        sparq
    );
    println!(
        "oxttl TurtleSerializer (pre-decoded): {:>10.3?}  ({ob} bytes)",
        ox_pre
    );
    println!("oxttl TurtleSerializer (+id decode) : {:>10.3?}", ox_full);

    // TriG: the corpus in the default graph AND in the named graph `doc:graph`.
    let doc = gen_doc(n);
    let mut nq = doc.clone();
    for line in doc.lines() {
        let body = line.strip_suffix(" .").expect("N-Triples line");
        nq.push_str(&format!("{body} <{DOC}graph> .\n"));
    }
    let dataset = Graph::load_dataset(&nq, "nquads").expect("dataset parses");
    let (trig, tb) = time_min(iters, || graph_to_trig_with(&dataset, &prefixes).len());
    println!(
        "sparq graph_to_trig_with            : {:>10.3?}  ({tb} bytes)",
        trig
    );
    #[cfg(feature = "streaming-serialization")]
    {
        let (st, stb) = time_min(iters, || {
            let mut buf = Vec::with_capacity(1 << 16);
            sparq_engine_serialize::serialize::graph_to_trig_streaming(
                &dataset, &prefixes, &mut buf,
            )
            .expect("in-memory write");
            buf.len()
        });
        println!(
            "sparq graph_to_trig_streaming       : {:>10.3?}  ({stb} bytes)",
            st
        );
    }
    let pretty = PrettyOptions::default();
    let (pt, ptb) = time_min(iters, || {
        graph_to_trig_pretty_with(&dataset, &prefixes, &pretty).len()
    });
    println!(
        "sparq graph_to_trig_pretty_with     : {:>10.3?}  ({ptb} bytes)",
        pt
    );
    println!("(min of {iters}; NON-canonical work-box timings)");
}
