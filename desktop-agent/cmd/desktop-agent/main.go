package main

import (
	"log"
	"net"
	"net/http"
	"os"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/npki"
)

func main() {
	addr := os.Getenv("DESKTOP_AGENT_ADDR")
	if addr == "" {
		addr = "127.0.0.1:8765"
	}
	if err := ensureLoopback(addr); err != nil {
		log.Fatal(err)
	}

	server := httpapi.NewServer(npki.NewService(nil))
	log.Printf("desktop agent listening on %s", addr)
	if err := http.ListenAndServe(addr, server.Handler()); err != nil {
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
