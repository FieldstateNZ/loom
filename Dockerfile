# syntax=docker/dockerfile:1

# Base images are pinned to the multi-arch index digests of the named tags.
# The digests were taken from anonymous public Docker Hub manifest responses
# (content SHA256 verified out-of-band). Bump the tag and the digest together;
# the CI `image` job prints the digests it resolved so drift is visible.

# ---- builder ----------------------------------------------------------------
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS builder
WORKDIR /build

# Locked build with offline SQL query metadata:
#  * RUSTUP_TOOLCHAIN pins cargo to the toolchain shipped in this image
#    (RUST_VERSION is set by the official rust image). Without it rustup would
#    honour rust-toolchain.toml's `stable` channel and download a different
#    compiler at build time, making the 1.94 base pin cosmetic.
#  * SQLX_OFFLINE compiles the sqlx query macros from the committed
#    crates/loom-store/.sqlx cache, so no database is contacted.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    SQLX_OFFLINE=true

# Copy the build context. Produce it from `git archive` (deploy/ci/docker_context.sh)
# so only committed files are ever sent to the daemon; .dockerignore is a
# second line of defence for local `docker build .` invocations.
COPY . .

# Build only the server binary in release mode. `--locked` refuses to proceed
# if Cargo.lock would need to change, so the dependency graph is exactly the
# committed one.
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    rustc --version && cargo --version \
 && cargo build --release --locked -p loom-server \
 && cp target/release/loom-server /usr/local/bin/loom-server

# ---- runtime ----------------------------------------------------------------
# Final stage: the server binary, the OS CA bundle, and an unprivileged user.
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --user-group --home /nonexistent --shell /usr/sbin/nologin loom

COPY --from=builder /usr/local/bin/loom-server /usr/local/bin/loom-server

USER loom
ENV LOOM_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Liveness endpoint served by loom-server.
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/loom-server", "--healthcheck"]

ENTRYPOINT ["/usr/local/bin/loom-server"]
