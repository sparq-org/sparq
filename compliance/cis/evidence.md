<!-- [OPUS-4.8] sq-toze (cert-cis) — CIS evidence pack (re-runnable verification). Authored
     while Fable unavailable — re-review when Fable returns. Engineer↔auditor loop (sq-toze). -->

# CIS — evidence pack

Each PASS / OPEN-gap claim in [`controls.md`](./controls.md) is reproduced here as a command an
auditor can re-run from the **repo root** plus the output to expect. NON-CANONICAL timing (this
session runs on an EC2 work box) — none of these are timing claims; they are presence/structure
checks. Commands are deterministic source/CI scans (no build needed) unless noted.

## E-1 — Container scan + Dockerfile-linter lane is ABSENT (the headline gap, GX-12)

```sh
grep -rIl -E 'trivy|grype|dockle|hadolint|docker.?scout|anchore' .github/workflows/
```

Expected: only `zk-toolchain.yml` and `ci-summary.yml` match, **and both are false positives** —
verify by reading the matched lines:

```sh
grep -rInE 'trivy|grype|dockle|hadolint|docker.?scout|anchore' .github/workflows/
#  zk-toolchain.yml:31:  ... "anchored" (substring of 'anchore')   <- comment, not a tool
#  ci-summary.yml:145:   ... job literally named `gate` ... "anchored"  <- comment
```

Conclusion: **no** Trivy/Grype image-CVE scan and **no** Dockle/Hadolint Dockerfile linter exists.
The only container-touching CI is the *smoke* run (E-7), which proves the image *boots and serves*,
not that it is CVE-free or CIS-Benchmark-linted. → GX-12 (bead **sq-toze.31**).

## E-2 — Runtime stage is non-root distroless, digest-pinned (D-4.1 / D-4.2 / D-4.7)

```sh
grep -nE '^FROM ' Dockerfile
#  Dockerfile:51:  FROM rust:1.88-slim-bookworm@sha256:38bc5a86...d89 AS builder
#  Dockerfile:76:  FROM gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae8e98...985
```

Both `FROM`s pin a **digest** (`@sha256:…`), not a floating tag → D-4.2/D-4.7. The runtime base is
`distroless/cc-debian12:nonroot`; the `:nonroot` distroless variant's default user is UID **65532**
(documented by the distroless project), and there is no `USER root` in the runtime stage:

```sh
grep -nE '^USER ' Dockerfile        # → no output (no override back to root)
```

→ D-4.1 PASS (runs non-root by default).

## E-3 — Minimal runtime: distroless, no shell/package-manager, single binary (D-4.3 / D-4.8)

```sh
# Nothing installed in the runtime stage; only the server binary is copied in:
grep -nE '^(RUN|COPY|ADD) ' Dockerfile
#  Dockerfile:57:  COPY . .                                   (builder context)
#  Dockerfile:64:  RUN cargo install --locked cargo-auditable (builder only)
#  Dockerfile:70:  RUN cargo auditable build --release ...    (builder only)
#  Dockerfile:84:  COPY --from=builder .../sparq-server /usr/local/bin/sparq-server
```

All `RUN`/install steps are in the **builder** stage (discarded); the runtime stage only `COPY`s
the single binary from the builder. Distroless ships no `apt`, shell, or coreutils → no setuid
utilities (D-4.8), minimal surface (D-4.3).

## E-4 — COPY not ADD; no secrets in the image (D-4.9 / D-4.10)

```sh
grep -nE '^ADD ' Dockerfile          # → no output → D-4.9 PASS (COPY only)
grep -niE 'secret|password|token|api[_-]?key|BEGIN .*PRIVATE KEY' Dockerfile
#  → only the security-note comments explaining that SPARQ_AUTH_TOKEN is passed at
#    `docker run -e`, NEVER baked (Dockerfile:27-31). No secret VALUE present.
```

`.dockerignore` keeps secrets-bearing dev state out of the build context:

```sh
grep -nE '\.git/|\.claude/|\.vscode/|\.idea/' .dockerignore   # all excluded
```

→ D-4.10 PASS.

## E-5 — Locked, auditable build (D-4.11 / C-2.1)

```sh
grep -nE 'cargo (auditable )?(install|build).*--locked' Dockerfile
#  Dockerfile:64:  RUN cargo install --locked cargo-auditable
#  Dockerfile:70:  RUN cargo auditable build --release --locked -p sparq-server ${CARGO_FLAGS}
```

