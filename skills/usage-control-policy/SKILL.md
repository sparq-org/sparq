---
name: usage-control-policy
description: Evaluate W3C ODRL 2.2 usage-control policies over RDF with the opt-in sparq-policy crate — parse an ODRL Set/Offer/Agreement into a typed Policy of Permission/Prohibition rules (action, target, assignee, Constraint, Duty), then evaluate an access Request to a fail-closed ALLOW/DENY Decision. Use when gating a query/asset by purpose, recipient, time window, count, or a duty obligation; when mapping ODRL to the sparq-solid WAC/ACP allow-deny model; or when wiring usage control above access control. Single-node base case; federated ODRL-to-MPC disclosure is deferred.
---

# usage-control-policy

`sparq-policy` is the declarative **usage-control** layer above access control. Where `sparq-solid` answers "may this agent **read** graph G?", `sparq-policy` answers "may this party **use** this asset *for purpose P, with obligation O, until time T, disclosing only to recipient R*?" — by evaluating a [W3C ODRL 2.2](https://www.w3.org/TR/odrl-model/) policy.

It parses an ODRL policy from RDF into a typed model (`Policy → {permissions, prohibitions}`, each `Rule` carrying an `Action`, `target`, `assignee`/`assigner`, `Constraint`s and `Duty`s) and evaluates an access `Request` to a **fail-closed** `Decision { allow, matched_rules, unmet_constraints }`. This is the **single-node base case**: ODRL over one node's data, reducing to the same allow/deny shape `sparq-solid` enforces.

> **Scope.** Single-node only. The headline federated-disclosure / ODRL→MPC composition (per-node ODRL drives the `sparq-mpc` disclosed-vs-hidden split; ODRL `Duty` → ZK proof obligation) is **deferred** — it inherits the MPC honest-majority/LAN envelope and the open ZK-soundness remediation. See `research/feature-research-odrl-policy.md`.

## Prerequisites

- **Cargo dep** — `sparq-policy` is `publish = false`, a non-default workspace member (nothing in core depends on it; `cargo tree -p sparq-core` never shows it). Add it as a path dep:
  ```toml
  sparq-policy = { path = "crates/sparq-policy" }
  ```
- No external toolchain. It depends only on `sparq-core` + `sparq-engine` + `oxrdf`; zero `unsafe`.

## Quickstart

Parse an ODRL `Set` and evaluate a time-windowed permission. Compiles and runs against the current API.

```rust
use sparq_policy::{evaluate, parse_policy_str, Request, Value};

// alice MAY read asset-X, on or before 2026-12-31, for research purpose.
let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/1> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime ] ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <urn:purpose/research> ] ] .
"#;
let policy = parse_policy_str(ttl, "turtle").expect("parse");

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
let req = Request::new(format!("{ODRL}read"))
    .on("urn:asset/x")
    .by("https://alice.ex/me")
    .with(format!("{ODRL}dateTime"), Value::DateTime("2026-06-16T09:00:00Z".into()))
    .with(format!("{ODRL}purpose"),  Value::Iri("urn:purpose/research".into()));

let d = evaluate(&policy, &req);
assert!(d.allow);                      // in-window, right party, right purpose
assert_eq!(d.matched_rules.len(), 1);  // the granting permission, for audit
```

## The model

- **`Policy`** — `permissions: Vec<Rule>`, `prohibitions: Vec<Rule>`, optional `iri`, and `conflict: Option<ConflictStrategy>` (the parsed `odrl:conflict` strategy — see `conflict_admissibility` under static analysis). `Set`/`Offer`/`Agreement` parse identically (subtype affects contracting, not single-node eval).
- **`Rule`** — `action: Action`, `target`/`assignee`/`assigner: Option<String>`, `constraints: Vec<Constraint>`, `logical_constraints: Vec<LogicalConstraint>`, `duties: Vec<Duty>` (duties on permissions only). The atomic `constraints` and the compound `logical_constraints` are **all** ANDed.
- **`Action`** — full IRI. `odrl:use` is the ODRL **umbrella** action: a permission for `use` permits any requested action *in the `use` subtree of the [ODRL 2.2 action hierarchy](https://www.w3.org/TR/odrl-vocab/)* — i.e. everything **except** the disjoint ownership-`transfer` subtree (`odrl:sell`/`odrl:give`/`odrl:transfer`). So a `use` permission grants `read`/`write`/`modify`/… but **not** `sell`. All non-`use` actions match by exact IRI. [OPUS-4.8] sq-euhr3.
- **`Constraint`** — atomic `(left: leftOperand-IRI, operator, right: Value)`. The request supplies the *actual* value for `left` in its `context`; the constraint's `right` is the bound.
- **`LogicalConstraint`** — a compound `odrl:LogicalConstraint`: a `LogicalOperator` (`And`/`Or`/`Xone`) over a set of operand `ConstraintNode`s (each atomic **or** nested compound). [OPUS-4.8] sq-a0zef.
- **`Duty`** — an obligation (`action`) that must be in the request's `discharged_duties` set, or the permission is denied.

## Evaluation semantics (fail-closed)

1. A `Rule` **matches** when its action permits the request action (per the ODRL action hierarchy — `use` subsumes its sub-actions but not the `transfer` subtree), its `target`/`assignee` (if set) agree (by IRI equality **or collection membership** — see below), and **every** atomic `Constraint` **and** compound `LogicalConstraint` is satisfied (all ANDed).
2. A `Permission` grants iff it matches **and** all its `Duty`s are discharged.
3. A matching `Prohibition` **overrides** any permission (carve-out — deny-overrides, the one conflict strategy implemented; NOT a spec default: the ODRL 2.2 IM default for an unset `odrl:conflict` is `invalid`, and the ODRL Formal Semantics CG report's conflict machinery is explicitly pending — see the crate README's ODRL conformance note).
4. **DENY by default:** no matching+discharged permission, or any matching prohibition ⇒ DENY. An empty/malformed policy denies everything; a constraint with no request value, an unknown operator, or a structurally incomplete constraint all fail closed.

## Constraint operators

`eq`, `neq`, `lt`, `lteq`, `gt`, `gteq`, `isPartOf` (set membership: `right` is a `|`/space/comma-separated set — **or**, for the taxonomic dimensions `purpose`/`spatial`, a transitive subsumption match over a request-supplied closure, see below), `isA` (identity, ≈ `eq`), `isAnyOf` (the value equals **at least one** member of the right-operand set — same set encoding and matching as `isPartOf`, incl. the taxonomic subsumption case; [FABLE-5] sq-uaz85), `isNoneOf` (the value equals **none** of them — the negative dual; a sub-value of an excluded member is also excluded under a supplied taxonomy, missing evidence stays *unprovable* — never vacuously satisfied — and a numeric/dateTime operand fails closed rather than negating a lexical representation gap). Numeric (`xsd:integer`/`decimal`/`double`/…) and `xsd:dateTime`/`date` operands compare by magnitude/instant; everything else by IRI/string value. Order comparison on non-orderable values is `false` (fail-closed). **`dateTime` ordering compares the real UTC instant each lexical form denotes — mixed timezone offsets are normalized to UTC before comparing** (`parse_instant().cmp()`, sq-qj2q; an unparseable operand is incomparable → fail-closed), so `2026-06-16T13:00:00+02:00` (= `11:00Z`) correctly orders *before* `2026-06-16T12:00:00Z`. The single offset-aware comparator is `sparq_policy::cmp_datetime` (one source of truth; doctest `cmp_datetime_orders_by_instant_not_lexical`).

**Multi-valued `rightOperand` ([FABLE-5] sq-ueydm):** the parser folds an RDF-list right operand (`odrl:rightOperand ( <a> <b> )` — the `rdf:first`/`rdf:rest` chain) and a multi-object one (`odrl:rightOperand <a>, <b>`) into that same `|`-separated set encoding, so the set-relation operators (`isPartOf`/`isAnyOf`/`isNoneOf`) see the full member set (previously first-binding-wins dropped members / degraded a list to an unmatchable blank-node string). A single value — including a one-element list — keeps the typed (numeric/dateTime) path unchanged. Fail-closed degradations: a multi-value under a **non-set** operator (`eq`, `lt`, …) is ambiguous → the unsatisfiable guard; a member carrying a separator character (`|`/whitespace/`,`) cannot be encoded faithfully → the whole constraint is unsatisfiable (never split into unintended members); an empty list (`rdf:nil`) stays the unmatchable nil IRI. A **malformed** collection right operand (broken/dangling tail, cycle, forked cons cell, or a member-less head asserting `rdf:rest` with no `rdf:first`) **REFUSES the whole parse** (`Err`) rather than contributing its valid prefix — or, for the member-less head, being read as an ordinary unmatchable blank node ([FABLE-5] sq-srjuc): dropping authored members widens decisions on *both* rule kinds — `isNoneOf` on a permission would exclude fewer values than authored, and any set-op constraint gating a prohibition would narrow the carve-out (deny-overrides bypassed) — and degrading the single constraint to the unsatisfiable guard is no better, since that *disables* a prohibition. Same refusal precedent as the combinator fold below. Tests: `tests/odrl_right_operand_sets.rs`. A list-valued *combinator* object (`odrl:or ( … )`) is folded into its member operands under the same well-formedness rule ([FABLE-5] sq-dkuff; `tests/odrl_constraint_matching.rs`).

## `odrl:spatial` region enforcement + region `isPartOf` trees — [OPUS-4.8] sq-wukl

A `spatial` constraint gates a rule to a **geographic region** (`spatial isPartOf <country/EU>`, "anywhere in the EU"). The new left-operand IRI is `ODRL_SPATIAL` (`odrl:spatial`); supply the request's region with `.with(ODRL_SPATIAL, Value::Iri(region))`, gated through the **same** `evaluate` constraint path as every other dimension.

The spatial dimension is **taxonomic**, so it reuses the **same** caller-supplied subsumption closure the DPV purpose taxonomy uses — there is *no* separate spatial evidence channel. Declare the region `isPartOf` tree with `.with_purpose_subsumption(narrower, broader)` (one edge) or `.with_purpose_taxonomy([(n, b), …])` (bulk), and a `spatial isPartOf <EU>` constraint is then satisfied by a request whose stated region is the EU **or transitively part-of** it — `Berlin ⊑ DEU ⊑ EU`.

- **Honesty / fail-closed:** the tree is *the requester's asserted subsumption*, never invented. With **no** edge supplied, `spatial` matching is exact membership (fully backward compatible) and a sub-region does **not** grant a broad-region permission — a missing edge fails closed, like a missing context value. Cycle-safe (the closure tolerates a malformed `A ⊑ B ⊑ A`).
- **Audit:** `spatial_status(&rule, &request) -> SpatialMatch` (`Satisfied`/`DefinitelyUnsatisfied`/`Unprovable`/`NotConstrained`) is the spatial twin of `purpose_status` — it runs the same subsumption-aware path `evaluate` does.

```rust,ignore
use sparq_policy::{evaluate, Request, Value, ODRL_SPATIAL};
// policy: distribute spatial isPartOf <country/EU>
let req = Request::new("http://www.w3.org/ns/odrl/2/distribute").on("urn:asset/x")
    .with(ODRL_SPATIAL, Value::Iri("urn:country/DEU".into()))
    .with_purpose_subsumption("urn:country/DEU", "urn:country/EU"); // DEU ⊑ EU
assert!(evaluate(&policy, &req).allow); // a sub-region of the named region grants
```

## Building the request

`Request::new(action_iri)` then chain `.on(target)`, `.by(party)`, `.with(left_operand_iri, Value)` for each context dimension (`dateTime`, `purpose`, `recipient`, `count`, `spatial`, …), `.with_purpose_subsumption(narrower, broader)` / `.with_purpose_taxonomy([(n, b), …])` per asserted subsumption edge (the DPV-purpose taxonomy / spatial region trees, above — one closure for both taxonomic dimensions), `.with_party_membership(party, collection)` / `.with_asset_membership(asset, collection)` (and bulk `…_memberships([…])`) per ODRL collection-membership edge (above), and `.discharge(duty_action_iri)` per discharged duty. `Value` is `Iri` | `Str` | `Num(f64)` | `DateTime(String)`. For purpose specifically, prefer the first-class `.for_purpose(Value)` (sugar over `.with(ODRL_PURPOSE, ..)`; read it back via `req.purpose()`); for the evaluation time prefer `.at(instant)` (sugar over `.with(ODRL_DATETIME, Value::DateTime(..))`; read it back via `req.request_time()`).

## The `odrl:use` umbrella + ODRL action hierarchy — [OPUS-4.8] sq-euhr3

`odrl:use` is the ODRL "umbrella" action, but it does **not** subsume *every* action — only those the [ODRL 2.2 vocabulary](https://www.w3.org/TR/odrl-vocab/) places `odrl:includedIn use` (39 actions transitively: `read`, `display`, `print`, `play`, `modify`, `delete`, `aggregate`, `reproduce`, …). The one disjoint top-level branch is the ownership-`transfer` subtree — `odrl:sell` and `odrl:give` are `includedIn odrl:transfer`, **not** `use` — so a `use` permission grants a `read`/`write` request but is **inactive** for a `sell`/`give`/`transfer` request.

- This is `Action::permits`: `use` permits a requested action iff it is `use` itself or *not* in the transfer subtree (the `is_transfer_subtree` set: `transfer`/`sell`/`give`); every other action — including non-vocabulary actions such as `odrl:write` — is treated as a use-subtree action (the broad "use this asset" reading). It matches the SolidLab reference evaluator (suite cases 007/008/009 Active vs 010/017 Inactive).
- A **non-`use`** action still matches only its own IRI exactly (`read` does not cover `write`).

## `odrl:PartyCollection` / `odrl:AssetCollection` membership — [OPUS-4.8] sq-k7itg

A rule whose `odrl:assignee` is an `odrl:PartyCollection` (or whose `odrl:target` is an `odrl:AssetCollection`) matches a request whose **party** (or **asset**) is a *member* of that collection — `member odrl:partOf collection` — not only a request whose party/asset IS the collection IRI. The membership relation is **request-supplied** evidence (read out of the state-of-the-world graph), declared with `.with_party_membership(party, collection)` / `.with_asset_membership(asset, collection)` (and the bulk `.with_party_memberships([…])` / `.with_asset_memberships([…])`).

- **Sound / fail-closed:** membership draws ONLY on the caller-supplied set — **never inferred**. With no membership edge, assignee/target matching is byte-for-byte the exact-IRI base case, so a non-member is denied and access is never widened. The edge must name the rule's exact collection (membership in a *different* collection does not match).
- **Reading the evidence back:** `req.party_collection_members(collection)` returns the parties the request evidenced as members of `collection` (sorted; empty for a plain party IRI or an un-evidenced collection). This is the read side a consumer needs when it must *persist* a collection-valued rule as a per-session-rechecked head rather than call the evaluator — see the ODRL→ACP bridge's `odrl:PartyCollection` head expansion below ([SONNET-4.6] sq-rf9uv), including why the DENY direction must not use it. It is **not a collection test**: the empty result is shared by a plain party IRI and by a collection this request happened to supply no edges for.
- **Collection identity, separate from membership:** `Policy::party_collections` is the set of IRIs the policy DOCUMENT identifies as an `odrl:PartyCollection` — every subject typed `a odrl:PartyCollection`, plus every object of an `odrl:partOf` edge the document states (minus anything explicitly typed `a odrl:AssetCollection`). `parse_policy_str`/`parse_policy` fill it; a hand-built `Policy` leaves it empty. This is what a consumer needs when it must fail CLOSED on collection-valued input: membership evidence is per-request and possibly partial, so "has ≥1 known member" is a lower bound on collection-ness, never a test for it ([SONNET-4.6] sq-rf9uv).

```rust,ignore
use sparq_policy::{evaluate, Request};
// policy: ex:partyCollection (a PartyCollection) may read ex:x.
let req = Request::new("http://www.w3.org/ns/odrl/2/read")
    .on("http://example.org/x").by("http://example.org/alice")
    .with_party_membership("http://example.org/alice", "http://example.org/partyCollection");
assert!(evaluate(&policy, &req).allow); // alice is a member → granted
```

## `odrl:LogicalConstraint` — `and` / `or` / `xone` compound constraints — [OPUS-4.8] sq-a0zef

A rule's `odrl:constraint` may point at a compound `odrl:LogicalConstraint` — a combinator (`odrl:and`/`odrl:or`/`odrl:xone`) over a *set* of operand constraints (each itself atomic **or** a nested `LogicalConstraint`, so an `or` of `and`s — a disjunction of time windows — is supported). The parser routes these into `Rule::logical_constraints`; the evaluator ANDs them with the atomic `constraints`.

- **`odrl:and`** — satisfied iff **every** operand holds. **`odrl:or`** — iff **at least one** holds. **`odrl:xone`** — iff **exactly one** holds.
- **Fail-closed (load-bearing):** each operand is three-valued (`Satisfied`/`DefinitelyUnsatisfied`/`Unprovable` for missing evidence). An `and`/`xone` is **never** claimed `Satisfied` on an `Unprovable` operand — a compound with an unprovable operand does not silently pass (an `and` with an unprovable operand and no definite-false is `Unprovable` → no grant; a `xone` with any unprovable operand is `Unprovable`). An empty/malformed compound (e.g. an `or` over only an unknown-operator operand) fails closed.

```rust,ignore
use sparq_policy::{evaluate, Request};
// policy: read permitted when (dateTime > 2024-01-01) AND (dateTime < 2024-12-31).
let req = Request::new("http://www.w3.org/ns/odrl/2/read").on("urn:asset/x")
    .at("2024-06-15T12:00:00Z");
assert!(evaluate(&policy, &req).allow); // inside the AND-window
// no time evidence → the AND is unprovable → fail-closed (no grant).
```

## `odrl:purpose` enforcement (faithful, fail-closed) — [OPUS-4.8] sq-q56r

A purpose constraint restricts a rule to a stated *purpose of use*. The request carries its purpose as **evidence** (`.for_purpose(Value)`), and it is gated through the **same** `evaluate` constraint path as every other dimension — so it is actually checked end-to-end (no claimed-but-unchecked enforcement), including through the opt-in bridge's real `accessible` / `query_as` path.

- **Match → grant; mismatch → deny; missing purpose → fail-closed.** A request stating **no** purpose is *unprovable*: a purpose-gated permission does **not** grant, and a purpose-gated prohibition is **not** withdrawn. "No purpose stated" is **never** treated as "any purpose allowed".
- **Match semantics (the boundary — do not over-claim):** **exact** IRI/string equality (`eq`/`isA`), or membership in the explicit `isPartOf` purpose *set* the constraint names, or `neq` (≠ the named one; still requires a stated purpose).
- **DPV / purpose-taxonomy subsumption — [OPUS-4.8] sq-z3ve.** Supply the request a purpose taxonomy with `.with_purpose_subsumption(narrower, broader)` (one `skos:broader`/`rdfs:subClassOf`/`dpv:isSubTypeOf` edge) or the bulk `.with_purpose_taxonomy([(n, b), …])`, and a purpose constraint naming `B` is also satisfied by a stated purpose `P` that is **transitively narrower** than `B` (`P ⊑ B`) — a `research` permission covers `clinical-research`, and a `neq research` carve-out *also* excludes that sub-purpose. **Sound, never over-claimed:** the `⊑` relation is the **caller-supplied transitive closure only** (never inferred from IRI string structure); with no taxonomy supplied it is byte-for-byte the exact-IRI base case, the broader-under-narrower direction never matches, and access is never widened on an unproven relation. The **same** closure also drives the `odrl:spatial` region tree (above) — one subsumption evidence channel for both taxonomic dimensions.
- **Audit:** `purpose_status(&rule, &request) -> PurposeMatch` reports exactly what the evaluator checks — `Satisfied` / `DefinitelyUnsatisfied` / `Unprovable` / `NotConstrained` (it runs the same subsumption-aware path `evaluate` does).
- Through the bridge, purpose stays **one-shot** (checked once at materialization; see the mapping table below) — it has no re-checked-condition analogue, so a changed purpose is re-evaluated on the next `refresh_odrl_grant`.

## `odrl:recipient` enforcement + `neq` / "everyone-except" — [OPUS-4.8] sq-5037

A `recipient` constraint restricts **who the data may be disclosed to**. The recipient-of-data is the requesting party, so a request that names a party (`.by(webid)`) but supplies **no** explicit `odrl:recipient` context is read as `recipient = party` — i.e. a `recipient` rule gates on *who is asking*, end-to-end through the same `evaluate` constraint path as every other dimension. An explicit `.with(ODRL_RECIPIENT, Value)` still takes precedence (the disclosure target need not be the authenticated principal in every deployment).

- **`recipient neq X` ("everyone EXCEPT X"):** grants/forbids for any recipient that is **not** `X`. A request whose recipient IS `X` is the carve-out (deny on a permission; the prohibition no longer carves *this* party out). **Missing identity (no `odrl:recipient` AND no party) is *unprovable* → fail-closed:** a `neq` permission does **not** grant to an unknown recipient, and a `neq` prohibition is **not** withdrawn.
- **`eq`/`isA`** = recipient IS the named party; **`isPartOf`/`isAnyOf`** = recipient ∈ static set; **`isNoneOf`** = recipient ∉ static set. Match is **exact** IRI/string equality by default — plus **party-collection membership** ([FABLE-5] sq-c2aze): when the request supplies `.with_party_membership(party, collection)` evidence, a recipient bound may name an `odrl:PartyCollection` the recipient is a *member* of (the same equality-or-membership lookup the `assignee` field gets), so `recipient isPartOf <PartyCollectionIRI>` is satisfied by a member — and the negative operators (`neq`/`isNoneOf`) also EXCLUDE a member of an excluded collection (the carve-out is not widened away). Membership draws ONLY on the caller-supplied evidence (never inferred); with no edge, recipient matching is byte-for-byte the flat base case.
- **Audit:** `recipient_status(&rule, &request) -> RecipientMatch` reports exactly what the evaluator checks — `Satisfied` / `DefinitelyUnsatisfied` / `Unprovable` / `NotConstrained` (the recipient dual of `purpose_status`).
- **Combined `recipient eq A AND neq B` (one rule):** the constraints are AND-combined — the recipient must BE `A` and must NOT BE `B`. Through the bridge this emits a single `ConditionalGrant` headed by `A` (positive) carrying an `exceptMatcher` carving out `B` (the per-head exception).
- Through the bridge, `recipient neq X` maps to an ACP **`noneOf`** exception, and `recipient isNoneOf <set>` to one exception matcher **per member** (the list-valued dual — [FABLE-5] sq-5fkpp; see the mapping table + the bridge note below) — re-checked per session.

## `odrl:dateTime` time-window enforcement (faithful, fail-closed) — [OPUS-4.8] sq-idnv

A `dateTime` constraint gates a permission/prohibition to a **time window** — e.g. `dateTime lteq T` (valid until `T`), `dateTime gteq T` (valid from `T`), or a two-sided window (`gteq lower` + `lteq upper`, AND-combined). The actual instant is the request's **evaluation time**, supplied as `xsd:dateTime` evidence via the first-class `Request::at(instant)` sugar (over `.with(ODRL_DATETIME, Value::DateTime(..))`); read it back via `req.request_time()`.

- **Semantics:** inside the window → grant (permission) / carve-out applies (prohibition); outside → deny / carve-out gone. Instants compare by magnitude (the `lt`/`lteq`/`gt`/`gteq`/`eq`/`neq` ordering). **Missing time → *unprovable* → fail-closed:** a time-gated permission does **not** grant on an unknown clock; a time-gated prohibition is **not** withdrawn (never silently read as "any time").
- **Audit:** `datetime_status(&rule, &request) -> DateTimeMatch` reports exactly what the evaluator checks — `Satisfied` / `DefinitelyUnsatisfied` / `Unprovable` / `NotConstrained` (the temporal dual of `purpose_status`/`recipient_status`).
- Through the bridge, an **inclusive** `dateTime` bound on an ALLOW (`lteq` → `auth:notAfter`, `gteq` → `auth:notBefore`) is now persisted onto the `ConditionalGrant` as a **live-clock window** ([OPUS-4.8] sq-0q7n): `AuthIndex::cond_applies` re-checks it against the session clock (`Session::now`, an `xsd:dateTime` string set via `Session::at(now)`) per request, so a lapsed window **denies immediately on the next request** — no `refresh_odrl_grant` pass needed. **Fail-closed:** a windowed grant evaluated with `now == None` never applies (mirrors the evaluator's fail-closed missing-clock); two `lteq`/two `gteq` bounds keep the *tightest* (intersection); a **strict** `lt`/`gt` bound stays **one-shot** (no inclusive `notBefore`/`notAfter` analogue — avoids an off-by-one). A dateTime window is mapped only on an allow; a time-windowed *deny* stays one-shot (a lapsed live deny would fail OPEN). The one-shot `materialize_odrl_permission` path is unchanged — it freezes the bound and relies on `refresh_odrl_grant` to retract. (Both the live-clock `window_admits` re-check and the `set_tighter` intersection-of-bounds compare by the **real UTC instant** via `sparq_policy::cmp_datetime` — the SAME offset-aware normalizer the evaluator uses, never raw lexical `str::cmp` — so a mixed-offset request like `now = 2026-06-16T11:00:00-02:00` (= `13:00Z`) is correctly DENIED against a `not_after = 2026-06-16T12:00:00Z` window, and an unparseable bound/clock fails closed. sq-0q7n.)

## `odrl:count` enforcement (stateful, opt-in `count-enforcement`) — [OPUS-4.8] sq-zi5w

`odrl:count` limits the **number of times** a permission may be exercised ("read at most 5 times"). Unlike `purpose`/`dateTime`/`recipient` (stateless — a single test against evidence the request carries), a count limit is **stateful**: it lives in a usage counter that persists *across* requests. Behind the off-by-default `count-enforcement` feature on `sparq-policy` (the default build is unchanged — it treats `odrl:count` as the stateless numeric comparison), `evaluate_and_exercise` runs the **real** `evaluate` decision and, on a grant, **atomically consumes** one unit of budget.

```rust,ignore
// cargo: sparq-policy with --features count-enforcement
use sparq_policy::{evaluate_and_exercise, InMemoryCounterStore, Request};
let store = InMemoryCounterStore::new();           // injectable UsageCounterStore
let req = Request::new("http://www.w3.org/ns/odrl/2/read")
    .on("urn:asset/x").by("https://alice.ex/me");  // policy: count lteq 3
for _ in 0..3 { assert!(evaluate_and_exercise(&policy, &req, &store).allow); }
assert!(!evaluate_and_exercise(&policy, &req, &store).allow); // 4th: limit reached → DENY
```

- **Semantics through the real path.** A grant requires the base `evaluate` to grant (the `odrl:count` constraint is stripped from *permissions* for that base check, since the stateless evaluator would otherwise deny for a missing count value; everything else — action/target/assignee, prohibitions, purpose, dateTime, recipient, duties — is checked unchanged) AND the count budget to have room. **First *N* exercises grant; the *(N+1)*th denies.** A *denied* request consumes **nothing**. `lteq N`/`eq N` = at most *N*; `lt N` = at most *N-1*; `gt`/`gteq`/`neq`/`isPartOf` express no ceiling (left to the stateless path).
- **Counter-store seam.** `UsageCounterStore` is an injectable trait with three shipped impls of increasing reach: `InMemoryCounterStore` (single-process, `Mutex`), `FileCounterStore` (cross-process on one host / sane NFS, an `O_EXCL` lockfile — [OPUS-4.8] sq-5z1q), and `BackendCounterStore<B: AtomicCounterBackend>` (cross-**host** over Redis/SQL — [OPUS-4.8] sq-u3yo, below). The budget key is `(rule_id, party, target)` (`CountKey`) — **per-assignee, per-asset, per-rule** (one party exhausting a limit never locks out another; the same rule on a different target counts separately). A partyless/targetless request shares the `""` bucket.
- **Atomicity / concurrency boundary (load-bearing).** The single mutating op is the **atomic** `try_consume` (the whole check-and-consume under one lock in the in-memory store) — deliberately **not** a `current()`+`increment()` pair, which would reintroduce a TOCTOU race (two concurrent exercises both reading `N-1`, both granting → over-grant). A concurrency test asserts exactly the limit is granted under 16 threads, never one more. **Multiple `odrl:count` constraints on one rule** all bind the *same* counter (`CountKey` is per `(rule, party, target)`, never per-constraint), so they are several *bounds on one budget*: the **tightest (minimum) bound governs**, and an exercise is a **single** atomic `try_consume` against that one effective limit — exactly one unit per exercise (never one-per-constraint), no read-then-consume gap. The multi-limit case is therefore as atomic as the single-limit case (a concurrency test asserts no over-grant under two constraints), closing the prior pre-check→consume window (sq-ea27). The in-memory store is atomic **in-process only**.
- **Cross-process counting — `FileCounterStore` ([OPUS-4.8] sq-5z1q).** For a multi-process deployment (two workers behind a load balancer, a CLI alongside a server) where the in-memory store would let *each process* exercise the full "at most N" budget, `FileCounterStore::new(dir)` persists all budgets in one file under `dir` and serializes the **whole** compare-and-increment with an OS-level lockfile (an `O_EXCL` `create_new` lock — the cross-process analogue of the in-memory `Mutex`). Every process that opens the same `dir` shares one budget; it is a **drop-in** for the same `UsageCounterStore` seam (same `try_consume` contract, identical `evaluate_and_exercise` semantics) and is **std-only — no new deps, no `unsafe`**. A cross-process test (this binary re-spawns 6 worker subprocesses hammering one shared file with 240 attempts against a budget of 60) asserts **exactly** the limit is granted across all processes, never an over-grant. **Boundary (honest):** `O_EXCL` create is atomic on a local filesystem (the deploy target) and on modern NFS, but not on old/misconfigured NFS — for a multi-*host* deployment over a questionable mount, prefer a Redis `INCR` / SQL `UPDATE … WHERE consumed < limit RETURNING` store against the same trait. Fail-closed throughout: a lock timeout (a crashed holder left the lockfile — never stolen), an I/O error, or a corrupt counter file all yield `ConsumeResult::Unavailable` → DENY.
- **Cross-host counting — `BackendCounterStore` over an `AtomicCounterBackend` ([OPUS-4.8] sq-u3yo).** For a multi-*host* deployment (workers on different machines) where even the file store cannot reliably share a budget, the bead's headline backends are **Redis** and **SQL**. `sparq-policy` is `publish = false`, **std-only, dependency-of-nothing**, so it does **not** bundle a `redis`/`tokio-postgres`/`rusqlite` client (which would also need a live service CI cannot provide). Instead it ships the **seam**: `AtomicCounterBackend` — the ONE primitive an operator implements: `checked_increment(key, limit) -> Incremented(new) | Exhausted | Err`, which **is** a Redis `INCR`-behind-a-Lua-cap and **is** a SQL `UPDATE … SET consumed = consumed + 1 WHERE key = ? AND consumed < ? RETURNING consumed` (the trait docs carry both statements verbatim). `BackendCounterStore::new(backend)` adapts any such backend into a full `UsageCounterStore`: it maps a `CountKey` to one collision-free (injective) backend key, calls the backend's single atomic op once (**no** read-then-write, so no TOCTOU window is added on top of the backend's atomicity), and translates the outcome — a `BackendError` → `ConsumeResult::Unavailable` (fail-closed). The adapter logic (the only part that lives in this crate) is **tested against an in-crate reference backend** reproducing the exact Redis/SQL atomic semantics — drop-in through the real `evaluate_and_exercise` path, an 8-thread no-over-grant oracle, injective key-encoding, and fail-closed-on-backend-error — so the wiring is proven without a live service. The operator writes ~one method and inherits a correct, fail-closed distributed store.
- **Fail-closed.** `ConsumeResult::Unavailable` (store outage / poisoned lock / backend error) or a malformed limit → **DENY** — never silently "unlimited". `count_status(&rule, &request, &store) -> CountStatus` is the side-effect-free audit surface (`Satisfied{consumed,limit}` / `DefinitelyUnsatisfied{..}` / `Unprovable` / `NotConstrained`), the count dual of `purpose_status`.
- **Wired THROUGH the bridge — a bridged grant self-retracts on exhaustion ([OPUS-4.8] sq-58mh).** Behind `sparq-solid`'s `count-enforcement` feature (which implies `odrl-bridge` + `sparq-policy/count-enforcement`), `PodStore::materialize_odrl_permission_counted(&policy, &request, &store)` routes the bridge's allow/deny through `evaluate_and_exercise` (atomic consume) and materializes the equivalent `principal auth:<mode> graph` allow into `<urn:sparq:auth>`. ACP is stateless (no per-session usage counter), so the count cannot be a re-checked ACP *condition* the way `recipient`/`dateTime` are; instead it is enforced through the EXISTING refresh/retraction ledger (sq-dpk4): the count is consumed once at exercise and re-checked **read-only** (never re-consumed) on `refresh_odrl_grants()` via `count_status`, so once the budget is exhausted the tracked `BridgeKind::PermissionCounted` entry emits nothing → the bridged grant is **retracted** → access is GONE through the real `accessible`/`query_as` path. Fail-closed: a base Deny / exhausted / unprovable (store outage) / store-missing re-check retracts (never re-grants beyond a budget we cannot account for); refresh never itself burns a unit. The store is shared via `Arc<dyn UsageCounterStore + Send + Sync>` so the same budgets back both exercise-time consumption and refresh-time re-checks.

## Static conflict + containment analysis (request-free) — [OPUS-4.8] sq-zabv

## Decision reports (opt-in `decision-report`) — [GPT-5.6] sq-mu4au

Enable `sparq-policy`'s default-off `decision-report` feature to aggregate already-computed
decisions without re-running or changing authorization. Supply the request action beside each
`Decision`; the report sorts action buckets and emits byte-stable compact JSON.

```rust,ignore
use sparq_policy::report::DecisionReport;

let decisions = [(request.action.as_str(), &decision)];
let report = DecisionReport::summarize(decisions);
assert_eq!(report.permitted + report.denied, report.total);
let stable_json = report.to_json();
```

`conflicts` counts denials whose completed decision contains `evaluate`'s canonical
matching-prohibition audit explanation. The helper is read-only and never makes or re-derives
an allow/deny decision.

Where `evaluate` answers *"may THIS request go through?"*, two **request-free** functions answer questions about the policies themselves (the query-containment comparison semantics, [arXiv 2509.05139 §comparison](https://arxiv.org/html/2509.05139v1)). Both are always compiled (no feature, no deps) and both are **sound / fail-closed — never over-claimed**.

```rust,ignore
use sparq_policy::{contains, detect_conflicts, Containment, Overlap};
// Lint a policy for permission/prohibition conflicts.
for c in detect_conflicts(&policy) {
    // c.permission_id is (wholly, if c.overlap == Overlap::Certain) carved out by c.prohibition_id
}
// Does the provider's offer permit everything the requester asks?
match contains(&provider_policy, &request_policy) {
    Containment::Contains => { /* every ask is covered */ }
    Containment::NotContained => { /* a witness ask the offer denies */ }
    Containment::Unknown => { /* undecidable under the conservative comparison */ }
}
```

- **`detect_conflicts(&policy) -> Vec<Conflict>`** — every permission/prohibition pair whose request footprints overlap (a prohibition carves the permission out — deny-overrides). `Conflict { permission_id, prohibition_id, overlap, action, target }`. `overlap` is `Overlap::Certain` **only** when the structural attributes (action / target / assignee) prove an overlap AND the prohibition adds **no** constraint the permission lacks (so the carve-out covers the *whole* permission); otherwise `Overlap::Possible` (the rules *might* overlap but we cannot prove they always do). A pair that **provably never** overlaps (disjoint concrete action / target / assignee) is omitted. Conflict is strictly across the permission/prohibition divide — two permissions never conflict.
- **`contains(outer, inner) -> Containment`** — does `outer` permit everything `inner` permits (refinement / requester-vs-provider containment)? `Containment::Contains` **only** when every `inner` permission is *provably* subsumed by some `outer` permission AND no `outer` prohibition could carve into it; `Containment::NotContained` when an `inner` permission *provably* grants a request `outer` denies (disjoint concrete target/action, or `inner` leaves a dimension `outer` restricts wide open); `Containment::Unknown` otherwise. An `inner` with no permissions is contained vacuously.
- **`conflict_admissibility(&policy) -> Result<(), String>`** ([OPUS-4.8] sq-ihqbl) — the fail-closed guard the bridge runs BEFORE materialising anything, deciding whether a policy's declared `odrl:conflict` conflict-resolution strategy (`Policy.conflict: Option<ConflictStrategy>`, parsed from `odrl:conflict`) is one the engine can faithfully honour. The bridge implements **exactly one** strategy — `odrl:prohibit` (deny-overrides), which is also the operative default when `odrl:conflict` is unset. Any other declared strategy returns `Err(reason)` (a **loud** refusal — see the bridge section) rather than being silently coerced into deny-overrides (an authorization-correctness hazard): `ConflictStrategy::Perm` (`odrl:perm`, permissions override prohibitions — unrepresentable by `∪ allow ∖ ∪ deny`) is **always** refused; `ConflictStrategy::Invalid` (`odrl:invalid`, a conflicting policy is void as a whole) is refused **iff** `detect_conflicts` finds a conflict (else admissible — nothing to void); `ConflictStrategy::Unknown(iri)` (an unrecognised `odrl:conflict` value) is **always** refused. **Honest boundary:** an *unset* `odrl:conflict` defaults to `prohibit`, NOT the ODRL 2.2 spec default of `invalid` — fully honouring `invalid` for every undeclared conflicting policy would refuse the bridge's core deny-overrides use case. This divergence is **deliberate and documented** (decided on issue [#1375](https://github.com/sparq-org/sparq/issues/1375), sq-zjtu1): deny-overrides is fail-closed and never authorises what a prohibition forbids; see the crate README's ODRL conformance note and the odrl-policy-bridge paper's Limitation #1.
- **Soundness boundary (honest).** Constraint satisfiability / query containment is undecidable in the general ODRL constraint language. This module decides only what it can *prove* from rule structure plus a conservative per-dimension constraint comparison (identical constraints; `eq v` admitted by an `outer` bound; a *tighter* same-direction order bound — `lt`/`lteq`, `gt`/`gteq` — implying a looser one). Everything else degrades to `Possible` / `Unknown` — it **never** reports `Certain` / `Contains` it cannot prove (that is the fail-OPEN failure mode: claiming an ask is covered when it is not). It also does not (yet) prove `NotContained` from a *looser* inner numeric bound reaching above a tighter outer one — that case honestly returns `Unknown`. DPV/`isPartOf` set-subset refinement is a deferred bead.

## Security-property ODRL profile — `secx:requires…` leftOperands (opt-in `secprop-leftoperands`) — [OPUS-4.8] sq-uor3g

A privacy/security preference — *"admit only a proof method whose security posture meets THIS bar"* — is expressed as an ODRL constraint over a **custom `secx:requires…` leftOperand** (`research/security-properties-ontology-design.md` §4.3.1). The load-bearing ODRL nuance: a `rightOperand` natively accepts any IRI, but a **custom `leftOperand` MUST be declared in a published [`odrl:Profile`](https://www.w3.org/TR/odrl-model/#profile)** and a policy using it must assert `odrl:profile <profileIRI>` — a conforming ODRL processor MAY reject an undeclared leftOperand. Phase 4 (this feature) ships that **profile document** (`crates/sparq-policy/ontologies/odrl-secprop-profile.ttl`) and makes the leftOperands **first-class** in the Rust model.

Behind the off-by-default `secprop-leftoperands` cargo feature (the default build already *parses* + *evaluates* such a constraint as an opaque custom leftOperand — this feature adds **recognition metadata**, not new eval semantics; the stateless `evaluate` path is unchanged). The `secprop` module gives the leftOperand IRIs as constants, the published profile IRI, a recogniser, and the leftOperand → security-property-dimension map (`secx:overDimension`):

```rust,ignore
// cargo: sparq-policy with --features secprop-leftoperands
use sparq_policy::secprop::{is_secprop_left_operand, over_dimension, PROFILE_IRI, REQUIRES_ASSURANCE};

assert!(is_secprop_left_operand(REQUIRES_ASSURANCE));
// the single fact the admissibility reduction reads: leftOperand -> dimension
assert_eq!(over_dimension(REQUIRES_ASSURANCE), Some("https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel"));
// a policy using a secx: leftOperand must assert `odrl:profile <PROFILE_IRI>`
let _ = PROFILE_IRI; // https://sparq.dev/ns/odrl-secprop-profile#
```

- **One leftOperand per requireable dimension** (`requiresUnlinkabilityScope`, `requiresPostQuantumForgery`, `requiresZeroKnowledge`, `requiresSoundness`, `requiresSelectiveDisclosure`, `requiresSingleUse`, `requiresAssurance`, …); the full set is `secprop::SECPROP_LEFT_OPERANDS`. Each is **sugar** for the generic primitive "a constraint over dimension X" — the one `secx:overDimension` fact per leftOperand is what the admissibility rule reads, so one rule covers all (design §4.5).
- **This is the bridge VOCABULARY only.** It NAMES the requireable dimensions; it does **not** perform the admissibility reduction (that is the N3 ruleset, Phase 2 `sq-ufsi9`, over the per-method annotation graph, Phase 3 `sq-bevd3` in `sparq-zk`), and it asserts **NO** security property of any method. Whether a sparq method actually *has* a property is recorded — with its epistemic basis — elsewhere; **sparq's whole ZK estate is research-grade and externally UNAUDITED** (bead `sq-qhy4`), and `sparq-mpc` is semi-honest-only. The conservative `requiresAssurance gteq secx:Proven` gate mechanically removes every unaudited method — this is how `sq-qhy4` enters the admissibility data flow, not merely the prose.

### Proof-discharge obligations — the ODRL half of the ZK↔ODRL envelope (same feature) — [SONNET-4.6] sq-yh427

`discharge_requirements(&Policy)` answers *"before I can decide this request, what would a presented proof have to establish?"* — it projects a parsed policy to the `secx:requires…` constraints it carries, each resolved to its dimension, operator and required level, in rule order (permissions before prohibitions). Rules that mention no `secx:` property are omitted, so an empty result means no presentation is needed.

```rust,ignore
// cargo: sparq-policy with --features secprop-leftoperands
use sparq_policy::secprop::{discharge_requirements, Deontic, DischargeExpr};

for r in discharge_requirements(&policy) {
    // r.rule / r.deontic / r.requirement — WALK the tree; the combinator decides.
    match &r.requirement {
        DischargeExpr::Any(alts) => { /* establishing ANY ONE of `alts` suffices */ }
        DischargeExpr::ExactlyOne(alts) => { /* establishing TWO of `alts` FAILS */ }
        DischargeExpr::All(conjuncts) => { /* every one must hold */ }
        DischargeExpr::Atomic(o) => { let _ = (o.dimension, &o.required); }
        DischargeExpr::Other(c) => { let _ = (&c.left, c.operator, &c.right); }
    }
}
```

- **The `odrl:` combinator is preserved, and is load-bearing.** The result is a `DischargeExpr` **tree per rule**, not a flat list: under `odrl:or` establishing *either* branch suffices, and under `odrl:xone` establishing more than one is a *failure*. `DischargeExpr::obligations()` gives a flat inventory of the leaves, but it is explicitly **NON-decisional** — `and`, `or` and `xone` over the same leaves flatten identically, so reading it as the requirement overstates what a presentation must prove.
- **Non-`secx:` branches survive WHOLE as `DischargeExpr::Other(Constraint)`** rather than being pruned or thinned: dropping a core-ODRL operand out of an `odrl:or` would turn an alternative into a mandate, and keeping only its `leftOperand` would leave `recipient eq Bob` and `recipient eq Carol` indistinguishable. The full `(leftOperand, operator, rightOperand)` triple is carried, so the branch is decidable from the tree alone — which is what makes `odrl:xone` readable, since there the *count* of holding branches, not the set of dimensions mentioned, is the verdict.
- **`Deontic` is load-bearing.** An obligation on a *prohibition* is not a bar to clear: a prohibition fires when its whole constraint expression holds, and deny-overrides. A host must not read the two the same way — and one leaf alone does not fire a multi-constraint prohibition.
- **Duty constraints are excluded** — the single-node base case checks a `Duty` by *action* discharge and never evaluates the duty's own constraints, so listing them would misdescribe what `evaluate` will check.
- **It reports; it does not discharge.** Extraction verifies nothing, orders no levels (the `secx:atLeast` partial order and the admissibility reduction live in `sparq-trust`, `sq-ufsi9`), and leaves the stateless `evaluate` path unchanged — a `secx:` constraint with no supplied evidence stays fail-closed unsatisfied.
- **The wire half of the envelope is deferred, deliberately.** A VC 2.0 Verifiable Presentation carrying a Data-Integrity proof under a `sparql-zk`-style cryptosuite is **not** implemented: no such cryptosuite exists in this workspace (`sparq-vc` implements `eddsa-rdfc-2022` only), its identifier and claim shape are still under cross-agent design with the Solid-server sibling (`#890`(b)), and sparq's ZK estate has **no** external accredited-cryptographer sign-off (`sq-qhy4`). <!-- privacy-claims-allow: NEGATIVE — records that the ZK presentation envelope is unimplemented and the estate unaudited; sq-qhy4 -->

## Bridge to WAC/ACP enforcement (opt-in `odrl-bridge`) — [OPUS-4.8] sq-h3uk

`sparq-solid` can **materialize** a matched ODRL permission into its `<urn:sparq:auth>` AUTH_GRAPH so the existing graph-level WAC/ACP enforcement applies it — **no new enforcement engine**. Behind the off-by-default `odrl-bridge` cargo feature on `sparq-solid` (it pulls in `sparq-policy` only when enabled; the default solid build stays ODRL-free). This is the **single-node** bridge of epic sq-3183, **research-track**, NOT the (gated) federated/ZK-disclosure path.

```rust,ignore
// cargo: sparq-solid with --features odrl-bridge
use sparq_solid::{PodStore, Session, Mode};
use sparq_policy::Request;
let req = Request::new("http://www.w3.org/ns/odrl/2/read")
    .on("https://pod.ex/notes/n1").by("https://alice.ex/card#me");
// On a definite Permit, appends `alice auth:read n1` to the auth view, then reindexes.
let out = store.materialize_odrl_permission(&policy, &req);
assert!(out.granted);
// …now honoured by the unchanged enforcement path:
assert!(!store.accessible(&Session { agent: Some("https://alice.ex/card#me"), client: None }, Mode::Read).is_empty());
```

**Action → mode** (the ODRL *request* action; conservative — narrowest mode only): `read`/`display`/`present`/`print`/`play` → `acl:Read`; `append` → `acl:Append`; `modify`/`delete`/`write` → `acl:Write`; **anything else (incl. the `odrl:use` umbrella) is unmapped → no grant**. `use` is left unmapped because it subsumes a whole subtree of actions (mapping it would have to pick the widest mode) — request `odrl:read` explicitly; a `use` permission still grants that concrete request (provided the request action is in the `use` subtree — not `sell`/`give`/`transfer`).

**SPARQL-query action contract ([SONNET-4.6] sq-lrtc3.2): use `odrl:read`.** sparq does not mint a
profile-specific SPARQL-query action IRI: executing a query observes graph content and maps exactly
to `Mode::Read`, while a private action would add interoperability cost without a distinct
enforcement capability. An `odrl:use` **request** does not transitively become query access and
materializes no grant, so `query_as` sees nothing from it. The hierarchy still applies while
evaluating a policy (`odrl:use` permission can cover a concrete `odrl:read` request), but the bridge
maps the concrete request action rather than guessing which descendant of the broad `use` umbrella
was intended. Always construct query requests with the concrete read action:
`Request::new("http://www.w3.org/ns/odrl/2/read")`.

**Fail-closed:** a grant is materialized only on a *definite Permit* AND a *mappable action* AND a *concrete party (WebID) + target graph*. A Deny, unsatisfied constraint, undischarged duty, unmapped action, or partyless/targetless request materializes **nothing**.

**Interactive surface** — [FABLE-5] sq-ixc3.15: the desktop GUI's **Policies (ODRL) tool** (`gui/`, opt-in `odrl` cargo feature on `gui/src-tauri`) drives exactly this one-shot bridge path end-to-end: author/validate a policy, evaluate a request, and preview the same SPARQL query per requester through `query_json_as` next to the ungated rows (`gui/src-tauri/src/odrl.rs`).

### Prohibitions → explicit `auth:deny*` (deny-overrides) — [OPUS-4.8] sq-w693

A matched ODRL **Prohibition** is materialized as the dual triple — `principal auth:deny<Mode> target` — via `materialize_odrl_prohibition` (or `materialize_odrl_policy`, which does both sides at once). The **same** action→mode mapping picks the mode, so the deny predicate is `auth:denyRead` / `auth:denyWrite` / `auth:denyAppend` / `auth:denyControl`.

```rust,ignore
// Prohibition side: appends `alice auth:denyWrite n1`, then reindexes.
let dreq = Request::new("http://www.w3.org/ns/odrl/2/modify")
    .on("https://pod.ex/notes/n1").by("https://alice.ex/card#me");
let out = store.materialize_odrl_prohibition(&policy, &dreq);
assert!(out.prohibited);
// Both sides of a policy at once (permit grant + matched-prohibition deny):
let out = store.materialize_odrl_policy(&policy, &req);
```

**Deny-overrides:** the deny is honoured by the **existing, unchanged** enforcement — the session layer already computes `∪ allow ∖ ∪ deny` (`AuthIndex::accessible`) and `Mode::from_pred` already parses `auth:deny*`, so a materialized deny **beats any allow grant** for the same principal+target+mode. No new enforcement engine; the bridge only emits the triple. (Within one policy the ODRL evaluator *also* applies deny-overrides upstream, so the permit allow triple is never even emitted when a prohibition carves the request out.)

**Loud refusal of an unimplementable conflict strategy** ([OPUS-4.8] sq-ihqbl): the bridge implements only `odrl:conflict odrl:prohibit` (deny-overrides). Every `materialize_*` entry point first runs `sparq_policy::conflict_admissibility(policy)`; a policy declaring a strategy the bridge cannot honour (`odrl:perm`, `odrl:invalid` **with** a detected conflict, or an unknown `odrl:conflict` IRI) materializes **NOTHING** — neither grant nor deny — and the returned `BridgeOutcome` carries `refused = true` plus a `REFUSED (odrl:conflict): …` reason. This is fail-closed and *distinct from a plain deny* (`refused` vs `granted == false`): silently coercing `odrl:perm` into deny-overrides would mis-apply the author's intent. An unset `odrl:conflict` is treated as `prohibit` and is **not** refused (the common case).

**Fail-closed (deny):** a deny is materialized only when a prohibition **matches** the request (decided by `sparq_policy::matched_prohibition` — the evaluator's own conflict test, *not* `Decision.allow == false`, which conflates a carve-out with a plain no-permission deny) AND the action is mappable AND the party+target are concrete. An unmatched / unmapped / partyless / targetless prohibition materializes **nothing** — and an unmappable carve-out is *reported* in `reasons`, never silently dropped (dropping a deny would widen access).

### Persisting a constraint as a re-checked ACP condition (`materialize_odrl_permission_conditional`) — [OPUS-4.8] sq-hiz4

The one-shot `materialize_odrl_permission` *freezes* every constraint into a single allow scoped to the supplied request party. `materialize_odrl_permission_conditional` instead persists a **faithfully-mappable** constraint as an ACP `auth:ConditionalGrant` (the same `noneOf` machinery the ACP materializer emits), so the granted agent is **re-checked per session** through the unchanged enforcement path — not re-running the ODRL evaluator. A constraint with **no** faithful ACP analogue keeps the one-shot behaviour (checked once, frozen).

```rust,ignore
// The recipient constraint names carol — NOT whoever materializes the grant.
let req = Request::new("http://www.w3.org/ns/odrl/2/read")
    .on("https://pod.ex/notes/n1").by("https://alice.ex/card#me");
store.materialize_odrl_permission_conditional(&policy, &req); // policy: recipient eq carol
// Re-checked per session: carol is granted; alice (the materializer) is NOT.
```

**Constraint → ACP condition mapping table** (fail-closed: map ONLY when the ACP analogue is the *same or stricter*):

| ODRL constraint | Operator | ACP analogue | Faithful? | Behaviour |
|---|---|---|---|---|
| `odrl:recipient` / `odrl:assignee` | `eq` / `isA` | `auth:agent <webid>` on a `ConditionalGrant` (agent matcher) | ✅ recipient-of-data IS the session agent | **persisted, re-checked per session** |
| `odrl:recipient` / `odrl:assignee` | `isPartOf` / `isAnyOf` (static set) | one `auth:agent` head per member (OR) | ✅ set membership = agent ∈ set (the evaluator matches both operators as the same flat lexical set) | **persisted** (one grant/member; `isAnyOf` [FABLE-5] sq-5fkpp) |
| `odrl:recipient` / `odrl:assignee` | `neq` ("everyone EXCEPT X") / `isNoneOf` (everyone except a static set) | `auth:Public` `ConditionalGrant` + one `auth:exceptMatcher` per excluded member (ACP `noneOf`) | ✅ everyone-except is exactly the ACP `noneOf` shape | **persisted, re-checked per session** ([OPUS-4.8] sq-5037; list-valued `isNoneOf` [FABLE-5] sq-5fkpp — a numeric/dateTime operand or an EMPTY member set stays one-shot, fail-closed) |
| `odrl:recipient` / `odrl:assignee` | order (`lt`/`gt`/…) | (none — not meaningful on a recipient) | ❌ | **one-shot** (frozen) |
| `odrl:purpose` | any (incl. `isPartOf` DPV hierarchy) | (none — a client app ≠ a purpose-of-use) | ❌ ACP session carries no purpose dimension; client-matcher would over-grant | **one-shot** (frozen) |
| `odrl:spatial` | any (incl. `isPartOf` region hierarchy) | (none — ACP session carries no region) | ❌ no spatial dimension to re-check | **one-shot** (frozen) ([OPUS-4.8] sq-wukl) |
| `odrl:dateTime` (ALLOW) | `lteq` ("until T") / `gteq` ("from T") | `auth:notAfter` / `auth:notBefore` window on the `ConditionalGrant`, re-checked vs `Session::now` | ✅ the window IS the request instant — a live-clock dimension | **persisted, re-checked per request** ([OPUS-4.8] sq-0q7n) |
| `odrl:dateTime` | strict (`lt` / `gt`) | (none — no inclusive `notBefore`/`notAfter` analogue) | ❌ inclusive/exclusive off-by-one | **one-shot** (frozen) |
| `odrl:dateTime` (DENY) | any | (none as a live window — a lapsed live deny would fail OPEN) | ❌ unsafe to lapse a deny | **one-shot** (frozen) |
| `odrl:count` | any | (none as a per-session *condition* — ACP is stateless; no usage counter) | ❌ as a re-checked condition | NOT a `ConditionalGrant` (no per-session counter to re-check), but **stateful through the bridge** via the refresh ledger: `PodStore::materialize_odrl_permission_counted` consumes atomically at exercise and `refresh_odrl_grants()` re-checks the budget read-only, so the bridged grant **self-retracts on exhaustion** ([OPUS-4.8] sq-58mh; `count-enforcement` feature — see the §`odrl:count` enforcement section above). In `materialize_odrl_permission_conditional` (the condition path) a `count` constraint still forces the rule **one-shot** (frozen). |
| `odrl:assignee` / `odrl:recipient` naming an `odrl:PartyCollection` (ALLOW head) | `eq` / `isA` / set, or the bare `odrl:assignee` property | one `auth:agent` head per **request-evidenced member**, plus the collection IRI itself | ✅ exactly the party set the evaluator admits under the supplied `odrl:partOf` evidence | **persisted, re-checked per session** ([SONNET-4.6] sq-rf9uv — see below) |
| `odrl:assignee` / `odrl:recipient` naming an `odrl:PartyCollection` (DENY head, or an ALLOW's `neq`/`isNoneOf` carve-out) | any | (none — a frozen head cannot re-check membership) | ❌ members would escape the restriction (fail-OPEN) | **one-shot** (frozen) — regardless of how many members were evidenced, including **zero** ([SONNET-4.6] sq-rf9uv) |
| `odrl:assignee` / `odrl:recipient` naming an `odrl:PartyCollection` (ALLOW head) with **no** `odrl:partOf` evidence | any | (none — nothing to expand to; a bare collection head binds no member session) | ❌ the frozen grant would bind nobody | **one-shot** (frozen) ([SONNET-4.6] sq-rf9uv) |
| any unrecognised left-operand | any | (none) | ❌ | **one-shot** (frozen) |
| *no constraint* | — | `auth:agent auth:Public` (action/target/duties already held) | ✅ | persisted (public) |

**The `neq`/`isNoneOf` / "everyone-except" → `noneOf` shape ([OPUS-4.8] sq-5037; list-valued `isNoneOf` [FABLE-5] sq-5fkpp):** a `recipient neq X` (or `recipient isNoneOf <set>`) rule emits a `ConditionalGrant` whose head is the positive recipient set (or `auth:Public` if there is none) plus one `auth:exceptMatcher <m>` per excluded `X`; the matcher `<m>` carries the accept-set facts the session layer reads (`solidx:acceptsAgentP <X>` + `solidx:acceptsClientP auth:AnyClient`). `AuthIndex` then suppresses the grant for any session the matcher accepts — i.e. for `X` under any client — so everyone keeps the grant **except** `X`. This is byte-for-byte the shape the ACP `noneOf` rules (`rules/acp-c.n3`) emit, re-checked by the same `cond_applies` code path. A `neq` recipient inside the reserved pair encoding **cannot** become an enforceable matcher (it would impersonate a minted pair principal), so rather than emit an exception that silently fails to bite (which would re-admit `X`), the whole rule falls back to the one-shot path — **fail-closed: never widen to a public everyone-except grant on an unenforceable exclusion** (a reserved-encoded member anywhere in an `isNoneOf` set sinks the whole rule the same way, as does a numeric/dateTime operand — which the evaluator never satisfies — or the degenerate EMPTY set).

**`odrl:PartyCollection` heads ([SONNET-4.6] sq-rf9uv):** the evaluator matches a collection-valued `odrl:assignee`/`odrl:recipient` by *identity-OR-membership*, but an ACP `auth:agent` head is matched against the session agent by **identity alone** — a session carries no membership evidence. So a lone collection-IRI head matches no member: as an ALLOW head the grant is dead (an over-restriction), and as a DENY head or `noneOf` matcher the restriction binds nobody (a widening). Exactly ONE shape survives on the frozen path: a **positive ALLOW head with membership evidence**, expanded to `{collection} ∪ members(collection)` from the request's own `.with_party_membership(s)` edges — the exact party set the evaluator would admit under that evidence, never wider. Every other collection-valued head falls back to the one-shot path, whose evaluator does the real identity-or-membership check per request.

The **DENY** dual, and an ALLOW's `neq`/`isNoneOf` **carve-out**, deliberately do NOT expand and stay **one-shot**: the evidence is caller-supplied and possibly partial, and the head is frozen at materialization, so a member the request did not evidence would *escape* the restriction (fail-OPEN — the widening the bridge exists to prevent). The one-shot path's evaluator does the real identity-or-membership check instead, materializing a frozen deny/grant scoped to the requesting party. The expansion is sound in the ALLOW direction only, because there a missing member merely under-grants.

**Where collection identity comes from — and why zero members is not a hole.** The one-shot routing keys off collection IDENTITY, not off the member count, so a collection the request supplied **no** `odrl:partOf` edges for is still recognised and still leaves the frozen path. Two sources are unioned: `Policy::party_collections` (retained by the parser from the policy document's own `a odrl:PartyCollection` declarations and `odrl:partOf` edges) and a non-empty `Request::party_collection_members` (for a caller reading membership out of the state-of-the-world graph instead). A head neither source identifies is treated as a plain party IRI — in ODRL 2.2 an IRI is a party collection because it is *declared* one, so a policy document asserting neither the type nor a membership edge, materialized against a request supplying no `odrl:partOf` evidence either, has given nothing in reach of the bridge a reason to read that head as a collection. (Hand-building a `Policy` rather than parsing one leaves `party_collections` empty; populate it if your rule heads are collections.) The closed behaviour is pinned by the bridge tests `zero_edge_collection_prohibition_emits_no_conditional_deny`, `zero_edge_collection_carve_out_emits_no_conditional_grant`, `collection_assignee_without_evidence_emits_no_conditional_grant` and `policy_stated_membership_edge_identifies_the_collection`.

**Fail-safe on mixed constraints:** a persisted condition is emitted ONLY when **every** constraint on the rule maps faithfully — but a faithfully-mapped recipient AND a faithfully-mapped inclusive `dateTime` window now COMPOSE on one grant (the recipient head carries the `auth:notBefore`/`auth:notAfter` window — [OPUS-4.8] sq-0q7n). A rule mixing a mappable constraint with an unmappable `purpose`/`count`/strict `dateTime` still falls back **entirely** to the one-shot path — persisting only the mappable part would silently drop the other bound and over-grant. Recipient IRIs inside the reserved pair encoding (`urn:sparq:` / `&client=`) are dropped from the grant head (anti-impersonation). The two ODRL "any recipient" sentinels fold onto auth principals: `odrl:All`/`odrl:Group` → `auth:Public`, `odrl:AllConnections` → `auth:Authenticated`.

**Why recipient/assignee and an inclusive dateTime map:** the ACP session re-check carries `(agent, client, issuer)` plus the request clock `Session::now`. The recipient-of-data is precisely the session **agent**, and an `odrl:dateTime` window *is* the request instant — both re-check with identical semantics. Purpose and count have **no** stateless per-session analogue, so persisting them would require either freezing the check (= the one-shot path, already correct) or a looser approximation that could over-grant — rejected. A dateTime window is mapped only on an ALLOW (a lapsed deny would fail open).

### Constraint-conditional DENY (`materialize_odrl_prohibition_conditional`) — [OPUS-4.8] sq-4r70

The dual of the conditional grant: a matched ODRL **prohibition** whose recipient/assignee constraints map faithfully is persisted as a re-checked `auth:ConditionalGrant` with **`auth:effect auth:Deny`** (rather than a frozen one-shot `auth:deny*`). The carve-out is re-verified per session through the SAME `accessible`/`query_as` path, and **composes with deny-overrides**: the session layer subtracts `∪ deny` from `∪ allow`, so a conditional deny that applies to a session removes the target from its accessible set — beating any allow for the same principal+target+mode.

- A prohibition `recipient eq carol` → a deny condition headed by carol (only carol's sessions are denied); `recipient neq bob` → a deny on **everyone EXCEPT bob** (an `exceptMatcher` carving bob back IN to access). Same mapping table as the allow path (recipient/assignee `eq`/`isA`/`isPartOf`/`isAnyOf`/`neq`/`isNoneOf` are faithful; `purpose`/`dateTime`/`count` are unmappable).
- **Fail-safe fallback:** a prohibition carrying an unmappable constraint (`purpose`/`dateTime`/`count`) falls back to the one-shot `materialize_odrl_prohibition` (frozen, materialized iff the prohibition currently matches) so the bound is still enforced — a persisted deny condition is emitted ONLY when every constraint maps faithfully. A reserved-encoded recipient cannot become a matcher, so the rule falls back to one-shot rather than emit a deny that silently fails to bite (which would FAIL OPEN — a dropped deny widens access).
- **Refresh.** Tracked as `BridgeKind::ProhibitionConditional`. On refresh the recipient carve-out is re-checked per session, so the deny is re-emitted while the prohibition still structurally names the request; a withdrawn prohibition re-emits nothing → the deny condition is **retracted** → access restored. A one-shot fallback uses the asymmetric deny-retraction rule below (re-emit on `Ambiguous`, retract only on a definite `Withdrawn`).

### Refresh / REVOCATION of bridged grants on policy change — [OPUS-4.8] sq-dpk4

The `materialize_odrl_*` calls only ever **append**. When the underlying ODRL policy changes — a permission is **withdrawn**, a **time window lapses**, or a re-evaluation now **Denies** — the previously-materialized grant would otherwise stay in the auth view, so access that should be gone persists (the sq-h3uk/#280 correctness gap). And a wholesale static WAC/ACP re-materialization rebuilds `<urn:sparq:auth>` and would drop every bridged grant. Both are reconciled by a **bridge ledger** + a refresh entry point.

- **Provenance.** Every auth triple the bridge writes into `<urn:sparq:auth>` is mirrored verbatim into a separate reserved graph `<urn:sparq:auth-bridged>` (`AUTH_BRIDGED_GRAPH`). A triple is **bridged** iff it appears there, **static** otherwise — so bridged and static grants are structurally distinguishable without inspecting predicate shape, and the enforcement reader (`AuthIndex`) is unchanged (it still reads `<urn:sparq:auth>`). The provenance graph is in the reserved `urn:sparq:` space, so a loaded dataset cannot forge it.
- **Refresh / retract.** `PodStore::refresh_odrl_grant(&new_policy, &new_request, kind)` updates the tracked grant slot `(kind, target, party)` with the new policy / request context, then rebuilds the view as `static_baseline ∪ replay(still-valid bridged entries)`: it resets `<urn:sparq:auth>` to the static baseline captured at the last `materialize_wac`/`materialize_acp`, clears the provenance graph, and re-evaluates every tracked `(policy, request)` through its original bridge entry point. An entry that no longer holds emits nothing → it is **retracted** (access gone). `refresh_odrl_grants()` (no args) replays everything as-tracked (used to reconcile after a static re-materialization, which is automatic).
- **Fail-closed (security-sensitive — access retraction).** A withdrawn / lapsed / now-Denied / now-prohibited / ambiguous re-evaluation of an **allow grant** loses access; the underlying evaluator is fail-closed, so on any doubt the grant is retracted, never left stale. A **static** WAC/ACP grant is never in the ledger, never re-evaluated, and always in the captured baseline (captured as the `install_auth_view` output verbatim, not by subtracting provenance — so a static grant byte-identical to a bridged one still survives) — refresh can neither widen nor drop it.

#### Deny RETRACTION is asymmetric to grant retraction — [OPUS-4.8] sq-2pcf

A materialized `auth:deny*` (from a `BridgeKind::Prohibition` / `Policy` entry) is **retracted on the OPPOSITE rule**: a deny carves access *out*, so retracting it *restores* access — that must happen only when the ODRL Prohibition is **definitely** withdrawn or lapsed, never on doubt. Reusing the grant rule (drop the deny whenever `matched_prohibition` no longer matches) would be **fail-OPEN**: an *ambiguous* re-eval — a prohibition still structurally naming the request but carrying a constraint the refresh request gives no evidence for — would silently restore access.

So deny retraction consults `sparq_policy::prohibition_status`, a three-valued refinement of `matched_prohibition`:

| `ProhibitionStatus` | meaning | deny on refresh |
|---|---|---|
| `Applies` | a prohibition still carves the request out | **kept** (re-emitted) |
| `Ambiguous` | still structurally names it, but a constraint is unprovable (no evidence) | **kept** (re-emitted) |
| `Withdrawn` | no prohibition names it, or every one is *definitely* false given the evidence | **retracted** (dropped) |

"Definitely false" means the refresh request supplied evidence for the dimension and the comparison failed (e.g. a `dateTime < 2026-01-01` window with an actual time of `2026-06-01` — provably lapsed). A retracted deny composes with deny-overrides: it may re-expose an allow grant for the same principal+target+mode — correct, *because the prohibition is genuinely gone*. Static (non-bridged) `auth:deny*` rules are never in the ledger and so are never re-evaluated or retracted.

```rust,ignore
// alice was bridged a read grant; the policy then WITHDRAWS the permission.
let (matched, retracted) =
    store.refresh_odrl_grant(&withdrawn_policy, &req, BridgeKind::Permission);
// matched == true, retracted == 1 → alice can no longer read (through accessible/query_as).

// A bridged DENY is the dual: retracted only when the Prohibition is DEFINITELY gone.
let (matched, retracted) =
    store.refresh_odrl_grant(&withdrawn_prohibition, &write_req, BridgeKind::Prohibition);
// definite withdrawal → retracted == 1 (deny gone, access restored if an allow exists);
// ambiguous re-eval (no constraint evidence) → retracted == 0 (deny KEPT, fail-closed).
```

## Pattern-scoped ODRL targets (sub-graph result masking) — [OPUS-4.8] sq-f9u1y

An ODRL target can denote a **triple-pattern sub-graph** of a source graph rather than the whole graph — a *pattern asset*. Two layers, at different maturity:

**Shipped — the `pattern-scope` masking primitive** (opt-in `pattern-scope` feature of `sparq-solid`, library-level, off by default). A `GraphScope` is a set of allow/deny `ScopePattern`s; each pattern's subject/predicate/object is a concrete `Term` or a **wildcard** (`None` = matches anything in that position). A triple is visible iff it matches ≥1 allow pattern **and** 0 deny patterns — **deny overrides allow**. Matching is **term-identity**, not value equality (`"01"^^xsd:integer` does not match a pattern naming the canonical `1`). `masked_graph(base, &scope)` **materializes** the masked sub-graph — it does not filter at query time — so the engine evaluates a real `Graph = D ∖ masked-triples`: OPTIONAL / EXISTS / MINUS / aggregate / COUNT / ASK equivalence is an identity, inherited by every engine fast path.

```rust,ignore
// Only phone triples are visible in this scope; every other triple is masked out.
let scope = GraphScope::allow_only(vec![
    ScopePattern::new(None, Some(phone_pred), None), // wildcard S/O, fixed predicate
]);
let masked = masked_graph(&contacts, &scope);
```

**Fail-closed:** `GraphScope::allow_only(vec![])` (an empty allow-set) grants **nothing** — never a whole-graph fallback.

**Designed — the ODRL→pattern-grant bridge vocabulary** (research §5). The policy parsing + `auth:PatternGrant` materialization are **follow-up beads, not yet wired**, so an ODRL policy cannot yet target a pattern asset end-to-end; the intended triples-native vocabulary (sparq ODRL-profile namespace) is:

```turtle
<urn:ex:asset:contacts-sans-phone> a sparq:PatternAsset ;
    sparq:sourceGraph <https://pod.ex/contacts> ;                 # exactly one
    sparq:pattern [ sparq:predicate <https://ex.dev/ns#phone> ] . # >=1; absent component = wildcard
```

`odrl:target <PatternAsset>` on a **permission** yields allow-patterns over `sourceGraph`; on a **prohibition**, deny-patterns (deny overrides allow, as in the graph-granular bridge). The materialized output (in the reserved `<urn:sparq:auth>` graph) is an `auth:PatternGrant` node (`auth:agent` / `auth:mode` / `auth:graph` + ≥1 `auth:allowPattern` / `auth:denyPattern` blank nodes), structurally parallel to the existing `auth:ConditionalGrant`.

**Fail-closed parse rule (designed):** a `PatternAsset` with **zero** parseable patterns, an **ambiguous** `sourceGraph`, or **any** non-concrete component ⇒ the rule materializes **nothing** (absence-of-grant, never a whole-graph fallback).

*Honest scope:* clear-path authorisation — no cryptographic / privacy / unlinkability guarantee is claimed. Design record: [`research/odrl-pattern-scoped-targets-2026-07.md`](../../research/odrl-pattern-scoped-targets-2026-07.md).

## Conformance — SolidLab ODRL Test Suite — [OPUS-4.8] sq-tmsd6

The evaluator is ratcheted against the MIT-licensed
[SolidLab ODRL Test Suite](https://github.com/SolidLabResearch/ODRL-Test-Suite)
(crate-local runner `crates/sparq-policy/tests/odrl_test_suite.rs`). Each of the 68
self-describing Turtle cases references — by UUID — a policy / request /
state-of-the-world / *expected compliance report*; the runner loads the policy
through the real `parse_policy_str`, derives a real `Request`, evaluates it through
the real `evaluate`, and asserts the decision matches the oracle the expected
report encodes:

| expected rule report | `report:activationState` | expected decision |
|---|---|---|
| `report:PermissionReport` | `report:Active` | ALLOW |
| `report:ProhibitionReport` | `report:Active` | DENY (prohibition fires) |
| either kind | `report:Inactive` | DENY |

At the pinned suite revision **67/68 pass** (floor `ODRL_SUITE_FLOOR`, may only
RISE; mirrored in the central conformance scoreboard). The constraint-matching batch
([OPUS-4.8] sq-euhr3 / sq-k7itg / sq-a0zef) raised the floor from 59 to 67 by
implementing — through the real `evaluate` path — `odrl:LogicalConstraint`
(`odrl:and`/`odrl:or`/`odrl:xone`, incl. nesting) compound constraints,
`odrl:PartyCollection` / `odrl:AssetCollection` membership matching, and the
`odrl:use` action hierarchy (`use` subsumes its sub-actions but **not** the
ownership-`transfer` subtree — so a `use` permission is Active for `read`/`write` but
Inactive for `sell`). The **1** remaining case is a documented **not-implemented**
divergence — a duty whose discharge state is unknown (`report:NonSet`); sparq is
fail-closed (a duty must be explicitly discharged) where SolidLab treats a
not-violated duty as active. It does NOT count as a failure and does NOT gate. Fetch
with `bash scripts/fetch-odrl-suite.sh` (the data is never committed); the runner
self-skips cleanly when the fetched dir is absent. Run:
`cargo test -p sparq-policy --test odrl_test_suite -- --nocapture`.

## Learn more

- Crate README: [`crates/sparq-policy/README.md`](../../crates/sparq-policy/README.md)
- Design record: `research/feature-research-odrl-policy.md` (epic sq-3183)
- Enforcement consumers: `sparq-server`'s opt-in `/authz/*` ODRL lane and `sparq-lws-core`'s opt-in read/query gate seam (`authz::odrl`, `LdpState::set_odrl_gate` — deny-overrides / permit-extends over WAC), both behind an `odrl-authz` feature ([SONNET-4.6] sq-elg47)
- Sibling access-control skill: [`skills/http-server`](../http-server/SKILL.md) (Solid WAC/ACP via `sparq-solid`)
- W3C [ODRL Information Model 2.2](https://www.w3.org/TR/odrl-model/) · [Formal Semantics](https://w3c.github.io/odrl/formal-semantics/)
