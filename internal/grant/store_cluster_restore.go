package grant

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/minio/minio-go/v7"
)

type clusterRestoreAdmission struct {
	TopologyDigest string   `json:"topologyDigest"`
	StorageDigest  string   `json:"storageDigest"`
	Version        int      `json:"version"`
	ClusterID      string   `json:"clusterId"`
	SnapshotDigest string   `json:"snapshotDigest"`
	Members        []string `json:"members"`
}

func restoreAdmission(cfg Config, snap StoreSnapshot) (clusterRestoreAdmission, error) {
	a := clusterRestoreAdmission{Version: 1, ClusterID: cfg.Database.ClusterID, SnapshotDigest: snap.Digest}
	if cfg.Database.Rhiza.ClusterID != "" {
		a.ClusterID = cfg.Database.Rhiza.ClusterID
	}
	members := cfg.Database.Members
	if len(cfg.Database.Rhiza.Members) > 0 {
		members = cfg.Database.Rhiza.Members
	}
	members = slices.Clone(members)
	sort.Slice(members, func(i, j int) bool { return members[i].ID < members[j].ID })
	for _, m := range members {
		if m.ID == "" || (len(a.Members) > 0 && a.Members[len(a.Members)-1] == string(m.ID)) {
			return a, fmt.Errorf("restore member IDs must be nonempty and unique")
		}
		a.Members = append(a.Members, string(m.ID))
	}
	topology, _ := canonicalJSON(members, "")
	a.TopologyDigest = digestBytes([]byte(topology))
	sort.Strings(a.Members)
	r := cfg.Database.Rhiza
	storage, _ := canonicalJSON([]string{r.ObjStoreProvider, r.ObjStoreEndpoint, r.ObjStoreBucket, strings.Trim(r.ObjStorePrefix, "/"), r.ObjStoreRegion}, "")
	a.StorageDigest = digestBytes([]byte(storage))
	if a.ClusterID == "" || a.ClusterID == snap.ClusterID || len(a.Members) < 3 || r.ObjStoreProvider != "s3" || r.ObjStoreBucket == "" || strings.Trim(r.ObjStorePrefix, "/") == "" || cfg.Database.RuntimeMode == "remote" || cfg.Database.RuntimeMode == "split" || cfg.Database.LocalNodeRole == "learner" {
		return a, fmt.Errorf("cluster restore requires a new cluster identity, at least three explicit voters, and a dedicated S3 prefix")
	}
	if snap.Backend == "rhiza" && snap.ClusterID == "" {
		return a, fmt.Errorf("Rhiza snapshot lacks source cluster identity")
	}
	return a, nil
}
func admissionKey(cfg Config) string {
	return strings.Trim(cfg.Database.Rhiza.ObjStorePrefix, "/") + "/grant-restore/admission.json"
}
func s3JSON(ctx context.Context, client *minio.Client, bucket, key string) ([]byte, error) {
	obj, e := client.GetObject(ctx, bucket, key, minio.GetObjectOptions{})
	if e != nil {
		return nil, e
	}
	defer obj.Close()
	return io.ReadAll(io.LimitReader(obj, 1<<20))
}
func missingS3(e error) bool {
	c := minio.ToErrorResponse(e).Code
	return c == "NoSuchKey" || c == "NoSuchObject" || c == "NotFound"
}

