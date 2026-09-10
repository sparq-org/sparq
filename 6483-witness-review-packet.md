# Issue6483 fixed comparator witness

```json
{
  "at": "2026-09-10T18:10:05.932957+00:00",
  "author": "GPT-6 Astra xhigh",
  "status": "DIAGNOSTIC_COMPLETE",
  "source_ref": "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464",
  "source_sha256": "41d8a1e664a3620dc23ec530af9c1908123c43f98131a61f1c9ca2b555d33eaa",
  "production_prefix_unchanged": true,
  "worktree_head": "9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed",
  "clean": true,
  "findings": {
    "distinct_lexical_allow_false": "Differs with original datasets",
    "distinct_lexical_allow_true": "Differs: integer normalization merges or duplicates rows; original datasets retained",
    "identical_lexical_allow_false": "Same",
    "identical_lexical_allow_true": "Same"
  },
  "causality": "Both sides contain3rawquads, but per-quad multiplicities are[p,p,q]vs[p,q,q]. With identical lexical forms, blank-node canonicalization yields equal sets, raw total lengths match, and strictSame returns before lexical adjudication. This prologue/comparable behavior predates5183 repair, byte-equivalent to459.",
  "contract_scope": "The actual production consumers supply complete N-Quads datasets or bound rows from a finite full(s,p,o[,g]) probe inventory; this is a raw duplicate-output detection gap in the comparator. RDF datasets themselves use set semantics. Arbitrary SPARQL projection/join/UNION bags can legitimately contain duplicates; no universal bag policy or engine result failure is established.",
  "validation": {
    "builds": 1,
    "filtered_processes": 1,
    "test_summary": "1passed;20filteredout",
    "comparison_records": 4,
    "phase_seconds": 13.424612791000001,
    "compile_seconds": 12.5258985,
    "test_process_seconds": 0.7356973339999993,
    "all_commands_exit0": true,
    "minimum_observed_free_bytes": 14817988608,
    "peak_observed_allocated_bytes": 14270464
  },
  "limits": {
    "aggregate_seconds": 90,
    "build_seconds": 60,
    "test_seconds": 20,
    "new_allocated_bytes": 67108864,
    "start_free_bytes": 2214592512,
    "continuous_free_bytes": 2147483648
  },
  "provenance": "Rust1.97.1/2021/O3/codegen16/strip recorded compiler; six direct dependency hashes rechecked, same feature-qualified libraries as completed5183 module tests. No Cargo or dependency build. Faithful manifest-relative allowlist copied byte-for-byte fromf50; compare modes are explicit and do not depend on ambient allowlist.",
  "binary_sha256": "2f3cbf7a61470d21d34c9df82616ff57560353b7d0e91f0c3d52341d9b98173e",
  "preparation_error": "One controller-writing tool input had a Python quoting SyntaxError before any directory/source/process. Preserved receipt; corrected controller preparation preceded the sole build/test. No failed behavioral outcome was retried.",
  "limits_of_evidence": [
    "No seed replay, fullsuite rerun, publicengine bug, productionfix or timingbenefit claim.",
    "Only one synthetic blank-node duplicate-redistribution structure and lexical control, two allow modes.",
    "Current futurecorrection should be scoped to completequad/raw-output contract; retain isomorphism and error checks."
  ],
  "next_smallest_step": "Root may update existing6483 with this concrete witness, then obtain a bounded source design for rejecting rawduplicate emissions or preserving multiplicity underblank-node mapping. No correction selected or implemented here.",
  "pending_commands": 0,
  "production_edits": 0,
  "remote_mutations": 0
}
```

## Raw observations

```text

running 1 test
test update_fuzz::copilot_duplicate_probe::distinct_control_and_identical_lexical_observation ... duplicate_inputs {"distinct":["_:x <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"identical":["_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"left":["_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .","_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> ."]}
duplicate_witness {"allow_integer_lexical":false,"case":"distinct_lexical_control","detail":"only in left:\n  _:c14n0 <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .\nonly in right:\n  _:c14n0 <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n","verdict":"Differs"}
duplicate_witness {"allow_integer_lexical":false,"case":"identical_lexical_redistribution","detail":null,"verdict":"Same"}
duplicate_witness {"allow_integer_lexical":true,"case":"distinct_lexical_control","detail":"integer-lexical adjudication refused: left: integer normalization merges or duplicates rows\nonly in left:\n  _:c14n0 <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .\nonly in right:\n  _:c14n0 <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n","verdict":"Differs"}
duplicate_witness {"allow_integer_lexical":true,"case":"identical_lexical_redistribution","detail":null,"verdict":"Same"}
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s


```

## Diagnostic-only delta

