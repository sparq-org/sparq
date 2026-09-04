// Outcome-blind factory-integrated manuscript. Every empirical scalar and H1/H2 branch
// enters through the canonical timing accessors; E3--E5 have no generated verdict prose.

#import "_lib/timing.typ": headline_timing, timing_verdict, timing_table, timing_figure, timing_provenance

#let anon = sys.inputs.at("anon", default: "false") == "true"
#let paper_heading_numbering = (..n) => {
  let ns = n.pos()
  numbering("1.", ..if ns.len() > 1 { ns.slice(1) } else { ns })
}
#let authors() = if anon {
  align(center)[#text(style: "italic")[Anonymous submission]]
} else {
  align(center)[
    #text(weight: "bold")[Jesse Wright] \
    #text(size: 0.9em)[sparq project]
  ]
}
#let h2_queries = ("q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8")
#let h2_metrics = (
  (label: [wall ratio], estimate: "wall.median_ratio",
    low: "wall.ratio_ci95.low", high: "wall.ratio_ci95.high"),
  (label: [wall elasticity], estimate: "wall.pod_elasticity",
    low: "wall.elasticity_ci95.low", high: "wall.elasticity_ci95.high"),
  (label: [CPU ratio], estimate: "process_cpu.median_ratio",
    low: "process_cpu.ratio_ci95.low", high: "process_cpu.ratio_ci95.high"),
  (label: [CPU elasticity], estimate: "process_cpu.pod_elasticity",
    low: "process_cpu.elasticity_ci95.low", high: "process_cpu.elasticity_ci95.high"),
)
#let h2_key(lane, domain, query, field) = {
  "ac_sparql.h2." + lane + "." + domain + "." + query + "." + field
}
#let h2_metric_rows(lane, domain) = h2_queries.fold((), (rows, query) =>
  rows + h2_metrics.map(metric => (
    [#raw(query)],
    metric.label,
    (key: h2_key(lane, domain, query, metric.estimate), digits: 3),
    (key: h2_key(lane, domain, query, metric.low), digits: 3),
    (key: h2_key(lane, domain, query, metric.high), digits: 3),
  ))
)
#let h2_verdict_rows(lane, lane_label, domain, domain_label) = h2_queries.map(query => (
  lane_label,
  domain_label,
  [#raw(query)],
  (key: h2_key(lane, domain, query, "minimal_overhead"),
    yes: [meets], no: [does not meet]),
))
#let scenario_key(scenario, lane, metric) = {
  "ac_sparql.scenario." + scenario + "." + lane + "." + metric
}
#let scenario_row(label, scenario, lane, metrics) = {
  (label,) + metrics.map(metric =>
    (key: scenario_key(scenario, lane, metric), digits: 0)
  )
}

#set document(title: "Access-Controlled SPARQL over Co-hosted Solid Pods: A Cost Model and Controlled Scaling Study")
#set page(paper: "a4", margin: (x: 21mm, y: 19mm), numbering: "1")
#set text(size: 10.5pt)
#set par(justify: true, leading: 0.68em)
#set heading(numbering: paper_heading_numbering)

#align(center)[
  #text(size: 17pt, weight: "bold")[
    Access-Controlled SPARQL over Co-hosted Solid Pods: \
    A Cost Model and Controlled Scaling Study
  ]
]
#authors()

#if not anon [
  #align(center)[#text(size: 0.82em, fill: gray)[
    Draft prepared for a Semantic Web systems/integration venue. Canonical empirical
    values and H1/H2 branches are rendered only through provenance-bearing accessors.
  ]]
]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

Solid servers may eventually host many independently controlled Pods, but a query made
by one principal must be evaluated only over resources that principal may read. The
central systems question is whether the compute cost of such a query must grow merely
because unrelated Pods are co-hosted. We formalize read authorization as an RDF-dataset
view and study two enforcement boundaries in the SPARQ codebase: a benchmark-composed,
materialized Pod-routed feasibility path that selects one Pod store before query
evaluation, and the current native handler exercised in process through its Axum router,
which assembles an authorized dataset by traversing the shared server hierarchy on each
request. The model predicts expected Pod-count-independent
request work for the routed path when the target Pod is fixed, but at least linear
candidate visitation for the current server-wide assembly path. We test these predictions
with a deterministic social/health generator that varies Pod count independently of
documents, triples, hierarchy, policy placement, and visibility; eight SPARQL operator
families; an independently derived physical-filter oracle; and a prospective,
correctness-gated analysis on a pinned single-CPU cloud host. The canonical campaign
admitted #headline_timing("ac_sparql.campaign.valid_observations", digits: 0)
observations over
#headline_timing("ac_sparql.campaign.completed_paired_cells", digits: 0) complete paired
cells; the correctness gate
#timing_verdict(
  "ac_sparql.correctness.passed",
  yes: [passed every admitted exact result-bag comparison],
  no: [did not pass every admitted exact result-bag comparison],
). Over 1--2,048 resident Pods, the routed matrix
#timing_verdict(
  "ac_sparql.h2.materialized.rollup.all_cells_meet",
  yes: [met the four-part criterion in every domain-by-query cell],
  no: [#timing_verdict(
    "ac_sparql.h2.materialized.rollup.no_cells_meet",
    yes: [met the criterion in no domain-by-query cell],
    no: [had a mixed cellwise result],
  )],
), while the native matrix
#timing_verdict(
  "ac_sparql.h2.http.rollup.all_cells_meet",
  yes: [met the criterion in every domain-by-query cell],
  no: [#timing_verdict(
    "ac_sparql.h2.http.rollup.no_cells_meet",
    yes: [met the criterion in no domain-by-query cell],
    no: [had a mixed cellwise result],
  )],
). Complete paired contrasts, work proxies, construction phases, and whole-process RSS
are reported descriptively without E3--E5 verdicts. Together, the model and campaign
define a bounded implementation envelope, not a forecast of future Pod populations. They
test tenant-local routing as one way to decouple warm query work from unrelated hosted
state while keeping startup, resident memory, updates, remote storage, and concurrent
capacity as separate costs.

#text(weight: "bold")[Keywords:] Solid; SPARQL; Web Access Control; multi-tenancy;
authorization views; RDF; reproducible performance evaluation

== Introduction <intro>

Solid organizes personal data as Web resources in user-controlled Pods and defines
resource-level authorization mechanisms for deciding which principals may access them
@solid-protocol @wac. A hosted Solid service may therefore hold resources for many
independent users. If that service also exposes SPARQL, a request from one user must not
gain query-visible access to another user's private graphs. A direct implementation can
walk the server, authorize each resource, build a temporary dataset from those admitted,
and then evaluate the query. Such an implementation is simple to reason about at the
result boundary, but it risks making the cost of every request depend on all hosted
resources, including resources that cannot affect the answer.

That dependence is not inevitable. An alternative is to compile authorization policy
ahead of time, route the request to a physically Pod-local store, and expose only the
authorized named graphs through a dataset view. The query engine then operates within a
tenant-sized namespace. The distinction matters because a logical authorization filter
alone is insufficient: an unbound `GRAPH ?g` operator may still enumerate every named
graph in the selected physical store before rejecting hidden graph names. The physical
routing boundary, rather than the syntax of the access-control rule, determines whether
unrelated tenants enter the request path.

Prior work provides access-controlled SPARQL over linked data and federated RDF cubes
@kirrane-query-ac @safe, and Solid work has proposed graph-centric Pod APIs, permissioned
SPARQL satellites, and materialized views @whats-in-a-pod @consolid @solid-ivm. Solid-oriented
benchmarks provide social, health, and search workloads @solidbench @tidal @espresso. To
our knowledge, the reviewed literature does not isolate the marginal server-compute effect
of adding unreadable, unrelated Pods while keeping one target query's authorized working
set fixed. Published counts also cannot establish what a
future, widely deployed Pod population will look like. We therefore treat them as
calibration anchors, not samples from an unknown population, and use controlled synthetic
interventions for the causal comparison.

This paper asks five questions. RQ1 tests authorization correctness against an oracle
whose readable-content selection is independent of both access-control implementations.
RQ2 measures whether warm request cost changes as unrelated resident Pods are added. RQ3
relates total request and construction costs to the backend-operation and allocation work
proxies that are actually recorded; it does not assign elapsed time to individual stages.
RQ4 measures the paired guarded-stack contrast against a query-answer-equivalent,
content-only physical reference. RQ5 examines
cold construction and whole-process resident capacity. We operationalize the phrase
_minimal unrelated-Pod overhead_ in advance: for each query family and domain, both the
upper 95% confidence endpoint for the median cost ratio between 2,048 and one Pod and the
upper endpoint for Pod-count elasticity must be at most 1.10 and 0.10, respectively, for
both wall time and process CPU. Failure of any of the four conditions prevents that
description for the affected cell.

The contributions are:

- *A semantics and cost model.* We express authorization as an RDF-dataset view, prove
  result equivalence to physical filtering, distinguish result-level absence from timing
  non-interference, and derive conditional request-cost predictions for two implemented
  enforcement boundaries.
- *An independently parameterized benchmark.* Pod count, origin topology, documents,
  triples, hierarchy depth, explicit ACL coverage, audience mixture, and content domain
  vary separately. A direct audience predicate selects the physical reference dataset
  without invoking the policy compiler, reasoner, SPARQL engine, or server endpoint.
- *A correctness-gated empirical study.* A routed feasibility composition built from
  production `PodStore` components, an in-process invocation of the native Axum handler,
  two vocabularies, eight SPARQL families, randomized guarded/plain pairs, five independently reconstructed
  process blocks, wall and process-CPU measures, and separate deterministic
  instrumentation test the theory without conflating request timing with counter overhead.
- *A reproducible resource-capped artifact.* Raw records, checksums, schedule seeds,
  environment metadata, analysis code, derived-output hashes, failure evidence, and a
  cloud-cost ledger make both successful and invalid attempts auditable.

The scope is deliberately narrower than the motivating deployment. The `/sparql`
surface studied here is this work's interface, not a ratified Solid endpoint. The policy
subset covers WAC read grants for owners, the public, and one named recipient; ACP, ODRL,
groups, client constraints, issuer constraints, and denies are not pooled into the
results. The experiment fixes concurrency at one, removes remote-storage variance with
in-memory storage seams, and proves no timing non-interference property. Its conclusions
concern the pinned implementation, workload, host, and tested range.

== Background and Problem Definition <background>

=== Solid resources and Web Access Control

The Solid Protocol describes resource-oriented storage accessed using Linked Data
Platform conventions @solid-protocol. Web Access Control (WAC) associates resources and
containers with access-control documents. An `acl:Authorization` can identify a named
principal through `acl:agent`, a public class through `acl:agentClass`, modes such as
`acl:Read`, direct scope through `acl:accessTo`, and inherited descendant scope through
`acl:default` @wac. WAC uses grant-otherwise-deny evaluation: absence of a matching grant
does not permit access. Both documents are Draft Community Group Reports, not W3C
Recommendations. Access Control Policy is a separate editor's draft with different
policy semantics @acp; measurements of it would require a separate experimental lane.

The generated common subset uses three content audiences. Public documents are readable
by every requester. Private documents are readable only by their Pod owner. Shared
documents are readable by the owner and one named recipient. Root and category-container
ACLs provide inherited grants, while a controlled fraction of content resources receives
an own ACL with the same decision. Changing ACL placement can therefore change policy
work without changing the oracle-visible dataset.

=== SPARQL datasets and named graphs

SPARQL evaluates a graph pattern against an RDF dataset consisting of a default graph and
zero or more named graphs @sparql11-query. In this study each content resource is exactly
one named graph, structural container and control documents occupy separate graphs, and
the standing default graph is empty. The query interface follows the SPARQL 1.1 HTTP
query operation in shape @sparql11-protocol, but its availability on a Solid server is a
design contribution of this implementation, not a Solid Protocol requirement.

Named graphs make the security boundary explicit: authorization determines which graph
names and contents are present in the active dataset. This construction is stronger than
post-filtering returned rows. A query can join data, test non-existence, aggregate, or
enumerate graph names; removing unauthorized graphs only after evaluation could allow
them to influence an otherwise innocuous result.

=== Observation and threat model

Let the requester control the SPARQL query and its authenticated or anonymous session.
The server and its policy store are trusted to execute the pinned implementation. The
adversary may know IRIs and may ask graph-bound or graph-enumerating queries, but cannot
modify data during a measured request. We require *result non-interference under fixed
policy and evaluation context*: changing graphs that remain unreadable must not change
the returned solution multiset. This is deterministic equality, not a cryptographic
indistinguishability claim.

This is not a complete non-interference claim. Response time, process CPU, allocations,
backend calls, or error behavior can reveal that additional resources exist even when the
answer is unchanged. Our measurements deliberately expose this distinction. Authentication
verification belongs to the native in-process handler interval, while client-side token and
DPoP-proof construction does not. Authentication correctness and resistance to active
network attackers are outside the contribution.

