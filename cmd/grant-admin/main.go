package main

import (
	"log/slog"
	"os"

	"github.com/mrchypark/grant/internal/adminserver"
)

func main() {
	if err := adminserver.Run(); err != nil {
		slog.Error("Grant admin stopped", "error", err)
		os.Exit(1)
	}
}
