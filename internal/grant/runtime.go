package grant

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"github.com/mrchypark/rhiza"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type Statement = rhiza.SQLStatement
type Store struct {
	db      *rhiza.DB
	replica *rhiza.ReadReplica
	cfg     Config
}

func requestID() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	return hex.EncodeToString(b[:])
}
func nowString() string { return time.Now().UTC().Format(time.RFC3339Nano) }
func OpenStore(ctx context.Context, cfg Config) (*Store, error) {
	return openStore(ctx, cfg, false, nil)
}

// OpenStoreWithOperator starts the private operator listener after Rhiza has
// opened its peer endpoint, before Grant waits for quorum or bootstraps data.
func OpenStoreWithOperator(ctx context.Context, cfg Config, start func(http.Handler) error) (*Store, error) {
	return openStore(ctx, cfg, false, start)
}

func effectiveRhizaConfig(d DatabaseConfig) (rhiza.Config, error) {
	c := d.Rhiza
	if c.NodeID == "" {
		c.NodeID = d.NodeID
	}
	if c.NodeID == "" {
		c.NodeID = "grant-1"
	}
	if c.ClusterID == "" {
		c.ClusterID = d.ClusterID
	}
	if c.PeerAddr == "" {
		c.PeerAddr = d.PeerAddr
	}
	if c.Members == nil {
		c.Members = d.Members
	} else if len(c.Members) == 0 && len(d.Members) > 1 {
		return c, fmt.Errorf("empty Rhiza membership cannot override a multi-voter cluster")
	}
	if c.Local && len(c.Members) > 1 {
		return c, fmt.Errorf("local Rhiza mode cannot serve a multi-voter cluster")
	}
	return c, nil
}

