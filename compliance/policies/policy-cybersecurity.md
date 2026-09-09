<!-- [OPUS-4.8] CRA cybersecurity-policy TEMPLATE — bead sq-d43g (gap GX-CRA-3, epic sq-toze);
     evidence rows compliance/cra/controls.md CRA-CA.1 (Art.13 risk assessment) + CRA-CA.6
     (Art.24 open-source-steward cybersecurity policy).
     TEMPLATE: requires org sign-off before adoption. Authored while Fable unavailable —
     re-review when Fable returns. NON-CANONICAL timing (EC2 work box); no measured numbers here. -->

# Cybersecurity policy (TEMPLATE) — EU CRA Art. 24 (steward) / Art. 13 (manufacturer)

> **STATUS: TEMPLATE — requires organizational sign-off before adoption.** This is the single,
> named, **org-adoptable** *cybersecurity policy* artifact the EU Cyber Resilience Act (Regulation
> (EU) 2024/2847) asks for: **Article 24** obliges an **open-source-software steward** to *"put in
> place and document a cybersecurity policy"*; **Article 13** obliges a **manufacturer** to base the
> product on a **documented cybersecurity risk assessment** and to run the vulnerability-handling
> processes. It is **not** a certificate, **not** an EU declaration of conformity, and **not** a CE
> marking — those are organizational/legal acts reserved to the manufacturer/steward of record
> ([`../cra/README.md`](../cra/README.md), [`../cra/controls.md`](../cra/controls.md) rows
> CRA-CA.2/CA.3, which this document does **not** assert). Every `<FILL-IN>` placeholder is an
> organizational decision (the steward/manufacturer of record, owners, dates, the formal risk-
> acceptance signatory) that the adopting entity completes and signs.

## 0. Why this template exists

The CRA's two governance hooks for a Rust RDF/SPARQL library + server distributed on
crates.io/npm/PyPI/ghcr are:

- **Art. 24 (open-source-software steward).** A steward — an entity supporting development of FOSS
  *intended for commercial use* but **not monetising it** — carries a lighter-touch regime whose
  **central documented artifact is a cybersecurity policy** (plus market-surveillance cooperation
  and Article-14 reporting). sparq today is a non-monetised, pre-1.0 research project (MIT), so the
  **steward** reading is the realistic frame ([`../cra/README.md`](../cra/README.md) §"Open-source-
  steward nuance").
- **Art. 13 (manufacturer).** The moment a party *commercialises* sparq (sells it, bundles it in a
  paid product, or offers it as a paid EU service), **that party becomes the manufacturer for their
  offering** and inherits the full conformity-assessment + CE-marking burden — for which Art. 13
  requires a **documented cybersecurity risk assessment** and the Annex I Part II processes. The
  technical substance is identical for both readings; only the formal conformity layer differs.

The **substance** of a cybersecurity policy already exists in sparq — but **scattered**:
coordinated disclosure in [`../../SECURITY.md`](../../SECURITY.md), secure-SDLC in
[`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) + [`../../AGENTS.md`](../../AGENTS.md), the risk
assessment in [`../../research/threat-model.md`](../../research/threat-model.md), the dependency
policy in [`../../deny.toml`](../../deny.toml), and release-signing in the CI workflows. A steward
or commercialising party that needs **one named, adoptable, signed cybersecurity-policy artifact**
had to assemble it from those sources. This template **consolidates them into one document**,
lifting CRA-CA.1 (Art. 13 risk assessment) and CRA-CA.6 (Art. 24 steward policy) from *audit-ready*
toward a clean org-level adoption — **without re-stating or forking** the underlying controls (they
remain single-sourced in the cited files, so the two never drift).

> **This is not a codebase gap.** The technical cybersecurity controls already exist and gate every
> PR. This is a **policy-template deliverable** (gap **GX-CRA-3** in
> [`../cra/gap-register.md`](../cra/gap-register.md)): the missing artifact was a *named,
> consolidated cybersecurity policy an org can adopt and sign*, not a missing control.

## 1. Purpose & scope

**Purpose.** This policy is the steward's/manufacturer's documented statement of how the
cybersecurity of sparq is governed: the **risk basis** (§2), the **secure development lifecycle**
(§3), the **dependency / supply-chain policy** (§4), the **coordinated vulnerability disclosure** and
vulnerability-handling process (§5), the **release-signing / integrity** policy (§6), the **support
period & end-of-life** posture (§7), and **roles, review, and authority reporting** (§8–§10). It is
the consolidated Art. 24 / Art. 13 artifact for CRA self-attestation as a steward, and the policy
floor a commercialising manufacturer adopts.

**Scope.**

- **In scope (the producer = the sparq project / the adopting org's fork or build of it):** the
  source, the build/test/lint gate, the supply-chain controls, the threat model, the secure-coding
  standard, the coordinated-disclosure programme, the release-integrity (signing/provenance/SBOM)
  pipeline, and the support-period statement.
- **Out of scope (the operator / deploying or commercialising organization owns):** the *operating
  environment* — production deployment, TLS termination, environment separation, runtime
  monitoring, and the org's own incident response; **end-user authentication / authorization**,
  which is, by documented design (threat-model boundary **B3**: "front with a gateway /
  sparq-solid"), the operator's control, not an in-scope gap; and the **conformity assessment / EU
  declaration of conformity / CE marking** (Art. 28, Annex V), which an agent and the source tree
  cannot self-certify.

This policy **cross-references, does not fork**, the sibling cross-framework templates under this
same `compliance/policies/` directory and the framework folders — see §11.

## 2. Cybersecurity risk basis (CRA Art. 13 risk assessment; controls.md CRA-CA.1)

The product is designed, developed, and maintained against a **documented cybersecurity risk
assessment**. The authoritative assessment is the **STRIDE threat model**
([`../../research/threat-model.md`](../../research/threat-model.md)) with its named trust boundaries
**B1–B5**; this section names it and records the org-level acceptance — it does **not** restate the
model.

| Boundary | What it covers | Risk posture (single-sourced; see threat-model.md) |
|---|---|---|
| **B1** | Untrusted RDF/Turtle/N-Triples/HDT input → `sparq-core` parser | Hardened-input: validate at the boundary, no untrusted-input-driven `unwrap`/panic/OOM (in-scope DoS); fuzz + Miri coverage. |
| **B2** | Untrusted SPARQL query → `sparq-engine` planner/executor | Bounded execution; reachable panic / unbounded allocation treated as an in-scope DoS bug. |
| **B3** | `sparq-server` HTTP surface | **No per-user authn/authz by design** — fail-closed non-loopback bind + optional Bearer token; end-user auth is the **operator's** control behind a gateway. An accepted architectural decision, surfaced by a loud no-auth startup warning — not a silent gap. |
| **B4** | Supply chain (dependencies, build, distribution) | cargo-deny + cargo-vet gating, SLSA provenance, signed releases (§4/§6). |
| **B5** | `sparq-core` `unsafe` surface (mmap / dict-spill / SIMD) | Highest-severity boundary; concentrated `unsafe` with `// SAFETY:` + register, Miri + fuzz + the mmap-corruption oracle ([`../memsafety/`](../memsafety/)). |

**Excluded from every risk claim:** the `sparq-zk*` / `sparq-mpc` estate provides **no production
security guarantee** today and is **not** credited as a risk control anywhere in this policy (§12).

| Risk-assessment governance item | Value |
|---|---|
| Authoritative assessment artifact | [`../../research/threat-model.md`](../../research/threat-model.md) (STRIDE, B1–B5) |
| Assessment review cadence (Art. 13) | `<FILL-IN: e.g. annually + on a new untrusted-input surface / new trust boundary / new crypto-bearing feature reaching GA>` |
| Residual-risk acceptance signatory | `<FILL-IN: steward / manufacturer of record — top management for a commercialising party>` |
| Last reviewed / next review | `<FILL-IN>` |

## 3. Secure development lifecycle (the per-stage security controls)

sparq runs a gated secure-SDLC: every change passes the security checks at each lifecycle stage,
enforced by CI + branch protection (`ci-summary / gate`). The **full per-stage criteria** are
single-sourced in the **Secure-SDLC policy template**
([`policy-secure-sdlc.md`](./policy-secure-sdlc.md), SSDF PO.1/PO.2) — this cybersecurity policy
**adopts that document by reference** rather than re-stating it. Summary of the load-bearing gates:

| SDLC stage | Mandated security control | Enforced by |
|---|---|---|
| Design / change-intent | Threat-model re-evaluation against B1–B5; PR-template checklist | `research/threat-model.md`; `.github/PULL_REQUEST_TEMPLATE.md` |
| Code authoring | Secure-coding standard (input validation, `unsafe` discipline with `// SAFETY:` + register, no content/path leakage in errors/logs) | `CONTRIBUTING.md` "Secure coding"; `compliance/memsafety/unsafe-register.md` |
| Static analysis | clippy `--all-targets -- -D warnings` (hard gate); CodeQL `security-and-quality`; code-scanning alerts kept at zero | `.github/workflows/ci.yml`, `codeql.yml` |
| Build | `--locked` release build; `cargo auditable build` embeds the dep manifest; distroless non-root container | `.github/workflows/release.yml`; `Dockerfile` |
| Test / conformance | `cargo test --workspace`; W3C SPARQL/SHACL/inference conformance ratchets (never lowered); coverage/perf floors | `.github/workflows/ci.yml` |
| Dynamic analysis | Miri UB lane over `sparq-core` `unsafe`; coverage-guided fuzzing of parsers + mmap loader; the B5 mmap-corruption oracle | `.github/workflows/miri.yml`, `fuzz.yml` |
| Review | PR-only flow (no direct pushes, incl. admins); required `@jeswr` review on CODEOWNERS security paths; all comments resolved before merge | `docs/branch-protection.md`; `CODEOWNERS` |
| Public-API change | Any changed public API updates the matching `skills/<surface>/SKILL.md` in the same change | `AGENTS.md` MAINTENANCE RULE |

## 4. Dependency / supply-chain policy

The dependency policy is single-sourced in [`../../deny.toml`](../../deny.toml) and the
supply-chain CI; this section names the standing rules:

- **Smallest maintained dependency** that does the job; new dependencies are reviewed at PR time.
- **`cargo deny check advisories`** is **GATING at PR time** (un-degraded) — advisories/bans/
  sources/licenses (`.github/workflows/supply-chain.yml#audit`); `[advisories] yanked = "deny"` with
  a fail-closed ignore policy. Each tolerated advisory carries a **reason + a tracking bead** in
  `deny.toml` and a row in the checked-in **VEX** (`supply-chain/vex.cdx.json`), kept 1:1.
- **`cargo vet --locked`** gating (`supply-chain.yml#vet`); a **daily advisory watchdog**
  (`dependency-monitoring.yml`) is defence-in-depth; **Dependabot** opens ungrouped per-advisory
  security PRs across four ecosystems (`.github/dependabot.yml`).
- The **SBOM-publication policy** (per-release CycloneDX + VEX, NTIA minimum elements) is
  single-sourced in the sibling template
  ([`../sbom/policy-sbom-publication.md`](../sbom/policy-sbom-publication.md)).

This satisfies the "made available **without known exploitable vulnerabilities**" essential
requirement on the **real PR-time advisory gate**, not aspiration
([`../cra/controls.md`](../cra/controls.md) I.2, II.1).

## 5. Coordinated vulnerability disclosure & vulnerability handling (CRA Annex I Part II)

The **live, in-effect** coordinated-vulnerability-disclosure (CVD) policy is
[`../../SECURITY.md`](../../SECURITY.md) + the RFC 9116 `.well-known/security.txt`; this policy
**adopts it by reference** and names the process:

- **Private intake** via GitHub Security Advisories or `mailto:` (two contacts in `security.txt` +
  `SECURITY.md`); **no public issue** for an unfixed vulnerability.
- **Response targets:** acknowledge within **5 business days**, initial assessment within **10**
  (`SECURITY.md`).
- **Remediate without delay:** fixes land on `main` and ship in the next release of the affected
  artifact; **public disclosure** of fixed vulnerabilities via GHSA + `CHANGELOG.md` + release notes
  (Annex I Part II.2/.4).
- **In-scope** includes a reachable panic / unbounded allocation from untrusted input (a DoS bug);
  the ZK/MPC estate's "no production guarantee" status is a **disclosed scope limitation**, never a
  met control (`SECURITY.md`).
- **Authority reporting (Art. 14, from 2026-09-11):** the early-warning (24h) / notification (72h) /
  final-report timeline to ENISA/CSIRT is operationalised in
  [`../cra/incident-reporting-runbook.md`](../cra/incident-reporting-runbook.md). The **act** of
  reporting is the steward/manufacturer's organizational duty, discharged via that runbook (§10).

## 6. Release-signing & artifact-integrity policy

The release-integrity policy is single-sourced in `.github/workflows/release.yml` /
`publish.yml` and the SLSA framework folder ([`../slsa/README.md`](../slsa/README.md)); this section
names the standing requirements:

- Every release carries **`SHA256SUMS`** over each archive + SBOM + VEX, and a **SLSA build-
  provenance attestation** (`actions/attest-build-provenance`, Sigstore/OIDC) on the archives, SBOM,
  and the ghcr image (`provenance: mode=max`, `sbom: true`). The tiered `dist.yml` binaries are
  provenance-attested too. Verify with `gh attestation verify <file> --repo sparq-org/sparq`.
- **CI actions are SHA-pinned**; `cargo-auditable` embeds the dependency manifest in the binary.
- **Published-package provenance:** npm `@sparq-org/sparq` publishes with native Sigstore provenance;
  crates.io `.crate` bytes carry an out-of-band attestation (no native link upstream — external
  sub-gap); the PyPI `sparq-rdf` PEP-740 lane is CI-wired awaiting a one-time maintainer
  Trusted-Publisher registration (GX-10 partial — [`../cra/gap-register.md`](../cra/gap-register.md)).
- **Reproducibility:** a measured double-build is byte-identical apart from one named non-determinism
  source ([`../slsa/reproducible-build.md`](../slsa/reproducible-build.md)); bit-for-bit CI
  enforcement is the documented residual (GX-8).

This is the honest **SLSA Build Level 2** posture (not L3) recorded in
[`../README.md`](../README.md); this policy makes **no L3 claim**.

## 7. Support period & end-of-life

The support-period / EOL statement is single-sourced in the sibling **support & EOL policy**
([`../cra/support-policy.md`](../cra/support-policy.md), Annex II A.6 / Annex I Part II.8): a
**proposed** 5-year support period from each release (the CRA minimum), the security-update channel,
and the EOL notification process. That document is itself **PROPOSED pending maintainer
ratification** — until ratified, `SECURITY.md` §"Supported versions" is authoritative. This
cybersecurity policy adopts it by reference; ratifying this policy and the support policy together
is part of the §13 sign-off.

## 8. Roles & responsibilities

| Role | Responsibility | Org assignment |
|---|---|---|
| Maintainer / development owner | Authors/reviews changes against §2–§4; runs the gate; merges only green, reviewed PRs. Upstream this is `@jeswr` (single maintainer; `CODEOWNERS`). | `<FILL-IN: org dev owner>` |
| Security contact | Maintains the threat model + the CVD programme; triages advisories; owns VEX exploitability assessments; the Art. 14 reporting decision (§10). | `<FILL-IN: org security contact>` |
| Release owner | Runs the release pipeline; confirms provenance + SBOM + VEX attached and attested (§6). | `<FILL-IN: org release owner>` |
| **Steward / manufacturer of record** | The CRA-accountable party: adopts + signs this policy, accepts residual risk (§2), and is the addressee of Art. 24 (steward) or Art. 13/28 (manufacturer) obligations. For a non-monetised upstream this is implicit (maintainer = steward); for a commercialising party it is a **named legal entity**. | `<FILL-IN: steward / manufacturer of record>` |
| Operator (deploying organization) | Owns the out-of-scope items: deployment, the B3 fronting gateway / authn, TLS, environment separation, runtime monitoring, its own incident response. | `<FILL-IN: operator owner>` |

## 9. Verification & evidence

The controls in §2–§6 are **automatically enforced** by the CI/branch-protection wiring cited
inline and indexed in [`../cra/evidence.md`](../cra/evidence.md) (by-artifact, with verification
commands) and [`../cra/controls.md`](../cra/controls.md) (per-requirement status). A reviewer can
spot-check any change against the `ci-summary / gate` result and verify a release with
`gh attestation verify <file> --repo sparq-org/sparq`. This policy does **not** restate that evidence —
it points to the single source so the two never drift.

## 10. Market-surveillance cooperation & authority reporting (Art. 24 / Art. 14)

As a steward (or manufacturer), the entity of record undertakes to **cooperate with
market-surveillance authorities** on request and to **report actively-exploited vulnerabilities and
severe incidents** to ENISA/the relevant CSIRT per **Article 14** (from 2026-09-11), using the
documented runbook ([`../cra/incident-reporting-runbook.md`](../cra/incident-reporting-runbook.md)).
The **runbook is adoptable documentation**; the **act of reporting** is the steward/manufacturer's
organizational/legal duty and is **not** self-certified here.

| Authority-cooperation item | Value |
|---|---|
| Single point of contact (vuln reporting) | `<FILL-IN: from SECURITY.md / security.txt — e.g. jesse@jeswr.org + the GHSA URL>` |
| Art. 14 reporting owner | `<FILL-IN: steward / manufacturer security contact>` |
| Competent CSIRT / national authority | `<FILL-IN: per the org's place of establishment>` |

## 11. Maintenance & review (org completes)

| Item | Value |
|---|---|
| This-policy review cadence | `<FILL-IN: e.g. annually + on a significant change>` |
| Risk-assessment review cadence (§2, Art. 13) | `<FILL-IN>` |
| Trigger events forcing an out-of-cycle review | `<FILL-IN: e.g. a GHSA advisory against sparq; a change to the B3 gateway assumption; a commercialisation event; a new crypto-bearing feature reaching GA>` |
| Policy owner / approver | `<FILL-IN>` |
| Last reviewed / next review | `<FILL-IN>` |

## 12. Related policies & frameworks (cross-references, not forks)

- **CRA mapping** — [`../cra/controls.md`](../cra/controls.md) (this policy is the artifact rows
  **CRA-CA.1** (Art. 13 risk assessment) + **CRA-CA.6** (Art. 24 steward policy) point at),
  [`../cra/evidence.md`](../cra/evidence.md), [`../cra/gap-register.md`](../cra/gap-register.md)
  (gap **GX-CRA-3**).
- **Secure-SDLC** — [`policy-secure-sdlc.md`](./policy-secure-sdlc.md) (the §3 per-stage controls,
  single-sourced).
- **Support & EOL** — [`../cra/support-policy.md`](../cra/support-policy.md) (the §7 support period).
- **Article 14 reporting** — [`../cra/incident-reporting-runbook.md`](../cra/incident-reporting-runbook.md)
  (the §5/§10 authority-reporting workflow).
- **Coordinated vulnerability disclosure (in-effect)** — [`../../SECURITY.md`](../../SECURITY.md);
  RFC 9116 `.well-known/security.txt`.
- **Dependency / SBOM** — [`../../deny.toml`](../../deny.toml);
  [`../sbom/policy-sbom-publication.md`](../sbom/policy-sbom-publication.md).
- **Release-signing / provenance** — `.github/workflows/release.yml`;
  [`../slsa/README.md`](../slsa/README.md).
- **ISO 27001 ISMS** — the clause-5 (leadership policy) + clause-7 SoA rows point at this
  `compliance/policies/` set ([`../iso27001/isms-templates-README.md`](../iso27001/isms-templates-README.md)).

## 13. Adoption & sign-off (org completes)

This policy is **PROPOSED** and takes effect **only** when the steward / manufacturer of record
explicitly adopts and signs it. Until then, the in-effect statements are the cited live sources
(`SECURITY.md`, `CONTRIBUTING.md`, `deny.toml`, `research/threat-model.md`).

| Field | Value |
|---|---|
| Status | **PROPOSED — not yet adopted** |
| Proposed by | SPARQ compliance agent (bead `sq-d43g`, gap GX-CRA-3, epic `sq-toze`) |
| Adopting authority (steward / manufacturer of record) | `<FILL-IN>` |
| Adoption / sign-off date | `<FILL-IN>` |
| Next review | `<FILL-IN — see §11>` |

On adoption, the steward/manufacturer should: (a) set the status above to "ADOPTED" with the date
and signatory; (b) complete every `<FILL-IN>` in §2/§8/§10/§11; (c) ratify the sibling support
policy ([`../cra/support-policy.md`](../cra/support-policy.md) §7); and (d) flip GX-CRA-3 in
[`../cra/gap-register.md`](../cra/gap-register.md) from "audit-ready (policy template)" to
"resolved".

## 14. Honesty footer

This is the organization's **documented cybersecurity policy** under the CRA steward (Art. 24) /
manufacturer (Art. 13) reading — it is **not** a certificate, **not** an EU declaration of
conformity, and **not** a CE marking, and it makes no claim of external attestation. sparq supplies
the *technical control substrate* (the gates, the threat model, the disclosure programme, the signed
release pipeline) and consolidates it into this adoptable artifact; the **organization** supplies
every `<FILL-IN>` decision, owner, cadence, and sign-off. **No statement in this document credits the
`sparq-zk*` / `sparq-mpc` estate as a met security control** — it carries no production guarantee
(the v1 ZK verifier was originally found **not** sound, then remediated (`sq-1s2`) with an
**internal** re-audit, external accredited-cryptographer sign-off **still pending**, `sq-qhy4`;
`SECURITY.md`), and crediting it here would be the exact overclaim the honesty contract and the
`privacy-claims` CI gate forbid.
<!-- privacy-claims-allow: states the no-guarantee/pending-audit limitation (historical "found not sound" + "still pending"); not a current soundness claim -->

## Provenance

Authored under epic `sq-toze`, bead **sq-d43g** (gap GX-CRA-3 — the single named CRA Art. 24 / Art. 13
cybersecurity-policy template). Single-model Opus 4.8 authorship while Fable is unavailable — carries
the repo's standard `re-review when Fable returns` flag.
