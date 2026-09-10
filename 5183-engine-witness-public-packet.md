# Issue5183 engine-only witness

{
  "task": "Issue5183 fixed local public-API witness",
  "model": "GPT-6 Astra xhigh",
  "created_utc": "2026-09-10T10:45:15.651565+00:00",
  "head": "781f667c19a8ebb779cfccb24b05ea432360b025",
  "branch": "codex/update-differential-replay",
  "clean": true,
  "result": "Both runtime-equivalent default builds pass the prescribed reduction and controls through both public update paths.",
  "observations": {
    "total": 12,
    "per_variant": 6,
    "lexical_pair": "2 exact lexical terms,2 WHERE rows,2 distinct new blank nodes,2\u21924 total quads.",
    "canonical_eight": "1 term,row,node;1\u21922 quads.",
    "eight_and_nine": "2 terms,rows,nodes;2\u21924 quads.",
    "main_parent_difference_observed": false
  },
  "inference": "For this reduced case, Sparq behavior on both revisions matches the repo/spec term-identity and per-solution blank-node contract. No6478 regression is observed in these executed default-feature cases.",
  "input_precision": "Canonical integer8 is written as an explicit typed literal in this harness, RDF-term-equivalent to the bare8 in captured operation4. Padded008 remains exact. This is a two-operation reduction candidate, not execution of the original10-operation seed.",
  "provenance": {
    "parent": {
      "source_commit": "ed66ef0931fa19dd521fac433870c86a78687a30",
      "compared_commit": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
      "engine": "libsparq_engine-17d69fdf30aada97.rlib",
      "core": "libsparq_core-e9ed6f0c10ca33fd.rlib",
      "equivalence": "Engine/core/lock diff is empty.",
      "engine_sha256": "2135748c75d5afd1ca3731e6d350c5a06b99caf9376b1388fbd472b08febed9d",
      "core_sha256": "9c6042a15b38ebfddaaef9b24e53df598201b72ec631310f201378c0b06ff7c2",
      "binary_sha256": "85bf9b04113307e8361ddfb38eb00ad42fd363de8ad179e973d4c0e7d293a35a",
      "binary_bytes": 5510096,
      "engine_features": [
        "default",
        "digest",
        "parallel",
        "regex"
      ],
      "core_features": [
        "default",
        "parallel"
      ]
    },
    "main": {
      "source_commit": "d07ca79f89e3a8945be46b516c3cd2f770ccd618",
      "compared_commit": "781f667c19a8ebb779cfccb24b05ea432360b025",
      "engine": "libsparq_engine-d082493c198a3f5b.rlib",
      "core": "libsparq_core-127354fe878a2f23.rlib",
      "equivalence": "Engine delta is README and cfg(test) only; saved exact diff. No production difference.",
      "engine_sha256": "2f17e066de1c77dd975ec2367c6e140175a31710685b05d8aadccd04f3b7b395",
      "core_sha256": "37e9d27afb0f2c35c565ce403d6f7c238e5ac40595b49cc2949fa50dc27e98be",
      "binary_sha256": "77a9ca937e28b041df329135a456b0744a3414ef41d9657d2ee10e87203c5dd5",
      "binary_bytes": 5507872,
      "engine_features": [
        "default",
        "digest",
        "parallel",
        "regex"
      ],
      "core_features": [
        "default",
        "parallel"
      ]
    }
  },
  "profile": {
    "opt_level": 3,
    "lto": false,
    "codegen_units": 16,
    "panic": "unwind",
    "rayon_threads": 1,
    "new_dependency_compilation": false
  },
  "first_command_failure": {
    "kind": "Agent-authored scratch harness format-string syntax error; no linking/query execution occurred.",
    "raw_log": "link-parent.log",
    "first_source": "witness-first-format-error.rs",
    "correction": "One extra escaped closing brace in JSON print format. Original source/log remain intact; successful final source and commands separately recorded.",
    "not_a_sample_retry": "No query result was rejected/repeated. This was a compile-only scratch syntax correction; original phase deadline retained."
  },
  "resources": {
    "original_phase_deadline_unix": 1789036925.2970343,
    "phase_elapsed_seconds": 117.17435359954834,
    "maximum_observed_allocated_bytes": 11546624,
    "free_bytes": 8273993728,
    "first_harness_compile_error_preserved": true
  },
  "limits": [
    "Default engine/core feature sets only: no algebra-rewrite/mmap/dict-spill parity with CI claimed.",
    "No direct Oxigraph runtime or canon execution; no full fixed-seed replay; no claim all8 failures have identical cause.",
    "Existing compiled libraries reused with source equivalence and hash evidence; no fresh exact-main/parent engine compilation.",
    "Main core rmeta hash matches prior frozen compiler dependency evidence; its rlib hash was first recorded in readiness and revalidated before/after this run.",
    "No production/comparator edits, source commit, Cargo invocation, network/model call, remote action, cache cleanup, or6481 change.",
    "No commands remain pending."
  ],
  "smallest_next_step": "Execute the pinned Oxigraph raw-term/count reference described in next-reference-plan.json before deciding any comparator/generator policy correction."
}

