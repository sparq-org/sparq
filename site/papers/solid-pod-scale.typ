// [GPT-6] Standalone research draft. Empirical baseline values are source-bound
// JSON extractions; scenario rates are generated from a prospective workload.
// This source does not register or change the earlier publication factory.
#let baseline = json("../../research/solid-pod-scale-baseline.json")
#let workload = json("../../bench/ac/million/workload.json")
#let derived = json("../../bench/ac/million/workload-derived.json")
#let frozen = json("../../bench/ac/million/campaign-20260906.json")
#let messaging = json("../../bench/ac/million/messaging-volume-config.json")
#let corpus = json("../../research/solid-pod-scale-corpus.json").calibration
#let pilot = json("../../research/solid-pod-scale-pilot.json")
#let pilot_row(profile, pods, model) = pilot.configurations.find(x => x.label == "pilot-" + profile + "-" + str(pods) + "-" + model)
#let history8 = pilot_row("history", 8, "wac")
#let history64 = pilot_row("history", 64, "wac")
#let entropy8 = pilot_row("entropy", 8, "wac")
#let observations = json("../../research/solid-pod-scale-observations.json")
#let old(key) = baseline.values.at(key).value
#let rounded(number, digits: 0) = str(calc.round(number, digits: digits))
#let rate(pods, scenario) = derived.population_rates.find(x => x.pods == pods and x.scenario == scenario)
#import "solid-pod-scale-results.typ": main-state, main-tables
#let campaign_path = sys.inputs.at("campaign", default: "")
#let campaign = if campaign_path == "" { none } else { json(campaign_path) }
#let campaign_state = main-state(campaign)
#let campaign_ready = campaign_state == "finalized and reviewed"
#let review_root = sys.inputs.at("artifact-root", default: "../../")
#let anon = sys.inputs.at("anon", default: "false") == "true"
#set document(title: "Access-Controlled SPARQL over Solid Pods on One Machine")
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
  #text(size: 17pt, weight: "bold")[Access-Controlled SPARQL over Solid Pods\ on One Machine]
  #v(0.6em)
  #if anon [Anonymous submission] else [Jesse Wright\ #text(size: 9pt)[SPARQ project]]
]

#if not campaign_ready {
  block(fill: luma(95%), inset: 8pt, width: 100%)[
    #text(size: 9pt)[Research draft. Main campaign evidence: #campaign_state.
    The results retain the earlier compact WAC study and completed exploratory HTTP pilot.
    Million-Pod service capacity and WAC/ACP equivalence remain unestablished.]
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
decision but does not establish million-Pod capacity. The HTTP pilot further
shows that compact fixtures conceal a material limitation: whole-Pod loading
under cache churn fails the interactive target for the richer retained histories,
even at small populations. The expanded evaluation separates this working-set
problem from hosted-Pod count. Its measurement boundary covers authenticated
local HTTP requests and queueing, with separate resource accounting. Network journeys
remain unmeasured; the latency target reserves time for network transit and client
work rather than establishing an end-to-end deployment result.

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
holding a million populated Pods, with a two-million-Pod extension. The frozen
campaign separates compact populated-Pod controls at those scales from bounded
retained-history experiments. We report successful configurations within the
selected CPU and memory limits. Success requires correct authorization and useful response
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
assumptions. The frozen sensitivity cells vary message stock and literal
compressibility.

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
exactly those named graphs, with an empty unnamed default graph. Every benchmark
template uses explicit `GRAPH` clauses, and the reference preserves that same
dataset convention. For a query $q$, the expected answer is

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

The request path in @architecture selects the Pod before graph enumeration.
HTTP storage uses one compressed N-Quads frame per Pod and an offset index.
A cache miss reads and decompresses the frame, parses RDF, builds in-memory
indexes and materializes authorization; a hit reuses this state. This is distinct
from the native indexed-file reopening tested in the separate component diagnostic.
The cache limits resident Pod count and retained input bytes. An external cgroup
bounds actual memory, including materialized state and charged file pages.
A manifest accounts for every persisted Pod.

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

