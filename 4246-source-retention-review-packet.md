# Focused independent review packet: PR6469 default-text retention

Exact candidate: `6334b338587fe5c635c69a09e134917ec35eaaca`, delta from reviewed `b86b5d5ad84bce900762defa094630eb358317a6`; base `a42a9e89dec485f6a319c47cb3635c59cb5a2270`.

Review this one-file source correction and the outside-tree compiler-derived declaration proposal. Actual prior Opus verdict was `approve_for_validation`, with no source blocker; normal Linux CI then exposed the mechanical neutral-tree failure described below. This is a focused continuation, not a new default-on performance admission. No code in the gate/proof tool or any manifest/default feature changed. Source is committed; the proposed declaration is evidence only.

Questions: Does the original default function/statement remain intact while the opt-in implementation and tests retain their meaning? Do the recorded actual compiler builds discharge both independent obligations under the unchanged repository protocol? Is the exact outside-tree proposal justified, with honest attribution and local-versus-CI limits? Full Linux validation/publication remains root-owned. No source/default enabling, permission changes or policy weakening is requested.

The unchanged proof implementation below includes its historical `[OPUS-5]` output template. This is not a claim that Opus executed or authored these measurements: GPT-6 Astra xhigh implemented this correction and executed the one local proof. Independent follow-up review is pending.

## Decision and limits

````markdown
Correction **6334b338587fe5c635c69a09e134917ec35eaaca** is clean and review-ready: one source file, +18/-13. The default function and tombstone insertion retain exact base compiled text; the opt-in algorithm is unchanged. No declaration has been added to source.

- Native validation: 4 feature-off + 9 feature-on tests pass; all six compiled controls are killed; all-target core clippy passes off/on.
- Original compiler protocol: **declared**, exit 0 in 544.822s; no retry/cutoff. Minimum observed free disk 11,595,014,144 bytes.
- Addition-neutral equals head; deletion-neutral equals base. All four bundles are 1,560,265 bytes. Base/head differ 12 bytes; the two successful obligations establish the repository protocol's metadata-only classification.
- Exact proposal: `proof/declaration-proposal.json`, outside the source tree. Its inherited `[OPUS-5]` template is not execution provenance: Astra ran this proof; actual independent review is pending.

The prior Linux b86 bundles were 24 bytes smaller; no cross-host byte comparison or full CI result is claimed. Default-off experimental costs and remaining full Linux validation are unchanged. Root owns actual Opus review, any declaration/publication, and normal CI.
````

## Exact proposed source delta

````diff
diff --git a/crates/sparq-core/src/store.rs b/crates/sparq-core/src/store.rs
index c495a305b..f965d738c 100644
--- a/crates/sparq-core/src/store.rs
+++ b/crates/sparq-core/src/store.rs
@@ -295,21 +295,26 @@ impl Overlay {
     /// correction to a base range count. The `added` side rides the cached perm-sorted
     /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
     /// default; the experimental feature opts into lazy sorted projections.
+    #[cfg(not(feature = "overlay-deleted-projections"))]
+    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
+        let order = perm.order();
+        let add = self.added_rows(perm, lo, hi).len();
+        let del = self
+            .deleted
+            .iter()
+            .filter(|t| {
+                let r = [t[order[0]], t[order[1]], t[order[2]]];
+                r >= lo && r <= hi
+            })
+            .count();
+        (add, del)
+    }
+
+    /// [GPT-6 Astra] Experimental range correction using cached deletion projections.
+    #[cfg(feature = "overlay-deleted-projections")]
     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
         let add = self.added_rows(perm, lo, hi).len();
-        #[cfg(feature = "overlay-deleted-projections")]
         let del = self.deleted_count(perm, lo, hi);
-        #[cfg(not(feature = "overlay-deleted-projections"))]
-        let del = {
-            let order = perm.order();
-            self.deleted
-                .iter()
-                .filter(|t| {
-                    let r = [t[order[0]], t[order[1]], t[order[2]]];
-                    r >= lo && r <= hi
-                })
-                .count()
-        };
         (add, del)
     }
 
