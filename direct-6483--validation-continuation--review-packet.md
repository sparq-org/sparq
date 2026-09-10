# Issue 6483 — exact candidate final review

{
  "decision": "ready for independent final source review; no publication or CI admission claim",
  "head": "0b4554b924a80432cc1b572bd19f8e58cdbfb4e6",
  "parent": "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464",
  "model": "GPT-6 Astra xhigh",
  "source_binding": "source-binding.json",
  "scope": "Private UPDATE differential complete-quad snapshots/probes only; rejects repeated raw full-quad emissions in strict Same branch after existing unequal-count check. No general bag uniqueness policy or engine/canonicalizer fix.",
  "change": "O(N log N) BTreeSet comparisons and O(N) borrowed references; deterministic side-first first repeated raw line with total occurrence count. No cloned raw strings in guard.",
  "validation": {
    "candidate": "26/26 actual module tests passed (existing20 + new6); exact original compiled candidate, zero rebuilds in continuation",
    "both_engine_scope": "Actual same SPO in default plus two named graphs; each renderer snapshot3; each default probe1 and named probe2 exact graph-bearing rows passed.",
    "control": "Removed only new guard; same source tests and pinned libraries. Compiled successfully, regression exits101 with equal-total raw duplicate redistribution returned Same.",
    "clippy": "Actual module direct clippy-driver --test metadata with -D warnings exit0; not full workspace lint.",
    "format": "Previously executed touched-region rustfmt against exact candidate before its successful compile; copied unchanged evidence. No whole-file reflow.",
    "diff_check": "exit0",
    "author_preflight": "exit1 solely privacy-claims shell line92 mapfile unavailable under host Bash3; other selected mechanical checks pass. Linux privacy proof remains required."
  },
  "interrupted_prior_run": {
    "manifest_sha256": "dfa1ff9fa266fe27756028bd148a7fccb77898bf8926c9c52bcd4bd5aaef6157",
    "files_verified_before_and_after": 40,
    "candidate_build_exit": 0,
    "candidate_test_exit": -15,
    "cause": "Controller allocation census raced LOAD sandbox file removal (ENOENT); no observed Rust assertion failure. Four tests passed then interruption. Old files retained unchanged.",
    "continuation": "Controller catches only ENOENT during traversal/stat, all other failures stop. Runtime TMPDIR moved to continuation; compile-time faithful allowlist remains old read-only location.",
    "enoent_observed_in_continuation": 0
  },
  "resource": {
    "phase_seconds": 17.056804625,
    "initial_combined_allocated_bytes": 108208128,
    "max_combined_allocated_bytes": 122343424,
    "minimum_free_bytes": 12985921536,
    "limit_combined_bytes": 201326592,
    "export_reserve_bytes": 8388608,
    "jobs": 1,
    "offline": true,
    "dependency_builds": 0,
    "candidate_rebuilds": 0,
    "control_builds": 1
  },
  "limits": [
    "Diagnostic macOS Rust1.97.1 edition2021 O3/unwind/no-LTO/codegen16 profile and recorded6rlibs, not authoritative Linux full-crate/workspace CI.",
    "No random additional seeds or performance measurement; unchanged fixed40/400/200 suites ran as part of26 module tests.",
    "New guard only resolves strict Same blind spot; canonicalization/lexical errors and unknown divergences stay strict.",
    "Predicate scoping source proof and runtime fixture apply to current two private full-quad probes; future lossy projection/join/UNION needs contract reassessment."
  ],
  "pending_commands": false
}

## Exact diff
```diff
diff --git a/crates/sparq-bench/src/update_fuzz.rs b/crates/sparq-bench/src/update_fuzz.rs
index 0ad908acd..e2e9a615f 100644
--- a/crates/sparq-bench/src/update_fuzz.rs
+++ b/crates/sparq-bench/src/update_fuzz.rs
@@ -939,8 +939,22 @@ enum Verdict {
     Differs(String),
 }
 
+// [GPT-6 Astra] Borrow raw lines so duplicate detection also works on unsorted
+// diagnostic inputs. O(N log N) comparisons and O(N) borrowed references.
+fn repeated_raw_line(lines: &[String]) -> Option<(&str, usize)> {
+    let mut seen = BTreeSet::new();
+    let repeated = lines.iter().find(|line| !seen.insert(line.as_str()))?;
+    let count = lines.iter().filter(|line| *line == repeated).count();
+    Some((repeated.as_str(), count))
+}
+
 /// Compares two snapshots under the canonical form the module docs describe.
 ///
+/// [GPT-6 Astra] Inputs are complete-quad snapshots or the full-quad rows from
+/// `PROBES`. Repeated raw quads are invalid emissions, even on both sides; this
+/// is not a uniqueness policy for arbitrary SPARQL projection/join/UNION bags.
+/// In particular, a duplicate-bearing snapshot does not compare equal to itself.
+///
 /// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
 /// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
 /// sides are sparq, so any lexical disagreement between them is a real bug.
@@ -972,6 +986,16 @@ fn compare(
                 b.len()
             ));
         }
+        for (label, lines) in [(label_a, a), (label_b, b)] {
+            if let Some((line, count)) = repeated_raw_line(lines) {
+                return Verdict::Differs(format!(
+                    "{label}: repeated raw full-quad line occurs {count} times \
+                     (raw totals: {label_a} {}, {label_b} {}):\n  {line}",
+                    a.len(),
+                    b.len()
+                ));
+            }
+        }
         return Verdict::Same;
     }
     if allow_integer_lexical {
@@ -1026,6 +1050,10 @@ fn compare(
 // by joining its cells is then a well-formed N-Quads line, so probe results go through
 // exactly the same canonical comparison as the dataset snapshots — which is what makes
 // them comparable at all once blank nodes are in play.
+// [GPT-6 Astra] Each probe projects every component of one triple/quad, including
+// the named graph. Adding projection loss, joins or UNION requires reassessing
+// this private comparator's unique-emission contract; ordinary result bags need
+// not be unique. The scope and projection tests below pin these two probes.
 
 const PROBES: &[(&str, &[&str])] = &[
     ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
@@ -1476,6 +1504,145 @@ mod tests {
         UpdateDivergenceAllowlist::load()
     }
 
+    // [GPT-6 Astra] Equal raw totals must not hide canon's duplicate collapse.
+    #[test]
+    fn raw_duplicate_redistribution_is_rejected() {
+        let p = format!("_:x <http://ex/p> \"020\"^^{XSD_INTEGER} .");
+        let q = format!("_:x <http://ex/q> \"021\"^^{XSD_INTEGER} .");
+        let left = vec![p.clone(), p.clone(), q.clone()];
+        let right = vec![p.clone(), q.clone(), q];
+        for allow in [false, true] {
+            match compare("left", &left, "right", &right, allow) {
+                Verdict::Differs(detail) => assert_eq!(
+                    detail,
+                    format!(
+                        "left: repeated raw full-quad line occurs 2 times \
+                         (raw totals: left 3, right 3):\n  {p}"
+                    )
+                ),
+                Verdict::Same => panic!("equal-total raw duplicate redistribution returned Same"),
+                Verdict::AdjudicatedIntegerLexical => panic!("raw duplicates were adjudicated"),
+            }
+        }
+    }
+
+    #[test]
+    fn raw_duplicate_nonadjacent_self_comparison_is_invalid() {
+        for subject in ["<http://ex/s>", "_:x"] {
+            // The literal also exercises mentions_blank_node's false-positive route.
+            for object in ["<http://ex/o>", "\"contains _: text\""] {
+                let p = format!("{subject} <http://ex/p> {object} .");
+                let q = format!("{subject} <http://ex/q> {object} .");
+                let rows = vec![p.clone(), q, p.clone()];
+                for allow in [false, true] {
+                    match compare("first", &rows, "second", &rows, allow) {
+                        Verdict::Differs(detail) => {
+                            assert!(detail
+                                .starts_with("first: repeated raw full-quad line occurs 2 times"));
+                            assert!(detail.ends_with(&p));
+                        }
+                        _ => panic!("duplicate-bearing snapshot must not compare equal to itself"),
+                    }
+                }
+            }
+        }
+    }
+
+    #[test]
+    fn raw_duplicate_guard_preserves_symmetry_and_graph_identity() {
+        let left = vec![
+            "_:a <http://ex/p> _:b .".into(),
+            "_:b <http://ex/p> _:a .".into(),
+        ];
+        let right = vec![
+            "_:y <http://ex/p> _:z .".into(),
+            "_:z <http://ex/p> _:y .".into(),
+        ];
+        assert!(matches!(
+            compare("left", &left, "right", &right, false),
+            Verdict::Same
+        ));
+        let rows = vec![
+            "_:s <http://ex/p> <http://ex/o> <http://ex/g1> .".into(),
+            "_:s <http://ex/p> <http://ex/o> <http://ex/g2> .".into(),
+        ];
+        assert!(matches!(
+            compare("left", &rows, "right", &rows, false),
+            Verdict::Same
+        ));
+        let duplicate = vec![rows[0].clone(), rows[0].clone()];
+        assert!(matches!(
+            compare("left", &duplicate, "right", &duplicate, false),
+            Verdict::Differs(_)
+        ));
+    }
+
+    #[test]
+    fn raw_duplicate_probe_projection_contract() {
+        assert_eq!(
+            PROBES,
+            &[
+                ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"][..]),
+                (
+                    "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
+                    &["s", "p", "o", "g"][..]
+                ),
+            ]
+        );
+    }
+
+    #[test]
+    fn raw_duplicate_probe_scope_is_complete_quads_in_both_engines() {
+        let triple = "<http://ex/s> <http://ex/p> <http://ex/o>";
+        let update = format!(
+            "INSERT DATA {{ {triple} . GRAPH <http://ex/g1> {{ {triple} }} \
+             GRAPH <http://ex/g2> {{ {triple} }} }}"
+        );
+        let graph = sparq_engine::update(&Graph::new(), &update).unwrap();
+        let store = Store::new().unwrap();
+        store.update(update.as_str()).unwrap();
+        let snapshots = (sparq_nquads(&graph), oxi_nquads(&store).unwrap());
+        assert_eq!(snapshots.0.len(), 3);
+        assert_eq!(snapshots.0, snapshots.1);
+        let expected = [
+            vec![format!("{triple} .")],
+            vec![
+                format!("{triple} <http://ex/g1> ."),
+                format!("{triple} <http://ex/g2> ."),
+            ],
+        ];
+        for ((query, vars), expected) in PROBES.iter().zip(expected) {
+            let sparq = sparq_probe(&graph, query).unwrap();
+            let oxi = oxi_probe(&store, query, vars).unwrap();
+            assert_eq!(sparq, expected, "Sparq scope for {query}");
+            assert_eq!(oxi, expected, "Oxigraph scope for {query}");
+            assert!(matches!(
+                compare("sparq", &sparq, "oxigraph", &oxi, false),
+                Verdict::Same
+            ));
+        }
+    }
+
+    #[test]
+    fn raw_duplicate_guard_preserves_canonicalization_errors() {
+        for line in [
+            "_:x not-an-iri <http://ex/o> .",
+            "_:x <http://ex/p> <<( _:nested <http://ex/q> <http://ex/o> )>> .",
+        ] {
+            let rows = vec![line.to_string(), line.to_string()];
+            match compare("left", &rows, "right", &rows, false) {
+                Verdict::Differs(detail) => {
+                    assert!(
+                        detail.contains("re-parse failed")
+                            || detail.contains("canonicalization failed")
+                    );
+                    assert!(!detail.contains("repeated raw full-quad"));
+                }
+                _ => panic!("existing parse/ground-profile error must stay strict"),
+            }
+        }
+    }
+
     /// The per-PR BLOCKING smoke: a fixed seed window through the full three-way
     /// differential (both sparq update paths vs Oxigraph, canonical dataset + probe
     /// binding sets per step). The randomized soak (advancing window) is the nightly
```

