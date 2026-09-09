# Automated release CI + multi-platform builds — design record (epic sq-v286) [OPUS-4.8]

> 🤖 SPARQ agent. Design-for-maintainer-review. This is a **design record**, not an
> implementation, and per the maintainer's instruction on `sq-v286` the *build* is
> gated on current engine features landing — **design now, build later**. It selects a
> concrete semantic-release mechanism for sparq's Rust workspace, defines the
> multi-platform build matrix (flagging feasible-now vs needs-the-GUI-epic vs
> needs-user-credentials), and — load-bearing — specifies that the new pipeline must
> **reuse** the substantial release/publish/SBOM/provenance estate that already exists,
> not duplicate it.
>
> **Honesty mandate.** Every claim about sparq is traced to a real file (`.github/workflows/*`,
> `Cargo.toml`, `crates/*`, `docs/release.md`, `.beads/issues.jsonl`). No performance
> numbers are asserted — this is release tooling and the work box is non-canonical.
> sparq is a Rust **library** engine: "building for Android/iOS" means language
> **bindings** (uniffi / cargo-ndk / xcframework / JNI), **not** a shippable mobile app —
> an actual mobile-app artifact is the separate GUI epic `sq-ixc3`. Platform support is
> not overstated: desktop/server CLI + browser-WASM only; no native mobile today.

---

## 1. Problem statement — what is actually missing

sparq already has a mature, tag-driven, attestation-emitting release estate (Section 5).
What it does **not** have is the one thing the epic asks for: an automated way to
**decide the next version, write the changelog, and cut the tag**. Today those three
steps are manual (`docs/release.md` §1–§2). The single strongest piece of evidence that
the manual path has fallen behind:

- `CHANGELOG.md` declares Keep-a-Changelog + Semantic Versioning adherence (lines 4–6),
  but its `## [Unreleased]` section is **empty** — there is nothing between the
  `## [Unreleased]` heading (line 8) and the `## [0.1.0] - 2026-06-13` heading (line 10) —
  despite hundreds of `feat`/`fix` commits landing since the `0.1.0` entry. The
  hand-maintained changelog is stale; this is the core argument for automation.

Supporting facts, all verified in this checkout:

- **Versioning is a single locked version.** All 36 workspace crates set
  `version.workspace = true` (verified 2026-06-19: `grep -l 'version.workspace = true' crates/*/Cargo.toml`
  returns 36), inheriting one value `[workspace.package] version = "0.1.0"`
  (`Cargo.toml:18`). There is no independent per-crate versioning. Several crates also
  carry hard-coded `version = "0.1.0"` pins on internal path-deps, so a bump touches two
  kinds of location — the dual-bump chore `docs/release.md` documents by hand.
- **Commits are already conventional, by discipline not by gate.** History uses
  `feat(scope): …` / `fix:` / `docs:` etc., but there is **no** commitlint / PR-title
  check in any workflow, and **zero** `BREAKING CHANGE:` / `type!:` markers exist — so the
  minor-vs-patch signal a SemVer tool reads is present, but the major-bump signal is not
  yet exercised.
- **There is no SemVer tag.** `git tag` lists exactly one tag, `pr670base` (not a
  version). No `v0.1.0` git tag exists in this checkout, even though the `0.1.0`
  changelog row does. (This matters for tool selection — Section 2.)

---

## 2. Recommendation — `release-plz` (with `git-cliff` as its changelog engine)

**Adopt `release-plz`** as the version + changelog + tag driver, in its **release-PR**
model, with **`git-cliff`** as the changelog engine (release-plz embeds git-cliff). **Do
not** adopt the npm `semantic-release` tool as the primary driver. This is not a new idea
to the repo: `docs/release.md:70` already records *"`cargo release` or `release-plz` can
automate 1–3 later; not adopted yet."* This record confirms which of those two, and
identifies the one config key that preserves the locked-version model.

### Why release-plz and not the alternatives

