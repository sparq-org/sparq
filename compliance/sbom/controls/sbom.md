<!-- [OPUS-4.8] SBOM control spine — epic sq-toze / bead sq-toze.12. One row per applicable control.
     This is the table the SBOM auditor checks. Evidence paths are repo-relative. -->
# SBOM + supply-chain — control → status → evidence → owner

Status legend (honesty contract):
- **Implemented & verified** — a technical control in the codebase/CI with checkable evidence.
- **Audit-ready** — control + documentation in place; the *attestation* needs an external body /
  process we cannot substitute for (labelled).
- **Gap** — not met; tracked in `gap-register.md` with severity + a `bd` bead.

Evidence paths: `path/to/file` is the artifact/config enforcing the control; `wf.yml#job` is the CI
gate (job names match `.github/workflows/*.yml` `jobs.<id>.name`). Commands under "verify" are
reproducible locally. The "SBOM-server probe" rows cite a real `cargo cyclonedx --all` run against
this branch (recorded in `evidence.md §3`).

## A. NTIA minimum elements (the 7 baseline SBOM data fields)

Source: NTIA, *The Minimum Elements for a Software Bill of Materials* (2021). Mapped to the CycloneDX
SBOM that `scripts/gen-sbom-vex.sh` produces per released binary.

| # | NTIA element | Status | Evidence | Owner |
|---|---|---|---|---|
| N1 | **Supplier name** (of each component) | **Implemented & verified** ([OPUS-4.8], sq-toze.26; GS-1 RESOLVED) | `cargo-cyclonedx` 0.5.9 leaves the per-component `supplier`/`publisher` slot empty, so `scripts/sbom-normalize.jq` now derives a per-component `supplier` (CycloneDX `organizationalEntity` = the NTIA Supplier-Name slot) **honestly** from each component's identity in the raw output: a crates.io-published crate (`registry+…`) → `supplier.name = "crates.io"` + its `crates.io/crates/<name>` URL (the distributor of record), with the crate's `author` carried into `publisher` where present (144/166); a first-party workspace crate (`path+file…/crates/sparq-*`) → `supplier = {name:"Jesse Wright", url:["https://github.com/sparq-org/sparq"]}` (matching the VEX top-level supplier); a vendored `[patch.crates-io]` crate (`…/vendor/spargebra`) → `supplier.name = "crates.io"` (its supplier-of-record — **not** the sparq project, which would be fabrication); anything else → **omitted** (no fabrication — none occurs today, so coverage is 100%). Both `sparq-cli`/`sparq-server` SBOMs validate against the official CycloneDX 1.5 schema with `supplier.name` on **every** component (174/174, 166/166). CI-gated by `supply-chain.yml#sbom-supplier` (`scripts/check-sbom-supplier.py` + self-test). The component `name` + crates.io `purl` continue to transitively identify the supplier-of-record. See `evidence.md §3` + `§8`. | SPARQ |
| N2 | **Component name** | **Implemented & verified** | Every component carries `components[].name` (probe: 166/166). `scripts/gen-sbom-vex.sh#L37` (`cargo cyclonedx --all`). | SPARQ |
| N3 | **Version of the component** | **Implemented & verified** | `components[].version` populated for all (probe: 166/166, e.g. `sparq-core@0.1.0`). | SPARQ |
| N4 | **Other unique identifiers** (PURL) | **Implemented & verified** | `components[].purl` = `pkg:cargo/<name>@<version>` for every component (probe: 166/166). | SPARQ |
| N5 | **Dependency relationship** | **Implemented & verified** | `dependencies[]` graph emitted (probe: 167 nodes incl. root). CycloneDX `dependencies` block links each `ref` to its `dependsOn` set. | SPARQ |
| N6 | **Author of the SBOM data** | **Implemented & verified (tooling-attested)** | `metadata.tools` = `{vendor: CycloneDX, name: cargo-cyclonedx, version: 0.5.9}` identifies the SBOM author/tool. The *human/org* author is the supplier in the VEX + the `release.yml#sbom` workflow identity (Sigstore attestation binds it to the org's GitHub workflow). Note: per-NTIA this field is "author of the SBOM data" (the tool/identity that produced it), which IS present — distinct from N1 (supplier of each component). | SPARQ |
| N7 | **Timestamp** | **Implemented & verified** | `metadata.timestamp` populated at generation (probe: an RFC-3339 instant, varies per run, e.g. `2026-06-15T23:49Z`); re-stamped per release in the VEX by `scripts/gen-sbom-vex.sh#L58`. | SPARQ |

**NTIA verdict:** **7 of 7 elements met** (N1,N2,N3,N4,N5,N6,N7). **N1 (per-component Supplier Name)
is now RESOLVED** ([OPUS-4.8], sq-toze.26): `scripts/sbom-normalize.jq` derives a per-component
`supplier` honestly from each component's identity (crates.io-published → `crates.io`; first-party
workspace crate → the project; vendored upstream → `crates.io`; otherwise omitted, none today), so
`supplier.name` is populated on **every** component (166/166, 174/174), schema-valid, CI-gated by
`supply-chain.yml#sbom-supplier`. N6 is met at tool-author granularity (the NTIA-intended reading).

