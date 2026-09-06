# Release runbook

How to cut a sparq release. Everything below is **maintainer-triggered**: the version PR merge
cuts the tag, while registry uploads require either the one-time bootstrap commands or an
explicit `publish.yml` dispatch.

## 0. One-time pre-release steps (before the first complete v0.1.1 release)

[GPT-5.6] `v0.1.0` already points at an abandoned release-plz CI commit and cannot be
reused safely. The first complete release is therefore **v0.1.1**; do not move or delete
the public `v0.1.0` tag as part of the release.

These are tracked as beads (`bd list -l area:release`); the procedure is documented
here for the runbook:

- **Root `LICENSE` — done.** The tracked MIT text is included in release archives by
  `release.yml`; Cargo manifests also declare `license = "MIT"` for crates.io.
- See **§0a crate-name availability** below — checked; all crates.io names are clear, the
  npm scope `@sparq-org/sparq` is clear, and the PyPI **distribution** name `sparq` is taken, so
  the Python wheel publishes as **`sparq-rdf`** (owner-approved; the import name stays
  `sparq` — done, see the Python wheels section).
- A crates.io login is configured locally for the one-time bootstrap (`cargo login`).
- The npm organization **`sparq-org`** exists and the publishing identity has rights to
  create public scoped packages.

## 0a. Crate-name availability

The old 2026-06-14 availability snapshot covered only part of the current workspace and
must not be used to authorize a publish. [GPT-5.6] The authoritative Rust set is derived
from the manifests by `scripts/release-interval-guard.py`: **37 crates**, including every
normal/optional workspace dependency needed by the public front doors. Re-run the registry
checks immediately before the bootstrap because names can be claimed at any time.

**Live re-check, 2026-08-31:** all 37 exact crates.io names returned 404;
`@sparq-org/sparq` and `@sparq-org/solid-server` returned 404; and
`@sparq-org/eyereasoner-compat` reported the existing 0.1.0 publication.

For the **PyPI** row, that re-run is one command (`scripts/check-pypi-name.py --check`,
sq-ed5): it reads the distribution name from `crates/sparq-py/pyproject.toml` (`sparq-rdf`)
and queries the PyPI JSON API with the same 404-available / 200-taken convention, exiting 0
(available) / 1 (taken) / 2 (indeterminate). `--expect available|taken` asserts the expected
state for a `&&` chain before `maturin upload`; `--name <n>` checks an explicit name. (Live
mode needs network; the hermetic decision-table + name-reader self-test runs in CI's
`ci-scripts` lane.) crates.io has no separate pre-flight script — `cargo publish` aborts on a
taken name itself.

| Name | Registry | Required preflight |
|---|---|---|
| the 37 names printed by `python3 scripts/release-interval-guard.py --dry-run` | crates.io | available on 2026-08-31; re-check before bootstrap |
| `@sparq-org/sparq` | npm | available on 2026-08-31; confirm organization rights before bootstrap |
| `@sparq-org/solid-server` | npm | available on 2026-08-31; confirm organization rights before bootstrap |
| `@sparq-org/eyereasoner-compat` | npm | already published at 0.1.0; no bootstrap needed |
| `sparq` | PyPI | **taken** — unrelated `shiventi/sparq` (SJSU degree-planning API client, latest 0.2.6) |
| `sparq-rdf` | PyPI | chosen distribution name (owner-approved; `pip install sparq-rdf`, `import sparq`) |

The PyPI **distribution** name `sparq` is taken, so the Python wheel ships as
**`sparq-rdf`** while the import remains `sparq`. The npm decision from #3399 is
**`@sparq-org` for all public packages**; the private `@sparq/client` workspace package is
an internal build dependency and is not a registry surface.

## 0b. v0.1.1 ships ahead of the external ZK review (issue #2552)

