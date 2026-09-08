package grant

import (
	"context"
	"fmt"
	"path/filepath"
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
