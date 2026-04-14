//go:build darwin

package assets

import _ "embed"

var (
	//go:embed tray-icon.png
	trayIconPNG []byte
)

func TrayIconBytes() []byte {
	return trayIconPNG
}
