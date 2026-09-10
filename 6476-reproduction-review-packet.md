[GPT-6 Astra] Issue6476: confirmed public-callback failure and restoration design. No production patch.


{
  "head": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
  "verdict": "CONFIRMED: nested public query erases the outer cancellation budget",
  "runtime_results": [
    {
      "inner": "none",
      "enforced": true,
      "result": "cancelled",
      "control": true,
      "callback_calls": 1,
      "same_installing_thread": true,
      "outer_flag_true": true,
      "subsequent_top_level_ask": true
    },
    {
      "inner": "unlimited ASK {}",
      "enforced": false,
      "result": "one row containing integer 7",
      "control": false,
      "callback_calls": 1,
      "same_installing_thread": true,
      "outer_flag_true": true,
      "subsequent_top_level_ask": true
    },
    {
      "inner": "ASK {} with explicit max_rows=1",
      "enforced": false,
      "result": "one row containing integer 7",
      "control": false,
      "callback_calls": 1,
      "same_installing_thread": true,
      "outer_flag_true": true,
      "subsequent_top_level_ask": true
    },
    {
      "inner": "parse rejection before install",
      "enforced": true,
      "result": "cancelled",
      "control": true,
      "callback_calls": 1,
      "same_installing_thread": true,
      "outer_flag_true": true,
      "subsequent_top_level_ask": true
    }
  ],
  "execution": "Four deterministic cases in observation mode (exit0); same four again with required-restoration invariant (expected exit101). Both successful nested cases violate restoration. No-nesting and parse-error controls prove actual post-callback polling. No timing assertions or private production instrumentation.",
  "state_restoration_requirements": [
    "Save and restore all Limits fields: on, deadline, row/byte maxima, byte_width, extra_bytes, cancel pointer; do not restore only original public QueryBudget values.",
    "Save and restore the prior EXCEEDED reason exactly. A handled child error is child-local; it must not erase or replace a previously sticky outer reason.",
    "Restored outer deadline and cancellation are reevaluated on next poll; elapsed time or flag changes during nested execution must remain observable.",
    "Error and unwind restore the enclosing state; entry from idle returns to OFF/None. A caught inner unwind must restore outer state before the callback resumes. Abort has no continuation/drop guarantee.",
    "Preserve thread affinity and owning-budget lifetimes, including scoped Rayon joins; do not assume PhantomData of the child budget also protects a saved parent pointer."
  ],
  "guard_safety_assessment": {
    "visibility": "lib.rs:26 makes exec private to external users. Within the crate, exec::budget, Guard, install, Limits and snapshot are pub(crate). Guard has private fields and lifetime/!Send markers, but install returns it by value.",
    "external_boundary": "External safe callers cannot name/install/drop budget guards or obtain Limits through the public API. The inspected 14 production installers are synchronous functions with local guards and concrete owned Result return types; callbacks receive no guard. Their returns, early errors and unwinds therefore currently nest dynamically. No guard escape, explicit early drop or forget was found at those installation sites.",
    "internal_boundary": "The crate-visible safe install API DOES NOT enforce LIFO. Guard<child> borrows only the child QueryBudget, not the outer guard/budget. A future safe internal caller can drop the outer guard and budget before dropping the inner guard. Therefore a naive saved-Limits field is not by itself a lifetime proof.",
    "counterexample_not_executed": "In a crate-internal caller: create outer budget owning its only cancel Arc; g1=install(&outer); g2=install(&inner); drop(g1); drop(outer); drop(g2); budget::check(0). A naive g2 restoration would reinstall the now-dangling outer CancelPtr. Both guards lifetimes satisfy their own borrowed budgets. This is a source/type argument, not an executed UB test or a claim of current externally reachable UAF.",
    "other_internal_hazard": "Even before the proposed patch, crate-internal safe code could retain the lifetime-free Copy Limits snapshot beyond guard/budget drop, or forget the guard. Current external API does not expose these operations. Do not silently advertise the existing crate-visible raw-pointer interfaces as intrinsically sound for arbitrary internal use.",
    "snapshot_actual_callers": [
      {
        "line": 2553,
        "operation": "parallel_json_fanout snapshot; par_chunks map collects owned fragments before returning"
      },
      {
        "line": 2755,
        "operation": "snapshot; par_chunks map collects owned strings before returning"
      },
      {
        "line": 8819,
        "operation": "EngineSnapshot; parallel fold/reduce returns owned rows before guard exit"
      },
      {
        "line": 9784,
        "operation": "snapshot; par_iter collect<Result<Vec<bool>>> joins before return, including errors"
      }
    ],
    "snapshot_scope_limit": "Inspected current snapshot callers keep borrowed cancellation pointers inside synchronous Rayon operations; no detached task or Limits return is present. This call-site audit supports current execution but does not make the lifetime-free snapshot signature safe against arbitrary future internal escape.",
    "recommended_minimal_boundary": "Do not ship only saved raw Limits under the current broadly callable returning-Guard interface. Prefer a private Guard/install within budget plus a scoped pub(crate) with_budget(b, closure) wrapper, migrating the 14 install sites and direct tests. It prevents other modules from early-dropping/forgetting the guard and gives restoration an enforced dynamic scope. Save/restore both state values inside that sealed wrapper; no new heap stack or public API is needed.",
    "unresolved_for_review": "The independent review must explicitly settle the snapshot boundary too: either accept/document and narrow the existing four audited synchronous internal consumers as part of the trusted implementation boundary, or use a lifetime-scoped snapshot interface/owned cancellation handle so a safe internal caller cannot retain a raw snapshot. Merely noting lexical current callers is not a type-level proof. A broader Arc/TLS ownership redesign is not implemented or assumed necessary here.",
    "alternative_limit": "Adding an Arc only to the restoring guard is insufficient if the restored TLS raw pointer outlives that guard. An owning-state solution must keep the active cancellation allocation alive in TLS and address stale outer restoration under non-LIFO drops, not just clone a parent Arc temporarily."
  },
  "direct_production_install_sites": [
    {
      "entry": "query_prepared_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1036
    },
    {
      "entry": "ask_prepared_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1073
    },
    {
      "entry": "query_json_prepared_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1109
    },
    {
      "entry": "query_json_chunks_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1134
    },
    {
      "entry": "query_json_stream_prepared_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1189
    },
    {
      "entry": "count_prepared_with_budget",
      "file": "crates/sparq-engine/src/lib.rs",
      "line": 1227
    },
    {
      "entry": "ResultCache::get_or_eval -> private eval on miss/noncacheable path",
      "file": "crates/sparq-engine/src/cache.rs",
      "line": 368
    },
    {
      "entry": "construct_prepared_with_budget",
      "file": "crates/sparq-engine/src/construct.rs",
      "line": 60
    },
    {
      "entry": "describe_prepared_with_budget",
      "file": "crates/sparq-engine/src/construct.rs",
      "line": 96
    },
    {
      "entry": "construct_or_describe_with_budget",
      "file": "crates/sparq-engine/src/construct.rs",
      "line": 123
    },
    {
      "entry": "explain_analyze_with_budget",
      "file": "crates/sparq-engine/src/explain.rs",
      "line": 93
    },
    {
      "entry": "explain_plan_analyze_with_budget",
      "file": "crates/sparq-engine/src/explain_json.rs",
      "line": 222
    },
    {
      "entry": "params PreparedUpdate -> update_in_place_prepared_with_budget",
      "file": "crates/sparq-engine/src/update.rs",
      "line": 535
    },
    {
      "entry": "update_in_place_with_budget/capturing/atomic -> update_in_place_core",
      "file": "crates/sparq-engine/src/update.rs",
      "line": 728
    }
  ],
  "next_validation": [
    "Preserve exact public cancellation witness with all four cases and require restoration in both successful nested cases.",
    "At the chosen safe scoped seam, test distinct nested limits, prior sticky errors, child errors isolated from parent, byte width/local-vocab accumulation, three levels, return/error/unwind and idle cleanup.",
    "Test both live cancel identities and outer deadline changes without sleeps; retain thread restrictions and scoped worker lifetime proof.",
    "Meaningful controls: omit prior Limits restoration -> public witness fails; omit EXCEEDED restoration -> sticky-error test fails; omit byte fields -> exact accounting test fails.",
    "For sealed installation, add compile-fail/API-visibility evidence that other modules cannot acquire/forget/drop a guard; do not execute deliberately dangling-pointer code."
  ],
  "behavioral_scope": "Restoration repairs the outer state after a nested invocation. It does not enforce a composite/global budget inside an explicitly separate inner call, nor make arbitrary callback work cancellable.",
  "provenance": {
    "tree": "cd47ccfcb442f46723d1ed16fe1bf1c279fed05e",
    "current_commit": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
    "cached_library_commit": "ed66ef0931fa19dd521fac433870c86a78687a30",
    "whole_tree_byte_identical": true,
    "method": "Standalone harness compiled with installed rustc1.97.1 against cached unmodified production release rlibs. Cache engine hash and dep-info match the frozen bounded-many-timing provenance; no fresh production rebuild under merged commit name.",
    "engine_features": "[\"default\", \"digest\", \"parallel\", \"regex\"]",
    "core_features": "[\"default\", \"parallel\"]",
    "engine_rlib_sha256": "2135748c75d5afd1ca3731e6d350c5a06b99caf9376b1388fbd472b08febed9d",
    "harness_sha256": "88eb9cab7dce8b37e5418ca1bc285678435d0a93b299e5f77161c25f5faa48cc",
    "binary_sha256": "515c9500cccc2920eca79aae7a674799869254dcfec6cc8b3ffc88d5ec4da5ab",
    "environment_overrides": {
      "CARGO_NET_OFFLINE": "true",
      "CARGO_INCREMENTAL": "0",
      "CARGO_BUILD_JOBS": "2",
      "RAYON_NUM_THREADS": "1"
    },
    "compiler_warning": "Only the multiple emit-type output-file-name adaptation warning; stderr preserved."
  },
  "limits": [
    "No production repair, commit, Cargo/dependency/workspace build, wasm/Miri/full feature suite, network request, model call, remote mutation, bd command or cleanup.",
    "No pointer misuse sequence was compiled/executed; no current external memory-unsafety claim.",
    "Deadline/sticky/accounting/unwind coverage remains planned rather than measured in this phase.",
    "All completed runtime evidence is outside the regenerable cache. Previous frozen bundles and completed issue3105 worktree were not edited."
  ]
}


