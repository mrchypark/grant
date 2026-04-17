package app

import (
	"net/http"
	"testing"
	"time"
)

func TestNewHTTPServerSetsReadHeaderTimeout(t *testing.T) {
	server := newHTTPServer("127.0.0.1:0", http.NewServeMux())

	if server.ReadHeaderTimeout != 15*time.Second {
		t.Fatalf("ReadHeaderTimeout = %s, want %s", server.ReadHeaderTimeout, 15*time.Second)
	}
	if server.ReadTimeout != 30*time.Second {
		t.Fatalf("ReadTimeout = %s, want %s", server.ReadTimeout, 30*time.Second)
	}
	if server.IdleTimeout != 60*time.Second {
		t.Fatalf("IdleTimeout = %s, want %s", server.IdleTimeout, 60*time.Second)
	}
}

func TestExamplePageURLDefaultsAndCanBeOverridden(t *testing.T) {
	t.Setenv("DESKTOP_AGENT_EXAMPLE_URL", "")
	if got := examplePageURL(); got != defaultExamplePageURL {
		t.Fatalf("examplePageURL() = %q, want %q", got, defaultExamplePageURL)
	}

	t.Setenv("DESKTOP_AGENT_EXAMPLE_URL", "http://localhost:9999/demo")
	if got := examplePageURL(); got != "http://localhost:9999/demo" {
		t.Fatalf("examplePageURL() override = %q, want %q", got, "http://localhost:9999/demo")
	}
}