`--locked` builds exactly the committed `Cargo.lock`; `cargo auditable` embeds the dependency
manifest into the binary so the shipped image is self-describing (`cargo audit bin
/usr/local/bin/sparq-server`). → D-4.11 + the artifact-side of C-2.1.

## E-6 — Image provenance + SBOM on push (D-4.5 / C-2.4)

```sh
grep -nE 'provenance:|sbom:|attest-build-provenance' .github/workflows/release.yml
#  release.yml: docker: job →  provenance: mode=max   /   sbom: true
#  release.yml: archives + SBOM also actions/attest-build-provenance (SLSA)
```

The pushed `ghcr.io/sparq-org/sparq-server` carries max-mode SLSA provenance + an embedded SBOM,
verifiable with `gh attestation verify` / cosign. → D-4.5.

## E-7 — Image boots + serves (the existing smoke gate; NOT a CVE scan)

```sh
grep -nE 'docker-smoke|docker run|/health' .github/workflows/ci.yml
#  ci.yml: docker-smoke: job builds the image (build-push-action, load:true) then runs
#          scripts/docker-smoke.sh sparq-server:smoke 3030  (docker run + curl /health + ASK)
```

This is the gate that makes a container which exits immediately un-shippable (it caught the sq-n6rv
fail-closed-bind regression). It is **functional**, not a security scan — explicitly distinguished
from GX-12 so the smoke job is never mistaken for vuln coverage.

## E-8 — Secure-by-default server config + loud no-auth warning (C-4.1)

```sh
grep -nE 'no.?auth|ALLOW_REMOTE|WARN' crates/sparq-server/src/main.rs crates/sparq-server/src/http.rs
#  main.rs:174  a non-loopback --addr is refused unless ... SPARQ_ALLOW_REMOTE / auth
#  main.rs:231-234  Unset = no auth ... loud startup note
#  http.rs:175-180  "By default the server has no authentication on any endpoint"
```

A non-loopback bind is **refused** (exits non-zero) unless explicitly opted in, and the binary logs
a loud no-auth WARNING at startup → secure-config of the artifact (C-4.1). The *per-user IAM* hole
is the documented B3 boundary (operator responsibility; mapped in the `asvs` slice), not a silent
CIS gap.

## E-9 — Dependency vuln scanning + allowlisting (C-2.6 / C-7.5 application side)

```sh
grep -nE 'deny check|cargo vet|cyclonedx' .github/workflows/supply-chain.yml
#  supply-chain.yml#deny:  cargo deny check bans sources licenses   (GATING)
#                          cargo deny check advisories              (GATING — GX-1 closed)
#  supply-chain.yml#vet:   cargo vet --locked                       (GATING)
#  supply-chain.yml#sbom:  cargo cyclonedx --all --format json
```

Plus daily `dependency-monitoring.yml` advisory watchdog and CodeQL SAST (`codeql.yml`). The
*application/dependency* half of CIS 7.5/7.6 is covered; only the *image-OS* half is the GX-12 gap.

## E-10 — CI access control + least-privilege (C-6.8)

```sh
head -1 CODEOWNERS                                  # owners-required review
grep -nE 'permissions:|contents: read' .github/workflows/ci.yml | head
#  ci.yml:  permissions: contents: read   (default least-privilege; per-job opt-ins only)
```

Branch protection requires the `ci-summary / gate` aggregator (governance per
`feedback-pr-workflow`); the default workflow token is read-only with scoped per-job escalation
(e.g. `release.yml` `docker:` adds only `packages: write`). → C-6.8 PASS (project-side).

## E-11 — Disclosure channel is machine-discoverable (C-7.1)

```sh
ls SECURITY.md .well-known/security.txt
head -2 .well-known/security.txt   # RFC 9116 channel pointing at SECURITY.md
```

→ C-7.1 PASS (GHSA + email + RFC 9116 `.well-known/security.txt`, GX-3 closed).

## Reproduction note

All of E-1…E-6, E-8…E-11 are pure source/CI scans (no Docker daemon, no build) and run in seconds.
E-7's *execution* needs a Docker daemon (`scripts/docker-smoke.sh`); the *presence* of the gate is a
source scan. No claim here depends on a benchmark timing.