=== Research questions and hypotheses

#figure(
  table(
    columns: (0.11fr, 0.43fr, 0.46fr),
    align: (left, left, left),
    table.header[Question][Estimand][Status or decision rule],
    [RQ1],
    [Exact answer authorization across principals and revocation.],
    [H1: every included request equals the independent readable-document multiset; one
     mismatch invalidates its run.],
    [RQ2],
    [Change in warm cost as unrelated Pods increase with target state fixed.],
    [H2a: routed materialization meets all four practical-growth margins. H2b: native
     assembly does not because it visits server-wide candidates.],
    [RQ3],
    [Total wall/CPU cost, allocations, and backend-operation counts as one data or policy
     factor changes; no stage-time attribution.],
    [Exploratory E3 (formerly H3): report each lane/domain/query/factor cell descriptively;
     no confirmatory threshold or supported/rejected verdict.],
    [RQ4],
    [Within-fixture paired guarded-stack effect relative to the query-answer-equivalent
     content-only physical reference.],
    [Exploratory E4 (formerly H4): report paired wall/CPU differences and ratios with
     block-cluster intervals by cell; no cross-query ordering or binary threshold.],
    [RQ5],
    [Cold construction phases and whole-process current/peak RSS as hosted state grows.],
    [Exploratory E5 (formerly H5): report levels and trends descriptively; no linearity
     threshold, population law, throughput claim, or supported/rejected verdict.],
  ),
  caption: [Research questions and estimands. Only H1 and H2 have confirmatory decision
  rules; E3--E5 are conservative, outcome-blind downgrades made before canonical result binding.],
) <tab-rqs>

== Authorization Semantics and Implemented Paths <system>

=== Authorized-dataset semantics

Let an RDF dataset be $D=(D_0,D_N)$, where $D_0$ is the default graph and
$D_N : G arrow "RDFGraph"$ is a finite partial mapping from graph names to RDF graphs.
The studied stores have $D_0=emptyset$. For policy state $pi$, session $s$, access mode
$m$, and fixed request/evaluation context $c$, let
$A_pi(D,s,m,c) = {g in "dom"(D_N) | "allow"_pi(D,s,m,g,c)}$. The authorization view is

$ V_N(D,A)(g) = cases(D_N(g) & "if " g in A, "absent" & "otherwise"), $

$ V(D,A)=(emptyset,V_N(D,A)), $

and an access-controlled read query denotes

$ "ACEval"_pi(q,D,s,m,c) = "Eval"_c(q,V(D,A_pi(D,s,m,c))). $ <eq-aceval>

The security mechanism therefore changes the dataset supplied to ordinary SPARQL
evaluation; it does not alter the query's algebra or remove rows after evaluation.
Here and below, $q$ ranges over deterministic read queries evaluated against the supplied
active dataset: external `SERVICE` calls, protocol-supplied dataset overrides, updates,
and nondeterministic functions are outside the claim. Authorized RDF views and SPARQL
view rewriting are established techniques @sparql-view-rewriting
@rdf-access-control-survey; the contribution is the physical Pod-local enforcement
boundary and its measured scaling behavior, not the idea of a view.

*Proposition 1 (view equivalence).* If a physical reference dataset has the same empty
default graph and named-graph mapping as $V(D,A_pi(D,s,m,c))$, evaluating $q$ over that dataset and
evaluating it through the authorization view return the same SPARQL solution multiset.

_Proof._ The two evaluations receive the same query, active dataset, and fixed evaluation
context. Their algebra evaluation, including multiplicities, is therefore identical.
$square$

*Corollary 1 (result non-interference).* For fixed $pi,s,m,c$ and a fixed resulting
authorization set $A_pi(D,s,m,c)$, changing or adding only graphs that remain outside
that set cannot change $"ACEval"_pi(q,D,s,m,c)$.

_Proof._ Applying $V$ discards every changed graph, leaving identical active datasets;
Proposition 1 then applies. $square$

The experimental physical oracle contains readable *content* graphs, not structural
container or ACL graphs, so a workload-specific condition is also required.

*Lemma 1 (oracle sufficiency for the benchmark).* Removing authorized container and ACL
graphs from the physical reference cannot change any of the eight generated query
templates' solution multisets.

_Proof._ `q1` names a generated content graph and content subject. Every mandatory graph
pattern in `q2`--`q8` requires a generated social or health class. In the materialized
fixture, container graphs carry only structural vocabulary; in the native fixture, seeded
container RDF bodies are empty. ACL graphs carry WAC vocabulary in both. None carries the
required domain class, so none can satisfy the mandatory pattern.
$square$

Corollary 1 speaks only about returned solutions. It permits cost differences caused by
discovering and rejecting unreadable resources, which RQ2 and RQ3 measure. Lemma 1 is
likewise confined to the generator invariants and these templates; it is not universal
equivalence for arbitrary queries over structural or policy vocabulary.

=== Materialized and Pod-routed evaluation

The benchmark harness composes a routed feasibility design from production
`sparq_solid::PodStore` components: it constructs one store per physical shard, and the
primary study uses one Pod per store. This composition is *not* the request path of the
current native `sparq-lws-core` `/sparql` endpoint. Each store parses its Pod's content,
containment, and WAC graphs and runs `materialize_wac`, compiling decisions into an
authorization index. A routing hash table maps the target Pod root to the appropriate
store. Results for this lane establish what that composition can achieve, not that the
native endpoint already routes Pod-first.

At request time, `query_as` resolves the session's readable graph-name set, using a
bounded sharded cache on repeated sessions, and supplies the set to the query engine as a
`DatasetView`. The view checks named-graph visibility without copying admitted triples.
The current string API parses the query, rewrites and serializes its algebra, and lets the
engine parse that serialization again; this preparation belongs to the measured routed
operation rather than being hidden in construction.
All graph enumeration and query evaluation after routing occurs within the selected
PodStore. An ACL replacement re-materializes the affected PodStore and invalidates
relevant session-cache entries; it is a write-side cost, not part of the timed warm read.

=== Native in-process handler authorized-dataset assembly

The native baseline invokes the current `sparq-lws-core` Axum `/sparql` router in process
through `tower::ServiceExt::oneshot`, with a valid authenticated request where applicable.
It exercises middleware, authentication, the handler, and response-body buffering, but no
socket, TLS, reverse proxy, or network transport. After request authentication and query parsing,
the handler takes a snapshot read guard and calls its authorized-dataset assembly path.
That path begins at the shared server root, recursively discovers containment into an
ordered set, adds a candidate ACL resource for every non-ACL resource, plans and checks
WAC for each candidate, reads admitted RDF, and serializes admitted triples into an
N-Quads buffer. It then parses that buffer into a temporary indexed graph, evaluates the
query, and serializes the SPARQL result while the snapshot guard remains held.

The benchmark uses the current handler, verifier, WAC planner, RDF parsers, query
engine, and serializer. Its in-memory SPARQL and blob stores remove network and object
store variance. Their admission limits are derived exactly from each generated corpus,
rather than leaving a deployment default to truncate the factor matrix.

#figure(
  table(
    columns: (0.18fr, 0.39fr, 0.43fr),
    align: (left, left, left),
    table.header[Stage][Materialized/routed composition][Native in-process handler],
    [Entry], [route by target Pod root], [authenticate and parse HTTP query],
    [Discovery], [one selected PodStore], [walk containment from shared server root],
    [Authorization], [cached materialized graph-name set], [plan/check every candidate],
    [Dataset], [zero-copy `DatasetView`], [read RDF, write N-Quads, parse temporary graph],
    [Evaluation], [selected PodStore only], [temporary authorized graph],
    [Output], [library result], [SPARQL Results JSON over HTTP],
    [Potential dependence on unrelated Pods], [route-table lookup and system effects],
    [containment, candidates, planning, and rejected-resource work],
  ),
  caption: [The two measured enforcement boundaries. They share the generated corpus and
  query engine but are architectural alternatives, not otherwise identical request paths.],
) <fig-architecture>

=== Comparison boundary

Both paired references contain only independently selected readable *content* graphs.
The guarded active datasets can additionally contain authorized structural/container and
control graphs. Lemma 1 establishes query-answer equivalence for the eight templates, not
physical-dataset identity. Consequently, the paired differences can include work caused by
those extra nonmatching graphs---especially graph-name enumeration in `q8`---as well as
authorization machinery.

The materialized paired reference runs the same query directly over the content-only
physical graph for Pod 0. Its guarded-minus-reference difference primarily covers routing,
authorization-set lookup, query normalization and reparsing, view membership checks, API
wrapping, and any work over admitted structural/control graphs. The native paired reference
evaluates the query directly and serializes JSON over the content-only physical graph, but
does not perform in-process HTTP authentication and routing, server-wide discovery, WAC
planning, or temporary authorized-dataset reconstruction. Its difference is therefore a
*guarded-stack/content-reference contrast*, not an isolated estimate of WAC rule matching or a
dataset-identical access-control overhead.
Absolute latency across the two lanes is likewise not an apples-to-apples speed ranking;
their scaling behavior and internal work are the meaningful comparison.

== Cost Analysis <theory>

=== Model and notation

We use a RAM cost model that charges for reading, copying, hashing, and comparing lexical
bytes. Consuming an input of $B$ bytes therefore costs at least $Omega(B)$. Hash-table
bounds are expected-case unless stated otherwise. Route keys, session fields, and RDF
terms are bounded in the Pod-count intervention; where they are not, the lexical-length
factor $ell$ remains explicit. Wall-clock time is not equated with operation count.
Backend operations stay symbolic because the storage trait admits in-memory and remote
implementations with different costs. The source audit supporting each implementation
term is listed in @app-source-audit.

#pagebreak()
#figure(
  text(size: 0.9em)[
    #table(
      columns: (0.17fr, 0.83fr),
      align: (left, left),
      table.header[Symbol][Meaning],
      [$P$], [resident Pods],
      [$B_p$], [serialized input bytes loaded for Pod $p$],
      [$n_p$], [source named graphs in Pod $p$, including content, container, and control graphs],
      [$g_p$], [named-graph slots in the selected `PodStore` at query time],
      [$R_p$], [distinct resources and structural containers in Pod $p$],
      [$F_p$], [policy and structural facts admitted to WAC reasoning],
      [$d(g)$], [ancestor steps traversed while processing graph $g$],
      [$J_p$], [reasoner candidate, join, intermediate-row, and insertion work],
      [$H_p$], [triples installed in Pod $p$'s materialized authorization graph],
      [$A_s$], [readable graph names for session $s$ in the routed Pod],
      [$N,E,K$], [native-path reachable resources, returned child occurrences, and candidate set],
      [$B_A,T_A,G_A$], [admitted RDF bytes, triples, and graph names in the native path],
      [$ell$], [maximum relevant lexical byte length],
      [$Q(q,D)$], [implementation query work, including scans and intermediates],
      [$I(q,D)$], [peak query-intermediate space],
      [$Y(q,D)$], [produced binding cells for the in-process result],
      [$Z(q,D)$], [serialized HTTP result bytes],
    )
  ],
  caption: [Cost-model notation. Pod count is not used as a proxy for data, policy,
  visibility, query, or output size.],
) <tab-notation>

The model assumes successful local evaluation and, for uncontended warm-cache claims, no
concurrent invalidation. Scheduler delay, paging, allocator effects, NUMA placement, and
network latency are empirical rather than RAM-model terms. The native analysis follows
the benchmark's default WAC-only features; enabling the optional ODRL gate adds its
decision cost to every candidate iteration.

=== Materialized construction and resident state

Let $L_p(B_p)$ denote parsing and graph-index construction for Pod $p$. The loader scans
named graphs, synthesizes resource and ancestor facts, consumes control documents, and
interns facts into the reasoner. Semi-naive evaluation uses hash-indexed facts and shared
join kernels; the resulting authorization view is installed and then scanned to rebuild
the authorization index. Under expected hash behavior, the byte-sensitive implemented
upper bound is

$ E[W_"build"(P)] = O(P + sum_(p=1)^P
  (L_p(B_p) + (n_p + sum_(g in p) d(g) + F_p + H_p) ell + J_p)). $

Because construction parses every Pod and installs every produced view, it also has the
lower bound

$ W_"build"(P) = Omega(P + sum_p B_p + sum_p H_p). $

If every Pod is non-empty and all per-Pod terms, including parser and reasoner work, are
uniformly bounded, expected construction is $Theta(P)$. This qualification matters: an N3
rule program can create large joins even when its final closure is small. The controlled
generator bounds hierarchy depth and rule shape, making structural and authorization
closure size $O(R_p+F_p)$, but we retain $J_p$ rather than inferring reasoning work from
closure size.

