### .github/feature-matrix.d/sparq-core.yml:72-79

```
72:   test: true
73: 
74: # [GPT-6 Astra] #4246: explicitly exercise the experimental deletion cache;
75: # ordinary/core defaults remain feature-off. Includes ownership/invalidation tests.
76: - name: "sparq-core (overlay-deleted-projections)"
77:   crate: "sparq-core"
78:   features: "overlay-deleted-projections"
79:   test: true
```

### scripts/assemble-feature-matrix.py:166-337

```
166: def load_legs():
167:     fragments = sorted(glob.glob(os.path.join(FRAGMENT_DIR, "*.yml")))
168:     if not fragments:
169:         sys.stderr.write(f"error: no fragment files found under {FRAGMENT_DIR}\n")
170:         sys.exit(1)
171:     legs = []
172:     seen_names = {}
173:     for path in fragments:
174:         rel = os.path.relpath(path)
175:         with open(path, "r", encoding="utf-8") as fh:
176:             data = yaml.safe_load(fh)
177:         if data is None:
178:             # An empty (comment-only) fragment is allowed — a crate placeholder.
179:             continue
180:         if not isinstance(data, list):
181:             sys.stderr.write(
182:                 f"error: {rel}: top level must be a YAML list of legs, got "
183:                 f"{type(data).__name__}\n"
184:             )
185:             sys.exit(1)
186:         for idx, leg in enumerate(data):
187:             where = f"{rel}[{idx}]"
188:             if not isinstance(leg, dict):
189:                 sys.stderr.write(f"error: {where}: leg must be a mapping\n")
190:                 sys.exit(1)
191:             keys = set(leg.keys())
192:             # [SONNET-4.6] sq-ldg8c: REQUIRED_KEYS must all be present; extras are allowed
193:             # ONLY from OPTIONAL_KEYS (tier / tier-reason / test-reason / weight). Any other
194:             # key is still a HARD error (the pre-tier behaviour, minus the optional keys).
195:             missing = REQUIRED_KEYS - keys
196:             extra = keys - REQUIRED_KEYS - OPTIONAL_KEYS
197:             if missing or extra:
198:                 msg = []
199:                 if missing:
200:                     msg.append(f"missing {sorted(missing)}")
201:                 if extra:
202:                     msg.append(f"unexpected {sorted(extra)}")
203:                 sys.stderr.write(f"error: {where}: bad keys ({'; '.join(msg)})\n")
204:                 sys.exit(1)
205:             if not isinstance(leg["name"], str) or not leg["name"].strip():
206:                 sys.stderr.write(f"error: {where}: `name` must be a non-empty string\n")
207:                 sys.exit(1)
208:             if not isinstance(leg["crate"], str) or not leg["crate"].strip():
209:                 sys.stderr.write(f"error: {where}: `crate` must be a non-empty string\n")
210:                 sys.exit(1)
211:             if not isinstance(leg["features"], str) or not leg["features"].strip():
212:                 # cargo rejects a bare `--features` with no value, so an empty feature
213:                 # set is never valid for a leg (the default build belongs in ci.yml's
214:                 # workspace lane, not here).
215:                 sys.stderr.write(
216:                     f"error: {where}: `features` must be a non-empty comma list\n"
217:                 )
218:                 sys.exit(1)
219:             if not isinstance(leg["test"], bool):
220:                 sys.stderr.write(f"error: {where}: `test` must be a boolean\n")
221:                 sys.exit(1)
222:             # [SONNET-4.6] sq-ldg8c: normalise the optional tier. A MISSING tier defaults
223:             # to `test` (a full leg — behaviour-preserving). A present-but-unrecognised
224:             # value (typo, null, non-string) is a HARD ERROR: a demotion is only ever a
225:             # reviewed `tier: check` edit, never inferred from a malformed value.
226:             tier = leg.get("tier", "test")
227:             if not isinstance(tier, str) or tier not in VALID_TIERS:
228:                 sys.stderr.write(
229:                     f"error: {where}: `tier` must be one of {list(VALID_TIERS)} "
230:                     f"(got {tier!r}); a missing `tier` defaults to 'test'. An "
231:                     f"unrecognised value is never silently demoted to the check tier.\n"
232:                 )
233:                 sys.exit(1)
234:             # [FABLE-5] CI-economy grouping: optional explicit weight — a positive,
235:             # finite number. Anything else is a HARD error (a malformed weight must
236:             # never silently skew the bin-packing).
237:             weight = leg.get("weight")
238:             if weight is not None and (
239:                 isinstance(weight, bool)
240:                 or not isinstance(weight, (int, float))
241:                 or not weight > 0
242:                 or weight != weight  # NaN
243:                 or weight == float("inf")
244:             ):
245:                 sys.stderr.write(
246:                     f"error: {where}: `weight` must be a positive finite number "
247:                     f"(got {weight!r}); omit it to use the crate-size heuristic\n"
248:                 )
249:                 sys.exit(1)
250:             name = leg["name"]
251:             if name in seen_names:
252:                 sys.stderr.write(
253:                     f"error: duplicate leg name {name!r} in {where} "
254:                     f"(first seen in {seen_names[name]}); two legs with the same "
255:                     f"check-run name would collapse into one gating check\n"
256:                 )
257:                 sys.exit(1)
258:             seen_names[name] = where
259:             legs.append(
260:                 {
261:                     "name": leg["name"],
262:                     "crate": leg["crate"],
263:                     "features": leg["features"],
264:                     "test": leg["test"],
265:                     # Internal only: the normalised tier drives filter_legs_by_tier and is
266:                     # STRIPPED before the matrix JSON is emitted (the workflow leg shape
267:                     # stays exactly {name, crate, features, test}).
268:                     "tier": tier,
269:                     # Internal only: explicit weight (None => leg_weight() heuristic).
270:                     "weight": weight,
271:                 }
272:             )
273:     return legs
274: 
275: 
276: def filter_legs_by_selection(legs, select_mode, affected_json):
277:     """[FABLE-5] sq-fmx4u.3 (design §5.2): change-based selection over the leg list.
278: 
279:     Keep only the legs whose `crate` is in the affected closure — but ONLY when
280:     the selection pre-job says `--select-mode selected`. Every other input is
281:     FAIL-CLOSED to the FULL leg set (running more is always sound, design §2/§4.3):
282:       * select_mode empty / "shadow" / "full" / anything else  => full set
283:       * affected missing, unparsable, or not a list of strings => full set
284:         (with a loud stderr warning — that combination means a wiring bug, and
285:         the sound degradation is the status quo, never a skip).
286:     An affected closure of [] legitimately yields ZERO legs; the workflow's
287:     `setup` job emits a `legs` count so the matrix job skips instead of
288:     exploding on an empty `include`. Note: matrix-key selection cannot be a
289:     job-level `if:` on GitHub Actions — the `matrix` context is not available
290:     there (docs: contexts availability), which is why this filtering happens at
291:     assembly time. The gate aggregator discovers checks by polling, so an
292:     unassembled leg is simply absent (never an "expected but missing" hang);
293:     requiredness continues to flow through `ci-summary / gate`.
294:     """
295:     if select_mode != "selected":
296:         return legs
297:     affected = None
298:     if affected_json:
299:         try:
300:             affected = json.loads(affected_json)
301:         except json.JSONDecodeError:
302:             affected = None
303:     if not isinstance(affected, list) or not all(isinstance(a, str) for a in affected):
304:         sys.stderr.write(
305:             "warning: --select-mode selected but --affected is missing/malformed; "
306:             "FAILING CLOSED to the full leg set (sq-fmx4u.3, design §4.3)\n"
307:         )
308:         return legs
309:     keep = set(affected)
310:     return [leg for leg in legs if leg["crate"] in keep]
311: 
312: 
313: def filter_legs_by_tier(legs, event, tier):
314:     """[SONNET-4.6] sq-ldg8c (design §3/§5): partition the leg list by tier for `event`.
315: 
316:     - TIERED event (pull_request / merge_group): return the legs whose effective
317:       tier equals the requested tier. `tier: check` legs are thus EXCLUDED from the
318:       default test-tier matrix and surface only under `--tier check` (the T1 output).
319:     - Any OTHER event (push / schedule / workflow_dispatch / unknown / absent): the
320:       FULL per-merge backstop — ALL legs run as full legs, so `--tier test` (the
321:       default) returns EVERY leg (byte-identical to today) and `--tier check` returns
322:       NONE (the check tier is empty on a full run).
323: 
324:     `tier` defaults to 'test' (the matrix output) when None. An unrecognised `--tier`
325:     value is a HARD ERROR (exit non-zero) — never a silent demotion. This filter ANDs
326:     with filter_legs_by_selection (order-independent; both narrow the set).
327:     """
328:     requested = "test" if tier is None else tier
329:     if requested not in VALID_TIERS:
330:         sys.stderr.write(
331:             f"error: --tier must be one of {list(VALID_TIERS)} (got {tier!r})\n"
332:         )
333:         sys.exit(2)
334:     if event not in TIERED_EVENTS:
335:         # FULL backstop: everything is a full leg; the check tier is empty.
336:         return list(legs) if requested == "test" else []
337:     return [leg for leg in legs if leg.get("tier", "test") == requested]
```

### scripts/assemble-feature-matrix.py:364-501

```
364: def group_legs(legs, capacity=GROUP_CAPACITY):
365:     """[FABLE-5] CI-economy grouping: deterministically bin-pack legs into groups.
366: 
367:     Same-crate legs are clustered FIRST (they share the group's warm target dir —
368:     consecutive feature-states of one crate recompile only the crate itself, never
369:     the dependency stack), then each crate's legs are chunked to the capacity and
370:     the chunks are packed first-fit-decreasing into bins. A single leg heavier
371:     than the capacity gets its own chunk (never dropped, never split).
372: 
373:     Returns a list of groups, each {"group", "cache_crate", "count", "legs"} where
374:     `legs` is the ORDERED list of leg dicts (workflow shape: name/crate/features/
375:     test). Group names/ids are NOT gate-critical — the gate-critical `opt-in
376:     <name>` per-leg check-runs are emitted by scripts/run-feature-matrix-group.py
377:     from inside the group job, name-preserved byte-for-byte."""
378:     by_crate = {}
379:     for leg in legs:
380:         by_crate.setdefault(leg["crate"], []).append(leg)
381:     # Crates ordered by total weight desc (then name for determinism).
382:     crate_order = sorted(
383:         by_crate,
384:         key=lambda c: (-sum(leg_weight(leg) for leg in by_crate[c]), c),
385:     )
386:     chunks = []  # (weight, crate, [legs]) — same-crate, each <= capacity where possible
387:     for crate in crate_order:
388:         cur, cur_w = [], 0.0
389:         for leg in by_crate[crate]:
390:             w = leg_weight(leg)
391:             if cur and cur_w + w > capacity:
392:                 chunks.append((cur_w, crate, cur))
393:                 cur, cur_w = [], 0.0
394:             cur.append(leg)
395:             cur_w += w
396:         if cur:
397:             chunks.append((cur_w, crate, cur))
398:     # First-fit-decreasing over the chunks (stable: weight desc, then crate name,
399:     # then original chunk position).
400:     bins = []  # each: {"weight": float, "chunks": [(weight, crate, legs)]}
401:     ordered_chunks = sorted(
402:         ((w, crate, i, chunk) for i, (w, crate, chunk) in enumerate(chunks)),
403:         key=lambda t: (-t[0], t[1], t[2]),
404:     )
405:     for w, crate, _i, chunk in ordered_chunks:
406:         placed = False
407:         for b in bins:
408:             if b["weight"] + w <= capacity:
409:                 b["chunks"].append((w, crate, chunk))
410:                 b["weight"] += w
411:                 placed = True
412:                 break
413:         if not placed:
414:             bins.append({"weight": w, "chunks": [(w, crate, chunk)]})
415:     groups = []
416:     for idx, b in enumerate(bins, start=1):
417:         # Dominant crate = the heaviest chunk's crate (chunks were appended in
418:         # weight-desc order, so the first chunk is the heaviest) — it keys the
419:         # group's rust-cache shared-key, reusing the existing per-crate cache
420:         # entries (sq-3sbrr strategy unchanged).
421:         dominant = b["chunks"][0][1]
422:         group_legs_flat = [leg for _w, _c, chunk in b["chunks"] for leg in chunk]
423:         groups.append(
424:             {
425:                 "group": f"g{idx:02d} {dominant}",
426:                 "cache_crate": dominant,
427:                 "count": len(group_legs_flat),
428:                 "legs": group_legs_flat,
429:                 "weight": round(b["weight"], 3),
430:             }
431:         )
432:     return groups
433: 
434: 
435: def _flag_value(argv, flag):
436:     """Value of `--flag value` in argv, or None."""
437:     for i, a in enumerate(argv):
438:         if a == flag and i + 1 < len(argv):
439:             return argv[i + 1]
440:     return None
441: 
442: 
443: def main():
444:     legs = load_legs()
445:     if "--names" in sys.argv[1:]:
446:         # The golden gate-name proof ALWAYS dumps the full set — selection must
447:         # never make the byte-identical name contract unverifiable.
448:         for name in sorted(f"opt-in {leg['name']}" for leg in legs):
449:             print(name)
450:         return
451:     argv = sys.argv[1:]
452:     # AND-composed filters (order-independent): selection narrows by affected crate,
453:     # tier partitions by event+tier, shard splits the check tier's build shards.
454:     legs = filter_legs_by_selection(
455:         legs,
456:         _flag_value(argv, "--select-mode"),
457:         _flag_value(argv, "--affected"),
458:     )
459:     legs = filter_legs_by_tier(
460:         legs,
461:         _flag_value(argv, "--event"),
462:         _flag_value(argv, "--tier"),
463:     )
464:     legs = filter_legs_by_shard(legs, _flag_value(argv, "--shard"))
465:     # Strip the internal `tier`/`weight` keys so the emitted leg shape stays exactly
466:     # {name, crate, features, test} (the workflow matrix contract). Rebuilding in
467:     # this fixed key order keeps the default output BYTE-IDENTICAL to the pre-tier
468:     # assembler (sq-ldg8c behaviour-preservation invariant).
469:     include = [
470:         {
471:             "name": leg["name"],
472:             "crate": leg["crate"],
473:             "features": leg["features"],
474:             "test": leg["test"],
475:         }
476:         for leg in legs
477:     ]
478:     if "--grouped" in argv:
479:         # [FABLE-5] CI-economy grouping: emit ONE matrix entry per bin-packed GROUP
480:         # of legs. `legs` is a JSON-encoded STRING (GitHub matrix values must be
481:         # scalars) — the group job passes it to scripts/run-feature-matrix-group.py,
482:         # which runs each leg and emits its gate-critical `opt-in <name>` check-run
483:         # (name byte-identical to the per-leg matrix this replaces). `count` totals
484:         # feed the workflow's `legs` output (skip-on-zero unchanged).
485:         groups = group_legs(legs)
486:         ginclude = []
487:         for g in groups:
488:             stripped = [
489:                 {
490:                     "name": leg["name"],
491:                     "crate": leg["crate"],
492:                     "features": leg["features"],
493:                     "test": leg["test"],
494:                 }
495:                 for leg in g["legs"]
496:             ]
497:             ginclude.append(
498:                 {
499:                     "group": g["group"],
500:                     "cache_crate": g["cache_crate"],
501:                     "count": g["count"],
```

