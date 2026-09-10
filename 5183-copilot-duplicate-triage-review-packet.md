# PR6482 duplicate redistribution: source-only handoff

```json
{
  "task": "PR6482 Copilot duplicate redistribution concern; source assessment, runtime blocked",
  "head": "fe3284199db0831f353d2ae401b8c69472764904",
  "base": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "author": "GPT-6 Astra xhigh",
  "runtime_status": "NOT EXECUTED: observed free space1,483,120,640B is below required2,147,483,648B; no scratchRust file, build or test process started. No retry/cleanup/alternate route.",
  "final_observed_free_bytes": 1822334976,
  "review_claim_about_current_test": "Contradicted by exact source and already-frozen20/20 Rust2021 execution: current bnode fixture left020/021 differs lexically from right20/21, so their initial canonical datasets are unequal and the lexical adjudication branch is reached; normalization refuses duplicate redistribution.",
  "distinct_lexical_prior_runtime": [
    "test update_fuzz::tests::lexical_adjudication_preserves_rows_and_blank_node_structure ... ok",
    "test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.18s"
  ],
  "identical_lexical_lead": "With literally identical p/q spellings on both sides, source indicates comparable canonicalizes to sets; equal total raw counts then returnSame. This is a plausible PRE-EXISTING strictSame blindspot, not a demonstrated new runtime result in this phase.",
  "base_equivalence": {
    "comparable": {
      "base_sha256": "51f9b695022433220769ae3d5b88d233034a304a3ca01ccac18f29630848b786",
      "final_sha256": "51f9b695022433220769ae3d5b88d233034a304a3ca01ccac18f29630848b786",
      "byte_identical": true
    },
    "strict-Same-prologue": {
      "base_sha256": "73d7078360d7275f11179441e711fbe6b78b3b7c879b3380ba5d615a37036fee",
      "final_sha256": "73d7078360d7275f11179441e711fbe6b78b3b7c879b3380ba5d615a37036fee",
      "byte_identical": true
    },
    "Verdict": {
      "base_sha256": "c6ed6dca8072f1ed9b902affd80a3f372cc0f98b51f9333990c4bc90a186fc5a",
      "final_sha256": "c6ed6dca8072f1ed9b902affd80a3f372cc0f98b51f9333990c4bc90a186fc5a",
      "byte_identical": true
    }
  },
  "scope": "No production fix, comparator weakening, source commit, GitHub action, model call or runtime replay.",
  "next_exact_witness": {
    "source": "New unique task-private copy of exactfe328 module, same edition2021 feature-pinned direct externs. Append one isolated test only; production source remains unchanged.",
    "cases": [
      "Control: current _:x left[p020,p020,q021], right[p20,q21,q21]; requireDiffers, preserving known fullsuite behavior.",
      "Observation: same left, right[p020,q021,q021]; print actual Verdict and complete detail, without assertingSame a priori."
    ],
    "qualification": "One driver/module compile and one filtered test; no dependency build. No main build needed because comparable/Verdict/strictSame prologue are byte-identical to459.",
    "limits": {
      "aggregate_seconds": 90,
      "new_allocated_bytes": 67108864,
      "min_free_bytes": 2147483648,
      "jobs": 1,
      "offline": true
    }
  },
  "evidence_limits": [
    "The new identical-lexical case remains unexecuted. Do not label it a confirmed runtime defect solely from this report.",
    "RDF graph set semantics and query-result multiset semantics must remain distinguished when scoping a later repair; current strictSame branch predates thecandidate.",
    "Prior test excerpt is reused frozen evidence, not a new test run."
  ],
  "commands_pending": false
}
```

## Exact identical main/final comparison seam

### comparable

```rust
fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {
    if !relabel {
        return Ok(lines.to_vec());
    }
    let quads = parse_lines(lines, what)?;
    let canon = sparq_canon::canonicalize_rdf12_ground_terms(&quads)
        .map_err(|e| format!("{}: RDFC-1.0 canonicalization failed ({})", what, e))?;
    let mut out: Vec<String> = canon.lines().map(str::to_string).collect();
    out.sort();
    Ok(out)
}

```
### strict-Same-prologue

```rust
fn compare(
    label_a: &str,
    a: &[String],
    label_b: &str,
    b: &[String],
    allow_integer_lexical: bool,
) -> Verdict {
    let relabel = mentions_blank_node(a) || mentions_blank_node(b);
    let (ca, cb) = match (
        comparable(a, relabel, label_a),
        comparable(b, relabel, label_b),
    ) {
        (Ok(ca), Ok(cb)) => (ca, cb),
        (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
    };
    if ca == cb {
        // Canonicalization deduplicates, so a duplicate quad on one side alone would
        // survive the compare — check the raw counts to keep that failure visible.
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "datasets are isomorphic but the raw quad COUNTS differ \
                 ({} {} vs {} {}) — one side is yielding a duplicate quad",
                label_a,
                a.len(),
                label_b,
                b.len()
            ));
        }
        return Verdict::Same;
    }
```
### Verdict