## Complete comparator, normalization, snapshots, probes, callers, allowlist and new tests
```rust

## crates/sparq-bench/src/update_fuzz.rs:1-156
//! SPARQL **UPDATE** differential fuzzer vs Oxigraph (beads sq-3dyje.4, sq-hodke). [FABLE-5]
//!
//! The query-side differential fuzzer (`fuzz.rs`) has zero UPDATE coverage, and
//! `sparq-engine/src/update.rs` is otherwise guarded by a single mechanism family
//! (unit tests + the fixed W3C conformance corpus). This module closes that gap with
//! a seeded generator of update sequences applied step-by-step to THREE independent
//! implementations:
//!
//!   1. sparq's rebuild path        (`sparq_engine::update` — decode / apply / rebuild),
//!   2. sparq's delta-overlay path  (`sparq_engine::update_in_place`),
//!   3. in-process Oxigraph 0.5     (`Store::update`).
//!
//! After EVERY step it asserts, per the AGENTS.md oracle-strength rule (term
//! structure and full answer sets, never row counts):
//!
//!   * **canonical dataset equality** — each store rendered to N-Quads lines and
//!     compared under the canonical form described in *Canonical comparison* below;
//!   * **probe SELECT equality** — the full sorted binding set of
//!     `SELECT ?s ?p ?o { ?s ?p ?o }` and `SELECT ?s ?p ?o ?g { GRAPH ?g { ?s ?p ?o } }`
//!     through each engine's own query path, under the same canonical form.
//!
//! A per-step compare means any divergence is localized to the exact offending
//! operation, and the final-store comparison is the last step's compare.
//!
//! ## Generated subset
//!
//! Deterministic by construction — no `NOW()`/`RAND()`/`UUID()`/`BNODE()`. (The
//! `BNODE()` FUNCTION is what would be non-deterministic; blank node LABELS written
//! into a data block or template are a pure function of the seed, and the labels the
//! engines then mint are exactly what the isomorphism-aware compare exists to absorb.)
//! `INSERT DATA` / `DELETE DATA` (with `GRAPH` blocks), `DELETE/INSERT … WHERE`
//! (ground and variable templates, `WITH`, `USING`, variable graph names,
//! `DELETE WHERE`), `CLEAR` / `DROP` / `CREATE` / `COPY` / `MOVE` / `ADD` (always
//! `SILENT`, so absent-graph error behaviour — which the spec leaves
//! implementation-defined — cannot masquerade as a semantic divergence; the state
//! compare is the whole oracle), plus occasional two-operation compound requests
//! (`op ; op`) for the within-one-update sequencing path.
//!
//! v2 (sq-hodke) adds four term/operation families on top of the v1 ground-term subset:
//!
//!   * **non-canonical numeric lexicals** (`"05"^^xsd:integer`, `"+7"^^xsd:integer`) —
//!     RDF term identity is lexical-form identity, so these must survive a store
//!     round-trip as terms DISTINCT from their canonical siblings. sparq's dictionary
//!     inlines only the canonical form (`dict::try_inline_lit`), so the non-canonical
//!     one takes the general path. This is the one family where Oxigraph is NOT a
//!     usable reference (see *Adjudicated divergences* below), so distinctness is
//!     asserted by sparq's two update paths against EACH OTHER — that compare is never
//!     adjudicated — and pinned directly by
//!     `tests::non_canonical_integer_lexicals_are_distinct_terms`.
//!   * **blank nodes** in `INSERT DATA` blocks and in `INSERT` templates (fresh per
//!     operation, SPARQL 1.1 §3.1.1; fresh per solution, §3.1.3) — which is what forces the
//!     isomorphism-aware comparison. Blank nodes never enter `DELETE DATA` or a
//!     ground `WHERE` condition (the spec forbids the former, and a blank node's
//!     label is not re-referenceable across operations anyway).
//!   * **`LOAD`** of a local `file://` document — see *LOAD* below.
//!   * **RDF-1.2 triple terms** — `<<( s p o )>>` (object position only; the parser
//!     rejects one as a subject, which is RDF 1.2's rule) in data blocks, in `INSERT`
//!     templates, and nested one level deep, plus the `<< s p o >>` reifier form
//!     (which desugars to a fresh blank node + `rdf:reifies`).
//!
//! Graph EXISTENCE (an empty named graph created by `CREATE`) is invisible to a
//! quad-level compare and remains out of scope.
//!
//! ## Canonical comparison
//!
//! With v1's ground terms, sorted N-Quads WAS a canonical form. With blank nodes it
//! is not: sparq labels them `_:fbN` off a process-wide counter, Oxigraph off a hash —
//! and sparq's OWN two update paths therefore disagree with each other. So when either
//! side carries a blank node, both snapshots are relabelled to their **RDFC-1.0**
//! canonical form (`sparq_canon`) before comparison, which decides RDF isomorphism.
//! Triple terms are outside W3C RDFC-1.0, so the comparator uses that crate's
//! CONSTRAINED `*_ground_terms` entry points: exactly RDFC-1.0 with triple terms as
//! opaque constants, failing closed if a blank node is ever nested inside a triple
//! term. The generator upholds that invariant (the triple-term template carries an
//! `isBlank` FILTER guard) and `tests::no_nested_blank_nodes_in_triple_terms` pins it,
//! so the non-standard nested-bnode descent is never reachable from here.
//!
//! Canonicalization deduplicates, so the RAW line counts are compared separately: a
//! store yielding the same quad twice is an invariant break worth failing on, and that
//! check would otherwise be lost.
//!
//! ## LOAD
//!
//! The bead's premise — "a local `file://` doc both engines can fetch" — does not hold,
//! and the code says so: Oxigraph 0.5's `eval_load` is HTTP-only and, without the
//! `http-client` feature this harness deliberately does not enable, returns
//! *"HTTP client is not available"* for ANY source; sparq's `load_document` is the exact
//! mirror image (`file://` only, and only under an allowlisted base). The two engines'
//! LOAD source surfaces are DISJOINT, so a same-request LOAD differential is impossible.
//! `tests::oxigraph_cannot_load_a_local_file` pins that reason so it is machine-checked
//! rather than merely asserted here.
//!
//! What the harness does instead: sparq's two paths get the real
//! `LOAD <file://doc.nt> [INTO GRAPH g]`, and the reference engine gets the
//! semantically-equivalent `INSERT DATA` of the SAME ground triples. The oracle is
//! therefore "sparq's LOAD produces exactly the dataset the reference engine reaches by
//! inserting that document's contents", plus a true two-implementation differential
//! between sparq's rebuild and delta-overlay LOAD paths. The document is written under a
//! per-run temporary directory allowlisted via `sparq_engine::with_load_base`, and the
//! generated IRI is RELATIVE (`<file://doc0.nt>`) so the request text — and hence the
//! seed repro — is independent of that directory's absolute path.
//!
//! ## Adjudicated divergences
//!
//! The mechanism mirrors `fuzz.rs`: the comparator consults
//! `bench/differential-divergences.json` for classes whose id starts with `update-`. A
//! listed class without a detector in this file stays STRICT, with a loud warning —
//! never a silent skip.
//!
//! One class is adjudicated today,
//! `update-oxigraph-integer-lexical-canonicalization`: Oxigraph's storage layer parses
//! every `xsd:integer` into a native integer and re-renders it canonically
//! (`storage/numeric_encoder.rs`), so `"05"^^xsd:integer` and `"5"^^xsd:integer` become
//! the SAME stored term. RDF 1.1 Concepts §3.3 makes them different terms (literal
//! equality is lexical-form equality), so sparq is spec-correct and Oxigraph is lossy.
//! It is NOT a blind skip: the comparator re-derives Oxigraph's normalization
//! independently — rewriting every `xsd:integer` literal on BOTH sides to its canonical
//! lexical form — and absorbs the step ONLY when that rewrite is injective on terms,
//! preserves rows and blank nodes, and the two datasets agree exactly under it.
//! Any residual difference still FAILS. The sparq-vs-sparq
//! compare never consults the allowlist: both sides are sparq, so any lexical
//! disagreement between them is a real bug.
//!
//! To keep that class narrow, non-canonical lexicals are generated only in INSERT
//! positions. An exact-term `DELETE DATA` of one (where sparq removes `"05"` and leaves
//! `"5"` while Oxigraph, holding one merged term, removes both) is a cascade no
//! normalization can undo; that shape is covered instead by the sparq-internal
//! `tests::non_canonical_integer_lexicals_are_distinct_terms`, which is the correct
//! oracle given Oxigraph cannot serve as a reference for it.
//!
//! [GPT-6 ASTRA] #5183 also exposed a solution-multiplicity cascade: coexisting `8`
//! and `"008"^^xsd:integer` produce two template blank nodes in sparq, but one in the
//! lossy reference. The comparator correctly rejects this. The reference corpus now
//! uses disjoint canonical/noncanonical integer value pools, with one spelling per
//! value globally, including LOAD and nested triple terms. This narrows coverage;
//! the exact historical sequence remains a sparq-only regression. Noncanonical
//! terms still reach storage, queries, templates and the reference comparison.
//! This correspondence is limited to the generated BGP/graph/isBlank operations
//! and full-quad probes: it does not justify lexical-sensitive STR/sameTerm queries.
//! A changed generator changes a seed's input; it does not resolve other historical
//! failures merely because their seed numbers pass under the new generator.
//!
//! KNOWN SHARED-ORACLE BLIND SPOT (honest boundary): both engines parse updates with
//! `spargebra`, so a parser-level desugaring bug (e.g. in COPY/MOVE/ADD expansion, or in
//! the `<< s p o >>` reifier desugaring) would affect both sides identically and cannot
//! be caught here — only evaluation-layer divergence is observable.
//!
//! Usage: `sparq-bench update-fuzz --seed-start N --seed-count M`
//! Every case is reproducible from its seed; on failure the log carries
//! `MISMATCH seed=N` lines plus a `FIRST FAILING CASE:` block (the same
//! machine-parseable contract as `fuzz.rs`, consumed by
//! `scripts/ci-file-differential-failure.py`).

