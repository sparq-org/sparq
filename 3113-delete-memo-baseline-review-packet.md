# Issue3113: delete-only numeric-memo baseline

Actual GPT-6 Astra xhigh. Diagnostic source only; no production fix. Current cleanfe328 core source applies to mainf50 through root-verified whole-tree equality.

One build succeeded and one exact filtered actual-core test failed after six successful case assertions. The failure was a preregistered fixture assumption: canonical integer999999 uses an inline ID, so dictionary length does not grow. No correction/retry occurred.

## Actual observations

[
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
  }
]

The initial graph has4098triples and4100dictionary IDs; inline integers do not occupy ordinary dictionary IDs. The observer counts actual cold-loop predicate visits. Nonempty real deletion, known-terms absent deletion and known-terms reinsertion each redo4100lookups. Immediate warm call and empty batch do none. No SPARQL query/bag/latency claim.

## Failure and limits

```text

thread 'delete_memo_baseline::fixed_delete_only_rescans' (4979994) panicked at diagnostic/core/src/lib.rs:11335:9:
assertion failed: graph.dict.len() > d
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

New-integer classification, exact/inexact decimal and sticky-state2 cases were not reached. Existing memo-survives-insert test was unchanged and compiled but filtered out.0tests passed;1failed;150filtered. No candidate or cache mutant. Original first-cold-scan issue remains open.

Rust1.97.1/edition2021,O3/unwind/noLTO/codegen16,coredefault+parallel,7cached directlibraries; no Cargo/dependency build. BinarySHA256: 1ab036a2f182aba7dceef91c315c669c4eca4e4cb2039665925916171861eb37. Resource limits held. Raw execution/resource/metadata receipts remain in the local evidence bundle.

## Complete observer/test delta

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
+                        delete_memo_baseline::visit();
+                        self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id)
+                    });
                 self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
                 found
             }
@@ -11255,3 +11259,94 @@
         assert_eq!(f2.numeric_value(id), Some(4.5));
     }
 }
+
+// [GPT-6 Astra] Diagnostic-only cold-branch work observer; no memo policy change.
+#[cfg(test)]
+mod delete_memo_baseline {
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
+    fn fixed_delete_only_rescans() {
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
+        graph.apply_delta(&[integer], &[]).unwrap();
+        assert!(graph.dict.len() > d);
+        check(&graph, "new-integer", false, graph.dict.len());
+        let exact = triple("urn:keep", "urn:query", Literal::new_typed_literal("1.5", xsd::DECIMAL));
+        graph.apply_delta(&[exact], &[]).unwrap();
+        check(&graph, "new-exact-decimal", false, graph.dict.len());
+
+        let inexact = triple("urn:keep", "urn:query", Literal::new_typed_literal("2.000000000000000003", xsd::DECIMAL));
+        graph.apply_delta(std::slice::from_ref(&inexact), &[]).unwrap();
+        let offender_id = graph.id_of(&inexact[2]).unwrap() as usize;
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

## Relevant unchanged source

```rust
crates/sparq-core/src/lib.rs
2726:     /// [OPUS-4.8] (sq-lr2ii) `true` if the graph holds any `xsd:decimal` literal with MORE
2727:     /// than 15 significant digits — a value the f64 `numerics` cache CANNOT represent exactly,
2728:     /// so the engine's f64-based sargable FILTER fast path could decide `=`/`<`/`>`/`<=`/`>=`
2729:     /// WRONGLY for it (e.g. `"1.000000000000000001"^^xsd:decimal` shares the f64 `1.0` with the
2730:     /// constant `1`, yet is not equal to it). The engine consults this at the sargable-decision
2731:     /// point to DECLINE that fast path and fall back to the exact general evaluator; a graph
2732:     /// without any such decimal keeps the fast path (a decimal of `<= 15` significant digits
2733:     /// round-trips through f64 unambiguously, and a large-integer collision needs a `> 15`-digit
2734:     /// CONSTANT, which the engine already declines). Integers/float/double are exempt: an
2735:     /// integer's exactness is the constant-side guard's job and a float/double's value IS its f64.
2736:     ///
2737:     /// Memoised (see `high_precision_decimal`): the first call scans the graph's numeric terms
2738:     /// once (short-circuiting on the first offender); later calls are an atomic load. A delta
2739:     /// that appends terms resets the memo so it is recomputed over the grown dictionary. This is
2740:     /// a CONSERVATIVE, graph-wide gate — one offending decimal declines the numeric fast path for
2741:     /// every comparison on the graph — chosen for safety (correctness is never at risk; only the
2742:     /// pushdown optimisation is skipped for affected graphs).
2743:     pub fn has_high_precision_decimal(&self) -> bool {
2744:         use std::sync::atomic::Ordering::Relaxed;
2745:         match self.high_precision_decimal.load(Relaxed) {
2746:             2 => true,
2747:             1 => false,
2748:             _ => {
2749:                 let found = (1..=self.dict.len() as Id)
2750:                     .any(|id| self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id));
2751:                 self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
2752:                 found
2753:             }
2754:         }
2755:     }

