# syntax=docker/dockerfile:1.7
FROM --platform=$BUILDPLATFORM golang:1.27.4-bookworm AS builder
ARG TARGETOS
ARG TARGETARCH
WORKDIR /workspace
COPY --link go.mod go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY --link internal ./internal
COPY --link cmd ./cmd
RUN --mount=type=cache,target=/go/pkg/mod --mount=type=cache,target=/root/.cache/go-build \
    GOOS=$TARGETOS GOARCH=$TARGETARCH CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o /workspace/grant ./cmd/grant
FROM alpine:3.21 AS runtime
RUN apk add --no-cache ca-certificates curl \
    && addgroup -g 10001 -S grant \
    && adduser -u 10001 -S -G grant -h /app -s /sbin/nologin grant \
    && mkdir -p /app /var/lib/grant/rhiza /var/lib/grant/backups /tmp \
    && chown -R 10001:10001 /app /var/lib/grant /tmp
WORKDIR /app
COPY --link --from=builder --chown=10001:10001 /workspace/grant /usr/local/bin/grant
RUN ln -s /usr/local/bin/grant /usr/local/bin/grant-admin \
    && ln -s /usr/local/bin/grant /usr/local/bin/replica-members
USER 10001:10001
ENV GRANT_BIND_ADDR=0.0.0.0:8080 GRANT_DATA_DIR=/var/lib/grant/rhiza GRANT_BACKUP_DIR=/var/lib/grant/backups
EXPOSE 8080 8100/udp
ENTRYPOINT ["/usr/local/bin/grant"]
