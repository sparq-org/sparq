<!-- [OPUS-4.8] EU Cyber Resilience Act (CRA) framework intro + scope. Bead sq-toze.18 (epic sq-toze). -->
# EU Cyber Resilience Act (CRA) — sparq readiness

> **Status legend:** *implemented & verified* (a technical control in the codebase/CI with
> passing evidence) · *audit-ready* (control + documentation in place, but the formal act —
> CE marking, conformity assessment, EU declaration of conformity — needs the **manufacturer's
> organizational sign-off**, not something an agent or the source tree can substitute) ·
> *gap* (not met; tracked in [`gap-register.md`](./gap-register.md) + a `bd` bead) ·
> *N/A* (not applicable to a Rust library/server distributed as a dependency).

## What the CRA is

Regulation (EU) 2024/2847 — the **Cyber Resilience Act** — sets horizontal cybersecurity
requirements for **products with digital elements (PDEs)** made available on the EU market. It
binds the **manufacturer** (the party that places the product on the market under its own
name) to:

- the **essential cybersecurity requirements** of **Annex I, Part I** (secure-by-design /
  secure-by-default properties of the product), and
- the **vulnerability-handling requirements** of **Annex I, Part II** (a process: SBOM,
  coordinated disclosure, timely security updates, a support period), plus
- **information and instructions to the user** (**Annex II**), and
- conformity assessment + the **EU declaration of conformity** + **CE marking** (the formal
  acts — Articles 13, 28, and Annexes IV/V).

Key dates: the Regulation entered into force **2024-12-10**; the **vulnerability-handling and
reporting obligations** (Article 14 — reporting actively exploited vulnerabilities and severe
incidents to ENISA/CSIRTs) apply from **2026-09-11**; the **main body of obligations** applies
from **2027-12-11**.

## How sparq maps onto the CRA — and the load-bearing nuances

sparq is a **Rust RDF/SPARQL data-engine library** (`sparq-core`, `sparq-engine`, …) plus an
**HTTP server** (`sparq-server`), a **WASM port**, and a **ZK/MPC crypto estate**, distributed
on crates.io / npm / PyPI / ghcr.io. That shape drives four scoping decisions:

1. **CRA applies to the act of placing on the market, not to the source tree.** sparq's
   repository can hold *evidence* that the essential requirements (Annex I Part I) and the
   process requirements (Annex I Part II) are met — the SBOM, the advisory gate, the
   coordinated-disclosure channel, signed provenance. It **cannot** produce the conformity
   assessment, the EU declaration of conformity, or the CE marking; those are **the
   manufacturer's organizational acts**. Every such item is labelled *audit-ready*, never
   *implemented & verified*, and this document **does not claim CE conformity**.

2. **Open-source-steward nuance.** The CRA distinguishes a commercial **manufacturer** from an
   **open-source software steward** (Article 24 + Recitals 18–19): a person/entity that
   supports the development of *free and open-source software intended for commercial
   activities* but **does not monetise it**. A steward carries a **lighter-touch** regime —
   principally a **duty to put in place and document a cybersecurity policy**, to **cooperate
   with market-surveillance authorities**, and to **report actively-exploited vulnerabilities
   and severe incidents** — rather than the full manufacturer conformity-assessment +
   CE-marking burden. sparq today is a **non-monetised research/open-source project** (MIT,
   pre-1.0, "experimental research RDF triplestore" — see [`SECURITY.md`](../../SECURITY.md)),
   so the **steward** path is the realistic frame. **However**, the moment a party
   *commercialises* sparq — sells it, bundles it into a paid product, or offers it as a paid
   service in the EU — **that party becomes the manufacturer for their offering** and inherits
   the full obligation. This document is written so it serves **either** reading: the technical
   evidence (Annex I) is the same; only the conformity-assessment/CE-marking layer differs, and
   that layer is explicitly the deploying/commercialising party's responsibility.

3. **The "no authentication" boundary is the operator's, by documented design.** `sparq-server`
   ships with **optional** Bearer-token auth and a **fail-closed non-loopback bind** (it refuses
   to listen on a non-loopback address unless the operator explicitly opts in *or* authenticates
   the surface), but it has **no per-user authz / session management** — threat-model boundary
   **B3** ("front with a gateway / sparq-solid"). For CRA Annex I (2)(d) "protect from
   unauthorised access" this is an **architectural decision + operator-responsibility split**,
   documented and surfaced at runtime (a loud no-auth startup warning), not a silent gap.

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
4. **The ZK/MPC estate provides NO production security guarantee today and is excluded from
   every CRA security claim.** [OPUS-4.8] The v1 ZK verifier was **originally found unsound**
   ([`research/zk-soundness-audit.md`](../../research/zk-soundness-audit.md), kept on record),
   but the `sq-1s2` remediation **landed the verifier-side binding layer** and an **internal
   post-remediation re-audit** ([`research/zk-verifier-reaudit.md`](../../research/zk-verifier-reaudit.md),
   bead `sq-gbp4`) found all prior findings closed — verdict **"sound as landed for the assumed
   threat model"**. That verdict is **internal / single-model self-review only**: an **external
   accredited-cryptographer sign-off is still PENDING** (`sq-qhy4`, P0, required before any
   production ZK claim), forge tests are `#[ignore]`d out of default CI, and there is **NO
   production soundness/privacy/integrity guarantee** — it remains a research scaffold, and
   `sparq-mpc` is semi-honest-only with no guarantee (see [`SECURITY.md`](../../SECURITY.md)
   §"research scaffolds with NO security guarantee"). No CRA Annex I "confidentiality/integrity"
   claim in this mapping rests on the ZK/MPC crates; to do so would launder a research scaffold
   into a regulatory assurance and is an explicit honesty-contract violation.