use oxigraph::store::Store;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, Quad, Term, Triple};

## crates/sparq-bench/src/update_fuzz.rs:707-1126
fn nquads_line(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("{} {} {} {} .", subject, predicate, object, g),
        None => format!("{} {} {} .", subject, predicate, object),
    }
}

/// A sparq `Graph` (default graph + named graphs) as SORTED N-Quads lines.
/// Duplicate lines are NOT collapsed — see the raw-count check in [`compare`].
fn sparq_nquads(g: &Graph) -> Vec<String> {
    fn triples_of(g: &Graph, graph: Option<&str>, out: &mut Vec<String>) {
        let scan = g.store.scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let t = scan.to_spo(r);
            out.push(nquads_line(
                &g.dict.term(t[0]).to_string(),
                &g.dict.term(t[1]).to_string(),
                &g.dict.term(t[2]).to_string(),
                graph,
            ));
        }
    }
    let mut out = Vec::new();
    triples_of(g, None, &mut out);
    for (name, sub) in &g.named {
        triples_of(sub, Some(&name.to_string()), &mut out);
    }
    out.sort();
    out
}

/// The Oxigraph store as SORTED N-Quads lines, rendered identically.
fn oxi_nquads(store: &Store) -> Result<Vec<String>, String> {
    use oxigraph::model::GraphName;
    let mut out = Vec::new();
    for q in store.iter() {
        let q = q.map_err(|e| format!("oxigraph iter error: {}", e))?;
        let graph = match &q.graph_name {
            GraphName::DefaultGraph => None,
            g => Some(g.to_string()),
        };
        out.push(nquads_line(
            &q.subject.to_string(),
            &q.predicate.to_string(),
            &q.object.to_string(),
            graph.as_deref(),
        ));
    }
    out.sort();
    Ok(out)
}

