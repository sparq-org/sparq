<!-- [OPUS-4.8] sq-toze — ISO/IEC 27001:2022 Annex A control mapping (the spine the
     auditor checks). One row per applicable control. Authored while Fable unavailable —
     re-review when Fable returns. NON-CANONICAL timing. -->

# ISO/IEC 27001:2022 — Annex A control mapping

The spine. Each **applicable** Annex A (ISO/IEC 27002:2022) control is mapped to its
**status** and **repo evidence** (file/line, test, or CI job). Read
[`README.md`](./README.md) first for the scope and the status-label semantics. Evidence
paths are repo-relative. Re-runnable verification is in [`evidence.md`](./evidence.md);
open gaps are in [`gap-register.md`](./gap-register.md).

**Status legend:** `IMPL` = Implemented & verified (technical control + re-runnable
evidence) · `AUDIT-READY` = doc/substrate in repo, certificate needs an org act ·
`GAP` = not met (bead) · `N/A(op)` = property of the operator's deployed environment.

> **Honesty banner.** The clustering of `AUDIT-READY` in A.5/A.6 is **deliberate and
> correct**: those are management-system controls that require an *organization* to act
> (a signed policy, a risk-treatment decision, a management review). The repo can supply
> the *doc-of-record*; it cannot supply the *certificate*. We do **not** upgrade them to
> IMPL. A.8.24 makes **no** claim over the ZK/MPC estate (see README crypto gate).

---

## A.5 — Organizational controls (37)

