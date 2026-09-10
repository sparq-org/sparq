# Issue6485: preserve numeric memo after dictionary-stable deltas

Author: GPT-6 Astra xhigh. Exact candidate 73769114f081c8455aa21a393d0b992bd1283b67; parent f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464. Actual Opus design review was conditional; this is the first implementation for final review.

## Change, invariant and verification

```json
{
  "scope": "One file,214 added/7 removed lines. Runtime delta is entry capture and a condition around existing CAS; most additions are four focused tests and small public-field docs. No API signature, cache representation, dependency, workflow or policy change.",
  "reason": "Nonempty deltas that introduce no stored dictionary terms need not discard the memoized absence of high-precision decimals. Existing state2 remains sticky and first cold dictionary scan is unchanged.",
  "f4": "Full Graph::id_of/Dict::lookup recursion plus mapped/frozen/table finder and equality source is supplied. Deletes are lookup-only; entry capture is neutral.",
  "invariant": "Coordinated Graph mutation interns append-only terms, extends numeric cache only for new IDs, and does not change old ID/value pairs without growth. New guard comment requires future old-entry updates to invalidate. Public fields are documented as bypassing caches; from_parts builds a replacement using coherent matching ID triples. No global guarantee for arbitrary field replacements.",
  "preflight": {
    "exit": 1,
    "passing_checks": [
      "G1 new-crate-completeness",
      "G2 public-api-to-skill",
      "G6 new-config-to-docs",
      "guard-untested"
    ],
    "sole_finding": "privacy-claims shell line92 uses mapfile unavailable in local Bash3; BrokenPipeError follows. Gate unchanged; actual Linux preflight/privacy remains required.",
    "skips": [
      "no-perf-numbers: no matching diff path",
      "readme-template: no matching diff path"
    ]
  },
  "format_diff": "Only the new test block was formatted with pinned rustfmt; final block check and git diff --check passed. Compiled candidate bytes equal committed file.",
  "profile": {
    "compiler": "Rust1.97.1 (8bab26f4f), aarch64-apple-darwin",
    "edition": 2021,
    "optimization": "O3/unwind/LTOoff/codegen16",
    "default": [
      "default",
      "parallel"
    ],
    "mapped": [
      "default",
      "parallel",
      "mmap",
      "dict-spill"
    ],
    "routes": "Direct rustc --test adaptation of actual recorded lib invocations; no Cargo/dependency compilation. Candidate and four mutants used7 rehashed direct rlibs; mapped used8 recorded-feature rlibs plus same proptest dev dependency.",
    "clippy": "Pinned clippy-driver, same source/extern cfgs, -D warnings, metadata-only; default lib/default cfgtest/mapped cfgtest all exit0.",
    "scope_limits": "Not full Cargo/workspace CI. Mapped WAL test actually ran; cfg dict-spill code compiled, but no new dict-spill ingest runtime test was selected. compact-index/wasm and broad gates remain CI obligations."
  },
  "no_latency_claim": "Predicate-visit baseline is preserved in separate bundles; this candidate uses memo-state assertions. No measured latency, throughput, heap or public SPARQL query benefit claimed.",
  "remaining": [
    "Actual Opus final patch review before publication.",
    "Authoritative full-workspace Clippy/tests, feature matrix including compact-index/dict-spill, wasm and applicable ratchets; Linux privacy check.",
    "Parent3113 first cold scan remains open; held PR4097/4354/4247 untouched.",
    "An intentional always-compiled optimizer declaration may be required under the normal per-PR feature-off process once root assigns a PR number; no byte-neutrality claim or wasm-floor change."
  ]
}
```

## Actual test/control results

```json
[
  {
    "variant": "candidate",
    "exit": 0,
    "summary": "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 149 filtered out",
    "passed": 4,
    "failed": 0,
    "filtered": 149,
    "failed_tests": [],
    "expected_behavior_observed": true,
    "binary_sha256": "a9cf407d004ebfaa2da3f3619cde149ccba5c672bce0816a0faf447391feb5cd"
  },
  {
    "variant": "unconditional-cas",
    "exit": 101,
    "summary": "test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 149 filtered out",
    "passed": 2,
    "failed": 2,
    "filtered": 149,
    "failed_tests": [
      "tests::has_high_precision_decimal_memo_fork_compact_isolation",
      "tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary"
    ],
    "expected_behavior_observed": true,
    "binary_sha256": "f2e538069b12f34f8dcdf9cee97e2396555de4aa015146d70e4b8b5626973b50"
  },
  {
    "variant": "no-cas",
    "exit": 101,
    "summary": "test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 149 filtered out",
    "passed": 1,
    "failed": 3,
    "filtered": 149,
    "failed_tests": [
      "tests::has_high_precision_decimal_memo_fork_compact_isolation",
      "tests::has_high_precision_decimal_memo_invalidates_stored_growth",
      "tests::has_high_precision_decimal_memo_survives_delta_insert"
    ],
    "expected_behavior_observed": true,
    "binary_sha256": "d98bfa8bcde6bdc0502e53eeb0bdb6a71b24fb09ad773c24cb1891f6d7f00973"
  },
  {
    "variant": "inverted-guard",
    "exit": 101,
    "summary": "test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 149 filtered out",
    "passed": 0,
    "failed": 4,
    "filtered": 149,
    "failed_tests": [
      "tests::has_high_precision_decimal_memo_fork_compact_isolation",
      "tests::has_high_precision_decimal_memo_invalidates_stored_growth",
      "tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary",
      "tests::has_high_precision_decimal_memo_survives_delta_insert"
    ],
    "expected_behavior_observed": true,
    "binary_sha256": "19bfe4fc56883993a1b8a1cc261647ec00d8e2ac145158b93e10ae7aad344844"
  },
  {
    "variant": "self-length",
    "exit": 101,
    "summary": "test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 149 filtered out",
    "passed": 1,
    "failed": 3,
    "filtered": 149,
    "failed_tests": [
      "tests::has_high_precision_decimal_memo_fork_compact_isolation",
      "tests::has_high_precision_decimal_memo_invalidates_stored_growth",
      "tests::has_high_precision_decimal_memo_survives_delta_insert"
    ],
    "expected_behavior_observed": true,
    "binary_sha256": "79c656a599892d5648f3f268702bf284934e1ce23618aecbb75bb11d841b6eeb"
  },
  {
    "variant": "mmap-candidate",
    "exit": 0,
    "summary": "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 225 filtered out",
    "passed": 5,
    "failed": 0,
    "filtered": 225,
    "failed_tests": [],
    "expected_behavior_observed": true,
    "binary_sha256": "8ce47c4cc1fb4091d4cd16dfda3ab1b77a0b6c779113b6bc31fdd2c507f930bb"
  }
]
```

### candidate

```text

running 4 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... ok
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... ok
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... ok
test tests::has_high_precision_decimal_memo_survives_delta_insert ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s


```

### unconditional-cas

```text

running 4 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... FAILED
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... ok
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... FAILED
test tests::has_high_precision_decimal_memo_survives_delta_insert ... ok

failures:

failures:
    tests::has_high_precision_decimal_memo_fork_compact_isolation
    tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary

test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s


```

```text

thread 'tests::has_high_precision_decimal_memo_fork_compact_isolation' (5018662) panicked at diagnostic/sources/unconditional-cas/core/src/lib.rs:9761:9:
assertion `left == right` failed
  left: 0
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary' (5018665) panicked at diagnostic/sources/unconditional-cas/core/src/lib.rs:9674:13:
assertion `left == right` failed: delete, sparse=false
  left: 0
 right: 1

```

### no-cas

```text

running 4 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... FAILED
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... FAILED
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... ok
test tests::has_high_precision_decimal_memo_survives_delta_insert ... FAILED

failures:

failures:
    tests::has_high_precision_decimal_memo_fork_compact_isolation
    tests::has_high_precision_decimal_memo_invalidates_stored_growth
    tests::has_high_precision_decimal_memo_survives_delta_insert

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s


```

```text

thread 'tests::has_high_precision_decimal_memo_fork_compact_isolation' (5018809) panicked at diagnostic/sources/no-cas/core/src/lib.rs:9777:9:
assertion failed: parent.has_high_precision_decimal()
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'tests::has_high_precision_decimal_memo_invalidates_stored_growth' (5018811) panicked at diagnostic/sources/no-cas/core/src/lib.rs:9721:17:
assertion `left == right` failed: growth: 1.5
  left: 1
 right: 0

thread 'tests::has_high_precision_decimal_memo_survives_delta_insert' (5018813) panicked at diagnostic/sources/no-cas/core/src/lib.rs:9636:9:
delta-inserted inexact decimal must flip the memo

```

### inverted-guard

```text

running 4 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... FAILED
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... FAILED
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... FAILED
test tests::has_high_precision_decimal_memo_survives_delta_insert ... FAILED

failures:

failures:
    tests::has_high_precision_decimal_memo_fork_compact_isolation
    tests::has_high_precision_decimal_memo_invalidates_stored_growth
    tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary
    tests::has_high_precision_decimal_memo_survives_delta_insert

test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.01s


```

```text

thread 'tests::has_high_precision_decimal_memo_fork_compact_isolation' (5018953) panicked at diagnostic/sources/inverted-guard/core/src/lib.rs:9763:9:
assertion `left == right` failed
  left: 0
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'tests::has_high_precision_decimal_memo_invalidates_stored_growth' (5018956) panicked at diagnostic/sources/inverted-guard/core/src/lib.rs:9729:17:
assertion `left == right` failed: growth: 1.5
  left: 1
 right: 0

thread 'tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary' (5018957) panicked at diagnostic/sources/inverted-guard/core/src/lib.rs:9676:13:
assertion `left == right` failed: delete, sparse=false
  left: 0
 right: 1

thread 'tests::has_high_precision_decimal_memo_survives_delta_insert' (5018958) panicked at diagnostic/sources/inverted-guard/core/src/lib.rs:9644:9:
delta-inserted inexact decimal must flip the memo

```

### self-length

```text

running 4 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... FAILED
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... FAILED
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... ok
test tests::has_high_precision_decimal_memo_survives_delta_insert ... FAILED

failures:

failures:
    tests::has_high_precision_decimal_memo_fork_compact_isolation
    tests::has_high_precision_decimal_memo_invalidates_stored_growth
    tests::has_high_precision_decimal_memo_survives_delta_insert

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s


```

