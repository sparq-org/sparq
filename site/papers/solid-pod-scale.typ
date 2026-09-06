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
#let observations = json("../../research/solid-pod-scale-observations.json")
#let old(key) = baseline.values.at(key).value
#let rounded(number, digits: 0) = str(calc.round(number, digits: digits))
#let rate(pods, scenario) = derived.population_rates.find(x => x.pods == pods and x.scenario == scenario)
#import "solid-pod-scale-results.typ": main-state, main-tables, observed-range, range-label, model-resources, storage-values, bounds, queue-only, grouped-integer, dataset-label
#let campaign_path = sys.inputs.at("campaign", default: "../../research/solid-pod-scale-main.json")
#let campaign = if campaign_path == "" { none } else { json(campaign_path) }
#let campaign_state = main-state(campaign)
#let campaign_ready = campaign_state == "finalized and reviewed"
#let cohort(dataset, group, ram: none) = campaign.cells.filter(c => c.dataset == dataset and c.group == group and (ram == none or c.memory_gib == ram))
#let p95-range(rows) = observed-range(rows.map(c => c.requests.latency_us.at("successful:scheduled_latency_us", default: (:)).at("p95", default: none)))
#let findings = if campaign_ready {
  assert(campaign.cells.all(c => c.valid_for_inference), message: "Final narrative requires the complete valid run set")
  let two = cohort("compact-2000000", "compact-fixed-population")
  let one = cohort("compact-1000000", "compact-fixed-population")
  let demand = campaign.cells.filter(c => c.group == "population-control")
  let thousand = cohort("history-1000", "retained-history-larger")
  let hot = cohort("history-8", "retained-history-hot", ram: 1)
  assert(two.len() > 0 and two.all(c => c.local_guard == "pass"))
  assert(demand.len() > 0 and demand.all(c => c.local_guard == "fail" and c.requests.success_fraction_of_offered < frozen.measurement.success_fraction))
  assert(thousand.len() > 0 and thousand.all(c => c.requests.successful == 0))
  assert(hot.all(c => c.requests.success_fraction_of_offered >= frozen.measurement.success_fraction and c.requests.deadline_fraction_of_offered >= frozen.measurement.deadline_fraction_of_all_offered))
  assert(campaign.cells.all(c => c.resources.passed and c.requests.mutation.reconciled))
  assert(campaign.dedicated_policy_checks.all(c => c.passed))
  (one: one, two: two, demand: demand, hot: hot, thousand: thousand,
   passing: campaign.cells.filter(c => c.local_guard == "pass").len(),
   queue_only: campaign.cells.filter(c => queue-only(c, frozen.measurement)).len(),
   offered: campaign.cells.map(c => c.requests.offered).sum(),
   two_storage: storage-values(campaign, "compact-2000000", "wac"))
} else { none }
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
Solid applications need timely answers over data whose access rights vary by
resource. We ask: *Can we support efficient, access-controlled SPARQL queries over
Solid Pods at scale?* We study a Pod-local request path: locate a persistent Pod,
prepare its authorization state, and evaluate SPARQL over readable resource graphs.
A bounded cache limits the active working set. The benchmark combines populated
Pods, equivalent WAC and ACP rights, and offered requests derived from declared
user journeys. Published observations anchor selected data volumes; other rates,
retention and sharing patterns remain assumptions. The scope is structured service
records and media metadata, excluding original photos, videos and attachments.
#if campaign_ready [
On a #frozen.host.physical_vcpus\-vCPU, #frozen.host.physical_memory_gib\-GiB host,
both policy variants served #findings.two.first().requests.offered_rate requests/s
over #(findings.two.first().requests.load_metadata.corpus.pods / 1000000) million
populated compact Pods, with successful-response p95 at most
#rounded(p95-range(findings.two).upper / 1000, digits: 1) ms. Server affinity was restricted to one
CPU with a one-GiB cgroup ceiling; each compact Pod held
#(findings.two_storage.records / findings.two.first().requests.load_metadata.corpus.pods)
service records. Both variants failed the workload-derived population demand.
Retained-history results likewise separate an interactive small working set from
poor delivery at larger histories. The low-load ACP/WAC latency intervals fall
outside the declared equivalence margin. Thus hosted-Pod count scales for compact
data and light demand; a million-user retained-history service is not established.
The local response target reserves network and client time, which remain unmeasured.
] else [
An earlier controlled study supports selecting the Pod before graph enumeration.
The completed HTTP pilot exposes a different constraint: loading and preparing
whole retained histories under cache churn misses the interactive target even at
small populations. The expanded campaign therefore separates million-Pod compact
controls from bounded retained-history experiments and measures correctness,
complete local responses and resource use. Its target reserves network and client
time within a one-second journey; those journey costs remain unmeasured. Capacity
claims require the specified corpus, offered load and resource limit together.
]

