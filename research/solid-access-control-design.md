# Solid access control on sparq: pods as named graphs, WAC/ACP as rules, queries as dataset views

Status: **shipped** (`crates/sparq-solid`), with this document now serving as the
architecture + design-rationale record. The L1 engine change specified in §5 **has been
implemented and wired** as `sparq-solid`'s default query path — the zero-copy `DatasetView`
(`crates/sparq-engine/src/lib.rs`) and the `exec::view` named-graph filter
(`crates/sparq-engine/src/exec.rs`), measured in §6.1; `sparq-core` itself is unchanged.
The update path (write gating + auto-re-materialization on `.acl`/`.acr` writes, §4.4/§7
item 6) **now also ships** as `PodStore::update_as` / `update_as_acp`
(`crates/sparq-solid/src/update.rs`); the core check + deny path + auto-re-materialization
are wired and tested (`tests/update.rs`), with the documented conservative sub-cases noted
in §4.4. <!-- [OPUS-4.8] doc-freshness sweep sq-7woa: L1 view shipped; header was stale; sq-xor3: write path now ships -->

Below, "L1 phase 1 ships X" and similar past-tense-as-future phrasing is preserved as the
original design narrative; the §6.1 "done" note and §7 record what actually landed.

User-facing documentation of the shipped crate (quick start, support matrix, security
model, API docs): [`crates/sparq-solid/README.md`](../crates/sparq-solid/README.md) and
`cargo doc -p sparq-solid`. This document is the design rationale and the measured-baseline
record those docs link back to.

## 0. Goal and binding constraints

Store Solid Pods in the knowledge graph with **named-graph-per-document** semantics; store
the WAC/ACP rules **also in the KG**; filter queries to the named graphs a given
(user, application) pair can access, via query rewriting / a generic dataset-view operation.
Ideally **zero Solid-specific logic in the engine**: access control = N3 rules + query
rewrites, with the engine optimized for the generic patterns those produce.

Binding user clarification (verbatim, condensed): *"it would be cleaner to have the access
control rules as triples in the database, but if it is more efficient to have custom code for
how they are stored then so be it."* Consequences adopted as hard defaults:

- **D1 (triples-native storage).** ACL/ACP documents are stored **as-is** as named graphs
  (plain triples, queryable like any other document). The *materialized authorization view*
  is also stored as triples, in a dedicated named graph (`<urn:sparq:auth>`), queryable with
  ordinary SPARQL (`GRAPH <urn:sparq:auth> { ?who auth:read ?doc }`).
- **D2 (custom storage must beat the baseline).** Any custom (non-triple) storage
  representation for authorizations is admissible **only** with recorded benchmark numbers
  beating this triples-native baseline; otherwise it is rejected. §6 records the baseline.
- **D3 (transient indexes are fine).** A per-session graph-set cache (e.g. a bitmap/hash-set
  of authorized graph names) is a *transient index over the triples*, not a storage
  substitute — allowed without a benchmark fight.
- **D4 (fail-closed).** A graph with no applicable grant is invisible. The materialized view
  is an allow-list; absence = deny.

Three layers:

- **L1 — engine (generic, follow-up thread):** a zero-copy "dataset view" restricting query
  evaluation to a subset of named graphs (§5). Knows nothing about Solid.
- **L2 — rules:** WAC + ACP encoded as N3 rule files run by `sparq-reason`'s N3 engine,
  materializing the authorization view as triples (§3).
- **L3 — `sparq-solid` crate:** loading conventions, the materializer pipeline, the
  (WebID, client) session cache, and the query-rewrite helper that works on **today's**
  public APIs (§4).

## 1. Research notes

### 1.1 WAC (https://solidproject.org/TR/wac, fetched 2026-06-11)

- An applicable `acl:Authorization` needs: `rdf:type acl:Authorization`; ≥1 `acl:accessTo`
  *or* `acl:default`; ≥1 `acl:mode`; ≥1 of `acl:agent` / `acl:agentGroup` / `acl:agentClass`
  / `acl:origin`.
- **Effective ACL discovery** is *nearest-ancestor*: if a resource has no own ACL
  representation, "repeat the steps using the container resource of resource". Inside the
  effective ACL document: if it is the resource's **own** ACL, only `acl:accessTo <resource>`
  authorizations apply; if it is an **ancestor**'s ACL, only `acl:default <thatAncestor>`
  authorizations apply ("denotes the container resource whose Authorization can be applied
  to a resource lower in the collection hierarchy" — and it does **not** apply to the
  container itself).
- Subjects: `acl:agent` (a WebID); `acl:agentClass foaf:Agent` = public ("any agent"),
  `acl:agentClass acl:AuthenticatedAgent` = any authenticated agent; `acl:agentGroup` → a
  `vcard:Group` whose members are `vcard:hasMember` (the group document is itself a pod
  resource — i.e. another named graph in our store).
- `acl:origin` further restricts an authorization to requests from a given origin — WAC's
  (coarse) user/app-pair dimension; an authorization with `acl:origin` requires agent AND
  origin AND mode to match.