```text

thread 'tests::has_high_precision_decimal_memo_fork_compact_isolation' (5019128) panicked at diagnostic/sources/self-length/core/src/lib.rs:9785:9:
assertion failed: parent.has_high_precision_decimal()
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'tests::has_high_precision_decimal_memo_invalidates_stored_growth' (5019130) panicked at diagnostic/sources/self-length/core/src/lib.rs:9729:17:
assertion `left == right` failed: growth: 1.5
  left: 1
 right: 0

thread 'tests::has_high_precision_decimal_memo_survives_delta_insert' (5019132) panicked at diagnostic/sources/self-length/core/src/lib.rs:9644:9:
delta-inserted inexact decimal must flip the memo

```

### mmap-candidate

```text

running 5 tests
test tests::has_high_precision_decimal_memo_fork_compact_isolation ... ok
test tests::has_high_precision_decimal_memo_invalidates_stored_growth ... ok
test tests::has_high_precision_decimal_memo_mapped_wal_precision ... ok
test tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary ... ok
test tests::has_high_precision_decimal_memo_survives_delta_insert ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 225 filtered out; finished in 0.33s


```

## Full final diff

```diff
diff --git a/crates/sparq-core/src/lib.rs b/crates/sparq-core/src/lib.rs
index ab2cf0136..d1bbbbc08 100644
--- a/crates/sparq-core/src/lib.rs
+++ b/crates/sparq-core/src/lib.rs
@@ -62,7 +62,12 @@ use store::{Pattern, TripleStore};
 
 /// An immutable, dictionary-encoded RDF graph ready for querying.
 pub struct Graph {
+    /// Term dictionary. [GPT-6 Astra] Direct writes bypass numeric/temporal cache
+    /// and memo maintenance; use [`Self::from_parts`] to rebuild a graph from an
+    /// edited dictionary and its matching ID triples.
     pub dict: Dict,
+    /// Indexes using [`Self::dict`]'s IDs. Direct replacement does not rebuild
+    /// private caches; use [`Self::from_parts`] for a coherent replacement graph.
     pub store: TripleStore,
     /// Parallel to the dictionary: the f64 value of each numeric literal (NaN for
     /// non-numeric terms). Lets the engine evaluate numeric filters / comparisons
@@ -3370,13 +3375,13 @@ impl Graph {
     /// The in-memory half of [`apply_delta`](Self::apply_delta) (no WAL append) — also
     /// the target the WAL replays into on [`open`](Self::open).
     fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
+        let old_len = self.dict.len();
         // A delete only matters if every term resolves — otherwise the triple cannot be
         // present, and deleting must NOT intern the (absent) terms.
         let del_ids: Vec<[Id; 3]> = deletes
             .iter()
             .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
             .collect();
-        let old_len = self.dict.len();
         let ins_ids: Vec<[Id; 3]> = inserts
             .iter()
             .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
@@ -3387,12 +3392,17 @@ impl Graph {
         // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
         // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
         // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
-        let _ = self.high_precision_decimal.compare_exchange(
-            1,
-            0,
-            std::sync::atomic::Ordering::Relaxed,
-            std::sync::atomic::Ordering::Relaxed,
-        );
+        // [GPT-6 Astra] Interning is append-only and extend_for touches only new IDs.
+        // Equal lengths therefore preserve every term/numeric value read by the memo.
+        // Any future path changing existing terms or cached values must invalidate it.
+        if self.dict.len() != old_len {
+            let _ = self.high_precision_decimal.compare_exchange(
+                1,
+                0,
+                std::sync::atomic::Ordering::Relaxed,
+                std::sync::atomic::Ordering::Relaxed,
+            );
+        }
         self.store.apply_delta(&ins_ids, &del_ids);
     }
 
@@ -9634,6 +9644,203 @@ mod tests {
         assert!(g.has_high_precision_decimal(), "delta-inserted inexact decimal must flip the memo");
     }
 
+    // [GPT-6 Astra] #6485: observe the memo itself so an unnecessary rescan fails.
+    #[test]
+    fn has_high_precision_decimal_memo_preserves_unchanged_dictionary() {
+        use std::sync::atomic::Ordering::Relaxed;
+        for sparse in [false, true] {
+            let mut g = Graph::load_str(
+                "@prefix : <urn:> . :s :p 7 . :other :p 8 . :s :exact 1.5 .",
+                "turtle",
+            )
+            .unwrap();
+            if sparse {
+                g.numerics = NumData::Sparse(
+                    (1..=g.dict.len() as Id)
+                        .filter_map(|id| g.numerics.lookup(id).map(|value| (id, value)))
+                        .collect(),
+                );
+            }
+            let triple = |s: &str, value: i32| {
+                [
+                    Term::NamedNode(NamedNode::new(s).unwrap()),
+                    Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+                    Term::Literal(Literal::from(value)),
+                ]
+            };
+            let d = g.dict.len();
+            assert!(!g.has_high_precision_decimal());
+            let removed = triple("urn:other", 8);
+            g.apply_delta(&[], std::slice::from_ref(&removed)).unwrap();
+            assert_eq!(g.len(), 2);
+            assert_eq!(
+                g.high_precision_decimal.load(Relaxed),
+                1,
+                "delete, sparse={sparse}"
+            );
+            let absent = triple("urn:s", 8);
+            assert!(absent.iter().all(|term| g.id_of(term).is_some()));
+            g.apply_delta(&[], &[absent]).unwrap();
+            assert_eq!(g.len(), 2);
+            assert_eq!(g.high_precision_decimal.load(Relaxed), 1, "absent delete");
+            g.apply_delta(&[removed], &[]).unwrap();
+            assert_eq!(g.len(), 3);
+            assert_eq!(
+                g.high_precision_decimal.load(Relaxed),
+                1,
+                "known reinsertion"
+            );
+            g.apply_delta(&[], &[]).unwrap();
+            assert_eq!(g.high_precision_decimal.load(Relaxed), 1, "empty delta");
+            let inline = triple("urn:s", 999999);
+            g.apply_delta(std::slice::from_ref(&inline), &[]).unwrap();
+            assert_eq!(g.id_of(&inline[2]), Some(dict::INLINE_BASE + 999999));
+            assert_eq!(g.dict.len(), d);
+            assert_eq!(g.high_precision_decimal.load(Relaxed), 1, "inline integer");
+            assert!(!g.has_high_precision_decimal());
+        }
+    }
+
+    #[test]
+    fn has_high_precision_decimal_memo_invalidates_stored_growth() {
+        use std::sync::atomic::Ordering::Relaxed;
+        for sparse in [false, true] {
+            let mut g = Graph::load_str("@prefix : <urn:> . :s :p 7 .", "turtle").unwrap();
+            if sparse {
+                g.numerics = NumData::Sparse(rustc_hash::FxHashMap::default());
+            }
+            assert!(!g.has_high_precision_decimal());
+            for (lexical, datatype, high_precision) in [
+                ("1.5", xsd::DECIMAL, false),
+                ("007", xsd::INTEGER, false),
+                ("2.000000000000000003", xsd::DECIMAL, true),
+            ] {
+                let d = g.dict.len();
+                let triple = [
+                    Term::NamedNode(NamedNode::new("urn:s").unwrap()),
+                    Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+                    Term::Literal(Literal::new_typed_literal(lexical, datatype)),
+                ];
+                g.apply_delta(std::slice::from_ref(&triple), &[]).unwrap();
+                assert!(g.dict.len() > d, "{lexical}");
+                let id = g.id_of(&triple[2]).unwrap();
+                assert!(g.numeric_value(id).is_some(), "{lexical}");
+                assert_eq!(g.dict.term(id), triple[2]);
+                assert_eq!(
+                    g.high_precision_decimal.load(Relaxed),
+                    0,
+                    "growth: {lexical}"
+                );
+                assert_eq!(g.has_high_precision_decimal(), high_precision);
+                if high_precision {
+                    g.apply_delta(&[], &[triple]).unwrap();
+                    assert_eq!(g.high_precision_decimal.load(Relaxed), 2, "sticky found");
+                    assert!(g.has_high_precision_decimal());
+                }
+            }
+        }
+    }
+
+    #[test]
+    fn has_high_precision_decimal_memo_fork_compact_isolation() {
+        use std::sync::atomic::Ordering::Relaxed;
+        let mut parent =
+            Graph::load_str("@prefix : <urn:> . :s :p 1.5 . :other :p 7 .", "turtle").unwrap();
+        assert!(!parent.has_high_precision_decimal());
+        let mut child = parent.fork();
+        assert!(matches!(&child.numerics, NumData::Forked { .. }));
+        assert_eq!(child.high_precision_decimal.load(Relaxed), 0);
+        assert!(!child.has_high_precision_decimal());
+        let d = child.dict.len();
+        let removed = [
+            Term::NamedNode(NamedNode::new("urn:other").unwrap()),
+            Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+            Term::Literal(Literal::from(7)),
+        ];
+        child
+            .apply_delta(&[], std::slice::from_ref(&removed))
+            .unwrap();
+        assert_eq!(child.high_precision_decimal.load(Relaxed), 1);
+        assert_eq!(parent.len(), 2);
+        assert_eq!(child.len(), 1);
+        child.compact().unwrap();
+        assert_eq!(child.dict.len(), d);
+        assert_eq!(child.high_precision_decimal.load(Relaxed), 1);
+        assert!(!child.has_high_precision_decimal());
+        child.apply_delta(&[removed], &[]).unwrap();
+        assert_eq!(child.high_precision_decimal.load(Relaxed), 1);
+        let snapshot = child.snapshot();
+        assert!(!snapshot.has_high_precision_decimal());
+        let inexact = [
+            Term::NamedNode(NamedNode::new("urn:s").unwrap()),
+            Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+            Term::Literal(Literal::new_typed_literal(
+                "2.000000000000000003",
+                xsd::DECIMAL,
+            )),
+        ];
+        parent
+            .apply_delta(std::slice::from_ref(&inexact), &[])
+            .unwrap();
+        assert!(parent.has_high_precision_decimal());
+        assert!(child.id_of(&inexact[2]).is_none());
+        assert!(!child.has_high_precision_decimal());
+        child
+            .apply_delta(std::slice::from_ref(&inexact), &[])
+            .unwrap();
+        assert_eq!(child.high_precision_decimal.load(Relaxed), 0);
+        assert!(child.has_high_precision_decimal());
+        assert!(snapshot.id_of(&inexact[2]).is_none());
+        assert!(!snapshot.has_high_precision_decimal());
+    }
+
+    #[cfg(feature = "mmap")]
+    #[test]
+    fn has_high_precision_decimal_memo_mapped_wal_precision() {
+        use std::sync::atomic::Ordering::Relaxed;
+        let dir = std::env::temp_dir().join(format!("sparq_numeric_memo_{}", std::process::id()));
+        Graph::load_str("@prefix : <urn:> . :s :p 1.5 . :other :p 7 .", "turtle")
+            .unwrap()
+            .save(&dir)
+            .unwrap();
+        let mut g = Graph::open(&dir).unwrap();
+        assert!(matches!(&g.numerics, NumData::Mapped(..)));
+        assert!(!g.has_high_precision_decimal());
+        let d = g.dict.len();
+        let removed = [
+            Term::NamedNode(NamedNode::new("urn:other").unwrap()),
+            Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+            Term::Literal(Literal::from(7)),
+        ];
+        g.apply_delta(&[], &[removed]).unwrap();
+        assert_eq!(g.dict.len(), d);
+        assert_eq!(g.high_precision_decimal.load(Relaxed), 1);
+        let inexact = [
+            Term::NamedNode(NamedNode::new("urn:s").unwrap()),
+            Term::NamedNode(NamedNode::new("urn:p").unwrap()),
+            Term::Literal(Literal::new_typed_literal(
+                "2.000000000000000003",
+                xsd::DECIMAL,
+            )),
+        ];
+        g.apply_delta(std::slice::from_ref(&inexact), &[]).unwrap();
+        assert_eq!(g.high_precision_decimal.load(Relaxed), 0);
+        assert!(g.numeric_value(g.id_of(&inexact[2]).unwrap()).is_some());
+        drop(g); // Reopen must obtain the new term and numeric cache through WAL replay.
+        let mut reopened = Graph::open(&dir).unwrap();
+        assert_eq!(reopened.high_precision_decimal.load(Relaxed), 0);
+        assert!(reopened.has_high_precision_decimal());
+        reopened.apply_delta(&[], &[inexact]).unwrap();
+        assert_eq!(reopened.high_precision_decimal.load(Relaxed), 2);
+        reopened.compact().unwrap(); // The dictionary retains the now-orphaned decimal.
+        assert!(reopened.has_high_precision_decimal());
+        drop(reopened);
+        let final_graph = Graph::open(&dir).unwrap();
+        assert!(final_graph.has_high_precision_decimal());
+        drop(final_graph);
+        std::fs::remove_dir_all(&dir).unwrap();
+    }
+
     /// The full sorted term-triple set of a graph (overlay merged), for state comparison.
     fn dump_terms(g: &Graph) -> Vec<(String, String, String)> {
         let scan = g.store.scan(&[None, None, None]);

```

