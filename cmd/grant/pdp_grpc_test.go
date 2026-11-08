package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
	"github.com/mrchypark/rhiza"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/health"
	healthpb "google.golang.org/grpc/health/grpc_health_v1"
)

func TestPDPGRPCConfiguration(t *testing.T) {
	cert, key := testPDPGRPCKeyPair(t)
	for _, tc := range []struct {
		name, addr, cert, key, role, mode, wantError string
		promotion, wantTLS                           bool
	}{
		{name: "disabled", role: "voter"},
		{name: "IPv4 loopback", addr: "127.3.2.1:0", role: "learner"},
		{name: "IPv6 loopback", addr: "[::1]:0", role: "learner"},
		{name: "voter", addr: "127.0.0.1:0", role: "voter", wantError: "passive Grant learner"},
		{name: "promotion learner", addr: "127.0.0.1:0", role: "learner", promotion: true, wantError: "passive Grant learner"},
		{name: "remote without replica", addr: "127.0.0.1:0", role: "learner", mode: "remote", wantError: "passive Grant learner"},
		{name: "hostname", addr: "localhost:0", role: "learner", wantError: "requires TLS"},
		{name: "wildcard", addr: ":0", role: "learner", wantError: "requires TLS"},
		{name: "IPv6 wildcard", addr: "[::]:0", role: "learner", wantError: "requires TLS"},
		{name: "mapped IPv4", addr: "[::ffff:127.0.0.1]:0", role: "learner", wantError: "requires TLS"},
		{name: "nonloopback", addr: "192.0.2.1:0", role: "learner", wantError: "requires TLS"},
		{name: "missing port", addr: "127.0.0.1:", role: "learner", wantError: "host:port"},
		{name: "certificate only", addr: "127.0.0.1:0", cert: cert, role: "learner", wantError: "configured together"},
		{name: "key only", addr: "127.0.0.1:0", key: key, role: "learner", wantError: "configured together"},
		{name: "TLS without listener", cert: cert, key: key, role: "learner", wantError: "requires GRANT_PDP_GRPC_LISTEN_ADDR"},
		{name: "missing TLS file", addr: ":0", cert: cert + "-missing", key: key, role: "learner", wantError: "cannot load"},
		{name: "TLS wildcard", addr: ":0", cert: cert, key: key, role: "learner", wantTLS: true},
		{name: "TLS loopback", addr: "127.0.0.1:0", cert: cert, key: key, role: "learner", wantTLS: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv(pdpGRPCListenEnv, tc.addr)
			t.Setenv(pdpGRPCCertEnv, tc.cert)
			t.Setenv(pdpGRPCKeyEnv, tc.key)
			db := grant.DatabaseConfig{LocalNodeRole: tc.role, RuntimeMode: tc.mode}
			if tc.promotion {
				db.Rhiza.Learner = &rhiza.Member{}
			}
			cfg, err := pdpGRPCFromEnv(db)
			if tc.wantError != "" {
				if err == nil || !strings.Contains(err.Error(), tc.wantError) {
					t.Fatalf("expected %q error, got %v", tc.wantError, err)
				}
				if strings.Contains(err.Error(), cert) || strings.Contains(err.Error(), key) {
					t.Fatal("TLS error disclosed a file path")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if cfg.addr != tc.addr || (cfg.tls != nil) != tc.wantTLS {
				t.Fatal("listener or TLS state does not match configuration")
			}
			if cfg.tls != nil && (cfg.tls.MinVersion != tls.VersionTLS12 || len(cfg.tls.Certificates) != 1) {
				t.Fatal("TLS must load the pair and require TLS 1.2 or later")
			}
			if cfg.addr == "" {
				listener, err := cfg.listen()
				if listener != nil {
					_ = listener.Close()
				}
				if err != nil || listener != nil {
					t.Fatalf("disabled configuration unexpectedly bound a listener: %v", err)
				}
			}
		})
	}
}

func testPDPGRPCKeyPair(t *testing.T) (string, string) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	cert := &x509.Certificate{
		SerialNumber: big.NewInt(1), DNSNames: []string{"pdp.example.test"},
		NotBefore: now.Add(-time.Minute), NotAfter: now.Add(time.Hour),
		KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}
	der, err := x509.CreateCertificate(rand.Reader, cert, cert, key.Public(), key)
	if err != nil {
		t.Fatal(err)
	}
	keyDER, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	certPath, keyPath := filepath.Join(dir, "cert.pem"), filepath.Join(dir, "key.pem")
	if err := os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyDER}), 0600); err != nil {
		t.Fatal(err)
	}
	return certPath, keyPath
}

func TestPDPGRPCListenerCollisionAndShutdown(t *testing.T) {
	listener, err := (pdpGRPCConfig{addr: "127.0.0.1:0"}).listen()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	collision, err := (pdpGRPCConfig{addr: listener.Addr().String()}).listen()
	if collision != nil {
		_ = collision.Close()
	}
	if err == nil {
		t.Fatal("a second listener accepted the same address")
	}

	server := grpc.NewServer()
	// A long-running stream makes a graceful shutdown wait on the client.
	// The health service here is test-only.
	healthpb.RegisterHealthServer(server, health.NewServer())
	done := make(chan error, 1)
	stop := servePDPGRPC(server, listener, done)
	t.Cleanup(stop)
	conn, err := grpc.NewClient(listener.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	stream, err := healthpb.NewHealthClient(conn).Watch(ctx, &healthpb.HealthCheckRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := stream.Recv(); err != nil {
		t.Fatalf("start test stream: %v", err)
	}
	stop()
	if err := <-done; err != nil {
		t.Fatalf("normal shutdown reported a serve error: %v", err)
	}
	if _, err := stream.Recv(); err == nil || ctx.Err() != nil {
		t.Fatalf("shutdown did not cancel the stream before its deadline: %v", err)
	}
}

func TestPDPGRPCReportsUnexpectedServeError(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	stop := servePDPGRPC(grpc.NewServer(), listener, done)
	defer stop()
	select {
	case err := <-done:
		if !errors.Is(err, net.ErrClosed) {
			t.Fatalf("expected closed-listener serve error, got %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("gRPC serve error was not delivered")
	}
}