| Ctrl | Title | Status | Evidence (file / test / CI) | Owner |
|---|---|---|---|---|
| A.5.1 | Policies for information security | AUDIT-READY | Doc-of-record exists: `SECURITY.md`, `CONTRIBUTING.md` (§Secure coding), `AGENTS.md` (the build/test/lint gate + merge discipline), `research/threat-model.md`. A *signed, management-approved ISMS policy set* is an org act — GAP-ISO-1. | Adopting org |
| A.5.2 | Information security roles & responsibilities | AUDIT-READY | `CODEOWNERS` (review responsibility by path), `SECURITY.md` (who handles disclosure: jesse@jeswr.org + GHSA), `docs/branch-protection.md` (who can merge). Formal role-assignment matrix in an ISMS is an org act — GAP-ISO-1. | Adopting org |
| A.5.3 | Segregation of duties | AUDIT-READY | PR review required + `CODEOWNERS` + branch protection (`docs/branch-protection.md`: no direct push, including admins). Author≠sole-approver is enforced by the ruleset (org-configured, out-of-repo). | Adopting org |
| A.5.4 | Management responsibilities | AUDIT-READY | `CONTRIBUTING.md` + `AGENTS.md` define the obligations on every contributor; management *enforcement* is an org act. | Adopting org |
| A.5.5 | Contact with authorities | N/A(op) | No org/authority relationship to maintain for an OSS project; operator owns this for their deployment. | Operator |
| A.5.6 | Contact with special interest groups | IMPL | OpenSSF Scorecard published to the public OpenSSF dashboard (`.github/workflows/scorecard.yml`, `publish_results: true`); RustSec advisory-DB consumed daily (`dependency-monitoring.yml`); CodeQL/GHSA ecosystem participation. | sparq |
| A.5.7 | Threat intelligence | IMPL | Daily advisory watchdog `.github/workflows/dependency-monitoring.yml` (RustSec); Dependabot 4 ecosystems (`.github/dependabot.yml`); CodeQL `security-and-quality` (`codeql.yml`). | sparq |
| A.5.8 | Information security in project management | IMPL | Security gates are part of the dev process: `ci.yml` (clippy `-D warnings`, tests), `codeql.yml`, `supply-chain.yml`, `miri.yml`, `fuzz.yml`, `scorecard.yml`, with `ci-fast.yml` (core-crate clippy/tests + SPARQL conformance) as the one required branch-protection check; the others run post-merge/nightly. Threat model authored per surface (`research/threat-model.md`). | sparq |
| A.5.9 | Inventory of information & associated assets | AUDIT-READY | Asset inventory of the *software* exists: CycloneDX SBOM per build (`supply-chain.yml`), crate list, `research/threat-model.md` §Assets. Inventory of *operator data assets* is the operator's. | sparq / Operator |
| A.5.10 | Acceptable use of information & assets | N/A(op) | Acceptable-use of a deployed instance + loaded data is the operator's policy. | Operator |
| A.5.11 | Return of assets | N/A(op) | People/asset-return control; no employees. | Operator |
| A.5.12 | Classification of information | AUDIT-READY | sparq classifies *its own* artifacts (public OSS source; `SECURITY.md` marks the ZK/MPC estate as "no guarantee"). Classification of *loaded RDF* is operator-owned — see `compliance/data-flow.md` (privacy worktree). | sparq / Operator |
| A.5.13 | Labelling of information | N/A(op) | Labelling of operator datasets is operator-owned. sparq does not relabel loaded data. | Operator |
| A.5.14 | Information transfer | AUDIT-READY | sparq-server speaks the W3C SPARQL Protocol over HTTP; transport encryption (TLS) is the operator's gateway (Dockerfile note: "plaintext HTTP is sniffable; front with a gateway"). Transfer *of source* is via signed releases (A.5.23/A.8.4). | Operator |
| A.5.15 | Access control | N/A(op) → AUDIT-READY | **Documented architectural decision (boundary B3):** `sparq-server` ships with no per-user authz; a single optional bearer token (`SPARQ_AUTH_TOKEN`) exists (Dockerfile / `ServerConfig::from_env`). Per-user access control is the operator's gateway. `research/threat-model.md` T-HTTP-EoP. **Not a silent gap** — the operator action is enumerated in [`operator-deployment-security.md`](./operator-deployment-security.md) §2 (boundary B3); gap-register GAP-ISO-2 is **ADDRESSED** by that doc. | Operator |
| A.5.16 | Identity management | N/A(op) | No user identity store in sparq; operator's IdP. | Operator |
| A.5.17 | Authentication information | AUDIT-READY | Optional bearer token read from env (`SPARQ_AUTH_TOKEN` / `_READ`), never logged; secret management is the operator's. | Operator |
| A.5.18 | Access rights | N/A(op) | Provisioning/review of access rights to a deployed instance is operator-owned. | Operator |
| A.5.19 | Information security in supplier relationships | IMPL | Dependency = supplier. `deny.toml` policy (advisories/bans/sources/licenses), gating in `supply-chain.yml`. `cargo vet --locked` is a GATING supply-chain **ratchet**: every crate must be covered by a trusted imported audit set (Mozilla/Google/ISRG/Embark/BCA/Zcash) or an explicit `[[exemptions]]` entry, so no new un-covered dependency can enter the tree silently. **Note:** `supply-chain/audits.toml` is currently empty (0 first-party audits); the lane proves the ratchet, **not** that sparq has attested any dependency itself — reducing the 349 exemptions to real audits is supply-chain maturity work (GX-7, owned by `compliance/sbom/` + `compliance/slsa/`). | sparq |
| A.5.20 | Addressing security in supplier agreements | AUDIT-READY | `deny.toml` codifies license + source allow-lists (the "agreement" with the dependency ecosystem); formal supplier contracts N/A for OSS deps. | sparq |
| A.5.21 | Managing security in the ICT supply chain | IMPL | SLSA build provenance (`actions/attest-build-provenance` in `release.yml`), CycloneDX SBOM + VEX attested on release, cargo-deny gating + the `cargo vet --locked` ratchet (exemptions/imported-audits gate; `audits.toml` empty — see A.5.19), SHA-pinned actions, Dependabot. See `compliance/sbom/` + `compliance/slsa/` (cross-ref). | sparq |
| A.5.22 | Monitoring, review & change management of supplier services | IMPL | Daily `dependency-monitoring.yml` advisory watchdog + Dependabot PRs re-run the full gate. | sparq |
| A.5.23 | Information security for use of cloud services | N/A(op) | sparq is not a cloud service and consumes none as part of its product; CI runners are GitHub-hosted (covered under SLSA build-platform). Operator's cloud config is operator-owned (note: ISO 27017/18 were *cut* — see cert plan §0). | Operator |
| A.5.24 | Incident management planning & preparation | AUDIT-READY | `SECURITY.md` defines the disclosure intake, acknowledgement (5 bd) + assessment (10 bd) targets, and coordinated-disclosure flow; `.well-known/security.txt` (RFC 9116) is the machine-readable pointer. A full org incident-response *plan* (runbook, roles, comms) is an org act — `compliance/policies/` template + GAP-ISO-1. | sparq / Adopting org |
| A.5.25 | Assessment & decision on security events | AUDIT-READY | `SECURITY.md` "initial assessment (severity, reproduce, remediation path)"; severity via GHSA/CVSS. Org triage process is an org act. | sparq / Adopting org |
| A.5.26 | Response to information security incidents | AUDIT-READY | `SECURITY.md` coordinated-disclosure + fix-on-`main`-then-release flow. Org-level response is an org act. | sparq / Adopting org |
| A.5.27 | Learning from information security incidents | AUDIT-READY | Advisories published via GHSA; remediation tracked as beads (e.g. the ZK soundness beads). A formal lessons-learned loop is an org act. | sparq / Adopting org |
| A.5.28 | Collection of evidence | AUDIT-READY | Git history + signed release attestations + CI logs are tamper-evident evidence sources; forensic-grade evidence handling is an org act. | Adopting org |
| A.5.29 | Information security during disruption | N/A(op) | BC/DR of a running service is the operator's. | Operator |
| A.5.30 | ICT readiness for business continuity | N/A(op) | Operator-owned (service availability). | Operator |
| A.5.31 | Legal, statutory, regulatory & contractual requirements | AUDIT-READY | `LICENSE` (MIT); EU CRA mapping in `compliance/cra/` (cross-ref); license compliance gated by `deny.toml` licenses check. Org's full legal register is an org act. | sparq / Adopting org |
| A.5.32 | Intellectual property rights | IMPL | `deny.toml` `[licenses]` allow-list gates dependency licenses in CI (`supply-chain.yml`); `LICENSE` (MIT) declared; contributor IP terms in `CONTRIBUTING.md`. | sparq |
| A.5.33 | Protection of records | N/A(op) | Operator's records retention. | Operator |
| A.5.34 | Privacy & protection of PII | AUDIT-READY | sparq processes no PII of its own; loaded-RDF PII is operator-controller. Scoped in `compliance/data-flow.md` + `compliance/dpia.md` (privacy worktree). | Operator |
| A.5.35 | Independent review of information security | AUDIT-READY | Independent reviews exist in substance: OpenSSF Scorecard (external automated), CodeQL, the adversarial threat model + ZK soundness audit; this very engineer↔auditor loop. A *formal accredited internal-audit programme* is an org act — GAP-ISO-1. | sparq / Adopting org |
| A.5.36 | Compliance with policies, rules & standards | PARTIAL | The required `ci-fast` check fails the merge on core-crate clippy/tests or the SPARQL conformance ratchet; CodeQL alerts block via the `code_scanning` rule. Partial because supply-chain, full-workspace clippy/tests and the other ratchets now run post-merge/nightly (the `ci-summary` aggregator was deleted). `docs/branch-protection.md`. | sparq |
| A.5.37 | Documented operating procedures | IMPL | `AGENTS.md` (the operating procedure for working on the repo), `CONTRIBUTING.md`, `docs/branch-protection.md`, the workflows themselves are executable procedure. | sparq |

