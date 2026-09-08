package main

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/Conalog/grant/internal/grant"
	_ "github.com/ncruces/go-sqlite3/driver"
	"hash"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

type args struct {
	database, revision, ledger, produced, output string
	generation                                   uint64
}

func main() {
	if e := run(os.Args[1:]); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}
func parseArgs(values []string) (args, error) {
	a := args{}
	m := map[string]string{}
	for i := 0; i < len(values); i += 2 {
		name := values[i]
		switch name {
		case "--database", "--source-revision", "--source-ledger-digest", "--generation", "--produced-at", "--output-dir":
		default:
			return a, fmt.Errorf("unknown argument %s", name)
		}
		if _, ok := m[name]; ok {
			return a, fmt.Errorf("duplicate argument %s", name)
		}
		if i+1 == len(values) {
			return a, fmt.Errorf("missing value for %s", name)
		}
		m[name] = values[i+1]
	}
	for _, k := range []string{"--database", "--source-revision", "--source-ledger-digest", "--generation", "--produced-at", "--output-dir"} {
		if _, ok := m[k]; !ok {
			return a, fmt.Errorf("missing %s", k)
		}
	}
	n, e := strconv.ParseUint(m["--generation"], 10, 64)
	if e != nil {
		return a, fmt.Errorf("invalid --generation")
	}
	a = args{m["--database"], m["--source-revision"], m["--source-ledger-digest"], m["--produced-at"], m["--output-dir"], n}
	if n == 0 || a.revision == "" || strings.TrimSpace(a.revision) != a.revision {
		return a, fmt.Errorf("invalid generation or source revision")
	}
	when, e := time.Parse(time.RFC3339, a.produced)
	if e != nil || when.UTC().Format("2006-01-02T15:04:05Z") != a.produced {
		return a, fmt.Errorf("--produced-at must be canonical UTC seconds")
	}
	if len(a.ledger) != 71 || !strings.HasPrefix(a.ledger, "sha256:") || strings.Trim(a.ledger[7:], "0123456789abcdef") != "" {
		return a, fmt.Errorf("invalid --source-ledger-digest")
	}
	return a, nil
}
func run(values []string) error {
	a, e := parseArgs(values)
	if e != nil {
		return e
	}
	if _, e = os.Lstat(a.output); e == nil {
		return fmt.Errorf("output directory already exists")
	} else if !os.IsNotExist(e) {
		return e
	}
	path, e := filepath.EvalSymlinks(a.database)
	if e != nil {
		return e
	}
	path, e = filepath.Abs(path)
	if e != nil {
		return e
	}
	for _, suffix := range []string{"-wal", "-shm"} {
		if _, e = os.Stat(path + suffix); e == nil {
			return fmt.Errorf("database is not a frozen checkpoint: %s exists", path+suffix)
		} else if !os.IsNotExist(e) {
			return e
		}
	}
	uri := url.URL{Scheme: "file", Path: path, RawQuery: "mode=ro&immutable=1"}
	db, e := sql.Open("sqlite3", uri.String())
	if e != nil {
		return e
	}
	defer db.Close()
	tx, e := db.BeginTx(context.Background(), &sql.TxOptions{ReadOnly: true})
	if e != nil {
		return e
	}
	defer tx.Rollback()
	snapshot, e := export(tx, a)
	if e != nil {
		return e
	}
	if e = tx.Commit(); e != nil {
		return e
	}
	snapshot.SnapshotDigest, e = grant.SnapshotDigest(snapshot)
	if e != nil {
		return e
	}
	raw, e := json.Marshal(snapshot)
	if e != nil {
		return e
	}
	digest := sha256.Sum256(raw)
	receipt := map[string]any{"schemaVersion": snapshot.SchemaVersion, "sourceId": snapshot.SourceID, "generation": snapshot.Generation, "snapshotDigest": snapshot.SnapshotDigest, "snapshotArtifactSha256": "sha256:" + hex.EncodeToString(digest[:]), "sourceRevision": snapshot.SourceRevision, "sourceLedgerDigest": snapshot.SourceLedgerDigest, "sourceWatermark": snapshot.SourceWatermark, "principalCount": len(snapshot.Principals), "organizationCount": len(snapshot.Organizations), "membershipCount": len(snapshot.Memberships), "applicationCount": len(snapshot.Applications), "effectiveBindingCount": len(snapshot.EffectiveBindings)}
	receiptRaw, e := json.Marshal(receipt)
	if e != nil {
		return e
	}
	return persistBundle(a.output, raw, receiptRaw)
}
func rows(tx *sql.Tx, query string) ([][]string, error) {
	r, e := tx.Query(query)
	if e != nil {
		return nil, e
	}
	defer r.Close()
	cols, e := r.Columns()
	if e != nil {
		return nil, e
	}
	out := [][]string{}
	for r.Next() {
		row := make([]string, len(cols))
		dest := make([]any, len(cols))
		for i := range row {
			dest[i] = &row[i]
		}
		if e = r.Scan(dest...); e != nil {
			return nil, e
		}
		out = append(out, row)
	}
	return out, r.Err()
}
func watermark(h hash.Hash, values ...string) {
	for _, v := range values {
		var size [8]byte
		binary.BigEndian.PutUint64(size[:], uint64(len(v)))
		h.Write(size[:])
		h.Write([]byte(v))
	}
}
func relations(raw string) ([]string, error) {
	if raw == "" {
		return []string{}, nil
	}
	if strings.HasPrefix(raw, "[") {
		var out []string
		e := json.Unmarshal([]byte(raw), &out)
		return out, e
	}
	return []string{raw}, nil
}
func sortedValues[T any](m map[string]T) []T {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]T, 0, len(keys))
	for _, k := range keys {
		out = append(out, m[k])
	}
	return out
}
func export(tx *sql.Tx, a args) (grant.AuthoritySnapshotV2, error) {
	out := grant.AuthoritySnapshotV2{SchemaVersion: 2, SourceID: "patch-data", Generation: a.generation, SourceRevision: a.revision, SourceLedgerDigest: a.ledger, ProducedAt: a.produced, Applications: []grant.AuthorityApplicationV2{}}
	principals := map[string]grant.AuthorityPrincipalV2{}
	organizations := map[string]grant.AuthorityOrganizationV2{}
	memberships := map[string]grant.AuthorityMembershipV2{}
	bindings := map[string]grant.EffectiveBindingV2{}
	h := sha256.New()
	orgRows, e := rows(tx, "SELECT id,'1' FROM organizations ORDER BY id")
	if e != nil {
		return out, e
	}
	for _, r := range orgRows {
		watermark(h, r...)
		organizations[r[0]] = grant.AuthorityOrganizationV2{ID: r[0], Active: r[1] == "1"}
	}
	produced, _ := time.Parse(time.RFC3339, a.produced)
	for _, spec := range [][2]string{{"user", "SELECT id,deleted,organizations FROM users ORDER BY id"}, {"viewer", "SELECT id,deleted,organizations FROM viewers ORDER BY id"}, {"temporary", "SELECT id,expired,organization FROM temporary_accounts ORDER BY id"}, {"service", "SELECT id,CAST(enabled AS TEXT),'' FROM service_principals ORDER BY id"}} {
		rr, e := rows(tx, spec[1])
		if e != nil {
			return out, e
		}
		for _, r := range rr {
			id, state, orgs := r[0], r[1], r[2]
			watermark(h, spec[0], id, state, orgs)
			active := state == ""
			if spec[0] == "service" {
				active = state == "1"
			}
			if spec[0] == "temporary" {
				expiry, e := time.Parse(time.RFC3339, state)
				active = e == nil && expiry.After(produced)
			}
			principals[id] = grant.AuthorityPrincipalV2{ID: id, Kind: spec[0], Active: active, ExternalSubject: "patch-data:" + spec[0] + ":" + id}
			if spec[0] != "service" {
				orgIDs, e := relations(orgs)
				if e != nil {
					return out, e
				}
				for _, org := range orgIDs {
					mid := spec[0] + "-" + id + "-" + org
					role := spec[0]
					if role == "user" {
						role = "member"
					}
					lifecycle := "disabled"
					if active {
						lifecycle = "active"
					}
					var validUntil *string
					if spec[0] == "temporary" {
						validUntil = &state
					}
					memberships[mid] = grant.AuthorityMembershipV2{ID: mid, PrincipalID: id, OrganizationID: org, Role: role, PrincipalType: spec[0], Lifecycle: lifecycle, ValidUntil: validUntil, SourceRefs: []string{spec[0] + ":" + id}}
				}
			}
		}
	}
	ownerRows, e := rows(tx, "SELECT id,owner FROM organizations WHERE owner<>'' ORDER BY id")
	if e != nil {
		return out, e
	}
	for _, r := range ownerRows {
		watermark(h, r...)
		id := "owner-" + r[1] + "-" + r[0]
		memberships[id] = grant.AuthorityMembershipV2{ID: id, PrincipalID: r[1], OrganizationID: r[0], Role: "owner", PrincipalType: "user", Lifecycle: "active", SourceRefs: []string{"organizations.owner"}}
	}
	descendants := map[string]map[string]bool{}
	hierarchy, e := rows(tx, "SELECT organization,parent FROM org_parent_map ORDER BY parent,organization")
	if e != nil {
		return out, e
	}
	for _, r := range hierarchy {
		watermark(h, r...)
		if r[0] != r[1] {
			if descendants[r[1]] == nil {
				descendants[r[1]] = map[string]bool{}
			}
			descendants[r[1]][r[0]] = true
		}
	}
	plants, e := rows(tx, "SELECT id,organization FROM plants WHERE isArchive=0 ORDER BY id")
	if e != nil {
		return out, e
	}
	for _, r := range plants {
		watermark(h, r...)
	}
	memberValues := sortedValues(memberships)
	for _, m := range memberValues {
		if m.Lifecycle != "active" || m.PrincipalType != "user" {
			continue
		}
		for _, p := range plants {
			if p[1] == m.OrganizationID || descendants[m.OrganizationID][p[1]] {
				insertBinding(bindings, m.PrincipalID, m.OrganizationID, p[0], "descendant", []string{m.ID})
			}
		}
	}
	permissions, e := rows(tx, "SELECT plant,users,viewers,temporary FROM permissions ORDER BY plant")
	if e != nil {
		return out, e
	}
	for _, r := range permissions {
		watermark(h, r...)
		for i, kind := range []string{"user", "viewer", "temporary"} {
			ids, e := relations(r[i+1])
			if e != nil {
				return out, e
			}
			for _, principal := range ids {
				for _, m := range memberValues {
					if m.PrincipalID == principal && m.PrincipalType == kind && m.Lifecycle == "active" {
						derivation := "plant-permission"
						if kind == "temporary" {
							derivation = "temporary-assignment"
						}
						insertBinding(bindings, principal, m.OrganizationID, r[0], derivation, []string{"permissions"})
						break
					}
				}
			}
		}
	}
	out.SourceWatermark = "sha256:" + hex.EncodeToString(h.Sum(nil))
	out.Principals = sortedValues(principals)
	out.Organizations = sortedValues(organizations)
	out.Memberships = sortedValues(memberships)
	out.EffectiveBindings = sortedValues(bindings)
	return out, nil
}
func insertBinding(out map[string]grant.EffectiveBindingV2, principal, org, plant, derivation string, refs []string) {
	key := principal + "\x00" + org + "\x00" + plant
	if _, ok := out[key]; ok {
		return
	}
	out[key] = grant.EffectiveBindingV2{PrincipalID: principal, ApplicationScope: grant.ApplicationScopeV2{Kind: "wildcard", OrganizationID: org}, ProviderID: "patch-data", ActionID: "patch-data.plant.read", ResourceType: "plant", ResourceID: plant, Effect: "allow", SourceRuleIDs: refs, Derivation: derivation}
}
func persistBundle(target string, snapshot, receipt []byte) error {
	parent := filepath.Dir(target)
	temp, e := os.MkdirTemp(parent, ".authority-snapshot-")
	if e != nil {
		return e
	}
	defer os.RemoveAll(temp)
	for name, data := range map[string][]byte{"snapshot.json": snapshot, "receipt.json": receipt} {
		if e = os.WriteFile(filepath.Join(temp, name), data, 0600); e != nil {
			return e
		}
	}
	if e = renameNoReplace(temp, target); e != nil {
		return fmt.Errorf("atomically publish immutable snapshot bundle without replacement: %w", e)
	}
	return nil
}
