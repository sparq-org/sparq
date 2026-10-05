<!-- [OPUS-5] #4572 — the review/bump policy for the one tag-pinned action reference in the repo. -->
# Trusted-builder tag pin — review and bump policy

**Scope:** the `slsa-framework/slsa-github-generator` trusted reusable builders called by the four
isolated-provenance lanes — `generator_generic_slsa3.yml` for the three file lanes
(`release.yml#provenance`, `release.yml#provenance-artifacts`, `dist.yml#provenance`) and
`generator_container_slsa3.yml` for the container lane (`release.yml#provenance-container`, #4635).
Two reusable workflows, **one** trust anchor: they share the upstream repo and must share the tag.
**Control:** SLSA `SL-B3-b` (`controls.md`) / gap **GX-11** (`gap-register.md`).
**Owner:** release engineering — the same owner column `controls.md` records for `SL-B3-b`.

## The decision

**Keep the tag reference. Do not SHA-pin it. Do not let Dependabot move it.**

This reference is the single, deliberate exception to the repo's SHA-pin convention, and it is an
exception because SHA-pinning it would *break* the property the lanes exist to provide, rather
than merely change how they are pinned:

> The generator derives the trusted-builder identity that ends up in the generated provenance —
> the identity a consumer's `slsa-verifier` policy is matched against — from its own `@ref`.
> Referenced by commit SHA it cannot resolve a builder identity a verifier will accept.

So here the tag *is* the trust anchor. That rationale is carried at each of the four call sites
so nobody has to find this file to avoid the mistake — in full above `release.yml#provenance` and
`dist.yml#provenance`, by cross-reference above `release.yml#provenance-artifacts` and
`release.yml#provenance-container`. This file is where the **policy** lives.

It is the only such exception: every other *third-party* action reference in the repo is pinned to
a full commit SHA (the remaining non-SHA `uses:` values are `./.github/workflows/*.yml` calls into
this same repo, where a SHA pin would mean nothing).

The authoritative value of the pin is the `uses:` lines themselves — never a copy:

```console
$ grep -nE 'generator_(generic|container)_slsa3' .github/workflows/release.yml .github/workflows/dist.yml
```

## Dependabot posture

`.github/dependabot.yml` carries an `ignore` entry for
`slsa-framework/slsa-github-generator*` covering all three `version-update:semver-*` types.

- **Why ignore at all.** Dependabot's `github-actions` ecosystem tracks reusable-workflow `uses:`
  references, so without the entry this pin is a candidate for the weekly `actions-minor-patch`
  grouped PR — a batched, unreviewed bump of the trust anchor, exactly what the call-site comments
  warn against. The entry is deliberately **fail-safe**: if Dependabot turns out never to have
  proposed a bump here, the entry is inert and costs nothing; if it does, the entry is the only
  thing stopping it.
- **Why the trailing `*`.** The exact dependency name Dependabot reports for a reusable workflow
  (`owner/repo`, or the full `owner/repo/.github/workflows/<file>.yml`) was **not confirmed** when
  this policy was written, and an `ignore` glob that matches neither form silently does nothing —
  a failure mode with no signal at all. The wildcard covers both, so the policy does not depend on
  getting that detail right.
- **Not the SHA-pin risk.** The issue that prompted this record (#4572) also asked whether
  Dependabot might *SHA-pin* the reference, which would break the builder identity outright rather
  than merely move it. That question is left unresolved on purpose: with version-updates ignored,
  Dependabot proposes nothing here for either failure mode, and the structural test rejects a SHA
  reference on every PR regardless of who wrote it (`check`'s `TRUSTED_BUILDER` pattern).
- **Why the `update-types` list is mandatory.** A bare `dependency-name` entry with no
  `update-types` also suppresses **security** updates. Listing the three `version-update:*`
  types silences routine bumps while leaving advisory-driven security updates flowing — the same
  shape as the neighbouring `dtolnay/rust-toolchain` entry. This is asserted on every PR
  (see *Enforcement* below), because it is a one-line edit away from silently going wrong.

## Cadence

**Quarterly**, mechanised by `.github/workflows/slsa-builder-pin-review.yml` (06:37 UTC on the
1st of January / April / July / October, plus `workflow_dispatch`).

The job derives the pinned tag from the workflows, compares it with the upstream latest release,
and opens **one** idempotent issue labelled `supply-chain:trusted-builder-pin` only when there is
an actual decision to take — upstream has published a newer release, or the pinned tag has
stopped resolving upstream (a retag or withdrawn release is itself a trust event). When the pin
is current again it closes the issue, so a stale reminder cannot linger past the review.

Quarterly rather than weekly on purpose: a builder-identity change is a trust review, and a
reminder that fires on a pin nobody should be bumping casually trains people to ignore it.

## The bump checklist

When the review issue opens, the bump is a reviewed change, not a version edit:

1. Read the upstream release notes and the diff between the two tags, with attention to the
   signing path and to any change in the builder identity string.
2. Confirm the new tag is a stable release (not a pre-release/RC) and is the version upstream
   currently recommends for **both** `generator_generic_slsa3.yml` and
   `generator_container_slsa3.yml` — they are versioned together upstream, but confirm it.
3. Confirm published consumer verification guidance still matches — the `slsa-verifier`
   invocation in `scripts/verify-release-provenance.sh`, and the `verify-artifact` /
   `verify-image` commands in `release.yml`'s release-notes body.
4. Update **all four** lanes in one change — both reusable-workflow names move together. A partial
   bump would put two builder identities in a single release;
   `scripts/tests/test_release_slsa_l3_provenance.py` rejects it on every PR.
5. Keep it a tag. SHA-pinning erases the verifiable builder identity — the whole reason for the
   exception.
6. Append the outcome to the review log below, including a "reviewed, no change" outcome.

## Enforcement

`scripts/tests/test_release_slsa_l3_provenance.py` runs on every PR (`docs-quality`) and pins the
structural half of this policy, each assertion proved non-vacuous by its own mutation:

- every trusted-builder call site — generic *and* container — is on the **same** tag, and all four
  lanes are present;
- the Dependabot `ignore` entry exists, its glob covers the reusable-workflow name form, and it
  lists the `version-update:*` types rather than being a bare entry that would also swallow
  security updates;
- this policy record and the quarterly workflow still exist, and that workflow is still on a
  `schedule:` — a cadence removed by deleting a trigger is otherwise invisible.

The workflow is `schedule`/`workflow_dispatch`-only, so it produces no check-run on a PR and is
not part of the `ci-summary / gate` aggregation (hence no `.github/advisory-registry.json` entry).

## Review log

| Date | Pinned | Upstream latest | Outcome |
|---|---|---|---|
| 2026-07-28 | `v2.1.0` | **not checked** | Policy established (#4572). The "is `v2.1.0` still the recommended release?" question was **not answered here** — it was not verifiable from the authoring environment, and guessing it is precisely the drift this record exists to stop. `.github/workflows/slsa-builder-pin-review.yml` answers it: `workflow_dispatch` it for the answer now, or let the next quarterly tick produce it. Either way the outcome lands in this table. |
| 2026-07-28 | `v2.1.0` | not re-checked | **Scope widened, pin unchanged (#4635).** The ghcr container lane (`release.yml#provenance-container`) was added on the SAME tag, using the sibling `generator_container_slsa3.yml` from the same upstream repo. Not a bump and not a bump review — no upstream comparison was made here; the quarterly tick still owns that question. The lane count enforced by the structural test moved 3 → 4. |