**Decision (maintainer, issue #2552, 2026-07-26): the first complete release goes out without waiting for the
external accredited-cryptographer review of the ZK estate (bead `sq-qhy4`), carrying
experimental warnings instead.** Recorded here because it is a release-scope decision and
nothing in CI encodes it: there is no audit gate in `release.yml`, `release-plz.yml`,
`release-plz.toml` or either release guard, and there never was — the review is a P0 task,
not a release blocker.

What the release ships instead of the review, and what must stay true of every future
release while `sq-qhy4` is open:

- The GitHub Release body carries an **Experimental** paragraph naming `sparq-zk` and
  `sparq-mpc` as research scaffolds, stating that no external accredited cryptographer has
  reviewed them and that the release makes no soundness/security/privacy claim for either,
  and linking `SECURITY.md`. It is pinned by
  `scripts/tests/test_release_publish_guard.py::TestReleaseCarriesTheExperimentalZkCaveat`
  — the release notes are the one surface a downloader who never opens the repo still
  reads, so it is a test, not a convention.
- `SECURITY.md` (§ *Scope and a critical caveat*), `README.md`, `crates/sparq-zk/README.md`,
  `crates/sparq-mpc/README.md`, `skills/zk-query-proofs/SKILL.md` and `skills/mpc/SKILL.md`
  already carry the matching "research scaffold / not externally audited / semi-honest
  only" language, and `scripts/check-privacy-claims.sh` gates against any unqualified
  soundness or privacy claim creeping back in.

The one thing this decision does **not** license: presenting a ZK "verified" result or an
MPC run as a production-grade guarantee anywhere. That stays false until `sq-qhy4` closes.

## 1. Version bump

Rust versions are locked through `[workspace.package] version` and release-plz's single
`version_group`. Every shipped workspace path dependency also has an explicit registry
version. The version PR must update the root version, every path-dependency requirement,
and `Cargo.lock` together; the release guard refuses an incomplete dependency closure.

The first-release PR sets these files explicitly to **0.1.1**. This is deliberately not a
release-plz-generated PR: before the first dependency-first crates.io bootstrap,
`release-plz update` runs `cargo package` while calculating changes and Cargo cannot resolve
the unpublished inter-crate registry dependencies. `release-plz.toml` therefore keeps
`git_only = true` only for the tag phase, using the existing `v0.1.0` tag as its baseline.
After all 37 crates exist, flip `git_only = false` only in the same change that enables
crates.io OIDC publishing; normal release-plz-generated version PRs resume then.

release-plz does not version npm workspaces. The public `@sparq-org/sparq` and
`@sparq-org/solid-server` manifests are explicitly set to **0.1.1** for this release.

## 2. Changelog

Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md` (Keep-a-Changelog format:
Added / Changed / Fixed / Removed). Performance claims go in only with a pointer to the
measurement (e.g. `bench/qlever-baselines.md`). Add the compare/tag link at the bottom.

## 3. Tag push → what CI does

Merging the maintainer-reviewed first-release/version PR runs `release-plz release`. Only
the dependency-final `sparq-cli` entry is allowed to create the workspace tag, so exactly
one `vX.Y.Z` is pushed. Manual fallback only:

```sh
git tag vX.Y.Z
git push origin vX.Y.Z
```

Pushing the tag triggers `.github/workflows/release.yml`:

0. **publish-cadence guard** — the `setup` job runs
   `scripts/release-interval-guard.py --enforce --released-tag vX.Y.Z` before anything is
   built. If the previous release was less than 24h ago the job fails and **every** job
   below it is blocked (they all depend on `setup`). See §8d — this is the tag-push half
   of the at-most-one-release-per-day policy. The tag stays pushed; delete it
   (`git push --delete <remote> vX.Y.Z`) and re-push once the window has passed.
1. **package** — builds `sparq-cli` for every hardware tier (same matrix as `dist.yml`:
   arm64/x64 darwin, x86-64 baseline/v2/v3/v4 + arm64 Linux, x64/arm64 Windows) and packages
   each as `sparq-cli-vX.Y.Z-<tier>.tar.gz` (`.zip` on Windows) containing the binary,
   `README.md`, `LICENSE` (if present), and `CHANGELOG.md`.
2. **release** — generates `SHA256SUMS` over all archives and creates the GitHub Release
   with every archive + the checksum manifest attached (plus auto-generated notes linking
   to `CHANGELOG.md`). Verify with `shasum -a 256 -c SHA256SUMS`.
3. **docker** — builds the `Dockerfile` and pushes
   `ghcr.io/sparq-org/sparq-server:{X.Y.Z, X.Y, latest}` using the workflow's own
   `GITHUB_TOKEN` (ghcr needs no extra secrets). **To disable container publishing**,
   delete the `docker` job from `release.yml` (or gate it with `if: false`) — the other
   jobs are independent of it.

Note: [GPT-5] `dist.yml` is manual-dispatch only; it does not run on `v*` tags. Both
`dist.yml` and `release.yml` call the reusable `build-matrix.yml`, which is the single
source of truth for the hardware-tier matrix and build steps. `dist.yml` selects
`mode: binary` for bare per-tier workflow artifacts, while `release.yml` selects
`mode: archive` for the versioned release archives described above.

## 3a. The LWS / Solid server binary — container only, no `dist`/`release` archive

<!-- [OPUS-5] sq-gg0qq.11 (issue #2741): the bead asks for a decision, recorded here so the
runbook is the single place a releaser looks. Related: research/lws-3-crate-split.md §3
(blast radius) and its §7 Q2 (the published image name is still a maintainer question). -->

**Decision: the Solid/LDP server binary is NOT added to `dist.yml` or to the `release.yml`
archives. It ships only as a container image, and it is labelled EXPERIMENTAL.**

`crates/sparq-lws-core` builds an implicit binary from `src/main.rs` (there is no `[[bin]]`
stanza and the crate is `publish = false`). A **second** workflow fires on a `v*` tag alongside
`release.yml` (`dist.yml` is dispatch-only, per the note above) —
`.github/workflows/lws-container.yml` — which smoke-tests
(`crates/sparq-lws-core/tests/container-smoke.sh`) and Trivy-scans a native amd64 image, then
pushes a multi-arch `linux/amd64,linux/arm64` index to
`ghcr.io/sparq-org/sparq-lws-core:{X.Y.Z, X.Y, latest}` with SBOM and max-mode build
provenance. It is also `workflow_dispatch`-able, which tags only `:latest` — that is the form
the demo and the `deploy/` templates reference. Nothing else about the LWS binary is published.

Why not a `dist`/`release` archive:

- **The crate's own description says `EXPERIMENTAL`**, and its whole distribution surface is a
  long-lived server process configured entirely through `SOLID_SERVER_*` / `PSS_*` environment
  variables. A bare `solid-server` archive next to `sparq-cli-vX.Y.Z-<tier>.tar.gz` reads as a
  supported product; a container carries the runtime contract with it and is the honest shape.
- **The crate boundary is still moving.** `sparq-lws` and `sparq-solid-server` do not exist yet
  and the partition that would create them is an open maintainer decision
  (research/lws-3-crate-split.md §5/§7); the container/deploy cutover is deliberately sequenced
  last in that record's §6 phase 4, because it is the only phase that can break a running demo.
  Adding hardware-tiered release archives now would pin a binary NAME (`sparq-lws-core` today,
  `solid-server` after the cutover) that the split is expected to change, and
  `.github/workflows/deploy-lint.yml` already asserts the current image string literally.
- **Cost.** `build-matrix.yml` builds ten hardware tiers. Doubling it for a binary whose
  supported deployment is a container buys nothing a consumer has asked for.

**Revisit when** (any one is enough): the `sq-gg0qq` split lands and the bin has a stable
crate + name; or a consumer needs a non-container deployment. The change is then small — add a
package/bin selector to `build-matrix.yml` — and it should ship as a clearly EXPERIMENTAL-
labelled OPT-IN artifact, not silently alongside the `sparq-cli` archives.

## 4. crates.io publication

**37 crates publish.** [GPT-5.6] This is the complete crates.io dependency closure, not
just the top-level product crates. `scripts/release-interval-guard.py` derives the set and
the dependency-first order directly from the workspace manifests, refuses public-to-private
path edges, and requires a registry version on every shipped workspace dependency. This
closure includes `sparq-algos`, a core-only leaf with full crates.io metadata.

Exact bootstrap commands, from the repo root on the tagged **v0.1.1** commit:

```sh
cargo publish -p sparq-core
cargo publish -p sparq-fedplan
cargo publish -p sparq-http3
cargo publish -p sparq-jsonld
cargo publish -p sparq-reason-ql
cargo publish -p sparq-secprop-vocab
cargo publish -p sparq-shaclc
cargo publish -p sparq-algos
cargo publish -p sparq-canon
cargo publish -p sparq-engine-serialize
cargo publish -p sparq-engine-service
cargo publish -p sparq-hdt
cargo publish -p sparq-introspect
cargo publish -p sparq-sim
cargo publish -p sparq-substrate
cargo publish -p sparq-wrapper

# CHECKPOINT before sparq-engine: the crates.io package resolves UPSTREAM spargebra 0.4.6,
# not the vendored copy (the [patch]/path override is stripped on publish). Dry-run it
# against upstream first — it must package + compile cleanly:
#   cargo publish --dry-run -p sparq-engine
cargo publish -p sparq-engine
cargo publish -p sparq-reason
cargo publish -p sparq-reason-el
cargo publish -p sparq-vc
cargo publish -p sparq-arrow
cargo publish -p sparq-geo
cargo publish -p sparq-nlq
cargo publish -p sparq-policy
cargo publish -p sparq-rsp
cargo publish -p sparq-serve
cargo publish -p sparq-shacl
cargo publish -p sparq-text
cargo publish -p sparq-zk
cargo publish -p sparq-forms
cargo publish -p sparq-trust
cargo publish -p sparq-vectors
cargo publish -p sparq-solid
cargo publish -p sparq-terse
cargo publish -p sparq-mcp
cargo publish -p sparq-server
cargo publish -p sparq-cli
```

- Modern cargo **waits for index propagation** after each publish, so the commands can be
  run back-to-back; if an older cargo complains a dependency isn't found, wait ~a minute
  and retry.
- Before uploading anything, run `cargo package --list -p <crate>` across the set. During
  bootstrap, `cargo publish --dry-run -p <crate>` becomes meaningful only after that crate's
  internal prerequisites exist on crates.io, so run it immediately before each real publish.
  After all 37 bootstraps, run `publish.yml`'s packaging/attestation lane; it now fails unless
  every `.crate` file is produced.
- Crates still marked `publish = false` are outside the registry closure. In particular,
  `sparq-py` ships through PyPI and `sparq-wasm` ships through npm.
- Publishing is **permanent** (versions can only be yanked, not deleted/reused).

> [OPUS-4.8] **Registry-publish signing (sq-jgt3 / GX-OSSF-2).** Scorecard's `Signed-Releases`
> is satisfied by the Sigstore SLSA build-provenance over the GitHub-Release archives
> (`release.yml`) — but that does **not** sign the bytes a consumer installs from a *package
> registry*. Honest per-registry status:
>
> - **crates.io — no first-party signing/provenance scheme exists upstream** (no equivalent of
>   npm `--provenance` or PyPI PEP-740 attestations). The tractable equivalent we DO ship is an
>   out-of-band attestation: [`publish.yml`](../.github/workflows/publish.yml)'s `crates` job runs
>   `cargo package` and attests the resulting `.crate` bytes (identical to what `cargo publish`
>   uploads) with `actions/attest-build-provenance`. This puts **no** provenance link on the
>   crates.io page — that needs upstream support — but a consumer who downloads the `.crate` can
>   `gh attestation verify <file> --repo sparq-org/sparq`. The crates.io-native sub-gap stays **OPEN**
>   (external — see `compliance/openssf/gap-register.md` GX-OSSF-2 / `compliance/gap-register.md`
>   GX-10). Do **not** describe a crates.io publish as "signed".
> - **npm `@sparq-org/sparq` — native Sigstore provenance** via [`publish.yml`](../.github/workflows/publish.yml)'s
>   `npm` job. [OPUS-4.8] sq-v286.11: the job now authenticates with **OIDC trusted publishing**
>   (no `NPM_TOKEN`) — npm exchanges the GitHub Actions OIDC token for a short-lived publish
>   credential and records the Sigstore-signed provenance automatically; consumers verify with
>   `npm audit signatures`. See §8 for the one-time Trusted-Publisher registration.
> - **PyPI `sparq-rdf` — native PEP-740 attestations** via Trusted Publishing (see the
>   "Python wheels" section below + §8) once the maintainer registers the Trusted Publisher.
>
> [OPUS-4.8] sq-v286.11: **crates.io Trusted Publishing** (GA 2025-07, RFC 3691) is now available
> as an *authentication* mechanism (a short-lived OIDC token via `rust-lang/crates-io-auth-action`,
> in place of a long-lived `CARGO_REGISTRY_TOKEN`). It is **not** a provenance/signing scheme — it
> does **not** put a provenance link on the crates.io page — so the "do not describe a crates.io
> publish as signed" caveat above is unchanged. The CI side of the auth flip is pre-wired (§8).

## 5. Docker (manual / local)

CI does this on tag; to build locally:

```sh
docker build -t sparq-server .                      # add --build-arg CARGO_FLAGS="-j 2" to cap parallelism
docker run --rm -p 3030:3030 -v "$PWD/data:/data:ro" sparq-server --format turtle /data/file.ttl
curl 'http://localhost:3030/sparql?query=SELECT%20*%20WHERE%20%7B%3Fs%20%3Fp%20%3Fo%7D%20LIMIT%201'
```

Image: multi-stage (`rust:1.87-slim-bookworm` builder → distroless `cc-debian12:nonroot`
runtime), `--locked` release build, binds `0.0.0.0:3030` (flags appended as CMD args
override the entrypoint defaults — the server's arg parser is last-wins).

## 6. Homebrew

`packaging/homebrew/sparq.rb` is a **formula template**; Homebrew installs from a tap, which
is a separate repo decision. To ship it:

1. Create the tap repo `sparq-org/homebrew-sparq` (one-time).
2. After the GitHub Release exists, copy the template to `Formula/sparq.rb` in the tap,
   set `version`, and replace each `REPLACE_WITH_SHA256_<tier>` with the matching line from
   the release's `SHA256SUMS` asset (tiers used: `arm64-darwin`, `x64-darwin`,
   `arm64-linux`, `x64-v2`).
3. `brew install sparq-org/sparq/sparq` then `brew test sparq` to verify.

Users get `sparq-cli` plus a `sparq` symlink on PATH.

## 7. Post-release

- Check the release page artifacts + `SHA256SUMS`, `docker run ghcr.io/sparq-org/sparq-server:X.Y.Z`,
  and the crates.io pages render the README.
- **Confirm the isolated-builder provenance verified** (issue #4571 / GX-11 / SL-B3-b). After the
  Release is cut, `release.yml`'s `verify-provenance` job calls
  `.github/workflows/release-verify.yml`, which re-downloads every published
  asset and checks, fail-closed, that: both `.intoto.jsonl` bundles are attached and listed in
  `SHA256SUMS`; `SHA256SUMS` matches the published bytes; and `slsa-verifier verify-artifact`
  accepts **every** asset against one of the two bundles (an asset covered by neither reds the
  run). It is part of the release run — look for the `verify published provenance` job on the same
  workflow run that cut the tag; its uploaded `provenance-verification.log` is the evidence record.

  > It is driven from `release.yml` deliberately, and `release-verify.yml`'s `release: published`
  > trigger is only an out-of-band net for a Release published **by hand**. GitHub does not start
  > a workflow run from an event generated by a workflow's own `GITHUB_TOKEN`, and that is the
  > token `release.yml` creates the Release with — so a normal `v*` release emits no
  > run-starting `release` event at all. `scripts/tests/test_verify_release_provenance.sh` pins
  > the caller structurally so this cannot silently regress to trigger-only.

  To run the identical checks by hand — e.g. to re-verify an older tag, or from a consumer's
  machine:

  ```bash
  go install github.com/slsa-framework/slsa-verifier/v2/cli/slsa-verifier@v2.7.1
  scripts/verify-release-provenance.sh --tag vX.Y.Z          # `gh release download` + verify
  scripts/verify-release-provenance.sh --tag vX.Y.Z --dir ./assets   # already-downloaded assets
  ```

  A **red** run means a *published* release does not carry the provenance the compliance estate
  claims for it: treat it as a supply-chain incident and fix forward — never relax the checks and
  never drop `release`'s `needs: [provenance, provenance-artifacts]`, which is what stops a
  Release being cut when a trusted builder fails in the first place.
- **Then, and only then, update the compliance estate.** A green verification run is the evidence
  `compliance/slsa/controls.md` SL-B3-b needs to move from **AR** to **IV** *for the artifacts that
  verified*. Record the run URL in `compliance/slsa/evidence.md` and narrow GX-11 in both gap
  registers. Keep the wording bounded: the verified property is *unforgeable provenance*, not a
  hardened build (the generic generator signs digests our build jobs reported), and the ghcr.io
  container image is still in-band **L2** — so GX-11 narrows, it does not close.
- Bump `[workspace.package] version` to the next `-dev` cycle if desired, and start a new
  `## [Unreleased]` section in `CHANGELOG.md`.

<!-- [OPUS-4.8] release publishing tracked: bead sq-7re (wheels matrix). PyPI name resolved by sq-8slf (was sq-ed5). -->
## Python wheels (PyPI) — wired through Trusted Publishing

`crates/sparq-py` packages the engine as the PyPI distribution **`sparq-rdf`** with the
import name **`sparq`** (`pip install sparq-rdf`, then `import sparq`) — pyo3 + maturin,
`abi3-py39` so one wheel per platform covers CPython ≥ 3.9.

The bare `sparq` distribution name is taken (see §0a), so `[project].name` is `sparq-rdf`.
`[tool.maturin].module-name = "sparq"` deliberately preserves the Python import name.

[`publish.yml`](../.github/workflows/publish.yml) builds manylinux x86_64/aarch64,
macOS arm64/x64, and Windows x64 wheels plus an sdist, then publishes through PyPI OIDC
Trusted Publishing with native PEP-740 attestations. The only remaining prerequisite is
the pending publisher registration in §8b; no manual PyPI bootstrap upload is needed.

<!-- [OPUS-4.8] sq-v286.11 (maintainer #758): CI publishing via OIDC trusted publishing. -->
## 8. CI publishing — OIDC trusted publishers (the one-time `needs:user` registry-side steps)

[OPUS-4.8] sq-v286.11 (maintainer #758): "publishing via CI with semantic release, using trusted
OIDC publishing for npm, and whatever the best practices are for the python ecosystem." The repo
holds only the **workflows**; each registry's **trust config is a one-time maintainer action** in
that registry's web UI (it cannot live in a repo file — that is the whole point of OIDC: the
registry, not a stored secret, decides which workflow it trusts). All three legs use the GitHub
Actions OIDC token (`id-token: write`); **no long-lived `NPM_TOKEN` / `CARGO_REGISTRY_TOKEN` is
stored**. Honest bootstrap note: **npm and crates.io require the package/crate to already exist**
(register the trusted publisher *after* one manual bootstrap publish); **PyPI supports a *pending*
publisher** so the very first publish can be trusted-publishing too.

### 8a. npm packages under `@sparq-org` — WIRED

The primary `npm` job authenticates entirely via OIDC trusted publishing (no `NODE_AUTH_TOKEN`). It pins
`npm@^11.5.1` (the trusted-publishing CLI floor — Node 22's bundled npm can be older) and keeps
`--provenance --access public`.

**needs:user (npmjs.com):** `@sparq-org/sparq` and `@sparq-org/solid-server` must each
exist before npm allows Trusted Publisher configuration:

1. Confirm the `sparq-org` npm organization and the publisher's package-creation rights.
2. From the tagged v0.1.1 checkout, build/inspect each tarball and perform one bootstrap
   publish using a short-lived **granular** token:
   `cd js && NPM_CONFIG_PROVENANCE=false npm publish --access public`, then
   `cd ../packages/solid-server && NPM_CONFIG_PROVENANCE=false npm publish --access public`.
   The explicit override is required because provenance can only be generated on a supported
   hosted CI runner; subsequent trusted-publisher runs generate it automatically.
3. Delete the granular token.
4. On each package's **Settings → Trusted Publisher → GitHub Actions**, configure:
   - Organization or user: **`sparq-org`**
   - Repository: **`sparq`**
   - Workflow filename: **`publish.yml`** (filename only, with extension — **not** a path)
   - Environment: **leave blank** (the `npm` job uses no GitHub Environment)
   - Allowed actions: **`npm publish`**

`@sparq-org/eyereasoner-compat` already exists at 0.1.0, so it only needs its Trusted
Publisher checked. Every subsequent CI publish is tokenless and provenance-bearing
(`npm audit signatures`). The bootstrap versions themselves are the unavoidable pre-OIDC
exception because npm cannot register a publisher for a package that does not yet exist.

### 8b. PyPI `sparq-rdf` — WIRED (`publish.yml` `pypi-publish` job)

Already implemented to current best practice: `pypa/gh-action-pypi-publish` with `attestations: true`
+ OIDC `id-token: write` + GitHub `environment: pypi`, no API token. Emits native PEP-740 provenance.

**needs:user (pypi.org):** PyPI → (project `sparq-rdf` if it exists, else *Your projects → Publishing*
for a **pending** publisher) → **Add a new publisher** → *GitHub*:
   - PyPI Project Name: **`sparq-rdf`**
   - Owner: **`sparq-org`**, Repository: **`sparq`**
   - Workflow name: **`publish.yml`**
   - Environment: **`pypi`** (matches the `pypi-publish` job's `environment:`)

Because PyPI allows a *pending* publisher, no manual bootstrap upload is required.

### 8c. crates.io (37 crates) — CI side PRE-WIRED, flip follows the bootstrap

crates.io Trusted Publishing (GA 2025-07, RFC 3691) supplies a short-lived OIDC token via
`rust-lang/crates-io-auth-action` — no `CARGO_REGISTRY_TOKEN`. The CI side is pre-wired as a
commented block on `release-plz.yml`'s `release-plz-release` job; `release-plz.toml` keeps
`publish = false` until the trust config exists (so a `publish=true` with no credential can't break
tag-cutting). This is the "config-flip" the design record (§6 item 4) calls "the point of adoption".

**needs:user (crates.io), per the 37 publishable crates (`docs/release.md` §4), leaf-first:**
1. ONE bootstrap `cargo publish` per crate (crates.io requires each crate to already exist).
2. For **each** crate: crates.io → crate → **Settings → Trusted Publishing → Add** → *GitHub*:
   - Repository owner: **`sparq-org`**, Repository name: **`sparq`**
   - Workflow filename: **`release-plz.yml`**
   - Environment: **leave blank** (the `release-plz-release` job uses no GitHub Environment)

**Then flip (three coordinated edits):**
- `release-plz.yml`: uncomment `id-token: write`, the `rust-lang/crates-io-auth-action` step
  (SHA-pinned `c6f97d4…` # v1.0.5), and the `CARGO_REGISTRY_TOKEN: ${{ steps.cratesio-auth.outputs.token }}` env.
- `release-plz.toml`: set `publish = true` and `git_only = false` together.
- `publish.yml`'s `crates` job then reverts to attest-only over the `.crate` bytes (release-plz
  becomes the publisher; the out-of-band attestation stays as the verifiable-bytes evidence).

> Provenance honesty: crates.io trusted publishing is an **auth** mechanism only — it does **not**
> put a provenance link on the crates.io page (no upstream scheme exists, unlike npm/PyPI). The
> "do not describe a crates.io publish as signed" caveat in §4 is unchanged.

### 8d. Publish-rate protections (issues #1135, #2552) — what stops a runaway release

**The policy: at most one release per day.** `MIN_RELEASE_INTERVAL` in
`scripts/release-interval-guard.py` is the single constant that states it (24h), and it is
enforced at **both** points a release can start — the Release-PR path (`release-plz.yml`)
and the `v*` tag push (`release.yml`). There is no override flag: releasing inside the
window is done by hand, deliberately.

**A crates.io version can never be unpublished.** Four protections stand between an
automated pipeline and the registry. All four are already in place; none of them is what
you flip.

1. **The Release PR can never be armed.** `scripts/release_pr_guard.py` is the single
   predicate every arming/merging path consults — `auto-arm.py`, `rearm-sweeper.py`, the
   `check-pr-arm-base.py` PreToolUse hook (which is where agent-typed `gh pr merge --auto`
   goes), `batch-merge.py`, `pr-backlog.py`. It keys on **head branch, author and title —
   never a label**, because anything holding `pull-requests: write` can add or remove a
   label. Adding `review:pass` to the Release PR does not make it armable. It fails closed:
   an unknown head branch refuses rather than admits. The Release PR is merged by a
   maintainer, by hand, deliberately.

   **Read "armed" literally.** The PreToolUse hook recognises `gh pr merge` **with**
   `--auto`. A direct `gh pr merge <n> --squash` (no `--auto`), `--admin`, a
   `gh api graphql … enablePullRequestAutoMerge` mutation, a REST
   `PUT …/pulls/<n>/merge`, a backslash line-continuation, or shell-variable indirection
   all reach `gh` unblocked — verified by executing the hook against a fake `gh`. That is a
   deliberate scope (the hook governs *arming*), not an oversight, and it is why
   protection 2 exists: the interval guard runs inside `release-plz.yml` itself and does
   not care how the merge happened. If the guard script itself cannot run,
   `.claude/settings.json`'s wrapper **denies** any `gh pr merge` rather than allowing it.
2. **A minimum release interval.** `scripts/release-interval-guard.py --enforce` runs in
   `release-plz.yml`'s `release-plz-release` job **before** the tag/publish step.
   `MIN_RELEASE_INTERVAL` is 24 hours, measured from `max(newest v* tag date, newest
   crates.io publication)`. It refuses on any indeterminacy — shallow checkout, unreadable
   tag list, unparseable date, unreachable crates.io, a future-dated last release. A
   definitive crates.io 404 is the only accepted "never published" answer. There is no
   override flag: publishing inside the window is done by hand, consciously.
3. **The same interval, on the tag-push path** (issue #2552). Protection 2 only covers
   releases that go through the Release PR. §3's canonical instruction — push a `vX.Y.Z`
   tag — fires `release.yml` **directly**, which was previously uncadenced: a hand-pushed
   tag (or a script pushing one) could cut a release minutes after the last. The guard now
   also runs in `release.yml`'s `setup` job, the job every other job there depends on, so a
   refusal stops the archives, the SBOM/VEX, the GitHub Release and the ghcr image.

   It runs there with `--released-tag vX.Y.Z`, and that flag is load-bearing: on a tag push
   `v<workspace version>` is already in the tag list, so without it the guard takes its
   "already tagged, nothing to release" branch and allows unconditionally. `--released-tag`
   excludes the tag being cut and suppresses that branch, so the interval is measured
   against the *previous* release. It refuses if handed anything that is not a `vX.Y.Z`
   tag.

   It is **unconditional** — `workflow_dispatch` builds are guarded too. They look like a
   developer/test path (pre-release, `dev-`-prefixed image tags), but the Release they
   create fires `release: published`, and `publish.yml`'s `npm` job runs on *any* `release`
   event, so a dispatch reaches a registry as well. Two consequences: `inputs.tag` on a
   dispatch **must** be a `vX.Y.Z` tag (a `-dev` suffix is fine), and the unbounded way to
   exercise the pipeline is a local build, not a dispatch.
   `scripts/tests/test_release_publish_guard.py` pins the step's `run:`, that it carries
   no `if:` and no `continue-on-error`, the `fetch-depth: 0` checkout it depends on, and
   that no job in `release.yml` escapes `setup`.
4. **Version-group and registry-closure coverage.** The same guard validates all 37 crates,
   refuses a public crate that points at an unpublished workspace member or lacks a registry
   version requirement, and reports any publishable crate absent from the locked
   `version_group`. Version-group drift is a warning while `publish = false` and a hard
   refusal after the flip; an invalid dependency closure always refuses.

See what would be published, without touching anything:

```sh
python3 scripts/release-interval-guard.py --dry-run
```

It prints the publishable crate list, each version, the dependency-first publish order and
the cadence verdict it *would* return. It only ever runs `git`, never `cargo`.

> [GPT-5.6] **Closure reconciliation complete for v0.1.1:** the guard reports 37
> publishable crates, all 37 are in the `sparq` version group, and the printed order is the
> exact bootstrap order in §4. Re-run it on the final Release PR commit; any mismatch blocks
> the crates.io flip.

### 8e. release-plz forge token — configured and verified (issue #3273)

For the first complete release, the checked-in v0.1.1 version PR replaces the generated
Release-PR because `release-plz update` cannot package the unpublished dependency closure.
The privileged forge token below is still required **before that PR merges**: the
`release-plz release` job uses it to push `v0.1.1` as a normal actor so the tag starts
`release.yml`. Once the 37-crate bootstrap and post-bootstrap config flip are complete,
the same token also restores normal generated Release-PRs.

`release-plz.yml` cannot open the Release-PR with the workflow's own `GITHUB_TOKEN`: the
repo/org setting **Settings → Actions → General → "Allow GitHub Actions to create and approve
pull requests"** is disabled, so every push to `main` ends in
`Failed to open PR … 403 Forbidden … /pulls`. The job is advisory and the known 403 is
contained loudly (signature + live probe), so nothing goes red — but no Release-PR is opened,
which is what the version/changelog automation and the GUI-download release depend on.

Both jobs now pick their forge token in this order, so **no workflow edit is needed** — only
the secret:

```text
App token (ORCHESTRATOR_APP_ID + ORCHESTRATOR_APP_PRIVATE_KEY)  ← preferred
  || RELEASE_PLZ_TOKEN     (fine-grained PAT: contents:write + pull_requests:write)
  || GITHUB_TOKEN          (fallback; cannot open PRs while the setting is disabled)
```

**Live verification, 2026-08-31:** `ORCHESTRATOR_APP_ID` and
`ORCHESTRATOR_APP_PRIVATE_KEY` are present, and run `33434042300` successfully minted the
App token in both the Release-PR and tag jobs. No PAT or Actions-setting change is needed
for v0.1.1.

**Recovery if that App credential is removed or expires — do exactly one:**
1. Provision `ORCHESTRATOR_APP_ID` + `ORCHESTRATOR_APP_PRIVATE_KEY` (the App used by
   `batch-merge.yml`; install it on this repo with contents + pull-requests write), **or**
2. add a `RELEASE_PLZ_TOKEN` repo secret (fine-grained PAT, same two permissions), **or**
3. enable the repo setting above.

Options 1–2 also fix a second, independent problem: GitHub suppresses workflow triggers for
events created with `GITHUB_TOKEN`, so a `v<version>` tag pushed by the `release-plz / tag` job
would **not** fire `release.yml` (`push: tags: ["v*"]`) — the tag→`release.yml` handoff §3
describes. A minted App token / PAT is a normal actor and does fire it. Option 3 alone does not.

Once a privileged token is configured the containment step no longer tolerates a 403 at all: a
provisioned token that still 403s is a real failure (App not installed, PAT expired or
under-scoped) and fails the job. Promotion: after a main run is observed opening/updating the
Release-PR, drop the `(advisory)` token from the job name, delete the containment step and
remove the entry from `.github/advisory-registry.json`.

**You do not have to watch for that moment.** The `release-plz-pr` job self-reports: on every
successful run it re-runs the same side-effect-free PR-creation probe (`POST /pulls` with
`head == base`, which can never create a PR), and the moment that probe returns **422** instead
of **403** — authorization passed, only the body was rejected, i.e. PR-creation is unblocked —
the run emits a `release-plz Release-PR promotion unblocked (sq-lonae)` warning naming the three
edits above. The probe runs even on success because `release-plz release-pr` also exits 0 when
there is simply nothing to release, which says nothing about whether PR-creation is allowed.
A probe that returns anything else (or does not complete) is reported as inconclusive and never
claims readiness, and the step can never fail the release path.
