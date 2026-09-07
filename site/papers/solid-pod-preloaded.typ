// [GPT-6] Prospective native-index manuscript. No new result binding exists.
#let protocol = json("../../bench/ac/preloaded/protocol.json")
#let workload = json("../../bench/ac/million/workload.json")
#let derived = json("../../bench/ac/million/workload-derived.json")
#let corpus = json("../../research/solid-pod-scale-corpus.json").calibration
#let observations = json("../../research/solid-pod-scale-observations.json")
#let messaging = json("../../bench/ac/million/messaging-volume-config.json")
// Reuse the declared response criteria and number formatter, not previous results.
#let response-criteria = json("../../bench/ac/million/campaign-20260906.json").measurement
#import "solid-pod-scale-results.typ": grouped-integer
#assert(protocol.study_id == "solid-pod-preloaded-v1")
#assert(protocol.preloaded.admission_outcomes_version == 1, message: "Method requires reviewed typed preload outcomes")
#let number(value, digits: 0) = str(calc.round(value, digits: digits))
#let percent(value) = number(100 * value) + "%"
#let demand(population, scenario) = derived.population_rates.find(r => r.pods == population and r.scenario == scenario)
#set document(title: "Access-Controlled SPARQL over Solid Pods with Prepared Native Indexes")
#set page(paper: "a4", margin: (x: 24mm, y: 22mm), numbering: "1")
#set text(font: "Libertinus Serif", size: 10.5pt)
#set par(justify: true, leading: 0.62em)
#set heading(numbering: "1.")
#show heading.where(level: 1): set text(size: 13pt)
#show heading.where(level: 2): set text(size: 11pt)
#show raw: set text(size: 0.88em)
#set table(inset: 5pt, align: left, stroke: (x: none, y: 0.35pt + luma(75%)))
#show table: set par(justify: false)

#align(center)[
  #text(size: 17pt, weight: "bold")[Access-Controlled SPARQL over Solid Pods\ with Prepared Native Indexes]
  #v(0.6em)
  Jesse Wright\ #text(size: 9pt)[SPARQ project]
]

#block(fill: luma(95%), inset: 8pt, width: 100%)[
  #text(size: 9pt)[*Research draft: new measurements pending.* The native preparation
  pilot and final performance matrix are not complete. This manuscript specifies
  the approach and prospective method; it establishes no new capacity result.]
]

#heading(numbering: none, outlined: false)[Abstract]
Can we support efficient, access-controlled SPARQL queries over Solid Pods at
scale? We investigate a single-server design that prepares persistent native
indexes and every Pod's authorization state before serving requests. Queries
reuse these structures; mapped content pages can be read from disk when memory
is constrained. The evaluation combines populated personal-data histories,
equivalent WAC and ACP rights, and demand derived from declared user journeys.
Published observations anchor selected volume distributions and means, while
retention, sharing and backend request rates remain explicit assumptions.
The scope is structured service records and media metadata, excluding original
photos, videos and attachments. The prospective comparison holds the native
representation and CPU allocation fixed, testing constrained memory before
128 and 256 GiB server limits. It measures preparation, complete-population
readiness, local response delivery, durable updates and resource use separately.
A 200 ms local response budget reserves time for network and client work.
New results and the final answer remain pending.

#text(size: 9.5pt)[*Keywords:* Solid, SPARQL, WAC, ACP, native indexes, personal data, benchmarking]

= Introduction

Moving service data into Solid Pods creates a practical query problem. A person
may want appointments involving a contact, spending over a period, or photographs
associated with an activity. These questions cross application boundaries, but
the requester may have permission to read only some resources. SPARQL can express
the joins and aggregates; the server must apply access control to the data on
which those operations depend.

Our question is: *Can we support efficient, access-controlled SPARQL queries over
Solid Pods at scale?* We aim to identify how many populated Pods one machine can
serve at a demand level appropriate to that population, and with what resources.
A capacity claim must therefore name the retained data, offered requests,
response requirement and hardware together. A large namespace with little data,
or low latency for a repeatedly accessed small fixture, answers only part of
the question.

We evaluate a simple division of work. Parse RDF and construct persistent native
indexes during preparation. At startup, open the complete population, prepare
every Pod's authorization state, and keep that state alive. At request time,
locate the Pod, verify the request, and evaluate the query over readable resource
graphs. Native content pages remain mapped and may be reclaimed under memory
pressure. This replaces repeated whole-Pod reconstruction with access to prepared
indexes, while making the population-wide startup and retained-memory costs visible.

