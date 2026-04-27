package main

import (
	"crypto/rand"
	"encoding/base64"
	"errors"
	"log"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"desktop-agent/internal/app"
	"desktop-agent/internal/approval"
	"desktop-agent/internal/httpapi"
	"desktop-agent/internal/npki"
)

func main() {
	addr := os.Getenv("DESKTOP_AGENT_ADDR")
	if addr == "" {
		addr = "127.0.0.1:8765"
	}
	if err := ensureLoopback(addr); err != nil {
		log.Fatal(err)
	}

	sessionSecret, err := loadOrCreateSessionSecret()
	if err != nil {
		log.Fatal(err)
	}
	server := httpapi.NewServer(npki.NewService(npki.DefaultConfig()), approval.NewPrompter(), httpapi.NewHTTPGatewayExecutor(), httpapi.NewCommandLocalExecutor(localExecutorPath()), httpapi.Config{
		AllowedOrigins: allowedOrigins(),
		GatewayBaseURL: gatewayBaseURL(),
		SessionTTL:     5 * time.Minute,
		SessionSecret:  sessionSecret,
	})
	log.Printf("REPass desktop agent listening on %s", addr)
	if err := app.Run(addr, server.Handler(), app.RunOptions{Addr: addr}); err != nil {
		log.Fatal(err)
	}
}

func ensureLoopback(addr string) error {
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return &net.AddrError{Err: "address must be loopback", Addr: addr}
	}
	return nil
}

func allowedOrigins() []string {
	raw := os.Getenv("DESKTOP_AGENT_ALLOWED_ORIGINS")
	if strings.TrimSpace(raw) == "" {
		return []string{
			"http://127.0.0.1:4173",
			"http://localhost:4173",
		}
	}
	parts := strings.Split(raw, ",")
	origins := make([]string, 0, len(parts))
	for _, part := range parts {
		if trimmed := strings.TrimSpace(part); trimmed != "" {
			origins = append(origins, trimmed)
		}
	}
	return origins
}

func gatewayBaseURL() string {
	return strings.TrimSpace(os.Getenv("DESKTOP_AGENT_GATEWAY_BASE_URL"))
}

func localExecutorPath() string {
	if override := strings.TrimSpace(os.Getenv("DESKTOP_AGENT_LOCAL_EXECUTOR")); override != "" {
		return override
	}
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	dir := filepath.Dir(exe)
	candidates := localExecutorCandidates(dir, runtime.GOOS, runtime.GOARCH)
	for _, candidate := range candidates {
		if info, err := os.Stat(candidate); err == nil && !info.IsDir() {
			return candidate
		}
	}
	return ""
}

func localExecutorCandidates(dir string, goos string, goarch string) []string {
	suffix := ""
	if goos == "windows" {
		suffix = ".exe"
	}
	return []string{
		filepath.Join(dir, "repass-local-worker-"+goos+"-"+goarch+suffix),
		filepath.Join(dir, "repass-local-worker"+suffix),
	}
}

func loadOrCreateSessionSecret() ([]byte, error) {
	if override := strings.TrimSpace(os.Getenv("DESKTOP_AGENT_SESSION_SECRET")); override != "" {
		if decoded, err := base64.StdEncoding.DecodeString(override); err == nil && len(decoded) >= 32 {
			return decoded, nil
		}
		if len(override) >= 32 {
			return []byte(override), nil
		}
		return nil, errors.New("DESKTOP_AGENT_SESSION_SECRET must be at least 32 bytes or base64 for 32 bytes")
	}

	configDir, err := os.UserConfigDir()
	if err != nil {
		return nil, err
	}
	stateDir := filepath.Join(configDir, "repass-desktop-agent")
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		return nil, err
	}
	secretPath := filepath.Join(stateDir, "session-secret.bin")
	if secret, err := os.ReadFile(secretPath); err == nil {
		if len(secret) < 32 {
			return nil, errors.New("desktop agent session secret is too short")
		}
		return secret, nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}

	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		return nil, err
	}
	if err := os.WriteFile(secretPath, secret, 0o600); err != nil {
		return nil, err
	}
	return secret, nil
}
