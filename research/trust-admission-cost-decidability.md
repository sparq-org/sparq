# Trust-graph admission — cost + decidability analysis (P8 spike)

Status: **design-for-review analysis record** for `sq-pfae.9` / P8 of the decomposition in
[`research/solid-trust-graph-authz-design.md`](solid-trust-graph-authz-design.md) §6.1
(epic `sq-pfae`, issue [#940](https://github.com/sparq-org/sparq/issues/940); this record is
issue [#3281](https://github.com/sparq-org/sparq/issues/3281)). P8's brief is three things:
**bound admission-rule evaluation cost**, **confirm every seeding direction is
one-side-bound**, and supply the **formal complexity bound the design record explicitly
does not prove** (§7.1 C′: *"No formal complexity bound is proven here"*).

This record is **analysis only**. It changes no code and asserts no measurement: every
claim below is derived from the source on `main` and is cited to `file:line` so a reviewer
can re-check it by reading. Where the honest answer is "not bounded", it says so.

<!-- [OPUS-5] sq-pfae.9 — P8 cost/decidability spike. 🤖 SPARQ agent.
Companion records: solid-trust-graph-authz-design.md (the model; §6.1 P8 + §7.1 C′ are
the anchors this record discharges), trust-graph-authorisation-2026-07.md (the estate
audit), trust-expression-spec.md (the verifier↔holder contract — a different surface). -->

> **HONESTY FRAMING (load-bearing).** No timing, throughput, or wall-clock figure appears
> in this record. Work-box / EC2 measurements are **non-canonical** and are not evidence
> for anything here; the deterministic-metrics substrate for this surface is
> [`bench/trust-admission/`](../bench/trust-admission/README.md) (`sq-r78pf`), which
> records structural counts only and — correctly — states that it does **not** close this
> spike. Everything the trust estate touches inherits the standing ZK/MPC discipline: the
> sparq ZK verifier has **no external accredited-cryptographer sign-off** (open gate
> `sq-qhy4`) and `sparq-mpc` is honest-majority **semi-honest only**. Nothing here is a
> cryptographic, privacy, or unlinkability claim; `sparq-trust` remains a **clear-path
> research prototype**.

## 0. Corrected premise — two things the brief and the design record get wrong

A researcher taking §7.1 C′ at face value would analyse the wrong thing twice over.
Verified against the code on `main`:

**(a) The "two-unbound-atom seeding blow-up" is NOT confined to the incremental counting
path.** §7.1 C′ says the war story "belongs to the **incremental counting path**, not the
path solid runs". That is wrong. The canonical recorded instance of it lives in
[`crates/sparq-solid/rules/common.n3:20-27`](../crates/sparq-solid/rules/common.n3) — a
**full-evaluator** rule file — whose own header comment records that a single rule with two
unbound join atoms made semi-naive evaluation "enumerate the full `?r × ?p` cross product"
on the access-control fixture, and that the rule was **split into candidate + filter on
purpose** so that "every seeding direction has the other side bound". So the war story is
native to the path solid actually runs, it was diagnosed and fixed there by hand, and the
discipline P8 is asked to confirm already has a live in-repo precedent. (The rule file's
header also carries a work-box figure for the regression; that figure is non-canonical and
is deliberately not restated here.)

**(b) The evaluator the WAC/ACP materialiser runs is no longer the text engine.** §7.1 C′
says "solid uses the full evaluator through both `reason_n3` and `reason_n3_stratified`".
Since `sq-zgbso.4` the production materialiser compiles its rules once and evaluates at the
id level through
[`sparq_reason::n3::compiled`](../crates/sparq-reason/src/n3/compiled.rs)
([`crates/sparq-solid/src/materialize.rs:25,47,55-60`](../crates/sparq-solid/src/materialize.rs));
the text `reason_n3` / `reason_n3_stratified` calls survive only as the differential
oracle in that module's `#[cfg(test)]` block (`materialize.rs:349-375`). This matters for
P8 because the two evaluators admit **different rule languages**, and only one of them
rejects anything.

Correcting (a) and (b) changes the question. P8 is not "does the incremental path
generalise"; it is: **what exactly can be handed to each evaluator on the admission path,
and what does that cost?**

## 1. The evaluation paths that actually run

Every place the admission stratum causes rule evaluation, with the evaluator it reaches
and who supplies the rules:

| # | Path | Entry point | Evaluator | Rule text supplied by |
|---|---|---|---|---|
| P-1 | Admitted facts ⋈ `.acr` ABAC rule ⇒ `auth:*` grants | `sparq_trust::wire::derive_grants` ([`wire.rs:83`](../crates/sparq-trust/src/wire.rs)) | **text `reason_n3`** (`wire.rs:103`) | the **caller**, as a `&str` parameter |
| P-2 | Same, per statically-admitted fact | `wire::derive_conditional_grants` (`wire.rs:141`) | text `reason_n3`, once per fact (`wire.rs:152`) | the caller |
| P-3 | HTTP `POST /authz/decide` trust block | `sparq_server::solid_authz` (`solid-authz-trust`, `sq-pfae.17`) | **compiled** WAC/ACP rule sets | **fixed `const`** rule files |
| P-4 | Certification-edge closure | `sparq_trust::graph::derive_effective_rules` (`cert-graph`) | **no reasoner** — pure Rust | n/a |
| P-5 | ODRL → proof-admissibility reduction | `sparq_trust::admissibility::admissible` (`secprop-admissibility`, [`admissibility.rs:210`](../crates/sparq-trust/src/admissibility.rs)) | text `reason_n3_terms` (`admissibility.rs:217`) | **fixed `const` ruleset** + typed-struct-synthesised data |
| P-6 | Trust-expression contract evaluation | `sparq_trust::expression` (`expression`) | `sparq-engine` **SPARQL**, plus P-5 | rewritten query, not N3 rules |

Two structural facts fall straight out of the table.

**P-3 does not take a rule from the wire.** The HTTP trust block parses typed JSON
(`WireTrustRule` / `WireCertification` / `WireCredential`,
[`solid_authz.rs:843-940`](../crates/sparq-server/src/solid_authz.rs)), runs the pure-Rust
closure and the unchanged `admit` gate, injects the surviving *facts* into the pod dataset
(`build_store_with_admitted`, `solid_authz.rs:1550`), and then runs the **unchanged**
materialiser over the shipped `const` rules. There is no channel by which a request
supplies N3. `derive_grants` is not called anywhere in `sparq-server`. **A remote
requester influences the fact extent, never the rule set.** That is the single most
important scoping fact for this analysis and it should be stated wherever the "arbitrary
recursive admission rule" risk is discussed, because it bounds the blast radius to
in-process library callers.

**The only arbitrary-rule channel is a Rust API parameter.** `abac_rule_n3: &str` on
`PodStore::admit_trust_credential_with_rule` / `..._static`
([`crates/sparq-solid/src/trust_wire.rs:139,199`](../crates/sparq-solid/src/trust_wire.rs))
and on `sparq_lws_core::authz::trust_admit::trust_admit_verdict`
([`trust_admit.rs:284`](../crates/sparq-lws-core/src/authz/trust_admit.rs)). The
store-sourced variant `abac_rule_n3_for` currently returns `""`
(`trust_wire.rs:219-221`) — i.e. **the design's `.acr`-authored-rule channel is not wired
yet**. When it is wired, an `.acr` document becomes a rule-supplying surface, and every
bound in §5 becomes load-bearing rather than advisory. That is the moment this record's
recommendations must be in place, and §8 sequences them accordingly.

## 2. The engine's cost model (text `reason_n3`)

Three mechanisms determine cost. All three are read directly from
[`crates/sparq-reason/src/n3/mod.rs`](../crates/sparq-reason/src/n3/mod.rs).

**2.1 Seeding directions.** The fixpoint is semi-naive and unions over **every** join
position: `for &k in joins { match_premise_seeded(…, Some((&delta, k)), …) }`
(`mod.rs:918-928`), where `joins` is every premise index for which `is_join_atom` holds
(`mod.rs:752-756`, classifier at `mod.rs:1450`). So "every seeding direction" in P8's brief
has an exact referent: **one direction per join atom of each rule.**

**2.2 What a seeding direction costs.** `match_premise_seeded` binds the delta atom first,
then evaluates the *remaining atoms in their existing order* with `delta_at = None`
(`mod.rs:1155-1172`, then the plain `for pat in premise` at `mod.rs:1174`). Each atom is
resolved through `FactIndex::candidates` (`mod.rs:146-165`), which picks an index purely by
which positions are **ground after substitution**:

| Bound positions of the atom | Index used | Rows scanned |
|---|---|---|
| predicate + subject | `ps` | objects of that (p, s) |
| predicate + object | `po` | subjects of that (p, o) |
| predicate only | `p` | **the whole extent of that predicate** |
| predicate unbound | `all` | **the whole closure** |

There is no boundness-driven reordering of join atoms anywhere: `order_premise` returns
`true` immediately for any join atom (`mod.rs:1412-1414`), so join atoms keep source order
and only builtins are moved. **Consequently the cost of a rule is a property of its written
atom order, not something the engine repairs.** This is exactly why `common.n3` had to be
split by hand.

**2.3 Rules that opt out of semi-naive entirely.** A rule with a scoped-negation or
aggregation premise, or whose join atoms could be proven by a backward rule, is marked
`needs_full` and re-evaluated against **all** facts every round (`mod.rs:757-768`,
`mod.rs:909-916`). Six of `wac.n3`'s seventeen rules carry `log:notIncludes` and are in
this class; they have **no seeding directions at all**, and their per-round cost does not
shrink as the fixpoint converges.

**2.4 No budget exists.** `run_closure` (`mod.rs:695`) takes no fuel, step, fact, or round
parameter and its driver is an unbounded `loop { … }` (`mod.rs:861`). The id-level
`compiled::eval` has no cap either. The only recursion bound anywhere in the engine is
`BW_DEPTH = 64` for goal-directed backward rules (`mod.rs:176`) and the `log:semantics`
import-cycle guard (`mod.rs:180-197`). **Neither applies to a forward-chaining
non-termination.** Any bound on P-1/P-2 must therefore come from *restricting the rule
language*, not from stopping the evaluator.

## 3. One-side-bound seeding, defined precisely

The `common.n3` comment states the property informally ("every seeding direction has the
other side bound"). Made exact against §2, for a forward rule `r`:

> **OSB(r)** — for every join position `k` of `r`, evaluate the sequence
> `[premise[k]] ++ (premise \ {k}, in order_premise order)` and accumulate bound
> variables. Every subsequent **join** atom must share at least one variable, in subject or
> object position, with the accumulated bound set.

Three notes on why the definition has to be that and not something weaker.

- **Sharing a *variable*, not merely having a ground position, is the load-bearing
  condition.** The pre-split `common.n3` rule had `?p solidx:isResource true` and
  `?r solidx:isResource true`: seeded at the first, the second still has a ground object
  (`true`) and a ground predicate, so it is "one side bound" in the naive reading — and it
  still enumerates the entire resource extent, because the bound side is a
  zero-selectivity constant and the atom shares nothing with the seed. Connectivity to the
  *binding*, not groundness, is what avoids the cross product.
- **Builtin atoms bind too.** `log:uri`, `string:scrape`, `string:concatenation` and the
  rest of the functional family (`Func`, `mod.rs:2406-2475`) are not join atoms but do
  extend the bound set, so the traversal must account for them. In `common.n3`'s
  `parentCand` rule the only join atom is `?r solidx:isResource true`; `?p` is bound by the
  bidirectional `log:uri`, which is why that rule has exactly one seeding direction and
  passes trivially.
- **OSB is not always achievable.** Because §2.2 fixes the order of the non-delta atoms,
  a *chain-shaped* premise `A(x,y), B(y,z), C(z,w)` fails OSB when seeded at `C`: the
  remaining atoms are re-entered at `A`, which shares nothing with `{z,w}`. OSB in **all**
  directions is therefore attainable only for premises that are star-shaped around a
  common variable, or that have at most two join atoms — unless the engine learns to order
  the non-delta atoms by boundness. That is a real structural limitation of the evaluator,
  not of the rules, and it is the highest-leverage finding in this record (§7, F-3).

## 4. The seeding audit

Audited atom by atom against §3: every ruleset `sparq-trust` owns, plus `common.n3` and
`wac.n3` — the WAC stratum that admitted facts flow into on P-3. The **ACP** (`acp-a/b/c.n3`)
and **ODRL** (`odrl-*.n3`) rule files, also reachable on P-3, are **not audited here** (§8
phase 7). `n` = join-atom count.

| Ruleset / rule | n | OSB | Note |
|---|---|---|---|
| P-1 PoC `.acr` rule `{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <r> }` (`wire.rs:72`, and the `tests/*_e2e.rs` fixtures) | 1 | **pass** | single join atom; `math:greaterThan` is a comparison, both sides bound when reached |
| `common.n3` `parentCand` | 1 | **pass** | the post-split form; `?p` arrives via `log:uri` |
| `common.n3` `parent` | 2 | **pass** | the hand-split filter; both directions share `?p` |
| `common.n3` `ancestor` (base) | 1 | **pass** | — |
| `common.n3` `ancestor` (recursive) | 2 | **pass** | seed either atom, the other shares `?p` |
| `wac.n3` `inheritedAcl` base + recursive | 2, 3 | n/a | `log:notIncludes` ⇒ `needs_full`: no seeding directions (§2.3); source order is bound-connected anyway |
| `wac.n3` grant rules (`appliesTo`/`grantsAgent`/`mode`, ×4) | 3 | n/a | `needs_full` as above; star-shaped on `?auth` |
| `wac.n3` own-ACL `appliesTo` | 4 | **fail** | seeded at `?auth a acl:Authorization`, the next atom `?r solidx:ownAcl ?acl` shares nothing → full `ownAcl` extent scan |
| `wac.n3` inherited-ACL `appliesTo` | 5 | **fail, 2 of 5** | seeded at `?auth a acl:Authorization` or `?auth acl:default ?c`, the re-entered atom `?r solidx:inheritedAcl ?acl` shares nothing |
| `wac.n3` `grantsAgent` agent / agentClass (×3) | 2 | **pass** | both atoms share `?auth` |
| `wac.n3` `grantsAgent` agentGroup | 3 | **fail, 1 of 3** | chain `?auth`–`?g`–`?a`; seeded at `?g vcard:hasMember ?a` the re-entered type atom shares nothing (§3, third note) |
| `wac.n3` origin-pair rules (×4) | 4 | **pass** | star-shaped on `?auth`; the minting builtins follow |
| `wac.n3` `acl:Control` governs the ACL resource | 2 | **pass** | both directions share `?r` |
| `admissibility.rs` `CLOSURE_RULES` transitivity | 2 | **pass** | both directions share `?b` |
| `admissibility.rs` `CLOSURE_RULES` strict⇒atLeast, reflexive | 1 | **pass** | — |
| `admissibility.rs` `DISCHARGE_RULE` | 8 | **fail, every direction** | see below |

**The `DISCHARGE_RULE` finding (F-1).** Its premise
(`admissibility.rs:155-160`) is two disjoint components joined only late: a constraint side
`{?c, ?lo, ?required, ?dim}` (atoms 0–3) and a method side `{?m, ?a, ?dim, ?have}` (atoms
4–6), first connected at atom 5 via `?dim`. Atom 4, `?m secx:hasProperty ?a`, shares no
variable with atoms 0–3, so under **every** seeding direction some evaluation step falls
back to a whole-predicate scan and the intermediate is a constraint × method cross product.
This is the `common.n3` shape exactly. It is *bounded* — the policy is synthesised from
typed structs and the annotations are resolved from a bundled per-method table
(`admit.rs:774-798`, `admit.rs:829-845`), so both extents are small and fixed — and it is
behind the default-OFF `secprop-admissibility` feature, so it is a **cost defect, not a
security defect**. The fix is a pure premise reorder to `0,1,2,3,5,6,4,7`, which makes
every step share a bound variable when seeded from the constraint side. Full OSB in all
eight directions is not reachable by reordering alone (§3, third note).

**The `wac.n3` findings (F-2)** are the same shape at smaller magnitude and are out of this
record's crate scope; they are recorded here because admitted facts feed that stratum on
P-3, and they are filed as follow-up work rather than fixed. All three share one cause: a
**low-selectivity type or marker atom written first** (`?auth a acl:Authorization`,
`?auth acl:default ?c`) becomes a whole-extent scan when the fixpoint re-enters the premise
at it. That is a recurring authoring hazard, not three unrelated defects, and it is what
makes F-3 the structural fix rather than a per-rule cleanup.

**Verdict on P8's second obligation.** *"Confirm every seeding direction is one-side-bound"*
— **not confirmable as stated, and the honest answer is more useful than a yes.** It holds
for every rule `sparq-trust` puts on the v1 admission path (P-1 and the `common.n3`
ancestry stratum admitted facts flow through). It fails in `DISCHARGE_RULE` and in three
`wac.n3` rules, in every case with **bounded, polynomial** cost over fixed-size or small
extents — never unbounded. The design record's sentence *"with predicate-IRI typing
and one-side-bound seeding they terminate"* (§3.3, Termination) conflates two different
properties: **OSB governs cost; it has nothing to do with termination.** Termination is
governed by §5 below, and a rule can be perfectly OSB and still not terminate.

## 5. The complexity bound

### 5.1 The fragment

Define **BAF** (bounded admission fragment) for a forward rule set `R` evaluated over an
input fact set `F`:

1. **Safety / range-restriction.** Every variable of a conclusion occurs in some premise
   **join** atom of the same rule.
2. **No head existentials.** No conclusion contains a blank node.
3. **Ground predicates, premise *and* conclusion.** Every premise join atom has an IRI
   predicate, **and so does every conclusion atom**.
4. **No generator on a recursive cycle.** Build the predicate dependency graph (edge
   `p → q` when `q` heads a rule with `p` in its premise). No rule that contains a
   *functional* builtin — `Func` (`mod.rs:2406`), a list generator (`mod.rs:1927`), or a
   `Binder` (`mod.rs:1889`) — may have its head predicate on a cycle of that graph.
5. **No scope re-entry.** No `log:semantics` / `log:content` / `log:conclusion` /
   `log:parsedAsN3` / `log:supports`, and no backward (`<=`) rules.
6. **No term construction in a conclusion.** No conclusion term is a compound
   `Term::List` / `Term::Triple` / `Term::Formula` ([`model.rs:14-24`](../crates/sparq-reason/src/n3/model.rs))
   that contains a variable.

Conditions (3)-on-conclusions and (6) are **not decorative** — each closes a term source
that (1)–(5) alone leave open, and both are reachable in this engine today:

- **A variable conclusion predicate is safe but unbounded in `P`.** (1) is satisfied by
  `{ ?x :grants ?p . ?p a :Mode } => { ?x ?p :r }`: `?p` occurs in a premise join atom, so
  the rule is range-restricted — yet the derived predicate is drawn from the active domain,
  not from a fixed rule-head set. Without (3)-on-conclusions, `P ⊆ A` and the Herbrand-base
  bound below degrades from `|P|·|A|²` to `|A|³`.
- **A safe head can *build* a term, with no builtin and no blank node.** `Term` is richer
  than RDF: `List` and `Triple` are compound terms, `apply_deep` substitutes **into** them
  (`mod.rs:1984-2003`), and `ground_triple`'s instantiation check accepts a compound whose
  components are all instantiated (`mod.rs:1740-1744`), so a list- or quoted-triple-valued
  conclusion is derived, not rejected (live test: `tests/n3_query.rs:183`). Hence
  `{ ?x :seen ?l } => { ?x :seen (?l) }` mints a strictly deeper term every round and
  **never reaches a fixpoint**, while satisfying (1), (2), (4) and (5) — it contains no
  blank node and no `Func`/`ListGen`/`Binder`. (6) is what excludes it.

Accepted / rejected, for a checker to pin (each row is a test case for the BAF gate of §8
phase 4 — the `math:greaterThan` acceptance is subject to §9 question 2):

| Rule | Verdict | Condition |
|---|---|---|
| `{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <r> }` | accept | the P-1 PoC rule; `math:greaterThan` is a comparison, not a generator |
| `{ ?r solidx:parent ?p . ?p solidx:ancestor ?a } => { ?r solidx:ancestor ?a }` | accept | recursive but datalog: ground head predicate, no constructor |
| `{ ?x :grants ?p . ?p a :Mode } => { ?x ?p :r }` | **reject** | (3) — variable conclusion predicate |
| `{ ?x :seen ?l } => { ?x :seen (?l) }` | **reject** | (6) — recursive list construction, non-terminating |
| `{ ?x :p ?y } => { ?x :q <<( ?x :p ?y )>> }` | **reject** | (6) — quoted-triple construction (terminating here, but the domain is no longer `A`) |
| `{ ?x :n ?n . (?n 1) math:sum ?m } => { ?x :n ?m }` | **reject** | (4) — generator on its own head predicate's cycle |

### 5.2 The bound

**Claim.** For `R ∈ BAF` over `F`, `reason_n3(F ∪ R)` terminates, and with
`A` = the **active domain** (every ground term occurring in `F`, in `R`'s constants, and in
the finitely many terms mintable by the acyclic generator builtins of (4)), `P` = the set of
IRIs occurring as a conclusion predicate in `R` — a **fixed, rule-determined** set, by (3) —
`v` = the largest number of distinct variables in any premise:

- closure size: `|C| ≤ |F| + |P|·|A|²`
- rounds to fixpoint: `≤ |C| − |F| + 1`
- total work: `O(|R| · |A|^v · |C|)` naively; semi-naive removes one `|C|` factor per
  non-`needs_full` rule, and each atom's real cost is the index row-count of §2.2.

**Why it holds.** The engine has **exactly three** ways to put a ground term into a derived
fact that was not already in `A`, and BAF closes all three: existential invention (removed
by (2)), term-minting builtins (confined by (4) to a stratified, finite cascade), and
**head term construction** — a conclusion `List`/`Triple`/`Formula` assembled from bound
variables, removed by (6). With all three closed, every derived term is in `A`. (1) means
every derived fact is fully ground. (3) does double duty: on the premise side it keeps every
atom off the `all`-scan branch of §2.2, and on the conclusion side it pins the derived
predicate set to the fixed `P` rather than letting it range over `A`. (5) removes the only
re-entrant control flow. So the Herbrand base is finite and bounded by `|P|·|A|²`; the
fixpoint is monotone (the engine never retracts) and adds at least one fact per round, so it
converges within `|C|` rounds. This is the classical datalog result — for
fixed `R`, **data complexity is PTIME** (Dantsin, Eiter, Gottlob & Voronkov, *Complexity and
expressive power of logic programming*, ACM Computing Surveys 33(3), 2001, §4) — and BAF
conditions (1)–(6) are precisely what reduce the N3 fragment to datalog-with-stratified-
functions. Nothing in this claim is novel; the contribution is pinning the exact conditions
under which sparq's engine satisfies its hypotheses.

**How the claim degrades if a condition is dropped**, since the two added this revision were
missing from an earlier draft of this record and the omission was not benign:

| Dropped | Effect |
|---|---|
| (3) on conclusions only | still terminates and still PTIME, but `P ⊆ A`, so the Herbrand-base bound becomes `\|A\|³` and `\|C\| ≤ \|F\| + \|A\|³` |
| (6) | **the claim fails outright** — the Herbrand base is infinite and the fixpoint need not exist (`{ ?x :seen ?l } => { ?x :seen (?l) }`) |
| (2) or (4) | as (6): unbounded domain growth, no fixpoint (§6, U-1/U-2) |

**What the bound does not say.** It is an upper bound in `|A|`, and `|A|` on the admission
path includes **externally attested terms** — the credential graph a requester presents.
A rule whose premise has `v` variables therefore has a worst case polynomial *of degree v*
in attacker-influenced input size. The bound is a decidability and asymptotic-shape result;
it is **not** a statement that any particular rule is cheap, and it must not be cited as
one.

### 5.3 Membership of the paths in §1

| Path | In BAF? | Consequent bound |
|---|---|---|
| P-1 / P-2 with the PoC `.acr` rule | yes | closure = input facts plus at most one grant per admitted fact; 2 rounds. P-2 runs one closure **per** admitted fact — linear in admitted-fact count, and the *only* super-linear term in the trust crate's own evaluation |
| P-1 / P-2 with an **arbitrary** caller rule | **unknown — nothing checks** | none; see §6 |
| P-3, WAC stratum (`common.n3` + `wac.n3`) | yes, **by construction** | `string:scrape`/`concatenation`/`encodeForUri` mint terms but their head predicates (`parentCand`, `auth:*` pair principals) are off every cycle, satisfying (4). Every conclusion in both files has a ground IRI predicate and no compound term, satisfying (3) and (6). Closure bounded by resources × authorizations × principals |
| P-3, ACP + ODRL strata (`acp-*.n3`, `odrl-*.n3`) | **not audited** | outside this record (§8 phase 7). Compilation by `n3::compiled::compile` establishes BAF (1)(2)(5)(6) for them — `sym` rejects list-, formula- and non-ground-quoted-triple constants outright (`compiled.rs:397-404`), which is (6) — but **not** (4), and **not** (3) on conclusions: a `Term::Var` in conclusion predicate position compiles as an ordinary bound slot (`compiled.rs:825-831`) |
| P-4 cert-graph closure | n/a (no reasoner) | signature verifications proportional to depth × anchors × certification edges; the shipped implementation is depth-1 (`bench/trust-admission/README.md`) |
| P-5 admissibility ruleset | yes | active domain = the 32 level IRIs + 7 leftOperand IRIs of `LEVEL_ORDERS` plus the policy's constraints; transitive closure over chains of length ≤ 4 |

Every **yes** row was re-checked against the added conditions (3)-on-conclusions and (6):
the PoC `.acr` rule, `common.n3`, `wac.n3` and `admissibility.rs`'s `CLOSURE_RULES` /
`DISCHARGE_RULE` all conclude with a ground IRI predicate and no compound term, so the
membership verdicts are unchanged by this revision. That the shipped rules *happen* to
satisfy conditions nothing enforces is exactly the §10 caveat, now over six conditions
rather than five.

## 6. What is not bounded, and how exposed it is

**U-1 — an arbitrary caller rule on P-1/P-2 is undecidable.** N3 is Turing-complete (the
engine's own module docs say so, `mod.rs:75`), `run_closure` has no budget (§2.4), and
`derive_grants` hands the caller's string straight to it (`wire.rs:100-103`) after no
validation beyond parsing. A rule that violates BAF (4) — a functional builtin whose output
feeds its own head predicate, e.g. a `math:sum` that increments a value the same rule
consumes — mints a fresh term every round and never reaches a fixpoint. The failure mode is
a **hang**, not an error: `derive_grants`' `Result` cannot express it.

**U-2 — a head existential on a cycle.** BAF (2) exists because conclusion blank nodes are
instantiated fresh per distinct conclusion-binding (`mod.rs:934-957`); a recursive rule that
feeds its own minted blank back into its premise grows the domain without bound. The
`fired` set deduplicates repeats of the *same* binding, so this needs a genuinely recursive
head — but nothing rejects one.

**U-3 — `needs_full` rules do not amortise.** §2.3: cost scales with rounds × full extent,
so a large closure with negation-guarded rules is superlinear even inside BAF.

**U-4 — a term-constructing head needs no builtin and no blank node.** BAF (6) exists
because `Term` carries compound `List` / `Triple` / `Formula` values (`model.rs:14-24`),
`apply_deep` substitutes into them (`mod.rs:1984-2003`) and the instantiation check accepts
the result (`mod.rs:1740-1744`). So a rule that is safe, existential-free and
builtin-free — `{ ?x :seen ?l } => { ?x :seen (?l) }` — still nests a fresh term every
round and hangs. This is the same hang as U-1 through a channel U-1's "functional builtin"
framing does **not** cover, and it is the one BAF condition the compiled subset already
enforces for free (§7, F-6).

**Exposure, stated plainly.** As of this record, U-1, U-2 and U-4 are reachable **only** by an
in-process Rust caller that passes a rule string, because (a) `abac_rule_n3_for` returns
`""` so no `.acr` rule is loaded from a store, and (b) the HTTP trust block supplies facts,
never rules (§1). So today these are **latent** defects, correctly classified as
availability rather than authorisation risks — an unbounded closure yields no grant, and
the whole trust path is additive and fail-closed, so a hang denies rather than escalates.
They become live the moment the design's `.acr` rule channel is wired, which is exactly
what P8 was scheduled to precede.

## 7. Findings

- **F-1** — `admissibility::DISCHARGE_RULE` violates OSB in every seeding direction
  (constraint × method cross product). Bounded, feature-gated, fixable by premise reorder.
- **F-2** — three `wac.n3` rules violate OSB in at least one direction (own-ACL
  `appliesTo`, inherited-ACL `appliesTo`, `grantsAgent` agentGroup), all from a
  low-selectivity type/marker atom written first. Bounded; out of crate scope here.
- **F-3** — the text engine does not order non-delta join atoms by boundness
  (`mod.rs:1174`), so OSB is unattainable in all directions for chain-shaped premises and
  every rule author must hand-tune atom order, as `common.n3` did. A greedy
  bound-first traversal in `match_premise_seeded` would make OSB automatic and retire the
  hand-splitting discipline. **Highest leverage finding in this record.**
- **F-4** — `derive_grants` performs no static check on a caller-supplied rule and the
  engine has no budget, so P-1/P-2 are undecidable in general (U-1, U-2, U-4).
- **F-5** — `derive_conditional_grants` runs a full closure **per admitted fact**
  (`wire.rs:146-152`). Correct (it is what preserves the holder↔grant association) but
  linear in fact count where one closure over holder-tagged facts would do.
- **F-6** — the fix for F-4 already exists and is tested: `n3::compiled::compile` rejects
  backward rules, conclusion existentials, `log:includes`/`supports`, list builtins,
  `math:`/`time:` builtins, list- and formula-valued terms, quoted triples with variables in
  a conclusion, and "rules whose builtin inputs no premise can bind" as **loud compile
  errors** (`compiled.rs:58-71`, `compiled.rs:397-404`, `compiled.rs:838-842`), and
  `tests/compiled_equivalence.rs` pins its closure set-equal to `reason_n3` on the
  access-control corpus. That is BAF (1)(2)(5)(6) and (4-partial) already implemented,
  behind the opt-in `compiled-rules` feature, and it is the evaluator the materialiser that
  consumes admitted facts already uses. It does **not** give (3) on conclusions: a variable
  in conclusion predicate position compiles as an ordinary bound slot
  (`compiled.rs:825-831`), so the residual checks phase 4 must add are (3)-on-conclusions
  and the (4) cycle check — two conditions, not one.

## 8. Recommendation and phased plan

**Recommendation.** Do **not** build a new rule analyser or a reasoner budget. Route the
admission path's rule text through the fragment gate that already exists, and add only the
two conditions that gate is missing — (3) on conclusions, and the (4) cycle check.

Each phase is a future bead; each is independently shippable and independently reviewable.

1. **Reorder `DISCHARGE_RULE`'s premise to `0,1,2,3,5,6,4,7`** (F-1). Pure constant edit in
   `admissibility.rs`; the existing golden §4.3.3 test pins the result set unchanged, so
   the obligation is "same admissible set, no cross-product intermediate". Smallest, lowest
   risk, and it makes the shipped ruleset consistent with the discipline this record states.
2. **Add an OSB checker to `sparq-reason` as a `debug_assert`-grade rule lint** (F-3
   precursor). A pure function over `parser::Parsed` implementing §3, plus a test asserting
   the pre-split `common.n3` rule fails it and the post-split pair passes. Gives every
   future rule author a mechanical answer instead of a war story.
3. **Bound-first traversal in `match_premise_seeded`** (F-3). Order the non-delta atoms
   greedily by "shares a bound variable" instead of by source position. Result-equivalence
   is the obligation (closure set-equality on the WAC/ACP/ODRL corpus, the oracle
   `tests/compiled_equivalence.rs` already establishes for the compiled path). This is the
   change that makes OSB a property of the *engine* rather than of every rule file.
4. **Gate P-1/P-2 on the compiled subset** (F-4/F-6). Add an opt-in
   `derive_grants_checked` to `wire.rs` that runs `n3::compiled::compile` on the caller's
   rule and returns `Err` — never a hang — when it is rejected, plus the two checks
   `compile` does not make: a BAF-(4) cycle check for the term-minting builtins it still
   admits (`string:concatenation`, `string:scrape`, `log:uri`), and a BAF-(3) rejection of a
   variable in conclusion predicate position. Feature-gated so the default build is unchanged; the
   `.acr` rule channel, when wired, uses only the checked entry point. **Blocked on §9
   question 2:** `compile` currently rejects `math:` builtins, so as written this phase
   would reject the design's own worked example (`math:greaterThan`) until comparison
   builtins are added to the compiled subset.
5. **Fold the per-fact closure of `derive_conditional_grants` into one run** (F-5), keeping
   the holder association by tagging rather than by isolation. Obligation: the four
   existing `wire.rs` conditional-grant tests, including
   `conditional_grant_drops_third_party_subject`, stay green unchanged.
6. **Extend `bench/trust-admission`'s deterministic schema with the structural cost inputs
   this record names** — per-rule join-atom count, OSB pass/fail per seeding direction,
   active-domain size, rounds to fixpoint. All are integers and clock-free, so they fit the
   existing schema contract, and they turn §5's bound into a *gated* rather than a
   *narrated* property. Only after this should `bench/trust-admission/README.md`'s "does
   not close that cost/decidability spike" line be revised.
7. **Audit `acp-a/b/c.n3` and `odrl-*.n3` against §3** (F-2 and the rest). Deliberately
   scoped out of this record — those files are `sparq-solid`'s, this record's crate scope
   is `sparq-trust`, and `wac.n3` was audited only because it is the stratum admitted facts
   flow into. Roughly 900 lines of rules remain unaudited.

Phases 1, 2, 6 and 7 are independent. Phase 3 should land before 4 (it changes what OSB
means operationally). Phase 4 is the one that must precede wiring the `.acr` rule channel.

## 9. Open questions for the maintainer

1. **Is the `.acr`-authored ABAC rule channel still intended?** `abac_rule_n3_for` is a
   stub returning `""`. If controller-authored rules will be read from pod documents, phase
   4 is a prerequisite and the exposure analysis in §6 changes from latent to live. If the
   channel is being dropped in favour of fixed server rules, phases 4 and 5 can be deferred
   indefinitely and P8's residual risk is essentially zero.
2. **Should the admission path accept only the compiled subset, permanently?** Doing so
   trades N3 expressivity (no `math:`, no lists) for a decidability guarantee. The PoC rule
   uses `math:greaterThan`, which `compile` currently **rejects** — so phase 4 as written
   would reject the design's own worked example unless comparison builtins are added to the
   compiled subset first. That is a genuine design fork, not an implementation detail, and
   it is the single question this record most needs answered.
3. **Is a cost bound of degree `v` in attacker-influenced input size acceptable** for a
   research PoC, or should the admission path additionally cap the admitted-fact count
   before evaluation? §5.2's bound is polynomial but the degree is set by the rule author.

## 10. What this record does NOT establish

- It proves **no** security property. Statement-type scoping, no-laundering and key-binding
  remain **designed, not verified** (design §7.1 E, `sq-pfae.4`), and the issuer-key
  forgery vector D′ (`sq-pfae.3`) is untouched by anything here.
- It contains **no measurement**. Every quantity is structural (atom counts, index choice,
  fragment membership). No timing was taken and none would be canonical if it had been.
- It does not bound `sparq-engine` SPARQL evaluation on P-6; the trust-expression rewrite
  has a different cost model and is out of scope.
- It leaves ~900 lines of `acp-*.n3` / `odrl-*.n3` unaudited (§8 phase 7).
- The BAF bound is **conditional on conditions (1)–(6) being checked**. Nothing on the
  text-engine admission path checks them today (the compiled path checks four of the six —
  §7 F-6). Until phase 4 lands, §5 describes a fragment the admission path *happens* to stay
  inside, not one it is *held* inside.
- The BAF conditions are a **sufficient** set, argued informally from the engine's term
  sources and index behaviour and cited to `file:line` — not a machine-checked proof, and
  not claimed minimal. Conditions (3)-on-conclusions and (6) were added after review found
  that (1)–(5) admitted both a variable-predicate head (breaking the `|P|·|A|²` bound) and a
  recursive compound-term head (breaking termination outright); a further such gap is
  possible, and the conditions should be treated as the current best statement rather than a
  closed result until the phase-4 checker and its rejection tests exist.