The metadata boundary is deliberate. Google Photos describes metadata supporting
both user-facing services and batch processing @photos-spanner. A Solid service
would similarly need to query descriptions of media without reading every
original object. Our corpus includes those descriptions and structured records
from other services. Original media, attachments and their transfer costs are
excluded; complete medical histories and fine-grained sensor streams are also
not modeled. The experiment tests the stated corpus, rather than all bytes a
person might ever retain.

The study contributes a prepared-index query design, a reproducible benchmark
linking personal-data scenarios to offered demand, and a paired evaluation of
WAC and ACP with equal intended rights. The method separates storage and startup
admission from response delivery, and distinguishes mapped content from heap
metadata. Its new empirical findings will occupy the results section after the
source and performance campaign are frozen and the evidence is verified.

= Querying prepared, authorized data

== Access rights define the query dataset

Solid organizes data as Web resources, often under containers @solid. We map
each RDF resource to a named graph. For a verified request context $s$ and a
target Pod $p$, the authorized dataset contains exactly the readable resource
graphs. The unnamed default graph is empty, and every workload template uses
explicit `GRAPH` clauses. A reference query receives the same graph convention
and preserves solution multiplicities and required ordering @sparql.

Authorization must constrain query evaluation, including unbound graph
enumeration, joins, aggregates, `OPTIONAL` and absence tests. Filtering rows after
evaluation is insufficient: hidden data may already have changed a count or
whether a row exists. For example, a calendar recipient can see appointments
but not the owner's private contacts. This query must not reveal those names:

```sparql
PREFIX p: <https://sparq.dev/bench/personal#>
SELECT ?event ?name WHERE {
  GRAPH ?calendar { ?event p:service p:calendar;
                          p:contact ?contact }
  GRAPH ?contacts { ?contact p:name ?name }
} ORDER BY ?event ?name LIMIT 20
```

The inner join has no corresponding row for that recipient. For the owner, both
sides are readable. An absence-test variant can legitimately return the event
when no contact name is readable; its meaning is absence from the authorized
dataset, not a claim that the hidden graph is empty.

WAC determines grants from the effective resource ACL, including container
defaults @wac. ACP combines applicable policies and context matchers with allow
and deny effects @acp. The benchmark compares effective rights, not equal policy
text or equal policy-triple counts. SPARQ exposes their decisions through a
common authorization index and requester-specific view. This creates a reason
to expect similar query work, but preparation cost, retained state and policy
updates can differ. Similarity is an empirical question.

== Prepare once, then retain the complete population