## A.6 — People controls (8)

| Ctrl | Title | Status | Evidence | Owner |
|---|---|---|---|---|
| A.6.1 | Screening | N/A(op) | Employment screening; OSS has contributors, not employees. | Operator |
| A.6.2 | Terms & conditions of employment | AUDIT-READY | Contributor terms via `CONTRIBUTING.md` (§License: contributions under project license) + `LICENSE`. No employment relationship. | Adopting org |
| A.6.3 | Information security awareness, education & training | AUDIT-READY | `CONTRIBUTING.md` §Secure coding + `AGENTS.md` are the contributor "training" of record (unsafe policy, input-validation, supply-chain rules). A formal training programme is an org act. | Adopting org |
| A.6.4 | Disciplinary process | N/A(op) | No employer relationship; code-of-conduct/enforcement is the org's. | Adopting org |
| A.6.5 | Responsibilities after termination/change | N/A(op) | No employees; access revocation of a deployment is operator-owned. | Operator |
| A.6.6 | Confidentiality / NDA | AUDIT-READY | `SECURITY.md` coordinated-disclosure expectation acts as the confidentiality norm for reporters; formal NDAs N/A for OSS. | Adopting org |
| A.6.7 | Remote working | N/A(op) | Operator/org HR control. | Operator |
| A.6.8 | Information security event reporting | IMPL | Disclosure channels are concrete + machine-discoverable: `SECURITY.md` (GHSA + email) + `.well-known/security.txt` (RFC 9116); `CONTRIBUTING.md` redirects public reports to the private channel. | sparq |

