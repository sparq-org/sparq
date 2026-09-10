# Baseline hash-build witness — fixed Q3136/P2048

Actual GPT-6 Astra xhigh diagnostic. One direct compiler invocation and one filtered test passed; no production change or hash cache exists. Root-reported merged main f50 equals source tree fe328; local engine/core/substrate/lock are byte-identical to main459. Local f50 Git object was absent; this limitation is preserved.

## Observed results

{
  "events": [
    {
      "binary": true,
      "budget_active": false,
      "estimates": [
        3136,
        2048
      ],
      "filters": [
        "2:< 16",
        null
      ],
      "kind": "plan",
      "patterns": [
        "?s <http://ex/q> ?o",
        "?s <http://ex/p> ?o"
      ],
      "residual": 0,
      "seed": 1,
      "seed_rows": 2048,
      "seed_sort": "s",
      "view_empty": false
    },
    {
      "actual": "o",
      "kernel": "hash",
      "kind": "step",
      "pattern": 0,
      "requested": 2,
      "scanned": true,
      "start": 0
    },
    {
      "build_rows": 64,
      "entries": 64,
      "key_cols": [
        [
          0,
          0
        ],
        [
          1,
          1
        ]
      ],
      "kind": "built",
      "left_rows": 1024,
      "postings": 64,
      "probe_rows": 1024,
      "rhs": true,
      "right_rows": 64,
      "tables": 1
    },
    {
      "actual": "o",
      "kernel": "hash",
      "kind": "step",
      "pattern": 0,
      "requested": 2,
      "scanned": false,
      "start": 1024
    },
    {
      "build_rows": 64,
      "entries": 64,
      "key_cols": [
        [
          0,
          0
        ],
        [
          1,
          1
        ]
      ],
      "kind": "built",
      "left_rows": 1024,
      "postings": 64,
      "probe_rows": 1024,
      "rhs": true,
      "right_rows": 64,
      "tables": 1
    }
  ],
  "result": {
    "bag": {
      "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"11\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"12\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"13\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"14\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"15\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"3\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"4\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"6\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"7\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4,
      "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>": 4
    },
    "build_calls": 2,
    "build_input_rows": 128,
    "rows": 64,
    "scan_closures": 1,
    "stored_postings": 128
  }
}

The completed serial tables show64build input rows and64postings each, twice. The unmodified build_table enumerates each input once, so128row visits follow from source; per-row visits were not separately instrumented. One RHS scan closure ran, then the existing scan cache supplied the second block. Every claimed path/bag assertion executed in the actual public query.

## Build and limitations

Rust1.97.1, edition2021, native aarch64, O3, unwind, LTOoff, codegen16. Engine default/digest/parallel/regex; core dev-unified default/dict-spill/mmap/parallel, all six BUILT permutations. Compiler plus17directlibraries rehashed; no dependencies built. One test passed;334filtered. Compile17.79s/test1.02s are operation receipts, not performance claims. BinarySHA256: 9010d888d464c77938bfea2f7bf20a1cff7f5219d0988dd7c5368f2be4f90302. Native linker emitted an Xcode filesystem-event/cache-directory warning; compilation and test exited0. Full raw diagnostics retained locally.

No timing/heap benefit claim, cached candidate, cache mutant, parallel64partition/armed-budget/ZK test, or admission. F5–F7 benefit ceiling, shared4MiB/crowdout/refund and safe first-hit/deferred-accounting design remain on hold.

## Complete diagnostic delta

