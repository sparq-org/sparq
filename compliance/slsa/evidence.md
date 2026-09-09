<!-- [OPUS-4.8] SLSA evidence pack (epic sq-toze / bead sq-toze.14, branch cert-slsa). -->
# SLSA evidence pack (sparq)

Concrete, checkable evidence for every **implemented & verified** SLSA control in
`controls.md`. Each item is a repo path + the line/anchor that enforces it + the command an
auditor runs to confirm. Paths are repo-relative; line numbers are indicative (anchor on the
quoted text, which is stable).

> **NON-CANONICAL timing** — no measured numbers here. This is a hosted-CI evidence pack; the
> only "runtime" facts are CI job presence + signed attestations in the Sigstore/Rekor log.

---

## 1. Signed build provenance over release archives (Build L2 core — SL-B1-b/c/d, SL-B2-b/c)

**File:** `.github/workflows/release.yml`, job `package`.

```yaml
# release.yml#package — permissions
permissions:
  contents: read
  id-token: write        # OIDC token → Sigstore Fulcio cert (signs the provenance)
  attestations: write    # write the attestation to the GitHub attestations store
...
- name: Attest build provenance (archives)
  uses: actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4.1.0
  with:
    subject-path: |
      pkg/*.tar.gz
      pkg/*.zip
```

- The action emits an **in-toto SLSA provenance predicate** binding each archive's SHA-256
  digest to this workflow run (repo + ref + run-id, via the OIDC identity), signs it with a
  short-lived Sigstore Fulcio certificate, and records it in the Rekor transparency log.
- **Verify:** `gh attestation verify sparq-cli-vX.Y.Z-x64-v3.tar.gz --repo sparq-org/sparq`
  (succeeds only if the artifact's digest matches a signed attestation from this repo's
  release workflow).
- **Hosted platform (SL-B2-a):** `runs-on:` is GitHub-hosted (`ubuntu-latest`, `macos-14`,
  `macos-15-intel`, `ubuntu-24.04-arm`, `windows-latest`) across the matrix — no self-hosted
  runners.

## 2. cargo-auditable — dependency manifest embedded in the binary (SL-B1-d, GX-7)

**File:** `.github/workflows/release.yml`, job `package`.

```yaml
- name: Install cargo-auditable
  uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # v2.81.10
  with: { tool: cargo-auditable }
- name: Build (...)
  run: RUSTFLAGS="$FLAGS" cargo auditable build --release --locked -p sparq-cli --target ${{ matrix.target }}
```

- Every released `sparq-cli` binary embeds its exact resolved dependency tree.
- **Verify:** `cargo audit bin ./sparq-cli` (or `auditable info ./sparq-cli`) prints the
  embedded manifest — the binary is self-describing for post-build SCA, independent of any
  external SBOM.

## 3. Per-release CycloneDX SBOM + VEX, themselves attested (SL-V-c, GX-2 — DONE)

**Files:** `.github/workflows/release.yml#sbom`; `scripts/gen-sbom-vex.sh`;
`supply-chain/vex.cdx.json`.

```yaml
# release.yml#sbom
- name: Generate per-release SBOM + VEX
  run: scripts/gen-sbom-vex.sh
- name: Attest build provenance (SBOM + VEX)
  uses: actions/attest-build-provenance@a2bbfa2…  # v4.1.0
  with: { subject-path: "sbom/*.sbom.cdx.json\nsbom/*.vex.cdx.json" }
```

- `gen-sbom-vex.sh` emits one CycloneDX SBOM per released binary (`sparq-cli`, `sparq-server`)
  with the **NTIA minimum elements** (supplier, component, version, purl, dependency
  relationships, author, timestamp — see the script header) plus a version-stamped VEX derived
  from the checked-in `supply-chain/vex.cdx.json`, kept 1:1 with `deny.toml [advisories].ignore`.
- The SBOM + VEX are **SLSA-attested** (so a swapped SBOM is detectable) and covered by
  `SHA256SUMS` (`release.yml#release` *Generate SHA256SUMS*).
- **Verify:** `gh attestation verify sparq-cli-vX.Y.Z.sbom.cdx.json --repo sparq-org/sparq`;
  `shasum -a 256 -c SHA256SUMS`.

## 4. Container provenance + embedded SBOM (SL-B1-b, SL-B2-b/c)

**File:** `.github/workflows/release.yml`, job `docker`.

```yaml
permissions: { contents: read, packages: write, id-token: write, attestations: write }
...
- name: Build and push
  uses: docker/build-push-action@f9f3042f… # v7.2.0
  with:
    push: true
    provenance: mode=max   # max-mode SLSA provenance attached to the image
    sbom: true             # embedded SBOM attestation
```

