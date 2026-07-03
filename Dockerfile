# syntax=docker/dockerfile:1

# ---- builder ----------------------------------------------------------------
FROM rust:1.94-bookworm AS builder
WORKDIR /build

# Copy the full workspace. (A cargo-chef dependency-cache layer can be added
# later; kept simple and correct for now.)
COPY . .

# Build only the server binary in release mode.
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p loom-server \
 && cp target/release/loom-server /usr/local/bin/loom-server

# ---- runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
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