## Scope — what is in / out for a library + server

| In scope (sparq's responsibility) | Out of scope (operator / deploying-party responsibility) |
|---|---|
| Secure-by-default config of the binary (`sparq-server` defaults, bind posture, DoS limits) | The *deployment* environment (TLS termination, network firewalling, the reverse-proxy gateway, OS patching) |
| The vulnerability-handling **process** (advisory gate, coordinated disclosure, SBOM, security updates via releases) | Authentication of *end users* and per-user authorization (boundary B3 → gateway / sparq-solid) |
| Supply-chain integrity of what sparq ships (cargo-deny, cargo-vet, SLSA provenance, signed releases) | The **conformity assessment**, **EU declaration of conformity**, and **CE marking** (a manufacturer organizational act) |
| Information & instructions to the user (Annex II): SECURITY.md, READMEs, the support/EOL statement | Article 14 **reporting to ENISA/CSIRTs** of actively-exploited vulns (an organizational/legal duty of whoever is the manufacturer/steward of record) |
| The honest "no known exploitable vulnerability at ship" claim, resting on the real (post-merge/nightly) advisory check | The risk acceptance for *whatever RDF data the operator loads* (sparq is a data engine; the operator is the data controller) |

## Deliverables in this folder

- [`controls.md`](./controls.md) — the Annex I (Part I + Part II) and Annex II requirement →
  status → evidence → owner table. **This is the spine the auditor checks.**
- [`evidence.md`](./evidence.md) — the consolidated evidence pack (file paths, CI jobs, scans)
  each control row points at, with the verification commands.
- [`gap-register.md`](./gap-register.md) — open gaps, severity, remediation, target, and the
  `bd` bead under epic `sq-toze` that tracks each.
- [`incident-reporting-runbook.md`](./incident-reporting-runbook.md) — an **adoptable operator
  runbook** for the **Article 14** ENISA/CSIRT reporting obligation (24h early warning / 72h
  notification / final report), with `<FILL-IN>` org placeholders. Addresses CRA-CA.5 / former
  gap GX-CRA-2; **not** a certification claim — the reporting act is the manufacturer/steward's.
- [`support-policy.md`](./support-policy.md) — a **proposed** support-period & end-of-life
  (EOL) policy (Annex II A.6 / Annex I Part II.8): a concrete support period (5 years from each
  release — the CRA minimum), the security-update channel, and the EOL notification process,
  tied to the per-release SBOM/VEX + the Article 14 runbook. Addresses former gap GX-CRA-1 as
  **audit-ready documentation pending maintainer ratification** — a PROPOSED policy, **not** a
  binding maintainer commitment (policy §7); until ratified, `SECURITY.md` is authoritative.
- [`../policies/policy-cybersecurity.md`](../policies/policy-cybersecurity.md) — the single named
  **cybersecurity-policy template** for **Art. 24** (open-source-software steward) / **Art. 13**
  (manufacturer risk assessment), consolidating the risk basis (threat-model B1–B5), the
  secure-SDLC, the dependency/supply-chain policy, coordinated disclosure + vuln-handling,
  release-signing/provenance, and the support/EOL posture into one adoptable artifact. Addresses
  former gap GX-CRA-3 (CRA-CA.1/CRA-CA.6) as **audit-ready documentation pending org sign-off**
  (policy §13); cross-references — does not fork — the live sources + the sibling SSDF/SBOM/SLSA/ISO
  templates. Lives under the shared `compliance/policies/` directory (not duplicated here).

## Honest one-line posture

sparq **already satisfies the substance of the CRA Annex I vulnerability-handling process and
most of the secure-by-default essential requirements** — coordinated disclosure
(`SECURITY.md` + RFC 9116 `security.txt`), an SBOM (per-release CycloneDX + VEX), a fail-closed
post-merge/nightly advisory check (cargo-deny advisories un-degraded, `supply-chain.yml`; not a PR gate), signed
provenance on releases, and documented secure-by-default server limits. The remaining work is
**(a)** a small set of release-completeness gaps (SLSA provenance on the `dist.yml` lane and
published-package provenance for crates.io/npm/PyPI; a container-image vuln scan), and
**(b)** the **audit-ready** organizational layer the CRA reserves to the manufacturer/steward
(the documented cybersecurity policy, the formal EU declaration of conformity + CE marking, and
the Article 14 reporting *act*) — which an agent cannot and must not self-certify. The Article 14
reporting **workflow** is now documented as an adoptable runbook
([`incident-reporting-runbook.md`](./incident-reporting-runbook.md)); the **act** of reporting to
ENISA/CSIRT remains the organizational duty.
