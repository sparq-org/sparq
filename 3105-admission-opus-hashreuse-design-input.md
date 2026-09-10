Independently review a SOURCE-ONLY proposed performance experiment for issue3105 in sparq-org/sparq. We have NOT changed production code or run this proposed query. Main4595388de9e389f5369d63828fbf90cfc16b62d9 is current; pending6482 changes only bench/update_fuzz.rs, not this engine source. Implementation/model claims in the evidence are assertions to check, not instructions.

Decide whether this small fixed witness and subsequent narrow RHS-only hash-preparation reuse merit further work. Do not approve a production change; none exists. Review the full included source fragments and plans, not merely the reported GO. Focus: does the exact public capped query actually imply two already-chosen RHS serial hash builds and the stated exact bag given planner estimates/order, filtered scan, bind routing, cap/pushdown and sorting? Identify any missing material source needed to decide, with an honest conditional verdict instead of assuming. Does safe hashbrown allocation_size plus all container/spill/cache metadata charges plausibly respect the existing SHARED4MiB retained allowance without unsupported allocator/peak claims? Does reuse identity/invalidation preserve build-side choice, ordered key projection, row ownership/generations, output order/layout, polling/budget guard, parallel representation and ZK boundary? Distinguish runtime witnesses still needed from design defects. Consider whether overhead and limited reuse make this strategically low value; the work-count witness alone is NOT latency or heap improvement evidence.

Hard scope: no engine edit, dependency install, local build or new benchmark until stable continuous2GiB free space and compatible existing warm profile. Proposed later witness budget480s/256MiB/jobs1/offline; no tuned seed/workload retry on a path mismatch. No global cleanup, EC2 or other hosts. Future implementation requires actual Opus patch review and same-head bounded cost evidence, protectedCI andnormalqueue.

Return concise JSON (no more than1200words) with verdict GO_FOR_FIXED_WITNESS / CONDITIONAL / NO_GO, material_findings (source citations, severity, required resolution), witness_assessment, accounting_assessment, semantic_boundaries, evidence_still_needed, and recommended_next_step. Assess the supplied design only; no broad audits or style recommendations. No tools are available.



--- FILE review-packet.md ---
# Capped RHS hash reuse: source-only readiness

**GO for one bounded instrumented witness, not implementation/performance admission.** Current main459 source is byte-identical in the clean fe328 worktree for every inspected runtime/dependency file.

The concrete SELECT witness and full algebra are in witness-plan.json. Q is written first so its numeric object filter is pushed there, but P is chosen as seed because its unfiltered estimate2048 is below Q2049. P scans by subject; Q's64passing rows scan by object. Two shared variables exclude bind, incompatible actual sorts exclude merge, and1024>64 selects RHS as hash-build side in both seed blocks. LIMIT65 cannot terminate a64-row answer early. Exact bag: each integer0..15 four times. A future observer must pin actual parsed order/filter assignment and actual table construction: baseline two64-row builds, candidate one, with scan cache already used once. No query has been run in this phase.

Pinned hashbrown0.17.1 exposes safe HashMap::allocation_size(), returning the underlying allocation layout size. Charge that for every retained map, plus tables Vec capacity, spilled Key and Posting capacities, and enlarged slot capacity, all within existing4MiB shared with scan rows/vars/text. This is requested retained storage, excluding allocator overhead and original transient/query state. Full formula and invalidation are in retention-plan.json. Measure/account only after the existing required build; nonfitting tables are used transiently then dropped. Identity is same owned scan generation, exact ordered build columns, original serial/64partition kind, and **only after unchanged current smaller-side selection chooses RHS**. Recompute probe columns/output layout; never cache whole Bindings clones or change planner decisions. Release old row/table charge before replacement.

Minimal future boundary is private exec.rs slot/accounting/cached hash adapter and cfg(test) probes; substrate already exposes required build/probe operations. No new dependency/storage/public API is needed. Preserve armed budget bypass, recorder/view admission, per-probe polling, parallel thresholds/64partitions and sticky check. Default compact/rewrite/custom-plan paths are not proven by this native-six-permutation witness.

Costs remain unresolved: accounting scans map entries, extra metadata impacts first-block hits, and retained tables can displace scan retention. No measured timing/peak benefit. First proposed execution is one filtered public-query witness, <=480s,<=256MiBnew, stable>=2GiB+64MiBstart and>=2GiBcontinuous,offline/jobs1/incremental0, confirmed warm profile or stop. Current reserve is unstable; this report starts no build.

Source excerpts contain exact loaded functions/callers and pinned capacity API. No code changes, GitHub reads/writes, builds/tests/benchmarks, other models or pending commands.


--- FILE report.json ---
{
  "author": "GPT-6 Astra xhigh",
  "recorded_utc": "2026-09-10T15:29:04.674171+00:00",
  "decision": "GO_FOR_ONE_BOUNDED_INSTRUMENTED_WITNESS_ONLY",
  "implementation_admission": false,
  "main": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "worktree_head": "fe3284199db0831f353d2ae401b8c69472764904",
  "clean": true,
  "network_reads_this_phase": 0,
  "builds_tests_benchmarks_this_phase": 0,
  "code_changes": 0,
  "fixed_witness": "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :q ?o . ?s :p ?o . FILTER(?o < 16) } LIMIT 65",
  "expected_unexecuted": "2actual RHS serial table builds(128row visits),1RHS scan,64result rows; candidate target1build64visits. Independently verify using public query + actual build observer.",
  "accounting_blocker_resolved_in_source": "Pinned hashbrown0.17.1 JoinTable has safe allocation_size(); add table-vector capacity, spilled Key/Posting capacities and charged inline cache metadata to existing shared4MiB. No length estimate or new public API needed.",
  "tradeoff": "After-build charge requires an O(buckets) iteration over stored keys/postings, plus extra retained metadata. First-block hits may pay this without reuse; first-fit tables can crowd out later scan reuse. Even passing the work witness cannot establish latency/heap benefit.",
  "next_step": "When root independently confirms stable reserve/warm exact profile, instrument and run ONLY the proposed current-main public witness first. Stop if parser/actualsort/cardinality/path differs; do not tune until it passes. Actual Opus design/patch review and bounded exact-head timing/heap evidence remain subsequent decisions.",
  "later_execution_budget_proposal": {
    "authorization": "Not authorization to execute now.",
    "initial_execution": "One engine cfg(test) filtered witness; no full workspace or external service. Rust2021, pinned toolchain, offline/locked, jobs1,incremental0, warm existing task cache profile only. Preserve source/feature/compiler provenance.",
    "hard_time_seconds": 480,
    "hard_new_allocated_output_bytes": 268435456,
    "minimum_free_before_start_bytes": 2214592512,
    "continuous_free_floor_bytes": 2147483648,
    "stop_conditions": [
      "No confirmed compatible warm profile",
      "Missing offline dependencies",
      "Output growth/time/free limit",
      "Actual path differs from declared oracle"
    ],
    "memory_estimate": "4097 synthetic triples,2048seed rows,64RHS rows and64output rows: runtime fixture is modest; no measured compiler footprint. Existing cfg(test) rebuild may exceed cap if cache is cold; stop rather than widen."
  },
  "reserve_observation_bytes": 2016718848,
  "no_runtime_claim": "All proposed new work counts/accounting design are static deductions; no new query, allocator calibration or compiled control executed.",
  "source_and_hold_preservation": "issue5183 branch/frozen bundles unchanged. No6483 witness, no GH reads or root-held task changes. Prior openPR dedupe is frozen historical evidence, not a fresh conflict guarantee.",
  "local_read_limitations": [
    "A local rg guessed hashbrown src/raw/mod.rs; actual pinned file is src/raw.rs, then read its authoritative body.",
    "A local rg guessed exec/budget.rs; budget is inline in exec.rs; no conclusion depends on that nonexistent path.",
    "Two overly broad read outputs were truncated; load-bearing sections were subsequently read in focused spans."
  ],
  "pending_commands": false
}