#text(size: 9.5pt)[*Keywords:* Solid, SPARQL, access control, WAC, ACP, multi-tenancy, benchmarking]

= Introduction

A Pod lets applications query across personal services: events involving a
contact, spending over a period, or photos associated with an activity. SPARQL
expresses these questions over RDF, but every part of the answer must respect the
requester's resource permissions. Sharing a server with more people should not
require a query to inspect their private graphs.

This leaves two scaling problems: avoiding work over unrelated Pods, and storing
and serving the content that users actually retain. A small, repeatedly queried
fixture can test the first while concealing the second. We ask: *Can we support
efficient, access-controlled SPARQL queries over Solid Pods at scale?* Our target
is a modest single machine supporting millions of populated Pods. The frozen
campaign tests one- and two-million-Pod compact controls and smaller retained
histories, with explicit CPU, memory and response requirements. It does not assume
that success for compact data establishes a full retained-history service.

We evaluate a direct design: locate the requested Pod, load it into a bounded
cache if necessary, and apply a WAC or ACP authorization view before SPARQL
execution. The contributions are this service evaluation, a reproducible
personal-data workload with population-derived demand, and a comparison of equal
rights expressed in the two policy languages. The workload distinguishes observed
quantities from assumptions. Original media payloads are excluded from storage
and transfer; their metadata remains queryable. This boundary accompanies every
capacity claim.

#if campaign_ready [The answer is conditional: the compact population scales,
but its modeled demand overwhelms this implementation, and larger retained
histories fail the response requirement. WAC and ACP both support fast low-load
queries; their compact-query latency falls outside the chosen equivalence tolerance.]

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

The research HTTP path in @architecture selects the Pod before graph enumeration.
Storage uses one compressed N-Quads frame per Pod and an offset index.
A cache miss reads and decompresses the frame, parses RDF, builds in-memory
indexes and materializes authorization; a hit reuses this state. This is distinct
from the native indexed-file reopening tested in the separate component diagnostic.
The cache limits resident Pod count and retained input bytes. An external cgroup
bounds actual memory, including materialized state and charged file pages.
A manifest accounts for every persisted Pod.

SPARQ's WAC and ACP engines supply a common authorization index and session-specific
view. Similar warm query costs are therefore plausible, but compilation, retained
policy state and update work may differ. We measure those phases separately.
Adding unrelated stored Pods need not add graph scanning, but can change disk
lookup and cache-hit rates. Low warm latency and bounded memory can consequently
coexist with poor cold performance. Shared-resource timing non-interference is
outside the claim.

Authentication verifies signed credentials and a request-bound proof using a
pinned issuer key. Verification is timed; issuer discovery, initial interactive
login and the initial TLS handshake are outside the loopback boundary. The server
timer covers request middleware through response production, including body
reading, authorization, loading and serialization. The client also times complete
body receipt, which includes socket delivery and backpressure.

Acknowledged policy writes must affect subsequently admitted queries and survive
cache eviction and restart. The implementation uses one authenticated owner-only
administration path for both languages, restricted to the target Pod's policy
resources. It durably records the update and rebuilds authorization state. This
check is outside the WAC and ACP engines: the experiment does not establish ACP
ACR self-access or general equivalence of policy-control authority. Content reads
and writes continue to use the selected engine. Queries already admitted follow
the documented snapshot rule; stale access after acknowledgment is a correctness
failure.

= A workload for personal data

== From service records to populated Pods

