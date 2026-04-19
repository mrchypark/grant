GO      ?= go
GOOS    ?= $(shell "$(GO)" env GOOS)
GOARCH  ?= $(shell "$(GO)" env GOARCH)
GOHOSTOS ?= $(shell "$(GO)" env GOHOSTOS)
GOHOSTARCH ?= $(shell "$(GO)" env GOHOSTARCH)
GOTOOLCHAIN ?= auto
BIN_DIR := dist
PKG     := ./cmd/desktop-agent
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
else
  BIN     := $(BIN_DIR)/desktop-agent-$(GOOS)-$(GOARCH)
  LDFLAGS :=
endif

.PHONY: all build build-plain test vet clean build-windows build-darwin build-linux

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

# darwin은 Cocoa cgo 의존 때문에 macOS 호스트에서만 동작.
build-darwin:
	$(MAKE) build GOOS=darwin GOARCH=arm64

build-windows:
	$(MAKE) build GOOS=windows GOARCH=amd64

build-linux:
	$(MAKE) build GOOS=linux GOARCH=amd64

clean:
	rm -rf $(BIN_DIR)
