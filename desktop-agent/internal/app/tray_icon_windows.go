//go:build windows

package app

import trayassets "github.com/cypark/account/desktop-agent/assets"

// Windows systray는 라이트/다크 템플릿 아이콘을 구분하지 않고 단일 ICO만 쓰므로
// darwin과 시그니처를 맞추되 두 번째 리턴값(regular)은 무시된다.
func trayIconBytes() ([]byte, []byte) {
	iconBytes := trayassets.TrayIconBytes()
	return iconBytes, iconBytes
}
