package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

type npkiService interface {
	ListCertificates(context.Context) ([]npki.CertificateMetadata, error)
	Sign(context.Context, npki.SignRequest) (npki.SignResult, error)
}

type Server struct {
	npki npkiService
	mux  *http.ServeMux
}

func NewServer(service npkiService) *Server {
	server := &Server{npki: service, mux: http.NewServeMux()}
	server.routes()
	return server
}

func (s *Server) Handler() http.Handler {
	return s.mux
}

func (s *Server) routes() {
	s.mux.HandleFunc("/healthz", s.handleHealthz)
	s.mux.HandleFunc("/certificates", s.handleCertificates)
	s.mux.HandleFunc("/sign", s.handleSign)
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w, http.MethodGet)
		return
	}
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("ok"))
}

func (s *Server) handleCertificates(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w, http.MethodGet)
		return
	}
	certificates, err := s.npki.ListCertificates(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": certificates})
}

func (s *Server) handleSign(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w, http.MethodPost)
		return
	}
	defer r.Body.Close()

	var req npki.SignRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid json body")
		return
	}
	req.CertificateID = strings.TrimSpace(req.CertificateID)
	req.Challenge = strings.TrimSpace(req.Challenge)
	req.Password = strings.TrimSpace(req.Password)

	switch {
	case req.Challenge == "":
		writeError(w, http.StatusBadRequest, "challenge must not be empty")
		return
	case req.Password == "":
		writeError(w, http.StatusBadRequest, "password must not be empty")
		return
	}

	result, err := s.npki.Sign(r.Context(), req)
	if err != nil {
		status := http.StatusInternalServerError
		switch {
		case errors.Is(err, npki.ErrNotImplemented):
			status = http.StatusNotImplemented
		case errors.Is(err, npki.ErrCertificateNotFound):
			status = http.StatusNotFound
		case errors.Is(err, npki.ErrInvalidPassword):
			status = http.StatusUnauthorized
		}
		writeError(w, status, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, result)
}

func writeMethodNotAllowed(w http.ResponseWriter, method string) {
	w.Header().Set("Allow", method)
	writeError(w, http.StatusMethodNotAllowed, "method not allowed")
}

func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}

func writeError(w http.ResponseWriter, status int, message string) {
	writeJSON(w, status, map[string]string{"error": message})
}
