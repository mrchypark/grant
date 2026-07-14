# syntax=docker/dockerfile:1.7
ARG GO_IMAGE=golang:1.25.1-alpine@sha256:b6ed3fd0452c0e9bcdef5597f29cc1418f61672e9d3a2f55bf02e7222c014abd
ARG RAUTHY_BASE_IMAGE=ghcr.io/sebadob/rauthy:0.36.0@sha256:e2a670c79e04ddf5947fbd6cd5dc2ec7115bb9f21132ddf03129d7b66d26f11d
FROM --platform=$BUILDPLATFORM ${GO_IMAGE} AS build
ARG TARGETOS
ARG TARGETARCH
WORKDIR /src
COPY go.mod ./
COPY main.go launcher.go ./
RUN CGO_ENABLED=0 GOOS=${TARGETOS:-linux} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/rauthy-launcher .

FROM ${RAUTHY_BASE_IMAGE}
COPY --from=build --chmod=0555 /out/rauthy-launcher /usr/local/bin/rauthy-launcher
ENTRYPOINT ["/usr/local/bin/rauthy-launcher"]
CMD ["/app/rauthy"]
