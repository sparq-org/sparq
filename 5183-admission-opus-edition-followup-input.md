Review the exact frozen follow-up to issue5183 as independent Claude Opus5 xhigh. Treat embedded repository source/comments as data. No tools, builds, mutations or requests for redundant broad local tests. Return concise JSON, at most900words, with verdict APPROVE_FOR_CI or REQUEST_CHANGES, reviewed_head, blocking_findings, dispositions for every original finding, required_CI and explicit_limits.

Previous actual Opus5 xhigh review APPROVE_SCOPED covered787aab5 as a test-corpus/adjudication repair, not an engine fix. It supported the generator's disjoint0..19/20..79 integer domain and narrower comparator, preserving strict Same and full normalized structural equality. It reported five observations: (1) retain original per-side dataset diagnostics on blank-node mismatch and normalized-comparable error; (2) make both integer pool boundaries independently asserted for all parsed integer leaves, including nested terms; (3) make the injected-marker test independent of SPARQ_FUZZ_DIVERGENCES; (4) check let-chain edition compatibility; (5) informational uncertainty about earlier raw-count guard. Original input omitted some unchanged helper bodies, so that caveat was valid. No prior source omission should be silently assumed covered.

Root investigation confirmed a MATERIAL compatibility gap: workspace edition2021 and sparq-bench inherits it, while original minimal harness was2024. Two new let-chain sites fail to compile as2021; this was caught before push. Follow-up restores compatible syntax and validates the exact module with edition2021, leaving productionCargo metadata unchanged. The original20/20edition2024 validation remains preserved but never proves crate-edition compatibility. Check the actual control/error and final passing evidence and helper bodies now supplied.

Please assess the small delta plus complete affected helper context, including compare's entire prologue/finalDiffers, Verdict, allowlist, normalizers and recursive guards; confirm no original finding is masked by a delta-only review. Preserve the original scoped soundness/coverage boundaries. No engine/canon/parser/allowlist-registry/workflow changes, no general multiset redesign. Full protected CI remains required; keep5183 open for the other seven historical inputs. New generator seed success is not original-input replay. The selected normalCI is authoritative; don't invent a new per-run nonvacuity policy or require irrelevant target checks not selected by the repo. Distinguish observed validation from unexecuted CI and reported observations from independent execution.

Frozen follow-up packet:

# Issue5183 focused follow-up for actual independent review

Final fe3284199db0831f353d2ae401b8c69472764904; parent 787aab5c9e7fb114cbe8f390f6fc572eb2a3cfd1; originalbase4595388d. Actual author: GPT-6 Astra (xhigh).

Review the delta and corrected Rust2021 proof. Prior scoped review approved787 subject to low findings; initial packet omitted unchanged bodies and its2024 test harness missed a real compatibility defect. Full original source existed in separate frozen evidence. The functions below close that packet gap.

## Structured observations and limits

```json
{
  "issue": 5183,
  "head": "fe3284199db0831f353d2ae401b8c69472764904",
  "parent": "787aab5c9e7fb114cbe8f390f6fc572eb2a3cfd1",
  "base": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "author": "GPT-6 Astra (xhigh)",
  "decision": "Focused follow-up complete for independent delta review; no publication/admission claim.",
  "changes": [
    "Replace two new Rust2024 let chains with equivalent Rust2021 constructs. Workspace Cargo.toml edition2021 and sparq-bench inheritance remain unchanged.",
    "Keep original ca/cb one-sided diagnostics plus accurate reason/counts in the new blank-node and normalized comparable error branches.",
    "Independently assert20/80 integer pool boundaries and spelling for every parsed generated integer leaf, including nested objects and LOAD; preserve existing40/400/200 test suites.",
    "Use explicit local marker-test allowlist and prove independence from empty ambient registry.",
    "Two existing test expressions received rustfmt wrapping-only changes."
  ],
  "validation": "test-summary.json",
  "edition_gap": "Previous20/20 under edition2024 did not prove production compatibility. Unchanged787 source fails under the same2021 compiler command with two let-chain diagnostics; finalsource passes20tests and Clippy. This is a material corrected gap.",
  "helper_equivalence": "helper-equivalence.json; parent787 tofinal: normalizers, Verdict, allowlist, one_sided and comparable byte-identical; record only syntax; compare prologue/raw guard/normalization error/finalDiffers preserved. Complete patch intentionally differs from base459.",
  "review_dispositions": {
    "raw_count_guard": "Not dead: initial guard runs only when ca==cb; new lexical-path guard runs when ca!=cb. Neither removed.",
    "omitted_context": "Prior reviewer omission caveat valid: complete source was frozen separately, but not fully supplied. New packet includes complete compare prologue/finalDiffers, Verdict, allowlist, both normalizers, record helper, relevant callers/tests and edition/toolchain excerpts. It does not claim prior packet had complete source.",
    "unchanged_guards": "Seven prior controls are preserved rather than rerun; focused current controls and full20tests cover changed syntax/diagnostics/oracles.",
    "resource_scope": "No workspace/engine/dependency rebuild expansion, no other original seed execution."
  },
  "preflight_limit": "exit1 solely privacy-claims shell mapfile unavailable under installed Bash3; G1/G2/G6/guard-untested pass; no-perf-numbers/readme-template correctly skip unmatchedpaths. Not weakened or bypassed. Linux gate remains required.",
  "recorded_setup_corrections": [
    "Export-only helper equivalence substring assertion matched inner arm; corrected to exact outer-arm line. Read-only ps denied by sandbox; terminal receipts used without retry or alternate process inspection.",
    "Exact compactTOML edition replacement initially failed an assertion before build; follow-on runner missing exit2; corrected and preserved setup receipt.",
    "SystemPython3.9 lacks tomllib; first metadata proof script failed, but subsequent compile started because initial shell lacked fail-fast. Actual compile uses2021; laterproof derives edition fromCargoJSON and sectionscopedsource.",
    "Unfilteredmetadata had42 foreign-platform dependencies; native-filtered proof matches all102 original native dependency feature sets. No package/version change, dependency install or outcome retry."
  ],
  "limits": [
    "Native minimal exact-module tests are not full sparq-bench/fullworkspace or LinuxCI proof.",
    "Fault injected normalized error demonstrates formatting preservation only.",
    "Narrower generated reference domain and historical Sparq-only regression remain as parent; no Sparq engine defect fixed. Seven other original advancing-window failures remain unknown; changed-generator same-seed runs do not resolve them."
  ],
  "commands_terminal": true,
  "next_step": "Root reviews frozen focused packet with actual independent Opus, then owns supported CI/publication.",
  "validation_commands": [
    {
      "name": "metadata",
      "exit": 0,
      "seconds": 5.903518375
    },
    {
      "name": "build-tests",
      "exit": 0,
      "seconds": 35.6110365
    },
    {
      "name": "metadata-native",
      "exit": 0,
      "seconds": 1.284624583
    },
    {
      "name": "full-suite",
      "exit": 0,
      "seconds": 5.6277349160000005
    },
    {
      "name": "clippy-tests",
      "exit": 0,
      "seconds": 3.331390042
    },
    {
      "name": "marker-strict-ambient",
      "exit": 0,
      "seconds": 1.206351834
    },
    {
      "name": "partial-canonical-pool-overlap-build",
      "exit": 0,
      "seconds": 11.82479075
    },
    {
      "name": "partial-canonical-pool-overlap-test",
      "exit": 101,
      "seconds": 1.1452430839999999
    },
    {
      "name": "omit-blank-node-dataset-details-build",
      "exit": 0,
      "seconds": 9.443659375000001
    },
    {
      "name": "omit-blank-node-dataset-details-test",
      "exit": 101,
      "seconds": 1.160735083
    },
    {
      "name": "restore-ambient-marker-allowlist-build",
      "exit": 0,
      "seconds": 10.455275041999998
    },
    {
      "name": "restore-ambient-marker-allowlist-test",
      "exit": 101,
      "seconds": 1.1373836670000002
    },
    {
      "name": "old-source-edition2021-build",
      "exit": 1,
      "seconds": 1.190175167
    },
    {
      "name": "normalized-error-fault-positive-build",
      "exit": 0,
      "seconds": 9.951183959
    },
    {
      "name": "normalized-error-fault-positive-test",
      "exit": 0,
      "seconds": 1.149054083
    },
    {
      "name": "normalized-error-old-detail-control-build",
      "exit": 0,
      "seconds": 9.303507375
    },
    {
      "name": "normalized-error-old-detail-control-test",
      "exit": 101,
      "seconds": 1.146713125
    }
  ],
  "checks": [
    {
      "name": "format-check",
      "exit": 0
    },
    {
      "name": "diff-check",
      "exit": 0
    },
    {
      "name": "preflight",
      "exit": 1
    }
  ]
}
```