| Tool | Infers version from commits? | Writes changelog? | Tags + publishes? | Workspace-aware (dep-order publish)? | Verdict for sparq |
|---|---|---|---|---|---|
| **release-plz** | Yes — Conventional Commits **plus** `cargo-semver-checks` API-break detection | Yes (via git-cliff) | Yes — Release-PR → on merge tags + `cargo publish` in dep order | Yes, first-class | **Adopt** |
| npm `semantic-release` | Yes | Yes | Only via npm-installed Rust bolt-ons (`semantic-release-cargo`) | No native multi-crate dep-DAG publish | Reject as primary — wrong layer (Node toolchain for a Rust release; new supply-chain surface) |
| `git-cliff` | No | Yes | No | n/a | Adopted **transitively** — it is release-plz's changelog engine |
| `cargo-release` | No (operator picks the bump) | No | Yes | Yes | Good for the *manual* chore, but does not deliver auto-inference the epic asks for |
| `cargo-smart-release` | Yes, **per-crate independent** | Yes | Yes | Yes | Mismatched — built for independent versioning, which this repo deliberately does not use |

The decisive points for release-plz over the others:

1. **It anchors on the registry, not on git tags.** release-plz compares local crates
   against what is published on crates.io to compute the next version. Since **there is
   no `v0.1.0` git tag** in this repo (Section 1), a tag-anchored tool (release-please
   style) has nothing to read; release-plz does not need a tag to exist.
2. **Stronger break detection.** Conventional-commit parsing alone would miss a real
   API break committed under a `fix:`/`feat:` message; release-plz layers
   `cargo-semver-checks` on top. (Pre-1.0 the API is explicitly unstable per the `0.1.0`
   changelog note, but this matters before any 1.0.)
3. **Changelog format already matches.** git-cliff emits Keep-a-Changelog, which is
   exactly the existing `CHANGELOG.md` format (lines 4–6).
4. **It automates the dual-bump + dep-order publish** that `docs/release.md` does by
   hand (the path-dep `version =` pins and the leaf-first crates.io order).

### The one workspace decision release-plz forces — and the answer

release-plz **defaults to independent** per-crate versions. This repo is **locked** (one
shared version). release-plz supports locked versioning via the **`version_group`** config
key — the name of a group of packages that must share a version; grouped crates all take
the highest computed next version. **The concrete adoption step is therefore: put all
publishable crates in one `version_group` in a `release-plz.toml`.** Without it,
release-plz would silently start versioning the publishable crates independently — a
behaviour change the maintainer should opt into deliberately, not by accident.

> Independent-vs-locked is a genuine fork. Independent versioning gives consumers tighter
> per-crate SemVer signal but multiplies release bookkeeping across all publishable
> crates; locked is simpler and matches today. **Recommendation: stay locked** initially
> via `version_group`; revisit only if downstream consumers complain about coarse bumps.

### Promote conventional commits from discipline to a gate

release-plz's version inference is only as good as the commit signal. History is currently
100% conforming but **unenforced**. Add a lightweight PR-title / commit check (commitlint
or a PR-title action) so the signal cannot silently drift, and start exercising
`feat!:` / `BREAKING CHANGE:` (history has none, so the major-bump path is untested).

---

## 3. Build matrix (OS / arch + artifacts) — feasible-now vs needs-GUI vs needs-user

The repo **already ships a hand-rolled, native-runner build matrix** in two workflows
(`dist.yml`, `release.yml`) that fire on `v*` tags. It is **not** cargo-dist —
there is no `dist-workspace.toml`, no `dist.toml`, and no `[workspace.metadata.dist]` in
`Cargo.toml` (verified). The matrix is hand-rolled on purpose because it carries
project-specific micro-architecture tiering that vanilla cargo-dist does not express:
multiple `sparq-cli` binaries per OS/arch keyed to CPU feature levels
(`x86-64`, `-v2`, `-v3`, `-v4`), each built with a `-Ctarget-cpu` flag, with Apple silicon
shipping a single binary because Darwin baseline already enables every Apple-silicon
feature (rationale and tier table in `dist.yml` and `release.yml`; both reference
`research/hardware/optimization-findings.md`).

### 3a. The existing tiers (verified in `release.yml` / `dist.yml`)