@@ -961,7 +966,7 @@ impl TripleStore {
                 #[cfg(feature = "overlay-deleted-projections")]
                 { deleted_changed |= ov.deleted.insert(*t); }
                 #[cfg(not(feature = "overlay-deleted-projections"))]
-                { ov.deleted.insert(*t); }
+                ov.deleted.insert(*t);
             }
         }
         for t in inserts {
````

## Local commit provenance

````text
commit 6334b338587fe5c635c69a09e134917ec35eaaca
Author:     Jesse Wright <63333554+jeswr@users.noreply.github.com>
AuthorDate: Wed Sep 9 15:08:37 2026 +0100
Commit:     Jesse Wright <63333554+jeswr@users.noreply.github.com>
CommitDate: Wed Sep 9 15:08:37 2026 +0100

    fix(core): preserve default overlay counting source under opt-in cfg
    
    Keep the original linear function and tombstone insert statement intact so the existing feature-off neutral-tree proof can evaluate the guarded additions. Preserve experimental behavior and accurate cache documentation; no gate or declaration change.
    
    Co-Authored-By: OpenAI GPT-6 Astra <noreply@openai.com>
````

## Prior exact Linux blocker (authoritative saved log diagnosis)

````json
{
  "observed_ci": {
    "source_identity": "Saved exact job log1850-1875 names basea42/headb86/PR6469; Git merge-base independently equalsa42. Both real base/head release-wasm builds succeeded.",
    "leg2_failure": {
      "log_lines": "1850-1856",
      "base_bytes": 1560241,
      "head_bytes": 1560241,
      "differing_bytes_from_autoderive": 12,
      "size_delta": 0,
      "reason": "Byte-for-byte inequality with no new per-PR declaration and unchanged legacy token1726. Equal size does not pass this gate."
    },
    "auto_derivation": {
      "log_lines": "1914-1967",
      "outcome": "neutral-build-failed",
      "exit": 2,
      "compiler_error": "E0425 cannot find value del at crates/sparq-core/src/store.rs:313:15; expression (add, del).",
      "reason": "Blanking added lines broke the first neutral build. No neutral/head equality was measured; deletion-neutral/base obligation was not reached. Refusal is expected for this source rewrite, not a failed head build.",
      "deleted_nonblank_lines": 20,
      "deleted_comment_lines": 10,
      "deleted_compiled_lines": 10,
      "added_nonblank_lines_blanked": 1129,
      "neutral_tree_files_mutated": 14
    },
    "aggregate": "Saved aggregate34358709393 log fail-fast names only artifact-exact-equality failure with29 siblings still running. It does not establish final results of those siblings."
  },
  "mechanism": [
    {
      "source": "scripts/feature_off_autodeclare.py:201-263,423-503",
      "finding": "Committed parser identifies added/removed line numbers from exact git diff --unified=0. First neutral is head with every added non-manifest line blanked (line counts retained), and changed manifests restored from base. It is not a semantic diff/reformatter equivalence checker."
    },
    {
      "source": "crates/sparq-core/src/store.rs:298-314 versus base259-271; static-reproduction.json and head-neutral-excerpt.txt",
      "finding": "The base let order and let del/filter/count statements were deleted. Both replacement cfg-on/cfg-off bindings are added lines. Blanking additions leaves original let add and final (add,del), without del. Exact in-memory neutral reproduces the reported source failure without invoking Rust or any builder."
    },
    {
      "source": "crates/sparq-core/src/store.rs:958-969 versus base912; deletion-neutral-excerpt.txt",
      "finding": "Original tombstone insert statement was also replaced by cfg blocks. If the addition obligation were repaired alone, base-side deleted-line blanking would still remove required count binding and the insert statement. Preserve all original compiled statements to make both obligations meaningful."
    },
    {
      "source": "scripts/feature_off_autodeclare.py:505-545",
      "finding": "Deletion-neutral proof is independently required for nonblank removals. Restoring only compile ability or tolerating12 bytes would not discharge byte equality in either direction."
    }
  ]
}
````

## Prior actual Opus result: verdict and required validation excerpt (full result archived)

````markdown
# Independent review — sparq #4246 (experimental default-off boundary)

**Reviewed head:** `b86b5d5ad84bce900762defa094630eb358317a6` (prior reviewed `cb638a42a54fc7c9e11e9101587910e668a5f92a`; main runtime `a42a9e89dec485f6a319c47cb3635c59cb5a2270`; timing control `0613b5c3d1f2b1174bcccfcd1a6bb64551ca9485`)

**Verdict: `approve_for_validation`.** The `cfg` boundary is complete and the default build is restored to main's linear counting and main's `Overlay` representation; the reviewed feature-on repair is intact and still guarded by killed mutants. I approve **only** progression to full validation. This is not a default-on admission, not a crossover finding, and not a canonical performance result.

## Blockers

**None.** I identify no correctness defect and no gap in the default-off boundary.

## Optional nits (do not bundle; no change requested)

- `apply_delta` duplicates `ov.deleted.insert(*t)` across two `cfg` blocks; a single `let changed = ...` with a gated use would read better. Cosmetic; clippy is clean both ways.
- The `deleted_by_perm` sole-mutator invariant is still prose-only (now only relevant under the feature); the prior `debug_assertions` cross-check suggestion still applies and still isn't required.
- `invalidate_added` still evicts on delete-only/no-op batches — unchanged, pre-existing in main, out of scope.

## Required validation (pending; none executed here)

1. Actual CI execution of the new leg: build, `cargo test`, and **`cargo clippy --all-targets -D warnings` with the feature** (local clippy was `--lib` only, so the new test module is not yet warning-checked).
2. Default-state gates unchanged: full workspace/nextest shards, W3C conformance, wasm bundle-size ratchet, canonical performance ratchets, mapped-I/O and WAL-replay suites, Miri.
3. Confirm the leg name is discovered as a required check by the ci-summary aggregator, and that the regenerated golden is byte-identical under CI's own interpreter.
4. One bounded check specific to this boundary: confirm no gating leg (perf ratchet, wasm size, memory ratchet) builds `sparq-core` with `--all-features`, which would silently enable the experiment inside a default-state gate; if one does, either exclude the feature there or record it knowingly.
5. Linux preflight (the local exit 1 is the known Bash-3 `mapfile` at `scripts/check-privacy-claims.sh:92`, environmental).

Publication, full-gate execution and any performance admission remain with root; this review authorizes progression to validation only.
````

## Current full Overlay representation, initialization, counting, merge, invalidation and heap bodies — store.rs:158–362

````rust
158: 
159: /// Pending updates layered over the immutable base indexes (the T17 delta-overlay):
160: /// triples INSERTED since the last compaction, and base triples DELETED since then.
161: /// Consulted at scan time — the base stays immutable (and mmap-able), and an update
162: /// batch costs O(batch) instead of the O(n) full rebuild. Invariants kept by
163: /// [`TripleStore::apply_delta`]: `added` is canonical-SPO sorted + deduplicated and
164: /// DISJOINT from both the base and `deleted`; `deleted` only ever holds base triples.
165: ///
166: /// `Clone` because [`TripleStore::fork`] carries the overlay into the forked store
167: /// BY VALUE (O(overlay), bounded by the compaction policy) while the base indexes are
168: /// shared structurally.
169: #[derive(Default, Clone)]
170: struct Overlay {
171:     added: Vec<[Id; 3]>,
172:     deleted: FxHashSet<[Id; 3]>,
173:     /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
174:     /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
175:     added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
176:     /// [GPT-6 Astra] Lazy deletion projections for range counts (#4246). Keep the
177:     /// hash set above for merge membership; even SPO needs its own sorted projection.
178:     /// Full use retains up to `BUILT.len()` vectors, each with `deleted.len()`
179:     /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
180:     /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
181:     /// these projections when a tombstone is inserted or removed.
182:     #[cfg(feature = "overlay-deleted-projections")]
183:     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
184: }
185: 
186: impl Overlay {
187:     /// All of `added` projected into `perm` column order and SORTED in it — computed
188:     /// ONCE per permutation and reused until `added` changes (sq-7d3dj.16).
189:     ///
190:     /// Built LAZILY on the first scan that needs this permutation rather than eagerly
191:     /// for all six in [`TripleStore::apply_delta`]: a write batch then stays O(batch)
192:     /// (it only drops the caches, see [`Overlay::invalidate_added`]) instead of paying
193:     /// O(6·k log k) per call, and a store only ever materialises the projections its
194:     /// query mix actually scans — so the memory cost is bounded by the permutations in
195:     /// use, not a flat 6×. SPO needs no projection or sort at all: `added` is already
196:     /// canonical-SPO sorted, so that permutation ALIASES it and costs nothing.
197:     ///
198:     /// `OnceLock` (not `RefCell`) because scans take `&self` and `TripleStore` must stay
199:     /// `Sync`; concurrent readers share the synchronized initialization.
200:     fn added_sorted(&self, perm: Perm) -> &[[Id; 3]] {
201:         let order = perm.order();
202:         if order == [0, 1, 2] {
203:             return &self.added; // SPO: `added` is already the projection, already sorted
204:         }
205:         self.added_by_perm[perm as usize].get_or_init(|| {
206:             let mut rows: Vec<[Id; 3]> =
207:                 self.added.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
208:             rows.sort_unstable();
209:             rows
210:         })
211:     }
212: 
213:     /// Drops added projections before any nonempty delta, preserving prior behavior.
214:     fn invalidate_added(&mut self) {
215:         for slot in &mut self.added_by_perm {
216:             slot.take();
217:         }
218:     }
219: 
220:     /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
221:     #[cfg(feature = "overlay-deleted-projections")]
222:     fn invalidate_deleted(&mut self) {
223:         for slot in &mut self.deleted_by_perm {
224:             slot.take();
225:         }
226:     }
227: 
228:     /// [GPT-6 Astra] Counts deletions using a lazily sorted projection (#4246).
229:     /// The first request costs O(d log d) and one additional vector; later requests
230:     /// use two binary searches. Clone copies initialized vectors by value, and
231:     /// apply_delta invalidates only the mutated overlay under exclusive access.
232:     /// Concurrent first readers of the same permutation wait for its one sorting
233:     /// initializer; the cold sort is serialized for that permutation.
234:     #[cfg(feature = "overlay-deleted-projections")]
235:     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
236:         if self.deleted.is_empty() {
237:             return 0;
238:         }
239:         let rows = self.deleted_by_perm[perm as usize].get_or_init(|| {
240:             let order = perm.order();
241:             let mut rows: Vec<[Id; 3]> = self
242:                 .deleted
243:                 .iter()
244:                 .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
245:                 .collect();
246:             rows.sort_unstable();
247:             rows
248:         });
249:         rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
250:     }
251: 
252:     /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
253:     /// `perm` column order, SORTED in that order. A BORROWED sub-slice of the cached
254:     /// perm-sorted projection located by two binary searches — O(log k + m) on k
255:     /// insertions and m matches, and allocation-free.
256:     fn added_rows(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> &[[Id; 3]] {
257:         let rows = self.added_sorted(perm);
258:         let start = rows.partition_point(|r| *r < lo);
259:         let end = rows.partition_point(|r| *r <= hi);
260:         &rows[start..end]
261:     }
262: 
263:     /// Merges the (perm-sorted) base rows with the overlay for one scan: base rows whose
264:     /// canonical triple is deleted are dropped, and the matching `added` rows are merge-
265:     /// interleaved — so the output keeps the permutation's sort order, preserving the
266:     /// guarantees downstream merge joins rely on. `added` is disjoint from the base, so
267:     /// no duplicate handling is needed.
268:     fn merge(&self, base: &[[Id; 3]], perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
269:         let add = self.added_rows(perm, lo, hi);
270:         let order = perm.order();
271:         let mut out = Vec::with_capacity(base.len() + add.len());
272:         let mut ai = 0;
273:         let check_deleted = !self.deleted.is_empty();
274:         for &row in base {
275:             if check_deleted {
276:                 let mut spo = [0; 3];
277:                 spo[order[0]] = row[0];
278:                 spo[order[1]] = row[1];
279:                 spo[order[2]] = row[2];
280:                 if self.deleted.contains(&spo) {
281:                     continue;
282:                 }
283:             }
284:             while ai < add.len() && add[ai] < row {
285:                 out.push(add[ai]);
286:                 ai += 1;
287:             }
288:             out.push(row);
289:         }
290:         out.extend_from_slice(&add[ai..]);
291:         out
292:     }
293: 
294:     /// How many overlay triples fall in the `[lo, hi]` range of `perm` — the exact
295:     /// correction to a base range count. The `added` side rides the cached perm-sorted
296:     /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
297:     /// default; the experimental feature opts into lazy sorted projections.
298:     #[cfg(not(feature = "overlay-deleted-projections"))]
299:     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
300:         let order = perm.order();
301:         let add = self.added_rows(perm, lo, hi).len();
302:         let del = self
303:             .deleted
304:             .iter()
305:             .filter(|t| {
306:                 let r = [t[order[0]], t[order[1]], t[order[2]]];
307:                 r >= lo && r <= hi
308:             })
309:             .count();
310:         (add, del)
311:     }
312: 
313:     /// [GPT-6 Astra] Experimental range correction using cached deletion projections.
314:     #[cfg(feature = "overlay-deleted-projections")]
315:     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
316:         let add = self.added_rows(perm, lo, hi).len();
317:         let del = self.deleted_count(perm, lo, hi);
318:         (add, del)
319:     }
320: 
321:     fn is_empty(&self) -> bool {
322:         self.added.is_empty() && self.deleted.is_empty()
323:     }
324: 
325:     fn heap_bytes(&self) -> usize {
326:         // The cached perm-sorted projections are part of the overlay's footprint; SPO
327:         // aliases `added`; [GPT-6 Astra] deleted projections own all requested perms.
328:         let cached: usize = self
329:             .added_by_perm
330:             .iter()
331:             .filter_map(|slot| slot.get())
332:             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
333:             .sum();
334:         #[cfg(feature = "overlay-deleted-projections")]
335:         let cached = cached + self.deleted_by_perm.iter()
336:             .filter_map(|slot| slot.get())
337:             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
338:             .sum::<usize>();
339:         self.added.capacity() * std::mem::size_of::<[Id; 3]>()
340:             + self.deleted.capacity() * 13
341:             + cached
342:     }
343: }
344: 
345: pub struct TripleStore {
346:     // Each permutation in its column order, sorted (so binary search on a bound prefix
347:     // is a plain lexicographic comparison of the leading columns) — owned or mmap'd.
348:     //
349:     // Behind an `Arc` so [`fork`](Self::fork) can SHARE the immutable base indexes
350:     // across snapshot generations (the structural fork): every store is born
351:     // shareable, a fork is an Arc bump. The only post-build mutation,
352:     // [`decompress_to_ram`](Self::decompress_to_ram), goes through `Arc::get_mut`
353:     // (it runs on freshly opened, never-yet-shared stores). Cost when unused: one
354:     // extra pointer indirection per scan/estimate CALL (not per row) — measured in
355:     // the flat-read benchmark as within noise.
356:     perms: std::sync::Arc<[PermData; 6]>,
357:     // Per-predicate stats keyed by predicate id (for the cost-based planner).
358:     // Arc-shared across forks like the permutations (read-only after build).
359:     pred_stats: std::sync::Arc<FxHashMap<Id, PredStat>>,
360:     // The delta-overlay of pending updates, `None` when there are none — so the scan
361:     // hot path pays exactly one (perfectly predicted) branch when no update happened.
362:     // NOTE: `pred_stats` is not overlay-adjusted (planner estimates only); `estimate`
````

## Current contains, apply_delta, fork, heap and scan/estimate production callers — store.rs:894–1198

````rust
894:     pub fn pred_stat(&self, predicate: Id) -> Option<PredStat> {
895:         self.pred_stats.get(&predicate).copied()
896:     }
897: 
898:     pub fn len(&self) -> usize {
899:         let base = self.perms[0].len();
900:         match &self.overlay {
901:             Some(ov) => base + ov.added.len() - ov.deleted.len(),
902:             None => base,
903:         }
904:     }
905: 
906:     pub fn is_empty(&self) -> bool {
907:         self.len() == 0
908:     }
909: 
910:     /// Whether a delta-overlay of pending updates exists (i.e. updates were applied
911:     /// since the base was built / last compacted).
912:     pub fn has_overlay(&self) -> bool {
913:         self.overlay.is_some()
914:     }
915: 
916:     /// [OPUS-4.8] (sq-5lf) Strong-reference count of the `Arc`-shared base permutation
917:     /// indexes — i.e. how many stores currently SHARE this exact base storage. 1 for a
918:     /// freshly built / just-compacted store; bumps by one for each live
919:     /// [`fork`](Self::fork) / [`Graph::snapshot`](crate::Graph::snapshot) of it. Used to
920:     /// PROVE structural sharing in tests (a cheap snapshot bumps this count rather than
921:     /// duplicating the index memory). Two stores share a base iff this is > 1 and they
922:     /// were derived from the same lineage.
923:     pub fn base_strong_count(&self) -> usize {
924:         std::sync::Arc::strong_count(&self.perms)
925:     }
926: 
927:     /// Whether the store (base merged with any overlay) contains the canonical triple.
928:     pub fn contains(&self, t: [Id; 3]) -> bool {
929:         match &self.overlay {
930:             Some(ov) => {
931:                 !ov.deleted.contains(&t)
932:                     && (ov.added.binary_search(&t).is_ok() || self.base_contains(t))
933:             }
934:             None => self.base_contains(t),
935:         }
936:     }
937: 
938:     /// Whether the immutable BASE (ignoring the overlay) contains the canonical triple —
939:     /// one binary search of the SPO permutation (always built, in every index set).
940:     #[inline]
941:     fn base_contains(&self, t: [Id; 3]) -> bool {
942:         self.perms[Perm::Spo as usize].count_in(t, t) > 0
943:     }
944: 
945:     /// Applies an update batch as a DELTA-OVERLAY: `deletes` first, then `inserts`
946:     /// (SPARQL's DELETE/INSERT order), each O(log n + batch · overlay) — instead of the
947:     /// O(n) rebuild. Set semantics: re-inserting a present triple and deleting an absent
948:     /// one are no-ops; a delete of a pending insertion simply retracts it. When the
949:     /// overlay nets out to nothing it is dropped entirely, so an untouched (or fully
950:     /// reverted) store scans with zero overhead.
951:     pub fn apply_delta(&mut self, inserts: &[[Id; 3]], deletes: &[[Id; 3]]) {
952:         if inserts.is_empty() && deletes.is_empty() {
953:             return;
954:         }
955:         let mut ov = self.overlay.take().unwrap_or_default();
956:         // [GPT-6 Astra] Added projections retain the existing conservative reset.
957:         // Preserve deletion projections across inserts/no-ops: only actual tombstone
958:         // changes require another sort. No overlay read occurs before publication.
959:         ov.invalidate_added();
960:         #[cfg(feature = "overlay-deleted-projections")]
961:         let mut deleted_changed = false;
962:         for t in deletes {
963:             if let Ok(i) = ov.added.binary_search(t) {
964:                 ov.added.remove(i); // retract a pending insertion
965:             } else if self.base_contains(*t) {
966:                 #[cfg(feature = "overlay-deleted-projections")]
967:                 { deleted_changed |= ov.deleted.insert(*t); }
968:                 #[cfg(not(feature = "overlay-deleted-projections"))]
969:                 ov.deleted.insert(*t);
970:             }
971:         }
972:         for t in inserts {
973:             if ov.deleted.remove(t) {
974:                 #[cfg(feature = "overlay-deleted-projections")]
975:                 { deleted_changed = true; }
976:                 continue; // re-insert of a deleted base triple: just undelete
977:             }
978:             if self.base_contains(*t) {
979:                 continue; // already present in the base
980:             }
981:             if let Err(i) = ov.added.binary_search(t) {
982:                 ov.added.insert(i, *t);
983:             }
984:         }
985:         #[cfg(feature = "overlay-deleted-projections")]
986:         if deleted_changed {
987:             ov.invalidate_deleted();
988:         }
989:         self.overlay = if ov.is_empty() { None } else { Some(ov) };
990:     }
991: 
992:     /// Decodes every block-compressed permutation into its raw in-RAM form, so later
993:     /// scans are pure binary-search slice borrows (zero decode cost) — the LOAD-TIME
994:     /// DECOMPRESSION mode for an opened compressed directory: pay one full decode up
995:     /// front, query at exactly raw-store speed. Raw/mapped permutations are untouched.
996:     ///
997:     /// Runs on freshly built/opened stores (load-time), which are never yet forked;
998:     /// on a structurally SHARED store (post-[`fork`](Self::fork)) it is a no-op —
999:     /// decompression is an optimisation, never a correctness requirement.
1000:     pub fn decompress_to_ram(&mut self) {
1001:         let Some(perms) = std::sync::Arc::get_mut(&mut self.perms) else {
1002:             return; // shared with a fork: leave the (immutable) base untouched
1003:         };
1004:         for slot in perms {
1005:             if let PermData::Compressed(c) = slot {
1006:                 *slot = PermData::Owned(c.decode_all());
1007:             }
1008:         }
1009:     }
1010: 
1011:     /// A structural FORK of this store: the immutable base permutation indexes and
1012:     /// planner stats are SHARED (Arc bumps, O(1)); the pending delta-overlay is
1013:     /// carried by value (O(overlay), bounded by the compaction policy). The fork and
1014:     /// the original then evolve independently through [`apply_delta`](Self::apply_delta)
1015:     /// — neither ever mutates the shared base, so existing readers are unaffected.
1016:     pub fn fork(&self) -> TripleStore {
1017:         TripleStore {
1018:             perms: std::sync::Arc::clone(&self.perms),
1019:             pred_stats: std::sync::Arc::clone(&self.pred_stats),
1020:             overlay: self.overlay.clone(),
1021:         }
1022:     }
1023: 
1024:     /// Number of pending overlay entries (insertions + deletions) — the input to a
1025:     /// compaction threshold policy (a fork costs O(this); folding it costs O(n)).
1026:     pub fn overlay_len(&self) -> usize {
1027:         self.overlay.as_ref().map_or(0, |ov| ov.added.len() + ov.deleted.len())
1028:     }
1029: 
1030:     /// Heap footprint of the permutation indexes in bytes (for benchmarking). Memory-
1031:     /// mapped permutations contribute 0 — their resident pages are OS page cache.
1032:     pub fn heap_bytes(&self) -> usize {
1033:         self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
1034:             + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
1035:     }
1036: 
1037:     /// Chooses the permutation whose sort order places all bound pattern
1038:     /// positions as a contiguous prefix, so the matches form one range. Returns
1039:     /// the permutation and the number of leading bound columns.
1040:     fn choose(pattern: &Pattern) -> (Perm, usize) {
1041:         // Prefer an order where every bound position precedes every unbound one.
1042:         let bound = |i: usize| pattern[i].is_some();
1043:         for &perm in BUILT {
1044:             let order = perm.order();
1045:             // count leading bound columns
1046:             let mut lead = 0;
1047:             while lead < 3 && bound(order[lead]) {
1048:                 lead += 1;
1049:             }
1050:             // valid if all bound positions are within the leading prefix
1051:             let total_bound = (0..3).filter(|&i| bound(i)).count();
1052:             if lead == total_bound {
1053:                 return (perm, lead);
1054:             }
1055:         }
1056:         (Perm::Spo, 0)
1057:     }
1058: 
1059:     /// Like [`choose`], but among the permutations whose sort order places every
1060:     /// bound position as a prefix, prefers one whose first *unbound* column is
1061:     /// `sort_col` (a position 0..3 into a canonical triple). This makes the scan
1062:     /// output sorted by that column, enabling a merge join on it.
1063:     fn choose_sorted(pattern: &Pattern, sort_col: usize) -> (Perm, usize) {
1064:         let bound = |i: usize| pattern[i].is_some();
1065:         let total_bound = (0..3).filter(|&i| bound(i)).count();
1066:         // Prefer: bound positions form the leading prefix AND column `sort_col`
1067:         // is the first column after the prefix.
1068:         for &perm in BUILT {
1069:             let order = perm.order();
1070:             let mut lead = 0;
1071:             while lead < 3 && bound(order[lead]) {
1072:                 lead += 1;
1073:             }
1074:             if lead == total_bound && lead < 3 && order[lead] == sort_col {
1075:                 return (perm, lead);
1076:             }
1077:         }
1078:         Self::choose(pattern)
1079:     }
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
1095:     /// [OPUS-4.8] (sq-7d3dj.30.4) Scans a SPECIFIC permutation `perm`, for callers that
1096:     /// need a particular SECONDARY column order rather than just a primary sort column
1097:     /// (e.g. the DISTINCT loose skip-scan wants the layout `[..bound.., P, J, ..]` so each
1098:     /// `P`-block is `J`-sorted). Returns `None` when `perm` is not built (e.g. the compact
1099:     /// index) or when the pattern's bound positions do not form a leading prefix of `perm`
1100:     /// (so a contiguous range scan is impossible). The returned rows are identical to what
1101:     /// `scan`/`scan_sorted` would yield had they chosen `perm` — only the choice differs.
1102:     pub fn scan_perm(&self, pattern: &Pattern, perm: Perm) -> Option<Scan<'_>> {
1103:         if !BUILT.contains(&perm) {
1104:             return None;
1105:         }
1106:         let order = perm.order();
1107:         let bound = |i: usize| pattern[i].is_some();
1108:         let total_bound = (0..3).filter(|&i| bound(i)).count();
1109:         let mut lead = 0;
1110:         while lead < 3 && bound(order[lead]) {
1111:             lead += 1;
1112:         }
1113:         // Every bound position must be within the leading prefix, else this permutation
1114:         // cannot answer the pattern with one contiguous range.
1115:         if lead != total_bound {
1116:             return None;
1117:         }
1118:         Some(self.scan_with(pattern, perm, lead))
1119:     }
1120: 
1121:     /// The inclusive [lo, hi] key bounds for a pattern's bound prefix in `perm` order.
1122:     #[inline]
1123:     fn bounds(pattern: &Pattern, perm: Perm, lead: usize) -> ([Id; 3], [Id; 3]) {
1124:         let order = perm.order();
1125:         let mut lo = [Id::MIN; 3];
1126:         let mut hi = [Id::MAX; 3];
1127:         for k in 0..lead {
1128:             let v = pattern[order[k]].unwrap();
1129:             lo[k] = v;
1130:             hi[k] = v;
1131:         }
1132:         (lo, hi)
1133:     }
1134: 
1135:     fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
1136:         let (lo, hi) = Self::bounds(pattern, perm, lead);
1137:         let base = self.perms[perm as usize].rows_in(lo, hi);
1138:         // The single overlay branch on the scan hot path: with no pending updates the
1139:         // base range is returned untouched (borrowed, zero copies); with an overlay the
1140:         // deleted triples are filtered out and the inserted ones merge-interleaved, so
1141:         // the rows keep the permutation's sort order (merge joins stay valid).
1142:         //
1143:         // ZERO-COPY FAST PATH (sq-7d3dj.3) [OPUS-4.8]: even WITH an overlay, most ranges a small
1144:         // overlay does not touch. `count_correction` tells us exactly how many
1145:         // `added`/`deleted` triples fall in this range; when it is `(0, 0)` the
1146:         // overlay contributes nothing here — no `added` row projects into `[lo, hi]` (so
1147:         // nothing is interleaved) and no in-range base row is deleted (so nothing is
1148:         // dropped) — hence `merge` would reproduce `base` verbatim, rows AND sort order.
1149:         // We therefore return the BORROWED base slice directly, restoring allocation-free
1150:         // scans for every untouched range (the read-mostly mutated-server common case)
1151:         // instead of paying the owned merge path — which copies the whole base range into a
1152:         // fresh `Vec` and merge-interleaves the (separately, already perm-sorted) in-range
1153:         // `added` rows. It never re-sorts the range; the cost is the copy plus the interleave.
1154:         let rows = match &self.overlay {
1155:             None => base,
1156:             Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
1157:             Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
1158:         };
1159:         Scan { rows, perm }
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
1181: /// owned when decoded from a compressed permutation — uniformly a `&[[Id;3]]` to callers.
1182: pub struct Scan<'a> {
1183:     pub rows: std::borrow::Cow<'a, [[Id; 3]]>,
1184:     pub perm: Perm,
1185: }
1186: 
1187: impl<'a> Scan<'a> {
1188:     /// Maps a stored row back to a canonical s,p,o triple.
1189:     #[inline]
1190:     pub fn to_spo(&self, row: &[Id; 3]) -> [Id; 3] {
1191:         let order = self.perm.order();
1192:         let mut out = [0; 3];
1193:         out[order[0]] = row[0];
1194:         out[order[1]] = row[1];
1195:         out[order[2]] = row[2];
1196:         out
1197:     }
1198: }
````