--- FILE witness-plan.json ---
{
  "status": "STATIC_DERIVATION_NOT_EXECUTED",
  "query": "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :q ?o . ?s :p ?o . FILTER(?o < 16) } LIMIT 65",
  "dataset": {
    "load": "Graph::load_str synthetic Turtle, ordinary base default graph",
    "p": "For i=0..2047 add :s{i} :p (i mod16) as canonical xsd:integer. Exactly2048 triples.",
    "q_matches": "For i=0..63 add :s{i} :q (i mod16). Exactly64 triples.",
    "q_fillers": "For j=0..1984 add distinct :f{j} :q (16+(j mod16)). No :f subject has :p. Exactly1985 triples.",
    "total_triples": 4097,
    "distinct_integer_values": 32,
    "no_duplicates_or_triple_terms": true
  },
  "qualification": "Native six-permutation default engine/core; no compact-index, no algebra-rewrite or installed custom planner/statistics, ordinary unlimited QueryBudget, recorder disarmed. Pin actual parsed BGP order/filter assignment in later test; do not silently adjust fixture if it differs.",
  "static_path": [
    "query -> query_with_budget(unlimited) -> PreparedQuery::parse -> query_prepared_with_budget -> eval_select -> eval_modified Slice -> try_capped Project/Filter(BGP) -> eval_bgp_binary_capped. No COUNT, DISTINCT, ORDER BY or single-pattern shortcut.",
    "flatten_conjunction preserves original BGP order. split_sargable assigns ?o<16 to first pattern Q at object column2, before GOO. prepare_bgp ignores filter selectivity: Q estimate2049, P estimate2048. Therefore seed=P(index1), RHS=Q(index0); only one GOO candidate exists each block.",
    "P has no pushed filter. goo_seed_sort chooses shared subject position0. Native Pso scan actually sorted_by s. Seed has2048 rows; block starts0,1024 with lengths1024,1024.",
    "Q requested scan_sort Some(2) from its pushed filter, overriding candidate merge subject. Pos scan actually sorted_by o, so merge_var s fails actual-sort equality. Two connecting variables(s,o) prohibit the one-variable bind branch.",
    "Canonical nonnegative integer column permits range-pruning before projection/reservation; Q contains exactly64 passing rows. Rows/vars/text fit existing4MiB scan allowance. Q scan closure runs once, then exact sort2 relation reused in block2.",
    "Each left block has1024 rows and right has64. hash_join_ref chooses RHS on both calls by existing smaller-side test(1024>64); build shared key is RHS columns[0,1] for(s,o),64 unique keys. Both build and probe are below50000, so existing serial table/probe paths apply.",
    "There are exactly64 solutions. LIMIT65 cannot be satisfied after any block, even if all64 matching subjects happen to be in block1; both blocks necessarily execute. This does not rely on dictionary ID ordering or match distribution."
  ],
  "expected_bag": {
    "projected_variable": "o",
    "terms": "Canonical xsd:integer0..15",
    "multiplicity_each": 4,
    "total_rows": 64,
    "full_mapping_optional": "Each(s_i,i mod16),i=0..63 occurs once; projected bag independently calculated, not derived from production query."
  },
  "baseline_work_oracle": {
    "capped_entries": 1,
    "seed_blocks": 2,
    "rhs_scan_closures": 1,
    "bind_calls": 0,
    "merge_calls": 0,
    "rhs_hash_join_calls": 2,
    "lhs_hash_build_calls": 0,
    "serial_rhs_table_constructions": 2,
    "rhs_rows_visited_in_build": 128,
    "parallel_partitioned_build_calls": 0
  },
  "instrumentation_required": "cfg(test) observer around actual engine call to sjoin::build_table/build_partitioned and branch before the call, tied to capped step/pattern/start; log actual build side,row lengths,ordered build key columns,scan_sort/actual sorted_by,tables len. Existing RHS-scan observer alone cannot prove hash work. Assert baseline64+64 build visits with exact bag before implementing reuse.",
  "later_candidate_oracle": "If reviewed cache is implemented and actual table+scan charge fits: same bag/path/decisions, one64-row serial table construction plus one cache hit. Negative control drops only retained hash preparation before block2, keeps scan cache valid: build count returns2/visits128 and headline1-build assertion fails; bag must remain correct.",
  "anti_vacuity_limits": [
    "Not a timing or throughput claim.",
    "Compact-index may yield a merge instead: this specific work witness must not be generalized to that configuration.",
    "Does not prove64-partition reuse or arbitrary projection changes; those require separate focused tests before broader admission.",
    "The earlier70000-row scan witness did not prove RHS-build selection; this fixture deliberately separates prefilter estimate from actual RHS cardinality."
  ]
}


--- FILE retention-plan.json ---
{
  "decision": "Feasible with existing safe APIs; no new dependency/public storage API required. Proposed only, not implemented.",
  "existing_allowance_bytes": 4194304,
  "formula": "slot_vec.capacity()*size_of<Option<NEW CappedRhs>>() + sum(existing retained Bindings charge) + sum(retained hash charge) <=4194304. All arithmetic checked; any overflow/unproved representation declines retention.",
  "hash_charge": [
    "tables.capacity()*size_of<sjoin::JoinTable>() for Vec object backing elements; Vec header and fixed key descriptor live in enlarged slot struct, already covered by slot capacity charge.",
    "For EACH table add table.allocation_size(): pinned hashbrown0.17.1 public safe API delegates to allocation layout size, covering bucket storage/control bytes/alignment requested from allocator. Do not substitute len or capacity*size_of(entry).",
    "For EACH stored(key,posting), add key.capacity()*size_of<Id>() iff key.spilled(), and posting.capacity()*size_of<usize>() iff posting.spilled(). Inline SmallVec payloads already included in table allocation. Spill capacity, not length, preserves duplicate-key posting growth accounting.",
    "Use a fixed at-most-three-column build projection descriptor ([usize;3]+len or equivalent); no retained Vec of probe columns/variables or copied JoinKeys. If actual RHS width>3, decline hash retention. Enlarged per-slot type must participate in pre-reservation and actual-capacity checks before allocating slots."
  ],
  "scope_of_bound": "Requested retained allocation layout/capacities per invocation only, same as existing policy. Allocator metadata/usable-size rounding, stack, pre-existing AST/prepared/planner/NDV state, seed/result storage, temporary key allocations/parallel tag vector and original transient table construction remain outside. No whole-query peak/RSS/global/nested-budget guarantee.",
  "identity": [
    "Invocation-local slot for immutable prepared pattern + immutable pushed filter + fixed graph/view. RHS requested sort already keys scan generation and actual sorted_by remains truthful. Hash tables must be owned WITH the corresponding retained Bindings, never borrowed across replaced/dropped rows.",
    "Recompute existing per-block GOO/bind/merge/hash decision and exact smaller-side choice first. Reuse only if RHS is currently chosen build side; equality still selects LHS and uses original build. Do not choose an old cached table over a different current strategy.",
    "Exact ordered RHS build-column projection AND serial-vs64-partition representation must match. Recompute current probe-column mapping, out_vars,probe_only and JoinKeys every invocation; only stored build-key projection is reused. Hash function/partition count are existing monomorphic constants, unchanged.",
    "Build selected new table through original build_table/build_partitioned path once, then account completed allocation. Publish into cache ONLY when RHS is already retained and combined charge fits. Otherwise probe with original transient table and drop it at the old boundary, no rescans/clones/eviction-policy substitution."
  ],
  "invalidation": [
    "Before RHS scan-sort replacement, detach/drop hash tables AND old rows and refund both charges before invoking new scan; table postings are old row indices.",
    "Before replacing a hash with a different ordered build key/kind, drop/refund old hash first, preserving scan entry. Never hold two retained generations.",
    "A bind/merge/LHS-build step cannot use hash cache; a still-valid unused hash can remain charged until later same-identity use/replacement/query exit. No forced strategy change or cross-query cache.",
    "Disabled slot cache/armed budget/unretained relation means no hash retention or new dynamic cache metadata allocation. Keep source try_capped recorder and view guards unchanged."
  ],
  "preservation": [
    "Native build threshold remains50000 with64partitions; no serial/parallel policy change, tables probed with original key_hash/probe_table and ascending posting indices.",
    "Existing serial EngineBudget poll per probe row, parallel EngineSnapshot/fold/reduce and final sticky budget check remain at same sites. Skip only redundant pure builds with no existing polling.",
    "Keep build/probe smaller-side layout and recomputed projection ordering. Serial enumeration stays identical; parallel scheduling remains existing behavior, not a promise of unspecified order stability.",
    "No substrate public API changes: existing build/probe methods and JoinTable alias suffice. Keep ordinary hash_join_ref callers uncached. Minimal prospective files:exec.rs private cached adapter/slot/accounting plus focused tests, documentation/declaration only if subsequently required by reviewed runtime change."
  ],
  "accounting_controls_for_later": [
    "Create equal-len different-capacity table fixtures; verify charge against allocation_size not len.",
    "Use Key with3columns and Posting with>2entries to force spills; omitting either spill charge must fail an exact capacity assertion.",
    "Boundary fit/one-byte-short and multiple table entries jointlyshare4MiB; ignoring table charge must demonstrably exceed reported retained allowance without merely failing compilation.",
    "Sort/key change drops/refunds table before rebuilding; compiled no-refund/drop control must fail lifetime/accounting assertion.",
    "No engine snapshot/cancellation/rowbudget checks may be weakened to demonstrate gain."
  ]
}


--- FILE provenance.json ---
{
  "main": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "head": "fe3284199db0831f353d2ae401b8c69472764904",
  "clean": true,
  "engine_source_equal_main": [
    {
      "path": "crates/sparq-engine/src/exec.rs",
      "sha256": "10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af",
      "bytes": 1067149,
      "equals_exact_main_blob": true
    },
    {
      "path": "crates/sparq-engine/src/lib.rs",
      "sha256": "074740149d0649b0334d1814ca289feaad54070ca0d90bf5737601f7021a7538",
      "bytes": 168716,
      "equals_exact_main_blob": true
    },
    {
      "path": "crates/sparq-engine/Cargo.toml",
      "sha256": "ab9734674fb46fcf5ad91415cff14f2552d4bc9e6a5f2994ca2e0905e2b99bc2",
      "bytes": 39603,
      "equals_exact_main_blob": true
    },
    {
      "path": "crates/sparq-substrate/src/join.rs",
      "sha256": "cd708a74d9f77a9c4ab2e519a1d6475c4863fb68f909889d2d62d8e1b9cefae5",
      "bytes": 90654,
      "equals_exact_main_blob": true
    },
    {
      "path": "crates/sparq-substrate/src/rows.rs",
      "sha256": "490d760db1825e85a08b7434725ffa0ba5d9c62adaaaa8f537442d7d48f42de0",
      "bytes": 5927,
      "equals_exact_main_blob": true
    },
    {
      "path": "crates/sparq-core/src/store.rs",
      "sha256": "6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c",
      "bytes": 112315,
      "equals_exact_main_blob": true
    },
    {
      "path": "Cargo.toml",
      "sha256": "9f820551d15c1430a25172c831f491fb7fd92231c15f124b2a048b1137d8ce73",
      "bytes": 9813,
      "equals_exact_main_blob": true
    },
    {
      "path": "Cargo.lock",
      "sha256": "6d7b6095f5fec01e4ace906d6001687c67d83fedc1e50180891a419c22026152",
      "bytes": 176396,
      "equals_exact_main_blob": true
    }
  ],
  "prior_partial_report_sha256": "1629c253aadc3e032e94d199d5272eed0e093348778996285bbbc77a84e6c8a3",
  "prior_retention_report_sha256": "bc2db4b26d113aa2ddf9a60811c77041b861d0e1c574cc539e3a884ba1b67b2e",
  "skills_used": [
    "rust-router",
    "m10-performance",
    "m02-resource"
  ],
  "dependency_sources": "Pinned local registry sources only; hashbrown0.17.1,smallvec1.15.2 identity bound by Cargo.lock and full source SHA in excerpts. No cached dependency edits."
}


