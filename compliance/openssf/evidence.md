<!-- [OPUS-4.8] OpenSSF evidence pack (epic sq-toze, bead sq-toze.15 / GX-4 sq-toze.5).
     Drafted Best-Practices badge answers + Scorecard per-check narrative, each grounded in
     a cited artifact. Authored while Fable unavailable — re-review when Fable returns. -->

# OpenSSF — evidence pack

Two parts: **§Badge** is the drafted bestpractices.dev self-certification (the text to
paste into the form when the maintainer files it, GX-4); **§Scorecard** is the per-check
evidence narrative. Every claim cites a repo-relative artifact. Nothing here is asserted
without a checkable source; where a criterion is *Met (justification)* or *Unmet* it says so.

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
> Honesty anchor: sparq's ZK/MPC crypto is a **research scaffold with NO production security
> guarantee**. [OPUS-4.8] The v1 ZK verifier was **originally found unsound**
> ([`research/zk-soundness-audit.md`](../../research/zk-soundness-audit.md), kept on record),
> then `sq-1s2` landed the verifier-side binding layer and an **internal post-remediation
> re-audit** ([`research/zk-verifier-reaudit.md`](../../research/zk-verifier-reaudit.md),
> `sq-gbp4`) found the prior findings closed → **"sound as landed for the assumed threat
> model"** — but that is **internal / single-model self-review only, with external
> accredited-cryptographer sign-off still PENDING (`sq-qhy4`, P0) and NO production guarantee**;
> `sparq-mpc` is semi-honest-only with no guarantee (see [`SECURITY.md`](../../SECURITY.md)).
> The badge `crypto_*` answers below concern **only** sparq's own cryptographic use (the
> Sigstore release-attestation / TLS-at-the-operator-boundary path). They make **no** claim
> about the research scaffolds. Any answer that did would be a high-severity overclaim.

---

## §Badge — drafted OpenSSF Best-Practices (passing/bronze) answers

Format per criterion: **`criterion_id`** — *Answer* (Met / Met w/justification / Unmet / N/A)
— evidence. Criterion ids follow bestpractices.dev. This drafts the **passing** level; notes
flag where sparq also satisfies **silver/gold**.

> **Machine-readable twin.** The same answers exist in import-ready structured form at
> [`best-practices-self-cert.json`](./best-practices-self-cert.json) (one object per criterion:
> `status` + `justification` + repo-relative `evidence` path(s)). That file is what a maintainer
> transcribes into the bestpractices.dev form, and it is **CI-gated** by
> [`scripts/check-bestpractices-evidence.py`](../../scripts/check-bestpractices-evidence.py)
> (`supply-chain.yml#openssf-selfcert`): every cited evidence path must resolve to a real
> artifact and every status/family token must be legal, so this prose and the JSON cannot drift
> out of step with the repo. The JSON keeps `project.filed = false` until the badge is actually
> filed (GX-4, the external human-owned step).

### Basics
- **`description_good`** — Met — [`README.md`](../../README.md) one-line pitch + what/why.
- **`interact`** — Met — GitHub issues/PRs/Discussions; [`CONTRIBUTING.md`](../../CONTRIBUTING.md).
- **`contribution`** — Met — [`CONTRIBUTING.md`](../../CONTRIBUTING.md) + [`AGENTS.md`](../../AGENTS.md) (the build/test/lint gate + merge discipline).
- **`contribution_requirements`** — Met — `CONTRIBUTING.md` "The gate"; PR template checklist.
- **`floss_license`** — Met — MIT ([`LICENSE`](../../LICENSE)), an OSI-approved FLOSS license.
- **`license_location`** — Met — `LICENSE` at repo root; SPDX `MIT`.
- **`documentation_basics`** — Met — `README.md` + per-crate READMEs + `skills/<surface>/SKILL.md`.
- **`documentation_interface`** — Met — docs.rs rustdoc (`#![doc = include_str!("../README.md")]`); HTTP API in `skills/server/SKILL.md`.
- **`sites_https`** — Met — GitHub-hosted (HTTPS).
- **`discussion`** — Met — GitHub Discussions/Issues.
- **`english`** — Met — all docs in English; `Preferred-Languages: en` in `security.txt`.
- **`maintained`** — Met w/justification (live signal) — based on active commit/PR cadence on `main` + `SECURITY.md`'s declared posture. This is the **same live signal** Scorecard's `Maintained` check reads at scan time, where it is labelled **AR** in [`controls/openssf.md`](./controls/openssf.md) §A; the badge self-cert and the Scorecard view therefore carry the *same* confidence (the answer is true only while the cadence holds, not assertable purely from a file). *(Silver: declare a maintenance/EOL policy — `SECURITY.md` "Supported versions" partially covers; note pre-1.0.)*

