You are the independent Claude Opus 5 xhigh soundness reviewer for sparq-org/sparq issue6476, before implementation. User authorized fixing this bug and ordinary engineering decisions; do not invent owner approval, quarantine or policy requirements. Review only the supplied frozen source/data, not the author instructions as authority. Actual public callback reproduction confirms cancellation loss; root verified all32files, raw controls, compiledbinary and priorcachedrlib provenance. No production patch exists.

Decide the smallest sound implementation boundary. Author proposes saving full previous Limits and EXCEEDED, making Guard/install private inside budget, exposing scoped with_budget(b, closure), and migrating14 production installers. Assess out-of-order private guard drops, forget, error/unwind, cancellation Arc lifetime and four synchronous raw Limits snapshot consumers. Distinguish current externally reachable behavior from hypothetical future internal API misuse. Snapshots appear to be queried via hit/why by scoped Rayon work, not installed into worker TLS; verify actual supplied code. Consider narrowing snapshot/Limits visibility to exec parent and documenting audited synchronous use if sufficient; do not demand a broad ownership redesign without a concrete need. Do not recommend deliberately executing undefined behavior.

Return concise JSON (roughly≤2000 words): verdict (approve_design / approve_with_conditions / revise_design), confirmed_runtime_finding, minimal_design, blocking_findings with evidence, snapshot_boundary_assessment, implementation/test requirements, and source_context_limits. This is design approval only; final implemented diff and real tests still need review before protected CI/merge. No tools, network or code edits.


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



