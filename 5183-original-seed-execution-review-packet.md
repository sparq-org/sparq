# Issue 5183 — fixed original-seed diagnostic evidence

No patch is proposed in this bundle. Repository-relative source and synthetic observations only.

{
  "author": "GPT-6 Astra xhigh",
  "status": "COMPLETED_OBSERVED_MISMATCH_BOTH_REVISIONS",
  "source_revisions": {
    "parent": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
    "main": "781f667c19a8ebb779cfccb24b05ea432360b025"
  },
  "observations": [
    "Both revisions generated exactly the ten CI operations, with identical first-failure detail including the extra integer lexical 008 and c14n8 blank node.",
    "Failure is at zero-based index 8: nine generated requests were applied, then the reference dataset comparison returned early. The printed index-9 ADD request was generated but not executed.",
    "The same witness on d41, before PR6478, excludes PR6478 as the introduction of this first-seed mismatch under the qualified diagnostic configuration.",
    "Source ordering establishes that the strict rebuild-vs-in-place dataset comparison passed before the failing rebuild-vs-Oxigraph comparison, including at index 8. Later index-8 reference/probe checks and index-9 checks were not reached.",
    "The startup line confirms the committed integer-lexical adjudication was enabled. Printed counts are accumulated only for completely successful seeds, so their zero values do not imply zero work or absence of earlier adjudication.",
    "The diagnostic prints the original canonical difference after normalization fails to remove residual differences; it is not a raw post-normalization diff."
  ],
  "interpretation": {
    "established": "The original first-seed failure is reproducible before and after PR6478 with unchanged generator, comparator, parser and LOAD sandbox. Existing reduced public-API results independently show Sparq retains two distinct integer lexical terms/WHERE rows/fresh blank nodes while pinned Oxigraph collapses the two terms into one before query evaluation.",
    "supported_explanation": "At sequence index 4 the lexical pair is inserted; index 8 allocates a fresh template blank node per WHERE solution. Post-hoc lexical normalization can merge literals, but cannot undo the additional blank node already allocated from the larger solution multiset.",
    "limits": "This explanation combines the unchanged sequence/source with prior reduced witnesses; this run did not instrument every intermediate WHERE row. Only the first of eight CI mismatches was executed. No generic comparator change or engine repair is validated."
  },
  "features": {
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
    "oxttl@0.1.8": [
      "default"
    ],
    "oxttl@0.2.3": [
      "default",
      "rdf-12"
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
  },
  "environment_limits": {
    "actual": "macOS aarch64 Rust1.97.1, O3, unwind, no LTO, codegen16, one Rayon thread, jobs1, incremental0, offline; actual rustc invocations and linked artifact hashes saved.",
    "ci_difference": "The actual Linux UPDATE job builds sparq-bench with release-fast (thin LTO/abort). This minimal unchanged-module diagnostic is not the full CLI build or exact Linux environment. Compiled mmap/dict-spill features do not assert a disk-backed runtime graph.",
    "performance": "No query timing, throughput, heap or peak-RSS claim. Command duration includes polling."
  },
  "resources": {
    "phase_max_new_allocated_bytes": 300384256,
    "phase_min_free_bytes": 5318758400,
    "new_limit_bytes": 536870912,
    "minimum_free_limit_bytes": 2147483648,
    "aggregate_limit_seconds": 1200,
    "per_build_limit_seconds": 600,
    "per_seed_limit_seconds": 60,
    "within_phase_limits": true
  },
  "next_review_proposal": "Obtain the planned independent soundness/diagnosis review of this first-seed reproduction together with the two reduced public-API witnesses. Review a narrowly specified way to recognize or avoid reference-state lexical collapse before stateful blank-node allocation while retaining strict Sparq rebuild/in-place comparisons, full multiset probes, unknown-divergence failures and calibrated unrelated-defect controls. Do not normalize away arbitrary blank nodes, skip the seed, weaken canonical comparisons, or presume the remaining seven failures share this cause. No implementation is supplied or approved here."
}

## Exact fixed sequence
```sparql
[0] INSERT DATA { GRAPH <http://ex/g0> { <http://ex/s4> <http://ex/p0> "lit2" . } <http://ex/s2> <http://ex/p3> <http://ex/o5> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s0> <http://ex/p2> "lit1" . <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . }
[1] INSERT DATA { <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . <http://ex/s1> <http://ex/p3> 6 . GRAPH <http://ex/g2> { <http://ex/s3> <http://ex/p0> 1 . } GRAPH <http://ex/g1> { <http://ex/s5> <http://ex/p0> "lit1" . } <http://ex/s2> <http://ex/p0> "lit1" . } ;
DELETE DATA { <http://ex/s2> <http://ex/p0> <http://ex/o2> . <http://ex/s4> <http://ex/p1> 18 . <http://ex/s0> <http://ex/p1> 8 . }
[2] DELETE DATA { <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . }
[3] INSERT { ?s <http://ex/p1> <<( ?s ?p ?o )>> } WHERE { ?s ?p ?o FILTER(!isBlank(?s) && !isBlank(?o)) }
[4] INSERT DATA { <http://ex/s3> <http://ex/p1> "lit1" . <http://ex/s5> <http://ex/p2> <http://ex/o4> . <http://ex/s2> <http://ex/p1> 8 . <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> . }
[5] LOAD SILENT <file://doc1.nt>
     (reference engine ran: INSERT DATA { <http://ex/s5> <http://ex/p1> <http://ex/o3> . <http://ex/s0> <http://ex/p0> <http://ex/o2> . <http://ex/s4> <http://ex/p1> <http://ex/o5> . })
[6] DELETE DATA { <http://ex/s0> <http://ex/p2> "lit1" . }
[7] CREATE SILENT GRAPH <http://ex/g0>
[8] INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }
[9] ADD SILENT GRAPH <http://ex/g0> TO GRAPH <http://ex/g2>
```

## Exact first failure (both revisions and CI)
```text
seed=4141222487
step=8 of 10
op: INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n8 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
only in oxigraph:
```

## Unchanged load-bearing source

### crates/sparq-bench/src/update_fuzz.rs:85–128
```rust
85: //! and the code says so: Oxigraph 0.5's `eval_load` is HTTP-only and, without the
86: //! `http-client` feature this harness deliberately does not enable, returns
87: //! *"HTTP client is not available"* for ANY source; sparq's `load_document` is the exact
88: //! mirror image (`file://` only, and only under an allowlisted base). The two engines'
89: //! LOAD source surfaces are DISJOINT, so a same-request LOAD differential is impossible.
90: //! `tests::oxigraph_cannot_load_a_local_file` pins that reason so it is machine-checked
91: //! rather than merely asserted here.
92: //!
93: //! What the harness does instead: sparq's two paths get the real
94: //! `LOAD <file://doc.nt> [INTO GRAPH g]`, and the reference engine gets the
95: //! semantically-equivalent `INSERT DATA` of the SAME ground triples. The oracle is
96: //! therefore "sparq's LOAD produces exactly the dataset the reference engine reaches by
97: //! inserting that document's contents", plus a true two-implementation differential
98: //! between sparq's rebuild and delta-overlay LOAD paths. The document is written under a
99: //! per-run temporary directory allowlisted via `sparq_engine::with_load_base`, and the
100: //! generated IRI is RELATIVE (`<file://doc0.nt>`) so the request text — and hence the
101: //! seed repro — is independent of that directory's absolute path.
102: //!
103: //! ## Adjudicated divergences
104: //!
105: //! The mechanism mirrors `fuzz.rs`: the comparator consults
106: //! `bench/differential-divergences.json` for classes whose id starts with `update-`. A
107: //! listed class without a detector in this file stays STRICT, with a loud warning —
108: //! never a silent skip.
109: //!
110: //! One class is adjudicated today,
111: //! `update-oxigraph-integer-lexical-canonicalization`: Oxigraph's storage layer parses
112: //! every `xsd:integer` into a native integer and re-renders it canonically
113: //! (`storage/numeric_encoder.rs`), so `"05"^^xsd:integer` and `"5"^^xsd:integer` become
114: //! the SAME stored term. RDF 1.1 Concepts §3.3 makes them different terms (literal
115: //! equality is lexical-form equality), so sparq is spec-correct and Oxigraph is lossy.
116: //! It is NOT a blind skip: the comparator re-derives Oxigraph's normalization
117: //! independently — rewriting every `xsd:integer` literal on BOTH sides to its canonical
118: //! lexical form, then deduplicating — and absorbs the step ONLY when the two datasets
119: //! agree exactly under it. Any residual difference still FAILS. The sparq-vs-sparq
120: //! compare never consults the allowlist: both sides are sparq, so any lexical
121: //! disagreement between them is a real bug.
122: //!
123: //! To keep that class narrow, non-canonical lexicals are generated only in INSERT
124: //! positions. An exact-term `DELETE DATA` of one (where sparq removes `"05"` and leaves
125: //! `"5"` while Oxigraph, holding one merged term, removes both) is a cascade no
126: //! normalization can undo; that shape is covered instead by the sparq-internal
127: //! `tests::non_canonical_integer_lexicals_are_distinct_terms`, which is the correct
128: //! oracle given Oxigraph cannot serve as a reference for it.
```

### crates/sparq-bench/src/update_fuzz.rs:809–907
```rust
809: /// here independently of BOTH engines; see the adjudicated class in the module docs.
810: fn oxigraph_normalized(lines: &[String], what: &str) -> Result<Vec<String>, String> {
811:     let quads = parse_lines(lines, what)?;
812:     let mut out: Vec<String> = quads
813:         .iter()
814:         .map(|q| {
815:             let graph = match &q.graph_name {
816:                 oxrdf::GraphName::DefaultGraph => None,
817:                 g => Some(g.to_string()),
818:             };
819:             nquads_line(
820:                 &q.subject.to_string(),
821:                 &q.predicate.to_string(),
822:                 &oxigraph_normalized_term(&q.object).to_string(),
823:                 graph.as_deref(),
824:             )
825:         })
826:         .collect();
827:     out.sort();
828:     out.dedup();
829:     Ok(out)
830: }
831: 
832: /// The lines on exactly one side — the human-readable core of a divergence report.
833: fn one_sided(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
834:     let bset: std::collections::BTreeSet<&String> = b.iter().collect();
835:     let aset: std::collections::BTreeSet<&String> = a.iter().collect();
836:     let mut s = String::new();
837:     s.push_str(&format!("only in {}:\n", label_a));
838:     for l in a.iter().filter(|l| !bset.contains(l)) {
839:         s.push_str(&format!("  {}\n", l));
840:     }
841:     s.push_str(&format!("only in {}:\n", label_b));
842:     for l in b.iter().filter(|l| !aset.contains(l)) {
843:         s.push_str(&format!("  {}\n", l));
844:     }
845:     s
846: }
847: 
848: /// The outcome of comparing two snapshots.
849: enum Verdict {
850:     /// Byte-identical, or RDF-isomorphic when blank nodes are in play.
851:     Same,
852:     /// Absorbed by the adjudicated `update-oxigraph-integer-lexical-canonicalization`
853:     /// class: the two datasets agree exactly once Oxigraph's numeric normalization is
854:     /// re-derived on both sides.
855:     AdjudicatedIntegerLexical,
856:     /// A real divergence (the string is the report body).
857:     Differs(String),
858: }
859: 
860: /// Compares two snapshots under the canonical form the module docs describe.
861: ///
862: /// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
863: /// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
864: /// sides are sparq, so any lexical disagreement between them is a real bug.
865: fn compare(
866:     label_a: &str,
867:     a: &[String],
868:     label_b: &str,
869:     b: &[String],
870:     allow_integer_lexical: bool,
871: ) -> Verdict {
872:     let relabel = mentions_blank_node(a) || mentions_blank_node(b);
873:     let (ca, cb) = match (
874:         comparable(a, relabel, label_a),
875:         comparable(b, relabel, label_b),
876:     ) {
877:         (Ok(ca), Ok(cb)) => (ca, cb),
878:         (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
879:     };
880:     if ca == cb {
881:         // Canonicalization deduplicates, so a duplicate quad on one side alone would
882:         // survive the compare — check the raw counts to keep that failure visible.
883:         if a.len() != b.len() {
884:             return Verdict::Differs(format!(
885:                 "datasets are isomorphic but the raw quad COUNTS differ \
886:                  ({} {} vs {} {}) — one side is yielding a duplicate quad",
887:                 label_a,
888:                 a.len(),
889:                 label_b,
890:                 b.len()
891:             ));
892:         }
893:         return Verdict::Same;
894:     }
895:     if allow_integer_lexical {
896:         let normalized = (
897:             oxigraph_normalized(a, label_a).and_then(|n| comparable(&n, relabel, label_a)),
898:             oxigraph_normalized(b, label_b).and_then(|n| comparable(&n, relabel, label_b)),
899:         );
900:         if let (Ok(na), Ok(nb)) = normalized {
901:             if na == nb {
902:                 return Verdict::AdjudicatedIntegerLexical;
903:             }
904:         }
905:     }
906:     Verdict::Differs(one_sided(label_a, &ca, label_b, &cb))
907: }
```

### crates/sparq-bench/src/update_fuzz.rs:1040–1205
```rust
1040: /// step `i` to all three engines, a marker quad is inserted into the OXIGRAPH store
1041: /// only, so the comparator MUST report a divergence at that step (see
1042: /// `tests::injected_divergence_is_caught`). `None` in production.
1043: fn check_seed(
1044:     seed: u64,
1045:     inject_divergence_at: Option<usize>,
1046:     allow: &UpdateDivergenceAllowlist,
1047: ) -> Result<SeedOutcome, String> {
1048:     let mut rng = Rng::new(seed);
1049:     let ops = gen_sequence(&mut rng);
1050:     let sandbox = if ops.iter().any(|o| o.sparq.starts_with("LOAD")) {
1051:         Some(LoadSandbox::new()?)
1052:     } else {
1053:         None
1054:     };
1055:     match sandbox.as_ref() {
1056:         // The allowlisted base is installed for the whole seed: `with_load_base` is a
1057:         // thread-local guard, and a seed's steps all run on this thread.
1058:         Some(s) => sparq_engine::with_load_base(s.path(), || {
1059:             apply_sequence(seed, &ops, Some(s), inject_divergence_at, allow)
1060:         }),
1061:         None => apply_sequence(seed, &ops, None, inject_divergence_at, allow),
1062:     }
1063: }
1064: 
1065: fn apply_sequence(
1066:     seed: u64,
1067:     ops: &[Op],
1068:     sandbox: Option<&LoadSandbox>,
1069:     inject_divergence_at: Option<usize>,
1070:     allow: &UpdateDivergenceAllowlist,
1071: ) -> Result<SeedOutcome, String> {
1072:     let mut g_rebuild = Graph::new();
1073:     let mut g_inplace = Graph::new();
1074:     let store = Store::new().map_err(|e| format!("oxigraph store init: {}", e))?;
1075:     let mut adjudicated_integer_lexical = 0u64;
1076:     let mut isomorphism_compares = 0u64;
1077: 
1078:     let fail = |step: usize, op: &Op, detail: String| -> String {
1079:         format!(
1080:             "step={} of {}\nop: {}\n{}\nrepro: cargo run -p sparq-bench --release -- \
1081:              update-fuzz --seed-start {} --seed-count 1\n--- full sequence ---\n{}",
1082:             step,
1083:             ops.len(),
1084:             op.sparq,
1085:             detail,
1086:             seed,
1087:             ops.iter()
1088:                 .enumerate()
1089:                 .map(|(i, o)| match &o.oxi {
1090:                     Some(x) if *x != o.sparq =>
1091:                         format!("[{}] {}\n     (reference engine ran: {})", i, o.sparq, x),
1092:                     Some(_) => format!("[{}] {}", i, o.sparq),
1093:                     None => format!("[{}] {}\n     (reference engine ran: nothing)", i, o.sparq),
1094:                 })
1095:                 .collect::<Vec<_>>()
1096:                 .join("\n")
1097:         )
1098:     };
1099: 
1100:     for (i, op) in ops.iter().enumerate() {
1101:         // A LOAD's document must exist before the request runs.
1102:         if let (Some(doc), Some(s)) = (&op.doc, sandbox) {
1103:             s.write(doc).map_err(|e| fail(i, op, e))?;
1104:         }
1105: 
1106:         // Apply to all three implementations. Every generated op is inside the
1107:         // supported deterministic subset, so an error from ANY engine is itself a
1108:         // divergence (strict — there is no unsupported-skip in this harness).
1109:         g_rebuild = sparq_engine::update(&g_rebuild, &op.sparq)
1110:             .map_err(|e| fail(i, op, format!("sparq update (rebuild path) error: {}", e)))?;
1111:         sparq_engine::update_in_place(&mut g_inplace, &op.sparq)
1112:             .map_err(|e| fail(i, op, format!("sparq update_in_place error: {}", e)))?;
1113:         if let Some(oxi) = &op.oxi {
1114:             store
1115:                 .update(oxi.as_str())
1116:                 .map_err(|e| fail(i, op, format!("oxigraph update error: {}", e)))?;
1117:         }
1118: 
1119:         if inject_divergence_at == Some(i) {
1120:             use oxigraph::model::{GraphName, NamedNode, Quad};
1121:             let n = |s: &str| NamedNode::new(s).expect("valid IRI");
1122:             let marker = Quad::new(
1123:                 n("http://ex/injected"),
1124:                 n("http://ex/injected"),
1125:                 n("http://ex/injected"),
1126:                 GraphName::DefaultGraph,
1127:             );
1128:             store
1129:                 .insert(&marker)
1130:                 .map_err(|e| format!("marker insert failed: {}", e))?;
1131:         }
1132: 
1133:         // (a) Canonical dataset equality. Localizes a divergence to this exact step.
1134:         // sparq-vs-sparq FIRST and STRICT (no adjudication): the two sparq paths must
1135:         // agree with each other whatever the reference engine does.
1136:         let nq_rebuild = sparq_nquads(&g_rebuild);
1137:         let nq_inplace = sparq_nquads(&g_inplace);
1138:         let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
1139:         if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
1140:             isomorphism_compares += 1;
1141:         }
1142:         if let Verdict::Differs(detail) = compare(
1143:             "sparq(rebuild)",
1144:             &nq_rebuild,
1145:             "sparq(in-place)",
1146:             &nq_inplace,
1147:             false,
1148:         ) {
1149:             return Err(fail(
1150:                 i,
1151:                 op,
1152:                 format!(
1153:                     "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
1154:                     detail
1155:                 ),
1156:             ));
1157:         }
1158:         for (label, nq) in [
1159:             ("sparq(rebuild)", &nq_rebuild),
1160:             ("sparq(in-place)", &nq_inplace),
1161:         ] {
1162:             match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
1163:                 Verdict::Same => {}
1164:                 Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1165:                 Verdict::Differs(detail) => {
1166:                     return Err(fail(
1167:                         i,
1168:                         op,
1169:                         format!("canonical dataset differs ({} vs oxigraph)\n{}", label, detail),
1170:                     ))
1171:                 }
1172:             }
1173:         }
1174: 
1175:         // (b) Probe SELECTs — the query-path view of the updated store, full sorted
1176:         // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
1177:         // one reads through the live delta overlay.
1178:         for (probe, vars) in PROBES {
1179:             let oxi = oxi_probe(&store, probe, vars)
1180:                 .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
1181:             for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
1182:                 let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
1183:                 match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
1184:                     Verdict::Same => {}
1185:                     Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1186:                     Verdict::Differs(detail) => {
1187:                         return Err(fail(
1188:                             i,
1189:                             op,
1190:                             format!(
1191:                                 "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
1192:                                 probe, label, detail
1193:                             ),
1194:                         ))
1195:                     }
1196:                 }
1197:             }
1198:         }
1199:     }
1200:     Ok(SeedOutcome {
1201:         ops: ops.len() as u64,
1202:         adjudicated_integer_lexical,
1203:         isomorphism_compares,
1204:     })
1205: }
```

### crates/sparq-bench/src/update_fuzz.rs:1233–1293
```rust
1233:     /// Load from `SPARQ_FUZZ_DIVERGENCES` (a CI/agent override), else the committed
1234:     /// repo default resolved relative to this crate's manifest (works from any cwd).
1235:     fn load() -> Self {
1236:         let path = std::env::var("SPARQ_FUZZ_DIVERGENCES").unwrap_or_else(|_| {
1237:             concat!(
1238:                 env!("CARGO_MANIFEST_DIR"),
1239:                 "/../../bench/differential-divergences.json"
1240:             )
1241:             .to_string()
1242:         });
1243:         match std::fs::read_to_string(&path) {
1244:             Ok(s) => Self::from_json(&s, &path),
1245:             Err(e) => Self::strict(&path, &format!("unreadable ({})", e)),
1246:         }
1247:     }
1248: 
1249:     /// Parse the allowlist JSON. An unknown `update-*` class id is IGNORED with a loud
1250:     /// warning (fail-STRICT: the comparator has no detector for it, so that class keeps
1251:     /// failing rather than being silently "absorbed" by nothing); malformed JSON is
1252:     /// also strict.
1253:     fn from_json(s: &str, path: &str) -> Self {
1254:         let v: serde_json::Value = match serde_json::from_str(s) {
1255:             Ok(v) => v,
1256:             Err(e) => return Self::strict(path, &format!("invalid JSON ({})", e)),
1257:         };
1258:         let ids: Vec<String> = v["classes"]
1259:             .as_array()
1260:             .map(Vec::as_slice)
1261:             .unwrap_or(&[])
1262:             .iter()
1263:             .filter_map(|c| c["id"].as_str())
1264:             .filter(|id| id.starts_with("update-"))
1265:             .map(str::to_string)
1266:             .collect();
1267:         if ids.is_empty() {
1268:             return Self::strict(path, "no adjudicated `update-*` classes");
1269:         }
1270:         let mut out = UpdateDivergenceAllowlist {
1271:             integer_lexical: false,
1272:             state: String::new(),
1273:         };
1274:         let mut enabled = Vec::new();
1275:         for id in &ids {
1276:             if id == INTEGER_LEXICAL_CLASS {
1277:                 out.integer_lexical = true;
1278:                 enabled.push(id.clone());
1279:             } else {
1280:                 eprintln!(
1281:                     "warning: divergence allowlist lists update class {:?} but update-fuzz has \
1282:                      no detector for it — that class stays STRICT",
1283:                     id
1284:                 );
1285:             }
1286:         }
1287:         out.state = format!(
1288:             "({}): adjudicated classes enabled {:?}; every other divergence fails",
1289:             path, enabled
1290:         );
1291:         out
1292:     }
1293: }
```

### crates/sparq-bench/src/update_fuzz.rs:1297–1350
```rust
1297: pub fn run(seed_start: u64, count: u64) {
1298:     let allow = UpdateDivergenceAllowlist::load();
1299:     println!("update-fuzz divergence allowlist {}", allow.state);
1300:     let mut checked = 0u64;
1301:     let mut ops_applied = 0u64;
1302:     let mut adjudicated_integer_lexical = 0u64;
1303:     let mut isomorphism_compares = 0u64;
1304:     let mut mismatch = 0u64;
1305:     let mut first_repro: Option<String> = None;
1306: 
1307:     for seed in seed_start..seed_start + count {
1308:         match check_seed(seed, None, &allow) {
1309:             Ok(outcome) => {
1310:                 checked += 1;
1311:                 ops_applied += outcome.ops;
1312:                 adjudicated_integer_lexical += outcome.adjudicated_integer_lexical;
1313:                 isomorphism_compares += outcome.isomorphism_compares;
1314:             }
1315:             Err(detail) => {
1316:                 mismatch += 1;
1317:                 // One machine-greppable line per failing seed (the contract
1318:                 // scripts/ci-file-differential-failure.py parses).
1319:                 eprintln!("MISMATCH seed={}", seed);
1320:                 if first_repro.is_none() {
1321:                     first_repro = Some(format!("seed={}\n{}", seed, detail));
1322:                 }
1323:             }
1324:         }
1325:     }
1326: 
1327:     println!(
1328:         "update-fuzz seeds {}..{} : checked={} update_requests={} \
1329:          isomorphism_compares={} adjudicated(integer-lexical)={} mismatch={}",
1330:         seed_start,
1331:         seed_start + count,
1332:         checked,
1333:         ops_applied,
1334:         isomorphism_compares,
1335:         adjudicated_integer_lexical,
1336:         mismatch
1337:     );
1338:     // NON-VACUITY GUARD: a window that never exercised the v2 isomorphism path has
1339:     // lost blank-node coverage without failing anything — say so loudly.
1340:     if mismatch == 0 && checked > 0 && isomorphism_compares == 0 {
1341:         eprintln!(
1342:             "warning: no step in this window needed RDFC-1.0 relabelling — the \
1343:              blank-node isomorphism path was never exercised"
1344:         );
1345:     }
1346:     if let Some(r) = first_repro {
1347:         println!("\nFIRST FAILING CASE:\n{}", r);
1348:         std::process::exit(1);
1349:     }
1350: }
```

## Prior fixed witnesses and normative context

The frozen engine-witness contains both qualified revisions × rebuild/in-place × lexical-pair/8-only/8+9 controls. The frozen oxigraph-witness uses pinned 0.5.9 with rdf-12 and no RocksDB/HTTP features: the lexical pair yields one term, one WHERE row and one fresh blank node; controls yield one and two. No canonicalizer participates in those reduced observations. RDF 1.1 Concepts §3.3 defines literal-term identity using lexical form; SPARQL 1.1 Update §3.1.3/4.2.3 specifies fresh template blank nodes per solution. These primary sources were already verified; this phase performed no network reads.

Full relevant module, actual workflow, lock/manifests, declaration-free default allowlist and unredacted local compiler diagnostics are separate evidence files. This compact packet omits the rest of the unchanged engine, the full regenerable tracked archive, host paths, raw build environment/logs and personal metadata. Prior root-verified witnesses supply the reduced runtime evidence; this packet does not claim all eight original seeds or full CI were rerun.
