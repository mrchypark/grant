package grant

import (
	"context"
	"fmt"
	"reflect"
	"testing"
)

func generationReadFixture(t *testing.T, count int) StageHttpCapabilityGeneration {
	t.Helper()
	stage := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: make([]HttpCapability, 0, count)}
	for i := range count {
		cap := HttpCapability{CapabilityID: fmt.Sprintf("read-%03d", i), Version: 1, Enabled: true, AccessMode: "public_authn", Method: "GET", PathTemplate: fmt.Sprintf("/read-%03d", i), Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{}, Redirects: "deny"}}
		var err error
		cap.ContractDigest, err = HttpCapabilityContractDigest(cap)
		if err != nil {
			t.Fatal(err)
		}
		stage.Capabilities = append(stage.Capabilities, cap)
	}
	return stage
}

func TestHttpCapabilityGenerationAdminStoredLifecycle(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: ReadLocal}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	if err := store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := NewService(cfg, store)
	stage := generationReadFixture(t, 400)
	stage.ResponseResources = []HttpResponseResource{{Key: "organizations", ActionID: "organization.profile.read", ResourceType: "organization", IDField: "id", IDFormat: "asciiToken", IdentityField: "kind", IdentityValue: "organization", Relations: []HttpResponseRelation{}}}
	stage.CatalogDigest, err = HttpCapabilityCatalogDigest("grant", stage)
	if err != nil {
		t.Fatal(err)
	}
	canonical, err := generationJSON("grant", stage)
	if err != nil || len(canonical) <= 60000 {
		t.Fatalf("large fixture: bytes=%d err=%v", len(canonical), err)
	}
	if _, err := svc.StageHttpCapabilityGenerationAdmin(ctx, "grant", stage); err != nil {
		t.Fatal(err)
	}
	check := func(want StageHttpCapabilityGeneration) {
		t.Helper()
		got, err := svc.HttpCapabilityGenerationAdmin(ctx, "grant", want.Generation)
		if err != nil {
			t.Fatal(err)
		}
		if got.ProviderID != "grant" || got.Generation != want.Generation || got.CatalogDigest != want.CatalogDigest || got.CapabilityCount != len(want.Capabilities) || got.ResponseResourceCount != len(want.ResponseResources) || !reflect.DeepEqual(got.Capabilities, want.Capabilities) || !reflect.DeepEqual(got.ResponseResources, want.ResponseResources) {
			t.Fatalf("stored generation changed: generation=%d", got.Generation)
		}
		digest, err := HttpCapabilityCatalogDigest(got.ProviderID, StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: got.Generation, Capabilities: got.Capabilities, ResponseResources: got.ResponseResources})
		if err != nil || digest != got.CatalogDigest {
			t.Fatalf("digest=%s err=%v", digest, err)
		}
	}
	check(stage)
	if _, err := svc.ActiveHttpCapabilityCatalogAdmin(ctx, "grant"); ErrorStatus(err) != 404 {
		t.Fatalf("staged read activated catalog: %v", err)
	}
	app := Application{ID: "generation-pep-app", OrganizationID: "conalog-grant", Slug: "generation-pep-app", Name: "PEP", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if err := store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := store.UpsertProviderPepClient(ctx, "grant", "generation-pep", "operator", UpsertProviderPepClient{ApplicationID: app.ID, Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ActivateHttpCapabilityGeneration(ctx, "grant", "operator", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	next := generationReadFixture(t, 1)
	next.Generation = 2
	next.ExpectedLatestGeneration = &stage.Generation
	next.ResponseResources = []HttpResponseResource{}
	next.CatalogDigest, err = HttpCapabilityCatalogDigest("grant", next)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := svc.StageHttpCapabilityGenerationAdmin(ctx, "grant", next); err != nil {
		t.Fatal(err)
	}
	check(next)
	check(stage)
	if _, err := store.ActivateHttpCapabilityGeneration(ctx, "grant", "operator", ActivateHttpCapabilityGeneration{ExpectedGeneration: &stage.Generation, ExpectedCatalogDigest: &stage.CatalogDigest, TargetGeneration: 2, TargetCatalogDigest: next.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	before, err := svc.ActiveHttpCapabilityCatalogAdmin(ctx, "grant")
	if err != nil {
		t.Fatal(err)
	}
	check(stage) // Historical read after a different generation becomes active.
	check(next)
	after, err := svc.ActiveHttpCapabilityCatalogAdmin(ctx, "grant")
	if err != nil || !reflect.DeepEqual(before, after) {
		t.Fatalf("read changed active pointer: %v", err)
	}
	for _, tc := range []struct {
		provider   string
		generation uint64
	}{{"grant", 3}, {"missing", 1}} {
		if _, err := svc.HttpCapabilityGenerationAdmin(ctx, tc.provider, tc.generation); ErrorStatus(err) != 404 {
			t.Fatalf("missing generation: %v", err)
		}
	}
}