// ── canonical comparison ─────────────────────────────────────────────────────────

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

// [GPT-6 Astra] Borrow raw lines so duplicate detection also works on unsorted
// diagnostic inputs. O(N log N) comparisons and O(N) borrowed references.
fn repeated_raw_line(lines: &[String]) -> Option<(&str, usize)> {
    let mut seen = BTreeSet::new();
    let repeated = lines.iter().find(|line| !seen.insert(line.as_str()))?;
    let count = lines.iter().filter(|line| *line == repeated).count();
    Some((repeated.as_str(), count))
}

/// Compares two snapshots under the canonical form the module docs describe.
///
/// [GPT-6 Astra] Inputs are complete-quad snapshots or the full-quad rows from
/// `PROBES`. Repeated raw quads are invalid emissions, even on both sides; this
/// is not a uniqueness policy for arbitrary SPARQL projection/join/UNION bags.
/// In particular, a duplicate-bearing snapshot does not compare equal to itself.
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
        for (label, lines) in [(label_a, a), (label_b, b)] {
            if let Some((line, count)) = repeated_raw_line(lines) {
                return Verdict::Differs(format!(
                    "{label}: repeated raw full-quad line occurs {count} times \
                     (raw totals: {label_a} {}, {label_b} {}):\n  {line}",
                    a.len(),
                    b.len()
                ));
            }
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

// ── probe SELECTs (full binding-set compare, per the oracle-strength rule) ────────
//
// The projection order is N-QUADS ORDER (`?s ?p ?o [?g]`) on purpose: a row rendered
// by joining its cells is then a well-formed N-Quads line, so probe results go through
// exactly the same canonical comparison as the dataset snapshots — which is what makes
// them comparable at all once blank nodes are in play.
// [GPT-6 Astra] Each probe projects every component of one triple/quad, including
// the named graph. Adding projection loss, joins or UNION requires reassessing
// this private comparator's unique-emission contract; ordinary result bags need
// not be unique. The scope and projection tests below pin these two probes.

const PROBES: &[(&str, &[&str])] = &[
    ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
    (
        "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
        &["s", "p", "o", "g"],
    ),
];

/// Renders one probe row's cells as an N-Quads line.
fn probe_line(cells: &[String]) -> String {
    format!("{} .", cells.join(" "))
}

/// The full sorted binding set of a probe through sparq's query path.
fn sparq_probe(g: &Graph, q: &str) -> Result<Vec<String>, String> {
    let r = sparq_engine::query(g, q).map_err(|e| format!("sparq probe error: {}", e))?;
    let mut rows: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            probe_line(
                &row.iter()
                    .map(|t| {
                        t.as_ref()
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "UNDEF".to_string())
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    rows.sort();
    Ok(rows)
}

/// The full sorted binding set of a probe through Oxigraph's query path.
// clippy: the differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn oxi_probe(store: &Store, q: &str, vars: &[&str]) -> Result<Vec<String>, String> {
    match store.query(q).map_err(|e| e.to_string())? {
        oxigraph::sparql::QueryResults::Solutions(s) => {
            let mut rows = Vec::new();
            for sol in s {
                let sol = sol.map_err(|e| e.to_string())?;
                rows.push(probe_line(
                    &vars
                        .iter()
                        .map(|v| {
                            sol.get(*v)
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| "UNDEF".to_string())
                        })
                        .collect::<Vec<_>>(),
                ));
            }
            rows.sort();
            Ok(rows)
        }
        _ => Err("probe did not return solutions".to_string()),
    }
}

// ── the LOAD document sandbox ────────────────────────────────────────────────────

/// A temporary directory holding the documents a seed's `LOAD` operations read, and
/// which `sparq_engine::with_load_base` allowlists for the duration of that seed.
///
/// Uniquified by process id AND a run-local counter so concurrently-running seeds
/// (cargo test runs these in parallel threads) never share a document. The path never

## crates/sparq-bench/src/update_fuzz.rs:1185-1351
fn check_seed(
    seed: u64,
    inject_divergence_at: Option<usize>,
    allow: &UpdateDivergenceAllowlist,
) -> Result<SeedOutcome, String> {
    let mut rng = Rng::new(seed);
    let ops = gen_sequence(&mut rng);
    let sandbox = if ops.iter().any(|o| o.sparq.starts_with("LOAD")) {
        Some(LoadSandbox::new()?)
    } else {
        None
    };
    match sandbox.as_ref() {
        // The allowlisted base is installed for the whole seed: `with_load_base` is a
        // thread-local guard, and a seed's steps all run on this thread.
        Some(s) => sparq_engine::with_load_base(s.path(), || {
            apply_sequence(seed, &ops, Some(s), inject_divergence_at, allow)
        }),
        None => apply_sequence(seed, &ops, None, inject_divergence_at, allow),
    }
}

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


## crates/sparq-bench/src/update_fuzz.rs:1358-1649
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

// ── entry point ──────────────────────────────────────────────────────────────────

pub fn run(seed_start: u64, count: u64) {
    let allow = UpdateDivergenceAllowlist::load();
    println!("update-fuzz divergence allowlist {}", allow.state);
    let mut checked = 0u64;
    let mut ops_applied = 0u64;
    let mut adjudicated_integer_lexical = 0u64;
    let mut isomorphism_compares = 0u64;
    let mut mismatch = 0u64;
    let mut first_repro: Option<String> = None;

    for seed in seed_start..seed_start + count {
        match check_seed(seed, None, &allow) {
            Ok(outcome) => {
                checked += 1;
                ops_applied += outcome.ops;
                adjudicated_integer_lexical += outcome.adjudicated_integer_lexical;
                isomorphism_compares += outcome.isomorphism_compares;
            }
            Err(detail) => {
                mismatch += 1;
                // One machine-greppable line per failing seed (the contract
                // scripts/ci-file-differential-failure.py parses).
                eprintln!("MISMATCH seed={}", seed);
                if first_repro.is_none() {
                    first_repro = Some(format!("seed={}\n{}", seed, detail));
                }
            }
        }
    }

    println!(
        "update-fuzz seeds {}..{} : checked={} update_requests={} \
         isomorphism_compares={} adjudicated(integer-lexical)={} mismatch={}",
        seed_start,
        seed_start + count,
        checked,
        ops_applied,
        isomorphism_compares,
        adjudicated_integer_lexical,
        mismatch
    );
    // NON-VACUITY GUARD: a window that never exercised the v2 isomorphism path has
    // lost blank-node coverage without failing anything — say so loudly.
    if mismatch == 0 && checked > 0 && isomorphism_compares == 0 {
        eprintln!(
            "warning: no step in this window needed RDFC-1.0 relabelling — the \
             blank-node isomorphism path was never exercised"
        );
    }
    if let Some(r) = first_repro {
        println!("\nFIRST FAILING CASE:\n{}", r);
        std::process::exit(1);
    }
}

// ── tests (the per-PR fixed-window smoke + comparator non-vacuity) ────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist() -> UpdateDivergenceAllowlist {
        UpdateDivergenceAllowlist::load()
    }

    // [GPT-6 Astra] Equal raw totals must not hide canon's duplicate collapse.
    #[test]
    fn raw_duplicate_redistribution_is_rejected() {
        let p = format!("_:x <http://ex/p> \"020\"^^{XSD_INTEGER} .");
        let q = format!("_:x <http://ex/q> \"021\"^^{XSD_INTEGER} .");
        let left = vec![p.clone(), p.clone(), q.clone()];
        let right = vec![p.clone(), q.clone(), q];
        for allow in [false, true] {
            match compare("left", &left, "right", &right, allow) {
                Verdict::Differs(detail) => assert_eq!(
                    detail,
                    format!(
                        "left: repeated raw full-quad line occurs 2 times \
                         (raw totals: left 3, right 3):\n  {p}"
                    )
                ),
                Verdict::Same => panic!("equal-total raw duplicate redistribution returned Same"),
                Verdict::AdjudicatedIntegerLexical => panic!("raw duplicates were adjudicated"),
            }
        }
    }

    #[test]
    fn raw_duplicate_nonadjacent_self_comparison_is_invalid() {
        for subject in ["<http://ex/s>", "_:x"] {
            // The literal also exercises mentions_blank_node's false-positive route.
            for object in ["<http://ex/o>", "\"contains _: text\""] {
                let p = format!("{subject} <http://ex/p> {object} .");
                let q = format!("{subject} <http://ex/q> {object} .");
                let rows = vec![p.clone(), q, p.clone()];
                for allow in [false, true] {
                    match compare("first", &rows, "second", &rows, allow) {
                        Verdict::Differs(detail) => {
                            assert!(detail
                                .starts_with("first: repeated raw full-quad line occurs 2 times"));
                            assert!(detail.ends_with(&p));
                        }
                        _ => panic!("duplicate-bearing snapshot must not compare equal to itself"),
                    }
                }
            }
        }
    }

    #[test]
    fn raw_duplicate_guard_preserves_symmetry_and_graph_identity() {
        let left = vec![
            "_:a <http://ex/p> _:b .".into(),
            "_:b <http://ex/p> _:a .".into(),
        ];
        let right = vec![
            "_:y <http://ex/p> _:z .".into(),
            "_:z <http://ex/p> _:y .".into(),
        ];
        assert!(matches!(
            compare("left", &left, "right", &right, false),
            Verdict::Same
        ));
        let rows = vec![
            "_:s <http://ex/p> <http://ex/o> <http://ex/g1> .".into(),
            "_:s <http://ex/p> <http://ex/o> <http://ex/g2> .".into(),
        ];
        assert!(matches!(
            compare("left", &rows, "right", &rows, false),
            Verdict::Same
        ));
        let duplicate = vec![rows[0].clone(), rows[0].clone()];
        assert!(matches!(
            compare("left", &duplicate, "right", &duplicate, false),
            Verdict::Differs(_)
        ));
    }

    #[test]
    fn raw_duplicate_probe_projection_contract() {
        assert_eq!(
            PROBES,
            &[
                ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"][..]),
                (
                    "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
                    &["s", "p", "o", "g"][..]
                ),
            ]
        );
    }

    #[test]
    fn raw_duplicate_probe_scope_is_complete_quads_in_both_engines() {
        let triple = "<http://ex/s> <http://ex/p> <http://ex/o>";
        let update = format!(
            "INSERT DATA {{ {triple} . GRAPH <http://ex/g1> {{ {triple} }} \
             GRAPH <http://ex/g2> {{ {triple} }} }}"
        );
        let graph = sparq_engine::update(&Graph::new(), &update).unwrap();
        let store = Store::new().unwrap();
        store.update(update.as_str()).unwrap();
        let snapshots = (sparq_nquads(&graph), oxi_nquads(&store).unwrap());
        assert_eq!(snapshots.0.len(), 3);
        assert_eq!(snapshots.0, snapshots.1);
        let expected = [
            vec![format!("{triple} .")],
            vec![
                format!("{triple} <http://ex/g1> ."),
                format!("{triple} <http://ex/g2> ."),
            ],
        ];
        for ((query, vars), expected) in PROBES.iter().zip(expected) {
            let sparq = sparq_probe(&graph, query).unwrap();
            let oxi = oxi_probe(&store, query, vars).unwrap();
            assert_eq!(sparq, expected, "Sparq scope for {query}");
            assert_eq!(oxi, expected, "Oxigraph scope for {query}");
            assert!(matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, false),
                Verdict::Same
            ));
        }
    }

    #[test]
    fn raw_duplicate_guard_preserves_canonicalization_errors() {
        for line in [
            "_:x not-an-iri <http://ex/o> .",
            "_:x <http://ex/p> <<( _:nested <http://ex/q> <http://ex/o> )>> .",
        ] {
            let rows = vec![line.to_string(), line.to_string()];
            match compare("left", &rows, "right", &rows, false) {
                Verdict::Differs(detail) => {
                    assert!(
                        detail.contains("re-parse failed")
                            || detail.contains("canonicalization failed")
                    );
                    assert!(!detail.contains("repeated raw full-quad"));
                }
                _ => panic!("existing parse/ground-profile error must stay strict"),
            }
        }
    }

    /// The per-PR BLOCKING smoke: a fixed seed window through the full three-way
    /// differential (both sparq update paths vs Oxigraph, canonical dataset + probe
    /// binding sets per step). The randomized soak (advancing window) is the nightly
    /// `differential-update.yml` lane, not this test — keep this window small enough
