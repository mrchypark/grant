//go:build !darwin

package app

import (
	"net/http"
)

func run(addr string, handler http.Handler, _ RunOptions) error {
	return http.ListenAndServe(addr, handler)
}