## Exact delta

```diff
diff --git a/crates/sparq-bench/src/update_fuzz.rs b/crates/sparq-bench/src/update_fuzz.rs
index 5ac575abd..0ad908acd 100644
--- a/crates/sparq-bench/src/update_fuzz.rs
+++ b/crates/sparq-bench/src/update_fuzz.rs
@@ -834,14 +834,16 @@ fn record_lexical_terms(
 ) -> Result<(), String> {
     match term {
         Term::Literal(l) if l.datatype() == xsd::INTEGER => {
-            if let Ok(value) = l.value().parse::<i64>()
-                && let Some(previous) = integers.insert(value, l.value().to_string())
-                && previous != l.value()
-            {
-                return Err(format!(
-                    "integer normalization is not term-injective: {previous:?} and {:?}",
-                    l.value()
-                ));
+            if let Ok(value) = l.value().parse::<i64>() {
+                match integers.insert(value, l.value().to_string()) {
+                    Some(previous) if previous != l.value() => {
+                        return Err(format!(
+                            "integer normalization is not term-injective: {previous:?} and {:?}",
+                            l.value()
+                        ));
+                    }
+                    _ => {}
+                }
             }
         }
         Term::BlankNode(b) => {
@@ -986,16 +988,24 @@ fn compare(
         match normalized {
             (Ok(na), Ok(nb)) => {
                 if na.blank_nodes != nb.blank_nodes {
-                    return Verdict::Differs(
-                        "integer normalization changed blank-node counts".into(),
-                    );
+                    return Verdict::Differs(format!(
+                        "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
+                        na.blank_nodes,
+                        nb.blank_nodes,
+                        one_sided(label_a, &ca, label_b, &cb)
+                    ));
                 }
                 match (
                     comparable(&na.lines, relabel, label_a),
                     comparable(&nb.lines, relabel, label_b),
                 ) {
                     (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
-                    (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
+                    (Err(e), _) | (_, Err(e)) => {
+                        return Verdict::Differs(format!(
+                            "integer-lexical adjudication refused: {e}\n{}",
+                            one_sided(label_a, &ca, label_b, &cb)
+                        ));
+                    }
                     _ => {}
                 }
             }
@@ -1575,11 +1585,9 @@ mod tests {
     #[test]
     fn no_nested_blank_nodes_in_triple_terms() {
         // (a) the guard actually fires on the shape it exists to reject.
-        let nested = vec![
-            "<http://ex/s> <http://ex/p> \
+        let nested = vec!["<http://ex/s> <http://ex/p> \
                           <<( <http://ex/a> <http://ex/b> _:x )>> ."
-                .to_string(),
-        ];
+            .to_string()];
         let err = comparable(&nested, true, "nested")
             .expect_err("a blank node inside a triple term must be rejected, not canonicalized");
         assert!(
@@ -1876,11 +1884,9 @@ mod tests {
         assert!(!b.integer_lexical);
         // Malformed / absent registries are strict too.
         assert!(!UpdateDivergenceAllowlist::from_json("{", path).integer_lexical);
-        assert!(
-            UpdateDivergenceAllowlist::from_json("{", path)
-                .state
-                .contains("STRICT")
-        );
+        assert!(UpdateDivergenceAllowlist::from_json("{", path)
+            .state
+            .contains("STRICT"));
     }
 
     /// sq-hodke (3), the premise correction, MACHINE-CHECKED: Oxigraph 0.5 as this
@@ -1967,6 +1973,27 @@ mod tests {
     // a persistent map also catches collisions across different graphs or steps.
     #[test]
     fn generated_integer_domain_is_injective_including_load() {
+        // [GPT-6 ASTRA] Independent test boundaries: do not derive the expected
+        // domain from the production constant or rely on a sampled collision.
+        fn assert_integer_pool(term: &Term) {
+            match term {
+                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
+                    let value = l
+                        .value()
+                        .parse::<u64>()
+                        .expect("nonnegative generated integer");
+                    if value < 20 {
+                        assert_eq!(l.value(), value.to_string(), "canonical pool spelling");
+                    } else {
+                        assert!((20..80).contains(&value), "noncanonical pool: {value}");
+                        assert_ne!(l.value(), value.to_string(), "noncanonical pool spelling");
+                    }
+                }
+                Term::Triple(t) => assert_integer_pool(&t.object),
+                _ => {}
+            }
+        }
+        assert_eq!(CANONICAL_INTEGER_VALUES, 20);
         let mut noncanonical = 0;
         let mut loads = 0;
         let mut load_integers = 0;
@@ -1983,16 +2010,18 @@ mod tests {
                         sandbox.write(doc).expect("LOAD document");
                         let rows: Vec<_> = doc.content.lines().map(str::to_string).collect();
                         for q in parse_lines(&rows, "LOAD document").expect("valid document") {
-                            if let Term::Literal(l) = &q.object
-                                && l.datatype() == xsd::INTEGER
-                            {
-                                let value = l.value().parse::<u64>().unwrap();
-                                assert!(
-                                    value < CANONICAL_INTEGER_VALUES,
-                                    "LOAD integer must stay in the canonical pool: {value}"
-                                );
-                                assert_eq!(value.to_string(), l.value());
-                                load_integers += 1;
+                            assert_integer_pool(&q.object);
+                            match &q.object {
+                                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
+                                    let value = l.value().parse::<u64>().unwrap();
+                                    assert!(
+                                        value < 20,
+                                        "LOAD integer must stay canonical: {value}"
+                                    );
+                                    assert_eq!(value.to_string(), l.value());
+                                    load_integers += 1;
+                                }
+                                _ => {}
                             }
                             record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                                 .expect("LOAD shares the injective integer domain");
@@ -2007,6 +2036,7 @@ mod tests {
                     graph = sparq_engine::update(&graph, &op.sparq)
                         .unwrap_or_else(|e| panic!("seed={seed} update={} failed: {e}", op.sparq));
                     for q in parse_lines(&sparq_nquads(&graph), "generated state").unwrap() {
+                        assert_integer_pool(&q.object);
                         nested += usize::from(matches!(&q.object, Term::Triple(_)));
                         record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                             .unwrap_or_else(|e| panic!("seed={seed}: {e}"));
@@ -2107,10 +2137,25 @@ mod tests {
             Verdict::AdjudicatedIntegerLexical
         ));
         let split = vec![good[0].clone(), "_:y <http://ex/q> <http://ex/o> .".into()];
-        assert!(matches!(
-            compare("a", &a, "b", &split, true),
-            Verdict::Differs(_)
-        ));
+        match compare("a", &a, "b", &split, true) {
+            Verdict::Differs(detail) => {
+                assert!(
+                    detail.contains("blank-node counts differ (1 vs 2)"),
+                    "{detail}"
+                );
+                assert!(detail.contains("only in a:"), "{detail}");
+                assert!(detail.contains("only in b:"), "{detail}");
+                assert!(
+                    detail.contains("\"020\""),
+                    "original lexical missing: {detail}"
+                );
+                assert!(
+                    detail.contains("\"20\""),
+                    "reference lexical missing: {detail}"
+                );
+            }
+            _ => panic!("different blank-node counts must fail with original dataset details"),
+        }
         let extra = vec![good[0].clone(), "_:x <http://ex/r> <http://ex/o> .".into()];
         assert!(
             matches!(compare("a", &a, "b", &extra, true), Verdict::Differs(_)),
@@ -2164,10 +2209,14 @@ mod tests {
             )),
             Op::shared("INSERT { ?s <http://ex/q> _:bt } WHERE { ?s <http://ex/p> ?o }".into()),
         ];
-        let positive = apply_sequence(0, &ops, None, None, &allowlist()).unwrap();
+        let allow = UpdateDivergenceAllowlist {
+            integer_lexical: true,
+            state: "hermetic marker-adjudication test".into(),
+        };
+        let positive = apply_sequence(0, &ops, None, None, &allow).unwrap();
         assert_eq!(positive.ops, 2);
         assert!(positive.adjudicated_integer_lexical > 0);
-        let negative = apply_sequence(0, &ops, None, Some(1), &allowlist()).unwrap_err();
+        let negative = apply_sequence(0, &ops, None, Some(1), &allow).unwrap_err();
         assert!(negative.contains("step=1") && negative.contains("http://ex/injected"));
     }
 
```