## F4 lookup proof (unchanged source)

```text

--- crates/sparq-core/src/lib.rs:2771-2782 ---
2771:     /// Resolves a term to its id, or `None` if the term is absent (so a pattern
2772:     /// bound to it cannot match).
2773:     pub fn id_of(&self, term: &Term) -> Option<Id> {
2774:         let id = self.dict.lookup(term);
2775:         if id == dict::NO_ID {
2776:             None
2777:         } else {
2778:             Some(id)
2779:         }
2780:     }
2781: 
2782:     /// [FABLE-5] (sq-1ivw7) Whether ANY triple with predicate `predicate` in the CURRENT store

--- crates/sparq-core/src/dict.rs:27-87 ---
27: /// from the id (QLever's value-id idea, kept in `u32` so the index stays compact).
28: /// Inline integers also sort by value in the permutations (enabling range pruning).
29: ///
30: /// The `u32` id space is partitioned: dictionary ids `[1, INLINE_BASE)` (≈2.1 billion
31: /// distinct terms — enough for e.g. full-Wikidata's term count without widening to `u64`,
32: /// which would double the index), inline integers `[INLINE_BASE, INLINE_BASE + 2^30)`, and
33: /// the engine's local-vocab ids `[INLINE_BASE + 2^30, 2^32)`. `0` is `NO_ID`.
34: pub const INLINE_BASE: Id = 1 << 31;
35: /// The largest value encodable inline (the inline range stays 2^30 wide; bigger integers
36: /// fall back to the dictionary).
37: const INLINE_MAX: u32 = (1 << 30) - 1;
38: 
39: /// [OPUS-4.8] (review 1409) On-disk format marker for the mmap dictionary (`dict-meta.bin`).
40: /// `INLINE_BASE` partitions the `u32` id space, so persisted RAW ids only mean what they say
41: /// under the SAME partition the file was written with. A file written before `INLINE_BASE`
42: /// moved (from `1 << 30` to `1 << 31`) encodes inline integers in `[1<<30, 1<<31)`, which the
43: /// current code would misread as dictionary ids — silent numeric corruption / panics. The
44: /// header records the partition so `open_mmap` can REJECT a mismatched/legacy store with a
45: /// clear rebuild-required error instead of silently misinterpreting its ids.
46: ///
47: /// `dict-meta.bin` previously began with `prefixes.len() as u32` (a small count). This magic
48: /// is chosen to be distinguishable from any plausible legacy prefix count, so the reader can
49: /// detect header-less legacy files. ("DMV1" — Dict Meta, V1; little-endian.)
50: // clippy/dead_code: the on-disk meta header is read/written only by the `mmap`/`dict-spill`
51: // persistence paths, which are cfg'd out of the default feature set.
52: #[allow(dead_code)]
53: pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
54: /// Bump when the on-disk meta layout changes incompatibly.
55: #[allow(dead_code)]
56: pub(crate) const DICT_META_VERSION: u32 = 1;
57: 
58: /// If a literal `value`/`datatype` is a canonical non-negative `xsd:integer` in
59: /// range, its inline id. Only the canonical lexical form (no leading zeros / sign)
60: /// inlines, so `"030"^^integer` stays a distinct dictionary term.
61: #[inline]
62: fn try_inline_lit(value: &str, datatype: &str) -> Option<Id> {
63:     if datatype == xsd::INTEGER.as_str() {
64:         if let Ok(v) = value.parse::<u32>() {
65:             if v <= INLINE_MAX && v.to_string() == value {
66:                 return Some(INLINE_BASE + v);
67:             }
68:         }
69:     }
70:     None
71: }
72: 
73: /// If `term` is a canonical non-negative `xsd:integer` in range, its inline id.
74: fn try_inline(term: &Term) -> Option<Id> {
75:     match term {
76:         Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
77:         _ => None,
78:     }
79: }
80: 
81: /// Whether an id encodes an inline integer value. (`INLINE_BASE << 1` would overflow `u32`,
82: /// so the upper bound is expressed via the inline width.)
83: #[inline]
84: pub fn is_inline(id: Id) -> bool {
85:     id >= INLINE_BASE && id - INLINE_BASE <= INLINE_MAX
86: }
87: 

--- crates/sparq-core/src/dict.rs:185-248 ---
185:     Triple([Id; 3]),
186: }
187: 
188: // ---- Content hashing ---------------------------------------------------------
189: // A term's hash must be identical whether computed from an `oxrdf::Term`, from parsed
190: // byte slices, or from the compact `Stored` arena form — so interning and rehashing
191: // never build a `Term` or concatenate an IRI. FxHasher is deterministic (no seed).
192: 
193: #[inline]
194: fn hash_iri_parts(prefix: &str, suffix: &str) -> u64 {
195:     // Length-prefix the prefix and make hash_iri route through here on the SAME split,
196:     // so the two paths issue an identical sequence of writes — FxHasher is word-chunked,
197:     // so `write(a); write(b)` does NOT equal `write(a++b)`; identical call sequences do.
198:     let mut h = rustc_hash::FxHasher::default();
199:     h.write_u8(0);
200:     h.write_usize(prefix.len());
201:     h.write(prefix.as_bytes());
202:     h.write(suffix.as_bytes());
203:     h.finish()
204: }
205: 
206: #[inline]
207: fn hash_iri(iri: &str) -> u64 {
208:     let (prefix, suffix) = split_iri(iri);
209:     hash_iri_parts(prefix, suffix)
210: }
211: 
212: #[inline]
213: fn hash_lit(value: &str, datatype: &str, lang: Option<&str>) -> u64 {
214:     let mut h = rustc_hash::FxHasher::default();
215:     h.write_u8(1);
216:     h.write_usize(value.len());
217:     h.write(value.as_bytes());
218:     h.write_usize(datatype.len());
219:     h.write(datatype.as_bytes());
220:     match lang {
221:         Some(l) => {
222:             h.write_u8(1);
223:             h.write(l.as_bytes());
224:         }
225:         None => h.write_u8(0),
226:     }
227:     h.finish()
228: }
229: 
230: #[inline]
231: fn hash_blank(label: &str) -> u64 {
232:     let mut h = rustc_hash::FxHasher::default();
233:     h.write_u8(2);
234:     h.write(label.as_bytes());
235:     h.finish()
236: }
237: 
238: /// Content hash of a structurally stored triple term — over the (already-interned) ids of
239: /// its components, NOT strings. Unlike the other term kinds this hash is dict-relative
240: /// (the ids are), so a `Term::Triple` is hashed only AFTER its children are resolved in
241: /// the target dict (`intern`/`lookup` handle this; `hash_term` cannot).
242: /// [OPUS-4.8] (sq-jvbr) `pub(crate)` so the dict-spill external builder can content-address
243: /// an RDF 1.2 triple term by its components' FINAL dense ids when it assembles the
244: /// triple-term dictionary records (the same `(hash, id)` pair `save_mmap`/`into_merged`
245: /// produce), keeping `dict-hash.bin`/`dict-hid.bin` consistent for triple terms too.
246: #[inline]
247: pub(crate) fn hash_triple_ids(ids: [Id; 3]) -> u64 {
248:     let mut h = rustc_hash::FxHasher::default();

--- crates/sparq-core/src/dict.rs:333-341 ---
333: fn hash_term(t: &Term) -> u64 {
334:     match t {
335:         Term::NamedNode(n) => hash_iri(n.as_str()),
336:         Term::Literal(l) => hash_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
337:         Term::BlankNode(b) => hash_blank(b.as_str()),
338:         _ => 0,
339:     }
340: }
341: 

--- crates/sparq-core/src/dict.rs:377-417 ---
377: fn stored_is_iri(s: &Stored, iri: &str, prefixes: &[Box<str>]) -> bool {
378:     match s {
379:         Stored::Iri { prefix, suffix } => {
380:             let p = &prefixes[*prefix as usize];
381:             iri.len() == p.len() + suffix.len() && iri.as_bytes().starts_with(p.as_bytes()) && iri[p.len()..] == **suffix
382:         }
383:         _ => false,
384:     }
385: }
386: 
387: #[inline]
388: fn stored_is_iri_parts(s: &Stored, prefix: &str, suffix: &str, prefixes: &[Box<str>]) -> bool {
389:     match s {
390:         Stored::Iri { prefix: pid, suffix: suf } => prefixes[*pid as usize].as_ref() == prefix && **suf == *suffix,
391:         _ => false,
392:     }
393: }
394: 
395: #[inline]
396: fn stored_is_lit(s: &Stored, value: &str, datatype: &str, lang: Option<&str>, datatypes: &[NamedNode]) -> bool {
397:     match s {
398:         Stored::Lit { value: v, datatype: dt, lang: lg } => {
399:             **v == *value && lg.as_deref() == lang && datatypes[*dt as usize].as_str() == datatype
400:         }
401:         _ => false,
402:     }
403: }
404: 
405: /// NOTE: `Term::Triple` always misses here — triple terms are matched by their component
406: /// IDS (see `Dict::lookup`, which resolves the children and intercepts before this path).
407: #[inline]
408: fn stored_eq_term(s: &Stored, q: &Term, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> bool {
409:     match q {
410:         Term::NamedNode(n) => stored_is_iri(s, n.as_str(), prefixes),
411:         Term::Literal(l) => stored_is_lit(s, l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref(), datatypes),
412:         Term::BlankNode(b) => matches!(s, Stored::Blank(x) if **x == *b.as_str()),
413:         _ => false,
414:     }
415: }
416: 
417: /// [OPUS-4.8] sq-cvug — the safe placeholders `reconstruct_triple` substitutes when a

--- crates/sparq-core/src/dict.rs:644-664 ---
644: 
645: /// NOTE: like `stored_eq_term`, `Term::Triple` always misses here — `Dict::lookup`
646: /// resolves a triple term's component ids first and matches `StoredRef::Triple` by id.
647: fn stored_ref_eq_term(s: &StoredRef, q: &Term, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> bool {
648:     match (s, q) {
649:         (StoredRef::Iri { prefix, suffix }, Term::NamedNode(n)) => {
650:             let p = &prefixes[*prefix as usize];
651:             let iri = n.as_str();
652:             iri.len() == p.len() + suffix.len() && iri.starts_with(p.as_ref()) && iri[p.len()..] == **suffix
653:         }
654:         (StoredRef::Lit { value, datatype, lang }, Term::Literal(l)) => {
655:             *value == l.value()
656:                 && lang.as_deref() == lang_with_dir(l).as_deref()
657:                 && datatypes[*datatype as usize].as_str() == l.datatype().as_str()
658:         }
659:         (StoredRef::Blank(x), Term::BlankNode(b)) => *x == b.as_str(),
660:         _ => false,
661:     }
662: }
663: 
664: /// A zero-copy `StoredRef` view of an arena `Stored` — so the appended-term (arena-over-

--- crates/sparq-core/src/dict.rs:744-788 ---
744:     blob: memmap2::Mmap,    // concatenated term records (the format `save_mmap` writes)
745:     offsets: memmap2::Mmap, // [u64; n] byte offset of each term record in `blob`
746:     hashes: memmap2::Mmap,  // [u64; n] content hashes, SORTED (for lookup)
747:     hashids: memmap2::Mmap, // [u32; n] term ids parallel to `hashes`
748: }
749: 
750: #[cfg(feature = "mmap")]
751: impl MappedDict {
752:     #[inline]
753:     fn slice_u64(m: &memmap2::Mmap) -> &[u64] {
754:         // SAFETY: `write_pod_slice` wrote these as raw NATIVE-endian u64 (little-endian on
755:         // every supported target — pinned by `DICT_ON_DISK_LE_GUARD`, sq-lvw8); mmap base is
756:         // page-aligned (>= 8). This pointer cast reads them back in the same native order.
757:         unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<u64>(), m.len() / 8) }
758:     }
759:     #[inline]
760:     fn offsets(&self) -> &[u64] {
761:         Self::slice_u64(&self.offsets)
762:     }
763:     #[inline]
764:     fn hashes(&self) -> &[u64] {
765:         Self::slice_u64(&self.hashes)
766:     }
767:     #[inline]
768:     fn hashids(&self) -> &[u32] {
769:         // SAFETY: `write_pod_slice` wrote these as raw NATIVE-endian u32 (little-endian on
770:         // every supported target — pinned by `DICT_ON_DISK_LE_GUARD`, sq-lvw8); mmap base is
771:         // page-aligned (>= 4). This pointer cast reads them back in the same native order.
772:         unsafe { std::slice::from_raw_parts(self.hashids.as_ptr().cast::<u32>(), self.hashids.len() / 4) }
773:     }
774:     /// The parsed term record for a 1-based id.
775:     ///
776:     /// [OPUS-4.8] sq-znld: this reads from an UNTRUSTED memory-mapped blob, so it uses the
777:     /// bounds-/UTF-8-CHECKED parser. `Dict::open_mmap` runs [`validate`](Self::validate)
778:     /// up front, which proves every offset is in range and every record parses, so by the
779:     /// time `stored` is reached the `expect` is structurally impossible — but the checked
780:     /// parse remains as a sound floor (no `from_utf8_unchecked` over attacker bytes, ever),
781:     /// and the cost is negligible against the mmap page-fault that produced the bytes.
782:     #[inline]
783:     fn stored(&self, id: Id) -> StoredRef<'_> {
784:         let off = self.offsets()[(id - 1) as usize] as usize;
785:         parse_stored_ref_checked(self.blob.get(off..).unwrap_or_default())
786:             .expect("mmap dict record validated at open (sq-znld); a failure here means the mapped file changed under us")
787:     }
788: 

--- crates/sparq-core/src/dict.rs:1260-1349 ---
1260: 
1261:     /// Finds a term in the MAPPED base by content hash (binary search of the mmap'd
1262:     /// sorted-hash index, verifying candidates with `eq`). `None` when not mapped or
1263:     /// absent — the caller then consults the table (blob base + appended arena terms).
1264:     #[cfg(feature = "mmap")]
1265:     #[inline]
1266:     fn mapped_find(&self, hash: u64, eq: impl Fn(&StoredRef) -> bool) -> Option<Id> {
1267:         let m = self.mapped.as_ref()?;
1268:         let hashes = m.hashes();
1269:         let mut i = hashes.partition_point(|&h| h < hash);
1270:         let ids = m.hashids();
1271:         while i < hashes.len() && hashes[i] == hash {
1272:             let id = ids[i];
1273:             if eq(&m.stored(id)) {
1274:                 return Some(id);
1275:             }
1276:             i += 1;
1277:         }
1278:         None
1279:     }
1280: 
1281:     /// The record of a TABLED id (blob base, or appended arena) as needed by the intern
1282:     /// comparison closures. Pure-arena dicts (`base == 0`) always take the arena branch,
1283:     /// so the bulk-load hot path is unchanged beyond one predictable comparison.
1284:     #[inline]
1285:     fn tabled_base_ref(&self, id: Id) -> StoredRef<'_> {
1286:         let (blob, offs) = self.blob.as_ref().expect("a tabled id below `base` requires the blob");
1287:         parse_stored_ref(&blob[offs[(id - 1) as usize] as usize..])
1288:     }
1289: 
1290:     #[inline]
1291:     fn tabled_is_iri(&self, id: Id, iri: &str) -> bool {
1292:         let i = (id - 1) as usize;
1293:         if i >= self.base {
1294:             stored_is_iri(&self.terms[i - self.base], iri, &self.prefixes)
1295:         } else {
1296:             stored_ref_is_iri(&self.tabled_base_ref(id), iri, &self.prefixes)
1297:         }
1298:     }
1299: 
1300:     #[inline]
1301:     fn tabled_is_iri_parts(&self, id: Id, prefix: &str, suffix: &str) -> bool {
1302:         let i = (id - 1) as usize;
1303:         if i >= self.base {
1304:             stored_is_iri_parts(&self.terms[i - self.base], prefix, suffix, &self.prefixes)
1305:         } else {
1306:             stored_ref_is_iri_parts(&self.tabled_base_ref(id), prefix, suffix, &self.prefixes)
1307:         }
1308:     }
1309: 
1310:     #[inline]
1311:     fn tabled_is_lit(&self, id: Id, value: &str, datatype: &str, lang: Option<&str>) -> bool {
1312:         let i = (id - 1) as usize;
1313:         if i >= self.base {
1314:             stored_is_lit(&self.terms[i - self.base], value, datatype, lang, &self.datatypes)
1315:         } else {
1316:             stored_ref_is_lit(&self.tabled_base_ref(id), value, datatype, lang, &self.datatypes)
1317:         }
1318:     }
1319: 
1320:     #[inline]
1321:     fn tabled_is_blank(&self, id: Id, label: &str) -> bool {
1322:         let i = (id - 1) as usize;
1323:         if i >= self.base {
1324:             matches!(&self.terms[i - self.base], Stored::Blank(b) if **b == *label)
1325:         } else {
1326:             matches!(self.tabled_base_ref(id), StoredRef::Blank(b) if b == label)
1327:         }
1328:     }
1329: 
1330:     #[inline]
1331:     fn tabled_is_triple(&self, id: Id, ids: [Id; 3]) -> bool {
1332:         let i = (id - 1) as usize;
1333:         if i >= self.base {
1334:             matches!(&self.terms[i - self.base], Stored::Triple(x) if *x == ids)
1335:         } else {
1336:             matches!(self.tabled_base_ref(id), StoredRef::Triple(x) if x == ids)
1337:         }
1338:     }
1339: 
1340:     #[inline]
1341:     fn tabled_eq_term(&self, id: Id, term: &Term) -> bool {
1342:         let i = (id - 1) as usize;
1343:         if i >= self.base {
1344:             stored_eq_term(&self.terms[i - self.base], term, &self.prefixes, &self.datatypes)
1345:         } else {
1346:             stored_ref_eq_term(&self.tabled_base_ref(id), term, &self.prefixes, &self.datatypes)
1347:         }
1348:     }
1349: 

--- crates/sparq-core/src/dict.rs:1380-1462 ---
1380:     #[inline]
1381:     fn find_iri(&self, hash: u64, iri: &str) -> Option<Id> {
1382:         #[cfg(feature = "mmap")]
1383:         if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_iri(s, iri, &self.prefixes)) {
1384:             return Some(id);
1385:         }
1386:         if let Some(f) = &self.frozen {
1387:             if let Some(id) = f.find_iri(hash, iri) {
1388:                 return Some(id);
1389:             }
1390:         }
1391:         self.table.find(hash, |&id| self.tabled_is_iri(id, iri)).copied()
1392:     }
1393: 
1394:     #[inline]
1395:     fn find_iri_parts(&self, hash: u64, prefix: &str, suffix: &str) -> Option<Id> {
1396:         #[cfg(feature = "mmap")]
1397:         if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_iri_parts(s, prefix, suffix, &self.prefixes)) {
1398:             return Some(id);
1399:         }
1400:         if let Some(f) = &self.frozen {
1401:             if let Some(id) = f.find_iri_parts(hash, prefix, suffix) {
1402:                 return Some(id);
1403:             }
1404:         }
1405:         self.table.find(hash, |&id| self.tabled_is_iri_parts(id, prefix, suffix)).copied()
1406:     }
1407: 
1408:     #[inline]
1409:     fn find_lit(&self, hash: u64, value: &str, datatype: &str, lang: Option<&str>) -> Option<Id> {
1410:         #[cfg(feature = "mmap")]
1411:         if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_lit(s, value, datatype, lang, &self.datatypes)) {
1412:             return Some(id);
1413:         }
1414:         if let Some(f) = &self.frozen {
1415:             if let Some(id) = f.find_lit(hash, value, datatype, lang) {
1416:                 return Some(id);
1417:             }
1418:         }
1419:         self.table.find(hash, |&id| self.tabled_is_lit(id, value, datatype, lang)).copied()
1420:     }
1421: 
1422:     #[inline]
1423:     fn find_blank(&self, hash: u64, label: &str) -> Option<Id> {
1424:         #[cfg(feature = "mmap")]
1425:         if let Some(id) = self.mapped_find(hash, |s| matches!(s, StoredRef::Blank(b) if *b == label)) {
1426:             return Some(id);
1427:         }
1428:         if let Some(f) = &self.frozen {
1429:             if let Some(id) = f.find_blank(hash, label) {
1430:                 return Some(id);
1431:             }
1432:         }
1433:         self.table.find(hash, |&id| self.tabled_is_blank(id, label)).copied()
1434:     }
1435: 
1436:     #[inline]
1437:     fn find_triple_ids(&self, hash: u64, ids: [Id; 3]) -> Option<Id> {
1438:         #[cfg(feature = "mmap")]
1439:         if let Some(id) = self.mapped_find(hash, |s| matches!(s, StoredRef::Triple(x) if *x == ids)) {
1440:             return Some(id);
1441:         }
1442:         if let Some(f) = &self.frozen {
1443:             if let Some(id) = f.find_triple_ids(hash, ids) {
1444:                 return Some(id);
1445:             }
1446:         }
1447:         self.table.find(hash, |&id| self.tabled_is_triple(id, ids)).copied()
1448:     }
1449: 
1450:     #[inline]
1451:     fn find_term(&self, hash: u64, term: &Term) -> Option<Id> {
1452:         #[cfg(feature = "mmap")]
1453:         if let Some(id) = self.mapped_find(hash, |s| stored_ref_eq_term(s, term, &self.prefixes, &self.datatypes)) {
1454:             return Some(id);
1455:         }
1456:         if let Some(f) = &self.frozen {
1457:             if let Some(id) = f.find_term(hash, term) {
1458:                 return Some(id);
1459:             }
1460:         }
1461:         self.table.find(hash, |&id| self.tabled_eq_term(id, term)).copied()
1462:     }

--- crates/sparq-core/src/dict.rs:1571-1620 ---
1571: 
1572:     /// Returns the id for a term if present, else `NO_ID`.
1573:     #[inline]
1574:     pub fn lookup(&self, term: &Term) -> Id {
1575:         // An RDF 1.2 triple term is matched STRUCTURALLY: resolve its components first
1576:         // (a missing component means the triple term cannot be present either), then
1577:         // find the triple by its component ids.
1578:         if let Term::Triple(t) = term {
1579:             let s = match t.subject {
1580:                 oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.lookup(&Term::NamedNode(n.clone())),
1581:                 oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.lookup(&Term::BlankNode(b.clone())),
1582:             };
1583:             let p = self.lookup(&Term::NamedNode(t.predicate.clone()));
1584:             let o = self.lookup(&t.object);
1585:             if s == NO_ID || p == NO_ID || o == NO_ID {
1586:                 return NO_ID;
1587:             }
1588:             return self.lookup_triple_ids([s, p, o]);
1589:         }
1590:         if let Some(id) = try_inline(term) {
1591:             return id;
1592:         }
1593:         let hash = hash_term(term);
1594:         // Mapped base first (binary search of the sorted hash index, verifying each
1595:         // equal-hash candidate — content hashes can collide), then the frozen shared
1596:         // base of a forked dict, then the table, which covers the blob base and any
1597:         // APPENDED terms (delta-overlay growth).
1598:         self.find_term(hash, term).unwrap_or(NO_ID)
1599:     }
1600: 
1601:     /// Returns the id for a literal given its components, else `NO_ID` — `lookup`
1602:     /// without constructing an `oxrdf::Term` (the fast path for resolving computed
1603:     /// BIND/aggregate values against the dictionary). Canonical small `xsd:integer`s
1604:     /// resolve to their inline id, exactly like `lookup`/`intern_lit`.
1605:     #[inline]
1606:     pub fn lookup_lit(&self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
1607:         if let Some(id) = try_inline_lit(value, datatype) {
1608:             return id;
1609:         }
1610:         let hash = hash_lit(value, datatype, lang);
1611:         self.find_lit(hash, value, datatype, lang).unwrap_or(NO_ID)
1612:     }
1613: 
1614:     /// Returns the id of a triple term whose components resolved to `ids`, else `NO_ID`.
1615:     /// Serves all three storage modes (arena, blob, mmap).
1616:     fn lookup_triple_ids(&self, ids: [Id; 3]) -> Id {
1617:         let hash = hash_triple_ids(ids);
1618:         self.find_triple_ids(hash, ids).unwrap_or(NO_ID)
1619:     }
1620: 

```

