package grant

import (
	"bytes"
	"context"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"github.com/mrchypark/rhiza"
	"io"
	"net/http"
	"strings"
	"time"
)

const remoteBodyLimit = 20 << 20

func encodeRemote(v any) ([]byte, error) {
	raw, e := json.Marshal(v)
	if e != nil {
		return nil, e
	}
	return raw, nil
}
func normalizeWireArgs(v []any) ([]any, error) {
	for i, a := range v {
		switch x := a.(type) {
		case json.Number:
			if n, e := x.Int64(); e == nil {
				v[i] = n
			} else if f, e := x.Float64(); e == nil {
				v[i] = f
			} else {
				return nil, e
			}
		case map[string]any:
			b, ok := x["$blob"].(string)
			if !ok || len(x) != 1 {
				return nil, fmt.Errorf("invalid SQL argument")
			}
			decoded, e := base64.StdEncoding.DecodeString(b)
			if e != nil {
				return nil, e
			}
			v[i] = decoded
		}
	}
	return v, nil
}
func wireArgs(args []any) []any {
	out := append([]any(nil), args...)
	for i, v := range out {
		if b, ok := v.([]byte); ok {
			out[i] = map[string]any{"$blob": base64.StdEncoding.EncodeToString(b)}
		}
	}
	return out
}
func (s *Store) remoteCall(ctx context.Context, path string, in, out any) error {
	if len(s.cfg.Database.RemoteNodes) == 0 {
		return fmt.Errorf("no quorum remote route configured")
	}
	switch v := in.(type) {
	case rhiza.QueryRequest:
		v.Args = wireArgs(v.Args)
		in = v
	case rhiza.ExecuteRequest:
		v.Args = wireArgs(v.Args)
		for i := range v.Statements {
			v.Statements[i].Args = wireArgs(v.Statements[i].Args)
		}
		in = v
	}
	payload, e := encodeRemote(in)
	if e != nil {
		return e
	}
	if len(payload) > remoteBodyLimit {
		return fmt.Errorf("runtime request too large")
	}
	// A failed mutation's commit status is unknown: never retry it at a different voter.
	address := s.cfg.Database.RemoteNodes[0] + path
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, address, bytes.NewReader(payload))
	if e != nil {
		return e
	}
	req.Header.Set("Authorization", "Bearer "+s.cfg.Database.RemoteAPISecret)
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 30 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	res, e := client.Do(req)
	if e != nil {
		return fmt.Errorf("runtime remote call failed (mutation status may be unknown): %w", e)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return fmt.Errorf("remote runtime returned %d", res.StatusCode)
	}
	raw, e := io.ReadAll(io.LimitReader(res.Body, remoteBodyLimit+1))
	if e != nil {
		return e
	}
	if len(raw) > remoteBodyLimit {
		return fmt.Errorf("remote response too large")
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if e = d.Decode(out); e != nil {
		return e
	}
	switch r := out.(type) {
	case *rhiza.QueryResponse:
		for i := range r.Rows {
			r.Rows[i], e = normalizeWireArgs(r.Rows[i])
			if e != nil {
				return e
			}
		}
	case *rhiza.ExecuteResponse:
		for i := range r.Statements {
			for j := range r.Statements[i].Rows {
				r.Statements[i].Rows[j], e = normalizeWireArgs(r.Statements[i].Rows[j])
				if e != nil {
					return e
				}
			}
		}
	}
	return nil
}
func (s *Store) RuntimeHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		secret := s.cfg.Database.RemoteAPISecret
		if len(secret) < 32 || subtle.ConstantTimeCompare([]byte(r.Header.Get("Authorization")), []byte("Bearer "+secret)) != 1 {
			http.Error(w, "unauthorized", 401)
			return
		}
		if s.db == nil {
			http.Error(w, "voter required", 503)
			return
		}
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", 405)
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), 25*time.Second)
		defer cancel()
		r.Body = http.MaxBytesReader(w, r.Body, remoteBodyLimit)
		raw, readErr := io.ReadAll(r.Body)
		if readErr != nil {
			http.Error(w, "runtime request too large", 413)
			return
		}
		if err := rejectDuplicateJSONKeys(raw); err != nil {
			http.Error(w, "invalid JSON", 400)
			return
		}
		d := json.NewDecoder(bytes.NewReader(raw))
		d.DisallowUnknownFields()
		d.UseNumber()
		var result any
		var err error
		switch r.URL.Path {
		case "/sql/query":
			var in rhiza.QueryRequest
			err = decodeRemoteRequest(d, &in)
			if err == nil {
				_, err = normalizeWireArgs(in.Args)
			}
			if err == nil && (in.Consistency != rhiza.ConsistencyLocal && in.Consistency != rhiza.ConsistencyLinearizable) {
				err = fmt.Errorf("invalid consistency")
			}
			if err == nil {
				var v rhiza.QueryResponse
				v, err = s.db.Query(ctx, in)
				for i := range v.Rows {
					v.Rows[i] = wireArgs(v.Rows[i])
				}
				result = v
			}
		case "/sql/execute":
			var in rhiza.ExecuteRequest
			err = decodeRemoteRequest(d, &in)
			if err == nil {
				_, err = normalizeWireArgs(in.Args)
				for i := range in.Statements {
					if err == nil {
						_, err = normalizeWireArgs(in.Statements[i].Args)
					}
				}
			}
			if err == nil {
				var v rhiza.ExecuteResponse
				v, err = s.db.Execute(ctx, in)
				for i := range v.Statements {
					for j := range v.Statements[i].Rows {
						v.Statements[i].Rows[j] = wireArgs(v.Statements[i].Rows[j])
					}
				}
				result = v
			}
		default:
			http.NotFound(w, r)
			return
		}
		if err != nil {
			http.Error(w, "runtime operation failed", 400)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(result)
	})
}

var _ = strings.TrimSpace

func decodeRemoteRequest(d *json.Decoder, v any) error {
	if e := d.Decode(v); e != nil {
		return e
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return fmt.Errorf("trailing JSON")
	}
	return nil
}
func rejectDuplicateJSONKeys(raw []byte) error {
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	var value func() error
	value = func() error {
		t, e := d.Token()
		if e != nil {
			return e
		}
		delim, ok := t.(json.Delim)
		if !ok {
			return nil
		}
		switch delim {
		case '{':
			seen := map[string]bool{}
			for d.More() {
				key, e := d.Token()
				if e != nil {
					return e
				}
				s, ok := key.(string)
				if !ok || seen[s] {
					return fmt.Errorf("duplicate JSON key")
				}
				seen[s] = true
				if e = value(); e != nil {
					return e
				}
			}
		case '[':
			for d.More() {
				if e = value(); e != nil {
					return e
				}
			}
		default:
			return fmt.Errorf("invalid JSON delimiter")
		}
		_, e = d.Token()
		return e
	}
	if e := value(); e != nil {
		return e
	}
	if _, e := d.Token(); e != io.EOF {
		return fmt.Errorf("trailing JSON")
	}
	return nil
}
