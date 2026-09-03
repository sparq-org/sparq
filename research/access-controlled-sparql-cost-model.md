# Cost model for access-controlled SPARQL over Solid Pods

Status: paper derivation record, 2026-09-03. This model describes the code at baseline
`8ad990e679cf11018d5fc6a3860741db9e897034` plus the paper artefact on
`codex/ac-sparql-paper`. It separates the routed materialized-view design from the
current native LWS endpoint; neither path's cost may be attributed to the other.

## 1. Semantics and notation

Let an RDF dataset be a finite mapping `D : G -> RDFGraph` from graph names to RDF
graphs. Pod data is stored only in named graphs; the standing default graph is empty.
For request context `s` and access mode `m`, let the policy decision function define

`A(s,m) = { g in G | allow(s,m,g) }`.

The authorization view is the dataset

`V(D,A)(g) = D(g)` when `g in A`, and absent otherwise.

An access-controlled read query has the denotation

`ACEval(q,D,s) = Eval(q,V(D,A(s,Read)))`.

The experiment uses the following size variables.

| Symbol | Meaning |
|---|---|
| `P` | resident Pods |
| `R_p` | enumerable resources and containers in Pod `p` |
| `G_p` | content named graphs in Pod `p` |
| `M_p` | content triples in Pod `p` |
| `F_p` | policy and structural facts used for WAC in Pod `p` |
| `H_p` | triples in the materialized authorization view for Pod `p` |
| `h(r)` | number of candidate ancestor ACLs for resource `r` |
| `A_s` | readable content graphs in the routed Pod for session `s` |
| `M_s` | content triples in those readable graphs |
| `I(q,D)` | query-evaluation intermediate cardinality/work |
| `Z(q,D)` | serialized result size |

The symbols are deliberately separate. Pod count is not a proxy for document count,
triples per document, hierarchy depth, policy density, visibility, or result size.

### Proposition 1: physical-filter oracle

If the generator's independent audience predicate produces exactly `A(s,Read)`, then
evaluating `q` over the physically filtered reference dataset and evaluating it through
the authorization view return the same SPARQL solution multiset.

*Proof.* Both evaluations receive datasets with the same default graph and the same
mapping for every named graph. SPARQL evaluation is a function of the query and active
dataset, so the multisets are equal. The benchmark tests this premise rather than
assuming it: policy decisions are compared per graph, and every timed answer is compared
as a duplicate-preserving multiset. QED.

### Proposition 2: result-level absence indistinguishability

For a fixed policy decision set `A`, changing or adding only graphs outside `A` cannot
change `ACEval(q,D,s)`.

*Proof.* `V(D,A)` discards every changed graph, leaving the active dataset identical;
apply Proposition 1. QED.

This is a result-level statement, not a complete non-interference theorem. A server-wide
enumeration can still reveal deployment size through latency, allocation, or backend
work even when the answer does not change. The native-endpoint experiment measures that
distinction directly.

## 2. Routed materialized authorization view

The evaluated design constructs one `sparq_solid::PodStore` per Pod and stores a hash
route from Pod root to vector index. Each store contains only that Pod's data and WAC
documents. WAC rules are compiled once per process; each store materializes its own auth
view. A request routes first, obtains a cached authorized graph-name set, installs it as
an engine `DatasetView`, and evaluates in place.

### 2.1 Construction and storage

For Pod `p`, construction has the decomposition

`C_p = ParseIndex(M_p + F_p) + MatWAC(F_p, R_p, h, H_p) + IndexAuth(H_p)`.

Consequently,

`C_materialized = sum_p C_p + Theta(P)`

and resident space is

`S_materialized = sum_p [Store(M_p + F_p) + Auth(H_p)] + Theta(P) + S_cache`.

`S_cache` is bounded by implementation constants per store (16 lock shards and at most
1,024 entries per shard), although the empty per-Pod cache still contributes fixed
`Theta(P)` metadata. The benchmark reports construction time and resident memory rather
than substituting an assumed bytes-per-triple constant.

