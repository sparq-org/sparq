---
name: rust-supply-chain-attestation
description: How sparq attests its Rust/cargo supply chain — cargo-deny (advisories/bans/licenses/sources), cargo-vet/cargo-auditable, CycloneDX SBOM + VEX, SLSA build provenance, and OpenSSF Scorecard — and which evidence each tool produces for the SBOM, SLSA, NIST SSDF, EU CRA, and OpenSSF certification frameworks. Use when working any of the sbom/slsa/ssdf/cra/openssf certification worktrees, when adding or changing a supply-chain CI lane, when filling out an SBOM/VEX or a SLSA-level claim, or when asked what evidence a given framework wants from a cargo workspace. Grounded in sparq's real deny.toml + supply-chain.yml + scorecard.yml + dependency-monitoring.yml + release.yml.
---

# Rust supply-chain attestation (sparq)

How sparq proves the integrity of what it ships — a Rust **library + binaries +
container** consumed as a dependency on crates.io / npm (WASM) / PyPI / ghcr.
The dominant risk for a dependency is **supply-chain integrity + build
provenance**, so this is the highest-fit framework family. This skill is the
cargo-specific recipe so the `sbom`, `slsa`, `ssdf`, `cra`, and `openssf`
engineers do not re-derive it.

> NON-CANONICAL timing. No measured numbers belong in this file.

## The tools, what they prove, and which framework consumes the evidence

| Tool | What it asserts | Evidence artifact | Frameworks |
|---|---|---|---|
| **cargo-deny** (advisories) | No known-vulnerable / yanked dependency in the tree | CI log + the daily tracking issue | SBOM, SSDF (RV), CRA (vuln-handling) |
| **cargo-deny** (bans) | No banned/duplicate-version crate sneaks in | CI log | SSDF (PW), SBOM |
| **cargo-deny** (licenses) | Every dependency is permissive / explicitly excepted | CI log + `deny.toml` allowlist | SBOM (NTIA "license"), ISO 27001, CRA |
| **cargo-deny** (sources) | Crates come only from crates.io (no rogue git/registry) | CI log + `deny.toml [sources]` | SLSA, SSDF (PS), SBOM (supplier) |
| **CycloneDX SBOM** (`cargo-cyclonedx`) | The full component inventory of the build | `*.cdx.json` | SBOM (the artifact itself), CRA (Annex I §2), SSDF (PS.3) |
| **VEX** (CycloneDX/CSAF) | Why a flagged advisory is *not exploitable* in sparq | `*.vex.json` alongside the SBOM | SBOM, CRA (vuln-handling) |
| **cargo-auditable** | The exact dependency manifest is embedded *inside* the binary | `cargo audit bin <file>` reads it | SLSA, SBOM, SSDF |
| **cargo-vet** | Each dependency carries a human audit attestation | `supply-chain/audits.toml` | SLSA (higher assurance), SSDF (PS) |
| **attest-build-provenance** | This binary was built by *this* workflow run (Sigstore-signed) | `gh attestation verify` | SLSA (the core L2 claim), CRA, SSDF |
| **OpenSSF Scorecard** | Aggregate posture score (pinned deps, branch protection, SAST…) | SARIF in Security tab + public dashboard | OpenSSF, SSDF |

## How each is wired in sparq today (ground truth — cite these, don't re-propose)