```diff
--- a/crates/sparq-engine/src/exec.rs
+++ diagnostic/engine/src/exec.rs
@@ -4503,6 +4503,8 @@
         None,
     );
 
+    #[cfg(test)]
+    hash_reuse_fixed_witness::plan(graph, patterns, &prepared, &pat_filters, residual.len(), seed, &seed_all);
     #[cfg(test)]
     capped_rhs_tests::observe(0);
     // [GPT-6 Astra] Query-local and lazy: never scan an unreached step. Pattern
@@ -9119,6 +9121,8 @@
     };
     #[cfg(not(feature = "parallel"))]
     let tables = vec![sjoin::build_table(&build.rows, &keys)];
+    #[cfg(test)]
+    hash_reuse_fixed_witness::built(left, right, build, probe, &keys, &tables);
     // The probe is read-only over the (partitioned) table, so for a large probe side build the
     // output in parallel on native.
     #[cfg(feature = "parallel")]
@@ -21664,6 +21668,7 @@
         scanned: bool,
         actual: Option<&Variable>,
     ) {
+        super::hash_reuse_fixed_witness::step(start, pattern, kernel, requested, scanned, actual);
         STEPS.with_borrow_mut(|steps| {
             if let Some(steps) = steps {
                 steps.push(Step {
@@ -22238,3 +22243,101 @@
         );
     }
 }
+
+// [GPT-6 Astra] Diagnostic-only baseline witness. No hash cache implementation.
+#[cfg(test)]
+mod hash_reuse_fixed_witness {
+    use super::*;
+    use std::cell::RefCell;
+    thread_local! {
+        static EVENTS: RefCell<Option<Vec<serde_json::Value>>> = const { RefCell::new(None) };
+    }
+    fn enabled() -> bool { EVENTS.with_borrow(|e| e.is_some()) }
+    fn record(value: serde_json::Value) {
+        println!("HASH_WITNESS {}", value);
+        EVENTS.with_borrow_mut(|e| e.as_mut().unwrap().push(value));
+    }
+    pub(super) fn plan(
+        graph: &Graph, patterns: &[TriplePattern], prepared: &[Prepared],
+        filters: &[Option<(usize, ScanCmp)>], residual: usize, seed: usize,
+        seed_all: &Bindings,
+    ) {
+        if !enabled() { return; }
+        let names: Vec<_> = patterns.iter().map(ToString::to_string).collect();
+        let estimates: Vec<_> = prepared.iter().map(|p| p.est).collect();
+        let filter_names: Vec<_> = filters.iter().map(|f| f.map(|(c, x)| format!("{c}:{}", x.render()))).collect();
+        record(serde_json::json!({"kind":"plan","patterns":names,"estimates":estimates,
+            "filters":filter_names,"residual":residual,"seed":seed,"seed_rows":seed_all.rows.len(),
+            "seed_sort":seed_all.sorted_by.as_ref().map(|v|v.as_str()),"budget_active":budget::active(),
+            "binary":bgp_uses_binary(patterns),"view_empty":view::default_is_empty()}));
+        assert_eq!(patterns.len(), 2);
+        assert!(matches!(&patterns[0].predicate, NamedNodePattern::NamedNode(n) if n.as_str()=="http://ex/q"));
+        assert!(matches!(&patterns[1].predicate, NamedNodePattern::NamedNode(n) if n.as_str()=="http://ex/p"));
+        assert_eq!(estimates, [3136,2048]);
+        assert_eq!(seed, 1);
+        assert_eq!(seed_all.rows.len(), 2048);
+        assert_eq!(seed_all.sorted_by.as_ref().map(|v|v.as_str()), Some("s"));
+        assert!(matches!(filters[0],Some((2,ScanCmp::Num(NumCmp::Lt(x)))) if x==16.0));
+        assert!(filters[1].is_none());
+        assert_eq!(residual,0);
+        assert!(!budget::active());
+        assert!(bgp_uses_binary(patterns));
+        assert!(!view::default_is_empty());
+        assert!(!graph.has_high_precision_decimal());
+    }
+    pub(super) fn step(start:usize, pattern:usize, kernel:&str, requested:Option<usize>, scanned:bool, actual:Option<&Variable>) {
+        if !enabled() { return; }
+        record(serde_json::json!({"kind":"step","start":start,"pattern":pattern,"kernel":kernel,
+            "requested":requested,"scanned":scanned,"actual":actual.map(|v|v.as_str())}));
+    }
+    pub(super) fn built(left:&Bindings,right:&Bindings,build:&Bindings,probe:&Bindings,keys:&sjoin::JoinKeys,tables:&[sjoin::JoinTable]) {
+        if !enabled() { return; }
+        let rhs=std::ptr::eq(build,right);
+        let entries:usize=tables.iter().map(|t|t.len()).sum();
+        let postings:usize=tables.iter().flat_map(|t|t.values()).map(|p|p.len()).sum();
+        record(serde_json::json!({"kind":"built","rhs":rhs,"left_rows":left.rows.len(),
+            "right_rows":right.rows.len(),"build_rows":build.rows.len(),"probe_rows":probe.rows.len(),
+            "key_cols":keys.key_cols,"tables":tables.len(),"entries":entries,"postings":postings}));
+        assert!(rhs);
+        assert_eq!((left.rows.len(),right.rows.len(),build.rows.len(),probe.rows.len()),(1024,64,64,1024));
+        assert_eq!(keys.key_cols, [(0,0),(1,1)]);
+        assert_eq!((tables.len(),entries,postings),(1,64,64));
+    }
+    #[test]
+    #[cfg(all(not(target_arch="wasm32"),not(feature="algebra-rewrite"),not(feature="zk")))]
+    fn fixed_q3136_p2048() {
+        use sparq_core::store::{Perm,BUILT};
+        println!("HASH_WITNESS_CFG parallel={} BUILT={BUILT:?}",cfg!(feature="parallel"));
+        assert!(cfg!(feature="parallel"));
+        assert_eq!(BUILT,&[Perm::Spo,Perm::Sop,Perm::Pso,Perm::Pos,Perm::Osp,Perm::Ops]);
+        let mut ttl=String::from("@prefix : <http://ex/> .\n");
+        for i in 0..2048 { ttl.push_str(&format!(":s{i} :p {} .\n",i%16)); }
+        for i in 0..64 { ttl.push_str(&format!(":s{i} :q {} .\n",i%16)); }
+        for j in 0..3072 { ttl.push_str(&format!(":f{j} :q {} .\n",16+j%16)); }
+        let graph=Graph::load_str(&ttl,"turtle").unwrap();
+        EVENTS.with_borrow_mut(|e|*e=Some(Vec::new()));
+        let result=crate::query(&graph,"PREFIX : <http://ex/> SELECT ?o WHERE { ?s :q ?o . ?s :p ?o . FILTER(?o < 16) } LIMIT 65").unwrap();
+        let events=EVENTS.with_borrow_mut(|e|e.take().unwrap());
+        let plans:Vec<_>=events.iter().filter(|e|e["kind"]=="plan").collect();
+        let steps:Vec<_>=events.iter().filter(|e|e["kind"]=="step").collect();
+        let builds:Vec<_>=events.iter().filter(|e|e["kind"]=="built").collect();
+        assert_eq!((plans.len(),steps.len(),builds.len()),(1,2,2));
+        for (i,start) in [0usize,1024].into_iter().enumerate() {
+            assert_eq!(steps[i]["start"],serde_json::json!(start));
+            assert_eq!(steps[i]["pattern"],serde_json::json!(0));
+            assert_eq!(steps[i]["kernel"],serde_json::json!("hash"));
+            assert_eq!(steps[i]["requested"],serde_json::json!(2));
+            assert_eq!(steps[i]["actual"],serde_json::json!("o"));
+            assert_eq!(steps[i]["scanned"],serde_json::json!(i==0));
+        }
+        let mut bag=std::collections::BTreeMap::<String,usize>::new();
+        for row in &result.rows {
+            assert_eq!(row.len(),1);
+            *bag.entry(row[0].as_ref().unwrap().to_string()).or_default()+=1;
+        }
+        let expected:std::collections::BTreeMap<_,_>=(0..16).map(|i|(Literal::from(i).to_string(),4)).collect();
+        println!("HASH_WITNESS_RESULT {}",serde_json::json!({"rows":result.rows.len(),"bag":bag,"build_calls":builds.len(),"build_input_rows":128,"stored_postings":128,"scan_closures":1}));
+        assert_eq!(result.rows.len(),64);
+        assert_eq!(bag,expected);
+    }
+}
```

