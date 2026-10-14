//! sq-mztg8.1 (epic sq-mztg8; design `research/fo-llm-bridge.md` §2.3/§3.3/§6
//! Phase 3): the **closed NL→NL A/B harness** for the read-path URI-hiding "compose" step,
//! run over the **real PKG**.
//!
//! # What this measures (and what it pointedly does NOT)
//!
//! The headline K4 question is *does hiding URIs help or hurt a real small model's answer
//! accuracy?* That needs a real-model NL→NL fan-out, which is **UNMEASURED** on this work box
//! (no API key; only the `sparq-nlq` record/replay fixtures exist). Reporting an accuracy
//! delta from this harness would mean **fabricating it** — which the empirical-honesty
//! mandate forbids, and which this project has been burned by (three char-proxy token claims
//! reversed by real measurement). So this harness measures the two things that ARE
//! measurable without a model, over the **real PKG answer tables**:
//!
//! 1. **Round-trip fidelity** — does the URI-hidden view *preserve answer identity*? Every
//!    shown label must map back to exactly the IRI it hid; a collision is a lost identity
//!    that would silently change the answer regardless of the model. This is the SOUNDNESS
//!    gate: a hide that fails it must not ship.
//! 2. **Presentation cost + coverage** — how compact the labelled view is to read
//!    (`char_ratio`) and the fraction of answer IRIs hiding can actually help
//!    (`coverage` — the un-labelled remainder falls open to the raw IRI).
//!
//! The accuracy verdict is registered **UNMEASURED** in `bench/EXPERIMENTS.md` with the
//! pre-registered null. These numbers are **MEASURED ON THIS WORK BOX, NON-CANONICAL**
//! (AGENTS.md "No hard-coded performance numbers"): deterministic over the checked-in PKG,
//! but a fidelity/cost sanity check, **not** a published accuracy benchmark.
//!
//! ```sh
//! cargo run -p sparq-vectors --features compose --example compose_ab -- <pkg.ttl> [more.ttl ...]
//! ```
//!
//! With no path argument it looks for the in-tree PKG under
//! `crates/sparq-kb/ingest/{pkg-instances,agents-findings}.ttl`.

use oxrdf::Term;
use sparq_core::dict::{self, TermParts};
use sparq_core::Graph;
use sparq_vectors::compose::{ab_report, ComposeConfig};