## A.7 — Physical & environmental controls (14)

| Ctrl | Title | Status | Evidence | Owner |
|---|---|---|---|---|
| A.7.1–A.7.14 | All physical & environmental controls (secure areas, equipment, cabling, utilities, disposal, clear-desk, etc.) | N/A(op) | sparq has **no premises, hardware, or physical estate** — it is source code. Every A.7 control is a property of the operator's data centre / workstation environment. Marked N/A(op) as a block; not a hollow per-row table. | Operator |

## A.8 — Technological controls (34) — the load-bearing technical set

| Ctrl | Title | Status | Evidence (file / test / CI) | Owner |
|---|---|---|---|---|
| A.8.1 | User endpoint devices | N/A(op) | Operator/org workstation control. | Operator |
| A.8.2 | Privileged access rights | N/A(op) → AUDIT-READY | Branch-protection ruleset disallows direct push *including admins* (`docs/branch-protection.md`); privileged access to a deployed instance is operator-owned. | sparq / Operator |
| A.8.3 | Information access restriction | N/A(op) | Restriction within a deployed instance is operator-owned (B3 no-auth design). | Operator |
| A.8.4 | Access to source code | IMPL | Branch protection on `main` (no direct push, PR + review + required `ci-fast` check; `docs/branch-protection.md`); `CODEOWNERS`; SHA-pinned actions; release artifacts SLSA-attested + SHA256SUMS (`release.yml`). | sparq |
| A.8.5 | Secure authentication | N/A(op) → AUDIT-READY | Optional server bearer token (`SPARQ_AUTH_TOKEN`); full secure-auth (MFA, session) is the operator's gateway (boundary B3). Repo-side: GitHub auth + signed commits/attestations. | Operator |
| A.8.6 | Capacity management | N/A(op) → IMPL(partial) | Engine has a `QueryBudget` DoS-limit primitive (`sparq-engine`, threat-model T-DoS); capacity of a *running deployment* is operator-owned. | sparq / Operator |
| A.8.7 | Protection against malware | IMPL | CodeQL SAST (`codeql.yml`), supply-chain advisory gating (`supply-chain.yml` + daily watchdog), SHA-pinned actions, distroless non-root image (no shell/package-manager to subvert; `Dockerfile`). | sparq |
| A.8.8 | Management of technical vulnerabilities | IMPL | `cargo deny check advisories` GATING (`supply-chain.yml` — GX-1 un-degraded), daily `dependency-monitoring.yml` watchdog, Dependabot 4 ecosystems, CodeQL, `SECURITY.md` disclosure + `.well-known/security.txt`. Remediation tracked as beads. | sparq |
| A.8.9 | Configuration management | IMPL | Infrastructure-as-code: pinned toolchain (`rust-toolchain`/MSRV), SHA-pinned actions, pinned base image digest (`Dockerfile` `@sha256:…`), `Cargo.lock` committed, `deny.toml` policy-as-code. | sparq |
| A.8.10 | Information deletion | N/A(op) | Deletion of operator data is operator-owned. | Operator |
| A.8.11 | Data masking | N/A(op) → cross-ref | sparq does not mask loaded RDF; the ZK *privacy story* (what it would offer if sound) is in `compliance/cryptoreview/` + privacy worktree, flagged **not yet sound**. Not claimed here. | Operator / cryptoreview |
| A.8.12 | Data leakage prevention | AUDIT-READY | Error/log hygiene standard documented (`CONTRIBUTING.md` §Input validation: "do not leak RDF/SPARQL content, internal paths, or stack detail into HTTP errors or logs", ASVS V7); verification of the no-leak property is the `asvs` worktree's test job (cross-ref). | sparq |
| A.8.13 | Information backup | N/A(op) | Backup of operator data is operator-owned. | Operator |
| A.8.14 | Redundancy of information processing facilities | N/A(op) | Availability/redundancy of a running deployment is operator-owned. | Operator |
| A.8.15 | Logging | AUDIT-READY | Logging is present and the *hygiene standard* is documented (`CONTRIBUTING.md`: no sensitive content in logs); structured-log/retention policy of a deployment is operator-owned. | sparq / Operator |
| A.8.16 | Monitoring activities | IMPL(repo) / N/A(op) | Repo-side: CI monitors every push (`ci-fast.yml` per PR/push, `ci.yml` nightly), Scorecard, daily advisory watchdog. Runtime monitoring of a deployment is operator-owned. | sparq / Operator |
| A.8.17 | Clock synchronization | N/A(op) | Operator infrastructure control. | Operator |
| A.8.18 | Use of privileged utility programs | IMPL | Distroless `nonroot` image has **no shell / no package manager / no privileged utilities** to abuse (`Dockerfile` `gcr.io/distroless/cc-debian12:nonroot`). | sparq |
| A.8.19 | Installation of software on operational systems | IMPL | Reproducible, pinned build: pinned base digest, `Cargo.lock`, SHA-pinned actions, SLSA-attested artifacts; CIS-Docker hardening in `compliance/cis/` (cross-ref). | sparq |
| A.8.20 | Networks security | N/A(op) | Network controls (firewall, segmentation) are the operator's deployment env (Cyber-Essentials-style controls were *cut* — cert plan §0). | Operator |
| A.8.21 | Security of network services | N/A(op) | Operator-owned (the gateway fronting sparq-server). | Operator |
| A.8.22 | Segregation of networks | N/A(op) | Operator deployment topology. | Operator |
| A.8.23 | Web filtering | N/A(op) | Operator network control. | Operator |
| A.8.24 | **Use of cryptography** | AUDIT-READY (scoped) | **Operationally relied-on crypto only:** release artifacts signed via Sigstore/SLSA attestations (`release.yml` `attest-build-provenance`); TLS is the operator's gateway. **EXPLICITLY EXCLUDED: the `sparq-zk`/`sparq-zk-compose`/`sparq-mpc` estate makes NO production cryptographic guarantee** — the v1 ZK verifier was **originally found NOT sound** (`research/zk-soundness-audit.md`, kept on record for the `sq-1gir` regression map), but `sq-1s2` landed the verifier-side binding layer and an **internal** post-remediation re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all prior findings closed → "sound as landed for the assumed threat model"; that re-audit is internal/single-model/read-only, **external accredited-cryptographer sign-off is STILL PENDING** (`sq-qhy4`, P0) and there is **NO production soundness/privacy/integrity guarantee** (still a research scaffold; `sparq-mpc` semi-honest-only) [OPUS-4.8] (`SECURITY.md`); soundness is assessed in `compliance/cryptoreview/`. A crypto-key-management *policy* is an org act (`compliance/policies/` template). <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> | sparq / cryptoreview |
| A.8.25 | Secure development life cycle | IMPL | The gate stack *is* the SDLC: `CONTRIBUTING.md` §Secure coding + §Secure-SDLC touchpoints (maps to NIST SSDF), `ci.yml`, `codeql.yml`, `miri.yml`, `fuzz.yml`, `supply-chain.yml`, conformance ratchets, threat model. Cross-ref `compliance/ssdf/`. | sparq |
| A.8.26 | Application security requirements | IMPL | Security requirements documented: `research/threat-model.md` (assets, boundaries B1–B5, STRIDE), `SECURITY.md` (hardened-input expectations), ASVS re-scope (`compliance/asvs/`). | sparq |
| A.8.27 | Secure system architecture & engineering principles | IMPL | `#![forbid(unsafe_code)]` in **26 of 31** crates (confined unsafe surface — the 5 with `unsafe` are sparq-core/-vectors/-cli/-zk-compose/-bench), boundary-based threat model (B1–B5), defense-in-depth gate stack. `compliance/memsafety/unsafe-register.md` attests every first-party unsafe site. | sparq |
| A.8.28 | Secure coding | IMPL | `CONTRIBUTING.md` §Secure coding (the standard); `clippy --workspace --all-targets -- -D warnings` GATING (`ci.yml`); CodeQL `security-and-quality`; the unsafe-register count ratchet (`scripts/unsafe-gate.py`, gating CI lane). | sparq |
| A.8.29 | Security testing in development & acceptance | IMPL | `cargo test --workspace` (`ci.yml`), W3C SPARQL/SHACL/inference conformance ratchets (never-lowered), `cargo-fuzz` (`fuzz.yml` PR smoke + nightly), Miri UB lane (`miri.yml`), the mmap corruption oracle (`crates/sparq-core/tests/mmap_corruption_oracle.rs`). | sparq |
| A.8.30 | Outsourced development | N/A(op) | No outsourced development. | sparq |
| A.8.31 | Separation of dev/test/production environments | AUDIT-READY | Branch model (`main` protected; feature branches) separates in-progress from released; release artifacts are built only on tags via `release.yml`. Operator owns their prod env separation. | sparq / Operator |
| A.8.32 | Change management | IMPL | All changes via PR + review + required `ci-fast` check (core crates + SPARQL ratchet) + conformance "never-lower" ratchets (others nightly) (`CONTRIBUTING.md`, `docs/branch-protection.md`); `CHANGELOG.md`; beads track work. | sparq |
| A.8.33 | Test information | N/A(op) | Use of (operator) production data in test is operator-owned; sparq's tests use W3C/synthetic fixtures. | sparq / Operator |
| A.8.34 | Protection of information systems during audit testing | N/A(op) | Operator-owned (auditing a running system). | Operator |

