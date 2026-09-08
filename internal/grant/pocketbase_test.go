package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestPocketBaseCollectionAndOptionalEmail(t *testing.T) {
	payload := `{"token":"fresh","record":{"id":"same_id","collectionId":"viewers_id","collectionName":"viewers","username":"Viewer","verified":false}}`
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/collections/viewers/auth-refresh" {
			t.Error(r.URL.Path)
		}
		fmt.Fprint(w, payload)
	}))
	defer upstream.Close()
	s := NewService(Config{Auth: AuthConfig{ProviderMode: "pocketbase", PocketBase: &PocketBaseConfig{BaseURL: upstream.URL, Collection: "viewers", ApplicationID: "app"}}}, nil)
	i, e := s.externalIdentity(context.Background(), "Bearer opaque")
	if e != nil || i.Email != nil || i.DisplayName != "Viewer" || i.Issuer != upstream.URL || i.CollectionID != "viewers_id" {
		t.Fatalf("identity=%+v err=%v", i, e)
	}
	payload = strings.ReplaceAll(payload, `"collectionName":"viewers"`, `"collectionName":"users"`)
	if _, e = s.externalIdentity(context.Background(), "Bearer opaque"); ErrorStatus(e) != 503 {
		t.Fatalf("collection mismatch accepted: %v", e)
	}
	for _, raw := range []string{`{"id":"a","collectionId":null,"collectionName":"users"}`, `{"id":"a","collectionId":"c","collectionName":null}`} {
		var r pocketBaseRecord
		if json.Unmarshal([]byte(raw), &r) == nil {
			t.Fatal("null identity accepted")
		}
	}
}

func TestPocketBaseScopedIdentityAndImport(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	email := "same@example.test"
	base := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "same_id", Email: &email, DisplayName: "User"}
	first, e := s.MaterializeExternalIdentity(ctx, base)
	if e != nil {
		t.Fatal(e)
	}
	again, e := s.MaterializeExternalIdentity(ctx, base)
	if e != nil || again.ID != first.ID {
		t.Fatal("identity retry changed user", e)
	}
	for _, other := range []ExternalIdentityMaterialization{{Provider: "pocketbase", Issuer: "https://other.example.test", CollectionID: base.CollectionID, ExternalUserID: base.ExternalUserID, Email: &email, DisplayName: "Other issuer"}, {Provider: "pocketbase", Issuer: base.Issuer, CollectionID: "viewers_id", ExternalUserID: base.ExternalUserID, Email: &email, DisplayName: "Other collection"}} {
		u, e := s.MaterializeExternalIdentity(ctx, other)
		if e != nil || u.ID == first.ID {
			t.Fatal("namespace/email collision", e)
		}
	}
	if _, e = s.BindIdentityToUser(ctx, BindIdentityInput{Provider: "pocketbase", UserID: first.ID, ExternalUserID: "same_id"}); e == nil {
		t.Fatal("unqualified PocketBase binding accepted")
	}
	in := PatchDataAuthorityImport{ManifestVersion: "grant.patch-data-authority.v1", Generation: "one", LedgerDigest: strings.Repeat("a", 64), Users: []PatchDataImportedUser{{UserID: "imported-user", Issuer: base.Issuer, CollectionID: base.CollectionID, ExternalUserID: "imported_id", DisplayName: "Imported", Active: true}}}
	in.ManifestDigest = PatchDataManifestDigest(in)
	if _, e = s.ImportPatchDataAuthority(ctx, in); e != nil {
		t.Fatal(e)
	}
	u, e := s.MaterializeExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: base.Issuer, CollectionID: base.CollectionID, ExternalUserID: "imported_id", DisplayName: "Current"})
	if e != nil || u.ID != "imported-user" {
		t.Fatal("explicit import namespace did not resolve", e)
	}
	bad := in
	bad.Users = append([]PatchDataImportedUser(nil), in.Users...)
	bad.Users[0].Issuer = ""
	bad.ManifestDigest = PatchDataManifestDigest(bad)
	if _, e = s.ImportPatchDataAuthority(ctx, bad); e == nil {
		t.Fatal("unqualified import accepted")
	}
}

func TestCapabilityServiceCredentialContract(t *testing.T) {
	action := "asset.read"
	c := HttpCapability{CapabilityID: "asset-read", Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET", PathTemplate: "/assets/{id}", ActionID: &action, Resource: &HttpCapabilityResource{ResourceType: "asset", ID: HttpResourceExtractor{Kind: "pathSegment", Name: "id", Format: "asciiToken"}}, Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{}, Redirects: "deny"}}
	before, e := HttpCapabilityContractDigest(c)
	if e != nil {
		t.Fatal(e)
	}
	c.Request.Authorization = "service-token"
	c.ContractDigest, e = HttpCapabilityContractDigest(c)
	if e != nil || c.ContractDigest == before {
		t.Fatal("service credential not bound to contract digest", e)
	}
	if e = validateCapability(c); e != nil {
		t.Fatal(e)
	}
	for _, mode := range []string{"public_authn", "invalid"} {
		c.AccessMode = HttpCapabilityAccessMode(mode)
		if validateCapability(c) == nil {
			t.Fatal("service credential on non-authorized route")
		}
	}
	c.AccessMode = "authorized"
	c.Request.Authorization = "arbitrary"
	if validateCapability(c) == nil {
		t.Fatal("unknown credential policy")
	}
}

func TestPocketBaseConfigOrigin(t *testing.T) {
	for _, tc := range [][2]string{{"http://patch-auth.patch.svc.cluster.local:8090/", "http://patch-auth.patch.svc.cluster.local:8090"}, {"https://AUTH.example.test:443/", "https://auth.example.test"}} {
		got, e := canonicalPocketBaseOrigin(tc[0])
		if e != nil || got != tc[1] {
			t.Fatal(got, e)
		}
	}
	for _, bad := range []string{"https://auth.example.test/path", "https://auth.example.test?", "https://u:p@auth.example.test", "https://auth.example.test/#fragment"} {
		if _, e := canonicalPocketBaseOrigin(bad); e == nil {
			t.Fatal("unsafe origin", bad)
		}
	}
}
