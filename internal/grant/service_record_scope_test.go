package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"testing"
)

func TestRecordScopeUsesExistingEvaluatorAndAdmission(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer store.Close()
	if e = store.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	app := Application{ID: "scope-app", OrganizationID: "conalog-grant", Slug: "scope-app", Name: "Scope", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if e = store.InsertApplicationWithRedirectURI(ctx, app, nil); e != nil {
		t.Fatal(e)
	}
	action := "organization.profile.read"
	if _, e = store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) VALUES(?,?)", app.ID, action); e != nil {
		t.Fatal(e)
	}
	for _, id := range []string{"p2", "p1"} {
		if _, e = store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','conalog-grant','conalog-grant','grant','plant',?,'role-system-bootstrap-owner','active','conalog-grant','now','now')", id, id); e != nil {
			t.Fatal(e)
		}
	}
	svc := NewService(cfg, store)
	principal := applicationAuthorizationContext{UserID: "conalog-grant", OrganizationID: "conalog-grant", ApplicationID: app.ID}
	cap := HttpCapability{ActionID: &action, Resource: &HttpCapabilityResource{ResourceType: "plant", ID: HttpResourceExtractor{Kind: "constant", Value: "plant-list"}}, RecordScope: &HttpRecordScopePolicy{MaxIDs: 10}}
	scope, e := svc.authorizedRecordScope(ctx, "grant", cap, principal)
	if e != nil || !reflect.DeepEqual(scope.IDs, []string{"p1", "p2"}) {
		t.Fatal(scope, e)
	}
	cap.RecordScope.MaxIDs = 1
	if _, e = svc.authorizedRecordScope(ctx, "grant", cap, principal); ErrorStatus(e) != 503 {
		t.Fatal("overflow was truncated", e)
	}
	cap.RecordScope.MaxIDs = 10
	if _, e = store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='p1'"); e != nil {
		t.Fatal(e)
	}
	scope, e = svc.authorizedRecordScope(ctx, "grant", cap, principal)
	if e != nil || !reflect.DeepEqual(scope.IDs, []string{"p2"}) {
		t.Fatal("revocation", scope, e)
	}
	if _, e = store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='p2'"); e != nil {
		t.Fatal(e)
	}
	scope, e = svc.authorizedRecordScope(ctx, "grant", cap, principal)
	if e != nil || scope == nil || scope.IDs == nil || len(scope.IDs) != 0 {
		t.Fatal("empty scope", scope, e)
	}
	if _, e = store.Exec(ctx, "DELETE FROM application_actions WHERE application_id=?", app.ID); e != nil {
		t.Fatal(e)
	}
	if _, e = svc.authorizedRecordScope(ctx, "grant", cap, principal); ErrorStatus(e) != 403 {
		t.Fatal("missing app scope allowed empty", e)
	}
	if _, e = store.Exec(ctx, "UPDATE organization_memberships SET status='disabled' WHERE user_id='conalog-grant'"); e != nil {
		t.Fatal(e)
	}
	if _, e = svc.authorizedRecordScope(ctx, "grant", cap, principal); ErrorStatus(e) != 403 {
		t.Fatal("inactive member allowed empty", e)
	}
}
func TestRecordScopePolicyIsBoundedAndExplicit(t *testing.T) {
	action := "plant.read"
	c := HttpCapability{CapabilityID: "list", Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET", PathTemplate: "/records", ActionID: &action, Resource: &HttpCapabilityResource{ResourceType: "plant", ID: HttpResourceExtractor{Kind: "constant", Value: "plant-list"}}, Query: HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "filter", Format: "text", MaxBytes: 1024}}}, Request: HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: HttpResponsePolicy{MaxBytes: 1024, Redirects: "deny", ForwardHeaders: []string{}}, RecordScope: &HttpRecordScopePolicy{FilterParameter: "filter", RowKey: "id", Grammar: "fexpr", MaxIDs: 100, MaxFilterBytes: 8192, AllowedFields: []string{"id", "name"}}}
	if e := validateTestCapability(c); e != nil {
		t.Fatal(e)
	}
	for _, field := range []string{"plant.owner", "@collection.users", "id || true"} {
		c.RecordScope.AllowedFields = []string{field}
		if e := validateTestCapability(c); e == nil {
			t.Fatal("unsafe field", field)
		}
	}
	c.RecordScope.AllowedFields = []string{"id"}
	c.RecordScope.MaxIDs = 200
	if e := validateTestCapability(c); e == nil {
		t.Fatal("unbounded expression count")
	}
	c.RecordScope.MaxIDs = 100
	c.Query.Parameters[0].Repeated = true
	if e := validateTestCapability(c); e == nil {
		t.Fatal("repeated filter")
	}
}

func TestVoterSnapshotRetriesRatherThanCombiningViews(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer store.Close()
	if e = store.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	fence, e := store.QueryRaw(ctx, ReadStrong, "SELECT 1")
	if e != nil {
		t.Fatal(e)
	}
	attempts := 0
	active, e := readAtFence(ctx, NewService(cfg, store), fence.AppliedSlot, func(readCtx context.Context) (bool, error) {
		attempts++
		if _, e := store.Query(readCtx, ReadStrong, "SELECT active FROM users WHERE id='conalog-grant'"); e != nil {
			return false, e
		}
		if attempts == 1 {
			if _, e := store.Exec(ctx, "UPDATE users SET active=0 WHERE id='conalog-grant'"); e != nil {
				return false, e
			}
		}
		rows, e := store.Query(readCtx, ReadStrong, "SELECT active FROM users WHERE id='conalog-grant'")
		if e != nil {
			return false, e
		}
		if _, e = store.Exec(readCtx, "UPDATE users SET active=1"); e == nil {
			t.Fatal("write admitted inside scoped read")
		}
		return rowInt(rows[0], "active") != 0, nil
	})
	if e != nil || active || attempts != 2 {
		t.Fatalf("active=%v attempts=%d err=%v", active, attempts, e)
	}
}

func TestConcretePlantCollectionAliases(t *testing.T) {
	raw, e := os.ReadFile("testdata/pocketbase-plants-list.json")
	if e != nil {
		t.Fatal(e)
	}
	var capability HttpCapability
	if e = json.Unmarshal(raw, &capability); e != nil {
		t.Fatal(e)
	}
	generation := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1}
	for i, alias := range []string{"plants", "30hrtar148uy66c"} {
		c := capability
		c.CapabilityID = fmt.Sprintf("plants.list.%d", i)
		c.PathTemplate = "/api/collections/" + alias + "/records"
		c.ContractDigest, e = HttpCapabilityContractDigest(c)
		if e != nil {
			t.Fatal(e)
		}
		generation.Capabilities = append(generation.Capabilities, c)
	}
	generation.CatalogDigest, e = HttpCapabilityCatalogDigest("patch-data", generation)
	if e != nil {
		t.Fatal(e)
	}
	if e = validateGeneration("patch-data", generation); e != nil {
		t.Fatal(e)
	}
	for _, c := range generation.Capabilities {
		if c.Resource.ResourceType != "plant" || *c.ActionID != "patch-data.plant.read" || c.RecordScope == nil {
			t.Fatal("alias changed authority mapping")
		}
	}
}