```rust
/// per-term slot (`Stored`) is far smaller than a full `oxrdf::Term`.
/// `Clone` exists for [`fork`](Dict::fork): on an already-forked dict it bumps the
/// shared-base `Arc`s and copies only the small extension; on a flat dict it is the
/// O(n) deep copy that `fork` freezes ONCE into the shared base. It is not intended
/// as a general-purpose copy (a `Graph` remains deliberately non-`Clone`).
#[derive(Default, Clone)]
pub struct Dict {
    prefixes: Vec<Box<str>>,                  // id -> IRI namespace prefix
    prefix_ids: FxHashMap<Box<str>, u32>,     // prefix -> id
    datatypes: Vec<NamedNode>,                // id -> literal datatype (a small set)
    datatype_ids: FxHashMap<Box<str>, u32>,   // datatype IRI -> id
    terms: Vec<Stored>,                       // id-1 -> compact term (empty once compacted)
    table: HashTable<Id>,                     // hash(term) -> id (bare ids, compared via the arena)
    // When `Some` (after `into_blob`), id->term is served from a single concatenated term
    // BLOB + per-term offsets instead of `Vec<Stored>` — no per-term `Box<str>` allocation
    // overhead, ~half the resident dict bytes. `table` is kept (lookup verifies via the
    // blob); `terms` is empty. The memory-bound (browser) storage mode for the dictionary.
    blob: Option<(Vec<u8>, Vec<u32>)>,
    // When `Some` (after `open_mmap`), term(id)/term_parts/lookup are served from mmap'd
    // files and `terms`/`table` are empty — the out-of-core, minimal-RAM dictionary.
    // Behind an `Arc` so a fork shares the (immutable) mapping instead of re-mmapping.
    #[cfg(feature = "mmap")]
    mapped: Option<std::sync::Arc<MappedDict>>,
    // APPEND-ONLY growth over the compacted storage modes (delta-overlay updates, T17):
    // ids `1..=base` are served by the blob / mmap'd record store; freshly interned terms
    // go to the `terms` arena with ids `base + i + 1`. Always 0 in the plain arena mode,
    // so the arena hot paths are unchanged (one predictable comparison).
    base: usize,
    // The SHARED IMMUTABLE BASE of a forked dictionary ([`fork`](Dict::fork)): ids
    // `1..=base` resolve through this complete, never-mutated dict (Arc-shared across
    // snapshot generations); freshly interned terms append to the local `terms` arena
    // above `base`, exactly like the blob/mmap growth modes. `None` for every
    // non-forked dict, so the bulk-load and flat-read hot paths pay only a predictable
    // never-taken branch. INVARIANT: a frozen base is itself flat (`frozen: None`),
    // so delegation never recurses more than one level; the fork's `prefixes` /
    // `datatypes` tables start as clones of the base's, keeping base record prefix /
    // datatype indices valid against the fork's tables.
    frozen: Option<std::sync::Arc<Dict>>,
}

/// The compact per-term storage. An IRI is `(prefix id, suffix)`; a literal is
/// `(value, datatype id, optional language)`; a blank node is its label. An RDF 1.2
/// TRIPLE TERM (`<<( s p o )>>`) is stored STRUCTURALLY as the ids of its three
/// components (which are interned first, so a child id is always lower than — or an
/// inline id distinct from — the triple's own id; nesting recurses through the object).
#[derive(Clone)]
enum Stored {
    Iri { prefix: u32, suffix: Box<str> },
    Lit { value: Box<str>, datatype: u32, lang: Option<Box<str>> },
    Blank(Box<str>),
    Triple([Id; 3]),
}

/// A borrowed view of a dictionary term's string components (no allocation), for
/// serialising results directly from ids. An IRI is its namespace prefix + local
```