```diff
--- production/update_fuzz.rs
+++ witness/update_fuzz.rs
@@ -2331,3 +2331,37 @@
         });
     }
 }
+
+// [GPT-6 Astra] Isolated Copilot witness; observes the strict identical-lexical path.
+#[cfg(test)]
+mod copilot_duplicate_probe {
+    use super::*;
+
+    fn observation(case: &str, allow: bool, verdict: &Verdict) {
+        let (name, detail) = match verdict {
+            Verdict::Same => ("Same", None),
+            Verdict::AdjudicatedIntegerLexical => ("AdjudicatedIntegerLexical", None),
+            Verdict::Differs(detail) => ("Differs", Some(detail.as_str())),
+        };
+        println!("duplicate_witness {}", serde_json::json!({
+            "case": case, "allow_integer_lexical": allow, "verdict": name, "detail": detail
+        }));
+    }
+
+    #[test]
+    fn distinct_control_and_identical_lexical_observation() {
+        let p = format!("_:x <http://ex/p> \"020\"^^{XSD_INTEGER} .");
+        let q = format!("_:x <http://ex/q> \"021\"^^{XSD_INTEGER} .");
+        let left = vec![p.clone(), p.clone(), q.clone()];
+        let distinct = vec![p.replace("020", "20"), q.replace("021", "21"), q.replace("021", "21")];
+        let identical = vec![p, q.clone(), q];
+        println!("duplicate_inputs {}", serde_json::json!({"left": left, "distinct": distinct, "identical": identical}));
+        for allow in [false, true] {
+            let control = compare("left", &left, "right", &distinct, allow);
+            observation("distinct_lexical_control", allow, &control);
+            assert!(matches!(control, Verdict::Differs(_)), "current differing-lexical control must refuse");
+            let observed = compare("left", &left, "right", &identical, allow);
+            observation("identical_lexical_redistribution", allow, &observed);
+        }
+    }
+}

```

## Unchanged strict comparator seam

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

## Actual consumer scope