```

## Relevant unchanged behavior tests
```rust
    fn duplicate_quad_is_caught_despite_canonicalization() {
        let a = vec![
            "_:x <http://ex/p> <http://ex/o> .".to_string(),
            "_:x <http://ex/p> <http://ex/o> .".to_string(),
        ];
        let b = vec!["_:y <http://ex/p> <http://ex/o> .".to_string()];
        match compare("a", &a, "b", &b, false) {
            Verdict::Differs(d) => assert!(
                d.contains("COUNTS differ"),
                "expected the raw-count guard to fire, got:\n{}",
                d
            ),
            _ => panic!("a duplicated quad on one side must not compare equal"),
        }
    }

    /// sq-hodke (1), comparator side: the adjudicated integer-lexical class absorbs
    /// EXACTLY Oxigraph's normalization and nothing more.
    fn blank_node_compare_is_isomorphism_not_relabelling_blindness() {
        let a = vec![
            "_:fb0 <http://ex/p> <http://ex/o> .".to_string(),
            "_:fb0 <http://ex/q> _:fb1 <http://ex/g> .".to_string(),
        ];
        let relabelled = vec![
            "_:zzz <http://ex/p> <http://ex/o> .".to_string(),
            "_:zzz <http://ex/q> _:aaa <http://ex/g> .".to_string(),
        ];
        assert!(
            matches!(compare("a", &a, "b", &relabelled, false), Verdict::Same),
            "a pure blank-node relabelling is the SAME dataset"
        );
        let mut extra = relabelled.clone();
        extra.push("_:zzz <http://ex/r> <http://ex/o2> .".to_string());
        extra.sort();
        assert!(
            matches!(compare("a", &a, "b", &extra, false), Verdict::Differs(_)),
            "an extra edge on a blank node is NOT an isomorphism and must fail"
        );
        // A different blank-node *shape* with the same edge count must also fail.
        let reshaped = vec![
            "_:p <http://ex/p> <http://ex/o> .".to_string(),
            "_:q <http://ex/q> _:r <http://ex/g> .".to_string(),
        ];
        assert!(
            matches!(compare("a", &a, "b", &reshaped, false), Verdict::Differs(_)),
            "splitting one blank node into two is NOT an isomorphism and must fail"
        );
    }

    /// A duplicate quad on one side alone survives canonicalization (RDFC-1.0
    /// deduplicates), so the raw-count guard must catch it. Pins the check that keeps
    /// v1's duplicate sensitivity alive under the new comparator.
    fn integer_lexical_adjudication_is_narrow() {
        let ncl = "<http://ex/s> <http://ex/p> \
                   \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
            .to_string();
        let canonical = "<http://ex/s> <http://ex/p> \
                         \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
            .to_string();
        let collapsed = vec![ncl.clone(), canonical.clone()];
        let oxi = vec![canonical.clone()];
        assert!(
            matches!(
                compare("sparq", &collapsed, "oxigraph", &oxi, true),
                Verdict::Differs(_)
            ),
            "a reference-state collapse is no longer adjudicated"
        );
        let sparq = vec![ncl.clone()];
        assert!(
            matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, true),
                Verdict::AdjudicatedIntegerLexical
            ),
            "a one-to-one lexical rewrite must still exercise the adjudicated class"
        );
        assert!(
            matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, false),
                Verdict::Differs(_)
            ),
            "with the class DISABLED the same case must fail — including for every \
             sparq-vs-sparq compare, which never passes the flag"
        );
        // An unrelated missing quad alongside the lexical difference must NOT be
        // absorbed: the class only fires when normalization makes the sets agree.
        let mut sparq_plus = sparq.clone();
        sparq_plus.push("<http://ex/s> <http://ex/q> <http://ex/o> .".to_string());
        sparq_plus.sort();
        assert!(
            matches!(
                compare("sparq", &sparq_plus, "oxigraph", &oxi, true),
                Verdict::Differs(_)
            ),
            "a real missing quad must survive the adjudication"
        );
        // A lexical difference the reference engine does NOT make is not this class.
        let lang = vec!["<http://ex/s> <http://ex/p> \"a\"@en .".to_string()];
        let lang2 = vec!["<http://ex/s> <http://ex/p> \"a\"@fr .".to_string()];
        assert!(
            matches!(
                compare("sparq", &lang, "oxigraph", &lang2, true),
                Verdict::Differs(_)
            ),
            "the class must not absorb a language-tag difference"
        );
    }

    /// Pins the committed registry ⟷ this comparator's detectors. A new `update-*`
    /// class added to the JSON without a detector here must NOT silently start
    /// absorbing divergences.
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