- buildkit attaches a SLSA provenance attestation (full build graph + materials) and an SBOM
  to the ghcr.io image.
- **Smoke-before-push (defence-in-depth):** `release.yml#docker` builds + `docker run`s the
  image (`scripts/docker-smoke.sh`) *before* the push step, so a broken image never reaches
  the registry.
- **Verify:** the buildkit `provenance: mode=max` output is a **cosign-style registry
  attestation** (attached to the image in the OCI registry), so verify it with
  `cosign verify-attestation --type slsaprovenance ghcr.io/sparq-org/sparq-server:<tag> …` (or the
  registry attestation API). Note: `gh attestation verify` is the verifier for the
  `actions/attest-build-provenance`-signed **archives + SBOM/VEX** (§1, §3); it is not the
  primary tool for the buildkit image attestation here.

## 5. Pinned + locked build materials (SL-B3-c, SL-S-e)

- **Cargo.lock checked in:** `git ls-files Cargo.lock` → tracked; every release/dist/Docker
  build uses `--locked`.
- **SHA-pinned base images:** `Dockerfile` — `FROM rust:1.88-slim-bookworm@sha256:38bc5a86…`
  (builder) and `FROM gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae8e98…` (runtime).
- **SHA-pinned actions:** every `uses:` across `.github/workflows/*.yml` is a full commit SHA
  with a `# vX.Y.Z` Dependabot-tracked trailer (release/supply-chain/scorecard quoted above).
- **Verify:** `grep -rn "uses: .*@[a-f0-9]\{40\}" .github/workflows/` shows SHA pins; a bare
  `@v4` tag would be a finding.

## 6. Source-track integrity (SL-S-*)

| Control | File / command |
|---|---|
| Two-person review (AR) | `CODEOWNERS` (catch-all `@jeswr` + high-risk path overrides); `docs/branch-protection.md` records zero required approving reviews, no required code-owner review, no stale-review dismissal, and an always-on repository-administrator bypass; the automated landing path does not use the bypass |
| Protected branch (AR) | `docs/branch-protection.md` — linear history, block force-push, block deletion |
| Single required gate (IV) | `.github/workflows/ci-summary.yml` (`ci-summary / gate`); required-check record in `docs/branch-protection.md` |
| Trusted dep sources (IV) | `deny.toml [sources]` (`unknown-registry/unknown-git = "deny"`); gated by `supply-chain.yml#audit` (`cargo deny check … sources`) |
| Per-dep audit attest (IV) | `supply-chain/{config.toml,audits.toml,imports.lock}`; gated by `supply-chain.yml#vet` (`cargo vet --locked`) |
| Vuln gate (IV) | `supply-chain.yml#audit` (`cargo deny check advisories`); `dependency-monitoring.yml` daily watchdog |
| Least-privilege tokens (IV) | top-level `permissions: contents: read` in every workflow; `persist-credentials: false` on `scorecard.yml` checkout |
| Disclosure channel (IV) | `.well-known/security.txt` (RFC 9116) + `SECURITY.md` |

- **Verify (vet/deny gating):** the jobs run on every PR and `merge_group`; `ci-summary`
  aggregates them as required check-runs (`docs/branch-protection.md` job map). A new
  unaudited/banned dependency fails the PR.

## 7. OpenSSF Scorecard (posture signal feeding SLSA confidence)

**File:** `.github/workflows/scorecard.yml`.

- `ossf/scorecard-action@4eaacf0…` with `publish_results: true` (push-to-main + weekly);
  uploads SARIF to code-scanning + the public OpenSSF dashboard. Scorecard's own checks
  (`Pinned-Dependencies`, `Token-Permissions`, `Branch-Protection`, `Signed-Releases`,
  `SAST`) corroborate several rows above.
- **Verify:** the OpenSSF dashboard entry for `github.com/sparq-org/sparq` + the Security-tab
  code-scanning results.

---

## What the evidence does NOT support (read alongside `gap-register.md`)

- **`dist.yml` binaries are now attested** (GX-9 closed, sq-toze.23): `dist.yml#build` runs
  `actions/attest-build-provenance` (+ cargo-auditable, `--locked`) with `id-token`/`attestations`
  write, so `gh attestation verify dist/sparq-cli-<tier> --repo sparq-org/sparq` succeeds.
