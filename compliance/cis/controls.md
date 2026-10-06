<!-- [OPUS-4.8] sq-toze (cert-cis) — CIS control table. Authored while Fable unavailable —
     re-review when Fable returns. Engineer↔auditor loop (epic sq-toze). -->

# CIS — control table

**Scope:** CIS Critical Security Controls v8 (artifact-applicable Safeguards) + CIS Docker
Benchmark v1.x (§4 image-build half). See [`README.md`](./README.md) for the scoping decisions and
the status legend (PASS / AUDIT-READY / OPEN-gap / N/A-operator). Evidence paths are repo-relative;
all of them are reproduced in [`evidence.md`](./evidence.md).

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
> Honesty note. No row below is satisfied by the ZK/MPC estate. [OPUS-4.8] Its v1 ZK verifier was
> originally found unsound (`research/zk-soundness-audit.md`), then `sq-1s2` landed the binding
> layer and an internal re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found the prior
> findings closed → "sound as landed for the assumed threat model" — but **internal/single-model
> only, external sign-off PENDING (`sq-qhy4`), no production guarantee** (`SECURITY.md`). CIS is
> artifact-hygiene and credits no production crypto guarantee from that estate.

## Part A — CIS Docker Benchmark §4 (Container Images and Build File)

Verified **against the actual `Dockerfile`** (read in full, lines 1–106) — not the comments alone.

| # | CIS Docker item | Status | Evidence (file / line / CI job) | Owner |
|---|---|---|---|---|
| D-4.1 | **Create a user for the container** (do not run as root). | **PASS** | Runtime stage `FROM gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae…` (`Dockerfile:76`). The `:nonroot` distroless variant runs as UID **65532** (non-root) by default — no `USER root` override anywhere in the runtime stage; the `ENTRYPOINT` (`Dockerfile:104`) inherits that UID. | sparq |
| D-4.2 | **Use trusted base images** (pin by digest, minimal). | **PASS** | Both stages pinned **by digest**: builder `rust:1.88-slim-bookworm@sha256:38bc…` (`Dockerfile:51`), runtime `distroless/cc-debian12:nonroot@sha256:b0ae…` (`Dockerfile:76`). Runtime is distroless (glibc + libgcc only). Scorecard `PinnedDependencies` covers this. | sparq |
| D-4.3 | **Do not install unnecessary packages** (minimise the image). | **PASS** | Runtime stage installs nothing: it is distroless (no `apt`, no package manager, no shell). Only artifact copied in is the single static-ish server binary (`COPY --from=builder … /usr/local/bin/sparq-server`, `Dockerfile:84`). Builder packages stay in the discarded builder stage. | sparq |
| D-4.4 | **Scan and rebuild images to include security patches.** | **OPEN-gap** (GX-12) | No Trivy/Grype image-CVE scan lane in CI (verified: `grep -rIl -E 'trivy\|grype' .github/` → only false-positive comment hits). Daily `dependency-monitoring.yml` watches *cargo* advisories, and SHA-pinned bases are bumped deliberately, but the *image's OS layer* is not CVE-scanned. Bead **sq-toze.31**. | sparq |
| D-4.5 | **Enable Content Trust / verify image provenance.** | **PASS** | Pushed image carries buildkit max-mode SLSA provenance + an embedded SBOM (`release.yml` `docker:` job — `provenance: mode=max`, `sbom: true`). Verifiable with `gh attestation verify` / cosign. (Full SLSA-level claim lives in the `slsa` slice.) | sparq |
| D-4.6 | **Add HEALTHCHECK instruction to the container image.** | **PASS** (was GX-13, closed by sq-toze.36) | The `Dockerfile` declares `HEALTHCHECK … CMD ["/usr/local/bin/sparq-server", "--health-probe"]`. Distroless has no shell/`curl`/`wget`, so the check is the server binary probing its own loopback `/health` (`crates/sparq-server/src/health_probe.rs`) and exiting 0/non-zero; exec-form (no shell). Bead **sq-toze.36** (RESOLVED). | sparq |
| D-4.7 | **Do not use `latest` tag** (use a versioned/pinned tag for FROM). | **PASS** | Both `FROM`s are digest-pinned (D-4.2). The *published* image gets a `latest` tag for consumers (`release.yml` metadata `type=raw,value=latest`) **alongside** semver tags — that is the consumer-facing tag, not a `FROM`; the build itself never depends on a floating tag. | sparq |
| D-4.8 | **Remove setuid/setgid permissions** from the image. | **PASS (by construction)** | The runtime image contains only the distroless base + one server binary copied with default perms; no setuid/setgid bit is set on the binary, and distroless ships no setuid utilities (no shell/coreutils). No `chmod u+s` anywhere. | sparq |
| D-4.9 | **Use COPY instead of ADD.** | **PASS** | Every file-bring-in is `COPY` (`Dockerfile:57`, `:84`); `grep -n '^ADD ' Dockerfile` → none. | sparq |
| D-4.10 | **Do not store secrets in the image / Dockerfile.** | **PASS** | No secret material in the `Dockerfile` (only `ARG CARGO_FLAGS` + the `SPARQ_ALLOW_REMOTE=1` boot flag, which is a *posture* env, not a secret). Auth tokens are passed at `docker run -e SPARQ_AUTH_TOKEN=…` (`Dockerfile:29–31`), never baked. `.dockerignore` excludes `.git/`, `.claude/`, secrets-bearing dev state. | sparq |
| D-4.11 | **Install verified packages only** (the build uses a locked, reproducible dep set). | **PASS** | Builder runs `cargo auditable build --release --locked -p sparq-server` (`Dockerfile:70`) — `--locked` builds exactly the committed `Cargo.lock`; `cargo install --locked cargo-auditable` (`Dockerfile:64`) pins the tool too. Dependency trust is gated by `cargo-deny` + `cargo-vet` (`supply-chain.yml`). | sparq |
| D-5.x | **Container runtime hardening** (`--read-only`, `--cap-drop=ALL`, `--pids-limit`, non-root `--user`, seccomp, `--security-opt`). | **N/A (operator)** | Runtime flags are set at `docker run`, not in the image. The `Dockerfile` header (`:16–34`) + `crates/sparq-server/README.md` document the recommended runtime hardening (gateway/TLS, Bearer token, read-only data mount `-v …:/data:ro` shown at `:11`). Operator responsibility. | operator |