### cargo-deny — `deny.toml` + `.github/workflows/supply-chain.yml`
The policy lives in `deny.toml` (schema targets cargo-deny >= 0.18, "version 2"
advisories/licenses). The `supply-chain.yml` `audit` job runs on the **host
stable toolchain** (not the EmbarkStudios Docker action — its bundled cargo 1.83
can't parse the vendored `spargebra`, which needs `edition2024` / cargo >= 1.85).

- **GATING now:** all four checks — `cargo deny check advisories bans sources licenses`
  — on push + PR, skipped only when the PR touches no Rust/dependency-graph file
  (the `rust_changed` step guard; `merge_group` is deliberately excluded).
- **Licenses:** an allowlist of permissive licenses only (MIT, Apache-2.0, BSD-*,
  ISC, Unicode-3.0, CC0-1.0, Zlib, BlueOak-1.0.0) plus **per-crate exceptions**
  (CeCILL-B for the sophia/HDT tree, AFL-3.0 for `ntriple`, MPL-2.0 for `resiter`,
  Apache-2.0-WITH-LLVM-exception, CDLA-Permissive-2.0 for `webpki-roots`). Each
  exception is scoped to one crate so the broad allowlist stays permissive-only.
- **Sources:** `unknown-registry = "deny"`, `unknown-git = "deny"`, only
  crates.io allowed. The vendored `spargebra` is a `[patch.crates-io]` PATH source
  (a path patch is not a registry/git source, so it doesn't trip the gate);
  `allow-git = []` is the hook for any deliberately-allowed future git source.
- **Advisories ignore list:** reserved for `unmaintained` *informational* advisories,
  and for advisories reachable only through a transitive dep with no fixed upstream
  release — each with a `reason` + tracking bead. A real, fixable **vulnerability** or a
  **yanked** crate FAILS the gate. Read `deny.toml [advisories].ignore` for the live set;
  do not quote a count from here. **Prefer removing the dependency to ignoring it** —
  the ignore list has shrunk that way three times: `paste` via wgpu (sq-l8bv,
  RUSTSEC-2024-0436) left when the GPU stack dropped it (it later returned via parquet);
  `rustls-pemfile` via ureq (sq-g2xs, RUSTSEC-2025-0134) left when the federation HTTP
  client moved ureq 2 → ureq 3; and when the solid-server-rs import brought
  `rustls-pemfile` back for its mTLS PEM parse, sq-5ah3p ([OPUS-5]) removed it again by
  migrating that parse to `rustls-pki-types`' `PemObject` — dropping the ignore, the VEX
  statement and the cargo-vet exemption in ONE change, which is what the `vex-deny-sync`
  gate requires. The gate re-flags any regression that reintroduces the crate.

### The advisories PR-gate (GX-1 — closed)
`cargo deny check advisories` is **GATING** — `supply-chain.yml`'s
`cargo-deny check (advisories) — GATING` step carries no `continue-on-error`
(the file contains none). The CVSS-4.0 parse blocker that once forced
`continue-on-error` — cargo-deny failing to *load* the freshly-cloned RustSec DB
("unsupported CVSS version: 4.0"), a DB-wide error before any per-advisory
`ignore` applied — is **resolved** (bead **sq-q8de**): cargo-deny >= 0.19 parses
CVSS v4.0, so the DB loads and the `ignore` rules apply as intended. The policy
is fail-closed (`deny.toml`: `yanked = "deny"`, advisories v2 ⇒ every unignored
advisory fails), so a real vulnerability or a yanked crate blocks the PR. The
daily watchdog (`dependency-monitoring.yml`) is **defence in depth**, not the
gate: it runs advisories-only off-peak and opens/updates a single idempotent
`security:dependency-vuln` tracking issue for advisories disclosed *between* PRs
on an unchanged graph. When an auditor asks "is there PR-time vuln gating?", the
honest answer is *yes* — cite that step.

### CycloneDX SBOM — `supply-chain.yml` `sbom` job
`cargo cyclonedx --all --format json` emits one SBOM per workspace member plus an
aggregate (`**/*.cdx.json`), uploaded as a CI **artifact**. Gaps the `sbom`/`cra`
engineers must close (GX-2): there is **no checked-in / per-release SBOM** and
**no VEX** yet. CRA/SBOM want a *published, per-release* SBOM with VEX to justify
non-applicable advisories. Map the SBOM to the **NTIA minimum elements**
(supplier, component name, version, unique identifier/PURL, dependency
relationship, author, timestamp) when scoring.

### SLSA build provenance — `release.yml`
- `actions/attest-build-provenance` (SHA-pinned) signs (Sigstore) an attestation
  binding each packaged archive's digest to the workflow run; verify with
  `gh attestation verify <file>`. Plus `SHA256SUMS` over all archives.
- The container build uses buildkit `provenance: mode=max` + `sbom: true`,
  attaching SLSA provenance + an embedded SBOM to the ghcr.io image (verifiable
  via cosign / gh).
- **Honest level:** ≈ **SLSA L2** on GitHub-hosted runners (signed provenance,
  hosted build platform). The gaps to higher (GX-7/GX-8): no `cargo-auditable`
  (binaries don't embed their dependency manifest), no `cargo-vet`, and no
  reproducible-build evidence. State the level *honestly* and name the gap —
  never inflate to L3.

### OpenSSF Scorecard — `scorecard.yml`
Runs on push-to-main + weekly; `publish_results: true` uploads to the public
OpenSSF dashboard (required for the Scorecard badge) and to code-scanning (SARIF).
Gaps for the `openssf` worktree: the **Best-Practices (bestpractices.dev / CII)**
questionnaire isn't filled (GX-4), and there's no `.well-known/security.txt`
(RFC 9116) machine-discoverable disclosure pointer (GX-3).

## How to use this when working a certification framework

1. **Don't re-propose existing controls.** The list above is the live posture;
   cite the file (`deny.toml:line`, `supply-chain.yml:job`) as evidence.
2. **Gap-fixes land test-first.** Use the `test-driven-development` skill — a new
   CI lane or a checked-in SBOM lands with a verifying check before the change.
3. **Frameworks → evidence:** SBOM → the `*.cdx.json` + NTIA mapping + VEX; SLSA →
   the provenance attestation + the honest level statement; SSDF → map PO/PS/PW/RV
   to these tools; CRA → SBOM + coordinated-disclosure discoverability +
   security-update channel; OpenSSF → Scorecard score + Best-Practices badge.
4. **Honesty contract.** Never let a control table launder a gap (the missing
   per-release SBOM/VEX, the SLSA-L2-not-L3 reality) into a "done". Record each
   gap as a bead, not a silent pass.

## Local commands
```
cargo install --locked cargo-deny cargo-cyclonedx cargo-auditable cargo-audit
cargo deny check                 # advisories + bans + sources + licenses
cargo deny check bans sources licenses   # the non-advisory-DB subset (offline)
cargo cyclonedx --all --format json      # CycloneDX SBOMs
gh attestation verify <archive> --owner jeswr   # SLSA provenance check
```

<!-- [OPUS-4.8] Authored for bead sq-toze.1 (epic sq-toze, cert framework). Grounded in
deny.toml + .github/workflows/{supply-chain,scorecard,dependency-monitoring,release}.yml.
Re-review when Fable returns. -->
