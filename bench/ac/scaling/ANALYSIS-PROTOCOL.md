# Analysis protocol: access-controlled SPARQL over many Solid Pods

**Protocol version:** 1.32

**Frozen before canonical timing:** 2026-09-03

**Code baseline:** `8ad990e679cf11018d5fc6a3860741db9e897034`
**Status:** prospective in-repository protocol; not an externally registered preregistration

This file fixes the research questions, primary outcomes, analysis rules, and resource
limits before the first canonical paper measurement is collected. It is deliberately separate
from a benchmark README: the README will explain how to run the artefact, whereas this
record determines which runs may support the paper.  Once a canonical run exists, this
text is append-only.  Any change must be recorded in the amendment log with its reason
and whether the author had inspected affected outcomes.

## 1. Scope and claims

The study evaluates **read-only SPARQL query processing under Web Access Control
(WAC)**.  The security contract is that query evaluation receives only RDF documents
for which the request principal has `acl:Read`; an unauthorized document must be
observationally indistinguishable from an absent document.  Authentication cost is
measured in the HTTP lane but authentication correctness is not a contribution of this
study.

WAC is the cross-layer experimental scope because it is implemented by both evaluated
paths: the `sparq-solid` materialized authorization view and the native
`sparq-lws-core` `/sparql` endpoint.  ACP and ODRL results, if later collected, are
supplemental and may not be pooled with WAC.  Their semantics are not treated as
interchangeable with WAC.

The phrase *minimal compute overhead* has a fixed operational meaning here.  For a
request whose authorized working set is held constant while unrelated Pods are added,
the materialized/sharded path qualifies only when all four conditions hold:

1. the upper endpoint of the 95% hierarchical-bootstrap confidence interval for
   `median latency(P_max) / median latency(P=1)` is at most **1.10**; and
2. the upper endpoint of the 95% confidence interval for Pod-count elasticity in the
   log-log latency model is at most **0.10**;
3. the corresponding upper confidence endpoint for
   `median process CPU(P_max) / median process CPU(P=1)` is at most **1.10**; and
4. the corresponding upper endpoint for process-CPU Pod-count elasticity is at most
   **0.10**.

If any condition fails, the paper reports the estimate and does not use the phrase
for that path or query family.  A ratio of 1.10 is an engineering relevance margin, not
a law of Solid deployments.  Results outside it can still show a useful but non-minimal
overhead.

The study can establish a scaling envelope for the tested implementation, workload,
and hardware.  It cannot establish the distribution of data or policies in a future
population of Solid Pods; no such population is presently observable.

## 2. Research questions and hypotheses

### RQ1 — authorization correctness

Does each query result equal the independently derived multiset over exactly the
readable documents, for owners, named recipients, strangers, and anonymous requests?

**H1.** Every timed request passes exact multiset equality and all explicit positive,
negative, and revocation probes.  This is a hard gate, not a statistical outcome: one
mismatch invalidates the affected run.

### RQ2 — unrelated-Pod scaling

With the requested Pod's data, policy, query, and result cardinality fixed, how does
request cost change as the number of unrelated co-hosted Pods increases?

**H2a.** A routed collection of independently materialized Pod stores has near-zero
Pod-count elasticity and meets the operational definition above after warm-up.

**H2b.** The current native LWS endpoint does not: because it begins each request by
walking containment from the server root and authorizing every candidate resource, its
work counters and latency increase with total hosted resources even for a graph-bound
query.

H2b is a characterization of the current baseline, not a claim that server-wide
assembly is inherent to access-controlled SPARQL.

### RQ3 — cost decomposition

Which terms dominate cold construction, authorization lookup, authorized-dataset
formation, query evaluation, and result serialization as Pod and document shapes vary?

**H3.** For the materialized-view path, cold materialization grows with policy/data
facts but warm request work is governed by the authorized slice and query.  For native
physical assembly, deterministic storage-operation and allocation counts grow with all
enumerated candidates, while query evaluation is governed by the admitted slice.

### RQ4 — access-control overhead

What is the paired cost of authorized evaluation relative to unguarded evaluation over
the same physically readable data and identical query?

**H4.** A warm materialized view adds no dataset reconstruction and its absolute and
relative overhead is smaller for queries whose own evaluation work is non-trivial.
No binary threshold is attached to H4; paired effect sizes and intervals are reported.

### RQ5 — resident capacity and construction

How do resident memory and cold construction cost change with increasing resident Pod
count and corpus shape?

**H5.** Total resident memory and cold startup work grow approximately linearly with
stored data and policies.  The study deliberately pins request concurrency to one so
that Pod-count effects are not confounded with scheduler and queueing effects; it makes
no concurrent-throughput or saturation claim.

## 3. Implementations under test

The two paths are intentionally not collapsed into one number.

1. **Materialized/routed.** One `sparq-solid::PodStore` per routing shard (the primary
   experiment uses one Pod per store).  WAC is materialized before serving.  The timed
   operation routes by Pod identifier and calls `query_as` through a zero-copy
   `DatasetView`.  A paired plain-engine evaluation over the same admitted graphs is the
   unguarded reference.
2. **Native HTTP assembly.** The real `sparq-lws-core` axum router and `/sparql` handler
   over its in-memory `Store` doubles.  A valid DPoP request traverses authentication,
   containment enumeration, WAC read planning/authorization, RDF parsing, temporary
   dataset construction, SPARQL evaluation, and serialization.  Client-side proof
   signing occurs outside the timed interval.  Backend-operation counters and allocation
   counts supplement wall time.

The in-memory backends intentionally remove network and object-store variance.  This is
a server-compute study, not an end-to-end Internet latency study.  The paper must label
library, in-process HTTP, and any later remote-storage results separately.
For the HTTP benchmark fixture, the in-memory backend's admission limits are derived
exactly from the generated corpus (aggregate content/control bytes and the root plus all
generated resources) and recorded. This disables the backend's small deployment default
as an experimental ceiling without making capacity unbounded or changing endpoint logic.

## 4. Synthetic corpus

### 4.1 Controlled corpus

The generator exposes independent parameters; no aggregate scale factor may stand in
for more than one:

- Pod count `P`;
- origin topology (one authority per Pod versus path Pods under one authority);
- RDF documents per Pod `D`;
- triples per document `T`;
- container depth;
- explicit control-document coverage;
- public/private/named-recipient audience mixture; and
- domain vocabulary (`social` or `health`).