| Tier | runner | target triple | `target-cpu` | State |
|---|---|---|---|---|
| arm64-darwin | `macos-14` | `aarch64-apple-darwin` | (none) | feasible-now |
| x64-darwin | `macos-15-intel` | `x86_64-apple-darwin` | `x86-64-v3` | feasible-now |
| x64-baseline / v2 / v3 / v4 | `ubuntu-latest` | `x86_64-unknown-linux-gnu` | `x86-64{,-v2,-v3,-v4}` | feasible-now |
| arm64-linux | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | `neoverse-n1` (+`lse`) | feasible-now |
| win-x64-baseline / v3 | `windows-latest` | `x86_64-pc-windows-msvc` | `x86-64{,-v3}` | feasible-now |
| win-arm64 | `windows-latest` | `aarch64-pc-windows-msvc` | (none) | feasible-now (the **single** cross-compile — MSVC cross-links from the x64 runner; no `cross`/QEMU anywhere) |

Strategy is native-runner-per-arch (each tier installs via `rustup target add` and builds
on a runner of its own architecture), except `win-arm64` which cross-links from the x64
Windows runner. There is **no** `x86_64-unknown-linux-musl` static target — all Linux
tiers are `-gnu` (glibc-dynamic), an honest limitation (a glibc floor tied to the runner
image), not flagged as a gap today.

### 3b. Artifacts the release produces today (verified in `release.yml`)

- **`sparq-cli` per-tier archives** — `tar.gz` (non-Windows) / `zip` (Windows), each
  containing the binary + `README.md` + `LICENSE` (now present at repo root) + `CHANGELOG.md`.
- **`SHA256SUMS`** over every archive, plus a GitHub Release with auto-notes.
- **`ghcr.io/sparq-org/sparq-server` container** (`{X.Y.Z, X.Y, latest}`), smoke-tested +
  Trivy-scanned before push, with max-mode SLSA provenance + embedded SBOM. **This is the
  only place the `sparq-server` binary ships.**
- **CycloneDX SBOM per binary + JS/npm SBOM + VEX**, SLSA-attested.
- Out-of-band via `publish.yml`: crates.io libraries, npm `@sparq-org/sparq` (WASM), PyPI
  `sparq-rdf` wheels (Section 5).

### 3c. What is NOT produced — gaps and their classification

- **`sparq-server` native binary archive — a real gap.** `sparq-server` is a binary
  target (`crates/sparq-server/Cargo.toml` declares `[[bin]] name = "sparq-server"`,
  `required-features = ["server"]`), but the matrix builds only `-p sparq-cli`.
  `sparq-server` ships **only** as a container today. If a release should serve users who
  do not want Docker, add a second `-p sparq-server` build (or `--bins`). **Feasible-now**;
  no credentials needed.
- **Native installers (`.deb` / `.rpm` / `.msi` / `.dmg` / `.pkg`) — absent.** The macOS
  and Windows artifacts are plain archives, not installers. The only OS-package path
  prepared is a Homebrew formula **template** (`packaging/homebrew/sparq.rb`) consumed by a
  separate tap, with `sha256` placeholders filled from the release's `SHA256SUMS`.
  **Aspirational** (this is exactly what a cargo-dist migration would auto-generate —
  Section 7).
- **Code-signing / notarization — absent and `needs:user`.** There is **no** Apple
  `notarytool`/`codesign` or Windows `signtool`/Authenticode anywhere in the workflows;
  the macOS/Windows archives are unsigned (Gatekeeper quarantine / SmartScreen on
  download). The only "signing" present is **build provenance** (SLSA attestation +
  cargo-auditable), which proves *who built it*, not *the OS will install it*. Real
  code-signing needs maintainer-held credentials: Apple Developer ID cert + notarization
  key, Windows EV/OV Authenticode cert. **`needs:user`.**

### 3d. Mobile — bindings vs the GUI app (cross-ref `sq-ixc3`)

