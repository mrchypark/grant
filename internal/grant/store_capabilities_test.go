package grant

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"
	"testing"
)

func TestCapabilityLifecycleAndSnapshot(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	app := Application{ID: "pep-app", OrganizationID: "conalog-grant", Slug: "pep-app", Name: "PEP", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if e = s.InsertApplicationWithRedirectURI(ctx, app, nil); e != nil {
		t.Fatal(e)
	}
	if _, e = s.UpsertProviderPepClient(ctx, "grant", "pep", "master", UpsertProviderPepClient{ApplicationID: app.ID, Lifecycle: "active"}); e != nil {
		t.Fatal(e)
	}
	app.ID, app.Slug = "pep-app-2", "pep-app-2"
	if e = s.InsertApplicationWithRedirectURI(ctx, app, nil); e != nil {
		t.Fatal(e)
	}
	if _, e = s.UpsertProviderPepClient(ctx, "grant", "pep-2", "master", UpsertProviderPepClient{ApplicationID: app.ID, Lifecycle: "active"}); e != nil {
		t.Fatal(e)
	}
	app.ID = "pep-app"
	cap := HttpCapability{CapabilityID: "health", Version: 1, Enabled: true, AccessMode: "public_authn", Method: "GET", PathTemplate: "/health", Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{}, Redirects: "deny"}}
	cap.ContractDigest, e = HttpCapabilityContractDigest(cap)
	if e != nil {
		t.Fatal(e)
	}
	in := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []HttpCapability{cap}}
	in.Capabilities = nil
	for i := range 400 {
		item := cap
		item.CapabilityID = fmt.Sprintf("cap-%03d", i)
		item.PathTemplate = "/" + item.CapabilityID
		item.ContractDigest, e = HttpCapabilityContractDigest(item)
		if e != nil {
			t.Fatal(e)
		}
		in.Capabilities = append(in.Capabilities, item)
	}
	in.CatalogDigest, e = HttpCapabilityCatalogDigest("grant", in)
	if e != nil {
		t.Fatal(e)
	}
	canonical, e := generationJSON("grant", in)
	if e != nil || len(canonical) < 128<<10 {
		t.Fatalf("large capability fixture %d %v", len(canonical), e)
	}
	if _, e = s.StageHttpCapabilityGeneration(ctx, "grant", in); e != nil {
		t.Fatal(e)
	}
	if _, e = s.ActivateHttpCapabilityGeneration(ctx, "grant", "master", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: in.CatalogDigest}); e != nil {
		t.Fatal(e)
	}
	if _, e = s.ActiveHttpCapabilityCatalog(ctx, "grant", "pep"); e != nil {
		t.Fatal(e)
	}
	snap, e := s.ExportSnapshot(ctx)
	if e != nil {
		t.Fatal(e)
	}
	if e = RestoreStoreSnapshot(ctx, Config{Database: DatabaseConfig{DataDir: filepath.Join(t.TempDir(), "restored")}}, snap); e != nil {
		t.Fatal(e)
	}
	if _, e = s.UpsertProviderPepClient(ctx, "grant", "pep", "master", UpsertProviderPepClient{ApplicationID: app.ID, Lifecycle: "disabled"}); e != nil {
		t.Fatal(e)
	}
	if _, e = s.ActiveHttpCapabilityCatalog(ctx, "grant", "pep"); e == nil {
		t.Fatal("disabled PEP must fail")
	}
}

func TestPersistedCapabilityCatalogAssertionRollsBack(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err := s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	entry := `{"capabilityId":"read","version":1,"enabled":true,"accessMode":"public_authn","method":"GET","pathTemplate":"/read","contractDigest":"digest","query":{},"request":{},"response":{}}`
	for _, tc := range []struct {
		name, entries string
		count         int64
		valid         bool
	}{
		{"valid", "[" + entry + "]", 1, true},
		{"empty", "[]", 0, true},
		{"wrong-count", "[" + entry + "]", 2, false},
		{"duplicate", "[" + entry + "," + entry + "]", 2, false},
		{"zero-version", "[" + strings.Replace(entry, `"version":1`, `"version":0`, 1) + "]", 1, false},
		{"missing-enabled", "[" + strings.Replace(entry, `"enabled":true,`, "", 1) + "]", 1, false},
		{"invalid-enabled", "[" + strings.Replace(entry, `"enabled":true`, `"enabled":2`, 1) + "]", 1, false},
		{"invalid-access", "[" + strings.Replace(entry, `"public_authn"`, `"unknown"`, 1) + "]", 1, false},
		{"missing-method", "[" + strings.Replace(entry, `"method":"GET",`, "", 1) + "]", 1, false},
		{"not-array", "{}", 0, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := s.Execute(ctx, requestID(),
				Statement{SQL: "INSERT INTO provider_http_capability_sets(provider_id,generation,catalog_digest,canonical_json,created_at) VALUES('grant',1,?,?,?)", Args: []any{tc.name, `{"capabilities":` + tc.entries + `}`, nowString()}},
				capabilityCatalogAssertion("grant", 1, tc.count))
			if (err == nil) != tc.valid {
				t.Fatalf("valid=%v: %v", tc.valid, err)
			}
			rows, err := s.Query(ctx, ReadStrong, "SELECT (SELECT COUNT(*) FROM provider_http_capability_sets) AS catalogs,(SELECT COUNT(*) FROM application_consent_assertions) AS assertions")
			if err != nil || rowInt(rows[0], "assertions") != 0 || (rowInt(rows[0], "catalogs") == 1) != tc.valid {
				t.Fatalf("atomic persisted assertion: %v %v", rows, err)
			}
			if _, err := s.Exec(ctx, "DELETE FROM provider_http_capability_sets"); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestSimplifiedSchemaKeepsCanonicalSnapshotOnly(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	rows, err := s.Query(ctx, ReadStrong, `SELECT name FROM sqlite_schema WHERE name IN (
 'authority_import_assertions','provider_http_capability_assertions','provider_http_capabilities','authority_snapshots_v2',
 'idx_organization_memberships_org_user','idx_parties_business_no','idx_group_memberships_group_member',
 'idx_provider_http_activations_provider','idx_provider_http_activations_receipt','idx_provider_http_capabilities_match')`)
	if err != nil || len(rows) != 0 {
		t.Fatalf("retired physical state remains: %v %v", rows, err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil || len(snapshot.Tables) != 31 {
		t.Fatalf("canonical snapshot: %d tables, %v", len(snapshot.Tables), err)
	}
	snapshot.SchemaDigest = "old-schema"
	if err := ValidateStoreSnapshot(snapshot); err == nil {
		t.Fatal("different schema must not be silently restored")
	}
}