// PrepareClusterRestore is run once before any new voter starts. It never opens
// the database and refuses an occupied prefix. Conditional PUT claims admission.
func PrepareClusterRestore(ctx context.Context, cfg Config, snap StoreSnapshot) error {
	if e := ValidateStoreSnapshot(snap); e != nil {
		return e
	}
	a, e := restoreAdmission(cfg, snap)
	if e != nil {
		return e
	}
	client, e := (&Store{cfg: cfg}).backupS3()
	if e != nil {
		return e
	}
	bucket := cfg.Database.Rhiza.ObjStoreBucket
	key := admissionKey(cfg)
	want, _ := json.Marshal(a)
	existing, e := s3JSON(ctx, client, bucket, key)
	if e == nil {
		if bytes.Equal(existing, want) {
			return nil
		}
		return fmt.Errorf("restore admission belongs to another snapshot or topology")
	}
	if !missingS3(e) {
		return e
	}
	for item := range client.ListObjects(ctx, bucket, minio.ListObjectsOptions{Prefix: strings.Trim(cfg.Database.Rhiza.ObjStorePrefix, "/") + "/", Recursive: true}) {
		if item.Err != nil {
			return item.Err
		}
		return fmt.Errorf("restore target prefix is not empty")
	}
	opts := minio.PutObjectOptions{ContentType: "application/json"}
	opts.SetMatchETagExcept("*")
	_, e = client.PutObject(ctx, bucket, key, bytes.NewReader(want), int64(len(want)), opts)
	return e
}