Let $rho_p$ be the byte length of Pod $p$'s route key. Resident state decomposes as

$ S_"routed" = Theta(P + sum_p rho_p) + sum_p
  (S_"store"(p) + S_"auth"(H_p)) + S_"cache". $

Under bounded route keys the leading term is $Theta(P)$ and includes the route plus one
store/cache header per Pod. The session cache
has 16 shards and at most 1,024 live entries per shard per store, but an entry-count cap is
not a byte cap. If $C_p$ is the cached-session set and $a_e$ its admitted graph-name count,
let $d_e$ be its dirty-origin count and let $U_p$ be the bytes occupied by queued recency
keys in store $p$. Then, up to representation constants,

$ S_"cache" = O(sum_p sum_(e in C_p)
  (abs("key"_e) + (a_e+d_e) ell) + sum_p U_p),
  quad abs(C_p) <= 16,384. $

The recency queue is separately bounded to twice 1,024 keys in each of 16 shards, so
$U_p$ is at most 32,768 times the maximum session-key bytes in that store. Dirty origins
matter because repeated scoped invalidations can accumulate them before the next fill.

Even an empty cache retains fixed locks and containers per Pod. The experiment therefore
reports observed whole-process memory and realized authorization triples rather than
assuming constant bytes per entry or triple.

#linebreak()
=== Routed authorization lookup and warm requests

A cold authorization-set lookup constructs bounded principal keys, unions grant and deny
vectors, checks conditional and exception entries, subtracts denies, and sorts admitted
names. Let $L_s,C_s$ count visited allow/deny graph occurrences, let $E_s$ count visited
conditional entries and exception references, let $X_s$ include temporal parsing and
other session/exception lexical work, and let $kappa_s$ be session-key bytes. Its expected
cost is

$ O(kappa_s + X_s + (L_s+C_s+E_s+abs(A_s) log(2+abs(A_s))) ell). $

The shorter $O(L_s+C_s+abs(A_s)log abs(A_s))$ expression is valid only when session
strings, exception lists, and temporal lexical forms are bounded. On a warm hit, the
implementation hashes the owned session key, takes one shard read lock, clones shared
references, and releases the lock before query evaluation. Its expected work is
$O(kappa_s)$ and hence $O(1)$ for the fixed session used in H2.

Let $R(q)$ denote `query_as`'s initial parse, algebra rewrite, serialization, and engine
reparse. A warm routed request has the decomposition

$ E[T_"routed"] = O(1)_"route" + O(kappa_s)_"cache" + R(q)
  + Q(q,V_p) + O(Y(q,V_p) ell). $

The timed closure materializes the library `QueryResult`; result canonicalization and
pair comparison happen outside it. Rust's randomized route map and the view's hash set
give expected constant-time lookup for generated bounded keys, not an adversarial
worst-case guarantee. An explicit `GRAPH <g>` first makes an expected constant-time
visibility check. If the target is invisible it stops there; if visible, the current
engine linearly searches the selected store's named-graph vector. An unbound `GRAPH ?g`
without a recognized graph-name prefix restriction scans all $g_p$ query-time named slots
and checks visibility, giving the store-local lower bound $Omega(g_p+Y)$. The engine can
instead range-scan a lazily built name index for a recognized
`STRSTARTS(STR(?g), prefix)` conjunct, so the full-scan lower bound is not universal. A
logical view over one monolithic multi-Pod store remains tenant-count-dependent for the
unrestricted path.

*Proposition 2 (conditional independence from unrelated Pods).* Consider deployments
indexed by $P$. Hold the target `PodStore`, route key, session and warm cache entry, query,
result, and engine context fixed; assume bounded key length, expected constant-time route
lookup, no concurrent invalidation, and no target-policy dependency on an unrelated
store. Then expected warm routed operation count is $Theta(1)$ as a function of unrelated
Pod count.

_Proof._ The immutable route map is the only request-path structure containing all Pods,
and its expected lookup cost is constant. The selected store is then fixed; its session
cache, authorization set, view, graph collection, and RDF indices do not change when
unrelated stores are appended. A request must perform at least its route access, giving
the matching $Omega(1)$ lower bound in $P$. $square$

The proposition is not constant in query or target-Pod size, and collision-degenerate
routing has worst-case $O(P)$. Nor does it promise equal wall time: larger resident state
can change cache residency, memory pressure, allocator behavior, or paging. H2 therefore
requires both wall and process-CPU confidence bounds.

=== Native construction and persistent state

The native fixture seeds one server root and writes each generated container, content
document, and ACL once; it constructs neither a per-Pod view nor a Pod route. Let $N_0$
and $B_0$ be seeded resources and body bytes and let $C_"write"(r,B_r)$ be backend write
cost, including metadata, blob, and containment-edge work. Construction obeys

$ W_("native,build") = Theta(1) + sum_(r=1)^(N_0) C_"write"(r,B_r)
  = Omega(N_0+B_0). $

The benchmark's in-memory metadata store keeps a container's children in a vector and
linearly checks a new edge. If $k_c$ is container $c$'s final direct-child count, bounded
IRIs and expected hash behavior give the more precise upper bound

$ E[W_("native,build")] = O(B_0 + N_0 ell + sum_c k_c^2 ell). $

The shared-origin Pod-count intervention does *not* satisfy uniformly bounded fan-out:
all $P$ Pod roots are inserted as children of the one server root. The duplicate check on
that growing vector therefore performs $Theta(P^2)$ string-equality invocations at the
root, yielding $Omega(P^2)$ work and $O(P^2 ell)$ lexical work even though fixed per-Pod
content makes $N_0,B_0=Theta(P)$. A linear specialization would require a bounded-fan-out
hierarchy or an indexed child set. This is a property of the current in-memory metadata
implementation and shared-root topology, not an inherent cost of WAC, Solid, or an
arbitrary remote store. Native construction trends must therefore be interpreted in light
of this root-fan-out term rather than against a linear-in-$P$ expectation.

Persistent space consists of stored bodies/metadata and authentication, ACL, and body
caches. The evaluated app configures a 1,024-entry verified-token cache; its ACL cache is
entry-budgeted (4,096 by default), whereas its body cache is byte-budgeted. A fixed entry
count still permits variable IRI, ETag, and parsed-triple payloads. Thus cache counts do
not imply constant bytes, and the in-memory store has the unavoidable lower bound
$Omega(N_0+B_0)$.

=== Native server-wide request assembly

The handler authenticates, parses the request and query, takes a snapshot read guard, and
retains that guard through authorized-dataset assembly, evaluation, and response
serialization. Assembly recursively enumerates containment into a comparison-ordered
set, clones the set, and adds one derived ACL candidate for each non-ACL resource. If $N$
resources are reached and $K$ is the resulting candidate set, then

$ N <= abs(K) <= 2N. $

If $E$ child-IRI occurrences are returned by containment listing, the enumeration term is

$ C_"enum" = sum_(c in "listed containers") C_"list"(c)
  + O((N+E) log(2+N) ell). $

This is deliberately not simplified unconditionally to $O(N log N)$: that shorthand
requires tree-like containment with $E=O(N)$ and bounded IRI comparisons. Every candidate
then constructs an ACL chain of length $h(r)$, obtains a combined read plan, resolves the
nearest present ACL, and evaluates WAC. Backend plan cost remains $C_"plan"(r,h(r))$.
Let $C_"resolve"(r,h(r),a_r)$ contain plan/candidate validation, the live metadata
confirmation of a plan-present ACL, and parsed-ACL cache work. A cache hit clones the
cached $a_r$-triple vector; a miss reads and parses ACL bytes, inserts the parse, and may
scan the fixed-capacity cache for LRU eviction. After resolution, the current local matcher
repeatedly scans its triple vector for subjects, scope, agents, origin, and modes; write
$C_"match"(a_r,ell)=O(a_r^2 ell)$ for its worst case. An allowed authenticated request
runs the matcher for both the user and public audience, which changes only the constant.

For admitted candidates, the handler reads and parses source RDF, scopes blank nodes,
writes all admitted triples to an aggregate N-Quads string, and parses that string into a
temporary authorized dataset $D_A$. Let $S_A$ be N-Quads lexical bytes. Preserving admitted empty
graphs calls the current linear-search `ensure_named` once per name, contributing
$Theta(G_A^2)$ term-equality invocations and $O(G_A^2 ell)$ lexical work for the distinct
names used here. The current request decomposition is therefore

$ T_"http" &= C_"authn" + C_"protocol" + C_"qparse"(q) \
  &+ C_"snapshot" \
  &+ C_"enum"(N,E,ell) \
  &+ sum_(r in K) (O(h(r)ell) + C_"plan"(r,h(r)) \
  &+ C_"resolve"(r,h(r),a_r) + C_"match"(a_r,ell)) \
  &+ C_"admit"(B_A,T_A) + C_"rebuild"(S_A,T_A,G_A) \
  &+ Q(q,D_A) + C_"ser"(q,Y,Z). $

Here $C_"snapshot"$ is constant request-local work only when lock acquisition is
uncontended; scheduler waiting is not RAM work. $C_"admit"$ contains admitted-body backend
reads and source parsing, while $W_"NQ"$ below contains blank-node scoping plus quad
formatting and aggregate-buffer emission:

$ C_"rebuild" = W_"NQ"(S_A,T_A) + L_"NQ"(S_A,T_A,G_A) + O(G_A^2 ell). $

Protocol dataset parameters are processed before assembly but do not prune the resources
assembled. When the reserved union-default graph is requested, its current vector
membership expansion adds another $O(G_A^2 ell)$ term for the benchmark's reserved-only
dataset. With $M_q$ other default-graph IRIs, the general upper is
$O(G_A(M_q+G_A)ell)$; without a query dataset it is a constant-time no-op. Generally
$C_"ser"=Omega(Z)$; for the benchmark's fixed projected width and bounded lexical terms,
$C_"ser"=Theta(Z)$. These quadratic terms are important, but remain constant with respect
to $P$ in H2 when the admitted target slice is fixed.

*Proposition 3 (server-wide visitation lower bound).* Suppose enumeration succeeds and
every resident Pod contributes at least one distinct reachable resource. The current
native endpoint performs $Omega(P)$ candidate-loop work per request even if every added
Pod is unreadable and the target answer is fixed.

_Proof._ Every reached resource enters the discovered set and then the candidate set, so
$abs(K)>=N$. The loop invokes the read-plan path before WAC can reject a resource. Since each
non-empty Pod contributes a distinct resource, $N>=P$ and at least one candidate-loop and
plan-call attempt occurs per Pod-contributed resource. $square$

This lower bound characterizes the pinned endpoint, not access-controlled SPARQL as a
problem. If containment is tree-like with $E=O(N)$ and aggregate listing work
$sum_c C_"list"(c)=O((N+E)ell)$; lexical length, hierarchy depth, policy size, parser work
per byte, and per-candidate plan/resolution backend costs are bounded; admitted
data/query/output are fixed; and $N=Theta(P)$, the corresponding conditional upper bound
is $O(P log P)+Q(q,D_A)+C_"ser"(q,Y,Z)$. Under the benchmark's fixed-width output this last
term is $Theta(Z)$. If $G_A$ instead grows with $P$, graph preservation or union-default
expansion can dominate.

Native request-local auxiliary space includes the traversal queue, discovered and
candidate sets, graph-name strings, aggregate N-Quads, the temporary indexed graph, one
resource's transient parse/plan state, query intermediates, and the response. Write
$U_"max"$ for the largest one-resource transient parse/plan state:

$ S_"http" = O((N+E)ell + S_A + S_"graph"(T_A) + U_"max"
  + I(q,D_A) + Z(q,D_A)). $

For $c$ simultaneous comparable native reads, aggregate request-local transient space is
$O(c S_"http")$, apart from shared caches and stores. The snapshot read guard is constant
space per request but extends the lifetime of this read phase: readers may overlap, while
an LDP mutation through the same `LdpState` and its matching write-guard path waits for all
of them. Lock acquisition may add unbounded scheduler wait under contention. Routed requests share
resident Pod stores and warm authorization sets but still replicate query intermediates
and outputs. These bounds do not predict concurrent throughput.

=== Composition with SPARQL complexity

For the classical graph-pattern language studied by Pérez, Arenas, and Gutiérrez,
general evaluation is PSPACE-complete in combined complexity, every fixed pattern is in
LOGSPACE in data complexity, and UNION-free well-designed patterns are coNP-complete in
combined complexity @perez-tods. These classify a formal decision problem, not elapsed
time. LOGSPACE is a space upper bound, not logarithmic time, and those statements do not
automatically cover all SPARQL 1.1 workload features, notably property paths and
aggregates. We therefore retain $Q(q,D)$, including scans, joins, `OPTIONAL`, negation,
paths, grouping, and intermediates, instead of converting complexity classes into
latency predictions. Successful full HTTP processing over a worst-case input family must
at least consume the query and emit the answer, giving the elementary input/output lower
bound $Omega(abs(q)+Z)$; the routed API must at least visit its $Y$ result cells and copy
the lexical bytes of bound terms.