```rust
crates/sparq-bench/src/update_fuzz.rs:716
716: fn sparq_nquads(g: &Graph) -> Vec<String> {
717:     fn triples_of(g: &Graph, graph: Option<&str>, out: &mut Vec<String>) {
718:         let scan = g.store.scan(&[None, None, None]);
719:         for r in scan.rows.iter() {
720:             let t = scan.to_spo(r);
721:             out.push(nquads_line(
722:                 &g.dict.term(t[0]).to_string(),
723:                 &g.dict.term(t[1]).to_string(),
724:                 &g.dict.term(t[2]).to_string(),
725:                 graph,
726:             ));
727:         }
728:     }
729:     let mut out = Vec::new();
730:     triples_of(g, None, &mut out);
731:     for (name, sub) in &g.named {
732:         triples_of(sub, Some(&name.to_string()), &mut out);
733:     }
734:     out.sort();
735:     out
736: }
737: 
738: /// The Oxigraph store as SORTED N-Quads lines, rendered identically.
739: fn oxi_nquads(store: &Store) -> Result<Vec<String>, String> {
740:     use oxigraph::model::GraphName;
741:     let mut out = Vec::new();
742:     for q in store.iter() {
743:         let q = q.map_err(|e| format!("oxigraph iter error: {}", e))?;
744:         let graph = match &q.graph_name {
745:             GraphName::DefaultGraph => None,
746:             g => Some(g.to_string()),
747:         };
748:         out.push(nquads_line(
749:             &q.subject.to_string(),
750:             &q.predicate.to_string(),
751:             &q.object.to_string(),
752:             graph.as_deref(),
753:         ));

crates/sparq-bench/src/update_fuzz.rs:1030
1030: const PROBES: &[(&str, &[&str])] = &[
1031:     ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
1032:     (
1033:         "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
1034:         &["s", "p", "o", "g"],
1035:     ),
1036: ];
1037: 
1038: /// Renders one probe row's cells as an N-Quads line.
1039: fn probe_line(cells: &[String]) -> String {
1040:     format!("{} .", cells.join(" "))
1041: }
1042: 
1043: /// The full sorted binding set of a probe through sparq's query path.
1044: fn sparq_probe(g: &Graph, q: &str) -> Result<Vec<String>, String> {
1045:     let r = sparq_engine::query(g, q).map_err(|e| format!("sparq probe error: {}", e))?;
1046:     let mut rows: Vec<String> = r
1047:         .rows
1048:         .iter()
1049:         .map(|row| {
1050:             probe_line(
1051:                 &row.iter()
1052:                     .map(|t| {
1053:                         t.as_ref()
1054:                             .map(|t| t.to_string())
1055:                             .unwrap_or_else(|| "UNDEF".to_string())
1056:                     })
1057:                     .collect::<Vec<_>>(),
1058:             )
1059:         })
1060:         .collect();
1061:     rows.sort();
1062:     Ok(rows)
1063: }
1064: 
1065: /// The full sorted binding set of a probe through Oxigraph's query path.
1066: // clippy: the differential oracle pins oxigraph's legacy Store::query semantics
1067: #[allow(deprecated)]
1068: fn oxi_probe(store: &Store, q: &str, vars: &[&str]) -> Result<Vec<String>, String> {
1069:     match store.query(q).map_err(|e| e.to_string())? {
1070:         oxigraph::sparql::QueryResults::Solutions(s) => {
1071:             let mut rows = Vec::new();
1072:             for sol in s {
1073:                 let sol = sol.map_err(|e| e.to_string())?;
1074:                 rows.push(probe_line(
1075:                     &vars
1076:                         .iter()
1077:                         .map(|v| {
1078:                             sol.get(*v)
1079:                                 .map(|t| t.to_string())
1080:                                 .unwrap_or_else(|| "UNDEF".to_string())
1081:                         })
1082:                         .collect::<Vec<_>>(),
1083:                 ));
1084:             }
1085:             rows.sort();
1086:             Ok(rows)
1087:         }
1088:         _ => Err("probe did not return solutions".to_string()),
1089:     }
1090: }
1091: 
1092: // ── the LOAD document sandbox ────────────────────────────────────────────────────
1093: 
1094: /// A temporary directory holding the documents a seed's `LOAD` operations read, and
1095: /// which `sparq_engine::with_load_base` allowlists for the duration of that seed.
1096: ///
1097: /// Uniquified by process id AND a run-local counter so concurrently-running seeds
1098: /// (cargo test runs these in parallel threads) never share a document. The path never
1099: /// appears in a generated request — the generated IRI is relative — so the seed repro
1100: /// is unaffected by it.
1101: struct LoadSandbox(PathBuf);
1102: 
1103: impl LoadSandbox {
1104:     fn new() -> Result<LoadSandbox, String> {
1105:         use std::sync::atomic::{AtomicU64, Ordering};
1106:         static NEXT: AtomicU64 = AtomicU64::new(0);
1107:         let dir = std::env::temp_dir().join(format!(
1108:             "sparq-update-fuzz-{}-{}",
1109:             std::process::id(),
1110:             NEXT.fetch_add(1, Ordering::Relaxed)
1111:         ));
1112:         std::fs::create_dir_all(&dir)
1113:             .map_err(|e| format!("LOAD sandbox {}: {}", dir.display(), e))?;
1114:         Ok(LoadSandbox(dir))
1115:     }
1116: 
1117:     fn path(&self) -> &Path {
1118:         &self.0

crates/sparq-bench/src/update_fuzz.rs:1250
1250:         let nq_rebuild = sparq_nquads(&g_rebuild);
1251:         let nq_inplace = sparq_nquads(&g_inplace);
1252:         let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
1253:         if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
1254:             isomorphism_compares += 1;
1255:         }
1256:         if let Verdict::Differs(detail) = compare(
1257:             "sparq(rebuild)",
1258:             &nq_rebuild,
1259:             "sparq(in-place)",
1260:             &nq_inplace,
1261:             false,
1262:         ) {
1263:             return Err(fail(
1264:                 i,
1265:                 op,
1266:                 format!(
1267:                     "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
1268:                     detail
1269:                 ),
1270:             ));
1271:         }
1272:         for (label, nq) in [
1273:             ("sparq(rebuild)", &nq_rebuild),
1274:             ("sparq(in-place)", &nq_inplace),
1275:         ] {
1276:             match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
1277:                 Verdict::Same => {}
1278:                 Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1279:                 Verdict::Differs(detail) => {
1280:                     return Err(fail(
1281:                         i,
1282:                         op,
1283:                         format!(
1284:                             "canonical dataset differs ({} vs oxigraph)\n{}",
1285:                             label, detail
1286:                         ),
1287:                     ));
1288:                 }
1289:             }
1290:         }
1291: 
1292:         // (b) Probe SELECTs — the query-path view of the updated store, full sorted
1293:         // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
1294:         // one reads through the live delta overlay.
1295:         for (probe, vars) in PROBES {
1296:             let oxi = oxi_probe(&store, probe, vars)
1297:                 .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
1298:             for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
1299:                 let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
1300:                 match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
1301:                     Verdict::Same => {}
1302:                     Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1303:                     Verdict::Differs(detail) => {
1304:                         return Err(fail(
1305:                             i,
1306:                             op,
1307:                             format!(
1308:                                 "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
1309:                                 probe, label, detail
1310:                             ),
1311:                         ));
1312:                     }
1313:                 }
1314:             }
1315:         }

```
