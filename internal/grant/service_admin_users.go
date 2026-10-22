package grant

import (
	"context"
	"strings"
)

type UserDirectory struct {
	Items      []User `json:"items"`
	NextCursor string `json:"nextCursor,omitempty"`
}

func (s *Service) ListUsersAdmin(ctx context.Context, q, cursor string, limit int) (UserDirectory, error) {
	if limit < 0 {
		return UserDirectory{}, serviceError(400, "limit must be positive")
	}
	if limit == 0 {
		limit = 50
	}
	if limit > 100 {
		limit = 100
	}
	pattern := "%" + strings.NewReplacer(`\`, `\\`, "%", `\%`, "_", `\_`).Replace(q) + "%"
	// Fetch the page and its lookahead in one strong snapshot, not separate reads.
	items, err := serviceRows[User](ctx, s, ReadStrong, `SELECT id,email,display_name,active FROM users
WHERE id > ? AND (id LIKE ? ESCAPE '\' OR email LIKE ? ESCAPE '\' OR display_name LIKE ? ESCAPE '\')
ORDER BY id LIMIT ?`, cursor, pattern, pattern, pattern, limit+1)
	if err != nil {
		return UserDirectory{}, err
	}
	out := UserDirectory{Items: items}
	if len(items) > limit {
		out.Items = items[:limit]
		out.NextCursor = out.Items[limit-1].ID
	}
	return out, nil
}
