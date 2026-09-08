package grant

import (
	"context"
	"reflect"
	"strings"
	"testing"
)

func TestPatchDataNativeCatalogBoundaries(t *testing.T) {
	stage, actions, err := PatchDataCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(stage.Capabilities) != 322 || len(actions) != 121 {
		t.Fatal("inventory", len(stage.Capabilities), len(actions))
	}
	collections, err := PatchDataCollections()
	if err != nil {
		t.Fatal(err)
	}
	byID := map[string]HttpCapability{}
	for _, cap := range stage.Capabilities {
		byID[cap.CapabilityID] = cap
	}
	for _, collection := range collections {
		for _, suffix := range []string{"create", "update", "delete"} {
			name, found := byID[collection.Name+"."+suffix+".0"]
			if collection.Type == "view" {
				if found {
					t.Fatal("view mutation", name)
				}
				continue
			}
			alias, aliasFound := byID[collection.Name+"."+suffix+".1"]
			if !found || !aliasFound || !reflect.DeepEqual(name.Resource, alias.Resource) || *name.ActionID != *alias.ActionID {
				t.Fatal("missing canonical mutation aliases", collection.Name, suffix)
			}
			if suffix == "create" {
				if name.Resource.ResourceType != "collection" || name.Resource.ID.Kind != "constant" || name.Resource.ID.Value != collection.Name {
					t.Fatal("create target", name.Resource)
				}
			} else if name.Resource.ResourceType != collection.ResourceType || name.Resource.ID.Kind != "pathSegment" || name.Resource.ID.Format != collection.RecordIDFormat {
				t.Fatal("mutation target", name.Resource)
			}
			if suffix != "delete" && (name.Request.Body != "opaque" || !reflect.DeepEqual(name.Request.ContentTypes, []string{"application/json", "multipart/form-data"})) {
				t.Fatal("native request encoding", name.Request)
			}
		}
		file, found := byID[collection.Name+".file-get.0"]
		if found != collection.HasFiles {
			t.Fatal("file mapping", collection.Name)
		}
		if found && (*file.ActionID == "patch-data."+collection.ResourceType+".read" || file.Resource.ResourceType != collection.ResourceType) {
			t.Fatal("file read must be independent exact-record authority")
		}
	}
	cap := byID["org_parent_map.get.0"]
	materializedID := strings.Repeat("61", 15) + "3a" + strings.Repeat("62", 15)
	if err = scalarFormat(cap.Resource.ID.Format, materializedID); err != nil {
		t.Fatal("native materialized ID rejected", err)
	}
	if err = scalarFormat(cap.Resource.ID.Format, "../escape"); err == nil {
		t.Fatal("traversal accepted")
	}
}

func TestPatchDataMutationActionsRequireExactGrant(t *testing.T) {
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
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	app := Application{ID: "mutation-app", OrganizationID: "conalog-grant", Slug: "mutation-app", Name: "Mutation", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if err = store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	if _, err = store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) SELECT ?,id FROM actions WHERE provider_id='patch-data'", app.ID); err != nil {
		t.Fatal(err)
	}
	for _, cap := range stage.Capabilities[140:] {
		if !strings.HasSuffix(cap.CapabilityID, ".0") || cap.Method == "HEAD" {
			continue
		}
		t.Run(cap.CapabilityID, func(t *testing.T) {
			id := "aaaaaaaaaaaaaaa"
			if cap.Resource.ID.Kind == "constant" {
				id = cap.Resource.ID.Value
			}
			check := UserActionCheck{UserID: "conalog-grant", OrganizationID: "conalog-grant", ProviderID: "patch-data", ResourceType: cap.Resource.ResourceType, ResourceID: id, ActionID: *cap.ActionID, ApplicationID: &app.ID, Consistency: ReadStrong}
			assert := func(c UserActionCheck, want bool) {
				t.Helper()
				result, e := store.EvaluateUserActions(ctx, []UserActionCheck{c})
				if e != nil || len(result) != 1 || result[0].Allowed != want {
					t.Fatal("decision", want, result, e)
				}
			}
			assert(check, false)
			if _, err = store.Exec(ctx, "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner',?)", check.ActionID); err != nil {
				t.Fatal(err)
			}
			if _, err = store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('mutation-binding','user','conalog-grant','conalog-grant','patch-data',?,?,'role-system-bootstrap-owner','active','conalog-grant','now','now')", check.ResourceType, check.ResourceID); err != nil {
				t.Fatal(err)
			}
			assert(check, true)
			other := check
			other.ResourceID = "bbbbbbbbbbbbbbb"
			assert(other, false)
			other = check
			other.ActionID = "patch-data.plant.read"
			assert(other, false)
			if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='mutation-binding'"); err != nil {
				t.Fatal(err)
			}
			assert(check, false)
			if _, err = store.Exec(ctx, "DELETE FROM role_bindings WHERE id='mutation-binding'"); err != nil {
				t.Fatal(err)
			}
			if _, err = store.Exec(ctx, "DELETE FROM role_actions WHERE role_id='role-system-bootstrap-owner' AND action_id=?", check.ActionID); err != nil {
				t.Fatal(err)
			}
		})
	}
}
