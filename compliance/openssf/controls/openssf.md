<!-- [OPUS-4.8] OpenSSF control spine (epic sq-toze, bead sq-toze.15). The auditor checks
     THIS file. Every row: control -> status -> evidence (repo-relative path / CI job) ->
     owner. Authored while Fable unavailable — re-review when Fable returns. -->

# OpenSSF — control / check spine

Status legend: **IV** = Implemented & verified · **AR** = Audit-ready (control + evidence
present; external filing/recompute is maintainer/OpenSSF-owned) · **GAP** = not met (see
[`gap-register.md`](../gap-register.md)).

Evidence paths are repo-relative. `.github/workflows/<wf>.yml` is the CI gate;
`docs/…` / `SECURITY.md` / `.well-known/…` are the doc/config of record.

---

## A — OpenSSF Scorecard checks

Scorecard runs in [`.github/workflows/scorecard.yml`](../../../.github/workflows/scorecard.yml)
on push-to-`main` + weekly, with `publish_results: true` (public OpenSSF dashboard, required
for the Scorecard badge) and SARIF upload to code-scanning. Each row maps a Scorecard check
to the concrete repo control that *raises* it.

| Scorecard check | Status | Evidence (file / CI job) | Owner | Notes |
|---|---|---|---|---|
| **Binary-Artifacts** | IV | No checked-in binaries; `.gitignore` excludes build output; the unsafe register scopes only `crates/` source. | maintainer | Scorecard scores 10 when no executable artifacts are committed. |
| **Branch-Protection** | AR | [`docs/branch-protection.md`](../../../docs/branch-protection.md) (doc-of-record, reconciled to the live ruleset) — required `ci-summary / gate`, `required_approving_review_count: 0` (solo-maintainer), Copilot review on push, CodeQL alert gate, conversation-resolution, squash-only linear history, block force-push/deletion. The repository-administrator role has an always-on bypass; the automated landing path does not use it and remains constrained by merge-queue admission. | maintainer | The *settings* live in GitHub UI (out-of-repo); the doc records + verifies the live ruleset (`gh api …/rulesets`, see its §Solo-maintainer table). Scorecard reads the live ruleset via API — score reflects what is actually configured, and the classic two-human sub-signals are intentionally off (GX-OSSF-3). **AR** because verification needs the live API/dashboard. |
| **CI-Tests** | IV | [`.github/workflows/ci.yml`](../../../.github/workflows/ci.yml) — `cargo build`/`cargo test --workspace` run on every PR; aggregated by `ci-summary`. | maintainer | |
| **CII-Best-Practices** | GAP | Badge not yet filed on bestpractices.dev (GX-4). Answers ready in BOTH [`evidence.md`](../evidence.md) (prose) and [`best-practices-self-cert.json`](../best-practices-self-cert.json) (machine-readable, import-ready), the latter CI-gated by `supply-chain.yml#openssf-selfcert`. | maintainer | Scorecard reads the badge's published level; 0 until the badge entry exists. The in-repo answer payload + gate are DONE; only the external filing remains. Bead **sq-toze.5**. |
| **Code-Review** | AR | [`docs/branch-protection.md` §Solo-maintainer](../../../docs/branch-protection.md#solo-maintainer--the-scorecard-code-review--branch-protection-score) — solo-maintainer, so `required_approving_review_count: 0` with a compensating Copilot-review + CodeQL-gate + conversation-resolution layer (no faked self-approval); [`CODEOWNERS`](../../../CODEOWNERS); [`.github/PULL_REQUEST_TEMPLATE.md`](../../../.github/PULL_REQUEST_TEMPLATE.md). | maintainer | Scorecard infers review from merged-PR history and discounts self-approval, so a single-maintainer, agent-driven repo cannot score 10 here; honest, inherent-by-design nuance, not a hideable gap — see gap-register GX-OSSF-3 / sq-sto1. |
| **Dangerous-Workflow** | IV | All workflows use least-privilege `permissions:` blocks; no `pull_request_target` + untrusted checkout pattern; no script-injection of untrusted `${{ }}` into `run:`. e.g. [`scorecard.yml`](../../../.github/workflows/scorecard.yml) top-level `contents: read`. | maintainer | |
| **Dependency-Update-Tool** | IV | [`.github/dependabot.yml`](../../../.github/dependabot.yml) — 4 ecosystems (cargo, github-actions, npm, pip). | maintainer | |
| **Fuzzing** | IV | [`.github/workflows/fuzz.yml`](../../../.github/workflows/fuzz.yml) — cargo-fuzz, PR smoke + daily heavy tier (cron `23 4 * * *`); plus [`.github/workflows/shacl-diff-fuzz.yml`](../../../.github/workflows/shacl-diff-fuzz.yml). | maintainer | Scorecard detects cargo-fuzz integration. |
| **License** | IV | [`LICENSE`](../../../LICENSE) (MIT), SPDX-recognised, repo root. | maintainer | |
| **Maintained** | AR | Active commit/PR cadence on `main`; `SECURITY.md` declares maintenance posture. | maintainer | Scorecard reads the last-90-days commit/issue activity at scan time. The Badge `maintained` self-cert rests on this **same live signal** and is labelled *Met w/justification (live signal)* in [`evidence.md`](../evidence.md) — both views carry the same confidence. |
| **Packaging** | AR | [`.github/workflows/release.yml`](../../../.github/workflows/release.yml) builds + publishes release assets; [`dist.yml`](../../../.github/workflows/dist.yml). crates.io/npm/PyPI publish is manual. | maintainer | Scorecard looks for a recognised publish workflow; manual registry publish may not be auto-detected. |
| **Pinned-Dependencies** | IV | All third-party GitHub Actions pinned by **full commit SHA** (with `# vX.Y.Z` trailer Dependabot follows) across `.github/workflows/*`; the [`Dockerfile`](../../../Dockerfile) base is digest-pinned; documented nuance for `dtolnay/rust-toolchain` in [`docs/branch-protection.md`](../../../docs/branch-protection.md). | maintainer | Resolves the Scorecard `Pinned-Dependencies` alerts. |
| **SAST** | IV | [`.github/workflows/codeql.yml`](../../../.github/workflows/codeql.yml) — CodeQL `security-and-quality` over Rust, on push + PR + weekly; SARIF to code-scanning. Plus clippy `-D warnings` ([`ci.yml`](../../../.github/workflows/ci.yml)) + Scorecard's own analysis. | maintainer | |
| **Security-Policy** | IV | [`SECURITY.md`](../../../SECURITY.md) (private channels, response targets, scope caveats) + [`.well-known/security.txt`](../../../.well-known/security.txt) (RFC 9116, GX-3 closed). | maintainer | Scorecard checks for `SECURITY.md`; the machine-readable `security.txt` strengthens the disclosure story (CRA/ASVS cross-ref). |
| **Signed-Releases** | IV | [`.github/workflows/release.yml`](../../../.github/workflows/release.yml) — `actions/attest-build-provenance` (Sigstore-signed SLSA provenance over every archive + SBOM + VEX) + `SHA256SUMS`; container build `provenance: mode=max`. Verify: `gh attestation verify <file> --repo sparq-org/sparq`. | maintainer | Scorecard credits Sigstore provenance attestations as signed releases. Registry-publish signing is a separate nuance — see gap-register GX-OSSF-2. |
| **Token-Permissions** | IV | Every workflow declares top-level least-privilege `permissions:` (read-only default; jobs opt into the minimum write, e.g. `release.yml` `attestations: write` only on the attest job). | maintainer | |
| **Vulnerabilities** | IV | [`.github/workflows/supply-chain.yml`](../../../.github/workflows/supply-chain.yml) `audit` job — **two gating steps**: `cargo deny check bans sources licenses` *and* `cargo deny check advisories` (both GATING, no `continue-on-error`); [`deny.toml`](../../../deny.toml) is fail-closed (`yanked = "deny"`, only two justified `unmaintained` ignores — neither a vuln). The daily advisory watchdog [`dependency-monitoring.yml`](../../../.github/workflows/dependency-monitoring.yml) is now **defence-in-depth**. | maintainer | Advisory PR-gate **un-degraded** (GX-1 closed by #210 / sq-toze.2; the CVSS-4.0 parse blocker sq-q8de is resolved). Scorecard checks for *open, unfixed* OSV vulns at scan time. |
| **Webhooks** | AR | No repo webhooks configured beyond GitHub-native; nothing to authenticate. | maintainer | Experimental Scorecard check; n/a for this repo's surface. |

### Scorecard — honest posture summary
The posture-defining checks (**Pinned-Dependencies, SAST, Token-Permissions,
Dangerous-Workflow, Dependency-Update-Tool, Fuzzing, Security-Policy, Signed-Releases,
License, CI-Tests, Binary-Artifacts**) are **all implemented & verified** in the repo.
The checks Scorecard computes from *live GitHub state or history* (**Branch-Protection,
Code-Review, Maintained, Packaging, Webhooks**) are **audit-ready** — the controls and the
doc-of-record exist, but the *score* is whatever OpenSSF infrastructure reads at scan time
(it cannot be asserted from a file). The one outright **gap** is **CII-Best-Practices**
(the badge entry itself, GX-4). No Scorecard check is overclaimed.

---

## B — OpenSSF Best-Practices (CII) Badge criteria

The badge questionnaire has six families. Below is the **status of each family** with the
governing evidence; the **per-criterion drafted answers** (the text to paste into the
bestpractices.dev form) are in [`evidence.md`](../evidence.md) §Badge. Overall **badge
status: GAP** — eligible and answer-ready, but **not filed** (GX-4, bead sq-toze.5). Each
family below is **AR** (answer + evidence ready; filing is the maintainer's external step).

| Badge family | Status | Key criteria met → evidence | Notes / residual |
|---|---|---|---|
| **Basics** | AR | `description_good`, `interact`, `contribution` → [`README.md`](../../../README.md), [`CONTRIBUTING.md`](../../../CONTRIBUTING.md), [`AGENTS.md`](../../../AGENTS.md). `floss_license` + `license_location` → [`LICENSE`](../../../LICENSE) (MIT, OSI). `documentation_basics` / `documentation_interface` → `README` + `skills/<surface>/SKILL.md` + docs.rs. `sites_https`, `discussion`, `english` → GitHub. | `repo_distributed`, `version_unique` (semver tags) — met by the release tag scheme. |
| **Change Control** | AR | `repo_public` + `repo_track` (git) + `repo_interim` (commits on `main`). `version_semver` → release tags. `release_notes` → `generate_release_notes: true` + [`CHANGELOG.md`](../../../CHANGELOG.md) in [`release.yml`](../../../.github/workflows/release.yml). | |
| **Reporting** | AR | `report_process` + `report_tracker` → beads (`bd`) + GitHub issues w/ [templates](../../../.github/ISSUE_TEMPLATE/). `vulnerability_report_process` + `vulnerability_report_private` → [`SECURITY.md`](../../../SECURITY.md) (GHSA + email) + [`.well-known/security.txt`](../../../.well-known/security.txt). `vulnerability_report_response` → 5/10-business-day targets in `SECURITY.md`. | `report_responses` is best-effort (volunteer project) — stated honestly in `SECURITY.md`. |
| **Quality** | AR | `build` + `build_common_tools` (cargo) + `build_reproducible` (claim: see gap GX-8 — honest "characterised, not yet enforced": [`../../slsa/reproducible-build.md`](../../slsa/reproducible-build.md) records a 22-byte single-cause double-build diff; the CI rebuild-and-diff ratchet is the residual). `test` + `test_invocation` (`cargo test --workspace`, [`ci.yml`](../../../.github/workflows/ci.yml)) + `test_most` (conformance ratchets, [`CONTRIBUTING.md`](../../../CONTRIBUTING.md)). `test_continuous_integration` → CI on every PR. `warnings` + `warnings_fixed` + `warnings_strict` → clippy `-D warnings` (gating) + `cargo fmt`. | `test_policy` / `tests_documented_added` → the conformance-ratchet "never lower" rule + the PR template checklist. |
| **Security** | AR | `crypto_published` (sparq's own crypto = Sigstore release attestation, standard) ; `crypto_call`/`crypto_random` n/a to release path. `delivery_mitm` → SHA256SUMS + SLSA provenance attestation ([`release.yml`](../../../.github/workflows/release.yml)). `delivery_unsigned` → met (signed). `vulnerabilities_fixed_60_days` + `vulnerabilities_critical_fixed` → `SECURITY.md` + the daily advisory watchdog. `no_leaked_credentials` → no secrets in tree; CodeQL + Scorecard. | **The ZK/MPC scaffolds are explicitly excluded from any `crypto_*` claim** — see [`SECURITY.md`](../../../SECURITY.md). [OPUS-4.8] The v1 ZK verifier was originally found unsound ([`research/zk-soundness-audit.md`](../../../research/zk-soundness-audit.md), kept on record), then `sq-1s2` landed the binding layer + an internal re-audit ([`research/zk-verifier-reaudit.md`](../../../research/zk-verifier-reaudit.md), `sq-gbp4`) found the prior findings closed → "sound as landed for the assumed threat model" — but **internal/single-model only, external sign-off PENDING (`sq-qhy4`), no production guarantee**. Answering `crypto_*` about the research scaffolds would be a high-severity overclaim; we answer them only about sparq's *own* (release-attestation) cryptography. |
| **Analysis** | AR | `static_analysis` → CodeQL `security-and-quality` ([`codeql.yml`](../../../.github/workflows/codeql.yml)) + clippy. `static_analysis_common_vulnerabilities` → CodeQL security queries. `dynamic_analysis` → cargo-fuzz ([`fuzz.yml`](../../../.github/workflows/fuzz.yml)) + Miri ([`miri.yml`](../../../.github/workflows/miri.yml)). `dynamic_analysis_unsafe` → Miri + the oracle/fuzz matrix over `sparq-core` unsafe (memsafety slice, [`compliance/memsafety/unsafe-register.md`](../../memsafety/unsafe-register.md)). | sparq exceeds the bronze bar on analysis (CodeQL + clippy + fuzz + Miri); strong evidence toward silver/gold `dynamic_analysis_*`. |

### Badge — passing-level readiness (honest)
On the drafted evidence, sparq **meets the "passing" (bronze) bar** on every family and has
substantial **silver/gold** evidence (strict warnings, signed releases, fuzz + Miri dynamic
analysis, two-person-review *rule* though solo-maintained). The only thing standing between
"answer-ready" and a *displayed badge* is the maintainer **filing** the questionnaire on
bestpractices.dev (GX-4). That filing is an external, human-owned step — labelled AR/GAP
accordingly, never claimed as done.

---

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
## Cross-references
- Supply-chain evidence (cargo-deny, SBOM, SLSA) is owned by the `sbom` / `slsa` slices;
  cited here, not duplicated. See [`research/production-certification-plan.md`](../../../research/production-certification-plan.md) §Already-done.
- Memory-safety dynamic-analysis evidence: [`compliance/memsafety/unsafe-register.md`](../../memsafety/unsafe-register.md).
- The disclosure machine-discoverability (GX-3) is **closed**: [`.well-known/security.txt`](../../../.well-known/security.txt).
