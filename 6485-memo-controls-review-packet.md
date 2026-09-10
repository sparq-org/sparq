# Issue6485 corrected memo controls

Author: GPT-6 Astra xhigh. Baseline only; no production patch.

## Results and limitations

```json
{
  "source_head": "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464",
  "execution": {
    "builds": 1,
    "filtered_runs": 1,
    "passed": 2,
    "failed": 0,
    "filtered_out": 149,
    "selected_names": [
      "memo_controls::has_high_precision_decimal_memo_corrected_controls",
      "tests::has_high_precision_decimal_memo_survives_delta_insert"
    ],
    "compiler_exit": 0,
    "test_exit": 0,
    "compiler_seconds": 11.779763459000002,
    "test_seconds": 1.0218791659999997,
    "phase_seconds": 13.054174291999999,
    "retries": 0,
    "dependency_builds": 0,
    "pending_commands": false
  },
  "correction": "Old diagnostic incorrectly assumed canonical integer999999 grows Dict. New preregistration pins its inline ID and unchanged D. The entire old48-file failure bundle was rehashed and left immutable.",
  "observations": [
    {
      "case": "initial",
      "dict_len": 4100,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4100
    },
    {
      "case": "immediate-warm",
      "dict_len": 4100,
      "before": 1,
      "state": 1,
      "result": false,
      "visits": 0
    },
    {
      "case": "existing-delete",
      "dict_len": 4100,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4100
    },
    {
      "case": "known-terms-absent-delete",
      "dict_len": 4100,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4100
    },
    {
      "case": "known-terms-reinsert",
      "dict_len": 4100,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4100
    },
    {
      "case": "empty-delta",
      "dict_len": 4100,
      "before": 1,
      "state": 1,
      "result": false,
      "visits": 0
    },
    {
      "case": "inline-integer",
      "dict_len": 4100,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4100
    },
    {
      "case": "new-exact-decimal",
      "dict_len": 4101,
      "before": 0,
      "state": 1,
      "result": false,
      "visits": 4101
    },
    {
      "case": "new-inexact-decimal",
      "dict_len": 4102,
      "before": 0,
      "state": 2,
      "result": true,
      "visits": 4102
    },
    {
      "case": "state2-after-delete",
      "dict_len": 4102,
      "before": 2,
      "state": 2,
      "result": true,
      "visits": 0
    },
    {
      "case": "state2-after-insert",
      "dict_len": 4102,
      "before": 2,
      "state": 2,
      "result": true,
      "visits": 0
    }
  ],
  "work_count_qualification": "Actual increments inside each cold-branch ID predicate; visits include nonnumeric dictionary entries/cache misses and are not decimal parse counts. No query bag, latency, throughput or heap benefit measured.",
  "profile": "Actual core module tree, Rust1.97.1 nativeaarch64/edition2021/O3/unwind/LTOoff/codegen16, default+parallel; seven rehashed direct rlibs. Direct rustc adaptation, not full Cargo/workspace/wasm or mmap/dict-spill/compact-index gate.",
  "limits": [
    "Only two selected tests executed; unchanged other tests compiled but filtered.",
    "No production candidate/mutant, full suite, concurrent reader witness or engine query executed.",
    "Public mutable dict/store prevents an unconditional all-caller state-invariant claim; see explicit caveat.",
    "Original first-cold-scan issue3113 stays open."
  ]
}
```

## Source invariant assessment