## Actual fixed-case observations

[
  {
    "variant": "parent",
    "case": "lexical-pair",
    "path": "rebuild",
    "lexical_terms": [
      "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "parent",
    "case": "canonical-eight",
    "path": "rebuild",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 1,
    "fresh_distinct_blank_nodes": 1,
    "before_quads": 1,
    "after_quads": 2
  },
  {
    "variant": "parent",
    "case": "different-values",
    "path": "rebuild",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "parent",
    "case": "lexical-pair",
    "path": "in-place",
    "lexical_terms": [
      "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "parent",
    "case": "canonical-eight",
    "path": "in-place",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 1,
    "fresh_distinct_blank_nodes": 1,
    "before_quads": 1,
    "after_quads": 2
  },
  {
    "variant": "parent",
    "case": "different-values",
    "path": "in-place",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "main",
    "case": "lexical-pair",
    "path": "rebuild",
    "lexical_terms": [
      "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "main",
    "case": "canonical-eight",
    "path": "rebuild",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 1,
    "fresh_distinct_blank_nodes": 1,
    "before_quads": 1,
    "after_quads": 2
  },
  {
    "variant": "main",
    "case": "different-values",
    "path": "rebuild",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "main",
    "case": "lexical-pair",
    "path": "in-place",
    "lexical_terms": [
      "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  },
  {
    "variant": "main",
    "case": "canonical-eight",
    "path": "in-place",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 1,
    "fresh_distinct_blank_nodes": 1,
    "before_quads": 1,
    "after_quads": 2
  },
  {
    "variant": "main",
    "case": "different-values",
    "path": "in-place",
    "lexical_terms": [
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
      "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ],
    "where_rows": 2,
    "fresh_distinct_blank_nodes": 2,
    "before_quads": 2,
    "after_quads": 4
  }
]

## Exact executed harness

```rust
// [GPT-6 ASTRA] Fixed synthetic RDF-term and UPDATE-template conformance witness.
// Rust guideline compliant 2026-02-21
use sparq_core::Graph;
use std::collections::BTreeSet;

const TEMPLATE: &str =
    "INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }";

fn dataset(graph: &Graph) -> Vec<String> {
    let scan = graph.store.scan(&[None, None, None]);
    let mut lines: Vec<String> = scan
        .rows
        .iter()
        .map(|row| {
            let ids = scan.to_spo(row);
            format!(
                "{} {} {} .",
                graph.dict.term(ids[0]),
                graph.dict.term(ids[1]),
                graph.dict.term(ids[2])
            )
        })
        .collect();
    lines.sort();
    lines
}

fn observe(case: &str, terms: &[&str], inplace: bool) {
    let input = terms
        .iter()
        .map(|term| format!("<http://ex/s2> <http://ex/p1> {term} ."))
        .collect::<Vec<_>>()
        .join(" ");
    let insert = format!("INSERT DATA {{ {input} }}");
    let mut graph = Graph::new();
    if inplace {
        sparq_engine::update_in_place(&mut graph, &insert).expect("in-place data insert");
    } else {
        graph = sparq_engine::update(&graph, &insert).expect("rebuild data insert");
    }
    let before = dataset(&graph);
    let result = sparq_engine::query(
        &graph,
        "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }",
    )
    .expect("public WHERE query");
    let mut rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row| row.iter().map(|t| t.as_ref().expect("bound term").to_string()).collect())
        .collect();
    rows.sort();
    let mut expected: Vec<String> = terms.iter().map(|term| (*term).to_string()).collect();
    expected.sort();
    let lexical_terms: Vec<String> = rows.iter().map(|row| row[1].clone()).collect();
    assert_eq!(lexical_terms, expected, "exact RDF lexical identities");
    assert_eq!(before.len(), terms.len());
    assert_eq!(rows.len(), terms.len());
    if inplace {
        sparq_engine::update_in_place(&mut graph, TEMPLATE).expect("in-place template insert");
    } else {
        graph = sparq_engine::update(&graph, TEMPLATE).expect("rebuild template insert");
    }
    let after = dataset(&graph);
    let output = sparq_engine::query(
        &graph,
        "SELECT ?b WHERE { <http://ex/s2> <http://ex/p0> ?b }",
    )
    .expect("public inserted-term query");
    let mut blank_nodes = BTreeSet::new();
    for row in &output.rows {
        let term = row[0].as_ref().expect("bound inserted term");
        assert!(term.is_blank_node(), "template must create blank nodes");
        blank_nodes.insert(term.to_string());
    }
    assert_eq!(output.rows.len(), terms.len());
    assert_eq!(blank_nodes.len(), terms.len(), "fresh blank node per solution");
    assert_eq!(after.len(), 2 * terms.len());
    let path = if inplace { "in-place" } else { "rebuild" };
    let blanks: Vec<_> = blank_nodes.into_iter().collect();
    println!(
        "{{\"case\":{case:?},\"path\":{path:?},\"lexical_terms\":{lexical_terms:?},\"where_rows\":{rows:?},\"blank_nodes\":{blanks:?},\"before\":{before:?},\"after\":{after:?}}}"
    );
}

fn main() {
    let eight = "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let padded = "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let nine = "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    for inplace in [false, true] {
        observe("lexical-pair", &[eight, padded], inplace);
        observe("canonical-eight", &[eight], inplace);
        observe("different-values", &[eight, nine], inplace);
    }
}
```

## Next reference step

{
  "status": "Plan only; no Oxigraph or full-seed execution in this phase.",
  "next_small_step": {
    "scope": "Pinned Oxigraph0.5.9 direct public in-memory Store/update/query witness for the same two operations and two controls. Export raw pre/post quads, WHERE row bindings and distinct new blank nodes; no canonicalizer needed to adjudicate counts.",
    "dependencies": "oxigraph = { version = \"=0.5.9\", default-features = false, features = [\"rdf-12\"] }; use repository Cargo.lock dependency versions and vendored spargebra0.4.6 patch to match CI parser provenance.",
    "why_no_complete_warm_link": "Authorized target has no Oxigraph rlib. Local Oxigraph0.5.9 source is present; no offline dependency-resolution closure/build-size proof exists yet.",
    "proposal_for_root": "Authorize one minimal offline task-local reference build with jobs1, same warm O3/unwind/no-LTO/codegen16 profile, separate measured growth receipt and hard limit chosen by root. Propose10min/512MiB aggregate growth and at least2GiB free, stopping on any missing dependency or bound. These are operational limits, not predicted build consumption.",
    "no_extra_features": "No RocksDB, HTTP-client features, installs, network, workspace/bench-wide build, or dependency version update.",
    "interpretation": "If raw Oxi outputs1/1 for lexical pair while controls behave1/1 and2/2, the history-dependent reference-cardinality explanation is directly established for the reduced case. A contrary result must be retained and investigated, not retried."
  },
  "exact_seed_followup": {
    "scope": "Minimal dedicated entry point for the unchanged update_fuzz.rs module, calling run(4141222487,1) on each source with all original comparator guards.",
    "dependency_surface": "Use engine algebra-rewrite, core mmap+dict-spill, canon rdf12-triple-terms, oxrdf matching workspace features, serde_json and pinned Oxigraph0.5.9; inspect actual module closure and preserve vendored spargebra patch. Omit unrelated query-fuzzer/benchmark modules.",
    "critical_seams": [
      "Preserve env!(CARGO_MANIFEST_DIR)/repository-relative allowlist lookup with equivalent crate layout and committed allowlist; missing file must not silently alter adjudication.",
      "Keep generator, LOAD sandbox/document content and reference INSERT substitute untouched.",
      "Preserve strict rebuild/in-place comparison before Oxi and per-step probes; do not normalize production input or suppress failures.",
      "Capture actual dependency feature/profile/source hashes; warm diagnostic profile is not identical to CI release-fast thin-LTO/abort profile."
    ],
    "execution": "One fixed seed per source, no advancing window/seed hunting. Compare failure step, raw per-engine snapshots and normalized snapshots in a separate observation harness if needed; do not change committed comparator.",
    "resource_gate": "Requires extra cold canon and feature compilation. Return measured readiness after the small reference build rather than assuming the full path fits its cap."
  }
}
