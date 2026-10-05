<!-- [OPUS-4.8] SSDF framework intro (cert framework `ssdf`, epic sq-toze, bead
     sq-toze.13). Re-review when Fable returns. NON-CANONICAL timing. -->

# NIST SSDF (SP 800-218 v1.1) — secure-software-development framework

The **NIST Secure Software Development Framework** (SP 800-218 v1.1) is the canonical
*implementer-side* secure-SDLC framework: a set of practices (grouped **PO** Prepare the
Organization, **PS** Protect the Software, **PW** Produce Well-Secured Software, **RV**
Respond to Vulnerabilities) that a software *producer* follows so the artifacts it ships
are well-secured. sparq is shipped as a dependency (crates.io / npm / PyPI / ghcr), so SSDF
is a high-fit framework — and, because the practices map directly onto sparq's existing
gate stack, it is largely a **mapping** exercise rather than a build-out.

## Why SSDF (vs a certification framework)

SSDF is **not a certificate**. There is no "SSDF auditor" who issues a pass/fail seal; it is
the practice catalogue that producers self-attest to (e.g. in a US federal software
attestation form). So the deliverables here are an honest **practice → evidence** mapping a
deploying organization can lift into its own attestation — not a pending external cert.

## What this folder contains

| File | Purpose |
|---|---|
| [`controls.md`](./controls.md) | The spine: every SSDF task (PO/PS/PW/RV, 42 task rows) → status → repo evidence (file / test / CI job) → owner. |
| [`evidence.md`](./evidence.md) | By-artifact index resolving each control claim to a concrete, checkable location + how to reproduce it locally. |
| [`gap-register.md`](./gap-register.md) | Open gaps (severity, remediation, target, `bd` bead). |

## Scope — what's in, what's the operator's job

**In scope (the producer = sparq project):**
- The secure-SDLC *gates* — clippy `-D warnings`, CodeQL SAST, `cargo test`/conformance
  ratchets, Miri, fuzz, the unsafe-count ratchet, cargo-deny advisories/bans/sources/
  licenses and cargo-vet (post-merge/nightly, not PR-gating), the CycloneDX SBOM + VEX, SLSA
  build provenance, and the single required `ci-fast` check (core crates).
- The secure-coding standard (`CONTRIBUTING.md`), the threat model
  (`research/threat-model.md`), the dependency policy (`deny.toml`), and the coordinated
  vulnerability-disclosure programme (`SECURITY.md` + `.well-known/security.txt`).

**Out of scope (the operator/deploying organization owns):**
- SSDF practices about the *operating environment* — production deployment, environment
  separation in *their* infra (PO.5 partially), runtime monitoring, and their own incident
  response. sparq is a library/engine; the deploying org runs the SSDF programme for its
  service.
- The server's **authentication/authorization** is, by documented design (threat-model
  boundary **B3**), the operator's responsibility ("front with a gateway / sparq-solid") —
  an explicit architectural decision, not an SSDF gap.
- **Formal external attestation** of the audit-ready practices — for a single-maintainer
  volunteer project these are documented + automated and asserted; an org adopting sparq
  performs the formal attestation against its own ISMS.

## Honesty posture

The coverage summary in `controls.md` reports **28 implemented & verified / 13 audit-ready
/ 1 gap** across **42 rows = 41 standard SP 800-218 v1.1 tasks + 1 flagged sparq-local row**
(`RV.1.4`, the daily advisory watchdog — evidence supporting standard task RV.1.3, **not** a
separate framework task; see the footnote in `controls.md`). <!-- [OPUS-4.8] sq-ce97: 41
standard (RV.1 has 3 tasks) + 1 local row = 42 rows; status tally 28/13/1 unchanged. -->
The single technical gap is **PW.6.2 reproducible-build**
(GX-8, bead **sq-toze.9**) — now *characterised*: the honest reproducibility statement is
documented ([`../slsa/reproducible-build.md`](../slsa/reproducible-build.md)), with only the CI
rebuild-and-diff enforcement outstanding. No row presents the `sparq-zk*` / `sparq-mpc`
research scaffold as a met security control. [OPUS-4.8] The v1 ZK verifier was **originally found
unsound** (`research/zk-soundness-audit.md`, kept on record), then `sq-1s2` landed the binding
layer and an **internal post-remediation re-audit** (`research/zk-verifier-reaudit.md`, `sq-gbp4`)
found the prior findings closed → **"sound as landed for the assumed threat model"** — but that
verdict is **internal / single-model self-review only, with external accredited-cryptographer
sign-off still PENDING (`sq-qhy4`, P0) and NO production guarantee** (`SECURITY.md`). That
remediated-but-externally-unaudited posture is a correctly-disclosed limitation, and SSDF
PW.4/PW.5/PW.8 are scored on the engine, never on the crypto scaffold.
<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
