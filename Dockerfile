FROM rust:1.91.1-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo build --release -p app

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates openssl libcurl4 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /workspace/target/release/app /usr/local/bin/certd

ENV CERTD_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080 8100 8200

CMD ["certd"]