2947:     pub fn apply_delta(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> Result<(), String> {
2948:         if inserts.is_empty() && deletes.is_empty() {
2949:             return Ok(());
2950:         }
2951:         #[cfg(feature = "mmap")]
2952:         if let Some(w) = &mut self.wal {
2953:             w.append_batch(inserts, deletes).map_err(|e| format!("WAL append failed: {e}"))?;
2954:         }
2955:         self.apply_delta_mem(inserts, deletes);
2956:         Ok(())
2957:     }

3371:     /// the target the WAL replays into on [`open`](Self::open).
3372:     fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
3373:         // A delete only matters if every term resolves — otherwise the triple cannot be
3374:         // present, and deleting must NOT intern the (absent) terms.
3375:         let del_ids: Vec<[Id; 3]> = deletes
3376:             .iter()
3377:             .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
3378:             .collect();
3379:         let old_len = self.dict.len();
3380:         let ins_ids: Vec<[Id; 3]> = inserts
3381:             .iter()
3382:             .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
3383:             .collect();
3384:         // Keep the numeric- and temporal-filter caches covering the grown dictionary.
3385:         self.numerics.extend_for(&self.dict, old_len);
3386:         self.temporals.extend_for(&self.dict, old_len);
3387:         // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
3388:         // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
3389:         // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
3390:         let _ = self.high_precision_decimal.compare_exchange(
3391:             1,
3392:             0,
3393:             std::sync::atomic::Ordering::Relaxed,
3394:             std::sync::atomic::Ordering::Relaxed,
3395:         );
3396:         self.store.apply_delta(&ins_ids, &del_ids);
3397:     }

9622:     /// [OPUS-4.8] sq-lr2ii — the memoised flag must NOT go stale: inserting an f64-inexact
9623:     /// decimal via a delta AFTER the flag was computed as `false` must flip it to `true`.
9624:     #[test]
9625:     fn has_high_precision_decimal_memo_survives_delta_insert() {
9626:         let mut g = Graph::load_str("<http://ex/s> <http://ex/p> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", "ntriples").unwrap();
9627:         // Compute (and memoise) the flag as false over the integer-only graph.
9628:         assert!(!g.has_high_precision_decimal());
9629:         // Insert a high-precision decimal; the memo must be invalidated + recomputed to true.
9630:         let s = Term::NamedNode(NamedNode::new_unchecked("http://ex/s"));
9631:         let p = Term::NamedNode(NamedNode::new_unchecked("http://ex/d"));
9632:         let o = Term::Literal(Literal::new_typed_literal("2.000000000000000003", xsd::DECIMAL));
9633:         g.apply_delta(&[[s, p, o]], &[]).unwrap();
9634:         assert!(g.has_high_precision_decimal(), "delta-inserted inexact decimal must flip the memo");
9635:     }
crates/sparq-core/src/dict.rs
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

1491:     pub fn intern_lit(&mut self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
1492:         if let Some(id) = try_inline_lit(value, datatype) {
1493:             return id;
1494:         }
1495:         let hash = hash_lit(value, datatype, lang);
1496:         if let Some(id) = self.find_lit(hash, value, datatype, lang) {
1497:             return id;
1498:         }
1499:         let datatype = self.intern_datatype(datatype);
1500:         self.push(hash, Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) })
1501:     }
1502: 
1503:     /// Interns a blank node from its label, returning its id.
1504:     #[inline]
1505:     pub fn intern_blank(&mut self, label: &str) -> Id {

1534:     pub fn intern(&mut self, term: &Term) -> Id {
1535:         match term {
1536:             Term::NamedNode(n) => self.intern_iri(n.as_str()),
1537:             Term::Literal(l) => self.intern_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
1538:             Term::BlankNode(b) => self.intern_blank(b.as_str()),
1539:             Term::Triple(t) => {
1540:                 let s = match t.subject {
1541:                     oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.intern_iri(n.as_str()),
1542:                     oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.intern_blank(b.as_str()),
1543:                 };
1544:                 let p = self.intern_iri(t.predicate.as_str());
1545:                 let o = self.intern(&t.object);
1546:                 self.intern_triple_ids([s, p, o])
1547:             }
1548:         }
1549:     }
```

Root owns any next scope, hunk-ownership check and actual independent review. No automatic rerun.
