# Outcome-blind AC-SPARQL publication bundle

This directory is a proposed standard-library Python publication layer for the canonical
AC-SPARQL study. It was designed from the frozen protocol, analyzer source/tests,
manuscript, and synthetic fixtures while acquisition was active. No canonical JSONL,
derived outcome, or canonical figure was inspected in its design or testing.

The only non-Python runtime dependency is the system `zstd` command. Both programs
resolve the executable without a shell, record its version and SHA-256 digest, and fail
if it is absent or invalid.

## Components

- `create_raw_archive.py` preserves the EC2-original manifest, installs the reviewed
  annotations, constructs the final run-root manifest, creates the deterministic
  sanitized archive, and then independently verifies it.
- `export_publication.py` validates all canonical inputs, creates five digest-bound
  figures, and emits the immutable object-keyed factory envelope.
- `test_export_publication.py` uses a full synthetic matrix only. The JSON templates are
  operator annotations; they are never trusted for derived pass/count claims.
- `PUBLICATION-PLACEHOLDER-ROUTING.md` separates generated scalar/Boolean evidence from qualitative
  prose that requires human review.

## Fail-closed checks

The exporter requires the exact frozen matrix and validates:

- 1,472 summary rows, 368 timing-profile paired-contrast rows, 256 construction rows,
  32 H2 cells, 1,280 process fixtures, and 1,280 checksummed raw JSONL files;
- the complete primary P grid, with query hash, exact result-bag hash, result row count,
  and joined target-readable-document count invariant within every
  profile/lane/domain/query/block group;
- every H2 wall/CPU ratio and elasticity verdict by recomputing the four prospective
  upper-bound tests, then deriving lane pass/fail/all/none/mixed values over all 16 cells;
- exactly 288 one-record correctness files and their exact matrix keys, yielding 5,760
  exact result-bag comparisons (4,608 in the materialized subset), plus source-commit-
  attested logs from the three frozen Cargo suites containing all four named security
  tests;
- every discovered `failures/**/FAILED.json` sentinel against the exclusion annotations,
  rather than accepting an operator-supplied exclusion count;
- final cost-component arithmetic, canonical and study-wide totals no greater than USD
  100, and the distinction between tag-scoped final billing and an accounted
  direct-resource estimate;
- a distinct clean 40-hex benchmark-source commit and post-run-analysis commit,
  protocol/analyzer/publisher hashes, environment/timestamps, and exact host duration;
  and
- the local compressed archive, final manifest, every decompressed member and header,
  exact member set, safe paths, absence of secret-like names/content, and every declared
  digest and byte length.

Any missing cell, malformed/non-finite value, schema drift, unexpected archive member,
hash mismatch, incomplete probe, unsafe path, or differing immutable target aborts the
export.

## Reporting boundary

H1 and H2 are the only hypotheses that can emit Boolean declarations. H3--H5 are
exploratory/descriptive and cannot receive a Boolean verdict. RQ3 is limited to
observable work proxies and source-derived theoretical decomposition; it makes no
empirical stage/time-dominance claim.

The paired publication object is `results.guarded_stack_content_reference`, and the
figure is `guarded-stack-content-reference.svg`. This is a guarded-stack/content-
reference contrast against a content-only physical reference that is
query-answer-equivalent for the eight frozen templates. It is not pure WAC overhead,
interface overhead, or an otherwise-identical same-data counterfactual. All cells are
retained; the exporter never chooses a favourable query or domain.

Timing-profile summaries expose wall and process-CPU estimates. Instrumentation profiles
expose allocation/backend work counters but not their perturbed timings. Construction
and RSS remain lane-specific; RSS is whole-process, and completion is not a capacity
maximum. The generated envelope contains only bound numeric outcomes and H1/H2
Booleans—not outcome-shaped narrative. Every scalar in the complete result tree has an
`analysis.result_bindings` pointer-to-artifact/hash/locator entry. To avoid duplicating a
large matrix into the factory ledger, `paper_evidence` contains only scalars actually
used by the manuscript; each remains a pointer into that same complete tree and has a
full result binding. See `PUBLICATION-PLACEHOLDER-ROUTING.md` for the prose split.

## Integration and exact order

Use these steps only after canonical acquisition has stopped and its original `DONE` and
`MANIFEST.sha256` verify.

1. Before outcome inspection, verify that the analyzer contains the reviewed
   block-aware paired intervals, derived-output hashes, and primary cross-P invariants,
   and that the protocol contains Amendments 1.29 and 1.30 with header version `1.30`.
2. Commit the analysis and publication-tool changes, and record that clean commit as the
   analysis commit. It must differ from the source commit embedded in the raw run.
3. Rerun the patched analyzer from the clean analysis commit with
   `--require-canonical`, the exact source commit, at least 10,000 bootstrap draws, and
   the recorded bootstrap seed.
4. Prepare the three annotation reports. From the captured study log/journal, make a
   content-addressed transcript for each Cargo command that records the independently
   established `source_commit=<40-hex>` and includes the relevant test output verbatim;
   do not rewrite a test status. Bind those transcripts in the correctness annotation.
   Do not type gate counts, pass claims, or exclusion counts into the templates.
5. Record the original EC2 manifest digest, then create the final archive:

   ```sh
   python3 bench/ac/scaling/create_raw_archive.py \
     --repo-root . \
     --run-root /path/to/verified/canonical-run \
     --run-id RUN \
     --cost-report /path/to/cost-report.json \
     --correctness-report /path/to/correctness-report.json \
     --exclusion-report /path/to/exclusion-report.json \
     --ec2-manifest-sha256 ORIGINAL_64_HEX \
     --zstd /absolute/path/to/zstd \
     --write
   ```

   Its JSON output supplies the final manifest and archive descriptors for
   `publication-metadata.json`. The archive path is fixed at
   `bench/canonical-competitor-results/ac-sparql/RUN/raw-sanitized.tar.zst`.

6. Publish into the exact run-specific locations:

   ```sh
   python3 bench/ac/scaling/export_publication.py \
     --repo-root . \
     --derived /path/to/verified/canonical-run/derived \
     --metadata /path/to/publication-metadata.json \
     --out-envelope bench/canonical-competitor-results/ac-sparql/RUN/paper-summary.json \
     --figure-dir site/papers/figures/canonical-timing/access-controlled-sparql-pod-scale/RUN \
     --zstd /absolute/path/to/zstd \
     --write
   ```

7. Add only the new run-specific envelope to `canonical-timing-sources.json`; run the
   factory sync/check, evidence verifier, paper build, and privacy/no-unbound-number
   gates. Commit the archive, envelope, and figures together. Rerun both commands with
   `--check` to prove byte-for-byte reproducibility.

An optional archive `public_url` is permitted only after a real HTTPS object exists. The
committed local `{path, sha256, bytes}` tuple is always required and authoritative.

## Synthetic verification

From the copied benchmark directory:

```sh
python3 -m py_compile export_publication.py create_raw_archive.py test_export_publication.py
python3 test_export_publication.py -v
```

The suite constructs a complete synthetic raw/derived run and archive. It tests stable
serialization, complete result-pointer/source binding, complete figure slices,
H2 recomputation, byte drift, missing cells, cross-P answer drift, USD-100 enforcement,
secret rejection, exact manifest/member agreement, immutable write/check behavior, and
reproducible archive creation. It never discovers a workspace result directory.