The earlier baseline uses small social and health fixtures. The new campaign
uses a distinct compact control containing small personal service records, plus
a larger personal-data corpus representing retained service histories across communication, contacts,
calendar, transactions, activity, location and media metadata. Records share
identities, dates and relationships so that joins have meaning. Increasing the
population creates additional people with their own stored content; it does not
create empty routes or generate records only when a query first reaches them.

Google Takeout documents service categories, vCard contacts and photo metadata
@takeout; Open Banking supplies transaction fields and account relationships
@openbanking; Geolife illustrates timestamped location histories @geolife. These
support data types and relationships, not the volume a future Pod owner retains.

MovieLens supplies a reproducibly extracted distribution of retained rating
counts @movielens. Its version and checksum are recorded. This historical,
selected cohort supplies one observed marginal; it does not model an entire
person's data or establish counts for other services.

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
@volumes shows the base rates. The shared non-rating intensity mixture assigns
#corpus.population_intensity.classes.map(c => str(c.weight) + "% at " + rounded(c.numerator / c.denominator, digits: 1) + "×").join(", ").
It imposes correlation among those domains; ratings are sampled independently.
Neither the class frequencies nor that correlation is empirically validated. The corpus manifest records the
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
  caption: [Central retained-history model, before applying the shared non-rating
  multiplier. Rating counts are sampled independently. Transaction base intensity produces a modeled population mean of
  #corpus.domains.transactions.effective_generator_mean records per month; the
  other numeric base rates and the activity mixture are assumptions.],
) <volumes>

The manifest records per-domain counts, triples, serialized bytes, resources and
largest Pods. These observations support a partially calibrated scenario, not a
representative joint distribution of future Pod contents. Joint link-degree,
selectivity and held-out validation have not been performed. Original media bytes
are excluded from storage totals and request bodies. Synthetic excerpts, daily
activity summaries and episodic location records also omit full email bodies,
clinical records and fine-grained wearable streams. Monthly packaging and evenly
distributed dates do not reproduce seasonal or burst patterns.

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

The #derived.journeys_per_active_person_day daily actions per active person in
@journeys are assumptions with a limited activity scale check. Andrews et al.
observed #workload.activity_anchor.observed_mean_uses_per_person_day mean daily
device uses in #workload.activity_anchor.analyzed_participants participants over
#workload.activity_anchor.measurement_days days @andrews. Their selected historical
cohort does not estimate Solid traffic: an interaction may cause no query or
several. Ofcom motivates the range of service domains @ofcom; caching and fanout
remain separate backend-demand factors @memcache. Google Photos describes metadata
serving both interactive services and batch processing @photos-spanner, supporting
the metadata boundary and inclusion of background work, without supplying its
per-person frequency.

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
client caching, fanout and background rates fixed. Foreground owner reads choose
uniformly from the first #rounded(100 * workload.population.daily_active_fraction)%
active Pods; recipient, public and background reads span all Pods. Journey fanout
and batching factors are fixed at one. Population and cache size therefore change
the working set within these declared selection universes. Skewed
activity and broader demand or policy sweeps remain outside these measurements.

The query families include point lookups, stars, joins, aggregates, filters,
optional values, absence tests, bounded paths and visible-graph enumeration.
A lookup binds a resource and record; broader joins and counts examine readable
candidates. Ordered `LIMIT 20` bounds output, not the work needed to find or sort
it. Readable cardinalities therefore accompany query selectivity.

For example, this cross-domain query displays calendar events with contact names.
A calendar recipient can read appointments but not the owner's contact graph,
so the inner join produces no corresponding row. The query demonstrates why
permissions must constrain both sides before the join:

