package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"unicode/utf8"
)

// Only immutable, unreferenced input is staged across commits. Canonical authorization
// rows, the generation receipt, and stage removal are changed by ONE final transaction.
func (s *Store) stageImport(ctx context.Context, id string, c PatchDataAuthorityImport, identities []map[string]any) error {
	groups := map[string]any{"users": c.Users, "organizations": c.Organizations, "memberships": c.Memberships, "hierarchyEdges": c.HierarchyEdges, "plantBindings": c.PlantBindings, "applicationAssignments": c.ApplicationAssignments, "identities": identities}
	for kind, values := range groups {
		raw, e := json.Marshal(values)
		if e != nil {
			return e
		}
		var rows []json.RawMessage
		if e = json.Unmarshal(raw, &rows); e != nil {
			return e
		}
		ordinal := 0
		for len(rows) > 0 {
			n := 0
			size := 2
			for n < len(rows) && size+len(rows[n])+1 < 60000 {
				size += len(rows[n]) + 1
				n++
			}
			if n == 0 {
				return fmt.Errorf("import entity exceeds staging limit")
			}
			payload, e := json.Marshal(rows[:n])
			if e != nil {
				return e
			}
			if _, e = s.Execute(ctx, requestID(), Statement{SQL: "INSERT INTO grant_import_payloads(import_id,kind,ordinal,payload) VALUES(?,?,?,?) ON CONFLICT(import_id,kind,ordinal) DO UPDATE SET payload=excluded.payload WHERE grant_import_payloads.payload=excluded.payload", Args: []any{id, kind, ordinal, string(payload)}, ExpectedRowsAffected: requireOne()}); e != nil {
				return e
			}
			rows = rows[n:]
			ordinal++
		}
	}
	return nil
}
func stagedStatements(id, j, ij string, ss []Statement) []Statement {
	re := regexp.MustCompile(`json_each\(\?(?:,\s*'\$\.([A-Za-z]+)')?\)`)
	for i := range ss {
		q := ss[i].SQL
		remove := map[int]bool{}
		for _, m := range re.FindAllStringSubmatchIndex(q, -1) {
			remove[strings.Count(q[:m[0]], "?")] = true
		}
		q = re.ReplaceAllStringFunc(q, func(fragment string) string {
			m := re.FindStringSubmatch(fragment)
			kind := m[1]
			if kind == "" {
				kind = "identities"
			}
			return stageRowsSQL(id, kind)
		})
		args := []any{}
		for index, a := range ss[i].Args {
			if remove[index] {
				continue
			}
			args = append(args, a)
		}
		ss[i].SQL = q
		ss[i].Args = args
	}
	return append(ss, Statement{SQL: "DELETE FROM grant_import_payloads WHERE import_id=?", Args: []any{id}})
}
func stageRowsSQL(id, kind string) string {
	return "(SELECT j.value FROM grant_import_payloads p,json_each(p.payload) j WHERE p.import_id='" + id + "' AND p.kind='" + kind + "')"
}

func (s *Store) stageText(ctx context.Context, id, kind, payload string) error {
	ordinal := 0
	for len(payload) > 0 {
		n := len(payload)
		if n > 60000 {
			n = 60000
		}
		for n > 0 && !utf8.ValidString(payload[:n]) {
			n--
		}
		if n == 0 {
			return fmt.Errorf("invalid UTF-8 staging payload")
		}
		if _, e := s.Execute(ctx, requestID(), Statement{SQL: "INSERT INTO grant_import_payloads(import_id,kind,ordinal,payload) VALUES(?,?,?,?) ON CONFLICT(import_id,kind,ordinal) DO UPDATE SET payload=excluded.payload WHERE grant_import_payloads.payload=excluded.payload", Args: []any{id, kind, ordinal, payload[:n]}, ExpectedRowsAffected: requireOne()}); e != nil {
			return e
		}
		payload = payload[n:]
		ordinal++
	}
	return nil
}
func stagedTextSQL() string {
	return "(SELECT group_concat(payload,'') FROM (SELECT payload FROM grant_import_payloads WHERE import_id=? AND kind=? ORDER BY ordinal))"
}
