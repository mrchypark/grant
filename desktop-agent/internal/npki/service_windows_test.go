//go:build windows

package npki

import (
	"context"
	"io/fs"
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
	originalReadDriveDir := readWindowsDriveDir
	defer func() {
		getLogicalDrivesMask = originalMask
		getDriveType = originalDriveType
		getFileAttributes = originalGetFileAttributes
		readWindowsDriveDir = originalReadDriveDir
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
		case filepath.Join(`C:\`, "GPKI"):
			return invalidFileAttributes, os.ErrNotExist
		case filepath.Join(`D:\`, "GPKI"):
			return invalidFileAttributes, os.ErrNotExist
		case filepath.Join(`D:\`, "NPKI"):
			return invalidFileAttributes, os.ErrNotExist
		default:
			return invalidFileAttributes, os.ErrNotExist
		}
	}
	readWindowsDriveDir = func(string) ([]os.DirEntry, error) {
		return nil, os.ErrNotExist
	}

	got := discoverWindowsDriveNPKIRoots(context.Background())
	want := []string{filepath.Join(`C:\`, "NPKI")}
	if !containsFold(got, want[0]) || len(got) != 1 {
		t.Fatalf("discoverWindowsDriveNPKIRoots() = %v, want %v", got, want)
	}
}

func TestDiscoverWindowsDriveNPKIRootsFindsDriveRootPKCS12Files(t *testing.T) {
	originalMask := getLogicalDrivesMask
	originalDriveType := getDriveType
	originalGetFileAttributes := getFileAttributes
	originalReadDriveDir := readWindowsDriveDir
	defer func() {
		getLogicalDrivesMask = originalMask
		getDriveType = originalDriveType
		getFileAttributes = originalGetFileAttributes
		readWindowsDriveDir = originalReadDriveDir
	}()

	getLogicalDrivesMask = func() uint32 {
		return 1 << 2
	}
	getDriveType = func(driveRoot string) uint32 {
		if driveRoot == `C:\` {
			return driveRemovable
		}
		return 0
	}
	getFileAttributes = func(string) (uint32, error) {
		return invalidFileAttributes, os.ErrNotExist
	}
	readWindowsDriveDir = func(path string) ([]os.DirEntry, error) {
		if path != `C:\` {
			return nil, os.ErrNotExist
		}
		return []os.DirEntry{
			testDirEntry{name: "protected.p12"},
			testDirEntry{name: "notes.txt"},
		}, nil
	}

	got := discoverWindowsDriveNPKIRoots(context.Background())
	want := []string{filepath.Join(`C:\`, "protected.p12")}
	if !containsFold(got, want[0]) || len(got) != 1 {
		t.Fatalf("discoverWindowsDriveNPKIRoots() = %v, want %v", got, want)
	}
}

type testDirEntry struct {
	name string
	dir  bool
}

func (e testDirEntry) Name() string               { return e.name }
func (e testDirEntry) IsDir() bool                { return e.dir }
func (e testDirEntry) Type() fs.FileMode          { return 0 }
func (e testDirEntry) Info() (fs.FileInfo, error) { return nil, nil }

func containsFold(haystack []string, needle string) bool {
	for _, h := range haystack {
		if strings.EqualFold(h, needle) {
			return true
		}
	}
	return false
}