```

## Both-engine graph-scope source proof (unchanged production dependencies)
{
  "finding": "No source contradiction: unqualified Sparq query reads graph.store; GRAPH iterates named subgraphs. Oxigraph Store::query uses new/default evaluator and QueryDatasetSpecification default is only GraphName::DefaultGraph, with named unrestricted. No union-setting method called by comparator probes. Runtime same-SPO default+two-named control required in sole candidate run.",
  "prior_compare_tests": "Inspected existing duplicate tests: unequal-count duplicate expectsCOUNTS; distinct-lexical redistribution expectsDiffers; no duplicate-bearing self-Same expectation found in the20existingtests. Full original20 will execute unchanged.",
  "sources": [
    {
      "path": "task-evidence/worktrees/issue6483/crates/sparq-engine/src/lib.rs",
      "sha256": "074740149d0649b0334d1814ca289feaad54070ca0d90bf5737601f7021a7538"
    },
    {
      "path": "task-evidence/worktrees/issue6483/crates/sparq-engine/src/lib.rs",
      "sha256": "074740149d0649b0334d1814ca289feaad54070ca0d90bf5737601f7021a7538"
    },
    {
      "path": "task-evidence/worktrees/issue6483/crates/sparq-engine/src/exec.rs",
      "sha256": "10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af"
    },
    {
      "path": "task-evidence/worktrees/issue6483/crates/sparq-engine/src/exec.rs",
      "sha256": "10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af"
    },
    {
      "path": "host-user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/oxigraph-0.5.9/src/store.rs",
      "sha256": "076529d63b49107b6d0b0dd690a3891381945bbb0a2e619059cb33a9c0a95e32"
    },
    {
      "path": "host-user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/oxigraph-0.5.9/src/store.rs",
      "sha256": "076529d63b49107b6d0b0dd690a3891381945bbb0a2e619059cb33a9c0a95e32"
    },
    {
      "path": "host-user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/spareval-0.2.6/src/lib.rs",
      "sha256": "a470a4300ad10a21035b95cf721549727aaf4a3c5d51de05cd3103b95758f2e9"
    },
    {
      "path": "host-user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/spareval-0.2.6/src/lib.rs",
      "sha256": "a470a4300ad10a21035b95cf721549727aaf4a3c5d51de05cd3103b95758f2e9"
    }
  ]
}

```text
crates/sparq-engine/src/lib.rs:812-831
812: /// When the query carries a dataset clause (FROM / FROM NAMED), the ACTIVE
813: /// dataset it describes — built from the store's named graphs; see
814: /// [`dataset::build_active`]. `None` (the common case) means: evaluate against
815: /// the store itself. Every query entry point calls this once after parsing, so
816: /// the no-clause path costs exactly one `Option` check.
817: pub(crate) fn active_dataset(graph: &Graph, q: &Query) -> Option<Graph> {
818:     q.dataset().map(|ds| dataset::build_active(graph, ds))
819: }
820: 
821: /// Suspends an installed [`DatasetView`] while a dataset-clause query evaluates:
822: /// [`dataset::build_active`] has already INTERSECTED the clause with the view
823: /// (non-visible ≡ absent), so re-filtering during evaluation would make a
824: /// non-visible `FROM NAMED` graph distinguishable from an absent one (both must
825: /// be the empty active graph, with its unit-row `GRAPH <g> {}` semantics). A
826: /// no-op when no view is installed or the query has no dataset clause.
827: pub(crate) fn view_scope(active: &Option<Graph>) -> Option<exec::view::Guard> {
828:     active.is_some().then(exec::view::suspend_all)
829: }
830: 
831: /// A SPARQL query parsed ONCE for repeated execution — the parse/plan-once seam

crates/sparq-engine/src/lib.rs:1016-1054
1016: /// Executes a SPARQL query string against a graph, materialising the solutions.
1017: pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
1018:     query_with_budget(graph, sparql, &QueryBudget::unlimited())
1019: }
1020: 
1021: /// [`query`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1022: pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
1023:     query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
1024: }
1025: 
1026: /// [`query`] over a [`PreparedQuery`] — no per-execution parse.
1027: pub fn query_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<QueryResult, String> {
1028:     query_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
1029: }
1030: 
1031: /// [`query_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1032: pub fn query_prepared_with_budget(
1033:     graph: &Graph,
1034:     prepared: &PreparedQuery,
1035:     budget: &QueryBudget,
1036: ) -> Result<QueryResult, String> {
1037:     let q = &prepared.query;
1038:     let active = active_dataset(graph, q);
1039:     let graph = active.as_ref().unwrap_or(graph);
1040:     let _view_scope = view_scope(&active);
1041:     exec::budget::with_budget(budget, || {
1042:         exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1043:         match q {
1044:             Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
1045:             // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
1046:             // is satisfiable — the standard "unit row" encoding of a boolean result.
1047:             Query::Ask { pattern, .. } => Ok(QueryResult {
1048:                 vars: Vec::new(),
1049:                 rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
1050:             }),
1051:             _ => Err("only SELECT and ASK queries are supported".into()),
1052:         }
1053:     })
1054: }