Exact public harness:
```rust
// [GPT-6 Astra] Public-callback reproduction for issue #6476; no production instrumentation.
use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_engine::{query, query_with_budget, query_with_functions_and_budget, FunctionRegistry, QueryBudget};
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

#[derive(Clone, Copy, Debug)]
enum Inner { None, Unlimited, ExplicitBudget, ParseError }

fn run(inner: Inner) -> bool {
    let graph = Graph::load_str("", "turtle").expect("empty graph fixture");
    let nested = Graph::load_str("", "turtle").expect("empty nested fixture");
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_cancel = Arc::clone(&cancel);
    let callback_calls = Arc::clone(&calls);
    let thread = std::thread::current().id();
    let mut registry = FunctionRegistry::new();
    registry.register("urn:nested", move |_| {
        assert_eq!(std::thread::current().id(), thread, "witness must run on the installing thread");
        callback_calls.fetch_add(1, Ordering::Relaxed);
        match inner {
            Inner::None => {},
            Inner::Unlimited => assert_eq!(query(&nested, "ASK {}").unwrap().rows.len(), 1),
            Inner::ExplicitBudget => {
                let budget = QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() };
                assert_eq!(query_with_budget(&nested, "ASK {}", &budget).unwrap().rows.len(), 1);
            },
            Inner::ParseError => assert!(query(&nested, "not a SPARQL query").is_err()),
        }
        callback_cancel.store(true, Ordering::Relaxed);
        Ok(Term::Literal(Literal::from(7)))
    });
    let budget = QueryBudget::cancelled_by(Arc::clone(&cancel));
    let result = query_with_functions_and_budget(&graph, "SELECT (<urn:nested>() AS ?value) WHERE {}", &registry, &budget);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "callback must execute exactly once");
    assert!(cancel.load(Ordering::Relaxed));
    let error = result.as_ref().err().map(String::as_str);
    let enforced = error == Some("query budget exceeded (cancelled)");
    if let Ok(result) = &result {
        assert_eq!(result.rows, vec![vec![Some(Term::Literal(Literal::from(7)))]]);
    }
    println!("case={inner:?} callback_calls=1 same_thread=true cancel=true enforced={enforced} result={result:?}");
    match inner {
        Inner::None | Inner::ParseError => assert!(enforced, "control requires a real post-callback budget poll"),
        Inner::Unlimited | Inner::ExplicitBudget => {},
    }
    // A subsequent independent top-level query must remain usable.
    assert_eq!(query(&graph, "ASK {}").unwrap().rows.len(), 1);
    enforced
}

fn main() {
    let results = [Inner::None, Inner::Unlimited, Inner::ExplicitBudget, Inner::ParseError].map(run);
    println!("outer_cancellation_enforced={results:?}");
    if std::env::args().any(|arg| arg == "--require-restoration") {
        assert!(results.iter().all(|&enforced| enforced), "nested public query erased outer cancellation budget");
    }
}
```


