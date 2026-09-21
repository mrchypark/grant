package grant

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"github.com/minio/minio-go/v7"
	"github.com/minio/minio-go/v7/pkg/credentials"
	_ "github.com/ncruces/go-sqlite3/driver"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
)

type SnapshotTable struct {
	Name    string   `json:"name"`
	Columns []string `json:"columns"`
	Rows    [][]any  `json:"rows"`
	Count   int64    `json:"count"`
	Digest  string   `json:"digest"`
}
type StoreSnapshot struct {
	ClusterID    string          `json:"clusterId,omitempty"`
	Version      int             `json:"version"`
	Backend      string          `json:"backend"`
	CreatedAt    string          `json:"createdAt"`
	SchemaDigest string          `json:"schemaDigest"`
	Tables       []SnapshotTable `json:"tables"`
	Digest       string          `json:"digest"`
}

func applicationTables() []string {
	re := regexp.MustCompile(`CREATE TABLE (?:IF NOT EXISTS )?([a-z_][a-z_0-9]*)`)
	var names []string
	for _, m := range re.FindAllStringSubmatch(schemaSQL, -1) {
		if m[1] != "grant_import_payloads" {
			names = append(names, m[1])
		}
	}
	sort.Strings(names)
	return names
}
func snapshotDigest(s StoreSnapshot) (string, error) {
	s.Digest = ""
	v, e := canonicalJSON(s, "")
	return digestBytes([]byte(v)), e
}
func tableDigest(t SnapshotTable) (string, error) {
	t.Digest = ""
	t.Rows = append([][]any(nil), t.Rows...)
	sort.Slice(t.Rows, func(i, j int) bool {
		a, _ := canonicalJSON(t.Rows[i], "")
		b, _ := canonicalJSON(t.Rows[j], "")
		return a < b
	})
	v, e := canonicalJSON(t, "")
	return digestBytes([]byte(v)), e
}
func (s *Store) ExportSnapshot(ctx context.Context) (StoreSnapshot, error) {
	out := StoreSnapshot{Version: 1, Backend: "rhiza", CreatedAt: nowString(), SchemaDigest: digestBytes([]byte(schemaSQL + indexesSQL)), Tables: []SnapshotTable{}}
	out.ClusterID = s.cfg.Database.ClusterID
	if s.cfg.Database.Rhiza.ClusterID != "" {
		out.ClusterID = s.cfg.Database.Rhiza.ClusterID
	}
	var queries []string
	for _, name := range applicationTables() {
		r, e := s.QueryRaw(ctx, ReadStrong, "SELECT * FROM "+quoteIdentifier(name)+" LIMIT 0")
		if e != nil {
			return out, e
		}
		cols := r.Columns
		parts := make([]string, len(cols))
		for i, c := range cols {
			parts[i] = quoteIdentifier(c)
		}
		out.Tables = append(out.Tables, SnapshotTable{Name: name, Columns: cols, Rows: [][]any{}})
		qcols := strings.Join(parts, ",")
		queries = append(queries, "SELECT '"+name+"' AS table_name, 0 AS kind, json_array(COUNT(*)) AS payload FROM "+quoteIdentifier(name), "SELECT '"+name+"',1,json_group_array(json(row_json)) FROM (SELECT json_array("+qcols+") AS row_json FROM "+quoteIdentifier(name)+" ORDER BY "+qcols+")")
	}
	// All table rows and authoritative counts come from one SQLite read snapshot.
	rows, e := s.Query(ctx, ReadStrong, "WITH RECURSIVE chunks(table_name,kind,payload,remaining,ordinal) AS (SELECT table_name,kind,substr(payload,1,60000),substr(payload,60001),0 FROM ("+strings.Join(queries, " UNION ALL ")+") UNION ALL SELECT table_name,kind,substr(remaining,1,60000),substr(remaining,60001),ordinal+1 FROM chunks WHERE remaining<>'' ) SELECT table_name,kind,payload FROM chunks ORDER BY table_name,kind,ordinal")
	if e != nil {
		return out, e
	}
	byName := map[string]*SnapshotTable{}
	for i := range out.Tables {
		byName[out.Tables[i].Name] = &out.Tables[i]
	}
	var merged []map[string]any
	for _, r := range rows {
		if len(merged) > 0 && rowString(merged[len(merged)-1], "table_name") == rowString(r, "table_name") && rowInt(merged[len(merged)-1], "kind") == rowInt(r, "kind") {
			m := merged[len(merged)-1]
			m["payload"] = rowString(m, "payload") + rowString(r, "payload")
		} else {
			merged = append(merged, r)
		}
	}
	for _, r := range merged {
		t := byName[rowString(r, "table_name")]
		var values []any
		d := json.NewDecoder(strings.NewReader(rowString(r, "payload")))
		d.UseNumber()
		if e = d.Decode(&values); e != nil {
			return out, e
		}
		values, e = normalizeWireArgs(values)
		if e != nil {
			return out, e
		}
		if rowInt(r, "kind") == 0 {
			t.Count = values[0].(int64)
		} else {
			for _, v := range values {
				row, ok := v.([]any)
				if !ok {
					return out, fmt.Errorf("invalid snapshot row group")
				}
				row, e = normalizeWireArgs(row)
				if e != nil {
					return out, e
				}
				t.Rows = append(t.Rows, row)
			}
		}
	}
	for i := range out.Tables {
		t := &out.Tables[i]
		if int64(len(t.Rows)) != t.Count {
			return out, fmt.Errorf("snapshot count mismatch for %s", t.Name)
		}
		t.Digest, e = tableDigest(*t)
		if e != nil {
			return out, e
		}
	}
	out.Digest, e = snapshotDigest(out)
	return out, e
}
func WriteStoreSnapshot(path string, snap StoreSnapshot) error {
	if err := ValidateStoreSnapshot(snap); err != nil {
		return err
	}
	b, e := json.Marshal(snap)
	if e != nil {
		return e
	}
	f, e := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if e != nil {
		return e
	}
	_, e = f.Write(b)
	if e == nil {
		e = f.Sync()
	}
	ce := f.Close()
	if e != nil {
		return e
	}
	return ce
}
func ReadStoreSnapshot(path string) (StoreSnapshot, error) {
	var s StoreSnapshot
	b, e := os.ReadFile(path)
	if e != nil {
		return s, e
	}
	d := json.NewDecoder(bytes.NewReader(b))
	d.DisallowUnknownFields()
	d.UseNumber()
	if e = d.Decode(&s); e != nil {
		return s, e
	}
	for i := range s.Tables {
		for j := range s.Tables[i].Rows {
			s.Tables[i].Rows[j], e = normalizeWireArgs(s.Tables[i].Rows[j])
			if e != nil {
				return s, e
			}
		}
	}
	return s, ValidateStoreSnapshot(s)
}
func ValidateStoreSnapshot(s StoreSnapshot) error {
	if s.Version != 1 || s.SchemaDigest != digestBytes([]byte(schemaSQL+indexesSQL)) {
		return fmt.Errorf("snapshot schema mismatch")
	}
	expected := applicationTables()
	if len(s.Tables) != len(expected) {
		return fmt.Errorf("snapshot table inventory mismatch")
	}
	for i, t := range s.Tables {
		if t.Name != expected[i] || int64(len(t.Rows)) != t.Count {
			return fmt.Errorf("snapshot count/inventory mismatch")
		}
		for _, r := range t.Rows {
			if len(r) != len(t.Columns) {
				return fmt.Errorf("snapshot row shape mismatch")
			}
		}
		d, e := tableDigest(t)
		if e != nil || d != t.Digest {
			return fmt.Errorf("snapshot table digest mismatch")
		}
	}
	d, e := snapshotDigest(s)
	if e != nil || d != s.Digest {
		return fmt.Errorf("snapshot digest mismatch")
	}
	return nil
}
func RestoreStoreSnapshot(ctx context.Context, cfg Config, snap StoreSnapshot) error {
	if e := ValidateStoreSnapshot(snap); e != nil {
		return e
	}
	if len(cfg.Database.Members) > 0 || len(cfg.Database.Rhiza.Members) > 0 || len(cfg.Database.RemoteNodes) > 0 || cfg.Database.RuntimeMode == "remote" || cfg.Database.RuntimeMode == "split" || cfg.Database.LocalNodeRole == "learner" || cfg.Database.Rhiza.ObjStoreProvider != "" || cfg.Database.Rhiza.ObjStoreEndpoint != "" || cfg.Database.Rhiza.ObjStoreDir != "" || cfg.Database.Rhiza.ObjStoreBucket != "" {
		return fmt.Errorf("logical restore requires isolated single-voter local configuration without remote members or shared object storage")
	}
	if _, e := os.Stat(cfg.Database.DataDir); !os.IsNotExist(e) {
		return fmt.Errorf("restore requires a new, absent data directory")
	}
	if e := os.Mkdir(cfg.Database.DataDir, 0700); e != nil {
		return e
	}
	pending := filepath.Join(cfg.Database.DataDir, "grant-restore-pending.json")
	if e := writeDurableExclusive(pending, []byte(snap.Digest)); e != nil {
		return e
	}
	s, e := openStore(ctx, cfg, true)
	if e != nil {
		return e
	}
	defer s.Close()
	if e = s.restoreCanonicalAtomic(ctx, snap, "offline"); e != nil {
		return e
	}
	restored, e := s.ExportSnapshot(ctx)
	if e != nil {
		return e
	}
	for i, t := range restored.Tables {
		if t.Digest != snap.Tables[i].Digest {
			return fmt.Errorf("restore digest verification failed: %s", t.Name)
		}
	}
	if e = s.VerifySchema(ctx); e != nil {
		return e
	}
	marker := map[string]string{"sourceDigest": snap.Digest, "verifiedAt": nowString(), "backend": "rhiza"}
	b, _ := json.Marshal(marker)
	if e = writeDurableExclusive(filepath.Join(cfg.Database.DataDir, "grant-cutover-verified.json"), b); e != nil {
		return e
	}
	if e = os.Remove(pending); e != nil {
		return e
	}
	return syncDirectory(cfg.Database.DataDir)
}