```sparql
PREFIX p: <https://sparq.dev/bench/personal#>
SELECT ?event ?name WHERE {
  GRAPH ?calendar { ?event p:service p:calendar;
                          p:contact ?contact }
  GRAPH ?contacts { ?contact p:name ?name }
} ORDER BY ?event ?name LIMIT 20
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
records and no-ops. Ingestion inserts full generated records; expiry deletes all
outgoing triples of selected records; modification replaces only `p:value`.
This finite-window workload keeps resource topology fixed;
creating new resources or containers requires a separate experiment.

== A local response budget

The primary target is a #workload.latency_budget.complete_server_response_p95_ms ms
95th-percentile complete response, including queueing, authentication, authorization,
loading, SPARQL and serialization. This reserves network transit and
#workload.latency_budget.client_processing_budget_ms ms of client work within a
one-second interactive journey. These are experimental usability budgets, not
Solid requirements @sre. The campaign measures local HTTP only: regional/mobile
profiles and complete network journeys remain unmeasured. Sequential requests
share the same journey budget, and summing component p95 values does not establish
a journey percentile.

= Evaluation method

Frozen campaign #raw(frozen.campaign_id) defines the selected cells in
@campaign-cells.
The earlier baseline and short HTTP pilot remain separate experiments.
The full host is a #raw(frozen.host.instance_type) with
#frozen.host.physical_vcpus vCPUs, #frozen.host.physical_memory_gib GiB RAM and
a #frozen.host.volume_gib GiB volume. Server and client use disjoint CPU-affinity
sets; CPU counts below are affinity counts, not scheduler quotas. GiB limits are
server cgroup memory ceilings, not the total host RAM requirement or evidence
that every request performs physical disk I/O.

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
      table.header([*Experiment*], [*Corpus and stored Pods*], [*CPU affinity*], [*GiB*], [*Offered rps*]),
      ..frozen.groups.map(g => (
        [#(
          "compact-fixed-population": "Fixed-load population",
          "population-control": "Population demand",
          "compact-rate-bracket": "Compact rate bracket",
          "retained-history-cold": "History cold/churn",
          "retained-history-larger": "History/size stress",
          "retained-history-hot": "History hot set",
        ).at(g.id)],
        [#g.datasets.map(dataset-label).join(", ")],
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

Timing admission compares effective rights with an oracle reading policy-neutral
generator records, without either policy compiler. Exact query results are checked
against physically selected readable content. Using the same SPARQL engine on
both paths provides independent authorization selection, not an independent
implementation of SPARQL semantics. Tests cover the declared grants and private
exceptions, hidden graph names, all query families, content writes and rejection
of invalid credentials. Dedicated HTTP probes check policy changes, revocation,
eviction and restart, including the separate owner-administration boundary.

Inventory accounting covers every persisted Pod. Query verification samples the
smallest and largest serialized Pod in every observed intensity class, breaking
ties by Pod ID; it does not prove every possible answer. Wrong authorization,
receipts or replayed state quarantine inference from that source. A resource or
admission failure instead rejects its dataset/configuration, preserving separately
verified controls.

== Storage, working set and service capacity

The frozen compact storage ladder calls for
#frozen.corpora.filter(c => c.profile == "smoke").map(c => grouped-integer(c.pods)).join(", ")
physically populated Pods per policy language. Only the populations in
@campaign-cells receive HTTP load cells. Retained-history cells are limited to
#calc.max(..frozen.corpora.filter(c => c.profile != "smoke").map(c => c.pods)) Pods;
no million-Pod retained-history run is included. Records are not thinned after
observing performance. Admission failures and unreached cells remain visible.
The earlier baseline supplies the fixed-target isolation evidence.

Each run begins with an operating-system cache drop and a fresh server, followed
by owner-read warm-up. Measurement starts when the warm-up client finishes;
there is no server queue-drain barrier. Timed-out warm-up work can therefore
continue into the measured window. This is the declared service condition, not
proof of a cold cache or steady state. Uniform target selection varies the active
working set; only responses carrying cache and phase headers expose their costs.
Missing headers mean unknown, not a cache miss. There is no skewed or first-touch
sweep, and the eight-Pod prefix omits heavy intensity classes present in larger
history populations.

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
fixed; there is no independent search for the fewest CPUs. Reported affinity and
memory limits identify tested configurations, not a global minimum or a complete
hosting budget. CPU counter differences cover the measured client window and may
include outstanding warm-up work. Work abandoned by a timed-out client can persist
until server termination; neither CPU utilization nor CPU per completed journey
is inferred.

== Comparing WAC and ACP

Each language receives identical content, intended rights and offered request
schedule. We alternate their order across independently restarted run pairs and
retain all failed attempts. Warm-up and measurement durations, schedule seeds,
timeouts and the minimum request count are frozen before the main campaign.
The shipping release profile runs on the dedicated Linux host; workstation
timings remain exploratory.

The practical equivalence interval for an ACP/WAC ratio is
$[1 / 1.10, 1.10]$, a declared engineering tolerance. Both variants must satisfy the
service objective. The paired p95 confidence interval and the separately bounded
operational-capacity ratio must each lie within the margin. Paired-run resampling
preserves independent repetitions; a nonsignificant difference or a same-load
completion ratio does not establish equivalence.

Capacity is bounded separately by each language's highest tested passing rate
and lowest consistently failing rate. Under a monotonic-capacity assumption,
bounds $L_W,U_W$ and $L_A,U_A$ imply ratio bounds $[L_A/U_W,U_A/L_W]$.
Missing bounds, mixed repetitions and nonmonotone results leave equivalence
unestablished. The frozen rate grid is too coarse to resolve the 10% capacity
margin even when passing points match; bootstrapping that tested-grid statistic
cannot narrow untested intervals. Loading, materialization and updates remain
separate diagnostics.

A tested configuration supports the modeled busy-period service only if all
required runs pass correctness, achieve at least
#rounded(100 * frozen.measurement.success_fraction)% successful completions, and
complete at least #rounded(100 * frozen.measurement.deadline_fraction_of_all_offered)%
of all offered requests successfully within the deadline and memory ceiling.
The queue guard requires #rounded(100 * frozen.measurement.queue_stability.required_timer_coverage_fraction_of_offered)%
timer coverage. Last-quarter queue p95 may exceed first-quarter p95 by at most the
larger of #rounded(frozen.measurement.queue_stability.allowed_growth_us_floor / 1000) ms
and #rounded(100 * frozen.measurement.queue_stability.allowed_growth_fraction_of_first_quarter)%
of that first-quarter value. A miss of this conservative short-window guard
rejects the cell under the frozen rule; by itself it does not demonstrate sustained
queue growth or poor response delivery. Results retain the guard verdict alongside
absolute latency and all-offered success fractions. Read-only measurements cannot
satisfy the mixed-operation criterion.

The loopback capacity lane also requires complete-body client receipt within
the same deadline for the required fraction of offered requests. This conservative
guard includes generator and local transport delay; the server's response-production
timer alone cannot establish complete-response latency.

= Results and evidence

== Service responses and resource limits

#if not campaign_ready [
#let pilot_order = (("smoke", 8), ("smoke", 64), ("history", 8), ("history", 64), ("entropy", 8))
The completed pilot exposes a cold-path limit: compact smoke requests meet the
diagnostic deadline, while retained histories miss it even at
#history8.pods Pods (@pilot-latency). Each configuration offered
#history8.requests.offered requests at #history8.requests.settings.rate requests/s,
using uniform Pod and template selection, owner reads and existing-value
replacements. One worker ran each language in fixed order on
#pilot.host_environment.at("CPU(s)") #pilot.host_environment.at("Model name") CPUs
(#pilot.host_environment.Architecture), with
#history8.server.settings.at("cache-pods") cached Pods and
#rounded(int(history8.server.settings.at("cache-bytes")) / calc.pow(2, 20)) MiB
of encoded input. Source #raw(pilot.source_commit.slice(0, 12)), checksums and
request accounting are preserved. This single-schedule pilot is distinct from
the main campaign's journey weights and stock-derived mutations.

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

The successful-response percentiles are conditional on completion. All offered
requests remain in the success and deadline columns. The other outcomes are
transport failures; their durations are consistent with the configured timeout,
but the pilot did not preserve an explicit timeout classification.

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

The seeded-text entropy variant also misses the deadline. Together these results
reject the interactive target for this whole-Pod loading path under churn; they
do not attribute the failure to namespace size or isolate disk reads from
parsing, materialization and queueing. No pilot schedule selected a policy write,
and the sampled oracle checks do not replace main admission conditions. The
pilot therefore establishes neither sustainable mixed-service capacity nor
WAC/ACP equivalence. Main evidence is *#campaign_state*; partial timings do not
enter these claims.
] else [
*Hosted count scales at light load; the modeled service demand does not.* Both
languages passed all #findings.two.len() policy runs across
#(findings.two.len() / 2) independent pairs at
#findings.two.first().requests.offered_rate requests/s over the two-million-Pod
compact corpus. Every offered request completed within the
#frozen.measurement.server_deadline_ms ms budget. The one-million-Pod control
also passed, with similar absolute response times. @main-response-latency shows
complete-response quality alongside frozen guard verdicts, rather than treating
fast errors as useful service.

#main-tables(campaign, summary: true)

== Population demand and retained histories

The compact corpus contains
#(findings.two_storage.records / findings.two.first().requests.load_metadata.corpus.pods)
service records per Pod. It tests directory size and concurrent service, not a
multi-year personal archive. With stored compact inventory, the frozen busy-period
model offers
#rounded(cohort("compact-1000000", "population-control").first().requests.offered_rate)
and #rounded(cohort("compact-2000000", "population-control").first().requests.offered_rate)
requests/s at the two populations. Both variants fail to deliver most requests at
those targets. The rate bracket gives the same warning earlier: at its upper
fixed rate, useful deadline completions are rare even though many requests still
return successfully. The corresponding failures are predominantly HTTP admission
rejections. A population-derived workload is therefore a materially stronger test
than holding offered load constant while adding Pods.

The eight-Pod history working set meets response-quality requirements at the
smallest tested server memory ceiling. Its remaining failures are solely the
short-window queue guard; raising that ceiling does not remove all guard misses.
This does not demonstrate a RAM shortage. The larger history populations instead
fail delivery: the history-#cohort("history-64", "retained-history-cold").first().requests.load_metadata.corpus.pods
runs complete only a minority of requests, and the history-#findings.thousand.first().requests.load_metadata.corpus.pods
runs complete none. The latter configuration's successful-response p95 is
therefore unavailable, not zero.

The entropy and higher-message-stock variants complete every offered request,
with the small deadline shortfall shown in the table; some runs miss only the
queue guard. They use a different CPU/worker configuration from the history hot
set. The entropy variant changes literal lengths as well as compressibility,
and larger history populations contain heavy Pods absent from the eight-Pod
prefix. These cells test their named configurations, not isolated effects of
CPU, compression or cache size.

== What the costs and policy comparison establish

The two-million compact corpus occupies
#rounded(storage-values(campaign, "compact-2000000", "wac").allocated_bytes / calc.pow(2, 30), digits: 2) GiB
with WAC and
#rounded(storage-values(campaign, "compact-2000000", "acp").allocated_bytes / calc.pow(2, 30), digits: 2) GiB
with ACP before journal growth. The successful fixed-load runs peak below
#calc.ceil(calc.max(model-resources(findings.two, "wac").peak_bytes.upper, model-resources(findings.two, "acp").peak_bytes.upper) / calc.pow(2, 20)) MiB
of server cgroup memory. These are measured server memory and storage costs on the full
host described above, not evidence that the whole deployment needs only one CPU
and one GiB. Grouped resource and storage results are in the
#link("solid-pod-scale-companion.pdf")[companion tables]; per-run counters and
snapshots remain in the linked analysis and source bundle.

There are #findings.passing passing runs,
#findings.queue_only queue-only guard failures and
#(campaign.cells.len() - findings.passing - findings.queue_only) other failing runs.
Every measured run has complete accounting and passes the resource checks; all
durable mutation receipts reconcile, and the dedicated grant/revoke probes pass.
These checks support the declared operations and sampled oracle comparisons,
not every possible query or full Solid conformance.

ACP's successful-response p95 is consistently higher in the compact low-load
cells. For the two-million fixed-load case, the paired ACP/WAC interval is
#bounds(campaign.paired_comparisons.find(p => p.dataset == "compact-2000000" and p.group == "compact-fixed-population").paired_p95_scheduled_response_ratio.ci95).
Both variants remain interactive, but this lies outside $[1/1.10,1.10]$.
The conservative capacity-ratio bounds are
#bounds(campaign.capacity_brackets.first().conservative_capacity_ratio_bounds);
the coarse grid and a queue-only miss at an intermediate ACP rate prevent a
precise capacity comparison. The different highest all-passing grid points are
not an estimate of a corresponding capacity difference. No overall WAC/ACP
performance-equivalence claim follows.

The failure diagnostics have narrower coverage than the response counts. The
available successful history replies report cache hits, while failed replies
lack cache and phase headers. They cannot directly time the failed requests.
HTTP rejections report a full worker queue, but the handler uses that response
for either a full or disconnected channel. The warm-up client can finish while
work remains in server queues. The results establish inadequate delivery under
that declared condition, without proving a parsing, storage or steady-state
queueing cause. Separately, the bounded indexed-file diagnostic finds that opening
validated indexes and preparing authorization is not automatically faster than
parsing these small histories. It is a component result with uncontrolled cache
state; its complete measurements are in the companion.

Source #raw(campaign.source_commit.slice(0, 12)) produced
#rounded(findings.offered / 1000000, digits: 2) million offered and recorded requests across
#campaign.cells.len() measured runs. The finalized
#link(review_root + "research/solid-pod-scale-main.json")[analysis] binds input hashes,
request/receipt accounting and resource records to the scoped benchmark-method
and request-accounting review. The #link(review_root + "research/solid-pod-scale-main-report.json")[audit summary]
provides the grouped verdicts and uncertainty intervals. These are accompanying
local review artifacts; public archival availability remains unresolved.

]

== Earlier evidence for Pod-local routing

The earlier WAC study used small health and social fixtures, distinct from the
new personal-service controls. Across #old("pod_counts").first()–#old("pod_counts").last()
stored Pods, the fixed-target routed path passed its unrelated-Pod overhead
criterion in #old("routed_pass_count")/#old("cells") query/domain cells; server-wide
assembly passed #old("native_pass_count")/#old("cells"). Its
#old("oracle_comparisons") readable-content comparisons
#if old("correctness_passed") [passed] else [failed]. The original wall/CPU criterion
limited upper confidence bounds to #rounded(100 * (old("endpoint_ratio_threshold") - 1))%
endpoint growth and #old("elasticity_threshold") log–log Pod-count elasticity.
This supports selecting the Pod before graph enumeration. In-process,
single-request timings on small fixtures establish neither million-Pod storage
nor ACP or cold-response performance.

The #link(review_root + baseline.source)[source summary] and
#link(review_root + baseline.accompanying_artifacts.directory + "/raw-sanitized.tar.zst")[raw archive]
retain all cell effect sizes, intervals and thresholds for run #raw(baseline.run_id),
commit #raw(baseline.source_commit.slice(0, 12)). Displayed values are extracted by
JSON pointer and source SHA-256. These review links resolve to the accompanying
local bundle; public archival availability remains unresolved.

= Discussion

A capacity statement needs the corpus, request mix, permissions, latency and
hardware together. Hosted count omits retained volume; successful-response p95
omits failures; a server memory ceiling omits the rest of the host. The campaign separates
these quantities so that compact controls cannot stand in for a million users'
retained histories.

Calibration remains partial: ratings provide one count distribution, payments a
mean and chat donations a volume sensitivity. Other volumes, correlations,
sharing and backend activity remain assumptions. Additional consenting exports
could change both storage and service requirements. Original media payloads
would also add storage, bandwidth, replication and backup costs beyond this
metadata experiment.

WAC and ACP can have similar warm evaluation but different preparation or update
costs. Equal effective rights support the comparison; static group membership
and shared owner administration limit its language coverage. The broader service
also needs high availability, disaster recovery, denial-of-service resistance,
full Solid conformance and issuer discovery. Arbitrary federation and network
journeys remain separate deployment questions.

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

#if campaign_ready [
Access-controlled SPARQL over millions of compact Solid Pods is practical at the
tested light load on one machine. Both WAC and ACP meet the complete-response
budget with a small server working set. That result does not extend to the
modeled user demand or to a million retained personal histories: the population
load and larger history experiments fail response delivery. The smaller history
working set often serves promptly despite conservative queue-guard misses.

The next constraint is sustained service over retained data and active demand,
not merely naming more Pods. WAC and ACP share a useful query interface, but
their measured compact-query costs exceed the chosen equivalence margin. The
conclusion is bounded by partially calibrated structured data and media metadata,
the declared warm-up condition, and unmeasured network journeys.
] else [
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
]

#{
  set text(size: 9pt)
  set par(leading: 0.45em, justify: false)
  bibliography("solid-pod-scale.refs.yml", style: "ieee", title: [References])
}
