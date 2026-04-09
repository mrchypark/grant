package app

import (
	"net/http"
	"strings"
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