### Change Control
- **`repo_public`** — Met — public GitHub repo `sparq-org/sparq`.
- **`repo_track`** — Met — git history.
- **`repo_interim`** — Met — interim commits land on `main` between releases (no long-lived release branches).
- **`repo_distributed`** — Met — git (distributed VCS).
- **`version_unique`** — Met — unique release tags.
- **`version_semver`** — Met (pre-1.0) — semver tags; `AGENTS.md` notes the API is unstable pre-1.0 (semver-compatible `0.x`). *(Honest nuance, not a fail.)*
- **`version_tags`** — Met — git tags per release.
- **`release_notes`** — Met — [`release.yml`](../../.github/workflows/release.yml) `generate_release_notes: true` + curated [`CHANGELOG.md`](../../CHANGELOG.md).
- **`release_notes_vulns`** — Met w/justification — security fixes flow through GHSA advisories (private→published) per `SECURITY.md`; the changelog points to the affected published artifact. *(Silver: cross-link CVE/GHSA ids in release notes when one is issued.)*

### Reporting
- **`report_process`** — Met — bug/feature [issue templates](../../.github/ISSUE_TEMPLATE/); work tracked in beads (`bd`).
- **`report_tracker`** — Met — GitHub Issues + the beads dependency tracker.
- **`report_responses`** — Met w/justification — best-effort, volunteer project; targets stated in `SECURITY.md` (not contractual SLAs). Honest.
- **`enhancement_responses`** — Met w/justification — best-effort, same basis.
- **`report_archive`** — Met — GitHub retains issue/PR/advisory history publicly (advisories until coordinated disclosure).
- **`vulnerability_report_process`** — Met — [`SECURITY.md`](../../SECURITY.md) "Reporting a vulnerability" (two private channels).
- **`vulnerability_report_private`** — Met — GitHub Security Advisories (`/security/advisories/new`) + `mailto:jesse@jeswr.org`; machine-readable in [`.well-known/security.txt`](../../.well-known/security.txt).
- **`vulnerability_report_response`** — Met — 5-business-day acknowledge / 10-business-day initial assessment targets in `SECURITY.md`.

### Quality
- **`build`** — Met — `cargo build --workspace`.
- **`build_common_tools`** — Met — Cargo (standard Rust toolchain).
- **`build_floss_tools`** — Met — Rust/Cargo are FLOSS.
- **`build_reproducible`** — **Unmet but characterised (honest)** — bit-for-bit reproducibility is not yet *demonstrated/enforced*, so the criterion stays Unmet; but it is no longer a bare "no evidence". A documented double-build ([`../slsa/reproducible-build.md`](../slsa/reproducible-build.md)) shows `sparq-cli` is **byte-identical apart from 22 bytes**, all from **one** non-determinism source (the C-compiled `mimalloc` `__DATE__`/`__TIME__` `.rodata` banner + the build-id it perturbs); the release also uses pinned actions + a hosted builder + SLSA provenance. The residual to flip this to Met is `SOURCE_DATE_EPOCH`/feature-drop + a CI rebuild-and-diff ratchet (GX-8/sq-toze.9, cross-framework with slsa/cra/sbom/ssdf). Recorded honestly, not claimed Met.
- **`test`** — Met — `cargo test --workspace` + the W3C SPARQL/SHACL/inference conformance ratchets.
- **`test_invocation`** — Met — documented in [`CONTRIBUTING.md`](../../CONTRIBUTING.md) "The gate" + [`AGENTS.md`](../../AGENTS.md).
- **`test_most`** — Met — workspace tests + conformance ratchets + coverage floors (the "never lower" rule, `CONTRIBUTING.md`). *(Silver/gold `test_statement_coverage_*`: coverage floors are ratcheted; cite the coverage cron in `ci.yml`.)*
- **`test_continuous_integration`** — Met — [`ci.yml`](../../.github/workflows/ci.yml) runs on every PR; `ci-summary / gate` is the required check.
- **`test_policy`** — Met — the conformance/coverage **ratchet "never lower"** rule (`CONTRIBUTING.md`) is the documented add-tests-for-changes policy.
- **`tests_documented_added`** — Met — PR template checklist ties changes to the post-batch re-evaluation table; ratchets enforce non-regression.
- **`warnings`** — Met — clippy lints enabled.
- **`warnings_fixed`** — Met — clippy `-D warnings` is a **hard gate** (`ci.yml`), so no warnings can land.
- **`warnings_strict`** — Met — `cargo clippy --workspace --all-targets -- -D warnings` (full-workspace, all-targets) is the **hard gate** ([`ci.yml`](../../.github/workflows/ci.yml) `clippy (gate) + fmt (non-blocking)` job). *(This is the gold-level strict-warnings posture; the criterion rests on the clippy hard-gate. `cargo fmt --all --check` runs **informationally**, not gating — pending the deferred one-time `cargo fmt --all` reformat, per `ci.yml` header — so it is not cited as enforcing the criterion.)*