## B. CycloneDX completeness

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| CDX-1 | Valid CycloneDX JSON (recognised `bomFormat` + `specVersion`) | **Implemented & verified** | Probe: `bomFormat=CycloneDX`, `specVersion=1.5`, `serialNumber` present. Generated by `cargo cyclonedx --format json --spec-version 1.5`; validated against the official CycloneDX 1.5 JSON schema ([OPUS-4.8], sq-toze.28). | SPARQ |
| CDX-2 | License data per component | **Implemented & verified** | Probe: `licenses` present on 166/166 components. Corroborated by the cargo-deny licenses gate (only permissive licenses + scoped exceptions) — `deny.toml` `[licenses]`; `supply-chain.yml#audit`. | SPARQ |
| CDX-3 | Spec version current (1.5/1.6) | **Implemented & verified** ([OPUS-4.8], sq-toze.28; GS-4 RESOLVED) | The SBOM is now `specVersion 1.5` — both call-sites pass `cargo cyclonedx --all --format json --spec-version 1.5` (native; 0.5.9 accepts 1.3/1.4/1.5), matching the **VEX** (`1.5`). `scripts/sbom-normalize.jq` populates the 1.5-only `metadata.lifecycles` slot (`[{"phase":"build"}]`, the only honestly-assertable phase). Both `sparq-cli`/`sparq-server` SBOMs validate against the official CycloneDX 1.5 JSON schema. 1.6 not adopted (pinned tool tops out at 1.5; no 1.6-only field populatable without fabrication). | SPARQ |
| CDX-4 | External references (repo / download URL) | **Implemented & verified** | Probe: `externalReferences` present on components; root `purl` carries the source. | SPARQ |

