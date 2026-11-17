package grant

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"math/big"
	"net/http"
	"strings"
	"sync"
	"time"
)

type jwksCache struct {
	mu          sync.RWMutex
	refreshMu   sync.Mutex
	keys        map[string]crypto.PublicKey
	fetchedAt   time.Time
	lastRefresh time.Time
}

type jwkDocument struct {
	Keys []jwkKey `json:"keys"`
}

type jwkKey struct {
	Kty string `json:"kty"`
	Use string `json:"use"`
	Alg string `json:"alg"`
	Kid string `json:"kid"`
	Crv string `json:"crv"`
	N   string `json:"n"`
	E   string `json:"e"`
	X   string `json:"x"`
	Y   string `json:"y"`
}

func (s *Service) jwksIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	token, err := rauthyBearer(authorization)
	if err != nil {
		return ExternalIdentity{}, err
	}
	if token == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt format")
	}
	headerBytes, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt header")
	}
	var hdr struct {
		Alg string `json:"alg"`
		Kid string `json:"kid"`
		Typ string `json:"typ"`
	}
	if err := json.Unmarshal(headerBytes, &hdr); err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt header")
	}
	if hdr.Alg != "RS256" && hdr.Alg != "ES256" && hdr.Alg != "EdDSA" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: unsupported jwt algorithm")
	}
	if strings.TrimSpace(hdr.Kid) == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt header missing kid")
	}
	pub, err := s.resolveJWKSKey(ctx, hdr.Kid)
	if err != nil {
		return ExternalIdentity{}, err
	}
	sigBytes, err := base64.RawURLEncoding.DecodeString(parts[2])
	if err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt signature encoding")
	}
	signingInput := []byte(parts[0] + "." + parts[1])
	if err := verifyJWSSignature(hdr.Alg, pub, signingInput, sigBytes); err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt signature")
	}

	payloadBytes, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt payload")
	}
	var claims struct {
		Iss           string          `json:"iss"`
		Sub           string          `json:"sub"`
		Aud           json.RawMessage `json:"aud"`
		Exp           *float64        `json:"exp"`
		Nbf           *float64        `json:"nbf"`
		ClientID      string          `json:"client_id"`
		Azp           string          `json:"azp"`
		Email         *string         `json:"email"`
		EmailVerified bool            `json:"email_verified"`
		Name          string          `json:"name"`
		Username      string          `json:"preferred_username"`
		GivenName     string          `json:"given_name"`
		FamilyName    string          `json:"family_name"`
	}
	if err := json.Unmarshal(payloadBytes, &claims); err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt claims")
	}
	now := float64(time.Now().Unix())
	if claims.Exp == nil || *claims.Exp <= now {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt is expired")
	}
	if claims.Nbf != nil && *claims.Nbf > now {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt is not yet valid")
	}
	cfg := s.cfg.Auth.Rauthy
	if cfg.Issuer != "" && claims.Iss != cfg.Issuer {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt issuer mismatch")
	}
	audList, err := parseJWTAudience(claims.Aud)
	if err != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid jwt audience")
	}
	if cfg.Audience != "" && !containsString(audList, cfg.Audience) {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt audience mismatch")
	}
	sub := strings.TrimSpace(claims.Sub)
	if sub == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt missing subject")
	}
	clientID := strings.TrimSpace(claims.ClientID)
	if clientID == "" {
		clientID = strings.TrimSpace(claims.Azp)
	}
	if clientID == "" && len(audList) == 1 {
		clientID = strings.TrimSpace(audList[0])
	}
	if clientID == "" {
		clientID = strings.TrimSpace(cfg.ClientID)
	}
	if clientID == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: jwt missing client id")
	}
	email := claims.Email
	if !claims.EmailVerified {
		email = nil
	}
	name := ""
	for _, v := range []string{claims.Name, claims.Username, strings.TrimSpace(claims.GivenName + " " + claims.FamilyName), ptrValue(email), sub} {
		if strings.TrimSpace(v) != "" {
			name = strings.TrimSpace(v)
			break
		}
	}
	provider := s.AuthProviderMode()
	if provider == "" {
		provider = "jwks"
	}
	return ExternalIdentity{
		Provider:       provider,
		ExternalUserID: sub,
		ClientID:       clientID,
		Email:          email,
		DisplayName:    name,
		Active:         true,
	}, nil
}

func (s *Service) resolveJWKSKey(ctx context.Context, kid string) (crypto.PublicKey, error) {
	if s.jwks == nil {
		s.jwks = &jwksCache{}
	}
	ttl := time.Duration(s.cfg.Auth.Rauthy.JWKSCacheTTLSeconds) * time.Second
	if ttl <= 0 {
		ttl = 300 * time.Second
	}

	s.jwks.mu.RLock()
	fresh := !s.jwks.fetchedAt.IsZero() && time.Since(s.jwks.fetchedAt) < ttl
	pub, ok := s.jwks.keys[kid]
	canRefresh := time.Since(s.jwks.lastRefresh) >= 5*time.Second
	s.jwks.mu.RUnlock()

	if fresh && ok {
		return pub, nil
	}
	if !fresh || (!ok && canRefresh) {
		if err := s.refreshJWKS(ctx, kid, ttl); err != nil {
			if ok {
				return pub, nil
			}
			return nil, err
		}
		s.jwks.mu.RLock()
		pub, ok = s.jwks.keys[kid]
		s.jwks.mu.RUnlock()
	}
	if !ok {
		return nil, serviceError(401, "unauthorized: unknown jwt key id")
	}
	return pub, nil
}