## Unchanged complete focused test suite

````rust
//! [GPT-6 Astra] Deletion projection behavior, ownership, and invalidation (#4246).

use super::{Id, Overlay, Perm, TripleStore, BUILT};
use std::borrow::Cow;

fn triples() -> Vec<[Id; 3]> {
    (1..=12)
        .flat_map(|s| (1..=4).map(move |p| [s, p, s + p]))
        .collect()
}

fn sweep(store: &TripleStore, reference: &[[Id; 3]]) {
    let rebuilt = TripleStore::from_triples(reference.to_vec());
    // Every built permutation, every leading-prefix length, present/absent keys.
    for &perm in BUILT {
        for triple in reference.iter().copied().chain([[0; 3], [Id::MAX; 3]]) {
            for lead in 0..=3 {
                let mut pattern = [None; 3];
                for &col in &perm.order()[..lead] {
                    pattern[col] = Some(triple[col]);
                }
                let actual = store.scan_perm(&pattern, perm).unwrap();
                let expected = rebuilt.scan_perm(&pattern, perm).unwrap();
                assert_eq!(actual.rows, expected.rows, "{perm:?} {pattern:?}");
                assert_eq!(store.estimate(&pattern), rebuilt.estimate(&pattern));
            }
        }
    }
    for perm in Perm::ALL {
        if !BUILT.contains(&perm) {
            assert!(store.scan_perm(&[None; 3], perm).is_none());
            #[cfg(feature = "overlay-deleted-projections")]
            if let Some(ov) = &store.overlay {
                assert!(ov.deleted_by_perm[perm as usize].get().is_none());
            }
        }
    }
    assert_eq!(store.len(), reference.len());
    for &t in reference {
        assert!(store.contains(t));
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_is_lazy_reused_and_accounted() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let ov = store.overlay.as_ref().unwrap();
    assert!(ov.deleted_by_perm.iter().all(|s| s.get().is_none()));
    let cold_heap = ov.heap_bytes();
    let mut expected_heap = cold_heap;
    for &perm in BUILT {
        // An untouched exact range still needs its correction before borrowing base.
        let pattern = [Some(12), Some(4), Some(16)];
        let scan = store.scan_perm(&pattern, perm).unwrap();
        assert_eq!(scan.rows.len(), 1);
        assert!(matches!(scan.rows, Cow::Borrowed(_)));
        let rows = ov.deleted_by_perm[perm as usize]
            .get()
            .expect("scan uses cached deletion count");
        assert_eq!(rows.len(), 3);
        assert!(rows.windows(2).all(|w| w[0] < w[1]));
        let pointer = rows.as_ptr();
        // The requested slot is stable across subsequent counts and scans.
        for _ in 0..5 {
            assert_eq!(ov.count_correction(perm, [0; 3], [Id::MAX; 3]), (0, 3));
            assert_eq!(
                ov.deleted_by_perm[perm as usize].get().unwrap().as_ptr(),
                pointer
            );
        }
        expected_heap += rows.capacity() * std::mem::size_of::<[Id; 3]>();
        assert_eq!(ov.heap_bytes(), expected_heap);
        for other in Perm::ALL {
            if other as usize > perm as usize {
                assert!(ov.deleted_by_perm[other as usize].get().is_none());
            }
        }
    }
    let empty = Overlay::default();
    assert_eq!(empty.deleted_count(Perm::Spo, [0; 3], [Id::MAX; 3]), 0);
    assert!(empty.deleted_by_perm.iter().all(|s| s.get().is_none()));
}

#[test]
fn deleted_cache_matches_rebuild_after_mixed_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    sweep(&store, &reference);
    // Growing deletions, undeleting base, retracting additions, duplicate/no-op
    // updates and delete-then-insert of the same row all traverse warmed caches.
    let batches = [
        (vec![[20, 2, 22]], vec![[1, 1, 2], [2, 2, 4]]),
        (vec![[1, 1, 2]], vec![[3, 3, 6], [20, 2, 22]]),
        (vec![[4, 4, 8], [4, 4, 8]], vec![[4, 4, 8], [99; 3]]),
        (vec![[2, 2, 4], [3, 3, 6]], vec![]),
    ];
    for (inserts, deletes) in batches {
        store.apply_delta(&inserts, &deletes);
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        // Correct rows/counts, not merely empty-slot observations, pin invalidation.
        sweep(&store, &reference);
    }
    assert!(!store.has_overlay());
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_survives_insert_only_and_noop_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    sweep(&store, &reference);
    let pointers: Vec<_> = BUILT
        .iter()
        .map(|&p| {
            store.overlay.as_ref().unwrap().deleted_by_perm[p as usize]
                .get()
                .unwrap()
                .as_ptr()
        })
        .collect();
    for (inserts, deletes) in [
        (vec![[20, 2, 22]], vec![]), // actual added-set change with tombstones present
        (vec![[20, 2, 22], [3, 3, 6]], vec![[1, 1, 2], [99; 3]]), // all no-ops
        (vec![], vec![[20, 2, 22]]), // retract an addition, not a base deletion
        (vec![], vec![]),
    ] {
        store.apply_delta(&inserts, &deletes);
        for (&perm, &pointer) in BUILT.iter().zip(&pointers) {
            assert_eq!(
                store.overlay.as_ref().unwrap().deleted_by_perm[perm as usize]
                    .get()
                    .expect("unchanged tombstones retain their projection")
                    .as_ptr(),
                pointer,
            );
        }
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference); // also pins the independent added-cache invalidation
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2]]);
    reference.retain(|t| *t != [1, 1, 2]);
    for (inserts, deletes) in [
        (vec![], vec![[2, 2, 4]]),          // grow the deleted set
        (vec![[1, 1, 2]], vec![]),          // undelete an existing tombstone
        (vec![[3, 3, 6]], vec![[3, 3, 6]]), // changes during batch, even if net unchanged
    ] {
        sweep(&store, &reference);
        store.apply_delta(&inserts, &deletes);
        assert!(store
            .overlay
            .as_ref()
            .unwrap()
            .deleted_by_perm
            .iter()
            .all(|s| s.get().is_none()));
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference);
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_inclusive_bounds_and_empty_ranges() {
    let mut ov = Overlay::default();
    ov.deleted.extend([[0, 1, 2], [2, 3, 4], [Id::MAX; 3]]);
    for perm in Perm::ALL {
        let order = perm.order();
        for &t in &ov.deleted {
            let row = [t[order[0]], t[order[1]], t[order[2]]];
            assert_eq!(ov.deleted_count(perm, row, row), 1);
        }
        assert_eq!(ov.deleted_count(perm, [0; 3], [Id::MAX; 3]), 3);
        assert_eq!(ov.deleted_count(perm, [42; 3], [42; 3]), 0);
    }
}

