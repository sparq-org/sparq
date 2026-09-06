// [GPT-6] Standalone research draft. Empirical baseline values are source-bound
// JSON extractions; scenario rates are generated from a prospective workload.
// This source does not register or change the earlier publication factory.
#let baseline = json("../../research/solid-pod-scale-baseline.json")
#let workload = json("../../bench/ac/million/workload.json")
#let derived = json("../../bench/ac/million/workload-derived.json")
#let protocol = json("../../bench/ac/million/protocol.json")
#let corpus = json("../../research/solid-pod-scale-corpus.json").calibration
#let old(key) = baseline.values.at(key).value
#let rounded(number, digits: 0) = str(calc.round(number, digits: digits))
#let rate(pods, scenario) = derived.population_rates.find(x => x.pods == pods and x.scenario == scenario)
#let campaign_path = sys.inputs.at("campaign", default: "")
#let campaign = if campaign_path == "" { none } else { json(campaign_path) }
#let anon = sys.inputs.at("anon", default: "false") == "true"
#set document(title: "Access-Controlled SPARQL over Solid Pods on One Machine")
#set page(paper: "a4", margin: (x: 24mm, y: 22mm), numbering: "1")
#set text(font: "Libertinus Serif", size: 10.5pt)
#set par(justify: true, leading: 0.62em)
#set heading(numbering: "1.")
#show heading.where(level: 1): set text(size: 13pt)
#show heading.where(level: 2): set text(size: 11pt)
#show raw: set text(size: 0.88em)
#set table(inset: 5pt, stroke: (x: none, y: 0.35pt + luma(75%)))

#align(center)[
  #text(size: 17pt, weight: "bold")[Access-Controlled SPARQL over Solid Pods\ on One Machine]
  #v(0.6em)
  #if anon [Anonymous submission] else [Jesse Wright\ #text(size: 9pt)[SPARQ project]]
]

#if campaign == none {
  block(fill: luma(95%), inset: 8pt, width: 100%)[
    #text(size: 9pt)[Research draft. The larger benchmark campaign is in progress.
    The reported empirical reference is the earlier compact WAC study; the
    million-Pod capacity and WAC/ACP comparison are prospective, not established results.]
  ]
}

#heading(numbering: none, outlined: false)[Abstract]
Solid gives users control over data held in personal Pods. A useful deployment must
also let applications query that data promptly when many users share a server.
We ask: *Can we support efficient, access-controlled SPARQL queries over Solid Pods
at scale?* We study a simple design: route each request to its target Pod, compile
the Pod's authorization policy, and evaluate SPARQL over a view containing only
readable resource graphs. Persistent storage and a bounded cache separate the
number of hosted Pods from the number active in memory. We evaluate the design
against three requirements: correct answers under changing permissions, interactive
responses at a population-derived offered load, and an explicit single-machine
resource budget. The benchmark generates coherent personal service histories,
distinguishes observed quantities from assumed retention and activity, and emits
WAC and ACP policies with the same intended rights. Original media payloads are
excluded; their metadata remains queryable. An earlier controlled study found
limited unrelated-Pod overhead in the routed path across its tested range, while
the server-wide assembly path failed the same criterion. This supports the routing
decision but does not establish million-Pod capacity. The expanded evaluation is
designed to determine that capacity, including storage, authentication, queueing,
network delay and policy updates, without extrapolating small fixtures into a
deployment claim.

#text(size: 9.5pt)[*Keywords:* Solid, SPARQL, access control, WAC, ACP, multi-tenancy, benchmarking]

= Introduction

A personal data Pod becomes more useful when applications can ask questions across
its contents: upcoming events involving a contact, spending over a period, or
photos associated with an activity. SPARQL expresses such questions over RDF
without prescribing a separate endpoint for every application. On a shared Solid
server, however, query execution must respect each user's resource permissions.
An application must obtain the same authorized answer regardless of how many
unrelated people use the hosting service.