## Cache and lifecycle context

```rust

--- Base unchanged crates/sparq-core/src/lib.rs:185-325 ---
/// `id`, NaN for non-numeric): owned dense in RAM, mmap'd from disk (out-of-core), or
/// SPARSE — only the numeric terms in a hash map. Most RDF terms are IRIs/strings (NaN),
/// and small integers inline (carrying their own value, never cached), so the dense
/// cache is mostly — often entirely — NaN; the sparse form stores only the few real
/// numeric literals, the right shape for the memory-bound browser store.
enum NumData {
    Owned(Vec<f64>),
    /// The mmap'd dense cache, plus a small side map for terms APPENDED after open
    /// (delta-overlay updates) — the mmap'd file cannot grow, the dictionary can.
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap, rustc_hash::FxHashMap<Id, f64>),
    Sparse(rustc_hash::FxHashMap<Id, f64>),
    /// A FORKED graph's cache: the base graph's cache SHARED immutably (Arc) plus a
    /// small side map for terms interned after the fork — the in-RAM twin of
    /// `Mapped`'s grow-over-immutable-base shape. Lookup: base first, side map as the
    /// fallback (the side map only ever holds ids the base does not cover).
    /// INVARIANT: the inner cache is never itself `Forked` (fork flattens one level).
    Forked { base: std::sync::Arc<NumData>, extra: rustc_hash::FxHashMap<Id, f64> },
}

impl NumData {
    /// The cached numeric value of a 1-based dictionary id, or `None` if it is not a
    /// (cached) numeric literal. The engine's O(1) numeric fast path.
    // [FABLE-5] Keep the dominant dense-cache probe small enough that LTO reliably places it
    // inside numeric gather loops; the larger storage-variant dispatch stays outlined.
    #[inline(always)]
    fn lookup(&self, id: Id) -> Option<f64> {
        if let NumData::Owned(values) = self {
            let value = *values.get((id - 1) as usize)?;
            return (!value.is_nan()).then_some(value);
        }
        self.lookup_non_owned(id)
    }

    #[inline(never)]
    fn lookup_non_owned(&self, id: Id) -> Option<f64> {
        match self {
            NumData::Sparse(m) => m.get(&id).copied(),
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => match self.as_slice().get((id - 1) as usize) {
                Some(v) if !v.is_nan() => Some(*v),
                Some(_) => None,
                // Beyond the mmap'd dense cache: a term appended after open.
                None => extra.get(&id).copied(),
            },
            NumData::Owned(_) => unreachable!("Owned handled by lookup"),
            NumData::Forked { base, extra } => {
                base.lookup(id).or_else(|| extra.get(&id).copied())
            }
        }
    }

    /// Appends cache entries for freshly interned dictionary ids `old_len+1 ..= dict.len()`
    /// (delta-overlay growth): the dense backing extends in place; the sparse / mmap'd
    /// backings record only the real numeric values in their side map.
    fn extend_for(&mut self, dict: &Dict, old_len: usize) {
        for i in old_len..dict.len() {
            let id = i as Id + 1;
            let v = numeric_of(&dict.term(id));
            match self {
                NumData::Owned(vec) => vec.push(v),
                NumData::Sparse(m) => {
                    if !v.is_nan() {
                        m.insert(id, v);
                    }
                }
                #[cfg(feature = "mmap")]
                NumData::Mapped(_, extra) => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
                NumData::Forked { extra, .. } => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
            }
        }
    }

    /// The cache for a structurally FORKED graph: the base cache shared immutably,
    /// new terms recorded in a per-fork side map. Forking a fork shares the same
    /// base Arc and copies the (small) side map; the FIRST fork of a flat cache
    /// pays a one-time O(n) copy to freeze the shareable base (the mmap'd backing
    /// is materialised — an `Mmap` cannot be cloned; forked graphs are in-memory).
    fn fork(&self) -> NumData {
        match self {
            NumData::Forked { base, extra } => NumData::Forked {
                base: std::sync::Arc::clone(base),
                extra: extra.clone(),
            },
            NumData::Owned(v) => NumData::Forked {
                base: std::sync::Arc::new(NumData::Owned(v.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            NumData::Sparse(m) => NumData::Forked {
                base: std::sync::Arc::new(NumData::Sparse(m.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => NumData::Forked {
                // Materialise the mmap'd dense part (an `Mmap` cannot be cloned);
                // ids the file does not cover stay in the carried side map.
                base: std::sync::Arc::new(NumData::Owned(self.as_slice().to_vec())),
                extra: extra.clone(),
            },
        }
    }

    /// Folds a forked cache's side map into a FRESH shared base (compaction): one
    /// dense/sparse backing covering the whole dictionary, kept in the SHAREABLE
    /// `Forked` shape (Arc base + empty side map) — so the side map stops growing
    /// AND the next fork is still an Arc bump, never an O(dict) re-freeze.
    /// Non-forked backings are returned as-is.
    fn fold(self, dict_len: usize) -> NumData {
        match self {
            NumData::Forked { base, extra } => {
                let folded = match &*base {
                    NumData::Owned(v) => {
                        let mut dense = v.clone();
                        dense.resize(dict_len, f64::NAN);
                        for (&id, &val) in &extra {
                            dense[(id - 1) as usize] = val;
                        }
                        NumData::Owned(dense)
                    }
                    NumData::Sparse(m) => {
                        let mut m = m.clone();
                        m.extend(extra);
                        NumData::Sparse(m)
                    }
                    // `fork` never freezes a Forked or Mapped base (invariant above).
                    _ => unreachable!("a forked numeric cache's base is Owned or Sparse"),
                };
                NumData::Forked { base: std::sync::Arc::new(folded), extra: rustc_hash::FxHashMap::default() }
            }
            other => other,
        }
    }


--- Base unchanged crates/sparq-core/src/lib.rs:790-825 ---

/// [OPUS-4.8] (sq-lr2ii) `true` iff dict id `id` is an `xsd:decimal` literal whose lexical
/// form carries MORE than 15 significant digits — i.e. its f64 image may be inexact, so the
/// engine's f64 sargable FILTER fast path is unsafe for it. Only `xsd:decimal` is checked:
/// integer exactness is handled by the engine's constant-side guard and float/double values
/// are their own f64. See [`Graph::has_high_precision_decimal`].
fn is_high_precision_decimal(dict: &Dict, id: Id) -> bool {
    matches!(dict.term_parts(id),
        dict::TermParts::Lit { value, datatype, lang: None }
            if datatype == xsd::DECIMAL.as_str() && decimal_significant_digits(value) > 15)
}

/// Significant decimal digits of a plain decimal lexical `[+-]?digits(.digits)?`: leading
/// integer zeros and leading fractional zeros (for a value `< 1`) are not significant, e.g.
/// `007.50` -> 3, `0.00123` -> 3, `1.000000000000000001` -> 19. `usize::MAX` for a lexical that
/// is not a plain decimal (treated as unsafe by the `> 15` guard). Kept LOCAL to sparq-core:
/// this crate is the leanest tier and cannot depend on `sparq-substrate`; the count mirrors the
/// engine's `sig_digits` (constant-side guard) so the two round-trip decisions stay consistent.
fn decimal_significant_digits(s: &str) -> usize {
    let s = s.trim();
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return usize::MAX;
    }
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    if int.is_empty() {
        frac.trim_start_matches('0').len()
    } else {
        int.len() + frac.len()
    }
}

/// True for `xsd:integer` and its derived integer subtypes (NOT decimal/double/float) —
/// the datatypes whose values are exact integers (parseable as `i128`).

--- Base unchanged crates/sparq-core/src/lib.rs:1380-1387 ---

    /// Builds a graph from an already-interned dictionary + triple set (e.g. after opt-in
    /// reasoning materialized additional triples). Public counterpart of the internal
    /// `build`.
    pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        Self::build(dict, triples)
    }


--- Base unchanged crates/sparq-core/src/lib.rs:1680-1703 ---
        let store = TripleStore::from_triples(triples);
        let numerics = NumData::Owned(numerics_of(&dict));
        // Unlike the numerics cache (dense f64 = 8 B/term), the dense temporal cells
        // are 16 B/term — go sparse straight away when temporals are rare (usually:
        // none at all -> an empty map, zero memory), keeping the load-time memory
        // metric flat for non-temporal datasets. Temporal-heavy data stays dense.
        let temporals = TempData::Owned(temporals_of(&dict)).into_sparse_if_worthwhile();
        Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: Vec::new(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) In-memory graph: no parent-level redo journal.
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }

    /// Like [`load_str`](Self::load_str) but stores the permutation indexes

--- Base unchanged crates/sparq-core/src/lib.rs:1910-1962 ---
            } else {
                // Neither format present — surface the original mmap error (NotFound on
                // dict-meta.bin), matching the previous behaviour for a corrupt directory.
                Dict::open_mmap(dir)?
            }
        };
        let np = dir.join("numerics.bin");
        let numerics = match std::fs::File::open(&np) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * std::mem::size_of::<f64>() => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                NumData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            _ => NumData::Owned(numerics_of(&dict)),
        };
        let tp = dir.join("temporals.bin");
        let temporals = match std::fs::File::open(&tp) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * 9 => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                TempData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            // Absent or stale (a graph saved before this cache existed): recompute —
            // backward compatible, like the numerics cache.
            _ => TempData::Owned(temporals_of(&dict)),
        };
        // [OPUS-4.8] (sq-3ui0, gh-45) Restore the named graphs (each opened memory-mapped
        // with its own per-graph WAL) so a save→open round-trip is lossless for the whole
        // dataset. Empty for a default-graph-only / pre-named-graph directory.
        let named = Self::open_named(dir)?;
        let mut g = Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named,
            graph_prefix_index: std::sync::Mutex::new(None),
            wal: None,
            txn: None,
        };
        // Replay any write-ahead log into the delta-overlay (recovery after a crash or a
        // plain not-yet-compacted close), stopping cleanly at the first torn record and
        // truncating the log there — then keep the log open for further appends.
        for (insert, t) in Wal::replay(dir)? {
            if insert {
                g.apply_delta_mem(&[t], &[]);
            } else {
                g.apply_delta_mem(&[], &[t]);
            }
        }
        g.wal = Some(Wal::open(dir)?);
        // [OPUS-4.8] (sq-ycle) AFTER the per-graph WALs are replayed (above + per named graph in
        // `open_named`), redo any committed parent-level transaction frame, idempotently. A
        // multi-op UPDATE body's resolved per-slot quad-delta is one atomic `txn.log` frame; if a

--- Base unchanged crates/sparq-core/src/lib.rs:2883-2902 ---
    /// generation pattern is an in-memory serving construct — durability stays with
    /// whatever owns the base.
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            // sq-lr2ii: the fork shares the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            // A fork is a fresh logical copy; rebuild the prefix index lazily on first use.
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) A fork/snapshot is a logically-independent in-memory copy with
            // NO directory association — no WAL and no redo journal (like `wal: None`).
            #[cfg(feature = "mmap")]
            txn: None,
        }

--- Base unchanged crates/sparq-core/src/lib.rs:2936-2959 ---

    // ---- Incremental updates (T17): delta-overlay + WAL durability ------------------

    /// Applies an incremental update batch — `deletes` first, then `inserts` (SPARQL's
    /// DELETE/INSERT application order) — through the store's DELTA-OVERLAY: O(batch)
    /// work instead of the O(n) full rebuild. New terms are interned APPEND-ONLY (the
    /// dictionary grows; existing ids never change), so readers of existing ids are
    /// unaffected. For a directory-backed graph (opened via [`open`](Self::open)) the
    /// batch is appended to the write-ahead log and fsync'd BEFORE it is applied, so a
    /// crash replays it on the next open. Fold the overlay back into the immutable base
    /// periodically with [`compact`](Self::compact).
    pub fn apply_delta(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> Result<(), String> {
        if inserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(w) = &mut self.wal {
            w.append_batch(inserts, deletes).map_err(|e| format!("WAL append failed: {e}"))?;
        }
        self.apply_delta_mem(inserts, deletes);
        Ok(())
    }

    /// [OPUS-4.8] (gh-1122) Insert a SINGLE triple from `oxrdf` terms — the ergonomic

--- Base unchanged crates/sparq-core/src/lib.rs:3400-3445 ---
    /// that keeps scans overlay-free). The dictionary is kept as-is — ids are stable;
    /// terms only referenced by deleted triples linger until a full reload (cheap, and
    /// it keeps compaction O(triples) with no re-interning). For a directory-backed
    /// graph the new base is persisted ATOMICALLY (written to a fresh sibling directory,
    /// then swapped in via rename) and the write-ahead log truncated; the graph re-opens
    /// memory-mapped from the new base.
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Fold the structural-fork layers first (ids unchanged throughout): named
        // graphs recursively, then this graph's dictionary extension into a fresh
        // frozen base (so the NEXT fork is O(1) again) and the forked caches flat.
        // No-ops on a never-forked graph.
        for (_, g) in &mut self.named {
            g.compact()?;
        }
        if self.dict.is_forked() {
            self.dict = self.dict.compacted();
            let n = self.dict.len();
            let numerics = std::mem::replace(&mut self.numerics, NumData::Sparse(rustc_hash::FxHashMap::default()));
            self.numerics = numerics.fold(n);
            let temporals = std::mem::replace(&mut self.temporals, TempData::Sparse(rustc_hash::FxHashMap::default()));
            self.temporals = temporals.fold(n);
        }
        if self.store.has_overlay() {
            let triples: Vec<[Id; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows.iter().map(|r| scan.to_spo(r)).collect()
            };
            self.store = TripleStore::from_triples(triples);
        } else {
            // Nothing pending: for a directory-backed graph just discard the (no-op) log.
            #[cfg(feature = "mmap")]
            if let Some(w) = &mut self.wal {
                w.truncate().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            self.persist_swap(&dir)?;
        }
        Ok(())
    }

    /// [OPUS-4.8] (sq-x32t) ROLLBACK-SAFE on-disk swap shared by [`compact`](Self::compact) and

--- Base unchanged crates/sparq-core/src/lib.rs:3460-3473 ---
    /// re-opened memory-mapped from the new base with a fresh, empty WAL.
    #[cfg(feature = "mmap")]
    fn persist_swap(&mut self, dir: &std::path::Path) -> Result<(), String> {
        // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be renamed
        // away with `dir`), then run the shared crash-safe swap, writing `self`'s CURRENT image as
        // the new base. Adopt the re-opened directory-backed graph in place. [OPUS-4.8] (sq-ft7u)
        self.wal = None;
        let reopened = swap_dir_to_new_base(dir, |new_dir| self.save(new_dir))
            .map_err(|e| e.to_string())?;
        *self = reopened;
        Ok(())
    }

    /// [OPUS-4.8] (sq-ft7u) PUBLIC crash-safe RESTORE-INTO-DURABLE seam. Atomically REPLACE the

--- Base unchanged crates/sparq-core/src/lib.rs:3538-3595 ---
    /// The live dataset (default graph + every named graph, recursively) is dumped to `[Term; 3]`
    /// and re-interned into a brand-new [`Graph`] with an empty [`Dict`]; that fresh image is then
    /// swapped in via the SAME rollback-safe `persist_swap` the compaction
    /// uses (atomic, crash-safe, WAL-truncated). The live triple set is preserved EXACTLY
    /// (round-trip). For an in-memory graph it just replaces the dictionary/store in place.
    ///
    /// PHYSICAL-ERASURE SCOPE (honest): this scrubs the engine's own on-disk segments + dictionary;
    /// it cannot reach bytes already copied off-box (filesystem snapshots, block-level COW history,
    /// external backups), which the storage/backup tier must handle per the retention-erasure
    /// runbook.
    pub fn vacuum(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Re-intern the whole live dataset into a fresh graph with an EMPTY dictionary, so any
        // term orphaned by a prior delete/drop is dropped (it is never re-interned).
        let mut fresh = self.reintern_live()?;
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be
            // renamed away). `persist_swap` writes `fresh`'s image to `dir` and re-opens
            // `fresh` memory-mapped from the new base with a fresh WAL; adopt that re-opened,
            // directory-backed graph back into `self` so subsequent updates WAL-append to it.
            self.wal = None;
            fresh.persist_swap(&dir)?;
        }
        // Adopt the freshly re-interned (and, when persisted, re-opened) image in place.
        std::mem::swap(self, &mut fresh);
        Ok(())
    }

    /// [OPUS-4.8] (sq-x32t) Builds a fresh in-memory [`Graph`] holding exactly this graph's LIVE
    /// triples (default + named, recursively), re-interned into a brand-new [`Dict`] — so terms
    /// orphaned by a delete/drop are absent from the result. Carries NO directory/WAL association
    /// (the caller's [`persist_swap`](Self::persist_swap) gives the swapped-in graph its own).
    fn reintern_live(&self) -> Result<Graph, String> {
        // Dump the live default-graph triples as Terms (overlay already merged by `scan`).
        let live: Vec<[Term; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                })
                .collect()
        };
        let mut fresh = Graph::from_parts(Dict::new(), Vec::new());
        if !live.is_empty() {
            fresh.apply_delta(&live, &[])?;
        }
        // Named graphs: re-intern each recursively and attach it as an IN-MEMORY sub-graph (no
        // directory/WAL — the parent's `save` writes the whole `named/` sub-tree afresh).
        for (name, sub) in &self.named {
            let sub_fresh = sub.reintern_live()?;
            fresh.named.push((name.clone(), sub_fresh));
        }
        Ok(fresh)
    }

--- Base unchanged crates/sparq-core/src/dict.rs:1218-1260 ---
    }

    #[inline]
    fn intern_prefix(&mut self, prefix: &str) -> u32 {
        if let Some(&id) = self.prefix_ids.get(prefix) {
            return id;
        }
        let id = self.prefixes.len() as u32;
        let boxed: Box<str> = prefix.into();
        self.prefixes.push(boxed.clone());
        self.prefix_ids.insert(boxed, id);
        id
    }

    #[inline]
    fn intern_datatype(&mut self, dt: &str) -> u32 {
        if let Some(&id) = self.datatype_ids.get(dt) {
            return id;
        }
        let id = self.datatypes.len() as u32;
        self.datatypes.push(NamedNode::new_unchecked(dt));
        self.datatype_ids.insert(dt.into(), id);
        id
    }

    /// Assigns the next id to a freshly built `Stored` and indexes it. With a compacted
    /// base (blob/mmap), the new term is APPENDED to the arena above `base` — the dict's
    /// append-only growth path for delta-overlay updates.
    #[inline]
    fn push(&mut self, hash: u64, stored: Stored) -> Id {
        let id = (self.base + self.terms.len()) as Id + 1; // 1-based
        // Enforced in release too: once the (non-inline) dictionary reaches INLINE_BASE
        // distinct terms, new ids would collide with the inline-integer range and decode
        // as integers — silent corruption. 2^31 ≈ 2.1B distinct non-integer terms is the
        // hard capacity limit of the u32 scheme; fail loudly (widen Id to u64).
        assert!(id < INLINE_BASE, "dictionary exceeded the id capacity (2^31 distinct non-integer terms); widen Id to u64");
        self.terms.push(stored);
        let (base, terms, blob, prefixes, datatypes) =
            (self.base, &self.terms, &self.blob, &self.prefixes, &self.datatypes);
        self.table.insert_unique(hash, id, |&i| hash_tabled(i, base, terms, blob, prefixes, datatypes));
        id
    }


--- Base unchanged crates/sparq-core/src/dict.rs:1463-1553 ---

    /// Interns an IRI term from its string, returning its id.
    #[inline]
    pub fn intern_iri(&mut self, iri: &str) -> Id {
        let hash = hash_iri(iri);
        if let Some(id) = self.find_iri(hash, iri) {
            return id;
        }
        let (p, suffix) = split_iri(iri);
        let prefix = self.intern_prefix(p);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns an IRI already split into (prefix, suffix) — used by the parallel-load
    /// merge, where the canonical split is already known (no re-split, no concat).
    #[inline]
    fn intern_iri_parts(&mut self, prefix: &str, suffix: &str) -> Id {
        let hash = hash_iri_parts(prefix, suffix);
        if let Some(id) = self.find_iri_parts(hash, prefix, suffix) {
            return id;
        }
        let prefix = self.intern_prefix(prefix);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns a literal from its components, returning its id. Canonical small
    /// `xsd:integer`s are encoded inline and never stored.
    #[inline]
    pub fn intern_lit(&mut self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
        if let Some(id) = try_inline_lit(value, datatype) {
            return id;
        }
        let hash = hash_lit(value, datatype, lang);
        if let Some(id) = self.find_lit(hash, value, datatype, lang) {
            return id;
        }
        let datatype = self.intern_datatype(datatype);
        self.push(hash, Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) })
    }

    /// Interns a blank node from its label, returning its id.
    #[inline]
    pub fn intern_blank(&mut self, label: &str) -> Id {
        let hash = hash_blank(label);
        if let Some(id) = self.find_blank(hash, label) {
            return id;
        }
        self.push(hash, Stored::Blank(label.into()))
    }

    /// Interns an RDF 1.2 triple term whose components are ALREADY interned in this dict
    /// (ids may include inline-integer ids). Content-addressed by the component ids, so
    /// separately-interned identical triples share one id.
    ///
    /// [OPUS-4.8] `pub(crate)` so the byte-level N-Triples parser (`nt`) can intern a
    /// `<<( s p o )>>` triple-term object STRUCTURALLY (interning s/p/o first, then the
    /// triple by their ids) — the identical path `intern(&Term::Triple(_))` takes — so the
    /// N-Triples and Turtle loaders agree on triple-term content-addressing.
    pub(crate) fn intern_triple_ids(&mut self, ids: [Id; 3]) -> Id {
        let hash = hash_triple_ids(ids);
        if let Some(id) = self.find_triple_ids(hash, ids) {
            return id;
        }
        self.push(hash, Stored::Triple(ids))
    }

    /// Interns a term, returning its id (creating it if new). Dispatches to the
    /// component interners so the `Term` and byte-slice paths share one code path.
    /// An RDF 1.2 triple term is stored STRUCTURALLY: its s/p/o are interned first
    /// (recursing through a nested triple-term object) and the triple holds their ids.
    #[inline]
    pub fn intern(&mut self, term: &Term) -> Id {
        match term {
            Term::NamedNode(n) => self.intern_iri(n.as_str()),
            Term::Literal(l) => self.intern_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
            Term::BlankNode(b) => self.intern_blank(b.as_str()),
            Term::Triple(t) => {
                let s = match t.subject {
                    oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.intern_iri(n.as_str()),
                    oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.intern_blank(b.as_str()),
                };
                let p = self.intern_iri(t.predicate.as_str());
                let o = self.intern(&t.object);
                self.intern_triple_ids([s, p, o])
            }
        }
    }

    /// Interns a borrowed term (already-split components) without building a `Term` or
    /// concatenating the IRI — the fast path used by the sharded parallel merge for LEAF
    /// terms.

--- Base unchanged crates/sparq-core/src/dict.rs:2307-2370 ---
            0
        }
    }

    /// Folds a forked dictionary's extension into a FRESH frozen base (ids
    /// unchanged), so subsequent [`fork`](Self::fork)s are O(1) again — the dict
    /// half of `Graph::compact`. Non-forked dicts are returned unchanged (well,
    /// cloned); O(n) — call it from a compaction path only.
    pub fn compacted(&self) -> Dict {
        if self.frozen.is_none() {
            return self.clone();
        }
        let n = self.len();
        let mut terms: Vec<Stored> = Vec::with_capacity(n);
        for id in 1..=n as Id {
            terms.push(match self.record(id) {
                StoredRef::Iri { prefix, suffix } => Stored::Iri { prefix, suffix: suffix.into() },
                StoredRef::Lit { value, datatype, lang } => {
                    Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) }
                }
                StoredRef::Blank(b) => Stored::Blank(b.into()),
                StoredRef::Triple(ids) => Stored::Triple(ids),
            });
        }
        // The fork's prefix/datatype tables are supersets of the frozen base's with
        // identical indices (fork-time clone + append-only growth), so every record
        // copied above stays valid against them.
        let mut flat = Dict {
            prefixes: self.prefixes.clone(),
            prefix_ids: self.prefix_ids.clone(),
            datatypes: self.datatypes.clone(),
            datatype_ids: self.datatype_ids.clone(),
            terms,
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: 0,
            frozen: None,
        };
        flat.build_table();
        Dict {
            prefixes: flat.prefixes.clone(),
            prefix_ids: flat.prefix_ids.clone(),
            datatypes: flat.datatypes.clone(),
            datatype_ids: flat.datatype_ids.clone(),
            terms: Vec::new(),
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: n,
            frozen: Some(std::sync::Arc::new(flat)),
        }
    }

    pub fn len(&self) -> usize {
        // `base` is the blob/mmap'd record count (0 for a plain arena dict); appended
        // (delta-overlay) terms live in the arena above it.
        self.base + self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0

```