## Part B — CIS Controls v8 (artifact-applicable Safeguards)

Only the Safeguards that are a property of *sparq's source/CI/disclosure* are scored PASS/
AUDIT-READY; the rest are **N/A (operator)** because they govern the deploying organisation's IT
estate (accounts, network, endpoints, the Docker host). Each is mapped, not dropped.

| # | CIS v8 Safeguard | Status | Evidence (file / test / CI job) | Owner |
|---|---|---|---|---|
| C-2.1 | **2.1 Establish & maintain a software inventory** — for the *delivered artifact*, the dependency inventory. | **PASS** | CycloneDX SBOM per release (`release.yml` `sbom:` job → `scripts/gen-sbom-vex.sh`, attached + SLSA-attested) and in CI (`supply-chain.yml` `sbom:` job, `cargo cyclonedx --all`). `cargo auditable build` embeds the manifest *into the binary* (`Dockerfile:70`) → `cargo audit bin`. | sparq |
| C-2.4 | **2.4 Utilise automated software-inventory tooling.** | **PASS** | SBOM generation is automated in CI (`supply-chain.yml#sbom`) and on release (`release.yml#sbom`). Dependabot tracks 4 ecosystems (`.github/dependabot.yml`). | sparq |
| C-2.6 | **2.6 Allowlist authorised software / block unauthorised dependencies.** | **PASS** | `cargo-deny` **bans + sources + licenses** all GATING (`supply-chain.yml#deny`, `deny.toml`); `cargo-vet --locked` GATING (`supply-chain.yml#vet`) — an un-audited crate cannot enter silently. | sparq |
| C-3.x | **3.x Data protection** (classify, encrypt, retain the *data sparq processes*). | **N/A (operator)** | sparq is a data *engine*; the operator is the controller for the RDF they load and owns classification/retention/encryption-at-rest. Mapped in the `privacy` slice (`compliance/data-flow.md` / `dpia.md`). | operator |
| C-4.1 | **4.1 Establish & maintain a secure-configuration process** — for the *shipped image*. | **PASS** | Hardened, digest-pinned distroless `Dockerfile` (Part A D-4.1/4.2/4.3); secure-by-default server posture: a non-loopback bind is **refused** unless `SPARQ_ALLOW_REMOTE` is set, and the binary logs a loud no-auth WARNING at startup (`crates/sparq-server/src/main.rs:174,231–234`; `http.rs:175–180`). | sparq |
| C-4.2 | **4.2 Maintain secure configuration on infrastructure (the deployed host/runtime).** | **N/A (operator)** | The `docker run` flags, host OS, daemon config are the operator's. Image-side guidance: `Dockerfile:16–34`. | operator |
| C-6.x | **6.x Access-control management** (accounts/MFA/least-privilege) — for the *running service's users*. | **N/A (operator)** | `sparq-server` has, by documented design (threat-model boundary **B3**), no per-user auth; it ships an optional Bearer-token gate (`SPARQ_AUTH_TOKEN[_READ]`) and tells the operator to front it with a gateway / sparq-solid (`Dockerfile:33`, `http.rs`). End-user IAM is the operator's. Mapped explicitly in the `asvs` slice (the B3 decision). | operator |
| C-6.8 | **6.8 Define & maintain role-based access control — for the *project's* CI/repo.** | **PASS (project-side)** | `CODEOWNERS` mandates review by owners; branch protection requires the `ci-fast` check (per `feedback-pr-workflow`); GitHub permissions are least-privilege (`permissions: contents: read` default in `ci.yml`, scoped opt-ins per job, e.g. `release.yml` `docker:` only adds `packages: write`). | sparq |
| C-7.1 | **7.1 Establish & maintain a vulnerability-management process.** | **PASS** | `SECURITY.md` (GHSA + email, response targets) + RFC 9116 `.well-known/security.txt`; the `cra` slice maps the coordinated-disclosure obligation. Disclosure channel is machine-discoverable. | sparq |
| C-7.3 | **7.3 Perform automated OS/application patch management — for the *delivered artifact's deps*.** | **PASS** | Dependabot (4 ecosystems, `.github/dependabot.yml`) opens dependency-bump PRs; SHA-pinned actions/bases are bumped deliberately. | sparq |
| C-7.5/7.6 | **7.5/7.6 Perform automated application + infrastructure *vulnerability scanning*.** | **PARTIAL** | *Application/dependency* side PASS: `cargo-deny check advisories` GATING (`supply-chain.yml#deny`, un-degraded — GX-1 closed) + daily `dependency-monitoring.yml` advisory watchdog + CodeQL SAST (`codeql.yml`, gating). **Container-IMAGE** side **OPEN-gap** — no Trivy/Grype image scan (GX-12, bead **sq-toze.31**). | sparq |
| C-7.7 | **7.7 Remediate detected vulnerabilities.** | **AUDIT-READY** | Process documented (`SECURITY.md` response targets, `compliance/policies/` vuln-management template); the *evidence of an SLA met over time* is an organisational record an external assessor reviews. | sparq + operator |
| C-16.1 | **16.1 Establish & maintain a secure application-development process (SSDLC).** | **PASS (cross-ref)** | Full SSDLC mapping in the `ssdf` slice (NIST SP 800-218); gates: clippy `-D warnings`, conformance/SHACL/inference ratchets, coverage ratchet, Miri, fuzz, the unsafe-count ratchet — all in `ci.yml`/`fuzz.yml`/`miri.yml`. `research/threat-model.md` (STRIDE). | sparq |
| C-16.11 | **16.11 Leverage vetted modules/services (supply-chain).** | **PASS** | `cargo-vet` per-dependency audit attestations GATING (`supply-chain.yml#vet`). | sparq |
| C-16.12 | **16.12 Implement code-level security checks (SAST).** | **PASS** | CodeQL (`codeql.yml`, gating, results to code-scanning); clippy `-D warnings` (`ci.yml#lint`). | sparq |
| C-16.13 | **16.13 Conduct application penetration testing — incl. fuzzing of the hostile-input surface.** | **PASS (fuzz) / AUDIT-READY (external pentest)** | cargo-fuzz over the RDF/SPARQL/SHACL/mmap surfaces (`fuzz/`, `fuzz.yml` PR smoke + nightly); the SHACL-diff differential fuzzer (`shacl-diff-fuzz.yml`). An *external* pentest engagement is an operator/org procurement, labelled. | sparq + operator |
| C-16.14 | **16.14 Conduct threat modelling.** | **PASS** | `research/threat-model.md` — STRIDE, boundaries B1–B5. | sparq |
| C-18.x | **18.x Penetration-testing *program*** (red team, scoped engagements). | **N/A (operator)** | An organisational program, not a source property. The artifact-side residual (fuzzing) is C-16.13. | operator |
| C-1 / 5 / 9 / 10 / 13 / 14 / 17 | **Enterprise asset inventory, account mgmt, email/web protections, malware defenses, network monitoring, awareness training, incident-response program.** | **N/A (operator)** | Organisational/endpoint/network controls of the *deploying org*, not properties of a Rust library + image. Listed here so the auditor sees they were considered, not dropped. | operator |

## Summary

- **Docker Benchmark §4:** 10 PASS, 1 OPEN-gap (D-4.4 image-CVE-scan = GX-12; D-4.6 HEALTHCHECK =
  GX-13 now CLOSED by sq-toze.36 via the in-binary `--health-probe`), §5 runtime N/A-operator. The
  image is genuinely hardened; the one remaining hole is *scanning*, not hardening.
- **CIS v8 (artifact-applicable):** the software-inventory (2.x), secure-config of the artifact
  (4.1), vuln-management/disclosure (7.1/7.3), CI access-control (6.8), and SSDLC/SAST/threat-model/
  fuzz (16.x) Safeguards are PASS; 7.5/7.6 is PARTIAL pending GX-12; the org/endpoint/network
  families are honestly N/A-operator.
- **The single material technical gap is GX-12** (no container-image CVE scan + no Dockerfile
  linter). Everything else is PASS, AUDIT-READY, or N/A-operator with a pointer.