--- FILE public-path-context.txt ---
SOURCE crates/sparq-engine/src/lib.rs; full source SHA256 074740149d0649b0334d1814ca289feaad54070ca0d90bf5737601f7021a7538
LINES 841-874
841: pub struct PreparedQuery {
842:     query: Query,
843: }
844: 
845: impl PreparedQuery {
846:     /// Parses a SPARQL query string into its reusable algebra form.
847:     ///
848:     /// With the opt-in `algebra-rewrite` feature ON, the parsed algebra is run
849:     /// through the result-equivalent pre-execution rewrite pass (`rewrite`
850:     /// module) here — the single seam every string query entry point
851:     /// ([`query`], [`ask`], [`count`], the JSON paths) funnels through — so
852:     /// production benefits without touching the executor. The `From<Query>`
853:     /// conversion deliberately does NOT rewrite: it takes an already-built
854:     /// algebra verbatim (the opt-out / test-baseline path). When the feature is
855:     /// OFF the algebra is stored verbatim and the build is byte-identical.
856:     pub fn parse(sparql: &str) -> Result<PreparedQuery, String> {
857:         // Feature-OFF arm is the VERBATIM pre-`algebra-rewrite` expression so the
858:         // default build's codegen is byte-identical (the `feature_off_exact` wasm
859:         // gate). Only the feature-ON arm introduces the rewrite call. [OPUS-4.8]
860:         #[cfg(not(feature = "algebra-rewrite"))]
861:         {
862:             Ok(PreparedQuery { query: SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())? })
863:         }
864:         #[cfg(feature = "algebra-rewrite")]
865:         {
866:             let query = rewrite::rewrite_query(SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?);
867:             Ok(PreparedQuery { query })
868:         }
869:     }
870: 
871:     /// The wrapped `spargebra` algebra (e.g. to inspect the query form or dataset
872:     /// clause before execution).
873:     pub fn query(&self) -> &Query {
874:         &self.query
LINES 1016-1056
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
1055: 
1056: /// Executes an ASK query: `true` iff the pattern has at least one solution.
SOURCE crates/sparq-engine/Cargo.toml; full source SHA256 ab9734674fb46fcf5ad91415cff14f2552d4bc9e6a5f2994ca2e0905e2b99bc2
LINES 24-43
24: [features]
25: # Forwards sparq-core's parallel index build. Default on for native; the wasm
26: # crate disables defaults so rayon is never pulled into the bundle.
27: # `regex` powers SPARQL REGEX/REPLACE; default-on for native, off for wasm (the wasm crate
28: # disables defaults) so the regex automata don't bloat the browser bundle.
29: # `digest` powers the SPARQL hash builtins (MD5/SHA1/SHA256/SHA384/SHA512); default-on
30: # for native, off for wasm (the wasm crate disables defaults) so the hash cores never
31: # enter the browser bundle.
32: default = ["parallel", "regex", "digest"]
33: parallel = ["dep:rayon", "sparq-core/parallel"]
34: regex = ["dep:regex"]
35: digest = ["dep:md-5", "dep:sha1", "dep:sha2"]
36: # Characteristic-set star-join cardinality estimation (Neumann & Moerkotte):
37: # `cs::CsTable` + `with_cs_table` make the greedy planner consult an injected CS
38: # table for star joins instead of the per-predicate independence model. OPT-IN and
39: # OFF by default (the wasm bundle and the default native build carry zero CS code);
40: # results are identical either way — only join order is affected.
41: cs-planner = []
42: # [FABLE-5] (sq-dzbg2) Persistent, explicitly-built planner statistics.  The
43: # catalog is stored beside a saved/mmap graph and contains deterministic
SOURCE crates/sparq-engine/src/exec.rs; full source SHA256 10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af
LINES 2450-2454
2450: /// `parallel` feature, so these paths compile to the sequential loops).
2451: #[cfg(feature = "parallel")]
2452: const PAR_THRESHOLD: usize = 50_000;
2453: 
2454: /// Maximum `offset + limit` (row budget) for the bounded-heap ORDER BY path.
LINES 21691-21768
21691:             *bag.entry(
21692:                 row.iter()
21693:                     .map(|v| v.as_ref().unwrap().to_string())
21694:                     .collect(),
21695:             )
21696:             .or_default() += 1;
21697:         }
21698:         bag
21699:     }
21700: 
21701:     #[test]
21702:     fn capped_rhs_changing_sort_mixed_kernels_preserves_full_bag() {
21703:         let mut ttl = String::from("@prefix : <http://ex/> .\n");
21704:         for i in 0..70_000 {
21705:             // Projection deliberately collapses pairs, making multiplicity observable.
21706:             ttl.push_str(&format!(":s{i} :p {} ; :q {i} ; :r {i} .\n", i / 2));
21707:         }
21708:         ttl.push_str(":extra1 :q 70001 ; :r 70001 . :extra2 :r 70002 .");
21709:         let graph = Graph::load_str(&ttl, "turtle").unwrap();
21710:         let query = "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :p ?o . ?s :q ?x . ?s :r ?x . FILTER(?o + 0 >= 0) }";
21711:         let full = crate::query(&graph, query).unwrap();
21712:         let (limited, steps) =
21713:             trace(|| crate::query(&graph, &format!("{query} LIMIT 70001")).unwrap());
21714:         println!("changing-sort actual steps: {steps:?}");
21715:         assert_eq!(limited.rows.len(), 70_000);
21716:         assert_eq!(bag(&limited), bag(&full));
21717:         let expected: std::collections::BTreeMap<_, _> = (0..35_000)
21718:             .map(|i| (vec![oxrdf::Literal::from(i).to_string()], 2))
21719:             .collect();
21720:         assert_eq!(bag(&limited), expected);
21721:         let q: Vec<_> = steps
21722:             .iter()
21723:             .filter(|s| s.pattern == 1)
21724:             .map(|s| {
21725:                 (
21726:                     s.start,
21727:                     s.kernel,
21728:                     s.requested,
21729:                     s.scanned,
21730:                     s.actual.as_deref(),
21731:                 )
21732:             })
21733:             .collect();
21734:         let r: Vec<_> = steps
21735:             .iter()
21736:             .filter(|s| s.pattern == 2)
21737:             .map(|s| (s.start, s.requested, s.scanned, s.actual.as_deref()))
21738:             .collect();
21739:         if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
21740:             assert_eq!(
21741:                 q,
21742:                 [
21743:                     (0, "bind", None, false, None),
21744:                     (1024, "merge", Some(0), true, Some("s")),
21745:                     (65536, "bind", None, false, None)
21746:                 ]
21747:             );
21748:             assert_eq!(
21749:                 r,
21750:                 [
21751:                     (0, None, true, Some("s")),
21752:                     (1024, Some(0), true, Some("s")),
21753:                     (65536, None, true, Some("s"))
21754:                 ]
21755:             );
21756:         } else {
21757:             // Three permutations return object order: no false subject-order claim,
21758:             // and the unchanged None request must reuse the actually unsorted-for-s RHS.
21759:             assert_eq!(
21760:                 q,
21761:                 [
21762:                     (0, "bind", None, false, None),
21763:                     (1024, "hash", None, true, Some("x")),
21764:                     (65536, "bind", None, false, None)
21765:                 ]
21766:             );
21767:             assert_eq!(
21768:                 r,


--- FILE planner-scan.txt ---
SOURCE crates/sparq-engine/src/exec.rs; full source SHA256 10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af
LINES 6285-6303
6285: pub(crate) fn is_conjunctive(p: &GraphPattern) -> bool {
6286:     match p {
6287:         GraphPattern::Bgp { .. } => true,
6288:         // A FILTER may only be flattened into the enclosing conjunction when every
6289:         // variable it mentions is bound INSIDE its own group — otherwise hoisting it
6290:         // changes scope (`{ :x :p ?v . { FILTER(?v = 1) } }` must see ?v UNBOUND).
6291:         // EXISTS is conservatively never flattened (it evaluates against the group's
6292:         // in-scope bindings).
6293:         GraphPattern::Filter { inner, expr } => {
6294:             if !is_conjunctive(inner) {
6295:                 return false;
6296:             }
6297:             let mut inner_vars: FxHashSet<Variable> = FxHashSet::default();
6298:             collect_pattern_vars(inner, &mut inner_vars);
6299:             filter_scope_ok(expr, &inner_vars)
6300:         }
6301:         GraphPattern::Join { left, right } => is_conjunctive(left) && is_conjunctive(right),
6302:         _ => false,
6303:     }
LINES 6345-6358
6345: pub(crate) fn flatten_conjunction(p: &GraphPattern, patterns: &mut Vec<TriplePattern>, filters: &mut Vec<Expression>) {
6346:     match p {
6347:         GraphPattern::Bgp { patterns: tps } => patterns.extend(tps.iter().cloned()),
6348:         GraphPattern::Join { left, right } => {
6349:             flatten_conjunction(left, patterns, filters);
6350:             flatten_conjunction(right, patterns, filters);
6351:         }
6352:         GraphPattern::Filter { expr, inner } => {
6353:             flatten_conjunction(inner, patterns, filters);
6354:             filters.push(expr.clone());
6355:         }
6356:         _ => unreachable!(),
6357:     }
6358: }
LINES 6500-6509
6500: /// [OPUS-4.8] (sq-lr2ii) A NUMERIC comparison is DECLINED (returns `None`) when the graph
6501: /// holds an f64-inexact decimal ([`Graph::has_high_precision_decimal`]): the f64 `numerics`
6502: /// cache the scan probes could then decide `=`/`<`/`>`/`<=`/`>=` wrongly for that value (e.g.
6503: /// `"1.000000000000000001"^^xsd:decimal` collapses onto the f64 `1.0`), so such comparisons
6504: /// fall back to the exact general evaluator instead. Temporal pushdown is unaffected, and a
6505: /// graph with no f64-inexact decimal keeps the numeric fast path.
6506: fn extract_sargable(graph: &Graph, e: &Expression) -> Option<(Variable, ScanCmp)> {
6507:     fn lit_num(e: &Expression) -> Option<f64> {
6508:         match e {
6509:             Expression::Literal(l) if is_numeric_dt(l) => {
LINES 6570-6631
6570:                 return Some((v, num_cmp(op, c)));
6571:             }
6572:             continue;
6573:         }
6574:         if let Some(t) = lit_temp(konst) {
6575:             return Some((v, ScanCmp::Temp(op, t)));
6576:         }
6577:     }
6578:     None
6579: }
6580: 
6581: /// The canonical position (0=subject, 1=predicate, 2=object) of a variable in a
6582: /// triple pattern, if it occurs there.
6583: fn pattern_var_pos(tp: &TriplePattern, var: &Variable) -> Option<usize> {
6584:     if matches!(&tp.subject, TermPattern::Variable(v) if v == var) {
6585:         return Some(0);
6586:     }
6587:     if matches!(&tp.predicate, NamedNodePattern::Variable(v) if v == var) {
6588:         return Some(1);
6589:     }
6590:     if matches!(&tp.object, TermPattern::Variable(v) if v == var) {
6591:         return Some(2);
6592:     }
6593:     None
6594: }
6595: 
6596: /// Splits FILTERs into per-pattern sargable numeric predicates (pushed into the
6597: /// scan of the first pattern that binds the variable) and the residual filters
6598: /// (applied normally afterwards).
6599: pub(crate) fn split_sargable(graph: &Graph, patterns: &[TriplePattern], filters: &[Expression]) -> (Vec<Option<(usize, ScanCmp)>>, Vec<Expression>) {
6600:     // zk-trace: a sargable FILTER pushed into the scan would make the scan
6601:     // record only the POST-filter rows (the rows that PASSED), losing the
6602:     // FILTER obligation and under-capturing the input set — the proof must
6603:     // witness the operand of every filtered row and the rows it excluded.
6604:     // Keep every filter residual while recording, so it flows through
6605:     // apply_filter (one FilterObligation) over the FULL unfiltered scan set.
6606:     // Result-equivalent; only the plan changes (zk module docs).
6607:     #[cfg(feature = "zk")]
6608:     if crate::zk::enabled() {
6609:         return (vec![None; patterns.len()], filters.to_vec());
6610:     }
6611:     let mut pat_filters: Vec<Option<(usize, ScanCmp)>> = vec![None; patterns.len()];
6612:     let mut residual = Vec::new();
6613:     for f in filters {
6614:         if let Some((var, cmp)) = extract_sargable(graph, f) {
6615:             if let Some((i, pos)) = patterns
6616:                 .iter()
6617:                 .enumerate()
6618:                 .find_map(|(i, tp)| pattern_var_pos(tp, &var).filter(|_| pat_filters[i].is_none()).map(|pos| (i, pos)))
6619:             {
6620:                 pat_filters[i] = Some((pos, cmp));
6621:                 continue;
6622:             }
6623:         }
6624:         residual.push(f.clone());
6625:     }
6626:     (pat_filters, residual)
6627: }
6628: 
6629: // ---- Spatial FILTER pushdown (sq-mg9) ------------------------------------------
6630: //
6631: // Recognise a `geof:` spatial FILTER over an indexed geometry variable and, when a
LINES 8208-8216
8208: pub(crate) fn prepare_bgp(graph: &Graph, patterns: &[TriplePattern]) -> Result<Vec<Prepared>, String> {
8209:     let mut prepared: Vec<Prepared> = Vec::with_capacity(patterns.len());
8210:     for tp in patterns {
8211:         let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
8212:         let est = if unsat { 0 } else { graph.store.estimate(&id_pat) };
8213:         prepared.push(Prepared { id_pat, pos_vars, est, unsatisfiable: unsat });
8214:     }
8215:     Ok(prepared)
8216: }
LINES 8262-8282
8262: /// GOO seed choice: the pattern with the smallest single-pattern cardinality.
8263: pub(crate) fn goo_seed(prepared: &[Prepared]) -> usize {
8264:     (0..prepared.len()).min_by_key(|&i| prepared[i].est).unwrap()
8265: }
8266: 
8267: /// The seed scan's requested sort column: a pushed-down filter scans in its own
8268: /// column's order (sequential numeric access); otherwise sort by the first seed
8269: /// variable shared with another pattern, to enable a merge join.
8270: pub(crate) fn goo_seed_sort(prepared: &[Prepared], seed: usize, filter_col: Option<usize>) -> Option<usize> {
8271:     if filter_col.is_some() {
8272:         return filter_col;
8273:     }
8274:     prepared[seed]
8275:         .pos_vars
8276:         .iter()
8277:         .flatten()
8278:         .find(|v| (0..prepared.len()).any(|j| j != seed && prepared[j].var_pos(v).is_some()))
8279:         .and_then(|v| prepared[seed].var_pos(v))
8280: }
8281: 
8282: /// The characteristic-set planning context (the opt-in `cs-planner` feature) the
LINES 8392-8448
8392: pub(crate) fn goo_pick(
8393:     graph: &Graph,
8394:     prepared: &[Prepared],
8395:     done: &[bool],
8396:     var_ndv: &FxHashMap<Variable, f64>,
8397:     cur_card: f64,
8398:     cs: &CsCtx,
8399: ) -> (usize, f64, bool) {
8400:     let mut best: Option<(usize, f64)> = None;
8401:     for i in 0..prepared.len() {
8402:         if done[i] {
8403:             continue;
8404:         }
8405:         let cs_score = cs.pick_score(i, cur_card);
8406:         let mut sel = 1.0f64;
8407:         let mut shared = cs_score.is_some();
8408:         for (pos, ov) in prepared[i].pos_vars.iter().enumerate() {
8409:             if let Some(v) = ov {
8410:                 // The subject variable of a CS-scored star candidate is already
8411:                 // accounted for by the conditional star estimate.
8412:                 if pos == 0 && cs_score.is_some() {
8413:                     continue;
8414:                 }
8415:                 if let Some(&rndv) = var_ndv.get(v) {
8416:                     shared = true;
8417:                     let pndv = pattern_var_ndv(graph, &prepared[i].id_pat, pos, prepared[i].est);
8418:                     sel /= rndv.max(pndv).max(1.0);
8419:                 }
8420:             }
8421:         }
8422:         if !shared {
8423:             continue;
8424:         }
8425:         let out = match cs_score {
8426:             Some(star) => star * sel,
8427:             None => cur_card * prepared[i].est as f64 * sel,
8428:         };
8429:         if best.is_none_or(|(_, bc)| out < bc) {
8430:             best = Some((i, out));
8431:         }
8432:     }
8433:     match best {
8434:         Some((i, out)) => (i, out.max(0.0), true),
8435:         None => {
8436:             // Disconnected: smallest-cardinality remaining (cross product).
8437:             let i = (0..prepared.len()).filter(|&j| !done[j]).min_by_key(|&j| prepared[j].est).unwrap();
8438:             (i, cur_card * prepared[i].est as f64, false)
8439:         }
8440:     }
8441: }
8442: 
8443: /// Estimated number of distinct values of the variable at canonical position
8444: /// `pos` in a pattern, from the per-predicate characteristic stats. Falls back to
8445: /// the pattern's cardinality (an upper bound) when the predicate is unbound or the
8446: /// other terminal is bound (so the column is effectively keyed).
8447: fn pattern_var_ndv(graph: &Graph, id_pat: &IdPattern, pos: usize, est: usize) -> f64 {
8448:     let est = (est as f64).max(1.0);
LINES 8874-9011
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
8907:     // permutation's order — NOT necessarily the requested `sort_col`: with fewer than
8908:     // six permutations the store may not have the requested order, in which case the
8909:     // engine must report the real one so merge joins fall back to hash and range-
8910:     // pruning is skipped (both keyed off the truthful `sorted_by` / `actual_sort`).
8911:     let actual_sort = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none());
8912:     let sorted_by = actual_sort.and_then(|c| pos_vars[c].clone());
8913: 
8914:     // Range-pruning: when the pushed-down filter is on the scan's ACTUAL sort column and
8915:     // that column holds inline integers (which sort by value), binary-search to the
8916:     // passing value range instead of scanning + filtering the whole relation. Safe
8917:     // only when EVERY value in the column is inline (so no dictionary-encoded
8918:     // numeric in another datatype, scattered below INLINE_BASE, is skipped).
8919:     let mut scan_rows: &[[Id; 3]] = scan.rows.as_ref();
8920:     if let Some((fpos, cmp)) = filter {
8921:         if actual_sort == Some(fpos) && scan_rows.first().is_some_and(|r| dict::is_inline(scan.to_spo(r)[fpos])) {
8922:             scan_rows = match inline_pass_values(cmp) {
8923:                 Some((lo, hi)) => {
8924:                     let (lo_id, hi_id) = (dict::INLINE_BASE + lo, dict::INLINE_BASE + hi);
8925:                     let start = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] < lo_id);
8926:                     let end = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] <= hi_id);
8927:                     &scan_rows[start..end]
8928:                 }
8929:                 None => &[],
8930:             };
8931:         }
8932:     }
8933: 
8934:     // Per-row builder: apply the semi-join prefilter (if any) and the pushed-down
8935:     // filter, then project (with the repeated-variable consistency check); `None`
8936:     // drops the row.
8937:     let build_row = |row: &[Id; 3]| -> Option<Row> {
8938:         let spo = scan.to_spo(row);
8939:         // [OPUS-4.8] (sq-gr8mb / §A3) Semi-join prefilter: drop a row whose connecting-
8940:         // variable id is absent from the other side's join-key set — it cannot survive the
8941:         // downstream join. EXACT membership, so the result is unchanged (only this wasted
8942:         // row is skipped before projection). Checked first: it is the cheapest reject.
8943:         #[cfg(feature = "semijoin-bitmap")]
8944:         if let Some((jpos, kf)) = prefilter {
8945:             if !kf.contains(spo[jpos]) {
8946:                 return None;
8947:             }
8948:         }
8949:         if let Some((fpos, cmp)) = filter {
8950:             if !cmp.test_id(graph, spo[fpos]) {
8951:                 return None;
8952:             }
8953:         }
8954:         let mut out = Row::with_capacity(vars.len());
8955:         for positions in &var_positions {
8956:             let v0 = spo[positions[0]];
8957:             if positions.iter().any(|&p| spo[p] != v0) {
8958:                 return None;
8959:             }
8960:             out.push(v0);
8961:         }
8962:         Some(out)
8963:     };
8964: 
8965:     // zk-trace hook (feature `zk`, armed recorder only): record the matched
8966:     // triples of this pattern scan — the rows `build_row` keeps, BEFORE
8967:     // projection (the witness needs whole triples, not just variable columns).
8968:     // One `enabled()` check per scan; zero per-row cost when disarmed.
8969:     #[cfg(feature = "zk")]
8970:     if crate::zk::enabled() {
8971:         let kept: Vec<[Id; 3]> = scan_rows
8972:             .iter()
8973:             .filter(|r| build_row(r).is_some())
8974:             .map(|r| scan.to_spo(r))
8975:             .collect();
8976:         crate::zk::record_scan_ids(graph, id_pat, pos_vars, &kept, false);
8977:     }
8978: 
8979:     // No LIMIT and a large relation: build the rows in parallel (order-preserving).
8980:     #[cfg(feature = "parallel")]
8981:     if limit.is_none() && scan_rows.len() >= PAR_THRESHOLD {
8982:         use rayon::prelude::*;
8983:         let rows: Vec<Row> = scan_rows.par_iter().filter_map(build_row).collect();
8984:         return Bindings { vars, rows, sorted_by };
8985:     }
8986: 
8987:     // Reserve only up to the LIMIT so a small LIMIT over a huge scan does not
8988:     // allocate for the whole relation (the point of early termination).
8989:     let cap = limit.map_or(scan_rows.len(), |n| n.min(scan_rows.len()));
8990:     let mut rows: Vec<Row> = Vec::with_capacity(budget::cap_alloc(cap));
8991:     for (i, row) in scan_rows.iter().enumerate() {
8992:         // Coarse budget check every 4096 scanned rows.
8993:         if i & 4095 == 0 && budget::exhausted(rows.len()) {
8994:             break;
8995:         }
8996:         if let Some(out) = build_row(row) {
8997:             rows.push(out);
8998:             // LIMIT early-termination: stop scanning once we have enough rows.
8999:             if let Some(n) = limit {
9000:                 if rows.len() >= n {
9001:                     break;
9002:                 }
9003:             }
9004:         }
9005:     }
9006:     Bindings { vars, rows, sorted_by }
9007: }
9008: 
9009: fn merge_join(left: Bindings, right: Bindings, jv: &Variable) -> Bindings {
9010:     merge_join_ref(&left, &right, jv)
9011: }
SOURCE crates/sparq-core/src/store.rs; full source SHA256 6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c
LINES 27-39
27: /// The permutations actually built and searched. The full six give every triple
28: /// pattern a sorted scan in the order any merge join wants. The `compact-index` set
29: /// {SPO, POS, OSP} still answers EVERY triple pattern from one index (SPO→S*/SP*,
30: /// POS→P*/PO*, OSP→O*/OS*) at half the memory, at the cost of some merge joins (and
31: /// some lazy-count fast paths) falling back to hashing / sorting.
32: // Compact set on wasm ALWAYS (memory-bound target), or on native opt-in via the
33: // `compact-index` feature (for testing). Keyed on `target_arch` — NOT just a feature —
34: // so the wasm choice does not leak to the native build via Cargo feature unification.
35: #[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
36: pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
37: #[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
38: pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];
39: 
LINES 1080-1094
1080: 
1081:     /// Returns the contiguous slice of rows (in `perm` order) matching the bound
1082:     /// prefix of the pattern, together with the chosen permutation.
1083:     pub fn scan(&self, pattern: &Pattern) -> Scan<'_> {
1084:         let (perm, lead) = Self::choose(pattern);
1085:         self.scan_with(pattern, perm, lead)
1086:     }
1087: 
1088:     /// Scans choosing a permutation whose output is sorted by canonical column
1089:     /// `sort_col` (when possible), for merge joins.
1090:     pub fn scan_sorted(&self, pattern: &Pattern, sort_col: usize) -> Scan<'_> {
1091:         let (perm, lead) = Self::choose_sorted(pattern, sort_col);
1092:         self.scan_with(pattern, perm, lead)
1093:     }
1094: 
LINES 1160-1180
1160:     }
1161: 
1162:     /// Estimated number of matches for a pattern (the range length) — the cardinality
1163:     /// estimate used by the greedy planner. Cheap for every storage mode: raw modes
1164:     /// subtract binary-search bounds; the compressed mode counts via the block directory
1165:     /// decoding at most two boundary blocks (never the whole range).
1166:     pub fn estimate(&self, pattern: &Pattern) -> usize {
1167:         let (perm, lead) = Self::choose(pattern);
1168:         let (lo, hi) = Self::bounds(pattern, perm, lead);
1169:         let base = self.perms[perm as usize].count_in(lo, hi);
1170:         match &self.overlay {
1171:             None => base,
1172:             Some(ov) => {
1173:                 let (add, del) = ov.count_correction(perm, lo, hi);
1174:                 base + add - del
1175:             }
1176:         }
1177:     }
1178: }
1179: 
1180: /// A range of rows in a permutation's column order. Borrowed from the raw index, or