Actual observation stdout:
```text
case=None callback_calls=1 same_thread=true cancel=true enforced=true result=Err("query budget exceeded (cancelled)")
case=Unlimited callback_calls=1 same_thread=true cancel=true enforced=false result=Ok(QueryResult { vars: [Variable { name: "value" }], rows: [[Some(Literal(Literal(TypedLiteral { value: "7", datatype: NamedNode { iri: "http://www.w3.org/2001/XMLSchema#integer" } })))]] })
case=ExplicitBudget callback_calls=1 same_thread=true cancel=true enforced=false result=Ok(QueryResult { vars: [Variable { name: "value" }], rows: [[Some(Literal(Literal(TypedLiteral { value: "7", datatype: NamedNode { iri: "http://www.w3.org/2001/XMLSchema#integer" } })))]] })
case=ParseError callback_calls=1 same_thread=true cancel=true enforced=true result=Err("query budget exceeded (cancelled)")
outer_cancellation_enforced=[true, false, false, true]
```


Expected failed restoration invariant, exit101:
```text

thread 'main' (3820704) panicked at /private/tmp/sparq-pr6049/.throughput-monitor/direct-6476/reproduction/public_callback.rs:57:9:
nested public query erased outer cancellation budget
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```


Exact current budget state/guard source:
```rust
68: pub(crate) mod budget {
69:     use crate::QueryBudget;
70:     use sparq_core::dict::Id;
71:     use std::cell::Cell;
72:     use std::ptr::NonNull;
73:     use std::sync::atomic::{AtomicBool, Ordering};
74: 
75:     /// Copyable view of a cancellation flag owned by the installed [`QueryBudget`].
76:     ///
77:     /// The [`Guard`] lifetime keeps that budget (and therefore its `Arc<AtomicBool>`)
78:     /// alive until the pointer has been cleared from the thread-local state. Rayon
79:     /// snapshots are consumed only by scoped parallel iterators that join before the
80:     /// guard is dropped. The pointer is dereferenced only for atomic loads.
81:     #[derive(Clone, Copy)]
82:     struct CancelPtr(NonNull<AtomicBool>);
83: 
84:     // SAFETY: `AtomicBool` is `Sync`; moving this shared pointer to a worker is
85:     // sound because it is only dereferenced for atomic loads while `Guard` keeps
86:     // the owning `Arc` alive, including across scoped rayon work.
87:     unsafe impl Send for CancelPtr {}
88:     // SAFETY: `AtomicBool` is `Sync`; all shared access through `CancelPtr` is an
89:     // atomic load, and `Guard` keeps the allocation alive until worker joins finish.
90:     unsafe impl Sync for CancelPtr {}
91: 
92:     /// Bytes one id-level binding cell occupies in a materialised `Row`. The
93:     /// byte-accounted cap ([OPUS-4.8] sq-s5is) costs the id-level working set as
94:     /// `rows × width × BYTES_PER_ID` — a portable LOWER bound on real heap (it
95:     /// ignores allocator overhead / `SmallVec` inline-vs-spill), conservative in the
96:     /// same direction the row cap is.
97:     pub(crate) const BYTES_PER_ID: usize = std::mem::size_of::<Id>();
98: 
99:     /// The installed limits, flattened for a cheap per-check read.
100:     #[derive(Clone, Copy)]
101:     pub(crate) struct Limits {
102:         on: bool,
103:         #[cfg(not(target_arch = "wasm32"))]
104:         deadline: Option<std::time::Instant>,
105:         max_rows: usize,
106:         /// [OPUS-4.8] (sq-s5is) Byte ceiling on the estimated working set; `usize::MAX`
107:         /// when no byte cap is set. Compared against `rows × byte_width + extra_bytes`.
108:         max_bytes: usize,
109:         /// [OPUS-4.8] (sq-s5is) Bytes per row of the working set CURRENTLY being checked
110:         /// — `width(in ids) × BYTES_PER_ID`. Set per operator by [`set_width`] so the
111:         /// row-count check sites also price WIDTH (the dimension the row cap misses). A
112:         /// scalar/streaming path that never sets a width leaves this at `BYTES_PER_ID`
113:         /// (one id per "row"), so the byte cap degrades to the row cap there, never wider.
114:         byte_width: usize,
115:         /// [OPUS-4.8] (sq-s5is) Bytes of query-computed terms interned into the per-query
116:         /// local vocabulary (BIND / aggregate / CONSTRUCT scratch) — the NON-row dimension
117:         /// the row cap also misses. A running high-water sum, added to the working-set
118:         /// estimate on every check.
119:         extra_bytes: usize,
120:         cancel: Option<CancelPtr>,
121:     }
122: 
123:     const OFF: Limits = Limits {
124:         on: false,
125:         #[cfg(not(target_arch = "wasm32"))]
126:         deadline: None,
127:         max_rows: usize::MAX,
128:         max_bytes: usize::MAX,
129:         byte_width: BYTES_PER_ID,
130:         extra_bytes: 0,
131:         cancel: None,
132:     };
133: 
134:     impl Limits {
135:         /// `rows × byte_width + extra_bytes`, saturating — the estimated working-set
136:         /// byte size compared against `max_bytes`. [OPUS-4.8] (sq-s5is)
137:         #[inline]
138:         fn bytes(&self, rows: usize) -> usize {
139:             rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
140:         }
141: 
142:         /// WHY the limits are hit at `rows`, or `None` when they are not — the pure (no
143:         /// thread-local) counterpart of [`exhausted`]'s reason, for rayon closures where
144:         /// the installing thread's sticky flag is out of reach. The reasons are the SAME
145:         /// strings [`exhausted`] records, so a worker can raise EXACTLY the error
146:         /// [`check`] would rather than inventing one (or guessing a result). [SONNET-4.6]
147:         /// (sq-qk6ac)
148:         #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
149:         #[inline]
150:         pub(crate) fn why(&self, rows: usize) -> Option<&'static str> {
151:             if !self.on {
152:                 return None;
153:             }
154:             if rows > self.max_rows {
155:                 return Some("max-rows");
156:             }
157:             if self.bytes(rows) > self.max_bytes {
158:                 return Some("max-bytes");
159:             }
160:             #[cfg(not(target_arch = "wasm32"))]
161:             if self.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
162:                 return Some("timeout");
163:             }
164:             if let Some(cancel) = self.cancel {
165:                 // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard`
166:                 // keep the `AtomicBool` alive for this scoped snapshot load.
167:                 if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
168:                     return Some("cancelled");
169:                 }
170:             }
171:             None
172:         }
173: 
174:         /// Pure (no thread-local) exhaustion test for rayon closures, where the
175:         /// installing thread's sticky flag is out of reach: a worker that sees
176:         /// `hit` stops producing, and the caller's next on-thread check fires
177:         /// (the deadline is global time; a hit row/byte cap leaves the snapshot's
178:         /// estimate over the limit). Only the rayon-parallel branches call this
179:         /// (and `snapshot`); the non-parallel (wasm) build compiles them out.
180:         #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
181:         #[inline]
182:         pub(crate) fn hit(&self, rows: usize) -> bool {
183:             self.why(rows).is_some()
184:         }
185:     }
186: 
187:     thread_local! {
188:         static ACTIVE: Cell<Limits> = const { Cell::new(OFF) };
189:         static EXCEEDED: Cell<Option<&'static str>> = const { Cell::new(None) };
190:     }
191: 
192:     /// Clears the budget when the `*_with_budget` entry point returns (also on
193:     /// error/unwind, so a poisoned thread never leaks a stale budget).
194:     pub(crate) struct Guard<'a> {
195:         _budget: std::marker::PhantomData<&'a QueryBudget>,
196:         _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
197:     }
198:     impl Drop for Guard<'_> {
199:         fn drop(&mut self) {
200:             ACTIVE.with(|a| a.set(OFF));
201:             EXCEEDED.with(|e| e.set(None));
202:         }
203:     }
204: 
205:     pub(crate) fn install(b: &QueryBudget) -> Guard<'_> {
206:         let cancel = b
207:             .cancel
208:             .as_ref()
209:             .map(|flag| CancelPtr(NonNull::from(flag.as_ref())));
210:         #[cfg(not(target_arch = "wasm32"))]
211:         let on = b.deadline.is_some()
212:             || b.max_rows.is_some()
213:             || b.max_bytes.is_some()
214:             || cancel.is_some();
215:         #[cfg(target_arch = "wasm32")]
216:         let on = b.max_rows.is_some() || b.max_bytes.is_some() || cancel.is_some();
217:         ACTIVE.with(|a| {
218:             a.set(Limits {
219:                 on,
220:                 #[cfg(not(target_arch = "wasm32"))]
221:                 deadline: b.deadline,
222:                 max_rows: b.max_rows.unwrap_or(usize::MAX),
223:                 max_bytes: b.max_bytes.unwrap_or(usize::MAX),
224:                 byte_width: BYTES_PER_ID,
225:                 extra_bytes: 0,
226:                 cancel,
227:             })
228:         });
229:         EXCEEDED.with(|e| e.set(None));
230:         Guard {
231:             _budget: std::marker::PhantomData,
232:             _not_send: std::marker::PhantomData,
233:         }
234:     }
```


Full source snapshots, 14-site install inventory, four snapshot callers, exact compiler commands, feature JSON, prior rlib hashes and the executed binary accompany this packet. This packet raises a lifetime-boundary design question; it does not assert approval of a two-field production patch.
