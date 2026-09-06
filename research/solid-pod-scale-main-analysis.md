# Main campaign analysis contract

[GPT-6] The independent extractor reads the frozen `campaign.json`, complete raw requests and durable audits, per-Pod inventories, cgroup snapshots, dedicated policy probes, source hashes and final checksum manifest. It does not rely on the runner's pass label to reproduce a local service verdict.

```sh
python3 bench/ac/million/analyze-campaign.py /path/to/finalized-campaign --review /path/to/source-review.json --output research/solid-pod-scale-main.json
python3 -m unittest discover -s bench/ac/million -p test_campaign_analysis.py
```

The Python standard library and the `zstd` command are required. JSONL inputs may be plain or `.jsonl.zst`, but simultaneous versions of the same input are rejected. GNU checksum entries such as `./filename` are normalized to the same resolved path beneath the artifact root used by readers; escaping paths and duplicate normalized members are rejected. Compressed files are hashed before and after streaming. Temporary SQLite indexes reconcile mutation identities and schedules without retaining the full population or receipt objects in memory. No generated main-result file is committed before actual artifacts exist.

The optional review document binds `source_commit` to the exact campaign commit and declares `status` as `passed`, `quarantined` or `unreviewed`. Its scope is **benchmark-method and request-accounting review**. It can record the resolved findings and scoped test evidence; it is neither a security audit, proof of complete correctness nor user approval. Missing or mismatched review bindings leave inferential admission unreviewed. Any later explicit `correctness-quarantine` event overrides a passed review and invalidates inference from that source. Raw diagnostics remain available with validity flags.

An admission or resource failure affects the applicable dataset/configuration. It does not invalidate already verified compact measurements merely because a larger history could not be admitted. A finalized failed campaign may preserve valid earlier cells; an unfinalized or checksum-invalid bundle cannot. Malformed request or audit records reject their cell. Every planned cell appears, including unreached and skipped cells as **unmeasured**, never as successful or failed timings.

[GPT-6] Main-format verification is bound to the complete streamed inventory: the declared sample must equal the minimum and maximum source-size Pod in every observed configured intensity class, with the lowest Pod ID breaking ties. The extractor retains only those extrema. Each representative needs exactly one matching successful process event and one ordered start/progress/completion log for that Pod. The frozen verifier performs four principal checks over eleven corpus queries and sixteen journey templates. Missing, duplicate, malformed, conflicting or incomplete successful evidence rejects the dataset. Explicit oracle mismatches quarantine the source; verifier OOM/admission/process failures reject that dataset without becoming an HTTP memory measurement or invalidating unrelated datasets. Plain-text diagnostics may accompany a declared failed verifier, but cannot substitute for successful completion. The older single `-verify.jsonl` pilot format is not accepted by this main-campaign extractor; pilot analysis remains in `analyze-pilot.py`.

Successful-response percentiles are reported separately from all-recorded-outcome durations. Every offered request stays in success and deadline denominators. The primary local deadline uses scheduled arrival through complete HTTP body receipt, and separately checks the server response-production timer. Missing timers, incomplete sequence coverage, client saturation, exhausted plans, short measurement windows, mismatched offered rates, wrong source/workload/corpus bindings or unsuccessful audit exits prevent admission. Queue quarters are based on all scheduled arrivals, including failed requests; missing timer coverage prevents a stable-queue verdict.

Raw load settings must match the frozen group and measurement: seed, scenario, selection, journey mix, request timeout, maximum in-flight requests, requested duration, minimum request count and rate override. These bindings supplement the observed schedule horizon, complete offered sequence and actual offered-rate checks; they do not change the frozen pass criteria.

[GPT-6] Each request summary includes `http_status.counts_of_recorded` and `http_status.by_outcome`. Integer codes from 100 through 599 retain their exact code as a JSON key, so service-unavailable responses remain distinguishable from authorization or protocol errors even when their outcome is `http-error`. Absent and null status values count as `missing`; other values count as `invalid` without string or numeric coercion. These counts cover recorded request rows only: an unsent request with no status stays missing, and an unrecorded offered request contributes no status count. This descriptive classification changes no validity checks, latency distributions, success/deadline criteria or offered-request denominators.

Mutation acknowledgements must match exactly one durable receipt for the same Pod and identity. Duplicate receipts, unexpected commits, missing acknowledgements, wrong counts and failed replay remain explicit failures. A transport or body failure is resolved as committed or not committed only after complete reconciliation. Selected absent-record modifications may be declared no-ops; successful policy attempts are split into changed and unchanged triples. Neither a no-op nor a nonzero policy delta establishes correct rights by itself. The dedicated calendar probes check the expected grant/revoke sequence, owner/recipient/anonymous query answers, unauthorized administration, eviction, and the runner-declared restart. Their scope remains separate from general Solid conformance.

Resource admission requires both cgroup snapshots, the configured peak-memory ceiling and no OOM events. CPU and process I/O differences, charged page-cache statistics, peak RSS and PSS remain separately available. The cgroup peak includes warmup. Stored population and record/byte totals are recomputed from the complete inventory and compared with the manifest. Pre-load disk allocation excludes later journal allocation unless additional measurements are supplied.