#[test]
fn deleted_cache_compressed_base_matches_rebuild() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples_compressed(reference.clone());
    store.apply_delta(&[[20, 2, 22]], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    reference.push([20, 2, 22]);
    sweep(&store, &reference);
    store.apply_delta(&[[1, 1, 2]], &[[3, 3, 6]]);
    reference.push([1, 1, 2]);
    reference.retain(|t| *t != [3, 3, 6]);
    sweep(&store, &reference);
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_fork_and_clone_are_independent() {
    let mut original = TripleStore::from_triples(triples());
    original.apply_delta(&[], &[[1, 1, 2]]);
    let cold_fork = original.fork();
    for &perm in BUILT {
        original.scan_perm(&[None; 3], perm).unwrap();
    }
    assert!(cold_fork
        .overlay
        .as_ref()
        .unwrap()
        .deleted_by_perm
        .iter()
        .all(|s| s.get().is_none()));
    let warm_fork = original.fork();
    let cloned_overlay = original.overlay.clone().unwrap();
    for &perm in BUILT {
        let slot = perm as usize;
        let original_rows = original.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let fork_rows = warm_fork.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let clone_rows = cloned_overlay.deleted_by_perm[slot].get().unwrap();
        assert_eq!(original_rows, fork_rows);
        assert_ne!(original_rows.as_ptr(), fork_rows.as_ptr());
        assert_ne!(original_rows.as_ptr(), clone_rows.as_ptr());
    }
    original.apply_delta(&[[1, 1, 2]], &[[2, 2, 4]]);
    for &perm in BUILT {
        for frozen in [&cold_fork, &warm_fork] {
            assert_eq!(
                frozen
                    .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                    .unwrap()
                    .rows
                    .len(),
                0
            );
            assert_eq!(
                frozen
                    .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                    .unwrap()
                    .rows
                    .len(),
                1
            );
        }
        assert_eq!(
            original
                .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(
            original
                .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                .unwrap()
                .rows
                .len(),
            0
        );
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_concurrent_first_reads_share_initialized_projection() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    BUILT
                        .iter()
                        .map(|&perm| {
                            assert_eq!(store.scan_perm(&[None; 3], perm).unwrap().rows.len(), 46);
                            let rows = store.overlay.as_ref().unwrap().deleted_by_perm
                                [perm as usize]
                                .get()
                                .unwrap();
                            (rows.as_ptr() as usize, rows.clone())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut results = handles.into_iter().map(|h| h.join().unwrap());
        assert_eq!(results.next().unwrap(), results.next().unwrap());
    });
}

#[test]
fn deleted_cache_graph_snapshot_retains_warm_generation() {
    use crate::Graph;
    use oxrdf::{NamedNode, Term};
    let term = |s| Term::from(NamedNode::new(s).unwrap());
    let a = [term("urn:a"), term("urn:p"), term("urn:o")];
    let b = [term("urn:b"), term("urn:p"), term("urn:o")];
    let mut graph = Graph::load_str(
        "<urn:a> <urn:p> <urn:o> . <urn:b> <urn:p> <urn:o> .",
        "turtle",
    )
    .unwrap();
    graph.apply_delta(&[], std::slice::from_ref(&a)).unwrap();
    for &perm in BUILT {
        assert_eq!(
            graph.store.scan_perm(&[None; 3], perm).unwrap().rows.len(),
            1
        );
    }
    let snapshot = graph.snapshot();
    graph
        .apply_delta(std::slice::from_ref(&a), std::slice::from_ref(&b))
        .unwrap();
    for (terms, snapshot_len, current_len) in [(a, 0, 1), (b, 1, 0)] {
        let pattern = terms.map(|t| Some(graph.dict.lookup(&t)));
        assert_eq!(snapshot.store.estimate(&pattern), snapshot_len);
        assert_eq!(graph.store.estimate(&pattern), current_len);
        for &perm in BUILT {
            assert_eq!(
                snapshot.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                snapshot_len
            );
            assert_eq!(
                graph.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                current_len
            );
        }
    }
}

// [GPT-6 Astra] Default-off must preserve main's representation and linear-count
// behavior, not merely return the same rows after allocating a hidden projection.
#[cfg(not(feature = "overlay-deleted-projections"))]
#[test]
fn deleted_projection_feature_off_preserves_main_layout_and_heap() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let before = store.heap_bytes();
    for &perm in BUILT {
        for _ in 0..3 {
            let scan = store
                .scan_perm(&[Some(12), Some(4), Some(16)], perm)
                .unwrap();
            assert_eq!(scan.rows.len(), 1);
            assert!(matches!(scan.rows, Cow::Borrowed(_)));
            assert_eq!(store.estimate(&[Some(1), Some(1), Some(2)]), 0);
            assert_eq!(
                store.heap_bytes(),
                before,
                "default reads retain no deletion projection"
            );
        }
    }
    let frozen = store.fork();
    assert_eq!(frozen.heap_bytes(), before);
    // These are the three fields of main's Overlay. All have pointer alignment
    // and sizes divisible by that alignment; there is no inter-field padding.
    let main_fields = std::mem::size_of::<Vec<[Id; 3]>>()
        + std::mem::size_of::<rustc_hash::FxHashSet<[Id; 3]>>()
        + std::mem::size_of::<[std::sync::OnceLock<Vec<[Id; 3]>>; 6]>();
    assert_eq!(std::mem::size_of::<Overlay>(), main_fields);
    assert_eq!(
        std::mem::align_of::<Overlay>(),
        std::mem::align_of::<usize>()
    );
}
````

## Static text-retention check (not a substitute for compiler proof)

````json
{
  "head": "6334b338587fe5c635c69a09e134917ec35eaaca",
  "base": "a42a9e89dec485f6a319c47cb3635c59cb5a2270",
  "default_function_exact_base_text": true,
  "default_tombstone_insert_exact_base_line": true,
  "removed_nonblank_store_lines": [
    {
      "line": 191,
      "text": "    /// `Sync`; a race just recomputes the same value and discards the loser."
    },
    {
      "line": 205,
      "text": "    /// Drops every cached projection \u2014 called whenever `added` is about to change, so a"
    },
    {
      "line": 206,
      "text": "    /// cache can never outlive the `added` it was derived from."
    },
    {
      "line": 257,
      "text": "    /// projection (O(log k), two binary searches); the `deleted` side is an unordered"
    },
    {
      "line": 258,
      "text": "    /// hash set and stays O(|deleted|)."
    },
    {
      "line": 279,
      "text": "        // aliases `added` and so never occupies a slot."
    },
    {
      "line": 903,
      "text": "        // `added` is about to change, so every cached perm-sorted projection of it is"
    },
    {
      "line": 904,
      "text": "        // stale from here on. Dropping them up front (O(1) per permutation) keeps the"
    },
    {
      "line": 905,
      "text": "        // write path O(batch) \u2014 the projections are rebuilt lazily by the next scan"
    },
    {
      "line": 906,
      "text": "        // that needs them, and only for the permutations it actually scans."
    }
  ],
  "removed_compiled_store_lines": 0,
  "actual_added_line_blanking_retains_original_default_function": true,
  "limits": "Static prerequisites only; neither required compiler byte comparison inferred from these assertions."
}
````

## Native commands and calibrated results

````json
[
  {
    "name": "semantic-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--lib",
      "store::overlay_deleted_tests::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 9.349166208005045,
    "summaries": [
      "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.04s"
    ]
  },
  {
    "name": "semantic-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--features",
      "overlay-deleted-projections",
      "--lib",
      "store::overlay_deleted_tests::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 5.609907208010554,
    "summaries": [
      "test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s"
    ]
  },
  {
    "name": "clippy-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "clippy",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--all-targets",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "seconds": 3.41067587499856
  },
  {
    "name": "clippy-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "clippy",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--features",
      "overlay-deleted-projections",
      "--all-targets",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "seconds": 3.061225541998283
  }
]
````

## tests/clippy-off.log

````text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `dev` profile [unoptimized] target(s) in 3.17s
````

## tests/clippy-on.log

````text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `dev` profile [unoptimized] target(s) in 2.89s
````

## tests/semantic-off.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 8.54s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 4 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.04s
````

## tests/semantic-on.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 4.97s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s
````

## Actual six-control driver (source restored after each run series)

````python
from pathlib import Path
import subprocess, os, json, difflib, re, time
root=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
out=Path(__file__).parent
source=root/'crates/sparq-core/src/store.rs'
original=source.read_text()
start=original.index('    fn deleted_count(')
end=original.index('\n    /// The `added` triples',start)
linear="""    fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
        let order = perm.order();
        self.deleted.iter().filter(|t| {
            let row = [t[order[0]], t[order[1]], t[order[2]]];
            row >= lo && row <= hi
        }).count()
    }
"""
cases={
 'unconditional_deleted_invalidation':original.replace('if deleted_changed {','if deleted_changed || !ov.deleted.is_empty() {'),
 'remove_deleted_invalidation':original.replace('            ov.invalidate_deleted();','            let _ = deleted_changed;'),
 'ignore_tombstone_insert_flag':original.replace('deleted_changed |= ov.deleted.insert(*t);','ov.deleted.insert(*t);'),
 'ignore_tombstone_remove_flag':original.replace('{ deleted_changed = true; }','{}'),
 'remove_deleted_cache_use':original[:start]+linear+original[end:]
}
env=os.environ.copy();env.update(RAYON_NUM_THREADS='2',CARGO_BUILD_JOBS='2',CARGO_TARGET_DIR='/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
cmd=['/Users/jesght/.cargo/bin/cargo','test','--locked','--offline','-p','sparq-core','--features','overlay-deleted-projections','--lib','store::overlay_deleted_tests::','--','--test-threads=1']
cases['force_cache_in_feature_off'] = original.replace('#[cfg(feature = \"overlay-deleted-projections\")]','').replace('#[cfg(not(feature = \"overlay-deleted-projections\"))]','#[cfg(any())]')
results=[]
try:
 for name,text in cases.items():
  assert text!=original
  source.write_text(text)
  (out/(name+'.diff')).write_text(''.join(difflib.unified_diff(original.splitlines(True),text.splitlines(True),fromfile='store.rs',tofile=name)))
  current_cmd=cmd if name!='force_cache_in_feature_off' else [cmd[0],'test','--locked','--offline','-p','sparq-core','--lib','deleted_projection_feature_off_preserves_main_layout_and_heap','--','--test-threads=1']
  begin=time.monotonic()
  with (out/(name+'.log')).open('w') as log: r=subprocess.run(current_cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=240)
  body=(out/(name+'.log')).read_text()
  summaries=re.findall(r'test result: .*',body)
  assert r.returncode==101 and 'test result: FAILED.' in body, (name,r.returncode)
  results.append(dict(name=name,exit=r.returncode,seconds=time.monotonic()-begin,compiled=True,command=current_cmd,summaries=summaries))
  (out/'results.json').write_text(json.dumps(dict(command=cmd,results=results),indent=2)+'\n')
  print(name,summaries,flush=True)
finally:
 source.write_text(original)
````

## controls/force_cache_in_feature_off.diff

````diff
--- store.rs
+++ force_cache_in_feature_off
@@ -179,7 +179,7 @@
     /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
     /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
     /// these projections when a tombstone is inserted or removed.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
 }
 
@@ -218,7 +218,7 @@
     }
 
     /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     fn invalidate_deleted(&mut self) {
         for slot in &mut self.deleted_by_perm {
             slot.take();
@@ -231,7 +231,7 @@
     /// apply_delta invalidates only the mutated overlay under exclusive access.
     /// Concurrent first readers of the same permutation wait for its one sorting
     /// initializer; the cold sort is serialized for that permutation.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
         if self.deleted.is_empty() {
             return 0;
@@ -295,7 +295,7 @@
     /// correction to a base range count. The `added` side rides the cached perm-sorted
     /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
     /// default; the experimental feature opts into lazy sorted projections.
-    #[cfg(not(feature = "overlay-deleted-projections"))]
+    #[cfg(any())]
     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
         let order = perm.order();
         let add = self.added_rows(perm, lo, hi).len();
@@ -311,7 +311,7 @@
     }
 
     /// [GPT-6 Astra] Experimental range correction using cached deletion projections.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
         let add = self.added_rows(perm, lo, hi).len();
         let del = self.deleted_count(perm, lo, hi);
@@ -331,7 +331,7 @@
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
             .sum();
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         let cached = cached + self.deleted_by_perm.iter()
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
@@ -957,21 +957,21 @@
         // Preserve deletion projections across inserts/no-ops: only actual tombstone
         // changes require another sort. No overlay read occurs before publication.
         ov.invalidate_added();
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         let mut deleted_changed = false;
         for t in deletes {
             if let Ok(i) = ov.added.binary_search(t) {
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
-                #[cfg(feature = "overlay-deleted-projections")]
+                
                 { deleted_changed |= ov.deleted.insert(*t); }
-                #[cfg(not(feature = "overlay-deleted-projections"))]
+                #[cfg(any())]
                 ov.deleted.insert(*t);
             }
         }
         for t in inserts {
             if ov.deleted.remove(t) {
-                #[cfg(feature = "overlay-deleted-projections")]
+                
                 { deleted_changed = true; }
                 continue; // re-insert of a deleted base triple: just undelete
             }
@@ -982,7 +982,7 @@
                 ov.added.insert(i, *t);
             }
         }
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         if deleted_changed {
             ov.invalidate_deleted();
         }
````

## controls/force_cache_in_feature_off.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.49s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 1 test
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap stdout ----

thread 'store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap' (2551685) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:367:13:
assertion `left == right` failed: default reads retain no deletion projection
  left: 3543
 right: 3495
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## controls/ignore_tombstone_insert_flag.diff

````diff
--- store.rs
+++ ignore_tombstone_insert_flag
@@ -964,7 +964,7 @@
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
                 #[cfg(feature = "overlay-deleted-projections")]
-                { deleted_changed |= ov.deleted.insert(*t); }
+                { ov.deleted.insert(*t); }
                 #[cfg(not(feature = "overlay-deleted-projections"))]
                 ov.deleted.insert(*t);
             }
````

## controls/ignore_tombstone_insert_flag.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 2.47s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2550365) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.08s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## controls/ignore_tombstone_remove_flag.diff

````diff
--- store.rs
+++ ignore_tombstone_remove_flag
@@ -972,7 +972,7 @@
         for t in inserts {
             if ov.deleted.remove(t) {
                 #[cfg(feature = "overlay-deleted-projections")]
-                { deleted_changed = true; }
+                {}
                 continue; // re-insert of a deleted base triple: just undelete
             }
             if self.base_contains(*t) {
````

## controls/ignore_tombstone_remove_flag.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 2.37s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2550859) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.10s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## controls/remove_deleted_cache_use.diff

````diff
--- store.rs
+++ remove_deleted_cache_use
@@ -233,20 +233,11 @@
     /// initializer; the cold sort is serialized for that permutation.
     #[cfg(feature = "overlay-deleted-projections")]
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
-        if self.deleted.is_empty() {
-            return 0;
-        }
-        let rows = self.deleted_by_perm[perm as usize].get_or_init(|| {
-            let order = perm.order();
-            let mut rows: Vec<[Id; 3]> = self
-                .deleted
-                .iter()
-                .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
-                .collect();
-            rows.sort_unstable();
-            rows
-        });
-        rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
+        let order = perm.order();
+        self.deleted.iter().filter(|t| {
+            let row = [t[order[0]], t[order[1]], t[order[2]]];
+            row >= lo && row <= hi
+        }).count()
     }
 
     /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
````

## controls/remove_deleted_cache_use.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.62s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... FAILED
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... FAILED
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... FAILED
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection stdout ----

thread '<unnamed>' (2551271) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:299:34:
called `Option::unwrap()` on a `None` value
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread '<unnamed>' (2551270) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:299:34:
called `Option::unwrap()` on a `None` value

thread 'store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection' (2551269) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:306:64:
called `Result::unwrap()` on an `Err` value: Any { .. }

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2551272) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:233:14:
called `Option::unwrap()` on a `None` value

---- store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted' (2551275) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:61:14:
scan uses cached deletion count

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2551277) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:124:18:
called `Option::unwrap()` on a `None` value


failures:
    store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## controls/remove_deleted_invalidation.diff

````diff
--- store.rs
+++ remove_deleted_invalidation
@@ -984,7 +984,7 @@
         }
         #[cfg(feature = "overlay-deleted-projections")]
         if deleted_changed {
-            ov.invalidate_deleted();
+            let _ = deleted_changed;
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
     }
````

## controls/remove_deleted_invalidation.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
warning: method `invalidate_deleted` is never used
   --> crates/sparq-core/src/store.rs:222:8
    |
