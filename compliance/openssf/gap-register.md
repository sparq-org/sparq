<!-- [OPUS-4.8] OpenSSF gap register (epic sq-toze, bead sq-toze.15). Authored while Fable
     unavailable — re-review when Fable returns. Every open gap: severity, remediation,
     target, bd bead. No gap is papered over. -->

# OpenSSF — gap register

Open gaps for the OpenSSF slice (Scorecard + Best-Practices badge). Severity follows the
plan's legend: **P1** needed for a perfect score on a high-value framework; **P2/P3** raise
maturity. Each row carries its tracking `bd` bead under epic `sq-toze`.

| ID | Gap | Sev | Status | Remediation | Target | Bead |
|---|---|---|---|---|---|---|
| **GX-4** | **OpenSSF Best-Practices (CII) badge not *filed*.** The questionnaire is fully drafted and evidence-grounded — now in BOTH a human-readable form (`evidence.md` §Badge) AND a machine-readable, import-ready self-cert ([`best-practices-self-cert.json`](./best-practices-self-cert.json): 63 criteria, each with status + justification + repo-relative evidence path), CI-gated by [`scripts/check-bestpractices-evidence.py`](../../scripts/check-bestpractices-evidence.py) (`supply-chain.yml#openssf-selfcert`) so the answers cannot silently drift. sparq meets the passing bar, but no entry exists on bestpractices.dev — so Scorecard's `CII-Best-Practices` check scores 0 and there is no badge to display. | P1 | PARTIAL — in-repo payload + CI gate DONE; external filing remains | Maintainer creates the bestpractices.dev project entry and transcribes the answers (now a structured JSON, not just prose). This is a human-owned external filing step — the agent cannot create the entry. Once filed, set `project.filed=true` + `badge_url` in the JSON (the gate then *requires* the URL) and link the badge in `README.md`, then re-run Scorecard. | maintainer to file | **sq-toze.5** (blocks **sq-toze.15**) |
| **GX-OSSF-2** | **Registry publishes are partly signing-attested (PARTIAL).** GitHub release assets carry Sigstore SLSA provenance (Scorecard `Signed-Releases` satisfied). **npm `@sparq-org/sparq` now publishes with native Sigstore provenance** (`.github/workflows/publish.yml#npm`, `npm publish --provenance`, GX-10/sq-toze.24). **crates.io** `.crate` bytes get an out-of-band `attest-build-provenance` attestation (`#crates`) but the registry stores **no native provenance/signature link** (no first-party scheme upstream — external). **PyPI `sparq-rdf` attestations/Trusted Publishing now WIRED in CI (sq-toze.37):** `publish.yml#pypi-publish` uploads via Trusted Publishing with native PEP-740 attestations (`pypa/gh-action-pypi-publish` `attestations: true` + OIDC); emits provenance once a maintainer registers the Trusted Publisher on the PyPI project. | P3 | PARTIAL | npm DONE (`publish.yml`). PyPI PEP-740 lane DONE in CI (`publish.yml#pypi-*`, sq-toze.37) — awaits one-time PyPI Trusted-Publisher registration (maintainer); crates.io native link awaits upstream (document honestly — no laundering it as "signed"). | next release-tooling pass | **sq-jgt3** (npm via **sq-toze.24**), **sq-toze.37** (PyPI) |
| **GX-OSSF-3** | **Scorecard Code-Review / Branch-Protection score is depressed by the solo-maintainer reality.** Scorecard infers review from *merged-PR history* and discounts self-approval, so a solo-maintained, agent-driven repo cannot score 10 on `Code-Review` / `Branch-Protection` regardless of the rule. The live ruleset is also out-of-repo (cannot be asserted from a tracked file), and its always-on repository-administrator bypass is an explicit exception to uniform enforcement. | P3 | PARTIAL — in-repo evidence DONE; external re-verify remains | **DONE (sq-sto1):** [`docs/branch-protection.md`](../../docs/branch-protection.md) now (a) documents the solo-maintainer reality in a dedicated [§Solo-maintainer & the Scorecard score](../../docs/branch-protection.md#solo-maintainer--the-scorecard-code-review--branch-protection-score) section (why the score is inherent + the compensating Copilot/CodeQL/conversation-resolution/merge-queue controls, with the administrator bypass stated explicitly), and (b) was **reconciled against the captured live ruleset** — several sections previously over-claimed an aspirational two-human flow (`≥1 approval` + CODEOWNERS gate + up-to-date + rebase merges); they were corrected to match the enforced reality (`required_approving_review_count: 0`, `require_code_owner_review: false`, `strict_…: false`, squash-only), with a `gh api repos/sparq-org/sparq/rulesets/<id>` verification procedure + a rule-by-rule match table. The automated landing path does not use the administrator bypass and remains constrained by merge-queue admission, squash-only history, and the force-push/deletion rules. **Remaining (external):** the maintainer periodically re-runs that procedure to catch live-ruleset drift. | maintainer to re-verify live ruleset periodically | **sq-sto1** |

## Cross-framework gaps that also touch OpenSSF (owned elsewhere — referenced, not duplicated)

These are tracked in their owning slice's register; they surface in OpenSSF scoring but the
remediation lives in the supply-chain / slsa frameworks.

| ID | Gap (as it touches OpenSSF) | Owning slice | Bead |
|---|---|---|---|
| **GX-8** | **No reproducible-build attestation.** Touches Badge `build_reproducible` (answered **Unmet** honestly in `evidence.md`; it is SUGGESTED-not-required at the passing level, so it does not block the bronze badge). | slsa / cra | (slsa gap-register; gap-fix under sq-toze) |

## Decision record — Scorecard SARIF is no longer uploaded to code-scanning

[OPUS-4.8] `chore-codescanning-triage`. The repo's standing requirement is **ZERO open
code-scanning alerts**. `scorecard.yml` previously uploaded its SARIF to the GitHub Security
tab, which surfaced five Scorecard *scores* as "alerts": `BranchProtection` (high),
`CodeReview` (high), `Maintained` (high), `VulnerabilitiesID` (high), `CIIBestPractices`
(low). **None is a code vulnerability** — they are posture scores, and the inherent-by-design
ones (`CodeReview`, `BranchProtection`) cannot be raised by a code change in this repo's
operating model. The residual-alert triage below also covers `Fuzzing`, named alongside
`Vulnerabilities` / `Maintained` in the residual-triage bead **sq-cgzx**:

| Scorecard check | Why it scores low (honest) | Disposition |
|---|---|---|
| `CodeReviewID` | Scorecard infers review from merged-PR history and discounts self-approval; this is a solo-maintained, agent-driven repo (Copilot + CODEOWNERS ruleset, not a 2-human-reviewer flow). | Inherent-by-design (GX-OSSF-3). Dismissed from code-scanning; not a fixable code issue. |
| `BranchProtectionID` | The live ruleset (`docs/branch-protection.md`) is ruleset-based, not classic branch-protection; Scorecard scores classic settings (stale-review-dismissal, required approvers) the model intentionally does not use. | Inherent-by-design (GX-OSSF-3). Dismissed. |
| `MaintainedID` | Scored low only because the repo is **<90 days old** (Scorecard's `Maintained` check counts recent commits/issue activity over the trailing 90 days); mechanical, self-resolves with age and ongoing commit cadence. | Time-based, not fixable now. Dismissed; residual-triage bead sq-cgzx. |
| `VulnerabilitiesID` | RUSTSEC-2025-0134 `rustls-pemfile` is retired for good ([OPUS-5], sq-5ah3p): the mTLS PEM parse in `sparq-lws-core`/`sparq-server` moved to `rustls-pki-types`' `PemObject`, so the archived crate left the tree and the ignore, the VEX statement and the cargo-vet exemption were dropped together. The remaining `deny.toml` ignores are the two maintenance-status notices with no fix available upstream (RUSTSEC-2024-0436 `paste` via parquet 59, RUSTSEC-2025-0141 `bincode` via hdt/qwt) and the two quick-xml <0.41 availability-DoS advisories reachable only through the transitive oxigraph 0.5.x copy (RUSTSEC-2026-0194/0195). The only other residual surfacing is one transitive JS devDep advisory (GHSA-qx2v-qp2m-jg93, PostCSS, site tooling). **No fixable Rust security advisory** — the cargo-deny advisory gate (GX-1, #210) is un-degraded and would FAIL on a real one. | Informational JS-only residual; no Rust fix outstanding. Dismissed; residual-triage bead sq-cgzx. |
| `FuzzingID` | **Now satisfied — no longer the "OSS-Fuzz = future" informational gap the bead anticipated.** Coverage-guided fuzzing is wired: [`fuzz.yml`](../../.github/workflows/fuzz.yml) runs `cargo-fuzz` (PR smoke + daily heavy tier) over the RDF/SPARQL parsers and the mmap store loader, plus the SHACL differential lane [`shacl-diff-fuzz.yml`](../../.github/workflows/shacl-diff-fuzz.yml) (bead **sq-ovnf**, closed). Scorecard's `Fuzzing` check detects the `cargo-fuzz` integration and scores it. OSS-Fuzz onboarding remains an optional future maturity nudge, not a posture failure. | Satisfied by `fuzz.yml`/`shacl-diff-fuzz.yml` (sq-ovnf). No fix needed; residual-triage bead sq-cgzx. |
| `CIIBestPracticesID` | The OpenSSF Best-Practices badge is not filed (GX-4) — a human-owned external step. | Known gap GX-4 (sq-toze.5). Dismissed; tracked. |

**Decision (option a):** keep Scorecard as the **public score + badge** (`publish_results:
true` → OpenSSF dashboard) and **stop uploading its SARIF to code-scanning** (removed the
`upload-sarif` step + the now-unneeded `security-events: write` scope). The full posture
detail remains on the public dashboard and as the build artifact; it no longer pollutes the
Security tab. The five pre-existing alerts were dismissed (`won't fix`) with the per-row
reasons above (and the residual `Vulnerabilities` / `Maintained` / `Fuzzing` triage of
**sq-cgzx** is recorded in the same table — `Fuzzing` is now satisfied, the other two are
informational/time-based). This loses no certification evidence — the score/badge is the
OpenSSF artifact, the Security tab was redundant noise.

## Closed gaps (cite as evidence, do not re-open)

| ID | Gap | Closed by | Evidence |
|---|---|---|---|
| **GX-3** | No `.well-known/security.txt` (RFC 9116) machine-discoverable disclosure pointer. | **CLOSED** | [`.well-known/security.txt`](../../.well-known/security.txt) (bead **sq-toze.4**, closed). Strengthens Scorecard `Security-Policy` + Badge `vulnerability_report_private`. |
| **GX-6** | No documented secure-coding standard. | **CLOSED** | [`CONTRIBUTING.md`](../../CONTRIBUTING.md) "Secure coding" (bead **sq-toze.7**). Strengthens Badge `contribution_requirements`. |
| **GX-5** | No per-site unsafe justification register. | **CLOSED** | [`compliance/memsafety/unsafe-register.md`](../memsafety/unsafe-register.md) (bead **sq-toze.6**). Strengthens Badge `dynamic_analysis_unsafe`. |
| **GX-1** | PR-time **advisory** gating was degraded (cargo-deny CVSS-4.0 parse blocker, sq-q8de). | **CLOSED** | [`supply-chain.yml`](../../.github/workflows/supply-chain.yml) `audit` job now runs `cargo deny check advisories` as a **real PR/push/merge_group gate** (no `continue-on-error`) over a fail-closed [`deny.toml`](../../deny.toml); closed by **#210** (bead **sq-toze.2**, closed; the CVSS-4.0 blocker sq-q8de is resolved). The daily watchdog ([`dependency-monitoring.yml`](../../.github/workflows/dependency-monitoring.yml)) is now defence-in-depth. Touches Scorecard `Vulnerabilities` / Badge `vulnerabilities_*`. |

## Honest bottom line

The OpenSSF posture is **strong**: every repository-level Scorecard check that can be
asserted from the codebase/CI is **implemented & verified**, and the Best-Practices badge is
**answer-ready at the passing level** (with silver/gold reach on warnings, signed releases,
and fuzz+Miri analysis). The **only blocking gap to a displayed badge / full Scorecard
posture is the external badge filing (GX-4)** — a human-owned step the agent cannot perform.
The two remaining gaps (GX-OSSF-2 registry-publish signing, GX-OSSF-3 solo-maintainer review
score) are maturity nudges, not posture failures. No control is overclaimed; the advisory
PR-gate is **un-degraded** (GX-1 closed by #210 / sq-toze.2 — now two gating `cargo deny`
steps over a fail-closed `deny.toml`), and the unattested registry publishes are recorded,
not laundered.
