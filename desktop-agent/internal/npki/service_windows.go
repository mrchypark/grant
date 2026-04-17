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

// discoverWindowsDriveNPKIRoots returns NPKI directories found at the root of
// every currently-mounted fixed or removable drive. CD-ROM과 네트워크 드라이브는
// 빈 트레이·연결 지연을 피하려고 GetDriveType으로 건너뛰고, 루트 존재 확인은
// os.Stat 대신 GetFileAttributesW로 처리해 removable media probe를 최소화한다.
func discoverWindowsDriveNPKIRoots(ctx context.Context) []string {
	if err := ctx.Err(); err != nil {
		return nil
	}

	mask := getLogicalDrivesMask()

	roots := make([]string, 0, 4)
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
		npkiRoot := filepath.Join(driveRoot, "NPKI")
		if attrs, err := getFileAttributes(npkiRoot); err == nil && attrs&fileAttributeDirectory != 0 {
			roots = append(roots, npkiRoot)
		}
	}
	return roots
}
