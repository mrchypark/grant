package grant

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"fmt"
	"github.com/mrchypark/rhiza"
	"io"
	"net/http"
	"time"
)

type replicaReadKey struct{}
type replicaIdentityKey struct{}
type replicaRead struct {
	fence, slot     uint64
	pinned, changed bool
}

func readScope(ctx context.Context) *replicaRead {
	v, _ := ctx.Value(replicaReadKey{}).(*replicaRead)
	return v
}

// Every successful response is read from one applied snapshot at or beyond a
// quorum fence obtained after this request started. A local-ready bit is not a fence.
func localReplicaRead[T any](ctx context.Context, s *Service, read func(context.Context) (T, error)) (T, error) {
	var zero T
	if s.cfg.Database.LocalNodeRole != "learner" || readScope(ctx) != nil {
		return read(ctx)
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	fence, e := s.store.readFence(ctx)
	if e != nil {
		return zero, serviceError(503, "unavailable: replica quorum fence failed")
	}
	return readAtFence(ctx, s, fence, read)
}
func readAtFence[T any](ctx context.Context, s *Service, fence uint64, read func(context.Context) (T, error)) (T, error) {
	var zero T
	for attempt := 0; attempt < 3; attempt++ {
		if s.store.replica != nil {
			if e := s.store.catchUpFence(ctx, fence); e != nil {
				return zero, serviceError(503, "unavailable: replica has not reached quorum fence")
			}
		}
		scope := &replicaRead{fence: fence}
		readCtx := context.WithValue(ctx, replicaReadKey{}, scope)
		result, e := read(readCtx)
		if scope.changed {
			continue
		}
		return result, e
	}
	return zero, serviceError(503, "unavailable: replica snapshot changed during decision")
}
func (s *Store) catchUpFence(ctx context.Context, fence uint64) error {
	if s.replica == nil {
		return fmt.Errorf("read replica required")
	}
	for {
		if e := s.replica.Sync(ctx); e != nil {
			return e
		}
		if s.replica.Status().AppliedSlot >= fence {
			return nil
		}
		timer := time.NewTimer(20 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
		}
	}
}
func (s *Store) readFence(ctx context.Context) (uint64, error) {
	if len(s.cfg.Database.ReadFenceKey) < 32 || len(s.cfg.Database.RemoteNodes) == 0 {
		return 0, fmt.Errorf("replica fence transport is not configured")
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	defer transport.CloseIdleConnections()
	client := http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, s.cfg.Database.RemoteNodes[0]+"/read-fence", nil)
	if e != nil {
		return 0, e
	}
	req.Header.Set("Authorization", "Bearer "+s.cfg.Database.ReadFenceKey)
	res, e := client.Do(req)
	if e != nil {
		return 0, e
	}
	defer res.Body.Close()
	if res.StatusCode != 200 {
		return 0, fmt.Errorf("fence unavailable")
	}
	d := json.NewDecoder(http.MaxBytesReader(nil, res.Body, 1024))
	start, e := d.Token()
	if e != nil || start != json.Delim('{') {
		return 0, fmt.Errorf("invalid fence response")
	}
	name, e := d.Token()
	if e != nil || name != "appliedSlot" {
		return 0, fmt.Errorf("missing fence slot")
	}
	var slot *uint64
	if e = d.Decode(&slot); e != nil || slot == nil {
		return 0, fmt.Errorf("invalid fence slot")
	}
	end, e := d.Token()
	if e != nil || end != json.Delim('}') {
		return 0, fmt.Errorf("unexpected fence response field")
	}
	if _, e = d.Token(); e != io.EOF {
		return 0, fmt.Errorf("trailing fence response data")
	}
	return *slot, nil
}
func (s *Store) serveReadFence(w http.ResponseWriter, r *http.Request) {
	key := s.cfg.Database.ReadFenceKey
	if len(key) < 32 || subtle.ConstantTimeCompare([]byte(r.Header.Get("Authorization")), []byte("Bearer "+key)) != 1 {
		http.Error(w, "unauthorized", 401)
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", 405)
		return
	}
	if s.db == nil {
		http.Error(w, "voter required", 503)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	result, e := s.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
	if e != nil {
		http.Error(w, "quorum unavailable", 503)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(struct {
		AppliedSlot uint64 `json:"appliedSlot"`
	}{result.AppliedSlot})
}