=== Per-request cost is not capacity

Even if Proposition 2 holds empirically, total construction and resident storage are at
least $Omega(P)$ for non-empty resident Pods. For $c$ worker cores and request arrival
rate $lambda$, $lambda E["CPU service"] < c$ is necessary but not sufficient for queue
stability when CPU service is measured in core-seconds per request, arrivals/service are
stationary, and the $c$ core-seconds per second are available to this workload. Snapshot
duration, locks, memory bandwidth, storage, other load, and heavy-tailed queries can make
that condition insufficient. The experiment fixes concurrency at one and cannot estimate
saturation. It isolates a single request's server work, not the maximum user population
or throughput of a service.

== Experimental Method <method>

=== Prospective protocol and evidence discipline

The study protocol was written before valid canonical timing and is versioned separately
from the operational README. Its research questions, factor matrix, repetitions,
estimators, practical-growth margins, resource guards, and publication rules are frozen.
After canonical measurement begins, changes are append-only and disclose their reason and
what outcomes, if any, had been inspected.

The final protocol is version 1.33. Two pre-result reliability amendments are especially
relevant to interpretation. One canonical attempt reached the primary campaign but was
invalidated when a study credential expired during a long cell; only the failure evidence
needed to identify the HTTP 401 was inspected, and every partial timing from that attempt
is excluded. A later launch encountered changing client egress addresses during source
transfer; no benchmark service ran. The amended launcher lengthens the study token to the
host watchdog and retries setup transport only after synchronizing the same exact `/32`
SSH ingress rule. Neither amendment changes data, queries, policies, endpoint logic,
measurements, hardware, or estimands. The full amendment log, including diagnostic pilots
that motivated measurement-boundary corrections, accompanies the artifact. Amendments
1.29 and 1.30 add block-aware paired intervals, output digests, cross-P causal-invariant
checks, and source-faithful claim qualifications. Amendment 1.31 corrects the publication
validator to the emitted 26-record correctness-file structure and semantic principal
labels. All three were fixed without inspecting a canonical performance outcome and
change no raw observation or H2 decision rule.

After performance outcomes had been inspected, the first full publication export refused
an inconsistent manifest-cardinality expectation. Amendment 1.32 records the correction:
all 1,280 raw files emit applicability metadata for all eight templates, so the emitted
record count is 10,240; the former 3,680 value counted only templates selected for
observation. The change is confined to the publisher's structural check and version gate.
It changes no raw or derived artifact, analyzer byte, interval, decision, figure, or
interpretive claim; the two analysis runs reported here were already byte-identical.
The next export then exposed a second stale publisher boundary: its exact paired-CSV
schema omitted the analyzer's pre-existing `inference_status` field. A complete
derived-schema audit also found that the publisher incorrectly required the
materializer-specific authorization-triple counter in native rows, where it is
intentionally blank. Amendment 1.33 makes both checks lane- and schema-faithful, binds
the H2 query-family labels to the summary, and closes duplicate-header and row-width
parsing gaps. Synthetic headers are now independent of publisher constants. These
changes likewise affect only the publisher, fixture, and version gate; they change no
empirical artifact or interpretation.

Canonical publication evidence must pass the analysis program's fail-closed validator.
It rejects missing or mismatched checksums, dirty trees, debug builds, wrong campaign
labels, mixed hosts or toolchains, incomplete five-block cells, absent schedule metadata,
incorrect warm-up/repetition settings, non-Linux process CPU, more than one Rayon worker,
or a kernel affinity mask containing anything other than one logical CPU. Failed and
interrupted streams remain available with stderr and the campaign sentinel; they are not
silently repaired or pooled.

=== Deterministic corpus generator

The generator exposes independent inputs for root seed, Pod count $P$, documents per Pod
$D$, exact content triples per document $T$, lexical/containment hierarchy depth,
per-mille own-ACL coverage, audience mixture, URI-origin topology, and content domain.
Increasing $P$ leaves the bytes of already-present Pods unchanged because every
Pod/document derives a stable sub-seed. Each content resource renders as exactly one
named graph. Container and WAC documents are generated separately, so $T$ never silently
includes policy or containment triples.

The *shared-origin* topology places all Pods beneath one authority and is used by the
native in-process handler lane. The *origin-per-Pod* topology assigns an authority to every Pod and is
used by the routed lane. These URI arrangements are also exercised in generator and
correctness tests, but the performance intervention is the physical boundary, not URI
spelling: the native endpoint is one shared server, whereas the primary materialized path
routes to one PodStore.

Social documents use profile/post/author/mention/tag/time-shaped predicates. Health
documents use patient/observation/code/value/unit/time-shaped predicates. Exact counts
are aligned between domains while predicates and local graph shapes differ. The domains
are controlled structural probes, not samples from deployed social-media or health Pods.

=== Calibration without a population claim

No defensible dataset describes the joint distribution of data size, policy shape, or
query workload in a future large-scale Solid deployment. We therefore use three labels:

- *Controlled synthetic cells* vary one named factor (or the predeclared placement by
  depth grid) while fixing the others.
- *Count-calibrated cells* choose settings near published corpus totals but do not copy
  source records, correlations, or query distributions.
- *Stress cells* test an implementation envelope and make no population-frequency claim.

SolidBench's published default contains 1,531 vaults, 158,233 RDF files, and 3,556,159
triples and derives its social structure from LDBC SNB @solidbench @ldbc-snb. Our
1,531-Pod setting uses 103 documents and 22 exact content triples per document. It is
count-calibrated: it does not reproduce SolidBench's exact totals or LDBC correlations.
TIDAL evaluates up to 256 participant Pods with one RDF/Turtle file per participant and
requests up to 128 clinical variables @tidal. Our corresponding setting matches only the
participant/file dimensions; its $T=128$ means 128 generated RDF triples, not 128
variables. Synthea offers structurally richer longitudinal synthetic records @synthea,
but no converted Synthea lane is part of the mandatory experiment. ESPRESSO's independent
variation of Pod size, file count, accessibility, Pods, and servers motivates broad stress
coverage without making those ranges representative @espresso.

#figure(
  table(
    columns: (0.24fr, 0.26fr, 0.25fr, 0.25fr),
    align: (left, left, left, left),
    table.header[Scenario][Generated setting][External anchor][Permitted interpretation],
    [Social small], [$P=100$, $D=103$, $T=22$], [SolidBench-shaped count scale],
    [controlled capacity precursor],
    [Social published-count anchor], [$P=1,531$, $D=103$, $T=22$],
    [1,531 vaults; published file/triple totals @solidbench],
    [count-calibrated, not SolidBench-derived],
    [Compact health], [$P=64$, $D=1$, $T=32$], [none], [synthetic compact cell],
    [Health participant/file anchor], [$P=256$, $D=1$, $T=128$],
    [256 participants; one file; up to 128 requested variables @tidal],
    [participant/file-calibrated; triple width is stress],
  ),
  caption: [Scenario-calibrated settings and the claims their external anchors do and do
  not support. Realized generated graph and triple counts are reported with the results.],
) <tab-scenarios>

=== Policies, principals, and independent oracle

Audience assignment is deterministic in the corpus seed. Private resources inherit or
receive an owner grant, public resources receive a public-read grant, and shared
resources receive owner and named-recipient grants. Category defaults preserve the same
decision as own-ACL coverage changes. The benchmark excludes groups, clients, issuers,
conditional rules, explicit denies, ACP, and ODRL so the independent oracle remains a
direct predicate over document ownership and assigned audience.

Four principal classes exercise the correctness matrix: the target Pod's owner, its
named recipient, an authenticated stranger, and an anonymous requester. Before any
engine call, the oracle selects documents using only the audience metadata and renders
them into a physical N-Quads dataset. Expected results are ordinary SPARQL evaluation
over that dataset. The two access-controlled paths are then compared with that expected
duplicate-preserving multiset. The audience predicate and physical content selection invoke
neither authorization path, so the check independently tests authorization-layer selection.
Expected and actual answers nevertheless use the same SPARQ evaluator and substantially the
same canonicalization/serialization code. The oracle therefore does not independently
validate SPARQL conformance or evaluator correctness.

=== Query families

Every corpus supplies eight deterministic SPARQL 1.1 `SELECT` templates targeting Pod 0.
The applicability threshold makes a missing predicate an explicit inapplicable record
rather than an empty substitute query.

#figure(
  table(
    columns: (0.13fr, 0.30fr, 0.13fr, 0.44fr),
    align: (left, left, right, left),
    table.header[ID][Family][Minimum $T$][Purpose],
    [`q1`], [graph-bound point lookup], [1], [fixed graph and subject; routing/cache floor],
    [`q2`], [selective star pattern], [3], [multi-predicate selectivity with a non-empty date filter],
    [`q3`], [multiway join], [8], [within-graph join and intermediate work],
    [`q4`], [`OPTIONAL`], [5], [left joins with alternating predicates],
    [`q5`], [`FILTER NOT EXISTS`], [5], [negation under authorized absence],
    [`q6`], [bounded property path], [8], [two-edge path evaluation],
    [`q7`], [aggregate/grouping], [2], [group by principal and count],
    [`q8`], [unbound `GRAPH ?g` scan], [1], [enumerate the authorized graph namespace],
  ),
  caption: [Primary query workload. With $T=8$, every family is applicable in the
  correctness and Pod-count campaigns.],
) <tab-queries>

=== Correctness gate

Canonical timing begins only after a matrix over both domains and topologies,
$P in {1,8,32}$, $D=8$, $T=8$, ACL coverage in ${0.10,1.00}$, and seeds
${17,42,101}$ succeeds for all four principal classes. The gate requires non-vacuous
owner allow-all, named-recipient allow-one-class, dedicated stranger deny-all, and
anonymous cases; explicit and variable graph queries; duplicate preservation;
unreadable-versus-absent equality; and revocation after replacing an applicable ACL.

Every timed process performs an additional lane-local oracle preflight before emitting
observations. A result mismatch, non-200 HTTP response, malformed result document, or
missing route terminates the process. H1 is therefore a validity gate rather than a
statistical null hypothesis.

=== Controlled factor matrix

The primary intervention varies resident Pods over
$P in {1,8,64,512,2048}$ with $D=16$, $T=8$, depth 3, own-ACL coverage 0.25, and an
all-private audience. The requester owns Pod 0, so exactly its 16 content documents remain
readable at every $P$ while added Pods are non-empty and unreadable. Both domains, both
lanes, and all eight query families run in every process block.

Before producing any canonical summary, the analyzer enforces that causal premise in both
timing and instrumentation profiles. Within every lane/domain/query/block it requires one
query hash and one result-hash/row-count pair across all five $P$ values. After joining each
fixture to its construction record, it also requires target and evaluation readable-document
counts to equal $D=16$ and remain constant across $P$, separately by profile, lane, domain,
and block. Corpus hashes and total stored-state counts are expected to change with $P$ and
are not subjected to this invariance rule.

#figure(
  table(
    columns: (0.21fr, 0.42fr, 0.37fr),
    align: (left, left, left),
    table.header[Block][Values; fixed baseline][Identified contrast],
    [Unrelated Pods], [$P={1,8,64,512,2048}$; target Pod, answer, $D=16$, $T=8$ fixed],
    [effect of unrelated resident population],
    [Documents/Pod], [$D={1,8,32,128,512}$; $P=16$, $T=8$, depth 3, coverage .25],
    [resource/document scale],
    [Triples/document], [$T={1,8,32,128}$; $P=16$, $D=32$, depth 3, coverage .25],
    [content width at fixed resource count],
    [Placement by depth], [coverage ${0,.10,.25,1}$ by depth ${1,3,6}$; $P=16$, $D=16$, $T=8$],
    [policy density and lexical inheritance depth],
    [Visibility], [public/readable fraction ${.01,.10,.50,1}$; anonymous, $P=1$, $D=512$, $T=8$],
    [authorized-slice size without deployment growth],
  ),
  caption: [Controlled sensitivity blocks. Owner-controlled blocks use an all-private
  audience; visibility changes only in its dedicated anonymous block.],
) <tab-factors>

Sensitivity cells select `q1` and `q8`, bracketing fixed-graph access and authorized
graph enumeration. Scenario cells select `q1` and primarily test construction and
resident capacity. They supplement rather than replace the all-eight-family Pod-count
intervention. A cell that cannot pass the resource guard is recorded as skipped with the
observed condition; it is never silently replaced by a smaller setting.

=== Measures and instrumentation boundaries

