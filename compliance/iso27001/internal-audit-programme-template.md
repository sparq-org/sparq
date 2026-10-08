<!-- [OPUS-4.8] sq-ez5z — ISO/IEC 27001:2022 clause 9.1/9.2 internal-audit programme
     TEMPLATE. Org-adoptable scaffold; NOT a certificate, NOT an accredited audit result.
     Remediation of gap GAP-ISO-1. Re-review when Fable returns. -->

# ISO/IEC 27001:2022 — internal-audit programme template (clauses 9.1 & 9.2)

> **THIS IS AN ADOPTABLE TEMPLATE, NOT A CERTIFICATION CLAIM.** An internal audit is an act of
> the **adopting organization**, carried out by auditors who are objective and impartial
> w.r.t. the work audited. No repo artifact is an internal-audit result, and **none of this is
> the external accredited audit** that issues an ISO 27001 certificate. Every result/finding
> cell is a `<FILL-IN>` placeholder. Remediates part of gap **GAP-ISO-1**. Read
> [`README.md`](./README.md), [`controls.md`](./controls.md), and
> [`soa-template.md`](./soa-template.md) first.

## What this template is for

ISO/IEC 27001:2022:

- **9.1 Monitoring, measurement, analysis & evaluation** — the org determines what to monitor,
  the methods, when, and who evaluates results, to assess ISMS performance and effectiveness.
- **9.2 Internal audit** — the org conducts internal audits at **planned intervals** to
  determine whether the ISMS conforms to (a) the org's own requirements and (b) ISO 27001's
  requirements, and is **effectively implemented and maintained**. It must:
  **9.2.2** plan, establish, implement and maintain an audit **programme** (frequency, methods,
  responsibilities, planning, reporting), considering importance and prior results; define
  **criteria and scope** for each audit; select auditors ensuring **objectivity and
  impartiality**; ensure results are reported to relevant management; and retain documented
  information as evidence of the programme and results.

This template provides the programme schedule, the per-audit plan, the criteria mapping, and
the finding-capture schema. The org **must** run the audits with impartial auditors and retain
the records.

## The independence / impartiality rule (load-bearing)

Clause 9.2 requires auditor **objectivity and impartiality** — an auditor must not audit their
own work. **This is exactly why sparq's repo cannot supply the internal audit:** the
maintainers' self-checks (CI, the engineer↔auditor compliance loop in `compliance/`) are
valuable **monitoring inputs (9.1)** but are **not** an impartial internal audit (9.2) for an
adopting organization. The org assigns auditors independent of the audited function. Where the
audited control is a **sparq component control**, the org audits *its use and configuration*
of the component, citing sparq's evidence (`controls.md` / `evidence.md`) as the substrate.

---

## Part A — Monitoring & measurement plan (clause 9.1)

| What to monitor | Method / source | Frequency | Evaluated by (org) | Acceptance criterion (org) |
|---|---|---|---|---|
| CI gate health (build/test/lint/conformance) | `ci-fast.yml` required check (+ nightly `ci.yml`) | Per push / continuous | `<FILL-IN>` | `<FILL-IN: green required to merge>` |
| Dependency advisories | `dependency-monitoring.yml` (daily) + Dependabot | Daily | `<FILL-IN>` | `<FILL-IN: no critical > N days>` |
| Supply-chain posture | OpenSSF `scorecard.yml` (published) | Per schedule | `<FILL-IN>` | `<FILL-IN: score floor>` |
| Conformance level | W3C SPARQL/SHACL/inference ratchets | Per PR | `<FILL-IN>` | Never-lowered floor |
| Security objectives | `risk-methodology-template.md` §D | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| `<FILL-IN: org deployment metrics — gateway authn failures, rate-limit hits, backup success>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

## Part B — Internal-audit programme schedule (clause 9.2.2)

> Plan audits over a cycle (commonly annual, all controls within a 1–3 year window) weighted
> by importance and prior results. Higher-risk areas (the B3 no-auth fronting, key management,
> supply chain) audited more often.

| Audit ID | Scope (ISMS area / Annex A clusters) | Criteria | Planned date | Frequency | Auditor (impartial — org assigns) | Status |
|---|---|---|---|---|---|---|
| IA-1 | Secure development & change management (A.8.25–A.8.32) | ISO 27001 + org SDLC policy; sparq evidence `controls.md` A.8.x | `<FILL-IN>` | Annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-2 | Supply chain & vulnerability mgmt (A.5.19–A.5.22, A.8.8) | ISO 27001 + org dependency policy | `<FILL-IN>` | Semi-annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-3 | Access control & the B3 fronting gateway (A.5.15–A.5.18, A.8.2–A.8.5) | ISO 27001 + org access policy; **operator-owned** controls | `<FILL-IN>` | Annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-4 | Cryptography use (A.8.24) — incl. the **ZK/MPC exclusion** | ISO 27001 + org crypto policy; confirm **no reliance** on the NOT-sound estate | `<FILL-IN>` | Annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-5 | Incident management & disclosure (A.5.24–A.5.27, A.6.8) | ISO 27001 + org IR plan; `SECURITY.md` substrate | `<FILL-IN>` | Annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-6 | ISMS management clauses (4–10) | ISO 27001 clauses 4–10; this template set | `<FILL-IN>` | Annual | `<FILL-IN>` | `<FILL-IN>` |
| IA-n | `<FILL-IN: org-specific scope>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

## Part C — Per-audit plan (one per audit, org completes)

**Audit ID:** `<FILL-IN>`  **Date:** `<FILL-IN>`  **Lead auditor (impartial):** `<FILL-IN>`
**Auditee(s):** `<FILL-IN>`

**Objective / scope / criteria:** `<FILL-IN>`

**Sampling & method:** `<FILL-IN: interviews, evidence review of controls.md/evidence.md,
config inspection, test re-runs>`

### Findings log

| Finding ID | Clause / Annex A ref | Conformity (C / Minor NC / Major NC / OFI) | Evidence examined | Description | Action ref (→ `management-review-template.md` §10.1) |
|---|---|---|---|---|---|
| `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: controls.md row, CI log, config>` | `<FILL-IN>` | `<FILL-IN>` |

**Audit conclusion & report to management (clause 9.2.2 f):** `<FILL-IN>`

> Nonconformities raised here flow into the **corrective-action log** in
> [`management-review-template.md`](./management-review-template.md) §10.1, and audit results
> are an **input** to the management review (§9.3.2 f).

## Honesty footer

Running this programme is the org's **internal** audit (clause 9.2). It is **not** the
external accredited certification audit, and no completion of it makes sparq "ISO 27001
certified." sparq supplies the *evidence substrate* (`controls.md`, `evidence.md`, the CI
lanes) that an internal auditor examines; the **organization** provides the impartial
auditors, the schedule, the findings, and the records. **No artifact in this repository
asserts that sparq is ISO 27001 certified.**