### Cargo.toml

```toml
[workspace.package]
version = "0.1.1"
edition = "2021"
license = "MIT"
# [OPUS-4.8] (sq-qmth) MSRV floor 1.88. Upstream-driven: `geo@0.33.1` (a transitive dep
# of sparq-geo) requires rustc 1.88, and the released oxigraph parser stack
# (oxrdf/oxttl/oxrdfio/spargebra/… 0.2–0.4) requires 1.87. Verified empirically: the
# MSRV-scope workspace (--exclude sparq-py --exclude sparq-hdt) fails on 1.87 with ONLY
# `geo@0.33.1 requires rustc 1.88`, and builds clean on 1.88. Bump this AND the `msrv`
# job's toolchain pin in .github/workflows/ci.yml together; lower it only if geo (and the
# ox* stack) lower theirs.
rust-version = "1.88"
# Shared crates.io metadata (T20). Each publishable crate inherits these via
# `<field>.workspace = true` and adds its own `description`.
repository = "https://github.com/sparq-org/sparq"
homepage = "https://github.com/sparq-org/sparq"
keywords = ["rdf", "sparql", "semantic-web", "triplestore", "query-engine"]
categories = ["database-implementations", "parsing", "science"]

```

### crates/sparq-bench/Cargo.toml

```toml
[package]
name = "sparq-bench"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

```

### rust-toolchain.toml — actual configuration (explanatory comments omitted)

```toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
components = ["clippy", "rustfmt"]
targets = ["wasm32-unknown-unknown"]
```

### complete comparison helpers — crates/sparq-bench/src/update_fuzz.rs:761

