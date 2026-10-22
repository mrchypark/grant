# syntax=docker/dockerfile:1.7
ARG GO_IMAGE=golang:1.27.1-alpine@sha256:8a5910f31396cd4d89662f56c68b3ae31d374308270a1c3bd96672ee5ed43414
ARG RAUTHY_BASE_IMAGE=ghcr.io/sebadob/rauthy:0.37.1@sha256:72b4734a7c52bd67cd544e7b72a4ab64932a7060844ef475fc2bb3d3da3074d9
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
CMD ["/app/rauthy", "serve"]
