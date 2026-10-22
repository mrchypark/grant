package main

import (
	"context"
	"errors"
	"fmt"
	"github.com/mrchypark/grant/internal/api"
	"github.com/mrchypark/grant/internal/grant"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"sync"
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
	var operator net.Listener
	if cfg.Database.OperatorBindAddr != "" {
		operator, e = net.Listen("tcp", cfg.Database.OperatorBindAddr)
		if e != nil {
			return e
		}
		defer operator.Close()
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	done := make(chan error, 4)
	servers := make([]*http.Server, 0, 3)
	startOperator := func(handler http.Handler) error {
		if operator == nil {
			return nil
		}
		server := &http.Server{Handler: handler, ReadHeaderTimeout: 5 * time.Second, IdleTimeout: 60 * time.Second}
		servers = append(servers, server)
		go func() { done <- server.Serve(operator) }()
		return nil
	}
	store, e := grant.OpenStoreWithOperator(ctx, cfg, startOperator)
	if e != nil {
		return e
	}
	defer store.Close()
	service := grant.NewService(cfg, store)
	promotionLearner := cfg.Database.Rhiza.Learner != nil
	passiveLearner := cfg.Database.LocalNodeRole == "learner"
	if !promotionLearner && !passiveLearner {
		if e = service.Bootstrap(ctx); e != nil {
			return fmt.Errorf("initialize Grant catalog: %w", e)
		}
	}
	var publicHandler http.Handler
	var runtimeHandler http.Handler
	var publicHandlerMu sync.RWMutex
	publicHandler = api.New(service, passiveLearner)
	runtimeHandler = store.RuntimeHandler()
	if promotionLearner {
		publicHandler = promotionGate{}
		runtimeHandler = promotionGate{private: true}
		go func() {
			if err := store.WaitForPromotion(ctx); err != nil {
				if ctx.Err() == nil {
					done <- err
				}
				return
			}
			if err := service.Bootstrap(ctx); err != nil {
				if ctx.Err() == nil {
					done <- fmt.Errorf("initialize promoted Grant catalog: %w", err)
				}
				return
			}
			publicHandlerMu.Lock()
			publicHandler = api.New(service, false)
			runtimeHandler = store.RuntimeHandler()
			publicHandlerMu.Unlock()
		}()
	}
	server := &http.Server{Addr: cfg.Server.BindAddr, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		publicHandlerMu.RLock()
		h := publicHandler
		publicHandlerMu.RUnlock()
		h.ServeHTTP(w, r)
	}), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}
	go func() { done <- server.Serve(public) }()
	servers = append(servers, server)
	if private != nil {
		runtimeServer := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			publicHandlerMu.RLock()
			h := runtimeHandler
			publicHandlerMu.RUnlock()
			h.ServeHTTP(w, r)
		}), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}
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

type promotionGate struct{ private bool }

func (g promotionGate) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if !g.private && r.URL.Path == "/healthz" {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"status":"ok"}`))
		return
	}
	http.Error(w, "Rhiza learner awaiting promotion", http.StatusServiceUnavailable)
}