## Complete final apply_delta_mem

```rust
    fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
        let old_len = self.dict.len();
        // A delete only matters if every term resolves — otherwise the triple cannot be
        // present, and deleting must NOT intern the (absent) terms.
        let del_ids: Vec<[Id; 3]> = deletes
            .iter()
            .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
            .collect();
        let ins_ids: Vec<[Id; 3]> = inserts
            .iter()
            .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
            .collect();
        // Keep the numeric- and temporal-filter caches covering the grown dictionary.
        self.numerics.extend_for(&self.dict, old_len);
        self.temporals.extend_for(&self.dict, old_len);
        // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
        // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
        // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
        // [GPT-6 Astra] Interning is append-only and extend_for touches only new IDs.
        // Equal lengths therefore preserve every term/numeric value read by the memo.
        // Any future path changing existing terms or cached values must invalidate it.
        if self.dict.len() != old_len {
            let _ = self.high_precision_decimal.compare_exchange(
                1,
                0,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        self.store.apply_delta(&ins_ids, &del_ids);
    }


```

## Core feature/test CI selection

```yaml
# Feature-matrix fragment for `sparq-core`.
#
# One self-contained leg mapping per opt-in feature(group). Assembled into the
# feature-matrix `opt-in-features` strategy by scripts/assemble-feature-matrix.py
# (the `setup` job emits the combined matrix as JSON consumed via fromJSON). Adding a
# leg for THIS crate edits no OTHER crate's fragment, so concurrent opt-in-feature PRs for
# DIFFERENT crates never textually conflict (bead sq-ibrze). [OPUS-4.8]
# SCOPE ([SONNET-4.6] issue #2384): adding/removing/renaming a leg changes the emitted
# leg-name set, and THAT is a TWO-FILE change — this fragment AND
# scripts/tests/feature-matrix-legnames.golden.txt. Every other edit here (features,
# test, tier, tier-reason, comments) leaves the name set alone and is a SINGLE-file
# change. See README.md in this directory.
#
# CONTRACT (gate-critical): each leg's emitted check-run NAME is `opt-in <name>`,
# discovered as a REQUIRED check by the ci-summary `gate` aggregator BY NAME. Keep
# `name` free of the whole words "advisory"/"informational" or the leg stops gating.
# Fields: name (required -> check-run name), crate (required), features (required,
# non-empty comma list), test (bool — run `cargo test` for the leg).

# sparq-core: the compact 3-index browser build, and the native out-of-core
# mmap + spillable build-time dictionary.
- name: "sparq-core (compact-index)"
  crate: "sparq-core"
  features: "compact-index"
  test: true

- name: "sparq-core (mmap, dict-spill)"
  crate: "sparq-core"
  features: "mmap,dict-spill"
  test: true

# [OPUS-4.8] sq-dvyi: opt-in JSON-LD ingest (oxjsonld). OFF by default so the
# lean wasm bundle stays under the perf floor; this leg runs the gated
# jsonld_* loader tests (object/array/base-IRI/`@graph`/malformed).
- name: "sparq-core (jsonld)"
  crate: "sparq-core"
  features: "jsonld"
  test: true

# [SONNET-4.6] #3547: exercise the native Turtle parser's inline differential,
# malformed-input parity, and loader tests on every feature-matrix run.
- name: "sparq-core (native-ttl)"
  crate: "sparq-core"
  features: "native-ttl"
  test: true

# [SONNET-4.6] #3547: exercise the opt-in RFC-3987 IRI validation path. These
# tests live in inline cfg(test) modules, so the structural C1 guard cannot infer
# their need for a dedicated feature executor.
- name: "sparq-core (iri-fast)"
  crate: "sparq-core"
  features: "iri-fast"
  test: true

# [OPUS-4.8] sq-wihld (survey §A1): opt-in per-block Bloom filters on the high-NDV
# leading column of a block-compressed permutation. OFF by default (lean core); this
# leg runs the bloom equivalence + block-skip tests (compress::tests::bloom_*).
- name: "sparq-core (block-bloom)"
  crate: "sparq-core"
  features: "block-bloom"
  test: true

# [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7: SPQCPRM2 frame-of-reference col2-reset format.
# The V2 EMITTER is OFF by default (the store still WRITES SPQCPRM1 bit-exactly); this leg runs
# the V2 emit-gate + write/open round-trip + streaming-writer byte-identity + cross-version +
# corrupt-magic mutation-witness + SPQCPRM1-vs-SPQCPRM2 differential + zigzag tests, PLUS the
# V2 corruption oracle (`mmap_loader_survives_corruption_compressed_v2`). `mmap` is co-enabled
# (spqcprm2 implies it) so the on-disk write/open path and FILE_MAGIC_V2 tests run.
- name: "sparq-core (spqcprm2, mmap)"
  crate: "sparq-core"
  features: "spqcprm2,mmap"
  test: true

# [GPT-6 Astra] #4246: explicitly exercise the experimental deletion cache;
# ordinary/core defaults remain feature-off. Includes ownership/invalidation tests.
- name: "sparq-core (overlay-deleted-projections)"
  crate: "sparq-core"
  features: "overlay-deleted-projections"
  test: true

```

Full final core source, dependency manifests, commands, binary hashes, raw diagnostics and mutant deltas are in the separate frozen evidence manifest. This packet includes the full final diff and load-bearing function bodies; it does not claim the entire unchanged core crate is reproduced here.