`storage_inventory_consistent` describes inventory/count agreement separately from query-verification admission; `inventory_consistent` additionally requires all representative verification evidence. Serialized source bytes are N-Quads volume, while packed payload plus offset-index bytes are file lengths. Whole-directory allocated bytes additionally include the uncompressed inventory and manifest. The packer records payload/index digests at creation; opening checks index length and representative verification checks query results, but does not rehash both complete payload files. The local review bundle may omit those payload files. Server CPU counts describe affinity, not a CPU bandwidth quota; changing the count also changes workers, total queue slots and per-worker cache partitioning. CPU-time deltas cover the measured client window and can include outstanding warmup work, because the runner does not drain server queues between client warmup completion and measurement. Timed-out client requests can continue on workers; remaining work is stopped at server termination. The counters therefore do not establish CPU per measured journey. Cgroup memory peaks include warmup and charged page cache, while driver state and other host allocations remain outside that cgroup. Neither the smallest tested ceiling nor these measurements establishes a whole-host resource minimum.

Paired latency intervals resample independent matched runs. Matching requires the same replicate, seed, offered rate and intended request-schedule hash; requests inside one run are not independent replicates. The schedule fingerprint includes request order/timing, target Pod, principal, channel, operation and template identity. For non-policy requests it also includes the recorded query digest, complete mutation request (including batch identity, offset, revision and count), and expected inserted/deleted triple counts. Equal template IDs or record counts therefore cannot hide changed query content or record targets. Policy attempts use their common intended rights, including the desired grant state, rather than language-specific policy query bytes. Outcomes, receipts and observed timings are excluded from the intent fingerprint. The estimate is the geometric mean ACP/WAC ratio, with a percentile bootstrap interval. A same-load completion-rate ratio is explicitly offered-load-capped and is not a capacity comparison.

A rate is classified as passing only when all required repetitions pass. It supplies a failing bracket only when every required repetition has valid, non-client-limited failing evidence; mixed or missing outcomes stay inconclusive. Under the declared monotonic operational-capacity assumption, the interval from highest passing to lowest consistently failing rates remains unsampled. The conservative ACP/WAC ratio uses both languages' interval endpoints. An equal highest passing grid point can therefore coexist with very broad capacity bounds. A bootstrap of the equal grid-point statistic cannot remove this uncertainty. Missing or nonmonotone brackets leave capacity equivalence unestablished. Even sufficiently narrow brackets describe the frozen operational criterion, rather than an interpolated exact maximum.

The full-service million-history admission field remains false in this extractor. A later final evidence assessment must separately establish the required large retained-history population, operations, resource accounting and network journeys. Compact population controls and local cell passes cannot silently supply that claim. Main-result presentation should replace the draft's detailed pilot table, keeping the manuscript concise.

## Manuscript binding

The standalone paper now binds the finalized [main analysis](solid-pod-scale-main.json)
by default. The [audit summary](solid-pod-scale-main-report.json) binds its numerical
cohorts to the main file's digest. Render the concise manuscript and exhaustive
companion from the repository root:

```sh
typst compile --root . site/papers/solid-pod-scale.typ output/pdf/solid-pod-scale.pdf
typst compile --root . site/papers/solid-pod-scale-companion.typ output/pdf/solid-pod-scale-companion.pdf
```

The companion uses relative links compatible with the same output directory.
The paper links to that sibling PDF and to source/audit JSON in the local review
bundle; these are not public archival URLs. An explicit empty `--input campaign=`
selects the historical pilot fallback. A supplied analysis must use the extractor's
schema and pass complete-checksum and matching source-review guards; quarantine
takes precedence. The final narrative also checks the valid run set and factual
conditions supporting its headline claims, rather than applying those claims to
an arbitrary future artifact.

The main response table selects experiment roles, not successful outcomes. It
omits secondary compact controls and additional hot-set memory tiers, which remain
in the companion together with every planned pass/fail/inconclusive/unmeasured
count, resource counter and paired interval. The main table separates queue-only
guard failures from other failures and keeps all offered requests in success and
deadline denominators. Conditional successful-response percentiles remain absent
when no request succeeds. Physical storage totals use their independent inventory
check, so representative admission failure does not erase valid stored totals.

The focused tests exercise source gates, queue-only classification, missing-value
handling, paired intervals and physical/resource accounting. Run them with
`TYPST_BIN=/path/to/typst python3 -m unittest discover -s bench/ac/million -p test_paper_results.py`.

## Cache and indexed component diagnosis

Each request summary counts observed cache hits, misses, missing headers and invalid cache values, with separate successful-response and outcome counts. Header coverage retains all offered requests in its denominator; hit fraction uses only classified headers. A timed-out or otherwise missing response is not inferred to be a cache miss. Existing phase distributions retain their measured-timer scope.

`indexed_component_diagnostics` extracts the bounded sequential indexed-history logs and final file lists through the same manifest-verifying reader. It checks completion, exact comparison totals and reference coverage, phase coverage, and final file counts/logical sizes against open-time snapshots. An explicit wrong indexed result quarantines inference from that source; missing or resource-failed component diagnostics do not invalidate independent compact HTTP cells. Component inference requires final checksums and the matching passed source review. Both analysis modules are included in `analysis_sources_sha256`.

The output separates generation, parse/index preparation, authorization construction, save, reopen/validation and query comparison coverage. Parse-plus-memory-ready is the sum of those measured preparation phases; open-to-authorized-ready includes existing index validation and authorization materialization. Save/open snapshots supply allocated-byte counts. The final remote file list binds file hashes and logical sizes to the campaign; index content files are not independently rehashed locally when absent from the review bundle. The files were just generated and the operating-system cache was uncontrolled. These are component measurements over a small sequential prefix, not independent timing repetitions, crash-durability evidence or million-Pod indexed capacity. The manuscript replaces a diagnostic paragraph with one compact table only when those records are admitted.