func TestRecordScopeRechecksPEPInsideFence(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Security: SecurityConfig{HTTPCapabilityPEPClientID: "replacement-pep", ManagementApplicationID: "pep-app"}, Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer store.Close()
	if e = store.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	app := Application{ID: "pep-app", OrganizationID: "conalog-grant", Slug: "pep-app", Name: "PEP", ApplicationType: "confidential", AppType: "server", Active: true, ProvisioningStatus: "ready", ConsentPolicy: "explicit", CreatedByUserID: "conalog-grant"}
	if e = store.InsertApplicationWithRedirectURI(ctx, app, nil); e != nil {
		t.Fatal(e)
	}
	if _, e = store.UpsertProviderPepClient(ctx, "grant", "replacement-pep", "master", UpsertProviderPepClient{Lifecycle: "active"}); e != nil {
		t.Fatal(e)
	}
	c := HttpCapability{CapabilityID: "health", Version: 1, Enabled: true, AccessMode: "public_authn", Method: "GET", PathTemplate: "/health", Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{}, Redirects: "deny"}}
	c.ContractDigest, e = HttpCapabilityContractDigest(c)
	if e != nil {
		t.Fatal(e)
	}
	generation := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []HttpCapability{c}}
	generation.CatalogDigest, e = HttpCapabilityCatalogDigest("grant", generation)
	if e != nil {
		t.Fatal(e)
	}
	if _, e = store.StageHttpCapabilityGeneration(ctx, "grant", generation); e != nil {
		t.Fatal(e)
	}
	if _, e = store.ActivateHttpCapabilityGeneration(ctx, "grant", "master", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: generation.CatalogDigest}); e != nil {
		t.Fatal(e)
	}
	_, e = NewService(cfg, store).recordScopeDecision(ctx, "grant", c, applicationAuthorizationContext{}, 1, generation.CatalogDigest, "previous-pep")
	if ErrorStatus(e) != 403 {
		t.Fatal("stale PEP admitted", e)
	}
}