The earlier baseline uses small social and health fixtures. The new campaign
uses a distinct compact control containing small personal service records, plus
a larger personal-data corpus representing retained service histories across communication, contacts,
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

Messaging donations provide a further retained-volume anchor. Dona reports median
counts of #observations.dona.whatsapp.message_count_median and
#observations.dona.facebook.message_count_median messages from
#observations.dona.whatsapp.donations WhatsApp and
#observations.dona.facebook.donations Facebook donations, with median spans of
#observations.dona.whatsapp.timespan_median_days and
#observations.dona.facebook.timespan_median_days days, respectively @dona.
WhatsApp donors were asked for their most important
#observations.dona.whatsapp.requested_important_chats.first()–#observations.dona.whatsapp.requested_important_chats.last()
chats; Facebook exports covered all chats. This young, self-selected cohort
supports testing substantial multi-year histories, not a population distribution.
It motivates the frozen #messaging.monthly_records.communication\-message/month
sensitivity while preserving the central scenario. Dividing these separate
medians would not estimate an observed arrival rate; text and binary sizes remain
outside this count-based evidence.

The remaining volume model specifies retention periods and event rates openly.
All frozen retained-history variants use #corpus.history_months.value months.
@volumes shows the base rates. The shared intensity mixture assigns
#corpus.population_intensity.classes.map(c => str(c.weight) + "% at " + rounded(c.numerator / c.denominator, digits: 1) + "×").join(", ").
It imposes cross-domain volume correlation; neither the class frequencies nor
that correlation is empirically validated. The corpus manifest records the
resulting distribution. Retention and activity sweeps remain future experiments.

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

The manifest records per-domain counts, triples, serialized bytes, resource
counts and the largest Pods. Joint link-degree and selectivity calibration,
held-out validation, and retention, activity and sharing sweeps remain outside
this campaign. The available observations support a partially calibrated scenario,
not a statistically representative joint distribution of future Pod contents.

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

The intended rights are fixed, application-motivated scenarios. Communication,
contacts, transactions, activity and location are owner-private. The owner retains
all content modes; one named recipient may read calendars, and
#messaging.group_members named recipients may read media metadata. Ratings are
publicly readable. One in every #messaging.private_exception_every calendar or
media monthly resources is owner-only; the compact fixture uses every second
resource.
These choices model private records, sharing and exceptions without claiming
measured policy frequencies.

The generator assigns these rights first, then emits WAC or ACP. WAC encodes
shared media with `agentGroup`; ACP expands the same fixed members into agent
matchers. A private resource ACL shadows inherited WAC grants; the equivalent
ACP exception denies inherited recipients. Membership, hierarchy and policy
placement remain fixed during timing. Shared owner administration changes the
selected calendar grant, with recipient, owner and anonymous probes before and
after revocation, eviction and restart. This does not test dynamic group membership
or additional client/issuer conditions. Correctness tests include unrelated
requesters; the performance identity mix is explicit below.

== From people to offered requests

The number of hosted users determines neither concurrency nor requests per
second by itself. We start with user journeys and make the conversion explicit.
Let $N$ be hosted people, $a$ the daily-active fraction, and $u_j$ actions per active
person per day for journey $j$. Let $r_j$ be logical reads per action, $c_j$ the
client cache-hit fraction, $f_j$ Pod fanout and $b_j$ the batching factor. The mean
foreground query rate is

$ lambda_Q = N a / 86400 sum_j u_j r_j (1-c_j) f_j / b_j. $

Background reads, content writes and policy writes add separately. A
#workload.offered_multiplier_scenarios.at("busy-period")× busy-period multiplier
raises the offered rate without changing the stored population.
Fanout is counted as separately authorized target-Pod requests; it does not
assume that one request provides unbounded federation over the whole service.

The reference workload uses #derived.journeys_per_active_person_day journeys per
active person per day across the domains in @journeys. This is an assumed
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

