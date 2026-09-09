<!-- [OPUS-4.8] sq-toze (cert-cis) — CIS framework intro/scope. Authored while Fable
     unavailable — re-review when Fable returns. Engineer↔auditor loop (epic sq-toze). -->

# CIS (Center for Internet Security) — framework slice

**Frameworks mapped here**

- **CIS Critical Security Controls v8** — the Safeguards (Implementation Group 1/2) that are
  *properties of the artifact sparq ships* (a Rust library + `sparq-server` + a container image),
  not properties of the network/endpoint estate the **operator** runs.
- **CIS Docker Benchmark v1.x** (the *image-build* half, §4 "Container Images and Build File") —
  the hardening posture of the repo's `Dockerfile`. The *host/daemon/runtime* half (§1–3, §5–7:
  the Docker daemon config, host kernel, `docker run` flags) is **operator responsibility** and is
  mapped as OOS below, because sparq ships an image, not a Docker host.

**What sparq is, for CIS scoping**

sparq is a Rust RDF/SPARQL **data engine** consumed as a dependency, plus an HTTP `sparq-server`
and a SHA-pinned distroless container image (`Dockerfile` → `ghcr.io/sparq-org/sparq-server`). It is
**not** an enterprise IT estate. The CIS Controls v8 are written for an *organisation* securing its
asset/software inventory, accounts, networks, and endpoints — so a large fraction of v8 maps to the
**operator** who deploys sparq, not to the source/CI of sparq itself. This slice scopes each
Safeguard honestly into one of:

## Status legend

- **PASS** — *implemented & verified*: a technical control in the codebase/CI with a re-runnable
  evidence anchor (file path + line / test name / `.github/workflows/<wf>.yml#<job>`). The auditor
  re-reads or re-runs the cited artifact.
- **AUDIT-READY** — control + documentation in place, but the *certificate* needs an accredited
  external assessor / an organizational ISMS we cannot substitute for.
- **OPEN-gap** — not met (or only partially); recorded in [`gap-register.md`](./gap-register.md)
  with a `bd` bead. **Not** papered over.
- **N/A (operator)** — the Safeguard governs the *deployment environment* (the org's network,
  endpoints, accounts, the Docker host/daemon, the live `docker run` flags), which is the
  deploying operator's responsibility, not a property of sparq's source. Mapped, not silently
  dropped.

## What is OUT OF SCOPE for a library + server + image (and why)

- **CIS Docker Benchmark §1 (Host), §2 (Daemon), §3 (Daemon config files), §5 (Container
  runtime), §6 (Operations), §7 (Swarm)** — these govern the *Docker host and the `docker run`
  invocation*. sparq ships a hardened *image*; the operator chooses the host, the daemon flags, and
  the runtime flags (`--read-only`, `--cap-drop=ALL`, `--security-opt`, `--pids-limit`, a non-root
  `--user`, seccomp). The image header (`Dockerfile` lines 16–34) documents the runtime hardening
  the operator should apply; CIS §5 is **N/A (operator)** with that pointer.
- **CIS Controls v8 families that are organisational/endpoint** — IG-level account management (CIS
  5/6), malware defenses (CIS 10), network monitoring (CIS 13), email/web protections (CIS 9),
  security-awareness training (CIS 14), incident-response *org program* (CIS 17), penetration
  testing *program* (CIS 18) — these are the **operator's** IT program, not sparq's source. The
  thin residual that *is* a sparq property (e.g. the source-code/dependency *inventory*, CI access
  control, the published disclosure channel) is mapped PASS/AUDIT-READY in the control table.

## The one real technical gap (honest headline)

The `Dockerfile` hardening is genuinely strong and *verified against the actual file* (distroless
`cc-debian12:nonroot`, both stages SHA-pinned by digest, `cargo auditable build --locked`,
no shell/package-manager in the runtime layer, minimal labels-only metadata, `.dockerignore`
scoping the context). A `docker-smoke` job builds + runs the image and curls it on every PR. **But
there is no automated container-image vulnerability scan (Trivy/Grype) and no Dockerfile linter
(Dockle/Hadolint) lane in CI** — verified by `grep -rIl -E 'trivy|grype|dockle|hadolint' .github/`
returning only false-positive comment matches. That is gap **GX-12** (bead **sq-toze.31**); see
[`gap-register.md`](./gap-register.md). Everything else in this slice is PASS or honestly N/A.

## Files in this slice

- [`controls.md`](./controls.md) — per-Safeguard / per-Benchmark-item → status → evidence → owner.
  The spine the auditor checks.
- [`evidence.md`](./evidence.md) — the re-runnable verification commands + their expected output
  for each PASS claim (so a reviewer can reproduce, not just trust).
- [`gap-register.md`](./gap-register.md) — open gaps, severity, remediation, target, `bd` bead id.

## Honesty note (carried from the contract)

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
This slice makes **no** claim about the ZK/MPC estate. The `sparq-zk`/`sparq-mpc` crates are
research scaffolds with **no production security guarantee**. [OPUS-4.8] The v1 ZK verifier was
**originally found unsound** (`research/zk-soundness-audit.md`, kept on record); `sq-1s2` then
landed the verifier-side binding layer and an **internal post-remediation re-audit**
(`research/zk-verifier-reaudit.md`, `sq-gbp4`) found the prior findings closed → **"sound as
landed for the assumed threat model"** — but that verdict is **internal / single-model
self-review only, with external accredited-cryptographer sign-off still PENDING (`sq-qhy4`, P0)
and NO production guarantee** (`SECURITY.md`). No CIS Safeguard here is satisfied by, or implies a
guarantee from, that estate. CIS is about the *deliverable artifact's* hygiene (image,
dependencies, build, disclosure), which is independent of the crypto soundness question.