Each RDF document is one named graph.  Audience assignment is deterministic from the
seed.  Private documents inherit an owner grant, public documents receive public read,
and shared documents receive a named-agent read grant.  The common experimental subset
uses no group, client, issuer, deny, ACP, or ODRL construct, so the independent oracle is
a direct predicate over Pod ownership and the assigned audience.  Richer constructs are
separate sensitivity lanes.

Social and health corpora share controlled counts but not predicates or local graph
shape.  This lets the experiment distinguish count effects from a limited form of shape
effect.  Neither vocabulary is described as a sample from deployed Pods.

### 4.2 Calibration and stress labels

- The social count anchor is SolidBench's published default corpus.  A corpus generated
  by SolidBench may be called *SolidBench-derived*.  A corpus that merely matches its
  counts must be called *count-calibrated* and may not inherit claims about LDBC social
  correlations.
- The compact-health participant/file anchor uses TIDAL's 256 Pods and one
  RDF/Turtle file per participant. Its `T=128` setting means 128 generated RDF triples,
  not TIDAL's 128 requested clinical variables; the two units are not equated. It is
  neither TIDAL-derived data nor a longitudinal electronic-health-record model.
- Synthea output, if converted and used, is a structurally rich synthetic-health lane;
  its realized distribution must be reported rather than replaced by planned averages.
- Settings beyond those anchors are called *stress* settings, not population forecasts.

## 5. Experimental matrix

### 5.1 Correctness gate

Before timing, execute both topologies and domains at:

- `P ∈ {1, 8, 32}`;
- `D = 8`, `T = 8`, making all eight query families applicable;
- explicit ACL coverage `∈ {0.10, 1.00}`; and
- corpus seeds `{17, 42, 101}`.

For every setting, exercise owner, named recipient, stranger, and anonymous sessions.
The following must all be non-vacuous and exact:

- allow-all-for-owner;
- allow-one audience class;
- deny-all-for-stranger when the public fraction is zero in the dedicated probe;
- unreadable-versus-absent observational equality;
- explicit-graph and `GRAPH ?g` answers;
- duplicate-preserving multiset equality; and
- revocation after replacing an applicable ACL.

Canonical timing begins only after this gate passes.

### 5.2 Controlled causal blocks

Outside the explicitly two-factor placement/inheritance grid, only one named factor
changes within a block. The owner-controlled Pod-count,
document-count, triple-count, and placement blocks use an all-private audience
(`0/1/0`). This keeps the admitted dataset equal to Pod 0 in both physical lanes; a
shared-origin HTTP fixture would otherwise admit public documents from background Pods
that the routed one-Pod fixture cannot contain. The visibility block varies its audience
explicitly. Both domains and both implementation lanes are run in every block.

| Block | Values |
|---|---|
| Unrelated Pod count | `P = {1, 8, 64, 512, 2048}` with requested Pod fixed; public/private/shared = `0/1/0` so the Pod-0 owner reads exactly `D` target documents and no background-Pod document |
| Documents per Pod | `D = {1, 8, 32, 128, 512}` with `P=16, T=8`, depth 3, and coverage .25 |
| Triples per document | `T = {1, 8, 32, 128}` with `P=16, D=32`, depth 3, and coverage .25 |
| Placement/inheritance | coverage `{0, .10, .25, 1}` × depth `{1, 3, 6}` with `P=16, D=16, T=8` |
| Visibility | public/readable fraction `{.01, .10, .50, 1}` for an anonymous principal with `P=1, D=512, T=8` |

URI topology is exercised as a correctness/generator dimension but is not treated as a
continuous performance factor.  The two implementations fix different physical
boundaries: the primary materialized lane routes to a one-Pod store, whereas the current
native endpoint exposes one shared server authority.  Consequently, varying URI
authorities without changing the physical shard would not identify a Pods-per-origin
effect.  A distributed multi-origin endpoint is outside this study.

The sensitivity blocks select the graph-bound point query and unbound `GRAPH ?g` scan:
these bracket a fixed-graph access and enumeration of the authorized slice while keeping
the secondary campaign tractable.  All eight families remain mandatory in the primary
Pod-count intervention.  Scenario-calibrated cells select the point query and primarily
measure construction/resident scale; they are not substituted into H2.

The large combinations are attempted only after the resource guard in section 8.  A
skipped cell is reported as skipped with the measured guard condition; it is never
silently replaced by a smaller cell.

### 5.3 Query families

Each applicable corpus supplies deterministic valid instances of:

1. graph-bound point lookup;
2. selective star pattern;
3. multiway join;
4. `OPTIONAL`;
5. `MINUS` or `NOT EXISTS`;
6. a bounded property path;
7. aggregate/grouping; and
8. unbound `GRAPH ?g` scan or explicit union-default opt-in.

Query result and intermediate cardinalities are recorded.  An operator family unsupported
by a corpus shape is marked inapplicable, not assigned an empty query.

### 5.4 Scenario-calibrated cells

The mandatory scenario cells, subject to the guard, are:

- social count-calibrated small: 100 Pods;
- social count-calibrated published anchor: 1,531 Pods, approximately 103 documents and
  22 triples per document;
- compact health: 64 Pods × one document × 32 generated triples;
- compact health participant/file anchor: 256 Pods × one document × 128 generated
  triples.

An official SolidBench-derived or Synthea-derived lane may supplement these cells, but
must record generator revision, invocation, conversion code, and realized per-Pod joint
distributions.  It may not replace the controlled causal blocks.

## 6. Measurements

Every raw request record contains enough identifiers to reconstruct its cell and order:
run UUID, source commit, dirty flag, UTC time, host and instance metadata, OS, architecture,
rustc, profile/features, measurement profile, lane, domain, topology, all corpus parameters, corpus hash,
query hash/class, principal class, process block, seed, warm-up/repetition index,
concurrency, configured Rayon workers, and the effective kernel CPU-affinity mask.

Primary outcomes:

- server wall-clock latency in nanoseconds;
- process CPU time where the platform supplies it;
- peak resident set size and resident bytes after construction;
- cold corpus load/materialization time;
- allocation/reallocation operations and requested bytes for isolated, instrumented requests;
- backend query/update/blob operation counts in the HTTP lane; and
- exact result multiset hash, row count, and correctness verdict.

