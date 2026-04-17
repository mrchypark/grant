//go:build windows

package app

import "testing"

func TestTrayIconBytes(t *testing.T) {
	templateBytes, regularBytes := trayIconBytes()

	if len(templateBytes) == 0 {
		t.Fatal("template tray icon bytes are empty")
	}
	if len(regularBytes) == 0 {
		t.Fatal("regular tray icon bytes are empty")
	}

	// ICO 매직: 2바이트 reserved(0) + 2바이트 type(1 for icon).
	wantPrefix := []byte{0x00, 0x00, 0x01, 0x00}
	if len(templateBytes) < len(wantPrefix) {
		t.Fatal("template tray icon bytes are too short")
	}
	if got := templateBytes[:4]; string(got) != string(wantPrefix) {
		t.Fatalf("template tray icon prefix = % x, want % x", got, wantPrefix)
	}
}
