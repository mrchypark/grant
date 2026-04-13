package app

import (
	"net/http"
	"strings"
	"time"
)

type RunOptions struct {
	Addr string
}

func shouldUseTray(goos string, disableEnv string) bool {
	return goos == "darwin" && strings.TrimSpace(disableEnv) == ""
}

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
