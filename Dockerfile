# syntax=docker/dockerfile:1
# sparq-server container image (T20).
#
# Multi-stage: a rust:slim builder compiles the release binary; the runtime stage is
# distroless (cc-debian12 — glibc + libgcc only, no shell, no package manager), so the
# final image is small (~25 MB + binary) and has a minimal attack surface. Both stages
# are Debian 12 (bookworm), so the glibc the binary links against matches at runtime.
#
# Build:    docker build -t sparq-server .
#           (pass --build-arg CARGO_FLAGS="-j 2" to cap build parallelism)
# Run:      docker run --rm -p 3030:3030 -v "$PWD/data:/data:ro" sparq-server \
#             --format turtle /data/dataset.ttl
#           (no data file => starts with an empty default graph)
# Endpoint: http://localhost:3030/sparql   Health: http://localhost:3030/health
#
# [OPUS-4.8] sq-n6rv — SECURITY POSTURE OF THIS IMAGE (read before exposing the port):
#   This image sets ENV SPARQ_ALLOW_REMOTE=1 so it BOOTS on `docker run` out of the box:
#   without a published port the server is useless, and inside the container the only
#   reachable bind is the non-loopback 0.0.0.0 the ENTRYPOINT requests. Running the
#   container is itself the operator's explicit choice to expose a network surface, so
#   the bind is permitted — but the server has NO built-in auth in this default posture,
#   i.e. anyone who can reach the published port can READ AND WRITE the whole dataset.
#   The binary logs a loud no-auth WARNING at startup; do not ignore it.
#
#   For any non-trivial deployment, gate the surface with the Bearer token (#71) — the
#   server reads SPARQ_AUTH_TOKEN / SPARQ_AUTH_TOKEN_READ from the ENVIRONMENT (see
#   ServerConfig::from_env), so pass them straight through with `-e`, no flag wiring:
#     # writes gated, reads open (QLever-style):
#     docker run --rm -p 3030:3030 -e SPARQ_AUTH_TOKEN=$TOK sparq-server
#     # whole surface gated (writes AND reads require the token):
#     docker run --rm -p 3030:3030 -e SPARQ_AUTH_TOKEN=$TOK -e SPARQ_AUTH_TOKEN_READ=1 sparq-server
#   Deliver the token over TLS (terminate at a reverse proxy); a bare Bearer token on
#   plaintext HTTP is sniffable. For per-user authz, front it with a gateway / sparq-solid.
#   See crates/sparq-server/README.md -> "Running the container image (ghcr.io)".
#
# A CI smoke test (docker run + curl /health + a basic query) gates this image on every
# PR and before each release push, so a container that exits immediately can never ship —
# see .github/workflows/ci.yml (docker-smoke) and release.yml.
#
# CI builds + pushes this to ghcr.io on version tags — see .github/workflows/release.yml.

# -------- builder --------
# Pin a toolchain >= the workspace's rust-version (1.88, the declared MSRV floor in the
# root Cargo.toml [workspace.package] — `cargo build --locked` REFUSES an older toolchain).
# Bump deliberately, in lock-step with that floor.
# [OPUS-4.8] SHA-pinned for supply-chain integrity (Scorecard PinnedDependencies). The
# human-readable tag is kept on its OWN comment line above the FROM, NOT as a trailing
# inline comment: Docker only treats `#` as a comment at the start of a line, so an inline
# `# tag` on a FROM is parsed as extra arguments and fails with "FROM requires either one
# or three arguments". Tag: rust:1.88-slim-bookworm
FROM rust:1.88-slim-bookworm@sha256:38bc5a86d998772d4aec2348656ed21438d20fcdce2795b56ca434cf21430d89 AS builder

ARG CARGO_FLAGS=""
WORKDIR /build

# .dockerignore keeps the context to the workspace sources (no target/, bench data, .git).
COPY . .

# [OPUS-4.8] sq-toze.8 (GX-7): cargo-auditable embeds the dependency manifest into the
# server binary so the shipped image is self-describing for post-build audit
# (`cargo audit bin /usr/local/bin/sparq-server`). `--locked` here uses cargo-auditable's
# OWN crates.io lockfile (a reproducible tool install) — it does NOT pin to this workspace's
# Cargo.lock; the server binary itself is pinned by the `--locked` on the build step below.
RUN cargo install --locked cargo-auditable

# --locked: build exactly the committed Cargo.lock. The workspace release profile is
# fat-LTO + codegen-units=1 (the shipped-binary configuration), so this step is slow
# but produces the same binary the benchmarks measured. `cargo auditable build` is a
# drop-in for `cargo build` that embeds the dependency manifest (GX-7).
RUN cargo auditable build --release --locked -p sparq-server ${CARGO_FLAGS}