Every raw record identifies its run UUID, source commit and dirty state, timestamp, host,
cloud metadata, OS, architecture, Rust version, profile and features, campaign, lane,
domain, topology, every corpus factor, corpus and query hashes, principal, process block,
configuration order and seed, configured warm-ups/repetitions, concurrency, Rayon worker
count, and effective kernel CPU affinity.

Primary measurements are server wall-clock latency, Linux process CPU, cold
generation/load/materialization/seeding phases, current and peak RSS, allocation and
reallocation operations/bytes, native backend operations and maximum in-flight calls,
response size, result cardinality/hash, and correctness. RSS is deliberately whole-process:
the current sample follows source-corpus release but includes the physical oracle graph,
and the high-water mark also includes generation. These are conservative capacity
indicators, not isolated production-server heap estimates.

Timing and instrumentation are separate profiles. The timing binary uses the production
server's mimalloc allocator and ordinary in-memory storage. The instrumentation binary
delegates through the same allocator while counting allocations and wraps both storage
seams to count backend calls. Its counter-inflated wall and CPU times never enter latency
or paired-overhead estimates. Construction allocation scopes cover service parsing,
materialization, routing, or LWS seeding/application assembly, and close before the oracle
graph is constructed.

Client-side token and DPoP proof construction occurs outside the in-process handler
interval; a fresh proof is generated for each request. Server-side verification, route
handling, dataset assembly, evaluation, response serialization, and complete body
consumption occur inside. Socket, TLS, proxy, and network transport are absent.

=== Repetition, ordering, and canonical environment

Each timing cell is independently rebuilt in five process blocks using corpus seeds
17, 42, 101, 314, and 2718. A block performs ten unmeasured warm-up guarded/plain pairs
per query and records thirty randomized pairs. The matching instrumentation block performs
one warm-up and one measured pair. The complete configuration order and guarded/plain
order are pseudorandomized from recorded seeds, reducing sensitivity to layout and drift
effects documented in systems measurement studies @mytkowicz. Repetition at the rebuilt
process/corpus level follows the principle that influential hierarchy levels, rather
than requests alone, require independent replication @kalibera-jones.

The canonical host runs Linux with one Rayon worker, concurrency one, and a kernel
affinity mask containing exactly one logical CPU. Host identity, instance type, region,
kernel, CPU model, memory, Rust toolchain, source commit, and effective affinity are
reported in the canonical provenance attached to every result accessor and generated
figure. Local workstation observations are diagnostic only and cannot enter headline evidence. Cloud
execution follows reproducibility controls for environment disclosure, repetition, and
uncertainty @papadopoulos-cloud.

Before each cell, the runner refuses to proceed if free disk is below 25 GiB or observed
peak process RSS reaches 70% of physical RAM; the disposable host also enforces a 70%
cgroup memory ceiling. A 12-hour host watchdog, delete-on-termination storage, tagged
orphan checks, restricted SSH, and an independent termination supervisor bound resource
exposure. Planned AWS spend is at most USD 80 with a hard study ceiling of USD 100.

=== Statistical analysis

Raw repetitions are retained; no minimum-of-$K$ statistic is used. For every cell we
report median, p95, p99, median absolute deviation, and empirical distributions. Paired
guarded/plain effects are summarized as both difference and ratio, because ratios can be
unstable when the reference query is extremely short.

The frozen protocol requires effect sizes *and uncertainty intervals* for the paired RQ4
guarded-stack/content-reference contrast. Amendment 1.29 fixes a hierarchical cluster
bootstrap that resamples the five complete process blocks and then intact guarded/plain
pair-effect vectors within each selected block. Wall and CPU components share each
bootstrap resample; non-positive ratio denominators and incomplete or unbalanced blocks
fail closed. Descriptive p95 values remain tail summaries and are not presented as
confidence intervals. These cellwise intervals do not support the former H4 cross-query
ordering because “non-trivial evaluation” was not operationalized and query order was not
randomized.

For H2, wall and positive process-CPU outcomes are analyzed separately for every lane,
domain, and query family. Let $m_(b,p)$ be the within-block median for block $b$ at Pod
count $p$. Pod-count elasticity is the equal-weight mean of block-specific ordinary
slopes from regressing $log m_(b,p)$ on $log p$. The endpoint ratio divides the median of
all requests at $P=2048$ by the corresponding median at $P=1$.

A hierarchical bootstrap resamples complete corpus/process blocks with replacement and
then resamples requests within every selected Pod-count cell. We use at least 10,000 draws
and a fixed recorded seed. A family meets the practical-growth definition only if the
upper percentile-bootstrap endpoint is at most 1.10 for the endpoint ratio and at most
0.10 for elasticity, for both wall and process CPU. Families are never pooled into one
pass. In particular, failure to reject a null hypothesis is not treated as equivalence.
Sensitivity and scenario trends without a preregistered decision threshold are descriptive
or explicitly labeled exploratory.

== Results <results>

=== Analysis population and authorization correctness (RQ1)

The fail-closed analysis admitted
#headline_timing("ac_sparql.campaign.valid_timing_observations", digits: 0)
timing observations and
#headline_timing("ac_sparql.campaign.valid_instrumentation_observations", digits: 0)
instrumentation observations, organized into
#headline_timing("ac_sparql.campaign.completed_paired_cells", digits: 0)
complete paired cells and
#headline_timing("ac_sparql.campaign.complete_process_fixtures", digits: 0)
complete process fixtures. It consumed
#headline_timing("ac_sparql.campaign.raw_files", digits: 0)
checksummed raw files and retained
#headline_timing("ac_sparql.campaign.excluded_attempts", digits: 0)
excluded or interrupted attempts outside every estimate. Host, source, analysis, protocol,
and raw-archive identities are carried by the provenance attached to every accessor.

The independent correctness matrix contains
#headline_timing("ac_sparql.correctness.records", digits: 0)
checksummed configurations and
#headline_timing("ac_sparql.correctness.oracle_comparisons", digits: 0)
exact query/principal comparisons, including
#headline_timing("ac_sparql.correctness.materialized_oracle_comparisons", digits: 0)
materialized-lane comparisons. The complete exact-result gate
#timing_verdict(
  "ac_sparql.correctness.passed",
  yes: [passed for every admitted configuration],
  no: [did not pass for every admitted configuration],
). The matrix itself covers positive, negative, and anonymous principals, unbound graph
enumeration, and duplicate-preserving result bags. The separately source-commit-attested
no-leak, unreadable-versus-absence, and two live-revocation tests
#timing_verdict(
  "ac_sparql.correctness.all_required_probes_passed",
  yes: [all passed],
  no: [did not all pass],
). Thus H1
#timing_verdict(
  "ac_sparql.correctness.passed",
  yes: [passes within the generated WAC subset and exact-result-bag scope],
  no: [does not pass within the generated WAC subset and exact-result-bag scope],
). This is authorization-selection evidence, not an independent SPARQL-conformance test
or a timing non-interference result.

#timing_table(
  "ac_sparql.correctness.records",
  columns: (0.68fr, 0.32fr),
  header: ([Evidence population], [Count]),
  rows: (
    ([Correctness configurations],
      (key: "ac_sparql.correctness.records", digits: 0)),
    ([Oracle comparisons],
      (key: "ac_sparql.correctness.oracle_comparisons", digits: 0)),
    ([Materialized oracle comparisons],
      (key: "ac_sparql.correctness.materialized_oracle_comparisons", digits: 0)),
    ([Timing observations],
      (key: "ac_sparql.campaign.valid_timing_observations", digits: 0)),
    ([Instrumentation observations],
      (key: "ac_sparql.campaign.valid_instrumentation_observations", digits: 0)),
    ([Complete paired cells],
      (key: "ac_sparql.campaign.completed_paired_cells", digits: 0)),
    ([Complete process fixtures],
      (key: "ac_sparql.campaign.complete_process_fixtures", digits: 0)),
    ([Retained excluded attempts],
      (key: "ac_sparql.campaign.excluded_attempts", digits: 0)),
  ),
  label_columns: 1,
  caption: [Complete evidence populations admitted by the canonical validator.
  A correctness mismatch invalidates its affected run; instrumentation timing is never
  used as latency evidence.],
) <tab-population>

=== Warm cost under unrelated-Pod growth (RQ2)

@fig-scaling plots the two endpoint query shapes over the complete Pod-count
intervention. Pod 0, its sixteen documents, readable graph set, result identity, query,
principal, and all non-P factors are invariant within each series. Lines are descriptive
medians; uncertainty and the four-part decision remain in the complete H2 tables.

#timing_figure(
  "ac_sparql.figure.pod_scaling_latency",
  caption: [Warm guarded-query wall time from one to 2,048 resident Pods for routed and
  native in-process lanes, both domains, and the predeclared endpoint query shapes.
  Process-CPU estimates and uncertainty are reported in the complete H2 tables.],
) <fig-scaling>

For the routed composition,
#headline_timing("ac_sparql.h2.materialized.rollup.pass_count", digits: 0)
of
#headline_timing("ac_sparql.h2.materialized.rollup.cells", digits: 0)
domain-by-query cells meet all four predefined wall/CPU criteria and
#headline_timing("ac_sparql.h2.materialized.rollup.fail_count", digits: 0)
do not. The routed result
#timing_verdict(
  "ac_sparql.h2.materialized.rollup.all_cells_meet",
  yes: [meets the minimal unrelated-Pod growth definition across the complete tested matrix],
  no: [#timing_verdict(
    "ac_sparql.h2.materialized.rollup.no_cells_meet",
    yes: [meets that definition in no tested cell],
    no: [is mixed across the domain-by-query cells],
  )],
).

For the native in-process handler,
#headline_timing("ac_sparql.h2.http.rollup.pass_count", digits: 0)
of
#headline_timing("ac_sparql.h2.http.rollup.cells", digits: 0)
cells meet the criterion and
#headline_timing("ac_sparql.h2.http.rollup.fail_count", digits: 0)
do not. The native result
#timing_verdict(
  "ac_sparql.h2.http.rollup.all_cells_meet",
  yes: [meets the practical-growth definition across the complete tested matrix; this is
    contrary to a blanket empirical prediction that every native cell would exceed a
    margin, although the source-level visitation lower bound remains],
  no: [#timing_verdict(
    "ac_sparql.h2.http.rollup.no_cells_meet",
    yes: [meets the definition in no tested cell, a pattern consistent with but not by
      itself proof of the source-derived server-wide visitation mechanism],
    no: [is mixed across cells and therefore neither uniformly supports nor uniformly
      contradicts the native-path prediction],
  )],
). All cellwise estimates are disclosed below; no family average substitutes for a
cell that does not meet a margin.

The next four tables give one row for each of the four preregistered metrics in every
query cell. The companion verdict table supplies the fifth line per cell. Ratio and
elasticity intervals are marginal 95% block-cluster intervals.

#timing_table(
  "ac_sparql.h2.materialized.social.q1.wall.median_ratio",
  columns: (0.13fr, 0.31fr, 0.19fr, 0.19fr, 0.18fr),
  header: ([Query], [Metric], [Estimate], [CI low], [CI high]),
  rows: h2_metric_rows("materialized", "social"),
  label_columns: 2,
  caption: [Routed feasibility composition, social domain: all eight query families and
  all four wall/CPU H2 metrics.],
) <tab-h2>

#timing_table(
  "ac_sparql.h2.materialized.health.q1.wall.median_ratio",
  columns: (0.13fr, 0.31fr, 0.19fr, 0.19fr, 0.18fr),
  header: ([Query], [Metric], [Estimate], [CI low], [CI high]),
  rows: h2_metric_rows("materialized", "health"),
  label_columns: 2,
  caption: [Routed feasibility composition, health domain: all eight query families and
  all four wall/CPU H2 metrics.],
)

#timing_table(
  "ac_sparql.h2.http.social.q1.wall.median_ratio",
  columns: (0.13fr, 0.31fr, 0.19fr, 0.19fr, 0.18fr),
  header: ([Query], [Metric], [Estimate], [CI low], [CI high]),
  rows: h2_metric_rows("http", "social"),
  label_columns: 2,
  caption: [Native in-process handler, social domain: all eight query families and all
  four wall/CPU H2 metrics.],
)

#timing_table(
  "ac_sparql.h2.http.health.q1.wall.median_ratio",
  columns: (0.13fr, 0.31fr, 0.19fr, 0.19fr, 0.18fr),
  header: ([Query], [Metric], [Estimate], [CI low], [CI high]),
  rows: h2_metric_rows("http", "health"),
  label_columns: 2,
  caption: [Native in-process handler, health domain: all eight query families and all
  four wall/CPU H2 metrics.],
)