- Modes: `acl:Read`/`acl:Write`/`acl:Append`/`acl:Control`. **Control grants read+write on
  the ACL resource itself**, not on the controlled resource ("Having `acl:Control` does not
  imply that the agent has `acl:Read` or `acl:Write` access to the resource itself, just to
  its corresponding ACL resource").
- ACL association is by `Link rel=acl` header in HTTP-land; in a storage we own this becomes
  a naming convention (§2.2). `acl:accessToClass` is an extension; out of scope.

### 1.2 ACP (https://solidproject.org/TR/acp, fetched 2026-06-11)

- Each resource has an Access Control Resource (ACR). ACRs hold Access Controls;
  `acp:accessControl` applies its policies to the ACR's resource; `acp:memberAccessControl`
  applies them to member resources **transitively** ("member control is transitive,
  therefore, further members of member resources will also be controlled"). Normatively:
  *"A Policy MUST control access to a resource if: it is applied by an Access Control of an
  ACR of the resource; or, it is applied by a member Access Control of an ACR of an ancestor
  of the resource."* So unlike WAC's nearest-ancestor-only ACL, **ACP inheritance is
  cumulative over all ancestors** — which conveniently needs *no negation* to encode.
- Policies: `acp:allow` / `acp:deny` connect policies to access modes (mode IRIs are open;
  `acl:Read` etc. are the conventional ones). **Deny-overrides** is normative: *"An Access
  Mode MUST be granted if and only if in the set of Effective Policies: a satisfied policy
  allows the Access Mode; and, no satisfied policy denies it."*
- Matchers: *"A Matcher MUST be satisfied if and only if: it defines at least one attribute;
  and, at least one value of each defined attribute matches the Context."* I.e. within a
  matcher: same attribute = OR, different attributes = AND. Attributes: `acp:agent`,
  `acp:client`, `acp:issuer`, `acp:vc`. **`acp:client` models the user/app pair natively**
  (a matcher with both `acp:agent` and `acp:client` is exactly an (agent ∧ client)
  condition). Special values: `acp:PublicAgent` (all contexts), `acp:AuthenticatedAgent`
  (contexts with an agent), `acp:CreatorAgent`/`acp:OwnerAgent` (context agent = resource
  creator/owner), `acp:PublicClient`, `acp:PublicIssuer`.
- Policy combinators: `acp:allOf` (all listed matchers must be satisfied), `acp:anyOf` (≥1),
  `acp:noneOf` (none may be satisfied). A policy must reference at least one matcher via
  allOf/anyOf to be satisfiable.
- The TR does not pin down ACR self-access; we follow the WAC-analogous convention (§2.2).

### 1.3 Prior art

- **Kirrane et al.**, *Access control and the resource description framework: a survey*
  (Semantic Web 8(2), 2017; https://www.semantic-web-journal.net/system/files/swj1280.pdf)
  surveys RDF access control incl. **SPARQL query-rewriting** enforcement (inject
  filters/graph restrictions into the algebra) vs **materialized/annotated** approaches
  (label each triple/graph with its policy decision ahead of time). Our design is the hybrid
  the survey points at: *materialize* the per-principal decision (rules → auth view), then
  *rewrite* queries to the authorized graph set — the rewrite stays trivial (a dataset
  clause) because the expensive policy logic ran at materialization time.
- **Stardog named-graph security**
  (https://docs.stardog.com/operating-stardog/security/named-graph-security): graphs the
  user cannot read are *"silently dropped from the RDF dataset for the query"* — precisely
  the L1 dataset-view semantics; Stardog notes it costs "a small overhead" and ships
  disabled by default. **GraphDB FGAC**
  (https://graphdb.ontotext.com/documentation/11.3/fine-grained-access-control.html) is
  finer (per-quad-pattern rules evaluated in-line) and correspondingly more invasive — the
  per-pattern cost model is what we *avoid* by deciding at graph granularity.
- **Community Solid Server** evaluates WAC/ACP imperatively per HTTP request: walk up the
  container chain, fetch the effective ACL/ACR document, evaluate in JS (see
  `@solid/community-server` `src/authorization/`). **Inrupt ESS** evaluates ACP natively per
  request. Per-request evaluation is the right shape for an HTTP gate on ONE resource; it is
  the wrong shape for SPARQL over thousands of graphs at once — hence materialization.
- sparq-internal precedent: `sparq-reason` already does RDFS/OWL-RL **materialization** with
  the same justification (query stays fast; closure computed at load time), and the engine
  already has a thread-local RAII install pattern for cross-cutting execution state
  (`exec::budget::install`) that L1 reuses.

### 1.4 sparq-reason N3 dialect inventory (read from `crates/sparq-reason/src/n3/`)

Relevant supported features (verified in `mod.rs`/`parser.rs`):

- `@prefix`, `{ … } => { … }` forward rules, `<=` backward rules, lists `( … )`.
- **`log:notIncludes` / `log:includes`** with an *empty-formula subject* (`{}`) = scoped
  negation-as-failure **against the current store**. Critical semantics: rules containing
  negation are re-evaluated every fixpoint round, but **derived facts are never retracted**
  — so a rule may fire on a not-yet-true negation and its conclusion will *stay*. ⇒ Negation
  is only sound over predicates that are **fully present before the stratum starts** (input
  facts, or output of an earlier `reason_n3` call). Our rule sets are stratified accordingly
  (§3.5).
- `string:scrape` `( str regex ) → first capture group` and `log:uri` (IRI ↔ string, both
  directions) — together these do IRI-ancestry ("parent container of an IRI") **in pure N3**.
- `string:concatenation` — deterministic minting of pair-principal / grant-node IRIs.
- `string:startsWith/contains/matches/…`, `math:*` comparisons, `list:member/in`,
  `log:dtlit`, `log:conjunction`.
- **Follow-ups shipped** (bead `sq-jwsp`, DONE): `log:collectAllIn` (scoped findall — with
  `math:memberCount` this is the count-over-*property-values* aggregate) and `log:forAllIn`
  (scoped universal quantification — the direct allOf collapse), both non-monotonic like the
  negation idiom (sound over stratum-complete predicates only), plus the multi-stratum entry
  point `reason_n3_stratified` (§7 item 2). URI-encoding shipped earlier
  (`string:encodeForUri`). Remaining by design: backward-rule depth is bounded (we use
  forward rules only). <!-- [FABLE-5] sq-jwsp -->

`reason_n3(dict, src)` is single-graph (facts are term-triples). The materializer therefore
*assembles* the reasoning input from the relevant named graphs + synthesized facts (§4.2) —
the named-graph structure is reflected into facts (`solidx:ownAcl`, `solidx:inDoc`, …)
rather than being visible to the reasoner directly.

## 2. Storage model

### 2.1 Pods in the KG

- Every pod document (RDF resource) is one **named graph whose name is the resource IRI**:
  `https://pod.example/alice/notes/note1.ttl` ⇒ graph of the same name.
- Containers are named graphs too (their `ldp:contains` listing lives there); container IRIs
  end in `/`. Containment is *structural*: the parent of `<…/a/b>` is `<…/a/>` (Solid slash
  semantics). We derive ancestry from IRI structure by rule (§3.2) — `ldp:contains` is then
  derivable/checkable rather than trusted.
- The store default graph is reserved for non-pod data; pod queries never touch it.

### 2.2 Access-control documents in the KG

- WAC: the ACL document of resource `R` is the named graph `<R + ".acl">` (for containers,
  `<C/ + ".acl">`, e.g. `https://pod.example/alice/.acl` — CSS-compatible). What HTTP
  discovers via `Link rel=acl`, the store fixes by naming convention. The loader emits the
  link as a fact (`<R> solidx:ownAcl <R.acl>`), so a deployment with a different convention
  only changes the loader, not the rules.
- ACP: ditto with `".acr"`.
- Both are **ordinary named graphs**: ACL/ACR triples stay queryable as stored (D1). Reading
  them *through the access-controlled path* requires `acl:Control` (WAC) — the rules grant
  `auth:read`/`auth:write` on the `.acl` graph to Control-holders (§3.3).

### 2.3 The materialized authorization view

One reserved named graph **`<urn:sparq:auth>`** holding plain triples (D1):

```turtle
@prefix auth: <https://sparq.dev/ns/auth#> .

# simple grants: PRINCIPAL  mode-predicate  GRAPH-NAME
<https://pod.example/bob/profile/card#me>  auth:read  <https://pod.example/alice/notes/note1.ttl> .
auth:Public                                auth:read  <https://pod.example/alice/public/post.ttl> .
auth:Authenticated                         auth:append <https://pod.example/alice/inbox/> .

# client-restricted grants: a deterministically minted PAIR principal
<urn:sparq:pair?agent=https://pod.example/bob/profile/card%23me&client=https://app.example/id>
    auth:read <https://pod.example/alice/shared/doc.ttl> .

# conditional grants (ACP noneOf): a minted grant node carrying its exceptions
<urn:sparq:grant?...> a auth:ConditionalGrant ;
    auth:agent auth:Public ; auth:client auth:AnyClient ;
    auth:mode acl:Read ; auth:graph <https://pod.example/alice/blog/> ;
    auth:exceptMatcher <https://pod.example/alice/blog/.acr#blockedMatcher> .

# ACP deny half (deny-overrides resolved at session time, §3.4):
<…#me> auth:denyWrite <https://pod.example/alice/finance/> .
```

Why this shape (the **principal model**) and not 4-ary reified quads:

- A grant is conceptually (agent, client, mode, graph). Collapsing (agent, client) into one
  **principal term** and mode into the **predicate** makes the common case a *single triple*
  — maximally queryable ("who can read G?" is one pattern) and maximally materializable by
  N3 rules (no fresh-bnode-per-grant; conclusions are plain ground triples).
- Principals: a WebID; `auth:Public` (any agent, incl. anonymous); `auth:Authenticated`
  (any logged-in agent); a minted pair `urn:sparq:pair?agent=A&client=C` for
  client-restricted grants (WAC `acl:origin`, ACP `acp:client`); `auth:AnyClient` is the
  client-dimension top. A session (WebID `A`, client `C`) *expands* to the principal set
  `{auth:Public, auth:Authenticated, A, pair(auth:Public,C), pair(auth:Authenticated,C),
  pair(A,C)}` (anonymous sessions: `{auth:Public, pair(auth:Public,C)}`) — at most 6
  lookups, done by the session layer (§4.3).
- Deny and `noneOf` do **not** collapse into the allow-list (a deny on agent A must override
  an allow on Public for A's sessions; a `noneOf` exception narrows a grant without denying
  other policies' grants). They stay first-class triples (`auth:deny*`, ConditionalGrant
  nodes) and are resolved per-session: `effective = ∪allow(principals) ∖ ∪deny(principals)`,
  conditional grants/denies gated by their `exceptMatcher`s. Pure-SPARQL consumers get the
  same answer with a `MINUS`/`FILTER NOT EXISTS` over the auth graph.

Pair-IRI minting uses raw concatenation (`urn:sparq:pair?agent=` + IRI + `&client=` + IRI)
because the N3 dialect has no percent-encoding builtin. Mitigation shipped (it would
otherwise be a grant-collision attack — roborev 1723): `urn:sparq:` is a **reserved IRI
space** — the loader REJECTS any agent/group-member/origin/client value containing the
literal `&client=` or starting with `urn:sparq:`; sessions whose agent/client values sit
in the reserved space get the EMPTY graph set (no pair-principal impersonation); and
`PodStore::new`/the materializer strip ALL reserved-named graphs from loaded datasets,
including a pre-existing `<urn:sparq:auth>` (only `install_auth_view` may create it) —
all fail-closed, regression-tested in tests/hardening.rs. The clean long-term fix stays a tiny
`string:encodeForUri` builtin follow-up (§7).

### 2.4 Security boundary of the reasoner

Only **access-control inputs** feed the materializer: `.acl`/`.acr` graphs, group documents
referenced via `acl:agentGroup` (fragment stripped → graph name), and loader-synthesized
structural facts. Pod *content* graphs are **excluded** — otherwise any agent able to write
a document could embed `acl:` triples and grant themselves access. Writing `.acl`/`.acr`
graphs is what `acl:Control` (WAC) / ACR write (ACP) gates.

That excludes pod *content*, but the `.acl`/`.acr`/group graphs are themselves emitted to
the reasoner **verbatim**, so there is a SECOND smuggling surface: the reasoner's own
**derivation-internal vocabulary** is the `solidx:` namespace (`solidx:creator`,
`solidx:owner`, `solidx:appliesToResource`, `solidx:isResource`, `solidx:isWebId`,
`solidx:provForResource`, …). Those facts are meant to be produced ONLY by the loader (from
trusted structural metadata + the caller-supplied `AccessProvenance`) or derived by the
rules. A writer who controls an `.acr` could otherwise place a forged
`<r> solidx:creator <self>` (cross-resource privilege escalation) or
`<pol> solidx:appliesToResource <secret>` (policy redirection onto a resource they do not
control) directly into the control document — and the rules cannot distinguish it from a
loader-synthesized trusted fact. **[OPUS-4.8] sq-3jtd.5:** the loader therefore HARD-REJECTS
(`is_reserved_derivation_predicate`) any control-graph or group-document triple whose
predicate is in `solidx:` space before it reaches the reasoner — the direct analogue of the
`urn:sparq:` reserved-principal guard (`validate_principal_iri`). The trusted channel for
creator/owner facts is `AccessProvenance` and **nothing else**. *Until that filter was in
place this boundary was NOT airtight: the original forgery test only covered a forged fact
in a **resource** graph (never fed to the reasoner anyway); a forged fact placed inside the
`.acr` itself escalated. The filter, the `acp:CreatorAgent`/`acp:OwnerAgent` resource-scoping
AND the `solidx:appliesToResource` redirection class are now closed — see tests
`acp_forged_{creator,owner,applies_to_resource}_in_acr_document_does_not_grant`.*

## 3. L2 — the rule sets (`crates/sparq-solid/rules/*.n3`)

Vocabulary: `auth:` = `https://sparq.dev/ns/auth#` (public view), `solidx:` =
`https://sparq.dev/ns/solidx#` (derivation-internal; kept out of the auth graph except where
the session layer needs it, §3.4).

### 3.1 Loader-synthesized input facts (Rust, §4.2)

For graphs and IRI structure (things the single-graph reasoner cannot see):

```text
<R>  solidx:isResource true .          # every pod graph + every structural container prefix
<R>  solidx:ownAcl <R.acl> .           # iff graph <R.acl> exists      (WAC)
<R>  solidx:ownAcr <R.acr> .           # iff graph <R.acr> exists      (ACP)
<S>  solidx:inDoc <D> .                # for every subject S in acl/acr graph D (provenance
                                       #   after the graph merge; bnodes skolemized per graph)
<A>  solidx:isWebId true .             # concrete (non-special) agent IRIs seen in acl/acr/group docs
```

### 3.2 Shared ancestry (pure N3 — IRI slash semantics)

```n3
# parent: strip the last non-empty segment; only keep it if the parent is a known resource.
{ ?r solidx:isResource true . ?r log:uri ?rs .
  (?rs "^(.*/)[^/]+/?$") string:scrape ?ps . ?p log:uri ?ps .
  ?p solidx:isResource true . }
=> { ?r solidx:parent ?p } .

{ ?r solidx:parent ?p }                          => { ?r solidx:ancestor ?p } .
{ ?r solidx:parent ?p . ?p solidx:ancestor ?a }  => { ?r solidx:ancestor ?a } .
```

### 3.3 WAC (`rules/wac.n3` — single stratum)

Effective-ACL resolution. The "no closer ACL exists" negation is `log:notIncludes` over
`solidx:ownAcl`, which is **input-only** (loader-emitted, never rule-derived) — safe under
the engine's no-retraction NAF (§1.4):

```n3
# nearest-ancestor inheritance, fail-closed
{ ?r solidx:parent ?p . ?p solidx:ownAcl ?acl .
  {} log:notIncludes { ?r solidx:ownAcl ?x } . }
=> { ?r solidx:inheritedAcl ?acl . ?r solidx:inheritsFrom ?p } .

{ ?r solidx:parent ?p . ?p solidx:inheritedAcl ?acl . ?p solidx:inheritsFrom ?c .
  {} log:notIncludes { ?r solidx:ownAcl ?x } . }
=> { ?r solidx:inheritedAcl ?acl . ?r solidx:inheritsFrom ?c } .

# which authorizations apply to which resource (provenance via solidx:inDoc)
{ ?r solidx:ownAcl ?acl . ?auth solidx:inDoc ?acl . ?auth a acl:Authorization .
  ?auth acl:accessTo ?r . }                                  => { ?auth solidx:appliesTo ?r } .
{ ?r solidx:inheritedAcl ?acl . ?r solidx:inheritsFrom ?c .
  ?auth solidx:inDoc ?acl . ?auth a acl:Authorization .
  ?auth acl:default ?c . }                                   => { ?auth solidx:appliesTo ?r } .

# subject dimension → principals
{ ?auth acl:agent ?a . }                          => { ?auth solidx:grantsAgent ?a } .
{ ?auth acl:agentClass foaf:Agent . }             => { ?auth solidx:grantsAgent auth:Public } .
{ ?auth acl:agentClass acl:AuthenticatedAgent . } => { ?auth solidx:grantsAgent auth:Authenticated } .
{ ?auth acl:agentGroup ?g . ?g vcard:hasMember ?a . } => { ?auth solidx:grantsAgent ?a } .

# grants — origin-free (the negation is over acl:origin, an INPUT predicate)
{ ?auth solidx:appliesTo ?r . ?auth solidx:grantsAgent ?p . ?auth acl:mode acl:Read .
  {} log:notIncludes { ?auth acl:origin ?o } . }  => { ?p auth:read ?r } .
#   …same for acl:Write→auth:write, acl:Append→auth:append, acl:Control→auth:control

# origin-restricted grants mint a pair principal
{ ?auth solidx:appliesTo ?r . ?auth solidx:grantsAgent ?a . ?auth acl:origin ?o .
  ?auth acl:mode acl:Read .
  ?a log:uri ?as . ?o log:uri ?os .
  ("urn:sparq:pair?agent=" ?as "&client=" ?os) string:concatenation ?ps .
  ?p log:uri ?ps . }                              => { ?p auth:read ?r } .
#   …×4 modes

# Control ⇒ read+write of the ACL graph itself (WAC §1.1: Control is about the ACL resource)
{ ?p auth:control ?r . ?r solidx:ownAcl ?acl . } => { ?p auth:read ?acl . ?p auth:write ?acl } .
```

(The shipped file has the full mode×origin matrix — 8 grant rules + 2 Control rules.)

### 3.4 ACP (`rules/acp-*.n3` — three strata)

ACP materialization must answer, per policy: *for which (agent, client) value combinations
is this policy satisfied?* The combination space is kept finite by evaluating only
**candidate principals**: values mentioned by the policy's own matchers, plus the dimension
tops (`auth:Public`, `auth:AnyClient`). Correctness: a matcher accepts a candidate principal
iff it accepts *every* context the principal denotes (principal-subsumption is monotone:
`Public ⊒ Authenticated ⊒ concrete-WebID`, `AnyClient ⊒ concrete-client`), so a surviving
candidate is a sound grant; and every satisfiable context combination is covered by some
candidate because non-mentioned values behave exactly like the top of their dimension.

**Stratum A (monotone)** — effective policies, matcher accept-sets, candidates:

```n3
# cumulative inheritance (§1.2): own ACR's accessControl + EVERY ancestor's memberAccessControl
{ ?r solidx:ownAcr ?acr . ?acr acp:accessControl ?c . ?c acp:apply ?pol . }
=> { ?pol solidx:appliesToResource ?r } .
{ ?r solidx:ancestor ?anc . ?anc solidx:ownAcr ?acr .
  ?acr acp:memberAccessControl ?c . ?c acp:apply ?pol . }
=> { ?pol solidx:appliesToResource ?r } .

# matcher raw values mapped into principal space (used for candidate generation)
{ ?m acp:agent ?a . ?a solidx:isWebId true . } => { ?m solidx:agentValP ?a } .
{ ?m acp:agent acp:PublicAgent . }        => { ?m solidx:agentValP auth:Public } .
{ ?m acp:agent acp:AuthenticatedAgent . } => { ?m solidx:agentValP auth:Authenticated } .
{ ?m acp:client ?c . ?c log:notEqualTo acp:PublicClient . }
                                          => { ?m solidx:clientValP ?c } .
{ ?m acp:client acp:PublicClient . }      => { ?m solidx:clientValP auth:AnyClient } .

# accept-sets (downward-closed over principal subsumption; closure restricted to candidates)
{ ?m solidx:agentValP ?a . }                                    => { ?m solidx:acceptsAgentP ?a } .
{ {} log:notIncludes { ?m acp:agent ?x } . ?m solidx:isMatcher true . }
                                                                => { ?m solidx:acceptsAgentP auth:Public } .
{ ?m solidx:acceptsAgentP auth:Public . }                       => { ?m solidx:acceptsAgentP auth:Authenticated } .
{ ?m solidx:acceptsAgentP auth:Authenticated . ?a solidx:isCandAgent true . }
                                                                => { ?m solidx:acceptsAgentP ?a } .
#   (client dimension analogous, with auth:AnyClient as top)

# candidates per policy: union of its matchers' raw values + the tops
{ ?pol solidx:hasMatcher ?m . ?m solidx:agentValP ?a . }  => { ?pol solidx:candAgent ?a . ?a solidx:isCandAgent true . } .
{ ?pol solidx:hasMatcher ?m . }                           => { ?pol solidx:candAgent auth:Public .
                                                               ?pol solidx:candClient auth:AnyClient . } .
{ ?pol acp:allOf ?m . } => { ?pol solidx:hasMatcher ?m . ?m solidx:isMatcher true . } .   # + anyOf, noneOf
```

**Stratum B (NAF over stratum-A output)** — per-matcher rejections and anyOf satisfaction,
on the candidate cross-product (`solidx:candAgent × solidx:candClient`, paired via minted
`urn:sparq:cand?…` nodes):

```n3
{ ?pol solidx:candPair ?k . ?k solidx:pairAgent ?pa . ?pol acp:allOf ?m .
  {} log:notIncludes { ?m solidx:acceptsAgentP ?pa } . }   => { ?k solidx:allOfRejected true } .
#   (+ client-dimension twin)
{ ?pol solidx:candPair ?k . ?k solidx:pairAgent ?pa . ?k solidx:pairClient ?pc .
  ?pol acp:anyOf ?m . ?m solidx:acceptsAgentP ?pa . ?m solidx:acceptsClientP ?pc . }
=> { ?k solidx:anyOfSat true } .
```

**Stratum C (NAF over stratum-B output)** — grants:

```n3
# simple grant: allOf all pass, anyOf passes (or none declared), NO noneOf declared
{ ?pol solidx:candPair ?k . ?k solidx:pairAgent ?pa . ?k solidx:pairClient ?pc .
  ?pol solidx:appliesToResource ?r . ?pol acp:allow acl:Read .
  {} log:notIncludes { ?k solidx:allOfRejected true } .
  ?k solidx:anyOfOk true .                       # derived: anyOfSat ∨ ¬∃acp:anyOf (input NAF)
  {} log:notIncludes { ?pol acp:noneOf ?nm } .
  ?pc log:equalTo auth:AnyClient . }             => { ?pa auth:read ?r } .
#   client-restricted twin mints pair(?pa,?pc); acp:deny twin emits auth:denyRead; ×4 modes

# noneOf ⇒ CONDITIONAL grant node carrying its exception matchers (session-evaluated, §4.3)
{ …same satisfaction premises… ?pol acp:noneOf ?nm .
  (…) string:concatenation ?gs . ?g log:uri ?gs . }
=> { ?g a auth:ConditionalGrant ; auth:agent ?pa ; auth:client ?pc ;
        auth:mode acl:Read ; auth:graph ?r ; auth:exceptMatcher ?nm . } .
```

**Deny-overrides** is *deliberately not* resolved at materialization: the normative rule
quantifies over the session's whole context (a deny matched via *any* principal kills allows
matched via *other* principals), which a per-principal materialized view cannot express
without enumerating the open world of agents. Both halves are materialized as triples; the
session layer (and equivalently a SPARQL `MINUS`) computes
`∪allow(principals) ∖ ∪deny(principals)` per mode — exact, cheap (≤6 principals), and
spec-faithful. `noneOf` similarly narrows a single policy's applicability (it is *not* a
deny: other policies may still grant the excluded agent), hence conditional grant nodes with
`auth:exceptMatcher` links; the matcher's accept-set facts are included in the auth graph so
the exception check is itself a triple lookup.

### 3.5 Stratification (why 1 + 3 passes)

The engine's NAF never retracts (§1.4), so every `log:notIncludes` must scope over a
predicate that is **complete** when its stratum runs. WAC negates only loader-input
predicates (`solidx:ownAcl`, `acl:origin`) ⇒ **one** `reason_n3` call. ACP needs
"∀ allOf-matcher accepts" = ¬∃ rejection, where rejection is itself ¬accepts ⇒ accepts
(A) → rejects (B) → grants (C): **three** calls, each seeded with the previous closure.
The materializer (§4.2) hardcodes this order; each stratum is still pure N3 in shipped
`.n3` files — the pipeline is scheduling, not logic.

### 3.6 Covered / not covered

Covered: WAC accessTo, default with nearest-ancestor discovery (incl. multi-level), agent,
agentClass foaf:Agent + acl:AuthenticatedAgent, agentGroup (+ group docs as graphs), origin
(as pair principal), 4 modes with Control→ACL-resource semantics; ACP accessControl +
transitive memberAccessControl, allOf/anyOf/noneOf, agent/client/**issuer** attrs incl.
Public/Authenticated/PublicClient specials, allow/deny with deny-overrides.
`acp:issuer` ([OPUS-4.8] sq-3jtd.6): the third principal dimension — the OIDC issuer that
vouched for the WebID — is the exact twin of the client dimension. A constrained issuer mints
a three-component `urn:sparq:triple?agent=A&client=C&issuer=I` principal (an unconstrained
issuer keeps the agent / `urn:sparq:pair?…` term byte-identical), the candidate enumeration
gains an `issuer × {matcher values, auth:AnyIssuer top}` factor (still bounded as the client
dimension is), and the session expands to ≤12 lookups (the pre-issuer ≤6 doubled).
`acp:CreatorAgent`/`acp:OwnerAgent` ([OPUS-4.8] sq-3jtd.5): the context agent must be the
resource's creator / owner. "Who created/owns `<r>`" is structural storage metadata the
trusted caller (PSS) supplies through the `AccessProvenance` channel and
`materialize_acp_with` — the loader synthesizes `<r> solidx:creator|owner <w>` facts from
THAT map ONLY. Neither pod content NOR the `.acr` document itself can supply them: the
loader hard-rejects any control-graph triple whose predicate is in `solidx:` space (§2.4),
so a writer cannot self-grant via a forged `solidx:creator` triple smuggled into the `.acr`
(tests `acp_forged_{creator,owner,applies_to_resource}_in_acr_document_does_not_grant`).
The grant is RESOURCE-SCOPED — a resource-tagged
`urn:sparq:provcand?…&res=R` candidate mints per (policy, creator/owner, resource), so the
creator of `R1` is never granted `R2`; the creator/owner agent composes with the matcher's
own `acp:client`/`acp:issuer` constraints (minting the same pair/triple principal). With no
provenance supplied, no `CreatorAgent`/`OwnerAgent` matcher grants (fail-closed). A provenance
matcher composes with client/issuer constraints on itself and with `anyOf`/`noneOf`/the
policy's other allOf matchers' client/issuer dimensions. It ALSO composes with a SECOND,
independent concrete-WebID `acp:agent` matcher under the same `allOf` ([OPUS-4.8] sq-az1b)
with no special-casing — the provenance candidate's agent dimension is the creator/owner
WebID, so the sibling concrete matcher's accept-set (that one WebID) and the allOf rejection
check (acp-b.n3) intersect the two agent constraints: the degenerate case where the concrete
WebID EQUALS the resource's creator/owner is supported (the agent satisfies both matchers,
the grant stays resource-scoped — the creator of `R1` is still never granted `R2`), and the
case where the concrete WebID is a DIFFERENT fixed agent grants nobody — correct-by-soundness
(the two matchers demand the agent be both the creator and a distinct fixed WebID, an
unsatisfiable conjunction), not a missing feature. Documented bound: only `acl:accessToClass` /
custom modes remain out of scope (below); `acp:vc` shipped in [SONNET-4.6] sq-ysv3u.
Not covered (documented gaps, §7): `acl:accessToClass`, custom ACP modes
(design supports any mode IRI; prototype maps the 4 standard ones). `acp:vc` IS now covered
(sq-ysv3u): exact-IRI requirement matching against the trusted `VerifiedCredentials` channel,
fail-closed with none supplied; see research/solid-vocab-gaps-design.md §2 *Outcome*.

## 4. L3 — the `sparq-solid` crate

### 4.1 Public surface (v1, prototype-backed)

```rust
pub fn materialize_wac(graph: &mut Graph) -> Result<MaterializeStats, String>;
pub fn materialize_acp(graph: &mut Graph) -> Result<MaterializeStats, String>;
//   both: strip reserved graphs → assemble facts (§4.2) → reason_n3 strata →
//   REPLACE <urn:sparq:auth> in graph.named

pub struct Session<'a> { pub agent: Option<&'a str>, pub client: Option<&'a str>, pub issuer: Option<&'a str> }
//   [OPUS-4.8] sq-3jtd.6: `issuer` (ACP acp:issuer, the OIDC IdP); None = any issuer.
pub enum Mode { Read, Write, Append, Control }
pub struct AuthIndex { /* principal → mode → graph names; conditional grants; matcher accept-sets */ }
impl AuthIndex {
    pub fn from_graph(g: &Graph) -> AuthIndex;            // reads GRAPH <urn:sparq:auth>
    pub fn accessible(&self, s: &Session, m: Mode) -> Vec<NamedNode>; // allow ∖ deny, conditionals applied
}
pub fn rewrite_for(query: &str, allowed: &[NamedNode]) -> Result<String, String>;

pub struct PodStore { pub graph: Graph, /* auth: Arc<AuthIndex>, epoch, per-session cache */ }
impl PodStore {
    pub fn new(graph: Graph) -> PodStore;                 // strips reserved urn:sparq: graphs
    pub fn materialize_wac(&mut self) -> Result<MaterializeStats, String>; // + reindex + cache clear
    pub fn materialize_acp(&mut self) -> Result<MaterializeStats, String>;
    pub fn accessible(&mut self, s: &Session, m: Mode) -> Arc<Vec<NamedNode>>; // cached per epoch
    pub fn query_as(&mut self, s: &Session, m: Mode, q: &str) -> Result<QueryResult, String>;
}
```

### 4.2 Materializer pipeline

1. Classify `graph.named` by name: `*.acl` → WAC docs, `*.acr` → ACP docs, group docs =
   targets of `acl:agentGroup` (fragment stripped), `<urn:sparq:auth>` skipped, rest = pod
   content (NOT fed to the reasoner — §2.4).
2. Serialize acl/acr/group graph triples to N3 source (N-Triples-shaped lines; blank nodes
   skolemized `urn:skolem:<graph-hash>:<label>` so the merge keeps per-document scoping and
   `solidx:inDoc` provenance stays sound).
3. Append synthesized facts (§3.1) + the stratum's rule file; `reason_n3` per stratum,
   re-serializing the closure between ACP strata.
4. Filter the final closure to the auth-view predicates (`auth:*` grants, ConditionalGrant
   nodes, and the `solidx:acceptsAgentP/acceptsClientP/agentUnconstrained/clientUnconstrained`
   facts of matchers referenced by `auth:exceptMatcher`); build a fresh sub-`Graph`
   (`Graph::from_parts`) and replace the `<urn:sparq:auth>` entry in `graph.named`.
5. Bump the epoch ⇒ `SessionCache` drops all cached graph-sets (D3: it is a transient index).

**Incremental maintenance is v1-deliberately-absent:** any `.acl`/`.acr`/group-doc change
re-runs the full pipeline (cheap at pod scale — measured in §6; the auth view is small).
N3-incremental maintenance is a real follow-up: `sparq-reason`'s exact-derivation-counting
(`MaterializedGraph`, T18) is RDFS-only today and does not know NAF; counting under
stratified NAF needs (de)derivation re-checks per stratum (§7).

### 4.3 Session model

`accessible(session, mode)`:
1. Expand principals: `{Public, pair(Public,C)} ∪ {Authenticated, A, pair(Authenticated,C),
   pair(A,C) if A}` (pairs only when `C` given).
2. `allow = ∪ grants[p][mode]`; add each ConditionalGrant whose (agent,client) ∈ principals
   and whose every `exceptMatcher` does NOT accept the session (matcher acceptance = its
   materialized accept-set ∩ session principal-dimension ≠ ∅ — a hash lookup, no policy
   logic).
3. `deny = ∪ denies[p][mode]` (conditional denies symmetric).
4. Result: sorted, deduped `allow ∖ deny` → `Arc<Vec<NamedNode>>` cached per
   (agent, client, mode, epoch). This cached set is the "graph-set bitmap" the L1 view takes.

### 4.4 v1 query path (today's public APIs) — and its honest cost

`rewrite_for` parses with `spargebra`, then:
1. **Per-pattern GRAPH wrapping** (union-default emulation): every default-graph triple
   pattern `t` becomes `GRAPH ?__sgN { t }` (fresh var per pattern, joined above — so
   cross-document joins work; patterns already under `GRAPH` are left alone, as are the
   graph-vars the user wrote). Divergence from true RDF-merge semantics: a triple asserted
   in k accessible graphs yields k duplicate rows where a merge would yield 1 — acceptable
   for v1 (DISTINCT restores set semantics), eliminated by L1's union-default mode.
2. **Dataset clause injection**: `dataset.named = allowed` (∩ pre-existing `FROM NAMED` if
   the query had one; `FROM`/default = empty). `GRAPH ?g` / `GRAPH <g>` then range over
   exactly the authorized graphs — enforced by the engine's existing `build_active`
   semantics ("the store's own graphs do NOT leak in"; absent graph = empty graph).
   Pitfall found by test: an EMPTY allowed list serializes to *no* `FROM NAMED` clause at
   all, and the reparsed query would see the whole store — the rewrite inserts a sentinel
   absent graph (`<urn:sparq:nothing>`) so the dataset clause survives the round-trip
   (fail-closed).
3. Serialize (`spargebra` `Display`) and run through `sparq_engine::query`.

Cost: `build_active` (crates/sparq-engine/src/dataset.rs) **decodes every listed graph to
term triples and rebuilds a fresh dictionary + permutation indexes per query** —
O(Σ triples in authorized graphs) time and memory *per query*. That is the copy the L1 view
exists to delete; §6 measures it so L1 has a baseline to beat.

Update path (**now shipped** — `crates/sparq-solid/src/update.rs`, `PodStore::update_as` /
`update_as_acp`; sq-xor3): a SPARQL Update is authorized *before* it mutates, mirroring the
read path's permission model. The update is parsed (`spargebra`), every graph it could
write is extracted as a `(graph, need)` requirement, and each is checked against the
session's per-mode accessible set:

- writes to graph `G` require `auth:write` on `G`; a **pure insert** is satisfied by
  `auth:write` OR `auth:append` (WAC `acl:Append` adds without removing); a delete/clear
  requires `auth:write`;
- `.acl`/`.acr` writes require `auth:write` on the `.acl`/`.acr` graph — and the WAC/ACP
  rules grant `auth:write` on those graphs only to `acl:Control` holders (§3.3: "Control ⇒
  read+write of the ACL graph itself"), so Control gates the rules through the *same* auth
  view, with **no Solid-specific branch in the write guard** (the original "`.acl` writes
  require `auth:control`" framing is realized as `auth:write` on the `.acl` graph);
- the **default graph** is never writable (pod data is never there, §2.1) — denied;
- the check runs entirely before `sparq_engine::update_in_place`, so an unauthorized target
  denies the whole update fail-closed (a multi-graph update touching one forbidden graph
  mutates nothing);
- after any permitted acl/acr/group-doc write, the view is **auto-re-materialized** (epoch
  bump → session cache dropped).

Conservative sub-cases (sound — they can only *deny* more, never permit; beaded as
follow-ups): a `DELETE`/`INSERT` template with a **variable** `GRAPH ?var` slot, or
`CLEAR`/`DROP` of `ALL`/`NAMED`, demands write on **every** graph in the store (the precise
per-solution graph-set is only known after the WHERE evaluates — a precise check is the
follow-up); and the `auth:append`-only insert-into-absent-graph edge.

## 5. L1 — engine dataset-view (the shipped spec; see §6.1 for the wiring)

Generic named-graph-subset evaluation; no Solid concepts. Mirrors Stardog's "silently drop
unreadable graphs" model (§1.3) with sparq's existing thread-local-guard idiom.

### 5.1 API (sparq-engine)

```rust
/// A zero-copy restriction of a Graph's dataset for one query execution.
pub struct DatasetView<'g> {
    pub base: &'g Graph,
    /// Visible named-graph names. Shared from the caller's session cache; the engine
    /// neither owns nor caches it. O(1) membership.
    pub named: &'g FxHashSet<Term>,         // callers hold it in an Arc; engine borrows
    pub default: DefaultGraphMode,
}
pub enum DefaultGraphMode {
    /// The store's own default graph (today's behaviour).
    StoreDefault,
    /// Empty default graph (Solid: pod data is never in the default graph).
    Empty,
    /// Phase 2 (see §5.4): default graph = set-union of the visible named graphs.
    UnionOfVisible,
}

pub fn query_view(v: &DatasetView, sparql: &str) -> Result<QueryResult, String>;
pub fn query_json_view(v: &DatasetView, sparql: &str) -> Result<String, String>;
pub fn count_view(v: &DatasetView, sparql: &str) -> Result<usize, String>;
pub fn ask_view(v: &DatasetView, sparql: &str) -> Result<bool, String>;
// (+ `_with_budget` variants; or fold both cross-cutting params into one
//  `query_opts(graph, sparql, &ExecOptions { budget, view })` — implementer's choice.)
```

### 5.2 Hook points (exact, from today's code)

New `exec::view` module modeled on `exec::budget` (thread-local + RAII guard installed by
the `*_view` entry points before evaluation):

1. **`eval_graph_named` (exec.rs ~1146)** — the only place named graphs are enumerated:
   - `GRAPH <g> { … }` (concrete, ~1177): if `!view.allows(g)` treat as the existing
     absent-graph branch (zero solutions).
   - `GRAPH ?g { … }` (~1194): `graph.named.iter().filter(|(t, _)| view.allows(t))`.
2. **Default-graph BGP/path evaluation** under `DefaultGraphMode::Empty`: short-circuit to
   zero rows **only at top-level graph scope** — `eval_graph_named` already swaps `graph` to
   the sub-`Graph` for its inner pattern, so the view guard carries an "inside GRAPH" RAII
   suspend flag; `eval_bgp`/path entry checks `view::default_is_empty()` only when not
   suspended.
3. **Dataset-clause interaction**: a query carrying `FROM (NAMED)` under a view first
   intersects with the view (`build_active` filtered by `view.allows`) — restriction must
   compose, never widen.

`GRAPH ?g {}` / absent graphs keep their existing semantics (a non-visible graph behaves
exactly like an absent one — indistinguishability is the security property).

### 5.3 Complexity & supply

- Membership check: O(1) hash per graph name; `GRAPH ?g` iteration O(|named graphs in
  store|) name-filter. Optional refinement if |store graphs| ≫ |view|: a lazily built
  `FxHashMap<Term, usize>` name→index on `Graph` lets the loop iterate the view set instead,
  O(|view|). Everything else is unchanged evaluation **in place on the existing
  sub-`Graph`s — zero decode, zero rebuild, zero copy** (vs `build_active`'s
  O(Σ|authorized graphs|) per query, §6).
- The per-(agent,app) set is computed and cached by sparq-solid (§4.3) as
  `Arc<FxHashSet<Term>>` keyed by (agent, client, mode, auth-epoch); the engine takes a
  borrow per call and holds no session state. Invalidation is entirely the caller's
  (epoch bump on re-materialization).
- Estimated size: ~60–100 LOC + tests, no data-structure changes, wasm-neutral.

### 5.4 Union-default (phase 2) — and why it is honestly hard

True `UnionOfVisible` (cross-graph joins inside ONE BGP against the merged triple set)
cannot be a zero-copy k-way merge of per-graph permutation scans today because **each named
sub-`Graph` owns a private dictionary** — ids are not comparable across graphs. Options, in
order of preference: (a) keep union-default EMULATED by the L3 per-pattern GRAPH wrap
(§4.4) — term-level joins already work across sub-graphs via LocalVocab, only duplicate-row
semantics differ; (b) a cached materialized union view, amortized over queries and
invalidated with the auth epoch (a copy, but once per ACL change instead of per query);
(c) shared-dictionary named graphs — a sparq-core refactor with global benefits (cross-graph
joins, smaller total dicts) and global risks; out of scope here, tracked as its own
follow-up. v1 ships (a); L1 phase 1 ships `StoreDefault`/`Empty` only.

## 6. v1 measured baseline (prototype, this machine — M1 MacBook Air, `--release`)

Fixture (`crates/sparq-solid` `fixture` module): synthetic pod, 864 documents + 259
containers (depth-4 tree, 1148 named graphs incl. ACLs/groups), WAC ACLs on ~10% of
containers + one resource-specific ACL, agents/groups incl. public, authenticated-only,
group, user/app-pair and deliberately narrowing/widening deep-ACL subtrees; an ACP
variant of the same tree (cumulative inheritance, allOf pair, deny-overrides, noneOf).

Correctness gate (all green, `cargo test -p sparq-solid`): WAC access matrix (depth-4
default inheritance, nearest-ACL shadowing incl. the alice-loses-team2 case,
accessTo-vs-default split, agentGroup, AuthenticatedAgent vs anonymous, deep +
resource-specific overrides, acl:origin pair, Control→ACL-doc, fail-closed anonymous);
ACP matrix (cumulative root policy at depth 4, anyOf, native (agent ∧ client) allOf
pair, deny-overrides, noneOf conditional grants); re-materialization revocation tests
(group-member removal; ACR policy swap); end-to-end same-query-three-sessions
(599 / 407 / 144 rows for alice / carol / anonymous — counts hand-derived in
tests/e2e.rs); cross-document join inside the sandbox; explicit `GRAPH <private>` and
attacker-supplied `FROM NAMED` cannot escape.

Run: `cargo run -p sparq-solid --example bench --release` (numbers below from
2026-06-11, best-of-3; fixture = 1148 named graphs / 3060 quads; "fat" variant = same
tree with 50 filler triples per document = 46 260 quads).

| measurement | value |
|---|---|
| WAC auth-view materialization (full pipeline, 1 stratum) | **1.00 s** → 3 783 auth triples (closure 15 475 facts) |
| ACP auth-view materialization (3 strata) | **1.13 s** → 6 168 auth triples (closures 10 372 / 10 388 / 16 625) |
| re-materialization after an ACL change (v1 = full re-run) | same as above (~1.0–1.1 s) — **this IS the incremental-maintenance baseline** |
| engine on FULL dataset, `GRAPH ?g` titles scan | 41 ms (864 rows) |
| v1 `query_as` (rewrite + `build_active` copy, 800 authorized graphs) | 30 ms (599 rows) |
| v1 copy cost isolated (rewritten query matching nothing) | **12 ms / query** |
| fat fixture: FULL-dataset query | 45 ms |
| fat fixture: v1 `query_as` | 83 ms |
| fat fixture: v1 copy cost isolated | **59 ms / query** |
| session graph-set, cold (AuthIndex walk + allow∖deny) | 0.30 ms |
| session graph-set, cached | 0.0006 ms |

Reading the numbers honestly:

- **Materialization is cheap and re-runs freely** (~1 s for ~1.1k graphs): "re-materialize
  on every ACL change" is a perfectly serviceable v1 maintenance story at pod scale. (One
  measured rule-authoring lesson is baked into `rules/common.n3`: a rule with TWO unbound
  join atoms made semi-naive seeding enumerate a ~2M-binding cross product — 117 s; split
  into candidate+filter rules → 0.7–1.0 s. N3 rules for this engine should keep every
  seeding direction one-side-bound.)
- **The v1 per-query copy is the real cost and it scales linearly with authorized data**:
  ~12 ms at 3k quads → ~59 ms at 46k quads (≈1.3 µs/quad + ~15 µs/graph for the dict +
  permutation rebuild). On the fat fixture the v1 path is ~1.8× SLOWER than querying the
  whole dataset with no security at all; extrapolated to a 1M-quad pod it is ~1.3 s of
  copying per query. This is the number the L1 zero-copy view (§5) must beat — its
  expected per-query overhead is O(1) hash checks per graph, i.e. effectively the
  41–45 ms unrestricted-query line.
- Session-set computation is negligible either way (sub-ms cold, ~0.5 µs cached), so the
  L1 view's `Arc<FxHashSet<Term>>` supply path adds nothing measurable per query.

D2 gate: any proposal to store authorizations in a custom (non-triple) structure must beat
these numbers — materialization, per-query, AND re-materialization — with the same
correctness suite green, or it is rejected.

### 6.1 v2 dataset-view measured (the §5 wiring, done)

The L1 zero-copy `DatasetView` merged into the engine and `sparq-solid` now routes
`query_as`/`query_json_as`/`ask_as` through it by default (`wrap_for_view` +
`query_view`, `DefaultGraphMode::Empty`); the v1 FROM-NAMED rewrite is kept as
`query_as_rewrite` (portability + differential oracle — tests/e2e.rs asserts both
paths return byte-identical SPARQL-JSON for every fixture session). Same machine,
same fixture, same bench (`cargo run -p sparq-solid --example bench --release`,
2026-06-11, best-of-3; both paths measured IN THE SAME RUN so the comparison is
honest under machine-load variance — absolute numbers swing ~1.6× between runs,
ratios stay put):

| measurement | v1 rewrite+copy | v2 dataset view | speedup |
|---|---|---|---|
| titles query, 800 authorized graphs (3 060 quads) | 28.98 ms | **18.35 ms** | 1.6× |
| per-query overhead isolated (empty pattern), 3k quads | 11.52 ms | **1.72 ms** | **6.7×** |
| titles query, fat fixture (46 260 quads) | 67.46 ms | **20.75 ms** | 3.3× |
| per-query overhead isolated, fat fixture | 43.21 ms | **1.58 ms** | **27×** |
| unrestricted FULL-dataset query, fat fixture (no security) | 33.58 ms | — | — |

Reading the numbers honestly:

- **The copy is gone and the overhead is flat**: v1's isolated overhead scales linearly
  with authorized data (11.5 ms → 43.2 ms from 3k → 46k quads); v2's is constant
  (~1.0–1.7 ms at BOTH sizes — it is not a copy at all but the `GRAPH ?__sgN`
  union-default emulation enumerating the ~800 visible graph names per wrapped
  pattern, plus the parse/serialize round-trip of the wrap).
- **Security is now cheaper than no security**: on the fat fixture the v2 restricted
  query (20.75 ms) beats the unrestricted full-dataset scan (33.58 ms) — the view
  prunes 348 graphs before they are touched — where v1 was 2× slower than
  unrestricted. The §5 prediction ("effectively the unrestricted-query line") was
  conservative.
- At 1M-quad pod scale the v1 extrapolation was ~1.3 s of copying per query; v2's
  per-query cost stays O(visible graphs) hash checks + wrap, i.e. ~milliseconds.
- Correctness gate held: the whole WAC/ACP/e2e/hardening suite passes through the
  view path, plus the byte-identical-JSON differential test against the rewrite path.

## 7. Follow-ups & gaps (explicit)

1. **L1 engine view** (§5) — DONE: shipped in the engine and wired as `sparq-solid`'s
   default query path; measured in §6.1.
2. **sparq-reason builtins** (small, in gap-priority order): `string:encodeForUri` —
   DONE (RFC 3986 / fn:encode-for-uri percent-encoding; wac.n3/acp-a.n3/acp-c.n3 pair
   and candidate minting now encode their components, the session side shares the same
   `encode_for_uri` helper, and the reserved-encoding validation is KEPT as defense in
   depth). Bead `sq-jwsp` — DONE: the multi-stratum entry point
   (`reason_n3_stratified(dict, &[src])`, in-memory closure carry with per-stratum blank
   scope + per-stratum sizes) and `log:collectAllIn` / `log:forAllIn` (scoped
   aggregation/universal quantification; with `math:memberCount` this is
   count-over-property-values) are shipped in `sparq-reason`
   (`crates/sparq-reason/tests/n3_collect_stratified.rs`). ACP pipeline adoption
   (materializer switch + collapsing strata B+C / simplifying allOf in the rules) is a
   beaded `sparq-solid` follow-up. <!-- [FABLE-5] sq-jwsp -->
3. **Incremental auth maintenance**: re-materialization is v1 (measured §6); N3-incremental
   needs derivation counting under stratified NAF — T18's counting is RDFS-only today.
4. **ACP issuer/vc/Creator/Owner**, custom ACP modes, `acl:accessToClass` — §3.6.
5. **Shared-dict named graphs** enabling true zero-copy union-default — §5.4(c).
6. Update-path enforcement (write gating + auto-re-materialization on acl/acr writes) —
   §4.4: **DONE** (sq-xor3) — `PodStore::update_as` / `update_as_acp`
   (`crates/sparq-solid/src/update.rs`); core check, deny path, and auto-re-materialization
   wired and tested (`tests/update.rs` + `*_write_enforcement_matches_grants` in
   tests/wac.rs / tests/acp.rs). Remaining (beaded follow-ups, both conservative/sound
   today): a *precise* per-solution check for variable `GRAPH ?var` template slots (today:
   require write on every store graph), and the `auth:append`-only insert-into-absent-graph
   edge. <!-- [OPUS-4.8] sq-xor3 -->
7. **Containment / `ldp:contains` view ownership** — **DECIDED: PSS-written, not
   sparq-derived** (sq-3jtd.4). A container's `ldp:contains` listing is explicit content the
   storage layer (PSS) writes in its UPDATE bodies; sparq-solid stores it as ordinary triples
   in the container's named graph (`fixture.rs`) and treats it as opaque content — it never
   derives `ldp:contains` from IRI structure, mutates/re-derives it on a write, or reads it
   into the reasoner. Containment *ancestry* is derived structurally (`solidx:parent`/
   `solidx:ancestor`, §3.2) only to drive ACL inheritance and is never surfaced as
   `ldp:contains`. Rationale: (i) avoids re-deriving a view on every write; (ii) keeps the
   §2.4 content/reasoner boundary clean — a derived listing would have to read pod content;
   (iii) the engine's atomic multi-op UPDATE (sq-ycle) is exactly the mechanism that keeps the
   PSS-written listing consistent. The invariant is pinned by
   `tests/containment_view_ownership.rs`. **Revisit only if** PSS asks sparq to own
   containment; that would be a separate, explicitly-scoped *structural-only* (IRI
   slash-semantics) derivation spike, never a content-reading one (`research/sparq-solid-scope.md`
   area 2). <!-- [OPUS-4.8] sq-3jtd.4 -->
