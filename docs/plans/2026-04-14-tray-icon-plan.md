# Tray Icon Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Apply the selected fingerprint `RE` icon to the macOS desktop-agent tray item.

**Architecture:** Keep the chosen SVG in `desktop-agent/assets/` as the editable source, rasterize it to a small PNG asset for systray consumption, and embed that PNG in a darwin-only helper used by `runtime_darwin.go`. Add a darwin-only unit test that verifies the embedded asset is present and looks like PNG data before wiring it into `systray.SetTemplateIcon`.

**Tech Stack:** Go 1.26, `embed`, `github.com/getlantern/systray`, macOS PNG tray asset

---

### Task 1: Add a failing darwin icon asset test

**Files:**
- Create: `desktop-agent/internal/app/tray_icon_darwin_test.go`

**Step 1: Write the failing test**

Add a darwin-only test that calls a helper such as `trayIconBytes()` and asserts:
- both returned byte slices are non-empty
- the first four bytes match the PNG signature prefix

**Step 2: Run test to verify it fails**

Run: `go test ./internal/app`
Expected: FAIL because `trayIconBytes` does not exist yet.

### Task 2: Add embedded tray icon asset

**Files:**
- Create: `desktop-agent/internal/app/tray_icon_darwin.go`
- Create: `desktop-agent/assets/tray-icon.png`
- Modify: `desktop-agent/assets/tray-icon.svg`

**Step 1: Generate the PNG asset**

Rasterize the selected SVG to a transparent PNG sized for tray use.

**Step 2: Write minimal implementation**

Use `//go:embed` in a darwin-only file to load the PNG bytes and return them from `trayIconBytes()`.

**Step 3: Run test to verify it passes**

Run: `go test ./internal/app`
Expected: PASS

### Task 3: Wire the icon into the tray runtime

**Files:**
- Modify: `desktop-agent/internal/app/runtime_darwin.go`

**Step 1: Apply the icon**

Call `systray.SetTemplateIcon(templateBytes, regularBytes)` in `onReady` before menu items are created.

**Step 2: Verify the package still builds**

Run: `go test ./internal/app` and `go test ./...`
Expected: PASS

### Task 4: Verify the app builds with the icon asset

**Files:**
- No new files

**Step 1: Build the desktop agent**

Run: `go build ./cmd/desktop-agent`
Expected: PASS
