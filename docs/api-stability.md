<!-- [OPUS-4.8] Governance: API stability & deprecation policy-of-record (bead sq-tvuw8; #1346 P0, #1248). -->
# API stability & deprecation policy

This is the **doc-of-record** for sparq's public-API stability tiers and its deprecation
policy. It exists to answer the single question a downstream consumer asks before it can
depend on sparq in production: **which parts of the API will not break under me, and on
what notice?**

It is filed against the Solid-server track's P0 consumer requirement — a *frozen,
semver-stable public API surface* ([#1346](https://github.com/sparq-org/sparq/issues/1346),
[#1248](https://github.com/sparq-org/sparq/issues/1248)). A consumer that git-pins an
*unstable* embedded API (as [`solid-server-rs`](https://github.com/jeswr/solid-server-rs)
does today, ADR 0001) takes on breakage risk on every revision bump. This document
**proposes** the tier-1 surface and the guarantees attached to it so that the freeze — the
governance act — has one well-defined shape to ratify.

## Status: PROPOSED, not yet in force

> **The tier-1 semver freeze is NOT active.** Declaring it active is the maintainer's
> (@jeswr's) governance call ([#1346](https://github.com/sparq-org/sparq/issues/1346) P0,
> [#1248](https://github.com/sparq-org/sparq/issues/1248) items 1+2). Until that ratification,
> **every crate here is pre-`1.0` and API-unstable**, and a minor pre-`1.0` release MAY
> still change any surface — tier-1 included. This document is the *proposal that tees up
> the ratification*, together with the in-code markers that annotate the proposed surface.
> Nothing in this file, and no in-code marker, creates a guarantee before ratification.

The workspace status is *experimental, pre-1.0, API unstable* (root `README.md`,
`CHANGELOG.md`); this policy does not change that. It describes the *shape* a future
stable release would commit to, so the shape is reviewable now and the freeze is a
one-line governance decision rather than a design exercise.

## The tiers

sparq's public API is partitioned into two tiers. The tier is a property of a **surface**
(a module, a set of functions, a set of types), not of a whole crate.

### Tier-1 — proposed-stable

The surface a downstream consumer is *encouraged to depend on*, and the surface the freeze
(once ratified) will cover. It is deliberately **small**: the in-process embedding front
door and the per-resource access-control decision API — the two surfaces the Solid-server
consumer actually binds to.

Once ratified, tier-1 carries the guarantees in [Tier-1 guarantees](#tier-1-guarantees)
below.

### Tier-2 — experimental / unstable (everything else)

**Everything not explicitly listed as tier-1 is tier-2.** Tier-2 is the default: it may
change or be removed in any release, including a minor pre-`1.0` release, without a
deprecation window. This includes — non-exhaustively — the rest of `sparq-serve` (the
scheduler, the `backup` / `change-stream` / `change-sink` / `result-cache` opt-in features, the
write-side footprint/commit internals), the graph-filtering and materialization halves of
`sparq-solid` (`query_as` / `update_as` / `materialize_*` / the conformance harnesses), and
every other crate in the workspace (`sparq-core`, `sparq-engine`, `sparq-server`'s HTTP
router and types, and the opt-in capability crates). A consumer may still use tier-2
surfaces; it just does so without the stability guarantee, and should pin an exact revision.

Note that `sparq-server`'s **Rust** surface being tier-2 is distinct from its **HTTP wire**
surface: what the server speaks over the network to an HTTP-only consumer has its own
proposed-frozen contract — [`docs/http-wire-contract.md`](http-wire-contract.md)
([#1416](https://github.com/sparq-org/sparq/issues/1416)) — with the same
proposed-until-ratified status as this document.

Opt-in, feature-gated capability crates remain tier-2 regardless of this policy: keeping the
core lean and the capability surfaces free to evolve is a deliberate architecture choice.

## The proposed tier-1 surface

Grounded in the code as it stands on `main`. Each item is annotated in-code with a
tier-1 (proposed-stable) marker that links back to this document.

### 1. The in-process embedding data path — `sparq_serve::embed`

The documented facade an external host uses to call the engine **in-process** instead of
over the SPARQL-1.1-over-HTTP transport (the seam landed for
[#1248](https://github.com/sparq-org/sparq/issues/1248) items 1+2; the generation ring is
[#1250](https://github.com/sparq-org/sparq/issues/1250)). Every entry point is a thin
re-export or one-line wrapper over an already-public engine path — no new behaviour.

- **Read / probe** (over `&Graph`): `query_json`, `query_json_with_budget`, `query`,
  `ask`, `exists`, `named_graph_exists`, `metadata` (and the `Metadata` struct).
- **Write** (over `&mut Graph`): `update_in_place`, `update_in_place_atomic`,
  `apply_delta_nquads`.
- **Concurrency wrapper** (re-exported runtime-agnostic core): `GenerationRing`,
  `Generation`, `GraphApplier`, `Writer`, and their configuration/associated types
  `RingConfig`, `WriterConfig`, `TimeTravelConfig`, `ApplyUpdates`, `WriteError`, `PodId`.

### 2. The per-resource access-control decision API — `sparq_solid`

The point-query authorization surface an LDP resource server calls per request (issue
[#992](https://github.com/sparq-org/sparq/issues/992) FR-1/5/6/7; the fail-closed decision
contract is [#1193](https://github.com/sparq-org/sparq/issues/1193)). This is the
**authorization-decision** surface only — it answers *"may principal X do mode M on
resource R?"* over an already-loaded dataset. It does **not** authenticate, and it makes no
cryptographic guarantee; a `Session` is a caller-asserted claim (see the `sparq-solid`
README / the `access-control` skill).

- `PodStore::decide`, `PodStore::decide_batch`, `PodStore::resolve_acl`,
  `PodStore::wac_allow`.
- The decision types: `WacDecision`, `AclStatus`, `AclScope`, `EffectiveAcl`, and the
  `Mode` / `Session` request vocabulary those methods take and return.

The load-time helpers (`PodStore::new`, `materialize_wac` / `materialize_acp`) are the
prerequisite for a decision but sit at the boundary of the frozen surface; they are listed
as **tier-1-adjacent** and would be resolved (in or out) at ratification.

## Tier-1 guarantees

These take effect **only once the freeze is ratified** (see [Status](#status-proposed-not-yet-in-force)).
They are stated here so the ratification is a decision about a known contract.

Within a stable `MAJOR.MINOR.PATCH` line, for a tier-1 item:

- **No breaking change without a major bump.** A function signature, a type's public
  shape, or a documented behaviour (e.g. the fail-closed rule: any uncertainty ⇒ deny) does
  not change incompatibly within a major version.
- **Additive change is allowed in a minor release.** New functions, new tier-1 items, and
  new fields on types marked `#[non_exhaustive]` may appear in a minor release; consumers
  are expected to match non-exhaustively and not to rely on a type's field count.
- **Non-breaking is defined behaviourally, not just by type signature.** The SPARQL Results
  JSON shape `query_json` returns, the fail-closed `AclStatus` → allow/deny mapping, and the
  "a `decide` allow is never wider than the oracle would grant" property are part of the
  contract, not incidental.
- **Internals may change freely.** Performance, memory layout, and the private
  implementation behind a tier-1 entry point are never part of the contract. This document
  carries **no performance numbers** by design; performance is not a stability guarantee.

What is explicitly **out of scope** of a tier-1 guarantee: any tier-2 surface reachable
*through* a tier-1 type (e.g. a field typed as a tier-2 struct pulls that struct's
instability along the edge — such edges are called out at ratification), and any behaviour
under an opt-in feature flag unless that specific item is listed as tier-1.

## Deprecation policy

Once a stable line exists, removing or breaking a tier-1 item follows a **deprecate-then-remove** window:

1. **Announce.** Mark the item `#[deprecated(since = "...", note = "...")]` with a note that
   names the replacement and points at this document. The deprecation ships in a minor
   release.
2. **Window.** Keep the deprecated item working, warning-only, for at least **one full
   minor release cycle** after the announcing release (a longer window for a
   consumer-critical surface such as the embed data path). The window is measured in
   releases, not wall-clock time.
3. **Remove.** Remove only in a subsequent **major** release, with the removal and its
   replacement recorded in `CHANGELOG.md`.

An *additive* replacement (new stable item alongside the old) is preferred over an
in-place breaking change wherever a compatible shape exists. Deprecations are tracked in
`CHANGELOG.md` and, where the removal is deferred work, as a bead.

## In-code markers

The proposed tier-1 surface is annotated in-code with lightweight, non-breaking rustdoc
markers (a module- or item-level *"API tier-1 (proposed-stable)"* line linking here). The
markers are documentation only: they change no signature and no behaviour, and — per
[Status](#status-proposed-not-yet-in-force) — they assert a *proposal*, not an active
guarantee. On ratification, the markers are updated from "proposed-stable" to the ratified
wording in the same pass that records the freeze.

## Ratification checklist (for the maintainer)

Activating the freeze is a governance decision. When @jeswr elects to ratify:

1. Confirm the tier-1 surface list above (add/remove items; resolve the tier-1-adjacent
   load-time helpers).
2. Cut a version line that carries the tier-1 contract (see `docs/release.md`) and record
   the freeze in `CHANGELOG.md`.
3. Flip the in-code markers and the [Status](#status-proposed-not-yet-in-force) section
   from *proposed* to *in force*.
4. Note the ratification on [#1248](https://github.com/sparq-org/sparq/issues/1248) /
   [#1346](https://github.com/sparq-org/sparq/issues/1346) so the consumer can pin the stable
   line.

Until step 3, the tier-1 guarantee is **not** in force.

## Related

- [`docs/http-wire-contract.md`](http-wire-contract.md) — the HTTP wire counterpart of this
  policy: the v1 served-surface contract for HTTP-only consumers (#1416).
- [`docs/release.md`](release.md) — how a release is cut (maintainer-triggered).
- [`docs/branch-protection.md`](branch-protection.md) — the `main` protection doc-of-record.
- `crates/sparq-serve/README.md` — the `embed` seam narrative.
- `crates/sparq-solid/README.md` / the `access-control` skill — the decision surface (and
  its explicit *does-not-authenticate* boundary).
- [#1346](https://github.com/sparq-org/sparq/issues/1346) (consumer requirements) ·
  [#1248](https://github.com/sparq-org/sparq/issues/1248) (embed + decision surface) ·
  [#1250](https://github.com/sparq-org/sparq/issues/1250) (generation ring) ·
  [#1193](https://github.com/sparq-org/sparq/issues/1193) (fail-closed decision contract).