Resident-memory fields describe the whole benchmark process, not a component-level heap
profile. `resident_bytes` is sampled after the generated source corpus has been dropped,
while the physically filtered oracle graph remains live for paired correctness checks;
`peak_resident_bytes` is the Linux process high-water mark and therefore also includes
corpus generation. These are conservative capacity indicators with an explicitly named
instrumentation contribution, not estimates of an isolated production server's heap.
Construction allocation counters close before any oracle graph is built and exclude
corpus-generation allocations; their interval is the store/materialization/route or
LWS-seeding/application construction named by the corresponding phase timers.

Latency and counter observations come from separate builds. The **timing** profile uses
the production server binary's mimalloc global allocator and unwrapped in-memory storage;
allocation and backend-counter fields are null. The **instrumentation** profile delegates
to that same allocator through a counting wrapper and decorates both storage seams; it
records allocation and backend work, but its wall and CPU times are diagnostic and never
enter latency estimates. This avoids charging atomic counter updates and operation-scope
bookkeeping to the implementation under test.

No minimum-of-K value is a primary outcome.  Raw repetitions are retained.  Phase times,
when instrumented, are explanatory secondary outcomes and must sum consistently with the
enclosing server interval up to explicitly named uninstrumented work.

## 7. Repetition, order, and analysis

Each canonical timing cell uses five process blocks with distinct corpus seeds `{17, 42,
101, 314, 2718}`. A timing process block rebuilds the fixture, executes ten untimed warm-up
requests per query/session combination, then records thirty requests. A matching
instrumentation block uses the same corpus and configuration-order seeds, one warm-up,
and one guarded/plain pair. The complete configuration cross-product within each process
block and the paired guarded/plain order are pseudorandomized by recorded seeds.
Client-side JWT and DPoP signing is excluded from timing.  All canonical requests run at
concurrency one with one Rayon worker and an effective Linux affinity mask containing
exactly one logical CPU; this isolates per-request server work but does not model a queued
production workload. Recording an environment variable is insufficient: canonical
validation checks the mask reported by `/proc/self/status`.

Analysis rules:

- summarize medians, p95, p99, median absolute deviation, and empirical distributions;
- report paired guarded/plain median differences and ratios;
- estimate Pod-count elasticity separately for each query family and domain as the
  equal-weight mean of complete-block slopes of block-median `log(latency)` on `log(P)`;
- apply the same estimator and confidence procedure to positive Linux process CPU time;
- use a cluster bootstrap that resamples complete corpus/process blocks, then request
  repetitions within every Pod-count cell;
- report 95% percentile intervals with a fixed bootstrap seed and at least 10,000 draws;
- treat corpus/process blocks, not repeated requests alone, as independent units;
- do not use a non-significant null-hypothesis test as evidence of equivalence;
- do not pool local and EC2 timings, architectures, or instance types; and
- do not pool or compare instrumentation-profile wall times with timing-profile outcomes; and
- retain failed/interrupted partial JSONL and stderr together with the campaign failure
  sentinel and exit status.

The H2 operational test is evaluated separately for each primary query family.  Multiple
families are not pooled into one pass.  Exploratory models and plots are allowed when
labelled exploratory.

## 8. Resource, cloud, and stopping rules

Local generation stops before either free disk falls below **25 GiB** or the greatest
recorded process high-water RSS reaches **70% of physical RAM**. Before every new cell,
the executable guard scans complete and interrupted JSONL construction records and fails
closed on malformed input. Generated corpora and build outputs remain outside the
repository or in ignored result directories. The local harness does not infer future
memory from prior cells; operators must restrict local matrices to known-safe sizes.
Cloud launch configuration additionally imposes a 70%-of-host-memory process/cgroup
ceiling so a first unexpectedly large cell cannot overrun the host before it produces a
construction record.

Cloud execution requires, in order:

1. the repository orphan-instance check;
2. current identity/region verification;
3. a current public on-demand price lookup for the exact instance type, plus an explicit
   conservative reserve for attached storage and public IPv4;
4. a per-run worst-case estimate and an accumulated-study cost ledger;
5. delete-on-termination storage;
6. an instance-local watchdog and an independent termination supervisor; and
7. checkpointed result recovery.

The study-wide AWS ceiling is **USD 100**, including compute and attached storage.  The
operating plan reserves USD 20 contingency and admits at most USD 80 of planned spend.
The stricter repository convention of approximately USD 5 per experimental day remains
the default; exceeding it requires an explicit logged reason while still staying under
the user's ceiling.  Any one instance has a maximum 12-hour lifetime.  Production or
development instances named by repository policy are never stopped, modified, or reused.

The campaign stops when all mandatory controlled and calibrated cells that fit the guard
have five valid process blocks, or when the budget guard prevents another block.  It does
not stop early because an effect is favorable or unfavorable.

## 9. Evidence and publication rules

- Raw JSONL is immutable and checksummed.
- Aggregate tables and figures are generated from raw files by a committed script.
- Every headline number enters `site/src/data/paper-evidence.json` with its source file,
  analysis invocation, commit, and canonical environment.
- Workstation timings are labelled indicative and cannot enter headline tables.
- Deterministic correctness and operation-count records may be canonical when the source
  artefact is pinned and platform independent.
- The paper reports all mandatory cells, exclusions, failures, and protocol amendments.
- The current LWS baseline and the materialized/routed prototype are named separately;
  no result from one is attributed to the other.

## 10. Amendment log

### 1.1 — 2026-09-03: isolate the authorized slice in the Pod-count block

The Pod-count block now states its all-private audience mix explicitly.  A common
10/70/20 mix makes public documents in every background Pod readable by the Pod-0 owner;
that changes both the total deployment and the authorized slice, contradicting RQ2's
fixed-working-set intervention.  The all-private setting keeps Pod 0 non-empty (its owner
reads all `D` documents) while every added Pod is inaccessible to that owner.  Visibility
effects remain in their separately named block.  This correction followed a local,
non-canonical two-Pod smoke run whose outputs were inspected; no canonical run existed and
no favorable or unfavorable outcome motivated the change.

### 1.2 — 2026-09-03: make configuration randomization explicit

