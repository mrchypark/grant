package app

import (
	"net/http"
	"testing"
	"time"
)

func TestShouldUseTray(t *testing.T) {
	testCases := []struct {
		name       string
		goos       string
		disableEnv string
		want       bool
	}{
		{name: "darwin enabled by default", goos: "darwin", disableEnv: "", want: true},
		{name: "darwin disabled explicitly", goos: "darwin", disableEnv: "1", want: false},
		{name: "windows kept headless for now", goos: "windows", disableEnv: "", want: false},
		{name: "linux kept headless", goos: "linux", disableEnv: "", want: false},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldUseTray(tc.goos, tc.disableEnv); got != tc.want {
				t.Fatalf("shouldUseTray(%q, %q) = %v, want %v", tc.goos, tc.disableEnv, got, tc.want)
			}
		})
	}
}

func TestNewHTTPServerSetsReadHeaderTimeout(t *testing.T) {
	server := newHTTPServer("127.0.0.1:0", http.NewServeMux())

	if server.ReadHeaderTimeout != 15*time.Second {
		t.Fatalf("ReadHeaderTimeout = %s, want %s", server.ReadHeaderTimeout, 15*time.Second)
	}
}
