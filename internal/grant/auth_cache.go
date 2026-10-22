package grant

import (
	"context"
	"crypto/sha256"
	"fmt"
	"strings"
	"time"
)

type cachedIdentity struct {
	Identity  ExternalIdentity
	ExpiresAt time.Time
}

func tokenCacheKey(token string) string {
	return fmt.Sprintf("%x", sha256.Sum256([]byte("rauthy\x00"+token)))
}
func rauthyBearer(value string) (string, error) {
	value = strings.TrimSpace(value)
	if strings.HasPrefix(value, "Bearer ") || strings.HasPrefix(value, "bearer ") {
		value = strings.TrimSpace(value[7:])
		if value != "" {
			return value, nil
		}
	}
	return "", serviceError(401, "unauthorized: missing bearer token")
}
func (s *Service) cachedIdentity(key string) (cachedIdentity, bool) {
	s.authMu.Lock()
	defer s.authMu.Unlock()
	v, ok := s.authCache[key]
	if ok && !v.ExpiresAt.After(time.Now()) {
		delete(s.authCache, key)
		ok = false
	}
	return v, ok
}
func (s *Service) cacheIdentity(key string, v cachedIdentity) {
	s.authMu.Lock()
	defer s.authMu.Unlock()
	now := time.Now()
	for k, c := range s.authCache {
		if !c.ExpiresAt.After(now) {
			delete(s.authCache, k)
		}
	}
	if v.ExpiresAt.After(now) {
		s.authCache[key] = v
	}
}
func (s *Service) resolveExternalIdentity(ctx context.Context, authorization string, c ReadConsistency) (ExternalIdentity, error) {
	if identity, ok := ctx.Value(replicaIdentityKey{}).(ExternalIdentity); ok {
		return identity, nil
	}
	if s.AuthProviderMode() != "rauthy" || s.store == nil {
		return s.externalIdentity(ctx, authorization)
	}
	token, bearerErr := rauthyBearer(authorization)
	if bearerErr != nil {
		return ExternalIdentity{}, bearerErr
	}
	if token == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	key := tokenCacheKey(token)
	if v, ok := s.cachedIdentity(key); ok {
		return v.Identity, nil
	}
	rows, err := s.store.Query(ctx, s.effectiveConsistency(c), "SELECT * FROM auth_identity_cache WHERE token_hash=? AND provider='rauthy'", key)
	if err != nil {
		return ExternalIdentity{}, err
	}
	if len(rows) == 1 {
		r := rows[0]
		expires, e := time.Parse(time.RFC3339Nano, rowString(r, "expires_at"))
		if e == nil && expires.After(time.Now()) {
			i := ExternalIdentity{Provider: "rauthy", ExternalUserID: rowString(r, "external_user_id"), ClientID: rowString(r, "client_id"), Email: rowNullable(r, "email"), DisplayName: rowString(r, "display_name"), Active: rowInt(r, "active") != 0}
			s.cacheIdentity(key, cachedIdentity{i, expires})
			return i, nil
		}
	}
	identity, err := s.externalIdentity(ctx, authorization)
	if err != nil {
		return ExternalIdentity{}, err
	}
	effective := s.effectiveConsistency(c)
	if effective == ReadStrong || effective == ReadPreferStrong {
		if v, ok := s.cachedIdentity(key); ok {
			err = s.store.Transaction(ctx, []Statement{{SQL: "DELETE FROM auth_identity_cache WHERE expires_at<=?", Args: []any{nowString()}}, {SQL: "INSERT INTO auth_identity_cache(token_hash,provider,external_user_id,client_id,email,display_name,active,expires_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(token_hash) DO UPDATE SET provider=excluded.provider,external_user_id=excluded.external_user_id,client_id=excluded.client_id,email=excluded.email,display_name=excluded.display_name,active=excluded.active,expires_at=excluded.expires_at,updated_at=excluded.updated_at", Args: []any{key, identity.Provider, identity.ExternalUserID, identity.ClientID, identity.Email, identity.DisplayName, identity.Active, v.ExpiresAt.UTC().Format(time.RFC3339Nano), nowString()}}})
			if err != nil {
				return ExternalIdentity{}, err
			}
		}
	}
	return identity, nil
}