The runner now pseudorandomizes the complete lane/domain/Pod-count cross-product within
each process block with a pinned, recorded schedule seed.  The prior deterministic
ascending/reversed order only counterbalanced Pod count and did not fully implement the
section-7 configuration-randomization rule.  This amendment also corrects the written
bootstrap hierarchy to match the implemented nesting: corpus seed, then process block,
then requests.  The changes were made before any canonical timing; only diagnostic
workstation smoke output had been inspected.

### 1.3 — 2026-09-03: separate resident scale from concurrent capacity

RQ5 and the controlled matrix no longer promise a concurrent-throughput experiment or a
partially shared-origin deployment that the two evaluated architectures do not implement
comparably.  The paper instead measures resident memory and cold construction while
pinning concurrency and Rayon parallelism to one.  This makes the estimand the incremental
per-request compute cost of unrelated resident Pods.  URI topology remains a generator
and correctness dimension; physical routing/sharding, rather than URI spelling, is the
architectural intervention.  This scope correction was made after only concurrency-one
workstation smoke output had been inspected.  No concurrency, saturation, or
partially-shared-origin outcome existed, and the change was not conditioned on a measured
effect.

### 1.4 — 2026-09-03: make the block estimand and resampling unit exact

The five primary process blocks now use five distinct corpus seeds rather than reusing
two of three seeds.  Each block is one independently rebuilt corpus/process unit across
the full Pod-count intervention.  H2's elasticity is estimated separately per query
family and domain from complete-block slopes, matching the rule that each family must
pass independently.  The bootstrap resamples those complete blocks and then their
request repetitions; it no longer describes a redundant seed-above-block level.  This
change corrects an analysis-specification mismatch found while testing the validator.
Only the one-block workstation smoke run had been analyzed; no canonical data existed,
and the change was not selected from alternative estimates.

### 1.5 — 2026-09-03: pin secondary-query subsets and visibility semantics

The secondary sensitivity schedule now names its point/graph-scan subset, and the
scenario cells name their point-query scope.  The placement baseline is fixed at 16 Pods.
Visibility is varied at one Pod with 512 documents by changing the public fraction seen
by an anonymous principal, so both implementation lanes receive the same active dataset
and the realized fraction is sufficiently resolved.  This makes the prose exactly match
the committed executable schedule and avoids conflating visibility with the native
endpoint's server-wide semantics.  The change preceded all secondary measurements and
was motivated by design identifiability, not observed timing.

### 1.6 — 2026-09-03: define the memory and allocation accounting boundary

The protocol now states that RSS is a whole-process, conservative capacity indicator:
the post-construction sample excludes the dropped source corpus but includes the paired
oracle graph, and the process high-water mark includes generation. The runner's cumulative
construction-allocation interval now closes before oracle construction in both lanes.
The source-corpus object is dropped before the current-RSS sample, although allocator or
OS retention can keep freed pages resident. This removes a cross-lane accounting mismatch
and prevents the paper from presenting a
harness RSS value as an isolated server heap. The amendment followed schema-2 workstation
smoke inspection; no canonical run existed, and neither the memory nor allocation outcome
was used to choose the boundary.

### 1.7 — 2026-09-03: exercise every query family in the correctness matrix

The correctness matrix now uses eight rather than four generated triples per document.
Applicability metadata from the schema-2 workstation gate showed that `T=4` intentionally
marked four query families inapplicable, leaving only the per-timed-run preflight and the
materialized cross-layer unit test to cover those families. At `T=8`, all eight declared
families execute for every lane, domain, principal, Pod count, ACL coverage, and seed.
This is a coverage correction based only on the declared applicability boundary, not on
latency or an observed correctness failure; no canonical timing had begun.

### 1.8 — 2026-09-03: size the HTTP test-double quota from each corpus

The HTTP fixture now supplies explicit in-memory-store limits equal to each generated
corpus's aggregate body bytes and resource count, and records both limits. The production
test double otherwise defaults to 4,096 entries and 64 MiB: an HTTP sensitivity pilot
correctly returned `507 Insufficient Storage` at 8,192 content documents, so the planned
Pod-count and count-calibrated matrices could not exercise the endpoint. The cap is a
test-double admission setting, not the factor under study. Deriving the smallest sufficient
bound preserves bounded storage while removing this unrelated ceiling. The failed pilot
emitted only applicability metadata and no timing observation; no canonical run existed.

### 1.9 — 2026-09-03: use the Linux high-resolution process CPU clock

Canonical process CPU time now comes from Linux `CLOCK_PROCESS_CPUTIME_ID` rather than
the centisecond-scale tick counters in `/proc/self/stat`. The latter routinely rounds the
materialized microsecond-scale requests to zero and cannot support a useful compute-time
decomposition. The clock call is immediately adjacent to the wall-time interval, reports
`None` on platform or syscall failure, and is required by the canonical validator. This
measurement correction followed only workstation pilots (where CPU time remains absent);
no canonical result existed.

### 1.10 — 2026-09-03: separate latency from deterministic counter instrumentation

Canonical latency now uses an uninstrumented build with the same mimalloc global allocator
as the production LWS binary and ordinary in-memory storage. Allocation and backend work
are collected by a separately compiled instrumentation profile, using the same five corpus
blocks but one measured pair per cell. The counting allocator was also changed from a
`System` delegate to a mimalloc delegate so its allocation behaviour matches the shipping
server more closely. Instrumentation-profile wall and CPU times are explicitly excluded
from latency and overhead estimates. This correction was made after local pilots revealed
that the HTTP lane could execute hundreds of thousands of counted storage calls per
request; charging the atomics, locks, and scope tracking to the server would bias the
effect being estimated. No canonical run existed, and no production-allocator result had
been inspected.

### 1.11 — 2026-09-03: verify CPU affinity and execute the memory stop rule

The raw schema now records the effective Linux `Cpus_allowed_list`, and canonical
validation requires a single logical CPU consistently across every campaign. One Rayon
worker alone does not prevent scheduler migration, so it did not fully establish the
quiet pinned environment promised by the protocol. The run harness now also executes the
70% resident-memory rule before every cell by scanning both complete and interrupted
construction records; malformed records fail closed. The EC2 launcher must pair this with
a hard 70% process/cgroup ceiling. These changes were made before canonical execution and
without inspecting any CPU-affinity or canonical outcome.

### 1.12 — 2026-09-03: make hierarchy depth a lexical WAC-ancestry depth

