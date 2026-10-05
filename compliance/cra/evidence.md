<!-- [OPUS-4.8] EU CRA evidence pack. Bead sq-toze.18. Each item maps to a controls.md row;
     "verify" = the command/path a reviewer (or an external assessor) runs to confirm it. NON-CANONICAL timing. -->
# EU CRA — evidence pack

The artifacts behind [`controls.md`](./controls.md), grouped by CRA requirement cluster, with
the **verification command or path** for each. Paths are repo-relative. This is the evidence an
external conformity assessor (or the manufacturer assembling the Annex VII technical
documentation) would inspect.

## E1 — Secure-by-default & secure-by-design (Annex I Part I (1), (2)(a)(h)(i))

| Evidence | Where | Verify |
|---|---|---|
| Threat model (STRIDE, boundaries B1–B5, incl. B3 no-auth + B5 unsafe surface) | `research/threat-model.md` | Read; cross-check B3 against the server bind/auth posture below. |
| Fail-closed non-loopback bind | `crates/sparq-server/src/main.rs` (sq-o4qf) | Run `sparq-server --addr 0.0.0.0:3030` without `--allow-remote`/auth → server **refuses** + exits non-zero. |
| Optional Bearer auth + read-gating | `crates/sparq-server/src/main.rs` (sq-zcby) | `SPARQ_AUTH_TOKEN=t sparq-server …` then a request without the bearer → 401; with `SPARQ_AUTH_TOKEN_READ=1` reads are gated too. |
| Distroless **non-root** runtime, SHA-pinned base | `Dockerfile` (runtime `FROM gcr.io/distroless/cc-debian12:nonroot@sha256:…`) | `docker inspect` the image → user is non-root, no shell present. |
| `#![forbid(unsafe_code)]` posture + concentrated unsafe under UB lanes | 31/36 crates; `compliance/memsafety/`, `.github/workflows/{miri,fuzz}.yml` | `grep -rl 'forbid(unsafe_code)' crates/*/src/lib.rs` → 31 (36 crates total); CI miri/fuzz lanes green. |
| Secure-coding standard (contributor-facing) | `CONTRIBUTING.md` §"unsafe policy"/"Input validation"/"Supply chain" | Read; maps to NIST SSDF PW + ASVS V1/V5/V7. |

## E2 — No known exploitable vulnerabilities at ship (Annex I Part I (2); Part II.1)