This creates two different scaling problems. First, adding hosted users must not
force each request to inspect their private data. Second, the service must store
those users' data and sustain the requests they actually generate. Solving the first
problem with a small, repeatedly queried Pod does not solve the second. A million
registered names is also insufficient: the Pods must be populated, and requests
must cover a realistic active working set.

Our question is therefore practical: *Can we support efficient, access-controlled
SPARQL queries over Solid Pods at scale?* The target is a single modest machine
holding a million populated Pods, with a two-million-Pod extension if resources
permit. We seek the smallest successful configuration within a stated set of CPU
and memory limits. Success requires correct authorization and useful response
times under a declared service workload. The population target is an experimental
question, rather than an assumed outcome.

The design is straightforward. A request identifies a target Pod. The server
locates its persistent store, loads it into a bounded active cache if necessary,
and applies an authorization view before SPARQL evaluation. WAC and ACP have
different policy semantics, but both can supply the set of resource graphs that
a verified requester may read. The expensive work should depend primarily on the
selected data, query and active working set. Other hosted Pods consume storage,
and may compete for resources through concurrent activity, but need not be scanned
for every query.

The study contributes a concrete evaluation of that design, a reproducible
personal-data workload, and a comparison of equivalent access rights expressed
in WAC and ACP. The workload is central to the contribution. We model retained
service histories, not just a fixed number of arbitrary triples, and derive the
offered request rate from explicit user journeys. Published observations and
public data provide calibration anchors; unsupported frequencies remain visible
assumptions with sensitivity experiments.

The data boundary is structured service records and metadata for photos, videos
and attachments. The benchmark does not store their original binary payloads.
This choice addresses the query and authorization problem while making the
storage boundary explicit. It must accompany any resulting capacity claim.

= Querying an authorized Pod

== Resource permissions and query answers

Solid organizes data as Web resources, commonly arranged in containers within a
Pod @solid. We map each queryable RDF resource to a named graph. Given a verified
request context $s$, a target Pod $p$, and its policy state at time $t$, let
$A(s,p,t)$ be the set of readable resource graphs. The query dataset contains
exactly those graphs; the default-graph convention is fixed by the endpoint and
shared by the reference evaluation. For a query $q$, the expected answer is

$ "answer"(q,s,p,t) = "SPARQL"(q, "view"(p,A(s,p,t))). $

The view must apply throughout evaluation. Removing unauthorized rows only after
the query has run is unsafe: a hidden graph can change an aggregate, a join,
`OPTIONAL`, or `NOT EXISTS`, even when its triples do not appear in the output.
Likewise, an unbound `GRAPH ?g` must not enumerate unreadable graph names. The
reference result is obtained by physically selecting readable content before
evaluating the same query, preserving SPARQL solution multiplicities and required
ordering @sparql.

WAC associates access rules with resources and supports inherited permissions
from containers. The nearest effective ACL determines a resource's rules; absence
of a matching grant denies access @wac. ACP combines applicable policies with
context matchers and allows or denies @acp. These are distinct mechanisms. The
benchmark compares their resulting rights, not policy text or equal numbers of
policy triples. The documents used here are community specifications, and the
query endpoint is an implementation interface rather than a Solid-mandated
SPARQL route.

== A request path that remains local to a Pod

