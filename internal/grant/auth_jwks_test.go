package grant

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"math/big"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

func signTestJWT(t *testing.T, alg, kid string, signer any, claims map[string]any) string {
	t.Helper()
	hdrJSON, _ := json.Marshal(map[string]string{"alg": alg, "kid": kid, "typ": "JWT"})
	claimsJSON, _ := json.Marshal(claims)
	input := base64.RawURLEncoding.EncodeToString(hdrJSON) + "." + base64.RawURLEncoding.EncodeToString(claimsJSON)
	var sig []byte
	switch alg {
	case "RS256":
		h := sha256.Sum256([]byte(input))
		s, err := rsa.SignPKCS1v15(rand.Reader, signer.(*rsa.PrivateKey), crypto.SHA256, h[:])
		if err != nil {
			t.Fatal(err)
		}
		sig = s
	case "ES256":
		h := sha256.Sum256([]byte(input))
		r, s, err := ecdsa.Sign(rand.Reader, signer.(*ecdsa.PrivateKey), h[:])
		if err != nil {
			t.Fatal(err)
		}
		sig = make([]byte, 64)
		rb, sb := r.Bytes(), s.Bytes()
		copy(sig[32-len(rb):32], rb)
		copy(sig[64-len(sb):64], sb)
	case "EdDSA":
		sig = ed25519.Sign(signer.(ed25519.PrivateKey), []byte(input))
	case "none", "HS256":
		sig = []byte("fake")
	default:
		t.Fatalf("unsupported test alg %s", alg)
	}
	return input + "." + base64.RawURLEncoding.EncodeToString(sig)
}

func TestJWKSIdentityVerificationAndRotation(t *testing.T) {
	rsaKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	ecKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	edPub, edPriv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}

	var includeRotated atomic.Bool
	var jwksCalls atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		jwksCalls.Add(1)
		keys := []map[string]string{
			{
				"kty": "RSA",
				"use": "sig",
				"kid": "rsa-1",
				"n":   base64.RawURLEncoding.EncodeToString(rsaKey.N.Bytes()),
				"e":   base64.RawURLEncoding.EncodeToString(big.NewInt(int64(rsaKey.E)).Bytes()),
			},
			{
				"kty": "EC",
				"use": "sig",
				"kid": "ec-1",
				"crv": "P-256",
				"x":   base64.RawURLEncoding.EncodeToString(ecKey.X.Bytes()),
				"y":   base64.RawURLEncoding.EncodeToString(ecKey.Y.Bytes()),
			},
		}
		if includeRotated.Load() {
			keys = append(keys, map[string]string{
				"kty": "OKP",
				"use": "sig",
				"kid": "ed-2",
				"crv": "Ed25519",
				"x":   base64.RawURLEncoding.EncodeToString(edPub),
			})
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"keys": keys})
	}))
	defer srv.Close()

	cfg := Config{Auth: AuthConfig{
		ProviderMode: "jwks",
		Rauthy: RauthyConfig{
			JWKSURL:             srv.URL,
			Issuer:              "https://goauthy.example.test",
			Audience:            "grant-api",
			JWKSCacheTTLSeconds: 300,
		},
	}}
	svc := NewService(cfg, nil)
	ctx := context.Background()
	now := time.Now().Unix()

	tokRSA := signTestJWT(t, "RS256", "rsa-1", rsaKey, map[string]any{
		"iss": "https://goauthy.example.test",
		"aud": "grant-api",
		"sub": "user-rsa",
		"azp": "client-1",
		"exp": now + 60,
	})
	id, err := svc.externalIdentity(ctx, "Bearer "+tokRSA)
	if err != nil || id.ExternalUserID != "user-rsa" || id.ClientID != "client-1" || id.Provider != "jwks" {
		t.Fatalf("RS256 failed: id=%+v err=%v", id, err)
	}

	tokEC := signTestJWT(t, "ES256", "ec-1", ecKey, map[string]any{
		"iss":       "https://goauthy.example.test",
		"aud":       []string{"grant-api"},
		"sub":       "user-ec",
		"client_id": "client-ec",
		"exp":       now + 60,
	})
	id, err = svc.externalIdentity(ctx, "Bearer "+tokEC)
	if err != nil || id.ExternalUserID != "user-ec" || id.ClientID != "client-ec" {
		t.Fatalf("ES256 failed: id=%+v err=%v", id, err)
	}
	if jwksCalls.Load() != 1 {
		t.Fatalf("expected 1 cached JWKS call, got %d", jwksCalls.Load())
	}

	svc.jwks.mu.Lock()
	svc.jwks.lastRefresh = time.Now().Add(-10 * time.Second)
	svc.jwks.mu.Unlock()
	includeRotated.Store(true)
	tokEd := signTestJWT(t, "EdDSA", "ed-2", edPriv, map[string]any{
		"iss":       "https://goauthy.example.test",
		"aud":       "grant-api",
		"sub":       "user-ed",
		"client_id": "client-ed",
		"exp":       now + 60,
	})
	id, err = svc.externalIdentity(ctx, "Bearer "+tokEd)
	if err != nil || id.ExternalUserID != "user-ed" || jwksCalls.Load() != 2 {
		t.Fatalf("EdDSA rotation failed: id=%+v err=%v calls=%d", id, err, jwksCalls.Load())
	}

	badTokens := []struct {
		name  string
		token string
	}{
		{"expired", signTestJWT(t, "RS256", "rsa-1", rsaKey, map[string]any{"iss": "https://goauthy.example.test", "aud": "grant-api", "sub": "u", "client_id": "c", "exp": now - 10})},
		{"future_nbf", signTestJWT(t, "RS256", "rsa-1", rsaKey, map[string]any{"iss": "https://goauthy.example.test", "aud": "grant-api", "sub": "u", "client_id": "c", "nbf": now + 60, "exp": now + 120})},
		{"bad_iss", signTestJWT(t, "RS256", "rsa-1", rsaKey, map[string]any{"iss": "https://evil.test", "aud": "grant-api", "sub": "u", "client_id": "c", "exp": now + 60})},
		{"bad_aud", signTestJWT(t, "RS256", "rsa-1", rsaKey, map[string]any{"iss": "https://goauthy.example.test", "aud": "other-api", "sub": "u", "client_id": "c", "exp": now + 60})},
		{"alg_none", signTestJWT(t, "none", "rsa-1", nil, map[string]any{"iss": "https://goauthy.example.test", "aud": "grant-api", "sub": "u", "client_id": "c", "exp": now + 60})},
		{"alg_hs256", signTestJWT(t, "HS256", "rsa-1", nil, map[string]any{"iss": "https://goauthy.example.test", "aud": "grant-api", "sub": "u", "client_id": "c", "exp": now + 60})},
	}
	for _, tc := range badTokens {
		if _, err := svc.externalIdentity(ctx, "Bearer "+tc.token); err == nil || ErrorStatus(err) != 401 {
			t.Fatalf("%s: expected 401, got err=%v", tc.name, err)
		}
	}
}