Generated hierarchy levels are now nested in both the LDP containment relation and the
resource URI path. The earlier generator linked `level-2/` beneath `level-1/` in
containment while spelling them as sibling paths; that did not guarantee that increasing
the factor increased an implementation's URI-ancestor ACL search. A regression test now
requires every child URI to have its declared parent as a lexical prefix. This correction
followed review of generator structure and correctness-only local outputs; no timing or
canonical outcome existed or informed the change.

### 1.13 — 2026-09-03: make the selective-star result non-empty in every primary block

The social and health date filters now select the final quarter rather than December
alone. Direct inspection of the deterministic generator showed that the December filter
returned no row for two of the five predeclared primary seeds at 16 target documents.
The revised threshold retains selectivity while producing three to five target-Pod rows
for every primary seed. This was decided from generated month values and before any
schema-5 timing run; no latency result existed or informed the threshold.

### 1.14 — 2026-09-03: separate TIDAL variable counts from generated triple counts

The compact-health scenario is now described as calibrated to TIDAL only in participant
and file count. The generator parameter `T` counts exact RDF triples; TIDAL reports up to
128 requested clinical variables, and its paper does not justify treating one such
variable as one triple in this corpus. The `T=128` scenario remains a declared compact
stress width but no longer inherits a variable-count claim. This construct-validity
correction preceded all schema-5 timing and was not outcome-driven.

### 1.15 — 2026-09-03: make configured warm-ups and repetitions auditable

Every raw row now records the process's configured warm-up and repetition counts, and
canonical validation requires 10/30 for timing and 1/1 for instrumentation. Counting
emitted observations alone established repetitions but could not establish that the
preregistered warm-up phase ran. This closes that provenance gap before any canonical
timing; schema-5 files remain diagnostic only.

### 1.16 — 2026-09-03: close resume, campaign-labelling, and live-revocation gaps

Canonical validation now rejects a timing/instrumentation profile hidden under the wrong
campaign name and rejects construction, correctness, or applicability records without a
corresponding observed run. Primary cells are resumed only when both their JSONL and checksum
sidecar exist. The native HTTP correctness suite now replaces a live ACL through the authenticated
LDP route and verifies immediate query revocation for the formerly admitted principal. Finally,
the disposable-host launcher gives every launch an idempotency token and matching unique tag, so
cleanup can discover a server-side-successful launch even if its client response was lost. These
are provenance, security-test, and resource-safety corrections; no workload or estimand changed,
and no timing result (local or canonical) had been inspected.

### 1.17 — 2026-09-03: require CPU-time as well as latency evidence for H2

The operational minimal-overhead decision now requires the preregistered 1.10 ratio and
0.10 elasticity margins for Linux process CPU time as well as wall latency. Process CPU was
already a primary recorded outcome, but leaving it descriptive would have allowed a claim about
*compute* scaling to rest on latency alone. The analysis emits both paired cluster-bootstrap
intervals and refuses non-positive canonical CPU samples. This stricter decision was fixed after
schema-6 correctness validation and before any local or cloud timing result was inspected.

### 1.18 — 2026-09-03: reconcile executable controls before the freeze commit

The secondary owner-controlled blocks now state their already-executable all-private
audience setting explicitly; this keeps the admitted dataset at Pod 0 for both the routed
and shared-origin lanes. The resource section no longer claims an unimplemented
two-point memory extrapolation, and the AWS rule now describes the implemented exact
instance-price lookup plus a conservative ancillary reserve rather than claiming a
separate exact storage quote. The header also distinguishes the canonical freeze from
earlier diagnostic pilots. These are prose-to-executable corrections found during the
freeze review; no schema-6 latency or cloud outcome had been inspected.

### 1.19 — 2026-09-03: remove ambiguous freeze-language quantifiers

The four-part minimal-overhead rule now says that failure of *any* condition prevents
the claim, the correctness gate explicitly precedes canonical timing, and the causal-block
description identifies placement/inheritance as the one predeclared two-factor grid.
These wording corrections remove readings that were weaker than the executable validator
or inaccurate about the declared design. They were made during the same freeze review,
before any schema-6 latency or cloud outcome had been inspected.

### 1.20 — 2026-09-03: isolate construction counters and fail fast on host termination

Construction allocation counters now exclude corpus-to-N-Quads rendering and client-side
key/token generation, retaining only Pod-store parsing/materialization/routing or native
store seeding/application assembly. The EC2 supervisor also stops polling when the
transient study service becomes failed or inactive without a sentinel, so a cgroup OOM or
other untrappable process-group failure cannot consume the remainder of the 12-hour
watchdog window. These measurement-boundary and cost-safety corrections were made during
source review before any schema-6 cloud timing existed.

### 1.21 — 2026-09-03: preserve the review base in the disposable-host bundle

The exact Git bundle now includes the frozen review base, the remote clone installs it as
`origin/main`, and the repository preflight is a hard Linux pre-timing stage. This makes
the Bash-4-only privacy delegate executable on the canonical host and ensures preflight
judges the study commit rather than an empty working-tree diff. The change followed a
local bundle-clone rehearsal and preceded all schema-6 cloud timing.

### 1.22 — 2026-09-03: verify checksums before resuming a completed cell

The campaign runner now skips a purportedly complete cell only after recomputing its
SHA-256 digest and matching the nonempty sidecar. A missing half or mismatch aborts the
resume rather than deferring discovery to final analysis. This closes a transfer- and
interruption-integrity gap found in source review before schema-6 cloud timing.

### 1.23 — 2026-09-03: name the implemented failure evidence

The retention rule now names the actual failure evidence: partial JSONL and stderr are
kept with the campaign `FAILED` sentinel and exit status. It no longer implies that each
possibly interrupted JSONL stream can append its own terminal status record. This
prose-to-executable correction preceded schema-6 cloud timing.

### 1.24 — 2026-09-03: permit the official bulk price list as an IAM-free quote source

The launcher's exact-instance price check now first attempts the AWS Price List Query
API and, if that call or its response is unusable, downloads AWS's official public
regional Price List bulk CSV. The fallback requires exactly one Linux, shared-tenancy,
on-demand, `Used`-capacity row for the frozen instance type and region; it records the
offer metadata, selected SKU/rate, source URL, response headers, and full-file SHA-256,
then applies the unchanged USD 80 per-run and USD 100 study ceilings. The first pilot
attempt reached only this pre-provision check: IAM denied `pricing:GetProducts`, no EC2
resource was created, and no benchmark process or outcome existed to inspect. This
amendment therefore restores an independently documented AWS quote path without changing
the workload, estimands, hardware, or spending rules.

