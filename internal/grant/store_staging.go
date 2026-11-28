package grant

import (
	"context"
	"fmt"
	"unicode/utf8"
)

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
