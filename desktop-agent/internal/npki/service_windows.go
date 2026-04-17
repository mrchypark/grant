//go:build windows

package npki

import (
	"context"
	"fmt"
	"os"
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

var readWindowsDriveDir = os.ReadDir

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
	entries, err := readWindowsDriveDir(driveRoot)
	if err != nil {
		return nil
	}
	paths := make([]string, 0, len(entries))
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if isPKCS12FileName(entry.Name()) {
			paths = append(paths, filepath.Join(driveRoot, entry.Name()))
		}
	}
	return paths
}