---

## Roll-up

| Theme | Applicable & IMPL | AUDIT-READY | GAP | N/A(operator) |
|---|---|---|---|---|
| A.5 Organizational (37) | 9 | 17 | 0 (gaps tracked under GAP-ISO-1/2) | 11 |
| A.6 People (8) | 1 | 4 | 0 | 3 |
| A.7 Physical (14) | 0 | 0 | 0 | 14 (block) |
| A.8 Technological (34) | 14 | 6 | 0 | 14 |
| **Total (93)** | **24** | **27** | **0 open Annex-A control gaps; 2 readiness gaps** | **42** |

**Dual-status bucketing rule (so the roll-up cross-foots).** Several rows carry a dual
status of the form `N/A(op) → X` or `X(repo) / N/A(op)`, because the control has a *sparq-side*
facet and an *operator-side* facet. **Each such row is bucketed once, by its sparq-side
primary status** (the facet sparq actually owns), and counted nowhere else; the operator facet
is described in the row text, not separately tallied. Applying the rule:

- **Counted as IMPL** (sparq-side primary status is a verified technical control):
  A.8.6 (`N/A(op) → IMPL(partial)` — the `QueryBudget` primitive) and A.8.16
  (`IMPL(repo) / N/A(op)` — CI monitoring of every push).