A Google Photos engineering account reports media and product metadata in
Spanner, serving both interactive services and batch processing @photos-spanner.
This supports treating metadata as a substantial query workload and including
background work. It supplies no per-person request rate for Solid; the background
frequencies below remain scenario assumptions.

#figure(
  {
    set text(size: 9pt)
    table(columns: (1.1fr, 0.6fr, 0.6fr, 0.6fr, 0.65fr, 1.45fr),
    table.header([*Journey*], [*Actions/day*], [*Reads/action*], [*Client cache*], [*Queries/day*], [*Conditional query mix*]),
    ..workload.journeys.map(j => (
      [#j.id], [#j.actions_per_active_person_day], [#j.logical_reads_per_action],
      [#rounded(100 * j.client_cache_hit_fraction)%],
      [#rounded(j.actions_per_active_person_day * j.logical_reads_per_action * (1-j.client_cache_hit_fraction) * j.pod_fanout / j.batch_factor, digits: 1)],
      [#j.query_family_weights.pairs().map(x => x.first() + " " + rounded(x.last() * 100) + "%").join(", ")],
    )).flatten(),
    )
  },
  caption: [Declared central workload per active person. Numeric choices are
  scenario parameters, not measured Solid activity. Background and write requests
  are added separately.],
) <journeys>

With #rounded(100 * workload.population.daily_active_fraction)% daily-active people,
#derived.per_hosted_person_daily_requests.at("background-read") background reads
and #derived.per_hosted_person_daily_requests.at("policy-write") policy attempts
per hosted person per day, this model implies
#rounded(rate(1000000, "daily-average").total_rps) total requests per second at
one million hosted Pods on an average day, and
#rounded(rate(1000000, "busy-period").total_rps) during the chosen busy period.
These are *derived demand targets*, not measured server throughput. The artifact
exposes each component, including writes. The frozen campaign keeps activity,
client caching, fanout and background rates fixed and selects target Pods uniformly.
Changing stored population and cache size exposes working-set effects. Skewed
activity and broader demand or policy sweeps remain outside these measurements.

The pilot exercises eleven concrete templates: a bound record lookup, all-visible
record count, transaction star, communication/contact join, per-service aggregate,
activity filter, optional media caption, calendar/media union, media without a
caption, communication thread path, and graph enumeration. A bounded lookup
selects a named resource and record; filters and joins examine matching candidates
within readable graphs; counts and graph enumeration cover all visible matching
records. Ordered `LIMIT 20` bounds returned rows, not the work required to find or
sort them. Selectivity must consequently be reported with readable cardinalities.

For example, this cross-domain query displays messages with names drawn from the
contact service. If a recipient can read a message but cannot read its contact
graph, the inner join produces no corresponding row. The query demonstrates why
permissions must constrain both sides before the join:

```sparql
PREFIX p: <https://sparq.dev/bench/personal#>
SELECT ?message ?name WHERE {
  GRAPH ?messages { ?message p:service p:communication;
                            p:contact ?contact }
  GRAPH ?contacts { ?contact p:name ?name }
} ORDER BY ?message ?name LIMIT 20
```

The main workload selects journeys by their derived server-request contribution,
then a service-appropriate template using the conditional mix in @journeys.
Communication and purchases join their own records to contacts; calendar stars
read appointments and its `NOT EXISTS` variant selects appointments without a
readable contact name. Social browsing uses media and communication/contact
records, since no separate social-feed service is modeled. The activity path is
a bounded two-edge owner/inverse-owner path within a monthly graph; its aggregate
counts activity and sums values. Media uses a bound lookup or optional captions.
Cross-domain search joins records to contacts or enumerates visible record graphs.
The pilot's uniformly selected templates do not implement these journey weights.

Read identities in the main scenario are
#rounded(100 * workload.execution_v2.read_requester_weights.owner)% owners,
#rounded(100 * workload.execution_v2.read_requester_weights.recipient)% recipients
and #rounded(100 * workload.execution_v2.read_requester_weights.public)% anonymous;
background synchronization and mutations use the owner. These are declared
sharing scenarios, not measured user proportions.

Writes are tied to retained inventory. For each service other than the contact
snapshot, the stationary-retention scenario assigns daily new and expired record
counts equal to its stock divided by the retention horizon. Service-specific
batch sizes convert these into HTTP writes; modifications add a declared rate
per new record. Contacts receive
#workload.execution_v2.contacts_modifications_per_retained_record_year modification
per retained contact per year. For the central theoretical stock mean, this
produces about
#rounded(derived.stock_based_writes.per_hosted_person_daily_content_requests.ingest, digits: 2)
ingestion,
#rounded(derived.stock_based_writes.per_hosted_person_daily_content_requests.expire, digits: 2)
expiry and
#rounded(derived.stock_based_writes.per_hosted_person_daily_content_requests.modify, digits: 2)
modification requests per hosted person per day. The executed schedule derives
these rates from the actual persisted inventory; this manuscript's population
projections use the declared mixture expectation. Balanced stock is an expectation,
not a guarantee for a finite run. Receipts must record actual inserted and deleted
records and no-ops. This finite-window workload keeps resource topology fixed;
creating new resources or containers requires a separate experiment.

== Latency includes the journey

The primary server target is a #workload.latency_budget.complete_server_response_p95_ms ms
95th-percentile complete response, including queueing, authentication, authorization,
loading where necessary, SPARQL and serialization. This reserves time for network
transit and client work within the declared one-second interactive journey,
including a #workload.latency_budget.client_processing_budget_ms ms client allowance.
These are experimental usability budgets, not deadlines specified by Solid.
User-relevant latency and availability must be measured explicitly @sre.

The workload also declares local, regional and mobile-stress network profiles,
but the current campaign measures local HTTP requests only. Its network and
client allowance remains a budget, not a measured journey result. A deployment
evaluation must measure whole journeys under the declared delay, jitter, bandwidth
and loss conditions. Sequential requests consume the same user budget repeatedly;
the proposed three-request journey therefore has a tighter per-request budget.
Summing component 95th percentiles would not establish a journey percentile.

= Evaluation method

Frozen campaign #raw(frozen.campaign_id) defines the selected cells in
@campaign-cells.
The earlier baseline and short HTTP pilot remain separate experiments.
The full host is a #raw(frozen.host.instance_type) with
#frozen.host.physical_vcpus vCPUs, #frozen.host.physical_memory_gib GiB RAM and
a #frozen.host.volume_gib GiB volume. Server and client use disjoint CPU sets.
A small server quota is a measured process-and-cache allowance, not the total
host RAM requirement or evidence that every request performs physical disk I/O.

Each cell schedules #frozen.seeds.len() independently restarted WAC/ACP pairs with
alternating order and fresh journals. A #frozen.measurement.warmup_seconds s
constant-arrival owner-read warm-up precedes at least
#frozen.measurement.measurement_seconds s and #frozen.measurement.minimum_offered
Poisson-scheduled requests, with a #frozen.measurement.request_timeout_ms ms timeout.
The client admits at most #frozen.measurement.maximum_inflight requests in flight;
each worker queues at most #frozen.measurement.queue_capacity_per_worker requests.
Swap is disabled. These controls do not change the declared demand assumptions.

#figure(
  {
    set text(size: 8.5pt)
    table(columns: (1.25fr, 1.65fr, 0.55fr, 0.8fr, 0.8fr), inset: 4pt,
      table.header([*Experiment*], [*Corpus and stored Pods*], [*CPUs*], [*GiB*], [*Offered rps*]),
      ..frozen.groups.map(g => (
        [#(
          "compact-fixed-population": "Fixed-load population",
          "population-control": "Population demand",
          "compact-rate-bracket": "Compact rate bracket",
          "retained-history-cold": "History cold/churn",
          "retained-history-larger": "History/size stress",
          "retained-history-hot": "History hot set",
        ).at(g.id)],
        [#g.datasets.map(d => d.replace("-", " ")).join(", ")],
        [#g.cpus.map(str).join(", ")], [#g.memory_gib.map(str).join(", ")],
        [#if g.at("rates", default: none) == none { "inventory-derived" } else { g.rates.map(str).join(", ") }],
      )).flatten(),
    )
  },
  caption: [Frozen selected cells, not a full cross-product of resource tiers.
  A listed cell can remain unmeasured under the recorded admission and stopping
  rules. Only compact controls reach the million-Pod targets in this matrix.],
) <campaign-cells>

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

Every persisted Pod is counted in a checksummed inventory. Before a dataset is
eligible for timing, the runner selects the smallest and largest serialized Pod
in every observed intensity class, using Pod ID to break ties, and checks those
representatives against the rights/result oracle. Dedicated HTTP probes check
policy changes. Inventory accounting is exhaustive; query verification is sampled
and does not prove every possible answer. A wrong authorization result, durable receipt or
replayed state quarantines all inferential results from that source. A resource
or admission failure instead rejects its dataset/configuration; it does not
invalidate independently verified compact controls.

== Storage, working set and service capacity

The frozen compact storage ladder calls for
#frozen.corpora.filter(c => c.profile == "smoke").map(c => str(c.pods)).join(", ")
physically populated Pods per policy language. Only the populations in
@campaign-cells receive HTTP load cells. Retained-history cells are limited to
#calc.max(..frozen.corpora.filter(c => c.profile != "smoke").map(c => c.pods)) Pods;
no million-Pod retained-history run is included. Records are not thinned after
observing performance. Admission failures and unreached cells remain visible.
The earlier baseline supplies the fixed-target isolation evidence.

Each main run begins with an operating-system cache drop and a fresh server,
then the declared owner-read warm-up. Measurement therefore starts from a recorded
post-warm-up state, not a guaranteed cold cache. Uniform target selection over
the bounded active cache creates misses as the population grows. Cache headers
and loading phases expose those events. There is no separate skewed or first-touch
sweep. The eight-Pod hot set is a fixed prefix and does not cover every intensity
class in the larger population.

All cells retain the journey and mutation mix. Population-demand cells derive
the rate from stored compact inventory; the other cells use the fixed diagnostic
rates in @campaign-cells. Open-loop scheduling records intended arrival, dispatch
and complete response, retaining unsent, rejected and timed-out requests. This
avoids coordinated omission @wrk2. Server production time, complete local HTTP
time and generator lag are separate; network journey latency is unmeasured.

Cgroup accounting includes charged file pages, with process RSS/PSS reported
separately. Memory peak includes warm-up; admission requires both resource
snapshots and no OOM. Content, policies, indexes and measured allocation are
reported; pre-load disk snapshots exclude subsequent journal growth. Audits check
durable receipts and replay after each run. The hot-set ladder stops at the first
memory tier passing both languages in all repetitions. Other resource cells are
fixed; there is no independent search for the fewest CPUs. Reported small quotas
identify tested configurations, not a global minimum or a complete hosting budget.

== Comparing WAC and ACP

Each language receives identical content, intended rights and offered request
schedule. We alternate their order across independently restarted run pairs and
retain all failed attempts. Warm-up and measurement durations, schedule seeds,
timeouts and the minimum request count are frozen before the main campaign.
The shipping release profile runs on the dedicated Linux host; workstation
timings remain exploratory.

The practical equivalence interval for an ACP/WAC ratio is
$[1 / 1.10, 1.10]$. For each required common-rights scenario, the confidence interval
must lie wholly within these bounds for both complete-response p95 and sustainable goodput.
Both variants must also satisfy the service objective. The margin is a declared
engineering tolerance. A nonsignificant difference, or two similar point
estimates, is not evidence of equivalence.

Confidence intervals resample independent paired runs, rather than pretending
that every request in one run is an independent experiment. The primary comparison
uses p95 at the same hardware and offered load. The compact rate grid bounds
operational capacity separately for each language under the same service objective. Each language supplies a highest tested passing rate and, where
reached, a consistently failing upper rate. Equal completion rates
at a shared low offered rate are capped by the client and cannot establish
capacity equivalence. Let the passing and failing bounds be $L_W, U_W$ for
WAC and $L_A, U_A$ for ACP. Under the stated monotonic-capacity assumption, the
conservative ratio bounds are $[L_A/U_W, U_A/L_W]$. They must lie within the
equivalence margin: identical highest passing points on a coarse grid are
insufficient. A paired confidence interval for the tested-grid statistic does
not narrow untested intervals. Missing bounds or repetitions, mixed pass/fail
repetitions, or nonmonotone results leave capacity equivalence unestablished.
The frozen grid's adjacent rates are too widely separated to establish this
10% capacity-equivalence margin, even if both languages share the same passing
grid points. Its capacity result is a bound, not a resolved equivalence test.
Loading, materialization and update costs remain separate diagnostics.

A tested configuration supports the modeled busy-period service only if all
required runs pass correctness, achieve at least
#rounded(100 * frozen.measurement.success_fraction)% successful completions, and
complete at least #rounded(100 * frozen.measurement.deadline_fraction_of_all_offered)%
of all offered requests successfully within the deadline and memory ceiling.
The queue guard requires #rounded(100 * frozen.measurement.queue_stability.required_timer_coverage_fraction_of_offered)%
timer coverage. Last-quarter queue p95 may exceed first-quarter p95 by at most the
larger of #rounded(frozen.measurement.queue_stability.allowed_growth_us_floor / 1000) ms
and #rounded(100 * frozen.measurement.queue_stability.allowed_growth_fraction_of_first_quarter)%
of that first-quarter value. Read-only
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
The endpoint ratio could increase by at most
#rounded(100 * (old("endpoint_ratio_threshold") - 1))%, and the elasticity upper
bound had to remain at most #old("elasticity_threshold"). Elasticity is the slope of log cost against log Pod count. @baseline-effects
summarizes the original cell estimates and intervals; the reused verdicts retain
the original thresholds.

#figure(
  {
    set text(size: 9pt)
    table(columns: (1.2fr, 0.7fr, 1fr, 0.85fr, 1.1fr, 0.85fr),
      table.header([*Path*], [*Metric*], [*Ratio range*], [*Largest upper CI*],
                   [*Elasticity range*], [*Largest upper CI*]),
      ..baseline.effect_size_summary.map(x => (
        [#if x.lane == "materialized-routed" [Routed] else [Server-wide]],
        [#if x.metric == "wall" [Wall] else [CPU]],
        [#rounded(x.endpoint_ratio_range.first(), digits: 3)–#rounded(x.endpoint_ratio_range.last(), digits: 3)],
        [#rounded(x.maximum_endpoint_ratio_ci95_high, digits: 3)],
        [#rounded(x.elasticity_range.first(), digits: 4)–#rounded(x.elasticity_range.last(), digits: 4)],
        [#rounded(x.maximum_elasticity_ci95_high, digits: 4)],
      )).flatten(),
    )
  },
  caption: [Earlier WAC study: endpoint cost ratios and log–log elasticities across
  all #old("cells") domain/query cells per path. Ranges span cell estimates;
  “largest upper CI” is the maximum of their separate 95% confidence bounds,
  not a pooled confidence interval.],
) <baseline-effects>

This result supports one decision: select the target Pod before inspecting its
graphs. It does not establish that the original in-memory collection fits a
million populated Pods, that cold requests are interactive, or that ACP has
equivalent costs. Its fixture was a controlled small dataset, its request
concurrency was one, and its native handler was exercised in process. These
boundaries prevent it from answering the new deployment question by extrapolation.

The source of these reused observations is run #raw(baseline.run_id), commit
#raw(baseline.source_commit.slice(0, 12)). The extracted JSON binds each displayed value to a
JSON pointer and the full source SHA-256. The accompanying review bundle contains
the #link(review_root + baseline.source)[original summary] and
#link(review_root + baseline.accompanying_artifacts.directory + "/raw-sanitized.tar.zst")[raw archive],
alongside the unchanged long-form source. These are local artifact links; public
archival availability remains unresolved.

== Expanded capacity evidence

#if not campaign_ready [
#let pilot_order = (("smoke", 8), ("smoke", 64), ("history", 8), ("history", 64), ("entropy", 8))
The completed HTTP pilot used source commit #raw(pilot.source_commit.slice(0, 12)).
Its input checksums and all request sequences pass the extractor's integrity
checks. Each configuration offered #history8.requests.offered requests at
#history8.requests.settings.rate requests/s, selected Pods uniformly and chose
uniformly among the eleven query templates. Reads and existing-value replacements
used the owner identity. The pilot host records
#pilot.host_environment.at("CPU(s)") #pilot.host_environment.at("Model name") CPUs
(#pilot.host_environment.Architecture); one HTTP worker served each language in a fixed order,
with a cache of #history8.server.settings.at("cache-pods") Pods and
#rounded(int(history8.server.settings.at("cache-bytes")) / calc.pow(2, 20)) MiB of
encoded input. These are pilot controls, not the population-derived canonical mix.

#figure(
  {
    set text(size: 9pt)
    table(columns: (0.9fr, 0.4fr, 0.45fr, 0.65fr, 0.65fr, 0.85fr, 0.85fr),
      table.header([*Corpus*], [*Pods*], [*Policy*], [*OK/offered*], [*Within* #linebreak() *200 ms*],
                   [*Reply p95 (ms)*], [*Server p95 (ms)*]),
      ..pilot_order.map(pair => ("wac", "acp").map(model => {
        let c = pilot_row(pair.at(0), pair.at(1), model)
        let r = c.requests
        (
          [#pair.at(0)], [#c.pods], [#upper(model)], [#r.successful/#r.offered],
          [#r.successful_within_scheduled_deadline/#r.offered],
          [#rounded(r.latency_us.successful_complete_body_from_scheduled_arrival.p95 / 1000, digits: 1)],
          [#rounded(r.latency_us.successful_server_response_production.p95 / 1000, digits: 1)],
        )
      })).flatten().flatten(),
    )
  },
  caption: [Exploratory pilot. “Reply” measures scheduled arrival to complete
  local HTTP body; server time ends at response production. Percentiles include
  successful responses only; OK and deadline columns retain all offered requests.
  One schedule per cell supplies no independent-run uncertainty estimate.],
) <pilot-latency>

The compact smoke corpus completed every request within the diagnostic deadline.
The history corpus did not: at #history8.pods Pods, each language completed
#history8.requests.successful of #history8.requests.offered requests successfully,
and none within the deadline. At #history64.pods Pods, only
#history64.requests.successful of #history64.requests.offered completed successfully.
The remaining outcomes are transport failures. Their durations are consistent
with the configured timeout, but the original error records do not preserve an
explicit timeout classification. Failed requests cannot be dropped when reading
the successful-response percentiles in @pilot-latency.

The history corpus is materially larger than the compact control. Its
#history64.pods\-Pod WAC pack contains
#rounded(history64.footprint.manifest.records / 1000000, digits: 2) million records,
#rounded(history64.footprint.manifest.source_bytes / calc.pow(2, 30), digits: 2) GiB
of serialized source and
#rounded(history64.footprint.manifest.packed_bytes / calc.pow(2, 20), digits: 1) MiB
of compressed content and policy data. The largest Pod requires
#rounded(history64.footprint.manifest.maximum_pod_source_bytes / calc.pow(2, 20)) MiB
of source, and #history64.pod_admission.stored_pods_above_limit stored Pods exceed
the configured per-Pod admission limit. Its observed process high-water RSS is
#rounded(history64.footprint.process_after_bytes.VmHWM / calc.pow(2, 20)) MiB.
These process and pre-load storage measurements do not establish a minimum
cgroup memory tier or include all later journal allocations.

The entropy variant uses seeded random text, including
#entropy8.literal_profile.message_text_bytes\-byte message excerpts and
#entropy8.literal_profile.short_text_bytes\-byte short labels, retaining the
history structure. It also fails the deadline, exposing sensitivity to the
compression and loading costs of generated values. Together, these measurements
are negative evidence for the present whole-Pod load-and-materialize path under
churn. They do not show that a large directory of compact Pods is slow, nor isolate
storage reads from decompression, parsing and queueing as a causal bottleneck.
The latter requires separate diagnostics and controlled interventions.

No pilot schedule selected a policy write, so these timings provide no policy
update or revocation measurement. The pack checks and sampled physical-reference
queries are useful correctness evidence but do not replace the full admission
conditions. The pilot supplies neither a sustainable mixed-service rate nor a
WAC/ACP equivalence verdict. Its summary records source hashes, per-operation
failures, absent coverage, loading phases and resource snapshots so those limits
remain auditable.

The main campaign has not yet supplied finalized, reviewed evidence for this
manuscript. Its status is *#campaign_state*. The completed pilot remains visible;
no partial main timings are used for capacity or equivalence claims.
] else [
The main campaign uses source #raw(campaign.source_commit.slice(0, 12)). Its final
checksum manifest is verified, and the analysis is bound to the benchmark-method
and request-accounting review of that source. This is a scoped experimental
review, not a proof of complete implementation correctness. The tables retain
all planned cells and apply the extractor's validity flags before inference.
The #link(review_root + "research/solid-pod-scale-main.json")[accompanying analysis]
binds source, input and extraction hashes to the full request-accounting and
resource evidence; public archival availability remains unresolved.

#main-tables(campaign)

The original pilot remains relevant context: at #history8.pods retained-history
Pods, each language completed #history8.requests.successful of
#history8.requests.offered requests, with none within the diagnostic deadline.
The compact smoke requests all met that deadline. That single-schedule pilot
used uniform query templates and existing-value replacements; the main campaign
uses journey weights and stock-derived mutations. Their response times must not
be pooled as repetitions of one workload.

A local passing cell supports only its named corpus, offered rate and resource
tier. Compact populated-Pod controls test hosted count separately from the larger
retained histories. The requested million-Pod retained-history service remains
unestablished without its full storage, operation and population coverage.
Network journeys remain unmeasured in this campaign.
]

= Discussion

A useful capacity statement must identify the populated corpus, request mix,
permissions, latency and hardware together. Pod count omits retained volume;
throughput omits failures and tail latency; an application cache limit omits
persistent storage and charged page cache. The reported conditions let readers
assess whether the result applies to their deployment.

Representativeness remains partial. Schemas establish recognizable service data;
ratings supply one retained-volume distribution; payment diaries anchor a mean;
selected chat donations motivate a messaging sensitivity. Other volumes,
cross-domain correlations and future application behavior remain assumptions.
Sensitivity experiments can expose dependence on those choices, but cannot turn
them into a population sample. Richer consenting exports could change the
inferred storage and service requirements.

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

This single-machine query study excludes high availability, disaster recovery,
denial-of-service resistance and full Solid conformance. Fanout counts separately
authorized target-Pod requests; arbitrary federation remains outside scope.
Issuer discovery is fixed during measurement. A deployment must provision these
additional operations and its unmeasured network journeys.

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

The retained-history pilot misses the interactive target under whole-Pod cache
churn even for small populations. Pod-local routing is supported; the broader
claim requires both sufficient retained-data capacity and a passing service
workload at the claimed population. The campaign distinguishes these conditions,
while its unmeasured network journeys remain a separate deployment limitation.

#{
  set text(size: 9pt)
  set par(leading: 0.45em, justify: false)
  bibliography("solid-pod-scale.refs.yml", style: "ieee", title: [References])
}