```json
{
  "claim": "Within apply_delta_mem on a coherent Graph, equal old/current dictionary length means this call introduced no stored term or changed existing numeric meaning. This is scoped to the inspected mutation path, not arbitrary public-field mutation.",
  "steps": [
    {
      "source": "crates/sparq-core/src/lib.rs:3372-3398",
      "evidence": "Takes &mut self; deletes resolve existing IDs only; every insert synchronously interns complete Term components; old_len is captured before insertion; caches extend only after all interning. No deferred ID or numeric-cache completion runs in this method."
    },
    {
      "source": "crates/sparq-core/src/dict.rs:1247-1259,1466-1553,2363-2366",
      "evidence": "Existing terms return old IDs. A novel stored term goes through push and increments terms.len; base+terms.len is dictionary length. Prefix/datatype additions append; no existing record is rewritten by these interners. Triple terms recursively intern children first."
    },
    {
      "source": "crates/sparq-core/src/dict.rs:62-72; crates/sparq-core/src/lib.rs:796-824",
      "evidence": "Only canonical in-range xsd:integer can return a new inline ID without stored growth. It cannot introduce xsd:decimal; the high-precision predicate inspects stored numeric IDs."
    },
    {
      "source": "crates/sparq-core/src/lib.rs:240-266,300-325",
      "evidence": "Owned/Sparse/Mapped/Forked numeric caches extend exactly old_len..dict.len; equal lengths execute zero iterations and cannot mutate cached values. Fork-cache fold preserves each existing ID/value."
    },
    {
      "source": "crates/sparq-core/src/lib.rs:1666-1699,2100-2235,2600-2645,4799-4811",
      "evidence": "Sharded/spilled deferred temporary IDs belong to builders, which finish consolidation/remapping before constructing/opening the Graph and numeric caches. They are not pending work inside apply_delta_mem."
    },
    {
      "source": "crates/sparq-core/src/lib.rs:2885-2895,3410-3445,3462-3471,3538-3595; crates/sparq-core/src/dict.rs:2315-2360",
      "evidence": "Fork starts its own memo at0. In-memory compact preserves dictionary IDs/terms and folds numeric cache; sticky2 remains conservative even after triples are deleted. Durable compact reopens a fresh Graph with0. Vacuum reinterns live terms into a fresh Graph and replaces whole state. Same size across arbitrary replacement is not used as an invariant."
    },
    {
      "source": "crates/sparq-core/src/lib.rs:1910-1962",
      "evidence": "Open constructs memo0, with numeric cache loaded/recomputed, before WAL replay into apply_delta_mem. This source-only qualification does not establish runtime mmap/dict-spill coverage."
    }
  ],
  "public_field_caveat": {
    "source": "crates/sparq-core/src/lib.rs:67-76",
    "fact": "Graph.dict and Graph.store are public. A safe external caller can replace/intern the dictionary outside the coordinated mutation methods; numeric caches are private and are not automatically synchronized by those field writes. Thus a universal guarantee for every safely constructible Graph is not proven.",
    "scope": "Focused search of core and engine production source found dict assignment only in compact and direct self.dict.intern only in apply_delta_mem; this is not a whole-workspace/external-client census.",
    "decision_needed": "Independent review should assess the established coordinated Graph invariant versus the exposed fields. Do not present length as a global dictionary-generation identity or expand this diagnostic into a public API redesign."
  },
  "smallest_candidate": "Condition only the existing state1\u21920 compare_exchange on self.dict.len()!=old_len after interning/cache extension; keep state2, state0, ordering, store mutation, fork/build/reopen policies unchanged. This is supported for the coherent apply_delta path; not yet implemented/approved.",
  "future_meaningful_tests": [
    "Keep existing false\u2192true insertion test unchanged. Promote focused cold/warm/unchanged-dictionary cases into scoped tests and require zero cold predicate visits after the proposed guard.",
    "Reverting the new length condition must execute and fail the saved no-rescan assertions; suppressing all invalidation must fail new stored inexact decimal false\u2192true.",
    "Include fork/compact and relevant mmap/dict-spill source/runtime coverage before broad admission; do not assume current default-only run executes these configurations.",
    "Do not close parent3113: first cold dictionary scan is unchanged and no whole public SPARQL query or latency benefit has been measured."
  ]
}
```

## Preregistered protocol

```json
{
  "at": "2026-09-10T17:06:09.624327+00:00",
  "fixture": "4096 :sN :p N integer triples plus :keep :query1 and :remove :p2; prefixurn:. D is measured actual dictionary length. All initial numeric values exact.",
  "sequence": [
    {
      "case": "initial",
      "expected_visits": "D",
      "result": false
    },
    {
      "case": "immediate-warm",
      "expected_visits": 0,
      "result": false
    },
    {
      "case": "existing-delete",
      "expected_visits": "D unchanged",
      "result": false
    },
    {
      "case": "known-terms-absent-delete",
      "expected_visits": "D unchanged",
      "result": false
    },
    {
      "case": "known-terms-reinsert",
      "expected_visits": "D unchanged",
      "result": false
    },
    {
      "case": "empty-delta",
      "expected_visits": 0,
      "result": false
    },
    {
      "case": "inline-integer",
      "expected_visits": "D unchanged",
      "result": false,
      "dictionary": "999999 is canonical integer, ID=INLINE_BASE+999999; no growth"
    },
    {
      "case": "new-exact-decimal",
      "expected_visits": "newD",
      "result": false,
      "dictionary": "1.5 decimal is stored, has two significant digits, numeric cache Some(1.5), no high precision"
    },
    {
      "case": "new-inexact-decimal",
      "expected_visits": "numeric offender dictionaryID",
      "result": true,
      "dictionary": "2.000000000000000003 decimal is stored, 19 significant digits, numeric cache present; first offender ID bounds short circuit"
    },
    {
      "case": "state2-after-delete",
      "expected_visits": 0,
      "result": true
    },
    {
      "case": "state2-after-insert",
      "expected_visits": 0,
      "result": true
    }
  ],
  "execution": "One actual-core direct rustc --test build; one substring-filtered run has_high_precision_decimal_memo --nocapture --test-threads=1. Exactly two tests statically selected. Stop on first unexpected result; no correction/retry after compiler launch.",
  "profile": "Rust1.97.1,edition2021,O3,unwind,LTOoff,codegen16,coredefault+parallel only. Adapt recorded actualcore lib invocation; add sole proptest devdependency from executed engine cfgtest invocation.",
  "limits": {
    "seconds": 480,
    "new_allocated_bytes": 268435456,
    "start_free": 2214592512,
    "continuous_free": 2147483648
  },
  "existing_test": "tests::has_high_precision_decimal_memo_survives_delta_insert remains unchanged and is selected alongside memo_controls::has_high_precision_decimal_memo_corrected_controls.",
  "source_basis": [
    "dict.rs:62-72 and1491-1501 canonical integer inline; decimal stored",
    "lib.rs:796-824 decimal precision classifier significant digits >15",
    "lib.rs:240-266 numeric cache extends only new IDs",
    "lib.rs:3372-3398 baseline memo resets state1 after every nonempty apply_delta_mem"
  ],
  "selected_test_names": [
    "has_high_precision_decimal_memo_survives_delta_insert",
    "has_high_precision_decimal_memo_corrected_controls"
  ]
}

```