### 1.25 — 2026-09-03: remove an unsupported transient-unit property

The disposable-host service retains `MemoryMax=70%` and
`KillMode=control-group`, but no longer passes `MemoryOOMGroup=yes` to
`systemd-run`; Ubuntu 24.04's systemd rejected that assignment as an unknown
transient-unit property. The failed pilot had completed bootstrap and source
verification but the study unit was never created, so no benchmark process or
outcome existed to inspect. Cleanup verified termination of the tagged instance.
The cgroup's aggregate 70% memory ceiling, failure sentinel path, supervisor,
watchdog, and stopping rules are unchanged.

### 1.26 — 2026-09-03: rotate the exact SSH ingress rule when the client IP changes

The launcher now retains the AWS identifier of its one SSH security-group rule and, at
each supervisor poll, compares that rule with the launching client's current public
IPv4. If the address changes, `modify-security-group-rules` atomically replaces the old
`/32`; it never adds a subnet or a second lasting ingress rule, and every rotation is
logged. During the successful pilot this machine's address changed by one host number,
causing three SSH polls to time out while the EC2 system and instance reachability checks
remained healthy. The rule was manually rotated with the new `/32` added before the stale
one was revoked; supervision recovered, all pilot correctness and capacity stages
completed, the manifest verified, and cleanup completed. Only pilot duration and peak
RSS were inspected (11 minutes 12 seconds and 2,581,172,224 bytes against a
23,111,131,136-byte cgroup limit); no latency/CPU ratio, elasticity, or hypothesis outcome
was inspected. This pre-canonical reliability amendment changes neither benchmark
execution nor the experimental estimands.

### 1.27 — 2026-09-03: keep the study credential valid for the longest cell

The paper runner now gives its cell-local access token a 12-hour validity window, equal
to the disposable host's hard watchdog, while retaining the production verifier cache's
independent five-minute validation-freshness bound and minting a fresh DPoP proof before
every request. The first canonical attempt completed the full correctness gate and then
failed in the first timing block for the 2,048-Pod native HTTP health cell: successful
responses continued for more than five minutes, after which query 7 received HTTP 401.
Inspection was limited to the failure sentinel, stderr, and the final records needed to
identify credential expiry; the partial cell and all other timings are invalid and will
not enter analysis. The earlier generic example helper retains its five-minute default,
and a regression test decodes the study token to assert the extended lifetime exactly.
This correction removes an unintended wall-duration censoring mechanism; it changes no
query, dataset, policy, server authorization path, cache policy, outcome, or estimand.

### 1.28 — 2026-09-03: retry setup transport after exact-ingress synchronization

The disposable-host launcher now retries setup-phase SSH and SCP transport failures up
to 20 times, refreshing the same single exact `/32` ingress rule before each attempt.
An SSH session that reaches the host but whose remote command fails is not retried, so
source verification and service-launch failures remain fail-closed. The launch following
amendment 1.27 encountered several rapid client egress-address changes and timed out
during source upload; the study service was never created, no benchmark ran, and tagged
instance cleanup completed. This bounded transport hardening changes neither the host,
benchmark, security-group scope, measurements, nor estimands, and preceded any valid
canonical outcome.

### 1.29 — 2026-09-03: make H4 uncertainty block-aware and register derived-artifact digests

During a blinded audit of the frozen protocol and analysis source, while canonical data
acquisition was still in progress and before any canonical timing or CPU outcome was
opened or inspected, we found that `paired-overhead.csv` reported the preregistered H4
paired medians but did not compute their confidence intervals. The omission conflicted
with Sections 2 and 7, which require paired effect sizes and intervals and identify the
corpus/process block—not an individual repeated request—as the independent unit. The gap
was identified from program structure alone; no observed effect estimate, interval, or
figure informed this amendment.

This amendment changes post-run validation and analysis only. It changes no corpus,
policy, query, implementation path, request order, stopping rule, raw record, exclusion,
or H4 estimand. For every timing-profile cell, H4 continues to report separately the
median of the within-pair guarded-minus-plain wall-time differences and the median of the
within-pair guarded/plain wall-time ratios. The same two estimands are reported for
positive process CPU time when the platform supplies it. Instrumentation-profile wall
and CPU measurements remain diagnostic and are excluded from H4 inference.

For each cell and metric, the 95% interval is now the percentile interval from the
hierarchical cluster bootstrap already prescribed in Section 7. Let the five
corpus/process blocks be the top-level clusters and let each recorded repetition provide
one intact guarded/plain pair effect. In each bootstrap draw, sample five complete block
identifiers with replacement; within each selected block occurrence, sample its paired
request effects with replacement to the block's original repetition count; pool the
sampled effects and take their median. Repeat this procedure at least 10,000 times and
take the 2.5th and 97.5th percentiles. Each pair is resampled once as a vector so wall and
CPU effects share a coherent bootstrap replicate. A SHA-256-derived cell sub-seed from the
recorded base bootstrap seed makes results independent of input-file and record order.
The analyzer rejects incomplete guarded/plain pairs, pair-metadata disagreement,
non-positive ratio denominators, unequal repetition counts across blocks, non-consecutive
repetition indices, and a cell spanning more than one process fixture per block. An
ordinary one-block pilot remains analyzable, but is explicitly marked descriptive-only
and has blank interval fields; the bootstrap is not run. The common campaign block-set
check is applied in canonical mode, where the patched argument validator also rejects
fewer than 10,000 draws and the existing data validator requires all five frozen blocks.

H4 retains no binary threshold and no null-hypothesis significance test. The intervals
are marginal, cell-wise uncertainty summaries; they are not simultaneous confidence
bands over queries or cells. With only five independent blocks, their finite-sample
coverage may be imperfect and this limitation must be stated with the results.

Finally, the analysis manifest now records the byte length and SHA-256 digest of every
derived artifact produced by that invocation, including each generated SVG figure, while
retaining the existing filename list for compatibility. `manifest.json` does not digest
itself, avoiding a recursive digest definition. This provenance addition does not alter
any numerical estimand.

### 1.30 — 2026-09-03: fail-closed causal checks and source-faithful publication claims