- **Published-package provenance — PARTIAL (GX-10 / sq-toze.24, `publish.yml`):**
  - **npm `@sparq-org/sparq` — provenance NOW EMITTED.** `publish.yml#npm` runs `npm publish
    --provenance --access public` in the GitHub-Actions OIDC context; the registry stores a
    Sigstore-signed SLSA provenance statement for the version, and the job's `npm audit signatures`
    step fails if it is absent. **Verify (consumer):** `npm audit signatures` in a project that has
    `@sparq-org/sparq` installed, or inspect the "Provenance" panel on the npmjs.com version page.
  - **crates.io — out-of-band attestation only.** `publish.yml#crates` attests the `cargo package`
    `.crate` bytes with `attest-build-provenance`; **verify** with `gh attestation verify
    <name>-<ver>.crate --repo sparq-org/sparq` against the downloaded crate. crates.io itself stores
    **no provenance link** (no upstream mechanism) — `gh attestation verify` against a crate fetched
    via `cargo` will only succeed if you point it at the attested `.crate` artifact; the registry
    page carries no badge. This is the honest, expected boundary (external sub-gap), not a bug.
  - **PyPI `sparq-rdf` — PEP-740 lane WIRED, awaiting maintainer PyPI config (sq-toze.37).**
    `publish.yml#pypi-build` (maturin matrix) + `#pypi-sdist` build the wheels+sdist; `#pypi-publish`
    uploads them via PyPI **Trusted Publishing** with native **PEP-740 attestations**
    (`pypa/gh-action-pypi-publish` `attestations: true`, OIDC `id-token: write`, GitHub `environment:
    pypi`). PyPI then records an in-toto/Sigstore-signed provenance statement per file. **Verify
    (consumer):** the "Provenance" panel on the PyPI release-files page, or `pypi-attestations verify`
    / `gh attestation verify`. **NOT-yet-true caveat:** this lane only emits attestations once a
    maintainer registers the Trusted Publisher on the `sparq-rdf` PyPI project (owner `sparq-org`, repo
    `sparq`, workflow `publish.yml`, env `pypi`) — a PyPI-account act that cannot be a tracked repo
    file. Until then the upload step fails to mint a token by design (no static API token is stored).
    Do NOT claim PyPI provenance is *emitted* until that registration is confirmed live.
- **No Build L3** evidence exists **yet**, and the L2 attestations must not be read as L3. The
  *mechanism* now exists for every artifact the `release.yml`/`dist.yml` pipelines publish except the container image — `release.yml#provenance` (archives,
  sq-toze.25), `release.yml#provenance-artifacts` (GUI bundles, SBOM/VEX, conformance report) and
  `dist.yml#provenance` (tiered binaries), both of the latter added by #4570 — each calling the
  isolated `slsa-github-generator` trusted builder in its own job. But `release.yml` is
  tag/dispatch-triggered and `dist.yml` dispatch-only, so **no run has produced a
  `sparq-cli-<version>.intoto.jsonl` / `sparq-artifacts-<version>.intoto.jsonl` /
  `sparq-dist.intoto.jsonl`, and nobody has verified one**. Wiring is not evidence: this line stays
  "no L3 evidence" until a `v*` tag emits bundles that `slsa-verifier verify-artifact <file>
  --provenance-path <bundle> --source-uri github.com/sparq-org/sparq` accepts. **Where that evidence
  will come from (#4571):** after cutting the Release, `release.yml`'s `verify-provenance` job
  calls `.github/workflows/release-verify.yml`,
  which runs `scripts/verify-release-provenance.sh` over the *published* assets — both bundles
  attached and listed in `SHA256SUMS`, `SHA256SUMS` matching the published bytes, and every asset
  accepted by `slsa-verifier` against one of the two bundles (an uncovered asset, an empty asset
  set, or a bundle covering nothing it published all red the run). The uploaded
  `provenance-verification.log` + its run URL are the evidence to cite here, and the flip is
  bounded to the artifacts that actually verified. **Verify (auditor, any published tag):**
  `scripts/verify-release-provenance.sh --tag vX.Y.Z` — same checks, run locally. The **ghcr container
  image** is the one artifact with no isolated lane at all — still attested in-band and L2 by
  construction (GX-11, narrowed twice).
- **Reproducible build — characterised, not yet bit-for-bit** (GX-8 / sq-toze.9):
  [`reproducible-build.md`](./reproducible-build.md) records a measured double-build of
  `sparq-cli` (`--release --locked`, same tier flags) → **identical size, byte-identical apart
  from 22 bytes**, all from **one** source (the C-compiled `mimalloc` `__DATE__`/`__TIME__`
  `.rodata` banner + the build-id it perturbs). SLSA does not *require* reproducibility for
  L2/L3, but CRA integrity + consumer trust want it; the honest non-repro reason is now
  documented (not a bare "no evidence"). Residual to a byte-for-byte claim — `SOURCE_DATE_EPOCH`
  (or feature-drop) + a CI rebuild-and-diff ratchet — keeps the bead open. **Verify:** the
  auditor quick-run in that doc.
