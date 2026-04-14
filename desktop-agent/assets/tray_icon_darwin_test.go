//go:build darwin

package assets

import (
	"bytes"
	"image/png"
	"testing"
)

func TestTrayIconBytesUseTransparentBackground(t *testing.T) {
	img, err := png.Decode(bytes.NewReader(TrayIconBytes()))
	if err != nil {
		t.Fatalf("decode tray icon png: %v", err)
	}

	bounds := img.Bounds()
	corners := []struct{ x, y int }{
		{bounds.Min.X, bounds.Min.Y},
		{bounds.Max.X - 1, bounds.Min.Y},
		{bounds.Min.X, bounds.Max.Y - 1},
		{bounds.Max.X - 1, bounds.Max.Y - 1},
	}
	for _, corner := range corners {
		_, _, _, a := img.At(corner.x, corner.y).RGBA()
		if a != 0 {
			t.Fatalf("corner (%d,%d) alpha = %d, want 0 for transparent background", corner.x, corner.y, a)
		}
	}

	nonTransparent := 0
	for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
		for x := bounds.Min.X; x < bounds.Max.X; x++ {
			_, _, _, a := img.At(x, y).RGBA()
			if a != 0 {
				nonTransparent++
			}
		}
	}
	if nonTransparent == 0 {
		t.Fatal("tray icon contains no visible pixels")
	}
}
