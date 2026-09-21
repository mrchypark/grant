// Package api implements Grant's HTTP compatibility boundary.
package api

import (
	"bytes"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"reflect"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/Conalog/grant/internal/grant"
)

type Server struct {
	service  *grant.Service
	mux      *http.ServeMux
	learner  bool
	patterns []string
}
type request struct {
	*http.Request
	principal   grant.AuthenticatedPrincipal
	consistency grant.ReadConsistency
	bearer      string
	org         string
}
type endpoint func(*request) (any, error)
type preparedEndpoint struct {
	prepare func(*request) (endpoint, error)
}

func New(service *grant.Service, learner bool) *Server {
	s := &Server{service: service, mux: http.NewServeMux(), learner: learner}
	s.routes()
	return s
}
func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if s.learner && r.URL.Path != "/healthz" && r.URL.Path != "/readyz" && r.URL.Path != "/api/v2/http-capabilities/decide" && !strings.HasPrefix(r.URL.Path, "/api/v2/http-capabilities/catalog/") {
		if _, pattern := s.mux.Handler(r); pattern == "" {
			http.NotFound(w, r)
			return
		}
		writeError(w, failure(503, "read replica only serves capability decisions and catalogs"))
		return
	}
	s.mux.ServeHTTP(w, r)
}
func failure(status int, msg string) error { return &grant.Error{Status: status, Message: msg} }
func writeError(w http.ResponseWriter, err error) {
	writeJSON(w, grant.ErrorStatus(err), map[string]string{"error": err.Error()})
}
func writeJSON(w http.ResponseWriter, status int, v any) {
	if snapshot, ok := v.(snapshotResponse); ok {
		w.Header().Set("ETag", snapshot.ETag)
		v = snapshot.Value
	}
	if v == nil {
		w.WriteHeader(status)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
func (s *Server) route(method, path, auth string, voter bool, status int, fn any) {
	var prepare func(*request) (endpoint, error)
	switch f := fn.(type) {
	case preparedEndpoint:
		prepare = f.prepare
	case func(*request) (any, error):
		prepare = func(*request) (endpoint, error) { return endpoint(f), nil }
	case endpoint:
		prepare = func(*request) (endpoint, error) { return f, nil }
	default:
		panic("invalid endpoint registration")
	}
	s.patterns = append(s.patterns, method+" "+path)
	s.mux.HandleFunc(method+" "+path, func(w http.ResponseWriter, r *http.Request) {
		if voter && s.learner {
			writeError(w, failure(503, "endpoint requires voter node"))
			return
		}
		q := &request{Request: r}
		var err error
		if r.Method == "GET" || strings.HasPrefix(r.URL.Path, "/api/v1/authz/") {
			q.consistency, err = readConsistency(r)
			if err != nil {
				writeError(w, err)
				return
			}
		}
		handler, err := prepare(q)
		if err != nil {
			writeError(w, err)
			return
		}
		if auth == "master" {
			if len(r.Header.Values("X-Grant-Master-Key")) > 0 {
				err = requireKey(r, "x-grant-master-key", s.service.Config().Security.MasterKey, "master key")
			} else if route, ok := grant.LookupManagementRoute(method, path); ok && route.Platform {
				q.bearer, err = bearerToken(r)
				if err == nil && q.bearer == "" {
					err = failure(401, "unauthorized: missing bearer token")
				}
				if err == nil {
					q.org, err = header(r, "x-organization-id", true)
				}
				if err == nil {
					var actor string
					actor, err = s.service.AuthorizePlatformManagement(r.Context(), q.bearer, q.org, route.ActionCode)
					if err == nil {
						r = r.WithContext(grant.WithManagementActor(r.Context(), actor))
						q.Request = r
					}
				}
			} else {
				err = failure(401, "unauthorized: missing x-grant-master-key header")
			}
		}
		if auth == "app" || auth == "application" {
			err = requireKey(r, "x-grant-app-auth-key", s.service.Config().Security.AppAuthKey, "application authorization key")
		}
		if err != nil {
			writeError(w, err)
			return
		}
		if auth == "user" || auth == "inactive" || auth == "application" || auth == "bearer" {
			q.bearer, err = bearerToken(r)
			if err == nil {
				q.org, err = header(r, "x-organization-id", auth == "user" || auth == "application" || (auth == "inactive" && s.service.AuthProviderMode() == "pocketbase"))
			}
			if err != nil {
				writeError(w, err)
				return
			}
			if auth == "application" && q.bearer == "" {
				writeError(w, failure(401, "unauthorized: missing authorization header"))
				return
			}
			if auth == "user" || auth == "inactive" {
				if r.Method != "GET" || strings.HasSuffix(r.URL.Path, "/consent") {
					q.consistency = "strong"
				}
				if auth == "inactive" {
					if s.service.AuthProviderMode() == "pocketbase" && q.org != r.PathValue("organizationId") {
						writeError(w, failure(403, "forbidden: organization path does not match selected organization"))
						return
					}
					q.principal, err = s.service.AuthenticateRequestForInactiveOrganization(r.Context(), q.bearer, r.PathValue("organizationId"), q.consistency)
				} else {
					q.principal, err = s.service.Authenticate(r.Context(), q.bearer, q.org, q.consistency)
				}
				if err == nil && auth != "inactive" && r.PathValue("organizationId") != "" && r.PathValue("organizationId") != q.principal.OrganizationID {
					err = failure(403, fmt.Sprintf("forbidden: organization path %s does not match active organization %s", r.PathValue("organizationId"), q.principal.OrganizationID))
				}
			}
		}
		if err != nil {
			writeError(w, err)
			return
		}
		result, err := handler(q)
		if err != nil {
			writeError(w, err)
			return
		}
		writeJSON(w, status, result)
	})
}
func header(r *http.Request, name string, required bool) (string, error) {
	values := r.Header.Values(name)
	if len(values) == 0 {
		if required {
			return "", failure(400, "missing "+name+" header")
		}
		return "", nil
	}
	value := values[0]
	if !utf8.ValidString(value) {
		return "", failure(400, name+" header is not valid UTF-8")
	}
	value = strings.TrimSpace(value)
	if value == "" {
		return "", failure(400, name+" header must not be empty")
	}
	return value, nil
}
func bearerToken(r *http.Request) (string, error) {
	v, e := header(r, "authorization", false)
	if e != nil || v == "" {
		return v, e
	}
	token, ok := strings.CutPrefix(v, "Bearer ")
	if !ok {
		token, ok = strings.CutPrefix(v, "bearer ")
	}
	if !ok {
		return "", failure(400, "authorization header must use Bearer")
	}
	if strings.TrimSpace(token) == "" {
		return "", failure(400, "authorization bearer token must not be empty")
	}
	return v, nil
}
func requireKey(r *http.Request, name, expected, label string) error {
	v, e := header(r, name, false)
	if e != nil {
		return e
	}
	if v == "" {
		return failure(401, "unauthorized: missing "+name+" header")
	}
	a, b := sha256.Sum256([]byte(v)), sha256.Sum256([]byte(expected))
	if subtle.ConstantTimeCompare(a[:], b[:]) != 1 {
		return failure(401, "unauthorized: invalid "+label)
	}
	return nil
}
func readConsistency(r *http.Request) (grant.ReadConsistency, error) {
	v, ok := r.URL.Query()["consistency"]
	if !ok {
		return "", nil
	}
	if len(v) != 1 {
		return "", failure(400, "duplicate consistency")
	}
	value := strings.ToLower(strings.TrimSpace(v[0]))
	if value != "strong" {
		return "", failure(400, "invalid consistency: "+value)
	}
	return "strong", nil
}

// decode preserves required non-null fields and closed structs instead of Go's
// default zero-value coercion. Legacy request DTOs explicitly allow unknown keys.
func decode[T any](r *http.Request, closed bool) (T, error) {
	var out T
	mt, _, e := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if e != nil || !(mt == "application/json" || strings.HasSuffix(mt, "+json")) {
		return out, failure(415, "Expected request with Content-Type: application/json")
	}
	raw, e := io.ReadAll(io.LimitReader(r.Body, 2*1024*1024+1))
	if e != nil {
		return out, failure(400, "Failed to read request body")
	}
	if len(raw) > 2*1024*1024 {
		return out, failure(413, "length limit exceeded")
	}
	if !utf8.Valid(raw) {
		return out, failure(400, "invalid JSON UTF-8")
	}
	if !json.Valid(raw) {
		return out, failure(400, "invalid JSON syntax")
	}
	if !validJSONSurrogates(raw) {
		return out, failure(400, "invalid JSON Unicode escape")
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if e = validateJSON(d, reflect.TypeFor[T](), closed); e != nil {
		return out, failure(422, e.Error())
	}
	if _, e = d.Token(); !errors.Is(e, io.EOF) {
		return out, failure(400, "trailing JSON input")
	}
	if e = json.Unmarshal(raw, &out); e != nil {
		return out, failure(422, e.Error())
	}
	if e = validateEnums(out); e != nil {
		return out, e
	}
	return out, nil
}

// encoding/json replaces unpaired UTF-16 escapes; serde_json rejects them.
// Run after json.Valid, including strings in ignored extension fields.
func validJSONSurrogates(raw []byte) bool {
	for i := 0; i < len(raw); i++ {
		if raw[i] != '\\' {
			continue
		}
		i++
		if raw[i] != 'u' {
			continue
		}
		v, _ := strconv.ParseUint(string(raw[i+1:i+5]), 16, 16)
		i += 4
		if v >= 0xdc00 && v <= 0xdfff {
			return false
		}
		if v < 0xd800 || v > 0xdbff {
			continue
		}
		if i+6 >= len(raw) || raw[i+1] != '\\' || raw[i+2] != 'u' {
			return false
		}
		low, e := strconv.ParseUint(string(raw[i+3:i+7]), 16, 16)
		if e != nil || low < 0xdc00 || low > 0xdfff {
			return false
		}
		i += 6
	}
	return true
}
func validateJSON(d *json.Decoder, t reflect.Type, closed bool) error {
	token, e := d.Token()
	if e != nil {
		return e
	}
	if t.Kind() == reflect.Pointer {
		if token == nil {
			return nil
		}
		t = t.Elem()
	}
	if token == nil && (t.Kind() == reflect.Interface || t == reflect.TypeFor[json.RawMessage]()) {
		return nil
	}
	if token == nil {
		return errors.New("required value must not be null")
	}
	delim, isDelim := token.(json.Delim)
	if !isDelim {
		return nil
	}
	if delim == '[' {
		var elem reflect.Type
		if t.Kind() == reflect.Slice || t.Kind() == reflect.Array {
			elem = t.Elem()
		} else {
			elem = reflect.TypeFor[any]()
		}
		for d.More() {
			if e = validateJSON(d, elem, closed); e != nil {
				return e
			}
		}
		_, e = d.Token()
		return e
	}
	if delim != '{' {
		return errors.New("invalid JSON delimiter")
	}
	fields := map[string]reflect.StructField{}
	if t.Kind() == reflect.Struct {
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			name := strings.Split(f.Tag.Get("json"), ",")[0]
			if name == "" {
				name = f.Name
			}
			if name != "-" {
				fields[name] = f
			}
		}
	}
	seen := map[string]bool{}
	for d.More() {
		key, e := d.Token()
		if e != nil {
			return e
		}
		name := key.(string)
		if seen[name] {
			return fmt.Errorf("duplicate field %s", name)
		}
		seen[name] = true
		f, ok := fields[name]
		if !ok && closed && t.Kind() == reflect.Struct {
			return fmt.Errorf("unknown field %s", name)
		}
		ft := reflect.TypeFor[any]()
		if ok {
			ft = f.Type
		}
		if e = validateJSON(d, ft, closed); e != nil {
			return e
		}
	}
	if _, e = d.Token(); e != nil {
		return e
	}
	for name, f := range fields {
		if !seen[name] && f.Type.Kind() != reflect.Pointer && !strings.Contains(f.Tag.Get("json"), "omitempty") && f.Tag.Get("optional") != "true" {
			return fmt.Errorf("missing field %s", name)
		}
	}
	return nil
}
func body[T any](closed bool, fn func(*request, T) (any, error)) preparedEndpoint {
	return preparedEndpoint{prepare: func(r *request) (endpoint, error) {
		v, e := decode[T](r.Request, closed)
		if e != nil {
			return nil, e
		}
		return func(r *request) (any, error) { return fn(r, v) }, nil
	}}
}