#timing_table(
  "ac_sparql.h2.materialized.social.q1.minimal_overhead",
  columns: (0.22fr, 0.22fr, 0.18fr, 0.38fr),
  header: ([Path], [Domain], [Query], [Four-part criterion]),
  rows:
    h2_verdict_rows("materialized", [Routed], "social", [Social])
    + h2_verdict_rows("materialized", [Routed], "health", [Health])
    + h2_verdict_rows("http", [Native], "social", [Social])
    + h2_verdict_rows("http", [Native], "health", [Health]),
  label_columns: 3,
  caption: [Joint H2 verdict for every path, domain, and query cell. Each verdict is the
  conjunction of the two ratio and two elasticity upper-bound rules shown above.],
)

=== Observable work proxies and cost decomposition (RQ3 / E3)

The instrumentation profile records work-volume proxies separately from uninstrumented
timing. It has no request-stage clocks and therefore cannot assign elapsed-time or CPU
shares to authentication, containment, WAC matching, RDF parsing, dataset reconstruction,
query evaluation, or serialization.

For the graph-bound q1 mechanism probe, both domains are reported at both Pod-count
endpoints together with the endpoint ratio:

#timing_table(
  "ac_sparql.mechanism.http_backend_ops.social.p1",
  columns: (0.26fr, 0.42fr, 0.32fr),
  header: ([Domain], [Quantity], [Value]),
  rows: (
    ([Social], [Operations at P=1],
      (key: "ac_sparql.mechanism.http_backend_ops.social.p1", digits: 0)),
    ([Social], [Operations at P=2,048],
      (key: "ac_sparql.mechanism.http_backend_ops.social.p2048", digits: 0)),
    ([Social], [Endpoint growth ratio],
      (key: "ac_sparql.mechanism.http_backend_ops.social.growth_ratio", digits: 3)),
    ([Health], [Operations at P=1],
      (key: "ac_sparql.mechanism.http_backend_ops.health.p1", digits: 0)),
    ([Health], [Operations at P=2,048],
      (key: "ac_sparql.mechanism.http_backend_ops.health.p2048", digits: 0)),
    ([Health], [Endpoint growth ratio],
      (key: "ac_sparql.mechanism.http_backend_ops.health.growth_ratio", digits: 3)),
  ),
  label_columns: 2,
  caption: [Native q1 backend-operation medians and endpoint ratios. Counts come from the
  deterministic instrumentation profile; its timings are excluded from latency evidence.],
)

#timing_figure(
  "ac_sparql.figure.http_backend_operations",
  caption: [Native-handler backend operations per q1 request over all Pod-count levels,
  separately for social and health. These are mechanism counts, not stage-time shares.],
) <fig-backend>

At every resident-Pod level, the social and health q1 backend-count series coincide and
increase with the hosted resource population. This is direct work-volume evidence for
root-first native discovery, but it does not assign any share of wall or CPU time to
discovery or another request stage.

The complete factor-sensitivity panel contains
#headline_timing("ac_sparql.campaign.sensitivity_guarded_cells", digits: 0)
guarded cells spanning document count, triples per document, ACL placement and depth, and
readable fraction. It reports the predeclared q1 and q8 timing medians and companion
instrumentation counts for both lanes and domains; no favorable query or domain is selected
and no fitted trend is treated as confirmatory.

#timing_figure(
  "ac_sparql.figure.factor_sensitivities",
  caption: [Complete exploratory factor-sensitivity matrix for both paths, domains, q1 and
  q8, showing timing-profile medians and instrumentation work counts. Panel-local scales
  are descriptive; there is no E3 verdict or stage-time attribution.],
) <fig-sensitivity>

Across the complete panel, the document and triple sweeps distinguish the routed
graph-bound q1 from the enumeration-heavy q8, whereas native q1 and q8 both respond to
document growth. Native backend resource-operation counts do not respond to triple or
visibility changes, even where time and allocation medians do. ACL placement/depth has
no preregistered ordering, and visibility changes the authorized set and q1 result
cardinality, so the visibility curves are not fixed-answer speedup estimates.

=== Paired guarded-stack/content-reference contrast (RQ4 / E4)

The physical reference contains independently selected readable content graphs only,
whereas guarded evaluation may also process authorized structural and control graphs.
The estimand is therefore a query-answer-equivalent guarded-stack/content-reference
contrast, not pure WAC cost or an otherwise-identical same-dataset counterfactual.

The primary contrast comprises
#headline_timing("ac_sparql.campaign.primary_h4_cells", digits: 0)
complete lane/domain/query/Pod cells. Across the complete timing campaigns,
#headline_timing("ac_sparql.campaign.all_h4_cells", digits: 0)
paired cells were retained and
#headline_timing(
  "ac_sparql.campaign.h4_cells_with_complete_clustered_intervals",
  digits: 0,
)
have complete clustered intervals. These are completeness statements, not equivalence or
effect verdicts.

#timing_figure(
  "ac_sparql.figure.guarded_stack_content_reference",
  caption: [All primary guarded-stack/content-reference wall and CPU ratios and
  differences, with marginal block-cluster intervals, for every lane, domain, query, and
  Pod count. E4 is descriptive and has no cross-query or binary verdict.],
) <fig-overhead>

Read separately within each lane, the routed primary contrast curves remain narrowly
grouped as resident Pod count changes, whereas the native ratios and differences rise
with Pod count across the displayed domains and queries. Ratio magnitudes are amplified
by very short content-reference queries and should be read alongside the paired
differences. These are within-lane descriptive patterns, not the causal effect of
switching implementations.

For the routed composition the contrast includes routing, authorization lookup, query
normalization, view membership, API wrapping, and work over any admitted structural or
control graphs. For the native handler it additionally includes server-side
authentication, in-process routing, server-wide discovery, WAC planning, source parsing,
temporary-dataset construction, evaluation, serialization, and body buffering. Absolute
cross-lane latency is not interpreted as a controlled speedup.

=== Cold construction and resident capacity (RQ5 / E5)

The construction matrix contains
#headline_timing("ac_sparql.campaign.construction_cells", digits: 0)
complete lane-specific cells. @fig-capacity reports every primary Pod-count level
against both resident Pods and realized content triples, retaining both domains and both
implementation paths.

#timing_figure(
  "ac_sparql.figure.construction_resident",
  caption: [Construction phases and current and peak whole-process resident memory over
  every primary Pod-count level and realized content-triple count, separated by path and
  domain. RSS includes the documented harness/oracle scope and is not isolated service
  heap.],
) <fig-capacity>

Within each lane and domain, every plotted primary construction-phase median and both
whole-process RSS measures increase with resident Pod count. Thus the bounded routed
warm-request result coexists with increasing construction and resident-state cost.

The materialized scenario tables keep corpus size, graph loading, WAC materialization,
route-index construction, and resident indicators separate:

#timing_table(
  "ac_sparql.scenario.social_small.materialized.content_documents",
  columns: (1.45fr, 0.82fr, 0.95fr, 0.82fr, 0.95fr, 0.65fr),
  header: (
    [Scenario], [Content docs], [Content triples], [Control docs], [Control triples],
    [Blocks],
  ),
  rows: (
    scenario_row([Social small (100 Pods)], "social_small", "materialized", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Social anchor (1,531 Pods)], "social_anchor", "materialized", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Compact health small (64 Pods)], "health_small", "materialized", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Health anchor (256 Pods)], "health_anchor", "materialized", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
  ),
  label_columns: 1,
  caption: [Materialized scenario corpus sizes and complete process blocks.],
) <tab-capacity>

#timing_table(
  "ac_sparql.scenario.social_small.materialized.graph_load_ns",
  columns: (1.45fr, 0.88fr, 0.88fr, 0.8fr, 0.95fr, 0.95fr),
  header: (
    [Scenario], [Graph load (ns)], [WAC build (ns)], [Route (ns)],
    [Current RSS (B)], [Peak RSS (B)],
  ),
  rows: (
    scenario_row([Social small (100 Pods)], "social_small", "materialized", (
      "graph_load_ns", "wac_materialization_ns", "route_index_ns", "resident_bytes",
      "peak_resident_bytes",
    )),
    scenario_row([Social anchor (1,531 Pods)], "social_anchor", "materialized", (
      "graph_load_ns", "wac_materialization_ns", "route_index_ns", "resident_bytes",
      "peak_resident_bytes",
    )),
    scenario_row([Compact health small (64 Pods)], "health_small", "materialized", (
      "graph_load_ns", "wac_materialization_ns", "route_index_ns", "resident_bytes",
      "peak_resident_bytes",
    )),
    scenario_row([Health anchor (256 Pods)], "health_anchor", "materialized", (
      "graph_load_ns", "wac_materialization_ns", "route_index_ns", "resident_bytes",
      "peak_resident_bytes",
    )),
  ),
  label_columns: 1,
  caption: [Materialized scenario construction and whole-process resident indicators.
  Values are lane-specific medians; completion of a named scenario is not a capacity
  maximum.],
)

The native tables report their corpus sizes and in-process service-seeding phase
separately rather than collapsing unlike construction paths:

#timing_table(
  "ac_sparql.scenario.social_small.http.content_documents",
  columns: (1.45fr, 0.82fr, 0.95fr, 0.82fr, 0.95fr, 0.65fr),
  header: (
    [Scenario], [Content docs], [Content triples], [Control docs], [Control triples],
    [Blocks],
  ),
  rows: (
    scenario_row([Social small (100 Pods)], "social_small", "http", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Social anchor (1,531 Pods)], "social_anchor", "http", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Compact health small (64 Pods)], "health_small", "http", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
    scenario_row([Health anchor (256 Pods)], "health_anchor", "http", (
      "content_documents", "content_triples", "control_documents", "control_triples",
      "complete_process_blocks",
    )),
  ),
  label_columns: 1,
  caption: [Native in-process scenario corpus sizes and complete process blocks.],
)

#timing_table(
  "ac_sparql.scenario.social_small.http.lws_seed_ns",
  columns: (1.55fr, 1fr, 1fr, 1fr),
  header: ([Scenario], [LWS seed (ns)], [Current RSS (B)], [Peak RSS (B)]),
  rows: (
    scenario_row([Social small (100 Pods)], "social_small", "http", (
      "lws_seed_ns", "resident_bytes", "peak_resident_bytes",
    )),
    scenario_row([Social anchor (1,531 Pods)], "social_anchor", "http", (
      "lws_seed_ns", "resident_bytes", "peak_resident_bytes",
    )),
    scenario_row([Compact health small (64 Pods)], "health_small", "http", (
      "lws_seed_ns", "resident_bytes", "peak_resident_bytes",
    )),
    scenario_row([Health anchor (256 Pods)], "health_anchor", "http", (
      "lws_seed_ns", "resident_bytes", "peak_resident_bytes",
    )),
  ),
  label_columns: 1,
  caption: [Native in-process scenario seeding and whole-process resident indicators.
  Shared-root vector de-duplication is an implementation/topology-specific construction
  cost; completion is not a maximum-user claim.],
)

#linebreak()
#block(above: 0.65em)[
  E5 has no binary or linearity verdict. Total state for non-empty resident Pods remains at
  least linear even if warm routed request work is insensitive to unrelated Pods; the native
  fixture also contains the source-derived quadratic shared-root insertion term. The
  measured total seeding curve does not establish quadratic growth or dominance of that
  term; no empirical construction exponent or stage dominance is claimed.
]

=== Confirmatory and exploratory summary

#figure(
  table(
    columns: (0.12fr, 0.25fr, 0.20fr, 0.43fr),
    align: (left, left, left, left),
    table.header[Item][Status][Evidence][Scope],
    [H1],
    [#timing_verdict(
      "ac_sparql.correctness.passed",
      yes: [passes],
      no: [does not pass],
    )],
    [@tab-population],
    [generated WAC subset and exact result bags],
    [H2a],
    [#timing_verdict(
      "ac_sparql.h2.materialized.rollup.all_cells_meet",
      yes: [all cells meet],
      no: [#timing_verdict(
        "ac_sparql.h2.materialized.rollup.no_cells_meet",
        yes: [no cells meet],
        no: [mixed across cells],
      )],
    )],
    [@tab-h2 and companion tables],
    [family/domain-specific four-part criterion],
    [H2b],
    [#timing_verdict(
      "ac_sparql.h2.http.rollup.all_cells_meet",
      yes: [all cells meet],
      no: [#timing_verdict(
        "ac_sparql.h2.http.rollup.no_cells_meet",
        yes: [no cells meet],
        no: [mixed across cells],
      )],
    )],
    [H2 tables and @fig-backend],
    [current native in-process endpoint only],
    [E3], [exploratory; no verdict], [@fig-backend and @fig-sensitivity],
    [complete aggregate work proxies and total costs; no stage-time attribution],
    [E4], [exploratory; no verdict], [@fig-overhead],
    [complete primary answer-equivalent stack/reference contrasts; no cross-query test],
    [E5], [exploratory; no verdict], [@fig-capacity and scenario tables],
    [whole-process capacity; concurrency one; native root-fan-out caveat],
  ),
  caption: [Only H1 and H2 use generated Boolean branches. E3--E5 are complete,
  descriptive disclosures and cannot receive supported/rejected verdicts.],
) <tab-hypotheses>

