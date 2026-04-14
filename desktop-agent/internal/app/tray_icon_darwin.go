//go:build darwin

package app

import trayassets "github.com/cypark/account/desktop-agent/assets"

func trayIconBytes() ([]byte, []byte) {
	iconBytes := trayassets.TrayIconBytes()
	return iconBytes, iconBytes
}
