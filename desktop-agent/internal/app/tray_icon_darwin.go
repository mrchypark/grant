//go:build darwin

package app

import trayassets "desktop-agent/assets"

func trayIconBytes() ([]byte, []byte) {
	iconBytes := trayassets.TrayIconBytes()
	return iconBytes, iconBytes
}
