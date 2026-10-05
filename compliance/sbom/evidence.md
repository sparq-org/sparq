<!-- [OPUS-4.8] SBOM evidence pack — epic sq-toze / bead sq-toze.12. Reproducible commands + recorded
     probe of a real `cargo cyclonedx` run on branch cert-sbom. NON-CANONICAL timing. -->
# SBOM + supply-chain — evidence pack

This pack gives the SBOM auditor reproducible commands and the **recorded output** of a real SBOM
generation on this branch, so every "implemented & verified" row in `controls/sbom.md` is checkable
without re-deriving it. All paths are repo-relative; all CI job names match
`.github/workflows/*.yml` `jobs.<id>.name`.

## 1. The artifacts (checked-in source-of-truth)

| Artifact | Path | What it is |
|---|---|---|
| VEX (source of truth) | `supply-chain/vex.cdx.json` | CycloneDX 1.5 VEX; 4 vulnerabilities — `RUSTSEC-2024-0436` (paste) / `RUSTSEC-2025-0141` (bincode) `not_affected`, `RUSTSEC-2026-0194` + `RUSTSEC-2026-0195` (quick-xml) honestly `exploitable`. ([OPUS-5] sq-5ah3p dropped `RUSTSEC-2025-0134` `rustls-pemfile` from both sides: the mTLS PEM parse moved to `rustls-pki-types`' `PemObject`, so the archived crate left the tree.) |
| SBOM+VEX generator | `scripts/gen-sbom-vex.sh` | Produces per-binary SBOM + version-stamped VEX into `./sbom/` for the release. |
| Dependency policy | `deny.toml` | cargo-deny advisories/bans/sources/licenses; `[advisories].ignore` = the 4 RUSTSEC IDs the VEX mirrors 1:1. |
| cargo-vet config | `supply-chain/config.toml`, `supply-chain/audits.toml`, `supply-chain/imports.lock` | Trusted import sets + exemptions; gates new unaudited deps. |

## 2. The CI / release wiring (cite these job + step names)

| Control | Workflow#job | Step (verbatim) |
|---|---|---|
| Advisories/bans/sources/licenses gating | `supply-chain.yml#audit` ("cargo-deny (advisories + bans + sources + licenses)") | "cargo-deny check (bans + sources + licenses) — GATING" + "cargo-deny check (advisories) — GATING" |
| Per-dependency audit attestations | `supply-chain.yml#vet` ("cargo-vet … — GATING") | "cargo-vet check — GATING" (`cargo vet --locked`) |
| CI SBOM artifact (every push/PR) | `supply-chain.yml#sbom` ("generate CycloneDX SBOM") | "Generate SBOM …" + "Upload SBOM artifact" (`sbom-cyclonedx`) |
| Per-release SBOM + VEX | `release.yml#sbom` ("CycloneDX SBOM + VEX") | "Generate per-release SBOM + VEX" (`scripts/gen-sbom-vex.sh`) |
| SBOM + VEX SLSA attestation | `release.yml#sbom` | "Attest build provenance (SBOM + VEX)" (`actions/attest-build-provenance`, SHA-pinned `a2bbfa2…`) |
| cargo-auditable embedded manifest | `release.yml#package` + `Dockerfile:L70` | "Build …" `cargo auditable build --release --locked` |
| Release asset attach + checksums | `release.yml#release` ("create GitHub Release") | "Generate SHA256SUMS" + "Create release" (`softprops/action-gh-release`, SHA-pinned) |
| Image SBOM + provenance | `release.yml#docker` ("build + push container") | "Build and push" `provenance: mode=max` + `sbom: true` |
| Daily advisory watchdog (defence-in-depth) | `dependency-monitoring.yml#audit` ("cargo-deny advisories -> tracking issue") | cron `13 5 * * *`; opens/updates `security:dependency-vuln` issue |

## 3. Recorded probe — NTIA elements on a real SBOM (branch cert-sbom)

Reproduce:

```sh
cargo cyclonedx --all --format json --spec-version 1.5   # [OPUS-4.8] sq-toze.28 (GS-4): 1.5, matches the VEX
# then inspect crates/sparq-server/sparq-server.cdx.json
```

Recorded result for `crates/sparq-server/sparq-server.cdx.json` (cargo-cyclonedx 0.5.9, default
features, this branch — re-run **2026-06-15** to verify the `author`/`supplier` counts below):

| Field | Observed value | NTIA / CDX control |
|---|---|---|
| `bomFormat` / `specVersion` | `CycloneDX` / `1.5` ([OPUS-4.8] sq-toze.28: `--spec-version 1.5`, matches the VEX) | CDX-1 (valid); CDX-3 ✅ (GS-4 RESOLVED) |
| `metadata.lifecycles` | `[{"phase":"build"}]` (1.5-only slot, injected by `scripts/sbom-normalize.jq`) | CDX-3 (1.5 lifecycle metadata) ✅ |
| `serialNumber` | present | CDX-1 |
| `metadata.timestamp` | `2026-06-15T23:49Z` (varies per run) | N7 ✅ |
| `metadata.tools` | `{vendor: CycloneDX, name: cargo-cyclonedx, version: 0.5.9}` | N6 ✅ (SBOM-author = tool) |
| `metadata.authors` / `metadata.supplier` | **absent** in raw output; per-component `supplier` injected by `scripts/sbom-normalize.jq` (sq-toze.26) | N1 ✅ (GS-1 RESOLVED) |
| components count | **166** | — |
| `components[].name` | present 166/166 | N2 ✅ |
| `components[].version` | present 166/166 (e.g. `sparq-core@0.1.0`) | N3 ✅ |
| `components[].purl` | present 166/166 (`pkg:cargo/<name>@<ver>`) | N4 ✅ |
| `components[].licenses` | present 166/166 | CDX-2 ✅ |
| `components[].externalReferences` | present | CDX-4 ✅ |
| `components[].author` | **present 144/166** (e.g. `spargebra → "Tpt <thomas@pellissier-tanon.fr>"`, `aho-corasick → "Andrew Gallant <jamslam@gmail.com>"`); the 22 without it are the 3 first-party workspace crates `sparq-core`/`sparq-engine`/`sparq-serve` + 19 deps (`axum`, `axum-core`, `crossbeam-*`, `rayon`, `rayon-core`, `hashbrown`, `futures-*`, `either`, `equivalent`, `typenum`, `quick-xml`, `pin-project-lite`, `find-msvc-tools`) | N6/N1 — component-author identity present on the majority |
| `components[].supplier` / `.publisher` | **absent in RAW output (0/166)**; after `scripts/sbom-normalize.jq` (sq-toze.26) `supplier.name` is present on **166/166** (`publisher` on 144/166, from `author`) | **N1 ✅ (GS-1 RESOLVED)** — see §8 |
| `dependencies[]` graph | present, 167 nodes (incl. root) | N5 ✅ |

**Honest reading (updated for sq-toze.26):** cargo-cyclonedx 0.5.9 *raw* output emits 6/7 NTIA
elements (N2,N3,N4,N5,N6,N7), leaving only the dedicated per-component `supplier`/`publisher` slot
empty (**0/166**). The publication normalizer `scripts/sbom-normalize.jq` now **derives** a per-component
`supplier` honestly from each component's identity (see §8), populating `supplier.name` on **166/166**
(and carrying `author`→`publisher` on 144/166), so the seventh element — *Supplier Name* (**N1/GS-1**) —
is now **met** in the published SBOM. The generated SBOM is `specVersion 1.5` ([OPUS-4.8],
sq-toze.28 — `cargo cyclonedx … --spec-version 1.5`), matching the VEX (`1.5`), and carries the
1.5-only `metadata.lifecycles` slot (`[{"phase":"build"}]`, injected by `scripts/sbom-normalize.jq`).

> **SBOM at CycloneDX 1.5 — spec-version gap RESOLVED (CDX-3/GS-4, bead sq-toze.28, [OPUS-4.8]):**
> Both SBOM call-sites (`scripts/gen-sbom-vex.sh` for the two released-binary SBOMs, and
> `supply-chain.yml#sbom` for the CI artifact) now pass `cargo cyclonedx --all --format json
> --spec-version 1.5`. cargo-cyclonedx 0.5.9 emits 1.5 **natively** (it accepts `--spec-version`
> `1.3`/`1.4`/`1.5`; default 1.3), so this is **not** a post-process specVersion bump — the tool
> writes a genuine 1.5 document. `scripts/sbom-normalize.jq` additionally populates the 1.5-only
> `metadata.lifecycles` array with the single phase that is honestly assertable for a build-time
> SBOM, `build`. No other 1.5/1.6-only field is fabricated. **1.6 was NOT adopted:** the pinned tool
> tops out at 1.5, and no 1.6-only field (e.g. cryptographic-asset components, formulation) could be
> populated truthfully without fabrication. **Validation:** both `sparq-cli`/`sparq-server` SBOMs were
> validated against the official CycloneDX **1.5** JSON schema (`bom-1.5.schema.json` + `spdx`/`jsf`
> sub-schemas) — both VALID; `metadata.lifecycles` present; 0 host-path leaks (the abs-path guard still
> passes); every `dependencies[].dependsOn` ref resolves to a component `bom-ref` (graph consistent);
> the normalizer is idempotent on the 1.5 output. The VEX (`supply-chain/vex.cdx.json`, already 1.5) is
> unchanged, so SBOM and VEX now share one spec version.

<!-- separate adjacent blockquotes (markdownlint MD028) -->

> **Root/component refs are normalized — abs-path leak RESOLVED (F-6/GS-6, bead sq-toze.30, [OPUS-4.8]):**
> cargo-cyclonedx 0.5.9 *raw* output stamps the absolute build dir into every workspace/path-dependency
> `bom-ref` (`path+file:///<abs-path>/crates/sparq-server#0.1.0`) and `purl`
> (`pkg:cargo/sparq-server@0.1.0?download_url=file://.`), so an unprocessed published SBOM would carry
> the **CI runner's absolute path**. The **published** SBOMs are now post-processed by
> `scripts/sbom-normalize.jq` (invoked from `scripts/gen-sbom-vex.sh` for the two released-binary SBOMs,
> and from `supply-chain.yml#sbom` over every CI-uploaded `*.cdx.json`), which rewrites each such ref to
> the canonical, host-independent `pkg:cargo/<name>@<version>` form and strips the whole
> `download_url=file://…` purl qualifier together with any trailing build-target `#src/…` subpath
> (the latter extended in bead `sq-uujh`, [OPUS-4.8] — it was the only residual non-canonical purl,
> e.g. `pkg:cargo/sparq-cli@0.1.0#src/main.rs`), while rewriting the dependency graph (root `bom-ref`, nested build-target sub-components,
> and every `dependencies[].ref` / `dependsOn[]` edge) in lock-step so all internal references still
> resolve. The transform is deterministic (pure function of the input — no time/host/RNG) and idempotent
> (a second pass is byte-identical). Both `gen-sbom-vex.sh` and `supply-chain.yml#sbom` additionally
> fail loudly if any `path+file://`, `download_url=file://`, or `/home/` survives. Verified locally
> against freshly-generated `sparq-cli`/`sparq-server` SBOMs: 0 host-path leaks, valid CycloneDX shell,
> all dependency refs resolve (167/175 respectively), idempotent. *Residual:* the abs-path leak is fixed
> only in the **post-processed publication path**; raw `cargo cyclonedx` still emits path refs (upstream
> behaviour, not changed here).

<!-- separate adjacent blockquotes (markdownlint MD028) -->

> The probe files (`**/*.cdx.json`) are **not committed** — `scripts/gen-sbom-vex.sh#L47` and the
> probe both delete them to keep the worktree clean. They are regenerable with the command above.

## 3a. Upstream verification — cargo-cyclonedx path-member purl/bom-ref defect (GS-6/GS-7; bead sq-4qo8 [OPUS-4.8])

GS-6/GS-7 are *resolved in sparq's published output* by the post-process normalizer
(`scripts/sbom-normalize.jq`), but the **root cause is upstream**: cargo-cyclonedx emits
non-canonical, host-revealing `purl`/`bom-ref` for every workspace / path member. Bead
**sq-4qo8** is the deferred upstreaming of that fix (file an issue/PR against
`CycloneDX/cyclonedx-rust-cargo`). This section records the **verification** half of sq-4qo8 —
that the defect *persists in the latest release* — and the **ready-to-file dossier** so the
owner-gated filing is one click. (Filing publicly under the project's GitHub identity to a
third-party community repo is a `needs:user` action per `AGENTS.md` §"Upstream blockers"; the
agent does not post it unprompted.)

**Latest release verified.** `cargo-cyclonedx` **0.5.9** is the latest published release
(crates.io / CHANGELOG, dated **2026-03-19**); the upstream `main` CHANGELOG has **no
"Unreleased" entry** touching purls/bom-refs/`download_url`. So the defect is current, not stale.

**Reproduction (deterministic, tool-only — no sparq code involved).** A minimal two-crate
workspace (`foo` with a `path` dependency on `bar`):

```sh
cargo cyclonedx --all --format json --spec-version 1.5   # cargo-cyclonedx 0.5.9
```

emits, for the path/workspace members:

| Field | Raw 0.5.9 output | Defect |
|---|---|---|
| root `purl` | `pkg:cargo/foo@0.1.0?download_url=file://.` | `download_url=file://.` is a *useless* download URL — not a real location (cf. upstream issue [#612]) |
| member `purl` | `pkg:cargo/bar@0.2.0?download_url=file://../bar` | host-layout-derived qualifier; non-canonical purl |
| build-target `purl` | `pkg:cargo/foo@0.1.0?download_url=file://.#src/lib.rs` | additionally carries a host `#src/…` subpath fragment |
| `bom-ref` / `dependsOn` | `path+file:///<ABS-BUILD-DIR>/crates/foo#0.1.0` | leaks the **absolute build-machine path** into the published SBOM |

Registry deps are already canonical
(`pkg:cargo/<name>@<version>` with the registry encoded in `bom-ref`) and are untouched.

**Pinpointed root cause (upstream source, `main`).**

- *purl* — `cargo-cyclonedx/src/purl.rs#get_purl`: for a local package (`package.source` is
  `None`) it unconditionally adds `builder.with_qualifier("download_url", &manifest_url)` where
  `manifest_url = format!("file://{}", package_dir)`. In-workspace it is made *relative* via
  `diff_utf8_paths` (→ `file://.` / `file://../…`); for a path dep **outside** the workspace it
  stays **absolute** (the upstream `local_package` unit test asserts
  `file:///home/shnatsel/Code/cargo-cyclonedx/cyclonedx-bom`). The `#src/…` subpath is appended by
  `with_subpath(to_purl_subpath(subpath))` on build-target sub-components.
- *bom-ref* — `cargo-cyclonedx/src/generator.rs#create_component`: the component `bom-ref` is
  `package.id.to_string()`, and for a local package `cargo metadata` reports `PackageId.repr` as
  the absolute `path+file:///…#<version>` form, which is propagated verbatim into `bom-ref` and
  every `dependencies[].ref` / `dependsOn[]` edge.

**Related existing upstream issue.** [#612] ("Replace qualifier with optional namespace for local
packages", open since 2024-02, no maintainer action) already argues the `download_url=file://.`
qualifier is meaningless and proposes a `pkg:cargo/<namespace>/<name>@<version>` namespace form.
It does **not** cover the absolute-path `bom-ref` leak or the build-target `#src/…` subpath — the
sq-4qo8 dossier extends it to the full defect surface.

**Ready-to-file upstream content (issue + PR sketch).**

- *Issue title:* "Path/workspace members get non-canonical, host-revealing purls + absolute-path
  bom-refs". *Body:* the reproduction table + root-cause file/function refs above; ask for
  canonical `pkg:cargo/<name>@<version>` purls (drop the `download_url=file://` qualifier and the
  `#src/…` subpath, or move identity into the optional namespace per #612) and a relative-or-omitted
  `bom-ref` so no absolute build path is emitted. Cross-link #612.
- *PR sketch (smallest defensible upstream change):* in `get_purl`, when `package.source` is `None`
  and the package is **not** under `workspace_root`, **omit** the `download_url` qualifier (it cannot
  be made host-independent) rather than emitting the absolute path; and gate the build-target
  `#src/…` subpath behind a `--include-source-subpath`-style opt-in (or drop it) so the default purl
  is canonical. The `bom-ref` change (relative-ise the local `package.id`) is a larger,
  maintainer-coordination change and is offered as a follow-up. The exact patch is left for the PR
  so it tracks `main` HEAD at filing time.

**What this section is NOT.** It does not claim the upstream fix has landed. sparq's *own*
published SBOMs are already canonical via the normalizer + CI backstop (§6) — that is the
roll-your-own half (GS-6/GS-7, `DONE`). sq-4qo8 stays **OPEN / `needs:user`** until the upstream
issue/PR is actually filed; this section makes that filing one click and records the proof it is
still needed.

[#612]: https://github.com/CycloneDX/cyclonedx-rust-cargo/issues/612

## 4. VEX ↔ deny.toml sync verification (GS-5 — automated, sq-toze.29 [OPUS-4.8])

The drift between the published VEX and the enforced cargo-deny gate is now a **GATING CI check**, not
a manual inspection (GS-5 RESOLVED). **Source of truth: `deny.toml [advisories].ignore`** — the list
cargo-deny actually enforces; the VEX must carry exactly one `vulnerabilities[].id` per ignored
advisory. The check asserts set equality and fails (exit 1) on any unjustified one-sided entry.

```sh
# the gate (parses deny.toml via tomllib + the VEX via json; robust to the {id,reason} ignore form)
python3 scripts/check-vex-deny-drift.py        # -> "VEX ↔ deny.toml in sync: 4 advisory id(s) match 1:1."
python3 scripts/tests/test_vex_deny_drift.py   # hermetic self-test (11 cases incl. the drift-fail paths)
```

Wired as `.github/workflows/supply-chain.yml#vex-deny-sync` (job name
`VEX ↔ deny.toml sync (GS-5) — GATING`; no `advisory`/`informational` whole word, so ci-summary gates
it). Recorded this branch: both = `{RUSTSEC-2024-0436, RUSTSEC-2025-0141, RUSTSEC-2026-0194,
RUSTSEC-2026-0195}` — **in sync** (no drift to resolve). ([OPUS-5] sq-5ah3p removed
`RUSTSEC-2025-0134` from both sides in one change, which is exactly the edit shape this gate exists to
police.) Negative test: temporarily dropping any one of the four deny.toml ignores makes the check exit 1 and name
the offending id. A genuinely-intended one-sided entry is recorded with a reason in the script's
`JUSTIFIED_DRIFT` allow-list (empty today).

## 5. Provenance verification (for a consumer)

```sh
gh attestation verify sparq-cli-<ver>.sbom.cdx.json --repo sparq-org/sparq   # SLSA provenance on the SBOM
shasum -a 256 -c SHA256SUMS                                              # release-asset integrity
cargo audit bin sparq-server                                            # read the embedded manifest (cargo-auditable)
cosign verify-attestation ghcr.io/sparq-org/sparq@<digest> ...              # image SBOM + provenance
```

These are the consumer-facing verification steps a downstream high-security integrator runs; they
correspond to SIG-1/SIG-2, DEP-5, and SIG-3 respectively.

> **Operating-effectiveness status (release-gated controls):** all four commands above are
> **unrunnable today** — `release.yml` triggers only on `push: tags: v*`, and the repo has **no
> `v*` tags, no GitHub Releases, and no Sigstore attestations yet** (verified `git tag -l 'v*'` → 0,
> `gh release list` → empty, `gh api repos/sparq-org/sparq/attestations/...` → 404, 2026-06-15). The
> SIG-*/PUB-*/VEX-4/DEP-5 controls are therefore **verified at the configuration level (workflow
> wiring reviewed and correct), not at the operating level** — no attested SBOM/VEX artifact has ever
> been produced. An external auditor must re-verify these by cutting (or observing) the first `v*`
> release and re-running the commands above against its assets. The control rows in
> `controls/sbom.md` are labelled **"Audit-ready (config-verified; operating-verification pending
> first release)"** accordingly.

## 6. purl-canonicality assertion (GS-6/GS-7 backstop — [OPUS-4.8], sq-tmyw)

The GS-6/GS-7 normalizer (`scripts/sbom-normalize.jq`) is hand-written against the exact purl
decoration `cargo-cyclonedx 0.5.9` emits (`?download_url=file://…`, `#src/…`). A future tool bump
could re-introduce a *different* decoration the pattern misses, silently re-leaking a host path or a
non-canonical purl into the published SBOM. `scripts/check-sbom-purl-canonical.py` is the **backstop**:
it asserts every purl in a normalized SBOM matches `^pkg:cargo/[^?#]+@[^?#]+$` (no `?query`, no
`#fragment`), reporting any offenders.

```sh
# regenerate + normalize (the exact publication transform), then assert canonical purls
cargo cyclonedx --all --format json --spec-version 1.5
for f in $(find . -name '*.cdx.json'); do jq -f scripts/sbom-normalize.jq "$f" > "$f.norm" && mv "$f.norm" "$f"; done
python3 scripts/check-sbom-purl-canonical.py $(find . -name '*.cdx.json')
python3 scripts/tests/test_sbom_purl_canonical.py   # hermetic self-test (9 cases incl. live workspace SBOM)
```

Recorded this branch (2026-06-16): **PASS** — all purls canonical (`sparq-server` 166 components /
169 purls, `sparq-cli` 174 components, every member + build-target + root purl
`pkg:cargo/<name>@<version>`; the per-member SBOMs likewise). Negative coverage (self-test): a
`?download_url=file://…` qualifier, a `#src/main.rs` subpath, a *hypothetical future*
`?repository_url=…` qualifier, a non-cargo purl, and an empty purl-set each FAIL the check. Wired as
the GATING job `.github/workflows/supply-chain.yml#sbom-purl-canonical` (job name
`SBOM purl-canonicality assertion (GS-6/GS-7) — GATING`; no `advisory`/`informational` whole word, so
ci-summary gates it), which runs the self-test then the live regenerate→normalize→assert.

## 7. JS / npm SBOM — shipped clients (GS-3 — [OPUS-4.8], sq-toze.27; [GPT-5.6], sq-epbw4)

The published npm package `@sparq-org/sparq` and the shared `@sparq/client` code bundled into the
site/GUI artifacts have dedicated CycloneDX 1.5 SBOMs, closing their JS supply-chain surfaces.

**Scope decision (honest):**

| Workspace | npm name | Shipped? | SBOM'd? | Why |
|---|---|---|---|---|
| `js/` | `@sparq-org/sparq` | **published to npm** | **YES** | the consumable WASM client; its lockfile tree is a real supply-chain surface |
| `packages/sparq-client/` | `@sparq/client` (`"private": true`) | **bundled into site/GUI artifacts** | **YES** | its runtime tree includes the lazy zstd and bzip2 codecs shipped to browser consumers |
| `site/` | `sparq-site` (`"private": true`) | never (GitHub-Pages demo) | **NO** (intentional) | not a shipped artifact; dev/showcase tree (Next.js/React/bb.js) covered by npm Dependabot |

```sh
scripts/gen-js-sbom.sh # writes runtime + development SBOMs for both JS clients
```

Two views per client are shipped (all CycloneDX **1.5**, matching the Rust SBOM + VEX;
`--package-lock-only`, with transient member locks derived deterministically from the committed
root workspace lock and no dependency resolution):

| Artifact | Tree | Components (this branch) | Audience |
|---|---|---|---|
| `sparq-js-<ver>.sbom.cdx.json` | runtime (`--omit dev`) | **1** — `pkg:npm/fzstd@0.1.1` | what a CONSUMER of `@sparq-org/sparq` installs (matches the published-tarball dep surface) |
| `sparq-js-dev-<ver>.sbom.cdx.json` | full build tree | `@sparq-org/sparq` runtime plus build dependencies | BUILD-time surface (SSDF parity with the Rust full-tree SBOM) |
| `sparq-js-client-<ver>.sbom.cdx.json` | runtime (`--omit dev`) | `fzstd`, `seek-bzip`, browser `buffer`, and their closure | codec dependencies bundled on demand into browser artifacts |
| `sparq-js-client-dev-<ver>.sbom.cdx.json` | full build tree | `@sparq/client` runtime plus build dependencies | full shared-client build surface |

**Validity:** all are VALIDATED against the official CycloneDX 1.5 JSON schema — cyclonedx-npm's built-in
`--validate` (default on) plus an independent `jsonschema` check against
`CycloneDX/specification` `bom-1.5.schema.json` (+ referenced `spdx`/`jsf` schemas): **VALID**, root
root components `pkg:npm/%40sparq-org/sparq@0.1.0` and `pkg:npm/%40sparq/client@0.1.0`, all component
purls `pkg:npm/…`. The generator also fails unless the shared-client runtime SBOM contains both
lazy codecs. Wired as the per-PR GATING
job `.github/workflows/supply-chain.yml#js-sbom` (uploads `sbom-js-cyclonedx`) + the per-release step
`.github/workflows/release.yml#sbom` "Generate per-release JS/npm SBOM" (the `sbom/*.sbom.cdx.json`
attest + attach + checksum globs already cover the JS SBOMs, so they are SLSA-attested and on the
Release alongside the Rust SBOMs).

## 8. Per-component supplier name (NTIA N1 — GS-1 RESOLVED, sq-toze.26 [OPUS-4.8])

cargo-cyclonedx 0.5.9 leaves the per-component `supplier`/`publisher` slot empty, so the literal NTIA
*Supplier Name* minimum element was unpopulated per component (only the 1.3-era `author` field was
present, on 144/166). The publication normalizer `scripts/sbom-normalize.jq` now derives a per-component
`supplier` (CycloneDX `organizationalEntity`) **honestly** — classifying each component by the identity
signal in the RAW cargo-cyclonedx `bom-ref` (read BEFORE the bom-ref is canonicalised), never fabricating
one where the class is undeterminable.

**What each component class gets (honest derivation):**

| Component class | Raw `bom-ref` signal | `supplier.name` | `supplier.url` | `publisher` | Rationale |
|---|---|---|---|---|---|
| crates.io-published dependency | `registry+https://github.com/rust-lang/crates.io-index#…` | `crates.io` | `https://crates.io/crates/<name>` | the crate's `author` where present (144/166); else absent | crates.io is the distributor / supplier-of-record |
| first-party workspace crate | `path+file…/crates/sparq-*#…` (+ the root component & its build-target sub-components) | `Jesse Wright` | `https://github.com/sparq-org/sparq` | — (these crates carry no `author` in raw output) | the project authors + ships these; matches the VEX top-level `metadata.supplier` |
| vendored `[patch.crates-io]` crate (today only `spargebra`) | `path+file…/vendor/<name>#…` | `crates.io` | `https://crates.io/crates/<name>` | its `author` (`Tpt <…>`) | a crates.io-published crate vendored as a patch replacement → crates.io is its supplier-of-record, **NOT** the sparq project (attributing it to sparq would be fabrication) |
| anything else | — | **omitted** | — | — | not determinable → omitted honestly per NTIA's omit/mark-unknown guidance. **None occurs today**, so coverage is **100%** |

**Before / after (a real registry component, `aho-corasick`, this branch):**

```jsonc
// BEFORE (raw cargo cyclonedx --all --spec-version 1.5)
{ "name": "aho-corasick", "version": "1.1.4",
  "author": "Andrew Gallant <jamslam@gmail.com>",
  "purl": "pkg:cargo/aho-corasick@1.1.4" }              // no supplier, no publisher

// AFTER (jq -f scripts/sbom-normalize.jq)
{ "name": "aho-corasick", "version": "1.1.4",
  "author": "Andrew Gallant <jamslam@gmail.com>",
  "supplier": { "name": "crates.io",
                "url": ["https://crates.io/crates/aho-corasick"] },
  "publisher": "Andrew Gallant <jamslam@gmail.com>",
  "purl": "pkg:cargo/aho-corasick@1.1.4" }
```

The first-party `sparq-core` gets `supplier {name:"Jesse Wright", url:["https://github.com/sparq-org/sparq"]}`;
the vendored `spargebra` gets `supplier {name:"crates.io", url:["https://crates.io/crates/spargebra"]}` +
`publisher "Tpt <thomas@pellissier-tanon.fr>"` (NOT attributed to sparq).

**Properties:** the derivation is a pure function of the raw `bom-ref`/`author` (deterministic), is
**non-destructive** (never overwrites a `supplier` already present, so a future cargo-cyclonedx that
populates it wins) and **idempotent** (the canonical output carries the derived supplier; a second pass
is byte-identical). It runs at BOTH publication call-sites via the same `scripts/sbom-normalize.jq`
(`scripts/gen-sbom-vex.sh` for the two released-binary SBOMs; `supply-chain.yml#sbom` for the CI
artifact).

**Validation:** both `sparq-cli`/`sparq-server` normalized SBOMs were validated against the official
CycloneDX **1.5** JSON schema (`bom-1.5.schema.json` + `spdx`/`jsf` sub-schemas) — **VALID** with
`supplier.name` on **every** component (`sparq-server` 166/166, `sparq-cli` 174/174); the existing
purl-canonicality, `metadata.lifecycles`, abs-path-leak guard, and dependency-graph consistency all
remain intact, and the transform stays idempotent.

```sh
# reproduce: generate -> normalize -> assert supplier on every component
cargo cyclonedx --all --format json --spec-version 1.5
jq -f scripts/sbom-normalize.jq crates/sparq-server/sparq-server.cdx.json > /tmp/s.json
python3 scripts/check-sbom-supplier.py /tmp/s.json   # -> "All N cargo component(s) carry supplier.name …"
python3 scripts/tests/test_sbom_supplier.py          # hermetic self-test + live workspace assertion
```

Wired as the GATING job `.github/workflows/supply-chain.yml#sbom-supplier` (job name
`SBOM per-component supplier-name assertion (GS-1) — GATING`; no `advisory`/`informational` whole word,
so ci-summary gates it), which regenerates+normalizes the SBOM and asserts every cargo component carries
a `supplier.name` — so a future cargo-cyclonedx bump that changes the component shape and silently stops
the derivation FAILS the PR rather than shipping an SBOM regressed to the empty-supplier state. Negative
coverage (self-test): a component missing its supplier, a blank/`name`-less supplier object, and a
supplier hiding only on the root `metadata.component`/sub-components each FAIL the check; the live test
additionally asserts the vendored `spargebra` is supplied by `crates.io` (never the project).
