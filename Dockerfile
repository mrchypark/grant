# syntax=docker/dockerfile:1.7
FROM golang:1.27-bookworm AS builder
WORKDIR /workspace
COPY --link go.mod go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY --link internal ./internal
COPY --link cmd ./cmd
RUN --mount=type=cache,target=/go/pkg/mod --mount=type=cache,target=/root/.cache/go-build \
    CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o /workspace/grant ./cmd/grant \
    && CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o /workspace/grant-patch-data-manifest-digest ./cmd/grant-patch-data-manifest-digest
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 grant \
    && useradd --system --uid 10001 --gid 10001 --home /app --shell /usr/sbin/nologin grant \
    && mkdir -p /app /data/rhiza /data/backups /tmp \
    && chown -R 10001:10001 /app /data/rhiza /data/backups /tmp
WORKDIR /app
COPY --link --from=builder --chown=10001:10001 /workspace/grant /usr/local/bin/grant
COPY --link --from=builder --chown=10001:10001 /workspace/grant-patch-data-manifest-digest /usr/local/bin/grant-patch-data-manifest-digest
USER 10001:10001
ENV GRANT_BIND_ADDR=0.0.0.0:8080 GRANT_DATA_DIR=/data/rhiza GRANT_BACKUP_DIR=/data/backups
EXPOSE 8080 8100/udp
ENTRYPOINT ["/usr/local/bin/grant"]
