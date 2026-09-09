<!-- [OPUS-4.8] EU CRA support-period & end-of-life policy (template/proposal). Bead sq-f8tv
     (gap GX-CRA-1, epic sq-toze). PROPOSED policy — NOT a binding maintainer commitment.
     NON-CANONICAL timing; org-specific details are <FILL-IN> placeholders. -->
# sparq — support period & end-of-life policy (PROPOSED, pending maintainer ratification)

> **What this is.** A **proposed** support-period and end-of-life (EOL) policy for `sparq`,
> drafted to satisfy the EU **Cyber Resilience Act** (Regulation (EU) 2024/2847) Annex II A.6
> ("the period during which security support is provided") and the Annex I Part II.8
> dissemination duty, and to close certification gap **GX-CRA-1**.
>
> **What this is NOT — the ratification caveat.** This document is **audit-ready
> documentation pending maintainer ratification**. It is **not** a binding legal commitment,
> a contractual SLA, a warranty, or a representation of CRA conformity / CE marking. The
> concrete support period below is a **proposal for the maintainer (or the
> manufacturer/open-source-software steward of record) to ratify**, amend, or reject. Until
> a maintainer explicitly adopts it (see §7 "Ratification"), the **authoritative, in-effect
> statement remains the informal posture in [`../../SECURITY.md`](../../SECURITY.md)
> §"Supported versions"** ("fixes land on `main`, ship in the next release; no LTS"). This
> policy proposes to *replace that informal posture with a concrete one* — it does not
> retroactively bind the project. Every party-specific detail is a **`<FILL-IN>`**
> placeholder the adopting entity must complete and have reviewed by its own
> legal/regulatory function.

## 1. Why a concrete support period is required

The CRA obliges the manufacturer of a product with digital elements to **determine and state
a support period** — the period during which it provides security updates (vulnerability
handling). Key parameters from the Regulation (non-canonical paraphrase; confirm against the
published text):

- The support period must reflect the **expected product lifetime**, and **shall be at least
  five (5) years** unless the product is reasonably expected to be in use for a shorter
  period, in which case the support period matches that shorter expected lifetime.
- The support period and the EOL date must be **communicated to users** in a clear,
  understandable way (Annex II A.6 — "information and instructions to the user").
- During the support period, security updates must be **disseminated without delay and free
  of charge** (Annex I Part II.8), with advisory messages.

`sparq` today states support **informally** in `SECURITY.md` ("next release", "no LTS") with
**no concrete support-period or EOL date** — recorded as gap **GX-CRA-1** in
[`gap-register.md`](./gap-register.md). This document proposes the concrete statement.

## 2. The proposed support period

> **PROPOSED — pending maintainer ratification (§7).**

| Parameter | Proposed value | Rationale |
|---|---|---|
| **Support period (per release)** | **5 years from the publication date of each release**, the CRA minimum | `sparq` is pre-1.0, experimental research software with no committed product lifetime; the CRA minimum is the conservative, defensible floor. A maintainer may shorten this only by stating a shorter *expected lifetime* (and must justify it), or lengthen it. |
| **What "supported" means** | Security fixes for in-scope vulnerabilities (see [`../../SECURITY.md`](../../SECURITY.md) §scope) are produced and shipped in a new release of the affected published artifact during the support window | Matches the existing "fix on `main` → next release" flow; does **not** promise feature back-ports or non-security maintenance |
| **Supported version line** | The **latest published release** of each affected artifact (the `sparq-*` crates on crates.io, `@sparq-org/sparq` on npm, `sparq` on PyPI, the `ghcr.io` image). Older releases are supported **only** through the "upgrade to the fixed release" path | `sparq` does **not** maintain parallel LTS branches; the security fix is delivered as a forward upgrade. This is a deliberate, stated limitation, not an omission |
| **Free of charge** | All security updates are free (MIT licence, public registries) | Satisfies Annex I Part II.8 "free of charge" |
| **Support period start clock** | The publication timestamp recorded in the per-release SBOM (`scripts/gen-sbom-vex.sh`, `release.yml#sbom`) is the authoritative start of that release's 5-year clock | Ties the support clock to an existing, machine-readable, attested artifact rather than a hand-maintained date |

