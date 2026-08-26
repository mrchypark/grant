# syntax=docker/dockerfile:1.7

FROM rust:1.91.1-bookworm AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends binutils pkg-config libssl-dev libcurl4-openssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install --locked cargo-chef
WORKDIR /workspace

FROM chef AS planner
COPY --link Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY --link crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN --mount=type=cache,id=grant-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=grant-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=grant-target,target=/workspace/target,sharing=locked \
    cargo chef cook --release --locked --recipe-path recipe.json
COPY --link Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY --link crates ./crates
RUN --mount=type=cache,id=grant-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=grant-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=grant-target,target=/workspace/target,sharing=locked \
    cargo build --release --locked -p grant \
        --bin grant \
        --bin grant-patch-data-authority-snapshot \
    && strip \
        /workspace/target/release/grant \
        /workspace/target/release/grant-patch-data-authority-snapshot \
    && cp /workspace/target/release/grant /workspace/grant \
    && cp /workspace/target/release/grant-patch-data-authority-snapshot \
        /workspace/grant-patch-data-authority-snapshot

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libcurl4 libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 grant \
    && useradd --system --uid 10001 --gid 10001 --home /app --shell /usr/sbin/nologin grant \
    && mkdir -p /app /data/hiqlite /tmp \
    && chown -R 10001:10001 /app /data/hiqlite /tmp

WORKDIR /app
COPY --link --from=builder --chown=10001:10001 /workspace/grant /usr/local/bin/grant
COPY --link --from=builder --chown=10001:10001 /workspace/grant-patch-data-authority-snapshot /usr/local/bin/grant-patch-data-authority-snapshot

USER 10001:10001
ENV GRANT_BIND_ADDR=0.0.0.0:8080
ENV HQL_DATA_DIR=/data/hiqlite
ENV RUST_LOG=info
ENV RUST_BACKTRACE=0

EXPOSE 8080 8100 8200

ENTRYPOINT ["/usr/local/bin/grant"]
