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
# Optional cargo job cap for memory-limited builders; unset keeps full parallelism.
ARG BUILD_JOBS=""
WORKDIR /build

COPY . .

# cargo-auditable's own lockfile pins its install; the workspace build separately
# uses the committed Cargo.lock and the crate's default feature set.
# An optional build_ca secret supplies a trusted CA bundle for proxied builds:
# docker build --secret id=build_ca,src=/path/to/ca-bundle.crt ...
# The certificate is mounted only for these commands and never enters an image layer.
RUN --mount=type=secret,id=build_ca \
    if [ -f /run/secrets/build_ca ]; then \
      export SSL_CERT_FILE=/run/secrets/build_ca CURL_CA_BUNDLE=/run/secrets/build_ca \
        CARGO_HTTP_CAINFO=/run/secrets/build_ca GIT_SSL_CAINFO=/run/secrets/build_ca RUSTUP_USE_CURL=1; \
    fi; \
    cargo install --locked ${BUILD_JOBS:+--jobs "$BUILD_JOBS"} cargo-auditable
RUN --mount=type=secret,id=build_ca \
    if [ -f /run/secrets/build_ca ]; then \
      export SSL_CERT_FILE=/run/secrets/build_ca CURL_CA_BUNDLE=/run/secrets/build_ca \
        CARGO_HTTP_CAINFO=/run/secrets/build_ca GIT_SSL_CAINFO=/run/secrets/build_ca RUSTUP_USE_CURL=1; \
    fi; \
    cargo auditable build --release --locked ${BUILD_JOBS:+--jobs "$BUILD_JOBS"} -p sparq-lws-core ${CARGO_FLAGS}

# Distroless has no mkdir; copy an empty mount point with the runtime ownership.
RUN mkdir /data

# -------- runtime --------
# Tag: gcr.io/distroless/cc-debian12:nonroot
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:fccdbb0a547c14e23fcf4ce8ad62ca5d43b4faae8d22cd292f490fef9946c96e

LABEL org.opencontainers.image.title="sparq-lws-core" \
      org.opencontainers.image.description="Native experimental Solid/LDP server; opt in to single-instance persistence with a /data volume and SOLID_SERVER_DATA_DIR=/data" \
      org.opencontainers.image.source="https://github.com/sparq-org/sparq" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.documentation="https://github.com/sparq-org/sparq/tree/main/crates/sparq-lws-core#readme"

COPY --from=builder /build/target/release/sparq-lws-core /usr/local/bin/sparq-lws-core
COPY --from=builder --chown=65532:65532 /data /data

# A fresh named volume inherits this ownership. Storage stays in-memory unless
# the operator also sets SOLID_SERVER_DATA_DIR=/data.
VOLUME ["/data"]

# The distroless nonroot identity is uid/gid 65532. Declare it explicitly so
# neither a base-image metadata change nor a caller's assumption can make the
# packaged process root by default.
USER 65532:65532

ENV SOLID_SERVER_BIND=0.0.0.0:3000
EXPOSE 3000

# Base URL, TLS, trusted issuer/audience, auth policy, and storage selection are
# intentionally not baked in. Operators supply the existing SOLID_SERVER_* / PSS_*
# contract; in particular, no development auth or loopback escape hatch is enabled.
# Optional history services: SOLID_SERVER_VERSIONS=1 and SOLID_SERVER_TRASH=1.
# Both default off; Solid Suite supplies both at deployment. Retention settings:
# SOLID_SERVER_VERSIONS_MAX=20, SOLID_SERVER_VERSIONS_RETENTION=P30D,
# SOLID_SERVER_TRASH_RETENTION=P30D. Requires the memory or embedded index.
ENTRYPOINT ["/usr/local/bin/sparq-lws-core"]
CMD []