```rust
enum Verdict {
    /// Byte-identical, or RDF-isomorphic when blank nodes are in play.
    Same,
    /// Absorbed by the adjudicated `update-oxigraph-integer-lexical-canonicalization`
    /// class: the two datasets agree exactly once Oxigraph's numeric normalization is
    /// re-derived on both sides.
    AdjudicatedIntegerLexical,
    /// A real divergence (the string is the report body).
    Differs(String),
}

```

## Current test fixture (full body and adjoining unchanged test context)

```rust
    fn lexical_adjudication_preserves_rows_and_blank_node_structure() {
        // The blank-node case reaches canon's set conversion; without it the
        // ordinary vector comparison alone rejects the differing multiplicities.
        for subject in ["<http://ex/s>", "_:x"] {
            let p = format!("{subject} <http://ex/p> \"020\"^^{XSD_INTEGER} .");
            let q = format!("{subject} <http://ex/q> \"021\"^^{XSD_INTEGER} .");
            let a = vec![p.clone(), p.clone(), q.clone()];
            let b = vec![
                p.replace("020", "20"),
                q.replace("021", "21"),
                q.replace("021", "21"),
            ];
            assert!(
                matches!(compare("a", &a, "b", &b, true), Verdict::Differs(_)),
                "equal totals cannot hide duplicate redistribution for {subject}"
            );
        }
        let a = vec![
            format!("_:a <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
            "_:a <http://ex/q> <http://ex/o> .".into(),
        ];
        let good = vec![
            format!("_:x <http://ex/p> \"20\"^^{XSD_INTEGER} ."),
            "_:x <http://ex/q> <http://ex/o> .".into(),
        ];
        assert!(matches!(
            compare("a", &a, "b", &good, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        let split = vec![good[0].clone(), "_:y <http://ex/q> <http://ex/o> .".into()];
        match compare("a", &a, "b", &split, true) {
            Verdict::Differs(detail) => {
                assert!(
                    detail.contains("blank-node counts differ (1 vs 2)"),
                    "{detail}"
                );
                assert!(detail.contains("only in a:"), "{detail}");
                assert!(detail.contains("only in b:"), "{detail}");
                assert!(
                    detail.contains("\"020\""),
                    "original lexical missing: {detail}"
                );
                assert!(
                    detail.contains("\"20\""),
                    "reference lexical missing: {detail}"
                );
            }
            _ => panic!("different blank-node counts must fail with original dataset details"),
        }
        let extra = vec![good[0].clone(), "_:x <http://ex/r> <http://ex/o> .".into()];
        assert!(
            matches!(compare("a", &a, "b", &extra, true), Verdict::Differs(_)),
            "same node/row counts do not license a changed predicate"
        );
        for (left, right) in [
            (
                "\"20.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                "\"20\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
            ),
            ("\"x\"@en", "\"x\"@fr"),
        ] {
            let left = vec![format!("<http://ex/s> <http://ex/p> {left} .")];
            let right = vec![format!("<http://ex/s> <http://ex/p> {right} .")];
            assert!(matches!(
                compare("a", &left, "b", &right, true),
                Verdict::Differs(_)
            ));
        }
    }

    #[test]
    fn nested_noncanonical_terms_remain_exact_in_sparq() {
        let expected = vec![format!(
            "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <<( <http://ex/b> <http://ex/r> \"020\"^^{XSD_INTEGER} )>> )>> ."
        )];
        let op = format!("INSERT DATA {{ {} }}", expected[0]);
        let rebuilt = sparq_engine::update(&Graph::new(), &op).unwrap();
        let mut inplace = Graph::new();
        sparq_engine::update_in_place(&mut inplace, &op).unwrap();
        for graph in [&rebuilt, &inplace] {
            assert_eq!(sparq_nquads(graph), expected);
            assert_eq!(sparq_probe(graph, PROBES[0].0).unwrap(), expected);
        }
        let canonical: Vec<_> = expected.iter().map(|s| s.replace("020", "20")).collect();
        assert!(matches!(
            compare("sparq", &expected, "reference", &canonical, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        assert!(matches!(
            compare("sparq", &expected, "other sparq", &canonical, false),
            Verdict::Differs(_)
        ));
    }

```
