<!-- [OPUS-4.8] sq-ez5z (productionized from sq-toze) — ISO/IEC 27001:2022 Statement of
     Applicability (SoA) TEMPLATE: the full Annex A control-by-control table + ISMS clauses-4-10
     pointers. This is a TEMPLATE for an adopting organization; it is NOT a certificate and NOT
     a signed policy. Remediation of gap GAP-ISO-1. Re-review when Fable returns. -->

# ISO/IEC 27001:2022 — Statement of Applicability (SoA) template

> **THIS IS AN ADOPTABLE TEMPLATE, NOT A CERTIFICATION CLAIM.** It is provided for an
> **adopting organization** to populate, decide, and sign as part of operating its own ISMS.
> Nothing here is a passed control, a management decision, or an accredited audit result.
> Populating it does **not** make sparq, or the adopting organization, "ISO 27001 certified" —
> certification is an external act of an **accredited certification body** after a Stage 1 +
> Stage 2 audit of an **operating ISMS over time**. The *sparq-side status + technical-evidence*
> columns are pre-filled from [`controls.md`](./controls.md) to save the organization work; the
> **org applicability decision, justification-of-inclusion/exclusion, org-status, and sign-off
> columns are left blank (`<FILL-IN>`) for the org to complete.** Remediates part of gap
> **GAP-ISO-1**.

## How to use this

ISO/IEC 27001 **clause 6.1.3(d)** requires a **Statement of Applicability** that, for every
Annex A control, records: (a) whether it is **applicable**, (b) the **justification** for
inclusion or exclusion, (c) the **implementation status**, and (d) a pointer to the
**control/evidence**. This template supplies (c) and (d) for **sparq-as-a-component** (the
`sparq-side status` + `sparq evidence` columns, imported from [`controls.md`](./controls.md))
so the organization only has to make the **decisions** it alone can make. The adopting
organization:

1. Sets **`Applicable? (org)`** for each control in *its* ISMS scope
   ([`isms-scope-template.md`](./isms-scope-template.md)).
2. For each **N/A(op)** sparq-side status, the control is typically **applicable in the org's
   ISMS** and the org records *its own* implementation in *its* deployment (these are the
   controls sparq cannot perform — access control, network, physical, backup, runtime).
3. For each **AUDIT-READY** sparq-side status, the org performs the **organizational act**
   (signs the policy, makes the risk-treatment decision, schedules the management review/audit)
   and records its own status + evidence.
4. For each **IMPL** sparq-side status, the org may **inherit** sparq's technical control as a
   supplier control, still recording its own configuration/verification.
5. Justifies any **excluded** control under clause 6.1.3(d).
6. Has **top management approve and sign** (clause 5.1 leadership) — the `Approved by` column.

**Status legend (sparq-side, from `controls.md`):** `IMPL` = Implemented & verified (technical
control + re-runnable evidence) · `AUDIT-READY` = doc/substrate in repo, certificate needs an
org act · `N/A(op)` = property of the operator's deployed environment · `GAP` = open readiness
gap (see [`gap-register.md`](./gap-register.md)).

> **Honesty banner (load-bearing).** The `sparq-side status` column is **not** an org SoA
> status — it states what sparq-the-component already evidences. An org **must not** copy an
> `IMPL` into its own `Status in org ISMS` without recording its own verification, and **must
> not** treat `AUDIT-READY` as "implemented." **A.8.24 makes NO claim over the `sparq-zk` /
> `sparq-zk-compose` / `sparq-mpc` estate** — the v1 ZK verifier was **originally found NOT
> sound** (`research/zk-soundness-audit.md`), but `sq-1s2` landed the binding layer and an
> **internal** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed
> → "sound as landed for the assumed threat model," with **external accredited-cryptographer
> sign-off STILL PENDING** (`sq-qhy4`, P0) and **NO production guarantee** (`SECURITY.md`)
> [OPUS-4.8]. No SoA completion may credit that estate as a cryptographic control.
> <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

---

## SoA — Annex A control-by-control (full 93)

> Columns: **Ctrl / Title** (fixed) · **sparq-side status** + **sparq evidence** (pre-filled
> from `controls.md`, do not edit) · **Applicable? (org)** / **Justification (org)** /
> **Status in org ISMS (org)** / **Approved by — date (org)** (org fills).

### A.5 — Organizational controls (37)