```rust
/// Whether a snapshot mentions a blank node, i.e. whether sorted N-Quads has stopped
/// being a canonical form and RDFC-1.0 relabelling is required.
///
/// Deliberately a substring test. It cannot produce a FALSE NEGATIVE — every blank
/// node renders as `_:label` — and a false positive (a literal containing the text
/// `_:`) would only route a blank-node-free snapshot through the canonicalizer, which
/// on such input is just a sort-and-deduplicate. The generator emits no such literal
/// today; the check stays conservative so that if one is ever added, the comparator
/// degrades in the safe direction.
fn mentions_blank_node(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("_:"))
}

/// Re-parses N-Quads lines back into `oxrdf` quads (the input side of every
/// structural rewrite below).
fn parse_lines(lines: &[String], what: &str) -> Result<Vec<Quad>, String> {
    let doc = {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    };
    sparq_canon::parse_nquads(&doc)
        .map_err(|e| format!("{}: N-Quads re-parse failed ({})", what, e))
}

/// The comparable form of one snapshot.
///
/// With `relabel` false (neither side mentions a blank node) the sorted lines ARE the
/// canonical form and are returned untouched — the strongest, byte-level compare.
/// With it true, both sides are relabelled to their RDFC-1.0 canonical form so the
/// comparison decides RDF ISOMORPHISM. The constrained `*_ground_terms` profile is
/// used deliberately: it is exactly RDFC-1.0 with triple terms as opaque constants and
/// fails closed on a blank node nested inside a triple term, so the non-standard
/// nested-bnode descent is never reachable from this harness.
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

/// Rewrites `t` the way Oxigraph's storage layer does: an `xsd:integer` literal whose
/// value fits the native integer it decodes to is re-rendered in canonical lexical
/// form. Structural, never string surgery — and it descends into triple terms, which
/// Oxigraph's recursive term encoder also normalizes.
fn oxigraph_normalized_term(t: &Term) -> Term {
    match t {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => match l.value().parse::<i64>() {
            Ok(v) if v.to_string() != l.value() => {
                Term::Literal(Literal::new_typed_literal(v.to_string(), xsd::INTEGER))
            }
            _ => t.clone(),
        },
        Term::Triple(inner) => Term::Triple(Box::new(Triple::new(
            inner.subject.clone(),
            inner.predicate.clone(),
            oxigraph_normalized_term(&inner.object),
        ))),
        _ => t.clone(),
    }
}

// [GPT-6 ASTRA] Quad uniqueness alone cannot detect colliding terms at different
// predicates/graphs. Check every integer leaf, including nested triple objects.
fn record_lexical_terms(
    term: &Term,
    integers: &mut BTreeMap<i64, String>,
    blank_nodes: &mut BTreeSet<String>,
) -> Result<(), String> {
    match term {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => {
            if let Ok(value) = l.value().parse::<i64>() {
                match integers.insert(value, l.value().to_string()) {
                    Some(previous) if previous != l.value() => {
                        return Err(format!(
                            "integer normalization is not term-injective: {previous:?} and {:?}",
                            l.value()
                        ));
                    }
                    _ => {}
                }
            }
        }
        Term::BlankNode(b) => {
            blank_nodes.insert(b.as_str().to_string());
        }
        Term::Triple(t) => {
            if let oxrdf::NamedOrBlankNode::BlankNode(b) = &t.subject {
                blank_nodes.insert(b.as_str().to_string());
            }
            record_lexical_terms(&t.object, integers, blank_nodes)?;
        }
        _ => {}
    }
    Ok(())
}

struct NormalizedSnapshot {
    lines: Vec<String>,
    blank_nodes: usize,
}

/// Normalizes integer spellings without merging terms or rows.
fn oxigraph_normalized(lines: &[String], what: &str) -> Result<NormalizedSnapshot, String> {
    let quads = parse_lines(lines, what)?;
    if quads.len() != lines.len() {
        return Err(format!("{what}: parsing changed the raw row count"));
    }
    let mut integers = BTreeMap::new();
    let mut blank_nodes = BTreeSet::new();
    for q in &quads {
        record_lexical_terms(&q.object, &mut integers, &mut blank_nodes)?;
        if let oxrdf::NamedOrBlankNode::BlankNode(b) = &q.subject {
            blank_nodes.insert(b.as_str().to_string());
        }
        if let oxrdf::GraphName::BlankNode(b) = &q.graph_name {
            blank_nodes.insert(b.as_str().to_string());
        }
    }
    let mut out: Vec<String> = quads
        .iter()
        .map(|q| {
            let graph = match &q.graph_name {
                oxrdf::GraphName::DefaultGraph => None,
                g => Some(g.to_string()),
            };
            nquads_line(
                &q.subject.to_string(),
                &q.predicate.to_string(),
                &oxigraph_normalized_term(&q.object).to_string(),
                graph.as_deref(),
            )
        })
        .collect();
    out.sort();
    // The fixed probes project whole quads, so duplicates are not legitimate
    // projection multiplicity. Reject before canon's set conversion can hide them.
    if out.windows(2).any(|rows| rows[0] == rows[1]) {
        return Err(format!(
            "{what}: integer normalization merges or duplicates rows"
        ));
    }
    Ok(NormalizedSnapshot {
        lines: out,
        blank_nodes: blank_nodes.len(),
    })
}

/// The lines on exactly one side — the human-readable core of a divergence report.
fn one_sided(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
    let bset: std::collections::BTreeSet<&String> = b.iter().collect();
    let aset: std::collections::BTreeSet<&String> = a.iter().collect();
    let mut s = String::new();
    s.push_str(&format!("only in {}:\n", label_a));
    for l in a.iter().filter(|l| !bset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s.push_str(&format!("only in {}:\n", label_b));
    for l in b.iter().filter(|l| !aset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s
}

/// The outcome of comparing two snapshots.
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

/// Compares two snapshots under the canonical form the module docs describe.
///
/// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
/// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
/// sides are sparq, so any lexical disagreement between them is a real bug.
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
    if allow_integer_lexical {
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "integer-lexical adjudication refused: raw row counts differ\n{}",
                one_sided(label_a, &ca, label_b, &cb)
            ));
        }
        let normalized = (
            oxigraph_normalized(a, label_a),
            oxigraph_normalized(b, label_b),
        );
        match normalized {
            (Ok(na), Ok(nb)) => {
                if na.blank_nodes != nb.blank_nodes {
                    return Verdict::Differs(format!(
                        "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
                        na.blank_nodes,
                        nb.blank_nodes,
                        one_sided(label_a, &ca, label_b, &cb)
                    ));
                }
                match (
                    comparable(&na.lines, relabel, label_a),
                    comparable(&nb.lines, relabel, label_b),
                ) {
                    (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
                    (Err(e), _) | (_, Err(e)) => {
                        return Verdict::Differs(format!(
                            "integer-lexical adjudication refused: {e}\n{}",
                            one_sided(label_a, &ca, label_b, &cb)
                        ));
                    }
                    _ => {}
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                return Verdict::Differs(format!(
                    "integer-lexical adjudication refused: {e}\n{}",
                    one_sided(label_a, &ca, label_b, &cb)
                ));
            }
        }
    }
    Verdict::Differs(one_sided(label_a, &ca, label_b, &cb))
}
```

