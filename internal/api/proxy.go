package api

import (
	"bytes"
	"errors"
	"io"
	"net"
	"net/http"
	"strings"
)

func (s *Server) proxy(w http.ResponseWriter, r *http.Request) {
	if s.service.AuthProviderMode() != "rauthy" {
		writeError(w, failure(404, "not found"))
		return
	}
	raw, e := io.ReadAll(io.LimitReader(r.Body, 10*1024*1024+1))
	if e != nil || len(raw) > 10*1024*1024 {
		writeError(w, failure(500, "length limit exceeded"))
		return
	}
	target := strings.TrimRight(s.service.RauthyBaseURL(), "/") + r.URL.EscapedPath()
	if r.URL.RawQuery != "" {
		target += "?" + r.URL.RawQuery
	}
	req, e := http.NewRequestWithContext(r.Context(), r.Method, target, bytes.NewReader(raw))
	if e != nil {
		writeError(w, failure(502, "Rauthy upstream request failed"))
		return
	}
	req.Header = r.Header.Clone()
	req.Header.Del("Host")
	req.Header.Del("Content-Length")
	res, e := s.service.RauthyProxyClient().Do(req)
	if e != nil {
		proxyFailure(w, e)
		return
	}
	defer res.Body.Close()
	data, e := io.ReadAll(res.Body)
	if e != nil {
		proxyFailure(w, e)
		return
	}
	for name, values := range res.Header {
		if strings.EqualFold(name, "Content-Length") {
			continue
		}
		for _, v := range values {
			w.Header().Add(name, v)
		}
	}
	w.WriteHeader(res.StatusCode)
	_, _ = w.Write(data)
}
func proxyFailure(w http.ResponseWriter, e error) {
	var ne net.Error
	if errors.As(e, &ne) && ne.Timeout() {
		writeError(w, failure(504, "Rauthy upstream request timed out"))
	} else {
		writeError(w, failure(502, "Rauthy upstream request failed"))
	}
}