func (s *Service) refreshJWKS(ctx context.Context, kid string, ttl time.Duration) error {
	s.jwks.refreshMu.Lock()
	defer s.jwks.refreshMu.Unlock()

	s.jwks.mu.RLock()
	fresh := !s.jwks.fetchedAt.IsZero() && time.Since(s.jwks.fetchedAt) < ttl
	_, ok := s.jwks.keys[kid]
	recentlyRefreshed := time.Since(s.jwks.lastRefresh) < 5*time.Second
	s.jwks.mu.RUnlock()
	if fresh && (ok || recentlyRefreshed) {
		return nil
	}

	s.jwks.mu.Lock()
	s.jwks.lastRefresh = time.Now()
	s.jwks.mu.Unlock()

	req, err := http.NewRequestWithContext(ctx, "GET", s.cfg.Auth.Rauthy.JWKSURL, nil)
	if err != nil {
		return serviceError(503, "unavailable: invalid jwks request")
	}
	resp, err := s.client.Do(req)
	if err != nil {
		return serviceError(503, "unavailable: jwks endpoint is unavailable")
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return serviceError(503, "unavailable: jwks endpoint returned error")
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 262145))
	if err != nil || len(body) > 262144 {
		return serviceError(503, "unavailable: jwks response is too large")
	}
	var doc jwkDocument
	if err := json.Unmarshal(body, &doc); err != nil {
		return serviceError(503, "unavailable: jwks response was invalid")
	}
	parsed := make(map[string]crypto.PublicKey, len(doc.Keys))
	for _, k := range doc.Keys {
		if k.Kid == "" || (k.Use != "" && k.Use != "sig") {
			continue
		}
		if pub, err := parseJWK(k); err == nil {
			parsed[k.Kid] = pub
		}
	}
	s.jwks.mu.Lock()
	s.jwks.keys = parsed
	s.jwks.fetchedAt = time.Now()
	s.jwks.mu.Unlock()
	return nil
}

func parseJWK(k jwkKey) (crypto.PublicKey, error) {
	switch k.Kty {
	case "RSA":
		nb, err := base64.RawURLEncoding.DecodeString(k.N)
		if err != nil || len(nb) == 0 {
			return nil, fmt.Errorf("invalid rsa modulus")
		}
		eb, err := base64.RawURLEncoding.DecodeString(k.E)
		if err != nil || len(eb) == 0 {
			return nil, fmt.Errorf("invalid rsa exponent")
		}
		n := new(big.Int).SetBytes(nb)
		e := int(new(big.Int).SetBytes(eb).Int64())
		if e <= 1 || n.BitLen() < 2048 {
			return nil, fmt.Errorf("unsafe rsa key")
		}
		return &rsa.PublicKey{N: n, E: e}, nil
	case "EC":
		if k.Crv != "P-256" {
			return nil, fmt.Errorf("unsupported ec curve")
		}
		xb, err := base64.RawURLEncoding.DecodeString(k.X)
		if err != nil {
			return nil, fmt.Errorf("invalid ec x")
		}
		yb, err := base64.RawURLEncoding.DecodeString(k.Y)
		if err != nil {
			return nil, fmt.Errorf("invalid ec y")
		}
		x := new(big.Int).SetBytes(xb)
		y := new(big.Int).SetBytes(yb)
		if !elliptic.P256().IsOnCurve(x, y) {
			return nil, fmt.Errorf("ec point not on P-256")
		}
		return &ecdsa.PublicKey{Curve: elliptic.P256(), X: x, Y: y}, nil
	case "OKP":
		if k.Crv != "Ed25519" {
			return nil, fmt.Errorf("unsupported okp curve")
		}
		xb, err := base64.RawURLEncoding.DecodeString(k.X)
		if err != nil || len(xb) != ed25519.PublicKeySize {
			return nil, fmt.Errorf("invalid ed25519 key")
		}
		return ed25519.PublicKey(xb), nil
	default:
		return nil, fmt.Errorf("unsupported kty")
	}
}

func verifyJWSSignature(alg string, pub crypto.PublicKey, signingInput, sig []byte) error {
	switch alg {
	case "RS256":
		rsaPub, ok := pub.(*rsa.PublicKey)
		if !ok {
			return fmt.Errorf("key type mismatch")
		}
		h := sha256.Sum256(signingInput)
		return rsa.VerifyPKCS1v15(rsaPub, crypto.SHA256, h[:], sig)
	case "ES256":
		ecPub, ok := pub.(*ecdsa.PublicKey)
		if !ok || len(sig) != 64 {
			return fmt.Errorf("key type or signature length mismatch")
		}
		h := sha256.Sum256(signingInput)
		r := new(big.Int).SetBytes(sig[:32])
		s := new(big.Int).SetBytes(sig[32:])
		if !ecdsa.Verify(ecPub, h[:], r, s) {
			return fmt.Errorf("ecdsa verify failed")
		}
		return nil
	case "EdDSA":
		edPub, ok := pub.(ed25519.PublicKey)
		if !ok {
			return fmt.Errorf("key type mismatch")
		}
		if !ed25519.Verify(edPub, signingInput, sig) {
			return fmt.Errorf("ed25519 verify failed")
		}
		return nil
	default:
		return fmt.Errorf("unsupported alg")
	}
}

func parseJWTAudience(raw json.RawMessage) ([]string, error) {
	if len(raw) == 0 || string(raw) == "null" {
		return nil, nil
	}
	var single string
	if err := json.Unmarshal(raw, &single); err == nil {
		return []string{single}, nil
	}
	var many []string
	if err := json.Unmarshal(raw, &many); err == nil {
		return many, nil
	}
	return nil, fmt.Errorf("invalid aud")
}

func containsString(list []string, target string) bool {
	for _, item := range list {
		if item == target {
			return true
		}
	}
	return false
}