func syncDirectory(path string) error {
	f, e := os.Open(path)
	if e != nil {
		return e
	}
	defer f.Close()
	return f.Sync()
}
func writeDurableExclusive(path string, b []byte) error {
	f, e := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if e != nil {
		return e
	}
	if _, e = f.Write(b); e == nil {
		e = f.Sync()
	}
	ce := f.Close()
	if e != nil {
		return e
	}
	if ce != nil {
		return ce
	}
	return syncDirectory(filepath.Dir(path))
}
func ExportLegacySQLite(ctx context.Context, path string) (StoreSnapshot, error) {
	out := StoreSnapshot{Version: 1, Backend: "hiqlite-sqlite", CreatedAt: nowString(), SchemaDigest: digestBytes([]byte(schemaSQL + indexesSQL)), Tables: []SnapshotTable{}}
	abs, e := filepath.Abs(path)
	if e != nil {
		return out, e
	}
	if info, e := os.Stat(abs); e != nil || !info.Mode().IsRegular() {
		return out, fmt.Errorf("legacy SQLite path must be an existing regular file")
	}
	u := url.URL{Scheme: "file", Path: abs}
	q := u.Query()
	q.Set("mode", "ro")
	u.RawQuery = q.Encode()
	db, e := sql.Open("sqlite3", u.String())
	if e != nil {
		return out, e
	}
	defer db.Close()
	tx, e := db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if e != nil {
		return out, e
	}
	defer tx.Rollback()
	for _, table := range []string{"role_action_field_views", "field_view_policy_audit_outbox", "field_view_policy_audit_deliveries", "mask_catalog_manifests", "mask_catalog_active"} {
		var exists int
		if e = tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name=?", table).Scan(&exists); e != nil {
			return out, e
		}
		if exists > 0 {
			var count int
			if e = tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM "+quoteIdentifier(table)).Scan(&count); e != nil {
				return out, e
			}
			if count > 0 {
				return out, fmt.Errorf("legacy active %s prevents coarse migration", table)
			}
		}
	}
	var fieldActions int
	if e = tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM actions WHERE visibility_granting<>0").Scan(&fieldActions); e != nil {
		return out, e
	}
	if fieldActions > 0 {
		return out, fmt.Errorf("legacy active field actions prevent coarse migration")
	}
	for _, name := range applicationTables() {
		rows, e := tx.QueryContext(ctx, "SELECT * FROM "+quoteIdentifier(name))
		if e != nil {
			return out, fmt.Errorf("legacy schema is not the frozen Grant schema: %w", e)
		}
		cols, e := rows.Columns()
		if e != nil {
			rows.Close()
			return out, e
		}
		t := SnapshotTable{Name: name, Columns: cols, Rows: [][]any{}}
		for rows.Next() {
			values := make([]any, len(cols))
			targets := make([]any, len(cols))
			for i := range values {
				targets[i] = &values[i]
			}
			if e = rows.Scan(targets...); e != nil {
				rows.Close()
				return out, e
			}
			t.Rows = append(t.Rows, values)
		}
		if e = rows.Err(); e != nil {
			rows.Close()
			return out, e
		}
		rows.Close()
		sort.Slice(t.Rows, func(i, j int) bool {
			a, _ := canonicalJSON(t.Rows[i], "")
			b, _ := canonicalJSON(t.Rows[j], "")
			return a < b
		})
		t.Count = int64(len(t.Rows))
		t.Digest, e = tableDigest(t)
		if e != nil {
			return out, e
		}
		out.Tables = append(out.Tables, t)
	}
	out.Digest, e = snapshotDigest(out)
	return out, e
}
func (s *Store) TriggerBackup(ctx context.Context) error {
	snap, e := s.ExportSnapshot(ctx)
	if e != nil {
		return e
	}
	dir := s.cfg.Database.BackupDir
	if dir == "" {
		dir = filepath.Join(s.cfg.Database.DataDir, "backups")
	}
	if e = os.MkdirAll(dir, 0700); e != nil {
		return e
	}
	name := "grant-" + time.Now().UTC().Format("20060102T150405.000000000Z") + "-" + requestID() + ".json"
	path := filepath.Join(dir, name)
	if e = WriteStoreSnapshot(path, snap); e != nil {
		return e
	}
	r := s.cfg.Database.Rhiza
	if r.ObjStoreProvider == "s3" {
		client, e := s.backupS3()
		if e != nil {
			return e
		}
		_, e = client.FPutObject(ctx, r.ObjStoreBucket, strings.Trim(r.ObjStorePrefix, "/")+"/grant-backups/"+name, path, minio.PutObjectOptions{ContentType: "application/json"})
		return e
	}
	return nil
}
func (s *Store) ListBackupsLocal(ctx context.Context) ([]BackupInfo, error) {
	dir := s.cfg.Database.BackupDir
	if dir == "" {
		dir = filepath.Join(s.cfg.Database.DataDir, "backups")
	}
	entries, e := os.ReadDir(dir)
	if os.IsNotExist(e) {
		return []BackupInfo{}, nil
	}
	if e != nil {
		return nil, e
	}
	out := []BackupInfo{}
	for _, v := range entries {
		if !v.IsDir() && strings.HasPrefix(v.Name(), "grant-") && strings.HasSuffix(v.Name(), ".json") {
			info, e := v.Info()
			if e != nil {
				return nil, e
			}
			n := uint64(info.Size())
			out = append(out, BackupInfo{Name: v.Name(), LastModified: info.ModTime().Unix(), Size: &n, Storage: "local"})
		}
	}
	return out, nil
}
func (s *Store) backupS3() (*minio.Client, error) {
	r := s.cfg.Database.Rhiza
	endpoint := r.ObjStoreEndpoint
	if endpoint == "" {
		endpoint = "s3.amazonaws.com"
	}
	secure := !r.ObjStoreInsecure
	if u, e := url.Parse(endpoint); e == nil && u.Host != "" {
		endpoint = u.Host
		secure = u.Scheme == "https"
	}
	return minio.New(endpoint, &minio.Options{Creds: credentials.NewStaticV4(r.ObjStoreAccessKey, r.ObjStoreSecretKey, r.ObjStoreSessionToken), Secure: secure, Region: r.ObjStoreRegion})
}
func (s *Store) ListBackupsS3(ctx context.Context) ([]BackupInfo, error) {
	r := s.cfg.Database.Rhiza
	if r.ObjStoreProvider != "s3" {
		return []BackupInfo{}, nil
	}
	client, e := s.backupS3()
	if e != nil {
		return nil, e
	}
	out := []BackupInfo{}
	for v := range client.ListObjects(ctx, r.ObjStoreBucket, minio.ListObjectsOptions{Prefix: strings.Trim(r.ObjStorePrefix, "/") + "/grant-backups/", Recursive: true}) {
		if v.Err != nil {
			return nil, v.Err
		}
		n := uint64(v.Size)
		out = append(out, BackupInfo{Name: v.Key, LastModified: v.LastModified.Unix(), Size: &n, Storage: "s3"})
	}
	return out, nil
}
func (s *Store) ListBackups(ctx context.Context) ([]BackupInfo, error) {
	local, e := s.ListBackupsLocal(ctx)
	if e != nil {
		return nil, e
	}
	r := s.cfg.Database.Rhiza
	if r.ObjStoreProvider == "" || r.ObjStoreProvider == "s3" {
		remote, e := s.ListBackupsS3(ctx)
		if e != nil {
			return nil, e
		}
		seen := map[string]bool{}
		for _, b := range local {
			seen[b.Name] = true
		}
		for _, b := range remote {
			if !seen[b.Name] {
				local = append(local, b)
			}
		}
	}
	return local, nil
}

var _ = io.EOF