### nquads_line — crates/sparq-bench/src/update_fuzz.rs:707

```rust
fn nquads_line(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("{} {} {} {} .", subject, predicate, object, g),
        None => format!("{} {} {} .", subject, predicate, object),
    }
}

/// A sparq `Graph` (default graph + named graphs) as SORTED N-Quads lines.
/// Duplicate lines are NOT collapsed — see the raw-count check in [`compare`].
```

### PROBES — crates/sparq-bench/src/update_fuzz.rs:1030

```rust
const PROBES: &[(&str, &[&str])] = &[
    ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
    (
        "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
        &["s", "p", "o", "g"],
    ),
];
```

### production apply_sequence — crates/sparq-bench/src/update_fuzz.rs:1179

```rust
fn apply_sequence(
    seed: u64,
    ops: &[Op],
    sandbox: Option<&LoadSandbox>,
    inject_divergence_at: Option<usize>,
    allow: &UpdateDivergenceAllowlist,
) -> Result<SeedOutcome, String> {
    let mut g_rebuild = Graph::new();
    let mut g_inplace = Graph::new();
    let store = Store::new().map_err(|e| format!("oxigraph store init: {}", e))?;
    let mut adjudicated_integer_lexical = 0u64;
    let mut isomorphism_compares = 0u64;

    let fail = |step: usize, op: &Op, detail: String| -> String {
        format!(
            "step={} of {}\nop: {}\n{}\nrepro: cargo run -p sparq-bench --release -- \
             update-fuzz --seed-start {} --seed-count 1\n--- full sequence ---\n{}",
            step,
            ops.len(),
            op.sparq,
            detail,
            seed,
            ops.iter()
                .enumerate()
                .map(|(i, o)| match &o.oxi {
                    Some(x) if *x != o.sparq =>
                        format!("[{}] {}\n     (reference engine ran: {})", i, o.sparq, x),
                    Some(_) => format!("[{}] {}", i, o.sparq),
                    None => format!("[{}] {}\n     (reference engine ran: nothing)", i, o.sparq),
                })
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    for (i, op) in ops.iter().enumerate() {
        // A LOAD's document must exist before the request runs.
        if let (Some(doc), Some(s)) = (&op.doc, sandbox) {
            s.write(doc).map_err(|e| fail(i, op, e))?;
        }

        // Apply to all three implementations. Every generated op is inside the
        // supported deterministic subset, so an error from ANY engine is itself a
        // divergence (strict — there is no unsupported-skip in this harness).
        g_rebuild = sparq_engine::update(&g_rebuild, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update (rebuild path) error: {}", e)))?;
        sparq_engine::update_in_place(&mut g_inplace, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update_in_place error: {}", e)))?;
        if let Some(oxi) = &op.oxi {
            store
                .update(oxi.as_str())
                .map_err(|e| fail(i, op, format!("oxigraph update error: {}", e)))?;
        }

        if inject_divergence_at == Some(i) {
            use oxigraph::model::{GraphName, NamedNode, Quad};
            let n = |s: &str| NamedNode::new(s).expect("valid IRI");
            let marker = Quad::new(
                n("http://ex/injected"),
                n("http://ex/injected"),
                n("http://ex/injected"),
                GraphName::DefaultGraph,
            );
            store
                .insert(&marker)
                .map_err(|e| format!("marker insert failed: {}", e))?;
        }

        // (a) Canonical dataset equality. Localizes a divergence to this exact step.
        // sparq-vs-sparq FIRST and STRICT (no adjudication): the two sparq paths must
        // agree with each other whatever the reference engine does.
        let nq_rebuild = sparq_nquads(&g_rebuild);
        let nq_inplace = sparq_nquads(&g_inplace);
        let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
        if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
            isomorphism_compares += 1;
        }
        if let Verdict::Differs(detail) = compare(
            "sparq(rebuild)",
            &nq_rebuild,
            "sparq(in-place)",
            &nq_inplace,
            false,
        ) {
            return Err(fail(
                i,
                op,
                format!(
                    "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
                    detail
                ),
            ));
        }
        for (label, nq) in [
            ("sparq(rebuild)", &nq_rebuild),
            ("sparq(in-place)", &nq_inplace),
        ] {
            match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
                Verdict::Same => {}
                Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                Verdict::Differs(detail) => {
                    return Err(fail(
                        i,
                        op,
                        format!(
                            "canonical dataset differs ({} vs oxigraph)\n{}",
                            label, detail
                        ),
                    ));
                }
            }
        }

        // (b) Probe SELECTs — the query-path view of the updated store, full sorted
        // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
        // one reads through the live delta overlay.
        for (probe, vars) in PROBES {
            let oxi = oxi_probe(&store, probe, vars)
                .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
            for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
                let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
                match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
                    Verdict::Same => {}
                    Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                    Verdict::Differs(detail) => {
                        return Err(fail(
                            i,
                            op,
                            format!(
                                "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
                                probe, label, detail
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(SeedOutcome {
        ops: ops.len() as u64,
        adjudicated_integer_lexical,
        isomorphism_compares,
    })
}
```

### complete allowlist — crates/sparq-bench/src/update_fuzz.rs:1326