186 | impl Overlay {
    | ------------ method in this implementation
...
222 |     fn invalidate_deleted(&mut self) {
    |        ^^^^^^^^^^^^^^^^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `sparq-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized] target(s) in 2.64s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... FAILED
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... FAILED
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... FAILED
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... FAILED
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2549955) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild' (2549958) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2549964) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:270:9:
assertion `left == right` failed
  left: 1
 right: 0

---- store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation' (2549965) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:337:9:
assertion `left == right` failed
  left: 0
 right: 1

---- store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas' (2549968) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication
    store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation
    store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas

test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.07s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## controls/unconditional_deleted_invalidation.diff

````diff
--- store.rs
+++ unconditional_deleted_invalidation
@@ -983,7 +983,7 @@
             }
         }
         #[cfg(feature = "overlay-deleted-projections")]
-        if deleted_changed {
+        if deleted_changed || !ov.deleted.is_empty() {
             ov.invalidate_deleted();
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
````

## controls/unconditional_deleted_invalidation.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.79s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2549558) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:139:22:
unchanged tombstones retain their projection
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## proof-prerequisites.json

````json
{
  "tool_checks": [
    {
      "command": [
        "/Users/jesght/.cargo/bin/rustc",
        "--version"
      ],
      "exit": 0,
      "stdout": "rustc 1.97.1 (8bab26f4f 2026-07-14)",
      "stderr": ""
    },
    {
      "command": [
        "/Users/jesght/.cargo/bin/rustup",
        "which",
        "--toolchain",
        "1.97.1",
        "rustc"
      ],
      "exit": 0,
      "stdout": "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc",
      "stderr": ""
    },
    {
      "command": [
        "/Users/jesght/.cargo/bin/rustup",
        "target",
        "list",
        "--installed"
      ],
      "exit": 0,
      "stdout": "aarch64-apple-darwin\nwasm32-unknown-unknown",
      "stderr": ""
    }
  ],
  "initial_disk_free_bytes": 14048661504,
  "proof_needs": "Raw cargo, rustc/rust-lld, installed wasm32 std, cached packages and pinned Git dependency; no wasm-pack/Node/bindgen CLI. Existing readiness inventory found642/642 packages and exact Git revision cached.",
  "protocol_policy": "Unchanged feature_off_autodeclare.py main/decide/cargo_wasm_builder, report-only; one900s attempt, CARGO_NET_OFFLINE=true, PATH cargo shim adds only --locked for build/tree/metadata. Each target remains inside its original task-owned exported or neutral tree. No injected prebuilt bundle, shared target, or unrelated target artifact. The original protocol itself copies its just-built head/base tree, including target, into each separate neutral tree; captured logs show actual recompilation of affected crates."
}
````

## provenance.json

````json
{
  "implementation_model": "OpenAI GPT-6 Astra",
  "reasoning_effort": "xhigh (inherited active lane)",
  "head": "6334b338587fe5c635c69a09e134917ec35eaaca",
  "base": "a42a9e89dec485f6a319c47cb3635c59cb5a2270",
  "previous_reviewed_head": "b86b5d5ad84bce900762defa094630eb358317a6",
  "execution_platform": {
    "system": "Darwin",
    "machine": "arm64",
    "release": "25.6.0"
  },
  "rustc_verbose": "rustc 1.97.1 (8bab26f4f 2026-07-14)\nbinary: rustc\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\ncommit-date: 2026-07-14\nhost: aarch64-apple-darwin\nrelease: 1.97.1\nLLVM version: 22.1.6\n",
  "cargo_version": "cargo 1.97.1 (c980f4866 2026-06-30)\n",
  "protocol_source_sha256": "3dddd713b18754cf9d53b9f0ba8375e24e3d3a4eebf4d52a6816160b91c39456",
  "protocol_unchanged_from_base_and_b86": true,
  "cargo_shim_sha256": "8d692e63b5c897e47693c10c979efeba7aaec437dd4d684008cb7ee7ce12d915",
  "run_original_protocol_sha256": "53fa4a1e62ad8782066831e9908d401d397e3f9112b392a7adee2829e5b1decd",
  "original_protocol_semantics": "Original main -> decide -> cargo_wasm_builder, no injected builder/prebuilt bundles. Original copytree duplicates its own head/base target into distinct neutral directories; compiler logs show actual rebuilds including sparq-core. No unrelated/shared target reuse. Shim adds --locked only and records outputs/bytes. CARGO_NET_OFFLINE=true applies to all calls.",
  "generated_template_attribution": "The exact unchanged declaration generator hard-codes [OPUS-5] in its reason. That string is preserved as raw generated evidence, not claimed as execution/review provenance. This proof was executed by GPT-6 Astra; actual independent Opus follow-up is pending.",
  "declaration_location": "proof/declaration-proposal.json only; source tree has no6469 declaration.",
  "native_environment": {
    "CARGO_BUILD_JOBS": "2",
    "RAYON_NUM_THREADS": "2",
    "CARGO_TARGET_DIR": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target",
    "CARGO_PROFILE_DEV_DEBUG": "0",
    "CARGO_PROFILE_TEST_DEBUG": "0"
  },
  "proof_environment_and_limits": "proof/execution-result.json",
  "no_remote_calls_in_this_revision": true,
  "no_installs_or_unrelated_cleanup": true
}
````

## proof/execution-result.json

````json
{
  "head": "6334b338587fe5c635c69a09e134917ec35eaaca",
  "command": [
    "/opt/homebrew/bin/python3",
    "-u",
    "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/source-retention/proof/run_original_protocol.py"
  ],
  "started_at": "2026-09-09T14:12:34.561420+00:00",
  "limit_seconds": 900,
  "disk_stop_margin_bytes": 11274289152,
  "hard_disk_floor_bytes": 10737418240,
  "initial_free_bytes": 13729923072,
  "env_overrides": {
    "CARGO_NET_OFFLINE": "true",
    "CARGO_BUILD_JOBS": "2",
    "CARGO_INCREMENTAL": "0",
    "RUSTUP_TOOLCHAIN": "1.97.1",
    "RAYON_NUM_THREADS": "1",
    "PYTHONDONTWRITEBYTECODE": "1",
    "TMPDIR": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334",
    "FEATOFF_PROOF_EVIDENCE": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/source-retention/proof",
    "PATH": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/source-retention/proof/shim:/Users/jesght/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  },
  "removed_overrides": {},
  "executed": true,
  "exit": 0,
  "stop_reason": null,
  "elapsed_seconds": 544.8218691670045,
  "minimum_observed_free_bytes": 11595014144,
  "final_free_bytes": 11602903040
}
````

## Actual byte comparisons — two independent obligations

````json
{
  "addition-neutral_vs_head": {
    "left": "addition-neutral",
    "right": "head",
    "equal": true,
    "differing_bytes": 0,
    "left_sha256": "4aa13bda98ad594604c1392f7bf74a6f76656bcc2701ca14d99a3ee80a7851f5",
    "right_sha256": "4aa13bda98ad594604c1392f7bf74a6f76656bcc2701ca14d99a3ee80a7851f5",
    "left_bytes": 1560265,
    "right_bytes": 1560265
  },
  "deletion-neutral_vs_base": {
    "left": "deletion-neutral",
    "right": "base",
    "equal": true,
    "differing_bytes": 0,
    "left_sha256": "ffc6b9a50be3c44e19a12f93f2e0d8f2b0ad27cadba3cf900ad726c48889ac9e",
    "right_sha256": "ffc6b9a50be3c44e19a12f93f2e0d8f2b0ad27cadba3cf900ad726c48889ac9e",
    "left_bytes": 1560265,
    "right_bytes": 1560265
  },
  "base_vs_head": {
    "left": "base",
    "right": "head",
    "equal": false,
    "differing_bytes": 12,
    "left_sha256": "ffc6b9a50be3c44e19a12f93f2e0d8f2b0ad27cadba3cf900ad726c48889ac9e",
    "right_sha256": "4aa13bda98ad594604c1392f7bf74a6f76656bcc2701ca14d99a3ee80a7851f5",
    "left_bytes": 1560265,
    "right_bytes": 1560265
  }
}
````

## Complete locked-only cargo recording shim

````python
#!/opt/homebrew/bin/python3
from pathlib import Path
import sys,os,subprocess,time,json,hashlib
out=Path(os.environ['FEATOFF_PROOF_EVIDENCE'])
args=sys.argv[1:];command=['/Users/jesght/.cargo/bin/cargo',*args]
if args and args[0] in ['build','tree','metadata'] and '--locked' not in args:command.append('--locked')
log=out/'cargo-invocations.jsonl';started=time.time()
record={'cwd':os.getcwd(),'received_args':args,'actual_command':command,'started':started}
with log.open('a') as f:f.write(json.dumps({'event':'start',**record})+'\n')
if args and args[0]=='build':
 tree=Path.cwd();parent=tree.parent.name
 role='addition-neutral' if parent.startswith('featoff-neutral-') else 'deletion-neutral' if parent.startswith('featoff-delneutral-') else tree.name
 r=subprocess.run(command,capture_output=True)
 (out/(role+'-cargo.stdout')).write_bytes(r.stdout);(out/(role+'-cargo.stderr')).write_bytes(r.stderr)
 sys.stdout.buffer.write(r.stdout);sys.stderr.buffer.write(r.stderr)
 target=Path(args[args.index('--target-dir')+1]) if '--target-dir' in args else tree/'target'
 wasm=target/'wasm32-unknown-unknown/release-wasm/sparq_wasm.wasm'
 record.update(role=role,target=str(target),exit=r.returncode,ended=time.time())
 if r.returncode==0 and wasm.is_file():
  b=wasm.read_bytes();(out/'binaries'/(role+'.wasm')).write_bytes(b);record.update(wasm_bytes=len(b),wasm_sha256=hashlib.sha256(b).hexdigest())
else:
 r=subprocess.run(command);record.update(exit=r.returncode,ended=time.time())
with log.open('a') as f:f.write(json.dumps({'event':'finish',**record})+'\n')
sys.exit(r.returncode)
````

## Exact original-protocol runner

````python
from pathlib import Path
import importlib.util,sys,os,json,hashlib
w=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
source=w/'scripts/feature_off_autodeclare.py'
spec=importlib.util.spec_from_file_location('original_feature_off_autodeclare',source);module=importlib.util.module_from_spec(spec);sys.modules[spec.name]=module;spec.loader.exec_module(module)
# Execute the original CLI/protocol. The only wrapper is an external cargo argv
# shim adding --locked and recording actual compiler output. No builder/result
# function is replaced, injected, mocked or short-circuited.
raise SystemExit(module.main(['--repo',str(w),'--base-sha','a42a9e89dec485f6a319c47cb3635c59cb5a2270','--head-sha','6334b338587fe5c635c69a09e134917ec35eaaca','--pr','6469','--report-only','--summary-file',str(Path(os.environ['FEATOFF_PROOF_EVIDENCE'])/'report-only-summary.md')]))
````

## Every received and actually forwarded cargo argv

````jsonl
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["tree", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--edges", "normal", "--prefix", "none"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "tree", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--edges", "normal", "--prefix", "none", "--locked"], "started": 1788963170.7035332}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["tree", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--edges", "normal", "--prefix", "none"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "tree", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--edges", "normal", "--prefix", "none", "--locked"], "started": 1788963170.7035332, "exit": 0, "ended": 1788963174.422581}
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["metadata", "--format-version", "1"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "metadata", "--format-version", "1", "--locked"], "started": 1788963174.614136}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["metadata", "--format-version", "1"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "metadata", "--format-version", "1", "--locked"], "started": 1788963174.614136, "exit": 0, "ended": 1788963199.4709408}
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/target", "--locked"], "started": 1788963199.952849}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/target", "--locked"], "started": 1788963199.952849, "role": "base", "target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/target", "exit": 0, "ended": 1788963284.33073, "wasm_bytes": 1560265, "wasm_sha256": "ffc6b9a50be3c44e19a12f93f2e0d8f2b0ad27cadba3cf900ad726c48889ac9e"}
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/target", "--locked"], "started": 1788963284.4850628}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/target", "--locked"], "started": 1788963284.4850628, "role": "head", "target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/target", "exit": 0, "ended": 1788963336.3628612, "wasm_bytes": 1560265, "wasm_sha256": "4aa13bda98ad594604c1392f7bf74a6f76656bcc2701ca14d99a3ee80a7851f5"}
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/target", "--locked"], "started": 1788963390.258}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/target", "--locked"], "started": 1788963390.258, "role": "addition-neutral", "target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/target", "exit": 0, "ended": 1788963438.12684, "wasm_bytes": 1560265, "wasm_sha256": "4aa13bda98ad594604c1392f7bf74a6f76656bcc2701ca14d99a3ee80a7851f5"}
{"event": "start", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/target", "--locked"], "started": 1788963646.122171}
{"event": "finish", "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree", "received_args": ["build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/target"], "actual_command": ["/Users/jesght/.cargo/bin/cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm", "--target", "wasm32-unknown-unknown", "--target-dir", "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/target", "--locked"], "started": 1788963646.122171, "role": "deletion-neutral", "target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/target", "exit": 0, "ended": 1788963698.852685, "wasm_bytes": 1560265, "wasm_sha256": "ffc6b9a50be3c44e19a12f93f2e0d8f2b0ad27cadba3cf900ad726c48889ac9e"}
````

## Actual base compiler stderr

````text
   Compiling proc-macro2 v1.0.106
   Compiling unicode-ident v1.0.24
   Compiling quote v1.0.45
   Compiling wasm-bindgen-shared v0.2.122
   Compiling rustversion v1.0.22
   Compiling syn v2.0.117
   Compiling bumpalo v3.20.3
   Compiling wasm-bindgen-macro-support v0.2.122
   Compiling wasm-bindgen v0.2.122
   Compiling wasm-bindgen-macro v0.2.122
   Compiling cfg-if v1.0.4
   Compiling getrandom v0.3.4
   Compiling once_cell v1.21.4
   Compiling zerocopy v0.8.50
   Compiling rand_core v0.9.5
   Compiling thiserror v2.0.18
   Compiling ppv-lite86 v0.2.21
   Compiling thiserror-impl v2.0.18
   Compiling rand_chacha v0.9.0
   Compiling rand v0.9.4
   Compiling oxilangtag v0.1.6
   Compiling oxiri v0.2.11
   Compiling oxrdf v0.3.3
   Compiling peg-runtime v0.8.6
   Compiling memchr v2.8.2
   Compiling foldhash v0.2.0
   Compiling allocator-api2 v0.2.21
   Compiling equivalent v1.0.2
   Compiling hashbrown v0.17.1
   Compiling oxttl v0.2.3
   Compiling peg-macros v0.8.6
   Compiling rustc-hash v2.1.3
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/crates/sparq-core)
   Compiling peg v0.8.6
   Compiling smallvec v1.15.2
   Compiling sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/crates/sparq-substrate)
   Compiling spargebra v0.4.6 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/vendor/spargebra)
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/crates/sparq-engine)
   Compiling sparq-wasm v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/base/crates/sparq-wasm)
    Finished `release-wasm` profile [optimized] target(s) in 1m 24s
````

## Actual head compiler stderr

````text
   Compiling proc-macro2 v1.0.106
   Compiling unicode-ident v1.0.24
   Compiling quote v1.0.45
   Compiling wasm-bindgen-shared v0.2.122
   Compiling rustversion v1.0.22
   Compiling syn v2.0.117
   Compiling bumpalo v3.20.3
   Compiling wasm-bindgen v0.2.122
   Compiling wasm-bindgen-macro-support v0.2.122
   Compiling getrandom v0.3.4
   Compiling zerocopy v0.8.50
   Compiling wasm-bindgen-macro v0.2.122
   Compiling once_cell v1.21.4
   Compiling cfg-if v1.0.4
   Compiling rand_core v0.9.5
   Compiling thiserror v2.0.18
   Compiling thiserror-impl v2.0.18
   Compiling ppv-lite86 v0.2.21
   Compiling rand_chacha v0.9.0
   Compiling rand v0.9.4
   Compiling oxilangtag v0.1.6
   Compiling oxiri v0.2.11
   Compiling oxrdf v0.3.3
   Compiling peg-runtime v0.8.6
   Compiling equivalent v1.0.2
   Compiling memchr v2.8.2
   Compiling allocator-api2 v0.2.21
   Compiling foldhash v0.2.0
   Compiling hashbrown v0.17.1
   Compiling oxttl v0.2.3
   Compiling peg-macros v0.8.6
   Compiling rustc-hash v2.1.3
   Compiling peg v0.8.6
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/crates/sparq-core)
   Compiling smallvec v1.15.2
   Compiling spargebra v0.4.6 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/vendor/spargebra)
   Compiling sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/crates/sparq-substrate)
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/crates/sparq-engine)
   Compiling sparq-wasm v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-trees-sx2mrvfi/head/crates/sparq-wasm)
    Finished `release-wasm` profile [optimized] target(s) in 51.77s
````

## Actual addition-neutral compiler stderr

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/crates/sparq-core)
   Compiling sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/crates/sparq-substrate)
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/crates/sparq-engine)
   Compiling sparq-wasm v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-neutral-thbdftec/tree/crates/sparq-wasm)
    Finished `release-wasm` profile [optimized] target(s) in 47.74s
````

## Actual deletion-neutral compiler stderr

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/crates/sparq-core)
   Compiling sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/crates/sparq-substrate)
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/crates/sparq-engine)
   Compiling sparq-wasm v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/proof-trees-6334/featoff-delneutral-fibrbfhu/tree/crates/sparq-wasm)
    Finished `release-wasm` profile [optimized] target(s) in 52.45s
````

## Exact original report-only result and proposal

````markdown
### feature-OFF declaration derivation

* **outcome**: `declared`
* every added line was proved to emit nothing (neutral bundle == head bundle) and every deleted line was proved to have emitted nothing (base-neutral bundle == base bundle); the drift is line-position metadata only

```json
{
  "base_bundle_bytes": 1560265,
  "head_bundle_bytes": 1560265,
  "size_delta_bytes": 0,
  "differing_bytes": 12,
  "closure_files_changed": [
    ".github/feature-matrix.d/sparq-core.yml",
    "bench/benchmarks.toml",
    "bench/overlay-count/README.md",
    "bench/overlay-count/src/counting.rs",
    "bench/overlay-count/src/lifecycle.rs",
    "bench/overlay-count/src/main.rs",
    "crates/sparq-core/README.md",
    "crates/sparq-core/src/store.rs",
    "crates/sparq-core/src/store/overlay_deleted_tests.rs",
    "scripts/tests/feature-matrix-legnames.golden.txt",
    "skills/sparql-query/SKILL.md"
  ],
  "manifest_files_changed": [
    "bench/overlay-count/Cargo.lock",
    "bench/overlay-count/Cargo.toml",
    "crates/sparq-core/Cargo.toml"
  ],
  "files_in_compiled_closure": [
    "crates/sparq-core/Cargo.toml",
    "crates/sparq-core/README.md",
    "crates/sparq-core/src/store.rs",
    "crates/sparq-core/src/store/overlay_deleted_tests.rs"
  ],
  "closure_dirs": [
    "crates/sparq-core/",
    "crates/sparq-engine/",
    "crates/sparq-substrate/",
    "crates/sparq-wasm/",
    "vendor/spargebra/"
  ],
  "deleted_nonblank_lines": 10,
  "added_nonblank_lines_blanked": 1123,
  "neutral_tree_files_mutated": 14
}
```

Commit this as `bench/feature-off-declarations/6469.json`:

```json
{
  "pr": 6469,
  "date": "2026-09-09",
  "reason": "[OPUS-5] #6469: DERIVED declaration (scripts/feature_off_autodeclare.py). The feature-OFF bundle moved 12 of 1560265 bytes at a size delta of +0, and the drift was proved to carry no compiled code. Specifically: rebuilding the head tree with all 1123 added non-blank line(s) blanked produced a BYTE-IDENTICAL bundle, and rebuilding the base tree with all 10 deleted non-blank line(s) blanked left the base bundle unchanged. Both directions were covered. What moved is line-position metadata (core::panic::Location line numbers shift when lines above them move); no always-compiled code entered or left the default build. Bundle SIZE remains governed separately by the wasm_bundle_bytes ratchet.",
  "derived": true,
  "evidence": {
    "base_bundle_bytes": 1560265,
    "head_bundle_bytes": 1560265,
    "size_delta_bytes": 0,
    "differing_bytes": 12,
    "closure_files_changed": [
      ".github/feature-matrix.d/sparq-core.yml",
      "bench/benchmarks.toml",
      "bench/overlay-count/README.md",
      "bench/overlay-count/src/counting.rs",
      "bench/overlay-count/src/lifecycle.rs",
      "bench/overlay-count/src/main.rs",
      "crates/sparq-core/README.md",
      "crates/sparq-core/src/store.rs",
      "crates/sparq-core/src/store/overlay_deleted_tests.rs",
      "scripts/tests/feature-matrix-legnames.golden.txt",
      "skills/sparql-query/SKILL.md"
    ],
    "manifest_files_changed": [
      "bench/overlay-count/Cargo.lock",
      "bench/overlay-count/Cargo.toml",
      "crates/sparq-core/Cargo.toml"
    ],
    "files_in_compiled_closure": [
      "crates/sparq-core/Cargo.toml",
      "crates/sparq-core/README.md",
      "crates/sparq-core/src/store.rs",
      "crates/sparq-core/src/store/overlay_deleted_tests.rs"
    ],
    "closure_dirs": [
      "crates/sparq-core/",
      "crates/sparq-engine/",
      "crates/sparq-substrate/",
      "crates/sparq-wasm/",
      "vendor/spargebra/"
    ],
    "deleted_nonblank_lines": 10,
    "added_nonblank_lines_blanked": 1123,
    "neutral_tree_files_mutated": 14
  }
}
```
````

## Complete unchanged repository proof source (context; no gate/tool modification)

````python
#!/usr/bin/env python3
"""
[OPUS-5] sq-v3nel-v3: DERIVE a feature-OFF wasm-bundle declaration instead of asserting one.

The `artifact-exact-equality (wasm bundle feature-OFF)` leg
(`.github/workflows/vectorized-feature-off.yml`, leg 2) compares the feature-OFF
`sparq-wasm` bundle built from the BASE tree against the one built from the HEAD tree,
BYTE-FOR-BYTE, and fails an undeclared difference. That is the right guard — it is what
catches `vectorized`/default-path code leaking into the default build.

It also fires on a class of change that carries NO semantic content at all. MEASURED on
this repo (see `bench/feature-off-declarations/README.md` § "Why the gate fires on
comment-only edits"): inserting 34 pure comment lines into `crates/sparq-core/src/compress.rs`
and rebuilding produces a bundle that DIFFERS from base in exactly 3 bytes at exactly the
same length, because `core::panic::Location` records embed the LINE NUMBER of every
panicking call site and those numbers move when lines above them move. Adding an
off-by-default `#[cfg(feature = ...)]` item mid-file does the same thing: the item itself
compiles out, but every line below it shifts. A worker therefore has to hand-write a
declaration for a diff that changed no compiled code whatsoever.

This tool decides that case MECHANICALLY, and — this is the whole point — it decides it by
RE-DERIVING the drift from the compiler, never by asserting a tolerance:

    1. Build the BASE tree's feature-OFF bundle and the HEAD tree's. If they are identical
       there is nothing to declare (the leg already passes).

    2. Construct a NEUTRAL tree: the HEAD tree with every line the diff ADDED replaced by an
       EMPTY line, and with every changed manifest (`Cargo.toml` / `Cargo.lock`) taken from
       the BASE tree. By construction the neutral tree holds the BASE tree's compiled
       content at the HEAD tree's LINE POSITIONS.

    3. Build it. If `neutral == head` byte-for-byte, then blanking every added line changed
       nothing the compiler emits — i.e. the added lines contributed no compiled code, and
       the whole base->head drift is line-position metadata. That is a POSITIVE PROOF from
       the compiler, not an inference from the diff's shape.

    4. Deleted lines are absent from both the head and the neutral tree, so step 3 cannot
       speak for them. When the diff removes any non-blank line from a `.rs` file, build a
       second neutral tree — the BASE tree with exactly those lines blanked — and require it
       to equal the BASE bundle. Same proof, run backwards.

Only when every obligation holds is a declaration written, and its `reason` records the
MEASURED evidence (byte counts, differing-byte count, which obligations were discharged).
Anything else REFUSES with a named reason and leaves the leg RED — an unexplained diff must
never be auto-declared, because that would convert the guard into a rubber stamp.

Deliberately NOT done here: this tool does not make the gate pass by itself and it is not
wired to any write-scoped token. It writes a file into the working tree; a human or the
authoring agent commits it, so the escape hatch stays inside the reviewed diff exactly as
mechanism V2 intends.

Usage (report only — what CI runs):
    python3 scripts/feature_off_autodeclare.py --repo . --base-sha <sha> --head-sha <sha> \
        --pr 1234 --report-only

Usage (write the declaration into the working tree):
    python3 scripts/feature_off_autodeclare.py --repo . --base-sha <sha> --head-sha <sha> \
        --pr 1234 --write
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

# ---------------------------------------------------------------------------
# Refusal reasons. Every one of these is a NAMED outcome so a census row can say
# *why* a PR was refused, and so each has its own test.
# ---------------------------------------------------------------------------
REFUSE_BASE_BUILD_FAILED = "base-build-failed"
REFUSE_HEAD_BUILD_FAILED = "head-build-failed"
REFUSE_NEUTRAL_BUILD_FAILED = "neutral-build-failed"
REFUSE_DELETION_BUILD_FAILED = "deletion-neutral-build-failed"
REFUSE_ADDED_LINES_ARE_SEMANTIC = "added-lines-are-semantic"
REFUSE_DELETED_LINES_ARE_SEMANTIC = "deleted-lines-are-semantic"
REFUSE_UNSUPPORTED_FILE_CHANGE = "unsupported-file-change"
REFUSE_VACUOUS_PROOF = "proof-would-be-vacuous"
REFUSE_DIFF_TREE_MISMATCH = "diff-paths-absent-from-both-trees"
REFUSE_NO_DIFF = "no-diff-between-base-and-head"

OUTCOME_NO_DRIFT = "no-drift"
OUTCOME_DECLARED = "declared"

_RUST_SUFFIX = ".rs"
_MANIFESTS = ("Cargo.toml", "Cargo.lock")


def _is_manifest(path: str) -> bool:
    return os.path.basename(path) in _MANIFESTS


def _is_rust(path: str) -> bool:
    return path.endswith(_RUST_SUFFIX)


# Root-level inputs that reach every build regardless of which crate they sit next to.
_ROOT_BUILD_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain")


def closure_dirs(tree: str, package: str = "sparq-wasm") -> list[str] | None:
    """The IN-REPO crate directories `package`'s DEFAULT wasm build actually compiles.

    DERIVED from cargo, never hardcoded: `cargo tree` for the wasm32 target with default
    features names the closure, and `cargo metadata` maps each package to its directory.

    REPORTING ONLY. This once decided which paths the proof covered, and that was the
    source of a live false pass — see `classify_paths`. Soundness no longer depends on it:
    every changed non-manifest path is blanked regardless of what this returns. It is kept
    because naming which changed files the bundle actually compiles is useful evidence in
    the declaration and in a refusal.

    Returns None when cargo cannot answer, which now costs nothing but the evidence line.
    """
    try:
        tree_out = subprocess.run(
            ["cargo", "tree", "-p", package, "--target", "wasm32-unknown-unknown",
             "--edges", "normal", "--prefix", "none"],
            cwd=tree, capture_output=True, text=True, check=True).stdout
        # WITH deps, deliberately. `--no-deps` returns WORKSPACE MEMBERS ONLY, which misses
        # any crate pulled in by `[patch.crates-io]` from a path the workspace `exclude`s —
        # `vendor/spargebra` on this repo, which really is compiled into the bundle.
        meta = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=tree, capture_output=True, text=True, check=True).stdout)
    except Exception:
        return None
    return closure_dirs_from(tree_out, meta, tree)


