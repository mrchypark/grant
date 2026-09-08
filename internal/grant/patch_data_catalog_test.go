package grant

import (
	"context"
	"reflect"
	"strings"
	"testing"
)

func TestPatchDataCatalogStageAndIndependentAuthority(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := NewService(cfg, store)
	if _, err = svc.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "patch-data", Name: "Patch Data", Active: true}); err != nil {
		t.Fatal(err)
	}
	stage, actions, err := PatchDataCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(stage.Capabilities) != 369 || len(actions) != 138 {
		t.Fatal("incomplete inventory")
	}
	// Reproduce the existing baseline's short action code before adding reads.
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "patch-data", []Action{{ID: "patch-data.plant.read", ProviderID: "patch-data", Code: "plant.read", Name: "Read plant", RiskLevel: "low", GrantableToApplications: true}}); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	app := Application{ID: "catalog-app", OrganizationID: "conalog-grant", Slug: "catalog-app", Name: "Catalog", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if err = store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	if _, err = store.UpsertProviderPepClient(ctx, "patch-data", "catalog-pep", "conalog-grant", UpsertProviderPepClient{ApplicationID: app.ID, Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err = store.StageHttpCapabilityGeneration(ctx, "patch-data", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = store.ActivateHttpCapabilityGeneration(ctx, "patch-data", "conalog-grant", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	active, err := store.ActiveHttpCapabilityCatalog(ctx, "patch-data", "catalog-pep")
	if err != nil || !reflect.DeepEqual(active.Capabilities, stage.Capabilities) || !reflect.DeepEqual(active.ResponseResources, stage.ResponseResources) {
		t.Fatal("published catalog changed", err)
	}
	principal := applicationAuthorizationContext{UserID: "conalog-grant", OrganizationID: "conalog-grant", ApplicationID: app.ID}
	for i := 0; i < 168; i += 4 {
		list, get, alias := stage.Capabilities[i], stage.Capabilities[i+1], stage.Capabilities[i+2]
		t.Run(list.Resource.ResourceType, func(t *testing.T) {
			if !reflect.DeepEqual(list.Resource, alias.Resource) || !reflect.DeepEqual(list.RecordScope, alias.RecordScope) || *list.ActionID != *get.ActionID || get.Resource.ResourceType != list.Resource.ResourceType {
				t.Fatal("alias/detail authority differs")
			}
			if _, err = svc.authorizedRecordScope(ctx, "patch-data", list, principal); ErrorStatus(err) != 403 {
				t.Fatal("missing application admission accepted", err)
			}
			for _, sql := range []string{"INSERT INTO application_actions(application_id,action_id) VALUES('catalog-app',?)", "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner',?)"} {
				if _, err = store.Exec(ctx, sql, *list.ActionID); err != nil {
					t.Fatal(err)
				}
			}
			// The same ID in every resource namespace must not carry prior grants.
			scope, err := svc.authorizedRecordScope(ctx, "patch-data", list, principal)
			if err != nil || len(scope.IDs) != 0 {
				t.Fatal("inherited authority", scope, err)
			}
			binding := "binding-" + list.Resource.ResourceType
			if _, err = store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','conalog-grant','conalog-grant','patch-data',?,'aaaaaaaaaaaaaaa','role-system-bootstrap-owner','active','conalog-grant','now','now')", binding, list.Resource.ResourceType); err != nil {
				t.Fatal(err)
			}
			for _, cap := range []HttpCapability{list, alias} {
				scope, err = svc.authorizedRecordScope(ctx, "patch-data", cap, principal)
				if err != nil || !reflect.DeepEqual(scope.IDs, []string{"aaaaaaaaaaaaaaa"}) {
					t.Fatal("exact grant", scope, err)
				}
			}
			if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id=?", binding); err != nil {
				t.Fatal(err)
			}
			scope, err = svc.authorizedRecordScope(ctx, "patch-data", alias, principal)
			if err != nil || len(scope.IDs) != 0 {
				t.Fatal("revoked grant", scope, err)
			}
			// Leave earlier namespaces granted to prove later collections and views
			// cannot inherit them, even with the exact same record ID.
			if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='active' WHERE id=?", binding); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestPatchDataCatalogContract(t *testing.T) {
	a, _, err := PatchDataReadCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	b, _, err := PatchDataReadCatalog(1, nil)
	if err != nil || !reflect.DeepEqual(a, b) {
		t.Fatal("nondeterministic output", err)
	}
	if _, _, err := PatchDataReadCatalog(0, nil); err == nil {
		t.Fatal("zero generation accepted")
	}
	previous := uint64(1)
	for _, input := range []struct {
		generation uint64
		previous   *uint64
	}{{2, nil}, {3, &previous}} {
		if _, _, err := PatchDataReadCatalog(input.generation, input.previous); err == nil {
			t.Fatal("nonconsecutive generation accepted")
		}
	}
	for _, c := range a.Capabilities {
		if c.Method != "GET" || c.AccessMode != "authorized" || c.Request.Authorization != "service-token" {
			t.Fatal("unexpected publication", c.CapabilityID)
		}
		if c.RecordScope == nil {
			continue
		}
		for _, field := range c.RecordScope.AllowedFields {
			if strings.ContainsAny(field, ".@") || field == "password" || field == "link" || field == "verified" || field == "emailVisibility" {
				t.Fatal("unsafe filter field", field)
			}
		}
		if c.Resource.ResourceType == "plant" && !reflect.DeepEqual(c.RecordScope.AllowedFields, []string{"created", "id", "isArchive", "name", "updated"}) {
			t.Fatal("plants query regression")
		}
	}
}

func TestMergePatchDataReadActionsPreservesExisting(t *testing.T) {
	_, reads, err := PatchDataReadCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, code := range []string{"plant.read", "patch-data.plant.read"} {
		existing := []Action{{ID: "patch-data.plant.read", ProviderID: "patch-data", Code: code, Name: "Existing plant", RiskLevel: "low"}, {ID: "patch-data.file.write", ProviderID: "patch-data", Code: "file.write", Name: "Write"}}
		merged, err := MergePatchDataReadActions(existing, reads)
		if err != nil || len(merged) != len(reads)+len(existing)-1 || !reflect.DeepEqual(merged[:2], existing) {
			t.Fatal("existing actions changed", merged, err)
		}
	}
}