/// `rdf:type`.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<String> = if args.is_empty() {
        vec![
            "crates/sparq-kb/ingest/pkg-instances.ttl".to_string(),
            "crates/sparq-kb/ingest/agents-findings.ttl".to_string(),
        ]
    } else {
        args
    };

    // Concatenate the PKG sources into one graph (the agent queries the whole PKG).
    let mut ttl = String::new();
    for p in &paths {
        match std::fs::read_to_string(p) {
            Ok(s) => {
                ttl.push_str(&s);
                ttl.push('\n');
            }
            Err(e) => {
                eprintln!("skip {p}: {e}");
            }
        }
    }
    if ttl.trim().is_empty() {
        eprintln!("no PKG content loaded from {paths:?}");
        std::process::exit(2);
    }
    let graph = match Graph::load_str(&ttl, "turtle") {
        Ok(g) => g,
        Err(e) => {
            eprintln!("PKG parse failed: {e}");
            std::process::exit(2);
        }
    };

    // Answer tables = the entities of each type present in the PKG (the shape of a real
    // "list all X" agent question). One A/B per class so a per-class coverage/fidelity
    // breakdown is visible.
    let cfg = ComposeConfig::default();
    let classes = types_present(&graph);

    println!("# compose URI-hiding A/B over the PKG (NON-CANONICAL, work-box)");
    println!("# sources: {}", paths.join(", "));
    println!(
        "# the ACCURACY half (does hiding help a real model?) is UNMEASURED — no API key; \
         see bench/EXPERIMENTS.md"
    );
    println!("class\tn_iri\tlexical\tlocal_name\tfell_open\tcoverage\tfidelity_ok\tcollisions\tchar_ratio");

    let mut total_iri = 0usize;
    let mut total_collisions = 0usize;
    let mut total_chars_visible = 0usize;
    let mut total_chars_hidden = 0usize;

    for (class_iri, members) in &classes {
        if members.is_empty() {
            continue;
        }
        let report = ab_report(&graph, members, &cfg);
        total_iri += report.iri_bindings;
        total_collisions += report.collisions;
        total_chars_visible += report.chars_visible;
        total_chars_hidden += report.chars_hidden;
        println!(
            "{}\t{}\t{}\t{}\t{}\t{:.3}\t{}\t{}\t{:.3}",
            local_name(class_iri),
            report.iri_bindings,
            report.lexical,
            report.local_name,
            report.fell_open,
            report.coverage(),
            report.fidelity_preserved(),
            report.collisions,
            report.char_ratio(),
        );
    }

    // Aggregate over EVERY typed entity at once — the whole-PKG answer table — so the
    // cross-class collision count is real (a label clash between two classes still loses
    // identity).
    let all: Vec<Term> = classes
        .iter()
        .flat_map(|(_, m)| m.iter().cloned())
        .collect();
    let overall = ab_report(&graph, &all, &cfg);
    println!(
        "ALL\t{}\t{}\t{}\t{}\t{:.3}\t{}\t{}\t{:.3}",
        overall.iri_bindings,
        overall.lexical,
        overall.local_name,
        overall.fell_open,
        overall.coverage(),
        overall.fidelity_preserved(),
        overall.collisions,
        overall.char_ratio(),
    );
    eprintln!("{}", overall.summary());

    // The soundness gate: the whole-PKG hide must preserve answer identity. A non-zero
    // collision count is a LOUD failure (the hide would silently change an answer), not a
    // warning — exit non-zero so a CI/bench run notices.
    if !overall.fidelity_preserved() {
        eprintln!(
            "FIDELITY FAILURE: {} answer IRIs collided onto a shared label — the hidden view \
             loses identity over this PKG. URI-hiding is UNSOUND here until the labels are \
             disambiguated.",
            overall.collisions
        );
        std::process::exit(1);
    }
    eprintln!(
        "fidelity preserved over {} typed answer IRIs ({} collisions); chars visible={} hidden={}",
        total_iri, total_collisions, total_chars_visible, total_chars_hidden
    );
}

/// Every (class IRI → its instances) in the graph, by scanning `rdf:type`. Deterministic
/// order (sorted class IRI, then member IRI).
fn types_present(graph: &Graph) -> Vec<(String, Vec<Term>)> {
    let Some(type_pid) = graph.id_of(&named(RDF_TYPE)) else {
        return Vec::new();
    };
    let mut by_class: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let scan = graph.store.scan(&[None, Some(type_pid), None]);
    for row in scan.rows.iter() {
        let [s, _, o] = scan.to_spo(row);
        let (Some(class_iri), Some(subj_iri)) = (iri_of(graph, o), iri_of(graph, s)) else {
            continue;
        };
        by_class.entry(class_iri).or_default().insert(subj_iri);
    }
    by_class
        .into_iter()
        .map(|(c, members)| {
            (
                c,
                members
                    .into_iter()
                    .map(|m| Term::NamedNode(oxrdf::NamedNode::new_unchecked(m)))
                    .collect(),
            )
        })
        .collect()
}

fn named(iri: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri))
}

fn iri_of(graph: &Graph, id: dict::Id) -> Option<String> {
    if dict::is_inline(id) {
        return None;
    }
    match graph.dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => Some(format!("{prefix}{suffix}")),
        _ => None,
    }
}

fn local_name(iri: &str) -> String {
    iri.rsplit(['#', '/']).next().unwrap_or(iri).to_string()
}