After canonical acquisition completed and the retrieved archive and manifest had been
verified, but before any canonical timing, CPU, allocation, backend-counter, construction,
RSS, derived-summary, or result-figure content was opened or inspected, an outcome-blind
peer review and a second bound-by-bound source audit were completed against frozen source
commit `b1c05c08d2279885d63a50f5208a943ad4b942b2`. They identified a missing validation of
the primary causal intervention, publication questions that exceeded the recorded
measures, one incorrect specialization of the native construction bound, and several
missing qualifications in the analytical model. This amendment records conservative
analysis and wording corrections before any empirical outcome is computed or bound.

This amendment changes no raw campaign, corpus, policy, query, principal, implementation
path, factor cell, request order, repetition, measurement, stopping rule, exclusion, H2
estimand, H2 decision margin, H4 point estimand, H4 bootstrap estimator, bootstrap seed, or
minimum draw count. It adds fail-closed pre-analysis validation and constrains the claims
that may be made from the frozen records and implementation.

#### Fail-closed validation of the unrelated-Pod intervention

For both the `pod-scaling` timing profile and the
`pod-scaling-instrumentation` profile, the analyzer groups guarded observations by
`(measurement_profile, lane, domain, query_id, process_block)`. Every group must contain
the complete Pod grid `{1, 8, 64, 512, 2048}`, one `query_hash_sha256`, and one
`(result_hash_sha256, result_rows)` identity across the grid. Lanes, domains, queries,
profiles, and process blocks remain separate because their URI topology, vocabulary,
query, instrumentation, or generated target slice may legitimately differ. Full corpus
hashes and stored-state counts are not cross-P invariants and are expected to change.

Each primary fixture is joined to exactly its construction record by `run_uuid`, with
request repetitions deduplicated at that fixture boundary. Construction records are then
grouped by `(measurement_profile, lane, domain, process_block)` and must cover the same
complete Pod grid with one `(target_readable_documents,
evaluation_readable_documents)` pair. All primary cells use the Pod-0 owner over an
all-private audience, so both counts must equal `documents_per_pod` (16 in the frozen
matrix). This expectation applies to both the origin-per-Pod routed composition and the
shared-origin native-handler composition: the owner reads every target-Pod content
document and no background-Pod content document.

The timing/instrumentation logical-fixture matcher additionally requires equal
`result_hash_sha256` and `result_rows`, as well as the existing corpus and query identity.
Any missing Pod level or drift in query, result multiset identity, result cardinality, or
readable-document count aborts canonical analysis before an estimate is emitted. Tests
cover acceptance of legitimate profile/lane/domain/query/block differences and rejection
of each protected cross-P drift class. The raw schema has no target-Pod byte hash, so exact
byte-level prefix invariance still rests on the frozen generator implementation and tests,
not on a per-record byte attestation.

#### Publication estimands and status of H3--H5

RQ3 is limited to theoretical source-derived decomposition and the measured scaling of
total uninstrumented wall/CPU cost, instrumentation-profile allocation totals, aggregate
backend-operation classes, and recorded cold-construction phase durations. The request
records have no stage-specific clocks. Backend operations and allocations are work-volume
proxies; they cannot identify elapsed-time or CPU shares for authentication, containment,
WAC planning or matching, RDF parsing, N-Quads rebuilding, query evaluation, or result
serialization. Publication text must not say that one such stage “dominates,” “accounts
for” a percentage, or was empirically timed unless a corresponding stage clock exists.

RQ4 reports, within each frozen campaign/lane/domain/query/Pod/block cell, the paired wall
and CPU difference and ratio between guarded execution and its content-only physical
reference. The reference is query-answer-equivalent for the eight generated templates,
but is not physically dataset-identical: a guarded dataset may additionally contain
authorized structural/container and control graphs. Their parse, index, membership, and
graph-enumeration work can enter the contrast. It is therefore a
“guarded-stack/content-reference contrast,” not pure WAC cost, a same-dataset comparison,
or isolated access-control overhead.

The preregistered labels H3, H4, and H5 remain in the audit trail but are published as
exploratory expectations E3, E4, and E5. E3 reports the recorded total and proxy measures
by frozen factor cell. E4 reports the paired difference and ratio by frozen cell relative
to the answer-equivalent content reference. E5 reports cold phase durations and
whole-process current/peak RSS by frozen scenario and cell. H3 and H5 had no operational
threshold; H4 neither operationalized “non-trivial query evaluation” nor specified a
cross-query ordering, and fixed query order was not randomized. The new per-cell intervals
do not create that missing cross-query estimand. E3--E5 therefore receive estimates and
descriptive interpretation only, never supported/rejected, pass/fail, or confirmatory
verdicts. H1 remains a correctness gate and H2 retains its frozen four-part decision rule.

#### Implementation, oracle, and prior-work boundaries

The materialized lane is a benchmark-composed Pod-routed feasibility design built from
production `sparq_solid::PodStore` components; it is not the current native endpoint's
request path. The native lane invokes the current Axum router and handler in process with
`tower::ServiceExt::oneshot`. It includes middleware, server-side authentication, handler
logic, authorized-dataset assembly, evaluation, serialization, and body buffering, but
excludes sockets, TLS, reverse proxies, kernel network transport, and client-side token or
proof construction. Publication wording must therefore use “routed feasibility
composition” and “native in-process handler lane,” not “two production paths,” “network
HTTP latency,” or “full server stack.” Absolute cross-lane latency is not a causal speedup
comparison.

The direct audience predicate selects readable content without either authorization
implementation, but expected and actual answers share the SPARQ evaluator and much of the
canonicalization/serialization path. H1 validates authorization-layer physical selection
for the generated WAC subset and query corpus; it is not an independent SPARQL conformance
or evaluator-correctness test. Live revocation claims must cite the named, checksummed
cross-layer test evidence rather than infer a particular probe from an aggregate
`correctness=true` row.

Novelty claims must acknowledge Dedecker et al.'s graph-centric Pod/SPARQL interfaces,
Werbrouck et al.'s ConSolid permissioned CSS/Fuseki/Express SPARQL satellite using ACL
lookup and `FROM`/`FROM NAMED` rewriting, and Staquet et al.'s proposed incrementally
maintained SPARQL views for Solid agents/aggregators. The defensible novelty is the
controlled marginal intervention on unreadable co-tenants, the comparison of physical
enforcement boundaries, and the correctness-gated, uncertainty-aware benchmark—not
access-controlled SPARQL over Pods, query rewriting, or authorization views themselves.