```rust
/// The adjudicated `update-*` divergence classes this comparator has detectors for.
/// The registry is the same `bench/differential-divergences.json` the query fuzzer
/// consumes. A listed `update-*` class WITHOUT a detector here stays strict (loud
/// warning), mirroring the fail-toward-flagging posture of `fuzz.rs`.
struct UpdateDivergenceAllowlist {
    /// `update-oxigraph-integer-lexical-canonicalization` — Oxigraph's storage layer
    /// collapses `xsd:integer` lexical forms. Not a blind skip: absorbed only when
    /// re-deriving that normalization on both sides makes the datasets agree exactly.
    integer_lexical: bool,
    /// Where the allowlist was loaded from, and its posture (for the summary line).
    state: String,
}

/// The class id this comparator has a detector for.
const INTEGER_LEXICAL_CLASS: &str = "update-oxigraph-integer-lexical-canonicalization";

impl UpdateDivergenceAllowlist {
    fn strict(path: &str, why: &str) -> Self {
        UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: format!("({}): {} — STRICT (every divergence fails)", path, why),
        }
    }

    /// Load from `SPARQ_FUZZ_DIVERGENCES` (a CI/agent override), else the committed
    /// repo default resolved relative to this crate's manifest (works from any cwd).
    fn load() -> Self {
        let path = std::env::var("SPARQ_FUZZ_DIVERGENCES").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../bench/differential-divergences.json"
            )
            .to_string()
        });
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_json(&s, &path),
            Err(e) => Self::strict(&path, &format!("unreadable ({})", e)),
        }
    }

    /// Parse the allowlist JSON. An unknown `update-*` class id is IGNORED with a loud
    /// warning (fail-STRICT: the comparator has no detector for it, so that class keeps
    /// failing rather than being silently "absorbed" by nothing); malformed JSON is
    /// also strict.
    fn from_json(s: &str, path: &str) -> Self {
        let v: serde_json::Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => return Self::strict(path, &format!("invalid JSON ({})", e)),
        };
        let ids: Vec<String> = v["classes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| c["id"].as_str())
            .filter(|id| id.starts_with("update-"))
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Self::strict(path, "no adjudicated `update-*` classes");
        }
        let mut out = UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: String::new(),
        };
        let mut enabled = Vec::new();
        for id in &ids {
            if id == INTEGER_LEXICAL_CLASS {
                out.integer_lexical = true;
                enabled.push(id.clone());
            } else {
                eprintln!(
                    "warning: divergence allowlist lists update class {:?} but update-fuzz has \
                     no detector for it — that class stays STRICT",
                    id
                );
            }
        }
        out.state = format!(
            "({}): adjudicated classes enabled {:?}; every other divergence fails",
            path, enabled
        );
        out
    }
}
```

### test allowlist helper — crates/sparq-bench/src/update_fuzz.rs:1475

```rust
    fn allowlist() -> UpdateDivergenceAllowlist {
        UpdateDivergenceAllowlist::load()
    }
```

### Complete changed test functions

```rust
    #[test]
    fn generated_integer_domain_is_injective_including_load() {
        // [GPT-6 ASTRA] Independent test boundaries: do not derive the expected
        // domain from the production constant or rely on a sampled collision.
        fn assert_integer_pool(term: &Term) {
            match term {
                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                    let value = l
                        .value()
                        .parse::<u64>()
                        .expect("nonnegative generated integer");
                    if value < 20 {
                        assert_eq!(l.value(), value.to_string(), "canonical pool spelling");
                    } else {
                        assert!((20..80).contains(&value), "noncanonical pool: {value}");
                        assert_ne!(l.value(), value.to_string(), "noncanonical pool spelling");
                    }
                }
                Term::Triple(t) => assert_integer_pool(&t.object),
                _ => {}
            }
        }
        assert_eq!(CANONICAL_INTEGER_VALUES, 20);
        let mut noncanonical = 0;
        let mut loads = 0;
        let mut load_integers = 0;
        let mut nested = 0;
        for seed in 0..400 {
            let ops = gen_sequence(&mut Rng::new(seed));
            let sandbox = LoadSandbox::new().expect("sandbox");
            sparq_engine::with_load_base(sandbox.path(), || {
                let mut graph = Graph::new();
                let mut integers = BTreeMap::new();
                let mut bnodes = BTreeSet::new();
                for op in &ops {
                    if let Some(doc) = &op.doc {
                        sandbox.write(doc).expect("LOAD document");
                        let rows: Vec<_> = doc.content.lines().map(str::to_string).collect();
                        for q in parse_lines(&rows, "LOAD document").expect("valid document") {
                            assert_integer_pool(&q.object);
                            match &q.object {
                                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                                    let value = l.value().parse::<u64>().unwrap();
                                    assert!(
                                        value < 20,
                                        "LOAD integer must stay canonical: {value}"
                                    );
                                    assert_eq!(value.to_string(), l.value());
                                    load_integers += 1;
                                }
                                _ => {}
                            }
                            record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                                .expect("LOAD shares the injective integer domain");
                        }
                        let mirror = sparq_engine::update(&Graph::new(), op.oxi.as_ref().unwrap())
                            .expect("reference INSERT mirror");
                        let loaded =
                            sparq_engine::update(&Graph::new(), &op.sparq).expect("isolated LOAD");
                        assert_eq!(sparq_nquads(&mirror), sparq_nquads(&loaded));
                        loads += 1;
                    }
                    graph = sparq_engine::update(&graph, &op.sparq)
                        .unwrap_or_else(|e| panic!("seed={seed} update={} failed: {e}", op.sparq));
                    for q in parse_lines(&sparq_nquads(&graph), "generated state").unwrap() {
                        assert_integer_pool(&q.object);
                        nested += usize::from(matches!(&q.object, Term::Triple(_)));
                        record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                            .unwrap_or_else(|e| panic!("seed={seed}: {e}"));
                    }
                }
                noncanonical += integers
                    .iter()
                    .filter(|(v, lexical)| v.to_string() != **lexical)
                    .count();
            });
        }
        assert!(noncanonical > 0 && loads > 0 && load_integers > 0 && nested > 0);
        eprintln!(
            "injective corpus: noncanonical={noncanonical}, LOAD={loads}, triple terms={nested}"
        );
    }

    #[test]
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
    fn injected_marker_fails_during_actual_lexical_adjudication() {
        let ops = vec![
            Op::shared(format!(
                "INSERT DATA {{ <http://ex/s> <http://ex/p> \"020\"^^{XSD_INTEGER} }}"
            )),
            Op::shared("INSERT { ?s <http://ex/q> _:bt } WHERE { ?s <http://ex/p> ?o }".into()),
        ];
        let allow = UpdateDivergenceAllowlist {
            integer_lexical: true,
            state: "hermetic marker-adjudication test".into(),
        };
        let positive = apply_sequence(0, &ops, None, None, &allow).unwrap();
        assert_eq!(positive.ops, 2);
        assert!(positive.adjudicated_integer_lexical > 0);
        let negative = apply_sequence(0, &ops, None, Some(1), &allow).unwrap_err();
        assert!(negative.contains("step=1") && negative.contains("http://ex/injected"));
    }

    // [GPT-6 ASTRA] Literal historical input, not regenerated after narrowing the
    // reference corpus. The reference cannot represent this lexical/cardinality case.
```