The compiled N3 engine uses semi-naive forward chaining and hash joins. A general rule
program can generate large joins, so the paper does not claim that `MatWAC` is universally
linear. In the controlled corpus, depth is bounded, parenthood and effective-ACL
relationships are functional, and grants per audience are bounded. Its structural,
inheritance, and authorization closure sizes are therefore `O(R_p h_max + F_p)` and
`O(R_p)` when `h_max` is fixed; observed closure size and construction work remain the
reported evidence.

An ACL replacement currently re-runs materialization over the entire affected
`PodStore`. Diff-based origin comparison narrows *session-cache invalidation*, not the
reasoning re-run. With one Pod per shard, that write-side cost depends on the affected
Pod rather than all resident Pods.

### 2.2 Request cost

On a cold session-set lookup, the auth index gathers grant vectors for the bounded set of
session principals, applies conditional grants, removes denies, and sorts the admitted
names. Write `L_s` for grant/deny entries visited and `C_s` for conditional rules checked.
Expected cold cost is

`T_set,cold = O(L_s + C_s + |A_s| log |A_s|)`.

On a warm hit, the sharded cache takes one read lock and clones two `Arc`s; its cost is
expected `O(1)` independent of `P`. Every named-graph visibility check by the query engine
is an expected `O(1)` hash-set membership test.

For a warm request routed to Pod `p`,

`T_routed = T_hash_route(P) + T_query_parse(|q|) + O(1)_cache + Q(q,V_p) + O(Z)`.

Rust's randomized hash map gives expected `T_hash_route(P)=O(1)`; this is not a
worst-case collision guarantee. `Q` includes the actual SPARQL implementation work and
cannot in general be reduced to `M_s`: joins, OPTIONAL, negation, paths, grouping,
intermediate cardinality, and output all matter.

The engine's `GRAPH ?g` implementation enumerates every named graph in the *selected
store* and checks the view for each. Therefore an unbound graph scan costs at least
`Omega(|G_p| + Z)`. The view by itself does **not** make an unsharded multi-Pod store
independent of tenant count. The evaluated independence claim follows from routing to a
Pod-sized store before evaluation.

### Proposition 3: conditional independence from unrelated Pods

Hold the target Pod, session, query, result, and per-Pod route key fixed. Under expected
constant-time hash routing, a warm routed request's operation count is independent of the
number of unrelated resident Pods.

*Proof.* The only request-path structure containing all Pods is the routing hash map. Its
expected lookup cost is constant. All subsequent cache, graph enumeration, and query work
touch only the target `PodStore`, whose input is held fixed. QED.

This proposition is about algorithmic work, not a promise of identical wall time.
Larger resident state can change cache residency, allocator behaviour, NUMA placement,
or trigger paging. The primary experiment therefore jointly requires a small latency
ratio and a small log-log elasticity confidence bound.

## 3. Current native LWS HTTP endpoint

The native endpoint authenticates the request, parses the query, takes a server snapshot
read lock, and calls `assemble_authorized_dataset`. That function starts at the server
base URL—not the requested Pod—recursively enumerates containment into a `BTreeSet`, adds
an ACL candidate for every non-ACL resource, plans and authorizes every candidate, reads
and parses admitted RDF, writes admitted triples into an N-Quads string, then parses that
string into a temporary indexed `Graph`. Query evaluation and result serialization
follow while the snapshot guard remains held.

Let `N = sum_p R_p` be the server-wide enumerable resource count, and let `K <= 2N` be
the candidate count after derived ACL names are added. Its request time decomposes as

```
T_http = T_authn + T_query_parse
       + O(N log N)                         containment/candidate BTreeSets
       + sum_{r in K} [Plan(h(r)) + WAC(r)]
       + ParseSource(M_s) + WriteNQ(M_s)
       + ParseIndexTemporary(M_s)
       + Q(q,D_s) + O(Z).
```