def closure_dirs_from(tree_out: str, meta: dict, tree: str) -> list[str]:
    """Pure half of `closure_dirs`: intersect a `cargo tree` listing with package dirs."""
    in_closure = {line.split()[0] for line in tree_out.splitlines() if line.strip()}
    real_tree = os.path.realpath(tree)
    dirs = []
    for pkg in meta.get("packages", []):
        if pkg["name"] not in in_closure:
            continue
        pkg_dir = os.path.realpath(os.path.dirname(pkg["manifest_path"]))
        # Registry crates live under ~/.cargo and cannot be touched by a PR diff; only
        # in-repo sources are addressable, so only those become directories to report.
        if pkg_dir == real_tree or not pkg_dir.startswith(real_tree + os.sep):
            continue
        dirs.append(os.path.relpath(pkg_dir, real_tree).rstrip("/") + "/")
    return sorted(set(dirs))


def classify_paths(paths: list[str],
                   closure: list[str] | None = None) -> tuple[list[str], list[str], list[str]]:
    """Split changed paths into (blankable, manifest, in_closure_report).

    EVERY changed path that is not a manifest is BLANKABLE. The classification is
    deliberately NOT used to decide what the proof covers, because that is precisely how
    this tool produced a live false pass:

        sparq's root manifest carries `exclude = ["vendor/spargebra", ...]` together with
        `[patch.crates-io] spargebra = { path = "vendor/spargebra" }`, so spargebra IS
        compiled into the feature-OFF bundle while NOT being a workspace member. An earlier
        version scoped blanking to a closure derived from `cargo metadata --no-deps` —
        which returns workspace members only — so `vendor/spargebra/src/parser.rs`
        classified INERT, was never blanked, and `neutral == head` held BY CONSTRUCTION.
        Three genuinely non-benign changes there all came back `declared`.

    A skipped path cannot be proved harmless, so nothing is skipped. Blanking a file the
    build never reads is free — the bundle is unchanged either way — while blanking one it
    DOES read changes the bundle and the derivation refuses. Letting the compiler answer
    costs nothing and removes an entire class of classification bug.

    Manifests are handled separately (restored from the base tree) because blanking a line
    out of a `Cargo.toml` yields a manifest, not an absence.

    `closure` is reported as evidence only; it no longer gates anything.
    """
    blankable: list[str] = []
    manifest: list[str] = []
    in_closure: list[str] = []
    for p in paths:
        root_input = p in _ROOT_BUILD_INPUTS or p.startswith(".cargo/")
        if _is_manifest(p) or root_input:
            manifest.append(p)
        else:
            blankable.append(p)
        if closure is not None and (root_input or any(p.startswith(d) for d in closure)):
            in_closure.append(p)
    return blankable, manifest, in_closure


