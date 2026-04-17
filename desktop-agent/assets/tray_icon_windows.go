//go:build windows

package assets

import _ "embed"

var (
	//go:embed tray-icon.ico
	trayIconICO []byte
)

func TrayIconBytes() []byte {
	return trayIconICO
}