func openStore(ctx context.Context, cfg Config, restoring bool, startOperator func(http.Handler) error) (*Store, error) {
	if cfg.Database.DataDir == "" {
		return nil, fmt.Errorf("Grant data directory is required")
	}
	dir, err := filepath.Abs(cfg.Database.DataDir)
	if err != nil {
		return nil, err
	}
	if cfg.Database.LegacyDataDir != "" {
		old, e := filepath.Abs(cfg.Database.LegacyDataDir)
		if e != nil {
			return nil, e
		}
		if dir == old {
			return nil, fmt.Errorf("legacy Hiqlite directory requires explicit export and separate Rhiza restore")
		}
	}
	for _, name := range []string{"state_machine", "raft", "raft_log"} {
		if _, e := os.Stat(filepath.Join(dir, name)); e == nil {
			return nil, fmt.Errorf("legacy Hiqlite state detected; refusing to create Rhiza files")
		}
	}
	if _, e := os.Stat(filepath.Join(dir, "grant-restore-pending.json")); e == nil && !restoring {
		return nil, fmt.Errorf("incomplete Grant restore: isolated directory cannot serve until verified")
	}
	restoreSnapshot, err := clusterRestorePreflight(ctx, cfg)
	if err != nil {
		return nil, err
	}
	c, err := effectiveRhizaConfig(cfg.Database)
	if err != nil {
		return nil, err
	}
	c.DataDir = dir
	s := &Store{cfg: cfg}
	if cfg.Database.RuntimeMode == "remote" {
		return s, nil
	}
	if cfg.Database.LocalNodeRole == "learner" || cfg.Database.RuntimeMode == "split" {
		r := cfg.Database.Replica
		if r.ObjStoreProvider == "" {
			r.ObjStoreProvider = c.ObjStoreProvider
			r.ObjStoreEndpoint = c.ObjStoreEndpoint
			r.ObjStoreBucket = c.ObjStoreBucket
			r.ObjStoreDir = c.ObjStoreDir
			r.ObjStorePrefix = c.ObjStorePrefix
			r.ObjStoreRegion = c.ObjStoreRegion
			r.ObjStoreInsecure = c.ObjStoreInsecure
			r.ObjStoreRetries = c.ObjStoreRetries
			r.ObjStoreAccessKey = c.ObjStoreAccessKey
			r.ObjStoreSecretKey = c.ObjStoreSecretKey
			r.ObjStoreSessionToken = c.ObjStoreSessionToken
			r.ObjStoreServiceAccount = c.ObjStoreServiceAccount
			r.ObjStoreAzureTenantID = c.ObjStoreAzureTenantID
			r.ObjStoreAzureClientID = c.ObjStoreAzureClientID
			r.ObjStoreAzureClientSecret = c.ObjStoreAzureClientSecret
			r.ObjStoreAzureStorageAccount = c.ObjStoreAzureStorageAccount
			r.ObjStoreAzureStorageAccountKey = c.ObjStoreAzureStorageAccountKey
			r.ObjStoreAzureConnectionString = c.ObjStoreAzureConnectionString
			r.ObjStoreAzureUserAssignedID = c.ObjStoreAzureUserAssignedID
		}
		r.DataDir = dir
		if r.ReplicaID == "" {
			r.ReplicaID = c.NodeID
		}
		if r.ClusterID == "" {
			r.ClusterID = c.ClusterID
		}
		if len(r.Members) == 0 {
			for _, m := range c.Members {
				member, e := rhiza.NewReplicaMember(c.ClusterID, m)
				if e != nil {
					return nil, e
				}
				r.Members = append(r.Members, member)
			}
		}
		s.replica, err = rhiza.OpenLearner(ctx, r)
		return s, err
	}
	s.db, err = rhiza.Open(ctx, c)
	if err != nil {
		return nil, err
	}
	if startOperator != nil {
		handler, handlerErr := s.OperatorHandler()
		if handlerErr != nil {
			s.Close()
			return nil, handlerErr
		}
		if err = startOperator(handler); err != nil {
			s.Close()
			return nil, fmt.Errorf("start Rhiza operator listener: %w", err)
		}
	}
	if c.Learner != nil {
		return s, nil
	}
	readyCtx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	if err = s.waitForReady(readyCtx, false); err != nil {
		s.Close()
		return nil, err
	}
	if restoreSnapshot != nil {
		if restoreSnapshot.Version == 0 {
			rows, e := s.Query(ctx, ReadStrong, "SELECT 1 AS valid FROM grant_restore_activation WHERE id='active' AND snapshot_digest=? AND cluster_id=?", restoreSnapshot.Digest, restoreSnapshot.ClusterID)
			if e != nil || len(rows) != 1 {
				s.Close()
				return nil, fmt.Errorf("activated restore receipt missing or unreadable: %v", e)
			}
			return s, nil
		}
		if err = s.activateClusterRestore(ctx, *restoreSnapshot); err != nil {
			s.Close()
			return nil, fmt.Errorf("activate cluster restore: %w", err)
		}
	}
	return s, nil
}

// WaitForPromotion waits for a native learner to become a voter. It has no
// deadline: membership admission is controlled by an external fence workflow.
func (s *Store) WaitForPromotion(ctx context.Context) error {
	return s.waitForReady(ctx, true)
}

