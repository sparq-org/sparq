//! (sq-1fz0) sparq-text BEIR retrieval driver — the sparq side of the
//! full-text-search **IR-quality axis** (design §3.4). It loads a BEIR cut's corpus
//! (as RDF literals) into a sparq `Graph`, builds a BM25 `TextIndex`, retrieves the
//! top-k documents for each query, and emits a **TREC run file** on stdout.
//!
//! That run is scored — Recall@100 / nDCG@10, emitted as DEFICITS (G4) — by the SAME
//! engine-agnostic scorer the kernel oracle uses:
//! `scripts/bench-adapters/beir_ir_adapter.py --score --run <this output> --qrels <qrels>`.
//! Because both sparq-text AND `lucene-anserini` are scored by that one scorer against
//! the SAME qrels, the comparison is apples-to-apples (design §3.4 fairness note).
//!
//! GATHER-ONLY: the BEIR corpus is not redistributable in-repo, so the inputs are
//! produced on a gather box by `scripts/gather-competitors.sh --run --only lucene-anserini`
//! (which downloads the cut and converts it to the plain formats below). This example
//! takes NO JSON dependency — it reads two plain text inputs:
//!
//! ```text
//! cargo run -p sparq-text --release --example beir_text -- <corpus.nt> <queries.tsv> [k] [tag]
//! ```
//!
//!   <corpus.nt>    N-Triples, one document per `<beir:doc:DOCID> <beir:text> "<text>" .`
//!                  triple (DOCID is the BEIR document id; the object literal is the
//!                  searchable text). Any other triples are ignored.
//!   <queries.tsv>  one `DOCID-free` query per line: `<qid>\t<query text>`.
//!   [k]            top-k to retrieve per query (default 100 — BEIR's Recall@100 depth).
//!   [tag]          the TREC run tag (default `sparq-text`).
//!
//! Output (stdout): TREC run rows `<qid> Q0 <docid> <rank> <score> <tag>`, rank 1-based,
//! best-first by BM25 score — exactly the format `beir_ir_adapter.py --score` parses.

use std::collections::HashMap;

use oxrdf::Term;
use sparq_core::Graph;
use sparq_text::TextIndex;

/// The corpus predicate + the document-id IRI prefix this driver expects (must match
/// what the gather harness writes). A document is `<PREFIX{docid}> <PRED> "text" .`.
const DOC_IRI_PREFIX: &str = "beir:doc:";

fn die(msg: &str) -> ! {
    eprintln!("beir_text: {msg}");
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let corpus_path = args
        .next()
        .unwrap_or_else(|| die("usage: beir_text <corpus.nt> <queries.tsv> [k] [tag]"));
    let queries_path = args
        .next()
        .unwrap_or_else(|| die("usage: beir_text <corpus.nt> <queries.tsv> [k] [tag]"));
    let k: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(100);
    let tag = args.next().unwrap_or_else(|| "sparq-text".to_string());

    // ---- load the corpus, recording each searchable literal id -> BEIR docid --------
    let nt = std::fs::read_to_string(&corpus_path)
        .unwrap_or_else(|e| die(&format!("read {corpus_path}: {e}")));
    let (dict, triples) = Graph::parse_to_triples(&nt, "ntriples")
        .unwrap_or_else(|e| die(&format!("parse {corpus_path}: {e}")));

    // literal object id -> docid (from the subject IRI's `beir:doc:` suffix). A literal
    // shared by two documents would be ambiguous; BEIR doc texts are distinct, but if a
    // collision occurs we keep the FIRST (deterministic) and never silently overwrite.
    let mut doc_of: HashMap<u32, String> = HashMap::new();
    for [s, _p, o] in &triples {
        // object must be a string literal to be a searchable document.
        if !matches!(dict.term(*o), Term::Literal(_)) {
            continue;
        }
        if let Term::NamedNode(n) = dict.term(*s) {
            if let Some(docid) = n.as_str().strip_prefix(DOC_IRI_PREFIX) {
                doc_of.entry(*o).or_insert_with(|| docid.to_string());
            }
        }
    }
    if doc_of.is_empty() {
        die("no `<beir:doc:DOCID> <pred> \"text\" .` triples found in the corpus");
    }

    let graph = Graph::from_parts(dict, triples);
    let index = TextIndex::build(&graph);

    // ---- retrieve top-k per query, emit a TREC run ----------------------------------
    let queries = std::fs::read_to_string(&queries_path)
        .unwrap_or_else(|e| die(&format!("read {queries_path}: {e}")));
    let mut out = String::new();
    for line in queries.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (qid, text) = match line.split_once('\t') {
            Some((q, t)) => (q, t),
            None => die(&format!(
                "bad queries.tsv line (want `<qid>\\t<text>`): {line:?}"
            )),
        };
        let hits = index.search(text);
        let mut rank = 0usize;
        for hit in hits.iter().take(k) {
            // Only hits whose literal maps to a BEIR document are part of the run; a hit
            // on some other literal in the graph (there should be none) is skipped.
            if let Some(docid) = doc_of.get(&hit.id) {
                rank += 1;
                out.push_str(&format!("{qid} Q0 {docid} {rank} {:.6} {tag}\n", hit.score));
            }
        }
    }
    print!("{out}");
}