| Evidence | Where | Verify |
|---|---|---|
| **GATING** PR-time advisory check (un-degraded) | `.github/workflows/supply-chain.yml#audit` step "cargo-deny check (advisories) — GATING" | `cargo deny check advisories` exits 0 on a clean tree; removing a justified `ignore` makes it exit non-zero (proves it is fail-closed, not cosmetic). |
| Fail-closed advisory/license/source policy | `deny.toml` (`yanked = "deny"`, permissive-only license allowlist, crates.io-only sources) | `cargo deny check bans sources licenses` (the always-gating subset) + `cargo deny check advisories`. |
| Daily advisory watchdog (defence-in-depth) | `.github/workflows/dependency-monitoring.yml` | Scheduled run opens/updates one idempotent `security:dependency-vuln` issue on a finding. |
| The tolerated advisory is justified + VEX'd | `deny.toml [advisories].ignore` ↔ `supply-chain/vex.cdx.json` | Four tolerated advisories, each with a justification + tracking bead, and the ignore list and the VEX `vulnerabilities[]` carry the same four ids (1:1, gated by the `VEX ↔ deny.toml drift check — GATING` step of `supply-chain.yml`): RUSTSEC-2024-0436 `paste` and RUSTSEC-2025-0141 `bincode` (maintenance-status notices, `not_affected`), RUSTSEC-2026-0194/0195 `quick-xml` (availability DoS reachable via the transitive oxigraph 0.5.x copy, honestly recorded as `exploitable`). rustls-pemfile/RUSTSEC-2025-0134 was retired by REMOVING the dependency rather than tolerating it — [OPUS-5] sq-5ah3p migrated the `sparq-lws-core`/`sparq-server` mTLS PEM parse to `rustls-pki-types`' `PemObject` and dropped the ignore, the VEX statement and the cargo-vet exemption in one change. That is the preferred remediation; when an advisory genuinely cannot be removed, add the ignore AND a matching VEX entry together. |
| npm-graph advisory disposition + staleness tripwire | [`supply-chain/npm-advisories.md`](../../supply-chain/npm-advisories.md), `scripts/check-npm-advisory-record.py` (GATING step "npm advisory-record drift check — GATING" in `supply-chain.yml`) | The cargo gate does not cover the npm graph, and the `js-sbom` step generates SBOMs without failing on advisories, so npm advisories surface **only via Dependabot** — which repeatedly concludes `security_update_not_possible` for `brace-expansion` (alerts #25/#26/#27/#46) and `postcss` (#45): a patched release exists but the reachable update path cannot land it. Nothing is suppressed — no `dependabot.yml` `ignore:` entry is added, so the alerts stay open and the future patch still notifies. The disposition (each instance, its version, and what pins it) is recorded and CI-enforced against `package-lock.json` + the root `overrides`, so it cannot go quietly stale; a RED means the graph moved and the disposition must be revisited (#3767). Honest level: **disposition-staleness enforcement, not npm vulnerability detection** — the check has no advisory feed and no network. |
| Excluded desktop-GUI workspace advisory disposition | [`supply-chain/gui-tauri-advisories.md`](../../supply-chain/gui-tauri-advisories.md) | The Tauri 2 GUI (`gui/src-tauri`) is a **standalone workspace excluded from the root** (`Cargo.toml [workspace].exclude`), so the root `cargo-deny` gate does not scan it; its advisories (incl. Dependabot alert #5, `glib`/**RUSTSEC-2024-0429**, and the GTK3-EOL cluster) surface **only via Dependabot**. The clean fix (`glib >= 0.20`) is upstream-blocked — Tauri 2's Linux GTK3 stack pins `glib ^0.18` and no Tauri 2.x minor moves off GTK3 (`glib 0.20` is GTK4-only). Disposition + resolver evidence recorded in that doc; upstream tracked (sq-x3r9b). |

## E3 — SBOM (Annex I Part II.1)

| Evidence | Where | Verify |
|---|---|---|
| Per-release CycloneDX SBOM (per shipped binary) | `scripts/gen-sbom-vex.sh`, `.github/workflows/release.yml#sbom` | On a tag, the Release carries `sparq-cli-<v>.sbom.cdx.json` + `sparq-server-<v>.sbom.cdx.json`. |
| NTIA minimum elements coverage | the `*.cdx.json` components | supplier, component name, version, PURL (`pkg:cargo/…`), dependency relationships, author, timestamp all present. *(GS-1/GS-3/GS-4 quality items RESOLVED — sq-toze.26/27/28; see `compliance/sbom/gap-register.md`.)* |
| Checked-in VEX (source of truth) | `supply-chain/vex.cdx.json` | CycloneDX 1.5; two `not_affected` entries mirroring `deny.toml`. |
| CI SBOM artifact every push | `supply-chain.yml#sbom` | `cargo cyclonedx --all --format json` → uploaded artifact `sbom-cyclonedx`. |

## E4 — Vulnerability handling & security updates (Annex I Part II.2/.7/.8)

| Evidence | Where | Verify |
|---|---|---|
| Security-fix flow (main → next release) + supported versions | `SECURITY.md` §"Supported versions" | Read; matches the release pipeline. |
| Per-advisory **ungrouped** security PRs, 4 ecosystems | `.github/dependabot.yml` | cargo / github-actions / npm / pip; security updates left ungrouped (each its own PR). |
| Integrity-protected distribution: SHA256SUMS + SLSA provenance | `.github/workflows/release.yml` (`SHA256SUMS`, `actions/attest-build-provenance`, buildkit `provenance: mode=max` + `sbom: true`) | `gh attestation verify <archive> --repo sparq-org/sparq`; `shasum -a 256 -c SHA256SUMS`. |
| **Reproducible-build statement** (build integrity — GX-8) — characterised, single named cause | [`../slsa/reproducible-build.md`](../slsa/reproducible-build.md) | Run the auditor quick-run in that doc: two `--release --locked` builds of `sparq-cli` → identical size + **byte-identical apart from 22 bytes** (the `mimalloc` build-time `__DATE__`/`__TIME__` banner + the build-id it perturbs). The bit-for-bit *enforcement* (CI rebuild-and-diff) is the residual, tracked under `sq-toze.9`. |
| `cargo-auditable` self-describing binaries | `release.yml#package`, `Dockerfile` (`cargo auditable build`) | `cargo audit bin <binary>` / `auditable info <binary>` reads the embedded manifest. |
| `cargo-vet` per-dependency audit attestations (GATING ratchet) | `supply-chain.yml#vet`, `supply-chain/{config.toml,audits.toml,imports.lock}` | `cargo vet --locked` exits 0; a new unaudited dep fails until audited/exempted. |
| SHA-pinned third-party actions | every workflow uses `uses: …@<full-sha> # vX.Y.Z` | grep workflows; Dependabot tracks the trailing-tag comment. |

## E5 — Coordinated disclosure & info-to-users (Annex I Part II.4/.5/.6; Annex II)

| Evidence | Where | Verify |
|---|---|---|
| Coordinated vulnerability disclosure policy | `SECURITY.md` (GHSA + email, response targets, no-public-issue, scope/caveats) | Read; the ZK "remediated-but-externally-unaudited, no production guarantee" + MPC semi-honest-only caveats are explicit (honesty contract). [OPUS-4.8] |
| RFC 9116 machine-discoverable channel | `.well-known/security.txt` | Contact ×2, Policy, Canonical, Expires (2027-06-15) present + in the future. |
| Public disclosure of fixed vulns | GitHub Security Advisories + `CHANGELOG.md` + release notes | GHSA list; changelog entries; release body links the changelog. |
| Annex II info: identity, product, intended use, limitations, secure-use | `SECURITY.md`, `README.md`, `AGENTS.md`, `Dockerfile` header, `crates/sparq-server/README.md`, OCI image labels | Read; the "experimental, pre-1.0, NO-guarantee ZK/MPC" framing is consistent across all. |
| **Article 14 incident-reporting runbook** (early-warning 24h / notification 72h / final report 14-day vuln · 1-month incident; ENISA single-reporting-platform + CSIRT routing; report-content checklist) — `[OPUS-4.8]` (sq-zbb5) | [`incident-reporting-runbook.md`](./incident-reporting-runbook.md) (sq-iy3p, controls.md CRA-CA.5) | Read; confirm it **extends** the `SECURITY.md` CVD flow (§7) without replacing it, that the trigger excludes the no-guarantee ZK/MPC crates (§1), and that every org-specific value is a `<FILL-IN>` placeholder. This is an **adoptable template**, not proof a report has occurred — see the honesty note in §E8. |

## E6 — Availability / DoS resilience (Annex I Part I (2)(f)(g))

| Evidence | Where | Verify |
|---|---|---|
| Request-size cap | `--max-body-bytes` (`crates/sparq-server/src/main.rs`) | Oversized body → 413. |
| **Zip-bomb guard** (decompression-ratio cap) | `--max-decompress-ratio` (sq-ebii) | A high-ratio gzip body is refused/413 before expanding. |
| Concurrency / load shedding | `--max-concurrent` | Beyond the cap requests are shed (metrics in `crates/sparq-server/src/metrics.rs`). |
| Result-set caps | `--max-results`, `--max-query-rows` | 413 beyond the limit. |
| Subscription caps (WebSocket) | `--max-subscriptions`, `--max-subscriptions-per-conn` | Enforced per-connection + globally. |
| Parser/loader fuzzing (panic/OOM as in-scope DoS) | `.github/workflows/fuzz.yml`, `SECURITY.md`/`CONTRIBUTING.md` | cargo-fuzz PR smoke + nightly; reachable panic from untrusted input is a security bug. |

## E7 — The exclusions and the operator split (honesty anchors)

| Item | Statement | Where |
|---|---|---|
<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
| ZK/MPC provides **no production** security guarantee | v1 ZK verifier originally found unsound, then `sq-1s2` landed the binding layer + internal re-audit (`sq-gbp4`) found findings closed → "sound as landed for the assumed threat model" — but **internal/single-model only, external sign-off PENDING (`sq-qhy4`), no production guarantee**; `sparq-mpc` semi-honest-only — excluded from every CRA confidentiality/integrity claim. | `SECURITY.md`, `research/zk-soundness-audit.md`, `research/zk-verifier-reaudit.md`, [`README.md`](./README.md) §nuance 4 |
| End-user auth/authz is the **operator's** (B3) | Optional Bearer auth is sparq's; per-user authz → gateway/sparq-solid. | `research/threat-model.md`, `Dockerfile` header |
| TLS/transport + at-rest encryption is the operator's | sparq emits a sniffable-token warning; TLS terminates at the operator's proxy. | `Dockerfile` header, `crates/sparq-server/README.md` |
| CE marking / EU declaration of conformity is **not** claimed | A manufacturer organizational/legal act; this tree provides only the supporting evidence. | [`controls.md`](./controls.md) §"formal conformity layer" |

## E8 — Article 14 reporting: audit-ready *documentation*, not proven *capability* `[OPUS-4.8]`

The Article 14 runbook is a genuine, inspectable artifact — but it is **audit-ready
documentation**, **not** an incident-response capability proven in a drill. The honest line:

| Item | Status | Note |
|---|---|---|
| The reporting **runbook** (timeline, ENISA/CSIRT routing, content checklist, CVD coordination) | **audit-ready (doc delivered)** | [`incident-reporting-runbook.md`](./incident-reporting-runbook.md); cited from controls.md CRA-CA.5 + gap-register.md (GX-CRA-2 addressed). |
| A **named legal entity / steward of record** with the `<FILL-IN>` org details completed | **residual (external)** | The runbook is a template; the manufacturer/steward must instantiate it. Out of agent scope. |
| **CSIRT registration + ENISA single-reporting-platform account** | **residual (external)** | An operational registration with the authority; cannot be performed by the source tree, a CI job, or an agent. |
| A **tested escalation** (table-top / drill exercising the 24h clock) | **residual (external)** | Documentation ≠ a rehearsed capability; the adopting org owns the drill + lessons-learned loop (runbook §5 step 9). |
| Pointer **from `SECURITY.md`** (governance doc) to this runbook | **residual (governance-owned, sq-zbb5)** | `SECURITY.md` is a root governance file outside the `compliance/cra` tree's write-scope; the one-line pointer is tracked as the residual leg of the cross-reference — proposed text in the runbook §"Cross-reference into the canonical governance doc". |

## Verification quick-run

```sh
# Supply-chain gates (the "no known exploitable vuln" + integrity spine)
cargo deny check bans sources licenses     # always-gating subset
cargo deny check advisories                # un-degraded PR-time gate
cargo vet --locked                         # per-dependency audit ratchet

# SBOM + VEX
cargo cyclonedx --all --format json        # CI SBOM shape
cat supply-chain/vex.cdx.json              # checked-in VEX (mirrors deny.toml ignores)

# Release-artifact provenance (on a published release)
gh attestation verify <archive> --repo sparq-org/sparq
shasum -a 256 -c SHA256SUMS

# Disclosure channel
cat .well-known/security.txt               # RFC 9116; Expires must be in the future
```