## C. VEX (Vulnerability Exploitability eXchange)

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| VEX-1 | A VEX document exists | **Implemented & verified** | `supply-chain/vex.cdx.json` (CycloneDX 1.5, 4 vulnerabilities). | SPARQ |
| VEX-2 | VEX states exploitability for **every** ignored advisory | **Implemented & verified** | The VEX carries exactly one statement per `deny.toml [advisories].ignore` id — `RUSTSEC-2024-0436` (paste) and `RUSTSEC-2025-0141` (bincode) as `not_affected` with a CycloneDX `justification` + detail, `RUSTSEC-2026-0194`/`RUSTSEC-2026-0195` (quick-xml) honestly as `exploitable`. ([OPUS-5] sq-5ah3p dropped `RUSTSEC-2025-0134` `rustls-pemfile` from deny.toml, the VEX and the cargo-vet exemptions together, having migrated the mTLS PEM parse to `rustls-pki-types`' `PemObject`.) | SPARQ |
| VEX-3 | VEX kept **1:1 in sync** with the dependency-policy ignore list | **Implemented & verified** | `deny.toml` `[advisories].ignore` = exactly the same four RUSTSEC IDs; the VEX `_comment` mandates the 1:1 invariant. **Now CI-enforced (GS-5 RESOLVED, sq-toze.29):** `scripts/check-vex-deny-drift.py` set-equates the deny.toml ignore-id set (parsed via `tomllib`, robust to the `{id,reason}` form) with the VEX `vulnerabilities[].id` set and exits non-zero on unjustified drift; wired as the GATING job `.github/workflows/supply-chain.yml#vex-deny-sync`, which also runs the hermetic self-test `scripts/tests/test_vex_deny_drift.py`. Source of truth = deny.toml (the enforced gate). Verified: in sync this branch (both = {2024-0436, 2025-0141, 2026-0194, 2026-0195}); negative test confirms removing any deny.toml ignore makes the check fail (exit 1). | SPARQ |
| VEX-4 | Per-release VEX published with the version stamped | **Audit-ready** (config-verified; operating-verification pending first release) | `scripts/gen-sbom-vex.sh#L49-67` stamps the released version + timestamp into `sparq-<version>.vex.cdx.json`; attached to the Release by `release.yml#release`. The script + wiring are reviewed and the **checked-in** VEX is verified, but **no per-release VEX has been published yet** (0 releases, 2026-06-15). | SPARQ |

## D. Signed / attested SBOM

> **Operating-effectiveness caveat (release-gated rows SIG-1/2/3):** these controls fire only on
> `push: tags: v*`, and **no `v*` tag, GitHub Release, or Sigstore attestation exists yet** (verified
> 2026-06-15: `git tag -l 'v*'` → 0, `gh release list` → empty, `gh api .../attestations/...` → 404).
> They are therefore **config-verified** (workflow wiring reviewed and correct), **not
> operating-verified** — no attested SBOM/VEX artifact has yet been produced. Status reads
> **"Audit-ready"** for the operating dimension; re-verify after the first `v*` release with the
> `gh attestation verify` / cosign commands cited.

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| SIG-1 | SBOM + VEX are **SLSA build-provenance attested** (Sigstore-signed) | **Audit-ready** (config-verified; operating-verification pending first release) | `release.yml#sbom` → `actions/attest-build-provenance` (SHA-pinned `a2bbfa2…`) over `sbom/*.sbom.cdx.json` + `sbom/*.vex.cdx.json` — wiring reviewed. **Not yet executed** (0 releases/tags/attestations, 2026-06-15). Verify after first release: `gh attestation verify <file> --repo sparq-org/sparq`. | SPARQ |
| SIG-2 | SBOM + VEX covered by a release checksum manifest | **Audit-ready** (config-verified; operating-verification pending first release) | `release.yml#release` "Generate SHA256SUMS" runs `sha256sum -- *` over **all** assets incl. SBOM+VEX; release notes: `shasum -a 256 -c SHA256SUMS`. Wiring reviewed; **no release has produced a SHA256SUMS yet**. | SPARQ |
| SIG-3 | Container image carries an **embedded SBOM + SLSA provenance** | **Audit-ready** (config-verified; operating-verification pending first release) | `release.yml#docker` buildkit `provenance: mode=max` + `sbom: true` (release.yml:L250-251) → SBOM + max-mode provenance attached to the ghcr.io image; verifiable via cosign / `gh`. Wiring reviewed; **no image has been built/pushed by a release yet**. | SPARQ |

## E. Per-release publication

> **Operating-effectiveness caveat (release-gated rows PUB-1/PUB-2):** as in section D, these fire on
> `v*` tags and have **never executed** (no releases yet, 2026-06-15) — config-verified, not
> operating-verified. PUB-3 (checked-in VEX) and PUB-4 (per-push CI SBOM) are **not** release-gated:
> they run on every push/PR and are fully operating-verified.

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| PUB-1 | An SBOM is generated on **every** release | **Audit-ready** (config-verified; operating-verification pending first release) | `release.yml#sbom` runs `scripts/gen-sbom-vex.sh` on tag `v*` (one SBOM per released binary). Wiring reviewed; **no release has produced one yet** (the per-push CI SBOM under PUB-4 *is* operating-verified). | SPARQ |
| PUB-2 | SBOM + VEX **attached as release assets** | **Audit-ready** (config-verified; operating-verification pending first release) | `release.yml#release` (`softprops/action-gh-release`, SHA-pinned) attaches `*.sbom.cdx.json` + `*.vex.cdx.json`; release-notes template documents the asset names. Wiring reviewed; **no release assets exist yet**. | SPARQ |
| PUB-3 | A checked-in **source-of-truth** for the VEX (not only a CI artifact) | **Implemented & verified** | `supply-chain/vex.cdx.json` is committed; the release stamps a per-version copy. (The SBOM itself is generated per release, not checked in — correct, since it is version-specific.) | SPARQ |
| PUB-4 | CI also emits an SBOM artifact on every push/PR (continuous transparency) | **Implemented & verified** | `supply-chain.yml#sbom` ("generate CycloneDX SBOM") uploads `**/*.cdx.json` as artifact `sbom-cyclonedx`, `if-no-files-found: error`. Runs on every push/PR — operating-verified. | SPARQ |

## F. Dependency transparency & gating

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| DEP-1 | **No known-vulnerable / yanked** dependency at PR time | **Implemented & verified** | `supply-chain.yml#audit` "cargo-deny check (advisories) — GATING"; `deny.toml` `yanked="deny"` + advisories v2 fail-closed. GX-1 (degraded gate) is **resolved** (sq-toze.2 CLOSED). Defence-in-depth: `dependency-monitoring.yml` daily watchdog (cron `13 5 * * *`) opens a single `security:dependency-vuln` tracking issue. | SPARQ |
| DEP-2 | **No banned / duplicate-version** crate | **Implemented & verified** | `supply-chain.yml#audit` "cargo-deny check (bans + sources + licenses) — GATING"; `deny.toml` `[bans]`. | SPARQ |
| DEP-3 | Crates come **only from crates.io** (no rogue git/registry) | **Implemented & verified** | `deny.toml` `[sources]` `unknown-registry="deny"`, `unknown-git="deny"`; the vendored `spargebra` is a `[patch.crates-io]` PATH source (not a registry/git source). Gated in `supply-chain.yml#audit`. | SPARQ |
| DEP-4 | Every dependency carries a **human audit attestation** (cargo-vet) | **Implemented & verified** | `supply-chain.yml#vet` "cargo-vet … — GATING" runs `cargo vet --locked` on every push/PR; trusted import sets in `supply-chain/config.toml` `[imports.*]` (Mozilla, Google, Bytecode Alliance, ISRG, Embark); ratchet blocks unaudited new deps. (GX-7 / **sq-toze.8 CLOSED**; control is **wired** + operating-verified on PR.) | SPARQ |
| DEP-5 | Shipped binaries **embed their dependency manifest** (cargo-auditable) | **Implemented & verified** (build step) — release-asset `cargo audit bin` verification pending first release | `release.yml#package` `cargo auditable build` (release.yml:L87) and `Dockerfile:L70` `cargo auditable build` are wired (GX-7 / **sq-toze.8 CLOSED**). The Docker image build path is exercised in CI; the **release-binary** `cargo audit bin <binary>` round-trip cannot be observed until the first `v*` release (0 releases, 2026-06-15). | SPARQ |
| DEP-6 | Permissive-license-only policy (license transparency) | **Implemented & verified** | `deny.toml` `[licenses]` allowlist + per-crate scoped exceptions; gated `supply-chain.yml#audit`. | SPARQ |

## G. SBOM-to-artifact integrity (binding the SBOM to what actually shipped)

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| INT-1 | SBOM built from the **actually-shipped feature set** (not `--all-features`) | **Implemented & verified** | `scripts/gen-sbom-vex.sh#L19-22` uses **default** features, matching `release.yml` / `Dockerfile` which build `-p sparq-cli`/`-p sparq-server` without `--all-features`. | SPARQ |
| INT-2 | SBOM evaluated against the **committed `Cargo.lock`** | **Implemented & verified** | Release/Docker builds use `--locked`; cargo-cyclonedx resolves the same `Cargo.lock`. cargo-vet runs `--locked`. | SPARQ |
| INT-3 | **Reproducible build** linking SBOM → bit-identical binary | **Gap (characterised, not enforced)** — see GS-2 (GX-8) | The honest reproducibility statement now exists ([`../slsa/reproducible-build.md`](../../slsa/reproducible-build.md)): a measured double-build of `sparq-cli` is **byte-identical apart from 22 bytes**, all from **one** source (the `mimalloc` build-time `__DATE__`/`__TIME__` `.rodata` banner + the build-id it perturbs). The SBOM↔binary link is asserted via provenance (SIG-1) + the embedded manifest (DEP-5); an auditor still cannot rebuild a **bit-identical** binary, but the non-determinism source is named, and the residual is the `SOURCE_DATE_EPOCH`/feature-drop + CI rebuild-and-diff enforcement. P2. Bead sq-toze.9. | SPARQ / external |
| INT-4 | **Canonical, host-independent purls** (no build-path / qualifier leak) — CI-asserted | **Implemented & verified** ([OPUS-4.8], sq-tmyw) | Every purl in the normalized SBOM matches `^pkg:cargo/[^?#]+@[^?#]+$`. Backstop for the GS-6/GS-7 normalizer: the GATING job `supply-chain.yml#sbom-purl-canonical` regenerates+normalizes the SBOM and runs `scripts/check-sbom-purl-canonical.py` (self-test `scripts/tests/test_sbom_purl_canonical.py`), so a future cargo-cyclonedx bump that re-introduces purl decoration FAILS the PR. Verified PASS this branch (all purls canonical); negative cases (download_url qualifier, `#src/…` subpath, a hypothetical future qualifier, non-cargo purl) FAIL in the self-test. See `evidence.md §6`. | SPARQ |

## H. JS / npm SBOM (published WASM client — scope coverage)

| ID | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| JS-1 | A CycloneDX SBOM exists for the **published npm surface** (`js/` = `@sparq-org/sparq`, the WASM client) | **Implemented & verified** ([OPUS-4.8], sq-toze.27; GS-3 RESOLVED) | `scripts/gen-js-sbom.sh` emits CycloneDX **1.5** from the committed `js/package-lock.json` (pinned `@cyclonedx/cyclonedx-npm@5.0.0`, `--package-lock-only`). Runtime tree (`--omit dev`): 1 component (`pkg:npm/fzstd@0.1.1`). Full build tree: 5 components. Wired GATING `supply-chain.yml#js-sbom` + per-release `release.yml#sbom`. See `evidence.md §7`. | SPARQ |
| JS-2 | The JS SBOM **validates** against the CycloneDX 1.5 schema | **Implemented & verified** ([OPUS-4.8], sq-toze.27) | cyclonedx-npm built-in `--validate` (default on) + independent `jsonschema` check against the official `bom-1.5.schema.json` (+ referenced `spdx`/`jsf`): **VALID** for both runtime + full-tree SBOMs. `evidence.md §7`. | SPARQ |
| JS-3 | **Scope is documented + honest** (what's in/out of the JS SBOM) | **Implemented & verified** ([OPUS-4.8], sq-toze.27) | `js/` (`@sparq-org/sparq`) IN — published to npm. `site/` (`sparq-site`) OUT — `"private": true`, never published; the Next.js demo's dev tree is covered by npm Dependabot, not a shipped artifact. Rationale in `scripts/gen-js-sbom.sh` header + `gap-register.md` GS-3 + `evidence.md §7`. | SPARQ |

## Summary counts

**35 control rows**, classified by the honesty-contract status legend:

- **Implemented & verified — 28:** N1–N7, CDX-1/2/3/4, VEX-1/2/3, PUB-3/4, DEP-1..6, INT-1/2/4,
  JS-1/2/3. Each runs on every push/PR (or is a checked-in artifact) and cites a file path, CI job, or
  a recorded probe. (N1 became Implemented & verified with sq-toze.26 — per-component supplier name
  now derived honestly + CI-gated. CDX-3 became Implemented & verified with sq-toze.28 — SBOM now
  CycloneDX 1.5. INT-4 added with sq-tmyw — purl-canonicality CI assertion. JS-1/2/3 added with
  sq-toze.27 — JS/npm SBOM for the published WASM client.)
- **Audit-ready (config-verified; operating-verification pending first `v*` release) — 6:**
  SIG-1/2/3, PUB-1/2, VEX-4. These are release-gated (`push: tags: v*`); the workflow wiring is
  reviewed and correct, but **no release/tag/attestation exists yet** (verified 2026-06-15), so no
  attested artifact has been produced. DEP-5 is a hybrid — the `cargo auditable build` step is wired
  + the Docker path is CI-exercised, but the release-binary `cargo audit bin` round-trip awaits the
  first release (it is counted under Implemented & verified for its build step, with the qualifier in
  its row).
- **Gap (recorded + beaded) — 1 control row:** INT-3/GS-2 (reproducible build, GX-8, bead sq-toze.9).
  (N1/GS-1, per-component supplier name, is now RESOLVED — sq-toze.26 — supplier on every component.
  CDX-3/GS-4, spec version, is RESOLVED — sq-toze.28 — SBOM at CycloneDX 1.5.)

**Tracked open gap *items* (gap-register.md) — 1 (GS-2):** GS-2/GX-8 (reproducible build, bead
sq-toze.9 — maps to control row INT-3). Plus the **RESOLVED**
GS-1 / sq-toze.26 (N1 per-component supplier name — now derived honestly via `scripts/sbom-normalize.jq`,
CI-gated `supply-chain.yml#sbom-supplier`),
GS-3 / sq-toze.27 (JS-lockfile SBOM — now `JS-1/2/3` rows + `gen-js-sbom.sh` + CI/release wiring),
GS-6 / sq-toze.30 (F-6: SBOM root `bom-ref` abs-path leak — sanitised), GS-7 / sq-uujh (workspace/
build-target purls canonical, now also CI-asserted via INT-4 / sq-tmyw —
`supply-chain.yml#sbom-purl-canonical`), GS-4 / sq-toze.28 (spec version — SBOM now 1.5), and
GS-5 / sq-toze.29 (VEX↔deny drift automation — now the GATING CI job
`supply-chain.yml#vex-deny-sync`, so VEX-3 is fully Implemented & verified with no residual sub-gap).

- **Overclaim audit:** none. Every "Implemented & verified" row cites a file path, CI job, or a
  recorded probe; the release-gated rows are downgraded to **Audit-ready** rather than overclaiming
  operating effectiveness on a repo with zero releases.
