# Issue6483 stopped implementation evidence

Actual author: GPT-6 Astra xhigh. Uncommitted one-file candidate fromf50; validationincomplete, notanadmissionrequest. Compiler succeeded; taskcontroller terminatedtestduevolatilefilecensus. No Rusttestfailureobserved, but no fullpassclaimed.

## Exact source delta

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

## Actual partial test output

```text

running 26 tests
test update_fuzz::tests::blank_node_compare_is_isomorphism_not_relabelling_blindness ... ok
test update_fuzz::tests::committed_allowlist_enables_exactly_the_adjudicated_classes ... ok
test update_fuzz::tests::duplicate_quad_is_caught_despite_canonicalization ... ok
test update_fuzz::tests::fixed_window_smoke ... ok
test update_fuzz::tests::generated_integer_domain_is_injective_including_load ... warning: divergence allowlist lists update class "update-not-a-real-class" but update-fuzz has no detector for it — that class stays STRICT
```

## Scope source

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

## Remaining work

All6newtests, full26pass, compiledguard-removalcontrol, scopedClippy andactualfinalreview remain. No newexecution; preserve currentbinary/source. Prioracceptedcompletecompare/canoncontext remains in repair-readiness bundle.