## Actual unmodified build callers/helpers

```rust
crates/sparq-engine/src/exec.rs
9071: fn hash_join_ref(left: &Bindings, right: &Bindings) -> Bindings {
9072:     // Build the hash table on the smaller side.
9073:     let (build, probe) = if left.rows.len() <= right.rows.len() {
9074:         (left, right)
9075:     } else {
9076:         (right, left)
9077:     };
9078:     // Shared vars relative to (build, probe).
9079:     let shared: Vec<(usize, usize)> = build
9080:         .vars
9081:         .iter()
9082:         .enumerate()
9083:         .filter_map(|(bi, v)| probe.col(v).map(|pi| (bi, pi)))
9084:         .collect();
9085:     let mut out_vars = build.vars.clone();
9086:     let probe_only: Vec<usize> = probe
9087:         .vars
9088:         .iter()
9089:         .enumerate()
9090:         .filter(|(_, v)| !build.vars.contains(v))
9091:         .map(|(i, v)| {
9092:             out_vars.push(v.clone());
9093:             i
9094:         })
9095:         .collect();
9096:     // The join column layout for the shared substrate kernel: `key_cols` are the (build, probe)
9097:     // shared-variable index pairs (the equi-join key); the probe-only columns are appended after
9098:     // the build row. The build/probe phases below are the substrate's `build_*` / `probe_emit`
9099:     // (sq-hknqs) — the engine only supplies this `Bindings`-derived layout and its budget.
9100:     let keys = sjoin::JoinKeys { key_cols: shared.clone(), right_only: Vec::new() };
9101:     // Build phase. Above PAR_THRESHOLD the build is radix-partitioned (Tier-1 #5 of
9102:     // research/parallelism-scaling.md): rows are tagged with their key-hash partition in
9103:     // parallel, then each partition builds its private map lock-free. Within a partition rows
9104:     // are scanned in ascending index, so each posting list stays in ascending build-row order —
9105:     // exactly the serial build — and the probe output is byte-identical.
9106:     // JoinTable = hashbrown::HashMap<Key, Posting, FxBuildHasher>; the type inference here
9107:     // avoids a dependency on rustc_hash::FxHashMap in the type annotation. [SONNET-4.6] sq-7d3dj.19
9108:     #[cfg(feature = "parallel")]
9109:     let tables = if build.rows.len() >= PAR_THRESHOLD {
9110:         use rayon::prelude::*;
9111:         let parts: Vec<u8> = build
9112:             .rows
9113:             .par_iter()
9114:             .map(|row| (key_hash(&keys.left_key(row)) % JOIN_PARTS as u64) as u8)
9115:             .collect();
9116:         sjoin::build_partitioned(&build.rows, &keys, &parts)
9117:     } else {
9118:         vec![sjoin::build_table(&build.rows, &keys)]
9119:     };
9120:     #[cfg(not(feature = "parallel"))]
9121:     let tables = vec![sjoin::build_table(&build.rows, &keys)];
9122:     // The probe is read-only over the (partitioned) table, so for a large probe side build the
9123:     // output in parallel on native.
9124:     #[cfg(feature = "parallel")]
9125:     if probe.rows.len() >= PAR_THRESHOLD {
9126:         use rayon::prelude::*;
9127:         // Budget snapshot for the workers (the installing thread's thread-local is
9128:         // invisible to them): a worker that hits the limits stops adding to its own
9129:         // accumulator; the caller's next on-thread check raises the actual error.
9130:         let snap = EngineSnapshot(budget::snapshot());
9131:         let rows: Vec<Row> = probe
9132:             .rows
9133:             .par_iter()
9134:             .fold(Vec::new, |mut acc, prow| {
9135:                 if !sjoin::BudgetSnapshot::hit(&snap, acc.len()) {
9136:                     sjoin::probe_emit(prow, &keys, &build.rows, &tables, &probe_only, &mut acc);
9137:                 }
9138:                 acc
9139:             })
9140:             .reduce(Vec::new, |mut a, mut b| {
9141:                 a.append(&mut b);
9142:                 a
9143:             });
9144:         let _ = budget::exhausted(rows.len()); // sticky gate on the combined size
9145:         return Bindings::unsorted(out_vars, rows);
9146:     }
9147:     let mut rows = Vec::new();
9148:     sjoin::hash_probe_serial(&probe.rows, &keys, &build.rows, &tables, &probe_only, &EngineBudget, &mut rows);
9149:     Bindings::unsorted(out_vars, rows)
9150: }
9151: 
crates/sparq-substrate/src/join.rs
283:             }
284:         }
285:     }
286: }
287: 
288: /// Builds the serial hash table for a hash join: maps each build-side key to the
289: /// ascending list of build-row indices sharing it. Returns a [`JoinTable`]
290: /// (`hashbrown::HashMap<Key, Posting, FxBuildHasher>`) so that the probe path can
291: /// call `raw_entry().from_hash` with the precomputed [`key_hash`] value.
292: #[inline]
293: pub fn build_table(build: &[Row], keys: &JoinKeys) -> JoinTable {
294:     let mut t = JoinTable::default();
295:     for (ri, row) in build.iter().enumerate() {
296:         t.entry(keys.left_key(row)).or_default().push(ri);
297:     }
298:     t
299: }
300: 
301: /// Builds the radix-partitioned hash tables for a parallel hash join: `JOIN_PARTS`
302: /// private maps, each over the build rows whose key-hash falls in that partition.
303: /// Within a partition rows are scanned in ascending index, so each posting list
304: /// stays in ascending build-row order — exactly the serial build — and the probe
305: /// output is byte-identical. Requires the `parallel` feature (the inner rayon
306: /// import is the caller's; this returns the per-partition maps). Returns
307: /// `Vec<`[`JoinTable`]`>` so the probe path can use `raw_entry().from_hash`.
308: #[inline]
309: pub fn build_partitioned(build: &[Row], keys: &JoinKeys, parts: &[u8]) -> Vec<JoinTable> {
310:     (0..JOIN_PARTS)
311:         .map(|p| {
312:             let mut t = JoinTable::default();
313:             for (ri, row) in build.iter().enumerate() {
314:                 if parts[ri] as usize == p {
315:                     t.entry(keys.left_key(row)).or_default().push(ri);
316:                 }
317:             }
318:             t
319:         })
320:         .collect()
321: }
```

Full actual-engine module copy, raw argv/compiler/environment/extern provenance, emitted dep-info, original bytes via hashes, rawtest observations, binary, resource samples and terminal receipts are preserved separately. This small packet omits unrelated engine bodies and host-specific raw logs.
