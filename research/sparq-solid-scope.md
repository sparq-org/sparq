# sparq-solid scope: backing a Solid server's storage/query needs (research track)

Status: **research / design scoping record** — *not* an implementation plan and explicitly
**not a cutover gate**. <!-- [OPUS-4.8] sq-3jtd / gh-55 -->

This record scopes the gaps between what `crates/sparq-solid` is **today** and what a real
Solid server would need to lean on it. The sibling **prod-solid-server** (PSS) — a production
Solid server that consumes sparq as its triplestore — is the concrete consumer this is framed
against, so the framing is conservative: per @jeswr's house rule (recorded in gh-55 / sq-3jtd),
**security-critical paths run through vetted TypeScript in PSS; sparq-solid is at most an
authorization *oracle*, never the sole gate.** Nothing here changes that. The goal is to map
*where sparq-solid could be a sound oracle* and *what is missing or research-open before it
could be one*, area by area, with honest feasibility verdicts.

It complements, and does **not** restate, two existing records:

- the architecture + design-rationale doc
  [`research/solid-access-control-design.md`](./solid-access-control-design.md) (storage model,
  WAC/ACP support matrix, the rule strata, the measured baseline, the threat model);
- the crate's user-facing surface in
  [`crates/sparq-solid/README.md`](../crates/sparq-solid/README.md) and `cargo doc -p sparq-solid`.

It also deliberately **does not** duplicate the server-side HTTP-contract work, which is
already beaded and tracked separately: gh-47 (named-graph query+update HTTP integration tests),
gh-48 / sq-ycle (`;`-separated multi-operation atomic UPDATE bodies), gh-50 (the
status/error-text contract for a transient-vs-permanent classifier). Those are `sparq-server`
HTTP concerns; this record is about the `sparq-solid` *library* surface underneath. gh-55's own
note is explicit that the HTTP-shaped outputs — 401-vs-403, `WWW-Authenticate`, the `WAC-Allow`
`user=""`/`public=""` both-keys split — **stay in PSS regardless** and are out of scope for
sparq-solid entirely.

## Scope boundary — what stays in PSS no matter what

These are PSS (HTTP/TypeScript) responsibilities, named here once so each area below can treat
them as out-of-scope:

- HTTP status semantics: 401 (unauthenticated) vs 403 (authenticated-but-forbidden),
  `WWW-Authenticate`, the `WAC-Allow` header's `user=`/`public=` key split.
- Authentication itself (Solid-OIDC / DPoP token validation, WebID resolution): sparq-solid
  takes an already-resolved `(WebID, client)` `Session` and answers a graph-set question. It
  does not authenticate.
- The HTTP request lifecycle, content negotiation, LDP `POST`/`PUT`/`PATCH` verb semantics,
  slug handling, ETags/conditional requests.
