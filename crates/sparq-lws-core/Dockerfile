# syntax=docker/dockerfile:1
# [GPT-5.6] sq-lmz40 — native sparq-lws-core production container.
#
# Build from the repository root:
#   docker build -f crates/sparq-lws-core/Dockerfile -t sparq-lws-core .
#
# The builder and runtime intentionally match the repository's sparq-server image:
# Debian 12 on both sides keeps glibc compatible, cargo-auditable embeds the Rust
# dependency manifest, and the final distroless image has no shell or package manager.

# -------- builder --------
# Tag: rust:1.88-slim-bookworm
FROM rust:1.88-slim-bookworm@sha256:38bc5a86d998772d4aec2348656ed21438d20fcdce2795b56ca434cf21430d89 AS builder

ARG CARGO_FLAGS=""
WORKDIR /build

COPY . .

# cargo-auditable's own lockfile pins its install; the workspace build separately
# uses the committed Cargo.lock and the crate's default feature set.
RUN cargo install --locked cargo-auditable
RUN cargo auditable build --release --locked -p sparq-lws-core ${CARGO_FLAGS}

# -------- runtime --------
# Tag: gcr.io/distroless/cc-debian12:nonroot
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:fccdbb0a547c14e23fcf4ce8ad62ca5d43b4faae8d22cd292f490fef9946c96e

LABEL org.opencontainers.image.title="sparq-lws-core" \
      org.opencontainers.image.description="Native experimental Solid/LDP server backed by the sparq RDF engine" \
      org.opencontainers.image.source="https://github.com/sparq-org/sparq" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.documentation="https://github.com/sparq-org/sparq/tree/main/crates/sparq-lws-core#readme"

COPY --from=builder /build/target/release/sparq-lws-core /usr/local/bin/sparq-lws-core

# The distroless nonroot identity is uid/gid 65532. Declare it explicitly so
# neither a base-image metadata change nor a caller's assumption can make the
# packaged process root by default.
USER 65532:65532

ENV SOLID_SERVER_BIND=0.0.0.0:3000
EXPOSE 3000

# Base URL, TLS, trusted issuer/audience, auth policy, and storage selection are
# intentionally not baked in. Operators supply the existing SOLID_SERVER_* / PSS_*
# contract; in particular, no development auth or loopback escape hatch is enabled.
ENTRYPOINT ["/usr/local/bin/sparq-lws-core"]
CMD []