This is the load-bearing honesty point. The matrix targets are desktop/server only —
there are **no** `aarch64-linux-android` / `aarch64-apple-ios` rows, and an exhaustive grep
for `uniffi` / `cargo-ndk` / `cargo-lipo` / `xcframework` / `cargo-apk` / `jni` across all
`crates/*/Cargo.toml`, `Cargo.toml`, and `.github/workflows/*.yml` returns **nothing**.
`.cargo/config.toml` configures only the `wasm32-unknown-unknown` target.

Because sparq is a **library** engine, "building for Android/iOS" splits into two very
different things:

- **A bound library artifact (feasible, small-to-moderate, no credentials to *build*).**
  The engine already compiles to `wasm32-unknown-unknown` and is wrapped as a `cdylib` for
  JS (`crates/sparq-wasm` → npm `@sparq-org/sparq`) and Python (`crates/sparq-py` → PyPI
  `sparq-rdf` via pyo3/maturin). Native Android (`.so` + JNI/Kotlin via cargo-ndk) and iOS
  (`.xcframework` + Swift) bindings are the **same cdylib recipe** pointed at a mobile
  triple — feasible, but **absent today**. Two caveats: (i) iOS forbids dynamically
  generated code, so the opt-in ZK/MPC surfaces that shell out to external provers
  (`nargo`/`bb`, see `zk-toolchain.yml`) cannot run on-device — a mobile binding ships the
  core query/store/reasoning surface only; (ii) the existing WASM build is single-threaded
  by design (`sparq-wasm` builds `sparq-core` with `default-features = false`, no rayon), so
  the mobile-web path runs but without parallel ingest/query.
- **An installable mobile APP (`.aab`/`.ipa` in a store) — aspirational, needs the GUI
  epic.** This is **not** a library concern. It is `sq-ixc3` ("EPIC: cross-platform GUI",
  open, p1), whose framework candidates are **Tauri 2** (reuses the React site + WASM
  engine; produces desktop *and* mobile app wrappers) vs egui/iced. The app artifact is a
  `sq-ixc3` deliverable, not a property of `dist.yml`. Store signing (Android keystore,
  Apple provisioning/certs) is `needs:user`.

So `sq-v286`'s own framing holds and is grounded here: *mobile builds of a Rust LIBRARY
engine = bindings (uniffi / cargo-ndk / xcframework), NOT an app — an actual mobile app
artifact requires the GUI epic.*

---

## 4. The "should produce" artifact ledger

| Artifact | State | Classification |
|---|---|---|
| `sparq-cli` per-tier archives (all 10 tiers) | shipping (`release.yml`) | feasible-now |
| `SHA256SUMS` + GitHub Release + auto-notes | shipping | feasible-now |
| `ghcr.io/sparq-org/sparq-server` container | shipping | feasible-now |
| CycloneDX SBOM (Rust + JS) + VEX, SLSA-attested | shipping | feasible-now |
| crates.io libraries | shipping (`publish.yml`) | feasible-now (needs crates.io login secret) |
| npm `@sparq-org/sparq` (WASM) w/ Sigstore provenance | shipping (`publish.yml`) | feasible-now (needs `NPM_TOKEN`) |
| PyPI `sparq-rdf` wheels w/ PEP-740 attestations | shipping (`publish.yml`) | feasible-now (needs PyPI Trusted-Publisher registration) |
| `sparq-server` native binary archive | **missing** | feasible-now gap |
| `.deb` / `.rpm` / `.msi` / `.dmg` installers | absent | aspirational |
| Signed/notarized macOS + Windows binaries | absent | `needs:user` |
| Android `.so` / iOS `.xcframework` library bindings | absent | aspirational (feasible, no infra yet) |
| Installable mobile app (`.aab` / `.ipa`) | absent | needs GUI epic `sq-ixc3` |

---

## 5. Reuse, do not duplicate — the existing release/publish/supply-chain estate

A release CI for sparq is mostly **wiring**, not new machinery. The following already
exist, are SHA-pinned, and must be reused verbatim.

- **Build matrix** lives in **both** `release.yml` and `dist.yml`, hand-synced (the
  deliberate non-`workflow_call` decision is documented in the `release.yml` header). New
  work should drive toward unifying these via `workflow_call`, **not** add a third copy.