// All peers acknowledge the identical admission before any peer opens Rhiza.
// Startup without opt-in refuses a pending admission instead of bootstrapping.
func clusterRestorePreflight(ctx context.Context, cfg Config) (*StoreSnapshot, error) {
	r := cfg.Database.Rhiza
	if r.ObjStoreProvider != "s3" || r.ObjStoreBucket == "" || r.ObjStorePrefix == "" {
		if cfg.Database.ClusterRestoreSnapshot != "" {
			return nil, fmt.Errorf("cluster restore requires prepared S3 admission")
		}
		return nil, nil
	}
	client, e := (&Store{cfg: cfg}).backupS3()
	if e != nil {
		return nil, e
	}
	raw, e := s3JSON(ctx, client, r.ObjStoreBucket, admissionKey(cfg))
	if missingS3(e) {
		if cfg.Database.ClusterRestoreSnapshot != "" {
			return nil, fmt.Errorf("cluster restore admission not prepared")
		}
		return nil, nil
	}
	if e != nil {
		return nil, e
	}
	var a clusterRestoreAdmission
	if e = json.Unmarshal(raw, &a); e != nil {
		return nil, e
	}
	configuredCluster := cfg.Database.ClusterID
	if cfg.Database.Rhiza.ClusterID != "" {
		configuredCluster = cfg.Database.Rhiza.ClusterID
	}
	if a.Version != 1 || configuredCluster != a.ClusterID {
		return nil, fmt.Errorf("cluster restore admission target identity mismatch")
	}
	if cfg.Database.ClusterRestoreSnapshot == "" {
		expected, err := restoreAdmission(cfg, StoreSnapshot{Digest: a.SnapshotDigest})
		if err != nil {
			return nil, err
		}
		expectedRaw, _ := json.Marshal(expected)
		if !bytes.Equal(expectedRaw, raw) {
			return nil, fmt.Errorf("activated restore topology differs from admission")
		}
		active, e := s3JSON(ctx, client, r.ObjStoreBucket, admissionKey(cfg)+".active")
		if e != nil || !bytes.Equal(active, raw) {
			return nil, fmt.Errorf("pending cluster restore requires snapshot opt-in on every voter")
		}
		return &StoreSnapshot{Digest: a.SnapshotDigest, ClusterID: a.ClusterID}, nil
	}
	snap, e := ReadStoreSnapshot(cfg.Database.ClusterRestoreSnapshot)
	if e != nil {
		return nil, e
	}
	expected, e := restoreAdmission(cfg, snap)
	if e != nil {
		return nil, e
	}
	want, _ := json.Marshal(expected)
	if !bytes.Equal(want, raw) {
		return nil, fmt.Errorf("cluster restore admission snapshot/topology mismatch")
	}
	if e = reserveRestoreDirectory(cfg, raw); e != nil {
		return nil, e
	}
	node := cfg.Database.NodeID
	if cfg.Database.Rhiza.NodeID != "" {
		node = cfg.Database.Rhiza.NodeID
	}
	found := false
	for _, id := range a.Members {
		if id == node {
			found = true
		}
	}
	if !found {
		return nil, fmt.Errorf("restore node is not an admitted voter")
	}
	key := admissionKey(cfg) + ".ack-" + strings.TrimPrefix(digestBytes([]byte(node)), "sha256:")
	opts := minio.PutObjectOptions{ContentType: "application/json"}
	if _, e = client.PutObject(ctx, r.ObjStoreBucket, key, bytes.NewReader(raw), int64(len(raw)), opts); e != nil {
		return nil, e
	}
	waitCtx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	for {
		all := true
		for _, id := range a.Members {
			b, e := s3JSON(waitCtx, client, r.ObjStoreBucket, admissionKey(cfg)+".ack-"+strings.TrimPrefix(digestBytes([]byte(id)), "sha256:"))
			if e != nil && !missingS3(e) {
				return nil, e
			}
			if !bytes.Equal(b, raw) {
				all = false
			}
		}
		if all {
			return &snap, nil
		}
		select {
		case <-waitCtx.Done():
			return nil, fmt.Errorf("wait for all restore voters: %w", waitCtx.Err())
		case <-time.After(100 * time.Millisecond):
		}
	}
}
func (s *Store) activateClusterRestore(ctx context.Context, snap StoreSnapshot) error {
	a, e := restoreAdmission(s.cfg, snap)
	if e != nil {
		return e
	}
	if e = s.restoreCanonicalAtomic(ctx, snap, a.ClusterID); e != nil {
		return e
	}
	client, e := s.backupS3()
	if e != nil {
		return e
	}
	b, _ := json.Marshal(a)
	active, activeErr := s3JSON(ctx, client, s.cfg.Database.Rhiza.ObjStoreBucket, admissionKey(s.cfg)+".active")
	if activeErr == nil {
		if bytes.Equal(active, b) {
			return nil
		}
		return fmt.Errorf("activated admission mismatch")
	}
	if !missingS3(activeErr) {
		return activeErr
	}
	restored, e := s.ExportSnapshot(ctx)
	if e != nil {
		return e
	}
	for i, t := range restored.Tables {
		if t.Digest != snap.Tables[i].Digest {
			return fmt.Errorf("cluster restore verification mismatch: %s", t.Name)
		}
	}
	node := s.cfg.Database.NodeID
	if s.cfg.Database.Rhiza.NodeID != "" {
		node = s.cfg.Database.Rhiza.NodeID
	}
	verifiedKey := func(node string) string {
		return admissionKey(s.cfg) + ".verified-" + strings.TrimPrefix(digestBytes([]byte(node)), "sha256:")
	}
	if _, e = client.PutObject(ctx, s.cfg.Database.Rhiza.ObjStoreBucket, verifiedKey(node), bytes.NewReader(b), int64(len(b)), minio.PutObjectOptions{ContentType: "application/json"}); e != nil {
		return e
	}
	barrierCtx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	for {
		all := true
		for _, id := range a.Members {
			v, readErr := s3JSON(barrierCtx, client, s.cfg.Database.Rhiza.ObjStoreBucket, verifiedKey(id))
			if readErr != nil && !missingS3(readErr) {
				return readErr
			}
			if !bytes.Equal(v, b) {
				all = false
			}
		}
		if all {
			break
		}
		select {
		case <-barrierCtx.Done():
			return fmt.Errorf("wait for restore verification barrier: %w", barrierCtx.Err())
		case <-time.After(100 * time.Millisecond):
		}
	}
	_, e = client.PutObject(ctx, s.cfg.Database.Rhiza.ObjStoreBucket, admissionKey(s.cfg)+".active", bytes.NewReader(b), int64(len(b)), minio.PutObjectOptions{ContentType: "application/json"})
	return e
}
func (s *Store) restoreCanonicalAtomic(ctx context.Context, snap StoreSnapshot, clusterID string) error {
	if e := ValidateStoreSnapshot(snap); e != nil {
		return e
	}
	existing, e := s.Query(ctx, ReadStrong, "SELECT snapshot_digest,cluster_id FROM grant_restore_activation WHERE id='active'")
	if e != nil {
		return e
	}
	if len(existing) > 0 {
		if rowString(existing[0], "snapshot_digest") == snap.Digest && rowString(existing[0], "cluster_id") == clusterID {
			return nil
		}
		return fmt.Errorf("cluster already restored from another snapshot")
	}
	id := strings.TrimPrefix(snap.Digest, "sha256:")
	byName := map[string]SnapshotTable{}
	for _, t := range snap.Tables {
		byName[t.Name] = t
		b, e := json.Marshal(t.Rows)
		if e != nil {
			return e
		}
		if e = s.stageText(ctx, id, t.Name, string(b)); e != nil {
			return e
		}
	}
	var empty []string
	for _, t := range snap.Tables {
		empty = append(empty, "NOT EXISTS(SELECT 1 FROM "+quoteIdentifier(t.Name)+")")
	}
	ss := []Statement{assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM grant_restore_activation) AND " + strings.Join(empty, " AND "))}
	// Physical foreign keys follow canonical DDL table order.
	for _, name := range schemaTableOrder() {
		t, ok := byName[name]
		if !ok {
			continue
		}
		var cols, values []string
		for i, c := range t.Columns {
			cols = append(cols, quoteIdentifier(c))
			values = append(values, fmt.Sprintf("json_extract(j.value,'$[%d]')", i))
		}
		ss = append(ss, Statement{SQL: "INSERT INTO " + quoteIdentifier(t.Name) + "(" + strings.Join(cols, ",") + ") SELECT " + strings.Join(values, ",") + " FROM json_each(" + stagedTextSQL() + ") j", Args: []any{id, t.Name}, ExpectedRowsAffected: &t.Count})
	}
	ss = append(ss, Statement{SQL: "INSERT INTO grant_restore_activation(id,snapshot_digest,cluster_id,activated_at) VALUES('active',?,?,?)", Args: []any{snap.Digest, clusterID, nowString()}}, Statement{SQL: "DELETE FROM grant_import_payloads WHERE import_id=?", Args: []any{id}})
	if e = s.Transaction(ctx, ss); e != nil {
		winner, readErr := s.Query(ctx, ReadStrong, "SELECT snapshot_digest,cluster_id FROM grant_restore_activation WHERE id='active'")
		if readErr == nil && len(winner) == 1 && rowString(winner[0], "snapshot_digest") == snap.Digest && rowString(winner[0], "cluster_id") == clusterID {
			return nil
		}
		return e
	}
	return nil
}
func schemaTableOrder() []string {
	var names []string
	for _, m := range regexp.MustCompile(`CREATE TABLE (?:IF NOT EXISTS )?([a-z_][a-z_0-9]*)`).FindAllStringSubmatch(schemaSQL, -1) {
		names = append(names, m[1])
	}
	return names
}
func reserveRestoreDirectory(cfg Config, raw []byte) error {
	dir := cfg.Database.DataDir
	marker := filepath.Join(dir, "grant-cluster-restore.json")
	if b, e := os.ReadFile(marker); e == nil {
		if bytes.Equal(b, raw) {
			return nil
		}
		return fmt.Errorf("local restore admission differs")
	}
	entries, e := os.ReadDir(dir)
	if e != nil && !os.IsNotExist(e) {
		return e
	}
	if len(entries) > 0 {
		return fmt.Errorf("cluster restore requires fresh data directory or matching interrupted admission")
	}
	if e = os.MkdirAll(dir, 0700); e != nil {
		return e
	}
	return writeDurableExclusive(marker, raw)
}
