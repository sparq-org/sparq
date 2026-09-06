<!-- [OPUS-4.8] Secure-SDLC policy TEMPLATE — epic sq-toze / bead sq-5ty0 (SSDF PO.1.1/PO.2.1).
     TEMPLATE: requires org sign-off before adoption. Authored while Fable unavailable —
     re-review when Fable returns. NON-CANONICAL timing (EC2 work box); no measured numbers here. -->

# Secure Software-Development-Lifecycle (Secure-SDLC) policy (TEMPLATE)

> **STATUS: TEMPLATE — requires organizational sign-off before adoption.** This is a
> stand-alone, **org-adoptable** consolidation of sparq's secure-SDLC posture. It is **not**
> a certificate and **not** an organizational policy on its own: every `<FILL-IN>` placeholder
> is an organizational decision (owner, dates, scope, attestation signatory) that the adopting
> organization completes and signs. Adopting it does **not** make sparq — or any adopting
> organization — "SSDF-attested"; SSDF (NIST SP 800-218) is an *implementer/producer practice
> catalogue with no certificate*, self-attested by the producer (see
> [`../ssdf/README.md`](../ssdf/README.md)).

## 0. Why this template exists

NIST SSDF practices **PO.1.1** ("define security requirements for software development; maintain
them over time") and **PO.2.1** ("define security checks/criteria for each SDLC stage") are
**evidenced today** by sparq's existing artifacts — [`CONTRIBUTING.md`](../../CONTRIBUTING.md)
("Secure coding" + "The gate"), [`SECURITY.md`](../../SECURITY.md), and
[`research/threat-model.md`](../../research/threat-model.md) — but only at the **mapping level**
(see [`../ssdf/controls.md`](../ssdf/controls.md) rows PO.1.1 / PO.2.1, both **Audit-ready**). A
deploying **organization** that needs a single, named *Secure-SDLC policy* artifact to fold into
its own attestation had to assemble one from those scattered sources. This template **consolidates
them into one adoptable document**, lifting PO.1.1/PO.2.1 from *audit-ready* toward a clean
**org-level attestation** — without re-stating or forking the underlying gates (they remain
single-sourced in the cited files).

> **This is not a codebase gap.** The technical secure-SDLC controls already exist and gate every
> PR. This is a **policy-template deliverable**: the missing artifact was a *named, consolidated
> policy document an org can adopt*, not a missing control.

## 1. Purpose & scope

**Purpose.** This policy states the security requirements that govern the development of sparq and
the security checks every change must pass at each SDLC stage. It is the consolidated PO.1
(requirements) + PO.2 (per-stage criteria) artifact for SSDF self-attestation.

**Scope.**

- **In scope (the producer = the sparq project / the adopting org's fork or build of it):** the
  source, the build/test/lint gate, the supply-chain controls, the threat model, the secure-coding
  standard, and the coordinated vulnerability-disclosure programme.
- **Out of scope (the operator/deploying organization owns):** the *operating environment* —
  production deployment, environment separation in the org's own infrastructure, runtime
  monitoring, and the org's own incident response. sparq is a library/engine; the deploying org
  runs the SSDF programme for **its** service. The server's **authentication/authorization** is, by
  documented design (threat-model boundary **B3**), the operator's responsibility ("front with a
  gateway / sparq-solid") — an explicit architectural decision, not an SSDF gap
  ([`SECURITY.md`](../../SECURITY.md), [`research/threat-model.md`](../../research/threat-model.md)).

This policy **cross-references, does not fork**, the related cross-framework policy templates under
this same `compliance/policies/` directory and the framework folders — see §7.

## 2. Security requirements for development (SSDF PO.1)

These are the standing security requirements every change is held to. They are
**documented + version-controlled** in the cited source-of-truth files; this section names and
consolidates them. The organization records the requirement *register* and its review cadence in
§6.

| # | Requirement | Source of truth (single-sourced; not duplicated here) | SSDF |
|---|---|---|---|
| R-1 | **Hardened-input posture.** The untrusted-input surfaces (`sparq-core` parser, `sparq-engine` planner/executor, `sparq-server` HTTP) validate at the boundary, prefer total functions, and never let untrusted input drive an `unwrap`/`expect`/`panic!` or an unbounded allocation (a reachable panic or OOM is an in-scope DoS bug). | `CONTRIBUTING.md` "Secure coding → Input validation"; `SECURITY.md` | PO.1.1, PW.5.1 |
| R-2 | **`unsafe` discipline.** Prefer safe Rust; the 26/31 `#![forbid(unsafe_code)]` crates stay that way. Every `unsafe` site carries a `// SAFETY:` comment + a row in `compliance/memsafety/unsafe-register.md`, and the unsafe-count ratchet only ever goes down or stays flat. | `CONTRIBUTING.md` "Every `unsafe` block needs a `// SAFETY:` comment and a register row"; `compliance/memsafety/unsafe-register.md` | PO.1.1, PW.5.1, PW.8.2 |
| R-3 | **Clean error/log output.** No RDF/SPARQL content, internal paths, or stack detail leaks into HTTP error responses or logs. | `CONTRIBUTING.md` "Secure coding"; `SECURITY.md` | PO.1.1, PW.5.1 |
| R-4 | **Supply-chain hygiene.** Add the smallest maintained dependency that does the job; cargo-deny (advisories/bans/sources/licenses) and cargo-vet gate dependency changes; each tolerated advisory carries a reason + a tracking bead in `deny.toml`. | `CONTRIBUTING.md` "Secure coding → Supply chain"; `deny.toml`; `.github/workflows/supply-chain.yml` | PO.1.1, PW.4 |
| R-5 | **Threat-model alignment.** Changes are assessed against the STRIDE threat model and its trust boundaries **B1–B5** (B3 = the documented no-auth server boundary; B5 = the highest-severity mmap/`unsafe` boundary). | `research/threat-model.md` | PO.1.1, PW.1.1 |
| R-6 | **Honest crypto-scope.** The `sparq-zk*` / `sparq-mpc` crates are research scaffolds that carry **no production security guarantee** and must **not** be presented as a cryptographic assurance; their status is a *disclosed limitation*, never a met control. | `SECURITY.md`; `research/zk-soundness-audit.md`; `research/zk-verifier-reaudit.md` <!-- privacy-claims-allow: states the no-guarantee limitation; not a soundness claim --> | PO.1.1 |
| R-n | `<FILL-IN: any org-specific development security requirements — e.g. data-classification handling, key-management for the org's deployment, regulated-data constraints>` | `<FILL-IN>` | `<FILL-IN>` |

**Maintenance of requirements over time (PO.1.1).** These requirements are version-controlled with
the source. A change to a requirement lands through the same gated PR flow as code (§3). The
organization sets the **review cadence** for this register in §6.

## 3. Security checks per SDLC stage (SSDF PO.2.1)

The check criteria for each stage. The authoritative gate definition is `AGENTS.md` "The gate" /
`CONTRIBUTING.md`; CI enforces it (`docs/branch-protection.md`, the `ci-summary / gate`
aggregator). **A change lands only when all applicable checks below are green.**

| SDLC stage | Mandated security checks (criteria) | Enforced by | SSDF |
|---|---|---|---|
| **Design / change-intent** | Threat-model re-evaluation against B1–B5; the PR-template re-evaluation checklist tied to `AGENTS.md`. | `research/threat-model.md`; `.github/PULL_REQUEST_TEMPLATE.md` | PO.2.1, PW.1.1 |
| **Code authoring** | The secure-coding standard (R-1…R-6); `// SAFETY:` + register on any new `unsafe`. | `CONTRIBUTING.md` "Secure coding"; `compliance/memsafety/unsafe-register.md` | PO.2.1, PW.5.1 |
| **Static analysis (SAST)** | clippy `--all-targets -- -D warnings` (hard gate); CodeQL `security-and-quality`; code-scanning alerts kept at zero. | `.github/workflows/ci.yml#clippy`; `.github/workflows/codeql.yml` | PO.2.1, PW.7 |
| **Build** | `--locked` release build; `cargo auditable build` embeds the dependency manifest; distroless non-root container. | `.github/workflows/release.yml`; `Dockerfile` | PO.2.1, PW.6.1 |
| **Test (functional + conformance)** | `cargo test --workspace`; the W3C SPARQL/SHACL/inference conformance ratchets (never lowered); the coverage/perf ratchets. | `.github/workflows/ci.yml`; the conformance/perf floors | PO.2.1, PW.8.1 |
| **Dynamic analysis** | Miri UB lane over the `sparq-core` `unsafe` surface; coverage-guided fuzzing over parsers + mmap loader; the mmap-corruption oracle for B5 sites Miri cannot reach. | `.github/workflows/miri.yml`; `.github/workflows/fuzz.yml` | PO.2.1, PW.8.2 |
| **Supply chain** | cargo-deny advisories (**gating** at PR time) + bans/sources/licenses; cargo-vet gating; daily advisory watchdog; per-build SBOM. | `.github/workflows/supply-chain.yml`; `.github/workflows/dependency-monitoring.yml`; `deny.toml` | PO.2.1, PW.4 |
| **Review** | PR-only flow (no direct pushes, incl. admins); required `@jeswr` review on CODEOWNERS security-sensitive paths; all review comments resolved before merge. | `docs/branch-protection.md`; `CODEOWNERS` | PO.2.1, PW.7.1 |
| **Public-API change** | Any changed public API (a `pub` item, CLI flag, HTTP route, or binding) updates the matching `skills/<surface>/SKILL.md` **in the same change** (enforced convention). | `AGENTS.md` MAINTENANCE RULE; `CONTRIBUTING.md` | PO.2.1 |
| **Release** | SLSA build-provenance attestation (Sigstore) over each archive + SBOM/VEX; `SHA256SUMS`; per-release SBOM + VEX. | `.github/workflows/release.yml` | PO.2.1, PS.2.1, PS.3.2 |
| **Vulnerability response** | Private intake (GitHub Security Advisories / email); acknowledge within 5 business days, initial assessment within 10; coordinated disclosure; `.well-known/security.txt`. | `SECURITY.md`; `.well-known/security.txt` | RV.1–RV.3 |
| `<FILL-IN>` | `<FILL-IN: org-stage-specific checks — e.g. the org's pre-deployment security review, environment-promotion gates, runtime monitoring sign-off>` | `<FILL-IN>` | `<FILL-IN>` |

The single required merge check is `ci-summary / gate`, which polls every sibling check-run and
fails if any concluded failure (`docs/branch-protection.md`).

## 4. Roles & responsibilities

| Role | Responsibility | Org assignment |
|---|---|---|
| Maintainer / development owner | Authors/reviews changes against §2/§3; runs the gate; merges only green, reviewed PRs. For the upstream project this is `@jeswr` (single maintainer; `CODEOWNERS`). | `<FILL-IN: org dev owner>` |
| Security contact | Maintains the threat model and the disclosure programme; triages advisories; owns the VEX exploitability assessments. | `<FILL-IN: org security contact>` |
| Release owner | Runs the release pipeline; confirms provenance + SBOM + VEX attached and attested. | `<FILL-IN: org release owner>` |
| Management / policy approver | Sets the requirement-review cadence; approves this policy and accepts residual posture (SSDF PO.2.3 management commitment). For a single-maintainer project this is implicit (maintainer = decision authority); for an adopting org it is a named approver. | `<FILL-IN: org approver>` |
| Operator (deploying organization) | Owns the out-of-scope items: deployment, the B3 fronting gateway / authn, environment separation, runtime monitoring, the org's own incident response. | `<FILL-IN: operator owner>` |

## 5. Verification & evidence

The checks in §3 are **automatically enforced** by the CI/branch-protection wiring cited inline and
indexed in [`../ssdf/evidence.md`](../ssdf/evidence.md) (by-artifact, with local-reproduction
commands) and [`../ssdf/controls.md`](../ssdf/controls.md) (per-task status). A reviewer can
spot-check any change against the `ci-summary / gate` result and verify a release with
`gh attestation verify <file> --repo sparq-org/sparq`. This policy does **not** restate that evidence —
it points to the single source so the two never drift.

## 6. Maintenance & review (org completes)

| Item | Value |
|---|---|
| Requirement-register review cadence (PO.1.1) | `<FILL-IN: e.g. annually + on significant change — new untrusted-input surface, new sparq major version, a new trust boundary>` |
| This-policy review cadence | `<FILL-IN>` |
| Trigger events forcing an out-of-cycle review | `<FILL-IN: e.g. a GHSA advisory against sparq, a change to the B3 gateway assumption, a new crypto-bearing feature reaching GA>` |
| Policy owner / approver | `<FILL-IN>` |
| Last reviewed / next review | `<FILL-IN>` |

## 7. Related policies & frameworks (cross-references, not forks)

- **SSDF mapping** — [`../ssdf/controls.md`](../ssdf/controls.md), [`../ssdf/evidence.md`](../ssdf/evidence.md), [`../ssdf/gap-register.md`](../ssdf/gap-register.md) (the one open technical gap is PW.6.2 reproducible-build, GX-8 / `sq-toze.9` — the honest reproducibility statement is documented in [`../slsa/reproducible-build.md`](../slsa/reproducible-build.md); only the CI rebuild-and-diff enforcement is outstanding).
- **Coordinated vulnerability disclosure** — [`SECURITY.md`](../../SECURITY.md) is the live human-readable disclosure policy; the consolidating CRA cybersecurity-policy template ([`policy-cybersecurity.md`](./policy-cybersecurity.md), `sq-d43g`) lives alongside this one and adopts this Secure-SDLC document by reference for its §3 per-stage controls.
- **Dependency / supply-chain policy** — [`deny.toml`](../../deny.toml); the SBOM-publication policy template ([`../sbom/policy-sbom-publication.md`](../sbom/policy-sbom-publication.md)).
- **Release-signing / provenance** — `.github/workflows/release.yml`; the SLSA framework folder ([`../slsa/README.md`](../slsa/README.md)).
- **ISO 27001 ISMS** — the clause-5 (leadership policy) and clause-7 SoA rows point at this `compliance/policies/` set ([`../iso27001/isms-templates-README.md`](../iso27001/isms-templates-README.md)).

## 8. Honesty footer

This is the organization's **secure-SDLC documented policy** — it is **not** a certificate and
**not** a sparq claim of external attestation. sparq supplies the *technical control substrate*
(the gates, the threat model, the disclosure programme) and consolidates it into this adoptable
artifact; the **organization** supplies every `<FILL-IN>` decision, owner, cadence, and sign-off.
**No statement in this document credits the `sparq-zk*` / `sparq-mpc` estate as a met security
control** — it carries no production guarantee (the v1 ZK verifier was originally found **not**
sound, then remediated (`sq-1s2`) with an **internal** re-audit, external accredited-cryptographer
sign-off **still pending**, `sq-qhy4`; `SECURITY.md`), and crediting it here would be the exact
overclaim the honesty contract and the `privacy-claims` CI gate forbid.
<!-- privacy-claims-allow: states the no-guarantee/pending-audit limitation (historical "found not sound" + "still pending"); not a current soundness claim -->

## Provenance

Authored under epic `sq-toze`, bead **sq-5ty0** (SSDF PO.1.1 — stand-alone Secure-SDLC policy
template). Single-model Opus 4.8 authorship while Fable is unavailable — carries the repo's standard
`re-review when Fable returns` flag.
