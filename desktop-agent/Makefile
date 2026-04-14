GO      ?= go
GOOS    ?= $(shell $(GO) env GOOS)
GOARCH  ?= $(shell $(GO) env GOARCH)
BIN_DIR := dist
PKG     := ./cmd/desktop-agent

ifeq ($(GOOS),windows)
  BIN     := $(BIN_DIR)/desktop-agent-$(GOOS)-$(GOARCH).exe
  LDFLAGS := -H=windowsgui
else
  BIN     := $(BIN_DIR)/desktop-agent-$(GOOS)-$(GOARCH)
  LDFLAGS :=
endif

.PHONY: all build test vet clean build-windows build-darwin build-linux

all: vet test build

vet:
	$(GO) vet ./...

test:
	$(GO) test ./...

build:
	@mkdir -p $(BIN_DIR)
	GOOS=$(GOOS) GOARCH=$(GOARCH) $(GO) build -ldflags "$(LDFLAGS)" -o $(BIN) $(PKG)

# darwin은 Cocoa cgo 의존 때문에 macOS 호스트에서만 동작.
build-darwin:
	$(MAKE) build GOOS=darwin GOARCH=arm64

build-windows:
	$(MAKE) build GOOS=windows GOARCH=amd64

build-linux:
	$(MAKE) build GOOS=linux GOARCH=amd64

clean:
	rm -rf $(BIN_DIR)