- **SLSA build provenance** via `actions/attest-build-provenance` over release archives,
  per-tier dist binaries, the SBOM/VEX, and the packaged `.crate` bytes; verify with
  `gh attestation verify`.
- **cargo-auditable** embeds the resolved dependency manifest into every shipped binary.
- **CycloneDX SBOM (1.5) + VEX** per-release (`scripts/gen-sbom-vex.sh`,
  `scripts/gen-js-sbom.sh`) and CI-time (`supply-chain.yml`), with purl-canonicality /
  bom-ref / supplier-name gating assertions.
- **Supply-chain gates** — cargo-deny (advisories/bans/sources/licenses, gating),
  cargo-vet (pinned, gating), a VEX↔deny drift gate, a daily advisories watchdog
  (`dependency-monitoring.yml`), OpenSSF Scorecard (`scorecard.yml`), and Rust CodeQL
  (`codeql.yml`).
- **Container provenance** — buildkit `provenance: mode=max` + `sbom: true`, smoke-test
  + Trivy gate before the ghcr push.
- **Per-registry provenance, honest status** (`publish.yml`): npm = native Sigstore
  provenance; PyPI = native PEP-740 attestations (strongest); crates.io = **no upstream
  provenance mechanism exists** — the most done is out-of-band `.crate` attestation, so a
  crates.io publish must **not** be described as "signed" (this sub-gap is tracked OPEN
  upstream).

### The gate any new release-gating job must respect

The single required branch-protection check is **`ci-summary / gate`** (`ci-summary.yml`).
It does not use `needs:` (it cannot span workflows); it polls sibling check-runs on the
head SHA and passes iff none failed. Two load-bearing rules: (1) a check whose name
contains the whole word `advisory` or `informational` is **non-gating** — everything else
gates; (2) a gating job must also trigger on `merge_group` or it hangs the merge queue.
Note `release.yml` / `publish.yml` / `dist.yml` are tag/release-triggered only, so they
never register on a PR ref and are correctly outside the PR gate. The actual ruleset is
configured server-side, not in the repo.

### Drift to reconcile (flag, do not silently fix)

- **17 publishable crates, not 16.** Counting `crates/*/Cargo.toml` without
  `publish = false` yields **17** (sparq-algos, -cli, -core, -engine, -geo, -hdt,
  -introspect, -nlq, -reason, -rsp, -serve, -server, -shacl, -sim, -solid, -text,
  -vectors). But `docs/release.md:110` says *"16 crates publish"* and its publish-order DAG
  omits **`sparq-algos`** — which has full crates.io metadata and **no** `publish = false`
  (`crates/sparq-algos/Cargo.toml`). `publish.yml`'s `crates` job globs `crates/*/Cargo.toml`
  and skips only `publish = false`, so it **would** package + attest sparq-algos while the
  runbook never publishes it. Reconcile the count to **17** and add sparq-algos (a
  core-only leaf) to the DAG.
- **dist/release double-build.** Both fire on `v*` and build the binaries twice. Drop
  `dist.yml`'s `tags:` trigger (keep `workflow_dispatch`) as part of this work.

### Out-of-repo prerequisites (cannot be wired from the repo — `needs:user`)

PyPI Trusted-Publisher registration (one-time PyPI-account step), `NPM_TOKEN` secret,
crates.io `cargo login`, and — for any signed multi-platform release — the code-signing
credentials in Section 3c. The estate is **wired but unexercised end-to-end**: version is
`0.1.0`, no `v*` tag has ever been pushed.

---

## 6. Sequencing — design now, build after engine features land

Per the maintainer's instruction on `sq-v286`, implementation is queued *after* current
engine features land. This record is the design deliverable; the ordered implementation
beads below are children of `sq-v286` and link this record. Ordering reflects real
dependencies: the cleanups and the `version_group` decision precede wiring release-plz;
the credential/store-signing items are `needs:user` and sit at the end of their chains.

1. **Reconcile the publishable-crate drift** (17 vs 16; add sparq-algos to the DAG) —
   prerequisite so any automated publish set matches reality.
