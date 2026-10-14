//! sq-bu7zs — self-relative throughput harness for the sparq-specific terse
//! keyword layer. There is no external peer format: the JSON envelope therefore records
//! `NOT-COMPARABLE` instead of implying a competitor result.
//!
//! The correctness gate runs before either stopwatch. Every generated canonical query must
//! survive compact -> expand byte-identically, and expanding that result must compact to the
//! same terse text. A deliberately corrupted query must make the gate fail, witnessing that
//! the check is non-vacuous.
//!
//! ```text
//! cargo run -p sparq-terse --release --example terse_bench -- --smoke
//! cargo run -p sparq-terse --release --example terse_bench
//! ```
//!
//! The two TSV rows use `<workload>\t<count>\t<us>`. The following single-line JSON envelope
//! carries byte counts and derived docs/s + MB/s; timings from a shared work box are advisory.

use sparq_terse::{legend, terse_to_sparql};
use std::hint::black_box;
use std::time::Instant;

const SMOKE_DOCS: usize = 64;
const DEFAULT_DOCS: usize = 10_000;

fn corpus(count: usize) -> Vec<String> {
    let terms = legend();
    assert!(
        terms.len() >= 3,
        "the frozen legend must contain three entries"
    );
    (0..count)
        .map(|i| {
            let (_, type_iri) = &terms[i % terms.len()];
            let (_, predicate_iri) = &terms[(i + 1) % terms.len()];
            let (_, object_iri) = &terms[(i + 2) % terms.len()];
            format!(
                "SELECT ?s WHERE {{ ?s <{type_iri}> <{object_iri}> ; <{predicate_iri}> ?o . FILTER(?o != <http://example.test/value/{i}>) }}"
            )
        })
        .collect()
}

fn compact(src: &str, terms: &[(&'static str, String)]) -> String {
    let mut terse = src.to_owned();
    for (name, iri) in terms {
        terse = terse.replace(&format!("<{iri}>"), &format!("K:{name}"));
    }
    terse
}

fn identity_gate(canonical: &[String], terms: &[(&'static str, String)]) -> Result<(), String> {
    for (index, source) in canonical.iter().enumerate() {
        let terse = compact(source, terms);
        identity_pair(source, &terse, terms)
            .map_err(|error| format!("document {index}: {error}"))?;
    }
    Ok(())
}

fn identity_pair(
    canonical: &str,
    terse: &str,
    terms: &[(&'static str, String)],
) -> Result<(), String> {
    let expanded = terse_to_sparql(terse).map_err(|error| format!("did not expand: {error}"))?;
    if expanded.canonical_sparql != canonical {
        return Err("changed during compact -> expand".to_owned());
    }
    if compact(&expanded.canonical_sparql, terms) != terse {
        return Err("changed during expand -> compact".to_owned());
    }
    Ok(())
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let count = match args.as_slice() {
        [] => DEFAULT_DOCS,
        [flag] if flag == "--smoke" => SMOKE_DOCS,
        _ => {
            eprintln!("usage: terse_bench [--smoke]");
            std::process::exit(2);
        }
    };

    let terms = legend();
    let canonical = corpus(count);

    // HARD gate before timing. Byte identity is stronger than parsed-query/RDF-term identity.
    identity_gate(&canonical, &terms).unwrap_or_else(|error| {
        eprintln!("terse_bench: ROUND-TRIP IDENTITY GATE RED: {error}");
        std::process::exit(4);
    });

    // Mutation witness: an unknown keyword must be rejected by the same gate.
    let corrupted = compact(&canonical[0], &terms).replacen("K:", "K:notAKeyword_", 1);
    assert!(
        identity_pair(&canonical[0], &corrupted, &terms).is_err(),
        "mutation witness did not turn the identity gate red"
    );

    let compact_start = Instant::now();
    let terse: Vec<String> = canonical
        .iter()
        .map(|source| black_box(compact(black_box(source), black_box(&terms))))
        .collect();
    let compact_us = compact_start.elapsed().as_micros();

    let expand_start = Instant::now();
    let expanded: Vec<String> = terse
        .iter()
        .map(|source| {
            black_box(
                terse_to_sparql(black_box(source))
                    .expect("pre-timing identity gate validated every generated document")
                    .canonical_sparql,
            )
        })
        .collect();
    let expand_us = expand_start.elapsed().as_micros();
    assert_eq!(
        expanded, canonical,
        "timed expansion drifted from the gated output"
    );

    let canonical_bytes: usize = canonical.iter().map(String::len).sum();
    let terse_bytes: usize = terse.iter().map(String::len).sum();
    println!("terse_compact\t{count}\t{compact_us}");
    println!("terse_expand\t{count}\t{expand_us}");

    let rows = [
        ("compact", canonical_bytes, compact_us),
        ("expand", terse_bytes, expand_us),
    ];
    let row_json = rows
        .iter()
        .map(|(workload, bytes, us)| {
            let seconds = (*us).max(1) as f64 / 1_000_000.0;
            format!(
                "{{\"workload\":\"{}\",\"count\":{},\"us\":{},\"input_bytes\":{},\"docs_per_s\":{:.3},\"mb_per_s\":{:.3}}}",
                json_escape(workload),
                count,
                us,
                bytes,
                count as f64 / seconds,
                *bytes as f64 / 1_000_000.0 / seconds
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "TERSE_BENCH_JSON {{\"axis\":\"terse-expand-compact\",\"round_trip_identity\":\"green\",\"mutation_witness\":\"green\",\"competitor_verdict\":\"NOT-COMPARABLE\",\"competitor_reason\":\"sparq-terse is a sparq-specific convenience format with no external peer\",\"canonical\":false,\"quiet_box\":false,\"rows\":[{row_json}]}}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_gate_accepts_corpus_and_rejects_mutation() {
        let terms = legend();
        let docs = corpus(8);
        identity_gate(&docs, &terms).expect("generated corpus must round-trip");

        let mutated = compact(&docs[0], &terms).replacen("K:", "K:unknown_", 1);
        assert!(identity_pair(&docs[0], &mutated, &terms).is_err());
    }
}