#figure(
  table(columns: (1fr, 1fr, 1fr), align: center,
    [*Prepare on disk* #linebreak() Parse RDF; construct and serialize native indexes],
    [*Start the server* #linebreak() Validate archive; open every Pod; prepare authorization],
    [*Serve requests* #linebreak() Verify identity; select readable graphs; query and respond],
  ),
  kind: image,
  caption: [Preparation, startup and request service are measured separately.
  Native content pages may fault during service; data activation may not.],
) <request-path>

The prepared archive preserves the engine's native dictionary and index encoding
for each resource graph. An archive directory locates those components without
one operating-system file and mapping per resource. Construction consumes the
complete generated RDF corpus specified below. It checks the input
manifest and hashes, then writes a new archive and sidecar; it refuses to replace
an existing archive. Serialization time and both intermediate and native storage
remain part of the preparation account.

Before HTTP listening, workers own disjoint Pod-ID partitions covering every
declared Pod exactly once. They open each native dataset, construct the actual
Pod store, materialize the selected WAC or ACP authorization index, and replay
trusted application journals. Readiness requires the complete declared
population, not a representative subset or the Pods first reached by warmup.
All stores remain alive until that repetition ends.

The archive's byte length and contents remain immutable while any view exists.
Native dictionary and index payloads can use mapped pages; graph and dictionary
descriptors, authorization state and mutation overlays still consume heap memory.
Keeping all Pods prepared therefore does not mean that every content page is
resident, nor that all retained state can spill to disk. These two types of
memory constrain population scale differently and are measured separately.

The request path performs no RDF parse, native dataset opening, journal replay
or initial authorization materialization after readiness. Worker counters are
checked at barriers before and after measurement. A forbidden activation
invalidates the run. Disk page faults in already mapped native data are expected
and remain inside request latency; excluding construction from the request path
does not exclude disk access.

== Authentication and durable updates

The research endpoint accepts Pod-local SPARQL requests with a signed access
token and a fresh request-bound DPoP proof. The measured path verifies these
credentials against a pinned issuer key and checks replay protection. Issuer
discovery, interactive login and an initial TLS handshake are outside the local
HTTP measurement. This endpoint is an implementation interface, not a
Solid-mandated SPARQL route.

Content mutations use the selected authorization engine. Updates affect mutable
overlays and a separate trusted application journal, leaving the mapped archive
unchanged. A successful acknowledgment follows the durable journal record.
Failed updates restore a shared graph snapshot and rebuild authorization;
ambiguous commit or failed recovery prevents further service until offline
replay. Recovery work is timed, counted and distinct from first-time Pod activation.

Both policy variants use the same authenticated owner-only administration path,
restricted to the target Pod's policy resources. Policy changes rebuild effective
authorization and affect subsequently admitted requests. This shared management
check is outside the WAC and ACP engines; the study does not establish ACP ACR
self-access or equivalence of general policy-control authority. Dedicated
recipient, owner and anonymous probes check grants, revocation and restart.
Process-restart durability on local storage is distinct from surviving host loss
or providing replication and high availability.

= A benchmark for personal service histories

== Data types, volumes and the limits of calibration

Each Pod contains communication, contacts, calendar, transactions, activity,
location, media metadata and ratings. Records share identifiers, dates and
relationships so queries can cross domains. Each service month is a named
resource graph; contacts form a retained snapshot. Every member of a generated
population has stored records, including people outside the daily-active cohort.

Google Takeout, Open Banking and Geolife support the selected kinds of records
and relationships @takeout @openbanking @geolife. Export schemas do not establish
how many records people keep. Volume calibration instead uses three limited
observational anchors. MovieLens supplies an empirical retained-rating count
distribution @movielens, reproduced by a checksum-bound extraction and inverse
CDF sampling. It is a historical selected cohort; synthetic rating values and
dates are not replayed observations.

For transactions, the Federal Reserve diary reports about
#corpus.domains.transactions.reported_total_payments_per_month payments per month,
including #corpus.domains.transactions.reported_cash_payments_per_month cash
payments, in #corpus.domains.transactions.source_year @payments. Assuming one
retained record per noncash payment gives an approximate mean. The generator's
base rate is normalized to that mean after applying its synthetic intensity
mixture. This does not validate the retained tail, payment-to-account duplication,
other countries or five-year retention.

Messaging donations motivate a larger-volume sensitivity. Dona reports a median
of #grouped-integer(observations.dona.whatsapp.message_count_median) WhatsApp messages across
#observations.dona.whatsapp.donations donations, with a median span of
#observations.dona.whatsapp.timespan_median_days days @dona. Donors selected their
most important #observations.dona.whatsapp.requested_important_chats.first()
to #observations.dona.whatsapp.requested_important_chats.last() chats. This young,
self-selected sample supports testing substantial retained histories, not a
population distribution or a message-arrival rate obtained by dividing medians.
The separate #grouped-integer(messaging.monthly_records.communication)\-message/month scenario is
motivated by that scale and leaves the central corpus unchanged.

#figure(
  table(columns: (1.05fr, 1fr, 1.65fr),
    table.header([*Domain*], [*Central base count*], [*Evidence status*]),
    [Communication], [#corpus.domains.communication.monthly_records / month], [Assumed; synthetic text excerpts],
    [Contacts], [#corpus.domains.contacts.retained_snapshot_records retained], [Assumed snapshot],
    [Calendar], [#corpus.domains.calendar.monthly_records / month], [Assumed],
    [Transactions], [#corpus.domains.transactions.normalized_unit_intensity_rate / month], [Mean normalized to payment observations],
    [Activity], [#corpus.domains.activity.monthly_records / month], [Assumed daily summaries],
    [Location], [#corpus.domains.location.monthly_records / month], [Assumed episodic records],
    [Media], [#corpus.domains.media.monthly_records / month], [Assumed metadata volume],
    [Ratings], [Empirical count CDF], [Observed retained marginal; synthetic values],
  ),
  caption: [Central history has #corpus.history_months.value months. Base counts
  precede the shared non-rating multiplier; rating counts are sampled independently.],
) <volumes>

The non-rating multiplier assigns
#corpus.population_intensity.classes.map(c => str(c.weight) + "% at " + number(c.numerator / c.denominator, digits: 1) + " times").join(", ").
It imposes positive cross-domain volume correlation. Neither the mixture
frequencies, that correlation, nor the chosen #corpus.history_months.value\-month
retention is empirically validated. The modeled transaction mean is
#corpus.domains.transactions.effective_generator_mean records per month; finite
populations fluctuate around it. Small deterministic prefixes may omit the heavy
class, so larger prefixes change the volume distribution as well as population.

The corpus is therefore partially calibrated, rather than a validated joint
distribution of future Solid data. We preserve the full histories and heavy
classes when increasing population. The compact personal-record fixture is a
separate control. A seeded-literal sensitivity changes text lengths and
compressibility, so it cannot isolate entropy alone. Manifests report actual
record and resource counts, quads, serialized bytes and per-Pod distributions.
No held-out export validation or empirical joint-selectivity calibration has yet
been performed.

== Equivalent rights with useful access patterns

Permissions represent ordinary application roles rather than measured sharing
frequencies. Communication, contacts, transactions, activity and location are
owner-private. One recipient reads calendar resources, four named recipients
read media metadata, and ratings are publicly readable. The owner retains all
content modes. Every tenth calendar or media monthly resource is private; the
compact fixture uses every second resource.

The generator assigns those intended rights before producing either language.
WAC represents media recipients with an `agentGroup` and uses a resource ACL to
shadow inherited access at private exceptions. ACP expands the same group members
into agent matchers and denies inherited recipient access at those exceptions.
This pairs the same effective content permissions while permitting different
policy sizes and preparation costs. Static membership and policy placement are
held constant. Timed owner administration toggles a calendar grant; arbitrary
group churn and additional client or issuer restrictions remain separate cases.

The rights mix exposes private and shared reads, visible-graph enumeration,
inheritance, and exceptions. It avoids a comparison consisting only of owners
who can read everything. An independent intended-rights oracle supplies expected
content modes without calling either policy compiler. Exact query answers are
also compared with physically selected readable RDF. Because that reference
uses the same SPARQL engine, it independently checks selection and enforcement,
not the whole implementation of SPARQL semantics.

== Turning population into offered demand

A million registered people do not imply a million concurrent requests. For
journey $j$, let $u_j$ be actions per active person per day, $r_j$ logical reads
per action, $c_j$ client cache hits, $f_j$ Pod fanout, and $b_j$ batching. With $N$
hosted people and daily-active fraction $a$, foreground query demand is

$ lambda_Q = (N a) / 86400 sum_j u_j r_j (1-c_j) f_j / b_j. $

Background queries, content writes and policy attempts add separately. The
central scenario uses #percent(workload.population.daily_active_fraction)
daily-active people, #derived.journeys_per_active_person_day actions per active
person, and a #workload.offered_multiplier_scenarios.at("busy-period")\-fold
busy-period multiplier. These are scenario choices. Andrews et al.'s measured
mean of #workload.activity_anchor.observed_mean_uses_per_person_day daily device
uses among #workload.activity_anchor.analyzed_participants participants supplies
only an activity-scale check @andrews. Device interactions are not server queries.

#figure(
  {
    set text(size: 9pt)
    table(columns: (1.0fr, 0.5fr, 0.55fr, 0.65fr, 1.8fr),
      table.header([*Journey*], [*Actions/day*], [*Reads/action*], [*Client cache*], [*Conditional query family*]),
      ..workload.journeys.map(j => (
        [#j.id], [#j.actions_per_active_person_day], [#j.logical_reads_per_action],
        [#percent(j.client_cache_hit_fraction)],
        [#j.query_family_weights.pairs().map(x => x.first() + " " + percent(x.last())).join(", ")],
      )).flatten(),
    )
  },
  caption: [Declared request model, preserved from the journey workload.
  Fanout and batching equal one; background and mutations are additional.],
) <journeys>

Each journey uses service-appropriate templates. Communication and purchases
join their records to contacts; calendar stars read events and its absence test
finds events without a readable contact name. Social browsing uses media and
communication records, not a separate modeled social-feed service. Activity uses
an aggregate or a bounded two-edge owner/inverse-owner path. Media reads a bound
record or optional captions. Cross-domain search joins to contacts or enumerates
visible record graphs. Ordered `LIMIT 20` bounds output, not the candidate work
or sorting. These fixed shapes exercise different selectivities, but do not
reproduce every application query or full-text search.

Read requesters are #percent(workload.execution_v2.read_requester_weights.owner)
owners, #percent(workload.execution_v2.read_requester_weights.recipient)
recipients and #percent(workload.execution_v2.read_requester_weights.public)
anonymous. Foreground owner reads select the first ceiling of $a N$ active Pods;
other reads can reach the full population. Within those declared universes,
the final matrix must state its selection distribution. Background reads use
the owner and count records across readable graphs. Their rate of
#workload.execution_v2.background_read_requests_per_hosted_day requests per
hosted person per day is assumed; Google Photos supports including batch work,
not this numeric frequency @photos-spanner. Request mix follows each journey's
derived contribution rather than choosing query templates uniformly.

Writes use actual retained inventory. For non-contact services, daily new and
expired records each equal retained stock divided by the declared retention
horizon. Service-specific batches convert records into requests; modifications
add a fraction of new-record volume. Contacts receive
#workload.execution_v2.contacts_modifications_per_retained_record_year
modification per retained record per year. Half the ingestion is background work
across all Pods and half is assigned to the active cohort. This stationary-stock
model is a workload assumption, not an observed arrival process.

Ingestion inserts complete generated records. Expiry deletes all outgoing triples
of selected records, while modification replaces only `p:value`. The workload
keeps resource topology fixed; it does not create new containers. Actual durable
receipts report changes and no-ops, including modification of a record already
deleted. Each ambiguous client outcome is reconciled with the journal rather
than retried or silently treated as an unsuccessful write. Owner policy attempts
add #workload.execution_v2.policy_attempts_per_hosted_day per hosted person per day.

For the central mixture expectation, the model yields about
#grouped-integer(calc.round(demand(1000000, "daily-average").total_rps)) requests/s at one million Pods,
or #grouped-integer(calc.round(demand(1000000, "busy-period").total_rps)) in the chosen busy period.
These are theoretical demand targets, not measured throughput. Executed write
rates derive from each fully persisted population's actual inventory, including
its observed heavy classes. Source hashes and the component calculation accompany
the schedule. Caching, activity and retention assumptions remain available for
future sensitivity work without relabelling this scenario as observed traffic.

= Evaluation method

The native protocol is prospective: #emph(protocol.status).
The preparation pilot chooses feasible complete populations and constrained
memory tiers before the timed matrix is frozen. Footprint projections can guide
that selection, but extrapolated storage, startup or anonymous-memory estimates
are not measurements at the projected population. The final campaign will name
every selected and unreached cell, its offered rate, repetitions and stop rules.

== Same representation and CPU allocation across RAM tiers

The intended measurement host is one `r7gd.12xlarge` with 48 ARM vCPUs and
384 GiB physical RAM. Sixteen CPU-affinity slots are assigned to server workers
and four disjoint slots to the client. Local NVMe holds the archive and
application journals; the operating-system volume is separate. Exact device
identity, size, filesystem and allocation are retained. Local storage is part
of the tested machine; a baseline gp3 throughput limit is not a general limit
on native paging.

The primary comparison first constrains server memory, then tests 128 and
256 GiB on the same host. Every tier uses the same prepared native representation
and fixed worker count. Changing to a parsed in-memory representation at the
larger tiers would confound the effect of memory. Likewise, worker count affects
partition ownership, queue slots and retained metadata, so CPU is held fixed.
These are CPU-affinity counts, not `cpu.max` quotas. A server cgroup ceiling is
not a claim that a complete deployment fits a physical machine of that size.
The host must also support the client, operating system and preparation tools.

The lower tiers will be chosen from measured preparation and startup footprints.
Each selected complete population must start successfully before receiving timed
requests. A confirmed cgroup-local out-of-memory kill or a still-running unit at
the declared startup deadline is recorded as a negative admission attempt. After
checked cleanup, the frozen grid continues to larger RAM tiers. Startup logs,
actual limits and terminal cgroup counters substantiate the outcome; an exit
signal alone is insufficient. A deadline failure means that startup did not
finish within the allowed time, not that the hardware could never admit that
population. Neither outcome has offered requests or a query latency percentile.
Dropping large Pods or shortening histories cannot turn it into a successful
cell. Million-Pod retained histories remain a target, not a presumed feasible cell.

== Count page-cache memory where it is used

Before each independent cell, the dedicated host synchronizes and drops clean
page-cache state, then starts a fresh limited server cgroup. Complete archive
hashing, validation and population startup occur inside that group. No other
process may warm those files between reset and measurement. Linux charges memory
to the group that instantiates it; moving a prepared process later does not move
the old charges @cgroup. This sequence prevents a preparation process from
quietly supplying file pages outside the measured server ceiling.

A cache drop releases clean reclaimable state, not every possible page @drop-caches.
Archive validation and warmup subsequently read data. The measured condition is
therefore a prepared server after declared warmup, not an untouched cold disk.
We retain anonymous and file-page charges, memory pressure, I/O, faults and
host headroom. A small cgroup limit alone does not demonstrate physical paging;
observed reads and faults must support that interpretation. Conversely, high
RAM does not by itself establish full residency.

Storage accounting separates service records, quads including policies and
structure, serialized N-Quads length, native file length, allocated filesystem
bytes, and journal growth. Native indexes need not have the compression ratio of
the transport frames. File and descriptor counts matter independently of bytes.
Preparation/startup time, CPU, memory and storage are reported alongside request
service, so the cost moved out of the request path remains visible.

== Complete barriers and offered-request accounting

An independent repetition starts from the same immutable archive and fresh
task-owned update journals. Dedicated restart tests deliberately retain journals
and are separate from this reset. After full startup, the server receives a
read-only warmup. Once its client finishes, an authenticated control barrier
pauses middleware admission before body extraction, waits for admitted handlers,
and places a FIFO fence in every worker. Only a completed barrier permits the
before-resource snapshot and timed workload.

The same barrier follows the measured client window. It includes queued work
whose client timed out; client cancellation does not imply cancelled execution.
Connections that never entered middleware are outside the barrier. Worker
snapshots must retain the entire partition and show no new data activation,
journal replay or initial authorization preparation. A drain failure invalidates
timing admission. CPU counter differences span the controlled measurement and
server-drain boundary, rather than assuming that every increment belongs to a
successfully received response.

Requests follow an open-loop Poisson schedule. Intended arrival, dispatch,
complete response, timeout and unsent/drop outcomes remain in the denominator,
avoiding coordinated omission @wrk2. This is a schedule of backend requests,
not a replay of complete sequential user-interface journeys. The principal
latency begins at scheduled arrival and ends after the complete successful body;
the HTTP-only and server production timers are diagnostics. Fast errors do not
establish responsive service. Missing phase or cache headers remain unknown.

The response criteria require at least
#percent(response-criteria.success_fraction) successful responses and
#percent(response-criteria.deadline_fraction_of_all_offered) of all offered
requests successful within #workload.latency_budget.complete_server_response_p95_ms ms.
That local budget leaves room for network transit and
#workload.latency_budget.client_processing_budget_ms ms of client processing
within a one-second journey @sre. Network paths and whole journeys remain
unmeasured; adding component p95 values cannot prove a journey percentile.

Successful-response p95 is conditional on success and is reported beside both
all-offered fractions and HTTP/transport failures. A run without successful
responses has no successful-response p95. A separate conservative queue guard
compares first- and last-quarter queue p95 with required timer coverage. Its
allowance is the larger of
#(response-criteria.queue_stability.allowed_growth_us_floor / 1000) ms and
#percent(response-criteria.queue_stability.allowed_growth_fraction_of_first_quarter)
of the first quarter. A small guard miss with good delivery is reported as such,
not proof of an unstable queue or a need for more RAM. The protocol retains
that criterion without treating its short window as a steady-state proof.

== Correctness and paired inference

Before timing, actual enforcement and query answers are checked against the
intended-rights oracle and physically filtered reference. Deterministic minimum
and maximum source-size representatives cover every observed intensity class.
Accounting separately confirms every persisted Pod and every retained startup
partition. This is broader than a single convenient Pod, but not proof of every
possible answer. Invalid credentials, unauthorized writes, empty ACP matchers,
private exceptions and visible graph enumeration require regression coverage.

Mutation IDs must match exactly one identical durable receipt when committed.
Missing, duplicate or unexpected receipts, wrong query answers, and incorrect
restart state quarantine inference from that source. Among failed startup
attempts, only the evidenced outcomes above permit continuation. Missing or ambiguous failure evidence,
malformed readiness, integrity errors and other execution failures halt the
campaign; global time and disk limits are never reclassified as cell timeouts.
Final analysis requires verified file hashes and matching source, binary,
campaign and review identities. Canonical correctness failures override a
successful summary; disagreement between reported and independently computed
counters or guards makes the affected cell inconclusive. Required source review,
build identity and scoped functional checks precede the final timing freeze.

WAC/ACP repetitions pair the same population, query schedule, requester roles,
memory and CPU allocation, with alternating model order and independent restarts.
Each run retains its own percentile. Paired 95% bootstrap intervals summarize the
ACP/WAC latency ratio; independent runs are not pooled as a single percentile.
The proposed practical-equivalence interval is $[1/1.10, 1.10]$; its margin must
be recorded in the frozen campaign before measurement. Without a frozen margin,
paired intervals remain descriptive. Equivalence requires the complete confidence
interval to lie inside that margin and both models to meet the response criteria
at the comparison load.

Operational capacity is bounded separately by independently established passing
and failing rates. The highest tested passing point is not an exact maximum.
Equal values on a coarse grid cannot resolve a ten-percent capacity difference,
and a ratio of completed requests at the same offered load is not a ratio of
sustainable capacities. The final rate grid and repetition count must be fixed
before performance measurements.

= Results: native campaign pending

No native preparation or timed-campaign result is bound to this manuscript.
The new empirical answer remains open. The final results will report, in order:
the complete populations that can be prepared and retained; response delivery
at population-derived demand; the measured effect of the fixed-CPU memory
tiers; and paired WAC/ACP costs. For each decisive configuration, absolute
successful-response p95 will appear beside all-offered success and timely-success
fractions, with storage and peak server memory. Preparation failure, evidenced
negative admission attempts, unmeasured cells and queue-only guard misses will
remain distinct.

Archive-size projections, a completed startup pilot, or a successful small correctness test cannot
establish million-Pod service capacity. The final abstract and conclusion require
verified native request and resource evidence for the claimed population.

= Scope and related work

The benchmark addresses a single-server query service over personal RDF resources.
It does not measure application migration, federation across providers, arbitrary
policy-language features, global search, original-media delivery, or high
availability. The native archive requires immutable backing bytes while mapped;
durable overlays and process restart do not establish host-crash recovery.
Longer deployments may require compaction and index replacement, whose full
operational cost lies outside the finite request window.

The most consequential data uncertainty is the unobserved joint distribution
of future personal-service histories. Schema fidelity and selected marginal
anchors are useful, but large sensor streams, longer retention, different
sharing rates or less compressible content could change the resource limit.
The stated workload makes these choices inspectable. Its CPU/RAM comparison can
answer what happens under those assumptions; it cannot validate the assumptions
through fast execution.

SolidBench studies link traversal over decentralized data and the structural
assumptions useful to query planning @solidbench. Our question is complementary:
the target Pod is known, and the service must execute over its authorized view
within a resource budget. LDBC's interactive workload motivates relating queries
and updates to application operations @ldbc, while our generated histories and
per-resource rights address a different domain. Neither benchmark supplies an
observed future Solid request rate.

Query-based Linked Data access control @query-ac and SAFE's access-controlled
federation over data cubes @safe demonstrate why permissions belong in query
processing. TIDAL examines controlled sharing of distributed personal data for
health research @tidal. The present evaluation focuses on the resource cost of
one prepared query server, matching effective WAC and ACP content rights. It
does not propose a new policy language or generalize from local measurements
to a distributed deployment.

= Conclusion: empirical answer reserved

The design makes the question testable: prepare native indexes, retain every
Pod's authorization state, and measure complete request delivery at a demand
level derived from the hosted population. Lower memory may trade resident file
pages for disk reads, while retained metadata and startup still limit scale.
Whether that trade permits efficient access-controlled queries over the target
populations, and whether WAC and ACP have practically equivalent cost, awaits
the new measurements. No new capacity conclusion is asserted in this draft.

#heading(numbering: none)[Artifact status]
The accompanying repository contains the corpus calibration, journey workload
and prospective protocol at `bench/ac/preloaded/protocol.json`. Source inputs
and supporting observations remain available in the supplied local study bundle.
Public archival availability is unresolved. This manuscript has no measurement
result input while the native campaign is pending.

#{
  set text(size: 9pt)
  set par(leading: 0.45em, justify: false)
  bibliography(("solid-pod-scale.refs.yml", "solid-pod-preloaded.refs.yml"), style: "ieee", title: [References])
}