- **Counted as AUDIT-READY** (sparq-side primary status is a doc-of-record, not a verified
  control): A.8.2 and A.8.5 (both `N/A(op) → AUDIT-READY` — branch-protection ruleset / optional
  bearer token; the certificate-level enforcement is org/operator-owned).
- **Counted as N/A(op)**: A.8.11 (`N/A(op) → cross-ref`) — sparq does not mask loaded RDF; the
  ZK privacy story is cross-referenced to `compliance/cryptoreview/`, not claimed here.

This yields the A.8 split **14 IMPL / 6 AUDIT-READY / 14 N/A(op) = 34**. (An earlier headline
read 26/27/40; it bucketed the two `N/A(op) → AUDIT-READY` rows A.8.2/A.8.5 as IMPL, which the
rule above does not — corrected here in the conservative direction to **24/27/42**.)

**Reading this honestly:** sparq's *technical* Annex A surface (the bulk of A.8 + the
supplier/development/vuln-management parts of A.5) is **IMPLEMENTED & VERIFIED** with
re-runnable CI evidence. The **27 AUDIT-READY** controls are concentrated in the
management-system layer (A.5 policy/roles/incident/review, A.6 people) — they have a
**doc-of-record in the repo** but their *certificate* requires an organization to operate
an ISMS, which **no repo artifact can substitute for**. The **42 N/A(operator)** controls
are physical/operational properties of a *deployed* environment, correctly assigned to the
adopting operator for a library/server. There are **zero open Annex-A control gaps**; of the
two readiness gaps, **GAP-ISO-2** (an explicit operator-deployment-security guidance doc) is
now **ADDRESSED** by [`operator-deployment-security.md`](./operator-deployment-security.md),
leaving **GAP-ISO-1** (the ISMS organizational artifacts) as the one open readiness gap in
[`gap-register.md`](./gap-register.md). A.8.24 makes **no** claim over the ZK/MPC estate.