== Discussion and Design Implications <discussion>

=== The physical boundary is load-bearing

For the routed materialized path, the predeclared H2 criterion is
#timing_verdict(
  "ac_sparql.h2.materialized.rollup.all_cells_meet",
  yes: [met by every domain/query cell],
  no: [#timing_verdict(
    "ac_sparql.h2.materialized.rollup.no_cells_meet",
    yes: [met by no domain/query cell],
    no: [met by some but not all domain/query cells],
  )],
); for the native in-process endpoint it is
#timing_verdict(
  "ac_sparql.h2.http.rollup.all_cells_meet",
  yes: [met by every domain/query cell],
  no: [#timing_verdict(
    "ac_sparql.h2.http.rollup.no_cells_meet",
    yes: [met by no domain/query cell],
    no: [met by some but not all domain/query cells],
  )],
). The causal intervention is resident Pod count within each lane. The contrast between
the two lane-wide outcomes triangulates with the source audit, but it does not identify
the causal effect of routing or materialization alone: topology, authentication, API and
serialization boundaries, and admitted structural/control graphs also differ. The cost
model explains why a logical visibility predicate is not sufficient on its own. A query
engine may enumerate graph names before the predicate rejects them, and request-time WAC
planning may discover resources before it knows that they are unreadable. Routing first
changes the namespace over which those operations execute. A server seeking tenant-count
isolation should therefore resolve a trusted Pod or shard key before containment
discovery and before unbound named-graph enumeration.

This implication remains conditional. A shard containing many Pods would restore a
dependence on shard size for unbound graph enumeration. A route table with adversarial
hash collisions would not have the expected constant lookup assumed by Proposition 2.
Paging or last-level-cache pressure can also make wall time grow even when algorithmic
operations do not. The empirical result bounds these effects only over the tested host,
resident state, and $P$ range.

=== Precomputation exchanges read cost for state and update work

Materializing WAC moves policy parsing, inheritance reasoning, and authorization-index
construction from each read to startup or mutation. The appropriate choice therefore
depends on the ratio of reads to policy/data updates, the acceptable revocation latency,
and available memory. SPARQ's evaluated replacement path re-materializes the affected
PodStore; cache invalidation is narrowed by changed origins, but reasoning is not
incrementally updated in this experiment. The measurements report read-side and cold-
construction costs, but do not identify a write-heavy break-even point.

=== Correct answers do not eliminate timing disclosure

Corollary 1 gives the result-level property, and H1 tests it for the generated subset:
unreadable graphs do not influence returned solution bags. @fig-backend reports
whether native backend work varies with the hosted population. A deployment whose privacy
goal includes hiding co-tenancy must treat
request timing and resource usage as observable channels and consider routing,
padding/batching, rate control, and storage isolation. This paper measures one such
channel but neither quantifies an attacker's inference accuracy nor proves mitigation.

=== From single-request work to service capacity

At concurrency one, process CPU estimates the compute service demanded by one request
without queueing. It is useful for capacity planning but does not determine capacity.
Thread scheduling, snapshot-lock duration, memory bandwidth, object-store latency,
admission control, and query-size tails interact under load. In particular, the native
snapshot read guard permits overlapping readers but can delay a writer until every reader
has completed assembly, evaluation, and serialization. A separate open- or closed-loop
experiment is needed before making throughput, latency-SLO, fairness, or maximum-user
claims.

=== Implications for a Solid query surface

A future Solid query interface would need to specify at least its active-dataset mapping,
authorization point, treatment of graph names, handling of absent versus unauthorized
resources, update semantics, and interaction with WAC and ACP. This work supplies an
implementation and a measurable design point, not a standards proposal. Its strongest
transferable lesson is that authorization semantics and physical tenant boundaries must
be designed together: a semantically correct view can still have undesirable scaling and
side-channel behavior when it is constructed over a server-wide namespace.

== Related Work <related>

=== Solid and SPARQL querying

The original Solid platform work described Pods, application/data separation, optional
SPARQL support, and a Meccano server that rewrote LDP/SPARQL requests for ACL checks
@solid-www-demo @solid-platform-report. Dedecker et al. subsequently argued for a
graph-centric interpretation of a Pod and explicitly included SPARQL among possible views
@whats-in-a-pod. Consequently, neither a Solid/SPARQL combination nor WAC-aware querying
is novel here.

ConSolid implements a permissioned SPARQL satellite around Community Solid Server and
Fuseki: it queries ACL information and rewrites requests with `FROM`/`FROM NAMED` clauses
@consolid. Its reported performance table is an order-of-magnitude check for one
10,718,851-triple, 1,288-graph Pod and three principals, without a repetition or uncertainty
analysis. It does not identify unrelated-co-tenant growth. Staquet et al. propose
incrementally maintained SPARQL materialized views for Solid web agents and aggregators,
but present a methodology and future implementation/benchmark agenda rather than a measured
WAC co-tenant-isolation system @solid-ivm. These works sharpen our novelty claim to the
controlled unrelated-Pod intervention and comparison of physical enforcement boundaries.

More recent work also examines client-side or decentralized query processing. Taelman and
Verborgh adapt link-traversal query processing to Solid's
structure and evaluate it with SolidBench @solidbench; their EDBT demonstration exposes
an authenticated Comunica client that queries simulated and live Pods
@solid-ltqp-demo. POD-QUERY places a query agent in front of a Pod and rewrites queries
through schema mappings @pod-query. These systems address discovery, heterogeneous
schemas, or traversal across authoritative Web sources. We instead hold one server-local
target answer fixed and identify the compute effect of unrelated co-hosted Pods under two
authorization boundaries.

=== RDF and SPARQL access control

Kirrane, Mileo, and Decker survey policy representations, views, data filtering, query
rewriting, and partial-result enforcement for RDF @rdf-access-control-survey. Le et al.
answer SPARQL queries over virtual views by rewriting them against base RDF, avoiding
materialization and maintenance while showing that rewritten queries can grow
exponentially in the query and views @sparql-view-rewriting. This makes explicit that an
authorization view is not our novelty; it also identifies a different point in the
rewrite--materialize tradeoff. Stojanov and Jovanovik interpose an authorization proxy
that rewrites intercepted SPARQL queries according to RDF/SPARQL policies
@authorization-proxy. Kirrane et al. formalize query-based access control for Linked Data
@kirrane-query-ac, while SAFE applies graph-level policies to federated RDF data cubes
@safe. Our policy language is neither a replacement for these systems nor a new
fine-grained policy formalism. The distinction is the deployment and enforcement
question: WAC-governed resource graphs on a co-hosted Solid server, with unrelated
population varied independently of the authorized dataset and physical Pod-local routing
compared with request-time server-wide assembly.

=== Solid search and workload calibration

ESPRESSO develops access-control-aware keyword indexing and source selection over
decentralized personal datastores, including performance experiments across Pod size,
file count, visibility, Pods, and servers @espresso @espresso-framework. Its 2026 privacy
analysis formalizes metadata and inference risks for granular-visibility keyword indexes
@espresso-privacy. These are important adjacent results, but keyword index construction,
source selection, and cross-server search are different from server-local SPARQL
evaluation. We borrow stress dimensions, not algorithmic or privacy claims.

SolidBench derives a decentralized social corpus and link-traversal queries from LDBC SNB
@solidbench @ldbc-snb. TIDAL evaluates distributed personal-data querying for health
research @tidal, and Synthea produces synthetic longitudinal health records @synthea.
Their roles in this study are explicitly limited to count anchors or future richer
workloads. None provides an observational distribution for the content and policy of
future deployed Pods.

=== Complexity and experimental method

The language-level complexity results of Pérez et al. constrain general SPARQL evaluation
but do not predict implementation latency @perez-tods. Our contribution is to compose an
implementation-specific authorization model with a symbolic query term and then measure
controlled operator families. The experimental design follows warnings about layout and
environment bias @mytkowicz, hierarchical replication and uncertainty
@kalibera-jones, and transparent cloud-performance reporting @papadopoulos-cloud. The
prospective equivalence margins and fail-closed evidence factory are study-specific
adaptations rather than implementations of those authors' exact estimators.

== Threats to Validity and Limitations <threats>

*Construct validity.* The generator isolates factors that are usually confounded, but its
social and health graphs are deliberately regular. Same-count vocabularies do not capture
real correlations, skew, binary resources, malformed RDF, very large literals, complex
medical histories, or arbitrary user policy styles. Count-calibrated scenarios are not
SolidBench- or TIDAL-derived data. One RDF resource maps to one named graph, the default
graph is empty, and only `SELECT` queries are measured. Alternative dataset mappings can
change both semantics and cost.

The WAC subset includes public, private, named-recipient, direct, and inherited read
grants. It excludes groups, origins, clients, issuers, conditions, explicit deny models,
ACP, and ODRL. Authentication cost is partly inside the in-process handler lane, but authentication
correctness is not evaluated as a contribution. Exact result-bag equality is evidence for
the generated subset, not proof that the implementation is complete, secure, or
side-channel-free.

*Internal validity.* Adding Pods leaves earlier corpus bytes and the target answer fixed,
and configuration/pair order is randomized within independently rebuilt blocks. However,
the paths differ in more than authorization: one is a routed library call and the other an
in-process Axum router/handler request without transport. We consequently do not interpret cross-lane absolute
latency as a pure treatment effect. The native plain reference is an engine-only lower
boundary over answer-equivalent content graphs, not a counterfactual HTTP server with only
WAC removed; structural/control graph work also belongs to the contrast.

Timing and instrumentation use separate builds to prevent counters from perturbing the
primary outcome; this also means counter/timing covariance is not observed within the
same request. Whole-process RSS includes explicitly documented harness state. Warm-up
does not guarantee every hardware and software cache has reached stationarity. CPU
pinning prevents migration but not frequency changes, interrupts, shared-cloud effects,
or memory-hierarchy changes as resident state grows. A single disposable host controls
between-host variance at the expense of hardware diversity.

*Statistical conclusion validity.* Five rebuilt blocks provide the independent units for
H2; thirty requests within a block improve median precision but are not treated as thirty
independent deployments. Percentile bootstrap intervals inherit the observed block and
request distributions, and five upper-level units limit tail resolution. The 1.10 and
0.10 margins encode an engineering relevance choice, not a consensus Solid service-level
objective. Results are family-specific; multiple families are not pooled, and no
non-significant null result is labeled equivalent. p95 and p99 describe requests in the
campaign rather than confidence bounds on future deployment tails. Sensitivity trends
without preregistered thresholds are exploratory.

*External validity.* The study evaluates one Rust implementation, one cloud instance
type and architecture, one OS/toolchain, $P<=2,048$, one requested Pod, and concurrency
one. In-memory storage removes remote latency and failure behavior; object storage,
databases, distributed caches, TLS termination, wide-area networks, multi-origin
federation, and client-side link traversal can dominate a production request. The study
does not measure writes, mixed read/write workloads, cold session caches, bursty
arrivals, saturation, tenant fairness, or horizontal scaling. A completed 1,531- or
2,048-Pod cell means only that the named synthetic configuration fit the measured host;
it is not a maximum supported user count.

*Protocol and researcher degrees of freedom.* The protocol has reached version 1.33
through an append-only amendment history reflecting issues found during implementation
and pilots. This history is longer than ideal, but it is visible and
outcome-inspection status is recorded for every change. The valid canonical analysis
used the analyzer frozen under v1.31; canonical publication requires the post-outcome,
publisher-only v1.32 and v1.33 corrections described above. The credential-expiry attempt and
setup-transfer failure cannot enter estimates. All mandatory cells, guard-based skips,
and unfavorable outcomes remain reportable, and the campaign does not stop because an
effect is favorable or unfavorable.

== Artifact, Reproducibility, and Resource Ethics <artifact>

The artifact consists of the paper-specific generator in `sparq-acbench`, cross-layer
oracle tests, the `ac_query_scale` runner, deterministic schedule generators, resource
guard, EC2 launcher/supervisor, raw-schema validator, analysis program, protocol, and
evidence ledger. The archival provenance records the exact source commit and review
baseline; the release metadata records the artifact-identifier status (unassigned for
this release) and the MIT license.

For every cell, raw JSON Lines and stderr are retained with a SHA-256 sidecar. Records
contain corpus and query hashes, source and environment identity, schedule position,
configured warm-ups/repetitions, process block, and correctness. The analysis produces
`summary.csv`, `paired-overhead.csv`, `construction.csv`, `h2.json`, generated SVGs, and
`manifest.json`; the manifest hashes all raw inputs, the analysis script, and its outputs.
Paper headline values are not copied into prose: each is bound through a JSON pointer in
the repository evidence file and is accepted only when marked canonical.

