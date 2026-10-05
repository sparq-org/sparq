<!-- [OPUS-4.8] sq-ez5z — ISO/IEC 27001:2022 clause 9.3 management-review + clause 10
     improvement TEMPLATE. Org-adoptable scaffold; NOT a certificate, NOT a signed review
     record. Remediation of gap GAP-ISO-1. Re-review when Fable returns. -->

# ISO/IEC 27001:2022 — management-review + improvement template (clauses 9.3 & 10)

> **THIS IS AN ADOPTABLE TEMPLATE, NOT A CERTIFICATION CLAIM.** A management review is an act
> of the **adopting organization's top management**; no repo artifact can be one. Every
> decision/output cell is a `<FILL-IN>` placeholder for the org to complete and sign.
> Populating it does **not** make sparq or the adopting organization "ISO 27001 certified."
> Remediates part of gap **GAP-ISO-1**. Read [`README.md`](./README.md) first.

## What this template is for

ISO/IEC 27001:2022 **clause 9.3** requires **top management** to review the ISMS at planned
intervals to ensure its continuing suitability, adequacy, and effectiveness. The review has a
mandated set of **inputs** (9.3.2) and must produce **outputs** (9.3.3) covering continual
improvement and any need to change the ISMS. **Clause 10** then requires acting on
nonconformities (10.1) and continually improving (10.2). This template provides the agenda,
the input checklist, and the decision-capture schema. The org **must** hold the actual
meeting and retain the minutes as documented information.

## How sparq feeds the review (inputs sparq can supply)

sparq cannot hold a management review, but several mandated inputs have a **standing data
source** in the repo/CI that the org can pull into the meeting pack:

- **CI / monitoring results** — `ci-fast.yml` required check, `ci.yml` nightly, `scorecard.yml`
  (OpenSSF Scorecard), the daily advisory watchdog (`dependency-monitoring.yml`).
- **Vulnerability / nonconformity feed** — RustSec/GHSA advisories, Dependabot PRs, the beads
  tracker (corrective-action items), `SECURITY.md` disclosure intake.
- **Audit-result substrate** — the conformance ratchets and the engineer↔auditor compliance
  loop (`compliance/`), the internal-audit programme
  ([`internal-audit-programme-template.md`](./internal-audit-programme-template.md)).

These are **inputs**, not the review itself. The org's management makes the judgements.

---

## Management-review record (org completes)

**Review date:** `<FILL-IN>`  **Period covered:** `<FILL-IN>`  **Chair (top management):**
`<FILL-IN>`  **Attendees:** `<FILL-IN>`

### Inputs reviewed (clause 9.3.2)

| # | Required input | sparq data source the org can pull | Summary discussed (org) |
|---|---|---|---|
| a | Status of actions from previous reviews | Prior minutes + beads | `<FILL-IN>` |
| b | Changes in external/internal issues relevant to the ISMS | `isms-scope-template.md` §4.1; new sparq major versions; new regulation | `<FILL-IN>` |
| c | Changes in interested-party needs/expectations | `isms-scope-template.md` §4.2 | `<FILL-IN>` |
| d | Feedback on ISMS performance: nonconformities & corrective actions | Beads tracker; GHSA advisories | `<FILL-IN>` |
| e | …monitoring & measurement results | `ci-fast.yml`, nightly `ci.yml`, Scorecard, conformance ratchets, advisory watchdog | `<FILL-IN>` |
| f | …audit results | Internal-audit programme; compliance engineer↔auditor findings | `<FILL-IN>` |
| g | …fulfilment of security objectives | `risk-methodology-template.md` §D objectives | `<FILL-IN>` |
| h | Feedback from interested parties | `<FILL-IN: customer/user/regulator feedback>` | `<FILL-IN>` |
| i | Results of risk assessment & status of risk-treatment plan | `risk-methodology-template.md` register + plan | `<FILL-IN>` |
| j | Opportunities for continual improvement | All of the above | `<FILL-IN>` |

### Outputs / decisions (clause 9.3.3)

| # | Decision area | Decision (org) | Owner | Due |
|---|---|---|---|---|
| 1 | Opportunities for continual improvement | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| 2 | Any need for changes to the ISMS (scope, policy, controls, objectives) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| 3 | Resource needs | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

**Top-management sign-off:** `<FILL-IN: name / role / date / signature>`

---

## Clause 10 — improvement (nonconformity & corrective action)

### 10.1 Nonconformity & corrective-action log

When a nonconformity occurs (a failed control, a missed objective, an audit finding, a
security incident), the org records it and the corrective action. sparq's **beads tracker**
and **GHSA advisory** flow are the natural mechanism for component-level corrective actions;
the org maps them into this log.

| NC ID | Source (audit / incident / review / advisory) | Description | Immediate correction | Root-cause analysis | Corrective action | Owner | Effectiveness check | Status |
|---|---|---|---|---|---|---|---|---|
| `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |

> **Worked example (illustrative, not a live org record).** The repo's own ZK-soundness audit
> (`research/zk-soundness-audit.md`) **originally found the v1 ZK verifier NOT sound** — a
> textbook nonconformity *if* an org had claimed it as a control. The honest disposition was
> **do not claim the control** (recorded in `controls.md` A.8.24, `SECURITY.md`), with
> remediation tracked as beads: `sq-1s2` then **landed the verifier-side binding layer**, and an
> **internal** post-remediation re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all
> prior findings closed → "sound as landed for the assumed threat model." Because that re-audit
> is **internal, single-model, read-only**, an **external accredited-cryptographer audit remains
> the required corrective action** (consolidated register `sq-qhy4`, P0) before any production ZK
> claim, and the estate still carries **NO production guarantee**. This shows the loop working:
> detect → disclose → do not overclaim → remediate → re-audit → still require external sign-off.
> <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

### 10.2 Continual improvement

The org records how it continually improves the ISMS's suitability, adequacy, and
effectiveness. sparq's standing improvement loop the org can reference: the **conformance
ratchets** (never-lowered floors), the **engineer↔auditor compliance loop**, **OpenSSF
Scorecard** trend, and the **daily advisory watchdog**. The org's own continual-improvement
metrics: `<FILL-IN>`.

## Honesty footer

This record schema does **not** constitute a management review — only the org's top management
holding the meeting and retaining the minutes does. sparq supplies the *input data sources*;
the **organization** owns the judgements, the decisions, and the sign-off. **No artifact in
this repository asserts that sparq is ISO 27001 certified.**
