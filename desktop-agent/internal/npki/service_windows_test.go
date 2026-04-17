//go:build windows

package npki

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDefaultLocalRootsForWindowsIncludesLocalLowAndProgramFiles(t *testing.T) {
	roots := defaultLocalRootsFor("windows", `C:\Users\tester`)

	wantLocalLow := filepath.Join(`C:\Users\tester`, "AppData", "LocalLow", "NPKI")
	if !containsFold(roots, wantLocalLow) {
		t.Errorf("defaultLocalRootsFor(windows) missing %q; got %v", wantLocalLow, roots)
	}

	if pf := strings.TrimSpace(os.Getenv("ProgramFiles")); pf != "" {
		want := filepath.Join(pf, "NPKI")
		if !containsFold(roots, want) {
			t.Errorf("defaultLocalRootsFor(windows) missing %q; got %v", want, roots)
		}
	}
}

func TestDiscoverWindowsDriveNPKIRootsRespectsContextCancel(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if got := discoverWindowsDriveNPKIRoots(ctx); got != nil {
		t.Errorf("expected nil when context is canceled, got %v", got)
	}
}

func TestDiscoverWindowsDriveNPKIRootsUsesFileAttributes(t *testing.T) {
	originalMask := getLogicalDrivesMask
	originalDriveType := getDriveType
	originalGetFileAttributes := getFileAttributes
	defer func() {
		getLogicalDrivesMask = originalMask
		getDriveType = originalDriveType
		getFileAttributes = originalGetFileAttributes
	}()

	getLogicalDrivesMask = func() uint32 {
		return (1 << 2) | (1 << 3)
	}
	getDriveType = func(driveRoot string) uint32 {
		if driveRoot == `C:\` || driveRoot == `D:\` {
			return driveFixed
		}
		return 0
	}
	getFileAttributes = func(path string) (uint32, error) {
		switch path {
		case filepath.Join(`C:\`, "NPKI"):
			return fileAttributeDirectory, nil
		case filepath.Join(`D:\`, "NPKI"):
			return invalidFileAttributes, os.ErrNotExist
		default:
			return invalidFileAttributes, os.ErrNotExist
		}
	}

	got := discoverWindowsDriveNPKIRoots(context.Background())
	want := []string{filepath.Join(`C:\`, "NPKI")}
	if !containsFold(got, want[0]) || len(got) != 1 {
		t.Fatalf("discoverWindowsDriveNPKIRoots() = %v, want %v", got, want)
	}
}

func containsFold(haystack []string, needle string) bool {
	for _, h := range haystack {
		if strings.EqualFold(h, needle) {
			return true
		}
	}
	return false
}
