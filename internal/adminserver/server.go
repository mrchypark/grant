// Package adminserver runs the standalone Grant admin web UI server.
package adminserver

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/mrchypark/grant/internal/adminauth"
	"github.com/mrchypark/grant/internal/adminui"
)

// Config holds validated environment settings for the admin UI server.
type Config struct {
	BindAddr                 string
	APIURL                   string
	ManagementOrganizationID string
	Issuer                   string
	ClientID                 string
	ClientSecret             string
	PublicURL                string
	Resource                 string
	Scopes                   []string
}

// Run starts the admin UI HTTP server and blocks until shutdown or error.
func Run() error {
	cfg, err := ConfigFromEnv(os.Getenv)
	if err != nil {
		return err
	}
	uiHandler, err := adminui.New(adminui.Config{
		APIURL: cfg.APIURL, ManagementOrganizationID: cfg.ManagementOrganizationID,
		HTTPClient: &http.Client{Timeout: 10 * time.Second},
	})
	if err != nil {
		return fmt.Errorf("initialize admin UI: %w", err)
	}
	handler, err := adminauth.New(adminauth.Config{
		Issuer: cfg.Issuer, ClientID: cfg.ClientID, ClientSecret: cfg.ClientSecret,
		PublicURL: cfg.PublicURL, Resource: cfg.Resource, Scopes: cfg.Scopes,
		HTTPClient: &http.Client{Timeout: 10 * time.Second},
	}, uiHandler)
	if err != nil {
		return fmt.Errorf("initialize admin authentication: %w", err)
	}
	listener, err := net.Listen("tcp", cfg.BindAddr)
	if err != nil {
		return fmt.Errorf("listen for admin UI: %w", err)
	}
	server := &http.Server{
		Handler: handler, ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout: 15 * time.Second, WriteTimeout: 30 * time.Second,
		IdleTimeout: 60 * time.Second,
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case err = <-done:
		if !errors.Is(err, http.ErrServerClosed) {
			return fmt.Errorf("serve admin UI: %w", err)
		}
	case <-ctx.Done():
		shutdown, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		if err = server.Shutdown(shutdown); err != nil {
			return fmt.Errorf("shut down admin UI: %w", err)
		}
		if err = <-done; !errors.Is(err, http.ErrServerClosed) {
			return fmt.Errorf("serve admin UI: %w", err)
		}
	}
	return nil
}

// ConfigFromEnv parses and validates admin server configuration from getenv.
func ConfigFromEnv(getenv func(string) string) (Config, error) {
	value := func(key string) string { return strings.TrimSpace(getenv(key)) }
	cfg := Config{
		BindAddr:                 value("GRANT_ADMIN_BIND_ADDR"),
		APIURL:                   value("GRANT_ADMIN_API_URL"),
		ManagementOrganizationID: value("GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID"),
		PublicURL:                value("GRANT_ADMIN_PUBLIC_URL"),
		Issuer:                   value("GRANT_ADMIN_GOAUTHY_ISSUER"),
		ClientID:                 value("GRANT_ADMIN_GOAUTHY_CLIENT_ID"),
		ClientSecret:             getenv("GRANT_ADMIN_GOAUTHY_CLIENT_SECRET"),
		Resource:                 value("GRANT_ADMIN_GOAUTHY_RESOURCE"),
		Scopes:                   strings.Fields(value("GRANT_ADMIN_GOAUTHY_SCOPES")),
	}
	if cfg.BindAddr == "" {
		cfg.BindAddr = "127.0.0.1:8095"
	}
	for key, field := range map[string]string{
		"GRANT_ADMIN_API_URL":                    cfg.APIURL,
		"GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID": cfg.ManagementOrganizationID,
		"GRANT_ADMIN_PUBLIC_URL":                 cfg.PublicURL,
		"GRANT_ADMIN_GOAUTHY_ISSUER":             cfg.Issuer,
		"GRANT_ADMIN_GOAUTHY_CLIENT_ID":          cfg.clientIDOrEmpty(),
		"GRANT_ADMIN_GOAUTHY_CLIENT_SECRET":      cfg.ClientSecret,
	} {
		if strings.TrimSpace(field) == "" {
			return Config{}, fmt.Errorf("required environment variable %s is empty", key)
		}
	}
	return cfg, nil
}

func (c Config) clientIDOrEmpty() string { return c.ClientID }