## Exact diagnostic delta

```diff
--- a/crates/sparq-core/src/lib.rs
+++ diagnostic/core/src/lib.rs
@@ -2747,7 +2747,11 @@
             1 => false,
             _ => {
                 let found = (1..=self.dict.len() as Id)
-                    .any(|id| self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id));
+                    .any(|id| {
+                        #[cfg(test)]
+                        memo_controls::visit();
+                        self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id)
+                    });
                 self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
                 found
             }
@@ -11255,3 +11259,105 @@
         assert_eq!(f2.numeric_value(id), Some(4.5));
     }
 }
+
+// [GPT-6 Astra] Diagnostic-only cold-branch work observer; no memo policy change.
+#[cfg(test)]
+mod memo_controls {
+    use super::*;
+    use std::cell::Cell;
+    use std::sync::atomic::Ordering::Relaxed;
+
+    thread_local! {
+        static VISITS: Cell<usize> = const { Cell::new(0) };
+    }
+
+    pub(super) fn visit() {
+        VISITS.with(|visits| visits.set(visits.get() + 1));
+    }
+
+    fn triple(subject: &str, predicate: &str, object: Literal) -> [Term; 3] {
+        [NamedNode::new(subject).unwrap().into(), NamedNode::new(predicate).unwrap().into(), object.into()]
+    }
+
+    fn check(graph: &Graph, name: &str, expected: bool, expected_visits: usize) {
+        let before = graph.high_precision_decimal.load(Relaxed);
+        VISITS.with(|visits| visits.set(0));
+        let result = graph.has_high_precision_decimal();
+        let visits = VISITS.with(Cell::get);
+        let state = graph.high_precision_decimal.load(Relaxed);
+        let d = graph.dict.len();
+        println!(r#"MEMO_WITNESS {{"case":"{name}","dict_len":{d},"before":{before},"state":{state},"result":{result},"visits":{visits}}}"#);
+        assert_eq!(result, expected, "{name}");
+        assert_eq!(visits, expected_visits, "{name}");
+        assert_eq!(state, if expected { 2 } else { 1 }, "{name}");
+    }
+
+    #[test]
+    fn has_high_precision_decimal_memo_corrected_controls() {
+        assert!(cfg!(feature = "parallel"));
+        assert!(!cfg!(feature = "mmap"));
+        assert!(!cfg!(feature = "dict-spill"));
+        assert_eq!(store::BUILT.len(), 6);
+        let mut ttl = String::from("@prefix : <urn:> .\n");
+        for i in 0..4096 {
+            ttl.push_str(&format!(":s{i} :p {i} .\n"));
+        }
+        ttl.push_str(":keep :query 1 .\n:remove :p 2 .\n");
+        let mut graph = Graph::load_str(&ttl, "turtle").unwrap();
+        let d = graph.dict.len();
+        assert!(d >= 4096);
+        assert_eq!(graph.len(), 4098);
+        check(&graph, "initial", false, d);
+        check(&graph, "immediate-warm", false, 0);
+
+        let removed = triple("urn:remove", "urn:p", Literal::from(2));
+        graph.apply_delta(&[], std::slice::from_ref(&removed)).unwrap();
+        assert_eq!(graph.len(), 4097);
+        assert_eq!(graph.dict.len(), d);
+        check(&graph, "existing-delete", false, d);
+
+        let absent = triple("urn:keep", "urn:p", Literal::from(2));
+        assert!(absent.iter().all(|term| graph.id_of(term).is_some()));
+        graph.apply_delta(&[], &[absent]).unwrap();
+        assert_eq!(graph.len(), 4097);
+        assert_eq!(graph.dict.len(), d);
+        check(&graph, "known-terms-absent-delete", false, d);
+
+        graph.apply_delta(&[removed], &[]).unwrap();
+        assert_eq!(graph.len(), 4098);
+        assert_eq!(graph.dict.len(), d);
+        check(&graph, "known-terms-reinsert", false, d);
+        graph.apply_delta(&[], &[]).unwrap();
+        check(&graph, "empty-delta", false, 0);
+
+        let integer = triple("urn:keep", "urn:query", Literal::from(999999));
+        graph.apply_delta(std::slice::from_ref(&integer), &[]).unwrap();
+        assert_eq!(graph.dict.len(), d, "canonical integer stays inline");
+        assert_eq!(graph.id_of(&integer[2]), Some(dict::INLINE_BASE + 999999));
+        check(&graph, "inline-integer", false, d);
+        let exact = triple("urn:keep", "urn:query", Literal::new_typed_literal("1.5", xsd::DECIMAL));
+        assert_eq!(decimal_significant_digits("1.5"), 2);
+        graph.apply_delta(std::slice::from_ref(&exact), &[]).unwrap();
+        assert!(graph.dict.len() > d);
+        let exact_id = graph.id_of(&exact[2]).unwrap();
+        assert!(!is_high_precision_decimal(&graph.dict, exact_id));
+        assert_eq!(graph.numeric_value(exact_id), Some(1.5));
+        check(&graph, "new-exact-decimal", false, graph.dict.len());
+
+        let inexact = triple("urn:keep", "urn:query", Literal::new_typed_literal("2.000000000000000003", xsd::DECIMAL));
+        let before_inexact = graph.dict.len();
+        assert_eq!(decimal_significant_digits("2.000000000000000003"), 19);
+        graph.apply_delta(std::slice::from_ref(&inexact), &[]).unwrap();
+        assert!(graph.dict.len() > before_inexact);
+        let offender_id = graph.id_of(&inexact[2]).unwrap() as usize;
+        assert!(is_high_precision_decimal(&graph.dict, offender_id as Id));
+        assert!(graph.numeric_value(offender_id as Id).is_some());
+        check(&graph, "new-inexact-decimal", true, offender_id);
+        graph.apply_delta(&[], &[inexact]).unwrap();
+        check(&graph, "state2-after-delete", true, 0);
+        let more = triple("urn:keep", "urn:query", Literal::from(999998));
+        graph.apply_delta(&[more], &[]).unwrap();
+        check(&graph, "state2-after-insert", true, 0);
+        println!("MEMO_WITNESS_COMPLETE cases=11 public_query_executed=false");
+    }
+}

```