#figure(
  {
    set par(justify: false)
    table(columns: (1fr, 1fr, 1fr, 1fr), align: center,
      [*Verify request* #linebreak() Identity and proof],
      [*Select Pod* #linebreak() Persistent lookup and cache],
      [*Select readable graphs* #linebreak() WAC or ACP view],
      [*Evaluate and respond* #linebreak() SPARQL and serialization],
    )
  },
  kind: image,
  caption: [The measured request includes every stage and server-side waiting.
  A cache miss also includes loading the Pod and preparing its authorization state.],
) <architecture>

The request path in @architecture selects the
Pod before graph enumeration. On a cache hit, the server reuses its indexes and
authorization state. On a miss, it reads the already populated persistent data and
reconstructs the same state. The active cache limits Pod count and retained input
bytes; the complete population is not represented by a resident `PodStore` for
each person. That input budget does not bound materialized heap state: an external
cgroup enforces the actual memory limit. A manifest
allows the generator and loader to account for every persisted Pod.

SPARQ exposes WAC and ACP materialization through a common authorization index
and session-specific dataset view. This makes similar warm query costs plausible:
after permissions have been compiled, the query evaluator can consume the same
readable graphs. It does not imply equal total costs. The languages can produce
different intermediate policy state, materialization work and cache-miss work.
The comparison therefore includes these phases and policy changes, as well as
repeated warm requests.

Authentication precedes authorization. The research HTTP configuration verifies
signed credentials and a request-bound proof using a pinned issuer key. The
measurement includes that verification; issuer discovery, initial interactive
login and the initial TLS handshake are outside the loopback request boundary.
Library calls using an already asserted principal remain component
measurements. They cannot substitute for authenticated server measurements.

The server timer runs from request middleware to completed response production.
It includes body reading, authorization, cache loading and serialization, but not
the socket flush. The client separately measures receipt of the complete response
body. The latter is necessary when assessing a complete-response deadline;
response production alone can underestimate delivery cost under backpressure.

Policy updates require an equally clear boundary. Once a policy-write response
acknowledges success, a subsequently admitted query must use the updated rights.
The implementation must invalidate or replace affected cached authorization state
and preserve the update across eviction and restart. Queries already admitted
when an update occurs need a documented snapshot rule. A fast stale grant is a
correctness failure, not a low-latency response.

The research implementation uses the same authenticated owner-only administration
path for policy writes in both languages. It checks that the requester owns the
target Pod and that the write targets that Pod's policy resources, then records
the update durably and rebuilds affected authorization state. This administrative
check is outside the WAC and ACP engines. The experiment measures its cost and
the effect on subsequent queries, but does not establish ACP ACR self-access or
general equivalence of policy-control authority. Query reads and content writes
continue to use the selected language's authorization engine.

== What should scale

For a warm request, the useful cost decomposition is routing, authentication,
authorization-view selection, query execution, serialization and queueing. A cold
request adds storage reads, index loading and policy preparation. These are
accounting categories, not an assertion that their elapsed times add independently
in a concurrent implementation.

The key prediction is conditional: if the target Pod and its readable data remain
fixed, unrelated stored Pods should not add graph scanning to the warm request.
A server-wide hierarchy traversal violates that condition by visiting resources
outside the selected Pod. A disk lookup can still change with directory size or
storage layout, and a larger active population can reduce the cache-hit rate.
Those effects belong in the experiment rather than being hidden by a constant
time claim.

Total storage necessarily grows with retained content and policy state. Memory
should grow with the active working set within the configured ceiling, but that
ceiling creates cache misses when activity exceeds it. Consequently, low warm
latency and low resident memory can coexist with poor cold performance. The
capacity evaluation must expose all three. We make no timing non-interference
claim: a shared CPU, cache or disk can still reveal contention through timing.

= A workload for personal data

== From service records to populated Pods

We use two kinds of corpus. A compact control preserves the earlier study's small
social and health fixtures for controlled comparisons. The main personal-data
corpus represents retained service histories across communication, contacts,
calendar, transactions, activity, location and media metadata. Records share
identities, dates and relationships so that joins have meaning. Increasing the
population creates additional people with their own stored content; it does not
create empty routes or generate records only when a query first reaches them.

The evidence supports different dimensions with different strength. Google
Takeout documents exportable service categories, contacts as vCard and additional
photo metadata in separate JSON files @takeout. Open Banking supplies transaction
fields and account relationships @openbanking. Geolife supplies a public example
of timestamped location histories @geolife. These help establish the type of data.
They do not establish how many messages, transactions or locations a future Pod
owner would retain.

MovieLens provides a public empirical distribution of retained rating records
@movielens. The generator records its version and checksum and derives the
per-person counts reproducibly. This is one observed marginal, not a model of
an entire person's data. The selected rating-service cohort and its historical
collection limit generalization. In particular, its count distribution cannot
be transferred to email or photo libraries simply by renaming predicates.

Payment volume has a separate mean anchor. The Federal Reserve's payment diary
reports approximately #corpus.domains.transactions.reported_total_payments_per_month
monthly payments per consumer, including
#corpus.domains.transactions.reported_cash_payments_per_month cash payments, for
#corpus.domains.transactions.source_year @payments. We use the difference as an
approximate noncash-payment mean and assume one retained transaction record per
payment. The synthetic intensity mixture is normalized to remain close to that
mean. This does not validate its tail, multi-year retention, other countries,
or duplicate representations across accounts and payment services.

The remaining volume model specifies retention periods and event rates openly.
The central scenario retains #corpus.history_months.value months; lower and heavier
variants vary retention and activity. Table @volumes shows the base rates. A shared
activity factor correlates volumes across
domains, preventing an implausible population in which every service is sampled
independently. That correlation is a modeling choice until joint observations
validate it. The corpus manifest distinguishes such assumptions from observed
source statistics and records the generated distribution rather than just its
mean.

#figure(
  {
    set par(justify: false)
    table(columns: (1.3fr, 1fr, 1.6fr), align: left,
      table.header([*Domain*], [*Base count*], [*Evidence status*]),
      [Communication], [#corpus.domains.communication.monthly_records / month], [Assumed; synthetic text excerpts],
      [Contacts], [#corpus.domains.contacts.retained_snapshot_records retained], [Assumed snapshot],
      [Calendar], [#corpus.domains.calendar.monthly_records / month], [Assumed],
      [Transactions], [#corpus.domains.transactions.normalized_unit_intensity_rate / month], [Normalized to an observed payment mean],
      [Activity], [#corpus.domains.activity.monthly_records / month], [Assumed daily summaries],
      [Location], [#corpus.domains.location.monthly_records / month], [Assumed episodic records],
      [Media], [#corpus.domains.media.monthly_records / month], [Assumed metadata records],
      [Ratings], [Empirical count distribution], [Observed retained marginal; synthetic values],
    )
  },
  caption: [Central retained-history model, before applying the shared activity
  multiplier. Transaction base intensity produces a modeled population mean of
  #corpus.domains.transactions.effective_generator_mean records per month; the
  other numeric base rates and the activity mixture are assumptions.],
) <volumes>

#figure(
  table(columns: (1.05fr, 1.45fr, 1.55fr),
    table.header([*Dimension*], [*Validation evidence*], [*What remains assumed*]),
    [Record types], [Export formats and service schemas], [Future service coverage and mapping choices],
    [Retained volume], [Rating-count marginal and payment mean], [Other rates, retention, tails and deletion],
    [Relationships], [Identifiers, accounts and temporal constraints], [Cross-service correlation and link degree],
    [Sharing], [Named household and collaboration scenarios], [Their frequency, audience size and churn],
    [Demand], [Observed device interaction counts], [Queries per journey, caching, fanout and background work],
  ),
  caption: [Evidence coverage. A validated schema or one empirical marginal does
  not establish the full joint distribution of future personal data.],
) <coverage>

Validation reports per-domain record counts, triples, serialized and indexed
bytes, resource counts, graph sizes, link degrees and query selectivity. It shows
the median and upper tail, including the largest Pods, and checks temporal and
referential constraints. Where export or trace samples permit it, generated
quantities are compared with held-out observations. Where they do not, the paper
reports scenario sensitivity. This distinction is necessary for an honest
representativeness argument: the corpus can cover plausible service data without
being a statistically representative sample of a population that does not yet
exist.

Original photos, videos and attachments are excluded from both storage totals
and request bodies. Their metadata includes the fields needed by the modeled
queries. Structured records are retained at the granularity declared by each
domain; a location summary is not silently treated as a raw continuous GPS trace.
The manifest records that granularity because it can change storage demand by
orders of magnitude.

The present generator also does not reproduce full email bodies, complete clinical
records, fine-grained wearable streams or every application-specific field. Its
monthly resource packaging and evenly distributed synthetic dates do not replay
real seasonal or burst patterns. It is consequently a partially calibrated
service-history scenario, rather than a complete export of all service data. A
capacity result for it must retain that qualification.

== Why these permissions

Private personal records motivate owner-only access. Calendars and selected
documents motivate sharing with named people. Household media and collaborative
collections motivate recipient sets and inherited folder permissions. Public
profiles motivate anonymous reads. A private item inside a shared collection
motivates an exception. These cases exercise the mechanisms applications need
without assuming that every resource has its own independent rule.

The generator first assigns intended rights to resources and recipient identities.
It then emits WAC or ACP policies for the same assignments. For example, replacing
inherited WAC permissions with a private resource ACL may require a different
ACP policy structure to achieve the same effective rights. A deny used for that
equivalent exception is part of the paired scenario. Additional client, issuer
or other language-specific conditions are reported separately.

Recipient count, hierarchy depth, direct-policy coverage and policy reuse vary
independently. Their central frequencies are scenario assumptions, not published
measurements of typical Pods. Reporting only owner reads would make most sharing
rules irrelevant, so correctness probes include recipients, unrelated authenticated
people, anonymous requests and revoked recipients. Performance schedules likewise
identify the requester mix instead of hiding it inside an authorization cache.

== From people to offered requests

The number of hosted users determines neither concurrency nor requests per
second by itself. We start with user journeys and make the conversion explicit.
Let $N$ be hosted people, $a$ the daily-active fraction, and $u_j$ actions per active
person per day for journey $j$. Let $r_j$ be logical reads per action, $c_j$ the
client cache-hit fraction, $f_j$ Pod fanout and $b_j$ the batching factor. The mean
foreground query rate is

$ lambda_Q = N a / 86400 sum_j u_j r_j (1-c_j) f_j / b_j. $

Background reads, content writes and policy writes add separately. A busy-period
multiplier raises the offered rate without changing the stored population.
Fanout is counted as separately authorized target-Pod requests; it does not
assume that one request provides unbounded federation over the whole service.

The reference workload uses #derived.journeys_per_active_person_day journeys per
active person per day across the domains in Table @journeys. This is an assumed
scenario, supported only by a limited activity scale check. Andrews et al.
observed a mean of #workload.activity_anchor.observed_mean_uses_per_person_day
device uses per day in #workload.activity_anchor.analyzed_participants participants
over #workload.activity_anchor.measurement_days days @andrews. Their interactive-state
measure, selected historical cohort and device scope do not estimate modern Solid
traffic. One interaction can trigger no request or several requests. Ofcom's
observations of broad service use motivate multiple domains, rather than a
numerical conversion from time online to SPARQL queries @ofcom. Application
caching and fanout are separate because they can substantially change backend
traffic @memcache.

#figure(
  table(columns: (1.5fr, 0.7fr, 0.7fr, 0.8fr, 0.8fr),
    table.header([*Journey*], [*Actions/day*], [*Reads/action*], [*Client cache*], [*Queries/day*]),
    ..workload.journeys.map(j => (
      [#j.id], [#j.actions_per_active_person_day], [#j.logical_reads_per_action],
      [#rounded(100 * j.client_cache_hit_fraction)%],
      [#rounded(j.actions_per_active_person_day * j.logical_reads_per_action * (1-j.client_cache_hit_fraction), digits: 1)],
    )).flatten(),
  ),
  caption: [Declared central workload per active person. Numeric choices are
  scenario parameters, not measured Solid activity. Background and write requests
  are added separately.],
) <journeys>

With the declared active fraction and background work, this model implies
#rounded(rate(1000000, "daily-average").total_rps) total requests per second at
one million hosted Pods on an average day, and
#rounded(rate(1000000, "busy-period").total_rps) during the chosen busy period.
These are *derived demand targets*, not measured server throughput. The artifact
exposes each component, including writes. Activity, cache effectiveness, fanout,
background synchronization and policy churn are varied to show how the capacity
conclusion depends on them. The same stored population is tested with both uniform
target selection and a skewed active working set.

== Latency includes the journey

The primary server target is a #workload.latency_budget.complete_server_response_p95_ms ms
95th-percentile complete response, including queueing, authentication, authorization,
loading where necessary, SPARQL and serialization. This reserves time for network
transit and client work within the declared one-second interactive journey. The
target is an experimental usability criterion, not a deadline specified by Solid.
User-relevant latency and availability must be measured explicitly @sre.

The network experiments use local, regional and mobile-stress conditions, with
the delay, jitter, bandwidth and loss parameters recorded in the workload file.
They measure whole journeys with one and three sequential requests. Sequential
requests consume the same user budget repeatedly, so the three-request scenario
uses a tighter per-request server budget. It would be incorrect to sum component
95th percentiles and call the result a journey percentile; the experiment measures
the completed journey directly.

= Evaluation method

== Correctness is an admission condition

Before accepting timings, we compare effective rights with an oracle that reads
the generator's policy-neutral records directly. The oracle does not invoke either
policy compiler. Agreement between WAC and ACP alone would be insufficient:
both could make the same mistake. Exact query results are compared with a physical
dataset containing only oracle-readable content. If the same SPARQL engine
evaluates both paths, this establishes independent authorization selection, not
independent implementation of SPARQL semantics.

The tests include direct and inherited grants, private exceptions, public data,
multiple recipients, hidden graph names, joins, `OPTIONAL`, `NOT EXISTS`, paths
and aggregation. They also exercise grants and revocations after warm reads,
cache eviction and process restart. Content-write authorization and the separate
owner-administration boundary are checked for the operations included in the
workload. Invalid, expired or
incorrectly bound credentials must be rejected before query evaluation.

Every persisted Pod is verified against a manifest and checksum. A seeded query
sample spans volume quantiles, domains, policy placement and sharing scenarios;
all distinct generated policy scenarios receive oracle coverage. This verifies
the actual population on disk without claiming that a small sample exhaustively
proves every possible query. Any correctness failure quarantines its affected
run and prevents a capacity claim.

== Storage, working set and service capacity

The scale ladder begins with pilot-sized populations and proceeds to the million-
and two-million-Pod targets where storage, resource and cost ceilings allow it.
The central and heavier personal corpora retain their stated volumes. If a corpus
cannot fit, that outcome is reported with observed bytes and the limiting resource;
its records are not thinned after inspecting performance. Loading time, index size,
policy size, startup and recovery are part of the resource report.

On the query path, a fixed-target experiment first isolates the effect of adding
unrelated stored Pods. The working-set experiments then compare a repeated hot
Pod, first touches, application-cache churn, uniform requests across the population
and skewed activity. Application and operating-system caches are distinguished:
a process restart does not by itself make disk data cold. The manifest records
the actual cache preparation for each run.

Capacity uses scheduled arrivals at rates derived from the population model. A
slow server must not cause the generator to lower the offered rate automatically.
We record the original arrival time, send time and completion time, including
requests that could not be sent, were rejected or timed out. This avoids the
coordinated-omission problem that can make overloaded services appear responsive
@wrk2. Server latency, network latency and generator lag are reported separately,
and all offered requests remain in the success and deadline denominators.

The machine is tested under explicit CPU and memory limits. Memory accounting
includes the operating-system page cache charged to the server's cgroup, alongside
process RSS and PSS. A byte-bounded application cache is useful but cannot alone
establish a memory bound for the service. Swap is disabled. Disk totals include
content, policies, indexes, allocated blocks and required journal state; replication
and backups are outside this single-machine experiment and identified as such.

We select the smallest passing memory tier and then the fewest passing CPUs
within the tested tiers. Storage and price are reported alongside that choice.
This procedure identifies a small tested configuration; it does not establish a
global hardware minimum across all implementations and device types.

== Comparing WAC and ACP

Each language receives identical content, intended rights and offered request
schedule. We alternate their order across independently restarted run pairs and
retain all failed attempts. Warm-up and measurement durations, schedule seeds,
timeouts and the minimum request count are fixed in the protocol before the
canonical campaign. The shipping release profile is used on a quiet dedicated
Linux host; workstation timings remain exploratory.

The practical equivalence margin is a ratio of
#protocol.decision_rules.equivalence_margin_ratio. For each required common-rights
scenario, the confidence interval for the ACP/WAC ratio must lie wholly between
its reciprocal and itself for both complete-response p95 and sustainable goodput.
Both variants must also satisfy the service objective. The margin is a declared
engineering tolerance. A nonsignificant difference, or two similar point
estimates, is not evidence of equivalence.

Confidence intervals resample independent paired runs, rather than pretending
that every request in one run is an independent experiment. The primary comparison
uses the same hardware and offered load. A separate saturation search reports the
highest tested passing rate and the adjacent failing rate, without inventing an
interpolated maximum. Cold loading, policy materialization, memory and update
latency are reported separately; similar warm queries would not erase differences
in those costs.

A tested configuration supports the modeled busy-period service only if all
required runs pass correctness, achieve at least 99% successful completions, and
complete at least 95% of all offered requests successfully within the server
deadline, without growing queues or exceeding the memory ceiling. Read-only
measurements establish read capacity. They do not satisfy the mixed-operation
service criterion by omitting its writes.

The loopback capacity lane also requires complete-body client receipt within
the same deadline for the required fraction of offered requests. This conservative
guard includes generator and local transport delay; the server's response-production
timer alone cannot establish complete-response latency.

= Results and evidence

== What the earlier experiment establishes

The earlier canonical study is retained as a compact WAC reference. It used
#old("process_blocks") independently reconstructed process blocks and
#old("timing_repetitions") timed repetitions per request cell, with population
sizes from #old("pod_counts").first() to #old("pod_counts").last(). Its independent
readable-content selection produced #old("oracle_comparisons") exact result
comparisons, and the correctness gate
#if old("correctness_passed") [passed] else [failed].

For a fixed target Pod, the routed path met that study's prespecified unrelated-
Pod overhead criterion in #old("routed_pass_count") of #old("cells") domain-by-query
cells. The server-wide assembly path met it in #old("native_pass_count") of
#old("cells") cells. The criterion jointly constrained wall time and process CPU,
using the upper confidence bounds of the endpoint ratio and Pod-count elasticity.
These verdicts are generated from the original source artifact, rather than
recomputed with a threshold selected for the new study.

This result supports one decision: select the target Pod before inspecting its
graphs. It does not establish that the original in-memory collection fits a
million populated Pods, that cold requests are interactive, or that ACP has
equivalent costs. Its fixture was a controlled small dataset, its request
concurrency was one, and its native handler was exercised in process. These
boundaries prevent it from answering the new deployment question by extrapolation.

The source of these reused observations is run #raw(baseline.run_id), commit
#raw(baseline.source_commit.slice(0, 12)). The extracted JSON binds each displayed value to a
JSON pointer and the full source SHA-256. The original raw archive, analysis and
long-form paper remain available unchanged.

== Expanded capacity evidence

#if campaign == none [
  The expanded campaign has not yet supplied an admitted result artifact to this
  draft. Consequently, this version reports no measured million-Pod capacity,
  minimum machine size, sustainable mixed-operation rate or WAC/ACP equivalence
  verdict. Those quantities cannot be inferred from the compact reference.

] else [
  #campaign.summary
]

= Discussion

The strongest useful outcome is a bounded capacity statement: a particular
machine stores a particular populated corpus and sustains its declared request
mix under the stated authorization and latency conditions. Every part matters.
Pod count alone ignores retained volume. Queries per second alone ignores
tail latency and failure. A small active cache alone ignores persistent storage
and page cache. The proposed measurements keep these quantities together so a
reader can assess whether their own deployment fits the same conditions.

The representativeness argument is necessarily partial. Public schemas and
selected traces establish that the corpus includes recognizable personal service
data. A rating history supplies an observed volume distribution for one domain,
and a payment diary anchors a transaction mean. The other retained volumes,
cross-domain correlations and future app behavior
remain model assumptions. Sensitivity results show how much the engineering
conclusion depends on them. They cannot turn uncertain assumptions into a
probability sample. Richer consenting export samples could materially change
the inferred storage and service requirements.

The binary-payload exclusion is also substantive. Media metadata can be queried
without transferring original photos or videos, but a complete Pod hosting
service still needs somewhere to store and serve those bytes. Their bandwidth,
backup, replication and durability costs are outside this experiment. The
single-machine result, if obtained, applies to the declared structured data and
metadata boundary; it is not a budget for a complete consumer media service.

Equal effective rights provide the right WAC/ACP comparison, while leaving room
for real differences. One policy language may need more state to encode the same
private exception, or perform more work after a policy change. A shared warm
dataset-view interface makes similar steady-state execution plausible, but only
the paired measurements can establish practical equivalence. A difference in
cold cost or update latency is useful even if repeated warm queries look alike.

Finally, this is a single-machine query study. It does not evaluate high
availability, disaster recovery, denial-of-service resistance or the entire
Solid protocol. Queries that span many Pods add both authorization work and
distributed-query costs; our fanout scenario accounts for target-Pod requests
without claiming to solve arbitrary federation. The authentication configuration
also fixes issuer discovery during measurement. These boundaries identify what
additional work a deployment must provision rather than weakening the measured
request path.

= Related work

Access-controlled querying is an established Linked Data problem. Kirrane et al.
study query-based access control @query-ac, while SAFE combines SPARQL federation
with access control over RDF data cubes @safe. These works motivate enforcing
permissions within query processing. Our focus is the deployment cost of that
enforcement when many personal stores share a modest host, including a bounded
active working set and equivalent WAC/ACP scenarios.

SolidBench evaluates link-traversal querying in decentralized Solid environments
and exploits structural information such as containers and type indexes
@solidbench. Its social workload draws on the LDBC Social Network Benchmark,
which emphasizes correlated data and interactive operations @ldbc. These are
valuable precedents for meaningful RDF relationships and query families. The
present study measures a complementary boundary: the server responsible for
storing and authorizing many co-hosted Pods. Network traversal between servers
and the capacity of one hosting server answer different questions.

TIDAL demonstrates the relevance of distributed personal data for health research
@tidal. A domain-specific research dataset, however, does not by itself estimate
all records retained across a person's services. We therefore keep compact
domain fixtures as controls and add an explicit retained-history and user-demand
model. The contribution is not simply a larger synthetic Pod count, but an
auditable connection between what is stored, who may query it, and how often the
service is asked to respond.

= Conclusion

Efficient access-controlled SPARQL over Solid requires more than a fast query
engine. The request must reach the right Pod, evaluate only readable graphs, and
remain responsive as the stored population and active workload grow. The earlier
controlled evidence supports selecting the Pod before graph enumeration. The
expanded design tests whether persistent storage and a bounded active cache turn
that property into practical single-machine capacity.

#if campaign == none [
  The available evidence establishes the value of Pod-local routing within the
  earlier tested range. Million-Pod capacity remains an open empirical question
  for the larger, partially calibrated service-history workload.
] else [#campaign.conclusion]

#bibliography("solid-pod-scale.refs.yml", style: "ieee", title: [References])
