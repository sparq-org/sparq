<!-- [OPUS-4.8] sq-toze — ISO/IEC 27001 evidence pack: re-runnable verification of each
     technical (IMPL) control claim in controls.md. Authored while Fable unavailable.
     NON-CANONICAL timing (EC2 box) — no times recorded. -->

# ISO/IEC 27001:2022 — evidence pack

Re-runnable verification for each **IMPLEMENTED & VERIFIED** (and the technical substrate
of each **AUDIT-READY**) control in [`controls.md`](./controls.md). Paths are repo-relative;
commands are run from the repo root. No timing is recorded (NON-CANONICAL EC2 box). An
auditor re-runs the command and re-reads the cited file to confirm the claim.

> **AUDIT-READY caveat.** A passing command below confirms the *technical substrate* of an
> AUDIT-READY control (the doc-of-record or CI lane exists and behaves as claimed). It does
> **not** confirm the *certificate* — that requires an accredited ISMS audit. See README.

## Confined unsafe surface (A.8.27 / A.8.28)

```sh
grep -rl 'forbid(unsafe_code)' crates/ --include='*.rs' | sed 's#crates/##;s#/src.*##' | sort -u | wc -l
```
→ **26** crates (of 31 total) carry `#![forbid(unsafe_code)]` (verified on this branch via
the per-crate-deduped count above); the 5 with `unsafe` are sparq-core/-vectors/-cli/
-zk-compose/-bench. A new `unsafe` in a forbid crate fails to compile. The confined unsafe
surface and its per-site register are
the `memsafety` framework's deliverable — `compliance/memsafety/unsafe-register.md`. This
control points there rather than re-deriving it.

## Lint + secure-coding gate (A.8.28 Secure coding)

```sh
grep -n 'cargo clippy' .github/workflows/ci.yml
```
→ `cargo clippy --workspace … -- -D warnings` is present and **not** `continue-on-error`
(the `lint` job, "clippy (deny warnings)"). The secure-coding *standard* of record is
`CONTRIBUTING.md` §"Secure coding" (read lines under that heading).

## SAST (A.8.7 / A.8.25 / A.8.28)

```sh
grep -nE 'queries:|languages:' .github/workflows/codeql.yml
```
→ CodeQL runs `security-and-quality`. Workflow `.github/workflows/codeql.yml`.

## Vulnerability management — advisory gating (A.8.8 / A.8.22)

```sh
grep -nE 'cargo deny check|continue-on-error' .github/workflows/supply-chain.yml
```
→ `cargo deny check advisories` and `cargo deny check bans sources licenses` are both
labelled **GATING** with **no `continue-on-error`** on the deny steps (GX-1 un-degraded).
Policy of record: `deny.toml` (read `[advisories]` — each `ignore` carries a reason + a
tracking bead). `cargo vet --locked` is also GATING — a supply-chain **ratchet**, not a set
of attestations: every crate must be covered by a trusted imported audit set
(Mozilla/Google/ISRG/Embark/BCA/Zcash) or an explicit `[[exemptions]]` entry, so no new
un-covered dependency enters the tree silently. `supply-chain/audits.toml` is currently
empty — verify with `wc -c supply-chain/audits.toml` (35 bytes, 0 `[[audits]]`) and
`grep -cE 'exemptions\.' supply-chain/config.toml` (349 exemptions): the lane proves the
ratchet, **not** that sparq has first-party-attested any dependency.

```sh
sed -n '1,40p' .github/workflows/dependency-monitoring.yml
```
→ daily advisory watchdog (RustSec) — the standing threat-intelligence lane (A.5.7).

```sh
grep -n 'package-ecosystem' .github/dependabot.yml
```
→ Dependabot covers **cargo, github-actions, npm, pip** (4 ecosystems).

## SBOM + supply-chain provenance (A.5.21 / A.8.19)

```sh
grep -nE 'CycloneDX|cyclonedx|attest-build-provenance' .github/workflows/supply-chain.yml .github/workflows/release.yml
```
→ CycloneDX SBOM generated per build (`supply-chain.yml`); SLSA build provenance over the
release archives **and** over the SBOM+VEX via `actions/attest-build-provenance` (SHA-pinned
`@a2bbfa25…`) in `release.yml`, verifiable with `gh attestation verify <file>`. Detail +
NTIA-element mapping live in `compliance/sbom/` and the SLSA level claim in `compliance/slsa/`.

## SHA-pinned actions + pinned base image (A.8.9 Configuration management)

```sh
grep -nE 'uses: .*@[0-9a-f]{40}' .github/workflows/release.yml | head
grep -nE 'FROM .*@sha256:' Dockerfile
```
→ Actions are SHA-pinned (40-hex digest); the runtime base is a pinned distroless digest
(`gcr.io/distroless/cc-debian12:nonroot@sha256:…`). `Cargo.lock` is committed.

## Distroless non-root hardening (A.8.7 / A.8.18 / A.8.19)

```sh
grep -nE 'FROM|nonroot|distroless' Dockerfile
```
→ Final stage is `gcr.io/distroless/cc-debian12:nonroot` — **no shell, no package manager,
runs as non-root**, so no privileged utility programs (A.8.18) and a reduced malware
surface (A.8.7). CIS-Docker scan evidence is the `cis` framework (`compliance/cis/`).

## Release integrity (A.8.4 Access to source code / A.8.24 operational crypto)

