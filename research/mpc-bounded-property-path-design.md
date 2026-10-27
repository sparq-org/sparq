<!-- [OPUS-4.8] Bounded-length property-path operator design (unroll to a fixed BGP chain), Opus 4.8 (Fable unavailable) — design-for-review; re-review when Fable returns. -->

# MPC Bounded-Length Property-Path Operator — Design Record (sq-py8h)

**Status:** Deep-research design record (no implementation; doc-only). Author: Opus 4.8
(Fable unavailable — flag for re-review). Date: 2026-06-15.
**Bead:** `sq-py8h` (child of the MPC research epic `sq-0jsc`). **Unblocked by** `sq-dvuc`
(BGW degree-reduction), now **MERGED** (PR #119, `degree_reduce` at
[`shamir.rs:406`](../crates/sparq-mpc/src/shamir.rs)).

**What this is.** The capability matrix
([`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) §4.3) names *property
paths over a secret graph* as the **OUT-OF-REACH** SPARQL operator for MPC — but flags the one
exception: *"Scope to BOUNDED length only (unroll to a fixed hop count), where it reduces to a
fixed BGP chain (P4+P6)."* That single line is the entire achievable slice; this record
resolves it to a concrete construction, a precise leakage statement, a security tier, a
qualitative cost, and a sequenced implementation plan. It is the design-for-review artifact for
the **only tractable family of property paths under MPC over a secret graph**.

**Relationship to the other records (extend, do NOT duplicate).**
- [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) — adopts its
  capability-tier vocabulary (BUILT / KNOWN / OPEN / IMPOSSIBLE), its primitive-class ladder
  (P0–P7), and its two-regime split (DISCLOSED / HIDDEN). This record **resolves its single
  property-path cell** (matrix §2 row "Property paths", §4.3) into a full operator design. The P4
  keystone this operator depends on — `degree_reduce` (BGW reshare-and-recombine, PR #119,
  `sq-dvuc`) — is **BUILT on `main`** ([`degree_reduce`](../crates/sparq-mpc/src/shamir.rs)); the
  matrix §1.2 reflects that (its earlier "in-flight in PR #119, not yet merged" draft note was
  reconciled in `sq-k4of`). That is what makes this bead actionable now rather than blocked.
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) — the
  3-axis security taxonomy, the seven-channel leakage taxonomy, the honest-envelope framing. §4
  here uses its vocabulary verbatim.
- `sparql-formal-semantics` skill / W3C SPARQL 1.1 §9 — the property-path algebra
  (`ArbitraryLengthPath`, `ZeroOrMorePath`, `OneOrMorePath`, `ZeroOrOnePath`, `seq`, `alt`,
  `NegatedPropertySet`) this record scopes against.

**Honesty contract (per the empirical-honesty rule).** No fabricated performance numbers.
Cost is qualitative (multiples of the join cost, classes of round complexity). Every claimed
reuse cites a file:line on `main`. The leakage of a bound `k` is stated precisely and is NOT
minimised: a bound IS a leak, intrinsic to the construction, and is called out as such.

---

## 1. Scope — what is achievable vs what is not (precise + honest)

### 1.1 The fundamental obstruction (why general property paths are OUT-OF-REACH)

A SPARQL property path `?x (p)* ?y` over a graph `G` evaluates the **transitive closure** of
the binary relation `p` — the set of pairs `(a, b)` connected by *some* finite chain
`a p z₁ p z₂ … p b` of *unknown, data-dependent* length. The clear-text engine computes this
by a **fixpoint iteration** (a `eval_path`-style breadth-first reachability loop) that runs
until no new pairs are added. Over a SECRET graph this is fatal for MPC on three independent
grounds, each sufficient on its own:

1. **The number of iterations is data-dependent and therefore leaks the diameter.** A fixpoint
   loop runs `d+1` times where `d` is the longest shortest-path in `G` restricted to `p`. The
   round count of the MPC protocol would be a public function of a private graph property — the
   protocol's *shape* would reveal `d`. To hide `d` you must pad to the worst case `d = |V|−1`
   (the whole vertex set as a chain), which is both unbounded a priori and catastrophic.
2. **Each fixpoint step reveals reachability growth.** Even an oblivious one-step expansion that
   never opens an intermediate set still has to decide *when to stop*; a secret stopping
   condition (a shared "did the frontier grow?" bit) is itself a comparison/equality chain whose
   repeated evaluation, opened to drive the loop, leaks the reachability-set size trajectory —
   a strong fingerprint of the graph structure (matrix §4.3 row "Property paths": *"each fixpoint
   iteration reveals reachability growth"*).
3. **Closure is super-linear and unbounded-depth.** The output relation can be `O(|V|²)` pairs;
   under full obliviousness the intermediate sets must be padded to that worst case at every
   step. No published protocol gives correct, attested transitive closure of a *secret* edge set
   at scale. GORAM (PVLDB'25) does ego-centric *traversal* over a federated graph but is
   **confidentiality-only** — not correctness, not attestation — and is not general path
   evaluation (matrix §6.2 / §4.3).

**Verdict for unbounded paths.** `(p)+`, `(p)*`, and any arbitrary-length / unknown-depth path
over a secret graph are **OPEN / effectively IMPOSSIBLE at scale** under MPC with correctness +
attestation. This is not a sparq gap to be closed by a bead; it is an obstruction inherent to
secret-graph closure. The registry's fail-closed discipline applies: a request for a hidden
unbounded path must be **truthfully refused**, never silently approximated.

### 1.2 The tractable slice — bounded fixed-`k` paths

The obstruction is entirely about *unbounded depth*. The moment a maximum hop count `k` is
fixed and public, the fixpoint disappears: the path becomes a **finite, statically-known
disjunction of fixed-length conjunctive chains**, each of which is exactly a BGP — the operator
class the crate already evaluates under MPC. The following are **TRACTABLE** (KNOWN construction;
realizable on the now-merged degree-reduction backend):

| Construct | Semantics | Unrolls to | Tractable? |
|---|---|---|---|
| **`a (p₁/p₂/…/p_k) b`** (fixed sequence) | exactly-`k`-hop chain through named predicates | one fixed `k`-pattern BGP chain | **YES** — a fixed BGP join (§2.1) |
| **`a (p){k} b`** (exact repetition) | exactly `k` hops of `p` | one fixed `k`-pattern BGP chain with `k−1` fresh intermediate vars | **YES** |
| **`a (p){1,k} b`** (bounded `+`) | between 1 and `k` hops of `p` | UNION of `k` fixed chains, lengths `1..k` | **YES** — `k` BGP chains, deduped (§2.2) |
| **`a (p){0,k} b`** (bounded `*`, reflexive) | between 0 and `k` hops | the `{1,k}` union PLUS the reflexive `a = b` pair (length-0) | **YES** — add the diagonal (§2.3) |
| **`a (p?) b`** (`ZeroOrOnePath`, the special case `{0,1}`) | 0 or 1 hop | reflexive pair ∪ one 1-hop pattern | **YES** |
| **`a (p₁ \| p₂) b`** (alternation) | one hop via `p₁` OR `p₂` | UNION of fixed chains, one per branch | **YES** — union of fixed BGPs (§2.4) |
| **bounded nesting / composition** of the above | e.g. `a (p/q){1,k} b` | distribute the bound; each fixed-length expansion is a fixed BGP | **YES** if every repetition operator carries an explicit finite bound |

**Honest boundary conditions (stated, not hidden):**

- **The bound `k` is PUBLIC and fixed before evaluation.** It is part of the (public) query plan
  — the verifier issues the query (matrix §1.3, convention #4: function/query privacy is
  deliberately forgone). `k` is therefore not itself a secret; what is secret is the graph, the
  intermediate nodes, and which (if any) chain length actually connects a given pair.
- **`{m,k}` with finite `m` and `k` is in; `{m,}` (open upper bound) and `+`/`*` are out.** The
  upper bound is what makes the union finite. An open lower bound `{m,}` is unbounded-depth and
  falls back to §1.1.
- **The unrolling is SOUND only as a *bounded approximation* of the true path.** `a (p){1,k} b`
  is NOT semantically equal to `a (p)+ b` unless the true graph has no `p`-path longer than `k`
  between any returned pair. If a pair is connected ONLY by a chain of length `> k`, the bounded
  operator returns nothing for it — a **false negative relative to the unbounded path**. This is
  a deliberate, documented semantic restriction, not a bug: the operator implements the
  *bounded* path `{1,k}`, which is a well-defined SPARQL 1.1 construct in its own right, and
  whose result the verifier can independently recompute over a disclosed plan. We never claim it
  computes the unbounded closure.
- **Length-revealing variants are out.** A query that wants to return *the length of the path*
  (or to rank by length, or `{m,k}` with the chosen length disclosed) is rejected: revealing the
  actual hop count of a hidden path leaks the graph distance, which is exactly the structure §1.1
  forbids. The operator may return the *binding* `(a, b)` but never the realized length.

### 1.3 Where this sits in the capability matrix

The matrix places bounded property-path in the **MEDIUM zone** (matrix §2 takeaway): it composes
the BGP-chain machinery (P2 equality + P4 mult-chaining for conjunctive match bits + P6
oblivious sort/dedup for the union/dedup). Every one of those was gated behind the
degree-reduction keystone `sq-dvuc`; that keystone is now merged, so this operator moves from
*blocked* to *buildable*. It is **KNOWN, not research-novel**: it is a finite BGP-union, and BGP
joins under MPC are a solved class (ORQ / Secrecy / Senate). The contribution is the *reduction*
(showing the bounded path is exactly this composition) and the *honest leakage accounting*, not
a new protocol.

---

## 2. The unrolling construction

The construction has one core idea and four composition rules. The core idea turns a
fixed-length path into a fixed secret-shared join chain; the rules build the bounded operators
on top.

### 2.1 Core: a fixed exactly-`k`-hop chain → a fixed secret-shared BGP join chain

A fixed exactly-`k` path `?a (p){k} ?b` introduces `k−1` **fresh intermediate variables**
`?z₁ … ?z_{k−1}` and rewrites to the `k`-pattern conjunctive BGP

```text
?a p ?z₁ .  ?z₁ p ?z₂ .  …  ?z_{k−2} p ?z_{k−1} .  ?z_{k−1} p ?b .
```

This is *exactly* a BGP — the operator class the crate already evaluates as a chained join. Two
regimes, mirroring the existing join code:

- **DISCLOSED regime (cheap, crypto-free).** If the path's predicate `p` and the chain's join
  keys are disclosed global IRIs, the whole chain is a plaintext fold of equi-joins computed
  OUTSIDE the crypto core, exactly as `DisclosedKeyJoin` already does — its
  `differential_three_holder_chain_equals_union` test ([`join.rs`](../crates/sparq-mpc/src/join.rs))
  IS a 3-pattern chain unroll evaluated crypto-free. A bounded path over disclosed keys needs no
  new crypto; it is a fixed sequence of `DisclosedKeyJoin` folds. This is the common case and the
  fast default (convention #4).
- **HIDDEN regime (the cryptographic core).** If the intermediate nodes `?z_i` are secret
  (private join values), the chain is a sequence of `k` secret-shared joins where **each
  intermediate node-set stays secret-shared between hops — never opened**. This is where the
  reused primitives live.

**How a single hop stays secret-shared.** Each hop is one `HiddenValueJoin`-style secret-shared
equi-join on the (secret) intermediate node:

1. Each holder secret-shares its private join key into the `n` Shamir parties; the cleartext
   key never leaves the holder ([`join.rs` `HiddenValueJoin` doc / `secure_equal`,
   join.rs:411](../crates/sparq-mpc/src/join.rs)).
2. For a candidate pair `(i, j)` the parties form `d = key_i − key_j` (local, free —
   [`sub_shares`, shamir.rs:634](../crates/sparq-mpc/src/shamir.rs)), draw a fresh nonzero mask
   `r`, compute `m = d·r` (one multiplication, [`mul_shares_raw`,
   shamir.rs:670](../crates/sparq-mpc/src/shamir.rs)), and obtain the **match bit**.
3. Crucially for a *chain*, the match bit must NOT be opened per pair (that is the L2 leak —
   §4). Instead the parties keep the match bit **secret-shared** and feed it into the next hop.

**Why the chain needs the now-merged degree-reduction (P4).** In `secure_equal` the single
product `m = d·r` is *opened* at degree `2t` ([`reconstruct_degree`,
shamir.rs:714](../crates/sparq-mpc/src/shamir.rs)) — that works for *one* multiplication because
the crate only ever opened it once. A `k`-hop chain needs a *product of `k` match bits* (a row
participates in the unrolled chain iff hop 1 AND hop 2 AND … AND hop `k` all match), and each
conjunction is a multiplication whose output must feed the next. A degree-`2t` product cannot be
multiplied again without first being reshared back to degree `t`. That reshare is exactly
[`ShamirDealer::degree_reduce`, shamir.rs:406](../crates/sparq-mpc/src/shamir.rs) (the BGW
reduce-and-recombine round, `sq-dvuc`, now merged): after each hop's match multiplication, reduce
the degree-`2t` product back to a fresh degree-`t` sharing, then AND it with the running chain
bit. The chain bit thus stays secret-shared across all `k` hops, and only the *final* per-row
chain bit drives the output path (§2.5).

**The intermediate node-sets stay secret end-to-end.** Because the per-hop match is a
secret-shared bit (never opened) and the degree reduction produces a fresh degree-`t` sharing
without revealing anything, no intermediate `?z_i` value and no per-hop match pattern is opened.
Only the final binding `(a, b)` (or its existence bit) is revealed, padded and shuffled (§2.5).

### 2.2 `{1,k}` (bounded `+`) → a UNION of `k` fixed chains, deduped

`?a (p){1,k} ?b` = `(?a (p){1} ?b) ∪ (?a (p){2} ?b) ∪ … ∪ (?a (p){k} ?b)` — the union of the
exactly-`ℓ` chains for `ℓ = 1..k`, each unrolled by §2.1. This is `k` fixed BGP chains evaluated
independently (or incrementally — §5 cost note) and unioned. SPARQL property-path semantics over
the bounded form return each connected `(a, b)` pair **once** (set semantics for the path
endpoints), so the union must be **deduplicated**: a pair reachable by both a 2-hop and a 4-hop
chain appears once, and the realized length is NOT disclosed (§1.2 length-revealing prohibition).

### 2.3 `{0,k}` (bounded `*`, reflexive) → `{1,k}` plus the diagonal

`?a (p){0,k} ?b` adds the **length-0 reflexive** case to `{1,k}`: every node is connected to
itself by the empty path. Per SPARQL 1.1 §9 `ZeroOrMorePath` semantics, the reflexive pairs are
`(x, x)` for every `x` in the relevant term set (subjects/objects in scope). The reflexive
contribution is *data-independent given the node set*: it is the identity binding `?a = ?b`,
added to the `{1,k}` union and deduplicated. In the hidden regime the diagonal is the set of
secret nodes each paired with itself — added as a length-0 candidate whose chain bit is the
constant shared `1` (no hop, trivially matched). `ZeroOrOnePath` (`p?`) is the special case
`{0,1}`: the diagonal plus one 1-hop pattern.

### 2.4 Alternation `(p₁ | p₂)` → a UNION of fixed chains, one per branch

A fixed-length path whose hops are alternations expands by distributing the alternation over each
hop position: `?a (p₁|p₂){2} ?b` = the union of the four 2-hop chains
`{p₁p₁, p₁p₂, p₂p₁, p₂p₂}`. Each is a fixed BGP chain by §2.1; their union is deduped by §2.2.
The number of unrolled chains is `(branches)^(hops)` — a static, public combinatorial count,
which is why §1.2 requires every repetition to carry an explicit finite bound (the blowup must be
statically bounded). For a small fixed `k` and a small alternation arity this is a modest fixed
number of BGP chains; for large `k`/arity it is the cost ceiling (§5) and a planner concern, not
a correctness issue.

### 2.5 Dedup + multiplicity handling, and the secret-shared output path

The union of §2.2–§2.4 produces, for each candidate endpoint pair `(a, b)`, one secret-shared
**"connected" bit** = OR over all unrolled chains of that chain's secret-shared AND-of-hops bit.
OR-of-bits is built from the AND/NOT primitives (De Morgan: `OR(x,y) = 1 − (1−x)(1−y)`), each a
single product reduced by `degree_reduce` — the same P4 chaining as the per-chain conjunction.
The result is one secret-shared connected-bit per endpoint pair, with the realized length and the
per-chain match pattern never opened (closing the length leak of §1.2 and the per-hop L2 leak).

Dedup and the result reveal then reuse the **landed oblivious output path** verbatim
([`oblivious_join.rs`](../crates/sparq-mpc/src/oblivious_join.rs), `sq-jnkm`): each endpoint pair
is a [`Candidate`](../crates/sparq-mpc/src/oblivious_join.rs) whose `matched` field is the
secret-shared connected-bit ([`MatchBit::SecretShared`](../crates/sparq-mpc/src/oblivious_join.rs));
[`oblivious_set_output`](../crates/sparq-mpc/src/oblivious_join.rs) then (1) oblivious-selects each
slot against the secret bit (`tag = bit · real_tag`, one product — no chain, so no reduction), (2)
oblivious-shuffles the slots so output position reveals nothing
([`oblivious::shuffle`](../crates/sparq-mpc/src/oblivious.rs)), and (3) reveals exactly a public
padded bound `B` of slots. Set-semantics dedup of the endpoint pairs (each `(a,b)` once,
regardless of how many chain lengths reached it) is achieved by computing the connected-bit per
*distinct* endpoint pair before the output path — the OR-fold already collapses the multiplicity
across lengths, so a pair reached by three different chain lengths yields ONE candidate with bit
`1`, not three. Endpoint-pair dedup across *distinct* secret pairs (the general DISTINCT-over-
hidden-keys case) needs the secure sort comparator (P6, `sq-rrz4`) and is the one piece that is
gated rather than reusable today (§3, §6).

### 2.6 Worked shape

`?a (knows){1,2} ?b` (find people reachable in 1 or 2 `knows`-hops), hidden intermediate:
- Unroll to `(?a knows ?b)  ∪  (?a knows ?z₁ . ?z₁ knows ?b)`.
- Hop bits, all secret-shared: chain-1 bit `c₁ = eq(a,b's-predecessor)`; chain-2 bit
  `c₂ = AND(eq(a,·), eq(·,b))` via one `mul_shares_raw` + one `degree_reduce`.
- Connected bit `= OR(c₁, c₂)` (one more product + reduce).
- Feed `(a,b)` candidates with their secret connected-bit into `oblivious_set_output` with a
  public bound `B ≥ |candidates|`; reveal `B` shuffled slots; recipient filters dummies.
- The result is the set of reachable `(a,b)` pairs; the graph, the intermediate `?z₁`, the
  per-pair match graph, and *which* length connected each pair are all hidden. Only `B` (an upper
  bound on the result size) and the public `k=2` are revealed.

---

## 3. Reuse of existing primitives (cite file:line on `main`)

The operator is a **composition of already-landed pieces** — its novelty is the reduction, not
new crypto. Every primitive below is on `main`.

`HiddenValueJoin::join` was removed; see `HiddenValueJoin::batched_join`.

| Step in the construction | Existing primitive reused | Location (on `main`) | Status |
|---|---|---|---|
| Disclosed-key chain (cheap regime) | `DisclosedKeyJoin` fold; the 3-holder chain test IS a 3-pattern unroll | [`join.rs:119`](../crates/sparq-mpc/src/join.rs); test `differential_three_holder_chain_equals_union` ([`join.rs:656`](../crates/sparq-mpc/src/join.rs)) | **BUILT** |
| Per-hop secret-shared equi-join + match bit | `HiddenValueJoin` / `secure_equal` (`d=a−b`, mask `m=d·r`) | [`join.rs:411`](../crates/sparq-mpc/src/join.rs) (`secure_equal`); inputs `HiddenKeyedRows` [`join.rs:326`](../crates/sparq-mpc/src/join.rs) | **BUILT** (semi-honest) |
| The difference `d = key_i − key_j` (local, free) | `sub_shares` (degree-`t` linear) | [`shamir.rs:634`](../crates/sparq-mpc/src/shamir.rs) | **BUILT** |
| One hop's match multiplication `m = d·r` | `mul_shares_raw` (degree-`2t` product) | [`shamir.rs:670`](../crates/sparq-mpc/src/shamir.rs) | **BUILT** |
| **Chaining `k` hops** (AND of match bits) — reshare degree-`2t` product → degree-`t` so it can be multiplied again | `ShamirDealer::degree_reduce` (BGW reduce-and-recombine, `sq-dvuc`) | [`shamir.rs:406`](../crates/sparq-mpc/src/shamir.rs) | **BUILT (PR #119 merged)** — the keystone |
| AND / OR / NOT of secret bits (conjunction per chain, OR-fold across the union) | `mul_shares_raw` + `degree_reduce` (AND), `add_constant`/`scale` for `1−x` (NOT) | [`shamir.rs:670`,`406`,`611`,`623`](../crates/sparq-mpc/src/shamir.rs) | **BUILT** |
| Opening a degree-`2t` value with the RS consistency check (the final reveal / a single match open) | `reconstruct_degree` (routes through the WI-1 RS checker; honest about zero redundancy at `n=2t+1`) | [`shamir.rs:714`](../crates/sparq-mpc/src/shamir.rs) | **BUILT** |
| Secret-shared output: select-by-secret-bit, shuffle, padded-prefix reveal (dedup of multiplicity + L1/L2 closure) | `oblivious_set_output` consuming `MatchBit::SecretShared`; `Candidate`; `OutputSlot` | [`oblivious_join.rs:254`,`151`,`207`](../crates/sparq-mpc/src/oblivious_join.rs) (`sq-jnkm`) | **BUILT** |
| The oblivious shuffle the output path rides on | `oblivious::shuffle` / `WaksmanNetwork` (sound, no reduction needed) | [`oblivious.rs:629`,`201`](../crates/sparq-mpc/src/oblivious.rs) (`sq-18lk`) | **BUILT** |
| Per-operator, per-`(n,t)` security reporting (so the operator's tier is legible, not a global bit) | `operator_descriptor(OperatorClass)` / `MpcBackend::operator_security` | [`shamir.rs:222`](../crates/sparq-mpc/src/shamir.rs) | **BUILT** |
| Fail-closed refusal of an unsatisfiable security request (e.g. dishonest-majority-malicious, or a hidden UNBOUNDED path) | `SecurityRequirement` + `BackendRegistry::select` / `select_for_operator` (`NoBackendSatisfies`) | [`backend.rs:918`,`1111`,`1124`](../crates/sparq-mpc/src/backend.rs) (`sq-a6p1`) | **BUILT** |

**The one genuinely gated sub-piece (not reusable today):** DISTINCT over *hidden-keyed distinct
endpoint pairs* (collapsing duplicate secret `(a,b)` pairs that came from different input rows,
as opposed to collapsing the multiplicity across chain LENGTHS — that latter is the OR-fold and
is fine) needs the **secure sort comparator** (P6), which is blocked on secure comparison
(`sq-rrz4`) → degree reduction. The sort *network* substrate is BUILT
([`oblivious.rs`](../crates/sparq-mpc/src/oblivious.rs) `SortingNetwork`/`sort_with_keys`) but its
secret comparator is the insecure test-only `SimulatedSecretComparator`. The first operator
increment (§6) therefore handles the common cases where endpoint dedup is over a *disclosed* key
or is unnecessary (e.g. the endpoints `?a`,`?b` are bound to disclosed global IRIs, which is the
headline federation case), and defers hidden-key endpoint dedup behind `sq-rrz4`.

---

## 4. Leakage + security tier

### 4.1 What a bound `k` reveals — precisely, and why it is intrinsic

**A bound `k` reveals exactly one thing: a PUBLIC UPPER BOUND on the path length the operator
will consider.** It is part of the public query plan (the verifier issues the query; query
privacy is forgone by convention #4), so `k` is not even a secret — it is a stated parameter.
What it leaks *about the data* is subtle and must be stated honestly:

- **`k` is an upper bound on the realized hop count of any returned binding** — and nothing
  finer. Because the per-chain match bits and the realized length are kept secret-shared and
  OR-folded before the reveal (§2.5), an observer who sees a returned pair `(a,b)` learns only
  that `a` reaches `b` in **at most `k`** hops of `p` — NOT the actual distance, NOT which chain
  length connected them, NOT the intermediate nodes. The realized length is *never* opened (§1.2
  length-revealing prohibition); revealing it would leak the secret graph distance.
- **`k` does NOT reveal the graph diameter** (contrast the unbounded path of §1.1, whose
  data-dependent iteration count WOULD). The unrolled chain count is `k` (or `(branches)^hops`
  for alternation) — a *public, query-derived* constant, independent of the secret graph. The
  protocol's shape is fixed by the public `k`, so it leaks `k` (already public) and nothing about
  `G`'s structure.
- **The padded bound `B` of the output path leaks an upper bound on the RESULT size** (matrix
  §4.1 L1; `oblivious_join.rs` residual-leakage note), and the public input cardinalities
  `|L|,|R|` / candidate count leak (L3) — these are the *same* standard MPC leaks the hidden join
  already has, NOT new to this operator. `B` and `k` are the two public knobs; both are
  upper-bound leaks by construction and are stated, not hidden.

**Why the `k`-leak is intrinsic, not a design flaw.** To hide `k` you would have to make the
protocol's round/communication shape independent of the path length — i.e. pad to the worst-case
unbounded depth — which is the very thing §1.1 shows is intractable over a secret graph.
Bounding is the *mechanism* by which the operator becomes tractable; the bound is therefore a
necessary, irreducible leak. The honest framing: **you trade a public length-upper-bound leak
for tractability.** A federation that cannot tolerate revealing even an upper bound on path
length cannot use this operator (and there is no tractable alternative over a secret graph — the
registry refuses the unbounded request).

### 4.2 What stays hidden

Under semi-honest honest-majority among cooperating holders (the crate's v1 model):

- **The secret graph** (which edges exist) — never opened; only secret-shared keys enter the
  protocol.
- **The intermediate nodes** `?z₁…?z_{k−1}` on every chain — never reconstructed; the per-hop
  match stays a secret-shared bit.
- **The per-hop / per-pair match graph (fan-out)** — closed because the connected-bit is
  OR-folded and the output rides the oblivious select+shuffle path (`oblivious_join.rs`), so the
  per-pair match bit is NEVER opened (this is the L2 leak the per-pair `secure_equal` open would
  otherwise expose; the output path closes it).
- **The realized path length** and **which chain length connected each pair** — never opened
  (§1.2 / §4.1).
- **The exact result size** — bounded to the public `B`, not revealed exactly (L1 closed to `B`).

### 4.3 Security tier and residual limits (honest)

- **Adversary / threshold:** **semi-honest, honest-majority** Shamir among cooperating holders
  (`t = ⌊(n−1)/2⌋`), the crate's v1 model — `degree_reduce` is explicitly documented as
  honest-majority/semi-honest, NOT maliciously secure ([`shamir.rs:391`-ish doc](../crates/sparq-mpc/src/shamir.rs)).
  The operator inherits exactly this; it claims nothing stronger.
- **Per-operator reporting:** the chain's per-hop equality is the `EqualityJoin` operator class;
  at the minimal honest-majority count `n = 2t+1` the degree-`2t` opens have **zero RS
  redundancy** → semi-honest-only (`operator_descriptor(EqualityJoin)`,
  [`shamir.rs:240`](../crates/sparq-mpc/src/shamir.rs)). The operator must surface its tier
  through this per-operator reporting, never a single global bit.
- **The malicious × confidentiality residual (carried, not solved):** a deviating party can feed
  inconsistent shares into a hop's multiplication or the degree reduction; at `n=2t+1` this is
  information-theoretically undetectable, and per coZK eprint 2025/1026 (cited in the security
  record §4.1 D×A) computing on an inconsistent witness can also be a *confidentiality* hole. The
  named fix is an IT-MAC on the degree-`2t` path (`sq-6d6g` seam / `sq-km34`), which the chain
  would inherit for free once it lands. This operator does NOT introduce a new malicious-security
  hole; it inherits the existing one and is honest about it.
- **Composability caveat (carried):** the operator opens NO value mid-chain (the win of keeping
  match bits secret-shared), so it avoids the `secure_equal`-style mid-pipeline open that the
  security record flags as a UC-composition risk — the *only* opens are the final padded-prefix
  reveal. This is a composition-security *improvement* over the per-pair join, and is worth
  stating as a positive.
- **Fail-closed for the unbounded request:** a hidden UNBOUNDED path (`+`/`*` with no finite
  bound) must be refused through the registry's `NoBackendSatisfies` discipline
  ([`backend.rs:1111`](../crates/sparq-mpc/src/backend.rs)), never silently approximated by some
  default `k` — the bounded result with an unstated `k` would be a *wrong* answer the verifier
  could not recompute. The bound must be explicit and public.

---

## 5. Cost (qualitative — no fabricated numbers)

Costs are given as multiples of the existing hidden-join cost and classes of round/communication
complexity, consistent with the matrix's qualitative-class discipline. The modelled cost can be
*counted* by the existing `CommCounter` once implemented; we do not assert wall-clock numbers.

- **A fixed exactly-`k` chain costs ≈ `k×` the single-hop hidden-join cost**, plus the
  chaining-multiplication overhead: each of the `k` hops is a `secure_equal`-class equi-join
  (the all-pairs `O(|L|·|R|)` structure the matrix flags as the cost center), and each
  conjunction of match bits adds **one `mul_shares_raw` + one `degree_reduce` round** per hop
  (`degree_reduce` is a single BGW reshare round — one simulated communication round,
  [`shamir.rs`](../crates/sparq-mpc/src/shamir.rs) doc step 2). So the round depth grows by
  `O(k)` reduction rounds on top of the per-hop join rounds.
- **`{1,k}` (union of `k` chains) costs ≈ `Σ_{ℓ=1..k} (ℓ × join)` ≈ `O(k²)` join-units** in the
  naive per-length unroll, plus the OR-fold (`k` products + reductions). An *incremental* unroll
  (reuse the length-`ℓ` frontier to extend to length `ℓ+1`) reduces the redundant recomputation —
  a planner optimisation, noted as future work, not a correctness concern.
- **Alternation multiplies the chain count by `(branches)^hops`** — a public combinatorial blowup
  that is the practical ceiling. Small `k` and small alternation arity are the viable regime; the
  planner should reject or warn on a statically-large unroll (the same df/dataset-cap discipline
  the benchmarks use).
- **The output path adds `B` select-multiplications (one parallel round) + the shuffle
  (`O(B log B)` switches, `O(log B)` depth) + `B` degree-`2t` opens** — the `ObliviousOutputCost`
  already modelled in [`oblivious_join.rs:183`](../crates/sparq-mpc/src/oblivious_join.rs), paid
  ONCE for the whole operator (not per hop).
- **The disclosed-key regime is `O(Σ_ℓ chain-length)` plaintext hash-joins, crypto-free** — the
  fast default; a bounded path over disclosed global IRIs costs essentially what the clear-text
  engine costs, with no MPC rounds at all.

**Honest cost framing:** even at SOTA, hidden joins are the cost center (matrix §4.1, ORQ anchor:
minutes-to-tens-of-minutes on LAN; the all-pairs join here is *not* SOTA). A `k`-hop bounded path
multiplies that by the chain length and the union size. This operator is therefore for **small
`k`, small fixed bounds, modest cardinalities** — the same viable envelope as the rest of the
hidden regime. We do not extrapolate beyond it.

---

## 6. Sequenced implementation plan → follow-up impl beads

Ordered so each increment is independently shippable, lands a differential test against the
clear-text engine, and defers the gated piece (hidden-key endpoint dedup) to the end.

1. **Disclosed-key bounded path (cheap regime), exactly-`k` and `{m,k}`.** Unroll a bounded path
   over disclosed global-IRI keys to a fixed `DisclosedKeyJoin` fold + union + dedup, recomputed
   crypto-free; differential test == clear-text `eval_path` over the union store for the bounded
   form. No new crypto. *(Closes the headline federation case — endpoints are disclosed global
   IRIs.)* → **bead filed: see below (#1).**
2. **Hidden-intermediate fixed exactly-`k` chain.** The secret-shared `k`-hop chain of §2.1:
   per-hop `secure_equal`-class match kept secret-shared, conjunction via `mul_shares_raw` +
   `degree_reduce`, final connected-bit into `oblivious_set_output`. Differential test ==
   plaintext bounded-path join over the union, keys never reconstructed. This is the core
   cryptographic deliverable; it depends on the now-merged `sq-dvuc`. → **bead filed (#2).**
3. **Bounded `{1,k}` / `{0,k}` / `p?` + alternation as union-of-fixed-chains, with the OR-fold
   dedup over chain lengths.** Build the union and reflexive diagonal on top of #2; the
   connected-bit OR-fold collapses multiplicity across lengths so each endpoint pair appears once
   without revealing its length. → **bead filed (#3).**
4. **Hidden-key endpoint DISTINCT (the gated piece).** Collapse duplicate *secret* endpoint pairs
   via the secure-comparator oblivious sort + adjacent-equality dedup. Blocked on secure
   comparison (`sq-rrz4`) → degree reduction (already merged). Filed with that dependency so it
   surfaces as ready only when `sq-rrz4` lands. → **bead filed (#4).**
5. **Planner guard + cost model wiring.** Reject/​warn on a statically-large unroll
   (`(branches)^hops` blowup), refuse a hidden UNBOUNDED path through the registry
   (`NoBackendSatisfies`), and wire the operator into `CommCounter`/the matrix runner so its
   modelled `k×`-join + reduction-round cost is *counted* (never a fabricated wall-clock). →
   **bead filed (#5).**

---

## 7. Headline summary

- **Tractable slice:** fixed-`k` paths — sequences `(p₁/…/p_k)`, exact `{k}`, bounded `{1,k}` /
  `{0,k}` / `p?`, and alternation as a union of fixed chains — ARE tractable: they unroll to a
  finite, statically-known disjunction of fixed BGP chains, the operator class the MPC join +
  (now-merged) degree-reduction + oblivious output path already evaluate.
- **Out of reach:** unbounded `+`/`*`, transitive closure of unknown depth, and any
  length-revealing variant over a secret graph — the data-dependent iteration count leaks the
  diameter and the reachability trajectory; refuse, don't approximate.
- **Reuse, not new crypto:** the operator composes `HiddenValueJoin`/`secure_equal`
  (per-hop match), `degree_reduce` (the keystone that lets the `k` match bits chain),
  `mul_shares_raw`/`sub_shares` (the conjunction), and `oblivious_set_output`/`shuffle` (the
  secret-shared, padded, shuffled reveal that closes L1/L2 and dedups multiplicity).
- **Leakage of a bound `k`:** a PUBLIC upper bound on the path length the operator considers
  (≤ `k` hops), nothing finer — never the realized length, the intermediate nodes, the per-pair
  match graph, or the graph diameter; the result size is bounded to the public padded `B`. The
  `k`-leak is intrinsic to tractability, not a flaw.
- **Tier:** semi-honest honest-majority among cooperating holders, per-operator-reported,
  inheriting (not worsening) the existing degree-`2t`-at-`n=2t+1` malicious residual; it opens no
  value mid-chain (a composition improvement over the per-pair join).

---

## 8. Sources

Reuses the corpora of [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md)
§9 and [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) §11. Load-bearing:
W3C SPARQL 1.1 Query §9 (property-path algebra: `ArbitraryLengthPath`, `ZeroOrMore/OneOrMore/
ZeroOrOnePath`, `seq`, `alt`); BGW + Damgård–Nielsen DN07 (CRYPTO'07) (degree reduction); ORQ
(SOSP'25, eprint 2025/1657) + Secrecy (NSDI'23) + Senate (USENIX'21) (BGP-join cost anchor,
joins are the cost center); GORAM (PVLDB'25) (confidentiality-only graph traversal — why it is
NOT general path evaluation); coZK soundness pitfalls (CRYPTO'25, eprint 2025/1026) (the
inconsistent-witness confidentiality interaction); Cleve (STOC'86) (no fairness/GOD without
honest majority — the registry-refusal discipline).

**In-repo ground truth (verified on `main`, 2026-06-15):**
`crates/sparq-mpc/src/shamir.rs` — `degree_reduce` (line 406, sq-dvuc / PR #119 MERGED),
`mul_shares_raw` (670), `reconstruct_degree` (714), `sub_shares` (634), `add_constant`/`scale`
(611/623), `operator_descriptor` (222);
`crates/sparq-mpc/src/join.rs` — `secure_equal` (411), `HiddenValueJoin` (382),
`HiddenKeyedRows` (326), `DisclosedKeyJoin` + chain test (119 / ~656);
`HiddenValueJoin::join` was removed; see `HiddenValueJoin::batched_join`.
`crates/sparq-mpc/src/oblivious_join.rs` — `oblivious_set_output` (254), `Candidate` (151),
`MatchBit::SecretShared` (170), `OutputSlot` (207), `ObliviousOutputCost` (183), sq-jnkm;
`crates/sparq-mpc/src/oblivious.rs` — `shuffle` (629), `WaksmanNetwork` (201), sq-18lk;
`crates/sparq-mpc/src/backend.rs` — `SecurityRequirement` (918), `BackendRegistry::select` /
`select_for_operator` (1111 / 1124), `NoBackendSatisfies`, sq-a6p1.
Beads: epic `sq-0jsc`; this operator `sq-py8h` (dep `sq-dvuc` MERGED); follow-up impl beads
filed by this record (§6) listed in the bead tracker and linked under `sq-py8h`.