### .github/workflows/feature-matrix.yml:356-476

```
356:   setup:
357:     needs: [changes, select]
358:     if: needs.changes.outputs.rust_changed == 'true'
359:     name: assemble feature matrix
360:     runs-on: ubuntu-latest
361:     permissions:
362:       contents: read
363:     outputs:
364:       matrix: ${{ steps.assemble.outputs.matrix }}
365:       # [FABLE-5] sq-fmx4u.3: leg COUNT, so `opt-in-features` can skip (instead of
366:       # exploding on an empty matrix) when enforced selection filters every leg out.
367:       legs: ${{ steps.assemble.outputs.legs }}
368:     steps:
369:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
370:         with:
371:           persist-credentials: false
372:       - uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
373:         with:
374:           python-version: "3.12"
375:       - name: Install PyYAML
376:         # [OPUS-4.8] HASH-pinned (Scorecard PinnedDependenciesID): reuses the same
377:         # full-closure PyYAML 6.0.2 hash set the docs-quality skill-frontmatter job pins,
378:         # so --require-hashes verifies every artifact's sha256 before use. PyYAML has no
379:         # runtime deps, so that file is the complete dependency closure.
380:         run: pip install --require-hashes -r .github/requirements/docs-quality.txt
381:       - name: Self-test the assembler + gate-critical leg-name preservation
382:         # [OPUS-4.8] sq-ibrze: run the hermetic test BEFORE assembling so a fragment edit
383:         # that would rename/drop/add a gating leg (or reintroduce a static list, or name a
384:         # leg "advisory"/"informational") fails THIS gating job — i.e. the gate-critical
385:         # invariant is enforced on every PR that touches a fragment, not just by review.
386:         run: python3 scripts/tests/test_feature_matrix_assemble.py
387:       - name: Enforce tier ratchet + sq-vya1 guard (feature-matrix-tiers.py --enforce)
388:         # [SONNET-4.6] sq-ldg8c (design §4): mechanizes the sq-vya1 GUARD as a HARD gate.
389:         # Reds THIS gating job if a leg carries `tier: check` while the detector still finds
390:         # feature-gated test code / a `cfg(not(feature))` brace (and no reviewed
391:         # `tier-reason:` override), OR if any sensitive feature has no `test: true` leg. So a
392:         # future feature-gated test added under a demoted feature re-promotes it at the next
393:         # PR — the ratchet. Fail-closed: a detector/IO error classifies a leg SENSITIVE (keep
394:         # the full leg), never a silent demotion. Stdlib + the PyYAML installed above; it
395:         # scans crates/ (checked out above). At ZERO annotations this is a no-op green.
396:         run: python3 scripts/feature-matrix-tiers.py --enforce
397:       - name: Structural guard C1 — feature-gated test execution (sq-qcnn.31)
398:         # [SONNET-4.6] sq-qcnn.31: Fail-closed guard — every feature-gated test module or
399:         # target (tests/ files with #![cfg(feature = "F")] inner attributes, OR [[test]]/
400:         # [[bench]] Cargo.toml entries with required-features) MUST map to a CI executor
401:         # (a feature-matrix leg, a coverage.sh case arm) or an explicit entry in
402:         # scripts/check-feature-test-execution.allowlist.json. A gated test with no
403:         # executor and no allowlist entry fails this gating job immediately — silently-
404:         # never-running tests are structurally impossible from the first commit they are
405:         # added. The --self-test run verifies the detector is not broken (exits 1 if the
406:         # negative fixture is unexpectedly clean, i.e. the guard would be dark). PyYAML
407:         # already installed by the step above.
408:         run: |
409:           set -euo pipefail
410:           python3 scripts/check-feature-test-execution.py --self-test
411:           python3 scripts/check-feature-test-execution.py --check
412:       - name: Assemble matrix from .github/feature-matrix.d/*.yml
413:         id: assemble
414:         # Emit the leg set as a one-line JSON object on the `matrix` output. The
415:         # assembler validates every fragment (required keys, non-empty features, no
416:         # duplicate leg names) and exits non-zero on any malformed fragment, so a bad
417:         # fragment fails THIS gating job instead of producing a silently-truncated matrix.
418:         # [FABLE-5] sq-fmx4u.3: --select-mode/--affected filter the legs to the
419:         # affected closure ONLY when the selection pre-job says mode == 'selected';
420:         # shadow/full/empty/malformed inputs fail-close to the FULL leg set inside
421:         # the assembler (unit-tested). `legs` is the post-filter count.
422:         env:
423:           SELECT_MODE: ${{ needs.select.outputs.mode }}
424:           SELECT_AFFECTED: ${{ needs.select.outputs.affected }}
425:         run: |
426:           set -euo pipefail
427:           # [FABLE-5] CI-economy: --grouped emits ONE matrix entry per bin-packed
428:           # GROUP of legs (see the opt-in-features job note); `legs` is the TOTAL
429:           # leg count across groups, so the skip-on-zero contract is unchanged.
430:           MATRIX="$(python3 scripts/assemble-feature-matrix.py --grouped --event "${{ github.event_name }}" --select-mode "${SELECT_MODE:-}" --affected "${SELECT_AFFECTED:-}")"
431:           echo "matrix=$MATRIX" >> "$GITHUB_OUTPUT"
432:           LEGS="$(echo "$MATRIX" | python3 -c 'import json,sys; m=json.load(sys.stdin)["include"]; print(sum(g["count"] for g in m))')"
433:           NGROUPS="$(echo "$MATRIX" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["include"]))')"
434:           echo "legs=$LEGS" >> "$GITHUB_OUTPUT"
435:           echo "Assembled $LEGS opt-in leg(s) into $NGROUPS group(s) (select mode: ${SELECT_MODE:-<unset>}, event ${{ github.event_name }}). Full leg-name set:"
436:           python3 scripts/assemble-feature-matrix.py --names
437:       - name: Emit the SELECTED leg set + metadata for the trusted reporter
438:         # [FABLE-5] PR #3511 review findings 1+2: the reporter runs in the separate,
439:         # default-branch-owned feature-matrix-report.yml (workflow_run) so it never
440:         # executes PR-controlled code with `checks: write`. It needs (a) the EXACT
441:         # legs this run should produce — the UNGROUPED selected set (same --event/
442:         # --select-mode/--affected filters as the grouped matrix, so byte-identical
443:         # legs) — as its completeness ground truth (finding 2), and (b) the head SHA
444:         # + originating event as metadata so it can validate the SHA against the
445:         # server-supplied workflow_run.head_sha (finding 1). This step runs PR-
446:         # controlled assembler code, so it holds NO token and NO checks:write — the
447:         # artifact is DATA the trusted reporter validates hostile-input-style.
448:         if: needs.changes.outputs.rust_changed == 'true'
449:         env:
450:           SELECT_MODE: ${{ needs.select.outputs.mode }}
451:           SELECT_AFFECTED: ${{ needs.select.outputs.affected }}
452:           HEAD_SHA: ${{ github.event.pull_request.head.sha || github.sha }}
453:           EVENT_NAME: ${{ github.event_name }}
454:         run: |
455:           set -euo pipefail
456:           mkdir -p fmg-selected fmg-metadata
457:           python3 scripts/assemble-feature-matrix.py \
458:             --event "${EVENT_NAME}" --select-mode "${SELECT_MODE:-}" \
459:             --affected "${SELECT_AFFECTED:-}" > fmg-selected/selected-matrix.json
460:           python3 -c 'import json,os; json.dump({"head_sha": os.environ["HEAD_SHA"], "event": os.environ["EVENT_NAME"]}, open("fmg-metadata/metadata.json","w"))'
461:       - name: Upload the selected-matrix artifact (reporter completeness ground truth)
462:         if: needs.changes.outputs.rust_changed == 'true' && steps.assemble.outputs.legs != '0'
463:         uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
464:         with:
465:           name: feature-matrix-selected
466:           path: fmg-selected/selected-matrix.json
467:           retention-days: 1
468:           if-no-files-found: error
469:       - name: Upload the run metadata artifact (head SHA + event for the reporter)
470:         if: needs.changes.outputs.rust_changed == 'true' && steps.assemble.outputs.legs != '0'
471:         uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
472:         with:
473:           name: feature-matrix-metadata
474:           path: fmg-metadata/metadata.json
475:           retention-days: 1
476:           if-no-files-found: error
```

### .github/workflows/feature-matrix.yml:700-739

```
700:     # filtered EVERY leg out (an empty `include` would otherwise be a matrix
701:     # error, i.e. a false RED). Fail-closed: a missing/empty `legs` output is
702:     # != '0' => run; and if `setup` itself failed, this job skips via `needs`
703:     # while the gate REDs on setup's failure.
704:     if: needs.changes.outputs.rust_changed == 'true' && needs.setup.outputs.legs != '0'
705:     # [FABLE-5] CI-economy GROUPED LEGS (maintainer directive 2026-07-18). The matrix
706:     # is ONE ENTRY PER BIN-PACKED GROUP of legs (assembler --grouped; same-crate legs
707:     # clustered, each group targeting <5 min of warm-cache work) instead of one runner
708:     # per leg — 159 legs used to mean 159 runners each paying checkout + toolchain +
709:     # cache-restore + the dependency compile.
710:     #
711:     # NAME CONTRACT PRESERVED: the gate-critical `opt-in <name>` check-run for EVERY
712:     # leg is still emitted, byte-identical (proven against the golden snapshot by
713:     # scripts/tests/test_feature_matrix_assemble.py) — but NOT from this job. THIS
714:     # JOB IS DELIBERATELY UNPRIVILEGED (PR #3511 review, critical finding): it
715:     # executes PR-controlled code — cargo build scripts, proc macros, tests, clippy
716:     # lints, plus the whole third-party dependency graph's build-time code — so it
717:     # must NEVER hold a `checks: write` token (malicious build code could forge
718:     # arbitrary check runs, incl. gate-critical sibling names) nor persisted git
719:     # credentials. It runs scripts/run-feature-matrix-group.py with NO token in the
720:     # environment and writes each leg's outcome to a results file, uploaded below as
721:     # an artifact for the separate, DEFAULT-BRANCH-owned reporter workflow
722:     # (feature-matrix-report.yml, workflow_run — see finding 1 in this file's header),
723:     # which executes no PR build code, validates the artifact hostile-input-style, and
724:     # posts the per-leg check-runs. A failed leg does not stop the group (the old fail-fast: false),
725:     # and the group job then FAILS LOUDLY naming exactly which legs failed — this
726:     # job's own conclusion is a required gating signal via `ci-summary / gate`
727:     # regardless of any check-run posting (sq-fmx4u.4).
728:     #
729:     # [OPUS-4.8] sq-ibrze (unchanged): legs are ASSEMBLED from the per-crate fragment
730:     # files (`.github/feature-matrix.d/<crate>.yml`). To add a leg, edit the relevant
731:     # crate fragment — do NOT reintroduce a static `include:` list here.
732:     # [SONNET-4.6] issue #2384: that is a TWO-FILE change, not a one-file change. A
733:     # leg's `name:` is pinned byte-for-byte by the gate-name golden, so adding /
734:     # renaming / removing a leg ALSO requires regenerating
735:     # `scripts/tests/feature-matrix-legnames.golden.txt`. Scope both files (never one)
736:     # — see `.github/feature-matrix.d/README.md`.
737:     name: opt-in group (${{ matrix.group }})
738:     runs-on: ubuntu-latest
739:     permissions:
```