### Security
- **`crypto_published`** — Met w/scope — sparq's *delivery* crypto is Sigstore/SLSA build-provenance (a published, standard scheme) over release assets ([`release.yml`](../../.github/workflows/release.yml)). **No claim is made about the `sparq-zk*`/`sparq-mpc` research scaffolds** (remediated but **externally unaudited — internal re-audit only, external sign-off PENDING `sq-qhy4`, no production guarantee**; `SECURITY.md`). [OPUS-4.8]
- **`crypto_call`** — Met w/justification — the release path calls Sigstore via the maintained `actions/attest-build-provenance`; it does not roll its own crypto. The ZK/MPC scaffolds are out of scope.
- **`crypto_floss`** — Met — Sigstore + Rust crypto deps are FLOSS.
- **`crypto_keylength`** / **`crypto_working`** / **`crypto_weaknesses`** — Met w/scope — apply to the Sigstore/TLS delivery path (modern defaults); explicitly **not** asserted about the scaffolds.
- **`crypto_pfs`** / **`crypto_password_storage`** / **`crypto_random`** — N/A — sparq stores no passwords and runs no auth/session crypto (the no-auth boundary B3 is the operator's gateway; see `research/threat-model.md`).
- **`delivery_mitm`** — Met — `SHA256SUMS` + Sigstore-signed SLSA **build-provenance attestation** over every release asset; verify `gh attestation verify <file> --repo sparq-org/sparq` ([`release.yml`](../../.github/workflows/release.yml)).
- **`delivery_unsigned`** — Met — releases are signed (provenance attestation, above). *(Gold-relevant.)*
- **`vulnerabilities_fixed_60_days`** — Met — `SECURITY.md` response targets + the daily advisory watchdog ([`dependency-monitoring.yml`](../../.github/workflows/dependency-monitoring.yml)) surface advisories promptly; fixes ship in the next release.
- **`vulnerabilities_critical_fixed`** — Met — no known unfixed critical vulns; the cargo-deny gate runs **two GATING steps** (`bans/sources/licenses` *and* `advisories`) on PR/push/merge_group with a fail-closed [`deny.toml`](../../deny.toml) (`yanked = "deny"`, two justified `unmaintained` ignores — neither a vuln). The daily watchdog ([`dependency-monitoring.yml`](../../.github/workflows/dependency-monitoring.yml)) is defence-in-depth. *(PR-time advisory gating is **un-degraded** — GX-1 closed by #210 / sq-toze.2; the CVSS-4.0 parse blocker sq-q8de is resolved.)*
- **`no_leaked_credentials`** — Met — no secrets in tree; CodeQL + Scorecard scan; workflows use `${{ secrets }}`/OIDC only.

### Analysis
- **`static_analysis`** — Met — CodeQL `security-and-quality` ([`codeql.yml`](../../.github/workflows/codeql.yml)) + clippy `-D warnings`.
- **`static_analysis_common_vulnerabilities`** — Met — CodeQL security query suite.
- **`static_analysis_fixed`** — Met — clippy is a hard gate (must be clean to land); CodeQL alerts treated as blocking (`docs/branch-protection.md`).
- **`static_analysis_often`** — Met — CodeQL + clippy on **every** PR + weekly cron.
- **`dynamic_analysis`** — Met — cargo-fuzz ([`fuzz.yml`](../../.github/workflows/fuzz.yml)) + Miri ([`miri.yml`](../../.github/workflows/miri.yml)).
- **`dynamic_analysis_unsafe`** — Met — Miri (UB/aliasing/provenance) over `sparq-core` pure-Rust unsafe + the mmap-corruption oracle + fuzz matrix over the B5 mmap sites; attested in [`compliance/memsafety/unsafe-register.md`](../memsafety/unsafe-register.md). *(Strong silver/gold evidence.)*
- **`dynamic_analysis_enable_assertions`** — Met w/justification — debug/test builds run with Rust's default `debug_assertions`; fuzz targets build with sanitiser-style instrumentation via cargo-fuzz.

### Badge level summary (drafted, honest)
- **Passing (bronze):** all criteria Met, except `build_reproducible` (**Unmet**, GX-8). Per
  badge rules a single justified `Met w/justification`/`N/A` does not block passing, but
  `build_reproducible` is **SUGGESTED** (not required) at passing — so its honest *Unmet*
  does **not** block the bronze badge. Confirm against the live form when filing.
- **Silver/Gold reach:** strict warnings (gold-grade), signed releases, two-person-review
  *rule* (solo-maintained in practice — the gold `two_person_review` honest nuance), fuzz +
  Miri dynamic analysis. sparq has unusually strong analysis evidence for its size.

---

## §Scorecard — per-check evidence narrative

(One line of *where the evidence lives* per check; the status labels are in
[`controls/openssf.md`](./controls/openssf.md) §A.)

- **Pinned-Dependencies** — every action SHA-pinned across `.github/workflows/*` + Docker base digest-pinned ([`docs/branch-protection.md`](../../docs/branch-protection.md) "Pinned-Dependencies" note).
- **Token-Permissions** — top-level `permissions:` least-privilege in every workflow (read-only default; per-job minimal writes).
- **SAST** — [`codeql.yml`](../../.github/workflows/codeql.yml) (`security-and-quality`) + clippy gate + Scorecard's own analysis.
- **Dangerous-Workflow** — no `pull_request_target`+untrusted-checkout; no untrusted `${{ }}` injected into `run:`.
- **Dependency-Update-Tool** — [`.github/dependabot.yml`](../../.github/dependabot.yml), 4 ecosystems.
- **Fuzzing** — [`fuzz.yml`](../../.github/workflows/fuzz.yml) (cargo-fuzz, PR + daily) + [`shacl-diff-fuzz.yml`](../../.github/workflows/shacl-diff-fuzz.yml).
- **Security-Policy** — [`SECURITY.md`](../../SECURITY.md) + [`.well-known/security.txt`](../../.well-known/security.txt).
- **Signed-Releases** — [`release.yml`](../../.github/workflows/release.yml): `attest-build-provenance` (Sigstore SLSA) + `SHA256SUMS` + container `provenance: mode=max`.
- **Branch-Protection** — [`docs/branch-protection.md`](../../docs/branch-protection.md) (doc-of-record; live ruleset out-of-repo). The solo-maintainer score-depression, the compensating controls, and a `gh api …/rulesets` verification procedure (with a rule-by-rule match table) are documented in its [§Solo-maintainer & the Scorecard score](../../docs/branch-protection.md#solo-maintainer--the-scorecard-code-review--branch-protection-score) (GX-OSSF-3 / sq-sto1).
- **Code-Review** — solo-maintainer, agent-driven: there is no second human, so the live ruleset sets `required_approving_review_count: 0` (documented honestly, **not** faked with a Scorecard-discounted self-approval); the **compensating** automated review layer is Copilot code review on push + the CodeQL code-scanning gate + conversation-resolution. [`CODEOWNERS`](../../CODEOWNERS) records ownership for when a second reviewer is added. See [`docs/branch-protection.md` §Solo-maintainer](../../docs/branch-protection.md#solo-maintainer--the-scorecard-code-review--branch-protection-score).
- **CI-Tests** — [`ci.yml`](../../.github/workflows/ci.yml) on every PR, aggregated by `ci-summary`.
- **License** — [`LICENSE`](../../LICENSE) (MIT).
- **Binary-Artifacts** — none committed.
- **Vulnerabilities** — [`supply-chain.yml`](../../.github/workflows/supply-chain.yml) `audit` job — `cargo deny check bans sources licenses` *and* `cargo deny check advisories`, **both gating** (no `continue-on-error`; fail-closed [`deny.toml`](../../deny.toml)) + [`dependency-monitoring.yml`](../../.github/workflows/dependency-monitoring.yml) watchdog as defence-in-depth (advisory PR-gate un-degraded — GX-1 closed by #210 / sq-toze.2).
- **CII-Best-Practices** — **gap** until the badge is filed (GX-4).

### Verifying the Scorecard score (reviewer instructions)
The published score is recomputed by OpenSSF infrastructure; to reproduce locally:
```sh
# Requires a GitHub token (read-only is fine).
scorecard --repo=github.com/sparq-org/sparq --show-details
# Or read the latest published result:
#   https://scorecard.dev/viewer/?uri=github.com/sparq-org/sparq
```
The SARIF from each run is also in the GitHub **Security → Code scanning** tab and as the
`scorecard.yml` `SARIF file` artifact (5-day retention).
