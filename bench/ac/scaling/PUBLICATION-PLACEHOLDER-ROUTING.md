# Manuscript placeholder routing

The current manuscript uses one temporary `#empirical("…")` mechanism for both numbers
and sentences. Those two roles should be separated. The timing factory intentionally
accepts only finite numbers and derived Booleans; it must not become a channel for
outcome-shaped prose.

## Replace with generated scalar or Boolean evidence

The exporter emits accessor-ready records for these classes:

- `campaign.*` counts, but with unambiguous names:
  `valid_observations`, `valid_timing_observations`,
  `valid_instrumentation_observations`, `completed_paired_cells`,
  `complete_process_fixtures`, `raw_files`, and `excluded_attempts`;
- `correctness.records` (288), `correctness.oracle_comparisons` (5,760),
  `correctness.materialized_oracle_comparisons` (4,608), `correctness.passed`, and
  `correctness.all_required_probes_passed`; the counts and Booleans are derived from
  every checksummed gate record and the source-commit-attested suite logs, not copied
  from an operator report;
- every atomic H2 wall/CPU ratio, interval endpoint, elasticity, interval endpoint, and
  per-cell verdict for all 32 lane × domain × query cells;
- H2 all-cell rollups for each lane: cell count, pass count, fail count, all/none/mixed
  Booleans;
- native q1 backend-operation endpoints and growth ratio for *each* domain, rather than
  one silently pooled value;
- lane-specific scenario corpus, construction-phase, current-RSS, peak-RSS, and
  completion fields for all four scenario labels;
- final study and canonical-run USD costs and exact host duration in seconds; and
- numeric completeness counts for the guarded-stack/content-reference intervals,
  sensitivity matrix, and construction matrix. Completeness is not an H3--H5 verdict.

The 32 compact H2 cells should become an accessor-built table from atomic records. A
single string-valued `*.compact` evidence record would hide its constituent estimates
and defeat independent pointer verification. Cost and duration should likewise be a
number plus a manuscript-supplied unit, not a preformatted sentence.

Every generated figure should use `#timing_figure`: Pod scaling, backend operations,
the guarded-stack/content-reference contrast, factor sensitivities, and
construction/RSS.

## Keep as hand-reviewed qualitative prose

These placeholders are interpretations, scope statements, or release metadata and
should not be emitted as timing-evidence strings:

- `abstract.h2_routed_summary`, `abstract.h2_http_summary`,
  `abstract.cost_summary`;
- `h2.*.overall_narrative`, `h2.*.family_verdicts`,
  `h2.materialized.claim_language`, and `h2.http.h2b_relation`;
- `mechanism.http_counter_decomposition`,
  `mechanism.materialized_work_summary`, and
  `mechanism.materialized_construction_summary`;
- every `sensitivity.*_summary`;
- `overhead.materialized_scope`, `overhead.materialized_summary`,
  `overhead.materialized_by_query`, and `overhead.http_summary`; these should be renamed
  away from “overhead” where rendered and describe the guarded-stack/content-reference
  boundary explicitly;
- `capacity.*_scaling_summary` and `capacity.linearity_statement`;
- `hypothesis.h3_verdict`, `hypothesis.h4_verdict`, and
  `hypothesis.h5_verdict`;
- `discussion.central_result`, `discussion.amortization_implication`;
- all `conclusion.rq*` prose and `conclusion.permitted_headline`; and
- environment descriptions, commit IDs, DOI, license, artifact paths, and release
  status strings.

Human review does not mean numbers may be pasted into these sentences. A sentence should
contain stable qualitative wording plus adjacent `#headline_timing`, `#timing_verdict`,
or table accessors wherever a measured scalar is needed. If wording depends on whether
all, none, or some H2 cells meet the criterion, select among three prewritten scoped
sentences using the generated all/none/mixed Booleans. Do not generate a list of
"successful" families; the complete per-cell table is the authoritative disclosure.

## Hypothesis-specific handling

- **H1:** the generated Boolean may render “passed” only within the stated generated-WAC
  and exact-result-bag scope.
- **H2a:** “minimal unrelated-Pod overhead across the tested matrix” is permitted only
  when `h2.materialized.rollup.all_cells_meet` is true. A mixed result must name the
  domain/query scope through the full table, not an average.
- **H2b:** do not reduce the source-derived native prediction to a convenient binary
  merely because one or more cells fail. Report all sixteen verdicts, backend work for
  both domains, and hand-review whether the observed pattern is consistent, mixed, or
  contrary.
- **E3 (formerly H3):** no confirmatory dominance threshold or stage timer was frozen.
  Describe complete work-count and factor panels cautiously; do not infer stage/time
  dominance from allocations or aggregate backend operations.
- **E4 (formerly H4):** the protocol has no binary threshold and did not operationalize
  “non-trivial query evaluation.” Report every guarded-stack/content-reference paired
  estimate and interval descriptively. An interval containing one/zero is not an
  equivalence result, and the contrast is not pure WAC overhead or a same-data
  counterfactual.
- **E5 (formerly H5):** no linear-model decision rule was frozen. Any linearity wording
  is exploratory, and scenario completion is never a maximum-user claim.

## Manuscript structural changes implied by the data

1. Rename “completed cells” to either complete paired cells or complete process fixtures;
   the existing phrase conflates two different counts.
2. Replace each `*.compact` H2 placeholder with five atomic rows/lines (wall ratio and
   interval, wall elasticity and interval, CPU ratio and interval, CPU elasticity and
   interval, Boolean verdict).
3. Split scenario construction/RSS columns by lane. The current one-cell-per-scenario
   table would otherwise collapse two materially different construction paths.
4. Replace the singular backend endpoint placeholders with social and health values.
5. Remove the request for “block structure” from the factor-sensitivity figure unless
   the analyzer is extended, while still blinded, to export block-level sensitivity
   summaries. `summary.csv` contains pooled descriptive medians only.
6. Treat backend counter decomposition as code-derived qualitative prose unless the
   analyzer is extended to emit the individual query/update/blob counters. The current
   summary output contains only total operations.
7. State release commits, DOI, license, and machine path directly from release metadata
   or the non-timing evidence ledger. They are identifiers, not timing results.

This division keeps the paper readable without allowing its prose to become an
unreviewed, outcome-dependent generated artifact.