func (s *Store) waitForReady(ctx context.Context, requireVoter bool) error {
	if s.db == nil {
		return fmt.Errorf("local Rhiza voter required")
	}
	for {
		_, e := s.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
		if e == nil && !requireVoter {
			break
		}
		if e == nil {
			status, statusErr := s.db.MembershipStatus()
			if statusErr == nil && status.Voting {
				break
			}
			if statusErr != nil && !errors.Is(statusErr, rhiza.ErrNotReady) {
				return fmt.Errorf("read Rhiza membership status: %w", statusErr)
			}
		} else if !errors.Is(e, rhiza.ErrNotReady) && !errors.Is(e, rhiza.ErrQuorumUnavailable) {
			return fmt.Errorf("startup strong read: %w", e)
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("wait for Rhiza quorum readiness: %w", ctx.Err())
		case <-time.After(250 * time.Millisecond):
		}
	}
	if err := s.Migrate(ctx); err != nil {
		return err
	}
	return nil
}
func (s *Store) Close() error {
	if s.replica != nil {
		return s.replica.Close()
	}
	if s.db != nil {
		return s.db.Close()
	}
	return nil
}
func (s *Store) Query(ctx context.Context, consistency, sql string, args ...any) ([]map[string]any, error) {
	response, err := s.QueryRaw(ctx, consistency, sql, args...)
	if err != nil {
		return nil, err
	}
	rows := make([]map[string]any, 0, len(response.Rows))
	for _, values := range response.Rows {
		row := make(map[string]any, len(values))
		for i, v := range values {
			row[response.Columns[i]] = v
		}
		rows = append(rows, row)
	}
	return rows, nil
}
func (s *Store) QueryRaw(ctx context.Context, consistency, sql string, args ...any) (rhiza.QueryResponse, error) {
	if scope := readScope(ctx); scope != nil {
		var result rhiza.QueryResponse
		var e error
		req := rhiza.QueryRequest{SQL: sql, Args: sqlArgs(args), Consistency: rhiza.ConsistencyLocal}
		if s.replica != nil {
			result, e = s.replica.Query(ctx, req)
		} else if s.db != nil {
			result, e = s.db.Query(ctx, req)
		} else {
			return result, fmt.Errorf("local authority store required")
		}
		if e != nil {
			return result, e
		}
		if result.AppliedSlot < scope.fence || (scope.pinned && scope.slot != result.AppliedSlot) {
			scope.changed = true
			return rhiza.QueryResponse{}, serviceError(503, "unavailable: replica snapshot changed")
		}
		scope.slot = result.AppliedSlot
		scope.pinned = true
		return result, nil
	}
	if consistency == "" {
		consistency = s.cfg.Database.DefaultReadConsistency
	}
	if consistency == "" {
		consistency = ReadLocal
	}
	mode := rhiza.ConsistencyLocal
	if consistency == ReadStrong || consistency == ReadPreferStrong {
		mode = rhiza.ConsistencyLinearizable
	} else if consistency != ReadLocal {
		return rhiza.QueryResponse{}, fmt.Errorf("invalid read consistency")
	}
	req := rhiza.QueryRequest{SQL: sql, Args: sqlArgs(args), Consistency: mode}
	if s.db == nil && (s.replica == nil || mode == rhiza.ConsistencyLinearizable) {
		var remote rhiza.QueryResponse
		err := s.remoteCall(ctx, "/sql/query", req, &remote)
		if err != nil && consistency == ReadPreferStrong && s.replica != nil {
			req.Consistency = rhiza.ConsistencyLocal
			return s.replica.Query(ctx, req)
		}
		return remote, err
	}
	var result rhiza.QueryResponse
	var err error
	if s.replica != nil {
		result, err = s.replica.Query(ctx, req)
	} else {
		result, err = s.db.Query(ctx, req)
	}
	if err != nil && consistency == ReadPreferStrong {
		req.Consistency = rhiza.ConsistencyLocal
		if s.replica != nil {
			return s.replica.Query(ctx, req)
		}
		return s.db.Query(ctx, req)
	}
	return result, err
}
func sqlArgs(args []any) []any {
	out := make([]any, len(args))
	for i, v := range args {
		switch x := v.(type) {
		case bool:
			if x {
				out[i] = int64(1)
			} else {
				out[i] = int64(0)
			}
		case int:
			out[i] = int64(x)
		case uint64:
			out[i] = int64(x)
		case *string:
			if x != nil {
				out[i] = *x
			}
		case *uint64:
			if x != nil {
				out[i] = int64(*x)
			}
		case *int64:
			if x != nil {
				out[i] = *x
			}
		default:
			out[i] = v
		}
	}
	return out
}
func (s *Store) Execute(ctx context.Context, id string, stmts ...Statement) (rhiza.ExecuteResponse, error) {
	if readScope(ctx) != nil {
		return rhiza.ExecuteResponse{}, serviceError(503, "unavailable: read-only decision cannot mutate authority")
	}
	if s.cfg.Database.LocalNodeRole == "learner" {
		return rhiza.ExecuteResponse{}, fmt.Errorf("mutation requires voter")
	}
	copied := append([]Statement(nil), stmts...)
	for i := range copied {
		copied[i].Args = sqlArgs(copied[i].Args)
	}
	for _, statement := range copied {
		if strings.Contains(statement.SQL, "INSERT INTO application_consent_assertions") {
			copied = append(copied, Statement{SQL: "DELETE FROM application_consent_assertions"})
			break
		}
	}
	if s.db == nil {
		var r rhiza.ExecuteResponse
		err := s.remoteCall(ctx, "/sql/execute", rhiza.ExecuteRequest{RequestID: id, Statements: copied}, &r)
		if err == nil && r.Status != rhiza.MutationCommitted {
			err = fmt.Errorf("remote transaction rejected: %+v", r.MutationReceipt)
		}
		return r, err
	}
	var r rhiza.ExecuteResponse
	request := rhiza.ExecuteRequest{RequestID: id, Statements: copied}
	err := retryNativeOutcome(ctx, func(attempt context.Context) error { var e error; r, e = s.db.Execute(attempt, request); return e })
	if err != nil {
		return r, err
	}
	if r.Status != rhiza.MutationCommitted {
		return r, fmt.Errorf("transaction rejected: %+v", r.MutationReceipt)
	}
	return r, nil
}
func (s *Store) Exec(ctx context.Context, sql string, args ...any) (int64, error) {
	r, e := s.Execute(ctx, requestID(), Statement{SQL: sql, Args: args})
	if e != nil {
		return 0, e
	}
	// Native Rhiza omits detailed statement receipts when no statement requests rows.
	// The aggregate receipt always owns the affected-row count.
	return r.RowsAffected, nil
}
func (s *Store) Transaction(ctx context.Context, stmts []Statement) error {
	_, e := s.Execute(ctx, requestID(), stmts...)
	return e
}
func (s *Store) Health(ctx context.Context) HealthStatus {
	// Each check has its own deadline so a healthy local fallback is observable
	// even when the quorum is unavailable. Kubernetes readiness allows 3 seconds.
	localCtx, cancelLocal := context.WithTimeout(ctx, time.Second)
	re := s.verifySchemaAt(localCtx, ReadLocal)
	cancelLocal()
	strongCtx, cancelStrong := context.WithTimeout(ctx, time.Second)
	we := s.verifySchemaAt(strongCtx, ReadStrong)
	cancelStrong()
	status := "ok"
	if re != nil && we != nil {
		status = "down"
	} else if re != nil || we != nil {
		status = "degraded"
	}
	mode := s.cfg.Database.RuntimeMode
	if mode == "" {
		mode = "embedded"
	}
	cons := s.cfg.Database.DefaultReadConsistency
	if cons == "" {
		cons = ReadLocal
	}
	return HealthStatus{Status: status, ReadReady: re == nil, WriteReady: we == nil, RuntimeMode: mode, DefaultReadConsistency: cons}
}
func rowString(r map[string]any, k string) string {
	if v, ok := r[k].(string); ok {
		return v
	}
	return ""
}
func rowInt(r map[string]any, k string) int64 {
	switch v := r[k].(type) {
	case int64:
		return v
	case float64:
		return int64(v)
	}
	return 0
}
func rowNullable(r map[string]any, k string) *string {
	if r[k] == nil {
		return nil
	}
	v := rowString(r, k)
	return &v
}
func quoteIdentifier(v string) string { return `"` + strings.ReplaceAll(v, `"`, `""`) + `"` }
