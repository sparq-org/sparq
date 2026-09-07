<!-- [OPUS-4.8] sq-toze — Master compliance index (lead-owned at consolidation). The full
     12-framework set. NON-CANONICAL timing; no measured numbers baked here. -->

# sparq compliance — certification readiness (full 12-framework set)

Per-framework certification evidence for `sparq` (a Rust RDF/SPARQL data-engine + HTTP server +
crypto estate consumed as a dependency in high-security settings). All **12** framework slices are on
`main`; each went through an adversarial **engineer↔auditor** audit (epic **sq-toze**).

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
> **Honesty contract.** Every control claim cites concrete evidence (file/test/CI). The ZK verifier
> was originally found **unsound** (`research/zk-soundness-audit.md`); `sq-1s2` landed the binding
> layer and the **internal, single-model** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`)
> found all findings closed ("sound as landed for the assumed threat model"), but external sign-off is
> **still pending** (`SECURITY.md`; `sq-qhy4`, P0) and there is **no production guarantee** — so the
> ZK/MPC estate is **excluded** from every security/privacy guarantee claim. No framework claims a *deployment*
> property (e.g. "sparq is GDPR compliant") — only the technical capabilities the engine provides.

## Certification readiness (start here)

- **[`CERTIFICATION-READINESS.md`](./CERTIFICATION-READINESS.md)** — the maintainer's one-page summary:
  per-framework verdicts, **what is demonstrable from the codebase/CI vs what needs an external act**,
  the security fix shipped this phase (**PR #241**), and the bottom line.
- **[`gap-register.md`](./gap-register.md)** — the **consolidated, deduplicated** cross-framework gap
  register: every open gap with its bead id + severity, the recurring cross-cutting gaps collapsed to
  one row each, and the **external-required residuals headlined**.

### The external-required residuals (the true self-certification ceiling)

These cannot be closed in-repo by an agent; they need an external assessor / accredited body /
external cryptographer / organisational act. **They must never be presented as satisfied.**

| Residual | Framework | Who must act | Tracking |
|---|---|---|---|
| **External accredited-cryptographer audit of the ZK/MPC estate** — v1 ZK verifier originally **unsound**; `sq-1s2` landed the binding layer and the **internal, single-model** re-audit (`sq-gbp4`) found all findings closed ("sound as landed for the assumed threat model"), but external sign-off is **pending** and `sparq-mpc` carries **no guarantee**; all current assurance is internal self-review | cryptoreview | external cryptographer | **`sq-qhy4` (P0, CRITICAL)** |
| **ISMS / Statement-of-Applicability** org act (scope, risk treatment, SoA, management review) | iso27001 | deploying org | GAP-ISO-1 (P1) |
| **External penetration test** + accredited ASVS L2 assessor of a deployed server | asvs (cis) | accredited assessor | AUDIT-READY |
| **CE marking / EU Declaration of Conformity / Article-14 reporting** | cra | commercialising party | CRA-CA.2/CA.3 |
| **SLSA Build L3** (isolated trusted builder) — **L2 is the honest ceiling today**; the isolated-builder lanes now cover every `release.yml`/`dist.yml` artifact except the ghcr container image (archives — sq-toze.25; GUI bundles, SBOM/VEX, conformance report, dist binaries — #4570), but every lane is unexercised and the container is still in-band | slsa | finish the trusted-builder migration (container) + one `v*` tag as evidence | GX-11 / `sq-toze.25` + #4570 (P3) |
| Accredited-body **certificates** (ISO 27001, SLSA-L3, SOC2/27701) | iso27001, slsa, privacy | accredited body | external by definition |

**Security fix shipped this phase — [PR #241](https://github.com/sparq-org/sparq/pull/241) (MERGED):**
the unauthenticated **error-body information leak** (echoing caller input / **loaded RDF data** /
filesystem paths on the B3 no-auth path) is now sanitized at the HTTP boundary, with 5 regression
no-echo tests. Refs `sq-cz89` / `sq-j9zs` / `sq-zg0u` — it closed the most material open engine gap
shared by the Privacy + ASVS slices.

**Opt-in-CI coverage gap (`sq-kzfi`):** the VoID/Service-Description catalogue surface is behind the
default-OFF `federation-descriptors` feature (stock build 404s, no lane compiles it) — exactly why
CDMC capability 2.1 sits at maturity 3, not 4. Now being addressed by a **feature-matrix CI lane**.

## Status legend

- **IMPLEMENTED & VERIFIED** — a technical control in the codebase/CI with passing, re-runnable evidence.
- **AUDIT-READY** — control + documentation in place, but the *certificate* needs an accredited external
  body / the operator's ISMS/PIMS / an external cryptographer.
- **OPERATOR-RESPONSIBILITY** — a property of the deployment, not the source (engine hook noted).
- **GAP** — not met; in the framework's `gap-register.md` (and the consolidated one) with a `bd` bead.

## Frameworks

Every slice below was driven through the engineer↔auditor loop. Memory-safety reached an auditor
**zero-findings SIGN-OFF**; the doc-heavy slices carry a `re-review when Fable returns` flag (recorded
honestly — single-model Opus 4.8 authorship, non-canonical timing).

| Framework | Folder | Scope | Verdict | Key residual |
|---|---|---|---|---|
| **Memory-safety** | [`memsafety/`](./memsafety/) | Justify every `unsafe` site; the B5 mmap loader surface | **Substantively PASS** (auditor SIGN-OFF, `FINDINGS: 0`) | Formal-methods proof / 3rd-party B5 audit (external) |
| **OWASP ASVS L2** | [`asvs/`](./asvs/) | `sparq-server` HTTP query/update API + RDF/SPARQL parsers | Applicable controls **IMPLEMENTED & VERIFIED**; external = AUDIT-READY | External L2 assessor + pentest |
| **CIS** | [`cis/`](./cis/) | CIS v8 safeguards + Docker Benchmark §4 image hardening | **PASS / N/A(operator)** except GX-12 (P1); GX-13 (P3) CLOSED by `sq-toze.36` (Dockerfile HEALTHCHECK via in-binary `--health-probe`) | Container CVE scan + Dockerfile linter (`sq-toze.31`) |
| **SBOM + supply-chain** | [`sbom/`](./sbom/) | NTIA elements, CycloneDX, VEX, signed/attested per-release SBOM | **Strong** — 28 verified / 6 audit-ready / 1 gap (GS-2 reproducible-build, P2; GS-1/3/4/5/6 RESOLVED) | Publication/signing operating-verified on first `v*` release |
| **NIST SSDF** | [`ssdf/`](./ssdf/) | SP 800-218 PO/PS/PW/RV → sparq control mapping | **28 verified / 13 audit-ready / 1 gap** | Reproducible-build (GX-8): honest non-determinism statement documented ([`slsa/reproducible-build.md`](./slsa/reproducible-build.md)); CI rebuild-and-diff enforcement remaining |
| **SLSA** | [`slsa/`](./slsa/) | Build provenance / supply-chain integrity of released artifacts | **Build Level 2** (honestly claimed; **not L3**) | L3 in-band-provenance ceiling; npm package now attested (`publish.yml`), crates.io out-of-band only (no native link, external), PyPI `sparq-rdf` PEP-740 lane CI-wired (`publish.yml#pypi-*`) awaiting one-time maintainer PyPI Trusted-Publisher config (GX-10 partial) |
| **OpenSSF** | [`openssf/`](./openssf/) | Scorecard checks + Best-Practices (CII) badge | **Strong**; badge answer-ready, **not filed** | File the Best-Practices badge (`sq-toze.5`) |
| **ISO/IEC 27001** | [`iso27001/`](./iso27001/) | Annex A control mapping + **org-adoptable ISMS template set** (clauses 4–10 + full SoA) + **operator deployment-security doc (B3)**, not a certificate | **Zero open Annex-A control gaps**; GAP-ISO-1 templates **delivered** + GAP-ISO-2 operator-doc **addressed**; only residual is the external certificate act | ISMS/SoA + certificate **org act** (GAP-ISO-1 residual, external) |
| **EU CRA** | [`cra/`](./cra/) | Annex I essential requirements + vuln-handling for a product w/ digital elements | Vuln-handling **substance + secure-by-default** met | CE marking / DoC / Article-14 (org act) |
| **Privacy** (GDPR + 27701 + SOC2-Privacy) | [`privacy/`](./privacy/) | Engine privacy capabilities under operator-controller / engine-processor split | Substantively **AUDIT-READY** (error-hygiene fixed, PR #241) | Operator PIMS + external auditor; ZK/MPC gated |
| **Cryptoreview (ZK/MPC)** | [`cryptoreview/`](./cryptoreview/) | Assurance-split of the crypto estate (sound Tier-A vs research Tier-B) | **Readiness doc** — verifier remediated (`sq-1s2`) + **internally** re-audited "sound as landed" (`sq-gbp4`), but **not externally audited**; no production guarantee | **External cryptographer audit (`sq-qhy4`, P0)** |
| **CDMC** | [`cdmc/`](./cdmc/) | EDM Council data-management maturity (6 components / 14 capabilities) | Engine-strong (two 4s), honest 3s, deliberate 2s | Lineage (CD-1) + access-audit (CD-2) P0 data-maturity |

## CDMC maturity overview (the headline)

Full scorecard: [`cdmc/scorecard.md`](./cdmc/scorecard.md). CDMC is scored conservatively (honesty
contract) — operator-owned governance *decisions* are capped at the maturity of sparq's enabling
*hook*, never credited as if sparq made the decision.

**The two earned 4s** — sparq is strongest where it is an engine:

- **4.1 Data is secured & controls are evidenced — 4.** The deep, CI-gated security estate:
  `#![forbid(unsafe_code)]`, Miri/fuzz, CodeQL, supply-chain attestation, DoS limits, distroless image.
- **6.1 Technical design supports the data platform — 4.** Sorted-permutation indexes, generation-ring
  MVCC snapshots, WAL durability, time-travel, zero-copy views, HDT/compressed ingest.

**The honest 3s** (real, documented, tested capability — exercisable today, but not enforced/gated):
1.1 data-control compliance · 1.3 sourcing & consumption · **2.1 catalogues** (VoID + Service-Description
— held at 3, not 4, only because it's behind the default-OFF `federation-descriptors` feature with no
CI lane: bead `sq-kzfi`) · 2.2 classification (SHACL/RDFS/OWL/N3) · 3.1 entitlements (WAC/ACP + token) ·
5.1 lifecycle (SPARQL 1.1 UPDATE) · 5.2 data quality (SHACL validation).

**The 2s** — deliberately low, where the capability is an operator governance *decision* or rests on
crypto carrying no production assurance: 1.2 data ownership · 3.2 access audit trail (CD-2) · **4.2 privacy framework** (ZK/MPC
remediated + internally re-audited but **not externally audited / no production guarantee** — contributes **zero**) · **4.3 sensitive-data encryption** (at-rest/in-transit is
operator-deployment; mmap files are plaintext) · 6.2 data lineage (CD-1).

| Component | Avg maturity | Verdict |
|---|---|---|
| 1 — Governance & Accountability | ~2.7 | Engine governance strong; data-ownership/sourcing operator-accountable |
| 2 — Cataloguing & Classification | ~3.0 | Real VoID/SD catalogue + SHACL hooks; catalogue held off 4 by not being CI-gated (`sq-kzfi`) |
| 3 — Accessibility & Usage | ~2.5 | Access *control* real (WAC/ACP/token); access *audit* + ODRL are gaps |
| 4 — Protection & Privacy | ~2.7 | Security **excellent (4)**; privacy/crypto **deliberately low (2)** — ZK/MPC not externally audited (no production guarantee) |
| 5 — Data Lifecycle | ~3.0 | Solid lifecycle *mechanism*; lifecycle *policy* operator-owned |
| 6 — Technical Architecture | ~3.0 | Architecture a strength **(4)**; lineage the weak axis (2) |

The single most important honesty statement, restated by both the CDMC scorecard and the crypto-review
register: the ZK verifier was remediated (`sq-1s2`) and an **internal, single-model** re-audit
(`sq-gbp4`) judged it "sound as landed for the assumed threat model", but it is **not externally
audited** (`sq-qhy4`, P0, pending) and carries **no production guarantee** — so the ZK/MPC estate still
contributes **zero** to any protection-by-cryptography maturity. Any future scorer crediting ZK/MPC as a
privacy control before external sign-off is overclaiming — flag it against the consolidated register's headline.

## Shared cross-framework artifacts (owned by the Privacy engineer)

| File | What |
|---|---|
| [`data-flow.md`](./data-flow.md) | Everywhere the binary can touch data + the operator/engine responsibility split |
| [`dpia.md`](./dpia.md) | DPIA *skeleton* — engine-risk half filled with evidence; operator completes the deployment half |
| [`threat-model.md`](./threat-model.md) | Privacy lens on the STRIDE model (references `research/threat-model.md`) |

## The operator-vs-engine split (the recurring frame)

sparq is a **data engine**, not a service that collects personal data or makes controller decisions.
Across the personal-data frameworks the recurring frame is: **the engine provides technical
capabilities** (access control, deletion, minimisation, confidentiality, auditability) **with repo
evidence; the deploying operator is the data controller** and owns lawful basis, purpose, retention
policy, TLS, at-rest encryption, and the certificate. See [`privacy/README.md`](./privacy/README.md).