```sh
grep -nE 'SHA256SUMS|attest|attestations: write' .github/workflows/release.yml
```
→ Release archives carry `SHA256SUMS` and a Sigstore-backed SLSA attestation binding each
archive digest to the workflow run. This is the **only** cryptographic guarantee claimed
under A.8.24 (signing of artifacts) — **NOT** the ZK/MPC estate.

## Branch protection + change management (A.5.36 / A.8.4 / A.8.32)

```sh
sed -n '1,40p' docs/branch-protection.md
```
→ doc-of-record for the `main` ruleset: PR-only (no direct push, incl. admins), required
review via `CODEOWNERS`, required `ci-fast` check. The ruleset itself is
configured **out-of-repo** on GitHub (org act — this is why A.5.3/A.8.4 enforcement is
AUDIT-READY at the *certificate* level even though the gate is technically live).

```sh
test -f CODEOWNERS && wc -l CODEOWNERS && sed -n '/High-risk/,$p' CODEOWNERS
```
→ `CODEOWNERS` lives at the **repo root** (a valid GitHub location, *not* `.github/`),
37 lines, populated. A catch-all `* @jeswr` plus explicit owner lines for the high-risk
paths (`/crates/sparq-zk*`, `/crates/sparq-mpc/`, `/crates/sparq-core/`,
`/crates/sparq-server/`, and the CI/workflows). This is the substrate for the
A.5.2/A.5.3/A.8.4 review-responsibility claims; the *certificate*-grade enforcement (the
branch-protection ruleset requiring CODEOWNERS review) is the org-configured ruleset of
record in `docs/branch-protection.md`, which is why those controls remain AUDIT-READY at
the certificate level. (Note: an earlier draft of this pack mis-checked `.github/CODEOWNERS`
and flagged a false gap; the file is at the root and is correct — no gap.)

## Conformance ratchets (A.8.29 Security testing / A.8.32 Change management)

The "never-lower" conformance floors (reported by `sparq-conformance` — the SPARQL
`conformance-report.md` is git-ignored and regenerated locally / published by CI, the
inference `inference-conformance-report.md` is committed — plus the SHACL floors,
`bench/perf-baseline.json`, and coverage floors) are described in `CONTRIBUTING.md`
§"The conformance-ratchet 'never lower' rule".
They are change-management acceptance criteria: a PR that regresses conformance cannot land.

## Security testing lanes (A.8.29)

```sh
ls .github/workflows/{miri,fuzz}.yml && grep -nE 'cargo miri|cargo fuzz|cargo-fuzz' .github/workflows/miri.yml .github/workflows/fuzz.yml | head
test -f crates/sparq-core/tests/mmap_corruption_oracle.rs && echo "corruption oracle present"
```
→ Miri UB lane (`miri.yml`), libFuzzer fuzz targets (`fuzz.yml`), and the deterministic
mmap corruption oracle. Detail is the `memsafety` framework.

## Incident management substrate (A.5.24–A.5.27 / A.6.8)

```sh
sed -n '1,55p' SECURITY.md
test -f .well-known/security.txt && sed -n '1,20p' .well-known/security.txt
```
→ `SECURITY.md` (private channels, 5/10-business-day targets, coordinated disclosure,
supported-versions, and the **research-scaffold NO-guarantee** caveats) + the RFC 9116
`.well-known/security.txt` machine-readable pointer (note its fixed `Expires` must be
refreshed annually).

## Threat model + application security requirements (A.8.26 / A.8.27)

```sh
sed -n '1,120p' research/threat-model.md
```
→ STRIDE model: assets (query-result integrity, memory-safety of the unsafe mmap path,
availability, dataset confidentiality, on-disk integrity, host env), boundaries B1–B5,
and the explicit **B3 no-auth** + **ZK-not-sound** exclusions. The `asvs` and `cra`
frameworks extend this; the `cryptoreview` framework owns the ZK/MPC verdict.

## Cryptography exclusion proof (A.8.24 honesty gate)

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
```sh
grep -nE 'remediated, but NOT externally audited|NO .*guarantee|research scaffold|external .*audit' SECURITY.md
sed -n '1,40p' research/zk-soundness-audit.md       # original UNSOUND verdict (kept on record)
sed -n '1,40p' research/zk-verifier-reaudit.md      # internal post-remediation re-audit
```
→ Confirms the reconciled verdict the A.8.24 row must **not** contradict: the v1 ZK verifier
was **originally found NOT sound** (`research/zk-soundness-audit.md`, kept on record for the
`sq-1gir` regression map), but `sq-1s2` landed the verifier-side binding layer and an
**internal** post-remediation re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all
prior findings closed → "sound as landed for the assumed threat model" [OPUS-4.8]. SECURITY.md's
heading now reads **"ZK verifier: remediated, but NOT externally audited"** (so the old grep for
`NOT sound` no longer matches — match the reconciled wording above). The re-audit is
**internal, single-model, read-only**; an **external accredited-cryptographer sign-off is STILL
PENDING** (`sq-qhy4`, P0, required before any production ZK claim) and there is **NO production
soundness/privacy/integrity guarantee** (`sparq-mpc` semi-honest-only). The A.8.24 control
claims **only** artifact-signing + operator TLS, never the ZK/MPC estate.

## Supplier / IP / license governance (A.5.19 / A.5.32 / A.8.8)

```sh
sed -n '1,60p' deny.toml
```
→ `deny.toml` codifies advisories/bans/sources/licenses policy (the dependency-supplier
"agreement"), gated in `supply-chain.yml`. License allow-list satisfies A.5.32 (IP).