> **Honest caveat on the pre-1.0 status.** `sparq` is pre-1.0 and explicitly "experimental;
> the API is unstable" (`AGENTS.md`, `SECURITY.md`). A 5-year security-support commitment is
> the CRA *floor for a product placed on the market*. While the project remains a
> non-monetised research effort, the realistic frame is the **open-source-software steward**
> route (see [`README.md`](./README.md) §"Open-source-steward nuance"), under which the
> obligation is lighter-touch. The 5-year figure is proposed so that **whoever
> commercialises sparq inherits a ready, conformant support statement** rather than a void —
> but it is the maintainer's / commercialising party's call to ratify, and the period should
> be revisited at the 1.0 milestone.

## 3. Security-update channel (how a supported fix is delivered)

This restates the existing, verified delivery mechanism (controls.md II.2 / II.7 / II.8) so
the policy is self-contained:

1. **Where fixes land.** Security fixes are developed on `main` and released as a new version
   of each affected published artifact (crates.io / npm / PyPI / `ghcr.io`).
2. **Integrity of the update.** Each release carries `SHA256SUMS` over every archive + SBOM +
   VEX, and a **SLSA build-provenance attestation** (`actions/attest-build-provenance`) on the
   archives/SBOM and the container image (`.github/workflows/release.yml`); actions are
   SHA-pinned; `cargo-auditable` embeds the dependency manifest in the binary. Consumers can
   verify provenance before applying an update. *(Distribution-completeness raises GX-9/GX-10/
   GX-12 — see [`gap-register.md`](./gap-register.md).)*
3. **Advisory message.** Each fixed vulnerability is published via a **GitHub Security
   Advisory (GHSA)** with description, impact, and remediation (controls.md II.4), recorded in
   `CHANGELOG.md`, and linked from the release notes. This is the "advisory message"
   Annex I Part II.8 requires.
4. **Automated surfacing for consumers.** Dependabot opens ungrouped per-advisory security
   PRs across the four ecosystems (`.github/dependabot.yml`); the daily advisory watchdog
   (`.github/workflows/dependency-monitoring.yml`) is defence-in-depth.
5. **Discovery of the SBOM.** The per-release CycloneDX SBOM (where the support clock starts,
   §2) is attached to each GitHub Release and checked in alongside the VEX
   (`supply-chain/vex.cdx.json`). This is the Annex II A.6 "where the SBOM can be obtained"
   answer.

## 4. End-of-life (EOL) policy and notification process

> **PROPOSED — pending maintainer ratification (§7).**

### 4.1 What EOL means

When a release reaches the end of its support period (§2), it becomes **end-of-life**: no
further security updates will be produced for that release, and consumers must upgrade to a
still-supported release to remain covered. EOL is a property of a **release**, not of the
project; the project itself reaches EOL only if the maintainer announces project-wide
discontinuation (§4.3).

### 4.2 EOL notification process

| Step | Action | Channel | Lead time (proposed) |
|---|---|---|---|
| 1. Advance notice | Announce the upcoming EOL date of a release line | `CHANGELOG.md` entry + a pinned GitHub Release note / repository discussion / `SECURITY.md` "Supported versions" table update | **≥ 6 months** before EOL |
| 2. EOL declaration | Mark the release line EOL in `SECURITY.md` §"Supported versions" and in this policy's history | `SECURITY.md` + repository announcement | On the EOL date |
| 3. Final advisory sweep | Confirm no open in-scope advisory is unfixed for a still-supported line; publish a final summary if a known-unfixed issue would persist past EOL | GHSA + `CHANGELOG.md` | At EOL |
| 4. Machine-readable signal | Reflect EOL status so automated consumers can detect it (e.g. yank superseded crates where appropriate; keep the SBOM/VEX accurate; consider an `endoflife.date`-style entry) | crates.io/npm/PyPI metadata; SBOM/VEX | At EOL |

> The **6-month advance-notice** figure is proposed, not binding. It gives downstream
> consumers a realistic window to plan an upgrade, consistent with the CRA's "clear and
> understandable" communication expectation. The maintainer may adjust it on ratification.

