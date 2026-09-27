# Capability-based model routing — sparq declares requirements, the registry owns the models [OPUS-5]

**Status:** design record, for maintainer review. **No implementation, no child beads yet** — the
requirement vocabulary in §3 is the crux and everything downstream depends on it, so it is settled
first. Requested 2026-07-26: *"I would also like for a longer term plan where … there is no mapping to
specific models via a toml file in sparq; and this is handled only in the agent account registry based
on requirements defined by the sparq setup so that it is easier to continue to migrate and update
models that are used."*

## 0. Grounding rules (binding on every claim below)

1. Every structural claim was checked against the tree on **2026-07-26**: sparq at `origin/main`
   (`b5548a0d4`) and `jeswr/agent-account-registry` at `master` (after PR #707 merged). Where a claim
   comes from operational observation rather than the tree, it says so **in the sentence**.
2. Three premises in the request brief turned out to be **wrong against the tree**. They are corrected
   in §2 rather than quietly carried forward, because two of them change the design.
3. No hard-coded performance numbers (repo hygiene). Counts below are dated corpus facts.
4. This record does not restate content from
   `research/agent-context-sharing.md` or
   `research/knowledge-management-strategy.md`; the one place it touches
   them is §9.4, and it references rather than duplicates.

## 1. The measured problem

**A model migration is not a one-file edit today, and the alias layer does not make it one.**

`orchestration/routing.toml` already interposes *aliases* (`haiku`, `sonnet`, `opus`, `fable`,
`opus5`, `terra`, `sol`, `luna`) between routing decisions and provider ids, and the `[models]`
catalog is the only place those aliases bind to a `provider_model`. That looks like the indirection
we want. It is not, for two reasons.

**Reason one: most consumers bypass the table.** A grep for a concrete provider model id
(`claude-*-N` / `gpt-5.6-*`) over the tracked tree hits **37 files**. They fall into three classes:

| class | files | migration obligation |
|---|---|---|
| Routing decisions that must be edited on a migration | ~25 — `orchestration/routing.toml`, 16 × `.claude/agents/*.md`, 3 × `.claude/workflows/*.js`, `scripts/fable/detect-tier.sh`, `.roborev.toml`, `.claude/settings.json`, `AGENTS.md`, `AGENTS-worker-core.md` | **the real blast radius** |
| Product code that calls an LLM (`sparq-nlq`, `sparq-kb` literature extraction, `genai-retrieval` skill) | 5 | **out of scope** — a library's default model is a product decision, not fleet routing |
| Historical provenance markers and dated measurements (`research/*`, `bench/*`) | 7 | **must not be rewritten** — `AGENTS.md` § *Model provenance*: existing markers are accurate HISTORY |

So the number that matters is roughly **25 files across two repos**, and the alias catalog covers
exactly one of them. The `.claude/**` surfaces pin full ids directly (`model: claude-opus-5` in 16
agent configs; `TIER = { fable: { model: 'claude-opus-5', … } }` in
`.claude/workflows/fable-architect-drain.js`; `const TOP_TIER_MODEL = 'claude-opus-5'` in two more).
The Opus 5 rollout on 2026-07-24 had to touch all of them, and the migration currently in flight
(Fable/Opus-4.8 → Opus 5, Sonnet/Haiku → sol for docs) is touching them again.

**Reason two: aliases are not stable names for capabilities, they are stable names for models.**
`opus` means *Opus 4.8*, permanently. When Opus 5 shipped, the fix was to add a **new** alias
`opus5` and demote `opus`/`fable` to tail fallbacks — which is exactly the migration cost we are
trying to remove, performed on the indirection layer itself. A capability name (`adversarial`)
survives a model change; a model nickname (`opus`) does not.

**The alias-lag argument, stated precisely.** The harness's *documented* short alias `opus` resolves
to Opus 4.8 even after Opus 5 shipped, so `.claude/agents/*.md` had to pin the full id
`claude-opus-5` (probe-verified via `modelUsage`, sparq PR #3763). This is the cleanest possible
demonstration of the thesis: **a provider-owned alias tracks the provider's release schedule, not our
routing intent.** If we want "the strongest model we are allowed to run adversarial security content
on" to keep meaning that across releases, we have to own the name, and we have to own it in one
place.

## 2. Premise corrections (checked against the tree)

Three statements in the brief are inaccurate as written. All three are load-bearing.

**2.1 — The registry is PUBLIC, not private.** `agent-account-registry/README.md:1` is literally
`# agent-account-registry (public)`; `gh repo view` reports `PUBLIC`. The README explains why: public
repos get free unlimited Actions minutes. What actually never crosses is **token values** (held as
encrypted GitHub secrets, referenced only by a `secret_ref` *name*) and **account PII** (emails).
Account **handles, per-account limits, live-usage probing, and the whole selection algorithm are
public by design** — `policy/repos.toml` enumerates the pool (`account_pool = ["acct01", …]`) in the
open.

The registry README still carries a stale line at `:531` — *"Public codebases request a worker and
receive an opaque claim; they never see account internals"* — that predates the public flip and
contradicts `:1-9`. sparq's own `research/issue-native-orchestration.md` has the same drift (*"No
public repo ever holds account handles, limits, usage, tokens, or selection logic"*).

**Design consequence:** this design must **not** be justified by "the registry is private". The
correct justification is ownership, not secrecy: *the registry owns the catalog and the solver
because that is where account capacity, usage probes and lease state already live.* The privacy
property we do rely on is narrow and real — credential values and PII never cross — and the design
below preserves it unchanged (the resolver returns a `secret_ref`, never a token).

**2.2 — A drifted chain does not currently park to `needs:user`.** On `master` today, a mint-vs-adopt
disagreement is a bare `SystemExit` in `review-fix.yml`'s adopt validator (`:668`, `:755`, `:759`) —
the job fails. The rejection classifier, the `adopt_disagreement` job and the park path are in
registry PR **#702, still open**. What *does* park today is a different axis: review-round budget
exhaustion, and it parks under the **machine-owned** `park_class="capacity"` pair
(`review:parked` / `status:parked`), not `needs:user`. `needs:user` is reserved for corrupt or
forged round markers.

**Design consequence:** the "requirement drift fails closed" guarantee this design needs is **not
already in place**; it lands with #702 or with this work. §5 treats it as work to do, not as an
existing backstop.

**2.3 — sparq's `model_chain` lists are already half-dead.** For the **review and fix lanes**, the
registry ignores them: `dispatch-claim.py:224-227` says the chain *"is computed HERE, never through
policy-resolve.resolve() (whose role=review row is always [opus]); resolve() supplies
account_pool/caps/gate/arm only."* Those lanes use registry-owned tables
(`REVIEW_CHAIN`, `FIX_CHAIN`, `worker-pr.ESCALATION_LADDERS`). sparq's routing table contributes
three things to those lanes: the `[models]` catalog (used at `review-fix.yml:505` to filter aliases
to those with a resolvable `provider_model`), the union of `[[route]].match_labels` keywords (fed to
the arm-side security classifier via `policy_resolve.routing_security_keywords`, fail-closed), and
the `role → agent` mapping. Only the **worker/impl lane** consumes `resolved["model_chain"]`
(`dispatch-claim.py:4092`, `:4201`, `:4228`).

**Design consequence — and it is a gift.** Cross-provider review independence is **already** expressed
in the registry as a relation over the implementer's provider, not as a model name, and sparq already
ships one genuinely requirement-shaped declaration (`match_labels` security keywords) that the
registry consumes. We are not inventing a pattern; we are generalising one that exists and finishing a
migration that is half done.

## 3. The requirement vocabulary (the crux)

### 3.1 The design constraint

Too coarse and routing is useless; too fine and it is model names with extra steps. The discipline
that keeps it honest: **a dimension is admitted only if a routing decision that exists today cannot be
expressed without it.** Each dimension below cites the decision. §3.4 lists what was rejected and why.

A second discipline, which matters more than it looks: **do not dress an ordinal decision in
capability language.** Most of this project's routing is genuinely ordinal — *"assign the cheapest
tier that can do the job soundly"* is written into the architect brief. Inventing
`can_write_documentation` / `can_implement_rust` predicates to describe a cost floor would produce
labels nothing enforces, which is precisely the decay mode §6 is about. So the vocabulary is **one
ordinal dimension plus the small number of genuinely non-ordinal predicates**, and no more.

### 3.2 The closed set — five fields, three of which are capabilities

| field | kind | values | meaning |
|---|---|---|---|
| `min_rank` | ordinal capability floor | `mechanical` < `standard` < `frontier` < `adversarial` | the **minimum** capability class. Not an exact match: resolution admits everything at or above the floor, and never anything below it. |
| `code_authorship` | non-ordinal capability predicate | `true` \| `false` | the task produces code or configuration diffs (as opposed to prose only). |
| `adversarial_safe` | non-ordinal capability predicate | `true` \| `false` | the model **completes** security / cryptography / adversarial content without a mid-run safeguard downgrade. |
| `independence` | relation (not a capability) | `none` \| `distinct-from-implementer` \| `provider:<id>` | a constraint on the model **relative to another actor in this task**. |
| `on_unsatisfiable` | failure policy (not a capability) | `defer` \| `park` \| `needs-user` | what happens when nothing satisfies the requirement. |

Being precise that only three of the five are capabilities is not pedantry — it is what stops the
vocabulary from growing. `independence` and `on_unsatisfiable` are *not* properties of a model and can
never be answered by a catalog lookup; conflating them with capabilities is how a routing schema turns
into a grab-bag.

### 3.3 Justification, one real decision per dimension

**`min_rank`** — four values, because the tree makes exactly four distinct cuts:

- `mechanical` — the mechanical-verify agent is `model: haiku`; the PKG natural-language tool is
  `model: haiku`. **Corrected 2026-07-26 (see §3.5):** this cut originally also cited
  `[[route]] role = "docs"` as haiku-led. That is no longer true — `463ae09cf` (#4211) moved the
  docs route to `model_chain = ["sol", "terra", "opus5"]`, so `mechanical` now rests on the
  `.claude/agents` pins alone.
- `standard` — the default impl lane; `sparq-rust-impl` is `model: sonnet`. (Still true at
  `origin/main`; PR #4331, unmerged as of this writing, would make the impl *chain* `["opus5"]`
  while leaving that frontmatter pin at `sonnet` — the two-surfaces-disagree problem this record
  is about.)
- `frontier` — `[[route]] role = "ci"` is deliberately frontier-only, and `routing-validate.py`
  enforces it structurally: `SUB_FRONTIER = ("sonnet", "haiku")` → *"role:ci chain contains
  sub-frontier model"*. The maintainer's standing rule (2026-07-17, restated in the registry's
  `policy/repos.toml`) is that frontier-tier models author **all** CI/infrastructure work.
- `adversarial` — the security `match_labels` override routes to the top tier and sets
  `escalate = true`.

Why `adversarial` is a **separate rank** and not just "frontier plus a predicate": the two happen to
coincide today, and collapsing them would be defensible if `adversarial_safe` were the only
difference. It is kept separate because the rank floor is what the escalation ladder walks (§7).
Keeping four ranks costs nothing and keeps the ladder expressible.

**Corrected 2026-07-26 (see §3.5):** this paragraph originally also justified the split by claiming
`escalate = true` is *"attached to that rank and to no other"*. That was already false when written.
At `origin/main` the flag appears on four routes — the security `match_labels` override
(`orchestration/routing.toml:108`), `role = "research"` (`:213`), `role = "review"` (`:234`) and
`role = "soundness"` (`:240`) — and `role:research` is not an adversarial lane. The split does not
need that claim, so it has been dropped rather than repaired.

**`code_authorship`** — this is the one predicate that is provably **orthogonal to rank**, which is
why it cannot be folded in. `terra` and `sonnet` are `docs_only`, enforced structurally in three
places (`route-resolve.validate_routing`, `review-fix.yml:493-496`, `worker-pr.py`), and
`worker-pr.py:211-226` states it explicitly: *"terra and sonnet are DOCS-ONLY models … structurally
excluded from every ladder"* — they sit **outside** the capability order `opus < luna < fable < opus5
< sol` entirely. A rank floor cannot express "sonnet-class, and permitted to write prose, and not
permitted to write code". Today that fact is encoded as a **model-name blocklist** repeated in three
files. As a requirement it is one declarative field, and the blocklist becomes a catalog property.

**`adversarial_safe`** — this is the dimension the maintainer specifically asked for, and the one that
most clearly cannot be a model name.

*Honest statement of the evidence.* The operational observation is that **Fable downgraded mid-run on
security/crypto content**, which is why security, ZK, MPC and adversarial work is routed to the Opus
line by design. That observation lives in the maintainer's operating notes; I could **not** find an
in-tree record of it, and I am not going to call it "measured in this repo" on that basis. What the
tree *does* corroborate is that a run's actual model differing from the dispatched model is a real,
designed-for class of event: `AGENTS.md:164` requires every agent brief to derive its provenance
marker from the harness's `--model` flag **at dispatch time**, precisely so that *"a worker executing
with the actual model (e.g. a downgraded Fable 5 session) produces correct attribution … not false
markers"*. The provenance regime exists because the running model is not always the requested one.

Either way the conclusion is the same and is the strongest argument in this record: **a routing
decision of the form "do not send security content to a model that will not finish it" is a property
of the model, discovered at runtime, and no model name can carry it.** `opus5` does not mean
"safe on adversarial content" — it means "Opus 5". If Opus 6 ships and is worse on this axis, the
alias tells us nothing; the predicate, backed by the runtime check in §6.2, tells us immediately.

**`independence`** — the maintainer's second named example, and the clearest case that a requirement
is strictly more expressive than a name. The registry's `REVIEW_CHAIN` is keyed by the **implementer's**
provider and yields the inverse: `{"anthropic": ["sol", "luna"], "openai": ["opus5", "opus", "fable"]}`.
No single model name can express *"a different provider from the one that authored this"* — the answer
depends on a fact about another actor in the same task. It is already a relation in the registry; this
design gives sparq a way to **say** it instead of leaving it implicit.

The `provider:<id>` value covers the `role = "site"` route (openai/codex leads, on *"original-builder
ownership"* grounds — codex built the registry dashboard). **This one is honestly not a capability**:
it is a provenance preference. It is admitted because it is a live decision that must be expressible,
but the schema requires it to carry a mandatory `because = "<reason>"` string so it can never
masquerade as a capability claim, and so it is auditable when the reason expires.

**`on_unsatisfiable`** — a real, deliberate, *three-way* distinction already made in the tree:

- `needs-user` — the security/review/soundness routes set `escalate = true`, whose consumer contract
  is *"on chain-exhaustion, label needs:user"*.
- `defer` — the `role = "ci"` route deliberately does **not** set it, and says why:
  *"NOT `escalate = true`: that flips a starved item to needs:user (human); this rule wants
  defer+retry."*
- `park` — budget exhaustion parks under the machine-owned capacity class (§2.2).

Making the failure policy an explicit field rather than a boolean named `escalate` is a small
improvement that pays for itself in §5: it removes the current situation where "what happens when we
run out" is inferred from a flag whose name describes only one of the three outcomes.

### 3.4 Deliberately rejected dimensions (and the admission rule)

The brief suggested several dimensions that the tree does not support. Adding them now would create
labels nothing enforces.

- **`long_context`** — no routing decision in the tree keys on context length. Opus 5 runs with a 1M
  window and the architect and reviewer roles are context-hungry, but no rule anywhere says "route
  this here *because* it is long". Rejected until a decision needs it.
- **`tool_use_fluency`** — same: no rule keys on it. It is also the hardest of all these to probe
  honestly, which makes it the most likely to decay into a label.
- **`docs_writing_quality`** — the brief lists this, and the tree contradicts it. The docs route
  leads with the **cheapest** model (`haiku`), i.e. the docs decision as encoded today is a cost
  floor, not a quality floor. See open question **Q1** — the migration currently in flight
  (Sonnet/Haiku → sol for docs) points the other way, and until that is resolved the honest position
  is that we do not know which of the two this decision is.
- **`latency_class` / `cost_class`** — already registry-owned and already per-target
  (`worker_timeout_minutes`, `max_concurrent`, `usage_safety_margin`). sparq should not restate them;
  cost preference is captured by "resolution walks the satisfying set cheapest-first" (§4.3).
- **`harness`** (`claude` vs `codex`) — derivable from provider, and its consequences (credential
  format, CLI flags, the `terra` empty-`provider_model` sentinel) are entirely registry-internal.

**Admission rule for a sixth dimension.** A new dimension is admitted only when all three hold:
(a) there is a **live routing decision** on which at least two catalog models differ; (b) the decision
**cannot** be expressed with the existing five; and (c) there is a **probe or a runtime observation**
that can verify the property (§6). Without (c) it is a label, and §6 explains why labels rot.

### 3.5 Update, 2026-07-26 — what moved under this record after it was written

This record's grounding commit was `b5548a0d4`. Three tree-facts moved the same day; the corrections
are inlined above, and the direction of travel is worth recording because it bears on §5.

1. **`escalate = true` now carries two incompatible failure policies.** On the security route its
   in-tree comment is *"consumed by dispatch: on chain-exhaustion, label needs:user"*
   (`orchestration/routing.toml:108`). PR #4331 attaches the same flag to `role:impl` with the
   explicitly opposite contract — capacity starvation routes to a self-lifting machine park and
   **never** `needs:user`. One boolean, two meanings, discriminated only by which route it sits on.
   This is the concrete case for §3's `on_unsatisfiable` being an explicit enumerated field rather
   than a boolean whose name describes one of three outcomes.
2. **Q1 (§11) has an answer in the tree.** The docs route moved off haiku/sonnet onto `sol`
   (`463ae09cf`, #4211). The docs decision is a **quality** floor, not the cost floor §3.4 assumed
   when it rejected `docs_writing_quality`. §3.4's rejection should be re-derived, not assumed.
3. **The thesis acquired a dated worked example.** #4331 reports that the `area:gui` carve-out
   (#4211) fires only when the resolved chain *already contains the string* `sol`, so a single-rung
   `["opus5"]` impl chain silently disarms it — and, in that PR's own words, *"it has no symptom"*,
   because both resolvers agree on the wrong answer and the cross-resolver agreement harness reports
   nothing. A rule keyed on a model **name** went inert when the model list changed. That is
   precisely the coupling §1 argues against.

Honesty note in the other direction: registry PR #742 records that `parse_preferences` **refuses** a
`[[chain_preference]]` block containing an unknown field, so a sparq table declaring a field the
registry does not yet know is a total dispatch outage, not a degraded carve-out. That is a real
instance of the cross-repo lockstep cost §10 already concedes, and it raises — not lowers — the bar
on §9's migration ordering.

## 4. Where the boundary sits, precisely

### 4.1 What crosses today

The registry **pulls**; sparq never calls the registry with a payload. `review-fix.yml` has a step
*"Checkout public target for protected routing data (pinned, no persisted token)"* which checks sparq
out at a pinned ref into `target-routing/`, resolves `policy["routing"]`
(`= "orchestration/routing.toml"` from `policy/repos.toml`), path-confines it
(`routing_path.relative_to(target_root)`), parses it, and validates it with
`policy_resolve._validated_routing`. The only sparq → registry call in the other direction is a
**zero-payload doorbell**: `gh workflow run dispatch.yml -R jeswr/agent-account-registry`, no inputs,
no return value.

**So the boundary artifact is a file, read at a pinned ref, not an API.** That is a good boundary and
this design keeps it. It means the requirement declaration is reviewable, diffable, and gated by
sparq's own CI before the registry ever sees it.

### 4.2 What crosses after

sparq ships **`orchestration/requirements.toml`** — role/label → requirement set + agent, and
**no model names, no chains, no `[models]` catalog**:

```toml
# sparq: what this work NEEDS. No model appears in this file, ever.
schema_version = 1

[defaults]
agent = "sparq-rust-impl"
min_rank = "standard"
code_authorship = true
adversarial_safe = false
independence = "none"
on_unsatisfiable = "defer"

[[requirement]]
match_labels = ["zk", "mpc", "reasoner", "crypto", "auth", "e2ee"]
agent = "sparq-reviewer"
min_rank = "adversarial"
code_authorship = true
adversarial_safe = true          # the load-bearing one: see §3.3
on_unsatisfiable = "needs-user"  # never degrade; a human decides

[[requirement]]
role = "docs"
agent = "sparq-docs"
min_rank = "mechanical"
code_authorship = false          # replaces the docs_only model-name blocklist
on_unsatisfiable = "defer"

[[requirement]]
role = "ci"
agent = "sparq-ci-infra"
min_rank = "frontier"            # replaces SUB_FRONTIER = ("sonnet","haiku")
on_unsatisfiable = "defer"       # deliberately not needs-user — retry next tick

[[requirement]]
role = "review"
agent = "sparq-reviewer"
min_rank = "adversarial"
adversarial_safe = true
independence = "distinct-from-implementer"   # no model name can say this
on_unsatisfiable = "needs-user"
```

The registry ships **`orchestration/capabilities.toml`** — the **only** file in either repo where a
concrete provider model id appears:

```toml
# registry: what each model IS. The single source of concrete model ids.
schema_version = 1
catalog_version = "2026-07-26.1"

[model.opus5]
provider = "anthropic"
harness = "claude"
provider_model = "claude-opus-5"
credential_format = "claude-oauth-token"
rank = "adversarial"
code_authorship = true
adversarial_safe = true
verified = { probe = "identity", at = "2026-07-24", evidence = "sparq PR #3763 modelUsage" }

[model.sonnet]
provider = "anthropic"
harness = "claude"
provider_model = "claude-sonnet-4-6"
credential_format = "claude-oauth-token"
rank = "standard"
code_authorship = false     # maintainer directive 2026-07-18, docs-only
adversarial_safe = false
verified = { probe = "identity", at = "2026-07-18", evidence = "worker-account probe" }
```

### 4.3 The resolver, and where it plugs in

One new registry module, `scripts/capability-resolve.py`:

```python
def satisfy(requirement, catalog, context):
    """Return (chain, receipt). Raise Unsatisfiable(receipt) if nothing qualifies.

    requirement — the parsed row from the target's requirements.toml
    catalog     — the parsed capabilities.toml
    context     — {"implementer_provider": str|None, "lane": str, "now": int}
    """
```

Three properties of the split, each chosen to reuse an existing seam rather than build beside it:

1. **It plugs in ABOVE `select-and-claim` and BELOW `policy-resolve`.** `satisfy()` produces the
   `chain` argument that `allocator.claim(repo, package, role, model_chain, holder, …)` already takes.
   `select-and-claim.py`'s CLI contract (`--models`, `--account-pool`, `--ttl`) and its return
   (`{account, secret_ref, provider, harness, credential_format, model, claim_id}`) are **unchanged**.
   Per-account limits, the CAS lease ledger, and cache affinity are untouched.
2. **sparq never declares an order.** The requirement is a *set* constraint; the **walk order is
   lane-specific registry policy** and stays where it is. This matters because the two lanes disagree
   today and both are correct: `FIX_CHAIN` is strongest-first (*"the allocator PREFERENCE walk"*),
   while `ESCALATION_LADDERS` is weakest-first (*"ladder index is capability rank"*). An ordered list
   of model names in sparq is exactly the thing we are deleting, so sparq must not encode order at
   all — it declares the floor and the predicates, and the registry decides the walk.
3. **The claim result gains one field and loses none**: `resolution`, the receipt (§8).

Ordering within the satisfying set defaults to **cheapest-rank-first**, which reproduces the standing
"assign the cheapest tier that can do the job soundly" doctrine without sparq having to restate it.

## 5. Failure semantics — fail closed, and never silently

Two kinds of "nothing satisfies this" that must never be conflated, because one is a bug in a file and
the other is a Tuesday afternoon.

**5.1 Structural UNSAT — no model in the catalog *has* the capability.** This is an authoring defect
(a requirement typo, a rank floor above anything in the catalog, a contradictory predicate pair). It
must be caught **before merge**, not at 3am in a dispatch tick. Mechanism: sparq's existing
`routing-self-tests.yml` gains a leg that resolves **every** requirement row against a committed
snapshot of the registry catalog and fails the PR if any row is UNSAT.

*Honest limit of that check:* the snapshot can be stale, so it proves the requirement is satisfiable
by *a plausible* catalog, not by the *live* one. It is a fast-fail convenience. The real guarantee is
5.2. Saying which of the two is the guarantee matters, because a check that looks like a guarantee and
is not is worse than no check.

**5.2 Capacity UNSAT — models qualify, no account is free.** This already exists and already behaves
correctly: `select-and-claim` returns `none-free` with a machine-readable reason
(`pr-single-flight`, `package-single-flight`, `lane-cap`, `no-account-slots`) and the item defers to
the next tick. Unchanged.

**5.3 The rules that make it fail closed.**

- **No implicit widening. Ever.** The resolver may not drop a predicate, lower a rank floor, or relax
  `independence` to find a match. There is exactly one sanctioned widening in the system today —
  `cross_provider_fallback` (per-target, default `false`, documented as *"honest default: stay queued
  + alert, never silently degrade"*) — and this design **forbids it when `min_rank = "adversarial"`
  or `adversarial_safe = true`**, structurally, in the resolver.
- **The rank floor is a floor, and the mechanism for that already exists.** `dispatch-claim.py`'s
  defer-not-fallback for a pinned fix floor is exactly the semantics we need: *"once a floor is
  pinned, tiers below it are never re-offered; with no at/above-floor account free the claim returns
  `None` and the item defers"*. A requirement floor **is** a pin floor. Reuse it verbatim.
- **UNSAT always emits an attributable receipt** (§8), including into the park/defer reason, so the
  answer to "why did nothing run?" is machine-readable and does not require re-deriving the decision
  by hand.
- **`on_unsatisfiable` is honoured literally.** For an `adversarial` requirement it is `needs-user`,
  i.e. a human decides; the resolver never picks a weaker model to keep the pipeline moving.

**5.4 The dangerous case, named.** *A silent downgrade on a security-review requirement.* Concretely:
an `adversarial` + `adversarial_safe` requirement is satisfied by a model that is `frontier` but not
adversarial-safe; the run completes; a `VERDICT: pass` is produced by a model that quietly stopped
engaging with the security content partway through; the PR arms. Nothing in the pipeline distinguishes
that from a real pass. **This is the failure this whole design exists to make impossible**, and it is
the reason the runtime leg in §6.2 is not optional. A resolution-time check alone cannot catch it,
because at resolution time the pick was correct.

## 6. Keeping requirements honest — three legs, plus an anti-vacuity construction

A requirement that nothing enforces decays into a label. The catalog says `adversarial_safe = true`;
what makes that true?

**6.1 Resolution-time: unverified is treated as absent.** Every capability assertion in
`capabilities.toml` carries `verified = { probe, at, evidence }`. A `verified.at` older than the
probe's staleness window makes the capability **UNVERIFIED**, and an unverified capability is treated
as **absent** — fail closed, not fail open. This generalises what the project already does ad hoc: the
Opus 5 pin was probe-verified (`claude --model claude-opus-5 -p` → OK; `modelUsage` evidence in sparq
PR #3763) and the registry rollout PR #562 recorded that. The change is turning a one-off human probe
into a scheduled one with an expiry.

*Honest limit:* the identity probe verifies **availability and identity** — that the id resolves and
answers as the model we asked for. It does **not** verify quality. `rank` is a maintainer judgment,
not a measurement, and this design does not pretend otherwise. What the probe genuinely protects
against is the alias-lag class of bug (§1), which is the one that has actually bitten.

**6.2 Runtime: assert the model that answered is the model we claimed.** This is the leg that catches
§5.4, and it does not exist today. `review-fix.yml` checks the *claimed* alias against the routing
catalog (`:986-988`) — claim-time, not runtime. Proposal:

- the worker emits `runtime_model` (the harness's own `modelUsage` id) into the run outcome;
- the outcome step asserts `runtime_model == catalog[claimed_alias].provider_model`;
- on mismatch: **fail closed** — do not grade the round, do not count it against the budget,
  re-claim with the offending alias excluded, and emit a `capability:downgrade` observation.

The last item closes the loop and is what gives the vocabulary teeth: **a downgrade observed on
adversarial content flips that model's `adversarial_safe` to `false` pending re-probe.** The predicate
stops being an assertion someone typed and becomes a claim the system has standing evidence for. It
also means that if the Fable-on-security-content behaviour recurs on any future model, the system
discovers it rather than a human noticing.

**6.3 Anti-vacuity: mutate the guard, and mutate the YAML seam.** This project's measured trap is that
a guard nobody deletes-tests is usually vacuous, and that **every uncaught mutant in a recent sweep
lived at the YAML seam, not in the Python.** The registry has already built the right construction for
exactly this problem and it should be reused rather than reinvented — `dispatch-claim.py:6091-6107`
mutates each routing table *in the live workflow YAML text* and asserts the executed value changes,
failing with *"drifting {table} in review-fix.yml did NOT change the executed table — this agreement
pin is vacuous"*.

The requirement resolver inherits the same obligation, in both places:

- **Python:** for each of the five fields, `capability-resolve.py --self-test` mutates the catalog
  (flip the predicate, drop the rank, remove the provider) and asserts the resolved chain **changes or
  becomes UNSAT**. A field whose mutation changes nothing is a field that is not doing any work.
- **YAML:** mutate the `review-fix.yml` step that passes the requirement to the resolver — the `if:`,
  the step, and the call site — and assert the executed behaviour changes. A resolver that is correct
  but never actually invoked is the exact defect shape this repo keeps finding.

**6.4 The agreement pins interact, and the interaction is favourable.** Registry PR #707 pinned
`review_chain` / `fix_chain` / `ladders` by an executed equality assertion between
`dispatch-claim.py` and the inline Python in `review-fix.yml`, with a non-vacuity loop on top. Those
pins exist because the table is **duplicated** — the dispatcher mints a claim and the run re-derives
it, so drift means the dispatcher mints on a model the run then rejects.

This design **reduces** the number of pinned duplicates rather than adding to them: the concrete-id
catalog stops being duplicated across sparq and the registry (§9.2), and the resolution moves to one
module both sides call. The pins on `review_chain`/`fix_chain`/`ladders` themselves stay exactly as
they are through the whole migration and are only revisited in step 6, after every route is converted
— because those tables are the safety net that makes the differential migration in §9 provable.

## 7. The escalation ladder and the review-round budget, in requirement terms

**Today.** `worker-pr.ESCALATION_LADDERS = {"anthropic": ["opus", "fable", "opus5"], "openai":
["luna", "sol"]}`, ordered weakest-first with the terminal tier last (*"ladder index is capability
rank"*). A pinned floor replaces the fix chain with the ladder tiers at or above the pin, cheapest
first; tiers below the floor are deliberately absent (defer-not-fallback). Rounds are bounded by
`max_review_rounds = 3` (per target, `policy/repos.toml`) with extensions to
`worker_pr.HARD_CAP_ROUNDS = 6`, adjudicated by `decide_budget(...)` →
`{continue, extend-pending-review, extend-model-pin, extend-progress, needs-user}`. The round counter
lives in durable bot-comment markers (`<!-- sparq-review-round:v1`), with void markers so a stale-head
attempt is not charged.

**Re-expressed.** An escalation step **raises the requirement's `min_rank` floor by exactly one rank
and never lowers it**, holding `independence` and the predicates fixed. The ladder stops being an
ordered list of model names and becomes a walk up a four-element total order. `extend-model-pin`
becomes "the floor rose", which is the same event under a name that survives a model change.

**Termination is preserved, and here is why it never depended on the ladder.** Three independent
bounds, of which the binding one is unchanged:

1. `min_rank` is a **finite total order of four values** and escalation is strictly monotone
   increasing, so at most **three** escalations are possible from the bottom.
2. The floor is monotone non-decreasing across rounds — the existing pin-floor invariant, reused.
3. `HARD_CAP_ROUNDS = 6` is an **absolute** bound on rounds across both extension mechanisms, asserted
   before any other branch in `decide_budget` (`rounds_used >= hard_cap` is evaluated *before*
   `rounds_used < base_rounds`), and `base_rounds > hard_cap` raises at construction.

Bound (3) is the binding one and is untouched by this design. Ladder *length* was never the
terminating bound, so replacing a 3-element and a 2-element list with a 4-element rank order cannot
weaken termination: the run terminates at `min(rank_escalations, HARD_CAP_ROUNDS)`, and the second
term still dominates.

**One subtlety worth stating rather than discovering later.** If several catalog models share a rank,
"raise the floor by one rank" may skip a model that a name-ordered ladder would have tried, or (with a
naive implementation) retry within a rank and consume a round. The rule is therefore: **escalation
steps over ranks, not over models**; within a rank the resolver offers the satisfying set to the
allocator **once**. A round is spent per rank, not per model.

## 8. Observability — the resolution receipt

An indirection layer buys migration cost down and sells debugging cost up. The question "why did this
task get that model?" currently has a two-file answer; after this it has a three-file answer across two
repos. That is a real cost and the receipt is what buys it back. It is part of the design, not an
afterthought — and it must be emitted on **UNSAT** too, because "nothing ran and there is no record of
why" is the worst debugging case in the system.

`satisfy()` returns, and the claim carries:

```json
{
  "resolution": {
    "schema_version": 1,
    "catalog_version": "2026-07-26.1",
    "requirement": {"min_rank": "adversarial", "adversarial_safe": true,
                    "independence": "distinct-from-implementer", "matched_by": "match_labels:zk"},
    "context": {"implementer_provider": "anthropic", "lane": "review"},
    "candidates": ["sol", "luna", "opus5", "opus", "fable", "sonnet", "haiku", "terra"],
    "rejected": [
      {"alias": "opus5", "dimension": "independence", "reason": "same provider as implementer"},
      {"alias": "sonnet", "dimension": "code_authorship", "reason": "catalog: false"},
      {"alias": "luna", "dimension": "adversarial_safe", "reason": "UNVERIFIED since 2026-05-02"}
    ],
    "chain": ["sol"],
    "order": "cheapest-rank-first",
    "outcome": "satisfied"
  }
}
```

Three delivery channels, all existing — no new surface:

- **the lease row** gains a `requirement_digest` (a hash, so the public ledger stays
  operational-identifiers-only, consistent with `claim_commit_message`'s *"Public ledger subject:
  operational identifiers only, never account identity"*);
- **the worker receipt comment** gains a collapsed `<details>` block with the full receipt, so it is
  readable by a human on the PR without registry access;
- **the park/defer reason** embeds the receipt on UNSAT.

The one property to insist on: `rejected` lists **which dimension** eliminated each candidate, not
just that it was eliminated. "No model qualified" is not a debuggable message; "every anthropic model
was eliminated by `independence` and `sol` was eliminated by `adversarial_safe: UNVERIFIED`" is.

## 9. Migration path — six steps, no flag day

The hard constraint that shapes the order: `policy_resolve._validated_routing` **requires** a non-empty
`[models]` table (*"routing models catalog must be a non-empty table"*), and `review-fix.yml:505` reads
it to filter aliases to those with a resolvable `provider_model`. **sparq therefore cannot drop the
catalog first.** The registry has to learn to supply it before sparq can stop shipping it.

Every step is independently revertible and leaves the fleet running.

**Step 1 — registry: add the catalog, consult nobody.** Add `orchestration/capabilities.toml` with
capability annotations for the eight current aliases, plus `capability-resolve.py` with a full
`--self-test` including the §6.3 mutation loop. Wired into nothing. Add the scheduled identity probe
(§6.1). *Fleet impact: none.*

**Step 2 — registry: catalog indirection, proven by a differential.** `policy-resolve` accepts an
optional `catalog = "registry"` key in a target's routing document; when present, the `[models]`
requirement is satisfied from `capabilities.toml` instead of the target file. Gate it with a
**differential self-test**: for every current sparq route, both paths must produce the identical alias
chain. This is the same construction as the existing agreement pins and is the strongest migration
safety this codebase has. *Fleet impact: none until a target opts in.*

**Step 3 — sparq: one PR, drop the concrete ids.** `orchestration/routing.toml` adds
`catalog = "registry"` and its `[models]` blocks lose `provider_model`. Step 2's differential
guarantees no behaviour change. **After this step, no concrete provider model id exists in sparq's
routing table.** *Fleet impact: none, by construction.*

**Step 4 — registry: teach the resolver to accept requirements.** A target route may carry **either**
`model_chain` (legacy) **or** `requires = { … }`. Differential self-test: for every current sparq
route, the requirement form must resolve to the **same** alias chain as the legacy form. This is the
crux gate — if a route's requirement form cannot reproduce its chain, the vocabulary is wrong for that
route and we find out here, on a self-test, rather than in production. *Fleet impact: none.*

**Step 5 — sparq: convert routes one at a time, in risk order.** One PR per route, each proven
equivalent by step 4's differential at conversion time:
`docs` → `impl` → `site` → `perf` → `research` → `ci` → `review`/`soundness` → the security
`match_labels` override **last**, because it is the one where a mistake is dangerous rather than
merely annoying. `on_unsatisfiable` replaces `escalate` per route as it is converted. *Fleet impact:
one route at a time, each revertible in one revert.*

**Step 6 — remove the legacy path.** Once every route carries `requires`, drop `model_chain` support
from `_validated_routing`, delete the alias vocabulary from sparq entirely, and rename
`orchestration/routing.toml` → `orchestration/requirements.toml` (updating the `routing` pointer in
`policy/repos.toml`). Revisit whether the `review_chain` / `fix_chain` / `ladders` agreement pins can
now be derived from the catalog rather than duplicated (§6.4).

### 9.4 The `.claude/**` surfaces — an honest limitation

Steps 1–6 remove concrete model ids from sparq's **routing table**. They do **not** remove them from
the 16 `.claude/agents/*.md` frontmatter pins or the three `.claude/workflows/*.js` tier tables, and
this cannot be fully fixed by this design, for a specific reason: **the Claude Code harness resolves
`model:` frontmatter locally, at dispatch, with no network hook.** There is nowhere for a registry
lookup to happen.

What is achievable, in decreasing order of confidence:

- **`.claude/workflows/*.js` — fully fixable.** These already funnel every dispatch through
  `dispatchModel(tier)`, a single function. It becomes a lookup against a committed catalog snapshot
  keyed by requirement rather than a literal table. This is the majority of automated local dispatch.
- **`.claude/agents/*.md` — a generated value, not a hand-maintained one.** Each brief gains a
  `requires:` frontmatter key (the durable statement of intent) and its `model:` becomes a
  **generated pin**, refreshed by a bot PR when the catalog changes — a document as a projection of
  the catalog, which is the same principle as
  `research/knowledge-management-strategy.md` §2 applied to a config file.
  **This collides with a standing rule** and needs a maintainer decision: `AGENTS.md` § item 11 makes
  `.claude/agents/*.md` a **PROTECTED surface** where *"agent-config self-modification is blocked by
  design"* and any task needing one edited is `needs:user`. See **Q3**.
- **Interactive/manual sessions** keep a hard-coded default, and that is fine — an interactive session
  has a human in it who can notice.

Also unchanged and deliberately so: the five product-code files (`sparq-nlq`, `sparq-kb`,
`genai-retrieval`). A library's default LLM is a product API decision with its own compatibility
obligations to downstream users; routing it through the fleet registry would be a category error.

## 10. What this costs, honestly

- **A new failure mode: a requirement that resolves to nothing.** Did not exist before — an alias
  chain either resolved or was empty at authoring time. Mitigated by the structural/capacity split
  (§5) and the pre-merge satisfiability leg, but the mitigation is a fast-fail, not a guarantee (§5.1
  says so explicitly).
- **A harder debugging story.** Three files, two repos, one solver. Bought back by the receipt (§8),
  which is mandatory on every claim including UNSAT.
- **A catalog that makes claims we cannot fully verify.** `rank` is judgment; only identity and
  availability are probed, and `adversarial_safe` is only evidenced *after* a downgrade has been
  observed once. This is a genuine weakness and the design does not paper over it — §6.1 states the
  probe's limit, and §6.2 is what upgrades `adversarial_safe` from assertion to evidence over time.
- **A cross-repo schema.** A requirement-schema change is a two-repo lockstep change. `schema_version`
  is carried on both files so a dual-support window is possible, but the discipline is unproven.
- **More moving parts in the trust plane.** `orchestration/` and `scripts/` are trust surfaces in the
  registry's `security_paths`, so every step above is human-armed. That is correct and it is a real
  throughput cost.

**What it does not fix.** It does not make routing decisions better — it makes them *expressible and
migratable*. If `min_rank = "frontier"` is the wrong call for CI work, this design will faithfully
keep making the wrong call, in one place, with a receipt.

## 11. Open questions for the maintainer (blocking the vocabulary)

**Q1 — is the docs route a cost floor or a quality floor?** As encoded, the docs chain leads with the
cheapest model, so `min_rank = "mechanical"`. The migration currently in flight (Sonnet/Haiku → sol
for docs) routes docs to a **frontier** model, which is not a cost floor at all. These imply different
vocabularies: if it is quality, `min_rank` is mis-named and mis-shaped, and a separate `prose_quality`
predicate may be justified after all (§3.4 rejected it on current evidence). **This is the single
question most likely to change §3.**

**Q2 — is `adversarial_safe` a property of the model, or of the (model, account, harness) triple?** If
a mid-run safeguard downgrade depends on account tier, harness, or system prompt rather than on the
model alone, then it **cannot live in a model catalog** and must move into the per-account records that
`select-and-claim` already reads — which moves the boundary in §4 and changes which component owns the
predicate. I do not have evidence either way and did not find an in-tree record of the original
observation (§3.3).

**Q3 — how do we handle `.claude/agents/*.md`, given it is a PROTECTED surface?** Either (a) permit a
bot to regenerate the `model:` pin from a `requires:` key, which requires carving an exception into
`AGENTS.md` item 11's self-modification block; or (b) accept that 16 agent configs keep hard-coded
model names permanently and every migration keeps touching them. There is no third option while the
harness resolves frontmatter locally. **(a) is more useful and strictly more dangerous** — a
compromised or buggy generator rewrites the security reviewer's model.

**Q4 (lower stakes) — does `provider:<id>` belong in the vocabulary at all?** It is the one value that
is a provenance preference rather than a capability (§3.3). Keeping it makes the `role = "site"` rule
expressible; dropping it forces that rule to be justified in capability terms or retired.

## 12. Relationship to the other in-flight design records

- `research/agent-context-sharing.md` (PR #4187) — bounded context sharing between
  fleet agents. **Orthogonal**: it governs *what an agent knows*, this governs *which model runs it*.
  One touchpoint: its lineage-handoff channel is the natural carrier for "the previous attempt ran at
  rank X and produced no diff", which is an input to an escalation decision (§7). Not designed here.
- `research/knowledge-management-strategy.md` (PR #4193) — documents as
  projections of a queryable graph. **Same principle, different corpus**: §9.4's generated `model:`
  pin is that record's "a document is a projection, not a source" applied to a config file. The
  capability catalog is a small, hand-maintained source of truth, which is consistent with that
  record's rule that facts live in one place and everything else is generated from it.
- [`issue-native-orchestration.md`](issue-native-orchestration.md) — the record this one extends. Note
  its §"Private account registry" heading and its claim that *"No public repo ever holds account
  handles, limits, usage, tokens, or selection logic"* are **stale** (§2.1); that drift is filed
  separately and is not corrected here, since this record does not own that file.

<!-- Authored by Claude Opus 5 as a SPARQ agent. Design only; no implementation, no child beads. -->