## Actual test output

```text

running 2 tests
test memo_controls::has_high_precision_decimal_memo_corrected_controls ... MEMO_WITNESS {"case":"initial","dict_len":4100,"before":0,"state":1,"result":false,"visits":4100}
MEMO_WITNESS {"case":"immediate-warm","dict_len":4100,"before":1,"state":1,"result":false,"visits":0}
MEMO_WITNESS {"case":"existing-delete","dict_len":4100,"before":0,"state":1,"result":false,"visits":4100}
MEMO_WITNESS {"case":"known-terms-absent-delete","dict_len":4100,"before":0,"state":1,"result":false,"visits":4100}
MEMO_WITNESS {"case":"known-terms-reinsert","dict_len":4100,"before":0,"state":1,"result":false,"visits":4100}
MEMO_WITNESS {"case":"empty-delta","dict_len":4100,"before":1,"state":1,"result":false,"visits":0}
MEMO_WITNESS {"case":"inline-integer","dict_len":4100,"before":0,"state":1,"result":false,"visits":4100}
MEMO_WITNESS {"case":"new-exact-decimal","dict_len":4101,"before":0,"state":1,"result":false,"visits":4101}
MEMO_WITNESS {"case":"new-inexact-decimal","dict_len":4102,"before":0,"state":2,"result":true,"visits":4102}
MEMO_WITNESS {"case":"state2-after-delete","dict_len":4102,"before":2,"state":2,"result":true,"visits":0}
MEMO_WITNESS {"case":"state2-after-insert","dict_len":4102,"before":2,"state":2,"result":true,"visits":0}
MEMO_WITNESS_COMPLETE cases=11 public_query_executed=false
ok
test tests::has_high_precision_decimal_memo_survives_delta_insert ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.01s


```

Full current core tree and relevant source excerpts are separate frozen files; this packet does not duplicate all unchanged source.