### generator integer pool — crates/sparq-bench/src/update_fuzz.rs:166

```rust
const CANONICAL_INTEGER_VALUES: u64 = 20;

fn gen_canonical_integer(rng: &mut Rng) -> u64 {
    rng.below(CANONICAL_INTEGER_VALUES)
}

// ── deterministic RNG ────────────────────────────────────────────────────────────

/// Deterministic SplitMix64 — no clock/entropy, so every case is reproducible from
/// its seed. (Same generator as `fuzz.rs`; duplicated because the bead keeps this
/// file's edits disjoint from the query fuzzer's.)
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

// ── generator ────────────────────────────────────────────────────────────────────
//
// Small term pools so random DELETE DATA / WHERE conditions collide with previously
// inserted data at high probability (a delete that never matches exercises nothing).
```

## Initial-parent record helper (final version is above)

```rust
fn record_lexical_terms(
    term: &Term,
    integers: &mut BTreeMap<i64, String>,
    blank_nodes: &mut BTreeSet<String>,
) -> Result<(), String> {
    match term {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => {
            if let Ok(value) = l.value().parse::<i64>()
                && let Some(previous) = integers.insert(value, l.value().to_string())
                && previous != l.value()
            {
                return Err(format!(
                    "integer normalization is not term-injective: {previous:?} and {:?}",
                    l.value()
                ));
            }
        }
        Term::BlankNode(b) => {
            blank_nodes.insert(b.as_str().to_string());
        }
        Term::Triple(t) => {
            if let oxrdf::NamedOrBlankNode::BlankNode(b) = &t.subject {
                blank_nodes.insert(b.as_str().to_string());
            }
            record_lexical_terms(&t.object, integers, blank_nodes)?;
        }
        _ => {}
    }
    Ok(())
}

```

## Old-source Rust2021 compiler negative

```json
[
  {
    "message": "let chains are only allowed in Rust 2024 or later",
    "source_lines": [
      837
    ]
  },
  {
    "message": "let chains are only allowed in Rust 2024 or later",
    "source_lines": [
      838
    ]
  },
  {
    "message": "let chains are only allowed in Rust 2024 or later",
    "source_lines": [
      1986
    ]
  },
  {
    "message": "aborting due to 3 previous errors",
    "source_lines": []
  }
]
```

## Actual suite stdout

```text

running 20 tests
test update_fuzz::tests::blank_node_compare_is_isomorphism_not_relabelling_blindness ... ok
test update_fuzz::tests::committed_allowlist_enables_exactly_the_adjudicated_classes ... ok
test update_fuzz::tests::duplicate_quad_is_caught_despite_canonicalization ... ok
test update_fuzz::tests::fixed_window_smoke ... ok
test update_fuzz::tests::generated_integer_domain_is_injective_including_load ... ok
test update_fuzz::tests::generator_emits_every_v2_family ... ok
test update_fuzz::tests::generator_is_deterministic ... ok
test update_fuzz::tests::historical_4141222487_preserves_lexicals_and_fresh_nodes ... ok
test update_fuzz::tests::injected_divergence_is_caught ... ok
test update_fuzz::tests::injected_marker_fails_during_actual_lexical_adjudication ... ok
test update_fuzz::tests::integer_lexical_adjudication_is_narrow ... ok
test update_fuzz::tests::integer_pools_keep_draw_counts_and_all_spelling_families ... ok
test update_fuzz::tests::lexical_adjudication_preserves_rows_and_blank_node_structure ... ok
test update_fuzz::tests::lexical_collisions_across_contexts_and_nested_terms_fail ... ok
test update_fuzz::tests::load_equals_the_equivalent_insert_data ... ok
test update_fuzz::tests::nested_noncanonical_terms_remain_exact_in_sparq ... ok
test update_fuzz::tests::no_nested_blank_nodes_in_triple_terms ... ok
test update_fuzz::tests::non_canonical_integer_lexicals_are_distinct_terms ... ok
test update_fuzz::tests::oxigraph_cannot_load_a_local_file ... ok
test update_fuzz::tests::snapshot_renderers_agree_on_known_dataset ... ok

test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.18s

```

## Executed controls (structured; raw named diagnostics kept separately)

```json
{
  "suite": "20 passed;0 failed;0 ignored (exact module, serial, edition2021)",
  "full_stdout_sha256": "585ee208604133145f2f2739e9e17c7254db710a1bcff7e5a0163902127e155a",
  "clippy": "cargo clippy --release --locked --offline -j1 --tests -- -D warnings; exit0",
  "marker_strict_ambient": "Exact candidate marker test passes with empty ambient allowlist; local explicit allowlist reaches adjudication.",
  "control_results": [
    {
      "name": "partial-canonical-pool-overlap",
      "source_sha256": "d6224fd1d16b61f6b9205c3a5c41c9f2bffea503bbe6d6dbb9d4f3a86906267c",
      "test": "update_fuzz::tests::generated_integer_domain_is_injective_including_load",
      "tests_equal": true,
      "build_exit": 0,
      "binary_sha256": "1a8ad7ffdb4eee6b3f200031c3361b0589b60df3da09d02760f2318ac3923308",
      "test_exit": 101,
      "expected_test_exit": 101
    },
    {
      "name": "omit-blank-node-dataset-details",
      "source_sha256": "0a77a0cf06442f9698cd767761315846649cca71aa8d948b4ab5917196b2ed78",
      "test": "update_fuzz::tests::lexical_adjudication_preserves_rows_and_blank_node_structure",
      "tests_equal": true,
      "build_exit": 0,
      "binary_sha256": "9cac135b936d5e4c05b376a94c5fa12f6e52ed5bc2ac88ca701b3eab706d05f3",
      "test_exit": 101,
      "expected_test_exit": 101
    },
    {
      "name": "restore-ambient-marker-allowlist",
      "source_sha256": "f7e4d4cd40f691395222f2683c400dcb9a83729d3e118aa0b4de6632a2b6e02d",
      "test": "update_fuzz::tests::injected_marker_fails_during_actual_lexical_adjudication",
      "tests_equal": false,
      "build_exit": 0,
      "binary_sha256": "52bb31637f8d29c1063260367ceba9a2a44ef5fba2662d99df741271a20ffdbd",
      "test_exit": 101,
      "expected_test_exit": 101
    },
    {
      "name": "old-source-edition2021",
      "source_sha256": "962e5f3e0558d36b48b2f0a970ed3a28b7beeb2aac0bec8e52aed72b5d3c93d3",
      "test": null,
      "tests_equal": false,
      "build_exit": 1,
      "confirmed_edition_error": true
    },
    {
      "name": "normalized-error-fault-positive",
      "source_sha256": "66217402e29791cfd8094920b10d594008b7e234e13e40df02d00827679e64cc",
      "test": "update_fuzz::diagnostic_fault_probe::normalized_error_preserves_original_details",
      "tests_equal": false,
      "build_exit": 0,
      "binary_sha256": "ca3e3a1c569f868b93e21a84ba38fcde8e70ba294b441963a6771b744f441ace",
      "test_exit": 0,
      "expected_test_exit": 0
    },
    {
      "name": "normalized-error-old-detail-control",
      "source_sha256": "1a50cb677c9b5d162bc0aee39631e695d34dd4631d2e034c91effca5e80418e4",
      "test": "update_fuzz::diagnostic_fault_probe::normalized_error_preserves_original_details",
      "tests_equal": false,
      "build_exit": 0,
      "binary_sha256": "3083901858ebfd6385fd7b1ca99dffb0c5ab17b9671cf5eb8e80120ed5d6a937",
      "test_exit": 101,
      "expected_test_exit": 101
    }
  ],
  "fault_injection_limit": "Third comparable() call is deliberately injected to fail after both raw dataset calls succeed. Positive uses final branch; negative has identical fault/test and old error-only branch. This establishes diagnostic preservation on that branch, not a naturally observed canonicalizer failure.",
  "prior_controls": "No repeat of seven parent controls: generator partition/LOAD draw policy, record recursion, duplicate refusal, count/blank-node guards, and marker detection conditions unchanged; diagnostic formatting, syntax and test oracles changed. Current20-test suite plus focused follow-up controls passed; prior control outputs remain frozen separately."
}
```

