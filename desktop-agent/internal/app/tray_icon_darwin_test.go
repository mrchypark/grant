//go:build darwin

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

	wantPrefix := []byte{0x89, 'P', 'N', 'G'}
	if got := templateBytes[:4]; string(got) != string(wantPrefix) {
		t.Fatalf("template tray icon prefix = %v, want %v", got, wantPrefix)
	}
	if got := regularBytes[:4]; string(got) != string(wantPrefix) {
		t.Fatalf("regular tray icon prefix = %v, want %v", got, wantPrefix)
	}
}
