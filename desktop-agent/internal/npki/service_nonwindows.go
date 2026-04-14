//go:build !windows

package npki

import "context"

// non-Windows 플랫폼에서는 DefaultConfig가 Windows AutoDiscover 분기로 들어올 일이
// 없지만, discoverWindowsDriveNPKIRoots 심볼 자체는 service.go 컴파일에 필요하다.
func discoverWindowsDriveNPKIRoots(context.Context) []string {
	return nil
}