#### Source-faithful cost-model corrections

The native construction expression already retained
`sum_c k_c^2 ell`, but its linear-in-P specialization was false for the actual shared-root
fixture. Every Pod root is inserted beneath `https://pod.example/`, and the in-memory
metadata store de-duplicates a child by linearly scanning that root's growing vector.
Consequently root insertion performs `Theta(P^2)` string-equality calls, giving
`Omega(P^2)` work and an `O(P^2 ell)` lexical upper bound even though fixed per-Pod state
makes `N_0,B_0=Theta(P)`. A linear specialization would require bounded fan-out or an
indexed child set. This is an implementation/topology-specific construction effect, not
an inherent WAC, Solid, or remote-backend cost; native construction results remain
descriptive.

The native request decomposition must distinguish backend read planning, post-plan ACL
resolution/cache work, local WAC matching, snapshot-lock acquisition, and output
serialization. A parsed-ACL cache hit still performs live metadata confirmation and clones
the parsed triple vector. A miss reads, parses, and inserts the ACL and may scan the
fixed-capacity LRU for eviction. The local matcher remains worst-case `O(a_r^2 ell)`. Blank-node
scoping and aggregate N-Quads formatting/writing belong to the N-Quads rebuild term.

Routed named-graph analysis uses the query-time named-slot count `g_p`, not source graph
count `n_p`. A denied concrete `GRAPH <g>` short-circuits after visibility checking; a
visible concrete target then reaches the named-vector search. The lower bound
`Omega(g_p+Y)` applies to unrestricted unbound `GRAPH ?g` enumeration without the
implementation's recognized `STRSTARTS(STR(?g),prefix)` range restriction, not to every
GRAPH query. Routed `QueryResult` materialization is `O(Y ell)`. Native response
serialization remains symbolic generally with lower bound `Omega(Z)` and specializes to
`Theta(Z)` only for the benchmark's fixed-width, bounded-term templates.

The conditional native `O(P log P)` upper additionally requires aggregate containment
listing `sum C_list(c)=O((N+E)ell)`, `E=O(N)`, and bounded per-candidate plan, ACL
resolution, and policy work; `E=O(N)` alone cannot bound an arbitrary backend's listing
cost. Materialized construction retains lexical factors on graph, ancestor, fact, and
installed-view terms outside the bounded-term intervention. Session-cache space includes
dirty-origin strings and bounded recency-queue key copies as well as live entries.

For comparable simultaneous reads, aggregate request-local transient memory is an
`O(c S_http)` upper bound. The snapshot read guard is constant space per request, but lock
acquisition may wait under contention; the guard covers assembly through serialization
and blocks only matching handler-mediated mutations within the same `LdpState`, not direct
store writers or other server instances. The necessary CPU-capacity condition
`lambda E[S_cpu] < c` assumes stationary work, CPU seconds per request, and `c` available
core-seconds per second; it is not sufficient for queue stability.

Finally, the benchmark oracle lemma is retained with corrected premises: materialized
container graphs carry structural vocabulary, native seeded container RDF bodies are
empty, and ACL graphs carry WAC vocabulary. None contains the generated mandatory social
or health classes, so removal of those graphs cannot change the eight template answers.
All three propositions remain intact with the conditions above. These analytical changes
were selected from frozen source structure, not from an observed effect direction or
magnitude.

### 1.31 — 2026-09-03: correctness-file structure and serialized principal labels

After the canonical run and archive verification had completed, but before any timing,
CPU, allocation, construction-duration, RSS, backend-counter, derived-summary, or result-
figure value had been opened or interpreted, an outcome-blind publisher integration check
identified two structural assumptions that disagreed with the frozen emitter. This
amendment corrects only the publication validator. It changes no raw record, acquisition,
analyzer, experimental factor, oracle, estimand, decision rule, exclusion rule, or
reporting choice.

First, each of the 288 checksummed correctness JSONL files is a complete one-repetition
run containing 26 records, not a one-record file: exactly eight `applicability` records,
one `correctness-gate`, one `construction`, and sixteen `observation` records (one paired
guarded/content-reference observation for each of the eight frozen query templates). The
publisher must extract exactly one gate from each file while also failing closed on an
unknown record type; a missing, extra, or duplicate record; a changed exact record schema;
mixed common run metadata; an incomplete or repeated applicability query; or an incomplete
or repeated query-by-operation observation pair. Performance, construction-time, RSS,
backend-counter, CPU, and allocation values in those files remain opaque to this
publisher-only structural check. The previously frozen totals remain 288 gate records,
5,760 exact result-bag comparisons overall, and 4,608 materialized-lane comparisons.

Second, the runner's command-line principal values `recipient` and `stranger` are labels
accepted by argument parsing and retained in filenames and cell labels; the serialized
`principal` field uses `named-recipient` and `authenticated-stranger`. The complete
serialized principal set is therefore exactly `{owner, named-recipient,
authenticated-stranger, anonymous}` for both lanes. The publisher validates that set and
the corresponding abbreviated cell labels rather than treating filename abbreviations as
record values. These corrections were derived from record types, schemas, and the frozen
source emitter before outcome interpretation.

### 1.32 — 2026-09-03: publication applicability-record cardinality

After the canonical performance outcomes had been inspected and the result narrative had
been reviewed, the first full publication export failed closed because its manifest
cardinality check confused two different quantities. The analyzer correctly recorded
10,240 emitted `applicability` records: every one of the 1,280 raw run files contains one
record for each of the eight frozen query templates, including templates marked
`selected=false`. The publisher incorrectly expected 3,680, which is the number of
selected query instances after applying campaign-specific q1/q8 or q1 selection.

The publisher now derives the expected emitted-record count as 1,280 times the eight-query
workload and retains the exact 10,240 requirement. A regression test supplies the former
3,680 selected-query count and requires rejection. This post-outcome correction changes
only the publication validator and protocol-version gate. It changes no raw or derived
artifact, analyzer byte, result value, confidence interval, H1/H2 decision, exploratory
interpretation, exclusion, cost, figure, or manuscript claim. The two independently
repeated analysis outputs were byte-identical, and the analyzer bytes are unchanged by
this correction. Publication metadata records the clean commit containing amendment 1.32
and the corrected publisher.
