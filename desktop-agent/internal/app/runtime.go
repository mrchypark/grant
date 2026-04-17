package app

import (
	"net/http"
	"os"
	"time"
)

type RunOptions struct {
	Addr string
}

const defaultExamplePageURL = "http://127.0.0.1:4173/"

func Run(addr string, handler http.Handler, opts RunOptions) error {
	return run(addr, handler, opts)
}

func newHTTPServer(addr string, handler http.Handler) *http.Server {
	return &http.Server{
		Addr:              addr,
		Handler:           handler,
		ReadHeaderTimeout: 15 * time.Second,
		ReadTimeout:       30 * time.Second,
		IdleTimeout:       60 * time.Second,
	}
}

func examplePageURL() string {
	if value := os.Getenv("DESKTOP_AGENT_EXAMPLE_URL"); value != "" {
		return value
	}
	return defaultExamplePageURL
}
