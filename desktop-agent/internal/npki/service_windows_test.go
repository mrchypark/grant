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

func containsFold(haystack []string, needle string) bool {
	for _, h := range haystack {
		if strings.EqualFold(h, needle) {
			return true
		}
	}
	return false
}