# ---------------------------------------------------------------------------
# Diff parsing: which NEW-file lines were added, which OLD-file lines were removed.
# ---------------------------------------------------------------------------

_HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def parse_unified_diff(diff_text: str) -> dict[str, dict[str, list[int]]]:
    """Map path -> {"added": [1-based new-file line numbers],
                    "removed": [1-based old-file line numbers]}.

    Only the line NUMBERS are needed: the content is read back from the real trees, so a
    truncated or context-less diff cannot smuggle content past the obligations.
    """
    out: dict[str, dict[str, list[int]]] = {}
    path: str | None = None
    old_path: str | None = None
    old_ln = new_ln = 0
    for line in diff_text.split("\n"):
        if line.startswith("diff --git "):
            path = old_path = None
            continue
        if line.startswith("--- "):
            src = line[4:].strip()
            old_path = None if src == "/dev/null" else (src[2:] if src.startswith("a/") else src)
            continue
        if line.startswith("+++ "):
            target = line[4:].strip()
            if target == "/dev/null":
                # File DELETED by this diff. Its removed lines must still be attributed, so
                # key them under the OLD path rather than dropping the hunk on the floor.
                path = old_path
            else:
                path = target[2:] if target.startswith("b/") else target
            if path is not None:
                out.setdefault(path, {"added": [], "removed": []})
            continue
        m = _HUNK.match(line)
        if m:
            old_ln = int(m.group(1))
            new_ln = int(m.group(3))
            continue
        if path is None:
            continue
        if line.startswith("+"):
            out[path]["added"].append(new_ln)
            new_ln += 1
        elif line.startswith("-"):
            out[path]["removed"].append(old_ln)
            old_ln += 1
        elif line.startswith(" ") or line == "":
            old_ln += 1
            new_ln += 1
        # "\\ No newline at end of file" and everything else: no line consumed.
    return out


def blank_lines(text: str, line_numbers: list[int]) -> str:
    """Replace the given 1-based lines of `text` with EMPTY lines, preserving line count.

    Preserving the line count is the load-bearing property: it is what makes the neutral
    tree hold the base tree's content at the head tree's line POSITIONS, so a byte-identical
    build proves the blanked lines emitted nothing.
    """
    lines = text.split("\n")
    for n in line_numbers:
        if 1 <= n <= len(lines):
            lines[n - 1] = ""
    return "\n".join(lines)


def nonblank_count(text: str, line_numbers: list[int]) -> int:
    """How many of the given 1-based lines carry non-whitespace content."""
    lines = text.split("\n")
    return sum(1 for n in line_numbers if 1 <= n <= len(lines) and lines[n - 1].strip())


# ---------------------------------------------------------------------------
# Tree materialisation + the real cargo builder.
# ---------------------------------------------------------------------------

def export_tree(repo: str, sha: str, dest: str) -> None:
    """Materialise `sha` into `dest` via `git archive` (never a checkout of the caller's tree)."""
    os.makedirs(dest, exist_ok=True)
    archive = subprocess.run(
        ["git", "-C", repo, "archive", "--format=tar", sha],
        capture_output=True, check=True,
    )
    subprocess.run(["tar", "-x", "-C", dest], input=archive.stdout, check=True)


def cargo_wasm_builder(tree: str) -> bytes | None:
    """Build the feature-OFF sparq-wasm bundle in `tree`; return its bytes, or None on failure.

    Mirrors the leg-2 build exactly: default features only (no `--features vectorized`),
    the `release-wasm` profile, the wasm32 target.
    """
    proc = subprocess.run(
        ["cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm",
         "--target", "wasm32-unknown-unknown",
         "--target-dir", os.path.join(tree, "target")],
        cwd=tree, capture_output=True,
        env={**os.environ, "CARGO_TARGET_DIR": os.path.join(tree, "target")},
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr.decode("utf-8", "replace")[-4000:])
        return None
    # PER-TREE target directory, deliberately. Pointing several materialised trees at ONE
    # shared CARGO_TARGET_DIR to keep the extra builds warm was TRIED and REVERTED: it
    # produced a demonstrably WRONG verdict. Run against #4350 — a PR whose merge-base diff
    # is a 299-line rewrite of crates/sparq-engine/src/exec.rs — the shared-directory run
    # reported the base and head bundles BYTE-IDENTICAL ("no-drift"), i.e. a stale artefact
    # was read back as a fresh build. `git archive` stamps each exported tree's files with
    # its COMMIT time, and cargo's freshness check is mtime-based, so an out-of-order
    # timestamp can leave a previous tree's `sparq_wasm.wasm` in place.
    #
    # A false "identical" is the worst failure this tool has: it would auto-declare a real
    # code change as line-position churn. Cold builds are the price of the proof meaning
    # anything, so `--target-dir` stays inside the tree being built and the environment
    # cannot override it.
    env_override = os.environ.get("CARGO_TARGET_DIR")
    if env_override:
        sys.stderr.write(
            f"[autodeclare] ignoring CARGO_TARGET_DIR={env_override!r}: each tree must build "
            "into its own target directory (a shared one has been observed returning a stale "
            "bundle, which reads as a false 'identical').\n")
    out = os.path.join(tree, "target", "wasm32-unknown-unknown", "release-wasm", "sparq_wasm.wasm")
    if not os.path.exists(out):
        return None
    with open(out, "rb") as fh:
        return fh.read()


def differing_bytes(a: bytes, b: bytes) -> int:
    """Count positions at which two byte strings differ (length difference counts as differing)."""
    n = min(len(a), len(b))
    return sum(1 for i in range(n) if a[i] != b[i]) + abs(len(a) - len(b))


# ---------------------------------------------------------------------------
# The decision.
# ---------------------------------------------------------------------------

class Verdict:
    def __init__(self, outcome: str, reason: str = "", evidence: dict | None = None):
        self.outcome = outcome            # OUTCOME_* or a REFUSE_* name
        self.reason = reason              # human-readable one-liner
        self.evidence = evidence or {}

    @property
    def declared(self) -> bool:
        return self.outcome == OUTCOME_DECLARED

    @property
    def refused(self) -> bool:
        return self.outcome not in (OUTCOME_DECLARED, OUTCOME_NO_DRIFT)


