# syntax=docker/dockerfile:1.7

FROM rust:1.91.1-bookworm AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev libcurl4-openssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install --locked cargo-chef
WORKDIR /workspace

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/workspace/target \
    cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/workspace/target \
    cargo build --release -p app \
    && cp /workspace/target/release/app /workspace/certd

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl openssl libcurl4 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 certd \
    && useradd --system --uid 10001 --gid 10001 --home /app --shell /usr/sbin/nologin certd \
    && mkdir -p /app /data/hiqlite /tmp \
    && chown -R 10001:10001 /app /data/hiqlite /tmp

WORKDIR /app
COPY --from=builder --chown=10001:10001 /workspace/certd /usr/local/bin/certd

USER 10001:10001
ENV CERTD_BIND_ADDR=0.0.0.0:8080
ENV HQL_DATA_DIR=/data/hiqlite
ENV RUST_LOG=info
ENV RUST_BACKTRACE=0

EXPOSE 8080 8100 8200

ENTRYPOINT ["/usr/local/bin/certd"]
