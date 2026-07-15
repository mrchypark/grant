package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"flag"
	"fmt"
	"math/big"
	"net"
	"net/http"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"
)

type eventLog struct {
	path string
	mu   sync.Mutex
}

func (l *eventLog) write(origin, path string, authorized bool) {
	l.mu.Lock()
	defer l.mu.Unlock()
	f, err := os.OpenFile(l.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	defer f.Close()
	fmt.Fprintf(f, "%s path=%s authorization=%t\n", origin, path, authorized)
}

func serial() *big.Int {
	n, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		panic(err)
	}
	return n
}

func certificates() ([]byte, tls.Certificate, error) {
	now := time.Now()
	caPrivate, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, tls.Certificate{}, err
	}
	caTemplate := &x509.Certificate{
		SerialNumber:          serial(),
		Subject:               pkix.Name{CommonName: "grant redirect fixture CA"},
		NotBefore:             now.Add(-time.Minute),
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature,
		BasicConstraintsValid: true,
	}
	caDER, err := x509.CreateCertificate(rand.Reader, caTemplate, caTemplate, &caPrivate.PublicKey, caPrivate)
	if err != nil {
		return nil, tls.Certificate{}, err
	}
	serverPrivate, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, tls.Certificate{}, err
	}
	serverTemplate := &x509.Certificate{
		SerialNumber: serial(),
		Subject:      pkix.Name{CommonName: "127.0.0.1"},
		NotBefore:    now.Add(-time.Minute),
		NotAfter:     now.Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		IPAddresses:  []net.IP{net.ParseIP("127.0.0.1")},
	}
	serverDER, err := x509.CreateCertificate(rand.Reader, serverTemplate, caTemplate, &serverPrivate.PublicKey, caPrivate)
	if err != nil {
		return nil, tls.Certificate{}, err
	}
	caPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: caDER})
	serverPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: serverDER})
	keyBytes, err := x509.MarshalPKCS8PrivateKey(serverPrivate)
	if err != nil {
		return nil, tls.Certificate{}, err
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyBytes})
	cert, err := tls.X509KeyPair(serverPEM, keyPEM)
	return caPEM, cert, err
}

func listen(cert tls.Certificate) (net.Listener, error) {
	return tls.Listen("tcp", "127.0.0.1:0", &tls.Config{
		Certificates: []tls.Certificate{cert},
		MinVersion:   tls.VersionTLS12,
	})
}

func main() {
	readyPath := flag.String("ready", "", "path for shell-readable connection data")
	logPath := flag.String("log", "", "path for request events")
	bodyPath := flag.String("body", "", "response body served by the redirect target")
	flag.Parse()
	if *readyPath == "" || *logPath == "" || *bodyPath == "" {
		fmt.Fprintln(os.Stderr, "--ready, --log, and --body are required")
		os.Exit(2)
	}
	body, err := os.ReadFile(*bodyPath)
	if err != nil {
		panic(err)
	}
	caPEM, cert, err := certificates()
	if err != nil {
		panic(err)
	}
	originListener, err := listen(cert)
	if err != nil {
		panic(err)
	}
	targetListener, err := listen(cert)
	if err != nil {
		originListener.Close()
		panic(err)
	}
	httpTargetListener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		originListener.Close()
		targetListener.Close()
		panic(err)
	}
	originURL := "https://" + originListener.Addr().String()
	targetURL := "https://" + targetListener.Addr().String()
	httpTargetURL := "http://" + httpTargetListener.Addr().String()
	logger := &eventLog{path: *logPath}

	originMux := http.NewServeMux()
	originMux.HandleFunc("/success", func(w http.ResponseWriter, r *http.Request) {
		logger.write("origin", r.URL.Path, r.Header.Get("Authorization") != "")
		http.Redirect(w, r, targetURL+"/blob", http.StatusTemporaryRedirect)
	})
	originMux.HandleFunc("/http", func(w http.ResponseWriter, r *http.Request) {
		logger.write("origin", r.URL.Path, r.Header.Get("Authorization") != "")
		http.Redirect(w, r, httpTargetURL+"/blob", http.StatusTemporaryRedirect)
	})
	originMux.HandleFunc("/loop", func(w http.ResponseWriter, r *http.Request) {
		logger.write("origin", r.URL.Path, r.Header.Get("Authorization") != "")
		http.Redirect(w, r, originURL+"/loop", http.StatusTemporaryRedirect)
	})
	originMux.HandleFunc("/failure", func(w http.ResponseWriter, r *http.Request) {
		logger.write("origin", r.URL.Path, r.Header.Get("Authorization") != "")
		http.Redirect(w, r, "https://127.0.0.1:1/blob", http.StatusTemporaryRedirect)
	})
	targetMux := http.NewServeMux()
	targetMux.HandleFunc("/blob", func(w http.ResponseWriter, r *http.Request) {
		logger.write("target", r.URL.Path, r.Header.Get("Authorization") != "")
		w.Header().Set("Content-Type", "application/octet-stream")
		w.Write(body)
	})
	httpTargetMux := http.NewServeMux()
	httpTargetMux.HandleFunc("/blob", func(w http.ResponseWriter, r *http.Request) {
		logger.write("http-target", r.URL.Path, r.Header.Get("Authorization") != "")
		w.Header().Set("Content-Type", "application/octet-stream")
		w.Write(body)
	})

	originServer := &http.Server{Handler: originMux, ReadHeaderTimeout: 5 * time.Second}
	targetServer := &http.Server{Handler: targetMux, ReadHeaderTimeout: 5 * time.Second}
	httpTargetServer := &http.Server{Handler: httpTargetMux, ReadHeaderTimeout: 5 * time.Second}
	go func() {
		if err := originServer.Serve(originListener); err != nil && !errors.Is(err, http.ErrServerClosed) {
			panic(err)
		}
	}()
	go func() {
		if err := targetServer.Serve(targetListener); err != nil && !errors.Is(err, http.ErrServerClosed) {
			panic(err)
		}
	}()
	go func() {
		if err := httpTargetServer.Serve(httpTargetListener); err != nil && !errors.Is(err, http.ErrServerClosed) {
			panic(err)
		}
	}()

	caPath := *readyPath + ".ca.pem"
	if err := os.WriteFile(caPath, caPEM, 0o600); err != nil {
		panic(err)
	}
	ready := fmt.Sprintf(
		"CA_CERT='%s'\nORIGIN_URL='%s'\nTARGET_URL='%s'\nHTTP_TARGET_URL='%s'\n",
		caPath, originURL, targetURL, httpTargetURL,
	)
	tmpReady := *readyPath + ".tmp"
	if err := os.WriteFile(tmpReady, []byte(ready), 0o600); err != nil {
		panic(err)
	}
	if err := os.Rename(tmpReady, *readyPath); err != nil {
		panic(err)
	}

	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM, syscall.SIGHUP)
	<-signals
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	var shutdowns sync.WaitGroup
	for _, server := range []*http.Server{originServer, targetServer, httpTargetServer} {
		shutdowns.Add(1)
		go func(server *http.Server) {
			defer shutdowns.Done()
			_ = server.Shutdown(ctx)
		}(server)
	}
	shutdowns.Wait()
	os.Remove(*readyPath)
	os.Remove(caPath)
}