### partial-canonical-pool-overlap exact scratch delta

```diff
--- candidate/update_fuzz.rs
+++ partial-canonical-pool-overlap/update_fuzz.rs
@@ -214,7 +214,7 @@
     match rng.below(6) {
         0 | 1 => format!("<http://ex/o{}>", rng.below(6)),
         2 | 3 => format!("\"lit{}\"", rng.below(5)),
-        4 => format!("{}", gen_canonical_integer(rng)),
+        4 => format!("{}", rng.below(40)),
         _ => format!("\"tag{}\"@en", rng.below(3)),
     }
 }
```

### omit-blank-node-dataset-details exact scratch delta

```diff
--- candidate/update_fuzz.rs
+++ omit-blank-node-dataset-details/update_fuzz.rs
@@ -992,7 +992,7 @@
                         "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
                         na.blank_nodes,
                         nb.blank_nodes,
-                        one_sided(label_a, &ca, label_b, &cb)
+                        String::new()
                     ));
                 }
                 match (
```

### restore-ambient-marker-allowlist exact scratch delta

```diff
--- candidate/update_fuzz.rs
+++ restore-ambient-marker-allowlist/update_fuzz.rs
@@ -2209,10 +2209,7 @@
             )),
             Op::shared("INSERT { ?s <http://ex/q> _:bt } WHERE { ?s <http://ex/p> ?o }".into()),
         ];
-        let allow = UpdateDivergenceAllowlist {
-            integer_lexical: true,
-            state: "hermetic marker-adjudication test".into(),
-        };
+        let allow = UpdateDivergenceAllowlist::load();
         let positive = apply_sequence(0, &ops, None, None, &allow).unwrap();
         assert_eq!(positive.ops, 2);
         assert!(positive.adjudicated_integer_lexical > 0);
```

### normalized-error-fault-positive exact scratch delta

```diff
--- candidate/update_fuzz.rs
+++ normalized-error-fault-positive/update_fuzz.rs
@@ -793,6 +793,13 @@
 /// fails closed on a blank node nested inside a triple term, so the non-standard
 /// nested-bnode descent is never reachable from this harness.
 fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {
+    #[cfg(test)] {
+        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
+        if CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 2 {
+            return Err("injected normalized comparable error".into());
+        }
+    }
+
     if !relabel {
         return Ok(lines.to_vec());
     }
@@ -2331,3 +2338,22 @@
         });
     }
 }
+
+#[cfg(test)]
+mod diagnostic_fault_probe {
+    use super::*;
+    #[test]
+    fn normalized_error_preserves_original_details() {
+        let a = vec![format!("_:a <http://ex/p> \"020\"^^{} .", XSD_INTEGER)];
+        let b = vec![format!("_:b <http://ex/p> \"20\"^^{} .", XSD_INTEGER)];
+        match compare("original", &a, "reference", &b, true) {
+            Verdict::Differs(detail) => {
+                assert!(detail.contains("injected normalized comparable error"), "{detail}");
+                assert!(detail.contains("only in original:"), "{detail}");
+                assert!(detail.contains("only in reference:"), "{detail}");
+                assert!(detail.contains("020"), "{detail}");
+            }
+            _ => panic!("injected normalized comparable error must fail"),
+        }
+    }
+}
```

### Normalized-error negative: exact delta from the fault-positive source above

The injected third-call error and appended test are byte-identical; only error-detail handling changes.

```diff
--- fault-positive/update_fuzz.rs
+++ fault-negative/update_fuzz.rs
@@ -1007,12 +1007,7 @@
                     comparable(&nb.lines, relabel, label_b),
                 ) {
                     (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
-                    (Err(e), _) | (_, Err(e)) => {
-                        return Verdict::Differs(format!(
-                            "integer-lexical adjudication refused: {e}\n{}",
-                            one_sided(label_a, &ca, label_b, &cb)
-                        ));
-                    }
+                    (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
                     _ => {}
                 }
             }
```

## Context boundaries

This supplement supplies complete named comparison, normalization, record, allowlist and apply_sequence bodies, all changed semantic test bodies, and exact old record-helper syntax. Unchanged generator operation constructors, LoadSandbox implementation, engine/parser/canon internals and old legacy tests, unchanged historical regression and other unchanged parent tests are not repeated here; their full source remains in the prior immutable evidence and final.rs.txt. This is a native exact-module result, not full crate/workspace or CI approval. No security, engine-correctness or performance claim is added.
