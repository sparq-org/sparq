//! Example SPARQL lookups over the ingested PKG graph — the proof-of-value for the
//! query-skill (sq-2m6zm.3) and the token-A/B (sq-2m6zm.4).
//!
//! sq-2m6zm.2 (epic sq-2m6zm). 🤖 SPARQ agent — PKG ingestion PoC. These
//! answer the design's §6 worked questions by returning the **minimal** triples
//! (binding rows) that satisfy the pattern — `O(answer)` tokens, not `O(corpus)`.
//! The answers are computed by sparq's own SPARQL engine over the ingested graph; the
//! engine bounds the result set, so within the store the answer cannot be fabricated.
//!
//! Run: `cargo test -p sparq-kb --features query --test ingest_query -- --nocapture`
#![cfg(feature = "query")]

use oxrdf::Term;
use sparq_engine::QueryResult;
use sparq_kb::query::{ask_pkg, load_pkg};

/// Pretty-print a result set so the proof appears in the PR description.
fn dump(label: &str, q: &str, r: &QueryResult) {
    eprintln!("\n=== {label} ===\n{q}\n--- {} row(s) ---", r.rows.len());
    for row in &r.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|c| match c {
                Some(Term::NamedNode(n)) => n
                    .as_str()
                    .rsplit(['#', '/'])
                    .next()
                    .unwrap_or("")
                    .to_string(),
                Some(Term::Literal(l)) => {
                    let s = l.value();
                    if s.len() > 110 {
                        format!("{}…", &s[..110])
                    } else {
                        s.to_string()
                    }
                }
                Some(t) => t.to_string(),
                None => "(unbound)".into(),
            })
            .collect();
        eprintln!("  {}", cells.join("  |  "));
    }
}

/// Q1 — "what's the merge discipline for a ZK PR?"
/// Minimal answer: the Findings about the ZK + merge-discipline topics, each with its
/// label + the AGENTS.md section it came from + its confidence — sourced, not prose.
#[test]
fn q1_zk_pr_merge_discipline() {
    let g = load_pkg().expect("PKG loads");
    let q = r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?section ?conf WHERE {
  ?f a pkg:Finding ;
     pkg:about ?topic ;
     rdfs:label ?label ;
     dcterms:source ?section ;
     pkg:confidence ?conf .
  FILTER(?topic IN (
    <https://sparq.dev/ns/pkg/kb#topic-zk-discipline>,
    <https://sparq.dev/ns/pkg/kb#topic-merge-discipline>))
} ORDER BY DESC(?conf)"#;
    let r = ask_pkg(&g, q).expect("query runs");
    dump("Q1: what's the merge discipline for a ZK PR?", q, &r);

    // The minimal answer must include the ZK-circuit gate, the privacy-claims gate,
    // and the base ci-summary gate — the load-bearing discipline for a ZK PR.
    let labels: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| match &row[0] {
            Some(Term::Literal(l)) => Some(l.value().to_lowercase()),
            _ => None,
        })
        .collect();
    assert!(!r.rows.is_empty(), "Q1 returned no rows");
    assert!(
        labels.iter().any(|l| l.contains("ci-summary")),
        "Q1 must surface the ci-summary gate; got {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("circuit") || l.contains("gate-count")),
        "Q1 must surface the ZK-circuit gate; got {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("privacy") || l.contains("soundness")),
        "Q1 must surface the privacy-claims gate; got {labels:?}"
    );
    // Every returned Finding carries a section anchor (its provenance) — that is the
    // citation half of the guardrail, queryable.
    assert!(
        r.rows
            .iter()
            .all(|row| matches!(&row[1], Some(Term::Literal(_)))),
        "every Q1 Finding must carry its dcterms:source section anchor"
    );
}

/// Q2 — "which standing rules apply to sub-agents?"
/// Minimal answer: the Findings about the sub-agent-rules topic (the shared contract,
/// worktree isolation, and the proceed-without-greenlight rule), each with its source
/// section, ordered by confidence.
#[test]
fn q2_subagent_standing_rules() {
    let g = load_pkg().expect("PKG loads");
    let q = r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?section ?conf WHERE {
  ?f a pkg:Finding ;
     pkg:about <https://sparq.dev/ns/pkg/kb#topic-subagent-rules> ;
     rdfs:label ?label ;
     dcterms:source ?section ;
     pkg:confidence ?conf .
} ORDER BY DESC(?conf)"#;
    let r = ask_pkg(&g, q).expect("query runs");
    dump("Q2: which standing rules apply to sub-agents?", q, &r);

    let labels: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| match &row[0] {
            Some(Term::Literal(l)) => Some(l.value().to_lowercase()),
            _ => None,
        })
        .collect();
    assert!(
        labels.len() >= 5,
        "Q2 should surface the sub-agent rule set; got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("shared contract")),
        "Q2 must surface the shared contract; got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("worktree")),
        "Q2 must surface worktree isolation; got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("greenlight")),
        "Q2 must surface the proceed-without-greenlight rule; got {labels:?}"
    );
}

/// Q3 — the §4.1 ready-frontier SPARQL over the mechanical bd projection (the
/// dependency half, verified-expressible TODAY). Open/in-progress, no OPEN
/// dependency, not human-gated, not an umbrella. Proves the bd→Task projection is
/// queryable as a read-model.
#[test]
fn q3_ready_frontier() {
    let g = load_pkg().expect("PKG loads");
    let q = r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT (COUNT(DISTINCT ?t) AS ?ready) WHERE {
  ?t a pkg:Task ; pkg:status ?s ; pkg:priority ?prio .
  FILTER(?s IN (pkg:Open, pkg:InProgress))
  FILTER NOT EXISTS { ?t pkg:dependsOn ?b . ?b pkg:status ?bs . FILTER(?bs != pkg:Closed) }
  FILTER NOT EXISTS { ?t pkg:label ?l . FILTER(STRSTARTS(STR(?l), "needs:")) }
  FILTER NOT EXISTS { ?child dcterms:isPartOf ?t }
}"#;
    let r = ask_pkg(&g, q).expect("frontier query runs");
    dump("Q3: §4.1 ready-frontier (dependency half)", q, &r);
    // The frontier is non-empty over a real backlog of 1255 tasks.
    let n = match &r
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|c| c.clone())
    {
        Some(Term::Literal(l)) => l.value().parse::<i64>().unwrap_or(0),
        _ => 0,
    };
    assert!(
        n > 0,
        "the ready-frontier must be non-empty over the bd backlog; got {n}"
    );
    eprintln!("\nQ3 ready-frontier count = {n}");
}