crates/sparq-engine/src/exec.rs:5048-5205
5048: fn eval_graph_named_pref(
5049:     graph: &Graph,
5050:     local: &mut LocalVocab,
5051:     name: &NamedNodePattern,
5052:     inner: &GraphPattern,
5053:     prefix: Option<&str>,
5054: ) -> Result<Bindings, String> {
5055:     fn eval_translated(
5056:         graph: &Graph,
5057:         local: &mut LocalVocab,
5058:         sub: &Graph,
5059:         #[cfg(feature = "zk")] gname: &Term,
5060:         inner: &GraphPattern,
5061:     ) -> Result<Bindings, String> {
5062:         // Inside GRAPH the evaluation graph IS the named sub-graph: suspend a
5063:         // view's empty-default short-circuit for the inner pattern (L1 view).
5064:         let _scope = view::enter_graph();
5065:         // zk-trace: tag the enclosed scans/filters with the named graph (the
5066:         // sub-graph has its own dictionary; terms are materialized at record
5067:         // time against it, so the tag is what attributes them). The `gname`
5068:         // parameter is cfg'd out entirely when the feature is off, so the
5069:         // default (wasm) build is byte-identical.
5070:         #[cfg(feature = "zk")]
5071:         let _zk = crate::zk::graph_scope(gname);
5072:         let mut sub_local = LocalVocab::default();
5073:         let b = eval_graph_pattern(sub, &mut sub_local, inner)?;
5074:         let rows: Vec<Row> = b
5075:             .rows
5076:             .iter()
5077:             .map(|r| {
5078:                 r.iter()
5079:                     .map(|&id| match term_of(sub, &sub_local, id) {
5080:                         Some(t) => value_to_id(graph, local, &Value::Term(t)),
5081:                         None => NO_ID,
5082:                     })
5083:                     .collect()
5084:             })
5085:             .collect();
5086:         Ok(Bindings::unsorted(b.vars, rows))
5087:     }
5088:     match name {
5089:         NamedNodePattern::NamedNode(n) => {
5090:             let target = Term::NamedNode(n.clone());
5091:             // A graph outside an installed dataset view takes the absent-graph
5092:             // branch below: non-visible must be INDISTINGUISHABLE from absent
5093:             // (the L1 view's security property).
5094:             let sub = if view::allows(&target) {
5095:                 graph.named.iter().find(|(t, _)| *t == target).map(|(_, sub)| sub)
5096:             } else {
5097:                 None
5098:             };
5099:             match sub {
5100:                 Some(sub) => eval_translated(
5101:                     graph,
5102:                     local,
5103:                     sub,
5104:                     #[cfg(feature = "zk")]
5105:                     &target,
5106:                     inner,
5107:                 ),
5108:                 // The named graph is absent → ZERO solutions (even for `GRAPH <g> {}`,
5109:                 // which must NOT yield the unit row), but with `inner`'s variable
5110:                 // schema — evaluate against an empty graph for the columns, then drop
5111:                 // any rows (an empty group pattern would otherwise produce one).
5112:                 None => {
5113:                     let _scope = view::enter_graph(); // schema eval matches the present-graph path
5114:                     // zk-trace: an absent graph still records the operator
5115:                     // boundary + (empty) pattern input sets under its name.
5116:                     #[cfg(feature = "zk")]
5117:                     let _zk = crate::zk::graph_scope(&target);
5118:                     let empty = Graph::load_str("", "ntriples").map_err(|e| e.to_string())?;
5119:                     let mut el = LocalVocab::default();
5120:                     let mut b = eval_graph_pattern(&empty, &mut el, inner)?;
5121:                     b.rows.clear();
5122:                     Ok(b)
5123:                 }
5124:             }
5125:         }
5126:         NamedNodePattern::Variable(v) => {
5127:             // [OPUS-4.8] (sq-zz8z) Accumulate the per-graph relations into ONE flat row buffer in
5128:             // a stable column schema (`?g` first, then the inner pattern's columns) rather than
5129:             // folding with `union_bindings` once per graph. The old fold re-copied the WHOLE
5130:             // accumulated relation on every graph, making `GRAPH ?g` over G graphs O(G²); a single
5131:             // shared schema makes it O(total rows). The `?g`-first schema matches the old
5132:             // `None`-branch insert order, so projected results are unchanged.
5133:             let mut out_vars: Option<Vec<Variable>> = None;
5134:             let mut out_rows: Vec<Row> = Vec::new();
5135:             let mut per_graph = |graph: &Graph, local: &mut LocalVocab, gname: &Term, sub: &Graph| -> Result<(), String> {
5136:                 // zk-trace: each iteration of `GRAPH ?g` tags the enclosed scans/filters with the
5137:                 // iteration's named graph — the scope is installed INSIDE eval_translated (one
5138:                 // place), so the operator boundary stream is not double-nested.
5139:                 let mut b = eval_translated(
5140:                     graph,
5141:                     local,
5142:                     sub,
5143:                     #[cfg(feature = "zk")]
5144:                     gname,
5145:                     inner,
5146:                 )?;
5147:                 let gid = value_to_id(graph, local, &Value::Term(gname.clone()));
5148:                 // Resolve this graph's columns into the shared `?g`-first schema (set on the first
5149:                 // graph; every named sub-graph yields the same `inner` schema, so it is stable).
5150:                 let schema = out_vars.get_or_insert_with(|| {
5151:                     let mut s = Vec::with_capacity(b.vars.len() + 1);
5152:                     s.push(v.clone());
5153:                     for var in &b.vars {
5154:                         if var != v {
5155:                             s.push(var.clone());
5156:                         }
5157:                     }
5158:                     s
5159:                 });
5160:                 match b.col(v) {
5161:                     // The inner pattern itself binds the graph variable (e.g.
5162:                     // `GRAPH ?g { ?g :p ?o }` or a VALUES/OPTIONAL inside): JOIN with
5163:                     // the active graph name — keep rows already bound to this graph,
5164:                     // fill unbound cells, drop conflicting rows.
5165:                     Some(c) => {
5166:                         b.rows.retain_mut(|row| {
5167:                             if row[c] == NO_ID {
5168:                                 row[c] = gid;
5169:                                 true
5170:                             } else {
5171:                                 row[c] == gid
5172:                             }
5173:                         });
5174:                     }
5175:                     None => {
5176:                         b.vars.insert(0, v.clone());
5177:                         for row in &mut b.rows {
5178:                             row.insert(0, gid);
5179:                         }
5180:                     }
5181:                 }
5182:                 // Map each row into the shared schema (column positions can differ per graph only
5183:                 // if the inner schema ever reordered — it does not — so this is a cheap permute).
5184:                 // Precompute schema-column -> position-in-`b.vars` ONCE per binding-set (instead of
5185:                 // an O(vars) linear `position(..)` search per (row, var) cell — an O(rows·vars²)
5186:                 // hotspot on large `GRAPH ?g` scans), then map each row with O(1) indexed lookups.
5187:                 let col_map: Vec<Option<usize>> = schema
5188:                     .iter()
5189:                     .map(|var| b.vars.iter().position(|x| x == var))
5190:                     .collect();
5191:                 for row in &b.rows {
5192:                     out_rows.push(
5193:                         col_map
5194:                             .iter()
5195:                             .map(|&pos| pos.map(|i| row[i]).unwrap_or(NO_ID))
5196:                             .collect(),
5197:                     );
5198:                 }
5199:                 Ok(())
5200:             };
5201:             match prefix {
5202:                 // Indexed range scan over only the prefix-matching graphs (O(log G + matches)).
5203:                 // The view-visibility (L1) check stays — a non-visible graph is still skipped.
5204:                 Some(pref) => {
5205:                     let mut err: Option<String> = None;

crates/sparq-engine/src/exec.rs:8874-8906
8874: fn scan_to_bindings(
8875:     graph: &Graph,
8876:     id_pat: &IdPattern,
8877:     pos_vars: &[Option<Variable>; 3],
8878:     sort_col: Option<usize>,
8879:     filter: Option<(usize, ScanCmp)>,
8880:     limit: Option<usize>,
8881:     // [OPUS-4.8] (sq-gr8mb / §A3) Optional semi-join prefilter: `(canonical position of
8882:     // the connecting variable, membership filter over the other side's join keys)`. A
8883:     // scanned row whose key at that position is ABSENT from the filter cannot match the
8884:     // downstream join, so it is dropped before projection. The filter is membership-exact
8885:     // (no false positives), so this never changes the RESULT — only fewer rows are kept.
8886:     // Only present under the opt-in `semijoin-bitmap` feature, so the default build's
8887:     // signature and per-row path are byte-identical.
8888:     #[cfg(feature = "semijoin-bitmap")] prefilter: Option<(usize, &crate::semijoin::KeyFilter)>,
8889: ) -> Bindings {
8890:     let mut vars: Vec<Variable> = Vec::new();
8891:     let mut var_positions: Vec<Vec<usize>> = Vec::new();
8892:     for (pos, v) in pos_vars.iter().enumerate() {
8893:         if let Some(v) = v {
8894:             if let Some(idx) = vars.iter().position(|x| x == v) {
8895:                 var_positions[idx].push(pos);
8896:             } else {
8897:                 vars.push(v.clone());
8898:                 var_positions.push(vec![pos]);
8899:             }
8900:         }
8901:     }
8902:     let scan = match sort_col {
8903:         Some(c) => graph.store.scan_sorted(id_pat, c),
8904:         None => graph.store.scan(id_pat),
8905:     };
8906:     // The TRUE sort column is the first unbound canonical column in the chosen

index.crates.io-1949cf8c6b5b557f/oxigraph-0.5.9/src/store.rs:240-249
240:     /// ```
241:     #[deprecated(note = "Use `SparqlEvaluator` interface instead", since = "0.5.0")]
242:     #[expect(deprecated)]
243:     pub fn query(
244:         &self,
245:         query: impl TryInto<Query, Error = impl Into<QueryEvaluationError>>,
246:     ) -> Result<QueryResults<'static>, QueryEvaluationError> {
247:         self.query_opt(query, SparqlEvaluator::new())
248:     }
249: 

index.crates.io-1949cf8c6b5b557f/oxigraph-0.5.9/src/store.rs:309-321
309:     pub fn query_opt_with_substituted_variables(
310:         &self,
311:         query: impl TryInto<Query, Error = impl Into<QueryEvaluationError>>,
312:         options: SparqlEvaluator,
313:         substitutions: impl IntoIterator<Item = (Variable, Term)>,
314:     ) -> Result<QueryResults<'static>, QueryEvaluationError> {
315:         let mut evaluator = options.for_query(query.try_into().map_err(Into::into)?);
316:         for (variable, term) in substitutions {
317:             evaluator = evaluator.substitute_variable(variable, term);
318:         }
319:         evaluator.on_store(self).execute()
320:     }
321: 

index.crates.io-1949cf8c6b5b557f/spareval-0.2.6/src/lib.rs:886-917
886: 
887: /// An extended SPARQL query [dataset specification](https://www.w3.org/TR/sparql11-query/#specifyingDataset).
888: ///
889: /// Allows setting blank node graph names and that the default graph is the union of all named graphs.
890: #[derive(Eq, PartialEq, Debug, Clone, Hash)]
891: pub struct QueryDatasetSpecification {
892:     default: Option<Vec<GraphName>>,
893:     named: Option<Vec<NamedOrBlankNode>>,
894: }
895: 
896: impl QueryDatasetSpecification {
897:     pub fn new() -> Self {
898:         Self {
899:             default: Some(vec![GraphName::DefaultGraph]),
900:             named: None,
901:         }
902:     }
903: 
904:     /// Checks if this dataset specification is the default one
905:     /// (i.e., the default graph is the store default graph, and all named graphs included in the queried store are available)
906:     pub fn is_default_dataset(&self) -> bool {
907:         // TODO: rename to is_default?
908:         self.default
909:             .as_ref()
910:             .is_some_and(|t| t == &[GraphName::DefaultGraph])
911:             && self.named.is_none()
912:     }
913: 
914:     /// Returns the list of the store graphs that are available to the query as the default graph or `None` if the union of all graphs is used as the default graph.
915:     /// This list is by default only the store default graph.
916:     pub fn default_graph_graphs(&self) -> Option<&[GraphName]> {
917:         self.default.as_deref()

index.crates.io-1949cf8c6b5b557f/spareval-0.2.6/src/lib.rs:1020-1039
1020: impl Default for QueryDatasetSpecification {
1021:     fn default() -> Self {
1022:         Self::new()
1023:     }
1024: }
1025: 
1026: impl From<QueryDataset> for QueryDatasetSpecification {
1027:     fn from(dataset: QueryDataset) -> Self {
1028:         Self {
1029:             default: Some(dataset.default.into_iter().map(Into::into).collect()),
1030:             named: dataset
1031:                 .named
1032:                 .map(|named| named.into_iter().map(Into::into).collect()),
1033:         }
1034:     }
1035: }
1036: 
1037: /// The explanation of a query.
1038: #[derive(Clone)]
1039: pub struct QueryExplanation {
```

## Actual test output
```text

running 26 tests
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
test update_fuzz::tests::raw_duplicate_guard_preserves_canonicalization_errors ... ok
test update_fuzz::tests::raw_duplicate_guard_preserves_symmetry_and_graph_identity ... ok
test update_fuzz::tests::raw_duplicate_nonadjacent_self_comparison_is_invalid ... ok
test update_fuzz::tests::raw_duplicate_probe_projection_contract ... ok
test update_fuzz::tests::raw_duplicate_probe_scope_is_complete_quads_in_both_engines ... ok
test update_fuzz::tests::raw_duplicate_redistribution_is_rejected ... ok
test update_fuzz::tests::snapshot_renderers_agree_on_known_dataset ... ok

test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.03s

warning: divergence allowlist lists update class "update-not-a-real-class" but update-fuzz has no detector for it — that class stays STRICT
injective corpus: noncanonical=22, LOAD=239, triple terms=1455
historical index8: 9 solutions, 9 fresh blank nodes, 4 for s2; both lexical terms retained
historical index9: ADD completed; all 10 original requests checked internally
```

## Guard-removal control
```diff
--- candidate/update_fuzz.rs
+++ control/update_fuzz.rs
@@ -986,16 +986,6 @@
                 b.len()
             ));
         }
-        for (label, lines) in [(label_a, a), (label_b, b)] {
-            if let Some((line, count)) = repeated_raw_line(lines) {
-                return Verdict::Differs(format!(
-                    "{label}: repeated raw full-quad line occurs {count} times \
-                     (raw totals: {label_a} {}, {label_b} {}):\n  {line}",
-                    a.len(),
-                    b.len()
-                ));
-            }
-        }
         return Verdict::Same;
     }
     if allow_integer_lexical {
```
```text

running 1 test
test update_fuzz::tests::raw_duplicate_redistribution_is_rejected ... FAILED

failures:

failures:
    update_fuzz::tests::raw_duplicate_redistribution_is_rejected

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 25 filtered out; finished in 0.01s

equal-total raw duplicate redistribution returned Same
```

## Runtime/profile receipts
[
  {
    "name": "candidate-tests",
    "exit": 0,
    "expected_exit": 0,
    "seconds": 2.111939875,
    "stopped": null
  },
  {
    "name": "control-build",
    "exit": 0,
    "expected_exit": 0,
    "seconds": 11.586124875000001,
    "stopped": null
  },
  {
    "name": "control-test",
    "exit": 101,
    "expected_exit": 101,
    "seconds": 0.7230305000000001,
    "stopped": null
  },
  {
    "name": "clippy-tests",
    "exit": 0,
    "expected_exit": 0,
    "seconds": 0.7263217920000002,
    "stopped": null
  }
]
{
  "edition": "2021",
  "native_target": "aarch64-apple-darwin",
  "recorded_feature_proof_sha256": "9de5da65386c2c87bf6ff20e95294020c881d8e2c30a9f0d938d9ffcd96a3faf",
  "selected_features": {
    "oxigraph@0.5.9": [
      "rdf-12"
    ],
    "oxrdf@0.2.4": [
      "default"
    ],
    "oxrdf@0.3.3": [
      "default",
      "oxsdatatypes",
      "rdf-12",
      "rdfc-10"
    ],
    "rdf-canon@0.15.3": [],
    "spargebra@0.4.6": [
      "default",
      "sep-0002",
      "sep-0006",
      "sparql-12"
    ],
    "sparq-canon@0.1.1": [
      "default",
      "parallel",
      "rdf12-triple-terms"
    ],
    "sparq-core@0.1.1": [
      "default",
      "dict-spill",
      "mmap",
      "parallel"
    ],
    "sparq-engine@0.1.1": [
      "algebra-rewrite",
      "default",
      "digest",
      "parallel",
      "regex"
    ],
    "sparq-substrate@0.1.1": [
      "compare",
      "join",
      "numeric",
      "rows"
    ]
  }
}


## Context limits
The complete final module and all raw local commands, binaries, diagnostics and hashes are frozen separately. Unchanged generator bodies and the full unchanged canonicalization implementation are not repeated in this packet; canonicalization-apis.txt supplies complete reusable issuer context locally. Prior independent design review considered that API and selected a raw-emission guard. This patch changes no engine or canonicalizer source.