### 4.3 Project-wide discontinuation

If `sparq` (or the manufacturer/steward of record) decides to **discontinue the project**,
the maintainer should: (a) announce the discontinuation and a final EOL date with the same
≥ 6-month advance notice; (b) state whether any successor / fork / hand-off path exists;
(c) keep the coordinated-disclosure channel (`SECURITY.md`, `.well-known/security.txt`) and
the published advisories reachable for the remainder of any release's support window; and
(d) update this policy and `SECURITY.md` to record the discontinuation. Discontinuation does
**not** retroactively shorten the support period already promised for a shipped release unless
that is itself announced with adequate notice.

## 5. How this ties to the rest of the CRA evidence

| This policy references | Existing artifact | Relationship |
|---|---|---|
| Support-period clock start (§2) | Per-release CycloneDX **SBOM** (`scripts/gen-sbom-vex.sh`, `release.yml#sbom`) | The SBOM's publication timestamp anchors each release's 5-year clock; the SBOM is also the Annex II A.6 "where to obtain the SBOM" answer |
| Affected-component identification at EOL / advisory time | **VEX** (`supply-chain/vex.cdx.json`) | The VEX states exploitability of known advisories per release, supporting "no known exploitable vulnerability at ship" (controls.md I.2) and EOL final-sweep decisions (§4.2 step 3) |
| Authority reporting during the support period | [`incident-reporting-runbook.md`](./incident-reporting-runbook.md) (Article 14 / GX-CRA-2) | If an actively-exploited vulnerability or severe incident occurs **within** a release's support window, the manufacturer/steward of record discharges the Article 14 reporting duty via that runbook; this policy defines the window during which that duty is active for a given release |
| Coordinated vulnerability disclosure | [`../../SECURITY.md`](../../SECURITY.md) | Defines how a report reaches the project and how a fix is coordinated; this policy defines for **how long** a release receives such fixes |
| Vulnerability-handling process controls | [`controls.md`](./controls.md) II.2 / II.4 / II.7 / II.8 / II-A.6 | This policy is the documented support-period statement those rows point at |

## 6. Operator / consumer responsibilities (the split)

Consistent with the library-vs-operator split in [`README.md`](./README.md):

- **sparq (the project / steward of record)** produces the security updates and advisories
  for supported releases and maintains this support statement.
- **The deploying operator / commercialising party** is responsible for **applying** updates
  within their own environment, for the CRA conformity-assessment / CE-marking layer if they
  place a product on the market, and for stating the support period of **their** product
  (which may differ from, but cannot exceed without independent maintenance, sparq's). A party
  that commercialises sparq becomes the manufacturer for their offering and inherits the duty
  to determine and state a support period — this policy gives them a ready baseline to adopt.

## 7. Ratification

This policy is **PROPOSED** and takes effect **only** when a maintainer (or the
manufacturer/open-source-software steward of record) explicitly ratifies it. Until then,
[`../../SECURITY.md`](../../SECURITY.md) §"Supported versions" remains the authoritative,
in-effect statement.

| Field | Value |
|---|---|
| Status | **PROPOSED — not yet ratified** |
| Proposed by | SPARQ compliance agent (bead `sq-f8tv`, epic `sq-toze`) |
| Ratifying authority | `<FILL-IN: maintainer / manufacturer / open-source-software steward of record>` |
| Ratification date | `<FILL-IN>` |
| Ratified support period (if amended from the proposed 5 years) | `<FILL-IN — default: 5 years from each release's publication>` |
| Next review | At the 1.0 milestone, or on any change of maintainer / commercialisation event |

On ratification, the maintainer should: (a) set the status above to "RATIFIED" with the date;
(b) update [`../../SECURITY.md`](../../SECURITY.md) §"Supported versions" to state the
concrete period and link this policy; and (c) flip GX-CRA-1 in
[`gap-register.md`](./gap-register.md) from "addressed pending ratification" to "resolved".

---

*This is a certification working document, not legal advice. Confirm all CRA dates,
thresholds, and the steward-vs-manufacturer determination against the published text of
Regulation (EU) 2024/2847 and your own legal/regulatory function.*
