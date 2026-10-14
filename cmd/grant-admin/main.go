package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/Conalog/grant/internal/adminauth"
	"github.com/Conalog/grant/internal/adminui"
)

type config struct {
	bindAddr, apiURL, managementOrganizationID          string
	issuer, clientID, clientSecret, publicURL, resource string
	scopes                                              []string
}

func main() {
	if err := run(); err != nil {
		slog.Error("Grant admin stopped", "error", err)
		os.Exit(1)
	}
}

func run() error {
	cfg, err := configFromEnv(os.Getenv)
	if err != nil {
		return err
	}
	uiHandler, err := adminui.New(adminui.Config{
		APIURL: cfg.apiURL, ManagementOrganizationID: cfg.managementOrganizationID,
		HTTPClient: &http.Client{Timeout: 10 * time.Second},
	})
	if err != nil {
		return fmt.Errorf("initialize admin UI: %w", err)
	}
	handler, err := adminauth.New(adminauth.Config{
		Issuer: cfg.issuer, ClientID: cfg.clientID, ClientSecret: cfg.clientSecret,
		PublicURL: cfg.publicURL, Resource: cfg.resource, Scopes: cfg.scopes,
		HTTPClient: &http.Client{Timeout: 10 * time.Second},
	}, uiHandler)
	if err != nil {
		return fmt.Errorf("initialize admin authentication: %w", err)
	}
	listener, err := net.Listen("tcp", cfg.bindAddr)
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

func configFromEnv(getenv func(string) string) (config, error) {
	value := func(key string) string { return strings.TrimSpace(getenv(key)) }
	cfg := config{
		bindAddr:                 value("GRANT_ADMIN_BIND_ADDR"),
		apiURL:                   value("GRANT_ADMIN_API_URL"),
		managementOrganizationID: value("GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID"),
		publicURL:                value("GRANT_ADMIN_PUBLIC_URL"),
		issuer:                   value("GRANT_ADMIN_GOAUTHY_ISSUER"),
		clientID:                 value("GRANT_ADMIN_GOAUTHY_CLIENT_ID"),
		clientSecret:             getenv("GRANT_ADMIN_GOAUTHY_CLIENT_SECRET"),
		resource:                 value("GRANT_ADMIN_GOAUTHY_RESOURCE"),
		scopes:                   strings.Fields(value("GRANT_ADMIN_GOAUTHY_SCOPES")),
	}
	if cfg.bindAddr == "" {
		cfg.bindAddr = "127.0.0.1:8095"
	}
	for key, field := range map[string]string{
		"GRANT_ADMIN_API_URL":                    cfg.apiURL,
		"GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID": cfg.managementOrganizationID,
		"GRANT_ADMIN_PUBLIC_URL":                 cfg.publicURL,
		"GRANT_ADMIN_GOAUTHY_ISSUER":             cfg.issuer,
		"GRANT_ADMIN_GOAUTHY_CLIENT_ID":          cfg.clientID,
		"GRANT_ADMIN_GOAUTHY_CLIENT_SECRET":      cfg.clientSecret,
	} {
		if strings.TrimSpace(field) == "" {
			return config{}, fmt.Errorf("required environment variable %s is empty", key)
		}
	}
	return cfg, nil
}