### .github/workflows/feature-matrix.yml:789-825

```
789:       - name: Run the group's legs (build -> test -> clippy; results to artifact)
790:         # Each leg: build (bounded sq-hhxc retry) -> test (where the fragment says
791:         # so) -> clippy --all-targets -D warnings — the exact step set of the old
792:         # per-leg job. NO token here (see the job note): outcomes go to the
793:         # results file; the trusted default-branch reporter workflow posts the
794:         # check-runs (feature-matrix-report.yml, workflow_run — finding 1).
795:         env:
796:           GROUP_LEGS: ${{ matrix.legs }}
797:           GROUP_NAME: ${{ matrix.group }}
798:           FMG_RESULTS: fmg-results/results.json
799:         run: python3 scripts/run-feature-matrix-group.py
800:       - name: Upload per-leg results for the trusted reporter
801:         # `!cancelled()`: a group with FAILED legs must still ship its results so
802:         # the reporter posts the red per-leg check-runs. `overwrite: true` keeps
803:         # re-runs of failed jobs working (v4+ artifacts are otherwise immutable
804:         # within a run). The artifact name is unique per matrix entry.
805:         if: ${{ !cancelled() }}
806:         uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
807:         with:
808:           name: feature-matrix-results-${{ strategy.job-index }}
809:           path: fmg-results/results.json
810:           retention-days: 1
811:           if-no-files-found: error
812:           overwrite: true
813: 
814:   # [FABLE-5] PR #3511 review, CRITICAL finding 1: the per-leg `opt-in <name>`
815:   # check-run reporter is NO LONGER a job in this PR-triggered workflow. It moved to
816:   # the separate, DEFAULT-BRANCH-owned .github/workflows/feature-matrix-report.yml,
817:   # triggered by `workflow_run` on this workflow's completion. Rationale: the reporter
818:   # is the only holder of `checks: write`, and a `checks: write` job that checks out
819:   # the PR head and runs the PR's copy of scripts/{assemble-feature-matrix,run-feature-
820:   # matrix-group}.py is the pwn-requests anti-pattern — a PR could edit either script
821:   # to forge `gate` or sibling check-runs. `workflow_run` always runs the default-
822:   # branch copy of the reporter workflow AND those scripts, isolated from PR-head
823:   # content; it downloads THIS run's artifacts (results + the selected-matrix +
824:   # metadata this workflow uploads), treats them strictly as validated DATA, validates
825:   # the recorded head SHA against the server-supplied workflow_run.head_sha, and posts
```

### .github/workflows/feature-matrix-report.yml:74-214

```
74: name: feature-matrix-report
75: 
76: on:
77:   # Fire after feature-matrix completes. `workflow_run.workflows` keys on the
78:   # workflow NAME (`feature-matrix`). GitHub always executes the default-branch copy
79:   # of this file — the whole point of the finding-1 fix.
80:   workflow_run:
81:     workflows: ["feature-matrix"]
82:     types: [completed]
83: 
84: # Read-only default; the reporter job opts in to the ONE write it needs.
85: permissions:
86:   contents: read
87: 
88: # One in-flight reporter per triggering feature-matrix run.
89: concurrency:
90:   group: feature-matrix-report-${{ github.event.workflow_run.id }}
91:   cancel-in-progress: false
92: 
93: jobs:
94:   report:
95:     name: feature-matrix per-leg report
96:     # Only report for a feature-matrix run that actually produced legs. workflow_run
97:     # fires for fork/cross-repo runs too; those are handled inside (FMG_FORK_PR event
98:     # gating) — we do NOT gate the job off head_repository here because a fork PR's
99:     # legs still deserve their (possibly fork-denied) attribution attempt, and the
100:     # server-supplied head_sha validation + event gating make it safe.
101:     if: >-
102:       github.event.workflow_run.conclusion != 'skipped' &&
103:       github.event.workflow_run.conclusion != 'cancelled'
104:     runs-on: ubuntu-latest
105:     timeout-minutes: 15
106:     permissions:
107:       contents: read
108:       # Cross-run artifact download (actions/download-artifact with run-id) needs
109:       # actions: read — unspecified permissions default to none and the download
110:       # would fail, deadlocking the structural reporter await (sol review on #3522).
111:       actions: read
112:       # The single privileged capability: emit the per-leg + summary check-runs.
113:       checks: write
114:     steps:
115:       # Checkout is the DEFAULT-BRANCH copy (workflow_run runs on the default branch),
116:       # so scripts/run-feature-matrix-group.py + scripts/assemble-feature-matrix.py are
117:       # the reviewed, committed versions — never the PR head's. persist-credentials:
118:       # false is belt-and-braces (the script authenticates via GH_TOKEN in env).
119:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
120:         with:
121:           persist-credentials: false
122:       - uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
123:         with:
124:           python-version: "3.12"
125:       # Two SEPARATE downloads so FMG_RESULTS_DIR contains ONLY the per-leg results
126:       # files — the metadata/selected artifacts must never be scanned as results.
127:       # run-id + github-token pulls artifacts from the SPECIFIC feature-matrix run
128:       # that triggered this reporter (workflow_run artifacts are otherwise not in
129:       # this run's own scope). A missing pattern is not an error (v4+), so a run that
130:       # assembled zero legs simply yields empty dirs — handled below.
131:       - name: Download the per-leg results artifacts (by run-id)
132:         uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
133:         with:
134:           pattern: feature-matrix-results-*
135:           path: fmg-results
136:           run-id: ${{ github.event.workflow_run.id }}
137:           github-token: ${{ github.token }}
138:       - name: Download the selected-matrix + metadata artifacts (by run-id)
139:         uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
140:         with:
141:           pattern: feature-matrix-{selected,metadata}
142:           path: fmg-meta
143:           run-id: ${{ github.event.workflow_run.id }}
144:           github-token: ${{ github.token }}
145:       - name: Validate head SHA against the server-supplied trigger, then report
146:         env:
147:           # ONLY the per-leg results live here (the reporter scans this recursively).
148:           FMG_RESULTS_DIR: fmg-results
149:           # The metadata + selected-matrix artifacts (never scanned as results).
150:           FMG_META_DIR: fmg-meta
151:           # SERVER-supplied, unforgeable — the ground truth the artifact's recorded
152:           # head SHA must match. Passed through env (never interpolated into a shell).
153:           TRIGGER_HEAD_SHA: ${{ github.event.workflow_run.head_sha }}
154:           TRIGGER_EVENT: ${{ github.event.workflow_run.event }}
155:           TRIGGER_HEAD_REPO: ${{ github.event.workflow_run.head_repository.full_name }}
156:           REPO: ${{ github.repository }}
157:           GROUP_JOBS_RESULT: ${{ github.event.workflow_run.conclusion }}
158:           GH_TOKEN: ${{ github.token }}
159:           DETAILS_URL: ${{ github.event.workflow_run.html_url }}
160:           # CORRELATION (PR #3511 review finding 2 — same-SHA stale-report race):
161:           # the TRIGGERING feature-matrix run id. Server-supplied + unforgeable; the
162:           # reporter embeds it as the `feature-matrix report` check's external_id so
163:           # ci_summary_gate.py binds the verdict to THIS group run and ignores a stale
164:           # same-SHA report from an earlier feature-matrix run (feature-matrix reruns
165:           # on ready_for_review / label events against the same head SHA).
166:           TRIGGER_RUN_ID: ${{ github.event.workflow_run.id }}
167:         run: |
168:           set -euo pipefail
169: 
170:           META="${FMG_META_DIR}/feature-matrix-metadata/metadata.json"
171:           SELECTED="${FMG_META_DIR}/feature-matrix-selected/selected-matrix.json"
172:           if [ ! -f "${META}" ] || [ ! -f "${SELECTED}" ]; then
173:             # NO-OP (green, no check posted). Two legitimate cases produce no grouped
174:             # metadata/selected artifacts:
175:             #  (1) BOOTSTRAP WINDOW: this reporter is on `main` AHEAD of the grouped
176:             #      feature-matrix (PR #3511). main's current feature-matrix uses the
177:             #      pre-grouping per-leg format and uploads NO metadata/selected
178:             #      artifacts, so every triggered reporter run lands here and exits
179:             #      cleanly WITHOUT posting a (misleading) success check-run. Once
180:             #      #3511's grouped format merges, real artifacts appear and the reporter
181:             #      begins posting. It NEVER posts for a run it cannot fully attribute.
182:             #  (2) ZERO-LEG runs (path-filter skip / empty selection) upload no metadata
183:             #      either; there is likewise nothing to gate.
184:             # A `::notice::` (not `::error::`): this is an expected no-op, not a failure,
185:             # so the reporter job stays green with no spurious error annotation.
186:             echo "::notice::no grouped feature-matrix metadata/selected artifacts on this run (bootstrap window before PR #3511's grouped format lands, or a zero-leg run) — nothing to report; posting no check-run."
187:             exit 0
188:           fi
189: 
190:           # --- HEAD-SHA TRUST GATE (finding 1) -------------------------------------
191:           # The artifact's recorded head SHA is attacker-influenced; the SERVER value
192:           # is not. Parse the SHA out of the JSON with python (as DATA — the artifact
193:           # string is never interpolated into a shell command).
194:           ART_HEAD_SHA="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["head_sha"])' "${META}")"
195:           if [ "${ART_HEAD_SHA}" != "${TRIGGER_HEAD_SHA}" ]; then
196:             echo "::error::artifact head SHA (${ART_HEAD_SHA}) does not match the server-supplied workflow_run.head_sha (${TRIGGER_HEAD_SHA}) — refusing to post check-runs on a possibly-spoofed commit."
197:             exit 1
198:           fi
199: 
200:           # --- FORK-PR event gate (finding 3) --------------------------------------
201:           # Tolerate a check-run POST permission denial ONLY on a fork PR — decided
202:           # from SERVER fields, not the API error string.
203:           FORK_PR=false
204:           if [ "${TRIGGER_EVENT}" = "pull_request" ] && [ "${TRIGGER_HEAD_REPO}" != "${REPO}" ]; then
205:             FORK_PR=true
206:           fi
207: 
208:           # --- Report (the reporter engine treats artifacts strictly as data) ------
209:           # FMG_RESULTS_DIR / GROUP_JOBS_RESULT / REPO / DETAILS_URL / GH_TOKEN come
210:           # from the step `env:`. The remaining three are the validated values.
211:           FMG_VALID_LEGS_FILE="${SELECTED}" \
212:           HEAD_SHA="${TRIGGER_HEAD_SHA}" \
213:           FMG_FORK_PR="${FORK_PR}" \
214:           python3 scripts/run-feature-matrix-group.py --report
```

### scripts/run-feature-matrix-group.py:179-277

```
179: def run_leg(leg):
180:     """Build -> (test) -> clippy, exactly the per-leg matrix step set.
181:     Returns None on success, else the name of the failing step."""
182:     crate, features = leg["crate"], leg["features"]
183:     if build_with_retry(crate, features) != 0:
184:         return "build"
185:     if leg["test"]:
186:         if run([CARGO, "test", "-p", crate, "--features", features]) != 0:
187:             return "test"
188:     if (
189:         run(
190:             [
191:                 CARGO,
192:                 "clippy",
193:                 "-p",
194:                 crate,
195:                 "--features",
196:                 features,
197:                 "--all-targets",
198:                 "--",
199:                 "-D",
200:                 "warnings",
201:             ]
202:         )
203:         != 0
204:     ):
205:         return "clippy"
206:     return None
207: 
208: 
209: def write_results(path, group, results):
210:     """Persist the per-leg outcomes for the trusted reporter job. Rewritten after
211:     every leg so an infra death mid-group still ships the completed legs."""
212:     payload = {
213:         "schema": RESULTS_SCHEMA,
214:         "group": group,
215:         "results": [
216:             {"name": leg["name"], "failed_step": failed_step}
217:             for leg, failed_step in results
218:         ],
219:     }
220:     tmp = f"{path}.tmp"
221:     with open(tmp, "w", encoding="utf-8") as fh:
222:         json.dump(payload, fh, ensure_ascii=False)
223:     os.replace(tmp, path)
224: 
225: 
226: def main():
227:     try:
228:         legs = json.loads(os.environ["GROUP_LEGS"])
229:     except (KeyError, json.JSONDecodeError) as exc:
230:         print(f"::error::GROUP_LEGS missing/malformed: {exc}", file=sys.stderr)
231:         return 2
232:     if not isinstance(legs, list) or not legs:
233:         print("::error::GROUP_LEGS must be a non-empty JSON array", file=sys.stderr)
234:         return 2
235:     results_path = os.environ.get("FMG_RESULTS", "")
236:     if not results_path:
237:         # The reporter job's per-leg check-runs depend on this file — a group job
238:         # running without it would silently drop every leg name from the gate's
239:         # discovery set, so refuse to run at all.
240:         print("::error::FMG_RESULTS unset — nowhere to write the per-leg results the reporter job posts from", file=sys.stderr)
241:         return 2
242:     os.makedirs(os.path.dirname(results_path) or ".", exist_ok=True)
243:     group = os.environ.get("GROUP_NAME", "?")
244:     print(f"feature-matrix group {group}: {len(legs)} leg(s)", flush=True)
245:     results = []
246:     for leg in legs:
247:         print(f"::group::opt-in {leg['name']}", flush=True)
248:         failed_step = run_leg(leg)
249:         print("::endgroup::", flush=True)
250:         results.append((leg, failed_step))
251:         write_results(results_path, group, results)
252:         if failed_step is not None:
253:             # Keep going: every leg reports (the old matrix was fail-fast: false).
254:             print(
255:                 f"::error::opt-in {leg['name']}: FAILED at {failed_step} "
256:                 "(continuing with the group's remaining legs)",
257:                 flush=True,
258:             )
259:     summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "")
260:     if summary_path:
261:         with open(summary_path, "a", encoding="utf-8") as fh:
262:             fh.write(f"## feature-matrix group `{group}`\n\n")
263:             fh.write("| leg | result |\n|---|---|\n")
264:             for leg, failed_step in results:
265:                 verdict = "✅ pass" if failed_step is None else f"❌ **{failed_step}**"
266:                 fh.write(f"| `opt-in {leg['name']}` | {verdict} |\n")
267:     failed = [leg["name"] for leg, failed_step in results if failed_step is not None]
268:     if failed:
269:         print(
270:             f"::error title=feature-matrix group {group} FAILED::"
271:             f"{len(failed)} of {len(results)} leg(s) failed: "
272:             + "; ".join(f"opt-in {n}" for n in failed),
273:             flush=True,
274:         )
275:         return 1
276:     print(f"feature-matrix group {group}: all {len(results)} leg(s) green", flush=True)
277:     return 0
```

