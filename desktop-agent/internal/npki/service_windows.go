//go:build windows

package npki

import (
	"context"
	"fmt"
	"path/filepath"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

var (
	npkiKernel32           = windows.NewLazySystemDLL("kernel32.dll")
	procGetLogicalDrives   = npkiKernel32.NewProc("GetLogicalDrives")
	procGetDriveTypeW      = npkiKernel32.NewProc("GetDriveTypeW")
	procGetFileAttributesW = npkiKernel32.NewProc("GetFileAttributesW")
	procSetThreadErrorMode = npkiKernel32.NewProc("SetThreadErrorMode")
)

const (
	driveRemovable         uint32 = 2
	driveFixed             uint32 = 3
	fileAttributeDirectory uint32 = 0x10
	invalidFileAttributes  uint32 = ^uint32(0)
)

var getLogicalDrivesMask = func() uint32 {
	maskRet, _, _ := procGetLogicalDrives.Call()
	return uint32(maskRet)
}

var getDriveType = func(driveRoot string) uint32 {
	wPtr, err := windows.UTF16PtrFromString(driveRoot)
	if err != nil {
		return 0
	}
	typeRet, _, _ := procGetDriveTypeW.Call(uintptr(unsafe.Pointer(wPtr)))
	return uint32(typeRet)
}

var getFileAttributes = func(path string) (uint32, error) {
	wPtr, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return invalidFileAttributes, err
	}
	ret, _, callErr := procGetFileAttributesW.Call(uintptr(unsafe.Pointer(wPtr)))
	attrs := uint32(ret)
	if attrs == invalidFileAttributes {
		if callErr == syscall.Errno(0) {
			return attrs, windows.ERROR_FILE_NOT_FOUND
		}
		return attrs, callErr
	}
	return attrs, nil
}

var findWindowsFiles = windowsFindMatches

// discoverWindowsDriveNPKIRoots returns concrete Windows certificate roots:
// <drive>:\NPKI, <drive>:\GPKI, and any root-level .p12/.pfx files. Returning
// specific paths avoids recursively walking the entire drive when autodiscovery
// later fans into walkBundleRoot.
func discoverWindowsDriveNPKIRoots(ctx context.Context) []string {
	if err := ctx.Err(); err != nil {
		return nil
	}

	mask := getLogicalDrivesMask()

	roots := make([]string, 0, 8)
	for i := 0; i < 26; i++ {
		if err := ctx.Err(); err != nil {
			return roots
		}
		if mask&(1<<i) == 0 {
			continue
		}
		driveRoot := fmt.Sprintf("%c:\\", 'A'+i)
		t := getDriveType(driveRoot)
		if t != driveRemovable && t != driveFixed {
			continue
		}
		for _, storageRootName := range []string{"NPKI", "GPKI"} {
			storageRoot := filepath.Join(driveRoot, storageRootName)
			if attrs, err := getFileAttributes(storageRoot); err == nil && attrs&fileAttributeDirectory != 0 {
				roots = append(roots, storageRoot)
			}
		}
		for _, pkcs12Path := range driveRootPKCS12Files(driveRoot) {
			roots = append(roots, pkcs12Path)
		}
	}
	return roots
}

func driveRootPKCS12Files(driveRoot string) []string {
	paths := make([]string, 0, 2)
	seen := make(map[string]struct{})
	for _, pattern := range []string{filepath.Join(driveRoot, "*.p12"), filepath.Join(driveRoot, "*.pfx")} {
		for _, path := range findWindowsFiles(pattern) {
			if _, ok := seen[path]; ok {
				continue
			}
			seen[path] = struct{}{}
			paths = append(paths, path)
		}
	}
	return paths
}

func windowsPathIsOnRemovableMedia(path string) bool {
	driveRoot, ok := windowsDriveRoot(path)
	if !ok {
		return false
	}
	return getDriveType(driveRoot) == driveRemovable
}

func windowsDriveRoot(path string) (string, bool) {
	volume := filepath.VolumeName(filepath.Clean(path))
	if len(volume) != 2 || volume[1] != ':' {
		return "", false
	}
	return volume + `\`, true
}

func windowsFindMatches(pattern string) []string {
	var previousMode uint32
	restoreMode := false
	if procSetThreadErrorMode.Find() == nil {
		r1, _, _ := procSetThreadErrorMode.Call(
			uintptr(windows.SEM_FAILCRITICALERRORS|windows.SEM_NOOPENFILEERRORBOX),
			uintptr(unsafe.Pointer(&previousMode)),
		)
		restoreMode = r1 != 0
	}
	if restoreMode {
		defer procSetThreadErrorMode.Call(uintptr(previousMode), uintptr(unsafe.Pointer(&previousMode)))
	}

	patternPtr, err := windows.UTF16PtrFromString(pattern)
	if err != nil {
		return nil
	}

	var data windows.Win32finddata
	handle, err := windows.FindFirstFile(patternPtr, &data)
	if err != nil {
		if errorsIsAny(err, windows.ERROR_FILE_NOT_FOUND, windows.ERROR_PATH_NOT_FOUND, windows.ERROR_NOT_READY, windows.ERROR_NO_MORE_FILES) {
			return nil
		}
		return nil
	}
	defer windows.FindClose(handle)

	matches := make([]string, 0, 2)
	for {
		name := windows.UTF16ToString(data.FileName[:])
		if name != "" && name != "." && name != ".." && data.FileAttributes&fileAttributeDirectory == 0 {
			matches = append(matches, filepath.Join(filepath.Dir(pattern), name))
		}
		if err := windows.FindNextFile(handle, &data); err != nil {
			if errorsIsAny(err, windows.ERROR_NO_MORE_FILES, windows.ERROR_FILE_NOT_FOUND) {
				break
			}
			break
		}
	}
	return matches
}

func errorsIsAny(err error, targets ...error) bool {
	for _, target := range targets {
		if err == target {
			return true
		}
	}
	return false
}
