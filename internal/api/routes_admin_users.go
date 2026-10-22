package api

import (
	"net/url"
	"strconv"
	"strings"
	"unicode/utf8"
)

func (s *Server) listAdminUsers(r *request) (any, error) {
	query, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil {
		return nil, failure(400, "malformed query")
	}
	for name, values := range query {
		maxBytes := 1024
		switch name {
		case "q", "cursor":
		case "limit":
			maxBytes = 20
		case "consistency":
			maxBytes = 32
		default:
			return nil, failure(400, "unknown query parameter: "+name)
		}
		if len(values) != 1 {
			return nil, failure(400, "duplicate "+name)
		}
		if !utf8.ValidString(values[0]) || len(values[0]) > maxBytes || strings.ContainsFunc(values[0], func(c rune) bool { return c < 32 || c == 127 }) {
			return nil, failure(400, "invalid "+name)
		}
	}
	if _, err := readConsistency(r.Request); err != nil {
		return nil, err
	}
	limit := 50
	if values, ok := query["limit"]; ok {
		parsed, err := strconv.Atoi(values[0])
		if err != nil || parsed < 1 || strings.ContainsFunc(values[0], func(c rune) bool { return c < '0' || c > '9' }) {
			return nil, failure(400, "limit must be positive")
		}
		limit = parsed
	}
	return s.service.ListUsersAdmin(r.Context(), query.Get("q"), query.Get("cursor"), limit)
}
