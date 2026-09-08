package main

import (
	"context"
	"errors"
	"fmt"
	"github.com/Conalog/grant/internal/api"
	"github.com/Conalog/grant/internal/grant"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"
)

func main() {
	if e := run(); e != nil {
		slog.Error("Grant stopped", "error", e)
		os.Exit(1)
	}
}
func run() error {
	if handled, err := commandLine(context.Background()); handled {
		return err
	}
	cfg, e := grant.ConfigFromEnv()
	if e != nil {
		return e
	}
	// Bind both listeners before touching storage. A collision cannot leave a
	// partially initialized database or expose private SQL on the public mux.
	public, e := net.Listen("tcp", cfg.Server.BindAddr)
	if e != nil {
		return e
	}
	defer public.Close()
	var private net.Listener
	if cfg.Database.RuntimeBindAddr != "" {
		private, e = net.Listen("tcp", cfg.Database.RuntimeBindAddr)
		if e != nil {
			return e
		}
		defer private.Close()
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	store, e := grant.OpenStore(ctx, cfg)
	if e != nil {
		return e
	}
	defer store.Close()
	service := grant.NewService(cfg, store)
	if cfg.Database.LocalNodeRole != "learner" {
		if e = service.Bootstrap(ctx); e != nil {
			return fmt.Errorf("initialize Grant catalog: %w", e)
		}
	}
	server := &http.Server{Addr: cfg.Server.BindAddr, Handler: api.New(service, cfg.Database.LocalNodeRole == "learner"), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}
	done := make(chan error, 2)
	go func() { done <- server.Serve(public) }()
	servers := []*http.Server{server}
	if private != nil {
		runtimeServer := &http.Server{Handler: store.RuntimeHandler(), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}
		servers = append(servers, runtimeServer)
		go func() { done <- runtimeServer.Serve(private) }()
	}
	defer func() {
		for _, srv := range servers {
			_ = srv.Close()
		}
	}()
	select {
	case e = <-done:
		if !errors.Is(e, http.ErrServerClosed) {
			return e
		}
	case <-ctx.Done():
		shutdown, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		for _, srv := range servers {
			if e = srv.Shutdown(shutdown); e != nil {
				return fmt.Errorf("HTTP shutdown: %w", e)
			}
		}
	}
	return nil
}
