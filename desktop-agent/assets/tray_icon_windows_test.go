//go:build windows

package assets

import "testing"

func TestTrayIconBytesIsValidICO(t *testing.T) {
	b := TrayIconBytes()
	if len(b) < 6 {
		t.Fatalf("tray icon ICO too short: %d bytes", len(b))
	}
	// ICONDIR header: WORD Reserved=0, WORD Type=1 (icon), WORD Count>=1.
	if b[0] != 0 || b[1] != 0 {
		t.Fatalf("ICO reserved bytes = % x, want 00 00", b[:2])
	}
	if b[2] != 1 || b[3] != 0 {
		t.Fatalf("ICO type bytes = % x, want 01 00 (icon)", b[2:4])
	}
	count := int(b[4]) | int(b[5])<<8
	if count < 1 {
		t.Fatalf("ICO icon count = %d, want >= 1", count)
	}
}