| Ctrl | Title | sparq-side status | sparq evidence (`controls.md`) | Applicable? (org) | Justification (org) | Status in org ISMS (org) | Approved by / date (org) |
|---|---|---|---|---|---|---|---|
| A.5.1 | Policies for information security | AUDIT-READY | `SECURITY.md`, `CONTRIBUTING.md`, `AGENTS.md`, threat model; signed ISMS policy is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.2 | Information security roles & responsibilities | AUDIT-READY | `CODEOWNERS`, `SECURITY.md`, `docs/branch-protection.md`; role matrix is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.3 | Segregation of duties | AUDIT-READY | PR review + `CODEOWNERS` + branch protection; org ruleset out-of-repo | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.4 | Management responsibilities | AUDIT-READY | `CONTRIBUTING.md` + `AGENTS.md` obligations; enforcement is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.5 | Contact with authorities | N/A(op) | Operator owns for their deployment | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.6 | Contact with special interest groups | IMPL | OpenSSF Scorecard published; RustSec/GHSA/CodeQL participation | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.7 | Threat intelligence | IMPL | Daily `dependency-monitoring.yml`; Dependabot; CodeQL | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.8 | Information security in project management | IMPL | CI gate stack (`ci.yml`, `codeql.yml`, `supply-chain.yml`, `miri.yml`, `fuzz.yml`, `scorecard.yml`; `ci-fast.yml` required); threat model | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.9 | Inventory of information & associated assets | AUDIT-READY | CycloneDX SBOM per build; crate list; threat-model §Assets. Operator data inventory is the operator's | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.10 | Acceptable use of information & assets | N/A(op) | Operator policy for deployed instance + data | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.11 | Return of assets | N/A(op) | People/asset-return; no employees | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.12 | Classification of information | AUDIT-READY | sparq classifies own artifacts; loaded-RDF classification is operator-owned (`compliance/data-flow.md`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.13 | Labelling of information | N/A(op) | Operator dataset labelling | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.14 | Information transfer | AUDIT-READY | W3C SPARQL Protocol over HTTP; TLS is operator's gateway; signed releases for source | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.15 | Access control | N/A(op) → AUDIT-READY | **Boundary B3:** no per-user authz; optional `SPARQ_AUTH_TOKEN`; operator gateway. Threat-model T-HTTP-EoP; see GAP-ISO-2 | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.16 | Identity management | N/A(op) | Operator's IdP | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.17 | Authentication information | AUDIT-READY | Optional bearer token from env, never logged; secret mgmt is operator's | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.18 | Access rights | N/A(op) | Provisioning/review for a deployed instance is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.19 | Information security in supplier relationships | IMPL | `deny.toml` policy + `supply-chain.yml`; `cargo vet --locked` ratchet (`audits.toml` empty — exemptions/imported-audit gate, not first-party attestations) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.20 | Addressing security in supplier agreements | AUDIT-READY | `deny.toml` license/source allow-lists; formal supplier contracts N/A for OSS deps | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.21 | Managing security in the ICT supply chain | IMPL | SLSA provenance (`release.yml`), CycloneDX SBOM + VEX, cargo-deny + vet ratchet, SHA-pinned actions, Dependabot. See `compliance/sbom/` + `compliance/slsa/` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.22 | Monitoring/review/change of supplier services | IMPL | Daily advisory watchdog + Dependabot re-run the full gate | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.23 | Information security for use of cloud services | N/A(op) | sparq is not a cloud service; CI runners under SLSA build-platform; operator's cloud config is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.24 | Incident management planning & preparation | AUDIT-READY | `SECURITY.md` intake/targets/flow; `.well-known/security.txt`. Org IR plan is an org act (`compliance/policies/` template) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.25 | Assessment & decision on security events | AUDIT-READY | `SECURITY.md` initial assessment; severity via GHSA/CVSS; org triage is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.26 | Response to information security incidents | AUDIT-READY | `SECURITY.md` coordinated-disclosure + fix-then-release; org response is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.27 | Learning from information security incidents | AUDIT-READY | GHSA advisories + beads remediation tracking; formal lessons-learned is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.28 | Collection of evidence | AUDIT-READY | Git history + signed release attestations + CI logs; forensic-grade handling is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.29 | Information security during disruption | N/A(op) | BC/DR of a running service is operator's | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.30 | ICT readiness for business continuity | N/A(op) | Operator-owned (service availability) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.31 | Legal/statutory/regulatory/contractual requirements | AUDIT-READY | `LICENSE` (MIT); EU CRA map (`compliance/cra/`); license gate (`deny.toml`). Org legal register is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.32 | Intellectual property rights | IMPL | `deny.toml` `[licenses]` gate; `LICENSE` (MIT); contributor IP terms in `CONTRIBUTING.md` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.33 | Protection of records | N/A(op) | Operator records retention | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.34 | Privacy & protection of PII | AUDIT-READY | sparq processes no PII of its own; loaded-RDF PII is operator-controller (`compliance/data-flow.md` + `dpia.md`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.35 | Independent review of information security | AUDIT-READY | OpenSSF Scorecard, CodeQL, adversarial threat model + ZK soundness audit, engineer↔auditor loop; accredited internal-audit programme is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.36 | Compliance with policies, rules & standards | PARTIAL | required `ci-fast` check fails merge on core-crate/SPARQL lanes only; other policy lanes post-merge/nightly; `docs/branch-protection.md` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.5.37 | Documented operating procedures | IMPL | `AGENTS.md`, `CONTRIBUTING.md`, `docs/branch-protection.md`, workflows as executable procedure | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

### A.6 — People controls (8)

| Ctrl | Title | sparq-side status | sparq evidence (`controls.md`) | Applicable? (org) | Justification (org) | Status in org ISMS (org) | Approved by / date (org) |
|---|---|---|---|---|---|---|---|
| A.6.1 | Screening | N/A(op) | Employment screening; OSS has contributors, not employees | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.2 | Terms & conditions of employment | AUDIT-READY | Contributor terms via `CONTRIBUTING.md` + `LICENSE`; no employment relationship | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.3 | Awareness, education & training | AUDIT-READY | `CONTRIBUTING.md` §Secure coding + `AGENTS.md` as contributor training-of-record; formal programme is an org act | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.4 | Disciplinary process | N/A(op) | No employer relationship; CoC/enforcement is the org's | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.5 | Responsibilities after termination/change | N/A(op) | No employees; access revocation of a deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.6 | Confidentiality / NDA | AUDIT-READY | `SECURITY.md` coordinated-disclosure norm; formal NDAs N/A for OSS | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.7 | Remote working | N/A(op) | Operator/org HR control | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.6.8 | Information security event reporting | IMPL | `SECURITY.md` (GHSA + email) + `.well-known/security.txt`; `CONTRIBUTING.md` redirects public reports to the private channel | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

### A.7 — Physical & environmental controls (14)

> sparq has **no premises, hardware, or physical estate** — every A.7 control is a property of
> the operator's data-centre / workstation environment. In the org's ISMS these are typically
> **applicable and operator-owned**; the org records its own physical controls. Listed
> individually so the org's SoA is complete (clause 6.1.3 requires *every* Annex A control).

| Ctrl | Title | sparq-side status | sparq evidence | Applicable? (org) | Justification (org) | Status in org ISMS (org) | Approved by / date (org) |
|---|---|---|---|---|---|---|---|
| A.7.1 | Physical security perimeters | N/A(op) | No premises; operator's data centre | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.2 | Physical entry | N/A(op) | Operator's facility | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.3 | Securing offices, rooms & facilities | N/A(op) | Operator's facility | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.4 | Physical security monitoring | N/A(op) | Operator's facility | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.5 | Protecting against physical & environmental threats | N/A(op) | Operator's facility | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.6 | Working in secure areas | N/A(op) | Operator's facility | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.7 | Clear desk & clear screen | N/A(op) | Operator/org workplace policy | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.8 | Equipment siting & protection | N/A(op) | Operator's hardware | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.9 | Security of assets off-premises | N/A(op) | Operator's hardware | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.10 | Storage media | N/A(op) | Operator's media | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.11 | Supporting utilities | N/A(op) | Operator's data centre | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.12 | Cabling security | N/A(op) | Operator's data centre | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.13 | Equipment maintenance | N/A(op) | Operator's hardware | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.7.14 | Secure disposal or re-use of equipment | N/A(op) | Operator's hardware | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

### A.8 — Technological controls (34)

| Ctrl | Title | sparq-side status | sparq evidence (`controls.md`) | Applicable? (org) | Justification (org) | Status in org ISMS (org) | Approved by / date (org) |
|---|---|---|---|---|---|---|---|
| A.8.1 | User endpoint devices | N/A(op) | Operator/org workstation control | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.2 | Privileged access rights | N/A(op) → AUDIT-READY | Branch-protection disallows direct push incl. admins; privileged access to a deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.3 | Information access restriction | N/A(op) | Restriction within a deployed instance is operator-owned (B3 no-auth) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.4 | Access to source code | IMPL | Branch protection on `main` (PR + review + `ci-fast` check); `CODEOWNERS`; SHA-pinned actions; SLSA-attested releases + SHA256SUMS | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.5 | Secure authentication | N/A(op) → AUDIT-READY | Optional bearer token; full secure-auth (MFA/session) is operator's gateway (B3); repo-side GitHub auth + signed attestations | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.6 | Capacity management | N/A(op) → IMPL(partial) | `QueryBudget` DoS-limit primitive (`sparq-engine`, T-DoS); capacity of a running deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.7 | Protection against malware | IMPL | CodeQL SAST; supply-chain advisory gating + daily watchdog; SHA-pinned actions; distroless non-root image | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.8 | Management of technical vulnerabilities | IMPL | `cargo deny check advisories` GATING; daily watchdog; Dependabot; CodeQL; `SECURITY.md` + `.well-known/security.txt`; beads | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.9 | Configuration management | IMPL | Pinned toolchain/MSRV, SHA-pinned actions, pinned base image digest, `Cargo.lock`, `deny.toml` policy-as-code | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.10 | Information deletion | N/A(op) | Deletion of operator data is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.11 | Data masking | N/A(op) → cross-ref | sparq does not mask loaded RDF; ZK privacy story (`compliance/cryptoreview/`) flagged **not yet sound** — not claimed | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.12 | Data leakage prevention | AUDIT-READY | Error/log no-leak standard (`CONTRIBUTING.md` §Input validation); error-body sanitization shipped (PR #241); verification in `compliance/asvs/` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.13 | Information backup | N/A(op) | Backup of operator data is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.14 | Redundancy of information processing facilities | N/A(op) | Availability/redundancy of a running deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.15 | Logging | AUDIT-READY | Logging present + hygiene standard (`CONTRIBUTING.md`); structured-log/retention policy of a deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.16 | Monitoring activities | IMPL(repo) / N/A(op) | Repo-side CI monitors every push + Scorecard + daily watchdog; runtime monitoring of a deployment is operator-owned | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.17 | Clock synchronization | N/A(op) | Operator infrastructure control | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.18 | Use of privileged utility programs | IMPL | Distroless `nonroot` image — no shell / no package manager / no privileged utilities (`Dockerfile`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.19 | Installation of software on operational systems | IMPL | Reproducible pinned build (base digest, `Cargo.lock`, SHA-pinned actions, SLSA-attested); CIS-Docker hardening (`compliance/cis/`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.20 | Networks security | N/A(op) | Network controls are the operator's deployment env | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.21 | Security of network services | N/A(op) | Operator-owned (the gateway fronting sparq-server) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.22 | Segregation of networks | N/A(op) | Operator deployment topology | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.23 | Web filtering | N/A(op) | Operator network control | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.24 | **Use of cryptography** | AUDIT-READY (scoped) | **Operationally relied-on crypto ONLY:** release artifacts Sigstore/SLSA-attested (`release.yml`); TLS is operator's gateway. **EXPLICITLY EXCLUDED: the `sparq-zk`/`sparq-zk-compose`/`sparq-mpc` estate makes NO production guarantee** — v1 ZK verifier **originally found NOT sound** (`research/zk-soundness-audit.md`), but `sq-1s2` landed the binding layer + an **internal** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed → "sound as landed for the assumed threat model"; **external sign-off STILL PENDING** (`sq-qhy4`, P0), **NO production guarantee** [OPUS-4.8] (`SECURITY.md`; `compliance/cryptoreview/`). Crypto-key-mgmt policy is an org act <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> | `<FILL-IN>` | `<FILL-IN: org MUST NOT cite the ZK/MPC estate as a control>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.25 | Secure development life cycle | IMPL | `CONTRIBUTING.md` §Secure coding + SDLC touchpoints; CI gate stack; threat model. `compliance/ssdf/` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.26 | Application security requirements | IMPL | `research/threat-model.md` (boundaries B1–B5, STRIDE); `SECURITY.md`; ASVS re-scope (`compliance/asvs/`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.27 | Secure system architecture & engineering principles | IMPL | `#![forbid(unsafe_code)]` in 31/36 crates; boundary threat model; defense-in-depth gate stack; `compliance/memsafety/unsafe-register.md` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.28 | Secure coding | IMPL | `CONTRIBUTING.md` §Secure coding; `clippy … -D warnings` GATING; CodeQL; unsafe-count ratchet (`scripts/unsafe-gate.py`) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.29 | Security testing in development & acceptance | IMPL | `cargo test --workspace`; W3C conformance ratchets; `cargo-fuzz`; Miri lane; mmap corruption oracle | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.30 | Outsourced development | N/A | No outsourced development | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.31 | Separation of dev/test/production environments | AUDIT-READY | Branch model separates in-progress from released; release artifacts built only on tags; operator owns prod env separation | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.32 | Change management | IMPL | PR + review + `ci-fast` check + conformance "never-lower" ratchets; `CHANGELOG.md`; beads | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.33 | Test information | N/A(op) | Use of operator production data in test is operator-owned; sparq tests use W3C/synthetic fixtures | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| A.8.34 | Protection of information systems during audit testing | N/A(op) | Operator-owned (auditing a running system) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

> **Maintaining this table.** The pre-filled `sparq-side status` + `sparq evidence` columns are
> a **snapshot of [`controls.md`](./controls.md)**. If `controls.md` changes (new evidence, a
> status correction), re-sync these two columns from it — they are the only source of truth for
> the sparq-side facts; this table must not drift from it. The org-owned columns persist across
> re-syncs.

---

## ISMS clauses 4–10 — companion templates (org must additionally produce)

These are the management-system clauses that **no repo artifact can be**. This template set
now supplies an adoptable scaffold for each; the org completes and signs them.

| Clause | Artifact the org must produce | Template in this directory | sparq input the org can reuse |
|---|---|---|---|
| **4** Context | ISMS scope statement; issues; interested parties | [`isms-scope-template.md`](./isms-scope-template.md) | This mapping + `research/threat-model.md` scope; the B3 + ZK/MPC interfaces |
| **5** Leadership | Signed information-security policy; roles/responsibilities | (org policy — seed from `compliance/policies/` cross-framework templates) | `SECURITY.md`, `CODEOWNERS`, `docs/branch-protection.md` |
| **6** Planning | Risk assessment + risk-treatment plan; **SoA** (this file); security objectives | [`risk-methodology-template.md`](./risk-methodology-template.md) + this SoA | `research/threat-model.md` seeds the risk register; `controls.md` seeds the SoA |
| **7** Support | Resources, competence, awareness, documented-info control | (org — seed from `CONTRIBUTING.md` + `AGENTS.md`) | `CONTRIBUTING.md` + `AGENTS.md` as the awareness baseline |
| **8** Operation | Operational planning + risk-treatment implementation | [`risk-methodology-template.md`](./risk-methodology-template.md) §C/§A.4 | The CI gate stack is the operational control evidence |
| **9** Performance evaluation | Monitoring/measurement; **internal audit**; **management review** | [`internal-audit-programme-template.md`](./internal-audit-programme-template.md) + [`management-review-template.md`](./management-review-template.md) | CI dashboards, Scorecard, advisory watchdog as monitoring inputs |
| **10** Improvement | Nonconformity + corrective action; continual improvement | [`management-review-template.md`](./management-review-template.md) §10 | GHSA advisories, beads tracker, conformance ratchets as the improvement loop |

## Companion policy templates (cross-framework)

Cross-framework policy templates an org would adopt (vulnerability-management / CRA disclosure,
secure-SDLC, dependency, release-signing) are intended to live under the shared
`compliance/policies/` directory and are owned across the `cra` / `ssdf` / `sbom` / `slsa`
framework worktrees (to avoid duplication). This ISO 27001 template set **references** them; it
does not duplicate them. Where they are not yet present, that is part of the respective
framework's remediation, not a separate ISO 27001 gap.

## Honesty footer

Populating this template set does **not** make sparq, or the adopting organization, "ISO 27001
certified." Certification is an **accredited certification body's** Stage 1 + Stage 2 audit of
an **operating ISMS over time**. This is the head start (the sparq-component half of the SoA +
adoptable scaffolds for clauses 4–10); the certificate is external by definition. The ZK/MPC
estate is **never** a cryptographic control in any SoA completion.
