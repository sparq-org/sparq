<!-- [OPUS-4.8] sq-toze — Certification-readiness summary for the maintainer. Lead-owned
     (consolidation). NON-CANONICAL timing; no measured numbers baked here. -->

# sparq certification readiness — maintainer summary

> 🤖 SPARQ agent. The one-page honest answer to: **what can sparq claim today, and what still needs
> an external assessor?** Full detail in [`README.md`](./README.md) (index + maturity) and
> [`gap-register.md`](./gap-register.md) (every gap, deduplicated). Epic **sq-toze**.

All 12 framework slices went through an adversarial **engineer↔auditor** loop. The proof-of-pattern
run (memory-safety) ended in an auditor **zero-findings SIGN-OFF**; the doc-heavy frameworks carry a
standing `re-review when Fable returns` flag (single-model Opus 4.8 authorship — non-canonical timing),
which is itself recorded honestly rather than presented as an independent second-model pass.

## Per-framework one-line verdict

| Framework | Verdict |
|---|---|
| **Memory-safety** | Substantively **PASS** — auditor SIGN-OFF (`FINDINGS: 0`); unsafe-register + gating count-ratchet + Miri/oracle/fuzz coverage. A *defensible attestation, not a rubber stamp*. |
| **ASVS L2** | Applicable controls **implemented & verified**; external L2 assessor + pentest is the AUDIT-READY ceiling. The one material engine leak (error-body info-leak) is **fixed (PR #241)**. |
| **CIS** | **PASS / N/A(operator)** except one P1 (no container-image CVE scan / Dockerfile linter, `sq-toze.31`) + one P3 (`HEALTHCHECK`). |
| **SBOM** | **Strong** — 22 implemented & verified / 6 audit-ready / 3 gap (all P2 completeness/hygiene); publication/signing half is config-verified, operating-verification pending first `v*` release. |
| **SSDF** | **High coverage** — 28 implemented & verified / 13 audit-ready / 1 gap (reproducible build). Self-attestation framework; no external SSDF auditor exists by design. |
| **SLSA** | **Build Level 2** for release archives + ghcr container + dist binaries + the `@sparq-org/sparq` npm package (honestly claimed; npm has native Sigstore provenance via `publish.yml`). **L3 NOT met** (in-band provenance ceiling); crates.io provenance is out-of-band only (no native registry link, external) and the PyPI `sparq-rdf` PEP-740 lane is CI-wired (`publish.yml#pypi-*`, Trusted Publishing + native attestations) but awaits a one-time maintainer PyPI Trusted-Publisher registration to go live (GX-10 partial, P1). |
| **OpenSSF** | **Strong** — every repo-level Scorecard check assertable from CI is verified; Best-Practices badge **answer-ready but not filed** (`sq-toze.5`). |
| **ISO 27001** | **Zero open Annex-A *control* gaps**; remaining is the **ISMS/SoA organisational act** (GAP-ISO-1) — a readiness pack, never a self-issued certificate. |
| **EU CRA** | Substance of the **Annex I vuln-handling process + most secure-by-default** essential requirements met; **CE-marking / DoC / Article-14 reporting** is the manufacturer org act. |
| **Privacy** (GDPR + 27701 + SOC2-Privacy) | Substantively **AUDIT-READY** — engine supports a compliant deployment; error-hygiene (P-12) **fixed (PR #241)**; certificate + controller duties are operator/external. |
| **Cryptoreview (ZK/MPC)** | **Readiness doc, NOT a certificate.** ZK verifier originally found **unsound** (`research/zk-soundness-audit.md`); `sq-1s2` landed the verifier binding layer and the **internal** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed → **"sound as landed for the assumed threat model"** — but **external cryptographer audit STILL REQUIRED** (`sq-qhy4`, P0) before any ZK/MPC security claim; `sparq-mpc` semi-honest-only, **no guarantee**. [OPUS-4.8] |
| **CDMC** | Engine-strong (two **4s**: security + architecture), honest **3s** (catalogue/classification/lifecycle), deliberate **2s** where the capability is an operator governance decision. |

## Genuinely demonstrable from the codebase / CI today

These are re-runnable, file-or-test-cited, and gating in CI — sparq can stand behind them now:

- `#![forbid(unsafe_code)]` across 31/36 crates; the concentrated `sparq-core` unsafe surface (mmap /
  dict-spill / SIMD = boundary **B5**) is enumerated, per-site justified, **count-ratcheted as a
  merge gate**, and covered by Miri + a corruption oracle + cargo-fuzz. (memsafety, SIGN-OFF)
- CodeQL SAST (gating), clippy `-D warnings` (gating), cargo-deny advisories/bans/licenses/sources
  **PR-gate** (GX-1 closed via #210), CycloneDX SBOM + VEX, OpenSSF Scorecard (published).
- SLSA **Build L2** provenance + buildkit `provenance: mode=max` + SHA256SUMS + cargo-auditable on the
  official release archives and the ghcr container; SHA-pinned actions; distroless non-root image with
  a docker-smoke gate.
- HTTP hardening: DoS body/limit controls, the documented **no-auth boundary B3** as an explicit
  architectural decision, structured-JSON error envelope, **and (this phase) sanitized error bodies
  that no longer leak caller input / loaded RDF data / filesystem paths to an unauthenticated caller**.
- A genuine data-engine capability surface: SPARQL 1.1 UPDATE lifecycle, SHACL/RDFS/OWL/N3 validation
  + reasoning, VoID + Service-Description catalogue endpoints, WAC/ACP fail-closed entitlement
  enforcement — the substance behind CDMC's two 4s and several honest 3s.
- A complete, machine-discoverable **coordinated-vulnerability-disclosure** posture (`SECURITY.md`,
  `.well-known/security.txt`, daily advisory watchdog, Dependabot, CODEOWNERS) — the CRA Annex I
  vuln-handling *substance*.

## Requires an external act (must NOT be presented as satisfied)

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

| External act | Framework | Tracking |
|---|---|---|
| **External accredited-cryptographer audit of the ZK/MPC estate** — the v1 ZK verifier was originally found **unsound** but `sq-1s2` landed the binding layer and the **internal, single-model** re-audit (`sq-gbp4`) found all findings closed ("sound as landed for the assumed threat model"); `sparq-mpc` carries **no guarantee**. All soundness assurance is still **internal self-review** — external sign-off is **pending**. **No ZK/MPC result may be relied upon in production until this closes.** | cryptoreview | **`sq-qhy4` (P0)** |
| **ISMS / Statement-of-Applicability** sign-off + operating programme | iso27001 | GAP-ISO-1 (`soa-template.md` is the scaffold) |
| **External penetration test** + accredited ASVS L2 assessor | asvs (cis) | AUDIT-READY |
| **CE marking / EU Declaration of Conformity / Article-14 reporting** | cra | CRA-CA.2/CA.3 (org act) |
| **SLSA Build L3** (isolated trusted builder) — L2 is the honest ceiling today; sq-toze.25 wired the isolated builder for the release **archives** and #4570 extended it to the GUI bundles, SBOM/VEX, conformance report and `dist.yml` binaries, but every lane is tag/dispatch-triggered and none has run yet, and the **ghcr container image** is still in-band | slsa | GX-11 / `sq-toze.25` + #4570 |
| Accredited-body **certificates** (ISO 27001, SLSA-L3, SOC2/27701) | iso27001, slsa, privacy | external by definition |
| Memory-safety **formal-methods** proof / accredited third-party B5 audit | memsafety | MS-G4 + external |
| Human-owned filings: **OpenSSF Best-Practices badge** (`sq-toze.5`), first `v*` release + first Sigstore/SLSA attestation, registry-publish provenance (crates.io has no upstream mechanism) | openssf, sbom, slsa | `sq-toze.5` / `sq-toze.24` / `sq-jgt3` |

## This phase's concrete deliverables

- **Security fix shipped — PR #241 (MERGED):** unauthenticated **error-body information leak**
  sanitized at the HTTP boundary. On the B3 no-auth path, error bodies previously echoed the caller's
  query/update token, the **loaded RDF term** (e.g. `patient_alice_smith is not a valid subject` —
  confirming loaded triples to an outsider), malformed-gzip bytes, and a `--persist` filesystem path.
  Fix routes full detail to the server-side `tracing` log and returns only a stable generic class
  message; 5 regression no-echo tests added. Refs `sq-cz89` (P1), `sq-j9zs` (ASVS-G3), `sq-zg0u`.
  This closed the single most material open engine gap shared by the Privacy + ASVS slices.
- **Opt-in-CI coverage gap (`sq-kzfi`)** now being addressed by a **feature-matrix CI lane** — the
  VoID/Service-Description catalogue surface is behind the default-OFF `federation-descriptors` feature
  (a stock build 404s and no lane compiles it), which is exactly why CDMC capability 2.1 sits at
  maturity **3, not 4**. CI-gating the feature earns the 4.

## Bottom line

sparq's **technical** security posture is genuinely strong and CI-gating, and every claim here is
evidence-cited and re-runnable. The certification *gaps* are concentrated in (a) external assessor /
accredited-body acts (the certificates themselves), (b) the **external cryptographer audit of the
ZK/MPC estate** — remediated and internally re-audited as "sound as landed" (`sq-gbp4`) but **not
externally signed off**, the single most important honesty boundary repo-wide — and (c) a handful
of in-repo completeness items (reproducible build, container CVE scan, published-package provenance,
the badge filing). **What sparq can honestly claim today:** a memory-safe, supply-chain-attested
(SLSA L2), SAST/fuzz/Miri-gated Rust data engine with a documented vuln-handling process and a
sanitized HTTP boundary. **What it cannot:** any ZK/MPC security/privacy guarantee, any
accredited-body certificate, or SLSA L3 — those need external assessors, and the docs say so plainly.
