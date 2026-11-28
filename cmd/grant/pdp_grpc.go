package main

import (
	"crypto/tls"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"os"
	"sync"
	"time"

	"github.com/mrchypark/grant/internal/api"
	"github.com/mrchypark/grant/internal/grant"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
)

const (
	pdpGRPCListenEnv = "GRANT_PDP_GRPC_LISTEN_ADDR"
	pdpGRPCCertEnv   = "GRANT_PDP_GRPC_TLS_CERT_FILE"
	pdpGRPCKeyEnv    = "GRANT_PDP_GRPC_TLS_KEY_FILE"
)

type pdpGRPCConfig struct {
	addr string
	tls  *tls.Config
}

func pdpGRPCFromEnv(db grant.DatabaseConfig) (pdpGRPCConfig, error) {
	addr, certFile, keyFile := os.Getenv(pdpGRPCListenEnv), os.Getenv(pdpGRPCCertEnv), os.Getenv(pdpGRPCKeyEnv)
	if (certFile == "") != (keyFile == "") {
		return pdpGRPCConfig{}, errors.New("PDP gRPC TLS certificate and key must be configured together")
	}
	if addr == "" {
		if certFile != "" {
			return pdpGRPCConfig{}, errors.New("PDP gRPC TLS requires GRANT_PDP_GRPC_LISTEN_ADDR")
		}
		return pdpGRPCConfig{}, nil
	}
	if db.LocalNodeRole != "learner" || db.Rhiza.Learner != nil || db.RuntimeMode == "remote" {
		return pdpGRPCConfig{}, errors.New("PDP gRPC listener requires a passive Grant learner")
	}
	host, port, err := net.SplitHostPort(addr)
	if err != nil || port == "" {
		return pdpGRPCConfig{}, errors.New("GRANT_PDP_GRPC_LISTEN_ADDR must be a TCP host:port")
	}
	cfg := pdpGRPCConfig{addr: addr}
	if certFile == "" {
		ip, err := netip.ParseAddr(host)
		if err != nil || !(ip == netip.IPv6Loopback() || ip.Is4() && ip.IsLoopback()) {
			return pdpGRPCConfig{}, errors.New("PDP gRPC requires TLS unless bound to a literal loopback address")
		}
		return cfg, nil
	}

	pair, err := tls.LoadX509KeyPair(certFile, keyFile)
	if err != nil {
		// File paths and parser details may contain sensitive configuration.
		return pdpGRPCConfig{}, errors.New("cannot load PDP gRPC TLS certificate and key")
	}
	cfg.tls = &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{pair}}
	return cfg, nil
}

func (cfg pdpGRPCConfig) listen() (net.Listener, error) {
	if cfg.addr == "" {
		return nil, nil
	}
	listener, err := net.Listen("tcp", cfg.addr)
	if err != nil {
		return nil, fmt.Errorf("bind PDP gRPC listener: %w", err)
	}
	return listener, nil
}

func (cfg pdpGRPCConfig) start(listener net.Listener, service *grant.Service, done chan<- error) func() {
	opts := []grpc.ServerOption{
		grpc.MaxRecvMsgSize(api.PDPGRPCMaxWireBytes),
		grpc.MaxSendMsgSize(api.PDPGRPCMaxWireBytes),
		// Stop also joins connection setup. Bound unauthenticated handshakes.
		grpc.ConnectionTimeout(5 * time.Second),
	}
	if cfg.tls != nil {
		opts = append(opts, grpc.Creds(credentials.NewTLS(cfg.tls)))
	}
	server := grpc.NewServer(opts...)
	api.RegisterPDPGRPC(server, service)
	return servePDPGRPC(server, listener, done)
}

func servePDPGRPC(server *grpc.Server, listener net.Listener, done chan<- error) func() {
	exited := make(chan struct{})
	go func() {
		defer close(exited)
		err := server.Serve(listener)
		if errors.Is(err, grpc.ErrServerStopped) {
			err = nil
		}
		if err != nil {
			err = fmt.Errorf("serve PDP gRPC: %w", err)
		}
		done <- err
	}()
	return sync.OnceFunc(func() {
		// Stop closes transports and cancels RPCs without waiting on clients.
		// Join Serve so it cannot outlive this listener's store.
		server.Stop()
		<-exited
	})
}