--- FILE engine-capped.txt ---
SOURCE crates/sparq-engine/src/exec.rs; full source SHA256 10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af
LINES 3355-3381
3355:         GraphPattern::Reduced { inner } => eval_modified(graph, local, inner),
3356:         GraphPattern::Slice { inner, start, length } => {
3357:             // LIMIT early-termination: a bare LIMIT (no ORDER BY / DISTINCT /
3358:             // aggregation above it, which all need the full result) can stop after
3359:             // start+length rows instead of materialising the whole relation — a
3360:             // single-pattern scan stops mid-scan, and a conjunctive / UNION /
3361:             // OPTIONAL / Join shape stops at the first seed blocks yielding enough
3362:             // fully-FILTERed rows (`try_capped`, sq-7d3dj.30.8; ASK evaluates as
3363:             // LIMIT 1 through here). LIMIT-without-ORDER-BY is order-insensitive so
3364:             // any rows are valid.
3365:             if let Some(len) = length {
3366:                 if let Some(cap) = start.checked_add(*len) {
3367:                     if let Some(mut b) = try_capped(graph, local, inner, cap)? {
3368:                         slice_bindings(&mut b, *start, *length);
3369:                         return Ok(b);
3370:                     }
3371:                     // Top-k ORDER BY: when the inner pattern (possibly under Project/Reduced)
3372:                     // contains an OrderBy and the budget k = offset + limit is within the
3373:                     // threshold, use bounded selection instead of a full stable sort.
3374:                     // Result is byte-identical to the full-sort+slice path. [SONNET-4.6] sq-7d3dj.30.2
3375:                     if cap <= TOP_K_ORDER_BY_THRESHOLD {
3376:                         if let Some(mut b) = try_topk_orderby(graph, local, inner, cap)? {
3377:                             slice_bindings(&mut b, *start, *length);
3378:                             return Ok(b);
3379:                         }
3380:                     }
3381:                 }
LINES 4289-4346
4289: fn try_capped(
4290:     graph: &Graph,
4291:     local: &mut LocalVocab,
4292:     inner: &GraphPattern,
4293:     cap: usize,
4294: ) -> Result<Option<Bindings>, String> {
4295:     // zk-trace: early termination would consume only part of each scan range,
4296:     // recording a TRUNCATED input set — but the completeness witness (the
4297:     // linear-sweep circuit) must see the whole scan range. Disable the cap
4298:     // while recording; the full path is result-equivalent (LIMIT is
4299:     // order-insensitive without ORDER BY).
4300:     #[cfg(feature = "zk")]
4301:     if crate::zk::enabled() {
4302:         return Ok(None);
4303:     }
4304:     if view::default_is_empty() {
4305:         return Ok(None); // empty-default view: the general path short-circuits at the BGP
4306:     }
4307:     match inner {
4308:         GraphPattern::Project { inner, variables } => {
4309:             Ok(try_capped(graph, local, inner, cap)?.map(|b| project_bindings(b, variables)))
4310:         }
4311:         GraphPattern::Reduced { inner } => try_capped(graph, local, inner, cap),
4312:         p if is_conjunctive(p) => {
4313:             let mut patterns = Vec::new();
4314:             let mut filters = Vec::new();
4315:             flatten_conjunction(p, &mut patterns, &mut filters);
4316:             if patterns.is_empty() {
4317:                 return Ok(None); // the unit relation: nothing to cap
4318:             }
4319:             if patterns.len() == 1 {
4320:                 let (pat_filters, residual) = split_sargable(graph, &patterns, &filters);
4321:                 if residual.is_empty() {
4322:                     // Single pattern, every FILTER pushed into the scan: stop the
4323:                     // SCAN itself at `cap` rows (the cheapest capped form).
4324:                     let (id_pat, pos_vars, unsat) = prepare_pattern(graph, &patterns[0])?;
4325:                     if unsat {
4326:                         return Ok(Some(Bindings::unsorted(collect_vars(&patterns), vec![])));
4327:                     }
4328:                     let filt = pat_filters[0];
4329:                     let sort_col = filt.map(|(c, _)| c);
4330:                     return Ok(Some(scan_to_bindings(
4331:                         graph,
4332:                         &id_pat,
4333:                         &pos_vars,
4334:                         sort_col,
4335:                         filt,
4336:                         Some(cap),
4337:                         #[cfg(feature = "semijoin-bitmap")]
4338:                         None,
4339:                     )));
4340:                 }
4341:             }
4342:             // Multi-pattern (or residual-FILTER) conjunctive shape: the block-driven
4343:             // capped join chain.
4344:             eval_bgp_binary_capped(graph, local, &patterns, &filters, cap)
4345:         }
4346:         GraphPattern::Union { left, right } => {
LINES 4435-4740
4435: 
4436: /// First seed block of the capped conjunctive chain: small enough that an ASK /
4437: /// LIMIT-1 hit in the first block costs a fraction of the full join, large enough
4438: /// to amortise the per-block chain setup.
4439: const CAPPED_SEED_BLOCK: usize = 1024;
4440: /// Second-tier block: one escalation before the remainder is processed whole, so a
4441: /// first-block miss still avoids the full chain when a solution lives within the
4442: /// first ~64k seed rows. Exactly two escalations bound a NO-solution query to
4443: /// three blocks; identical non-bind RHS scans may be reused within a private storage
4444: /// allowance when no budget is armed.
4445: const CAPPED_SEED_BLOCK_2: usize = 65_536;
4446: 
4447: /// Capped conjunctive (BGP + FILTER) evaluation — the ASK / LIMIT-k first-solutions
4448: /// short-circuit THROUGH JOINS (sq-7d3dj.30.8). Runs the same greedy (GOO) binary
4449: /// join chain as `eval_bgp_binary`, but drives it from at most three geometrically
4450: /// growing SLICES of the seed scan (`CAPPED_SEED_BLOCK` rows, then up to
4451: /// `CAPPED_SEED_BLOCK_2`, then the remainder), applying the residual FILTERs per
4452: /// block and stopping as soon as `cap` fully-FILTERed rows exist.
4453: ///
4454: /// SOUNDNESS. Every chain step is per-seed-row local — a join (bind / merge / hash /
4455: /// cross) or a row-wise FILTER over a seed subset yields exactly the full result's
4456: /// rows that originate from that subset — so the blocks are disjoint, their
4457: /// concatenation over the whole seed IS the full result, and stopping early only
4458: /// truncates it (the `try_capped` contract). A row counts toward `cap` only after
4459: /// the WHOLE chain including every residual FILTER, so a first candidate eliminated
4460: /// by a late FILTER never satisfies the cap. The join ORDER is re-derived per block
4461: /// from the planner estimates (a block's size can change the bind-join admission),
4462: /// which affects row order only — the contract is multiset-level and a BGP join is
4463: /// order-independent.
4464: ///
4465: /// Returns `None` (caller falls back to full evaluation) for the shapes the binary
4466: /// chain does not cover: cyclic (WCOJ / LFTJ) BGPs and quoted-triple (RDF 1.2)
4467: /// constraint decompositions.
4468: fn eval_bgp_binary_capped(
4469:     graph: &Graph,
4470:     local: &mut LocalVocab,
4471:     patterns: &[TriplePattern],
4472:     filters: &[Expression],
4473:     cap: usize,
4474: ) -> Result<Option<Bindings>, String> {
4475:     if !bgp_uses_binary(patterns) {
4476:         return Ok(None); // cyclic -> LFTJ: no capped variant (v1 boundary)
4477:     }
4478:     let (_, constraints) = extract_quoted_constraints(patterns);
4479:     if !constraints.is_empty() {
4480:         return Ok(None); // triple-term decomposition: keep the full path
4481:     }
4482:     let (pat_filters, residual) = split_sargable(graph, patterns, filters);
4483:     let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };
4484:     let prepared = prepare_bgp(graph, patterns)?;
4485:     if prepared.iter().any(|p| p.unsatisfiable) {
4486:         return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
4487:     }
4488:     if cap == 0 {
4489:         // Zero rows requested: an empty partial answer is valid under the contract
4490:         // (`|result| >= cap` — the caller's slice truncates to nothing either way).
4491:         return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
4492:     }
4493:     let seed = goo_seed(&prepared);
4494:     let seed_sort_col = goo_seed_sort(&prepared, seed, pfilter(seed).map(|(c, _)| c));
4495:     let seed_all = scan_to_bindings(
4496:         graph,
4497:         &prepared[seed].id_pat,
4498:         &prepared[seed].pos_vars,
4499:         seed_sort_col,
4500:         pfilter(seed),
4501:         None,
4502:         #[cfg(feature = "semijoin-bitmap")]
4503:         None,
4504:     );
4505: 
4506:     #[cfg(test)]
4507:     capped_rhs_tests::observe(0);
4508:     // [GPT-6 Astra] Query-local and lazy: never scan an unreached step. Pattern
4509:     // filters are immutable here; each slot also keys the requested scan order.
4510:     // Armed budgets retain the old per-step lifetime and polling: their working-set
4511:     // estimate does not account for multiple retained RHS relations.
4512:     let reuse_rhs = !budget::active();
4513:     let (mut rhs_cache, mut rhs_remaining) = capped_rhs_cache(prepared.len(), reuse_rhs);
4514:     let mut acc: Option<Bindings> = None;
4515:     let mut start = 0usize;
4516:     while start < seed_all.rows.len() {
4517:         #[cfg(test)]
4518:         capped_rhs_tests::observe(1);
4519:         let end = if start == 0 {
4520:             CAPPED_SEED_BLOCK.min(seed_all.rows.len())
4521:         } else if start == CAPPED_SEED_BLOCK {
4522:             CAPPED_SEED_BLOCK_2.min(seed_all.rows.len())
4523:         } else {
4524:             seed_all.rows.len()
4525:         };
4526:         // One seed slice through the whole chain. A contiguous slice of a sorted
4527:         // scan stays sorted, so merge-join eligibility is preserved.
4528:         let mut result = Bindings {
4529:             vars: seed_all.vars.clone(),
4530:             rows: seed_all.rows[start..end].to_vec(),
4531:             sorted_by: seed_all.sorted_by.clone(),
4532:         };
4533:         let mut cs_ctx = CsCtx::new(&prepared);
4534:         let mut done = vec![false; prepared.len()];
4535:         done[seed] = true;
4536:         cs_ctx.note_done(seed);
4537:         let mut cur_card = prepared[seed].est as f64;
4538:         let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
4539:         record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);
4540:         for _ in 1..prepared.len() {
4541:             let (i, new_card, _connected) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
4542:             cur_card = new_card;
4543:             done[i] = true;
4544:             cs_ctx.note_done(i);
4545:             // Same step selection as `eval_bgp_binary`: bind-join when the running
4546:             // block is much smaller than the pattern; else merge / hash / cross.
4547:             let connecting: Vec<Variable> =
4548:                 result.vars.iter().filter(|v| prepared[i].var_pos(v).is_some()).cloned().collect();
4549:             if connecting.len() == 1
4550:                 && distinct_pattern_vars(&prepared[i].pos_vars)
4551:                 && result.rows.len().saturating_mul(8) < prepared[i].est
4552:             {
4553:                 let jv = &connecting[0];
4554:                 let rk = result.col(jv).unwrap();
4555:                 let pp = prepared[i].var_pos(jv).unwrap();
4556:                 #[cfg(test)]
4557:                 capped_rhs_tests::observe(3);
4558:                 #[cfg(test)]
4559:                 capped_rhs_tests::step(start, i, "bind", None, false, None);
4560:                 result = bind_join(graph, result, &prepared[i].id_pat, &prepared[i].pos_vars, rk, pp, pfilter(i));
4561:             } else {
4562:                 let filt = pfilter(i);
4563:                 let merge_var = result.sorted_by.clone().filter(|sv| prepared[i].var_pos(sv).is_some());
4564:                 let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().map(|jv| prepared[i].var_pos(jv).unwrap()));
4565:                 let mut uncached = None;
4566:                 let mut spare_slot = None;
4567:                 let slot = rhs_cache.get_mut(i).unwrap_or(&mut spare_slot);
4568:                 #[cfg(test)]
4569:                 let mut scanned = false;
4570:                 let rhs = capped_rhs(slot, &mut rhs_remaining, &mut uncached, scan_sort, || {
4571:                     #[cfg(test)]
4572:                     {
4573:                         capped_rhs_tests::observe(2);
4574:                         scanned = true;
4575:                     }
4576:                     scan_to_bindings(
4577:                         graph,
4578:                         &prepared[i].id_pat,
4579:                         &prepared[i].pos_vars,
4580:                         scan_sort,
4581:                         filt,
4582:                         None,
4583:                         #[cfg(feature = "semijoin-bitmap")]
4584:                         None,
4585:                     )
4586:                 });
4587:                 let connected = prepared[i].pos_vars.iter().flatten().any(|v| result.vars.contains(v));
4588:                 if let Some(jv) = merge_var.filter(|jv| rhs.sorted_by.as_ref() == Some(jv)) {
4589:                     #[cfg(test)]
4590:                     capped_rhs_tests::step(start, i, "merge", scan_sort, scanned, rhs.sorted_by.as_ref());
4591:                     result = merge_join_ref(&result, rhs, &jv);
4592:                 } else if connected {
4593:                     #[cfg(test)]
4594:                     capped_rhs_tests::step(start, i, "hash", scan_sort, scanned, rhs.sorted_by.as_ref());
4595:                     result = hash_join_ref(&result, rhs);
4596:                 } else {
4597:                     #[cfg(test)]
4598:                     capped_rhs_tests::step(start, i, "cross", scan_sort, scanned, rhs.sorted_by.as_ref());
4599:                     result = cross_product_ref(&result, rhs);
4600:                 }
4601:             }
4602:             record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
4603:             if result.rows.is_empty() {
4604:                 break;
4605:             }
4606:         }
4607:         // Residual FILTERs per block — a row only counts toward the cap once it has
4608:         // passed EVERY filter (the late-FILTER safety property: a partial solution
4609:         // never fires the early exit).
4610:         // [FABLE-5] (sq-1ivw7) Install the snapshot-aware non-literal column set (indexing THIS
4611:         // block's `result` layout — the BGP variables) so the id fast path fires on capped fused
4612:         // BGP+FILTER shapes too. Drain-safe as in `eval_flat_conjunctive`.
4613:         #[cfg(feature = "id-filter-fastpath")]
4614:         let idfast_cols = {
4615:             let bgp = GraphPattern::Bgp { patterns: patterns.to_vec() };
4616:             nonliteral_filter_cols(graph, &bgp, &result)
4617:         };
4618:         for f in &residual {
4619:             if result.rows.is_empty() {
4620:                 break;
4621:             }
4622:             #[cfg(feature = "id-filter-fastpath")]
4623:             with_idfast_nonlit_cols(idfast_cols.clone(), || apply_filter(graph, local, &mut result, f))?;
4624:             #[cfg(not(feature = "id-filter-fastpath"))]
4625:             apply_filter(graph, local, &mut result, f)?;
4626:         }
4627:         let merged = match acc.take() {
4628:             None => result,
4629:             Some(a) => union_bindings(a, result),
4630:         };
4631:         budget::check(merged.rows.len())?;
4632:         let satisfied = merged.rows.len() >= cap;
4633:         acc = Some(merged);
4634:         if satisfied {
4635:             break;
4636:         }
4637:         start = end;
4638:     }
4639:     Ok(Some(acc.unwrap_or_else(|| Bindings::unsorted(collect_vars(patterns), vec![]))))
4640: }
4641: 
4642: // [GPT-6 Astra] Conservative private allowance for retained scan storage, not a
4643: // public QueryBudget or a limit on transient join/scan allocations. Keep room below
4644: // the diagnostic's extra-heap rejection threshold; do not retain one RHS per pattern.
4645: const CAPPED_RHS_STORAGE: usize = 4 * 1024 * 1024;
4646: // Requested order, immutable scan relation, and its charged allocated storage.
4647: type CappedRhs = (Option<usize>, Bindings, usize);
4648: 
4649: fn capped_rhs_cache(len: usize, enabled: bool) -> (Vec<Option<CappedRhs>>, usize) {
4650:     let mut slots = Vec::new();
4651:     if !enabled
4652:         || len
4653:             .checked_mul(std::mem::size_of::<Option<CappedRhs>>())
4654:             .is_none_or(|bytes| bytes > CAPPED_RHS_STORAGE)
4655:         || slots.try_reserve_exact(len).is_err()
4656:     {
4657:         return (slots, 0);
4658:     }
4659:     let Some(bytes) = slots
4660:         .capacity()
4661:         .checked_mul(std::mem::size_of::<Option<CappedRhs>>())
4662:         .filter(|&bytes| bytes <= CAPPED_RHS_STORAGE)
4663:     else {
4664:         return (Vec::new(), 0);
4665:     };
4666:     slots.resize_with(len, || None);
4667:     (slots, CAPPED_RHS_STORAGE - bytes)
4668: }
4669: 
4670: // [GPT-6 Astra] Count capacities, not planner estimates or populated lengths.
4671: // Scan rows have at most three ids and fit inline; decline an unproved spilled
4672: // representation. Variable owns a String, whose capacity is exposed by its safe
4673: // consuming API; move it out and back without cloning or allocating its text.
4674: fn capped_rhs_storage(rhs: &mut Bindings, allowance: usize) -> Option<usize> {
4675:     let mut bytes = rhs
4676:         .rows
4677:         .capacity()
4678:         .checked_mul(std::mem::size_of::<Row>())?
4679:         .checked_add(
4680:             rhs.vars
4681:                 .capacity()
4682:                 .checked_mul(std::mem::size_of::<Variable>())?,
4683:         )?;
4684:     if bytes > allowance || rhs.rows.iter().any(Row::spilled) {
4685:         return None;
4686:     }
4687:     for variable in rhs.vars.iter_mut().chain(rhs.sorted_by.iter_mut()) {
4688:         let name =
4689:             std::mem::replace(variable, Variable::new_unchecked(String::new())).into_string();
4690:         let capacity = name.capacity();
4691:         *variable = Variable::new_unchecked(name);
4692:         bytes = bytes.checked_add(capacity)?;
4693:         if bytes > allowance {
4694:             return None;
4695:         }
4696:     }
4697:     Some(bytes)
4698: }
4699: 
4700: // [GPT-6 Astra] Reuse requires a pure scan of the same immutable prepared pattern,
4701: // filters and requested order. Actual sorted_by remains the scan's truthful value.
4702: // Fitting entries live until order replacement/query exit; non-fitting entries live
4703: // only in the caller's per-step scratch. Allocator metadata is outside this allowance.
4704: fn capped_rhs<'a>(
4705:     slot: &'a mut Option<CappedRhs>,
4706:     remaining: &mut usize,
4707:     uncached: &'a mut Option<Bindings>,
4708:     sort: Option<usize>,
4709:     scan: impl FnOnce() -> Bindings,
4710: ) -> &'a Bindings {
4711:     if slot
4712:         .as_ref()
4713:         .is_none_or(|(cached_sort, _, _)| *cached_sort != sort)
4714:     {
4715:         if let Some(old) = slot.take() {
4716:             *remaining += old.2;
4717:             drop(old); // release stale ownership/accounting before its replacement scan
4718:         }
4719:         let mut rhs = scan();
4720:         if let Some(bytes) = capped_rhs_storage(&mut rhs, *remaining) {
4721:             *remaining -= bytes;
4722:             *slot = Some((sort, rhs, bytes));
4723:         } else {
4724:             *uncached = Some(rhs);
4725:             return uncached.as_ref().unwrap();
4726:         }
4727:     }
4728:     &slot.as_ref().unwrap().1
4729: }
4730: 
4731: /// Distinct (non-repeated) variable positions of a prepared pattern, or `None` if
4732: /// a variable repeats (e.g. `?x p ?x`), which would make range counts over-count.
4733: pub(crate) fn distinct_pattern_vars(pos_vars: &[Option<Variable>; 3]) -> bool {
4734:     let vars: Vec<&Variable> = pos_vars.iter().flatten().collect();
4735:     let mut sorted = vars.clone();
4736:     sorted.sort();
4737:     sorted.dedup();
4738:     sorted.len() == vars.len()
4739: }
4740: 