def decide(base_tree: str, head_tree: str, diff_text: str, builder,
           base_bytes: bytes | None = None, head_bytes: bytes | None = None) -> Verdict:
    """Derive whether the base->head feature-OFF drift is line-position churn ONLY.

    `builder(tree_dir) -> bytes | None` compiles a materialised tree. Injected so the
    decision logic is testable without cargo; production passes `cargo_wasm_builder`.

    `base_bytes` / `head_bytes` let a caller hand in bundles the leg-2 job already built,
    so the derivation costs one extra build (two when the diff deletes code) rather than
    four. They are used verbatim — the obligations below still rebuild the NEUTRAL trees
    with the same builder, so a handed-in bundle cannot short-circuit any proof.
    """
    changes = parse_unified_diff(diff_text)
    if not changes:
        return Verdict(REFUSE_NO_DIFF, "the base..head diff is empty — nothing to attribute")

    closure = closure_dirs(head_tree)
    rust_paths, manifest_paths, closure_paths = classify_paths(sorted(changes), closure)

    # The diff and the trees must actually describe the same thing. A changed path that
    # exists in NEITHER tree means they disagree — a mis-parsed diff, a bad path prefix, a
    # stale export. Every such path would be silently skipped by the blanking loop, so the
    # proof would quietly cover less than it claims. Caught for real: an ad-hoc harness
    # rewrote `git diff --no-index` prefixes in the wrong order, yielding paths like
    # `bvendor/spargebra/src/parser.rs`; nothing matched, nothing was blanked, and only the
    # non-vacuity guard stood between that and a false declaration. Name it instead.
    missing = [p for p in rust_paths
               if not os.path.exists(os.path.join(head_tree, p))
               and not os.path.exists(os.path.join(base_tree, p))]
    if missing:
        return Verdict(REFUSE_DIFF_TREE_MISMATCH,
                       "the diff names paths that exist in neither the base nor the head "
                       "tree, so the diff and the trees disagree and the proof would cover "
                       "less than it claims: " + ", ".join(sorted(missing)[:10]))

    # Blanking rewrites files in place, which is meaningless for a symlink or a submodule
    # gitlink — the proof would silently cover nothing. Refuse rather than pretend.
    nonregular = [p for p in rust_paths
                  if os.path.islink(os.path.join(head_tree, p))
                  or os.path.islink(os.path.join(base_tree, p))]
    if nonregular:
        return Verdict(REFUSE_UNSUPPORTED_FILE_CHANGE,
                       "changed inside the wasm build closure but not a regular file, so "
                       "blanking cannot speak for it: " + ", ".join(sorted(nonregular)))

    if base_bytes is None:
        base_bytes = builder(base_tree)
    if base_bytes is None:
        return Verdict(REFUSE_BASE_BUILD_FAILED, "the BASE tree did not build")
    if head_bytes is None:
        head_bytes = builder(head_tree)
    if head_bytes is None:
        return Verdict(REFUSE_HEAD_BUILD_FAILED, "the HEAD tree did not build")

    if base_bytes == head_bytes:
        return Verdict(OUTCOME_NO_DRIFT,
                       "base and head bundles are byte-identical; leg 2 already passes",
                       {"bundle_bytes": len(head_bytes)})

    evidence = {
        "base_bundle_bytes": len(base_bytes),
        "head_bundle_bytes": len(head_bytes),
        "size_delta_bytes": len(head_bytes) - len(base_bytes),
        "differing_bytes": differing_bytes(base_bytes, head_bytes),
        "closure_files_changed": rust_paths,
        "manifest_files_changed": manifest_paths,
        "files_in_compiled_closure": closure_paths,
        "closure_dirs": closure,
    }

    # ---- Obligation 1: additions contribute no compiled code ----------------
    # Neutral tree = HEAD with every added .rs line blanked and every changed manifest
    # restored from BASE. It therefore holds BASE's compiled content at HEAD's line
    # positions. If its bundle equals HEAD's, the additions emitted nothing.
    removed_nonblank = 0
    to_blank: dict[str, list[int]] = {}
    for path in rust_paths:
        removed = changes[path]["removed"]
        if not removed:
            continue
        src = os.path.join(base_tree, path)
        if not os.path.exists(src):
            continue
        with open(src, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        n = nonblank_count(text, removed)
        if n:
            removed_nonblank += n
            to_blank[path] = removed
    evidence["deleted_nonblank_lines"] = removed_nonblank

    neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-neutral-"), "tree")
    shutil.copytree(head_tree, neutral, symlinks=True)
    added_nonblank = 0
    mutated: list[str] = []
    for path in rust_paths:
        added = changes[path]["added"]
        if not added:
            continue
        target = os.path.join(neutral, path)
        if not os.path.exists(target):
            continue
        with open(target, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        added_nonblank += nonblank_count(text, added)
        blanked = blank_lines(text, added)
        if blanked != text:
            mutated.append(path)
        with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
            fh.write(blanked)
    for path in manifest_paths:
        src = os.path.join(base_tree, path)
        dst = os.path.join(neutral, path)
        head_bytes_of = open(dst, "rb").read() if os.path.exists(dst) else None
        if os.path.exists(src):
            shutil.copyfile(src, dst)
            if open(dst, "rb").read() != head_bytes_of:
                mutated.append(path)
        elif os.path.exists(dst):
            os.remove(dst)
            mutated.append(path)
    evidence["added_nonblank_lines_blanked"] = added_nonblank
    evidence["neutral_tree_files_mutated"] = len(mutated)

    # NON-VACUITY. Obligation 1 concludes "the additions emitted nothing" from
    # `build(neutral) == build(head)`. If the neutral tree is IDENTICAL to the head tree,
    # that comparison is `head == head` — it holds by construction and proves nothing, so a
    # declaration derived from it would be exactly the rubber stamp this leg exists to
    # prevent. This is the general form of the vendored-crate false pass: whenever the diff
    # is real but nothing got blanked, the drift came from somewhere the proof does not
    # reach. Refuse, and say where the diff actually was.
    if not mutated and not to_blank:
        return Verdict(
            REFUSE_VACUOUS_PROOF,
            "neither obligation has anything to prove: the neutral tree is identical to the "
            "head tree (nothing was blanked) and the diff deletes no non-blank line, so "
            "comparing the bundles would be `head == head` and would hold by construction. "
            "The bundle moved anyway, so the drift originates outside what the proof "
            "reaches — do not declare it. Changed paths: "
            + ", ".join(sorted(changes)[:20]),
            evidence)

    neutral_bytes = builder(neutral)
    if neutral_bytes is None:
        # Blanking the added lines broke the build => at least one of them was load-bearing.
        return Verdict(REFUSE_NEUTRAL_BUILD_FAILED,
                       "blanking the added lines broke the build, so they are compiled code, "
                       "not comments or compiled-out cfg-gated tokens",
                       evidence)
    if neutral_bytes != head_bytes:
        evidence["neutral_vs_head_size_delta"] = len(head_bytes) - len(neutral_bytes)
        evidence["neutral_vs_head_differing_bytes"] = differing_bytes(neutral_bytes, head_bytes)
        return Verdict(REFUSE_ADDED_LINES_ARE_SEMANTIC,
                       "blanking the added lines CHANGED the compiled bundle, so the diff adds "
                       "code to the default build; an intentional always-compiled change must "
                       "be declared by its author, not derived",
                       evidence)

    # ---- Obligation 2: deletions removed no compiled code -------------------
    # Deleted lines are absent from BOTH head and neutral, so obligation 1 cannot speak for
    # them. Run the same proof on the base side: blank exactly those lines in BASE and
    # require the bundle to be unchanged.
    if to_blank:
        del_neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-delneutral-"), "tree")
        shutil.copytree(base_tree, del_neutral, symlinks=True)
        for path, removed in to_blank.items():
            target = os.path.join(del_neutral, path)
            with open(target, encoding="utf-8", errors="surrogateescape") as fh:
                text = fh.read()
            with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
                fh.write(blank_lines(text, removed))
        del_bytes = builder(del_neutral)
        if del_bytes is None:
            return Verdict(REFUSE_DELETION_BUILD_FAILED,
                           "blanking the deleted lines in the BASE tree broke the build, so the "
                           "diff removes compiled code",
                           evidence)
        if del_bytes != base_bytes:
            evidence["delneutral_vs_base_size_delta"] = len(base_bytes) - len(del_bytes)
            evidence["delneutral_vs_base_differing_bytes"] = differing_bytes(del_bytes, base_bytes)
            return Verdict(REFUSE_DELETED_LINES_ARE_SEMANTIC,
                           "blanking the deleted lines in the BASE tree CHANGED the compiled "
                           "bundle, so the diff removes code from the default build; that is an "
                           "always-compiled change its author must declare",
                           evidence)

    return Verdict(
        OUTCOME_DECLARED,
        "every added line was proved to emit nothing (neutral bundle == head bundle) and "
        "every deleted line was proved to have emitted nothing (base-neutral bundle == base "
        "bundle); the drift is line-position metadata only",
        evidence,
    )


def _obligation_sentence(ev: dict) -> str:
    """Say which obligation actually carried the proof.

    A count of zero is normal on one side — a pure-deletion diff blanks no added line, and
    an addition-only diff deletes none — but phrasing it as "all 0 lines blanked produced an
    identical bundle" reads as a confession that nothing was checked. It also read that way
    when something really HAD been missed: the vendored-crate false pass emitted exactly
    that sentence with both counts at zero. The non-vacuity guard now refuses that state
    outright; this wording makes the remaining zeroes unambiguous.
    """
    added = ev.get("added_nonblank_lines_blanked", 0)
    deleted = ev.get("deleted_nonblank_lines", 0)
    parts = []
    if added:
        parts.append(f"rebuilding the head tree with all {added} added non-blank line(s) "
                     "blanked produced a BYTE-IDENTICAL bundle")
    if deleted:
        parts.append(f"rebuilding the base tree with all {deleted} deleted non-blank line(s) "
                     "blanked left the base bundle unchanged")
    if not parts:  # unreachable: the non-vacuity guard refuses before this point
        return ("NO obligation was discharged — this declaration must not have been derived."
                " Treat it as a bug in the derivation, not as evidence.")
    joined = ", and ".join(parts)
    covered = ("Both directions were covered" if len(parts) == 2
               else "The diff changes lines in one direction only, so that is the whole "
                    "obligation")
    return f"Specifically: {joined}. {covered}."


def declaration_json(pr: int, verdict: Verdict, date: str | None = None) -> dict:
    """The per-PR declaration file content, carrying the MEASURED evidence."""
    ev = verdict.evidence
    return {
        "pr": pr,
        "date": date or datetime.date.today().isoformat(),
        "reason": (
            f"[OPUS-5] #{pr}: DERIVED declaration (scripts/feature_off_autodeclare.py). The "
            f"feature-OFF bundle moved {ev.get('differing_bytes')} of "
            f"{ev.get('head_bundle_bytes')} bytes at a size delta of "
            f"{ev.get('size_delta_bytes'):+d}, and the drift was proved to carry no compiled "
            f"code. {_obligation_sentence(ev)} What moved is line-position metadata "
            "(core::panic::Location line numbers shift when lines above them move); no "
            "always-compiled code entered or left the default build. Bundle SIZE remains "
            "governed separately by the wasm_bundle_bytes ratchet."
        ),
        "derived": True,
        "evidence": ev,
    }


def census_row(pr: int, verdict: Verdict) -> str:
    """One machine-greppable census line, emitted on every run."""
    return (
        "FEATURE-OFF-CENSUS "
        f"pr={pr} outcome={verdict.outcome} "
        f"size_delta={verdict.evidence.get('size_delta_bytes', 'na')} "
        f"differing_bytes={verdict.evidence.get('differing_bytes', 'na')} "
        f"reason={verdict.reason.split(';')[0][:160]!r}"
    )


# ---------------------------------------------------------------------------
# Census sweep: how big is this class right now, and what happened to each member?
# ---------------------------------------------------------------------------

LEG_NAME = "artifact-exact-equality (wasm bundle feature-OFF)"


def _gh_json(path: str) -> object:
    proc = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip()[:400])
    return json.loads(proc.stdout)


def _default_pr_lister(repo: str) -> list[dict]:
    proc = subprocess.run(
        ["gh", "pr", "list", "--repo", repo, "--state", "open", "--limit", "200",
         "--json", "number,headRefOid,isDraft,title"],
        capture_output=True, text=True, check=True,
    )
    return json.loads(proc.stdout)


def _default_check_lister(repo: str, sha: str) -> list[dict]:
    runs: list[dict] = []
    page = 1
    while page <= 5:
        d = _gh_json(f"/repos/{repo}/commits/{sha}/check-runs?per_page=100&page={page}")
        batch = d.get("check_runs", [])  # type: ignore[union-attr]
        runs.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return runs


def census_sweep(repo: str, declarations_dir_listing: set[str],
                 pr_lister=_default_pr_lister, check_lister=_default_check_lister) -> dict:
    """Tally the live population of the feature-OFF declaration class.

    Membership is decided by the LEG's own conclusion on each PR's CURRENT head SHA (matched
    by NAME EQUALITY, and paginated), never by a cached label — a stale classification is how
    this class goes quietly wrong. Returns counts plus the per-PR rows.
    """
    rows: list[dict] = []
    prs = pr_lister(repo)
    for pr in prs:
        try:
            checks = check_lister(repo, pr["headRefOid"])
        except Exception as exc:
            rows.append({"pr": pr["number"], "state": "check-lookup-failed", "detail": str(exc)})
            continue
        # NAME EQUALITY, newest run wins. A substring match would sweep in an advisory twin,
        # and a first-run-wins scan would read a superseded red on a re-run head.
        leg: dict | None = None
        for c in checks:
            if c["name"] != LEG_NAME:
                continue
            if leg is None or (c.get("started_at") or "") >= (leg.get("started_at") or ""):
                leg = c
        if leg is None:
            state = "leg-absent"
        elif leg.get("conclusion") == "failure":
            state = "in-class-red"
        elif leg.get("status") != "completed":
            state = "leg-running"
        else:
            state = "leg-green"
        rows.append({
            "pr": pr["number"], "draft": pr.get("isDraft"), "state": state,
            "declared": f"{pr['number']}.json" in declarations_dir_listing,
        })
    in_class = [r for r in rows if r.get("state") == "in-class-red"]
    return {
        "open_prs": len(prs),
        "in_class_red": len(in_class),
        "in_class_already_declared": sum(1 for r in in_class if r.get("declared")),
        "in_class_undeclared": sum(1 for r in in_class if not r.get("declared")),
        "leg_running": sum(1 for r in rows if r.get("state") == "leg-running"),
        "leg_absent": sum(1 for r in rows if r.get("state") == "leg-absent"),
        "lookup_failed": sum(1 for r in rows if r.get("state") == "check-lookup-failed"),
        "rows": rows,
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--repo", default=".", help="git repository to export trees from")
    ap.add_argument("--base-sha")
    ap.add_argument("--head-sha")
    ap.add_argument("--pr", type=int)
    ap.add_argument("--declarations-dir", default="bench/feature-off-declarations")
    ap.add_argument("--base-wasm", help="a base-tree bundle the caller already built")
    ap.add_argument("--head-wasm", help="a head-tree bundle the caller already built")
    ap.add_argument("--write", action="store_true",
                    help="write the declaration file into --repo's working tree")
    ap.add_argument("--report-only", action="store_true",
                    help="print the verdict and the declaration that WOULD be written")
    ap.add_argument("--census", metavar="OWNER/REPO",
                    help="sweep the live open-PR population of this class and exit")
    ap.add_argument("--summary-file", default=os.environ.get("GITHUB_STEP_SUMMARY", ""))
    args = ap.parse_args(argv)

    if args.census:
        decl_dir = os.path.join(args.repo, args.declarations_dir)
        listing = set(os.listdir(decl_dir)) if os.path.isdir(decl_dir) else set()
        rep = census_sweep(args.census, listing)
        print("FEATURE-OFF-CENSUS-SWEEP " + " ".join(
            f"{k}={v}" for k, v in rep.items() if k != "rows"))
        for r in rep["rows"]:
            if r.get("state") in ("in-class-red", "check-lookup-failed"):
                print("  " + json.dumps(r))
        return 0

    missing = [n for n in ("base_sha", "head_sha", "pr") if getattr(args, n) is None]
    if missing:
        ap.error("required unless --census is given: " + ", ".join("--" + m.replace("_", "-")
                                                                   for m in missing))

    prebuilt: dict[str, bytes | None] = {"base": None, "head": None}
    for key, path in (("base", args.base_wasm), ("head", args.head_wasm)):
        if path:
            with open(path, "rb") as fh:
                prebuilt[key] = fh.read()

    # ATTRIBUTION BASE = the MERGE BASE, not `pull_request.base.sha`. Those are different
    # commits whenever the branch is behind its base: GitHub sets `base.sha` to the base
    # BRANCH TIP at the last sync, so a `base.sha`..`head` diff also carries, in reverse,
    # every base-branch commit the PR does not have. MEASURED on the live population: 3 of
    # the 6 open PRs red on this leg had a non-ancestor `base.sha`, and one of them showed
    # 22 changed files against `base.sha` for a ONE-file PR. Attributing that to the PR
    # would be wrong in both directions, so the proof runs against the merge base and the
    # difference is reported rather than hidden.
    merge_base = subprocess.run(
        ["git", "-C", args.repo, "merge-base", args.base_sha, args.head_sha],
        capture_output=True, text=True, check=True).stdout.strip()
    if merge_base != args.base_sha:
        print(f"[autodeclare] NOTE: leg 2's base ({args.base_sha[:12]}) is not the merge base "
              f"({merge_base[:12]}); attributing against the merge base, and the supplied "
              "base bundle is ignored because it was built from a different tree.")
        prebuilt["base"] = None

    workdir = tempfile.mkdtemp(prefix="featoff-trees-")
    base_tree = os.path.join(workdir, "base")
    head_tree = os.path.join(workdir, "head")
    export_tree(args.repo, merge_base, base_tree)
    export_tree(args.repo, args.head_sha, head_tree)
    diff_text = subprocess.run(
        ["git", "-C", args.repo, "diff", "--no-color", "--unified=0",
         merge_base, args.head_sha],
        capture_output=True, text=True, check=True,
    ).stdout

    verdict = decide(base_tree, head_tree, diff_text, cargo_wasm_builder,
                     base_bytes=prebuilt["base"], head_bytes=prebuilt["head"])

    print(census_row(args.pr, verdict))
    print(f"[autodeclare] outcome: {verdict.outcome}")
    print(f"[autodeclare] {verdict.reason}")
    print("[autodeclare] evidence: " + json.dumps(verdict.evidence, indent=2, default=str))

    lines = [
        "### feature-OFF declaration derivation",
        "",
        f"* **outcome**: `{verdict.outcome}`",
        f"* {verdict.reason}",
        "",
        "```json",
        json.dumps(verdict.evidence, indent=2, default=str),
        "```",
    ]
    if verdict.declared:
        doc = declaration_json(args.pr, verdict)
        rel = os.path.join(args.declarations_dir, f"{args.pr}.json")
        lines += ["", f"Commit this as `{rel}`:", "", "```json",
                  json.dumps(doc, indent=2), "```"]
        if args.write:
            dest = os.path.join(args.repo, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "w", encoding="utf-8") as fh:
                json.dump(doc, fh, indent=2)
                fh.write("\n")
            print(f"[autodeclare] wrote {dest}")
        else:
            print(json.dumps(doc, indent=2))
    if args.summary_file:
        try:
            with open(args.summary_file, "a", encoding="utf-8") as fh:
                fh.write("\n".join(lines) + "\n")
        except OSError:
            pass

    # Exit 0 when a declaration was derived or none was needed; non-zero on a REFUSAL so a
    # caller that wired this in cannot mistake "refused" for "handled".
    return 0 if not verdict.refused else 2


if __name__ == "__main__":
    raise SystemExit(main())
````

## Frozen prior manifest verification

````json
{
  "experimental": {
    "files": 88,
    "manifest_sha256": "c0ce2485c772011abf5dd1684d93c4920155a31659b8d868b2c9c06c71eac228",
    "mismatches": []
  },
  "wasm-failure-b86": {
    "files": 14,
    "manifest_sha256": "c05d4ff763fde65988a4cd58c0c28c5d97b873681e0768a10721e69d09da97c6",
    "mismatches": []
  }
}
````

## Exact final source scope

````json
{
  "head": "6334b338587fe5c635c69a09e134917ec35eaaca",
  "status_porcelain": "",
  "delta_files": [
    "crates/sparq-core/src/store.rs"
  ],
  "all_other_tracked_files_including_tests_golden_workflows_manifests_docs_unchanged_from_b86": true,
  "commit_model": "OpenAI GPT-6 Astra; inherited current xhigh lane; no external model call",
  "source_policy": "No declaration file, gate change, runtime algorithm change, threshold, API, Clone/Arc redesign or default enablement."
}
````

## Archived evidence and remaining limits

The adjacent manifest inventories the full base-to-head diff, complete current store and Cargo source snapshots, exact per-tree count source/manifests, all four raw wasm binaries, received/forwarded argv, test logs, control patches/logs, unchanged complete proof source and the exact outside-tree declaration. No test, source, compiler or proof result was mocked. Original protocol `copytree` retains its own per-tree build cache when making neutral copies; the captured logs show actual sparq-core/closure recompilation. No shared or unrelated target is used.

The macOS bundles are 24 bytes larger than the earlier Linux b86 bundles; comparisons are within one environment. Normal Linux gates on the corrected public head have not run. The live PR still points to b86; its failure is not forgiven by this artifact. The feature remains default-off and its disclosed cold/update/retained-generation costs persist. No timing or admission claim is added.