- Reconciliation of the on-disk/S3 representation against the index (PSS's existing reconciler).

Everything below is strictly the *triplestore-side authorization-and-derivation* question.

---

## 1. Write-path enforcement

### Current state (in sparq-solid)

The write path **ships** (sq-xor3, closed): `PodStore::update_as` / `update_as_acp`
(`crates/sparq-solid/src/update.rs`) authorize a SPARQL Update against the materialized auth
view *before* mutating anything, fail-closed, then apply via `sparq_engine::update_in_place`.
The model mirrors the read path: every graph an update could write is reduced to a
`(graph, need)` requirement and checked against the session's per-mode accessible set
(`Need::Write` for delete/clear/drop, `Need::Write ∨ Need::Append` for a pure insert), with
`.acl`/`.acr` writes Control-gated through the same view (the rules grant `auth:write` on a
control graph only to `acl:Control` holders — no Solid-specific branch). Default-graph writes
are always denied. On a permitted write that touched a control/group document the view
auto-re-materializes (`update.rs` `Permit.rematerialize` → `materialize_wac/acp`).

The hard case — a `DELETE/INSERT … WHERE` with a **variable** `GRAPH ?var` template slot — is
handled **precisely** for the common shape (sq-biss, closed): `resolve_var_graphs`
(`update.rs:294`) evaluates the operation's WHERE over the full store to enumerate exactly the
concrete graphs `?var` binds to, and requires write only on *those*. It is fail-closed: a
blank-node graph binding, an un-evaluable WHERE, or — critically — **any `USING`/`WITH` clause**
falls back to the conservative all-graphs wildcard (require write on *every* store graph, or
deny).

### The gap vs a real Solid server's needs

1. **The `USING`/`WITH` fallback is over-restrictive on a real PSS write shape.** sq-cnor
   already tracks the precise root cause: the engine's apply re-scopes the WHERE through
   `Dataset::build_using`, whose `named: None` (the parser's encoding of `WITH`) keeps *all*
   store named graphs, whereas a plain serialized `SELECT`'s `FROM`/`FROM NAMED` (`build_active`)
   treats `named: None` as the *empty* named set — so a serialized binding-enumeration query
   would **under-count** the `GRAPH ?g` bindings and risk a hole. Rather than reproduce
   update-side dataset semantics, sparq-solid bails to the wildcard whenever a re-scope is
   present (`update.rs:305`). The wildcard is sound but denies updates a per-solution check
   would allow — and PSS's `setAclPointer`/`putContainer` use `DELETE/INSERT … WHERE` with
   `OPTIONAL` (gh-47), which can carry a `WITH`. **This is the single most load-bearing
   write-path gap for the oracle story.** See sq-cnor for the two resolution options (expose a
   `pub build_using`/active-dataset helper from sparq-engine, or translate the `QueryDataset`
   into an explicit `FROM NAMED` list of the kept store graphs before serializing).
2. **Atomicity is the engine's, not sparq-solid's.** A multi-operation body
   (`DROP SILENT GRAPH <r> ; INSERT DATA { … }`, PSS's `putDocument`/`deleteResource` shape)
   must be all-or-nothing so a crash mid-update cannot desync `<parent> ldp:contains` from the
   child graph. sparq-solid's `update_inner` checks the whole update, then calls
   `update_in_place` once; **the atomicity guarantee lives in sparq-engine + sparq-server**
   (sq-ycle / gh-48), and the WAL-durable named-graph CLEAR/DROP needed for the `DROP …` half
   to be crash-safe just landed (sq-glw2, closed). sparq-solid relies on that; it does not
   provide it. *Cross-reference only — do not re-bead.*
3. **`auth:append`-into-absent-graph edge.** Already noted in the design doc §7 and the README
   limitations: a pure `INSERT` satisfied by `Append` into a not-yet-existing graph is a sound
   but slightly conservative corner. Minor.
4. **No precise wildcard for `CLEAR/DROP ALL|NAMED`.** This is genuinely all-graphs, so the
   conservative wildcard is correct here — *not* a gap.

### Recommended approach

- Close the `USING`/`WITH` precision gap by the **least-invasive of sq-cnor's two options**.
  Preference: translate the `QueryDataset` into an explicit `FROM NAMED <g…>` enumeration of the
  store's kept named graphs before serializing the binding-enumeration `SELECT` (keeps the fix
  inside sparq-solid, no new sparq-engine public surface). Validate it against the engine's
  `build_using` semantics with a differential test (the resolved set must equal the set
  `update_in_place` actually writes). If that proves to drift, fall back to exposing
  `pub build_using` from sparq-engine.
- Keep the atomicity and HTTP-status concerns where they already live (engine/server beads
  above); add a sparq-solid integration test asserting that a denied multi-op update leaves the
  store byte-identical (regression-guarding the fail-closed-before-apply invariant).

### Sequenced tasks

1. **(near-term, feasible)** sq-cnor — precise variable-GRAPH write check under `USING`/`WITH`.
   *Already filed; this record promotes it to the top of the write-path queue.*
2. *(new — **sq-3jtd.1**)* Differential test: for the PSS `setAclPointer`/`putContainer`
   `DELETE/INSERT … WHERE OPTIONAL` shapes (incl. `WITH`), assert the precise-resolution set
   equals the engine's actual write set across the fixture.
3. *(new — **sq-3jtd.2**)* sparq-solid test: a denied update leaves the store unchanged
   (fail-closed-before-apply regression guard, incl. the multi-op `;`-separated shape).

**Verdict: near-term feasible.** The hard precision case is already root-caused (sq-cnor); the
rest is test hardening. No research-open items here.

---

## 2. Incremental re-materialization

### Current state (in sparq-solid)

Re-materialization is **full-rebuild only**, deliberately (design doc §4.2 "Incremental
maintenance is v1-deliberately-absent", §6, §7 item 3). Any `.acl`/`.acr`/group-document write
re-runs the whole pipeline: re-assemble facts → run the N3 strata (1 for WAC, 3 for ACP) →
filter the closure → replace `<urn:sparq:auth>` → bump epoch → **drop the entire session cache**
(`PodStore::reindex`, `lib.rs:174`). The measured cost is ~1.0 s (WAC) / ~1.1 s (ACP) on the
~1.1k-graph fixture (design doc §6); this is acknowledged as "perfectly serviceable at pod
scale". The auto-trigger fires on any control-doc write, any precisely-resolved variable-graph
write (sq-nlze tracks that this over-fires when the targets are all plain content), and any
wildcard update.

There is a second, distinct notion of "re-materialization" that a Solid server needs and which
sparq-solid does **not** address at all: **derived containment / materialized views over pod
content** (e.g. `ldp:contains` listings, container membership, usage aggregates). Today
`ldp:contains` exists in sparq-solid only as *pre-supplied fixture data* — the loader stores it
as ordinary triples in the container's named graph (`fixture.rs:75-84`) and never derives,
checks, or maintains it. Containment *ancestry* is derived structurally from IRI slash-semantics
(`solidx:parent`/`solidx:ancestor`, design doc §3.2) purely to drive ACL inheritance — never
emitted as `ldp:contains`, never re-derived on a write.

### The gap vs a real Solid server's needs

1. **Auth-view incremental maintenance.** Re-running the full N3 pipeline on *every* ACL change
   invalidates *all* sessions' caches (epoch bump) — fine at pod scale, but for a large pod or a
   hot ACL it is wasteful, and gh-55 explicitly calls out "a costly full re-materialize
   invalidating all sessions on every ACL change" as a blocker to oracle status. The honest
   blocker (design doc §4.2, §7 item 3): `sparq-reason`'s exact-derivation-counting
   (`MaterializedGraph`, "T18") is **RDFS-only and does not know NAF**, and the ACP rules use
   *stratified negation-as-failure* (`log:notIncludes`, design doc §1.4 / §3.5). Counting under
   stratified NAF needs (de)derivation re-checks per stratum — a genuine open research problem,
   not an engineering task.
2. **Containment / `ldp:contains` derivation is absent.** A Solid server treats a container's
   `ldp:contains` listing as authoritative metadata that changes on *every* child create/delete.
   PSS maintains it explicitly in its UPDATE bodies (the `INSERT … GRAPH <parent> ldp:contains …`
   half of gh-48). sparq-solid neither derives `ldp:contains` from structure nor incrementally
   re-derives any view when a resource changes — there is no "materialized view of containment"
   to incrementally update. Whether sparq-solid *should* own this (vs PSS continuing to write it
   explicitly) is itself a design question.
3. **Usage / aggregate views.** PSS's `usage` query (gh-47) is `COALESCE(SUM(?size)) +
   COUNT(DISTINCT ?g)` with `FILTER(STRSTARTS(STR(?g), prefix))` — computed live per request
   today. A materialized-and-incrementally-maintained usage view would be the same class of
   problem as containment.

### Recommended approach

- **Auth-view incremental:** treat as **research-open**. The pragmatic near-term win is *not*
  full N3-incremental but **scoping the re-materialization blast radius**: (a) skip
  re-materialization when a precisely-resolved write touched no control/group doc (sq-nlze,
  already filed); (b) investigate *partial* invalidation — only drop session-cache entries whose
  authorized set could have changed, rather than the whole cache (needs a dependency map from
  control-doc → affected resource subtree, derivable from the `solidx:ancestor` closure). The
  full counting-under-NAF approach should be a separate research record, not attempted blind.
- **Containment / views:** **out-of-scope for the research track as an obligation**, but record
  the design question. Recommendation: keep `ldp:contains` as *PSS-written explicit triples*
  (current arrangement) rather than sparq-solid-derived, because (i) it avoids sparq-solid having
  to re-derive on every write, (ii) it keeps the security boundary clean (derived containment
  would have to read pod content, which §2.4 deliberately excludes from the reasoner), and
  (iii) the engine's atomic multi-op UPDATE (sq-ycle) is precisely the mechanism that keeps the
  PSS-written `ldp:contains` consistent. If sparq-solid ever *did* own containment derivation it
  would be a structural-only derivation (IRI slash-semantics, like `solidx:parent`) and a
  separate research effort.

### Sequenced tasks

1. **(near-term, feasible)** sq-nlze — skip needless re-materialization after precise
   variable-GRAPH updates with no auth-doc/group target. *Already filed; reduces blast radius
   cheaply.*
2. *(new, research-open — **sq-3jtd.3**)* Research record: incremental auth-view maintenance
   under stratified NAF — survey counting/DRed/Differential-Dataflow approaches,
   partial-cache-invalidation via the `solidx:ancestor` dependency map, and whether either beats
   the measured ~1 s full rebuild at realistic pod scale. (Depends conceptually on sq-jwsp's
   `reason_n3_stratified` work.)
3. *(scoping only, no obligation — **sq-3jtd.4**)* Record the `ldp:contains` ownership decision
   (PSS-written vs sparq-derived) — captured in this doc; the bead tracks the decision and a
   structural-derivation spike *only if* PSS later wants sparq to own it.

**Verdict: auth-view incremental is research-open** (the NAF-counting blocker is real and
honest); **blast-radius reduction is near-term feasible** (sq-nlze + partial invalidation);
**containment/view derivation is out-of-scope-for-now** (kept PSS-side by deliberate design).

---

## 3. Vocabulary gaps

### Current state (in sparq-solid)

sparq-solid understands (verified against `rules/*.n3`, `loader.rs`, design doc §3.6):

- **WAC** (`http://www.w3.org/ns/auth/acl#`): `acl:Authorization`, `acl:accessTo`,
  `acl:default` (with nearest-ancestor effective-ACL discovery), `acl:agent`, `acl:agentClass`
  (`foaf:Agent` = public, `acl:AuthenticatedAgent`), `acl:agentGroup` (+ `vcard:hasMember`
  resolution, the group document loaded as a graph), `acl:origin` (as a minted pair principal),
  `acl:mode` ∈ {`acl:Read`, `acl:Write`, `acl:Append`, `acl:Control`} with Control→ACL-resource
  semantics.
- **ACP** (`http://www.w3.org/ns/solid/acp#`): `acp:accessControl`, `acp:memberAccessControl`
  (transitive/cumulative inheritance), `acp:apply`, `acp:allow`/`acp:deny` (deny-overrides),
  `acp:allOf`/`acp:anyOf`/`acp:noneOf`, `acp:agent` / `acp:client` matchers incl.
  `acp:PublicAgent`, `acp:AuthenticatedAgent`, `acp:PublicClient`.
- **LDP** (`http://www.w3.org/ns/ldp#`): `ldp:contains` is *stored and queryable* but **not
  understood** — it is fixture data, never derived/validated (see area 2).
- **vcard** `http://www.w3.org/2006/vcard/ns#hasMember`; **foaf** `foaf:Agent`.

### The gap vs a real Solid server's needs

Documented in design doc §3.6 / §7 item 4 and gh-55, all currently **missing**:

| Vocabulary term | Spec | Status | Shape of the work |
|---|---|---|---|
| `acl:accessToClass` | WAC (extension) | missing | needs a class-membership join over pod content — crosses the §2.4 reasoner/content boundary; non-trivial |
| `acp:CreatorAgent` / `acp:OwnerAgent` | ACP | **done ([OPUS-4.8] sq-3jtd.5)** | per-resource creator/owner WebIDs the trusted caller supplies via `AccessProvenance` + `materialize_acp_with`; loader synthesizes `<r> solidx:creator\|owner <w>` from THAT map only (never pod content, §2.4); resource-scoped grant (`urn:sparq:scand?…&res=R`) so a creator of `R1` is never granted `R2`; THREE-VALUED — on a resource with no recorded creator/owner the matcher is undecided in every role, so an allow that needs it does not apply and a deny that needs it does. Composes under `allOf` with a second, independent concrete-WebID `acp:agent` matcher ([OPUS-4.8] sq-az1b): the two agent constraints intersect — WebID==creator/owner is granted (resource-scoped), a distinct fixed WebID grants nobody (correct-by-soundness) |
| `acp:issuer` | ACP | missing | "same shape as `acp:client`" (design doc §3.6) — extend the principal from a pair to a triple `(agent, client, issuer)`, combinatorial blow-up noted |
| `acp:vc` | ACP | missing | Verifiable-Credential-gated matcher — needs VC verification, genuinely large; ties to the sparq ZK/VC estate |
| custom ACP mode IRIs | ACP | partial | the design supports *any* mode IRI; the prototype maps only the 4 standard ones — the auth-view predicate space is fixed to read/write/append/control |
| `acl:agentGroup` indirection depth | WAC | partial | one-hop `vcard:hasMember` resolved; nested groups / `vcard:hasMember` chains not closed over |

LDP beyond `ldp:contains` (`ldp:BasicContainer`, `ldp:RDFSource`, membership-resource
predicates) is **not modeled** at all — but a Solid server typically writes those as plain
metadata, so this is "store-and-serve", not "understand" (see area 2's containment decision).

### Recommended approach

Prioritize by *what PSS actually needs as an oracle*, conservatively:

- **`acp:CreatorAgent`/`OwnerAgent` first** — these are common in real ACP pods and the only
  missing piece is a *loader-synthesized fact* (`<r> solidx:creator <webid>`), which PSS already
  knows (it created the resource). Low-risk, high-value, stays inside the §2.4 boundary (the
  creator fact is structural metadata, not pod content the writer controls — but PSS must supply
  it through a trusted channel, not from the resource graph itself; this is a security note worth
  pinning).
- **`acp:issuer` second** — mechanical extension of the pair-principal to a triple-principal;
  the combinatorial cost is bounded by candidate-enumeration (same as the existing client
  dimension). Feasible but touches the principal-encoding everywhere (`urn:sparq:pair?…` →
  `urn:sparq:triple?…`), so it is a non-trivial refactor.
- **`acl:accessToClass`, `acp:vc`, custom modes, nested groups: research-open / lower priority** —
  `accessToClass` and `acp:vc` both cross into "the reasoner must look at typed content / verify
  a credential", which is exactly what §2.4 walls off; doing them soundly is a design effort, not
  a vocab addition.

### Sequenced tasks

1. **(near-term, feasible — **sq-3jtd.5**)** `acp:CreatorAgent`/`OwnerAgent` support —
   loader-synthesized `solidx:creator`/`solidx:owner` facts (supplied by the trusted caller, not
   read from pod content) + the ACP stratum-A/C rules to grant on them.
2. **(feasible, larger — **sq-3jtd.6**)** `acp:issuer` — extend the principal model from pair to
   triple; sized as a follow-up because it touches all minting + the session expansion.
3. *(research-open — **sq-3jtd.7**)* `acl:accessToClass`, `acp:vc`, nested `acl:agentGroup`
   chains, custom ACP mode IRIs — **the per-gap design notes this bead asked for now exist:
   [`research/solid-vocab-gaps-design.md`](./solid-vocab-gaps-design.md) [OPUS-4.8]**. Net of
   that analysis: `acl:accessToClass` and `acp:vc` stay research-open (the former breaches the
   §2.4 content boundary unless PSS supplies trusted class facts; the latter needs the ZK/VC
   estate's verifier and a verify-in-PSS / match-in-rules split); **custom ACP mode IRIs is
   near-term feasible** (no soundness barrier — a bounded `Mode`-enum→IRI refactor with a
   public-API/`SKILL.md` cost, demand-driven); **nested `acl:agentGroup` chains is feasible as
   an N3 transitive closure** but gated on first confirming recursive expansion is the CSS/ESS
   reference behaviour (do not over-grant) and bounding expansion to loaded group docs.

**Verdict:** `CreatorAgent`/`OwnerAgent` and `issuer` are **near-term feasible** (landed);
within sq-3jtd.7, **custom-modes and nested-groups are near-term feasible** (the latter gated on
a conformance decision), while **`accessToClass`/`vc` remain research-open** (they breach the
content/reasoner boundary or need verification machinery) — see the design notes for the per-gap
prerequisites.

---

## 4. WAC/ACP conformance

### Current state (in sparq-solid)

There is a **strong correctness suite but no external-conformance story** (design doc §6,
`crates/sparq-solid/tests/`). The crate has `tests/wac.rs`, `tests/acp.rs`, `tests/e2e.rs`,
`tests/update.rs`, `tests/hardening.rs` — a hand-derived access matrix over a synthetic
depth-4 fixture (nearest-ACL shadowing, accessTo-vs-default, agentGroup, AuthenticatedAgent vs
anonymous, deep overrides, origin pairs, Control→ACL-doc; ACP cumulative inheritance, anyOf,
allOf `(agent ∧ client)` pairs, deny-overrides, noneOf conditional grants; the write-path
enforcement matrix; fail-closed anonymous; attacker-supplied `FROM NAMED` cannot escape). These
are *internally* authored against the agent's reading of the specs — they are **not** the
community/normative conformance suites, and no test runs against an external corpus.

### The gap vs a real Solid server's needs

A real Solid server's authorization layer is judged against the **normative specs and the
community test harnesses**, not a bespoke matrix:

- WAC — W3C/Solid spec: <https://solidproject.org/TR/wac> (and the editor's draft).
- ACP — W3C/Solid spec: <https://solidproject.org/TR/acp>.
- The **Solid Conformance Test Harness** (CTH) and the Solid test suites
  (<https://github.com/solid-contrib/conformance-test-harness>,
  <https://github.com/solid/specification-tests>) exercise a *server* over HTTP, including the
  authorization behaviours.

The honest gaps:

1. **No mapping from sparq-solid's library verdicts to a CTH run.** The CTH drives an HTTP
   server and asserts on HTTP-shaped outputs (status codes, `WAC-Allow`) — which gh-55 says
   **stay in PSS**. So sparq-solid *cannot directly* run the CTH; it can only be conformance-
   tested *through PSS*, or against a *library-level* oracle derived from the spec test fixtures.
2. **No oracle parity check.** The credible conformance story for an *oracle* is: take the
   authorization-relevant fixtures from the community suites (the `.acl`/`.acr` documents and
   their expected allow/deny decisions per `(agent, mode, resource)`), feed them to
   `PodStore`/`AuthIndex`, and assert the materialized verdict matches the suite's expected
   decision — a **library-level differential conformance** harness. This does not exist.
3. **No differential oracle against a reference WAC/ACP engine.** Community Solid Server (WAC/ACP
   in JS) and Inrupt ESS (ACP) are reference evaluators (design doc §1.3). A differential test —
   same ACL corpus, compare sparq-solid's decision to CSS's — would catch semantic drift the
   hand-derived matrix cannot.

### Recommended approach

- **Do not attempt to make sparq-solid pass the CTH directly** — it has no HTTP surface and the
  CTH asserts on HTTP outputs that are PSS's by design. That route is **out-of-scope**.
- **Build a library-level conformance oracle harness** instead: ingest the authorization
  fixtures from the Solid spec-tests / CTH corpus as N-Quads (one named graph per document, the
  storage model already matches), materialize, and assert `AuthIndex::accessible` /
  `update_as` decisions equal the corpus's expected `(agent, client, mode, resource) → allow/deny`.
  This is the realistic, achievable conformance signal for an oracle.
- **Add a differential check against CSS** (or ESS for ACP) as an *aspirational* second oracle:
  run the same corpus through CSS's authorizer and diff decisions. Higher setup cost (JS
  toolchain, pinning CSS), so a separate, lower-priority effort.

### Sequenced tasks

1. **(DONE — **sq-3jtd.8** [OPUS-4.8])** WAC harness landed: `sparq_solid::wac_conformance`
   (`src/wac_conformance.rs`) is a table-driven scenario runner over the WAC engine
   (`materialize_wac` + `AuthIndex::accessible`, with a `run_via_podstore` twin proving the
   `PodStore` method-form path), sharing the decision/expectation/report vocabulary
   (`conformance::{Decision, Expect, ScenarioReport}`) with the ACP harness. The scenario
   corpus in `tests/conformance_wac.rs` isolates each WAC construct: `acl:agent` on the
   resource's own ACL (`acl:accessTo`), `acl:agentClass foaf:Agent` (public) /
   `acl:AuthenticatedAgent`, `acl:agentGroup` via `vcard:hasMember`, `acl:default`
   inheritance vs `acl:accessTo`, **nearest-ACL shadowing** (the key WAC difference from
   ACP's cumulative inheritance), resource-specific override, the `acl:origin` (user, app)
   pair, mode independence, `acl:Control` governing the `.acl` document, fail-closed, and
   the multi-agent union. **Decision recorded (same as ACP):** scenarios are derived from
   the WAC spec's normative semantics and declared as data (an `AclBuilder` corpus + an
   expected-decision table), rather than vendoring the live JS spec-tests/CTH corpus over
   HTTP — that route has no library entry point here (it asserts on HTTP outputs, PSS's by
   design). The table-driven corpus is the natural input for the aspirational CSS
   differential oracle (item 3).
2. **(DONE — **sq-3jtd.9** [OPUS-4.8])** ACP harness landed: `sparq_solid::conformance`
   (`src/conformance.rs`) is a table-driven scenario runner over the ACP engine
   (`materialize_acp` + `AuthIndex::accessible`), with the scenario corpus in
   `tests/conformance_acp.rs` (matchers incl. `acp:PublicAgent`/`acp:AuthenticatedAgent`,
   `acp:allOf`/`acp:anyOf`/`acp:noneOf`, the (user, app) pair, deny-overrides, cumulative
   ancestor inheritance, mode independence, fail-closed). **Decision recorded:** scenarios are
   derived from the ACP spec's normative semantics and declared as data (an `AcrBuilder` corpus
   + an expected-decision table), rather than vendoring the live JS spec-tests/CTH corpus over
   HTTP — that route has no library entry point here (it asserts on HTTP outputs, PSS's by
   design). The ACP harness defined the in-crate pattern; the WAC harness (sq-3jtd.8, item 1
   above) now mirrors it, sharing the `conformance::{Decision, Expect, ScenarioReport}`
   vocabulary (the `ScenarioReport::build` core takes a `FnMut` "granted?" oracle, so both
   harnesses differ only in which auth view they consult).
3. *(research-open / aspirational — STILL NOT STARTED)* Differential oracle against CSS (WAC/ACP):
   run the same corpus through the JS reference evaluator and diff decisions. Deferred for the
   JS-toolchain cost; the table-driven corpus in `tests/conformance_acp.rs` is the natural input
   for it. Tracked as a follow-up.

**Verdict: library-level conformance is near-term feasible** (the storage model already matches
the corpus shape); **CTH-over-HTTP conformance is out-of-scope** for sparq-solid (it is PSS's,
through PSS); **the CSS differential oracle is research-open/aspirational** (worth it, but JS
toolchain cost).

---

## Summary of feasibility verdicts

| Area | Near-term feasible | Research-open | Out-of-scope (now) |
|---|---|---|---|
| 1. Write-path enforcement | `USING`/`WITH` precision (sq-cnor) + test hardening | — | atomicity (engine/server: sq-ycle/gh-48), HTTP status (PSS) |
| 2. Incremental re-materialization | blast-radius reduction (sq-nlze, partial cache invalidation) | full auth-view incremental under stratified NAF | `ldp:contains`/usage view derivation (kept PSS-side by design) |
| 3. Vocabulary gaps | `acp:CreatorAgent`/`OwnerAgent`, `acp:issuer` | `acl:accessToClass`, `acp:vc`, custom modes, nested groups | LDP container metadata beyond `ldp:contains` |
| 4. WAC/ACP conformance | library-level decision-parity harness (WAC then ACP) | CSS differential oracle | CTH-over-HTTP (PSS's, through PSS) |

## Follow-up beads filed by this record

All parented under **sq-3jtd**, labelled `area:sparq-solid`:

| Bead | Area | Verdict |
|---|---|---|
| sq-3jtd.1 | write-path | near-term — differential test: precise var-GRAPH set = engine write set (PSS `DELETE/INSERT…WHERE OPTIONAL`, incl. `WITH`) |
| sq-3jtd.2 | write-path | near-term — regression test: denied update leaves store byte-identical (incl. multi-op `;` body) |
| sq-3jtd.3 | re-materialization | research-open — incremental auth-view maintenance under stratified NAF (deps sq-jwsp) |
| sq-3jtd.4 | re-materialization | out-of-scope-now — `ldp:contains`/containment view ownership decision (PSS-written vs sparq-derived) |
| sq-3jtd.5 | vocab | near-term — `acp:CreatorAgent`/`OwnerAgent` via trusted loader facts |
| sq-3jtd.6 | vocab | feasible-larger — `acp:issuer` (pair→triple principal) |
| sq-3jtd.7 | vocab | research-open — `acl:accessToClass`, `acp:vc`, custom modes, nested groups |
| sq-3jtd.8 | conformance | near-term — library-level WAC decision-parity harness |
| sq-3jtd.9 | conformance | feasible-after-WAC — library-level ACP decision-parity harness (+ CSS differential, aspirational) |

## Cross-references (do not duplicate)

- **Already-filed beads** this record relies on / promotes: **sq-cnor** (write-path `USING`/`WITH`
  precision), **sq-nlze** (skip needless re-materialization), **sq-jwsp** (sparq-reason N3
  builtins incl. `reason_n3_stratified`), **sq-glw2** (WAL-durable CLEAR/DROP — *closed*),
  **sq-ycle** (atomic multi-op UPDATE — server), **sq-biss**/**sq-xor3** (write path — *closed*).
- **Server HTTP-contract work — explicitly NOT duplicated here:** gh-47 (named-graph HTTP
  integration tests), gh-48/sq-ycle (multi-op atomic UPDATE), gh-50 (status/error-text contract).
- **Parent issue:** gh-55 (this scoping is its deliverable); parent bead **sq-3jtd**.