The `O(N log N)` term is a safe comparison-based upper bound for the set insertions, not
a fitted latency model. Backend operation count is reported separately because storage
implementations can give `Plan` very different wall costs. Source RDF is parsed before
N-Quads emission and the aggregate N-Quads is then parsed again, so admitted triples
cross two parsing boundaries. Peak per-request auxiliary space includes `O(N)` resource
and candidate sets plus `O(M_s)` serialized and indexed authorized data and `O(Z)` output.

### Proposition 4: server-wide enumeration lower bound

Suppose each added Pod contributes at least one enumerable resource, while the target
answer remains fixed and added Pods are unreadable. The current native endpoint performs
`Omega(P)` candidate visits for every request.

*Proof.* Assembly begins at the common server root and visits every descendant returned
by authoritative containment. Each added non-empty Pod contributes a distinct resource
inserted into `contained`, then at least one iteration in the candidate loop, irrespective
of its eventual WAC decision. Thus candidate visits grow at least linearly with `P`. QED.

This is an implementation characterization, not a lower bound for access-controlled
SPARQL. Routing before enumeration or maintaining an authorization view removes the
server-wide term.

The snapshot is an `RwLock` read guard. Read requests may overlap, but each holds the
guard through assembly, evaluation, and serialization; a mutation requiring the write
guard must wait for all such readers. The native benchmark's operation-scoped maximum
in-flight counter identifies actual backend overlap rather than inferring it from async
syntax.

## 4. SPARQL evaluation and access-control composition

For general SPARQL, combined-complexity classifications delimit what no implementation
can make uniformly cheap; they are not request-time formulae. The paper reports the
known complexity of the language fragments from the primary literature, then keeps the
implementation term symbolic as `Q(q,D)`. For every materialized result, any server has
the elementary lower bound `Omega(|q| + Z)` if it parses the query text and emits the
complete output. In this implementation, an unbound graph-name scan additionally has
the store-local lower bound in section 2.2.

Access control composes at the dataset boundary rather than by post-filtering answers:

`T_AC(q,s) = T_authorize(s) + Q(q,V(D,A_s)) + T_serialize`.

The paired experiment evaluates the same query over the exact same physically readable
graphs, so its difference estimates authorization/routing/view overhead rather than a
change in input cardinality. For the HTTP lane the guarded side intentionally includes
authentication, server-wide discovery, per-resource WAC, reconstruction, and HTTP
serialization; its plain reference is an engine-only lower boundary, not a claim that
all of the difference is WAC rule matching.

## 5. Capacity implications

For `c` available worker cores and arrival rate `lambda`, the utilization condition
`lambda * E[CPU service time] < c` is necessary for stability but is not sufficient when
shared locks, memory bandwidth, storage, or tail-heavy queries dominate. Little's law
(`L=lambda W`) relates mean concurrency, throughput, and response time only for a stable
measured interval. The present experiment deliberately fixes concurrency and Rayon
parallelism to one to estimate per-request work. It therefore does not estimate a
saturation point or claim that single-request scaling implies concurrent throughput;
those require a separately controlled queueing experiment.

Even when Proposition 3 holds, total construction and resident storage remain at least
`Omega(P)` for non-empty resident Pods. The paper's claim is therefore narrowly about
*warm request compute as unrelated Pods are added*, not free storage, free startup, or an
unbounded capacity result.

## 6. Refutable predictions

1. With Pod 0 fixed and all background Pods private, materialized routed backend/query
   work should be flat in `P`; wall time may depart from flatness only through system
   effects such as cache pressure.
2. Native HTTP backend operations should increase with total enumerable resources even
   for `GRAPH <target>` and an unchanged result.
3. Materialized construction time, auth-view triples, and resident memory should grow
   with total stored Pods.
4. At fixed `P`, increasing ACL coverage raises `F` and materialization work without
   changing the oracle-visible document set.
5. At fixed deployment size, increasing visibility changes `M_s`, query work, and output
   but not native endpoint candidate enumeration.

Each prediction maps to a controlled factor in
`bench/ac/scaling/ANALYSIS-PROTOCOL.md`; failed predictions remain reportable results.
