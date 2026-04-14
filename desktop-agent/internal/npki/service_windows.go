//go:build windows

package npki

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"unsafe"

	"golang.org/x/sys/windows"
)

var (
	npkiKernel32         = windows.NewLazySystemDLL("kernel32.dll")
	procGetLogicalDrives = npkiKernel32.NewProc("GetLogicalDrives")
	procGetDriveTypeW    = npkiKernel32.NewProc("GetDriveTypeW")
)

const (
	driveRemovable uint32 = 2
	driveFixed     uint32 = 3
)

// discoverWindowsDriveNPKIRoots returns NPKI directories found at the root of
// every currently-mounted fixed or removable drive. CD-ROM과 네트워크 드라이브는
// 빈 트레이·연결 지연으로 os.Stat hang을 유발할 수 있어 GetDriveType으로 건너뛴다.
func discoverWindowsDriveNPKIRoots(ctx context.Context) []string {
	if err := ctx.Err(); err != nil {
		return nil
	}

	maskRet, _, _ := procGetLogicalDrives.Call()
	mask := uint32(maskRet)

	roots := make([]string, 0, 4)
	for i := 0; i < 26; i++ {
		if err := ctx.Err(); err != nil {
			return roots
		}
		if mask&(1<<i) == 0 {
			continue
		}
		driveRoot := fmt.Sprintf("%c:\\", 'A'+i)
		wPtr, err := windows.UTF16PtrFromString(driveRoot)
		if err != nil {
			continue
		}
		typeRet, _, _ := procGetDriveTypeW.Call(uintptr(unsafe.Pointer(wPtr)))
		t := uint32(typeRet)
		if t != driveRemovable && t != driveFixed {
			continue
		}
		npkiRoot := filepath.Join(driveRoot, "NPKI")
		if info, err := os.Stat(npkiRoot); err == nil && info.IsDir() {
			roots = append(roots, npkiRoot)
		}
	}
	return roots
}
