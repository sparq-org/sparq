# Exploratory HTTP pilot analysis

[GPT-6] `bench/ac/million/analyze-pilot.py` extracts `solid-pod-scale-pilot.json` from the accompanying local artifact directory. It does not admit canonical capacity or policy-language equivalence. The manuscript reads its numerical observations from that generated JSON.

Run from the repository root:

```sh
python3 bench/ac/million/analyze-pilot.py /path/to/completed-pilot --output research/solid-pod-scale-pilot.json
python3 -m unittest discover -s bench/ac/million -p test_analyze_pilot.py
```

The summary binds parsed inputs, runner scope, source commit, declared source-input hashes and analysis script to SHA-256 digests. The extractor verifies every safe file named by the final manifest without exposing log or authentication contents. Missing manifests remain provisional. JSONL parse errors, absent end records, missing sequences, duplicate sequences and inconsistent totals remain explicit. A checksummed artifact establishes integrity, not experimental sufficiency.

Successful complete-body percentiles exclude unsuccessful requests and are labeled accordingly. The deadline fraction includes every offered request, including client admission drops and missing records. The scheduled-arrival timer includes dispatch delay and client credential preparation; the HTTP-dispatch timer covers complete body receipt; the server header ends at response production. Phase percentiles are separate statistics and must not be added. Observed successful completions divided by elapsed time, including drain, is not a sustainable-capacity measurement.

Generic transport failures remain transport failures. Their elapsed time can be compared with the configured timeout, but the original generic error string does not establish a timeout cause. Policy writes with zero or missing triple delta are distinguished from reported changed triples; neither a successful reply nor a nonzero triple delta proves revocation. Content writes in this pilot replace existing values, rather than ingesting or expiring service records. Absent operations remain absent coverage.

The resource report preserves packed and raw bytes, largest Pods, per-Pod distributions, process high-water RSS, PSS and process I/O deltas. Pre-load disk allocation excludes later journals. Source-byte cache limits are not heap limits, and the process snapshots cannot establish the minimum cgroup RAM tier. Persisted Pod sizes above the configured admission limit are counted from summaries; this size comparison does not classify generic transport failures as explicit server rejections.

The initial pilot uses owner-only requests, uniform template selection and a short fixed-rate schedule. It is exploratory evidence about the implementation and corpus, even after all checksums pass. Its unsuccessful richer-history runs are not a test of population-derived traffic at the requested million-Pod scale. The next campaign must preserve this result and state precisely which workload or implementation changes it introduces.

The raw bundle currently accompanies this local review. Public archival availability is unresolved; no public artifact URL is asserted.