#block(inset: 8pt, stroke: 0.5pt + gray)[
  #text(size: 0.78em)[#timing_provenance("ac_sparql.artifact.study_cost_usd")]
]

Reproduction proceeds in four stages: run unit and cross-layer correctness tests; execute
the full correctness matrix; execute timing and instrumentation campaigns on a pinned
single-CPU Linux host; and invoke the analyzer with `--require-canonical`, the exact
40-hex commit, fixed bootstrap seed, and at least 10,000 draws. The reproduction checklist
lists commands and schema checks. The artifact also retains the append-only amendment log
and failed-attempt evidence, allowing reviewers to distinguish design corrections from
outcome-contingent changes.

Only synthetic data and identities are used; no personal or clinical record is ingested.
The host launcher refuses a dirty tree or an ambiguous price, records an official AWS
price source, enforces a study-wide USD 100 ceiling, creates only tagged disposable
resources, restricts SSH to one exact address, uses delete-on-termination storage, and
terminates by both local watchdog and independent supervisor. The final study-wide
accounted direct-resource estimate is #headline_timing(
  "ac_sparql.artifact.study_cost_usd",
  digits: 2,
  suffix: [ USD],
); the estimated canonical-run component is #headline_timing(
  "ac_sparql.artifact.canonical_run_cost_usd",
  digits: 2,
  suffix: [ USD],
), and canonical host time is #headline_timing(
  "ac_sparql.artifact.canonical_host_duration_seconds",
  digits: 1,
  suffix: [ s],
). These controls bound both financial and
infrastructure impact; they do not erase the environmental cost of computation, which is
reported alongside the artifact.

== Conclusion <conclusion>

Access-controlled SPARQL over a multi-Pod server has two separable scaling questions:
how much state the service holds and how much unrelated state a single query must touch.
By expressing WAC authorization as an active-dataset view, the theoretical analysis shows
that a Pod-routed materialized path can remove unrelated Pods from warm query operations,
whereas the pinned native endpoint's root-first assembly must visit resources contributed
by every non-empty co-hosted Pod. A correctness-gated, fixed-working-set intervention
then tests whether those algorithmic differences survive the memory hierarchy and the
measured in-process request stack.

The correctness matrix
#timing_verdict(
  "ac_sparql.correctness.passed",
  yes: [passed its predeclared oracle checks],
  no: [did not pass its predeclared oracle checks],
). Across the complete H2 family, the routed materialized criterion was
#timing_verdict(
  "ac_sparql.h2.materialized.rollup.all_cells_meet",
  yes: [met in every cell],
  no: [#timing_verdict(
    "ac_sparql.h2.materialized.rollup.no_cells_meet",
    yes: [met in no cell],
    no: [met in some but not all cells],
  )],
), while the native endpoint criterion was
#timing_verdict(
  "ac_sparql.h2.http.rollup.all_cells_meet",
  yes: [met in every cell],
  no: [#timing_verdict(
    "ac_sparql.h2.http.rollup.no_cells_meet",
    yes: [met in no cell],
    no: [met in some but not all cells],
  )],
). Exploratory instrumentation showed Pod-count-invariant routed allocation-count and
allocated-byte medians within every primary domain/query series, while native backend-
operation and allocation medians increased with Pod count. Construction phases and
whole-process resident state also increased with hosted state. E4 reports the complete
guarded-stack/content-reference contrast but neither isolates WAC nor compares the lanes
causally; E3--E5 receive no verdict. These claims remain bounded to the measured
implementation, workload, host, Pod range, and concurrency. No result makes startup,
resident memory, updates, remote I/O, or concurrent capacity free, and no synthetic count
anchor predicts a future Solid population. The defensible design lesson is narrower and
more useful: establish a tenant-local physical boundary before authorization-driven graph
discovery and query evaluation if unrelated hosted state should not determine warm
request work.

== Appendix A: Query Templates <app-queries>

The following schematic templates preserve the operator structure of the generated
queries. Domain-specific class, principal, date, and alternating predicates are filled
deterministically, and `<g0>` and `<item0>` identify the first graph/item in Pod 0.

```sparql
# q1: graph-bound point lookup
SELECT ?class WHERE { GRAPH <g0> { <item0> a ?class } }

# q2: selective star (domain-specific date filter omitted here)
SELECT ?g ?item ?date WHERE {
  GRAPH ?g { ?item a <Class> ; <principal> <owner> ; <date> ?date . FILTER(...) }
}

# q3: multiway join
SELECT ?g ?item WHERE {
  GRAPH ?g { ?item a <Class> ; <schema:isPartOf> ?g . ?g <schema:about> ?item }
}

# q4: OPTIONAL
SELECT ?g ?item ?left ?right WHERE {
  GRAPH ?g {
    ?item a <Class> .
    OPTIONAL { ?item <alternating-left> ?left }
    OPTIONAL { ?item <alternating-right> ?right }
  }
}

# q5: negation
SELECT ?g ?item WHERE {
  GRAPH ?g { ?item a <Class> . FILTER NOT EXISTS { ?item <alternating-left> ?v } }
}

# q6: bounded property path
SELECT ?g ?item WHERE {
  GRAPH ?g { ?item a <Class> ; <schema:isPartOf>/<schema:about> ?item }
}

# q7: aggregate/grouping
SELECT ?principal (COUNT(?item) AS ?count) WHERE {
  GRAPH ?g { ?item a <Class> ; <principal> ?principal }
} GROUP BY ?principal

# q8: unbound graph scan
SELECT ?g ?item WHERE { GRAPH ?g { ?item a <Class> } }
```

The artifact records the exact query text and SHA-256 for every applicable observation;
the schematic appendix is explanatory and is not the executable source of truth.

== Appendix B: Full H2 Reporting Contract <app-h2>

The machine-readable `h2.json` is the authoritative full-precision table. For each of
two paths, two domains, and eight queries it records the tested Pod vector, complete
process blocks, seeds, request count, wall endpoint ratio and interval, wall elasticity
and interval, process-CPU endpoint ratio and interval, CPU elasticity and interval,
predefined margins, four component verdicts, joint verdict, bootstrap draws, and seed.
The publication table may round values for legibility but must preserve the machine record
and must not replace a family-level failure with an average pass.

The compact cells in @tab-h2 are generated from these atomic values:

```text
wall ratio [95% CI]; wall beta [95% CI]
CPU ratio [95% CI]; CPU beta [95% CI]; joint verdict
```

All thirty-two predeclared entries are shown in the H2 tables above. Full-precision
values and their analysis metadata are retained in `h2.json` and the canonical evidence
envelope.

== Appendix C: Reproduction and Provenance Checklist <app-reproduce>

The archival README supplies copyable commands; this checklist states the required
evidence boundary independently of shell syntax.

1. Check out the exact source commit and verify a clean tree and the frozen review base.
2. Run generator invariants, `sparq-solid` deployment/oracle tests, and the live native
   endpoint revocation/query tests.
3. Run the full correctness matrix before any timing campaign.
4. Record official instance pricing, accumulated study cost, identity and region; verify
   no tagged orphan; create the unique disposable host.
5. Verify Linux preflight, source object, release features, one Rayon worker, and one-CPU
   effective affinity.
6. Execute timing and instrumentation profiles separately for Pod scaling, sensitivities,
   and calibrated scenarios. Preserve `.partial` files, stderr, and failure sentinels.
7. Recompute every JSONL SHA-256 before resuming or analyzing a cell.
8. Run canonical analysis with the exact commit, fixed bootstrap seed, and at least
   10,000 draws; require five complete blocks for every mandatory cell.
9. Hash the raw inputs, analyzer, CSV/JSON outputs, and SVGs in the derived manifest.
10. Bind every paper result through a canonical evidence record and JSON pointer; run the
    paper factory's performance-number, privacy, and evidence-binding checks.
11. Publish the protocol, amendments, raw/derived manifests, skipped-cell ledger, final
    cost, and either a persistent artifact identifier or an explicit unassigned-identifier
    status with the manuscript.

#pagebreak()
== Appendix D: Protocol Deviations and Invalid Attempts <app-deviations>

The protocol's amendment log is normative and must be archived verbatim. The main paper
summarizes rather than conceals its two canonical-era interruptions:

- *Credential expiry (amendment 1.27).* The first canonical attempt completed correctness
  and then encountered HTTP 401 after the study token outlived its original validity. The
  partial timing cell and every timing in that attempt are invalid. The token validity was
  changed to the same twelve-hour bound as the disposable-host watchdog; production
  verifier freshness and per-request DPoP proof creation remain unchanged.
- *Setup transport (amendment 1.28).* A later launch experienced rapid egress-IP changes
  during source upload. The service never started and no benchmark ran. Setup transport
  now retries a bounded number of times while atomically rotating the same single exact
  `/32` ingress rule; a reached-host command failure remains fail-closed.

Earlier amendments document corrections to the fixed authorized slice, randomization,
independent block seeds, memory/allocation boundaries, applicability, store limits,
high-resolution CPU measurement, profile separation, CPU affinity, lexical hierarchy,
query non-vacuity, calibration language, resume/checksum validation, live revocation,
cloud pricing, systemd limits, and SSH supervision. For each, the log records whether
only source/correctness/capacity evidence or any timing outcome had been inspected.

Amendments 1.32 and 1.33 are the only post-outcome publication corrections. The first
fixes an emitted-applicability cardinality check that had used the selected-query count.
The second follows a complete derived-schema audit: it admits and validates the
pre-existing paired `inference_status`, treats the authorization-triple construction
counter according to lane, makes the H2 schema exact, and rejects malformed CSV widths.
Both are explicitly non-analytical: the raw inputs, analyzer outputs, H1/H2 decisions,
exploratory panels, and claims are unchanged.

== Appendix E: Implementation Source Audit <app-source-audit>

The theoretical bounds were derived from the frozen implementation rather than from an
idealized architecture. The artifact's cost-model ledger records commit-qualified line
ranges; the stable file/function anchors are summarized here so a reviewer can reproduce
the derivation after checking out the recorded source commit.

- *Routed composition:* `crates/sparq-lws-core/examples/ac_query_scale.rs` constructs the
  per-Pod store vector and route and defines the timed guarded/plain closures.
  `crates/sparq-solid/src/materialize.rs` and `loader.rs` implement rule compilation,
  loader fact assembly, fixpoint invocation, and view installation;
  `crates/sparq-reason/src/n3/compiled.rs` implements indexed semi-naive rule evaluation.
- *Authorization index and cache:* `crates/sparq-solid/src/authindex.rs`, `lib.rs`, and
  `session_cache.rs` contain index reconstruction, session-set derivation, shard/cap
  constants, invalidation, and the warm hit path. `rewrite.rs` plus
  `crates/sparq-engine/src/lib.rs` expose query normalization and reparsing.
- *Graph visibility:* `crates/sparq-engine/src/exec.rs` contains visibility membership,
  concrete named-graph lookup, unrestricted variable-graph enumeration, and the recognized
  graph-name prefix range index. These sites distinguish constant expected membership from
  conditional linear store-local graph scans.
- *Native in-process handler path:* `crates/sparq-lws-core/src/app.rs`, `auth.rs`, and
  `sparql_endpoint.rs` cover authentication, protocol parsing, snapshot scope, root
  traversal, candidate formation, admitted-source parsing, N-Quads rebuilding, dataset
  handling, evaluation, and serialization.
- *WAC and backend terms:* `crates/sparq-lws-core/src/authz/wac.rs` and `acl.rs` contain
  ACL-chain planning, live cache confirmation, cache-hit triple-vector cloning, and repeated
  matcher scans. `store/mod.rs` and `store/sparq.rs` define the backend-plan contract and
  benchmark in-memory implementation; the latter also contains the shared-root child-vector
  scan during seeding. `crates/sparq-core/src/lib.rs` contains linear-search `ensure_named`.
- *State and contention:* `crates/sparq-lws-core/src/ldp/handler.rs`, `acl_cache.rs`, and
  `store/body_cache.rs` provide the snapshot read/write barrier and entry- versus
  byte-budgeted caches. The benchmark example's support module configures authentication
  caching and corpus-derived backend limits.
- *Generator assumptions:* `crates/sparq-acbench/src/deployment.rs` defines bounded
  hierarchy and audience/control shapes, separates content/container/control graphs, and
  emits the eight query templates used by Lemma 1.

This map is explanatory. The archived, checksummed source tree and commit-qualified
ledger are authoritative if a path or line number later moves.

#heading(level: 2, numbering: none)[References]
#bibliography("access-controlled-sparql-pod-scale.refs.yml", style: "ieee", title: none)