### scripts/run-feature-matrix-group.py:289-368

```
289: def load_valid_legs(path):
290:     """The assembled leg set (committed fragments via the committed assembler) —
291:     the ONLY names/metadata the reporter will ever put in a check-run. Returns
292:     {leg_name: leg_dict} or None on any malformation (reporter-side config bug)."""
293:     try:
294:         with open(path, "r", encoding="utf-8") as fh:
295:             obj = json.load(fh)
296:     except (OSError, json.JSONDecodeError) as exc:
297:         _report_error(f"cannot read valid-legs file {path!r}: {exc}")
298:         return None
299:     include = obj.get("include") if isinstance(obj, dict) else None
300:     if not isinstance(include, list) or not include:
301:         _report_error(f"valid-legs file {path!r} is not an assembler include-object")
302:         return None
303:     valid = {}
304:     for leg in include:
305:         if (
306:             not isinstance(leg, dict)
307:             or not isinstance(leg.get("name"), str)
308:             or not isinstance(leg.get("crate"), str)
309:             or not isinstance(leg.get("features"), str)
310:         ):
311:             _report_error(f"valid-legs file {path!r} carries a malformed leg: {leg!r}")
312:             return None
313:         valid[leg["name"]] = leg
314:     return valid
315: 
316: 
317: def validate_results_file(path, valid_legs, seen_names):
318:     """HOSTILE-INPUT validation of one group-results artifact file. The file was
319:     produced inside a job that ran arbitrary PR-controlled build code, so every
320:     byte is attacker-controlled: only a name that resolves in the assembled leg
321:     set, a failed_step from the fixed enum, and a conservatively-shaped group id
322:     are accepted. Returns [(leg_name, group, failed_step)] or None on ANY
323:     violation (the reporter then fails closed)."""
324:     try:
325:         with open(path, "r", encoding="utf-8") as fh:
326:             obj = json.load(fh)
327:     except (OSError, json.JSONDecodeError, UnicodeDecodeError) as exc:
328:         _report_error(f"results file {path!r}: unreadable/not JSON ({exc})")
329:         return None
330:     if not isinstance(obj, dict) or set(obj.keys()) != {"schema", "group", "results"}:
331:         _report_error(f"results file {path!r}: top level must be exactly {{schema, group, results}}")
332:         return None
333:     if obj["schema"] != RESULTS_SCHEMA:
334:         _report_error(f"results file {path!r}: unknown schema {obj['schema']!r} (want {RESULTS_SCHEMA!r})")
335:         return None
336:     group = obj["group"]
337:     if not isinstance(group, str) or not GROUP_NAME_RE.match(group):
338:         _report_error(f"results file {path!r}: group id fails the conservative shape check")
339:         return None
340:     if not isinstance(obj["results"], list) or not obj["results"]:
341:         _report_error(f"results file {path!r}: results must be a non-empty list")
342:         return None
343:     out = []
344:     for entry in obj["results"]:
345:         if not isinstance(entry, dict) or set(entry.keys()) != {"name", "failed_step"}:
346:             _report_error(f"results file {path!r}: entry must be exactly {{name, failed_step}}: {entry!r}")
347:             return None
348:         name, failed_step = entry["name"], entry["failed_step"]
349:         if not isinstance(name, str) or name not in valid_legs:
350:             # PR-supplied free text stops HERE: an unassembled leg name is never
351:             # allowed to become a check-run.
352:             _report_error(f"results file {path!r}: leg name {name!r} is not in the assembled leg set")
353:             return None
354:         if failed_step is not None and failed_step not in VALID_STEPS:
355:             _report_error(f"results file {path!r}: failed_step {failed_step!r} not in {VALID_STEPS}")
356:             return None
357:         if name in seen_names:
358:             # Duplicate sibling check-runs are exactly the latest-run-confusion
359:             # attack the trusted split exists to prevent — refuse them from the
360:             # honest side too.
361:             _report_error(f"leg {name!r} appears in multiple results files ({seen_names[name]} and {path!r})")
362:             return None
363:         seen_names[name] = path
364:         out.append((name, group, failed_step))
365:     return out
366: 
367: 
368: def post_check_run(repo, head_sha, name, group, leg, failed_step, details_url, fork_pr):
```

### scripts/run-feature-matrix-group.py:368-472

