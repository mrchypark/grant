GO      ?= go
CARGO   ?= cargo
GOOS    ?= $(shell "$(GO)" env GOOS)
GOARCH  ?= $(shell "$(GO)" env GOARCH)
GOHOSTOS ?= $(shell "$(GO)" env GOHOSTOS)
GOHOSTARCH ?= $(shell "$(GO)" env GOHOSTARCH)
GOTOOLCHAIN ?= auto
BIN_DIR := dist
PKG     := ./cmd/desktop-agent
ROOT_DIR := $(abspath $(CURDIR)/..)
# Keep this version in sync with packages/web-sdk/scripts/build-wasm.mjs.
GARBLE_VERSION ?= v0.16.0
GARBLE_ENABLED ?= 1
GARBLE_FLAGS ?= -literals
GARBLE_BIN_DIR := $(CURDIR)/.bin/$(GOHOSTOS)-$(GOHOSTARCH)/$(GARBLE_VERSION)

ifeq ($(GOHOSTOS),windows)
  GARBLE_BIN := $(GARBLE_BIN_DIR)/garble.exe
else
  GARBLE_BIN := $(GARBLE_BIN_DIR)/garble
endif

$(GARBLE_BIN):
	@mkdir -p "$(GARBLE_BIN_DIR)"
	GOTOOLCHAIN="$(GOTOOLCHAIN)" GOOS= GOARCH= GOBIN="$(GARBLE_BIN_DIR)" "$(GO)" install mvdan.cc/garble@$(GARBLE_VERSION)

ifeq ($(GOOS),windows)
  BIN     := $(BIN_DIR)/desktop-agent-$(GOOS)-$(GOARCH).exe
  LDFLAGS := -H=windowsgui
  WORKER_EXE_EXT := .exe
else
  BIN     := $(BIN_DIR)/desktop-agent-$(GOOS)-$(GOARCH)
  LDFLAGS :=
  WORKER_EXE_EXT :=
endif

ifeq ($(GOOS)/$(GOARCH),darwin/arm64)
  CARGO_BUILD_TARGET ?= aarch64-apple-darwin
else ifeq ($(GOOS)/$(GOARCH),darwin/amd64)
  CARGO_BUILD_TARGET ?= x86_64-apple-darwin
else ifeq ($(GOOS)/$(GOARCH),windows/amd64)
  CARGO_BUILD_TARGET ?= x86_64-pc-windows-msvc
else ifeq ($(GOOS)/$(GOARCH),windows/arm64)
  CARGO_BUILD_TARGET ?= aarch64-pc-windows-msvc
else ifeq ($(GOOS)/$(GOARCH),linux/amd64)
  CARGO_BUILD_TARGET ?= x86_64-unknown-linux-gnu
else ifeq ($(GOOS)/$(GOARCH),linux/arm64)
  CARGO_BUILD_TARGET ?= aarch64-unknown-linux-gnu
endif

ifneq ($(strip $(CARGO_BUILD_TARGET)),)
CARGO_TARGET_FLAGS := --target $(CARGO_BUILD_TARGET)
WORKER_TARGET_BIN := $(ROOT_DIR)/target/$(CARGO_BUILD_TARGET)/release/repass-local-worker$(WORKER_EXE_EXT)
WORKER_DIST_BIN := $(BIN_DIR)/repass-local-worker-$(GOOS)-$(GOARCH)$(WORKER_EXE_EXT)
endif

.PHONY: all build build-plain build-with-worker build-local-worker test vet clean build-windows build-windows-amd64 build-darwin build-darwin-arm64 build-darwin-amd64 build-linux

all: vet test build

vet:
	GOTOOLCHAIN="$(GOTOOLCHAIN)" "$(GO)" vet ./...

test:
	GOTOOLCHAIN="$(GOTOOLCHAIN)" "$(GO)" test ./...

build: $(if $(filter 1,$(GARBLE_ENABLED)),$(GARBLE_BIN))
	@mkdir -p "$(BIN_DIR)"
ifeq ($(GARBLE_ENABLED),1)
	# This repo runs make recipes via sh/bash, so PATH remains ':'-delimited on Windows too.
	GOTOOLCHAIN="$(GOTOOLCHAIN)" PATH="$$(goroot="$$(GOTOOLCHAIN="$(GOTOOLCHAIN)" "$(GO)" env GOROOT)"; if command -v cygpath >/dev/null 2>&1; then cygpath -u "$$goroot"; else printf '%s' "$$goroot"; fi)/bin:$$PATH" GOOS=$(GOOS) GOARCH=$(GOARCH) "$(GARBLE_BIN)" $(GARBLE_FLAGS) build -ldflags "$(LDFLAGS)" -o "$(BIN)" $(PKG)
else
	GOTOOLCHAIN="$(GOTOOLCHAIN)" GOOS=$(GOOS) GOARCH=$(GOARCH) "$(GO)" build -ldflags "$(LDFLAGS)" -o "$(BIN)" $(PKG)
endif

build-plain:
	$(MAKE) build GARBLE_ENABLED=0

build-with-worker: build build-local-worker

build-local-worker:
	@mkdir -p "$(BIN_DIR)"
	@test -n "$(CARGO_BUILD_TARGET)" || { echo "unsupported local worker target GOOS=$(GOOS) GOARCH=$(GOARCH)" >&2; exit 1; }
	cd "$(ROOT_DIR)" && "$(CARGO)" build -p app --bin repass-local-worker --release $(CARGO_TARGET_FLAGS)
	cp "$(WORKER_TARGET_BIN)" "$(WORKER_DIST_BIN)"
	cp "$(WORKER_TARGET_BIN)" "$(BIN_DIR)/repass-local-worker$(WORKER_EXE_EXT)"

# darwin은 Cocoa cgo 의존 때문에 macOS 호스트에서만 동작.
build-darwin: build-darwin-arm64 build-darwin-amd64

build-darwin-arm64:
	$(MAKE) build-with-worker GOOS=darwin GOARCH=arm64

build-darwin-amd64:
	$(MAKE) build-with-worker GOOS=darwin GOARCH=amd64

# Official Windows GUI release target. Additional Windows architectures need
# matching manifest .syso resources before being treated as release artifacts.
build-windows: build-windows-amd64

build-windows-amd64:
	$(MAKE) build-with-worker GOOS=windows GOARCH=amd64

build-linux:
	$(MAKE) build-with-worker GOOS=linux GOARCH=amd64

clean:
	rm -rf $(BIN_DIR)