2. **Unify the duplicated build matrix** via `workflow_call`; drop `dist.yml`'s `tags:`
   trigger to end the double-build.
3. **Add a `version_group` `release-plz.toml`** covering the publishable crates to
   preserve the locked single-version model.
4. **Add the release-plz `release-pr` + `release` workflow** (opens a Release PR on
   `main`; on merge tags `vX.Y.Z`, which fires the existing `release.yml` unchanged, and
   publishes to crates.io in dep order). Reconcile the two crates.io paths (let release-plz
   publish and have `publish.yml` only attest, or keep manual publish — the former is the
   point of adoption).
5. **Add a Conventional-Commits PR-title gate** (commitlint / PR-title action) so the
   SemVer signal cannot drift; begin using `feat!:` / `BREAKING CHANGE:`.
6. **Add a `sparq-server` native binary archive** to the matrix (`-p sparq-server`) so it
   ships outside Docker.
7. *(needs:user)* Complete out-of-repo publish prerequisites — PyPI Trusted Publisher,
   `NPM_TOKEN`, crates.io login.
8. *(needs:user)* Desktop code-signing + notarization (Apple Developer ID + notarytool;
   Windows Authenticode) for the macOS/Windows artifacts.

---

## 7. Explicitly out of scope of this record

- **A cargo-dist migration.** cargo-dist would auto-generate installers (`.msi`,
  shell/PowerShell installer scripts, Homebrew formula) the repo hand-maintains, at the
  cost of the measured micro-arch tiering (which is project-specific value, not a gap). It
  is a judgement call for a later record, not part of the release-plz adoption.
- **The GUI desktop/mobile app** (`sq-ixc3`) and its store-signing automation.
- **On-device ZK/MPC** — excluded from any mobile binding (external provers; iOS
  no-codegen rule).
- **No performance numbers** are asserted anywhere in this record.

---

## Files cited (all real, in this checkout)

- `Cargo.toml` (`:18` workspace version; 36-member workspace; no `[workspace.metadata.dist]`)
- `crates/*/Cargo.toml` (all 36 use `version.workspace = true`; 17 publishable;
  `crates/sparq-algos/Cargo.toml` has no `publish = false`;
  `crates/sparq-server/Cargo.toml` `[[bin]] name = "sparq-server"`,
  `required-features = ["server"]`)
- `crates/sparq-wasm/` + `crates/sparq-py/` (the existing `cdylib` binding pattern)
- `.cargo/config.toml` (only `wasm32-unknown-unknown` configured)
- `.github/workflows/release.yml`, `dist.yml`, `publish.yml` (tag/release-driven
  build/package/publish/provenance; duplicated matrix; per-registry provenance)
- `.github/workflows/supply-chain.yml`, `dependency-monitoring.yml`, `scorecard.yml`,
  `codeql.yml`, `container-scan.yml`, `ci-summary.yml`, `zk-toolchain.yml`
- `docs/release.md` (`:70` "release-plz … not adopted yet"; `:110` "16 crates publish" +
  the publish-order DAG that omits sparq-algos)
- `CHANGELOG.md` (`:4-6` Keep-a-Changelog + SemVer; empty `[Unreleased]` at `:8`)
- `packaging/homebrew/sparq.rb` (formula template; `sha256` placeholders)
- `git tag` (only `pr670base`; no SemVer tag); `git log` (conventional commits; no
  `BREAKING CHANGE`/`!`)
- `.beads/issues.jsonl` — `sq-v286` (this record's parent epic) and `sq-ixc3` (GUI/mobile-app epic)

## Sources (current tooling)

- <https://github.com/release-plz/release-plz> and <https://release-plz.dev/docs/config>
  (`version_group`, `semver_check`)
- <https://blog.orhun.dev/automated-rust-releases/>
- <https://git-cliff.org/>
- <https://crates.io/crates/cargo-release>
- <https://github.com/GitoxideLabs/cargo-smart-release>
- <https://crates.io/crates/semantic-release-cargo> (the npm-tool Rust bolt-on, illustrating the Node-toolchain coupling)
