<!-- [OPUS-4.8] SLSA framework intro (epic sq-toze / bead sq-toze.14, branch cert-slsa). -->
# SLSA build provenance — sparq certification slice

**Framework:** [SLSA](https://slsa.dev) (Supply-chain Levels for Software Artifacts) v1.0 —
Build track + Source/Provenance/Verification dimensions.
**Bead:** `sq-toze.14` (epic `sq-toze`). **Branch:** `cert-slsa`.

## Honest headline

> **sparq's official release archives (`sparq-cli` tar.gz/zip) and the ghcr.io `sparq-server`
> container image reach SLSA Build Level 2.** They carry signed, hosted-platform-generated
> provenance (`actions/attest-build-provenance` / buildkit `provenance: mode=max`). sparq does
> **not** claim Build L3 (provenance is generated in-band with the build, not by an isolated
> trusted builder). The `dist.yml` tiered binaries are now attested (GX-9 closed). The
> `@sparq-org/sparq` **npm** package now carries native Sigstore provenance (`publish.yml#npm`) and
> the crates.io `.crate` bytes get an out-of-band attestation (`publish.yml#crates`); **crates.io
> has no native provenance-link mechanism upstream**. The **PyPI `sparq-rdf`** lane is now CI-wired
> for native PEP-740 attestations via Trusted Publishing (`publish.yml#pypi-*`) — it activates once
> a maintainer registers the PyPI Trusted Publisher (GX-10 partial) — see `gap-register.md`.

This is a bounded, evidence-backed claim. We never publish "SLSA L3" or "all artifacts attested"
because the provenance does not back it.

## Why SLSA is a high-fit framework for sparq

sparq is consumed as a **dependency** (crates.io / npm-WASM / PyPI / ghcr). For a dependency the
dominant supply-chain question is *"was this artifact built from the source it claims, by a build
I can verify?"* — which is exactly what SLSA's provenance answers. sparq already emits signed
provenance on release; this slice declares the **honest level**, maps each control to concrete
repo evidence, and records the gaps to a higher/complete posture.

## Scope — what's in and out for a library/server

**In scope (sparq's build pipeline):** the `release.yml` archive build + the `release.yml#docker`
container build (where provenance lives); the source-track integrity controls SLSA's provenance
binds to (version control, two-person review, pinned+locked deps, cargo-vet/deny, least-privilege
tokens); consumer-side verification documentation.

**Out of scope / operator-owned:**
- **Deploy-time admission enforcement** — requiring `gh attestation verify` / a cosign policy
  before running an artifact is the *operator's* control. sparq ships verifiable provenance + the
  documented verify command (`controls.md` SL-V-b).
- **The SLSA-level *certificate*** — an accredited-assessor attestation is an external-body
  activity. This slice makes the controls + evidence **audit-ready**; the certificate is external.
- **crates.io published-package provenance (native link)** — crates.io has no upstream
  provenance-link mechanism yet, so a provenance badge on the crates.io page is not closable from
  our side (folded into GX-10 as an external sub-gap). sparq does emit an **out-of-band** SLSA
  attestation over the `.crate` bytes (`publish.yml#crates`); the *registry-native* link is external.
- **PyPI Trusted-Publisher registration** — the `publish.yml#pypi-*` lane emits native PEP-740
  attestations once the `sparq-rdf` PyPI project has a Trusted Publisher registered (owner/repo/
  workflow/env binding). That registration lives on the PyPI account, not in the repo, so it is a
  maintainer act (folded into GX-10 / sq-toze.37). The CI wiring is in-repo and complete.

## Files

| File | Contents |
|---|---|
| `controls.md` | The control spine: SLSA Build L1→L3 + Source/Verification controls, each → status (IV/AR/GAP) → evidence (file/CI-job) → owner. The honest per-artifact level table. |
| `evidence.md` | The verifiable evidence pack — exact workflow snippets + the `gh attestation verify` / `cargo audit bin` / `cargo vet` commands an auditor runs. |
| `gap-register.md` | Open gaps (GX-8/9/10/11), severity, remediation, target, the `bd` bead each tracks. |
| `trusted-builder-pin-policy.md` | The review/bump policy for `slsa-github-generator`'s tag pin — the one deliberate exception to the repo's SHA-pin convention (#4572): why the tag *is* the trust anchor, the Dependabot posture that keeps a bot from moving it, the quarterly review cadence + owner, the bump checklist, and the review log. |
| `reproducible-build.md` | The GX-8 reproducible-build evidence (cross-cutting with cra/sbom/ssdf/openssf): the measured double-build diff, the single named non-determinism source, the scoped remediation, the auditor quick-run. |

## Status summary (for `compliance/README.md`)

| Posture | Detail |
|---|---|
| **Implemented & verified** | Build L2 for release archives + container **+ the `dist.yml` tiered binaries** **+ the `@sparq-org/sparq` npm package** (native Sigstore `npm publish --provenance` + `npm audit signatures` gate, `publish.yml#npm`, GX-10/sq-toze.24) (signed provenance, hosted runner, cargo-auditable, attested SBOM/VEX; GX-9 closed via sq-toze.23); source-track integrity (pinned+locked deps, cargo-vet + cargo-deny GATING, least-privilege tokens, security.txt). |
| **Audit-ready** | Two-person review + protected-branch ruleset (configured out-of-repo, recorded in `docs/branch-protection.md`); consumer verification policy (operator-enforced); the SLSA-level certificate (external assessor). |
| **Gap** | published-package provenance PARTIAL (GX-10/sq-toze.24 + sq-toze.37): **npm CLOSED** (`publish.yml#npm`), **crates.io** has an out-of-band `.crate` attestation but the **registry-native link is external/OPEN**, **PyPI `sparq-rdf`** PEP-740 lane WIRED in CI (`publish.yml#pypi-*`, Trusted Publishing + native attestations) but awaits a one-time maintainer PyPI Trusted-Publisher registration; reproducible-build CHARACTERISED not enforced (GX-8/sq-toze.9 — `reproducible-build.md`: 22-byte single-cause diff documented, CI rebuild-and-diff ratchet remaining); Build L3 not met — NARROWED TWICE (GX-11/sq-toze.25 then #4570): every `release.yml`/`dist.yml` artifact but the container image now routes provenance through the isolated `slsa-github-generator` trusted builder — `release.yml#provenance` (archives), `release.yml#provenance-artifacts` (GUI bundles, SBOM/VEX, conformance report) and `dist.yml#provenance` (tiered binaries), each a separate job with digests threaded across the boundary and `release` `needs:`-ing both of its lanes — but **no lane is exercised** (tag/dispatch-triggered) and the **ghcr container image** is still in-band L2, so the published level stays **L2**. *(GX-9 dist.yml binaries — CLOSED, now SLSA Build L2 / sq-toze.23.)* |

## Do-not-re-propose (already in the posture — cite, don't re-add)

`actions/attest-build-provenance` + buildkit `provenance: mode=max` (release.yml), cargo-auditable
(release.yml), cargo-vet GATING (supply-chain.yml), cargo-deny sources/advisories GATING, SHA-pinned
actions + base images, Cargo.lock + `--locked`, ci-summary required gate, CODEOWNERS +
branch-protection doc, `.well-known/security.txt`, per-release SBOM+VEX. These are evidence, not
new work.
