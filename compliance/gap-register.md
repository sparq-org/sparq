<!-- [OPUS-4.8] sq-toze — CONSOLIDATED cross-framework gap register. Lead-owned (consolidation).
     Collects every open gap from the 12 framework slices, deduplicated. NON-CANONICAL timing;
     no measured numbers baked here. -->

# sparq compliance — consolidated cross-framework gap register

> 🤖 SPARQ agent. This is the single deduplicated view of **every open gap** across the 12
> certification frameworks (epic **sq-toze**). Each per-framework `gap-register.md` is authoritative
> for its own slice; this file collapses the recurring cross-cutting gaps into one row each and
> headlines the items that **only an external party can close**.

Severity legend: **P0** blocks an honest high-value claim / is externally required · **P1** needed
for a perfect-score / no-residual posture · **P2** raises maturity or hygiene · **P3** aspirational.

---

## HEADLINE — external-required residuals (no agent / no in-repo fix can close these)

These are the items that, by definition, need an **external assessor, accredited body, external
cryptographer, or an organisational act** outside the source tree. They are the true ceiling on what
sparq can self-certify. **They must never be presented as already satisfied.**

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

| ID | Residual | Framework(s) | Who must act | Tracking |
|---|---|---|---|---|
| **CR-G1** | **External accredited-cryptographer audit of the ZK/MPC estate** (verifier binding layer, Schnorr-over-Baby-JubJub issuer sig, Noir circuits, composition seam). The verifier was originally found **unsound** (`research/zk-soundness-audit.md`); `sq-1s2` landed the binding layer and the **internal, single-model (Opus 4.8)** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed ("sound as landed for the assumed threat model") — but all current assurance is **internal self-review** and external sign-off is **pending**. Until it closes, `SECURITY.md`'s posture (**ZK verifier remediated but NOT externally audited / `sparq-mpc` NO guarantee / no production guarantee**) is the relying-party truth. | cryptoreview (privacy, cdmc gate on it) | external cryptographer | **`sq-qhy4` (P0, CRITICAL)** |
| **GAP-ISO-1** | **ISMS / Statement-of-Applicability org act** — scope statement, risk assessment + treatment plan, SoA over the 93 Annex-A controls, management review, internal-audit programme. `iso27001/soa-template.md` is the scaffold; the operating ISMS + sign-off is organisational. | iso27001 | deploying organisation | bead to create (P1) |
| **ASVS L2 external verification / penetration test** | Accredited ASVS L2 assessor verification and/or an external pentest of a *deployed* `sparq-server`. Cannot be self-issued. | asvs (cis AUDIT-READY rows) | accredited assessor / pentester | AUDIT-READY (external body) |
| **CE-marking / conformity assessment** | EU Declaration of Conformity + CE marking + Article 14 reporting are a manufacturer/steward **organisational act**, deliberately NOT recorded as a fixable in-tree gap (CRA-CA.2 / CRA-CA.3). | cra | commercialising/deploying party | AUDIT-READY (external body) |
| **SLSA Build L3** | Build **L2 is met and honestly claimed**; **L3 is NOT**. The trusted-builder migration now covers every artifact the `release.yml`/`dist.yml` pipelines publish except the container image — archives (sq-toze.25) plus GUI bundles, SBOM/VEX, conformance report and `dist.yml` binaries (#4570) — but every lane is **unexercised** (tag/dispatch-triggered) and the **ghcr container image** is still in-band, so nothing L3 is claimed. Post-publication verification of the bundles is now automated and fail-closed (`release.yml#verify-provenance` -> `release-verify.yml`, #4571) — that produces the evidence on the first tag; it does not substitute for it. The certificate-grade L3 audit remains external regardless. | slsa | finish the trusted-builder migration (container image) + one `v*` tag + one `dist` dispatch as evidence + accredited assessor | **`sq-toze.25` (P3, GX-11)** |
| **ISO 27001 / SLSA-L3 / SOC2 / 27701 certificates** | Every *certificate* (vs readiness pack) is issued only by an accredited body after a Stage-1 + Stage-2 / equivalent audit of an **operating** programme over time. | iso27001, slsa, privacy | accredited certification body | external by definition |
| **Memory-safety formal-methods / third-party audit** | Certificate-grade assurance of the unsafe mmap validators (Kani/Prusti proof) and an accredited third-party audit of the B5 surface are external. Current assurance is Miri + oracle + fuzz + per-site argument (a *defensible attestation, not a rubber stamp*). | memsafety | formal-methods / external auditor | **`sq-toze` MS-G4 (P2)** + external |

**Human-owned external filings (one-step, no outside body but not self-issuable by an agent):**
OpenSSF Best-Practices badge filing on bestpractices.dev (**GX-4 / `sq-toze.5`, P1**); first `v*`
release + first Sigstore/SLSA attestation (SBOM publication/signing half is config-verified, not yet
operating-verified); registry-publish provenance for crates.io / npm / PyPI (**`sq-toze.24` /
`sq-jgt3`** — crates.io has **no upstream provenance mechanism**); live GitHub branch-protection
ruleset verification against `docs/branch-protection.md` (**`sq-sto1`**).

---

## Cross-cutting gaps (recur across slices — one deduplicated row each)

| ID | Gap | Sev | Framework(s) | Bead | Status |
|---|---|---|---|---|---|
| **GX-8** | **Reproducible-build CHARACTERISED, not yet enforced** — the honest "not reproducible because X" statement the gap asked for is now **documented** ([`slsa/reproducible-build.md`](./slsa/reproducible-build.md)): a measured double-build of `sparq-cli` (`--release --locked`) is **identical size + byte-identical apart from 22 bytes**, all traced to **one** non-determinism source (the C-compiled `mimalloc` `__DATE__`/`__TIME__` `.rodata` banner + the build-id it perturbs). The SBOM↔binary integrity link remains asserted (SLSA provenance + cargo-auditable), not yet *bit-identically* reproducible. Residual to a byte-for-byte claim: `SOURCE_DATE_EPOCH`/feature-drop + a CI rebuild-and-diff ratchet. Recurs as SBOM **GS-2**, SSDF **SSDF-G1 (PW.6.2)**, SLSA SL-B3-adjacent, OpenSSF Badge `build_reproducible`, CRA integrity. | P2 | sbom, ssdf, slsa, openssf, cra | **`sq-toze.9`** | OPEN (doc DONE; CI ratchet remaining) |
| ~~**GX-9**~~ | `dist.yml` tiered `sparq-cli` binaries built with **no provenance** — dist/launcher binary unattested. **CLOSED (sq-toze.23):** `dist.yml#build` now has `id-token`/`attestations: write`, `cargo auditable build --locked`, and `actions/attest-build-provenance@a2bbfa2…` over each per-tier binary → **SLSA Build L2** (mirrors `release.yml#package`); verify `gh attestation verify dist/sparq-cli-<tier> --repo sparq-org/sparq`. | P1 | slsa (cited by cra) | **`sq-toze.23`** | ADDRESSED |
| **GX-10** | **Published packages carry no provenance** — there was no publish workflow (js/python lanes test-only); crates.io/npm/PyPI artifacts published out-of-CI, unattested. **PARTIAL (sq-toze.24 + sq-toze.37):** added `.github/workflows/publish.yml` — **npm `@sparq-org/sparq` CLOSED** (`#npm` `npm publish --provenance` in OIDC + `npm audit signatures` gate); **crates.io PARTIAL** (`#crates` attests the `cargo package` `.crate` bytes via `attest-build-provenance` — out-of-band `gh attestation verify`; crates.io has **no native provenance link** upstream → external sub-gap OPEN); **PyPI CI-WIRED, awaiting maintainer config (sq-toze.37):** `#pypi-build`/`#pypi-sdist`/`#pypi-publish` build the `sparq-rdf` wheels+sdist (maturin) and upload via PyPI **Trusted Publishing** with native **PEP-740 attestations** (`attestations: true`, OIDC `id-token: write`). PyPI's PEP-740 path is the strongest of the three (native provenance link on PyPI). The ONE non-repo step: a maintainer must register the Trusted Publisher on the `sparq-rdf` PyPI project (owner `sparq-org`, repo `sparq`, workflow `publish.yml`, env `pypi`) — until then the OIDC token mint fails by design (no static token stored). | P1 | slsa (cited by cra), openssf (GX-OSSF-2) | **`sq-toze.24`** (npm DONE / crates.io partial), **`sq-toze.37`** (PyPI CI wired; maintainer PyPI Trusted-Publisher config pending), **`sq-jgt3`** (registry signing) | PARTIAL |
| **GX-11** | **SLSA Build L3 not met — coverage residual CLOSED (sq-toze.25 → #4570 → #4635).** **Every** artifact the `release.yml`/`dist.yml` pipelines publish now routes provenance through an isolated `slsa-github-generator` trusted builder — four separate jobs, each a bare `uses:` with its own OIDC identity. Three use the *generic* generator with a threaded file-digest list crossing the boundary: `release.yml#provenance` (archives), `release.yml#provenance-artifacts` (GUI bundles, SBOM/VEX, conformance report), `dist.yml#provenance` (tiered binaries); `release` `needs:` both of its lanes and both signed `.intoto.jsonl` bundles are attached + in `SHA256SUMS`. The fourth, `release.yml#provenance-container` (#4635), uses the *container* generator (`generator_container_slsa3.yml`) over the OCI index digest the push reported, attaching the attestation to the image in ghcr.io. **Still OPEN on three counts:** no lane is **exercised** (`release.yml` is tag/dispatch-triggered, `dist.yml` dispatch-only — wiring is not evidence; #4571 automated post-publication verification of the two Release bundles so the first tag yields a citable verdict, but no release has fired it and that sweep does not cover the container attestation); the container lane is **not fail-closed** (an image must be pushed before it has a digest to attest, so publication precedes attestation); and both generators sign digests our build jobs reported rather than building inside the trusted builder (unforgeable provenance, not a hardened build). Published level stays **L2**. | P3 | slsa | **`sq-toze.25`** + **#4570** + **#4635** | OPEN (all four lanes wired-unverified) |
| **GX-12** | **No container-image CVE scan (Trivy/Grype) + no Dockerfile linter (Dockle/Hadolint) lane in CI** — only the docker-smoke test exists. | P1 | cis (cited by cra) | **`sq-toze.31`** | OPEN |
| ~~**GX-13**~~ | No `HEALTHCHECK` in the Dockerfile (distroless has no shell; needs a static probe / `--health-probe` subcommand). **CLOSED (sq-toze.36):** the `Dockerfile` now declares `HEALTHCHECK … CMD ["/usr/local/bin/sparq-server", "--health-probe"]`; since distroless has no shell/`curl`/`wget`, the server binary probes its own loopback `/health` (`crates/sparq-server/src/health_probe.rs`, exec-form) and exits 0/non-zero. Override addr via `--health-probe-addr` / `SPARQ_HEALTH_PROBE_ADDR`. | P3 | cis | **`sq-toze.36`** | ADDRESSED |

**Closed cross-cutting gaps (cited as evidence across slices — do NOT re-propose):** GX-1 cargo-deny
advisories PR-gate (`sq-toze.2`, closed by #210; CVSS-4.0 blocker `sq-q8de` resolved) · GX-2 per-release
SBOM + VEX (`sq-toze.3`) · GX-3 `.well-known/security.txt` RFC 9116 (`sq-toze.4`) · GX-5 unsafe-justification
register + cargo-geiger ratchet (`sq-toze.6`, owned by memsafety) · GX-6 CONTRIBUTING secure-coding section
(`sq-toze.7`) · GX-7 cargo-auditable / cargo-vet (`sq-toze.8`, landed via #210 — bead still open as a
watch item).

---

## Per-framework open gaps

### Memory-safety — substantively PASS (auditor SIGN-OFF, `FINDINGS: 0`)

| ID | Gap | Sev | Bead |
|---|---|---|---|
| ~~MS-G2~~ | ~~No first-party `clippy::undocumented_unsafe_blocks` lint~~ — **CLOSED (sq-8wbn).** `#![warn(clippy::undocumented_unsafe_blocks)]` is now crate-root on all 5 unsafe crates; the `-D warnings` gate mechanically rejects any undocumented `unsafe` (tree verified clippy-clean). | — | CLOSED |
| ~~MS-G3~~ | ~~No standalone AddressSanitizer lane outside cargo-fuzz~~ — **CLOSED (sq-hybl).** `.github/workflows/asan.yml` runs the deterministic mmap corruption corpus under `-Zsanitizer=address` (nightly, non-blocking). | — | CLOSED |
| MS-G4 | No formal verification / model-checking of the unsafe mmap validators (Kani/Prusti) — assurance ceiling, not a defect. Partly addressed: a Kani bounded-proof of the `.spqv` validator exists (sq-hkud); the dict validator awaits an `&[u8]`-seam refactor. | P2 | `sq-toze` (Kani follow-up on dict validator) |
| MS-G5 | Cross-doc unsafe-count drift (`research/threat-model.md` says 42, register/ratchet say 44 after sq-vkz7) | P2 | `sq-toze` (sync threat-model count) |

### ASVS — applicable controls PASS; external verification AUDIT-READY

| ID | Gap | Sev | Bead |
|---|---|---|---|
| ASVS-G1 | ~~`sparq-server` sets no security response headers (min `X-Content-Type-Options: nosniff`; no CSP/X-Frame-Options/HSTS)~~ **Resolved** ([OPUS-4.8] `sq-cmvh`, residual auth-path `Cache-Control: no-store` + auth-gated test under `sq-2bhm`) — central `map_response` layer in `harden()` stamps `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` on every response (success / streamed / error / auth-gated 401), and the sensitive 401 additionally carries `Cache-Control: no-store`; asserted by `tests/hardening.rs::security_headers_*`. | Medium | `sq-cmvh`, `sq-2bhm` |
| ASVS-G2 | No first-party CORS allowlist option (documented safe-default decision, not a vuln) | Low | `sq-o7o0` |
| ASVS-G3 | (Error-body info-leak) no test asserting no FS-path/internal-host leak — **FIX SHIPPED (PR #241)**; regression bead open | Medium | `sq-j9zs` (parent `sq-cz89`) |
| ASVS-G4 | No explicit SPARQL parse-depth bound (nested-input DoS bounded only by `--max-body-bytes`) | Low | `sq-53s1` |

### CIS — PASS / N/A(operator) except GX-12 (P1); GX-13 (P3) CLOSED by sq-toze.36 — see cross-cutting table.

### SBOM — 1 open gap (GS-2 = GX-8, all P2)

<!-- [OPUS-4.8] Synced with the authoritative per-framework slice `compliance/sbom/gap-register.md`:
     GS-1, GS-3, GS-4, GS-5, GS-6 and GS-7 are all RESOLVED (verified scripts named below); only
     GS-2 (reproducible-build = GX-8) remains open. See the slice for full remediation evidence. -->

| ID | Gap | Sev | Bead |
|---|---|---|---|
| GS-2 | = **GX-8** reproducible-build (cross-cutting) | P2 | `sq-toze.9` |

> **RESOLVED (do not re-open — see `compliance/sbom/gap-register.md`):** GS-1 per-component
> Supplier Name (NTIA N1) — `scripts/sbom-normalize.jq` derives a per-component `supplier`,
> CI-gated by `supply-chain.yml#sbom-supplier` (`scripts/check-sbom-supplier.py`), `sq-toze.26`;
> GS-3 npm/JS-lockfile SBOM for the WASM client — `scripts/gen-js-sbom.sh` + `supply-chain.yml#js-sbom`,
> `sq-toze.27`; GS-4 SBOM spec version — now CycloneDX 1.5 via `--spec-version 1.5`, `sq-toze.28`;
> GS-5 VEX↔deny.toml sync — gating `supply-chain.yml#vex-deny-sync` (`scripts/check-vex-deny-drift.py`),
> `sq-toze.29`; GS-6 abs-path leak — `scripts/sbom-normalize.jq` publication normalisation, `sq-toze.30`;
> GS-7 purl-canonicality — `supply-chain.yml#sbom-purl-canonical`, `sq-uujh`/`sq-tmyw`.

### SSDF — 28 implemented & verified / 13 audit-ready / 1 gap

| ID | Gap | Sev | Bead |
|---|---|---|---|
| SSDF-G1 | = **GX-8** reproducible-build (PW.6.2) — the only technical gap | P2 | `sq-toze.9` |
| (new) | Stand-alone Secure-SDLC policy template (PO.1.1; org-act deliverable, lifts PO.1.1/PO.2.1 from audit-ready) | — | `sq-5ty0` |

### SLSA — Build L2 met; GX-9/GX-10/GX-8/GX-11 (all in cross-cutting table). GX-11 narrowed by sq-toze.25 (archives) then #4570 (GUI bundles, SBOM/VEX, conformance report, dist binaries) onto isolated trusted builders — all wired, none yet exercised (#4571 automated the consumer-side verification that will evidence the first run); the ghcr image is the remaining in-band artifact.

### OpenSSF — strong; badge eligible-not-filed

| ID | Gap | Sev | Bead |
|---|---|---|---|
| GX-4 | OpenSSF Best-Practices (CII) badge not filed — questionnaire answer-ready at passing bar, but no bestpractices.dev entry, so Scorecard `CII-Best-Practices` scores 0 | P1 | `sq-toze.5` (blocks `sq-toze.15`) |
| GX-OSSF-2 | Registry publishes not signing-attested (crates.io has no first-party scheme; npm `--provenance`, PyPI Trusted Publishing available) | P3 | `sq-jgt3` |
| GX-OSSF-3 | Scorecard Code-Review/Branch-Protection depressed by solo-maintainer reality + live ruleset out-of-repo | P3 (partly external) | `sq-sto1` |

### ISO 27001 — zero open Annex-A *control* gaps; two readiness gaps

| ID | Gap | Sev | Bead |
|---|---|---|---|
| GAP-ISO-1 | **ISMS / SoA org act** (see HEADLINE) | P1/High | bead to create |
| GAP-ISO-2 | N/A(operator) controls implicit, not in one operator-facing deployment-security doc (42 Annex-A controls are the operator's) | P2 | bead to create |

### CRA — substance of vuln-handling + secure-by-default met; org/CE layer audit-ready

| ID | Gap | Sev | Bead |
|---|---|---|---|
| GX-CRA-1 | No concrete support/EOL-period statement | P1 | `sq-f8tv` |
| GX-CRA-2 | No Article 14 ENISA/CSIRT reporting runbook (24h/72h/final) | P1 | `sq-iy3p` |
| GX-CRA-3 | No single named "cybersecurity policy" document (substance scattered) | P2 | `sq-d43g` |
| (CE) | EU DoC + CE marking + conformity assessment — see HEADLINE | — | external (CRA-CA.2/CA.3) |

### Privacy — substantively AUDIT-READY; one engine fix shipped this phase

| ID | Gap | Sev | Bead |
|---|---|---|---|
| PR-G1 | Error bodies echo caller input incl. loaded RDF to an unauthenticated caller (audit raised from Low) — **FIX SHIPPED (PR #241)** | Medium | `sq-cz89` (code) |
| PR-G5 | No regression test pinning the error-body no-echo property — **SHIPPED with PR #241** | Low | `sq-zg0u` (test) |
| PR-G2 | No built-in structured/queryable access/audit log (only `--verbose` tower-http traces) | Low | `sq-toze.32` |
| PR-G3 | `--persist` WAL not erasure-complete (deleted triples persist in append-only segments until compaction; Art. 17) | Medium | `sq-toze.33` |
| PR-G4 | No request-log redaction control (`--verbose` logs full SPARQL incl. embedded PII) | Low | `sq-toze.34` |
| PR-X1 | Any privacy claim resting on ZK/MPC is **gated** — verifier remediated (`sq-1s2`) + internally re-audited "sound as landed" (`sq-gbp4`) but **not externally audited / no production guarantee** (gate, not a privacy fix) | — | `sq-toze.35` gate + epic `sq-1s2` |

### Cryptoreview — readiness doc; ZK verifier remediated + internally re-audited, NOT externally audited

| ID | Gap | Sev | Bead |
|---|---|---|---|
| CR-G1 | **External accredited-cryptographer audit** — see HEADLINE | P0/CRITICAL | `sq-qhy4` |
| CR-G2 | Crypto-chain forge tests `#[ignore]`d out of default CI | High | `sq-f9tl` (closed) |
| CR-G3 | Public-input empirical anchors missing for `filter_f64_d*` + k=2 scan members | High | `sq-f9tl` (closed) |
| CR-G4 | No FIPS 140-3 validated module; honest negative posture | Low | `sq-cu32` (P2) |
| CR-G5 | No constant-time / side-channel analysis of secret-bearing paths (issuer signing, MPC share ops) | P2 | `sq-egx6` |
| CR-G6 | Residual ZK privacy/binding deferrals (in-circuit salt binding; per-graph salt + list-IRI/version disclosed → linkability; `HolderPop` not credential-bound) — privacy, not soundness re-openings | Medium | `sq-hyhj`, `sq-93h`, `sq-i1dt`, `sq-42e3` (epic `sq-1s2`) |
| CR-G7 | `sparq-mpc` malicious-security / collaborative-proof deferred (research-only) | High (for any MPC reliance) | `sq-bjl`, `sq-34ml`, `sq-ox16` |

### CDMC — engine-strong (two 4s), governance-honest (the 2s)

| ID | Gap | Sev | Bead |
|---|---|---|---|
| CD-1 | No first-class data lineage (no W3C-PROV ingest→graph→result capture) — caps 6.2 (2→3) | P0 | rec CD-R1 (bead under `sq-toze`) |
| CD-2 | No per-query access audit trail (only aggregate Prometheus metrics) — cap 3.2 (2→3) | P0 | rec CD-R2 (bead under `sq-toze`) |
| CD-8 | Catalogue capability not CI-gated (VoID + Service-Description behind default-OFF `federation-descriptors`; stock build 404s; holds 2.1 at 3) | P1 | **`sq-kzfi`** (+ feature-matrix lane) |
| CD-3 | Classification taxonomy + encryption-at-rest silently operator-owned (no handoff doc) — caps 2.2/4.3 | P1 | rec CD-R3 |
| CD-4 | No ODRL usage control (entitlements are access-control only) — cap 3.1 (3→4) | P1 | rec CD-R4 |
| CD-5 | Retention not policy-driven (ring ages by age/count, not declarative TTL) — cap 5.1 (3→4) | P1 | rec CD-R5 |
| CD-6 | No machine-readable dataset-ownership convention — cap 1.2 (2→3) | P2 | rec CD-R6 |
| CD-7 | No published CDMC operator-responsibility split doc | P2 | rec CD-R7 |

---

## Consolidated count by severity (open gaps only)

| Severity | Count | Notes |
|---|---|---|
| **P0 / CRITICAL** | **3** | CR-G1 (external cryptographer), CDMC CD-1 + CD-2 (lineage + access-audit data-maturity). CR-G1 is **external-required**. |
| **P1 / High** | **~8** | GX-10, GX-12, GX-4, GAP-ISO-1, GX-CRA-1, GX-CRA-2, CDMC CD-3/4/5/8 (P1 cluster). GAP-ISO-1 is an **org act**. *(GX-9 closed — sq-toze.23, dist.yml binaries now SLSA Build L2. MS-G2 closed — sq-8wbn, first-party `undocumented_unsafe_blocks` lint.)* |
| **P2 / Medium** | **~11** | GX-8 (one row, recurs in 5 slices; the SBOM slice's GS-2 is this same gap), SSDF-G1(=GX-8), MS-G4/G5, ASVS-G1/G3, GAP-ISO-2, GX-CRA-3, PR-G3, CR-G4/G5, CDMC CD-6/7. *(GS-1/3/4/5/6 closed — sq-toze.26/27/28/29/30. MS-G3 closed — sq-hybl, standalone ASan lane.)* |
| **P3 / Low** | **~7** | GX-11, GX-OSSF-2/3, ASVS-G2/G4, PR-G2/G4/G5. *(GX-13 closed — sq-toze.36, Dockerfile HEALTHCHECK via in-binary `--health-probe`.)* |

Counts collapse the recurring cross-cutting gaps (GX-8, GX-9, GX-10, GX-12) to **one row each**; the
per-slice gap-registers list them once per framework. The only **P0** that is a hard external ceiling
is **CR-G1** (`sq-qhy4`); the two CDMC P0s are data-management-maturity items closable in-repo.

---

## Honesty posture (binding across all frameworks)

Every framework **excludes the not-externally-audited ZK/MPC crypto from its security/privacy claims** and the
crypto-review auditor's tripwire confirmed **zero soundness overclaims**: the memsafety audit cleared
its ZK honesty tripwire (the `sparq-zk-compose` flock sites are scoped as memory-safety FFI only, no
crypto guarantee); CDMC scores ZK/MPC **zero** toward protection-by-cryptography (caps 4.2/4.3 held at
2); privacy gates PR-X1 on it and claims no privacy benefit; ISO routes A.8.24 crypto to the
cryptoreview verdict. No slice launders the ZK/MPC scaffold into an assurance. The canonical posture
remains `SECURITY.md`: the v1 ZK verifier was originally found unsound (`research/zk-soundness-audit.md`),
`sq-1s2` landed the binding layer and the **internal, single-model** re-audit
(`research/zk-verifier-reaudit.md`, `sq-gbp4`) found it **"sound as landed for the assumed threat model"**,
but it is **NOT externally audited** and **`sparq-mpc` carries no guarantee** — **no production guarantee**
until **CR-G1 / `sq-qhy4`** closes with an external cryptographer.