```
368: def post_check_run(repo, head_sha, name, group, leg, failed_step, details_url, fork_pr):
369:     """POST one terminal `opt-in <name>` check-run. Returns 'ok', 'fork-denied'
370:     (the EXPECTED read-only-token degradation — accepted ONLY when fork_pr is True)
371:     or 'failed' (unexpected — the caller fails the reporter job: fail CLOSED)."""
372:     check_name = f"opt-in {name}"
373:     if failed_step is None:
374:         conclusion, title = "success", "leg passed"
375:         summary = (
376:             f"build/test/clippy green for `-p {leg['crate']} --features "
377:             f"{leg['features']}` (grouped leg — ran inside feature-matrix group "
378:             f"`{group}`; posted by the trusted reporter job)."
379:         )
380:     else:
381:         conclusion, title = "failure", f"leg FAILED at {failed_step}"
382:         summary = (
383:             f"`cargo {failed_step}` failed for `-p {leg['crate']} --features "
384:             f"{leg['features']}`. See the feature-matrix group job `{group}` log "
385:             f"for the full output."
386:         )
387:     payload = {
388:         "name": check_name,
389:         "head_sha": head_sha,
390:         "status": "completed",
391:         "conclusion": conclusion,
392:         "output": {"title": title, "summary": summary},
393:     }
394:     if details_url:
395:         payload["details_url"] = details_url
396:     retry_sleep = float(os.environ.get("FMG_RETRY_SLEEP", "5"))
397:     err = ""
398:     for attempt in range(1, POST_ATTEMPTS + 1):
399:         proc = subprocess.run(
400:             [GH, "api", f"repos/{repo}/check-runs", "--method", "POST", "--input", "-"],
401:             input=json.dumps(payload).encode("utf-8"),
402:             stdout=subprocess.DEVNULL,
403:             stderr=subprocess.PIPE,
404:         )
405:         if proc.returncode == 0:
406:             return "ok"
407:         err = proc.stderr.decode("utf-8", "replace").strip()
408:         if FORK_DENIAL_MARKER in err and fork_pr:
409:             # EVENT-GATED (finding 3): tolerated ONLY on a fork-PR workflow_run,
410:             # where the base repo genuinely may not annotate the fork's head. The
411:             # group job's own conclusion remains the (coarser) gating signal. On a
412:             # push / merge_group / same-repo PR the SAME error falls through to the
413:             # fail-closed path below (a real auth regression, never a benign denial).
414:             print(
415:                 f"::warning::could not create check-run {check_name!r}: read-only "
416:                 "token (fork PR) — the group job's own conclusion remains the "
417:                 "gating signal",
418:                 flush=True,
419:             )
420:             return "fork-denied"
421:         if attempt < POST_ATTEMPTS:
422:             print(
423:                 f"check-run POST for {check_name!r} failed (attempt {attempt}/"
424:                 f"{POST_ATTEMPTS}): {err.splitlines()[-1] if err else 'unknown error'}; retrying...",
425:                 file=sys.stderr,
426:                 flush=True,
427:             )
428:             time.sleep(retry_sleep)
429:     reason = (
430:         "This is not a fork-PR workflow_run (FMG_FORK_PR!=true), so a permission "
431:         "denial here is a real auth regression"
432:         if (FORK_DENIAL_MARKER in err and not fork_pr)
433:         else "This is not the fork-PR read-only degradation"
434:     )
435:     _report_error(
436:         f"check-run POST for {check_name!r} failed after {POST_ATTEMPTS} attempts "
437:         f"with an UNEXPECTED error ({err.splitlines()[-1] if err else 'unknown error'}). "
438:         f"{reason} — failing CLOSED so the preserved per-leg check contract never "
439:         "silently evaporates."
440:     )
441:     return "failed"
442: 
443: 
444: def post_summary_check(repo, head_sha, conclusion, title, summary, details_url, fork_pr, trigger_run_id=""):
445:     """POST the head-SHA `feature-matrix report` summary check-run so ci-summary can
446:     DISCOVER the reporter's verdict on the PR head (the reporter runs on a
447:     workflow_run event, off the head commit). Returns 'ok' / 'fork-denied' /
448:     'failed' with the same event-gated fork tolerance as post_check_run.
449: 
450:     CORRELATION (PR #3511 review finding 2): the check's `external_id` is set to the
451:     TRIGGERING feature-matrix run id. That id comes from github.event.workflow_run.id
452:     (server-supplied, unforgeable — not any PR-influenced artifact) and is the SAME id
453:     the current `opt-in group (…)` check-runs carry in their details_url. The gate
454:     correlates the two so a STALE same-SHA report (from an earlier feature-matrix run —
455:     feature-matrix reruns on ready_for_review / labels against the same head) can never
456:     satisfy the reporter-await for a FRESH group run."""
457:     payload = {
458:         "name": REPORT_SUMMARY_NAME,
459:         "head_sha": head_sha,
460:         "status": "completed",
461:         "conclusion": conclusion,
462:         "output": {"title": title, "summary": summary},
463:     }
464:     if trigger_run_id:
465:         # The gate parses this back out and requires it to equal the latest
466:         # `opt-in group (…)` run id (extracted from those checks' details_url).
467:         payload["external_id"] = str(trigger_run_id)
468:     if details_url:
469:         payload["details_url"] = details_url
470:     retry_sleep = float(os.environ.get("FMG_RETRY_SLEEP", "5"))
471:     err = ""
472:     for attempt in range(1, POST_ATTEMPTS + 1):
```

### scripts/run-feature-matrix-group.py:515-619

```
515: def report_main():
516:     results_dir = os.environ.get("FMG_RESULTS_DIR", "")
517:     valid_legs_file = os.environ.get("FMG_VALID_LEGS_FILE", "")
518:     repo = os.environ.get("REPO", "")
519:     head_sha = os.environ.get("HEAD_SHA", "")
520:     if not results_dir or not valid_legs_file or not repo or not head_sha:
521:         _report_error("--report needs FMG_RESULTS_DIR, FMG_VALID_LEGS_FILE, REPO and HEAD_SHA")
522:         return 2
523:     details_url = os.environ.get("DETAILS_URL", "")
524:     # CORRELATION (finding 2): the TRIGGERING feature-matrix run id, embedded as the
525:     # summary check-run's external_id so the gate can bind the verdict to THIS group
526:     # run and reject a stale same-SHA report from an earlier feature-matrix run.
527:     trigger_run_id = os.environ.get("TRIGGER_RUN_ID", "").strip()
528:     # EVENT-GATED fork tolerance (finding 3): the reporter workflow sets FMG_FORK_PR
529:     # from server-supplied fields; only then is a permission-denied POST benign.
530:     fork_pr = os.environ.get("FMG_FORK_PR", "").lower() == "true"
531:     # The SELECTED leg set = the exact ground truth this run should have produced
532:     # (finding 2). It is the setup job's selected-matrix artifact, NOT a default-
533:     # branch re-assemble (which would not know a leg the PR adds).
534:     valid_legs = load_valid_legs(valid_legs_file)
535:     if valid_legs is None:
536:         return 2
537:     files = sorted(glob.glob(os.path.join(results_dir, "**", "*.json"), recursive=True))
538:     if not files:
539:         if os.environ.get("GROUP_JOBS_RESULT", "") == "success":
540:             msg = (
541:                 "the group jobs report success but produced ZERO results artifacts — "
542:                 "an inconsistency that would silently drop every per-leg check-run"
543:             )
544:             _report_error(msg)
545:             return _emit_summary_and_return(
546:                 repo, head_sha, details_url, fork_pr, False,
547:                 "no results despite green groups", msg, 1, trigger_run_id)
548:         print(
549:             "::warning::no group results artifacts found (group jobs did not "
550:             "succeed) — nothing to report; the failed/skipped group jobs gate via "
551:             "ci-summary",
552:             flush=True,
553:         )
554:         # The group jobs' own (non-success) conclusions gate; record a neutral-ish
555:         # summary so the reporter's verdict is visible but non-blocking here.
556:         return _emit_summary_and_return(
557:             repo, head_sha, details_url, fork_pr, True,
558:             "no results to report",
559:             "the group jobs did not succeed, so there are no per-leg results to "
560:             "post; the failed/skipped group jobs gate via ci-summary.", 0, trigger_run_id)
561:     seen_names = {}
562:     all_results = []
563:     for path in files:
564:         validated = validate_results_file(path, valid_legs, seen_names)
565:         if validated is None:
566:             return _emit_summary_and_return(
567:                 repo, head_sha, details_url, fork_pr, False,
568:                 "malformed group results artifact",
569:                 f"a group results artifact under {results_dir} failed hostile-input "
570:                 "validation (schema / leg-name / duplicate). See the reporter log.", 1,
571:                 trigger_run_id)
572:         all_results.extend(validated)
573: 
574:     # COMPLETENESS (finding 2): the observed leg-name set must equal the SELECTED
575:     # set EXACTLY — no subset (a lost/hidden leg), no superset (already refused by
576:     # validate_results_file, which rejects names not in valid_legs), no duplicates
577:     # (already refused via seen_names). Here we catch the remaining case: a leg the
578:     # selected set requires that NO artifact reported. A malicious group job that
579:     # exited 0 while omitting a failed leg is caught here even though its own
580:     # conclusion lied.
581:     observed = {name for name, _group, _step in all_results}
582:     expected = set(valid_legs.keys())
583:     missing = sorted(expected - observed)
584:     if missing:
585:         msg = (
586:             f"artifact completeness violation: {len(missing)} selected leg(s) have "
587:             f"no result and were dropped from the per-leg checks: "
588:             + "; ".join(f"opt-in {n}" for n in missing[:20])
589:             + (" …" if len(missing) > 20 else "")
590:         )
591:         _report_error(msg)
592:         # Still post the legs we DID observe (their check-runs are honest), then
593:         # fail closed on the incompleteness via the summary check.
594:         _post_all(repo, head_sha, all_results, valid_legs, details_url, fork_pr)
595:         return _emit_summary_and_return(
596:             repo, head_sha, details_url, fork_pr, False,
597:             "incomplete per-leg results", msg, 1, trigger_run_id)
598: 
599:     fork_denied, failed, posted = _post_all(
600:         repo, head_sha, all_results, valid_legs, details_url, fork_pr
601:     )
602:     print(
603:         f"reporter: {posted} check-run(s) posted, {fork_denied} fork-denied, "
604:         f"{failed} failed, across {len(files)} group artifact(s)",
605:         flush=True,
606:     )
607:     if failed:
608:         msg = f"{failed} per-leg check-run POST(s) failed unexpectedly — failing closed"
609:         _report_error(msg)
610:         return _emit_summary_and_return(
611:             repo, head_sha, details_url, fork_pr, False,
612:             "per-leg check-run POST failed", msg, 1, trigger_run_id)
613:     return _emit_summary_and_return(
614:         repo, head_sha, details_url, fork_pr, True,
615:         "all per-leg checks posted",
616:         f"posted {posted} per-leg `opt-in <name>` check-run(s) matching the selected "
617:         f"leg set exactly ({fork_denied} fork-denied). The group jobs' own "
618:         "conclusions remain the coarse gating signal.", 0, trigger_run_id)
619: 
```

### scripts/ci_summary_gate.py:1213-1235

```
1213: def is_declared_advisory(name: str) -> bool:
1214:     """[OPUS-5] #3773 — is this check-run EXPLICITLY DECLARED non-gating?
1215: 
1216:     Matched WHOLE-NAME against the installed `.github/advisory-registry.json` keys
1217:     (case-insensitive; `${{ … }}` in a key matches its runtime expansion). No
1218:     substring reach, no word search: an undeclared check GATES no matter what it is
1219:     called, and a declared job that gets RENAMED stops matching — i.e. it GATES,
1220:     fail-closed, while C4 in scripts/check-advisory-registry.py REDs on the drift."""
1221:     candidate = (name or "").strip()
1222:     return any(m.fullmatch(candidate) for m in _DECLARED_ADVISORY)
1223: 
1224: 
1225: def is_advisory(name: str) -> bool:
1226:     """[OPUS-5] #3773 — the SINGLE non-gating classifier: DECLARED in the advisory
1227:     registry, or on the exact platform-managed allow-list. NOT a name rule — the
1228:     display-name regex this used to consult was the #3773 correctness hole.
1229: 
1230:     Every consumer inherits the same answer — the verdict (render_verdict),
1231:     fail-fast (failfast_failures) AND the resolver's run-level synthetic check
1232:     (advisory_only_failure). Splitting them would leave the Dependabot workflow's red
1233:     run to acquire a synthetic gating verdict and red the gate anyway."""
1234:     return is_declared_advisory(name) or is_platform_managed_advisory(name)
1235: 
```

### scripts/ci_summary_gate.py:1285-1466

```
1285: def is_fm_group(name: str) -> bool:
1286:     """[FABLE-5] PR #3511 finding 1: is this an `opt-in group (…)` check-run? These
1287:     are the feature-matrix GROUP jobs — they run on the PR/merge_group event only
1288:     when the setup job assembled >=1 leg (rust_changed && legs != '0') and post
1289:     their own conclusions directly on the head SHA (no privileged token), so their
1290:     PRESENCE is the robust, reporter-timing-independent proof that legs were
1291:     selected and the trusted `feature-matrix report` reporter must post its
1292:     verdict for this head.
1293: 
1294:     ZERO-LEG SKELETON (production incident, PR #3524 2026-07-19): when the setup
1295:     job selects ZERO legs the skipped matrix job still posts ONE skeleton
1296:     check-run named with the UNEXPANDED placeholder — literally
1297:     `opt-in group (${{ matrix.group }})`, conclusion=skipped. That is proof legs
1298:     did NOT run; counting it as group presence made every docs/config-only PR
1299:     await a reporter verdict the reporter correctly never posts (zero-leg no-op)
1300:     and time out RED after the full gate budget.
1301: 
1302:     SECURITY (sol on #3525): the exclusion must NOT be name-only — matrix.group
1303:     names come from the PR-controlled assembler, so a real successful group named
1304:     to contain `${{` could masquerade as the skeleton and drop the reporter
1305:     requirement (fail-open). The skeleton is identified by BOTH marks: the
1306:     unexpanded placeholder in the name AND conclusion == skipped (server-set).
1307:     is_real_fm_group(run) implements that; this name-only helper is for callers
1308:     that have no conclusion in hand (run-id extraction, where a forged name only
1309:     ADDS a candidate id — never removes the requirement)."""
1310:     return name.startswith(FM_GROUP_PREFIX)
1311: 
1312: 
1313: def is_real_fm_group(run: dict) -> bool:
1314:     """A group check that PROVES legs ran: group-prefixed name, excluding only the
1315:     zero-leg skeleton (unexpanded `${{` placeholder AND server-set skipped)."""
1316:     name = run.get("name", "")
1317:     if not name.startswith(FM_GROUP_PREFIX):
1318:         return False
1319:     if "${{" in name and (run.get("conclusion") or "") == "skipped":
1320:         return False
1321:     return True
1322: 
1323: 
1324: def is_fm_report(name: str) -> bool:
1325:     """[FABLE-5] PR #3511 finding 1: the trusted reporter's summary check-run
1326:     (`feature-matrix report`, posted from the default-branch-owned
1327:     feature-matrix-report.yml). Its terminal-success is the verdict ci-summary
1328:     must structurally await whenever `opt-in group (…)` legs ran."""
1329:     return name == FM_REPORT_NAME
1330: 
1331: 
1332: def fm_run_id_of(run: dict) -> str:
1333:     """The feature-matrix Actions run id a group check-run belongs to, parsed from
1334:     its `/actions/runs/<id>` url (details_url, else html_url). "" when the url has no
1335:     parseable run id (a group check that carries no locating url). Used to bucket
1336:     group checks by the run they belong to so presence and correlation are decided
1337:     relative to the LATEST run — including the zero-leg skeleton, which is itself a
1338:     check-run of that run and so carries its run id."""
1339:     url = run.get("details_url") or run.get("html_url") or ""
1340:     m = RUNS_URL_RE.search(url)
1341:     return m.group(1) if m else ""
1342: 
1343: 
1344: def fm_group_run_id(runs: list[dict]) -> str:
1345:     """[FABLE-5] PR #3511 finding 2: the CURRENT feature-matrix run id, extracted as
1346:     the MAXIMUM `/actions/runs/<id>` seen across the `opt-in group (…)` check-runs'
1347:     details_url — name-only (is_fm_group), so the zero-leg skeleton (a check-run of
1348:     the current run) is INCLUDED. GitHub Actions run ids are monotonically ascending,
1349:     so the max is the LATEST feature-matrix run on this head — the one whose reporter
1350:     verdict the gate must await (or, when that run is zero-leg, the one that proves no
1351:     reporter is expected; see fm_report_status). On a same-SHA rerun (ready_for_review
1352:     / label) the commit carries group check-runs from BOTH the stale and the fresh run;
1353:     the fresh run's id is the larger. Returns "" if no group check carries a parseable
1354:     run id (then correlation degrades to the pre-finding-2 any-report behaviour — never
1355:     a false RED, and the stale-report race is only reachable on a same-SHA rerun)."""
1356:     best = ""
1357:     for r in runs:
1358:         if not is_fm_group(r.get("name", "")):
1359:             continue
1360:         rid = fm_run_id_of(r)
1361:         if rid:
1362:             # Numeric max (ids are ints of possibly-different width); compare as ints.
1363:             if best == "" or int(rid) > int(best):
1364:                 best = rid
1365:     return best
1366: 
1367: 
1368: def fm_report_status(runs: list[dict]) -> str:
1369:     """[FABLE-5] PR #3511 finding 1 (HIGH) + finding 2 (HIGH, correlation): the
1370:     reporter-await status over the (already forgiveness-filtered, self-excluded)
1371:     sibling set. Returns one of:
1372: 
1373:       * "n/a"     — the LATEST feature-matrix run on this head produced NO legs, so
1374:                     NO reporter is expected. Two shapes: (1) no `opt-in group (…)`
1375:                     check-run at all — a doc-only PR, a fully change-selected-out
1376:                     matrix, or a merge_group that skipped the lane; (2) the latest
1377:                     run posted ONLY the zero-leg skeleton (unexpanded placeholder,
1378:                     server-set skipped) — no real leg ran, so the reporter correctly
1379:                     posts nothing. Presence is LATEST-RUN-RELATIVE (finding, r3): an
1380:                     OLDER real group run's leftover check on a same-SHA rerun does
1381:                     NOT resurrect the requirement when the current run is zero-leg.
1382:       * "ok"      — legs ran AND a terminal-SUCCESS `feature-matrix report`
1383:                     check-run FOR THE CURRENT GROUP RUN is present: the reporter
1384:                     posted its green verdict.
1385:       * "failed"  — legs ran AND the CURRENT run's reporter check-run is terminal
1386:                     but NOT success (crashed / completeness-violation / POST error).
1387:                     The caller REDs. (Such a check also fails the normal gating-set
1388:                     render on its own — this is belt-and-braces so the reporter
1389:                     requirement is explicit and cannot be silently dropped.)
1390:       * "pending" — legs ran but the CURRENT run's `feature-matrix report` check-run
1391:                     is either ABSENT (its workflow_run has not landed it yet, or
1392:                     crashed before posting) or present-but-not-terminal. The gate
1393:                     must keep polling (still-settling); budget exhaustion in this
1394:                     state FAILS CLOSED via render_verdict's reporter belt — never
1395:                     a conclude-by-timing over the group jobs' bare successes.
1396: 
1397:     CORRELATION (finding 2 — same-SHA stale-report race): feature-matrix reruns on the
1398:     SAME head SHA (ready_for_review / label events), so a STALE report from an earlier
1399:     run can sit on the commit while the CURRENT run's reporter is delayed/crashed. Each
1400:     reporter embeds its TRIGGERING feature-matrix run id as the report's external_id
1401:     (server-supplied, unforgeable); the CURRENT group run's id is fm_group_run_id(runs)
1402:     (max `/actions/runs/<id>` across the group checks). Only a report whose external_id
1403:     equals that id counts — a stale report (older run id) is IGNORED, so it can never
1404:     satisfy the hold for a fresh group run. When the current group run id is
1405:     unresolvable (no parseable url — not expected in prod) OR no report carries an
1406:     external_id (legacy reporter), we fall back to matching ANY report: a graceful
1407:     degradation to the pre-finding-2 behaviour that is never a false RED and only
1408:     weakens the stale-race defence, which is unreachable absent a same-SHA rerun.
1409: 
1410:     SAFETY: a reporter that FAILED to post (delayed / crashed) is indistinguishable
1411:     from one that has not posted YET, and both map to "pending" — the fail-CLOSED
1412:     direction. A same-SHA fork PR whose reporter is fork-denied (cannot POST with a
1413:     read-only token) will hold here to timeout and RED; that is acceptable — a fork
1414:     PR is not auto-merged into the queue and fail-closed is the safe posture the
1415:     finding demands."""
1416:     # PRESENCE is decided relative to the LATEST feature-matrix run (finding, r3).
1417:     # A same-SHA rerun (ready_for_review / label) leaves an OLDER real group run's
1418:     # check-runs on the commit alongside a NEWER zero-leg skeleton run; keying
1419:     # presence off ANY real group (the old run's) while keying the run id off the
1420:     # newer skeleton deadlocked the gate — it awaited a reporter the zero-leg run
1421:     # correctly never posts. So: bucket the group checks by the run they belong to
1422:     # and judge the presence + reporter requirement of the LATEST run only.
1423:     group_checks = [r for r in runs if is_fm_group(r.get("name", ""))]
1424:     if not group_checks:
1425:         return "n/a"  # no feature-matrix legs on this head at all
1426:     real_groups = [r for r in group_checks if is_real_fm_group(r)]
1427:     if not real_groups:
1428:         return "n/a"  # only zero-leg skeleton(s) anywhere — no real leg ever ran
1429:     current_run_id = fm_group_run_id(runs)  # max run id across ALL group checks
1430:     # Can we bucket by run id? Only if a latest run id resolved AND every REAL group
1431:     # carries a parseable run id — otherwise a real leg of an UNKNOWN (possibly newer)
1432:     # run may exist, so we cannot safely declare the latest run zero-leg. In that
1433:     # fail-CLOSED case we keep the reporter requirement (real legs ran somewhere) and
1434:     # let current_run_id (possibly "") drive the graceful any-report degradation below.
1435:     real_unparseable = any(not fm_run_id_of(r) for r in real_groups)
1436:     if current_run_id and not real_unparseable:
1437:         latest_run_checks = [r for r in group_checks
1438:                              if fm_run_id_of(r) == current_run_id]
1439:         if not any(is_real_fm_group(r) for r in latest_run_checks):
1440:             # The LATEST run posted only the zero-leg skeleton — no leg ran, no
1441:             # reporter is expected. (A stale OLDER real run's leftover check does not
1442:             # matter; its own reporter, if any, is not what this head awaits.)
1443:             return "n/a"
1444:         # else: the latest run DID run real legs (a forged placeholder name with a
1445:         # server-set NON-skipped conclusion still counts as real — security, r2) —
1446:         # require its reporter, correlated to current_run_id.
1447:     reports = [r for r in runs if is_fm_report(r.get("name", ""))]
1448:     if not reports:
1449:         return "pending"  # legs ran; reporter verdict not on the head SHA yet
1450:     # CORRELATION: bind the verdict to the CURRENT group run. Prefer the report(s)
1451:     # whose external_id equals the current group run id; ignore stale reports.
1452:     any_report_has_extid = any((r.get("external_id") or "") for r in reports)
1453:     if current_run_id and any_report_has_extid:
1454:         matched = [r for r in reports if (r.get("external_id") or "") == current_run_id]
1455:         if not matched:
1456:             # Every report on this SHA is for an OLDER feature-matrix run (or carries
1457:             # no external_id). The current run's verdict has not landed — keep waiting
1458:             # (fail-closed on budget exhaustion). A stale success can never green us.
1459:             return "pending"
1460:         reports = matched
1461:     # If ANY (current-run) report check is non-terminal, keep waiting; else judge them.
1462:     if any(r.get("status") != "completed" for r in reports):
1463:         return "pending"
1464:     if all(r.get("conclusion") == "success" for r in reports):
1465:         return "ok"
1466:     return "failed"
```

### scripts/ci_summary_gate.py:1548-1730

```
1548: def render_verdict(runs: list[dict], summary_path: str = "", tier_ctx: TierContext | None = None) -> int:
1549:     """Shared by the clean-converge, graceful-timeout, and post-extension paths, so
1550:     every path applies IDENTICAL gating semantics. Returns the process exit code.
1551: 
1552:     DRAFT-TIER INTEGRITY ([FABLE-5], see the header): with a TierContext,
1553:       * a FULL-tier pull_request verdict REDs while any draft-tier-marked select
1554:         INSTANCE lacks its own later full-tier successor (stale draft-tier leg
1555:         set — at least one selecting workflow's ready_for_review full run has
1556:         not registered on this SHA);
1557:       * a DRAFT-tier verdict that would otherwise be SUCCESS first re-reads the
1558:         PR's CURRENT draft state from the API: no-longer-draft => FAILURE ("stale
1559:         draft-tier run, full run pending"), and an unreadable state fail-closes
1560:         to FAILURE after bounded retries. A draft-tier verdict that is already a
1561:         FAILURE skips the re-check (a RED can never be latched by the queue).
1562:     Without a TierContext (tests / push / merge_group) the semantics are exactly
1563:     the pre-draft-tier ones.
1564: 
1565:     SELECTION SEMANTICS ([FABLE-5] sq-fmx4u.3, design §5.3): a `skipped`
1566:     conclusion is satisfied ONLY when the change-based selection pre-job
1567:     (is_select) succeeded — a skip is trustworthy iff the thing that decided to
1568:     skip ran to a successful conclusion. Concretely:
1569:       * every select check-run present must have conclusion == "success";
1570:         anything else (failure, cancelled, skipped, neutral, stale) REDs the
1571:         gate outright, even if every other sibling is green — an unobservable
1572:         selection means the skips on this commit are unattributable (§4.3);
1573:       * with select green (or absent — e.g. a pre-selection sibling set, where
1574:         no skip was produced by selection), `skipped` stays non-failing exactly
1575:         as before. Absent-select degradation is deliberately the PRE-sq-fmx4u.3
1576:         behaviour, never a new failure mode.
1577:     A job that FAILED still fails the gate regardless of selection — selection
1578:     can only ever decide whether a SKIP is satisfied, never mask a failure."""
1579:     # [FABLE-5] Draft-tier belt: a full-tier pull_request gate must never conclude
1580:     # over a leg set whose selection was assembled draft-tier (checked FIRST — it
1581:     # invalidates the whole set, including an otherwise-green one).
1582:     if tier_ctx and tier_ctx.run_tier == "full" and tier_ctx.event_name == "pull_request":
1583:         stale = draft_selects_unsuperseded(runs)
1584:         if stale:
1585:             counts: dict[str, int] = {}
1586:             for n in stale:
1587:                 counts[n] = counts.get(n, 0) + 1
1588:             detail = ", ".join(
1589:                 f"{n} ×{c}" if c > 1 else n for n, c in sorted(counts.items())
1590:             )
1591:             _emit(
1592:                 "### ci-summary: FAILED — stale draft-tier run, full run pending. The "
1593:                 "selection on this head SHA is (at least partly) draft-tier-assembled: "
1594:                 f"{len(stale)} draft-marked select instance(s) have no OWN later "
1595:                 f"full-tier successor ({detail}). Each selecting workflow's "
1596:                 "ready_for_review full-tier re-run must register its own successor "
1597:                 "(ci/bench/feature-matrix/fuzz share one select name — one full-tier "
1598:                 "select must never release the hold for the others). A draft-tier leg "
1599:                 "set must never admit a non-draft PR to the merge queue "
1600:                 "(docs/branch-protection.md §Draft-tier CI). " + UNSAT_HOLD_REMEDY,
1601:                 summary_path,
1602:             )
1603:             print("::error::ci-summary failed — stale draft-tier leg set on a non-draft head.")
1604:             return 1
1605:     # [FABLE-5] PR #3511 finding 1 (HIGH): STRUCTURAL AWAIT of the trusted
1606:     # feature-matrix reporter. If `opt-in group (…)` legs ran for this head, the
1607:     # `feature-matrix report` summary check MUST be present and terminal-SUCCESS
1608:     # before the gate can conclude green. This is reached only on a would-CONCLUDE
1609:     # render (clean settle or budget-exhaustion timeout), so a "pending" here at
1610:     # RENDER time is FAIL-CLOSED, never a conclude-by-timing: the poll loop holds
1611:     # the settle window open while the report is missing/pending (report_pending),
1612:     # and a render still finding it unresolved means the reporter never landed
1613:     # within the loop's own timeout. A "failed" reporter (crashed / completeness
1614:     # violation) REDs here too (belt-and-braces: its own check-run also fails the
1615:     # gating-set render below). Checked BEFORE the empty-set / normal-render paths
1616:     # so a group set that is otherwise all-green cannot pass over an absent verdict.
1617:     fm = fm_report_status(runs)
1618:     if fm != "n/a" and fm != "ok":
1619:         if fm == "failed":
1620:             _emit(
1621:                 "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
1622:                 "concluded a NON-SUCCESS verdict (crashed / artifact-completeness "
1623:                 "violation / check-run POST error). The feature-matrix legs ran (an "
1624:                 "`opt-in group (…)` check-run is present on this head), so the reporter's "
1625:                 "verdict is required and it failed (fail-closed, PR #3511 finding 1).",
1626:                 summary_path,
1627:             )
1628:             print("::error::ci-summary failed — the feature-matrix reporter concluded non-success.")
1629:         else:
1630:             _emit(
1631:                 "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
1632:                 "verdict never landed on this head SHA within the gate's budget. The "
1633:                 "feature-matrix legs ran (an `opt-in group (…)` check-run is present), so "
1634:                 "its `feature-matrix report` summary check-run is STRUCTURALLY REQUIRED — "
1635:                 "a delayed or crashed reporter must never race past the gate. Fail-closed "
1636:                 "(PR #3511 finding 1): re-run the feature-matrix-report workflow (or push "
1637:                 "a new head) so the reporter posts its verdict.",
1638:                 summary_path,
1639:             )
1640:             print("::error::ci-summary failed — the feature-matrix reporter verdict is missing (fail-closed).")
1641:         return 1
1642:     total = len(runs)
1643:     if total == 0:
1644:         if _draft_recheck(tier_ctx, summary_path) != 0:
1645:             return 1
1646:         _emit("ci-summary: no sibling checks to aggregate (stable empty set) — passing.", summary_path)
1647:         return 0
1648:     gating = [r for r in runs if not is_advisory(r.get("name", ""))]
1649:     excluded = total - len(gating)
1650:     # [OPUS-5] #3773: make the formerly-silent exclusion loud in BOTH directions —
1651:     # every check that carries an advisory name token but is NOT declared is listed
1652:     # here and IS in `gating` above. (Diagnostic only; see undeclared_token_names.)
1653:     for undeclared in undeclared_token_names(runs):
1654:         _emit(
1655:             f"note: `{undeclared}` carries an advisory/informational NAME token but has no "
1656:             f"declaration in {ADVISORY_REGISTRY_PATH} — it GATES (#3773). Declare it there "
1657:             f"(with an owner_bead + promotion_criteria) or drop the misleading token.",
1658:             summary_path,
1659:         )
1660:     # Selection pre-job health — searched over ALL runs (not just gating) so a
1661:     # hypothetical advisory-renamed select could still never green-light a skip.
1662:     # NB superseded-cancelled select INSTANCES are already dropped upstream by
1663:     # forgive_superseded (including the deterministic-select same-name SAME-TIER
1664:     # success race-loser rule for the PURE select pre-job, sq-fmx4u.3
1665:     # hardening), so any cancelled select that SURVIVES to here has NO
1666:     # qualifying sibling (no strictly-later successor, and — for a pure select —
1667:     # no same-tier same-name success either) and rightly REDs.
1668:     select_runs = [r for r in runs if is_select(r.get("name", ""))]
1669:     select_ok = all(r.get("conclusion") == "success" for r in select_runs)
1670:     skipped_ct = sum(1 for r in gating if r.get("conclusion") == "skipped")
1671:     if not select_ok:
1672:         _emit(
1673:             f"### ci-summary: FAILED — the change-based test-selection pre-job did not "
1674:             f"succeed, so the {skipped_ct} skipped gating check(s) on this commit cannot "
1675:             f"be attributed to a sound selection (fail-closed, sq-fmx4u.3 / design §4.3).",
1676:             summary_path,
1677:         )
1678:         for r in select_runs:
1679:             _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
1680:         print("::error::ci-summary failed — the selection pre-job must conclude success.")
1681:         return 1
1682: 
1683:     def _satisfied(r: dict) -> bool:
1684:         c = r.get("conclusion")
1685:         if c == "skipped":
1686:             return select_ok  # always True past the gate above; kept explicit so a
1687:             # future refactor that moves this check cannot silently trust a skip.
1688:         return c in _PASSING
1689: 
1690:     failed = [r for r in gating if not _satisfied(r)]
1691:     if failed:
1692:         _emit(
1693:             f"### ci-summary: FAILED — {len(failed)} non-passing gating check(s) of "
1694:             f"{len(gating)} gating ({excluded} advisory check(s) excluded — each "
1695:             f"DECLARED in {ADVISORY_REGISTRY_PATH} or on the platform-managed "
1696:             f"allow-list)",
1697:             summary_path,
1698:         )
1699:         for r in failed:
1700:             _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
1701:         print("::error::ci-summary failed — see the non-passing gating checks above.")
1702:         return 1
1703:     if _draft_recheck(tier_ctx, summary_path) != 0:
1704:         return 1
1705:     _emit(
1706:         # [OPUS-5] #3774 review: the excluded set is DECLARED-in-the-registry OR on the
1707:         # exact platform-managed allow-list (PLATFORM_MANAGED_ADVISORY_NAMES) — saying
1708:         # "each DECLARED in the registry" understated the second, smaller source.
1709:         f"### ci-summary: PASSED — all {len(gating)} gating check(s) green (or skipped/neutral); "
1710:         f"{excluded} advisory check(s) excluded (each DECLARED in "
1711:         f"{ADVISORY_REGISTRY_PATH}, or on the exact platform-managed allow-list); "
1712:         f"set stable."
1713:         + (
1714:             " DRAFT-TIER verdict (reduced leg set; PR draft state re-confirmed). This "
1715:             f"check-run is `{DRAFT_TIER_GATE_NAME}`, never the required `{GATE_CHECK_NAME}` "
1716:             "context — it cannot satisfy branch protection; the full matrix re-runs at "
1717:             "ready_for_review and only its full-tier gate can."
1718:             if tier_ctx and tier_ctx.run_tier == "draft"
1719:             else ""
1720:         ),
1721:         summary_path,
1722:     )
1723:     if select_runs:
1724:         _emit(
1725:             f"selection: {len(gating) - skipped_ct} of {len(gating)} gating check(s) ran, "
1726:             f"{skipped_ct} skipped (selection and/or path-filter; selection pre-job succeeded).",
1727:             summary_path,
1728:         )
1729:     return 0
1730: 
```

### scripts/ci_summary_gate.py:2320-2339

```
2320: def make_fetch_check_runs(repo: str, sha: str):
2321:     def fetch() -> list[dict]:
2322:         # started_at + id feed the superseded-run ordering (draft-tier CI): a
2323:         # cancelled/stale check-run is forgiven only for a strictly LATER
2324:         # same-normalized-name successor.
2325:         return _gh_json_lines(
2326:             [
2327:                 f"repos/{repo}/commits/{sha}/check-runs",
2328:                 "--paginate",
2329:                 "--jq",
2330:                 # external_id carries the reporter's finding-2 correlation token (the
2331:                 # triggering feature-matrix run id) so fm_report_status can bind a
2332:                 # `feature-matrix report` verdict to the CURRENT `opt-in group (…)`
2333:                 # run and reject a stale same-SHA report from an earlier run.
2334:                 ".check_runs[] | {name, status, conclusion, details_url, html_url, started_at, id, external_id}",
2335:             ]
2336:         )
2337: 
2338:     return fetch
2339: 
```

### .github/workflows/ci-summary.yml:277-317

```
277:       # pull_request the workflow file itself already comes from the merge ref, so
278:       # scripting from the same ref does not widen anything. [FABLE-5] [SONNET-4.6]
279:       # [OPUS-5] #3773: `.github/advisory-registry.json` is LOAD-BEARING — it is the
280:       # ONLY thing that can make a check-run non-gating, and the gate exits 1 loudly
281:       # when it is absent. Pinned by scripts/tests/test_ci_summary_gate.py
282:       # ::TestAdvisoryRegistryWiring so this list cannot silently drop it again.
283:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
284:         with:
285:           persist-credentials: false
286:           sparse-checkout: |
287:             scripts/ci_summary_gate.py
288:             .github/advisory-registry.json
289:           sparse-checkout-cone-mode: false
290:       - name: Wait for all other checks, then aggregate
291:         env:
292:           GH_TOKEN: ${{ github.token }}
293:           REPO: ${{ github.repository }}
294:           # On pull_request the head commit is the PR head; on push AND on merge_group
295:           # it's github.sha (the merge_group ref's commit) — github.event.pull_request
296:           # is undefined there, so the fallback selects the right SHA automatically and
297:           # the check-run discovery reads the merge_group commit's siblings. [OPUS-4.8]
298:           SHA: ${{ github.event.pull_request.head.sha || github.sha }}
299:           # This workflow RUN's id — unique to THIS gate run. Every check-run's
300:           # details_url embeds the run id of the workflow run that produced it
301:           # (…/actions/runs/<RUN_ID>/job/<JOB_ID>), so we self-exclude by exact
302:           # run id rather than by the job NAME. A name match ("gate") is fragile:
303:           # it would also drop any FUTURE sibling job literally named `gate` in any
304:           # workflow, undermining the "add/rename jobs freely" goal. The run id is
305:           # ours alone, so the exclusion is exact and collision-proof (anchored to
306:           # "/runs/<id>(/|$)" in the script). [OPUS-4.8]
307:           SELF_RUN_ID: ${{ github.run_id }}
308:           # [FABLE-5] Draft-tier CI: the tier THIS run evaluates is decided by its
309:           # own trigger payload — pull_request + draft == true => draft tier. The
310:           # script re-checks the PR's LIVE draft state (PR_NUMBER) at conclusion
311:           # time and refuses a draft-tier success on a now-ready PR ("stale
312:           # draft-tier run, full run pending"). All three are empty off the PR
313:           # path (push/merge_group), which the script reads as full tier.
314:           EVENT_NAME: ${{ github.event_name }}
315:           PR_DRAFT: ${{ github.event.pull_request.draft }}
316:           PR_NUMBER: ${{ github.event.pull_request.number }}
317:         run: python3 scripts/ci_summary_gate.py
```

### .github/workflows/bench.yml:203-235

```
203:        contains(needs.select.outputs.affected, '"sparq-cli"') ||
204:        contains(needs.select.outputs.affected, '"sparq-bench"') ||
205:        contains(needs.select.outputs.affected, '"sparq-wasm"'))
206:     runs-on: ubuntu-latest
207:     # [OPUS-4.8] Job-scoped writes (least privilege; the rest of the workflow stays read-only):
208:     #   contents: write     — auto-ratchet commits bench/perf-baseline.json back to main and
209:     #                         github-action-benchmark pushes the point to the benchmark-data branch
210:     #   deployments: write  — github-action-benchmark records a GitHub deployment status
211:     #   pull-requests: write— comment-always posts the comparison table on (same-repo) PRs;
212:     #                         fork PRs get a read-only token, so the action skips the comment.
213:     #   issues: write       — [OPUS-4.8] the soft-zone triage step files/updates a deduped
214:     #                         `bench-flake` issue instead of @-mentioning anyone (no maintainer CC).
215:     #                         Guarded to the canonical repo (fork PRs get a read-only token).
216:     permissions:
217:       contents: write
218:       deployments: write
219:       pull-requests: write
220:       issues: write
221:     steps:
222:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
223:       - uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8 # stable
224:         with:
225:           targets: wasm32-unknown-unknown   # enables the wasm bundle-size metric in ci-bench.sh
226:       - uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2
227:       # [OPUS-4.8] (sq-7d3dj.14) binaryen (wasm-opt) — installed unconditionally so the
228:       # TREND-ONLY wasm_opt_bundle_bytes metric runs on BOTH PR and main tiers. ci-bench.sh
229:       # guards on `command -v wasm-opt` so the step is a no-op skip when absent. The version
230:       # is logged per CI run (see ci-bench.sh wasm_opt_bundle_bytes block).
231:       - name: Install binaryen (wasm-opt for shipped-size trend metric)
232:         run: sudo apt-get update -q && sudo apt-get install -y binaryen
233:       # [OPUS-4.8] Well-known-suite toolchains (sp2b/dbpsb/watdiv/bsbm/lubm), MAIN + `bench-full` PRs.
234:       # On push to main the full well-known-suite set runs; on PRs we DELIBERATELY do NOT install
235:       # these BY DEFAULT, so the ci-bench.sh suite hooks SKIP gracefully (each is guarded on
```

### .github/workflows/bench.yml:283-305

```
283:           key: bench-dbpsb-${{ runner.os }}-${{ hashFiles('bench/dbpsb/fetch.sh') }}
284:       - name: Build release binaries
285:         run: cargo build --release -p sparq-cli -p sparq-bench
286:       # [FABLE-5] (sq-6vshe.6, MAINTAINER-DIRECTED) On a PR run ONLY the DETERMINISTIC byte-count
287:       # metrics (`--deterministic-only`): the hard-gated store/dict/wasm bytes ratchet — a pure
288:       # function of the code, seconds not minutes, immune to shared-runner noise. The full noisy
289:       # timing suite (query latencies + well-known / crate-example suites + the parse timing metric)
290:       # is RELOCATED to the EC2 full-suite lane (bench-ec2.yml — MANUAL DISPATCH ONLY since #3784
291:       # retired its cron) so it no longer drags the merge queue or
292:       # flaps the gate. On push-to-main / schedule / workflow_dispatch (NOT a PR) the FULL suite runs,
293:       # so the auto-ratchet floor + the benchmark-data history (main-only steps below) stay continuous
294:       # exactly as before — the per-commit trend on the Pages dashboard is unaffected. The two-line
295:       # split keeps the emitted JSON path identical (bench-results.json) for every downstream step.
296:       - name: Run benchmarks (PR — deterministic byte-count ratchet only)
297:         if: github.event_name == 'pull_request'
298:         run: bash scripts/ci-bench.sh --deterministic-only 200000 bench-results.json
299:       - name: Run benchmarks (full suite — main / schedule / dispatch)
300:         if: github.event_name != 'pull_request'
301:         run: bash scripts/ci-bench.sh 200000 bench-results.json
302:       # [OPUS-4.8] HARD regression RATCHET on the gated metrics (store/dict bytes-per-{triple,term},
303:       # store_bytes_per_triple_small, wasm_bundle_bytes = DETERMINISTIC byte counts; parse_ns_per_byte =
304:       # TIMING, made noise-robust via best-of-N re-measure — see the Hard gate step below).
305:       # github-action-benchmark's fail-on-alert is a single GLOBAL switch that would also trip on the noisy
```

### .github/workflows/bench.yml:347-357

```
347:       - name: Hard gate — perf regression (deterministic strict; timing best-of-N robust)
348:         env:
349:           # Deliberate-bump escape hatch: comma-separated metric names allowed to regress THIS run.
350:           # Leave empty normally; set to e.g. 'wasm_bundle_bytes' on a commit/run that intentionally
351:           # grows the bundle. The allowed metric passes here AND is re-floored by the auto-ratchet step.
352:           PERF_GATE_ALLOW: ${{ vars.PERF_GATE_ALLOW }}
353:         run: |
354:           python3 scripts/perf-gate.py bench-results.json bench/perf-baseline.json \
355:             --remeasure-cmd 'bash scripts/ci-bench.sh --parse-only /tmp/perf-remeasure.json >/dev/null 2>&1 && cat /tmp/perf-remeasure.json' \
356:             --remeasure-k 4
357:       # [OPUS-4.8] AUTO-RATCHET-DOWN (sq-52e): on push to main, AFTER the gate passes, lower any floor a
```

### scripts/ci-bench.sh:61-65

```
61: fi
62: CLI=target/release/sparq-cli
63: GEN=target/release/sparq-bench
64: Q=bench/qlever-synthetic/queries
65: # [OPUS-4.8] Operator-coverage suite (one query per SPARQL operator family). Wall-clock /
```

### scripts/ci-bench.sh:234-246

```
234: assert_wasm_simd128() {
235:   rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || return 0
236:   command -v python3 >/dev/null 2>&1 || return 0
237:   [ -f scripts/check-wasm-features.py ] || return 0
238:   local probe="target/wasm32-unknown-unknown/release-wasm/sparq_wasm.wasm"
239:   if CARGO_PROFILE_RELEASE_WASM_STRIP=none cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
240:     if [ -f "$probe" ] && ! python3 scripts/check-wasm-features.py "$probe" --require simd128; then
241:       echo "ci-bench: FATAL — wasm gate build is missing +simd128 (workspace .cargo/config.toml regressed?)" >&2
242:       exit 1
243:     fi
244:   fi
245: }
246: 
```

### scripts/ci-bench.sh:284-322

```
284: # byte-count / memory-layout metrics the perf-gate HARD-gates, then exit. This is the FAST per-PR /
285: # merge_group form (bench.yml): NO timing loops, NO well-known / crate-example suites. Every command
286: # here is byte-for-byte the same one the full run below uses (one load summary for store/dict bytes,
287: # one compressed-profile load, one fixed-scale small load, one wasm build) — so the ratchet compares
288: # the identical values, just measured without the noisy latency work. `add` is idempotent-per-name;
289: # the full run measures these too when it runs on the nightly EC2 lane.
290: if [ "$DET_ONLY" = "1" ]; then
291:   "$GEN" dump "$SCALE" "$TMP/data.nt" >/dev/null 2>&1
292:   # store/dict bytes-per-{triple,term} — DETERMINISTIC memory-layout from the load summary line.
293:   "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e" || true
294:   dbtriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
295:   [ -n "${dbtriple:-}" ] && add store_bytes_per_triple bytes "$dbtriple"
296:   dbterm=$(grep -oE '[0-9]+ B/term' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
297:   [ -n "${dbterm:-}" ] && add dict_bytes_per_term bytes "$dbterm"
298:   # compressed-profile B/triple (SPARQ_STORE_PROFILE=compressed => into_compressed()) — DETERMINISTIC.
299:   SPARQ_STORE_PROFILE=compressed "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e_comp" || true
300:   dcbtriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e_comp" | head -1 | grep -oE '[0-9]+' | head -1)
301:   [ -n "${dcbtriple:-}" ] && add comp_store_bytes_per_triple bytes "$dcbtriple"
302:   # store bytes-per-triple at the SECOND (fixed 50k) scale — catches per-triple-overhead regressions.
303:   DET_FIX="${PARSE_FIX:-50000}"
304:   "$GEN" dump "$DET_FIX" "$TMP/fix.nt" >/dev/null 2>&1
305:   "$CLI" bench "$TMP/fix.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/fe" || true
306:   dbtriple2=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/fe" | head -1 | grep -oE '[0-9]+' | head -1)
307:   [ -n "${dbtriple2:-}" ] && add store_bytes_per_triple_small bytes "$dbtriple2"
308:   # wasm bundle size (bytes, DETERMINISTIC) — same release-wasm profile the shipped bundle uses.
309:   if rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
310:     if cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
311:       DET_WASM=$(ls target/wasm32-unknown-unknown/release-wasm/*.wasm 2>/dev/null | head -1)
312:       [ -n "${DET_WASM:-}" ] && add wasm_bundle_bytes bytes "$(wc -c < "$DET_WASM" | tr -d ' ')"
313:     fi
314:   fi
315:   # [FABLE-5] (sq-3ul2n.2) assert the gate build carries +simd128 — AFTER wasm_bundle_bytes read the
316:   # stripped artifact (this clobbers it with an unstripped probe; nothing below reuses it).
317:   assert_wasm_simd128
318:   emit_json > "$OUT"
319:   echo "wrote $OUT (--deterministic-only):"; cat "$OUT"
320:   exit 0
321: fi
322: 
```

### scripts/ci-bench.sh:1068-1085

```
1068: # the shipped bundle uses. Skipped gracefully when the wasm target isn't installed.
1069: WASM_BIN=""
1070: if rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
1071:   if cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
1072:     WASM_BIN=$(ls target/wasm32-unknown-unknown/release-wasm/*.wasm 2>/dev/null | head -1)
1073:     if [ -n "${WASM_BIN:-}" ]; then
1074:       add wasm_bundle_bytes bytes "$(wc -c < "$WASM_BIN" | tr -d ' ')"
1075:     fi
1076:   fi
1077: fi
1078: 
1079: # [OPUS-4.8] (sq-7d3dj.14) wasm_opt_bundle_bytes — TREND-ONLY shipped size after wasm-opt -Oz.
1080: # The raw wasm_bundle_bytes above is the HARD-GATED deterministic ratchet (scripts/perf-gate.py,
1081: # 2% band). This companion series emits the post-wasm-opt -Oz size (the "shipped" artifact after
1082: # the same wasm-bindgen/wasm-opt pass that `wasm-pack build` runs for the published npm bundle).
1083: # The ~10% gap between the two is name-section and producer metadata that browsers strip at load
1084: # time — real bundle-size wins from the optimisation program show up here while the raw gate stays
1085: # bit-identical. TREND-ONLY: scripts/perf-gate.py is intentionally UNTOUCHED. The wasm-opt version
```

### .github/workflows/vectorized-feature-off.yml:368-413

```
368:       - name: Build BASE-tree feature-OFF wasm (cache miss only)
369:         if: steps.changes.outputs.rust_changed == 'true' && steps.base.outputs.base_sha != '' && steps.base-cache.outputs.cache-hit != 'true'
370:         # Build WITHOUT --features vectorized using the base tree's OWN release-wasm profile
371:         # and a separate target dir so it never clobbers the head build cache.
372:         run: |
373:           set -euo pipefail
374:           cargo build --profile release-wasm -p sparq-wasm --target wasm32-unknown-unknown \
375:             --manifest-path base-tree/Cargo.toml --target-dir base-tree/target
376:           BASE_WASM=$(find base-tree/target/wasm32-unknown-unknown/release-wasm -name "sparq_wasm*.wasm" | head -1)
377:           if [ -z "$BASE_WASM" ]; then
378:             echo "ERROR: no sparq_wasm*.wasm found in base-tree build output" >&2
379:             exit 1
380:           fi
381:           mkdir -p base-wasm
382:           cp "$BASE_WASM" base-wasm/base.wasm
383:           # Base-tree declaration file may not exist on older base commits -> synthesize an
384:           # empty one so the token reads 0 (fail-safe default).
385:           if [ -f base-tree/bench/feature-off-declaration.json ]; then
386:             cp base-tree/bench/feature-off-declaration.json base-wasm/base-decl.json
387:           else
388:             echo '{}' > base-wasm/base-decl.json
389:           fi
390:           # MECHANISM V2 (sq-v3nel-v2): snapshot the base-tree per-PR declarations directory
391:           # so leg 2 can set-difference it against the head tree. Absent on pre-V2 base
392:           # commits -> synthesize an EMPTY dir (fail-safe: an absent dir reads as no
393:           # declarations, so an undeclared byte change still fails).
394:           mkdir -p base-wasm/base-declarations
395:           if [ -d base-tree/bench/feature-off-declarations ]; then
396:             cp -a base-tree/bench/feature-off-declarations/. base-wasm/base-declarations/
397:           fi
398:           echo "Base-tree feature-OFF wasm: $(wc -c < base-wasm/base.wasm) bytes"
399: 
400:       - name: Build HEAD-tree feature-OFF wasm
401:         if: steps.changes.outputs.rust_changed == 'true'
402:         # Builds WITHOUT --features vectorized — default features only. The release-wasm
403:         # profile is size-optimised (opt-level z for cold parse crates, strip symbols).
404:         run: |
405:           set -euo pipefail
406:           cargo build --profile release-wasm -p sparq-wasm --target wasm32-unknown-unknown
407:           HEAD_WASM=$(find target/wasm32-unknown-unknown/release-wasm -name "sparq_wasm*.wasm" | head -1)
408:           if [ -z "$HEAD_WASM" ]; then
409:             echo "ERROR: no sparq_wasm*.wasm found under target/wasm32-unknown-unknown/release-wasm" >&2
410:             exit 1
411:           fi
412:           cp "$HEAD_WASM" head.wasm
413:           echo "Head-tree feature-OFF wasm: $(wc -c < head.wasm) bytes"
```

### .github/workflows/ci.yml:1154-1169

```
1154:       # [FABLE-5] sq-01xlp: the opt-in `stateful` (pre-parsed ParsedGraph handle) state.
1155:       - name: Build sparq-shacl-wasm for the browser target (stateful feature)
1156:         run: cargo build -p sparq-shacl-wasm --target wasm32-unknown-unknown --features stateful
1157:       # [FABLE-5] sq-01xlp: the combined state (`shacl-af`+`stateful` — both opt-in features
1158:       # can be enabled together, and stateful validation inherits sh:rule behaviour).
1159:       - name: Build sparq-shacl-wasm for the browser target (all features)
1160:         run: cargo build -p sparq-shacl-wasm --target wasm32-unknown-unknown --all-features
1161:       - name: clippy (wasm32 target, sparq-shacl-wasm, deny warnings)
1162:         run: cargo clippy -p sparq-shacl-wasm --target wasm32-unknown-unknown --all-targets -- -D warnings
1163:       - name: clippy (wasm32 target, sparq-shacl-wasm, shacl-af feature, deny warnings)
1164:         run: cargo clippy -p sparq-shacl-wasm --target wasm32-unknown-unknown --all-targets --features shacl-af -- -D warnings
1165:       - name: clippy (wasm32 target, sparq-shacl-wasm, stateful feature, deny warnings)
1166:         run: cargo clippy -p sparq-shacl-wasm --target wasm32-unknown-unknown --all-targets --features stateful -- -D warnings
1167:       - name: clippy (wasm32 target, sparq-shacl-wasm, all features, deny warnings)
1168:         run: cargo clippy -p sparq-shacl-wasm --target wasm32-unknown-unknown --all-targets --all-features -- -D warnings
1169:       - name: Guard the wasm dependency graph (no native-only deps)
```

### .github/workflows/ci.yml:1273-1279

```
1273:       - name: Headless wasm tests (sparq-shacl-wasm, wasm-pack test --node, stateful feature)
1274:         working-directory: crates/sparq-shacl-wasm
1275:         run: wasm-pack test --node --features stateful
1276:       - name: Headless wasm tests (sparq-shacl-wasm, wasm-pack test --node, all features)
1277:         working-directory: crates/sparq-shacl-wasm
1278:         run: wasm-pack test --node --all-features
1279:       # [OPUS-4.8] sq-aq5: sparq-introspect wasm-safety. The crate is pure sorted scans over
```

### .github/workflows/ci.yml:1393-1416

```
1393:       # not a second full build (the bulk of the dependency + first-party graph is already
1394:       # compiled by the default-features clippy above).
1395:       - name: clippy (deny warnings, workspace — all features)
1396:         run: cargo clippy --workspace --all-targets --all-features -- -D warnings
1397:       # [OPUS-4.8] sq-8gsv: rustdoc -D warnings, WORKSPACE-WIDE. A broken/private intra-doc
1398:       # link (e.g. a public item linking to a crate-private const/fn, or an unresolved
1399:       # `[`Foo`]` that renders as literal text) is a rustdoc warning — NOT a clippy/build
1400:       # warning — so it slips past every other gate here and mis-renders the published docs.
1401:       # This was previously scoped to sparq-solid (sq-z1rm) and sparq-mpc (sq-h1w2) only,
1402:       # because the rest of the workspace carried ~174 pre-existing intra-doc-link warnings
1403:       # across ~20 crates. sq-8gsv cleared that backlog, so the gate now covers the WHOLE
1404:       # doc surface: any new public item that links to a private/unresolved one fails CI.
1405:       # Two feature states cover the feature-gated doc surface (e.g. sparq-solid's
1406:       # `odrl-bridge`/`count-enforcement` modules, sparq-mpc's `insecure-test-rng`,
1407:       # sparq-prov's `reason` bridge): default features, then `--all-features`.
1408:       - name: rustdoc (deny warnings, workspace — default features)
1409:         run: cargo doc --workspace --no-deps
1410:         env:
1411:           RUSTDOCFLAGS: "-D warnings"
1412:       - name: rustdoc (deny warnings, workspace — all features)
1413:         run: cargo doc --workspace --no-deps --all-features
1414:         env:
1415:           RUSTDOCFLAGS: "-D warnings"
1416:       # [OPUS-4.8] sq-hqmm: invoke the standalone bench/dict (dict-baseline) selftest from a CI
```

### crates/sparq-core/Cargo.toml:14-35

```
14: readme = "README.md"
15: 
16: # [OPUS-4.8] wire docs.rs to build all-features + the `docsrs` cfg so the README
17: # front page and every feature-gated item render on docs.rs.
18: [package.metadata.docs.rs]
19: all-features = true
20: rustdoc-args = ["--cfg", "docsrs"]
21: 
22: [features]
23: # Parallel index construction via rayon. On by default for native builds; turn it
24: # off (e.g. for wasm, which has no threads by default) for a smaller, simpler build.
25: default = ["parallel"]
26: # `memchr` rides `parallel`: it is only used by the parallel Turtle terminator pre-scan
27: # (T2), so the wasm build (which disables `parallel`) never links it. [OPUS-4.8]
28: parallel = ["dep:rayon", "dep:memchr"]
29: # Build only THREE permutation indexes (SPO, POS, OSP) instead of six. Every triple
30: # pattern is still answered by one of them, but some merge joins fall back to hashing.
31: # ~halves index memory (36 vs 72 B/triple) — for the memory-constrained browser target.
32: compact-index = []
33: # [GPT-6 Astra] Experimental lazy tombstone projections. OFF by default: cold
34: # reads sort and allocate; forks copy retained projections. See the crate README.
35: overlay-deleted-projections = []
```

### .cargo/config.toml:1-18

```
1: # getrandom 0.3 has no default backend for wasm32-unknown-unknown; select the browser backend
2: # (crypto.getRandomValues) so the wasm build (oxrdf -> rand -> getrandom, for blank-node ids)
3: # compiles. Pairs with `getrandom = { features = ["wasm_js"] }` in crates/sparq-wasm/Cargo.toml.
4: #
5: # [FABLE-5] sq-3ul2n.2: `+simd128` is set HERE at the workspace level (not in a crate-level
6: # .cargo/config.toml) so EVERY wasm32 build — the shipped `wasm-pack` build, the root-invoked
7: # `wasm_bundle_bytes` gate build in scripts/ci-bench.sh, and all five wasm crates — compiles
8: # with WASM SIMD128 identically. cargo discovers `.cargo/config.toml` by CWD and a crate-level
9: # `rustflags` REPLACES (not merges) the root one, so keeping both files would silently drop
10: # getrandom_backend for a root build or simd128 for a crate build. These flags are
11: # target-scoped (`[target.wasm32-unknown-unknown]`) — native builds are byte-identical.
12: # NOTE: simd128 is ~speed-neutral for sparq today (no hand-written vector kernels; measured in
13: # research/hardware/consumer-and-targets.md §5) and gives a small one-time bundle-size decrease;
14: # the browser support floor (Chrome 91+/Firefox 89+/Safari 16.4+) is already required by the
15: # shipped bundle, so this is not a support-floor change. scripts/check-wasm-features.py asserts
16: # the built artifact actually carries the simd128 target_features flag.
17: [target.wasm32-unknown-unknown]
18: rustflags = ['--cfg', 'getrandom_backend="wasm_js"', '-C', 'target-feature=+simd128']
```