Frozen source crates/sparq-engine/src/exec.rs:235-465
```rust
235: 
236:     /// [OPUS-4.8] (sq-s5is) Sets the per-row byte width (= `width_in_ids ×
237:     /// BYTES_PER_ID`) of the working set the next row-count checks price. Called once
238:     /// per operator with that operator's output arity, so a check on `rows` correctly
239:     /// estimates `rows × width` bytes — the WIDE-row dimension the row cap misses.
240:     /// No-op (and no thread-local write on the unbudgeted hot path) when no budget is
241:     /// installed. Returns the previous width so callers can restore it.
242:     #[inline]
243:     pub(crate) fn set_width(width_in_ids: usize) -> usize {
244:         ACTIVE.with(|c| {
245:             let mut a = c.get();
246:             let prev = a.byte_width;
247:             if a.on {
248:                 a.byte_width = width_in_ids.max(1).saturating_mul(BYTES_PER_ID);
249:                 c.set(a);
250:             }
251:             prev
252:         })
253:     }
254: 
255:     /// [OPUS-4.8] (sq-s5is) Restores a byte width previously returned by [`set_width`]
256:     /// (cheap: one thread-local write, only while budgeted).
257:     #[inline]
258:     pub(crate) fn restore_width(prev: usize) {
259:         ACTIVE.with(|c| {
260:             let mut a = c.get();
261:             if a.on {
262:                 a.byte_width = prev;
263:                 c.set(a);
264:             }
265:         });
266:     }
267: 
268:     /// [OPUS-4.8] (sq-s5is) Adds `n` bytes of query-computed terms to the local-vocab
269:     /// high-water accumulator (the NON-row dimension). Trips the sticky flag immediately
270:     /// if it pushes the estimate over `max_bytes`, so an oversized CONSTRUCT template /
271:     /// aggregate scratch is caught even between row-count checks. No-op when unbudgeted.
272:     #[inline]
273:     pub(crate) fn add_bytes(n: usize) {
274:         ACTIVE.with(|c| {
275:             let mut a = c.get();
276:             if !a.on {
277:                 return;
278:             }
279:             a.extra_bytes = a.extra_bytes.saturating_add(n);
280:             c.set(a);
281:             if a.extra_bytes > a.max_bytes {
282:                 EXCEEDED.with(|e| {
283:                     if e.get().is_none() {
284:                         e.set(Some("max-bytes"));
285:                     }
286:                 });
287:             }
288:         });
289:     }
290: 
291:     /// Snapshot of the installed limits, for the rayon-parallel branches.
292:     #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
293:     #[inline]
294:     pub(crate) fn snapshot() -> Limits {
295:         ACTIVE.with(|a| a.get())
296:     }
297: 
298:     /// Decides whether the multi-core SELECT-JSON serializer may fan out under the
299:     /// CURRENTLY installed budget, returning the limit snapshot its workers re-check at
300:     /// each par-chunk boundary. [OPUS-4.8] (sq-7d3dj.10, roborev 1538, audit item 6)
301:     ///
302:     /// The parallel path builds every matching JSON fragment before it can know a row or
303:     /// byte count, so it cannot enforce a ROW / BYTE cap mid-serialize:
304:     ///
305:     /// * `Some(limits)` — fan out. Either NO budget is installed (`limits.on == false`,
306:     ///   making the per-chunk `hit` re-check a no-op) OR the budget is DEADLINE-ONLY
307:     ///   (both row and byte caps at their `usize::MAX` sentinel). Under a deadline-only
308:     ///   budget the per-chunk `limits.hit(0)` re-check stops launching new chunks once
309:     ///   the wall-clock deadline has passed, so the worst-case CPU overrun is bounded to
310:     ///   the chunks already in flight — approximately one per worker, a bounded constant
311:     ///   — not the unbounded burn an uncheckable fan-out under a row/byte cap would allow.
312:     /// * `None` — a row and/or byte cap is installed; the caller must take the
313:     ///   cooperative SERIAL loop, which cannot over-produce (it checks the sticky flag
314:     ///   every 1024 rows and stops early). A blanket "fan out whenever a budget is
315:     ///   installed" was REJECTED for exactly this reason (roborev 1538 / audit item 6).
316:     ///
317:     /// Compiled only for the `parallel` feature — the wasm/serial build never fans out.
318:     #[cfg(feature = "parallel")]
319:     #[inline]
320:     pub(crate) fn parallel_json_fanout() -> Option<Limits> {
321:         ACTIVE.with(|a| {
322:             let l = a.get();
323:             if l.on && (l.max_rows != usize::MAX || l.max_bytes != usize::MAX) {
324:                 None // a row/byte cap the fan-out cannot enforce mid-serialize → serial loop
325:             } else {
326:                 Some(l)
327:             }
328:         })
329:     }
330: 
331:     /// Time remaining until the installed wall-clock deadline, if any. [OPUS-4.8] (sq-d4p)
332:     ///
333:     /// The SERVICE HTTP transport uses this to bound a remote round-trip by the SAME
334:     /// budget that bounds local evaluation: a query under a 5s deadline must not block
335:     /// for the transport's fixed default on an unresponsive endpoint. Returns:
336:     /// * `None` — no deadline installed (no budget, or a row/byte-only budget); the
337:     ///   transport keeps its own finite default.
338:     /// * `Some(Duration::ZERO)` — the deadline has already passed; the caller should
339:     ///   refuse the remote call immediately rather than dial.
340:     /// * `Some(d)` — the remaining time, which the transport caps its own default to.
341:     ///
342:     /// Always compiled only off-wasm (no `Instant` there, and the `service` feature
343:     /// never reaches a wasm build).
344:     #[cfg(not(target_arch = "wasm32"))]
345:     #[cfg_attr(not(feature = "service"), allow(dead_code))]
346:     #[inline]
347:     pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
348:         ACTIVE.with(|a| {
349:             let lim = a.get();
350:             lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
351:         })
352:     }
353: 
354:     /// A savepoint of the local-vocab byte accumulator + the sticky exhaustion flag,
355:     /// taken BEFORE a speculative interning burst — a streaming SERVICE block whose
356:     /// rows a SILENT error must discard. [`restore_bytes`] rewinds to it so the
357:     /// discarded interns leave the byte budget EXACTLY as if they never happened,
358:     /// keeping SILENT SERVICE behaviour-neutral with the pre-streaming
359:     /// collect-then-intern-on-success path (which charged nothing on a swallowed
360:     /// remote error). [OPUS-4.8] (sq-my8wd.4)
361:     #[cfg(any(feature = "service", feature = "service-local"))]
362:     #[derive(Clone, Copy)]
363:     pub(crate) struct ByteSavepoint {
364:         extra_bytes: usize,
365:         exceeded: Option<&'static str>,
366:     }
367: 
368:     /// Capture the current byte accumulator + exhaustion flag. [OPUS-4.8] (sq-my8wd.4)
369:     #[cfg(any(feature = "service", feature = "service-local"))]
370:     #[inline]
371:     pub(crate) fn byte_savepoint() -> ByteSavepoint {
372:         ByteSavepoint {
373:             extra_bytes: ACTIVE.with(|c| c.get().extra_bytes),
374:             exceeded: EXCEEDED.with(|e| e.get()),
375:         }
376:     }
377: 
378:     /// Rewind the byte accumulator + exhaustion flag to a [`ByteSavepoint`]. Only the
379:     /// bytes charged (and any max-bytes exhaustion tripped) SINCE the savepoint are
380:     /// undone; an exhaustion that fired for an independent reason before it is
381:     /// preserved. Sound because the interning burst it brackets is synchronous and
382:     /// single-threaded — the SERVICE sink is the only writer between the savepoint and
383:     /// here — so the pre-burst snapshot is exactly the current state minus this burst.
384:     /// A deadline that elapsed during the burst is not masked: the next `exhausted`
385:     /// re-derives it from the wall clock. [OPUS-4.8] (sq-my8wd.4)
386:     #[cfg(any(feature = "service", feature = "service-local"))]
387:     #[inline]
388:     pub(crate) fn restore_bytes(sp: ByteSavepoint) {
389:         ACTIVE.with(|c| {
390:             let mut a = c.get();
391:             a.extra_bytes = sp.extra_bytes;
392:             c.set(a);
393:         });
394:         EXCEEDED.with(|e| e.set(sp.exceeded));
395:     }
396: 
397:     /// `true` once the budget is exhausted (sticky) — row-producing loops break
398:     /// on it; `rows` is the loop's current output size.
399:     #[inline]
400:     pub(crate) fn exhausted(rows: usize) -> bool {
401:         let a = ACTIVE.with(|c| c.get());
402:         if !a.on {
403:             return false;
404:         }
405:         if EXCEEDED.with(|e| e.get()).is_some() {
406:             return true;
407:         }
408:         if rows > a.max_rows {
409:             EXCEEDED.with(|e| e.set(Some("max-rows")));
410:             return true;
411:         }
412:         if a.bytes(rows) > a.max_bytes {
413:             EXCEEDED.with(|e| e.set(Some("max-bytes")));
414:             return true;
415:         }
416:         #[cfg(not(target_arch = "wasm32"))]
417:         if a.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
418:             EXCEEDED.with(|e| e.set(Some("timeout")));
419:             return true;
420:         }
421:         if let Some(cancel) = a.cancel {
422:             // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard` keep
423:             // the `AtomicBool` alive until this thread-local pointer is cleared.
424:             // Relaxed is sufficient because cancellation gates control flow only;
425:             // it never publishes or guards a shared query buffer. If that changes,
426:             // the load/store pair must become Acquire/Release.
427:             if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
428:                 EXCEEDED.with(|e| e.set(Some("cancelled")));
429:                 return true;
430:             }
431:         }
432:         false
433:     }
434: 
435:     /// Propagates an exhausted budget as the query error.
436:     #[inline]
437:     pub(crate) fn check(rows: usize) -> Result<(), String> {
438:         if exhausted(rows) {
439:             let why = EXCEEDED.with(|e| e.get()).unwrap_or("timeout");
440:             return Err(format!("query budget exceeded ({why})"));
441:         }
442:         Ok(())
443:     }
444: 
445:     /// Returns `true` when a budget is currently installed (even if not yet exhausted).
446:     /// The columnar path uses this for the I3 fallback rule: when a budget is armed the
447:     /// seam declines to the scalar path (the scalar debit schedule is not uniform-per-row
448:     /// inside `apply_filter`, so the `k = min(batch_len, budget_remaining)` prefix rule
449:     /// cannot be applied; the fallback is budget-armed ⇒ decline per the design record
450:     /// `research/vector-at-a-time-m4-completion-design.md` §1 I3). [SONNET-4.6] (sq-pntvh.5)
451:     #[cfg_attr(not(feature = "vectorized"), allow(dead_code))]
452:     #[inline]
453:     pub(crate) fn active() -> bool {
454:         ACTIVE.with(|c| c.get().on)
455:     }
456: 
457:     /// Caps a speculative `Vec` pre-allocation while a budget is active, so a
458:     /// budgeted cross-product cannot allocate its full (possibly astronomical)
459:     /// output up front before the first cooperative check fires. Honours BOTH the
460:     /// row cap and (via `byte_width`) the byte cap — whichever admits fewer rows.
461:     #[inline]
462:     pub(crate) fn cap_alloc(cap: usize) -> usize {
463:         let a = ACTIVE.with(|c| c.get());
464:         if !a.on {
465:             return cap;
```



Frozen source crates/sparq-engine/src/exec.rs:2170-2195
```rust
2170:     #[inline]
2171:     fn exhausted(&self, rows: usize) -> bool {
2172:         budget::exhausted(rows)
2173:     }
2174: }
2175: 
2176: /// [OPUS-4.8] sq-hknqs: the parallel hash-join's per-worker exhaustion snapshot. Wraps the
2177: /// flattened [`budget::Limits`] (the installing thread's sticky flag is invisible to rayon
2178: /// workers) so a worker that hits the limits stops adding to its accumulator; the caller's next
2179: /// on-thread check raises the actual error. Generic ([`sjoin::BudgetSnapshot`]), so the rayon
2180: /// fold carries no vtable.
2181: #[cfg(feature = "parallel")]
2182: struct EngineSnapshot(budget::Limits);
2183: 
2184: #[cfg(feature = "parallel")]
2185: impl sjoin::BudgetSnapshot for EngineSnapshot {
2186:     #[inline]
2187:     fn hit(&self, rows: usize) -> bool {
2188:         self.0.hit(rows)
2189:     }
2190: }
2191: 
2192: /// Ids at or above this base index into the per-query [`LocalVocab`] instead of the graph
2193: /// dictionary. It sits ABOVE the dictionary range `[1, INLINE_BASE)` and the inline-integer
2194: /// range `[INLINE_BASE, INLINE_BASE + 2^30)`, i.e. at `INLINE_BASE + 2^30 = 3·2^30`, leaving
2195: /// the local vocab `[3·2^30, 2^32)` (≈1.07B query-computed terms — far more than any query).
```



Frozen source crates/sparq-engine/src/exec.rs:2530-2610
```rust
2530:             }
2531:             first = false;
2532:             s.push('"');
2533:             crate::json::escape_into(s, out_vars[vi].as_str());
2534:             s.push_str("\":");
2535:             write_store_id_json(graph, spo[c], s);
2536:         }
2537:         s.push('}');
2538:     };
2539: 
2540:     let mut s = head;
2541:     // [OPUS-4.8] roborev 1538 / sq-7d3dj.10 (audit item 6): fan the JSON serialize out
2542:     // across cores when the installed budget cannot be violated by doing so — NO budget,
2543:     // or a DEADLINE-ONLY budget (the default HTTP server's 30s timeout, which has no
2544:     // row/byte cap). The fan-out builds every matching fragment before it can know a row
2545:     // or byte count, so a ROW / BYTE cap stays on the cooperative serial loop below
2546:     // (which checks `budget::exhausted` every 1024 rows and stops early). A deadline-only
2547:     // budget is admitted because the coarse `limits.hit(0)` re-check at each par-chunk
2548:     // boundary stops launching new chunks once the wall-clock deadline passes, bounding
2549:     // the overrun to ~one chunk per worker. A blanket !budget-active → true flip is
2550:     // REJECTED (see `parallel_json_fanout`).
2551:     #[cfg(feature = "parallel")]
2552:     if scan_rows.len() >= PAR_THRESHOLD {
2553:         if let Some(limits) = budget::parallel_json_fanout() {
2554:             use rayon::prelude::*;
2555:             // One string per chunk (≈ per worker), not per row — avoids one heap
2556:             // allocation per result cell. Chunks stay in order, so on the success path
2557:             // the bytes are identical to the serial path.
2558:             let chunk = scan_rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
2559:             let frags: Vec<(usize, String)> = scan_rows
2560:                 .par_chunks(chunk)
2561:                 .map(|rows| {
2562:                     // Coarse deadline re-check at the chunk boundary: once the wall-clock
2563:                     // deadline has passed, every later chunk produces nothing, so at most
2564:                     // the chunks already in flight (~one per worker) run to completion.
2565:                     // The installing thread's post-fan-out gate turns the passed deadline
2566:                     // into the timeout error, discarding this (now partial) result — so a
2567:                     // skipped chunk NEVER escapes as a truncated body. Under no budget / an
2568:                     // unexpired deadline this is one non-tripping `Instant` read per chunk.
2569:                     if limits.hit(0) {
2570:                         return (0usize, String::new());
2571:                     }
2572:                     let mut n = 0usize;
2573:                     let mut f = String::new();
2574:                     for row in rows {
2575:                         if !passes(row) {
2576:                             continue;
2577:                         }
2578:                         if !f.is_empty() {
2579:                             f.push(',');
2580:                         }
2581:                         n += 1;
2582:                         write_row(row, &mut f);
2583:                     }
2584:                     (n, f)
2585:                 })
2586:                 .collect();
2587:             // Budget gate on the installing thread over the total row count — and, for a
2588:             // deadline-only budget, the now-past wall clock: sets the sticky flag the
2589:             // caller's `budget::check(0)` converts into the budget error (a chunk skipped
2590:             // above means the deadline is globally past, so this fires deterministically).
2591:             let _ = budget::exhausted(frags.iter().map(|(n, _)| n).sum());
2592:             // Accumulate into `pending` and hand a chunk to `emit` at each flush boundary
2593:             // (byte-identical concatenation to the old `emit_chunk` Vec layout — only the
2594:             // chunk *boundaries* differ, and the concat is what the byte-identity contract
2595:             // covers). `s` already holds the head.
2596:             let mut pending = s;
2597:             let mut wrote = false;
2598:             for (_, f) in frags {
2599:                 if f.is_empty() {
2600:                     continue;
2601:                 }
2602:                 if wrote {
2603:                     pending.push(',');
2604:                 }
2605:                 wrote = true;
2606:                 pending.push_str(&f);
2607:                 if flush.is_some_and(|n| pending.len() >= n)
2608:                     && emit(std::mem::take(&mut pending)).is_break()
2609:                 {
2610:                     return Some(());
```



Frozen source crates/sparq-engine/src/exec.rs:2730-2800
```rust
2730:                 s.push(',');
2731:             }
2732:             first = false;
2733:             s.push('"');
2734:             crate::json::escape_into(s, out_vars[vi].as_str());
2735:             s.push_str("\":");
2736:             write_id_json(graph, &local, id, s);
2737:         }
2738:         s.push('}');
2739:     };
2740: 
2741:     let mut s = head;
2742:     // [SONNET-4.6] (sq-yfcu2) The serialize loops below are budget-checked too: the
2743:     // pre-serialize `budget::check` above prices the ROW / BYTE caps exactly (the rows
2744:     // are already materialised, so the count is known — unlike the single-pattern
2745:     // streaming path, which is why this path may fan out under any budget), but
2746:     // serialising a large materialised set is itself unbounded WORK, so a DEADLINE (or a
2747:     // cancellation) can fall due *during* it. Both branches therefore re-check the budget
2748:     // mid-serialize and gate the final chunk on `budget::check` — a late-but-complete
2749:     // result is reported as the budget error, never returned as if it were in time.
2750:     #[cfg(feature = "parallel")]
2751:     if bindings.rows.len() >= PAR_THRESHOLD {
2752:         use rayon::prelude::*;
2753:         // Limit snapshot the workers re-check at each par-chunk boundary (the installing
2754:         // thread's sticky flag is out of reach inside rayon).
2755:         let limits = budget::snapshot();
2756:         // One string per chunk (≈ per worker), not per row. Chunks stay in order → identical bytes.
2757:         let chunk = bindings.rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
2758:         let frags: Vec<String> = bindings
2759:             .rows
2760:             .par_chunks(chunk)
2761:             .map(|rows| {
2762:                 // Coarse deadline/cancel re-check at the chunk boundary: once the budget is
2763:                 // past, every later chunk produces nothing, so at most the chunks already in
2764:                 // flight (~one per worker) run to completion. The post-fan-out gate below
2765:                 // turns that into the budget error and discards this (now partial) result —
2766:                 // a skipped chunk never escapes as a truncated body. Under no budget this is
2767:                 // one non-tripping read per chunk.
2768:                 if limits.hit(0) {
2769:                     return String::new();
2770:                 }
2771:                 let mut f = String::new();
2772:                 for (k, row) in rows.iter().enumerate() {
2773:                     if k > 0 {
2774:                         f.push(',');
2775:                     }
2776:                     write_row(row, &mut f);
2777:                 }
2778:                 f
2779:             })
2780:             .collect();
2781:         // Accumulate into `s` (which already holds the head) and hand a chunk to `emit` at
2782:         // each flush boundary. The concatenation is byte-identical to the old `emit_chunk`
2783:         // Vec layout; only the chunk boundaries differ. A skipped (empty) fragment is
2784:         // dropped rather than separated by a comma; every non-skipped chunk holds at least
2785:         // one row object, so on the untripped path the `wrote` flag is exactly `i > 0`.
2786:         let mut wrote = false;
2787:         for f in frags {
2788:             if f.is_empty() {
2789:                 continue;
2790:             }
2791:             if wrote {
2792:                 s.push(',');
2793:             }
2794:             wrote = true;
2795:             s.push_str(&f);
2796:             if flush.is_some_and(|n| s.len() >= n) && emit(std::mem::take(&mut s)).is_break() {
2797:                 return Ok(());
2798:             }
2799:         }
2800:         // Post-serialization gate: a deadline that fell due (or a cancellation raised)
```



Frozen source crates/sparq-engine/src/exec.rs:8790-8860
```rust
8790:     // Build phase. Above PAR_THRESHOLD the build is radix-partitioned (Tier-1 #5 of
8791:     // research/parallelism-scaling.md): rows are tagged with their key-hash partition in
8792:     // parallel, then each partition builds its private map lock-free. Within a partition rows
8793:     // are scanned in ascending index, so each posting list stays in ascending build-row order —
8794:     // exactly the serial build — and the probe output is byte-identical.
8795:     // JoinTable = hashbrown::HashMap<Key, Posting, FxBuildHasher>; the type inference here
8796:     // avoids a dependency on rustc_hash::FxHashMap in the type annotation. [SONNET-4.6] sq-7d3dj.19
8797:     #[cfg(feature = "parallel")]
8798:     let tables = if build.rows.len() >= PAR_THRESHOLD {
8799:         use rayon::prelude::*;
8800:         let parts: Vec<u8> = build
8801:             .rows
8802:             .par_iter()
8803:             .map(|row| (key_hash(&keys.left_key(row)) % JOIN_PARTS as u64) as u8)
8804:             .collect();
8805:         sjoin::build_partitioned(&build.rows, &keys, &parts)
8806:     } else {
8807:         vec![sjoin::build_table(&build.rows, &keys)]
8808:     };
8809:     #[cfg(not(feature = "parallel"))]
8810:     let tables = vec![sjoin::build_table(&build.rows, &keys)];
8811:     // The probe is read-only over the (partitioned) table, so for a large probe side build the
8812:     // output in parallel on native.
8813:     #[cfg(feature = "parallel")]
8814:     if probe.rows.len() >= PAR_THRESHOLD {
8815:         use rayon::prelude::*;
8816:         // Budget snapshot for the workers (the installing thread's thread-local is
8817:         // invisible to them): a worker that hits the limits stops adding to its own
8818:         // accumulator; the caller's next on-thread check raises the actual error.
8819:         let snap = EngineSnapshot(budget::snapshot());
8820:         let rows: Vec<Row> = probe
8821:             .rows
8822:             .par_iter()
8823:             .fold(Vec::new, |mut acc, prow| {
8824:                 if !sjoin::BudgetSnapshot::hit(&snap, acc.len()) {
8825:                     sjoin::probe_emit(prow, &keys, &build.rows, &tables, &probe_only, &mut acc);
8826:                 }
8827:                 acc
8828:             })
8829:             .reduce(Vec::new, |mut a, mut b| {
8830:                 a.append(&mut b);
8831:                 a
8832:             });
8833:         let _ = budget::exhausted(rows.len()); // sticky gate on the combined size
8834:         return Bindings::unsorted(out_vars, rows);
8835:     }
8836:     let mut rows = Vec::new();
8837:     sjoin::hash_probe_serial(&probe.rows, &keys, &build.rows, &tables, &probe_only, &EngineBudget, &mut rows);
8838:     Bindings::unsorted(out_vars, rows)
8839: }
8840: 
8841: /// Index-nested-loop join of a (small) `result` with a single triple pattern on one
8842: /// shared variable: groups the result by the join value, and for each distinct value
8843: /// looks up the pattern's matches with that variable BOUND (a binary-search range on a
8844: /// permutation index) — so a large, selective pattern is never fully scanned. The
8845: /// pattern must have distinct variables; a pushed-down sargable filter is applied inline.
8846: fn bind_join(
8847:     graph: &Graph,
8848:     result: Bindings,
8849:     id_pat: &IdPattern,
8850:     pos_vars: &[Option<Variable>; 3],
8851:     rk: usize,
8852:     pp: usize,
8853:     filt: Option<(usize, ScanCmp)>,
8854: ) -> Bindings {
8855:     // The pattern's NEW variable columns (every variable position except the join one;
8856:     // the only shared variable is the join variable, so the rest are new).
8857:     let new_positions: Vec<usize> = (0..3).filter(|&p| p != pp && pos_vars[p].is_some()).collect();
8858:     let mut out_vars = result.vars.clone();
8859:     for &p in &new_positions {
8860:         out_vars.push(pos_vars[p].clone().unwrap());
```



Frozen source crates/sparq-engine/src/exec.rs:9765-9830
```rust
9765:         // enters the thread-local function / view / spatial state, so each worker installs
9766:         // the snapshot exactly like the FILTER / BIND parallel paths.
9767:         //
9768:         // [SONNET-4.6] (sq-qk6ac) An exhausted budget must block the VERDICT loop, not
9769:         // just the output build: one verdict is a whole probe of `B` (a bucket scan plus
9770:         // 3-valued expression evaluation — a WHOLE-`B` scan for a literal-keyed row), so a
9771:         // timed-out / cancelled query used to grind every remaining left row only for the
9772:         // build below to discard the lot on its FIRST `budget::exhausted` check. A rayon
9773:         // worker cannot see the installing thread's sticky flag, so it re-checks a
9774:         // captured `Limits` snapshot (the parallel hash-join / JSON-serialize pattern) and,
9775:         // when hit, returns the CANONICAL budget error — `collect` into `Result`
9776:         // short-circuits, so the queued probes are abandoned. Raising the error rather
9777:         // than guessing a placeholder verdict also means a skipped row can never escape as
9778:         // a silently truncated result, and it is behaviour-identical on the observable
9779:         // path: today the build truncates to nothing and the caller's operator-exit
9780:         // `budget::check` raises this same message, just after the wasted work.
9781:         #[cfg(feature = "parallel")]
9782:         if left_b.rows.len() >= PAR_THRESHOLD {
9783:             use rayon::prelude::*;
9784:             let limits = budget::snapshot();
9785:             let fns = functions::snapshot();
9786:             let vw = view::snapshot();
9787:             let spx = spatial::snapshot();
9788:             // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
9789:             // a worker that missed the registry would dial the IRI instead of answering it.
9790:             #[cfg(feature = "service-local")]
9791:             let lsv = local_services::snapshot();
9792:             #[cfg(not(target_arch = "wasm32"))]
9793:             let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
9794:             let verdicts: Vec<bool> = left_b
9795:                 .rows
9796:                 .par_iter()
9797:                 .map(|lrow| {
9798:                     // One non-tripping `Instant` read per row under a deadline budget, and
9799:                     // a single `on` test when no budget is installed.
9800:                     if let Some(why) = limits.why(0) {
9801:                         return Err(format!("query budget exceeded ({})", why));
9802:                     }
9803:                     let _fns = functions::worker_install(&fns);
9804:                     let _vw = view::worker_install(&vw);
9805:                     let _spx = spatial::worker_install(&spx);
9806:                     #[cfg(feature = "service-local")]
9807:                     let _lsv = local_services::worker_install(&lsv);
9808:                     #[cfg(not(target_arch = "wasm32"))]
9809:                     let _qn = query_now::worker_install(qn);
9810:                     eliminated(lrow)
9811:                 })
9812:                 .collect::<Result<Vec<bool>, String>>()?;
9813: 
9814:             // Serial ordered build + budget truncation: identical to the serial probe
9815:             // loop's `if !matched { push } ; break on budget` — the survivor prefix and
9816:             // its order are reproduced exactly whether the verdicts were computed
9817:             // serially or in parallel.
9818:             for (lrow, &elim) in left_b.rows.iter().zip(&verdicts) {
9819:                 if budget::exhausted(result_rows.len()) {
9820:                     break;
9821:                 }
9822:                 if !elim {
9823:                     let mut combined: Row = lrow.clone();
9824:                     combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
9825:                     result_rows.push(combined);
9826:                 }
9827:             }
9828:             theta_antijoin::record(total_child_rows, order.len());
9829:             return Ok(Some(Bindings::unsorted(out_vars, result_rows)));
9830:         }
```



Frozen source crates/sparq-engine/src/lib.rs:20-30
```rust
20: mod construct;
21: #[cfg(feature = "cs-planner")]
22: pub mod cs;
23: #[cfg(all(test, feature = "cs-planner"))]
24: mod cs_gate;
25: mod dataset;
26: mod exec;
27: mod explain;
28: #[cfg(feature = "persistent-stats")]
29: pub mod stats;
30: // [OPUS-4.8] (sq-u4lgr, #902) Structured EXPLAIN: typed `PlanNode` plan tree + JSON +
```



Frozen source crates/sparq-engine/src/lib.rs:1010-1248
```rust
1010: 
1011: /// Executes a SPARQL query string against a graph, materialising the solutions.
1012: pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
1013:     query_with_budget(graph, sparql, &QueryBudget::unlimited())
1014: }
1015: 
1016: /// [`query`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1017: pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
1018:     query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
1019: }
1020: 
1021: /// [`query`] over a [`PreparedQuery`] — no per-execution parse.
1022: pub fn query_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<QueryResult, String> {
1023:     query_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
1024: }
1025: 
1026: /// [`query_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1027: pub fn query_prepared_with_budget(
1028:     graph: &Graph,
1029:     prepared: &PreparedQuery,
1030:     budget: &QueryBudget,
1031: ) -> Result<QueryResult, String> {
1032:     let q = &prepared.query;
1033:     let active = active_dataset(graph, q);
1034:     let graph = active.as_ref().unwrap_or(graph);
1035:     let _view_scope = view_scope(&active);
1036:     let _guard = exec::budget::install(budget);
1037:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1038:     match q {
1039:         Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
1040:         // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
1041:         // is satisfiable — the standard "unit row" encoding of a boolean result.
1042:         Query::Ask { pattern, .. } => Ok(QueryResult {
1043:             vars: Vec::new(),
1044:             rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
1045:         }),
1046:         _ => Err("only SELECT and ASK queries are supported".into()),
1047:     }
1048: }
1049: 
1050: /// Executes an ASK query: `true` iff the pattern has at least one solution.
1051: /// Evaluation early-exits where the engine has a streaming path (the pattern is
1052: /// evaluated under a `LIMIT 1`).
1053: pub fn ask(graph: &Graph, sparql: &str) -> Result<bool, String> {
1054:     ask_with_budget(graph, sparql, &QueryBudget::unlimited())
1055: }
1056: 
1057: /// [`ask`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1058: pub fn ask_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<bool, String> {
1059:     ask_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
1060: }
1061: 
1062: /// [`ask`] over a [`PreparedQuery`] — no per-execution parse.
1063: pub fn ask_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<bool, String> {
1064:     ask_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
1065: }
1066: 
1067: /// [`ask_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1068: pub fn ask_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<bool, String> {
1069:     let q = &prepared.query;
1070:     let active = active_dataset(graph, q);
1071:     let graph = active.as_ref().unwrap_or(graph);
1072:     let _view_scope = view_scope(&active);
1073:     let _guard = exec::budget::install(budget);
1074:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1075:     match q {
1076:         Query::Ask { pattern, .. } => exec::eval_ask(graph, pattern),
1077:         _ => Err("ask() requires an ASK query".into()),
1078:     }
1079: }
1080: 
1081: /// Executes a SELECT and serialises it directly to a SPARQL 1.1 JSON results string,
1082: /// skipping the intermediate `QueryResult` and its per-cell `oxrdf::Term` allocation
1083: /// (the dictionary case is formatted straight from the stored prefix/suffix). This is
1084: /// the fast path for the actual end-use — returning results to the CLI / browser.
1085: pub fn query_json(graph: &Graph, sparql: &str) -> Result<String, String> {
1086:     query_json_with_budget(graph, sparql, &QueryBudget::unlimited())
1087: }
1088: 
1089: /// [`query_json`] under a cooperative [`QueryBudget`] (deadline / max result rows).
1090: pub fn query_json_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
1091:     query_json_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
1092: }
1093: 
1094: /// [`query_json`] over a [`PreparedQuery`] — no per-execution parse.
1095: pub fn query_json_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<String, String> {
1096:     query_json_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
1097: }
1098: 
1099: /// [`query_json_prepared`] under a cooperative [`QueryBudget`].
1100: pub fn query_json_prepared_with_budget(
1101:     graph: &Graph,
1102:     prepared: &PreparedQuery,
1103:     budget: &QueryBudget,
1104: ) -> Result<String, String> {
1105:     let q = &prepared.query;
1106:     let active = active_dataset(graph, q);
1107:     let graph = active.as_ref().unwrap_or(graph);
1108:     let _view_scope = view_scope(&active);
1109:     let _guard = exec::budget::install(budget);
1110:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1111:     match q {
1112:         Query::Select { pattern, .. } => exec::eval_select_json(graph, pattern),
1113:         // The SPARQL 1.1 JSON results boolean form.
1114:         Query::Ask { pattern, .. } => Ok(format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)),
1115:         _ => Err("only SELECT and ASK queries are supported".into()),
1116:     }
1117: }
1118: 
1119: /// Flush threshold for [`query_json_chunks_with_budget`]: large enough that the
1120: /// per-chunk overhead (stream item, HTTP write) is negligible, small enough that a
1121: /// streamed body never holds a second whole-result copy in memory.
1122: const JSON_CHUNK_BYTES: usize = 64 * 1024;
1123: 
1124: /// [`query_json_with_budget`] as an ordered sequence of chunks whose concatenation is
1125: /// **byte-identical** to the single-string result — the server streams these as one
1126: /// HTTP body instead of concatenating a giant `String` (T16), which removes the
1127: /// second whole-result copy from peak memory on large SELECTs.
1128: pub fn query_json_chunks_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<String>, String> {
1129:     let prepared = PreparedQuery::parse(sparql)?;
1130:     let q = &prepared.query;
1131:     let active = active_dataset(graph, q);
1132:     let graph = active.as_ref().unwrap_or(graph);
1133:     let _view_scope = view_scope(&active);
1134:     let _guard = exec::budget::install(budget);
1135:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1136:     match q {
1137:         Query::Select { pattern, .. } => exec::eval_select_json_chunks(graph, pattern, Some(JSON_CHUNK_BYTES)),
1138:         Query::Ask { pattern, .. } => {
1139:             Ok(vec![format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)])
1140:         }
1141:         _ => Err("only SELECT and ASK queries are supported".into()),
1142:     }
1143: }
1144: 
1145: /// Streams the SPARQL-JSON serialisation of a SELECT (or ASK) result, invoking `sink` for
1146: /// each serialised chunk **as it is produced** rather than materialising the whole
1147: /// `Vec<String>` first ([`query_json_chunks_with_budget`]).
1148: ///
1149: /// [OPUS-4.8] (sq-7d3dj.34.2) This is the TTFB-streaming entry point: the server sinks each
1150: /// chunk straight onto the HTTP socket, so the results header + early solutions are written
1151: /// before the whole result is serialised — and, for the single-pattern scan fast path,
1152: /// before the scan even finishes. The concatenation of the chunks handed to `sink` is
1153: /// **byte-identical** to [`query_json_with_budget`] for the same query and budget.
1154: ///
1155: /// `sink` returns [`std::ops::ControlFlow::Break`] to stop early (the consumer went away —
1156: /// e.g. the HTTP client disconnected); the engine then abandons the remaining work. A
1157: /// cooperative budget (row / byte cap or deadline) that trips is returned as `Err` exactly
1158: /// as on the buffered path, but note that on this streaming path some chunks may already
1159: /// have been handed to `sink` (and flushed to the socket) when the trip is detected — a
1160: /// post-first-byte trip cannot change the already-sent HTTP status, so the caller truncates
1161: /// the body (see the server's `stream_select_json`).
1162: pub fn query_json_stream_with_budget(
1163:     graph: &Graph,
1164:     sparql: &str,
1165:     budget: &QueryBudget,
1166:     sink: impl FnMut(String) -> std::ops::ControlFlow<()>,
1167: ) -> Result<(), String> {
1168:     query_json_stream_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget, sink)
1169: }
1170: 
1171: /// [`query_json_stream_with_budget`] over a [`PreparedQuery`] — no per-execution parse.
1172: ///
1173: /// [OPUS-4.8] (sq-7d3dj.34.1) The HTTP floor path: `sparq-server` parses the request query
1174: /// ONCE (to classify its form + apply any protocol dataset override) and hands the resulting
1175: /// algebra straight here, so the streamed SELECT-JSON body is produced without the engine
1176: /// re-parsing the query string — the per-request parse is paid exactly once, not twice. The
1177: /// concatenation of the chunks handed to `sink` is byte-identical to
1178: /// [`query_json_stream_with_budget`] for the same query and budget.
1179: pub fn query_json_stream_prepared_with_budget(
1180:     graph: &Graph,
1181:     prepared: &PreparedQuery,
1182:     budget: &QueryBudget,
1183:     mut sink: impl FnMut(String) -> std::ops::ControlFlow<()>,
1184: ) -> Result<(), String> {
1185:     let q = &prepared.query;
1186:     let active = active_dataset(graph, q);
1187:     let graph = active.as_ref().unwrap_or(graph);
1188:     let _view_scope = view_scope(&active);
1189:     let _guard = exec::budget::install(budget);
1190:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1191:     match q {
1192:         Query::Select { pattern, .. } => {
1193:             exec::eval_select_json_emit(graph, pattern, Some(JSON_CHUNK_BYTES), &mut sink)
1194:         }
1195:         Query::Ask { pattern, .. } => {
1196:             let doc = format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?);
1197:             let _ = sink(doc);
1198:             Ok(())
1199:         }
1200:         _ => Err("only SELECT and ASK queries are supported".into()),
1201:     }
1202: }
1203: 
1204: /// Counts the solutions of a SELECT query *without* materialising the result
1205: /// terms (the id-level row count equals the solution count). Used to measure
1206: /// engine compute in isolation from result serialisation.
1207: pub fn count(graph: &Graph, sparql: &str) -> Result<usize, String> {
1208:     count_with_budget(graph, sparql, &QueryBudget::unlimited())
1209: }
1210: 
1211: /// [`count`] under a cooperative [`QueryBudget`] (the server's budgeted ASK path).
1212: pub fn count_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<usize, String> {
1213:     count_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
1214: }
1215: 
1216: /// [`count`] over a [`PreparedQuery`] — no per-execution parse.
1217: pub fn count_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<usize, String> {
1218:     count_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
1219: }
1220: 
1221: /// [`count_prepared`] under a cooperative [`QueryBudget`].
1222: pub fn count_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<usize, String> {
1223:     let q = &prepared.query;
1224:     let active = active_dataset(graph, q);
1225:     let graph = active.as_ref().unwrap_or(graph);
1226:     let _view_scope = view_scope(&active);
1227:     let _guard = exec::budget::install(budget);
1228:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
1229:     match q {
1230:         Query::Select { pattern, .. } => exec::count_select(graph, pattern),
1231:         // An ASK counts its unit row: 1 when satisfiable, 0 otherwise.
1232:         Query::Ask { pattern, .. } => Ok(usize::from(exec::eval_ask(graph, pattern)?)),
1233:         _ => Err("only SELECT and ASK queries are supported".into()),
1234:     }
1235: }
1236: 
1237: #[derive(Debug)]
1238: pub struct QueryResult {
1239:     pub vars: Vec<Variable>,
1240:     /// Each row has one entry per `vars` position; `None` is unbound.
1241:     pub rows: Vec<Vec<Option<Term>>>,
1242: }
1243: 
1244: impl QueryResult {
1245:     pub fn len(&self) -> usize {
1246:         self.rows.len()
1247:     }
1248: 
```



Frozen source crates/sparq-engine/src/cache.rs:345-382
```rust
345:             hits: inner.hits,
346:             misses: inner.misses,
347:             entries: inner.map.len(),
348:         }
349:     }
350: 
351:     /// The configured maximum number of resident entries.
352:     pub fn capacity(&self) -> usize {
353:         self.capacity
354:     }
355: }
356: 
357: /// Evaluate one SELECT / ASK query to a [`QueryResult`], mirroring
358: /// [`crate::query_prepared_with_budget`] (the dataset / base / budget wiring) so a
359: /// cached result is identical to the uncached path.
360: fn eval(
361:     graph: &sparq_core::Graph,
362:     query: &Query,
363:     budget: &QueryBudget,
364: ) -> Result<QueryResult, String> {
365:     let active = crate::active_dataset(graph, query);
366:     let graph = active.as_ref().unwrap_or(graph);
367:     let _view_scope = crate::view_scope(&active);
368:     let _guard = exec::budget::install(budget);
369:     exec::set_query_base(query.base_iri().map(|b| b.as_str()));
370:     match query {
371:         Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
372:         Query::Ask { pattern, .. } => Ok(QueryResult {
373:             vars: Vec::new(),
374:             rows: if exec::eval_ask(graph, pattern)? {
375:                 vec![Vec::new()]
376:             } else {
377:                 Vec::new()
378:             },
379:         }),
380:         _ => Err("result cache only stores SELECT and ASK queries".into()),
381:     }
382: }
```



Frozen source crates/sparq-engine/src/construct.rs:35-160
```rust
35:     construct_with_budget(graph, sparql, &QueryBudget::unlimited())
36: }
37: 
38: /// [`construct`] under a cooperative [`QueryBudget`] (deadline / max WHERE-solution rows).
39: pub fn construct_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<Triple>, String> {
40:     construct_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
41: }
42: 
43: /// [`construct`] over a [`PreparedQuery`] — no per-execution parse.
44: pub fn construct_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<Vec<Triple>, String> {
45:     construct_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
46: }
47: 
48: /// [`construct_prepared`] under a cooperative [`QueryBudget`].
49: pub fn construct_prepared_with_budget(
50:     graph: &Graph,
51:     prepared: &PreparedQuery,
52:     budget: &QueryBudget,
53: ) -> Result<Vec<Triple>, String> {
54:     let q = prepared.query();
55:     let active = crate::active_dataset(graph, q);
56:     let graph = active.as_ref().unwrap_or(graph);
57:     let _view_scope = crate::view_scope(&active);
58:     match q {
59:         Query::Construct { template, pattern, .. } => {
60:             let _guard = crate::exec::budget::install(budget);
61:             let solutions = crate::exec::eval_select(graph, pattern)?;
62:             Ok(instantiate(template, &solutions))
63:         }
64:         _ => Err("construct() requires a CONSTRUCT query".into()),
65:     }
66: }
67: 
68: /// Executes a DESCRIBE query, returning the union of the concise bounded
69: /// descriptions (see module docs) of every described resource.
70: pub fn describe(graph: &Graph, sparql: &str) -> Result<Vec<Triple>, String> {
71:     describe_with_budget(graph, sparql, &QueryBudget::unlimited())
72: }
73: 
74: /// [`describe`] under a cooperative [`QueryBudget`] (deadline / max rows).
75: pub fn describe_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<Triple>, String> {
76:     describe_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
77: }
78: 
79: /// [`describe`] over a [`PreparedQuery`] — no per-execution parse.
80: pub fn describe_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<Vec<Triple>, String> {
81:     describe_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
82: }
83: 
84: /// [`describe_prepared`] under a cooperative [`QueryBudget`].
85: pub fn describe_prepared_with_budget(
86:     graph: &Graph,
87:     prepared: &PreparedQuery,
88:     budget: &QueryBudget,
89: ) -> Result<Vec<Triple>, String> {
90:     let q = prepared.query();
91:     let active = crate::active_dataset(graph, q);
92:     let graph = active.as_ref().unwrap_or(graph);
93:     let _view_scope = crate::view_scope(&active);
94:     match q {
95:         Query::Describe { pattern, .. } => {
96:             let _guard = crate::exec::budget::install(budget);
97:             let solutions = crate::exec::eval_select(graph, pattern)?;
98:             cbd(graph, &solutions)
99:         }
100:         _ => Err("describe() requires a DESCRIBE query".into()),
101:     }
102: }
103: 
104: /// Executes a CONSTRUCT *or* DESCRIBE query and returns the resulting RDF graph as a
105: /// deduplicated triple list — the form-agnostic producer behind the graph-valued query
106: /// forms. CONSTRUCT instantiates its template per WHERE solution; DESCRIBE returns each
107: /// described resource's concise bounded description (see module docs). A SELECT/ASK query
108: /// is rejected. Callers wanting a serialised string use [`construct_ntriples`].
109: pub fn construct_or_describe(graph: &Graph, sparql: &str) -> Result<Vec<Triple>, String> {
110:     construct_or_describe_with_budget(graph, sparql, &QueryBudget::unlimited())
111: }
112: 
113: /// [`construct_or_describe`] under a cooperative [`QueryBudget`].
114: pub fn construct_or_describe_with_budget(
115:     graph: &Graph,
116:     sparql: &str,
117:     budget: &QueryBudget,
118: ) -> Result<Vec<Triple>, String> {
119:     let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
120:     let active = crate::active_dataset(graph, &q);
121:     let graph = active.as_ref().unwrap_or(graph);
122:     let _view_scope = crate::view_scope(&active);
123:     let _guard = crate::exec::budget::install(budget);
124:     match q {
125:         Query::Construct { template, pattern, .. } => {
126:             let solutions = crate::exec::eval_select(graph, &pattern)?;
127:             Ok(instantiate(&template, &solutions))
128:         }
129:         Query::Describe { pattern, .. } => {
130:             let solutions = crate::exec::eval_select(graph, &pattern)?;
131:             cbd(graph, &solutions)
132:         }
133:         _ => Err("construct_or_describe() requires a CONSTRUCT or DESCRIBE query".to_string()),
134:     }
135: }
136: 
137: /// Executes a CONSTRUCT *or* DESCRIBE query and serialises the resulting graph as
138: /// N-Triples. N-Triples is a syntactic subset of Turtle, so the returned string is
139: /// also a valid `text/turtle` document (the server serves it under either type).
140: pub fn construct_ntriples(graph: &Graph, sparql: &str) -> Result<String, String> {
141:     construct_ntriples_with_budget(graph, sparql, &QueryBudget::unlimited())
142: }
143: 
144: /// [`construct_ntriples`] under a cooperative [`QueryBudget`].
145: pub fn construct_ntriples_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
146:     let triples = construct_or_describe_with_budget(graph, sparql, budget)?;
147:     Ok(triples_to_ntriples(&triples))
148: }
149: 
150: /// Serialises triples as canonical N-Triples (one `s p o .` line per triple).
151: pub fn triples_to_ntriples(triples: &[Triple]) -> String {
152:     use std::fmt::Write;
153:     let mut out = String::with_capacity(triples.len() * 64);
154:     for t in triples {
155:         // oxrdf's Display for the term types is canonical N-Triples syntax.
156:         let _ = writeln!(out, "{} {} {} .", t.subject, t.predicate, t.object);
157:     }
158:     out
159: }
160: 
```



Frozen source crates/sparq-engine/src/explain.rs:75-140
```rust
75:     // [OPUS-4.8] (sq-7d3dj.30.1) ANALYZE the ACTUAL executed plan (feature-gated rewrite).
76:     #[cfg(feature = "algebra-rewrite")]
77:     let q = crate::rewrite::rewrite_query(q);
78:     let active = crate::active_dataset(graph, &q);
79:     let graph = active.as_ref().unwrap_or(graph);
80:     let _view_scope = crate::view_scope(&active);
81:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
82:     let (form, pattern) = query_form_pattern(&q);
83:     if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
84:         return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use EXPLAIN for CONSTRUCT/DESCRIBE)".into());
85:     }
86: 
87:     let mut out = String::new();
88:     let _ = writeln!(out, "EXPLAIN ANALYZE ({form}) — plan below, then the per-operator execution trace.");
89:     let _ = writeln!(out, "Plan:");
90:     render_pattern(graph, pattern, &mut out, 1)?;
91: 
92:     // Execute under the budget with the operator trace installed.
93:     let _bguard = exec::budget::install(budget);
94:     let _tguard = exec::trace::install();
95:     #[cfg(not(target_arch = "wasm32"))]
96:     let start = std::time::Instant::now();
97:     let total_rows = match &q {
98:         Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
99:         Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
100:         _ => unreachable!(),
101:     };
102:     #[cfg(not(target_arch = "wasm32"))]
103:     let total_nanos = start.elapsed().as_nanos() as u64;
104:     #[cfg(target_arch = "wasm32")]
105:     let total_nanos = 0u64;
106:     let nodes = exec::trace::take();
107: 
108:     let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
109:     for n in &nodes {
110:         let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
111:     }
112:     let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
113:     Ok(out)
114: }
115: 
116: fn query_form_pattern(q: &Query) -> (&'static str, &GraphPattern) {
117:     match q {
118:         Query::Select { pattern, .. } => ("SELECT", pattern),
119:         Query::Ask { pattern, .. } => ("ASK", pattern),
120:         Query::Construct { pattern, .. } => ("CONSTRUCT", pattern),
121:         Query::Describe { pattern, .. } => ("DESCRIBE", pattern),
122:     }
123: }
124: 
125: fn indent(depth: usize) -> String {
126:     "  ".repeat(depth)
127: }
128: 
129: fn fmt_nanos(n: u64) -> String {
130:     if n >= 1_000_000_000 {
131:         format!("{:.2}s", n as f64 / 1e9)
132:     } else if n >= 1_000_000 {
133:         format!("{:.2}ms", n as f64 / 1e6)
134:     } else if n >= 1_000 {
135:         format!("{:.1}µs", n as f64 / 1e3)
136:     } else {
137:         format!("{n}ns")
138:     }
139: }
140: 
```



Frozen source crates/sparq-engine/src/explain_json.rs:195-255
```rust
195:     let _view_scope = crate::view_scope(&active);
196:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
197:     let pattern = query_pattern(&q);
198:     Ok(plan_from_pattern(graph, pattern))
199: }
200: 
201: /// Builds a structured plan tree AND executes the query (SELECT / ASK only),
202: /// filling in each node's `actual` output rows, wall `nanos`, and per-operator
203: /// `q_error`. The ANALYZE analogue of [`explain_plan`]; mirrors
204: /// [`crate::explain_analyze`] but returns a typed [`PlanNode`].
205: pub fn explain_plan_analyze(graph: &Graph, sparql: &str) -> Result<PlanNode, String> {
206:     explain_plan_analyze_with_budget(graph, sparql, &QueryBudget::unlimited())
207: }
208: 
209: /// [`explain_plan_analyze`] under a cooperative [`QueryBudget`] (deadline / max rows).
210: pub fn explain_plan_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<PlanNode, String> {
211:     let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
212:     let active = crate::active_dataset(graph, &q);
213:     let graph = active.as_ref().unwrap_or(graph);
214:     let _view_scope = crate::view_scope(&active);
215:     exec::set_query_base(q.base_iri().map(|b| b.as_str()));
216:     if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
217:         return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use explain_plan for CONSTRUCT/DESCRIBE)".into());
218:     }
219: 
220:     // Execute under the budget with the operator trace installed (exactly as the
221:     // text `explain_analyze` does), then reconstruct the typed tree from the trace.
222:     let _bguard = exec::budget::install(budget);
223:     let _tguard = exec::trace::install();
224:     match &q {
225:         Query::Select { pattern, .. } => {
226:             exec::eval_select(graph, pattern)?;
227:         }
228:         Query::Ask { pattern, .. } => {
229:             exec::eval_ask(graph, pattern)?;
230:         }
231:         _ => unreachable!(),
232:     }
233:     let nodes = exec::trace::take();
234:     tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
235: }
236: 
237: /// The query's root graph pattern (independent of form).
238: fn query_pattern(q: &Query) -> &spargebra::algebra::GraphPattern {
239:     match q {
240:         Query::Select { pattern, .. }
241:         | Query::Ask { pattern, .. }
242:         | Query::Construct { pattern, .. }
243:         | Query::Describe { pattern, .. } => pattern,
244:     }
245: }
246: 
247: // ---- planning-only tree (no execution) ----------------------------------------
248: 
249: /// Builds a planning-only [`PlanNode`] tree from the algebra, attaching the BGP
250: /// cardinality estimate to conjunctive nodes (the only operators the planner sizes)
251: /// — the same estimate the text EXPLAIN prints, computed by replaying the planner
252: /// (`exec::bgp_estimate`).
253: fn plan_from_pattern(graph: &Graph, p: &spargebra::algebra::GraphPattern) -> PlanNode {
254:     use spargebra::algebra::GraphPattern as G;
255:     if exec::is_conjunctive(p) {
```



Frozen source crates/sparq-engine/src/update.rs:510-575
```rust
510: /// re-parse — a hostile bound value can never re-enter the parser). [OPUS-4.8] (sq-rp3um)
511: fn apply_update_rebuild(graph: &Graph, upd: &Update) -> Result<Graph, String> {
512:     let mut ds = Dataset::decode(graph);
513:     for op in &upd.operations {
514:         apply_op(&mut ds, op)?;
515:     }
516:     Ok(ds.build())
517: }
518: 
519: /// [OPUS-4.8] (sq-rp3um) [`update`] over an ALREADY-PARSED bound `Update` — the rebuild
520: /// path for a parameterized [`crate::PreparedUpdate`] (`params` feature).
521: #[cfg(feature = "params")]
522: pub(crate) fn update_prepared_impl(graph: &Graph, upd: &Update) -> Result<Graph, String> {
523:     apply_update_rebuild(graph, upd)
524: }
525: 
526: /// [OPUS-4.8] (sq-rp3um) [`update_in_place_with_budget`] over an ALREADY-PARSED bound
527: /// `Update` (parameterized [`crate::PreparedUpdate`], `params` feature). Applies the
528: /// bound algebra in place through the delta overlay without re-serialising it.
529: #[cfg(feature = "params")]
530: pub(crate) fn update_in_place_prepared_with_budget(
531:     graph: &mut Graph,
532:     upd: &Update,
533:     budget: &crate::QueryBudget,
534: ) -> Result<(), String> {
535:     let _budget = crate::exec::budget::install(budget);
536:     apply_update_in_place(graph, upd, None)
537: }
538: 
539: // --- the delta-overlay path ------------------------------------------------------------------
540: 
541: /// Groups (graph-slot, triple) pairs per graph slot, preserving first-seen slot order.
542: fn group_by_slot(items: Vec<SlotTriple>) -> Vec<(GraphSlot, Vec<TripleTerms>)> {
543:     let mut out: Vec<(GraphSlot, Vec<TripleTerms>)> = Vec::new();
544:     for (slot, t) in items {
545:         match out.iter_mut().find(|(s, _)| *s == slot) {
546:             Some((_, v)) => v.push(t),
547:             None => out.push((slot, vec![t])),
548:         }
549:     }
550:     out
551: }
552: 
553: /// Applies one insert/delete batch to the graph slot through the delta overlay,
554: /// auto-creating an empty named graph for an insert into an absent one.
555: fn apply_slot_delta(
556:     graph: &mut Graph,
557:     slot: &GraphSlot,
558:     inserts: &[TripleTerms],
559:     deletes: &[TripleTerms],
560: ) -> Result<(), String> {
561:     match slot {
562:         None => graph.apply_delta(inserts, deletes),
563:         Some(name) => {
564:             if let Some(i) = graph.named.iter().position(|(n, _)| n == name) {
565:                 return graph.named[i].1.apply_delta(inserts, deletes);
566:             }
567:             if inserts.is_empty() {
568:                 return Ok(()); // deleting from an absent graph is a no-op
569:             }
570:             // [OPUS-4.8] (sq-7cxr, gh-44) Route NEW named-graph creation through
571:             // `Graph::ensure_named` so that on a DIRECTORY-BACKED parent the sub-graph is
572:             // born durable (its own WAL + manifest entry) — its first triples then persist
573:             // across a restart like any other write. For an in-memory parent this is the
574:             // unchanged `empty_graph()` path.
575:             let i = graph.ensure_named(name)?;
```



Frozen source crates/sparq-engine/src/update.rs:700-815
```rust
700: /// [`update_in_place_atomic_with_budget`] to bound a `DELETE/INSERT … WHERE`.
701: pub fn update_in_place_atomic(graph: &mut Graph, sparql: &str) -> Result<(), String> {
702:     update_in_place_atomic_with_budget(graph, sparql, &crate::QueryBudget::unlimited())
703: }
704: 
705: /// [OPUS-4.8] (sq-o1wp) [`update_in_place_atomic`] under a cooperative [`crate::QueryBudget`] (the
706: /// budget bounds a `DELETE/INSERT … WHERE` exactly as in [`update_in_place_with_budget`]). The
707: /// fork is applied under the budget; a budget-exceeded error rolls the request back whole.
708: pub fn update_in_place_atomic_with_budget(
709:     graph: &mut Graph,
710:     sparql: &str,
711:     budget: &crate::QueryBudget,
712: ) -> Result<(), String> {
713:     let mut working = graph.fork();
714:     update_in_place_with_budget(&mut working, sparql, budget)?;
715:     *graph = working;
716:     Ok(())
717: }
718: 
719: /// The shared in-place apply core. When `sink` is `Some`, every operation's RESOLVED effect is
720: /// recorded into it so a durable mirror can replay the exact committed delta (see
721: /// [`update_in_place_capturing`]); when `None`, capture is fully elided.
722: fn update_in_place_core(
723:     graph: &mut Graph,
724:     sparql: &str,
725:     budget: &crate::QueryBudget,
726:     sink: EffectSink,
727: ) -> Result<(), String> {
728:     let _budget = crate::exec::budget::install(budget);
729:     let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
730:     apply_update_in_place(graph, &upd, sink)
731: }
732: 
733: /// The shared per-operation in-place apply loop over an ALREADY-PARSED `Update`.
734: /// Split out of [`update_in_place_core`] so the parameterized prepared-update path
735: /// ([`crate::PreparedUpdate`], `params` feature) can apply a bound algebra without
736: /// re-serialising it to a string. [OPUS-4.8] (sq-rp3um)
737: fn apply_update_in_place(
738:     graph: &mut Graph,
739:     upd: &Update,
740:     mut sink: EffectSink,
741: ) -> Result<(), String> {
742:     for op in &upd.operations {
743:         match op {
744:             GraphUpdateOperation::InsertData { data } => {
745:                 // Fresh blank nodes for the whole operation (roborev 1646) — see apply_op.
746:                 let mut fresh = FreshBnodes { map: FxHashMap::default() };
747:                 let triples: Vec<_> = data.iter().map(|q| quad_to_triple_fresh(q, &mut fresh)).collect();
748:                 for (slot, ins) in group_by_slot(triples) {
749:                     apply_slot_delta(graph, &slot, &ins, &[])?;
750:                     record_delta(&mut sink, &slot, &ins, &[]);
751:                 }
752:             }
753:             GraphUpdateOperation::DeleteData { data } => {
754:                 for (slot, del) in group_by_slot(data.iter().map(ground_quad_to_triple).collect()) {
755:                     apply_slot_delta(graph, &slot, &[], &del)?;
756:                     record_delta(&mut sink, &slot, &[], &del);
757:                 }
758:             }
759:             // [OPUS-4.8] (sq-glw2) CLEAR keeps the named-graph SLOT but empties it. On a
760:             // directory-backed parent, retract the existing graph's contents through its own WAL
761:             // (`Graph::clear_named_durable`) so the emptied state is fsync'd BEFORE the ack — the
762:             // old `entry.1 = empty_graph()` dropped the sub-graph's WAL/dir association, so the
763:             // clear was durable only at the next compaction. Absent graph: a no-op; in-memory
764:             // parent: a plain store clear (unchanged).
765:             GraphUpdateOperation::Clear { graph: target, .. } => {
766:                 match target {
767:                     GraphTarget::DefaultGraph => replace_default(graph)?,
768:                     GraphTarget::NamedNode(n) => {
769:                         graph.clear_named_durable(&Term::NamedNode(n.clone()))?;
770:                     }
771:                     GraphTarget::NamedGraphs => clear_all_named_durable(graph)?,
772:                     GraphTarget::AllGraphs => {
773:                         replace_default(graph)?;
774:                         clear_all_named_durable(graph)?;
775:                     }
776:                 }
777:                 if let Some(s) = &mut sink {
778:                     s.push(UpdateEffect::Clear(target.clone()));
779:                 }
780:             }
781:             // [OPUS-4.8] (sq-glw2) DROP makes the named graph cease to exist. On a directory-
782:             // backed parent, `Graph::drop_named_durable` removes the sub-dir + manifest entry
783:             // durably (fsync'd) so the removal survives a reopen immediately — the old
784:             // `graph.named.retain(...)` dropped only the in-memory entry, leaving the on-disk
785:             // sub-dir + manifest entry so a reopen RESTORED the dropped graph. Absent graph: a
786:             // no-op; in-memory parent: a plain entry removal (unchanged).
787:             GraphUpdateOperation::Drop { graph: target, .. } => {
788:                 match target {
789:                     GraphTarget::DefaultGraph => replace_default(graph)?,
790:                     GraphTarget::NamedNode(n) => {
791:                         graph.drop_named_durable(&Term::NamedNode(n.clone()))?;
792:                     }
793:                     GraphTarget::NamedGraphs => drop_all_named_durable(graph)?,
794:                     GraphTarget::AllGraphs => {
795:                         replace_default(graph)?;
796:                         drop_all_named_durable(graph)?;
797:                     }
798:                 }
799:                 if let Some(s) = &mut sink {
800:                     s.push(UpdateEffect::Drop(target.clone()));
801:                 }
802:             }
803:             GraphUpdateOperation::Create { graph: name, .. } => {
804:                 // [OPUS-4.8] (sq-7cxr, gh-44) `ensure_named` makes the new graph DURABLE on a
805:                 // directory-backed parent (empty graphs are persisted via the manifest, so an
806:                 // empty CREATE survives a restart) and is a no-op when it already exists.
807:                 let name = Term::NamedNode(name.clone());
808:                 graph.ensure_named(&name)?;
809:                 if let Some(s) = &mut sink {
810:                     s.push(UpdateEffect::Create(name));
811:                 }
812:             }
813:             GraphUpdateOperation::Load { silent, source, destination } => {
814:                 match load_document(source.as_str()) {
815:                     Ok(triples) => {
```