--- FILE hash-build-probe.txt ---
SOURCE crates/sparq-engine/src/exec.rs; full source SHA256 10090299f707dbf4adb4cae6349bd390a06ed2a50645e8c3417bcc3b90fb20af
LINES 9066-9154
9066: fn hash_join(left: Bindings, right: Bindings) -> Bindings {
9067:     hash_join_ref(&left, &right)
9068: }
9069: 
9070: // [GPT-6 Astra] Borrow rows so the capped caller can reuse its RHS without cloning it.
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
9152: /// Index-nested-loop join of a (small) `result` with a single triple pattern on one
9153: /// shared variable: groups the result by the join value, and for each distinct value
9154: /// looks up the pattern's matches with that variable BOUND (a binary-search range on a
SOURCE crates/sparq-substrate/src/join.rs; full source SHA256 cd708a74d9f77a9c4ab2e519a1d6475c4863fb68f909889d2d62d8e1b9cefae5
LINES 96-141
96: pub struct JoinKeys {
97:     /// `(left_col, right_col)` index pairs whose ids form the equi-join key — the
98:     /// columns hashed / merged on. For a hash join these are split into the build
99:     /// and probe key projections; for a merge join this is the single sorted
100:     /// variable. Build and probe must agree on the order of these pairs so the key
101:     /// tuples line up.
102:     pub key_cols: Vec<(usize, usize)>,
103:     /// The right-side columns appended (in order) after the left row's columns to
104:     /// form the combined output row.
105:     pub right_only: Vec<usize>,
106: }
107: 
108: impl JoinKeys {
109:     /// The left-side key column indices, in `key_cols` order.
110:     ///
111:     /// **Single-column fast path ([OPUS-4.8] sq-4r8uy).** The overwhelmingly
112:     /// common join is on ONE variable (`key_cols.len() == 1`). For it, building the
113:     /// [`Key`] by iterating the heap `key_cols` `Vec` and running the general
114:     /// `SmallVec::from_iter` collect machinery costs measurably more than a direct
115:     /// one-element push — the descriptor's per-row projection was the whole measured
116:     /// `hash_probe` overhead over a hand-specialised single-column probe (the #1810
117:     /// canonical delta). This branch special-cases it to `single_key`, which
118:     /// pushes exactly the one projected id, and falls through to the general
119:     /// `iter().collect()` for multi-column keys (byte-identical to the old path).
120:     /// The result is IDENTICAL to the general path in every case — same length,
121:     /// same ids, same order — so the join semantics are unchanged; only the
122:     /// single-column key derivation is cheaper.
123:     #[inline]
124:     pub fn left_key(&self, row: &[Id]) -> Key {
125:         if let [(lc, _)] = self.key_cols.as_slice() {
126:             return single_key(row[*lc]);
127:         }
128:         self.key_cols.iter().map(|&(lc, _)| row[lc]).collect()
129:     }
130: 
131:     /// The right-side key column indices, in `key_cols` order — projected to the
132:     /// SAME key tuple shape as [`left_key`](JoinKeys::left_key) so build and probe
133:     /// keys are equal exactly when the join columns are equal.
134:     ///
135:     /// Carries the same single-column fast path as [`left_key`](JoinKeys::left_key)
136:     /// ([OPUS-4.8] sq-4r8uy) — for `key_cols.len() == 1` it pushes exactly the one
137:     /// projected right-side id via `single_key` rather than running the general
138:     /// `iter().collect()`. The single-column result is IDENTICAL to the general
139:     /// path, so a build key and a probe key still compare equal exactly when the
140:     /// join columns agree — the hash-join correctness invariant is preserved.
141:     #[inline]
LINES 180-224
180: 
181: /// Select the serial table or the radix partition for a precomputed key hash.
182: ///
183: /// Converting the documented 64-partition shape to an array reference gives LLVM
184: /// a statically-known bound; the masked partition index therefore needs no slice
185: /// bounds check. The fallback preserves the former behavior for malformed or
186: /// non-standard table slices.
187: #[inline(always)]
188: fn probe_table(tables: &[JoinTable], hash: u64) -> &JoinTable {
189:     if let [table] = tables {
190:         return table;
191:     }
192:     let partition = (hash % JOIN_PARTS as u64) as usize;
193:     if let Ok(partitioned) = <&[JoinTable; JOIN_PARTS]>::try_from(tables) {
194:         return &partitioned[partition];
195:     }
196:     &tables[partition]
197: }
198: 
199: /// The partition/lookup hash for a join key — build and probe must agree on it.
200: #[inline]
201: pub fn key_hash(key: &Key) -> u64 {
202:     use std::hash::{Hash, Hasher};
203:     let mut h = rustc_hash::FxHasher::default();
204:     key.hash(&mut h);
205:     h.finish()
206: }
207: 
208: /// Number of radix partitions for the parallel hash-join build. 64 spreads well at
209: /// high thread counts while keeping the per-partition tag-scan cheap.
210: pub const JOIN_PARTS: usize = 64;
211: 
212: /// The hash table type for hash-join build-side storage: a [`hashbrown::HashMap`] keyed on
213: /// [`Key`] (the equi-join projection), mapping to the sorted posting list of matching
214: /// build-row indices, backed by [`FxBuildHasher`].
215: ///
216: /// Using `hashbrown::HashMap` (rather than `std::collections::HashMap`) lets [`probe_emit`]
217: /// and [`probe_gather_indices`] call `raw_entry().from_hash` to skip the second internal
218: /// re-hash on the partitioned probe path — a single [`key_hash`] call derives the radix
219: /// partition **and** performs the table lookup. The `FxBuildHasher` keeps the stored hash
220: /// identical to the explicit [`key_hash`] computation so the precomputed hash hits the
221: /// correct bucket every time.
222: ///
223: /// [SONNET-4.6] sq-7d3dj.19
224: pub type JoinTable = HashMap<Key, Posting, FxBuildHasher>;
LINES 287-354
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
322: 
323: /// Emits, for one probe row, every combined output row (its build-side matches
324: /// from `tables`, each extended with the probe-only columns). Shared by the serial
325: /// and parallel probe paths so they are byte-identical. `tables` is either one
326: /// serial [`JoinTable`] or `JOIN_PARTS` radix partitions; `probe_only` are the probe
327: /// columns appended after the build row.
328: ///
329: /// **Single-hash optimisation ([SONNET-4.6] sq-7d3dj.19):** the join key is hashed
330: /// ONCE via [`key_hash`]; the hash bits select the radix partition and
331: /// `raw_entry().from_hash` performs the table lookup without a second internal
332: /// re-hash. `out.reserve(matches.len())` allocates for all matches in one call —
333: /// the **batch-emission contract** the sq-pntvh M4 morsel pipeline inherits:
334: /// reserve then materialise once per key group, not per match.
335: #[inline]
336: pub fn probe_emit(
337:     prow: &Row,
338:     keys: &JoinKeys,
339:     build: &[Row],
340:     tables: &[JoinTable],
341:     probe_only: &[usize],
342:     out: &mut Vec<Row>,
343: ) {
344:     let key: Key = keys.right_key(prow);
345:     // [SONNET-4.6] sq-7d3dj.19: hash ONCE — partition selection and raw_entry lookup
346:     // share this value; no second internal re-hash on the partitioned probe path.
347:     let h = key_hash(&key);
348:     let table = probe_table(tables, h);
349:     let Some((_, matches)) = table.raw_entry().from_hash(h, |k| probe_key_eq(k, &key)) else {
350:         return;
351:     };
352:     // Reserve exact match count — one allocation for the entire key group.
353:     // Batch-emission contract (sq-pntvh M4): materialise after reserve, not per match.
354:     // [SONNET-4.6] sq-7d3dj.19
LINES 391-416
391: 
392: /// Serial **hash join** probe: for every probe row, emit its build-side matches.
393: /// `budget` is polled once per probe row (the engine's pre-move per-row check).
394: /// The build table(s) and the layout are the caller's; this is the probe hot loop.
395: /// Delegates per-row emission to [`probe_emit`] (single-hash + batch-reserve).
396: #[inline]
397: pub fn hash_probe_serial<B: Budget>(
398:     probe: &[Row],
399:     keys: &JoinKeys,
400:     build: &[Row],
401:     tables: &[JoinTable],
402:     probe_only: &[usize],
403:     budget: &B,
404:     out: &mut Vec<Row>,
405: ) {
406:     for prow in probe {
407:         // Coarse budget check once per probe row.
408:         if budget.exhausted(out.len()) {
409:             break;
410:         }
411:         probe_emit(prow, keys, build, tables, probe_only, out);
412:     }
413: }
414: 
415: /// Index-nested-loop **bind join** combine step: given the groups of result rows
416: /// keyed by the join value and, for one distinct value, the projected new-variable
SOURCE crates/sparq-substrate/src/rows.rs; full source SHA256 490d760db1825e85a08b7434725ffa0ba5d9c62adaaaa8f537442d7d48f42de0
LINES 47-66
47: /// common case) so a join produces no heap allocation per row — the dominant cost on large
48: /// join results.
49: ///
50: /// Mirrors the engine's private `exec::Row` byte for byte (`SmallVec<[Id; 4]>`) so the
51: /// later join-kernel move is a pure code-move.
52: pub type Row = SmallVec<[Id; 4]>;
53: 
54: /// A join / group key (the ids of the shared or grouping columns). Inlined up to 2
55: /// columns — most joins are on one or two variables — so building a hash table or probing
56: /// it allocates nothing per key.
57: ///
58: /// Mirrors the engine's private `exec::Key` (`SmallVec<[Id; 2]>`).
59: pub type Key = SmallVec<[Id; 2]>;
60: 
61: /// A hash-table posting list (row indices sharing a key). Inlined up to 2 — many join keys
62: /// (and almost all OPTIONAL keys) match only one or two rows — so the build allocates
63: /// nothing per bucket in the common case.
64: ///
65: /// Mirrors the engine's private `exec::Posting` (`SmallVec<[usize; 2]>`).
66: pub type Posting = SmallVec<[usize; 2]>;


--- FILE accounting-dependencies.txt ---
SOURCE Cargo.toml; full source SHA256 9f820551d15c1430a25172c831f491fb7fd92231c15f124b2a048b1137d8ce73
LINES 28-37
28: [patch.crates-io]
29: spargebra = { path = "vendor/spargebra" }
30: 
31: [workspace.package]
32: version = "0.1.1"
33: edition = "2021"
34: license = "MIT"
35: # [OPUS-4.8] (sq-qmth) MSRV floor 1.88. Upstream-driven: `geo@0.33.1` (a transitive dep
36: # of sparq-geo) requires rustc 1.88, and the released oxigraph parser stack
37: # (oxrdf/oxttl/oxrdfio/spargebra/… 0.2–0.4) requires 1.87. Verified empirically: the
LINES 65-73
65: # Fast hashing / maps. These are the NEWEST side of a duplicate pair in Cargo.lock;
66: # the lagging copies are held by third-party crates. Blockers are surveyed once, in
67: # `deny.toml` § [bans] (the duplicate-version blocker registry) — read that before
68: # re-investigating a `multiple-versions` warning.
69: rustc-hash = "2"
70: hashbrown = "0.17"
71: # Inline small rows (avoid a heap allocation per solution row).
72: smallvec = "1"
73: # Parallelism.
SOURCE Cargo.lock; full source SHA256 6d7b6095f5fec01e4ace906d6001687c67d83fedc1e50180891a419c22026152
LINES 2028-2039
2028: [[package]]
2029: name = "hashbrown"
2030: version = "0.17.1"
2031: source = "registry+https://github.com/rust-lang/crates.io-index"
2032: checksum = "ed5909b6e89a2db4456e54cd5f673791d7eca6732202bbf2a9cc504fe2f9b84a"
2033: dependencies = [
2034:  "allocator-api2",
2035:  "equivalent",
2036:  "foldhash 0.2.0",
2037: ]
2038: 
2039: [[package]]
LINES 4782-4788
4782: [[package]]
4783: name = "smallvec"
4784: version = "1.15.2"
4785: source = "registry+https://github.com/rust-lang/crates.io-index"
4786: checksum = "8ed6a63f02c8539c91a8685a86f4099661ba3da017932f6ebbea6de3f0fa7c90"
4787: 
4788: [[package]]
SOURCE dependency:hashbrown-0.17.1/src/map.rs; full source SHA256 b79497ce537ffc5ed4f8f3399434b9216c01e7927fdc434fee190e9e9ce2abb0
LINES 1999-2010
1999: 
2000:     /// Returns the total amount of memory allocated internally by the hash
2001:     /// set, in bytes.
2002:     ///
2003:     /// The returned number is informational only. It is intended to be
2004:     /// primarily used for memory profiling.
2005:     #[inline]
2006:     pub fn allocation_size(&self) -> usize {
2007:         self.table.allocation_size()
2008:     }
2009: }
2010: 
SOURCE dependency:hashbrown-0.17.1/src/raw.rs; full source SHA256 0c8ad353ba95817e72b0a8fea48fa2599099ea3def374f254ab6402a9c468d22
LINES 695-707
695: 
696:     /// Returns the total amount of memory allocated internally by the hash
697:     /// table, in bytes.
698:     ///
699:     /// The returned number is informational only. It is intended to be
700:     /// primarily used for memory profiling.
701:     #[inline]
702:     pub(crate) fn allocation_size(&self) -> usize {
703:         // SAFETY: We use the same `table_layout` that was used to allocate
704:         // this table.
705:         unsafe { self.table.allocation_size_or_zero(Self::TABLE_LAYOUT) }
706:     }
707: 
LINES 3169-3181
3169:     #[inline]
3170:     unsafe fn allocation_size_or_zero(&self, table_layout: TableLayout) -> usize {
3171:         if self.is_empty_singleton() {
3172:             0
3173:         } else {
3174:             // SAFETY:
3175:             // 1. We have checked that our table is allocated.
3176:             // 2. The caller ensures that `table_layout` matches the [`TableLayout`]
3177:             // that was used to allocate this table.
3178:             unsafe { self.allocation_info(table_layout).1.size() }
3179:         }
3180:     }
3181: 
SOURCE dependency:smallvec-1.15.2/src/lib.rs; full source SHA256 a779bc485235ccb746b680be71d57b707865341209a585b53e63a91c4bda6cae
LINES 988-1003
988:     pub fn capacity(&self) -> usize {
989:         self.triple().2
990:     }
991: 
992:     /// Returns a tuple with (data ptr, len, capacity)
993:     /// Useful to get all `SmallVec` properties with a single check of the current storage variant.
994:     #[inline]
995:     fn triple(&self) -> (ConstNonNull<A::Item>, usize, usize) {
996:         unsafe {
997:             if self.spilled() {
998:                 let (ptr, len) = self.data.heap();
999:                 (ptr, len, self.capacity)
1000:             } else {
1001:                 (self.data.inline(), self.capacity, Self::inline_capacity())
1002:             }
1003:         }
LINES 1022-1028
1022: 
1023:     /// Returns `true` if the data has spilled into a separate heap-allocated buffer.
1024:     #[inline]
1025:     pub fn spilled(&self) -> bool {
1026:         self.capacity > Self::inline_capacity()
1027:     }
1028: 