# -------- runtime --------
# [OPUS-4.8] SHA-pinned (Scorecard PinnedDependencies). Tag kept on its own comment line
# above the FROM (an inline trailing `# tag` breaks FROM's arg parser — see builder above).
#
# [OPUS-5] #2312: this digest is an OCI image INDEX, not a single-architecture manifest, so
# it is the correct pin for the multi-platform release push (release.yml builds
# `platforms: linux/amd64,linux/arm64`) — buildx selects the matching descriptor per target.
# The index carries linux/amd64, linux/arm64/v8, linux/arm/v7, s390x and ppc64le; the builder
# stage above is likewise pinned to an index. #2312 reported this pin as arm64-only and
# therefore unbuildable for amd64; that is not the case — verified by resolving the digest
# against gcr.io. Do NOT "fix" it by repinning. If you replace it, replace it with another
# INDEX digest (`docker buildx imagetools inspect` must list both release platforms), never
# with a per-architecture child manifest digest — that is what would actually break amd64.
# Tag: gcr.io/distroless/cc-debian12:nonroot
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:adcd20c7b4c988b73cbfbddb26d2eee574571e6d7c9ffea29b3821e0690efb77

LABEL org.opencontainers.image.title="sparq-server" \
      org.opencontainers.image.description="W3C SPARQL 1.1 Protocol server for the sparq RDF triplestore (dictionary-encoded, six permutation indexes, parallel execution)" \
      org.opencontainers.image.source="https://github.com/sparq-org/sparq" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.documentation="https://github.com/sparq-org/sparq#readme"

COPY --from=builder /build/target/release/sparq-server /usr/local/bin/sparq-server

# Conventional mount point for read-only datasets.
VOLUME ["/data"]
EXPOSE 3030

# [OPUS-4.8] sq-n6rv: permit the non-loopback bind the ENTRYPOINT requests.
# After the fail-closed bind posture (sq-o4qf) landed, the server REFUSES a non-loopback
# `--addr` (and exits non-zero) unless --allow-remote / SPARQ_ALLOW_REMOTE=1 is set. Inside
# a container the ONLY useful bind is 0.0.0.0 (loopback is unreachable through Docker's port
# mapping), and running the container is itself the operator's explicit choice to publish a
# network surface — so we set it here so the image BOOTS out of the box. This does NOT add
# auth: see the security note in the header and crates/sparq-server/README.md. Gate the
# surface in production with `-e SPARQ_AUTH_TOKEN=...` (and `-e SPARQ_AUTH_TOKEN_READ=1` for
# a fully-gated surface) — the server reads those from the environment.
ENV SPARQ_ALLOW_REMOTE=1

# [OPUS-4.8] sq-toze.36 (cert gap GX-13, CIS Docker Benchmark §4.6): self-declared HEALTHCHECK
# so a container runtime can tell an unhealthy-but-still-running server apart from a healthy one
# (`docker ps` STATUS shows healthy/unhealthy; `--health-cmd` orchestrators can act on it).
#
# DISTROLESS CONSTRAINT: this runtime stage has NO shell and NO curl/wget, so the classic
# `HEALTHCHECK CMD curl -f .../health` cannot run here. Instead the server binary probes ITSELF:
# `--health-probe` opens a TCP connection to the loopback /health endpoint and exits 0 (healthy)
# / non-zero (unhealthy) — no external tool, no extra layer. EXEC form (a JSON array) is required
# precisely because there is no shell to interpret a string CMD. The probe defaults to
# 127.0.0.1:3030 (the loopback inside this container's netns), matching the ENTRYPOINT's
# 0.0.0.0:3030 bind; a remapped internal port can be passed via SPARQ_HEALTH_PROBE_ADDR.
#
# Note: k8s/Nomad typically run their OWN liveness/readiness probe against /health and ignore the
# image HEALTHCHECK — this primarily benefits bare `docker run` / docker-compose / Swarm. It is
# baked once here and costs nothing when an orchestrator overrides it. --start-period covers the
# initial dataset load before the first failing check counts against the container.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/sparq-server", "--health-probe"]

# Bind 0.0.0.0 inside the container (the binary's default 127.0.0.1 is unreachable through
# Docker's port mapping). Flags repeat-override in sparq-server's arg parser, so callers can
# still append e.g. `--addr 0.0.0.0:8080` plus `--format ntriples /data/file.nt` as CMD args.
ENTRYPOINT ["/usr/local/bin/sparq-server", "--addr", "0.0.0.0:3030"]
CMD []
