# Pod-scoped (incremental) materialization of the Solid auth view

Status: **design-for-review** (research record). Nothing here is implemented. This record
decides *whether and how* a `put_acl`/`delete_acl` at origin `O` can re-derive only `O`'s
slice of `<urn:sparq:auth>` instead of re-running the whole-store N3 materializer, and it
records a **negative result about the obvious design** that the implementation must not
skip. Bead `sq-nmx4l`; follow-up to `sq-b7k7u` (PR #1585); issue #1571 ask 1.

No performance numbers are quoted. Any timing taken on a work box is non-canonical; §5.1
mandates a profile *before* the first implementation bead, because the cost attribution of
the write path is currently unmeasured and the design's value depends on it.

<!-- [OPUS-5] sq-nmx4l design record. Read-only research pass over crates/sparq-solid
(rules/*.n3, src/loader.rs, src/materialize.rs, src/authindex.rs, src/lib.rs,
src/write_through.rs, src/update.rs) + tests/incremental_remat.rs. -->

## 0. Executive summary

- The brief's stated premise — **output** confinement (an `.acl`/`.acr` at origin `O` can
  only produce auth triples whose graph object is at `O`) — **is correct**, and §2.1 gives
  the proof with rule citations.
- The premise the brief does *not* state, and which the design actually needs — **input**
  confinement (`O`'s slice is a function of `O`'s own documents plus its referenced group
  documents) — **is false**. §2.2 gives three concrete counterexamples, two of which are
  already RED-then-GREEN regression tests in `crates/sparq-solid/tests/incremental_remat.rs`.
  `sq-b7k7u` hit exactly this and *abandoned* the confinement lemma for a diff-based
  invalidation that assumes nothing (`crates/sparq-solid/src/lib.rs:650-672`).
- A materializer cannot use that escape hatch. A diff needs the new full index, which is the
  thing we are trying not to compute. So pod-scoping the **materializer** genuinely does need
  a soundness argument — and a bare origin partition is **not** it.
- Two further blockers live in the current code and are prerequisites, not details:
  **positional skolemization** (§3.1) and the **non-partitionable matcher residue** (§3.2).
- Recommendation (§5): a **dependency-closure scoped materializer** with a mandatory
  fail-safe fallback to a full run, staged behind a profile and two hardening beads. §7
  gives the property test the invariant hangs on; §8 the disjoint child beads.

## 1. What `sq-b7k7u` actually shipped (premise check)

The brief says `sq-b7k7u` "delivered the scoped session-cache invalidation + per-origin
decide but LEFT the materializer whole-store". Verified against `origin/main` — accurate:

| Surface | State on `main` |
| --- | --- |
| `AuthIndex.allow`/`deny`/`cond` | origin-bucketed (`authindex.rs:286-296`) |
| `AuthIndex::accessible_in_origin` | present (`authindex.rs:490`) |
| `SessionEntry.per_origin` / `dirty` | present (`lib.rs:316-321`) |
| `reindex_with(ReindexScope::Origin)` | **diff-based**, no lemma assumed (`lib.rs:650-672`) |
| `materialize_wac` / `materialize_acp_*` | **whole-store**, unchanged (`materialize.rs:147`, `:252`) |
| `AuthIndex::from_graph` | **whole-view**, unchanged (`authindex.rs:306`) |
| `rematerialize_scoped` | exists, but only threads the *cache* scope (`write_through.rs:340`) |

Two corrections to the brief:

1. **`rematerialize_scoped` already exists** (`write_through.rs:340-350`). It is not a
   scoped materializer — it selects WAC vs ACP and passes a `ReindexScope` through to the
   cache layer. The name is a `sq-vhhl0` doc-sweep artefact. A scoped materializer needs a
   *different* seam; reusing this name will confuse the diff.
2. The brief lists three open questions (foreign group documents, skolem/dict scoping,
   group-document change tracking). All three are real. It **omits ACP cross-document
   indirection**, which is the sharpest of the four and is what makes the naive partition
   unsound (§2.2 case B).

## 2. The confinement analysis

### 2.1 Output confinement — holds

**Claim.** Every auth triple's graph object is at the same origin as the control document
that produced it.

*WAC.* The only rules deriving `auth:*` are `rules/wac.n3:57-64` (origin-free grants),
`:71-90` (pair-principal grants) and `:93-94` (Control gates the ACL resource). All five
forms bind the resource through `?auth solidx:appliesTo ?r`, derived at `:37-44` from
either

- `?r solidx:ownAcl ?acl . ?auth solidx:inDoc ?acl . … ?auth acl:accessTo ?r` — `ownAcl` is
  loader-emitted by naming convention, `<R>` ↦ `<R + ".acl">` (`loader.rs:229-232`), so
  `origin(?acl) = origin(?r)` by construction; or
- `?r solidx:inheritedAcl ?acl . ?r solidx:inheritsFrom ?c . … ?auth acl:default ?c` —
  `inheritedAcl` walks `solidx:parent` (`wac.n3:27-33`), and `parent` is
  `common.n3:23-27`: a `string:scrape` of `^(.*/)[^/]+/?$` **filtered by
  `?p solidx:isResource true`**.

The scrape capture is always a *prefix* of the input, so it can only shorten the path, never
change the authority. The walk terminates at the pod root because the only shorter prefix is
the authority-less `https://`, which is never in the loader's `resources` set
(`loader.rs:214-222` seeds it from graph names and `parent_iri`, and `parent_iri` returns
`None` at the authority root, `loader.rs:427-440`). `:93-94` maps `?r` to `?r`'s own
`ownAcl`, same origin.

Note the sharpest consequence: `<a.ex/.acl>` asserting `acl:accessTo <b.ex/n1>` grants
**nothing**, because `<b.ex/n1> solidx:ownAcl` is `<b.ex/n1.acl>`, not `<a.ex/.acl>`. The
existing test `cross_origin_foreign_subject_grant_revoke`
(`tests/incremental_remat.rs:342-383`) hedges on this ("depends on the rule semantics") and
asserts only equivalence; §2.1 settles it — no grant is produced.

*ACP.* `rules/acp-a.n3:31-35` derives `?pol solidx:appliesToResource ?r` from `?r`'s own
`ownAcr`, or from `?r solidx:ancestor ?anc . ?anc solidx:ownAcr ?acr` with
`acp:memberAccessControl`. Both pin `?r` to `origin(?acr)` by the same two facts. Every
grant in `acp-c.n3:63-84` and every conditional grant in `:94-137` binds its graph either to
`?pol solidx:appliesToResource ?r` or to `?k solidx:provForResource ?r`, and the latter is
minted only under `?pol solidx:appliesToResource ?r` (`acp-a.n3:158-159`, `:173-174`).

**Guard the implementation must add.** The termination argument depends on `<https://>`
never being a resource. A dataset that contains a named graph literally named `<https://>`
(plus `<https://.acr>`, which `is_control_graph` accepts) would make every pod root a
descendant of a single global container. This is a property of the *current* whole-store
materializer, not of scoping, and its reachability is **unconfirmed** — see §9 Q4 and the
filed follow-up. A scoped materializer must reject authority-less origin roots explicitly
rather than inherit the assumption.

### 2.2 Input confinement — false

`O`'s slice is **not** a function of `O`'s documents. Three routes:

**(A) Foreign group documents (WAC).** `wac.n3:53-54` resolves `acl:agentGroup ?g` through
`?g vcard:hasMember ?a`; the loader reads the group document wherever it lives
(`loader.rs:257-284`, keyed on the fragment-stripped IRI from `collect_agents`,
`loader.rs:392-397`). The brief flags this. Test:
`cross_origin_agent_group_revoke` (`tests/incremental_remat.rs:285-331`) — explicitly noted
as RED under the old lemma-based code.

Sharper than the brief states: **a group document need not be an `.acl`**. The group-doc
pass has no suffix filter, so an ordinary pod content graph can be a group document. That
matters twice — for the closure (§5.2) and because `affects_auth_view` returns `true` only
for `.acl`/`.acr` (`update.rs:114-120`), so a static-target `update_as` write to a
content-graph group document **does not re-materialize today**. That is a pre-existing
stale-grant window, documented at `update.rs:115-117` with "goes through the explicit
`materialize_*` path" as the mitigation. Filed as a follow-up; it is a *prerequisite* for
any dependency-tracking design, because a dependency map with no trigger is decorative.

**(B) ACP cross-document indirection (not in the brief).** `acp-a.n3:31-35` needs
`?acr acp:accessControl ?c . ?c acp:apply ?pol`; `:38-40` needs `?pol acp:allOf/anyOf/noneOf ?m`;
`:43-62` need `?m acp:agent/client/issuer/vc …`. Nothing binds `?c`, `?pol` or `?m` to the
ACR's own document — they are plain IRIs, and *every* `.acr` graph in the dataset is
assembled into one fact set (`loader.rs:239-256`). So `<b.ex/.acr>` may name a policy whose
body lives in `<a.ex/.acr>`, and `O = b.ex`'s slice then depends on `a.ex`'s document.

This is not a bug — ACP deliberately allows policies and matchers to live elsewhere. It is
the reason a bare origin partition cannot be sound for ACP, and it is why §5.2 needs a
document dependency graph rather than an origin filter.

**(C) Split authorization nodes (WAC).** `wac.n3:47-54` (`grantsAgent`) and `:57-64`
(`acl:mode`) carry **no `solidx:inDoc` guard** — only `appliesTo` is document-local. Blank
nodes are skolemized per graph (`loader.rs:143-148`), but *named* authorization nodes are
shared verbatim. So if `<o.ex/.acl>` declares `<o.ex/.acl#a1> a acl:Authorization ;
acl:accessTo <o.ex/n1>` and some foreign `.acl` declares `<o.ex/.acl#a1> acl:agent
<mallory> ; acl:mode acl:Write`, the grant fires on `o.ex/n1` from a foreign document.

Unlike (B), this one is a *defect*, not a spec feature: WAC has no notion of an
Authorization whose subject line is split across documents.

**But the obvious fix does not work.** Adding an existential `?auth solidx:inDoc ?doc` guard to `:47-64` closes nothing, because
`inDoc` is **subject**-scoped, not triple-scoped: the loader collects the subject of every
triple in a control graph into one set and emits one `inDoc` fact per subject per document
(`loader.rs:240-254`; `wac.n3:8` states this — "for every subject S of ACL graph D"). If the
owning `<o.ex/.acl>` supplies `<#a1> a acl:Authorization ; acl:accessTo <o.ex/n1>` and a
foreign `.acl` supplies `<#a1> acl:agent <mallory> ; acl:mode acl:Write`, then `<#a1>` has
`inDoc` facts for **both** documents, so an existential guard is satisfied and the foreign
`acl:agent`/`acl:mode` triples still combine with the local `appliesTo`.

Nor is a *document-qualified* join over the current facts enough — deriving
`grantsAgentIn(?auth, ?doc, ?a)` from `?auth solidx:inDoc ?doc . ?auth acl:agent ?a` and
joining it to `appliesTo` on the same `?doc` still fires, because the foreign `acl:agent`
triple carries no document tag of its own and joins happily against the *local* `inDoc`
fact. Excluding the foreign contribution requires binding **each contributing triple** to
the document that supplied it — i.e. real per-triple provenance, which the loader does not
currently keep (`assemble_facts` pushes each control-document triple into one flat, untagged
`Vec<[Term; 3]>`, `loader.rs:248-250`). Possible shapes, none free, and the choice belongs
to P3 (§8):

- retain per-triple graph provenance in the fact set and join `acl:agent`/`acl:mode`
  against it, so applicability and the contributing properties share one document; or
- have the loader mint document-qualified copies of each control document's triples and
  derive document-qualified intermediate facts, emitting a grant only after applicability
  and the properties agree on the document identity.

Until one of those lands, case (C) stays open and **stays in the dependency closure** —
§5.2 keeps the shared-subject edge, and §8 P3 is respecified accordingly.

### 2.3 The inert-closure lemma (a genuine negative result about a *non*-problem)

ACP's accept-set lattice closes downward over **global** candidate sets:
`acp-a.n3:78-79` (`rawAgentP auth:Authenticated` + `?a solidx:isCandAgent true` ⇒
`rawAgentP ?a`), `:111-112` (client), `:118-119` (issuer). `isCandAgent`/`isCandClient`/
`isCandIssuer` are seeded from *every* policy's candidates (`:142-144`), across all origins.
A scoped run over `O`'s inputs alone therefore produces a **smaller** accept-set than the
full run.

Does that change any decision? No — and the argument is worth recording because it is not
obvious:

1. *Grant rules.* `acp-b.n3:17-41` and `:44-48` only ever probe `?m accepts*P ?p` for `?p`
   drawn from `?k solidx:pairAgent/pairClient/pairIssuer`, i.e. from the candidate triples of
   the policy under test (`acp-a.n3:190-198`, `:173-185`). Those come from that policy's own
   `candAgent`/`candClient`/`candIssuer` (`:123-144`), which — modulo cases (B) and (C) —
   are `O`-local. Any *foreign* entry the global closure added is never probed.
2. *Session-time exception matchers.* `AuthIndex::matcher_accepts`
   (`authindex.rs:418-442`) short-circuits each dimension on the lattice top:
   `set.contains(PUBLIC)`, `set.contains(AUTHENTICATED)`, `set.contains(ANY_CLIENT)`,
   `set.contains(ANY_ISSUER)`. Every accept-set that the global closure widens *already*
   contains the top that admitted the widening (`acp-a.n3:78`, `:111`, `:118`). So the
   concrete entries the closure adds are never load-bearing for `matcher_accepts` either.

**But the auth-view triple set still differs.** `MATCHER_FACTS`
(`materialize.rs:73-77`) copies `solidx:acceptsAgentP`/`acceptsClientP`/`acceptsIssuerP`
into `<urn:sparq:auth>` for every matcher referenced by an `auth:exceptMatcher`. A scoped
run emits fewer of them. That is decision-inert but **triple-visible** — which forces the
choice in §4, and which would also make `matchers_eq` (`authindex.rs:588`) report a
difference on every scoped write and collapse `reindex_with` straight back to
`ReindexScope::Full` (`lib.rs:653-654`), destroying the win `sq-b7k7u` bought.

## 3. Two blockers in the current code

### 3.1 Skolemization is positional, and `put_acl` permutes positions

`skolemize` keys on `gix`, the **index** of the graph in `graph.named`
(`loader.rs:143-148`, enumerated at `:197` and `:258`). `take_named_slot` uses
`Vec::swap_remove` and the new content is `push`ed (`write_through.rs:249-251`, `:295-298`),
so a `put_acl`/`delete_acl` at a non-final position **renumbers other documents' skolem
IRIs**.

Those IRIs can reach the auth view: a blank-node `acp:noneOf` matcher — idiomatic in Turtle
ACRs — surfaces as the object of `auth:exceptMatcher` (`acp-c.n3:104`, `:115`) and as the
subject of the copied `MATCHER_FACTS`. Today that is merely wasteful (the whole view is
rebuilt consistently each time), but it means:

- **for `sq-b7k7u`, already:** a `put_acl` on such a store churns `matcher_agents` for
  unrelated documents ⇒ `matchers_eq` false ⇒ full cache clear on every ACL write. A real
  perf defect in the shipped diff. No current fixture has a blank-node matcher, so it is
  untested. Filed as a follow-up.
- **for `sq-nmx4l`:** a splice is impossible while retained slices hold skolem IRIs that the
  re-derived slice renumbers. **Hard prerequisite.**

Fix: key the skolem on the graph **IRI** (or on a stable per-document id), not on the vector
index. Cheap, local to `loader.rs`, and independently valuable.

### 3.2 Matcher facts are the non-partitionable residue

`MATCHER_FACTS` are attached to matcher IRIs, which have no origin (`iri_origin` on a
skolem or a foreign matcher yields the whole IRI, `loader.rs:413-424`) and which may be
shared by policies at several origins. This is the same reason `AuthIndex.matcher_*` is not
origin-bucketed and `reindex_with` falls back to `Full` when it changes
(`authindex.rs:582-591`, `lib.rs:653`). A scoped materializer must either keep these in a
separate globally-recomputed slice or declare stores with cross-origin shared exception
matchers ineligible for scoping (fall back to a full run).

## 4. What "scoped == full" must mean

Two candidate equivalence relations, and the choice is load-bearing:

| Relation | Cost | Risk |
| --- | --- | --- |
| **Triple-level**: the installed `<urn:sparq:auth>` triple set is identical | Scoped run must reproduce the global candidate seeds (§2.3) and the global matcher residue (§3.2) | None new; mechanically checkable; keeps `matchers_eq` quiet |
| **Decision-level**: `AuthIndex::accessible`/`decide` agree for all sessions/modes/resources | Cheaper — the inert closure can be dropped | Rests on the §2.3 lemma, which is a *hand* proof over unaudited rule text; any future rule that reads a concrete accept-set entry silently breaks it |

**Recommendation: triple-level.** The §2.3 lemma is correct today but is exactly the kind of
invariant that a later rule change (a new matcher attribute, a lattice tweak) invalidates
silently and fail-open. Triple-level equality needs no lemma, is a one-line assertion, and
keeps `matchers_eq` from forcing a full cache clear. Concretely: carry the global
`isCandAgent`/`isCandClient`/`isCandIssuer` seed sets and the matcher residue as a
materializer-level side-table, so the scoped reasoner run reproduces the same closure. These
are small term sets, not documents.

## 5. Recommended design: dependency-closure scoped materializer

### 5.1 First, measure — the write path is four terms, not one

A `put_acl` today costs:

1. **assembly** — `assemble_facts` walks every named graph twice (`loader.rs:197`, `:258`):
   `O(all graphs)`;
2. **reasoning** — `eval` fixpoint over the whole fact set (`materialize.rs:158`, `:268-271`):
   the term this bead targets;
3. **view install** — `install_auth_view` re-interns the whole closure into a fresh `Dict`
   (`materialize.rs:300-337`): `O(closure)`;
4. **index rebuild** — `AuthIndex::from_graph` re-reads the whole view (`lib.rs:644`):
   `O(auth triples)`.

Pod-scoping term 2 alone leaves 1, 3 and 4 at `O(all)`. Terms 1 and 4 have their own fixes
(§8 P2, P6) and term 3 needs the splice (§5.4). **No implementation bead should land before
a profile attributes the write path across these four.** If term 2 does not dominate, the
priority order changes and this design is the wrong first move. The repo has no solid write
benchmark today (`crates/sparq-bench/benches` has none; `crates/sparq-solid/examples/bench.rs`
is a read-path harness) — building one is P0.

### 5.2 The document dependency graph

Maintain, on `PodStore`, `deps: doc IRI → set of doc IRIs it reads`, populated during
assembly:

- WAC: `<o.acl> → {group document of every acl:agentGroup object}` (fragment-stripped,
  `loader.rs:392-397`);
- ACP: `<o.acr> → {document containing each acp:accessControl / apply / allOf / anyOf /
  noneOf / matcher-attribute subject reachable from it}`, computed as a fixpoint over the
  subject→document inverted index;
- WAC case (C): `<o.acl> → {every other .acl sharing a subject IRI}`. This edge is **not**
  removable by an existential `inDoc` guard (§2.2 C shows why that guard is a no-op); it may
  be dropped only once P3 lands per-triple/document-qualified provenance **and** the
  split-node regression (§7 (7)) is green. Treat the edge as load-bearing until then — P4
  must not assume case (C) is eliminated.

`closure(O)` = documents at `O` (control graphs and, for the resource/container facts, `O`'s
graph names) ∪ reachable set. Also needed: the **reverse** map, so a write to a group
document or a shared policy document re-derives every referencing origin.

The common case — a pod whose ACLs reference nothing outside it — has `closure(O)` = `O`'s
own documents, which is the case the `solid-server-rs` ACL-write-churn consumer cares about.
The design's value is entirely in that case; the closure exists to make the *other* cases
sound rather than fast.

### 5.3 Scoped assembly

`assemble_facts_scoped(graph, system, prov, creds, closure)` emits:

- `solidx:isResource` for `O`'s graph names + `O`'s structural container prefixes only
  (needs the origin→graph-names index, §8 P2, or it stays `O(all graphs)`);
- `ownAcl`/`ownAcr` + `inDoc` + document triples for the control graphs in `closure(O)`;
- group-document triples for the group documents in `closure(O)`;
- `provenance` entries whose resource is at `O`; `credentials` in full (caller-supplied,
  small, and global by construction);
- the carried global candidate seeds (§4).

The container chain is within-origin (§2.1), so `common.n3` needs nothing extra.

### 5.4 The splice

Partition the installed view by the **graph object's** origin:

- simple grants `?p auth:mode ?r` ↦ `origin(?r)`;
- a `ConditionalGrant` node and *all* its triples ↦ `origin` of its `auth:graph` object;
- `MATCHER_FACTS` ↦ the global residue slice (§3.2).

Keep the per-origin slices as owned `Vec<[Term; 3]>` on `PodStore` and rebuild the auth
`Graph` from the concatenation, since `install_auth_view` builds a fresh `Dict` per run
(`materialize.rs:311-329`) and there is no delete-by-pattern on `Graph`. That rebuild stays
`O(all auth triples)` — acceptable only if the profile (§5.1) says term 3 is not the
bottleneck; otherwise this needs an origin-partitioned auth graph instead.

Atomicity must be preserved: `materialize_*` currently does every fallible step before the
first mutation so an `Err` leaves the previous view byte-identical
(`materialize.rs:150-153`, guarded by
`a_failed_rematerialization_leaves_the_previous_auth_view_in_place`,
`materialize.rs:500-525`). The splice must keep that ordering.

### 5.5 `AuthIndex` per-origin patch

With the splice in place, replace `AuthIndex::from_graph` on the scoped path with a patch
that drops and rebuilds only `O`'s buckets in `allow`/`deny`/`cond`, plus the matcher maps
when the residue changed. `reindex_with` then compares old vs new for `O` only. Note this
*removes* the diff's current safety net: today the diff catches cross-origin effects
automatically because it recomputes everything (`lib.rs:643-644`, `authindex.rs:281-285`).
Once the index is patched rather than rebuilt, **the closure (§5.2) becomes the sole
soundness argument**. That is the single riskiest step in the plan and must land last,
behind §7's property test.

### 5.6 Fallback discipline (fail-safe, not fail-open)

Fall back to a full materialize + `ReindexScope::Full` whenever:

- the written document is (or was) a group document or a shared policy/matcher document with
  more than one referencing origin;
- the closure cannot be bounded (unresolved indirection, a matcher shared across origins,
  §3.2 residue changed);
- the origin key is not `scheme://authority` — `iri_origin` returns the whole IRI for
  `urn:`-style names (`loader.rs:413-424`);
- an authority-less origin root is present (§2.1 guard).

Every fallback must be counted and surfaced in `MaterializeStats`, so a deployment can see
whether it is getting the fast path at all.

## 6. Options considered and rejected

- **Bare per-origin partition (what the bead title suggests).** Unsound: §2.2 (A), (B), (C).
  Rejected. Recording this explicitly because it is the design a reader will reach for.
- **Reuse the `sq-b7k7u` diff.** Impossible for the materializer: the diff's inputs are the
  old and new full `AuthIndex`, and computing the new one is the cost being avoided.
- **Decision-level equivalence + drop the inert closure.** Cheaper, and correct today, but
  rests on §2.3's hand proof. Rejected in favour of triple-level (§4) — reconsider only if
  the profile shows the carried seeds are themselves the bottleneck.
- **Generic incremental N3 in `sparq-reason` (DRed / counting over a stratified program).**
  Strictly more general — it would fix this for every reasoner consumer and subsume the
  skolem/dict/container questions. Also strictly harder: the engine's `log:notIncludes`
  never retracts (`materialize.rs:4-7`), so deletion under negation needs real
  non-monotonic incremental maintenance. Out of scope for `sq-nmx4l`, but it is the honest
  long-term answer and §9 Q3 asks the maintainer whether to invest there instead.
- **Locality guard on ACP references** (require policies/matchers to be in the ACR's own
  document). Would collapse §2.2 (B) entirely and make the whole design near-trivial. It is
  a **semantic restriction** of ACP, not hardening — see §9 Q2.

## 7. Mandated property test

The invariant, asserted after **every** step of a randomized operation sequence:

> `scoped_materialize(store)` installs the **identical** `<urn:sparq:auth>` triple set as a
> from-scratch `materialize_wac`/`materialize_acp` on the same dataset, **and** the patched
> `AuthIndex` equals `AuthIndex::from_graph` of that view.

Extend `crates/sparq-solid/tests/incremental_remat.rs`, which already carries the shape
(`assert_equals_fresh_rebuild`, `:86`) and the adversarial cross-origin cases (`:285-455`).
The operation alphabet must include, at minimum:

1. `put_acl`/`delete_acl` within one origin (the fast path);
2. group-document edits, including a group document that is **not** an `.acl`
   (the §2.2 (A) sharpening) and one hosted at a different origin from the referencing pod;
3. ACP cross-document indirection: `<b.ex/.acr>` naming a policy defined in `<a.ex/.acr>`,
   then editing `<a.ex/.acr>` (§2.2 (B)) — **this case does not exist in the suite today**;
4. blank-node `acp:noneOf` matchers, with writes at non-final vector positions, so the
   skolem-stability fix (§3.1) is non-vacuously exercised;
5. an exception matcher shared by policies at two origins (§3.2 residue);
6. a `delete_acl` that revokes — no stale grant may survive (the fail-open-critical case);
7. **split authorization nodes** (§2.2 C): `<o.ex/.acl>` declaring
   `<#a1> a acl:Authorization ; acl:accessTo <o.ex/n1>` and a *foreign* `.acl` declaring the
   same subject `<#a1> acl:agent <mallory> ; acl:mode acl:Write`. Assert the foreign `agent`
   and `mode` cannot contribute — no grant to `<mallory>` on `<o.ex/n1>` — and that editing
   the foreign `.acl` therefore leaves `O`'s slice fixed. This case does not exist in the
   suite today; on `main` it is *expected* RED (§2.2 (C)'s rule-text argument — derived from
   the rules, **not yet observed**), so P3 must show it red before fixing it, and it doubles
   as P3's acceptance test. While it is red the §5.2 shared-subject closure edge must remain.

Mutation obligation: each new assertion must be shown red when the scoped path is
deliberately wrong (e.g. splice `O`'s slice without dropping the stale one), not merely green.

## 8. Phased plan — proposed child beads

`bd` is not available in this checkout, so these are **specified, not created**. Each is
single-crate; all are file-disjoint except P1 and P3, which both touch `loader.rs` (see the
sequencing note below).

| # | Bead | Crate | Tier | Invariant | Acceptance test |
| --- | --- | --- | --- | --- | --- |
| P0 | Write-path profile + `put_acl` benchmark; attribute cost across assembly / reason / install / index | `sparq-solid`, `sparq-bench` | sonnet | Measurement only; no behaviour change | A committed harness that reports the four-term split; no numbers in markdown |
| P1 | Skolemize on graph IRI, not vector index | `sparq-solid` (`loader.rs`) | sonnet | Auth view is invariant under `graph.named` permutation | Permute `named`, assert identical view; blank-node-matcher ACR fixture |
| P2 | `origin → graph names` index on `PodStore`, maintained on write | `sparq-solid` | sonnet | Index equals a full scan at every step | Differential vs `graph.named.iter().filter(...)` |
| P3 | Document-qualified WAC authorizations (§2.2 C): per-triple provenance in the fact set, or loader-minted document-qualified triples, so `acl:agent`/`acl:mode` join applicability on the **same** document. NOT a bare existential `inDoc` guard — §2.2 (C) shows that is a no-op | `sparq-solid` (`rules/wac.n3` + `loader.rs`) | opus | A split authorization node grants nothing: a foreign document's `acl:agent`/`acl:mode` cannot combine with a local `appliesTo` | §7 (7), shown RED first; WAC conformance suite unchanged |
| P4 | Document dependency graph + reverse map, built during assembly | `sparq-solid` | opus | `closure(O)` ⊇ true dependency set | Property: perturbing any document outside `closure(O)` leaves `O`'s slice fixed |
| P5 | `assemble_facts_scoped` + scoped reasoner run + carried candidate seeds | `sparq-solid` | opus | Scoped closure ≡ full closure restricted to `O` | §7 (1)–(4), triple-level |
| P6 | Splice + per-origin auth-view slices; `MaterializeStats` fallback counters | `sparq-solid` | opus | Atomicity: `Err` leaves prior view byte-identical | §7 all, plus the existing failed-rematerialization test |
| P7 | `AuthIndex` per-origin patch replacing `from_graph` on the scoped path | `sparq-solid` | opus | Patched index ≡ `from_graph` of the spliced view | §7 (5)–(6); lands **last** |

P0 gates everything. P1, P2 and P3 are independently valuable, but P3 touches `loader.rs`
as well as `rules/wac.n3`, so it is **not** file-disjoint from P1 — sequence it after P1
rather than in parallel. P4 gates P5; P5 gates P6; P6 gates P7. P3 does not gate
P4, but it determines whether P4 may drop the case-(C) shared-subject edge (§5.2): if P3
slips or is declined, P4 ships with that edge retained.

## 9. Open questions for the maintainer

- **Q1 — equivalence relation.** §4 recommends triple-level equality, at the cost of carrying
  the global candidate seeds. Decision-level is cheaper but rests on §2.3's hand proof. Which?
- **Q2 — ACP remote policies.** Does sparq intend to support an `.acr` referencing policies
  or matchers defined in another pod's document at all? If **no**, a locality guard
  (§6) removes §2.2 (B), collapses P4, and makes this design roughly a third of the size.
  This is the highest-leverage question in the record.
- **Q3 — scope of the fix.** Solid-specific closure-scoping (this record), or invest in
  generic incremental/differential N3 evaluation in `sparq-reason` (§6)? The latter is much
  larger and needs non-monotonic maintenance under `log:notIncludes`, but it is the answer
  that does not need re-deriving for the next consumer.
- **Q4 — the `<https://>` guard.** §2.1 identifies a termination assumption whose violation
  would make every pod root a descendant of one global container. Reachability is
  **unconfirmed** (it needs an agent able to create a graph named `<https://>` *and*
  `<https://.acr>`). Worth a dedicated security review pass rather than a guard bolted onto
  this bead?
- **Q5 — group-document triggering.** `affects_auth_view` (`update.rs:114-120`) does not
  fire for a static-target write to a content-graph group document. Should P4 also close
  that (making the dependency map the trigger), or does it stay a deployment-discipline
  matter as documented today?

## 10. Uncertainties

- **No measurement.** The claim that the reasoner fixpoint dominates the write path is the
  brief's, not this record's. It is plausible (the fixpoint is superlinear in the fact set,
  the scans are linear) but **unverified**. P0 exists to settle it, and the plan should be
  re-ordered if it comes out otherwise.
- **§2.1 is a hand proof over rule text**, not a mechanized one. It is careful and cites
  every rule, but it is not a Kani/Coq artefact. The property test in §7 is the actual
  safety net; the proof explains *why* to expect it to pass.
- **§2.3's inert-closure lemma** is the least robust claim here — it depends on
  `matcher_accepts`' short-circuit structure (`authindex.rs:418-442`) staying as it is. §4
  recommends not depending on it.
- **Blank-node matchers are unexercised.** No fixture in `crates/sparq-solid` uses one, so
  §3.1's consequences are derived from the code, not observed. P1's acceptance test should
  add the fixture first and show it red.
- ACP coverage of the rules was read at the source; the `odrl-bridge` feature adds a second
  grant source that replays onto the static baseline (`lib.rs:468-473`). Its interaction with
  a spliced view is **not analysed here** and needs its own pass before P6.
